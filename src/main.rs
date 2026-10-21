use anyhow::Result;
use clap::{Parser, ValueEnum};
use muxcraft::{config::Config, pipeline};
use std::path::PathBuf;

#[derive(Clone, Copy, ValueEnum)]
enum Mode {
    Run,
    Check,
    Plan,
}
#[derive(Parser)]
#[command(version, about = "Batch remuxing with auditable TOML configuration")]
struct Cli {
    /// TOML configuration file for the batch
    config: PathBuf,
    /// run: execute; check: validate media and fonts; plan: show paths and track order
    #[arg(long, value_enum, default_value = "run")]
    mode: Mode,
    /// Show the plan without requiring the media files to be present
    #[arg(long)]
    no_check: bool,
    /// Check or run only the specified episode IDs (comma separated)
    #[arg(long, value_delimiter = ',')]
    episodes: Vec<String>,
    /// Show detailed tool diagnostics in addition to the standard command log
    #[arg(long, global = true)]
    verbose: bool,
}
fn execute() -> Result<()> {
    let cli = Cli::parse();
    muxcraft::ui::initialize(cli.verbose);
    anyhow::ensure!(
        !cli.no_check || matches!(cli.mode, Mode::Plan),
        "--no-check can only be used with --mode plan"
    );
    let path = cli.config.canonicalize()?;
    let base = path.parent().unwrap();
    let mut cfg = Config::load(&path)?;
    for id in &cli.episodes {
        anyhow::ensure!(
            cfg.episodes.iter().any(|e| &e.id == id),
            "Unknown episode: {id}"
        );
    }
    if !cli.episodes.is_empty() {
        cfg.episodes.retain(|p| cli.episodes.contains(&p.id));
    }
    let plans = cfg.plan(base, !cli.no_check)?;
    match cli.mode {
        Mode::Plan => println!("{}", serde_json::to_string_pretty(&plans)?),
        Mode::Check => pipeline::check(&cfg, base, &plans)?,
        Mode::Run => pipeline::run(&cfg, base, &plans)?,
    }
    Ok(())
}
fn main() {
    if let Err(e) = execute() {
        muxcraft::ui::error(&format!("{e:#}"));
        std::process::exit(1);
    }
}
