use clap::{Args, Subcommand};
use cytiq_provenance::{RunRecorder, outcome::CommandOutcome};
use cytiq_specimen::runner::{self, PlanConfig};
use serde::Serialize;
use std::path::PathBuf;

#[derive(Debug, Serialize, Subcommand)]
pub(crate) enum SpecimenCommand {
    /// Validate protocol definitions, sources and acquisition selection.
    Validate(PlanInputs),
    /// Freeze a specimen plan in an atomically published package.
    Plan(PlanArgs),
    /// Reconcile observation packages without reading event-level data.
    Run(RunArgs),
}

#[derive(Debug, Serialize, Args)]
pub(crate) struct PlanInputs {
    #[arg(long)]
    manifest: PathBuf,
    #[arg(long)]
    layout: PathBuf,
    #[arg(long)]
    specimen: String,
    /// Explicit acquisition selection, repeat for panels with multiple files.
    #[arg(long = "select", value_name = "PANEL=ACQUISITION")]
    selections: Vec<String>,
}

impl From<PlanInputs> for PlanConfig {
    fn from(args: PlanInputs) -> Self {
        Self {
            manifest: args.manifest,
            layout: args.layout,
            specimen: args.specimen,
            selections: args.selections,
        }
    }
}

#[derive(Debug, Serialize, Args)]
pub(crate) struct PlanArgs {
    #[command(flatten)]
    inputs: PlanInputs,
    /// Output package directory (contains plan.json).
    #[arg(long)]
    output: PathBuf,
    #[arg(long)]
    overwrite: bool,
}

#[derive(Debug, Serialize, Args)]
pub(crate) struct RunArgs {
    /// plan.json from a completed specimen plan package.
    #[arg(long)]
    plan: PathBuf,
    /// Population observation package; repeat for each selected source.
    #[arg(long = "observations")]
    observations: Vec<PathBuf>,
    #[arg(long)]
    output: PathBuf,
    #[arg(long)]
    overwrite: bool,
}

#[derive(Debug)]
pub(crate) struct OutcomeExit(pub CommandOutcome);
impl std::fmt::Display for OutcomeExit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}: {}", self.0.reason, self.0.message)
    }
}
impl std::error::Error for OutcomeExit {}

pub(crate) fn finish(
    command: &str,
    result: cytiq_specimen::Result<CommandOutcome>,
) -> Result<(), Box<dyn std::error::Error>> {
    let outcome = match result {
        Ok(outcome) => outcome,
        Err(error) => CommandOutcome::new(
            command,
            false,
            runner::error_reason(&error),
            error.to_string(),
        ),
    };
    println!("{}", serde_json::to_string(&outcome)?);
    if outcome.exit_code == 0 {
        Ok(())
    } else {
        Err(Box::new(OutcomeExit(outcome)))
    }
}

pub(crate) fn run(
    command: SpecimenCommand,
    recorder: RunRecorder,
) -> Result<(), Box<dyn std::error::Error>> {
    match command {
        SpecimenCommand::Validate(args) => finish(
            "cytiq specimen validate",
            runner::validate_plan(&args.into(), recorder),
        ),
        SpecimenCommand::Plan(args) => finish(
            "cytiq specimen plan",
            runner::write_plan(&args.inputs.into(), &args.output, args.overwrite, recorder),
        ),
        SpecimenCommand::Run(args) => finish(
            "cytiq specimen run",
            runner::run_specimen(
                &args.plan,
                &args.observations,
                &args.output,
                args.overwrite,
                recorder,
            ),
        ),
    }
}
