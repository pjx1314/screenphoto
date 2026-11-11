//! Maintenance-only native tooling; never shipped with the runtime binary.

use std::path::PathBuf;

use clap::{Parser, Subcommand};

use screenphoto::devtools::{self, util};

#[derive(Parser)]
#[command(
    name = "screenphoto-dev",
    version,
    about = "Maintenance tools for the native screenphoto renderer"
)]
struct Cli {
    /// Project root (defaults to the current directory)
    #[arg(long, global = true, default_value = ".", value_name = "DIR")]
    root: PathBuf,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Regenerate sample images, comparison sheets and the corpus contact sheet
    Gallery {
        /// Output directory, relative to --root unless absolute
        #[arg(long, default_value = "samples")]
        out: PathBuf,
    },
    /// Time the full render_bytes API on the fixed screenshot corpus
    Benchmark {
        /// Timed repetitions per input
        #[arg(long, default_value_t = 10)]
        repeat: usize,
        /// Fixed shooting seed
        #[arg(long, default_value_t = devtools::SAMPLE_SEED)]
        seed: u64,
        /// Optional JSON report path
        #[arg(long = "json")]
        json: Option<PathBuf>,
    },
    /// Verify the fixed bot-realism corpus against its manifest
    #[command(name = "verify-corpus")]
    VerifyCorpus {
        /// Fixture directory containing manifest.json
        #[arg(long)]
        fixtures: Option<PathBuf>,
    },
    /// Build the native Skill distribution ZIPs and checksums
    Package {
        /// Built runtime binary to stage
        #[arg(long)]
        binary: PathBuf,
        /// Platform target triple used in the archive name
        #[arg(long)]
        target: Option<String>,
        /// Output directory, relative to --root unless absolute
        #[arg(long, default_value = "dist")]
        out: PathBuf,
    },
}

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        std::process::exit(if error.is_usage() { 2 } else { 1 });
    }
}

fn run() -> util::Result<()> {
    let cli = Cli::parse();
    let root = std::env::current_dir()?.join(&cli.root);
    let root = root
        .canonicalize()
        .map_err(|error| util::Error::usage(format!("--root {}: {error}", cli.root.display())))?;

    match cli.command {
        Command::Gallery { out } => {
            let out = root.join(&out);
            devtools::gallery::run(&root, &out)
        }
        Command::Benchmark { repeat, seed, json } => devtools::benchmark::run(
            &root,
            &devtools::benchmark::Options {
                repeat,
                seed,
                json_path: json,
            },
        ),
        Command::VerifyCorpus { fixtures } => {
            let fixtures = fixtures
                .map(|path| root.join(&path))
                .unwrap_or_else(|| root.join("tests/fixtures/bot_realism"));
            devtools::corpus::run(&fixtures)
        }
        Command::Package {
            binary,
            target,
            out,
        } => devtools::package::run(
            &root,
            &devtools::package::Options {
                binary,
                target: target.unwrap_or_else(|| devtools::TARGET.to_string()),
                out,
            },
        ),
    }
}
