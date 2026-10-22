use std::env;
use std::io::{self, IsTerminal, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::OnceLock;

use clap::{Parser, Subcommand, ValueEnum};
use cytiq_autospectral::{
    AutoSpectralConfig, AutoSpectralRunner,
    read_controls_manifest as read_autospectral_controls_manifest,
    read_detectors_manifest as read_autospectral_detectors_manifest, read_samples_manifest,
};
use cytiq_autospill::{
    AutospillConfig, AutospillRunner, AutospreadConfig, AutospreadRunner, MatrixKind,
    export_matrix, export_spread_matrix, read_controls_manifest, read_detectors_manifest,
    read_transform_manifest,
};
use cytiq_cytoscan::{
    CytoScanConfig, CytoScanRunner, FeatureMethod, FlagMethod, PreprocessingMode,
    parse_channel_list as parse_cytoscan_channel_list,
};
use cytiq_flowsom::{
    FlowSomConfig, FlowSomRunner, parse_channel_list as parse_flowsom_channel_list,
};
use cytiq_peacoqc::{
    PeacoQcConfig, PeacoQcMethod, PeacoQcRunner, parse_channel_list as parse_peacoqc_channel_list,
};
use cytiq_transform::{
    BiexponentialParameters, LogicleParameters, TransformConfig, TransformFormat, TransformMethod,
    TransformParameters, TransformRunResult, TransformRunner,
    parse_channel_list as parse_transform_channel_list,
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
        /// Worker thread limit; defaults to all available logical CPUs.
        #[arg(long, value_parser = parse_threads)]
        threads: Option<usize>,
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
        /// Worker thread limit; defaults to all available logical CPUs.
        #[arg(long, value_parser = parse_threads)]
        threads: Option<usize>,
    },
    /// Unmix spectral flow cytometry samples from single-stain controls.
    Autospectral {
        /// Controls manifest CSV with `filename,dye[,primary_channel]`; include one `dye=Autofluorescence` row.
        #[arg(long)]
        controls: PathBuf,
        /// Samples manifest CSV with `filename` rows for fully stained FCS files.
        #[arg(long)]
        samples: PathBuf,
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
        /// Worker thread limit; defaults to all available logical CPUs.
        #[arg(long, value_parser = parse_threads)]
        threads: Option<usize>,
    },
    /// Remove unstable acquisition regions using PeacoQC-style peak-based QC.
    Peacoqc {
        #[arg(long)]
        input: PathBuf,
        #[arg(long)]
        output: PathBuf,
        /// Comma-separated channel names used for peak detection and QC.
        #[arg(long)]
        channels: Option<String>,
        /// Channel name used for peak detection and QC; can be repeated.
        #[arg(long = "channel")]
        channel: Vec<String>,
        #[arg(long, value_enum, default_value_t = CliPeacoQcMethod::All)]
        method: CliPeacoQcMethod,
        /// Remove detector margin events before peak-based QC.
        #[arg(long)]
        remove_margins: bool,
        /// Ignore zero values during peak detection; useful for mass cytometry.
        #[arg(long)]
        remove_zeros: bool,
        #[arg(long)]
        debug_dir: Option<PathBuf>,
        /// Worker thread limit; defaults to all available logical CPUs.
        #[arg(long, value_parser = parse_threads)]
        threads: Option<usize>,
    },
    /// Flag anomalous FCS files using a CytoScan-style cohort workflow.
    Cytoscan {
        /// Input .fcs file, glob pattern, or directory. Can be passed multiple times.
        #[arg(long, required = true)]
        input: Vec<PathBuf>,
        /// Reference .fcs file, glob pattern, or directory for novelty detection.
        #[arg(long)]
        reference: Vec<PathBuf>,
        /// Output directory for summary, feature, and configuration tables.
        #[arg(long)]
        output: PathBuf,
        /// Comma-separated channel names used for feature generation.
        #[arg(long)]
        channels: Option<String>,
        /// Channel name used for feature generation; can be repeated.
        #[arg(long = "channel")]
        channel: Vec<String>,
        #[arg(long, value_enum, default_value_t = CliCytoScanFeatureMethod::Quantiles)]
        method: CliCytoScanFeatureMethod,
        #[arg(long, value_enum, default_value_t = CliCytoScanFlagMethod::Outlier)]
        flag_method: CliCytoScanFlagMethod,
        /// `default` truncates at `$PnR`, compensates with `SPILL` and applies
        /// `asinh(x / 150)`; `none` uses already normalized input values as-is.
        #[arg(long, value_enum, default_value_t = CliCytoScanPreprocessing::Default)]
        preprocessing: CliCytoScanPreprocessing,
        #[arg(long)]
        debug_dir: Option<PathBuf>,
        /// Worker thread limit; defaults to all available logical CPUs.
        #[arg(long, value_parser = parse_threads)]
        threads: Option<usize>,
    },
    /// Transform FCS signal intensities into analysis-ready values.
    Transform {
        /// Input .fcs file, glob pattern, or directory. Can be passed multiple times.
        #[arg(short, long, required = true)]
        input: Vec<PathBuf>,
        /// Output file path for one input, or output directory for multiple inputs.
        #[arg(short, long)]
        output: PathBuf,
        #[arg(long, value_enum, default_value_t = CliTransformMethod::Arcsinh)]
        method: CliTransformMethod,
        /// Cofactor for arcsinh transformation.
        #[arg(long, default_value_t = 150.0, value_parser = parse_positive_f64)]
        cofactor: f64,
        /// Logicle/hyperlog T parameter, top of scale.
        #[arg(long, default_value_t = 262_144.0, value_parser = parse_positive_f64)]
        logicle_t: f64,
        /// Logicle/hyperlog M parameter, number of decades.
        #[arg(long, default_value_t = 4.5, value_parser = parse_positive_f64)]
        logicle_m: f64,
        /// Logicle/hyperlog W parameter, width of linear region.
        #[arg(long, default_value_t = 0.5, value_parser = parse_non_negative_f64)]
        logicle_w: f64,
        /// Logicle/hyperlog A parameter, additional negative decades.
        #[arg(long, default_value_t = 0.0, value_parser = parse_non_negative_f64)]
        logicle_a: f64,
        /// Worker thread limit; defaults to all available logical CPUs.
        #[arg(long, value_parser = parse_threads)]
        threads: Option<usize>,
        /// Comma-separated channel names to transform.
        #[arg(long)]
        channels: Option<String>,
        /// Comma-separated channel names to leave untransformed.
        #[arg(long)]
        exclude_channels: Option<String>,
        #[arg(long, value_enum, default_value_t = CliTransformFormat::Fcs)]
        format: CliTransformFormat,
        /// Output filename suffix when writing to a directory.
        #[arg(long, default_value = ".transformed")]
        suffix: String,
        /// Allow replacing existing output files.
        #[arg(long)]
        overwrite: bool,
        /// Print resolved inputs, channels, and outputs without writing files.
        #[arg(long)]
        dry_run: bool,
        /// Optional path to write transformation metadata as JSON.
        #[arg(long)]
        metadata_json: Option<PathBuf>,
        /// Optional directory for transformation metrics and Plotly report.
        #[arg(long)]
        debug_dir: Option<PathBuf>,
    },
    /// Cluster cytometry events using a deterministic FlowSOM-style workflow.
    Flowsom {
        #[arg(long)]
        input: PathBuf,
        #[arg(long)]
        output: PathBuf,
        /// Comma-separated channel names used for clustering.
        #[arg(long)]
        channels: Option<String>,
        /// Channel name used for clustering; can be repeated.
        #[arg(long = "channel")]
        channel: Vec<String>,
        #[arg(long)]
        debug_dir: Option<PathBuf>,
        /// Worker thread limit; defaults to all available logical CPUs.
        #[arg(long, value_parser = parse_threads)]
        threads: Option<usize>,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum CliPeacoQcMethod {
    All,
    Mad,
    It,
    PeaksOnly,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum CliTransformMethod {
    Arcsinh,
    Biexponential,
    Hyperlog,
    Logicle,
    None,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum CliCytoScanFeatureMethod {
    Summary,
    Quantiles,
    Emd,
    Binning,
    Fingerprint,
}

impl From<CliCytoScanFeatureMethod> for FeatureMethod {
    fn from(value: CliCytoScanFeatureMethod) -> Self {
        match value {
            CliCytoScanFeatureMethod::Summary => Self::Summary,
            CliCytoScanFeatureMethod::Quantiles => Self::Quantiles,
            CliCytoScanFeatureMethod::Emd => Self::Emd,
            CliCytoScanFeatureMethod::Binning => Self::Binning,
            CliCytoScanFeatureMethod::Fingerprint => Self::Fingerprint,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum CliCytoScanFlagMethod {
    Outlier,
    Novelty,
    Both,
}

impl From<CliCytoScanFlagMethod> for FlagMethod {
    fn from(value: CliCytoScanFlagMethod) -> Self {
        match value {
            CliCytoScanFlagMethod::Outlier => Self::Outlier,
            CliCytoScanFlagMethod::Novelty => Self::Novelty,
            CliCytoScanFlagMethod::Both => Self::Both,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum CliCytoScanPreprocessing {
    Default,
    None,
}

impl From<CliCytoScanPreprocessing> for PreprocessingMode {
    fn from(value: CliCytoScanPreprocessing) -> Self {
        match value {
            CliCytoScanPreprocessing::Default => Self::Default,
            CliCytoScanPreprocessing::None => Self::None,
        }
    }
}

impl From<CliTransformMethod> for TransformMethod {
    fn from(value: CliTransformMethod) -> Self {
        match value {
            CliTransformMethod::Arcsinh => Self::Arcsinh,
            CliTransformMethod::Biexponential => Self::Biexponential,
            CliTransformMethod::Hyperlog => Self::Hyperlog,
            CliTransformMethod::Logicle => Self::Logicle,
            CliTransformMethod::None => Self::None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum CliTransformFormat {
    Fcs,
    Csv,
    Parquet,
}

impl From<CliTransformFormat> for TransformFormat {
    fn from(value: CliTransformFormat) -> Self {
        match value {
            CliTransformFormat::Fcs => Self::Fcs,
            CliTransformFormat::Csv => Self::Csv,
            CliTransformFormat::Parquet => Self::Parquet,
        }
    }
}

impl From<CliPeacoQcMethod> for PeacoQcMethod {
    fn from(value: CliPeacoQcMethod) -> Self {
        match value {
            CliPeacoQcMethod::All => Self::All,
            CliPeacoQcMethod::Mad => Self::Mad,
            CliPeacoQcMethod::It => Self::It,
            CliPeacoQcMethod::PeaksOnly => Self::PeaksOnly,
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
fn run() -> Result<(), Box<dyn std::error::Error>> {
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
            threads,
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
                threads,
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
            threads,
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
                threads,
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
        Commands::Autospectral {
            controls,
            samples,
            input_dir,
            output,
            detectors,
            debug_dir,
            fsc,
            ssc,
            threads,
        } => {
            info!(
                "starting autospectral: controls={}, samples={}, input_dir={}, output={}",
                controls.display(),
                samples.display(),
                input_dir.display(),
                output.display()
            );
            debug!("autospectral channels: FSC=`{fsc}`, SSC=`{ssc}`");
            let control_manifest = read_autospectral_controls_manifest(&controls)?;
            let sample_manifest = read_samples_manifest(&samples)?;
            info!(
                "loaded {} control rows and {} sample rows",
                control_manifest.len(),
                sample_manifest.len()
            );
            let detectors = detectors
                .map(read_autospectral_detectors_manifest)
                .transpose()?
                .unwrap_or_default();
            if detectors.is_empty() {
                info!("no detector manifest provided; spectral detector order will be inferred");
            } else {
                info!("loaded {} detector specs", detectors.len());
            }
            let config = AutoSpectralConfig {
                fsc,
                ssc,
                detectors,
                output_dir: output.clone(),
                debug_dir,
                threads,
            };
            let summary =
                AutoSpectralRunner::run(&control_manifest, &sample_manifest, &input_dir, &config)?;
            info!(
                "autospectral finished {} samples across {} components; condition_number={:.6e}",
                summary.samples.len(),
                summary.components.len(),
                summary.condition_number
            );
            info!("wrote autospectral outputs to {}", output.display());
            info!("autospectral completed successfully");
            Ok(())
        }
        Commands::Peacoqc {
            input,
            output,
            channels,
            channel,
            method,
            remove_margins,
            remove_zeros,
            debug_dir,
            threads,
        } => {
            info!(
                "starting peacoqc: input={}, output={}, method={:?}",
                input.display(),
                output.display(),
                method
            );
            let channels = parse_peacoqc_channels(channels.as_deref(), &channel)
                .map_err(|message| io::Error::new(io::ErrorKind::InvalidInput, message))?;
            let config = PeacoQcConfig {
                channels,
                method: method.into(),
                remove_margins,
                remove_zeros,
                debug_dir,
                threads,
            };
            let result = PeacoQcRunner::run(&input, &output, &config)?;
            info!(
                "peacoqc retained {} of {} events",
                result.summary.metrics.events_after, result.summary.metrics.events_before
            );
            if let Some(debug_dir) = config.debug_dir.as_ref() {
                info!("wrote peacoqc debug artifacts to {}", debug_dir.display());
            }
            info!("peacoqc completed successfully");
            Ok(())
        }
        Commands::Cytoscan {
            input,
            reference,
            output,
            channels,
            channel,
            method,
            flag_method,
            preprocessing,
            debug_dir,
            threads,
        } => {
            info!(
                "starting cytoscan: inputs={}, references={}, output={}, method={:?}, flag_method={:?}, preprocessing={:?}",
                input.len(),
                reference.len(),
                output.display(),
                method,
                flag_method,
                preprocessing
            );
            let channels = parse_cytoscan_channels(channels.as_deref(), &channel)
                .map_err(|message| io::Error::new(io::ErrorKind::InvalidInput, message))?;
            let config = CytoScanConfig {
                inputs: input,
                reference_inputs: reference,
                output_dir: output,
                channels,
                feature_method: method.into(),
                flag_method: flag_method.into(),
                preprocessing: preprocessing.into(),
                debug_dir,
                threads,
            };
            let result = CytoScanRunner::run(&config)?;
            info!(
                "cytoscan wrote {} summary row(s) and {} feature column(s)",
                result.summary_rows.len(),
                result.test_features.feature_names.len()
            );
            if let Some(debug_dir) = config.debug_dir.as_ref() {
                info!("wrote cytoscan debug artifacts to {}", debug_dir.display());
            }
            info!("cytoscan completed successfully");
            Ok(())
        }
        Commands::Transform {
            input,
            output,
            method,
            cofactor,
            logicle_t,
            logicle_m,
            logicle_w,
            logicle_a,
            threads,
            channels,
            exclude_channels,
            format,
            suffix,
            overwrite,
            dry_run,
            metadata_json,
            debug_dir,
        } => {
            info!(
                "starting transform: inputs={}, output={}, method={:?}",
                input.len(),
                output.display(),
                method
            );
            let channels = parse_optional_transform_channels(channels.as_deref())
                .map_err(|message| io::Error::new(io::ErrorKind::InvalidInput, message))?;
            let exclude_channels =
                parse_optional_transform_channels(exclude_channels.as_deref())
                    .map_err(|message| io::Error::new(io::ErrorKind::InvalidInput, message))?;
            let config = TransformConfig {
                inputs: input,
                output,
                method: method.into(),
                cofactor,
                logicle: LogicleParameters {
                    t: logicle_t,
                    m: logicle_m,
                    w: logicle_w,
                    a: logicle_a,
                },
                biexponential: BiexponentialParameters::default(),
                channels,
                exclude_channels,
                format: format.into(),
                suffix,
                overwrite,
                dry_run,
                metadata_json,
                debug_dir,
                threads,
            };
            let result = TransformRunner::run(&config)?;
            if result.dry_run {
                print_transform_dry_run(&result);
            }
            if let Some(debug_dir) = config.debug_dir.as_ref() {
                info!("wrote transform debug artifacts to {}", debug_dir.display());
            }
            info!("transform completed for {} file(s)", result.files.len());
            Ok(())
        }
        Commands::Flowsom {
            input,
            output,
            channels,
            channel,
            debug_dir,
            threads,
        } => {
            info!(
                "starting flowsom: input={}, output={}",
                input.display(),
                output.display()
            );
            let channels = parse_flowsom_channels(channels.as_deref(), &channel)
                .map_err(|message| io::Error::new(io::ErrorKind::InvalidInput, message))?;
            let config = FlowSomConfig {
                channels,
                debug_dir,
                threads,
            };
            let result = FlowSomRunner::run(&input, &output, &config)?;
            info!(
                "flowsom assigned {} events across {} SOM nodes and {} metaclusters",
                result.summary.metrics.events,
                result.summary.metrics.som_nodes,
                result.summary.metrics.metaclusters
            );
            if let Some(debug_dir) = config.debug_dir.as_ref() {
                info!("wrote flowsom debug artifacts to {}", debug_dir.display());
            }
            info!("flowsom completed successfully");
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

fn parse_threads(raw: &str) -> Result<usize, String> {
    let threads = raw
        .parse::<usize>()
        .map_err(|error| format!("invalid thread count `{raw}`: {error}"))?;
    if threads == 0 {
        Err("thread count must be greater than zero".to_owned())
    } else {
        Ok(threads)
    }
}

fn parse_positive_f64(raw: &str) -> Result<f64, String> {
    let value = raw
        .parse::<f64>()
        .map_err(|error| format!("invalid number `{raw}`: {error}"))?;
    if value.is_finite() && value > 0.0 {
        Ok(value)
    } else {
        Err("value must be finite and greater than zero".to_owned())
    }
}

fn parse_non_negative_f64(raw: &str) -> Result<f64, String> {
    let value = raw
        .parse::<f64>()
        .map_err(|error| format!("invalid number `{raw}`: {error}"))?;
    if value.is_finite() && value >= 0.0 {
        Ok(value)
    } else {
        Err("value must be finite and greater than or equal to zero".to_owned())
    }
}

fn parse_optional_transform_channels(raw: Option<&str>) -> Result<Vec<String>, String> {
    raw.map(parse_transform_channel_list)
        .transpose()
        .map_err(|error| error.to_string())
        .map(Option::unwrap_or_default)
}

fn print_transform_dry_run(result: &TransformRunResult) {
    println!("cytiq transform dry run");
    for file in &result.files {
        println!(
            "{} -> {}",
            file.input_path.display(),
            file.output_path.display()
        );
        println!("  method: {}", file.method.as_str());
        let parameters = format_transform_parameters(&file.parameters);
        if !parameters.is_empty() {
            println!("  parameters: {parameters}");
        }
        println!("  events: {}", file.events);
        println!("  channels: {}", file.channels.join(","));
        if !file.excluded_channels.is_empty() {
            println!("  excluded: {}", file.excluded_channels.join(","));
        }
    }
    if let Some(path) = result.metadata_path.as_ref() {
        println!("metadata: {}", path.display());
    }
}

fn format_transform_parameters(parameters: &TransformParameters) -> String {
    if let Some(cofactor) = parameters.cofactor {
        return format!("cofactor={cofactor}");
    }
    if let Some(logicle) = parameters.logicle {
        return format!(
            "logicle_t={}, logicle_m={}, logicle_w={}, logicle_a={}",
            logicle.t, logicle.m, logicle.w, logicle.a
        );
    }
    if let Some(hyperlog) = parameters.hyperlog {
        return format!(
            "logicle_t={}, logicle_m={}, logicle_w={}, logicle_a={}",
            hyperlog.t, hyperlog.m, hyperlog.w, hyperlog.a
        );
    }
    if let Some(biexponential) = parameters.biexponential {
        return format!(
            "length={}, max_range={}, pos={}, neg={}, width={}",
            biexponential.length,
            biexponential.max_range,
            biexponential.pos,
            biexponential.neg,
            biexponential.width
        );
    }
    String::new()
}

fn parse_peacoqc_channels(
    channels: Option<&str>,
    repeated_channels: &[String],
) -> Result<Vec<String>, String> {
    let mut parsed = channels
        .map(parse_peacoqc_channel_list)
        .transpose()?
        .unwrap_or_default();
    for channel in repeated_channels {
        let channel = channel.trim();
        if channel.is_empty() {
            return Err("channel must not be empty".to_owned());
        }
        parsed.push(channel.to_owned());
    }
    if parsed.is_empty() {
        Err("at least one `--channels` or `--channel` value is required".to_owned())
    } else {
        Ok(parsed)
    }
}

fn parse_flowsom_channels(
    channels: Option<&str>,
    repeated_channels: &[String],
) -> Result<Vec<String>, String> {
    let mut parsed = channels
        .map(parse_flowsom_channel_list)
        .transpose()?
        .unwrap_or_default();
    for channel in repeated_channels {
        let channel = channel.trim();
        if channel.is_empty() {
            return Err("channel must not be empty".to_owned());
        }
        parsed.push(channel.to_owned());
    }
    if parsed.is_empty() {
        Err("at least one `--channels` or `--channel` value is required".to_owned())
    } else {
        Ok(parsed)
    }
}

fn parse_cytoscan_channels(
    channels: Option<&str>,
    repeated_channels: &[String],
) -> Result<Vec<String>, String> {
    let mut parsed = channels
        .map(parse_cytoscan_channel_list)
        .transpose()?
        .unwrap_or_default();
    for channel in repeated_channels {
        let channel = channel.trim();
        if channel.is_empty() {
            return Err("channel must not be empty".to_owned());
        }
        parsed.push(channel.to_owned());
    }
    if parsed.is_empty() {
        Err("at least one `--channels` or `--channel` value is required".to_owned())
    } else {
        Ok(parsed)
    }
}
