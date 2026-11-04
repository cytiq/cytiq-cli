use std::path::PathBuf;

use clap::Args;
use cytiq_provenance::{RunRecorder, record_path_for_file};
use cytiq_stats::{DEFAULT_BINS, StatsConfig, StatsRunner};
use log::info;
use serde::Serialize;

use crate::provenance::record_provenance;

#[derive(Debug, Serialize, Args)]
pub(crate) struct StatsArgs {
    /// Input FCS file.
    #[arg(long)]
    input: PathBuf,
    /// Output JSON path.
    #[arg(long)]
    output: PathBuf,
    /// Histogram bins per channel (2 to 4096).
    #[arg(long, default_value_t = DEFAULT_BINS)]
    bins: usize,
    /// Replace an existing output file.
    #[arg(long)]
    overwrite: bool,
}

pub(crate) fn run(
    args: StatsArgs,
    mut recorder: RunRecorder,
) -> Result<(), Box<dyn std::error::Error>> {
    let StatsArgs {
        input,
        output,
        bins,
        overwrite,
    } = args;
    info!("starting stats: input={}", input.display());
    recorder.snapshot_roles(&[("input", input.clone())])?;
    let config = StatsConfig {
        input,
        output,
        bins,
        overwrite,
    };
    let report = StatsRunner::run(&config)?;
    record_provenance(
        recorder,
        std::slice::from_ref(&config.output),
        &record_path_for_file(&config.output),
    )?;
    info!(
        "stats: {} event(s), {} channel(s); wrote {}",
        report.event_count,
        report.channels.len(),
        config.output.display()
    );
    Ok(())
}
