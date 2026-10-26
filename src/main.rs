//! `langton-ga` command line interface.
//!
//! Subcommands:
//!   sim       simulate one rule, optionally render/analyze
//!   analyze   measured metrics of one rule (CSV friendly)
//!   evolve    single-population genetic algorithm
//!   island    one island of the distributed island model (TCP)
//!   monitor   cluster telemetry collector
//!   demo      launch a complete local cluster
//!   bench     simulator throughput benchmark
//!   renders   helper to render a genome

mod fitness;
mod ga;
mod genome;
mod grid;
mod monitor;
mod net;
mod png;
mod render;
mod rng;
mod sim;

use fitness::{evaluate, Weights};
use ga::{GaConfig, Island};
use genome::Genome;
use render::RenderOptions;
use sim::{analyze, simulate, simulate_with, SimParams};
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::Ordering;
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        print_help();
        return;
    }
    let cmd = args[0].clone();
    let opts = match Opts::parse(&args[1..]) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(2);
        }
    };
    let result = match cmd.as_str() {
        "sim" => cmd_sim(&opts),
        "analyze" => cmd_analyze(&opts),
        "evolve" => cmd_evolve(&opts),
        "island" => cmd_island(&opts),
        "monitor" => monitor::run(&opts),
        "demo" => cmd_demo(&opts),
        "bench" => cmd_bench(&opts),
        "render" => cmd_render(&opts),
        "help" | "--help" | "-h" => {
            print_help();
            Ok(())
        }
        other => Err(format!(
            "unknown subcommand {other:?}; try `langton-ga help`"
        )),
    };
    if let Err(e) = result {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

fn print_help() {
    print!(
        "\
langton-ga -- distributed genetic algorithm for generalized Langton ants

USAGE:
  langton-ga <command> [--key value ...]

SIMULATION
  sim       --rule classic|<hex> --steps 20000 [--states 2] [--colors 2]
            [--render out.png] [--scale 2] [--max-cells 600] [--ascii]
            [--trace out.csv --trace-every 100] [--analyze] [--table]
  analyze   --rule classic|<hex> --steps 20000 [--out metrics.csv]
  render    --rule classic|<hex> --steps 20000 --out pattern.png [--scale 2]
  bench     [--iters 50] [--steps 20000]

EVOLUTION
  evolve    --seed 1 --gens 100 --pop 64 --steps 20000 [--states 2] [--colors 2]
            [--threads 4] [--mutation 0.03] [--elite 2] [--tournament 3]
            [--weights 0.50,0.25,0.15,0.10] [--out history.csv] [--best-out best.txt]
            [--render best.png]

DISTRIBUTED ISLAND MODEL
  island    --id 1 --bind 127.0.0.1:7001 --peer 127.0.0.1:7002
            --monitor 127.0.0.1:7100 --seed 1 --gens 100 --pop 64
            --steps 20000 [--migration-interval 10] [--migrants 2]
            [--threads 1] --out island1.csv --best-out best1.txt [--render p.png]
  monitor   --bind 127.0.0.1:7100 --islands 4 --out cluster.csv
            [--finals cluster_final.csv] [--timeout-secs 3600]
  demo      [--islands 4] [--gens 80] [--pop 64] [--steps 12000] [--base-port 7300]
            [--seed 1] [--profiles mover,builder,weaver,balanced]

Common options: --states 2 --colors 2 --mutation 0.03 --weights ...
"
    );
}

// ---------------------------------------------------------------------------
// Option parsing
// ---------------------------------------------------------------------------

pub struct Opts {
    map: HashMap<String, String>,
}

impl Opts {
    fn parse(args: &[String]) -> Result<Self, String> {
        let mut map = HashMap::new();
        let mut i = 0;
        while i < args.len() {
            let arg = &args[i];
            if !arg.starts_with("--") {
                return Err(format!("unexpected argument {arg:?}"));
            }
            let key = arg.trim_start_matches("--").to_string();
            if let Some(eq) = key.find('=') {
                map.insert(key[..eq].to_string(), key[eq + 1..].to_string());
                i += 1;
            } else if i + 1 < args.len() && !args[i + 1].starts_with("--") {
                map.insert(key, args[i + 1].clone());
                i += 2;
            } else {
                map.insert(key, "true".to_string());
                i += 1;
            }
        }
        Ok(Self { map })
    }

    fn get(&self, key: &str) -> Option<&str> {
        self.map.get(key).map(|s| s.as_str())
    }

    fn s(&self, key: &str, default: &str) -> String {
        self.get(key).unwrap_or(default).to_string()
    }

    fn u8(&self, key: &str, default: u8) -> Result<u8, String> {
        match self.get(key) {
            Some(v) => v.parse().map_err(|e| format!("--{key}: {e}")),
            None => Ok(default),
        }
    }

    fn usize(&self, key: &str, default: usize) -> Result<usize, String> {
        match self.get(key) {
            Some(v) => v.parse().map_err(|e| format!("--{key}: {e}")),
            None => Ok(default),
        }
    }

    fn u32(&self, key: &str, default: u32) -> Result<u32, String> {
        match self.get(key) {
            Some(v) => v.parse().map_err(|e| format!("--{key}: {e}")),
            None => Ok(default),
        }
    }

    fn u64(&self, key: &str, default: u64) -> Result<u64, String> {
        match self.get(key) {
            Some(v) => v.parse().map_err(|e| format!("--{key}: {e}")),
            None => Ok(default),
        }
    }

    fn f64(&self, key: &str, default: f64) -> Result<f64, String> {
        match self.get(key) {
            Some(v) => v.parse().map_err(|e| format!("--{key}: {e}")),
            None => Ok(default),
        }
    }

    fn bool(&self, key: &str, default: bool) -> bool {
        match self.get(key) {
            Some(v) => matches!(v, "true" | "1" | "yes"),
            None => default,
        }
    }

    fn weights(&self) -> Result<Weights, String> {
        match self.get("weights") {
            Some(v) => Weights::parse(v),
            None => Ok(Weights::default()),
        }
    }

    fn require(&self, key: &str) -> Result<String, String> {
        self.get(key)
            .map(|s| s.to_string())
            .ok_or_else(|| format!("missing required --{key}"))
    }
}

fn parse_genome(opts: &Opts) -> Result<Genome, String> {
    let states = opts.u8("states", 2)?;
    let colors = opts.u8("colors", 2)?;
    match opts.get("rule").unwrap_or("classic") {
        "classic" | "langton" => {
            if states == 2 && colors == 2 {
                Ok(Genome::langton())
            } else {
                Err("--rule classic requires --states 2 --colors 2".into())
            }
        }
        "random" => Ok(Genome::random(
            states,
            colors,
            &mut rng::Rng::new(opts.u64("seed", 1)?),
        )),
        hex => Genome::from_hex(states, colors, hex),
    }
}

fn sim_params(opts: &Opts, steps_default: u32) -> Result<SimParams, String> {
    Ok(SimParams {
        steps: opts.u32("steps", steps_default)?,
        start_x: 0,
        start_y: 0,
        dir: opts.u8("dir", 0)?,
        state: opts.u8("start-state", 0)?,
    })
}

// ---------------------------------------------------------------------------
// Small CSV / file helpers
// ---------------------------------------------------------------------------

struct Csv {
    out: BufWriter<File>,
    path: PathBuf,
}

impl Csv {
    fn create(path: &Path, header: &str) -> Result<Self, String> {
        if let Some(dir) = path.parent() {
            if !dir.as_os_str().is_empty() {
                fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
            }
        }
        let file = File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut out = BufWriter::new(file);
        writeln!(out, "{header}").map_err(|e| e.to_string())?;
        Ok(Self {
            out,
            path: path.to_path_buf(),
        })
    }

    fn row(&mut self, fields: &[String]) -> Result<(), String> {
        writeln!(self.out, "{}", fields.join(",")).map_err(|e| e.to_string())
    }

    fn flush(&mut self) {
        let _ = self.out.flush();
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for Csv {
    fn drop(&mut self) {
        let _ = self.out.flush();
    }
}

fn write_text(path: &Path, text: &str) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        if !dir.as_os_str().is_empty() {
            fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
    }
    fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))
}

