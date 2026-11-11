use std::path::{Path, PathBuf};
use std::time::Instant;

use serde::Serialize;

use crate::devtools::util::{self, Error, Result};

const CONTRACT: &str = "render_bytes(data, seed) -> (jpeg, seed)";

#[derive(Serialize)]
struct Stats {
    n: usize,
    p50: f64,
    p95: f64,
    max: f64,
}

impl std::fmt::Display for Stats {
    fn fmt(&self, output: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(output, "{:.1}/{:.1}/{:.1}", self.p50, self.p95, self.max)
    }
}

#[derive(Serialize)]
struct Row {
    input: String,
    input_sha256: String,
    input_bytes: usize,
    width: u32,
    height: u32,
    bytes: usize,
    api: Stats,
}

#[derive(Serialize)]
struct Environment {
    os: &'static str,
    arch: &'static str,
    cpu_count: Option<usize>,
    target: &'static str,
    version: &'static str,
    build_profile: &'static str,
}

#[derive(Serialize)]
struct Pooled {
    api: Stats,
}

#[derive(Serialize)]
struct Report {
    contract: &'static str,
    environment: Environment,
    seed: u64,
    repeat: usize,
    rows: Vec<Row>,
    pooled: Pooled,
}

impl Stats {
    fn new(values: &[f64]) -> Self {
        let mut ordered = values.to_vec();
        ordered.sort_by(|left, right| left.total_cmp(right));
        Self {
            n: ordered.len(),
            p50: round3(percentile(&ordered, 50.0)),
            p95: round3(percentile(&ordered, 95.0)),
            max: round3(ordered.last().copied().unwrap_or(0.0)),
        }
    }
}

pub struct Options {
    pub repeat: usize,
    pub seed: u64,
    pub json_path: Option<PathBuf>,
}

pub fn run(root: &Path, options: &Options) -> Result<()> {
    if options.repeat < 1 {
        return Err(Error::usage("--repeat must be >= 1"));
    }
    let fixtures = root.join("tests/fixtures/bot_realism");
    let manifest = util::read_manifest(&fixtures)?;
    let inputs = manifest.images;
    eprintln!(
        "benchmarking {} input(s), repeat={} ...",
        inputs.len(),
        options.repeat
    );

    let mut rows = Vec::with_capacity(inputs.len());
    let mut pooled = Vec::new();
    for entry in inputs {
        let blob = std::fs::read(fixtures.join(&entry.file))?;
        let (warmup, _) = crate::render_bytes(&blob, Some(options.seed))?;
        std::hint::black_box(warmup.len());
        let dimensions = image::ImageReader::with_format(
            std::io::Cursor::new(&warmup),
            image::ImageFormat::Jpeg,
        )
        .into_dimensions()?;

        let mut api = Vec::with_capacity(options.repeat);
        for _ in 0..options.repeat {
            let started = Instant::now();
            let (rendered, _) = crate::render_bytes(&blob, Some(options.seed))?;
            api.push(round3(started.elapsed().as_secs_f64() * 1000.0));
            std::hint::black_box(rendered.len());
        }
        pooled.extend_from_slice(&api);
        rows.push(Row {
            input: entry.file,
            input_sha256: util::sha256_hex(&blob),
            input_bytes: blob.len(),
            width: dimensions.0,
            height: dimensions.1,
            bytes: warmup.len(),
            api: Stats::new(&api),
        });
    }

    let payload = Report {
        contract: CONTRACT,
        environment: environment(),
        seed: options.seed,
        repeat: options.repeat,
        rows,
        pooled: Pooled {
            api: Stats::new(&pooled),
        },
    };
    print_report(&payload);

    if let Some(json_path) = &options.json_path {
        let destination = root.join(json_path);
        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(
            &destination,
            format!("{}\n", serde_json::to_string_pretty(&payload)?),
        )?;
        println!("\njson: {}", destination.display());
    }
    Ok(())
}

fn environment() -> Environment {
    Environment {
        os: std::env::consts::OS,
        arch: std::env::consts::ARCH,
        cpu_count: std::thread::available_parallelism()
            .map(|count| count.get())
            .ok(),
        target: super::TARGET,
        version: super::VERSION,
        build_profile: if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        },
    }
}

fn print_report(payload: &Report) {
    let environment = &payload.environment;
    println!("screenphoto native benchmark");
    println!("  contract   {}", payload.contract);
    println!(
        "  build      {} / screenphoto {}",
        environment.target, environment.version
    );
    let cpus = environment
        .cpu_count
        .map_or_else(|| "unknown".to_string(), |count| count.to_string());
    println!(
        "  platform   {} {}, {} CPUs ({})",
        environment.os, environment.arch, cpus, environment.build_profile
    );
    println!(
        "  corpus     {} inputs, repeat {} per input, seed {}",
        payload.rows.len(),
        payload.repeat,
        payload.seed
    );
    println!("\n{:<28} {:>20}", "input", "API p50/p95/max");
    for row in &payload.rows {
        println!("{:<28} {:>20}", row.input, row.api);
    }
    println!("{:<28} {:>20}", "POOLED", payload.pooled.api);
}

fn round3(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
}

fn percentile(ordered: &[f64], percentile: f64) -> f64 {
    if ordered.is_empty() {
        return 0.0;
    }
    if ordered.len() == 1 {
        return ordered[0];
    }
    let position = (ordered.len() - 1) as f64 * percentile / 100.0;
    let lower = position.floor() as usize;
    let upper = (lower + 1).min(ordered.len() - 1);
    ordered[lower] + (ordered[upper] - ordered[lower]) * (position - lower as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentile_interpolates_between_samples() {
        let values = [1.0, 2.0, 3.0, 4.0];
        assert_eq!(percentile(&values, 50.0), 2.5);
        assert!((percentile(&values, 95.0) - 3.85).abs() < 1e-12);
        assert_eq!(percentile(&[7.0], 95.0), 7.0);
    }
}
