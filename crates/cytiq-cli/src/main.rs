use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};
use cytiq_autospill::{
    AutospillConfig, AutospillError, AutospillRunner, MatrixKind, export_matrix,
    read_controls_manifest,
};

#[derive(Debug, Parser)]
#[command(name = "cytiq")]
#[command(about = "Command-line tools for cytiq workflows.")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Estimate a spillover or compensation matrix from single-color controls.
    Autospill {
        #[arg(long)]
        controls: PathBuf,
        #[arg(long)]
        input_dir: PathBuf,
        #[arg(long)]
        output: PathBuf,
        #[arg(long, value_enum, default_value_t = CliMatrixKind::Compensation)]
        matrix_kind: CliMatrixKind,
        #[arg(long, default_value = "FSC-A")]
        fsc: String,
        #[arg(long, default_value = "SSC-A")]
        ssc: String,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum CliMatrixKind {
    Compensation,
    Spillover,
}

impl From<CliMatrixKind> for MatrixKind {
    fn from(value: CliMatrixKind) -> Self {
        match value {
            CliMatrixKind::Compensation => Self::Compensation,
            CliMatrixKind::Spillover => Self::Spillover,
        }
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::from(1)
        }
    }
}

fn run() -> Result<(), AutospillError> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Autospill {
            controls,
            input_dir,
            output,
            matrix_kind,
            fsc,
            ssc,
        } => {
            let manifest = read_controls_manifest(&controls)?;
            let config = AutospillConfig {
                fsc,
                ssc,
                matrix_kind: matrix_kind.into(),
            };
            let result = AutospillRunner::run(&manifest, &input_dir, &config)?;
            export_matrix(&result, config.matrix_kind, &output)?;
            Ok(())
        }
    }
}
