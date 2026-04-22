use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};
use cytiq_autospill::{
    AutospillConfig, AutospillError, AutospillRunner, AutospreadConfig, AutospreadRunner,
    MatrixKind, export_matrix, export_spread_matrix, read_controls_manifest,
    read_detectors_manifest, read_transform_manifest,
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
        #[arg(long)]
        detectors: Option<PathBuf>,
        #[arg(long)]
        transform_file: Option<PathBuf>,
        #[arg(long)]
        debug_dir: Option<PathBuf>,
        #[arg(long, value_enum, default_value_t = CliMatrixKind::Compensation)]
        matrix_kind: CliMatrixKind,
        #[arg(long, default_value = "FSC-A")]
        fsc: String,
        #[arg(long, default_value = "SSC-A")]
        ssc: String,
    },
    /// Estimate a spillover spreading matrix from single-color controls.
    Autospread {
        #[arg(long)]
        controls: PathBuf,
        #[arg(long)]
        input_dir: PathBuf,
        #[arg(long)]
        output: PathBuf,
        #[arg(long)]
        detectors: Option<PathBuf>,
        #[arg(long)]
        debug_dir: Option<PathBuf>,
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
            detectors,
            transform_file,
            debug_dir,
            matrix_kind,
            fsc,
            ssc,
        } => {
            let manifest = read_controls_manifest(&controls)?;
            let detectors = detectors
                .map(read_detectors_manifest)
                .transpose()?
                .unwrap_or_default();
            let transforms = transform_file
                .map(read_transform_manifest)
                .transpose()?
                .unwrap_or_default();
            let config = AutospillConfig {
                fsc,
                ssc,
                matrix_kind: matrix_kind.into(),
                detectors,
                transforms,
                debug_dir,
            };
            let result = AutospillRunner::run(&manifest, &input_dir, &config)?;
            export_matrix(&result, config.matrix_kind, &output)?;
            Ok(())
        }
        Commands::Autospread {
            controls,
            input_dir,
            output,
            detectors,
            debug_dir,
            fsc,
            ssc,
        } => {
            let manifest = read_controls_manifest(&controls)?;
            let detectors = detectors
                .map(read_detectors_manifest)
                .transpose()?
                .unwrap_or_default();
            let config = AutospreadConfig {
                fsc,
                ssc,
                detectors,
                debug_dir,
            };
            let result = AutospreadRunner::run(&manifest, &input_dir, &config)?;
            export_spread_matrix(&result, &output)?;
            Ok(())
        }
    }
}
