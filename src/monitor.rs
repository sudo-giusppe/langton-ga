//! Cluster telemetry collector.
//!
//! Listens for `TEL` and `END` lines from the islands and writes two CSV files
//! suitable for plotting with pgfplots: a per-generation telemetry stream and
//! a final summary per island.

use crate::net::{self, parse_end, parse_telemetry};
use crate::{Csv, Opts};
use std::collections::HashSet;
use std::path::Path;
use std::sync::mpsc;
use std::time::{Duration, Instant};

pub fn run(opts: &Opts) -> Result<(), String> {
    let bind = opts.require("bind")?;
    let expected = opts.usize("islands", 4)?;
    let out = opts.s("out", "results/cluster.csv");
    let finals_path = opts.s("finals", "results/cluster_final.csv");
    let timeout_secs = opts.u32("timeout-secs", 3600)?;
    let quiet = opts.bool("quiet", false);

    let (tx, rx) = mpsc::channel();
    net::spawn_listener(&bind, tx).map_err(|e| format!("bind {bind}: {e}"))?;
    let mut csv = Csv::create(
        Path::new(&out),
        "arrival_ms,island,gen,best,mean,diversity,evaluations,elapsed_ms",
    )?;
    let mut finals = Csv::create(
        Path::new(&finals_path),
        "island,best,gen,elapsed_ms,messages,bytes",
    )?;
    let start = Instant::now();
    let timeout = Duration::from_secs(timeout_secs as u64);
    let mut ended: HashSet<u32> = HashSet::new();
    let mut best_of_best = f64::MIN;

    println!("monitor listening on {bind}, expecting {expected} islands");

    loop {
        if start.elapsed() > timeout {
            println!("monitor timeout after {timeout_secs}s");
            break;
        }
        match rx.recv_timeout(Duration::from_millis(200)) {
            Ok(line) => {
                if let Some(t) = parse_telemetry(&line) {
                    csv.row(&[
                        start.elapsed().as_millis().to_string(),
                        t.island.to_string(),
                        t.gen.to_string(),
                        format!("{:.6}", t.best),
                        format!("{:.6}", t.mean),
                        format!("{:.6}", t.diversity),
                        t.evaluations.to_string(),
                        t.elapsed_ms.to_string(),
                    ])?;
                    if !quiet && (t.gen % 10 == 0 || t.gen == 0) {
                        println!(
                            "[{:6.1}s] island {} gen {:3} best {:.4} mean {:.4} div {:.2}",
                            start.elapsed().as_secs_f64(),
                            t.island,
                            t.gen,
                            t.best,
                            t.mean,
                            t.diversity
                        );
                    }
                } else if let Some(f) = parse_end(&line) {
                    if ended.insert(f.island) {
                        finals.row(&[
                            f.island.to_string(),
                            format!("{:.6}", f.best),
                            f.gen.to_string(),
                            f.elapsed_ms.to_string(),
                            f.messages.to_string(),
                            f.bytes.to_string(),
                        ])?;
                        finals.flush();
                        best_of_best = best_of_best.max(f.best);
                        println!(
                            "island {} finished: best {:.6} in {:.1}s ({} msgs, {} B)",
                            f.island,
                            f.best,
                            f.elapsed_ms as f64 / 1000.0,
                            f.messages,
                            f.bytes
                        );
                        if ended.len() >= expected {
                            break;
                        }
                    }
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    csv.flush();
    finals.flush();
    if best_of_best > f64::MIN {
        println!("best across islands: {best_of_best:.6}");
    }
    println!("telemetry -> {}", out);
    println!("finals    -> {finals_path}");
    Ok(())
}