fn best_report(genome: &Genome, fit: &fitness::Fitness, generation: u32, extra: &str) -> String {
    format!(
        "hex={}\nstates={}\ncolors={}\ngeneration={}\nfitness={:.6}\nmobility={:.6}\ncompactness={:.6}\ncomplexity={:.6}\ncolorfulness={:.6}\ndisplacement={:.3}\nnew_cells={}\nrevisits={}\nbbox_area={}\ncolors_used={}\nneighborhoods={}\n{}\n",
        genome.to_hex(),
        genome.states,
        genome.colors,
        generation,
        fit.total,
        fit.mobility,
        fit.compactness,
        fit.complexity,
        fit.colorfulness,
        fit.displacement,
        fit.new_cells,
        fit.revisits,
        fit.bbox_area,
        fit.colors_used,
        fit.neighborhoods,
        extra
    )
}

// ---------------------------------------------------------------------------
// sim / analyze / render / bench
// ---------------------------------------------------------------------------

fn cmd_sim(opts: &Opts) -> Result<(), String> {
    let genome = parse_genome(opts)?;
    let params = sim_params(opts, 11_000)?;
    if opts.bool("table", false) {
        print!("{}", genome.table());
    }
    if opts.bool("analyze", false) {
        let analysis = analyze(&genome, &params);
        print_analysis(&analysis, &genome, opts)?;
        return Ok(());
    }

    let trace_path = opts.get("trace").map(PathBuf::from);
    let every = opts.u32("trace-every", 100)?.max(1);
    let mut trace: Option<Csv> = None;
    let outcome = if let Some(path) = &trace_path {
        let mut csv = Csv::create(path, "step,x,y,displacement")?;
        let start = Instant::now();
        let out = simulate_with(&genome, &params, |step, x, y| {
            if step % every == 0 || step == params.steps {
                let _ = csv.row(&[
                    step.to_string(),
                    x.to_string(),
                    y.to_string(),
                    format!("{:.3}", ((x * x + y * y) as f64).sqrt()),
                ]);
            }
        });
        let elapsed = start.elapsed();
        csv.flush();
        trace = Some(csv);
        eprintln!(
            "traced {} steps in {:.1} ms",
            params.steps,
            elapsed.as_secs_f64() * 1e3
        );
        out
    } else {
        simulate(&genome, &params)
    };

    let fit = fitness::Fitness::score(&outcome, genome.colors, &opts.weights()?);
    println!("rule      {}", genome.to_hex());
    println!("steps     {}", outcome.steps);
    println!("end       ({}, {})", outcome.pos.0, outcome.pos.1);
    println!(
        "heading   {} (dir {})",
        sim::DIR_NAMES[outcome.dir as usize],
        outcome.dir
    );
    println!("state     {}", outcome.state);
    println!("speed     {:.6} cells/step", fit.mobility);
    println!("new cells {}", outcome.new_cells());
    println!("revisits  {}", outcome.revisits());
    if let Some(b) = outcome.bbox {
        println!(
            "bbox      {}x{} ({} cells, density {:.4})",
            b.width(),
            b.height(),
            b.area(),
            fit.compactness
        );
    }
    println!("colors    {} of {}", outcome.colors_used(), genome.colors);
    println!("fitness   {:.6}", fit.total);
    if let Some(csv) = trace {
        println!("trace     {}", csv.path().display());
    }

    if opts.bool("ascii", false) {
        let max = opts.u32("max-cells", 100)? as i32;
        println!("{}", render::ascii(&outcome.grid, max));
    }

    if let Some(out) = opts.get("render") {
        let opts_render = RenderOptions {
            max_cells: opts.usize("max-cells", 600)?,
            scale: opts.usize("scale", 2)?,
            focus: if opts.s("focus", "center") == "final" {
                Some(outcome.pos)
            } else {
                None
            },
            pad: opts.u32("pad", 0)? as i32,
        };
        let rendered = render::write_png(&outcome, Path::new(out), &opts_render)
            .map_err(|e| format!("{out}: {e}"))?;
        println!(
            "render    {} ({}x{} px, {} cells)",
            rendered.path, rendered.width, rendered.height, rendered.cells
        );
    }
    Ok(())
}

