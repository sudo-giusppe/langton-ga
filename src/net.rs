//! Minimal line-oriented TCP transport.
//!
//! Messages are newline-terminated UTF-8 lines.  The protocol is deliberately
//! small (and easy to inspect with `nc`), because the point of the experiment
//! is the island model, not a middleware stack:
//!
//! ```text
//! MIG <sender> <generation> <genome-hex>[,<genome-hex>...]
//! TEL <island> <gen> <best> <mean> <diversity> <evaluations> <elapsed_ms>
//! END <island> <best> <gen> <elapsed_ms> <messages> <bytes>
//! ```
//!
//! Every message fits in a single TCP segment in practice (genomes are 12-72
//! hex characters), so head-of-line blocking is bounded by the read timeout.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

#[derive(Default)]
pub struct NetStats {
    pub messages_sent: AtomicU64,
    pub bytes_sent: AtomicU64,
    pub send_micros: AtomicU64,
    pub failures: AtomicU64,
    pub messages_received: AtomicU64,
}

impl NetStats {
    pub fn snapshot(&self) -> (u64, u64, u64, u64, u64) {
        (
            self.messages_sent.load(Ordering::Relaxed),
            self.bytes_sent.load(Ordering::Relaxed),
            self.send_micros.load(Ordering::Relaxed),
            self.failures.load(Ordering::Relaxed),
            self.messages_received.load(Ordering::Relaxed),
        )
    }
}

/// Spawn a background accept loop.  Every received line is forwarded to `tx`.
pub fn spawn_listener(bind: &str, tx: Sender<String>) -> std::io::Result<thread::JoinHandle<()>> {
    let listener = TcpListener::bind(bind)?;
    listener.set_nonblocking(false)?;
    let handle = thread::spawn(move || {
        for stream in listener.incoming() {
            match stream {
                Ok(stream) => {
                    let tx = tx.clone();
                    thread::spawn(move || {
                        let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
                        let mut reader = BufReader::new(stream);
                        let mut line = String::new();
                        loop {
                            line.clear();
                            match reader.read_line(&mut line) {
                                Ok(0) => break,
                                Ok(_) => {
                                    let msg = line.trim_end().to_string();
                                    if !msg.is_empty() && tx.send(msg).is_err() {
                                        return;
                                    }
                                }
                                Err(_) => break,
                            }
                        }
                    });
                }
                Err(_) => break,
            }
        }
    });
    Ok(handle)
}

/// Send one line, opening a fresh connection.  Returns the number of bytes
/// written (including the newline).
pub fn send_line(addr: &str, line: &str, timeout_ms: u64) -> std::io::Result<usize> {
    let timeout = Duration::from_millis(timeout_ms);
    let sock = addr
        .to_socket_addrs()?
        .next()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "bad address"))?;
    let mut stream = TcpStream::connect_timeout(&sock, timeout)?;
    let _ = stream.set_write_timeout(Some(timeout));
    let payload = format!("{line}\n");
    stream.write_all(payload.as_bytes())?;
    stream.flush()?;
    Ok(payload.len())
}

/// Best-effort send that records metrics instead of propagating failures.
pub fn send_tracked(addr: &str, line: &str, stats: &Arc<NetStats>) {
    let t0 = std::time::Instant::now();
    match send_line(addr, line, 200) {
        Ok(n) => {
            stats.messages_sent.fetch_add(1, Ordering::Relaxed);
            stats.bytes_sent.fetch_add(n as u64, Ordering::Relaxed);
            stats
                .send_micros
                .fetch_add(t0.elapsed().as_micros() as u64, Ordering::Relaxed);
        }
        Err(_) => {
            stats.failures.fetch_add(1, Ordering::Relaxed);
        }
    }
}

pub fn parse_migration(line: &str) -> Option<(u32, u32, Vec<String>)> {
    let mut it = line.split_whitespace();
    if it.next()? != "MIG" {
        return None;
    }
    let sender = it.next()?.parse().ok()?;
    let gen = it.next()?.parse().ok()?;
    let genomes = it
        .next()?
        .split(',')
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect();
    Some((sender, gen, genomes))
}

#[derive(Clone, Copy, Debug)]
pub struct Telemetry {
    pub island: u32,
    pub gen: u32,
    pub best: f64,
    pub mean: f64,
    pub diversity: f64,
    pub evaluations: u64,
    pub elapsed_ms: u64,
}

pub fn parse_telemetry(line: &str) -> Option<Telemetry> {
    let mut it = line.split_whitespace();
    if it.next()? != "TEL" {
        return None;
    }
    Some(Telemetry {
        island: it.next()?.parse().ok()?,
        gen: it.next()?.parse().ok()?,
        best: it.next()?.parse().ok()?,
        mean: it.next()?.parse().ok()?,
        diversity: it.next()?.parse().ok()?,
        evaluations: it.next()?.parse().ok()?,
        elapsed_ms: it.next()?.parse().ok()?,
    })
}

#[derive(Clone, Copy, Debug)]
pub struct Final {
    pub island: u32,
    pub best: f64,
    pub gen: u32,
    pub elapsed_ms: u64,
    pub messages: u64,
    pub bytes: u64,
}

pub fn parse_end(line: &str) -> Option<Final> {
    let mut it = line.split_whitespace();
    if it.next()? != "END" {
        return None;
    }
    Some(Final {
        island: it.next()?.parse().ok()?,
        best: it.next()?.parse().ok()?,
        gen: it.next()?.parse().ok()?,
        elapsed_ms: it.next()?.parse().ok()?,
        messages: it.next()?.parse().ok()?,
        bytes: it.next()?.parse().ok()?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_over_loopback() {
        let (tx, rx) = std::sync::mpsc::channel();
        // Port 0 lets the OS choose a free port, but spawn_listener does not
        // expose it; use a high port that is very likely free in CI.
        let port = 49_991;
        let _h = spawn_listener(&format!("127.0.0.1:{port}"), tx).unwrap();
        thread::sleep(Duration::from_millis(80));
        let n = send_line(&format!("127.0.0.1:{port}"), "MIG 1 7 deadbeef", 500).unwrap();
        assert!(n > 5);
        let line = rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let (sender, gen, genomes) = parse_migration(&line).unwrap();
        assert_eq!((sender, gen), (1, 7));
        assert_eq!(genomes, vec!["deadbeef"]);
    }

    #[test]
    fn parsing() {
        let t = parse_telemetry("TEL 2 10 0.51 0.4 0.8 640 1234").unwrap();
        assert_eq!(t.island, 2);
        assert_eq!(t.evaluations, 640);
        let e = parse_end("END 2 0.7 99 5000 8 400").unwrap();
        assert_eq!(e.bytes, 400);
        assert!(parse_migration("NOPE").is_none());
    }
}
