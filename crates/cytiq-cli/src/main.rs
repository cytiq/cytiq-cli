use std::env;
use std::io::{self, IsTerminal, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::OnceLock;

use clap::{Parser, Subcommand, ValueEnum};
use cytiq_autospill::{
    AutospillConfig, AutospillError, AutospillRunner, AutospreadConfig, AutospreadRunner,
    MatrixKind, export_matrix, export_spread_matrix, read_controls_manifest,
    read_detectors_manifest, read_transform_manifest,
};
use log::{LevelFilter, Log, Metadata, Record, SetLoggerError, debug, info};

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
        /// Controls manifest CSV with `filename,dye`; primary channels are inferred from FCS metadata.
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
        /// Controls manifest CSV with `filename,dye`; primary channels are inferred from FCS metadata.
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

static LOGGER: SimpleLogger = SimpleLogger::new();

struct SimpleLogger {
    level: OnceLock<LevelFilter>,
}

impl SimpleLogger {
    const fn new() -> Self {
        Self {
            level: OnceLock::new(),
        }
    }
}

impl Log for SimpleLogger {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        self.level.get().copied().unwrap_or(LevelFilter::Off) >= metadata.level().to_level_filter()
    }

    fn log(&self, record: &Record<'_>) {
        if !self.enabled(record.metadata()) {
            return;
        }

        let target = record.module_path_static().unwrap_or(record.target());
        let mut stderr = io::stderr();
        let _ = writeln!(stderr, "[{}] {}: {}", record.level(), target, record.args());
    }

    fn flush(&self) {}
}

fn main() -> ExitCode {
    if let Err(error) = init_logging() {
        eprintln!("failed to initialize logging: {error}");
    }

    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::from(1)
        }
    }
}

#[allow(clippy::too_many_lines)]
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
            info!(
                "starting autospill: controls={}, input_dir={}, output={}, matrix_kind={:?}",
                controls.display(),
                input_dir.display(),
                output.display(),
                matrix_kind
            );
            debug!("autospill channels: FSC=`{fsc}`, SSC=`{ssc}`");
            let manifest = read_controls_manifest(&controls)?;
            info!(
                "loaded {} control rows from {}",
                manifest.len(),
                controls.display()
            );
            let detectors = detectors
                .map(read_detectors_manifest)
                .transpose()?
                .unwrap_or_default();
            if detectors.is_empty() {
                info!("no detector manifest provided; detector order will be inferred");
            } else {
                info!("loaded {} detector specs", detectors.len());
            }
            let transforms = transform_file
                .map(read_transform_manifest)
                .transpose()?
                .unwrap_or_default();
            if transforms.is_empty() {
                debug!("no transform manifest provided");
            } else {
                info!("loaded {} transform specs", transforms.len());
            }
            let config = AutospillConfig {
                fsc,
                ssc,
                matrix_kind: matrix_kind.into(),
                detectors,
                transforms,
                debug_dir,
            };
            let result = AutospillRunner::run(&manifest, &input_dir, &config)?;
            info!(
                "autospill finished computation for {} dyes across {} detectors",
                result.dyes.len(),
                result.detectors.len()
            );
            export_matrix(&result, config.matrix_kind, &output)?;
            info!(
                "wrote {:?} matrix to {}",
                config.matrix_kind,
                output.display()
            );
            if let Some(debug_dir) = config.debug_dir.as_ref() {
                info!("wrote debug artifacts to {}", debug_dir.display());
            }
            info!("autospill completed successfully");
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
            info!(
                "starting autospread: controls={}, input_dir={}, output={}",
                controls.display(),
                input_dir.display(),
                output.display()
            );
            debug!("autospread channels: FSC=`{fsc}`, SSC=`{ssc}`");
            let manifest = read_controls_manifest(&controls)?;
            info!(
                "loaded {} control rows from {}",
                manifest.len(),
                controls.display()
            );
            let detectors = detectors
                .map(read_detectors_manifest)
                .transpose()?
                .unwrap_or_default();
            if detectors.is_empty() {
                info!("no detector manifest provided; detector order will be inferred");
            } else {
                info!("loaded {} detector specs", detectors.len());
            }
            let config = AutospreadConfig {
                fsc,
                ssc,
                detectors,
                debug_dir,
            };
            let result = AutospreadRunner::run(&manifest, &input_dir, &config)?;
            export_spread_matrix(&result, &output)?;
            info!(
                "autospread finished computation for {} dyes across {} detectors",
                result.dyes.len(),
                result.detectors.len()
            );
            info!("wrote spread matrix to {}", output.display());
            if let Some(debug_dir) = config.debug_dir.as_ref() {
                info!("wrote debug artifacts to {}", debug_dir.display());
            }
            info!("autospread completed successfully");
            Ok(())
        }
    }
}

fn init_logging() -> Result<(), SetLoggerError> {
    let level = configured_log_level();
    let _ = LOGGER.level.set(level);
    log::set_logger(&LOGGER)?;
    log::set_max_level(level);
    Ok(())
}

fn configured_log_level() -> LevelFilter {
    env::var("CYTIQ_LOG")
        .ok()
        .or_else(|| env::var("RUST_LOG").ok())
        .as_deref()
        .and_then(parse_log_level)
        .unwrap_or_else(default_log_level)
}

fn default_log_level() -> LevelFilter {
    if io::stderr().is_terminal() {
        LevelFilter::Info
    } else {
        LevelFilter::Warn
    }
}

fn parse_log_level(raw: &str) -> Option<LevelFilter> {
    raw.split(',')
        .rev()
        .find_map(|token| parse_log_level_token(token.trim()))
}

fn parse_log_level_token(token: &str) -> Option<LevelFilter> {
    let candidate = token.rsplit('=').next().unwrap_or(token).trim();
    match candidate.to_ascii_lowercase().as_str() {
        "off" => Some(LevelFilter::Off),
        "error" => Some(LevelFilter::Error),
        "warn" | "warning" => Some(LevelFilter::Warn),
        "info" => Some(LevelFilter::Info),
        "debug" => Some(LevelFilter::Debug),
        "trace" => Some(LevelFilter::Trace),
        _ => None,
    }
}