fn print_analysis(analysis: &sim::Analysis, genome: &Genome, opts: &Opts) -> Result<(), String> {
    let fit = fitness::Fitness::score(&analysis.outcome, genome.colors, &opts.weights()?);
    let executed = analysis.outcome.steps;
    println!("rule      {}", genome.to_hex());
    println!("states    {}  colors {}", genome.states, genome.colors);
    println!(
        "steps     {} (requested {})",
        executed,
        opts.u32("steps", 20_000)?
    );
    match analysis.cycle {
        Some(c) => {
            println!(
                "cycle     period {} steps, drift ({}, {}), found at step {}",
                c.period, c.dx, c.dy, c.at_step
            );
            println!(
                "highway   speed {:.6} cells/step ({:.3} deg)",
                ((c.dx * c.dx + c.dy * c.dy) as f64).sqrt() / c.period as f64,
                (c.dy as f64).atan2(c.dx as f64).to_degrees()
            );
        }
        None => println!("cycle     none within {} steps", executed),
    }
    if let Some(ch) = analysis.check {
        println!(
            "pattern   translation match {:.5} ({} / {} interior cells)",
            ch.fraction(),
            ch.matched,
            ch.total
        );
    }
    println!("speed     {:.6}", fit.mobility);
    println!("density   {:.6}", fit.compactness);
    println!(
        "complex.  {:.6} ({} neighborhoods)",
        fit.complexity, fit.neighborhoods
    );
    println!("colors    {} of {}", fit.colors_used, genome.colors);
    println!("fitness   {:.6}", fit.total);
    Ok(())
}

