use crate::config::StreamingConfig;
use image::codecs::jpeg::JpegEncoder;
use sdl2::pixels::PixelFormatEnum;
use sdl2::render::WindowCanvas;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

#[derive(Clone)]
pub struct Streamer {
    frame: Arc<RwLock<Option<Vec<u8>>>>,
    frame_interval: Duration,
    jpeg_quality: u8,
    last_capture: Arc<std::sync::Mutex<Instant>>,
}

impl Streamer {
    pub fn start(config: &StreamingConfig) -> anyhow::Result<Self> {
        let frame = Arc::new(RwLock::new(None));
        let address = format!("{}:{}", config.bind_address, config.port);
        let listener =
            TcpListener::bind(&address).map_err(|error| anyhow::anyhow!(error))?;
        listener.set_nonblocking(true).ok();

        crate::diagnostics::record(
            "INFO",
            format!("Flux MJPEG démarré sur http://{address}/stream"),
        );

        let server_frame = Arc::clone(&frame);
        std::thread::spawn(move || {
            serve(listener, server_frame);
        });

        Ok(Self {
            frame,
            frame_interval: Duration::from_secs_f64(1.0 / config.fps as f64),
            jpeg_quality: config.jpeg_quality,
            last_capture: Arc::new(std::sync::Mutex::new(
                Instant::now().checked_sub(Duration::from_secs(1)).unwrap(),
            )),
        })
    }

    pub fn capture(&self, canvas: &WindowCanvas) -> Result<(), String> {
        let mut last = self.last_capture.lock().map_err(|_| "stream lock".to_string())?;
        if last.elapsed() < self.frame_interval {
            return Ok(());
        }
        *last = Instant::now();

        let (output_width, output_height) = canvas.output_size()?;
        let width = output_width;
        let height = output_height;
        let pixels = canvas.read_pixels(None, PixelFormatEnum::RGB24)?;
        let mut jpeg = Vec::new();
        JpegEncoder::new_with_quality(&mut jpeg, self.jpeg_quality)
            .encode(&pixels, width, height, image::ColorType::Rgb8)
            .map_err(|error| error.to_string())?;

        if let Ok(mut frame) = self.frame.write() {
            *frame = Some(jpeg);
        }
        Ok(())
    }
}

fn serve(listener: TcpListener, frame: Arc<RwLock<Option<Vec<u8>>>>) {
    loop {
        match listener.accept() {
            Ok((mut stream, _)) => {
                let frame = Arc::clone(&frame);
                std::thread::spawn(move || {
                    let _ = handle_client(&mut stream, frame);
                });
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(error) => {
                crate::diagnostics::record("ERROR", format!("Erreur serveur MJPEG: {error}"));
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }
}

fn handle_client(
    stream: &mut TcpStream,
    frame: Arc<RwLock<Option<Vec<u8>>>>,
) -> anyhow::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let mut request = [0u8; 4096];
    let size = stream.read(&mut request)?;
    let request_text = String::from_utf8_lossy(&request[..size]);

    if request_text.starts_with("GET /stream") {
        stream.write_all(
            b"HTTP/1.1 200 OK\r\n\
              Content-Type: multipart/x-mixed-replace; boundary=frame\r\n\
              Cache-Control: no-cache\r\n\
              Connection: close\r\n\r\n",
        )?;

        loop {
            let jpeg = frame
                .read()
                .ok()
                .and_then(|current| current.clone());
            if let Some(jpeg) = jpeg {
                if stream
                    .write_all(
                        format!(
                            "--frame\r\nContent-Type: image/jpeg\r\nContent-Length: {}\r\n\r\n",
                            jpeg.len()
                        )
                        .as_bytes(),
                    )
                    .is_err()
                {
                    break;
                }
                if stream.write_all(&jpeg).is_err()
                    || stream.write_all(b"\r\n").is_err()
                {
                    break;
                }
            }
            std::thread::sleep(Duration::from_millis(200));
        }
    } else {
        let html = r#"<!doctype html><html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>Plotter</title></head><body style="margin:0;background:#111"><img src="/stream" style="width:100vw;height:100vh;object-fit:contain"></body></html>"#;
        stream.write_all(
            format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                html.len()
            )
            .as_bytes(),
        )?;
        stream.write_all(html.as_bytes())?;
    }
    Ok(())
}
