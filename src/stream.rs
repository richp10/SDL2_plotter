use crate::config::StreamingConfig;
use image::codecs::jpeg::JpegEncoder;
use sdl2::pixels::PixelFormatEnum;
use sdl2::render::WindowCanvas;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc::{sync_channel, SyncSender, TrySendError};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

/// One captured frame, as read back from the canvas, waiting to be encoded.
struct RawFrame {
    pixels: Vec<u8>,
    width: u32,
    height: u32,
}

#[derive(Clone)]
pub struct Streamer {
    /// Latest encoded frame, written by the encoder thread and read by the
    /// client threads. Only tests read it through the struct.
    #[cfg_attr(not(test), allow(dead_code))]
    frame: Arc<RwLock<Option<Vec<u8>>>>,
    frame_interval: Duration,
    last_capture: Arc<std::sync::Mutex<Instant>>,
    /// Hands raw frames to the encoder thread. Capacity one: if the encoder
    /// is still busy with the previous frame, the new one is dropped rather
    /// than queued, so the stream lags by at most one frame.
    encoder: SyncSender<RawFrame>,
}

impl Streamer {
    /// The streamer the configuration asks for, or `None` when streaming is
    /// off or the address cannot be bound. A bind failure is logged as an
    /// ERROR and the chart plotter carries on without the stream: a wrong
    /// address in plotter.toml must never keep the chart off the screen.
    pub fn start_or_disable(config: &StreamingConfig) -> Option<Self> {
        if !config.enabled {
            return None;
        }
        match Self::start(config) {
            Ok(streamer) => Some(streamer),
            Err(error) => {
                crate::diagnostics::record(
                    "ERROR",
                    format!(
                        "Flux MJPEG désactivé: impossible d'écouter sur {}:{} ({error}).                          Vérifier [streaming] bind_address dans plotter.toml",
                        config.bind_address, config.port
                    ),
                );
                None
            }
        }
    }

    pub fn start(config: &StreamingConfig) -> anyhow::Result<Self> {
        let frame = Arc::new(RwLock::new(None));
        let address = format!("{}:{}", config.bind_address, config.port);
        let listener = listen(&address).map_err(|error| anyhow::anyhow!(error))?;

        crate::diagnostics::record(
            "INFO",
            format!("Flux MJPEG démarré sur http://{address}/stream"),
        );

        let server_frame = Arc::clone(&frame);
        std::thread::spawn(move || {
            serve(listener, server_frame);
        });

        let (encoder, raw_frames) = sync_channel::<RawFrame>(1);
        let encoder_frame = Arc::clone(&frame);
        let quality = config.jpeg_quality;
        std::thread::spawn(move || {
            for raw in raw_frames {
                let mut jpeg = Vec::new();
                match JpegEncoder::new_with_quality(&mut jpeg, quality).encode(
                    &raw.pixels,
                    raw.width,
                    raw.height,
                    image::ColorType::Rgb8,
                ) {
                    Ok(()) => {
                        if let Ok(mut frame) = encoder_frame.write() {
                            *frame = Some(jpeg);
                        }
                    }
                    Err(error) => {
                        crate::diagnostics::record("WARN", format!("Encodage JPEG échoué: {error}"))
                    }
                }
            }
        });

        Ok(Self {
            frame,
            frame_interval: Duration::from_secs_f64(1.0 / config.fps as f64),
            last_capture: Arc::new(std::sync::Mutex::new(
                Instant::now().checked_sub(Duration::from_secs(1)).unwrap(),
            )),
            encoder,
        })
    }

    /// Reads the rendered frame back from the canvas, at most `fps` times a
    /// second, and hands it to the encoder thread. The readback has to happen
    /// on the render thread; the JPEG encoding does not.
    pub fn capture(&self, canvas: &WindowCanvas) -> Result<(), String> {
        let mut last = self
            .last_capture
            .lock()
            .map_err(|_| "stream lock".to_string())?;
        if last.elapsed() < self.frame_interval {
            return Ok(());
        }
        *last = Instant::now();

        let (width, height) = canvas.output_size()?;
        let pixels = canvas.read_pixels(None, PixelFormatEnum::RGB24)?;
        self.submit(pixels, width, height);
        Ok(())
    }