fn cmd_analyze(opts: &Opts) -> Result<(), String> {
    let genome = parse_genome(opts)?;
    let params = sim_params(opts, 20_000)?;
    let analysis = analyze(&genome, &params);
    let fit = fitness::Fitness::score(&analysis.outcome, genome.colors, &opts.weights()?);
    if let Some(out) = opts.get("out") {
        let header = "hex,states,colors,requested_steps,executed_steps,cycle_period,drift_x,drift_y,cycle_step,pattern_match,mobility,compactness,complexity,colorfulness,neighborhoods,fitness,displacement,new_cells,revisits,bbox_width,bbox_height,colors_used";
        let mut csv = Csv::create(Path::new(out), header)?;
        let (period, dx, dy, at) = match analysis.cycle {
            Some(c) => (
                c.period.to_string(),
                c.dx.to_string(),
                c.dy.to_string(),
                c.at_step.to_string(),
            ),
            None => ("0".into(), "0".into(), "0".into(), "0".into()),
        };
        let (bw, bh) = analysis
            .outcome
            .bbox
            .map(|b| (b.width(), b.height()))
            .unwrap_or((0, 0));
        csv.row(&[
            genome.to_hex(),
            genome.states.to_string(),
            genome.colors.to_string(),
            params.steps.to_string(),
            analysis.outcome.steps.to_string(),
            period,
            dx,
            dy,
            at,
            format!("{:.6}", analysis.check.map(|c| c.fraction()).unwrap_or(0.0)),
            format!("{:.6}", fit.mobility),
            format!("{:.6}", fit.compactness),
            format!("{:.6}", fit.complexity),
            format!("{:.6}", fit.colorfulness),
            fit.neighborhoods.to_string(),
            format!("{:.6}", fit.total),
            format!("{:.3}", fit.displacement),
            fit.new_cells.to_string(),
            fit.revisits.to_string(),
            bw.to_string(),
            bh.to_string(),
            fit.colors_used.to_string(),
        ])?;
        csv.flush();
        println!("{}", csv.path().display());
    } else {
        print_analysis(&analysis, &genome, opts)?;
    }
    Ok(())
}

fn cmd_render(opts: &Opts) -> Result<(), String> {
    let genome = parse_genome(opts)?;
    let params = sim_params(opts, 20_000)?;
    let out = opts.require("out")?;
    let outcome = simulate(&genome, &params);
    let render_opts = RenderOptions {
        max_cells: opts.usize("max-cells", 700)?,
        scale: opts.usize("scale", 2)?,
        focus: if opts.s("focus", "center") == "final" {
            Some(outcome.pos)
        } else {
            None
        },
        pad: opts.u32("pad", 0)? as i32,
    };
    let rendered = render::write_png(&outcome, Path::new(&out), &render_opts)
        .map_err(|e| format!("{out}: {e}"))?;
    println!(
        "wrote {} ({}x{} px, {} visited cells, {} simulation steps)",
        rendered.path, rendered.width, rendered.height, rendered.cells, outcome.steps
    );
    Ok(())
}

