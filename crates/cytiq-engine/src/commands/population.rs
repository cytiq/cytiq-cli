use super::specimen::finish;
use clap::{Args, Subcommand};
use cytiq_provenance::{RunRecorder, outcome::CommandOutcome};
use cytiq_specimen::runner::{self, ObserveRequest};
use serde::Serialize;
use std::path::PathBuf;

#[derive(Debug, Serialize, Subcommand)]
pub(crate) enum PopulationCommand {
    /// Validate annotation data, provenance, lineage and source definitions.
    Validate(ObserveInputs),
    /// Adapt native annotations into typed population observations.
    Observe(ObserveArgs),
}

#[derive(Debug, Serialize, Args)]
pub(crate) struct ObserveInputs {
    #[arg(long)]
    plan: PathBuf,
    #[arg(long)]
    acquisition: String,
    #[arg(long)]
    source: String,
    /// Existing membership.parquet or predictions.parquet.
    #[arg(long)]
    annotations: PathBuf,
    #[arg(long)]
    producer_provenance: PathBuf,
    /// The one final combined mask used by both the producer and QC.
    #[arg(long)]
    mask: PathBuf,
    #[arg(long)]
    qc_report: PathBuf,
    #[arg(long)]
    qc_provenance: PathBuf,
}

impl From<ObserveInputs> for ObserveRequest {
    fn from(a: ObserveInputs) -> Self {
        Self {
            plan: a.plan,
            acquisition: a.acquisition,
            source: a.source,
            annotations: a.annotations,
            producer_provenance: a.producer_provenance,
            mask: a.mask,
            qc_report: a.qc_report,
            qc_provenance: a.qc_provenance,
        }
    }
}

#[derive(Debug, Serialize, Args)]
pub(crate) struct ObserveArgs {
    #[command(flatten)]
    inputs: ObserveInputs,
    #[arg(long)]
    output: PathBuf,
    #[arg(long)]
    overwrite: bool,
}

pub(crate) fn run(
    command: PopulationCommand,
    recorder: RunRecorder,
) -> Result<CommandOutcome, Box<dyn std::error::Error>> {
    match command {
        PopulationCommand::Validate(args) => finish(
            "cytiq population validate",
            runner::validate_population(&args.into(), recorder),
        ),
        PopulationCommand::Observe(args) => finish(
            "cytiq population observe",
            runner::write_observations(&args.inputs.into(), &args.output, args.overwrite, recorder),
        ),
    }
}