    /// Queues RGB24 pixels for encoding. Returns false when the frame was
    /// dropped because the encoder had not finished the previous one.
    pub(crate) fn submit(&self, pixels: Vec<u8>, width: u32, height: u32) -> bool {
        match self.encoder.try_send(RawFrame {
            pixels,
            width,
            height,
        }) {
            Ok(()) => true,
            Err(TrySendError::Full(_)) | Err(TrySendError::Disconnected(_)) => false,
        }
    }
}

fn listen(address: &str) -> std::io::Result<TcpListener> {
    let listener = TcpListener::bind(address)?;
    listener.set_nonblocking(true)?;
    Ok(listener)
}

fn serve(listener: TcpListener, frame: Arc<RwLock<Option<Vec<u8>>>>) {
    loop {
        match listener.accept() {
            Ok((mut stream, _)) => {
                // The listener is non-blocking so this loop can poll it. On
                // Windows an accepted socket inherits that flag (Linux does
                // not), and the handler's first read would then fail with
                // WouldBlock whenever the request has not arrived yet,
                // dropping the connection. Each client socket must block.
                if let Err(error) = stream.set_nonblocking(false) {
                    crate::diagnostics::record(
                        "WARN",
                        format!("Client MJPEG refusé (mode bloquant): {error}"),
                    );
                    continue;
                }
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
    // A viewer that stops reading must not keep this thread blocked forever.
    stream.set_write_timeout(Some(Duration::from_secs(10)))?;
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
            let jpeg = frame.read().ok().and_then(|current| current.clone());
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
                if stream.write_all(&jpeg).is_err() || stream.write_all(b"\r\n").is_err() {
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

#[cfg(test)]
mod tests {
    use super::*;

    const FAKE_JPEG: &[u8] = &[0xff, 0xd8, 0x01, 0x02, 0x03, 0xff, 0xd9];

    /// Starts the real server loop on a free loopback port with a frame
    /// already available, exactly as `Streamer::start` would.
    fn start_test_server() -> std::net::SocketAddr {
        let listener = listen("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let frame = Arc::new(RwLock::new(Some(FAKE_JPEG.to_vec())));
        std::thread::spawn(move || serve(listener, frame));
        address
    }

    /// Connects, waits `request_delay` before sending the request (as a
    /// browser or a slow network might), and returns how long the first
    /// complete frame took to arrive.
    fn time_to_first_frame(
        address: std::net::SocketAddr,
        request_delay: Duration,
    ) -> Result<Duration, String> {
        let started = Instant::now();
        let mut client = TcpStream::connect(address).map_err(|e| e.to_string())?;
        client
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        std::thread::sleep(request_delay);
        client
            .write_all(b"GET /stream HTTP/1.1\r\nHost: plotter\r\n\r\n")
            .map_err(|e| format!("request not accepted: {e}"))?;

        let mut received = Vec::new();
        let mut chunk = [0u8; 4096];
        loop {
            match client.read(&mut chunk) {
                Ok(0) => return Err(format!("connection closed after {} bytes", received.len())),
                Ok(n) => {
                    received.extend_from_slice(&chunk[..n]);
                    if received.windows(FAKE_JPEG.len()).any(|w| w == FAKE_JPEG) {
                        return Ok(started.elapsed());
                    }
                }
                Err(e) => return Err(format!("no frame after {:?}: {e}", started.elapsed())),
            }
        }
    }

    /// cargo test --release bench_ -- --ignored --nocapture
    #[test]
    #[ignore]
    fn bench_jpeg_encode_of_a_full_frame() {
        let (width, height) = (1600u32, 800u32);
        let pixels: Vec<u8> = (0..width * height)
            .flat_map(|i| {
                let (x, y) = (i % width, i / width);
                [(x % 256) as u8, (y % 256) as u8, ((x + y) % 256) as u8]
            })
            .collect();
        let mut samples: Vec<Duration> = (0..10)
            .map(|_| {
                let start = Instant::now();
                let mut jpeg = Vec::new();
                JpegEncoder::new_with_quality(&mut jpeg, 70)
                    .encode(&pixels, width, height, image::ColorType::Rgb8)
                    .unwrap();
                start.elapsed()
            })
            .collect();
        samples.sort();
        println!(
            "bench jpeg encode 1600x800 q70: {:?}",
            samples[samples.len() / 2]
        );
    }

    fn config_on(port: u16) -> StreamingConfig {
        StreamingConfig {
            enabled: true,
            bind_address: "127.0.0.1".into(),
            port,
            fps: 5,
            jpeg_quality: 70,
        }
    }

    #[test]
    fn an_unbindable_address_disables_the_stream_instead_of_failing() {
        // Occupy a port, then ask for a streamer on the same port.
        let taken = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = taken.local_addr().unwrap().port();
        assert!(Streamer::start_or_disable(&config_on(port)).is_none());
        drop(taken);
    }

    #[test]
    fn a_free_address_starts_the_stream() {
        assert!(Streamer::start_or_disable(&config_on(0)).is_some());
    }

    #[test]
    fn streaming_disabled_in_config_starts_nothing() {
        let mut config = config_on(0);
        config.enabled = false;
        assert!(Streamer::start_or_disable(&config).is_none());
    }

    fn test_streamer() -> Streamer {
        Streamer::start(&StreamingConfig {
            enabled: true,
            bind_address: "127.0.0.1".into(),
            port: 0,
            fps: 5,
            jpeg_quality: 70,
        })
        .unwrap()
    }

    fn gradient(width: u32, height: u32) -> Vec<u8> {
        (0..width * height)
            .flat_map(|i| {
                let (x, y) = (i % width, i / width);
                [(x % 256) as u8, (y % 256) as u8, ((x + y) % 256) as u8]
            })
            .collect()
    }

    #[test]
    fn submitted_pixels_are_encoded_off_the_calling_thread() {
        let streamer = test_streamer();
        let pixels = gradient(1600, 800);
        let started = Instant::now();
        assert!(streamer.submit(pixels, 1600, 800));
        let submit_took = started.elapsed();

        let encode_started = Instant::now();
        let deadline = encode_started + Duration::from_secs(20);
        let jpeg = loop {
            if let Some(jpeg) = streamer.frame.read().unwrap().clone() {
                break jpeg;
            }
            assert!(Instant::now() < deadline, "no frame was encoded");
            std::thread::sleep(Duration::from_millis(1));
        };
        let encode_took = encode_started.elapsed();
        assert_eq!(&jpeg[..2], &[0xff, 0xd8], "frame should be a JPEG");
        // The submitting thread must not pay for the encoding. Compared with
        // the encode time, not a fixed limit, so a loaded machine (a full
        // parallel test run) slows both sides and the test stays true.
        assert!(
            submit_took * 5 < encode_took,
            "submit took {submit_took:?}, the encode after it {encode_took:?}"
        );
    }

    #[test]
    fn a_frame_submitted_while_the_encoder_is_busy_is_dropped_not_queued() {
        let streamer = test_streamer();
        let pixels = gradient(1600, 800);
        let mut accepted = 0;
        for _ in 0..5 {
            if streamer.submit(pixels.clone(), 1600, 800) {
                accepted += 1;
            }
        }
        assert!(
            (1..=2).contains(&accepted),
            "accepted {accepted} of 5 back-to-back frames"
        );
    }

    /// cargo test --release bench_ -- --ignored --nocapture --test-threads=1
    #[test]
    #[ignore]
    fn bench_submit_frame_cost_on_the_render_thread() {
        let streamer = test_streamer();
        let pixels = gradient(1600, 800);
        let mut samples: Vec<Duration> = (0..10)
            .map(|_| {
                std::thread::sleep(Duration::from_millis(150));
                let frame = pixels.clone();
                let start = Instant::now();
                streamer.submit(frame, 1600, 800);
                start.elapsed()
            })
            .collect();
        samples.sort();
        println!(
            "bench submit frame (render thread cost): {:?}",
            samples[samples.len() / 2]
        );
    }

    #[test]
    fn first_frame_arrives_promptly_when_the_request_is_sent_immediately() {
        let address = start_test_server();
        let elapsed = time_to_first_frame(address, Duration::ZERO).unwrap();
        assert!(elapsed < Duration::from_millis(1500), "{elapsed:?}");
    }

    #[test]
    fn first_frame_arrives_promptly_when_the_request_is_sent_after_a_pause() {
        let address = start_test_server();
        let elapsed = time_to_first_frame(address, Duration::from_millis(100)).unwrap();
        assert!(elapsed < Duration::from_millis(1500), "{elapsed:?}");
    }

    #[test]
    fn every_connection_in_a_burst_is_served() {
        let address = start_test_server();
        let failures: Vec<String> = (0..20)
            .filter_map(|_| time_to_first_frame(address, Duration::from_millis(5)).err())
            .collect();
        assert!(
            failures.is_empty(),
            "{} of 20 failed: {:?}",
            failures.len(),
            failures.first()
        );
    }
}