fn cmd_bench(opts: &Opts) -> Result<(), String> {
    let iters = opts.usize("iters", 50)?;
    let steps = opts.u32("steps", 20_000)?;
    let genome = Genome::langton();
    let params = SimParams {
        steps,
        ..Default::default()
    };
    // Warm-up
    let _ = evaluate(&genome, &params, &Weights::default());
    let start = Instant::now();
    let mut cells = 0u64;
    for _ in 0..iters {
        cells += evaluate(&genome, &params, &Weights::default()).new_cells;
    }
    let elapsed = start.elapsed().as_secs_f64();
    let total_steps = iters as f64 * steps as f64;
    println!("iterations        {iters}");
    println!("steps per run     {steps}");
    println!("elapsed           {:.3} s", elapsed);
    println!("evaluations/s     {:.2}", iters as f64 / elapsed);
    println!("steps/s           {:.0}", total_steps / elapsed);
    println!("cells/s           {:.0}", cells as f64 / elapsed);
    Ok(())
}

// ---------------------------------------------------------------------------
// evolve
// ---------------------------------------------------------------------------

fn cmd_evolve(opts: &Opts) -> Result<(), String> {
    let cfg = ga_config(opts)?;
    let out = opts.s("out", "results/evolve.csv");
    let best_out = opts.s("best-out", "results/best.txt");
    let mut island = Island::new(cfg.clone());
    let mut csv = Csv::create(
        Path::new(&out),
        "generation,best,mean,median,worst,stddev,diversity,behavioral_diversity,evaluations,elapsed_ms,best_mobility,best_compactness,best_complexity,best_colorfulness",
    )?;
    let start = Instant::now();
    for _ in 0..cfg.gens {
        let s = island.step();
        csv.row(&[
            s.generation.to_string(),
            format!("{:.6}", s.best),
            format!("{:.6}", s.mean),
            format!("{:.6}", s.median),
            format!("{:.6}", s.worst),
            format!("{:.6}", s.stddev),
            format!("{:.6}", s.diversity),
            format!("{:.6}", s.behavioral),
            s.evaluations.to_string(),
            start.elapsed().as_millis().to_string(),
            format!("{:.6}", s.best_mobility),
            format!("{:.6}", s.best_compactness),
            format!("{:.6}", s.best_complexity),
            format!("{:.6}", s.best_colorfulness),
        ])?;
        if s.generation % 10 == 0 || s.generation + 1 == cfg.gens {
            println!(
                "gen {:3}  best {:.4}  mean {:.4}  div {:.2}  evals {:6}  {:.1}s",
                s.generation,
                s.best,
                s.mean,
                s.behavioral,
                island.evaluations,
                start.elapsed().as_secs_f64()
            );
            csv.flush();
        }
    }
    let elapsed = start.elapsed();
    let best = island.best().expect("evaluated population").clone();
    let fit = best.fitness;
    let extra = format!(
        "elapsed_ms={}\nevaluations={}\nseed={}\npop={}\ngens={}\nsteps={}",
        elapsed.as_millis(),
        island.evaluations,
        cfg.seed,
        cfg.pop,
        cfg.gens,
        cfg.steps
    );
    write_text(
        Path::new(&best_out),
        &best_report(&best.genome, &fit, island.generation - 1, &extra),
    )?;
    println!(
        "\nbest fitness {:.6}  mobility {:.4}  compactness {:.4}  colors {}/{}",
        fit.total, fit.mobility, fit.compactness, fit.colors_used, cfg.colors
    );
    println!("genome {}", best.genome.to_hex());
    println!("history {}", out);
    println!("best    {best_out}");

    if let Some(png) = opts.get("render") {
        let params = SimParams {
            steps: opts.u32("render-steps", 60_000)?,
            ..Default::default()
        };
        let outcome = simulate(&best.genome, &params);
        let rendered = render::write_png(
            &outcome,
            Path::new(png),
            &RenderOptions {
                max_cells: opts.usize("max-cells", 700)?,
                scale: opts.usize("scale", 2)?,
                focus: None,
                pad: opts.u32("pad", 0)? as i32,
            },
        )
        .map_err(|e| format!("{png}: {e}"))?;
        println!(
            "render  {} ({}x{} px, {} cells)",
            rendered.path, rendered.width, rendered.height, rendered.cells
        );
    }
    Ok(())
}

