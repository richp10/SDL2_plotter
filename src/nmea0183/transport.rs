//! NMEA 0183 over IP: bytes from a TCP connection or UDP datagrams, cut
//! into lines.

use std::time::Duration;

use tokio::io::AsyncReadExt;
use tokio::net::{TcpStream, UdpSocket};

/// A sentence is at most 82 characters, and a gateway can put a tag block
/// (`\...\`) before it. A longer line is not NMEA 0183.
pub(super) const MAX_LINE: usize = 256;
/// No data on a TCP connection for this long: connect again.
pub(super) const READ_TIMEOUT: Duration = Duration::from_secs(30);

/// Cuts a byte stream into lines. Keeps a partial line between reads.
/// Drops a line longer than `MAX_LINE`, up to its next newline, and a
/// line that is not UTF-8, so a wrong port cannot fill memory.
#[derive(Default)]
pub(super) struct LineSplitter {
    line: Vec<u8>,
    overflow: bool,
}

impl LineSplitter {
    pub fn push(&mut self, bytes: &[u8]) -> Vec<String> {
        let mut lines = Vec::new();
        for &byte in bytes {
            if byte == b'\n' {
                if !self.overflow {
                    if let Ok(line) = String::from_utf8(std::mem::take(&mut self.line)) {
                        if !line.is_empty() {
                            lines.push(line);
                        }
                    }
                }
                self.line.clear();
                self.overflow = false;
            } else if !self.overflow {
                self.line.push(byte);
                if self.line.len() > MAX_LINE {
                    self.line.clear();
                    self.overflow = true;
                }
            }
        }
        lines
    }
}

/// Connects to `address` and passes every line to `on_line`. Connects
/// again after an error, a close or `READ_TIMEOUT` with no data, waiting
/// one second more after each failed try, at most ten. Never returns.
pub(super) async fn tcp(address: &str, mut on_line: impl FnMut(&str)) {
    let mut attempt: u64 = 0;
    loop {
        attempt += 1;
        match tokio::time::timeout(READ_TIMEOUT, TcpStream::connect(address)).await {
            Ok(Ok(mut stream)) => {
                crate::diagnostics::record(
                    "INFO",
                    format!("NMEA 0183: connecté à {address} (tentative {attempt})"),
                );
                attempt = 0;
                let mut splitter = LineSplitter::default();
                let mut buffer = [0u8; 2048];
                loop {
                    match tokio::time::timeout(READ_TIMEOUT, stream.read(&mut buffer)).await {
                        Ok(Ok(0)) => {
                            crate::diagnostics::record(
                                "WARN",
                                format!("NMEA 0183: {address} a fermé la connexion"),
                            );
                            break;
                        }
                        Ok(Ok(count)) => {
                            for line in splitter.push(&buffer[..count]) {
                                on_line(&line);
                            }
                        }
                        Ok(Err(error)) => {
                            crate::diagnostics::record(
                                "WARN",
                                format!("NMEA 0183: erreur de lecture sur {address}: {error}"),
                            );
                            break;
                        }
                        Err(_) => {
                            crate::diagnostics::record(
                                "WARN",
                                format!(
                                    "NMEA 0183: aucune donnée de {address} depuis {}s; reconnexion",
                                    READ_TIMEOUT.as_secs()
                                ),
                            );
                            break;
                        }
                    }
                }
            }
            Ok(Err(error)) => {
                crate::diagnostics::record(
                    "WARN",
                    format!(
                        "NMEA 0183: connexion à {address} échouée (tentative {attempt}): {error}"
                    ),
                );
            }
            Err(_) => {
                crate::diagnostics::record(
                    "WARN",
                    format!("NMEA 0183: connexion à {address} trop longue (tentative {attempt})"),
                );
            }
        }
        tokio::time::sleep(Duration::from_secs(attempt.clamp(1, 10))).await;
    }
}

/// Binds `address` and passes every line of every datagram to `on_line`.
/// Binds again after ten seconds when the address cannot be bound. Never
/// returns.
pub(super) async fn udp(address: &str, mut on_line: impl FnMut(&str)) {
    loop {
        match UdpSocket::bind(address).await {
            Ok(socket) => {
                let _ = socket.set_broadcast(true);
                crate::diagnostics::record("INFO", format!("NMEA 0183: écoute UDP sur {address}"));
                let mut buffer = [0u8; 2048];
                loop {
                    match socket.recv_from(&mut buffer).await {
                        Ok((count, _)) => {
                            let mut splitter = LineSplitter::default();
                            let mut lines = splitter.push(&buffer[..count]);
                            lines.extend(splitter.push(b"\n"));
                            for line in lines {
                                on_line(&line);
                            }
                        }
                        Err(error) => {
                            crate::diagnostics::record(
                                "WARN",
                                format!(
                                    "NMEA 0183: erreur de réception UDP sur {address}: {error}"
                                ),
                            );
                            tokio::time::sleep(Duration::from_secs(1)).await;
                        }
                    }
                }
            }
            Err(error) => {
                crate::diagnostics::record(
                    "ERROR",
                    format!("NMEA 0183: impossible d'écouter sur {address}: {error}"),
                );
                tokio::time::sleep(Duration::from_secs(10)).await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_are_split_across_reads() {
        let mut s = LineSplitter::default();
        assert!(s.push(b"$GPRMC,1").is_empty());
        assert_eq!(
            s.push(b"23*00\r\n$GPHDT"),
            vec!["$GPRMC,123*00\r".to_string()]
        );
    }

    #[test]
    fn a_line_longer_than_the_limit_is_dropped() {
        let mut s = LineSplitter::default();
        assert!(s.push(&[b'x'; 300]).is_empty());
        assert_eq!(s.push(b"yyy\n$OK\n"), vec!["$OK".to_string()]);
    }

    #[test]
    fn a_tag_block_does_not_make_a_full_sentence_too_long() {
        let line = format!("\\s:sim,c:1764000000*00\\${}*00", "x".repeat(79));
        let mut s = LineSplitter::default();
        assert_eq!(s.push(format!("{line}\r\n").as_bytes()).len(), 1);
    }

    #[test]
    fn bytes_that_are_not_utf8_are_dropped() {
        let mut s = LineSplitter::default();
        assert_eq!(s.push(b"\xff\xfe\n$OK\n"), vec!["$OK".to_string()]);
    }
}