fn ga_config(opts: &Opts) -> Result<GaConfig, String> {
    let states = opts.u8("states", 2)?;
    let colors = opts.u8("colors", 2)?;
    if states == 0 || states > genome::MAX_STATES as u8 {
        return Err(format!("--states must be in 1..={}", genome::MAX_STATES));
    }
    if colors == 0 || colors > genome::MAX_COLORS as u8 {
        return Err(format!("--colors must be in 1..={}", genome::MAX_COLORS));
    }
    Ok(GaConfig {
        pop: opts.usize("pop", 64)?,
        gens: opts.u32("gens", 100)?,
        steps: opts.u32("steps", 20_000)?,
        states,
        colors,
        tournament: opts.usize("tournament", 3)?,
        elite: opts.usize("elite", 2)?,
        mutation_rate: opts.f64("mutation", 0.03)?,
        reset_rate: opts.f64("reset", 0.01)?,
        dedupe: !opts.bool("no-dedupe", false),
        seed: opts.u64("seed", 1)?,
        threads: opts.usize("threads", 1)?,
        weights: opts.weights()?,
        classic_seed: !opts.bool("no-classic-seed", false),
    })
}

// ---------------------------------------------------------------------------
// island (distributed)
// ---------------------------------------------------------------------------

fn cmd_island(opts: &Opts) -> Result<(), String> {
    let id = opts.u32("id", 1)?;
    let bind = opts.require("bind")?;
    let peer = opts.require("peer")?;
    let monitor_addr = opts.require("monitor")?;
    let migration_interval = opts.u32("migration-interval", 10)?;
    let migrants = opts.usize("migrants", 2)?.max(1);
    let out = opts.s("out", &format!("results/island{id}.csv"));
    let best_out = opts.s("best-out", &format!("results/island{id}_best.txt"));
    let cfg = ga_config(opts)?;
    let weights = cfg.weights;
    let steps = cfg.steps;

    let (tx, rx) = mpsc::channel();
    net::spawn_listener(&bind, tx).map_err(|e| format!("bind {bind}: {e}"))?;
    let net_stats = Arc::new(net::NetStats::default());
    let mut island = Island::new(cfg.clone());
    let mut csv = Csv::create(
        Path::new(&out),
        "generation,best,mean,median,worst,stddev,diversity,behavioral_diversity,evaluations,elapsed_ms,best_mobility,best_compactness,best_complexity,best_colorfulness,immigrants_received,immigrants_accepted",
    )?;
    let start = Instant::now();
    let mut total_received = 0u64;
    let mut total_accepted = 0u64;

    println!(
        "island {id} up  bind={bind} peer={peer} monitor={monitor_addr} pop={} states={} colors={}",
        cfg.pop, cfg.states, cfg.colors
    );

    for _ in 0..cfg.gens {
        let s = island.step();

        // Receive any migration messages that arrived since the last generation.
        let mut immigrants: Vec<Genome> = Vec::new();
        while let Ok(line) = rx.try_recv() {
            net_stats.messages_received.fetch_add(1, Ordering::Relaxed);
            if let Some((_sender, _gen, hexes)) = net::parse_migration(&line) {
                for hex in hexes {
                    match Genome::from_hex(cfg.states, cfg.colors, &hex) {
                        Ok(g) => immigrants.push(g),
                        Err(e) => eprintln!("island {id}: bad immigrant: {e}"),
                    }
                }
            }
        }
        let accepted = if immigrants.is_empty() {
            0
        } else {
            island.inject(&immigrants)
        };
        total_received += immigrants.len() as u64;
        total_accepted += accepted as u64;

        // Send migrants to the next island in the ring.
        if migration_interval > 0 && s.generation > 0 && s.generation % migration_interval == 0 {
            let mut migrants_hex: Vec<String> = Vec::new();
            let mut order: Vec<usize> = (0..island.population.len()).collect();
            order.sort_unstable_by(|&a, &b| {
                island.population[b]
                    .fitness
                    .total
                    .partial_cmp(&island.population[a].fitness.total)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            for &i in order.iter().take(migrants) {
                migrants_hex.push(island.population[i].genome.to_hex());
            }
            let line = format!("MIG {} {} {}", id, s.generation, migrants_hex.join(","));
            net::send_tracked(&peer, &line, &net_stats);
        }

        csv.row(&[
            s.generation.to_string(),
            format!("{:.6}", s.best),
            format!("{:.6}", s.mean),
            format!("{:.6}", s.median),
            format!("{:.6}", s.worst),
            format!("{:.6}", s.stddev),
            format!("{:.6}", s.diversity),
            format!("{:.6}", s.behavioral),
            s.evaluations.to_string(),
            start.elapsed().as_millis().to_string(),
            format!("{:.6}", s.best_mobility),
            format!("{:.6}", s.best_compactness),
            format!("{:.6}", s.best_complexity),
            format!("{:.6}", s.best_colorfulness),
            immigrants.len().to_string(),
            accepted.to_string(),
        ])?;

        let tel = format!(
            "TEL {} {} {:.6} {:.6} {:.6} {} {}",
            id,
            s.generation,
            s.best,
            s.mean,
            s.behavioral,
            island.evaluations,
            start.elapsed().as_millis()
        );
        net::send_tracked(&monitor_addr, &tel, &net_stats);
        if s.generation % 10 == 0 || s.generation + 1 == cfg.gens {
            println!(
                "island {id} gen {:3}  best {:.4}  mean {:.4}  div {:.2}  recv {:4}",
                s.generation, s.best, s.mean, s.behavioral, total_received
            );
            csv.flush();
        }
    }

    let elapsed = start.elapsed();
    let best = island.best().expect("evaluated population").clone();
    let fit = best.fitness;
    let extra = format!(
        "elapsed_ms={}\nevaluations={}\nseed={}\npop={}\ngens={}\nsteps={}\nreceived={}\naccepted={}\nid={}\nweights={:?}",
        elapsed.as_millis(),
        island.evaluations,
        cfg.seed,
        cfg.pop,
        cfg.gens,
        steps,
        total_received,
        total_accepted,
        id,
        weights
    );
    write_text(
        Path::new(&best_out),
        &best_report(&best.genome, &fit, island.generation - 1, &extra),
    )?;

    if let Some(png) = opts.get("render") {
        let params = SimParams {
            steps: opts.u32("render-steps", 60_000)?,
            ..Default::default()
        };
        let outcome = simulate(&best.genome, &params);
        let rendered = render::write_png(
            &outcome,
            Path::new(png),
            &RenderOptions {
                max_cells: opts.usize("max-cells", 700)?,
                scale: opts.usize("scale", 2)?,
                focus: None,
                pad: opts.u32("pad", 0)? as i32,
            },
        )
        .map_err(|e| format!("{png}: {e}"))?;
        println!("island {id} render {}", rendered.path);
    }

    let (msgs, bytes, micros, failures, received) = net_stats.snapshot();
    let end = format!(
        "END {} {:.6} {} {} {} {}",
        id,
        fit.total,
        island.generation - 1,
        elapsed.as_millis(),
        msgs,
        bytes
    );
    net::send_tracked(&monitor_addr, &end, &net_stats);
    println!(
        "island {id} done  best {:.6}  evals {}  {:.1}s  net: {} msgs / {} B / {:.2} ms avg / {} fail / {} recv",
        fit.total,
        island.evaluations,
        elapsed.as_secs_f64(),
        msgs,
        bytes,
        if msgs > 0 {
            micros as f64 / msgs as f64 / 1000.0
        } else {
            0.0
        },
        failures,
        received
    );
    println!("island {id} genome {}", best.genome.to_hex());
    Ok(())
}

// ---------------------------------------------------------------------------
// demo
// ---------------------------------------------------------------------------

fn profile_weights(name: &str) -> &'static str {
    match name {
        "mover" => "0.80,0.05,0.05,0.10",
        "builder" => "0.05,0.75,0.10,0.10",
        "weaver" => "0.20,0.20,0.50,0.10",
        "balanced" => "0.50,0.25,0.15,0.10",
        _ => "0.50,0.25,0.15,0.10",
    }
}

fn cmd_demo(opts: &Opts) -> Result<(), String> {
    let n = opts.usize("islands", 4)?.max(1);
    let base_port = opts.usize("base-port", 7300)?;
    let gens = opts.u32("gens", 80)?;
    let pop = opts.usize("pop", 64)?;
    let steps = opts.u32("steps", 12_000)?;
    let dir = opts.s("out-dir", "results/demo");
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    fs::create_dir_all(&dir).map_err(|e| format!("{dir}: {e}"))?;
    let monitor_addr = format!("127.0.0.1:{}", base_port + 90);
    let timeout = opts.u32("timeout-secs", 900)?;

    println!("launching monitor on {monitor_addr}");
    let mut monitor_child = Command::new(&exe)
        .args([
            "monitor",
            "--bind",
            &monitor_addr,
            "--islands",
            &n.to_string(),
            "--out",
            &format!("{dir}/telemetry.csv"),
            "--finals",
            &format!("{dir}/finals.csv"),
            "--timeout-secs",
            &timeout.to_string(),
        ])
        .spawn()
        .map_err(|e| format!("spawn monitor: {e}"))?;
    std::thread::sleep(Duration::from_millis(400));

    let default_weights = opts.s("weights", "0.50,0.25,0.15,0.10");
    let profiles: Vec<String> = opts
        .s("profiles", "")
        .split(',')
        .filter(|p| !p.trim().is_empty())
        .map(|p| p.trim().to_string())
        .collect();
    let states = opts.u32("states", 2)?;
    let colors = opts.u32("colors", 2)?;
    let threads = opts.usize("threads", 1)?;
    let migration_interval = opts.u32("migration-interval", 5)?;
    let migrants = opts.usize("migrants", 2)?;
    let render_scale = opts.usize("render-scale", 2)?;
    let max_cells = opts.usize("max-cells", 700)?;
    let render_steps = opts.u32("render-steps", steps.saturating_mul(3))?;
    // Each island gets a reproducible seed derived from the run seed, so that
    // repeating a run with the same --seed replays the same initial
    // populations, while different islands still explore independently.
    let base_seed = opts.u64("seed", 1)?;
    let mut children = Vec::new();
    for i in 0..n {
        let port = base_port + i;
        let peer = base_port + (i + 1) % n;
        let weights = if profiles.is_empty() {
            default_weights.clone()
        } else {
            profile_weights(&profiles[i % profiles.len()]).to_string()
        };
        let child = Command::new(&exe)
            .args([
                "island",
                "--id",
                &(i as u32 + 1).to_string(),
                "--bind",
                &format!("127.0.0.1:{port}"),
                "--peer",
                &format!("127.0.0.1:{peer}"),
                "--monitor",
                &monitor_addr,
                "--seed",
                &(base_seed.saturating_mul(100).saturating_add(i as u64 + 1)).to_string(),
                "--gens",
                &gens.to_string(),
                "--pop",
                &pop.to_string(),
                "--steps",
                &steps.to_string(),
                "--states",
                &states.to_string(),
                "--colors",
                &colors.to_string(),
                "--threads",
                &threads.to_string(),
                "--weights",
                &weights,
                "--migration-interval",
                &migration_interval.to_string(),
                "--migrants",
                &migrants.to_string(),
                "--out",
                &format!("{dir}/island{}.csv", i + 1),
                "--best-out",
                &format!("{dir}/island{}_best.txt", i + 1),
                "--render",
                &format!("{dir}/island{}.png", i + 1),
                "--render-steps",
                &render_steps.to_string(),
                "--scale",
                &render_scale.to_string(),
                "--max-cells",
                &max_cells.to_string(),
            ])
            .spawn()
            .map_err(|e| format!("spawn island {i}: {e}"))?;
        children.push(child);
    }
    for child in &mut children {
        let _ = child.wait();
    }
    let _ = monitor_child.wait();
    println!("demo finished; results in {dir}/");
    Ok(())
}
