use std::env;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::OnceLock;

use clap::{Args, Parser, Subcommand, ValueEnum};
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
use cytiq_clean::{
    ChannelOverrides, CleanConfig, CleanRunner, DEFAULT_FSC_A, DEFAULT_FSC_H, DEFAULT_FSC_W,
    DEFAULT_MAX_FIT_EVENTS, DEFAULT_SINGLET_MAD, DEFAULT_SSC_A, DEFAULT_VIABILITY_COFACTOR,
    DEFAULT_VIABILITY_CONTROL_QUANTILE, DebrisConfig, DebrisMode, FilterKind, FilterStatus,
    ViabilityConfig,
};
use cytiq_compensate::{CompensateConfig, CompensateRunner, MatrixKind as CompensateMatrixKind};
use cytiq_cytoscan::{
    CytoScanConfig, CytoScanRunner, FeatureMethod, FlagMethod,
    parse_channel_list as parse_cytoscan_channel_list,
};
use cytiq_flowsom::{
    FLOWSOM_METACLUSTERS, FLOWSOM_RANDOM_SEED, FLOWSOM_XDIM, FLOWSOM_YDIM, FlowSomConfig,
    FlowSomRunner, parse_channel_list as parse_flowsom_channel_list,
};
use cytiq_gating::{GatingConfig, GatingRunner, OutputFormat as GateOutputFormat};
use cytiq_mask::{MaskCombineConfig, MaskCombineRunner};
use cytiq_meta::{MetaConfig, MetaRunner};
use cytiq_peacoqc::{
    PeacoQcConfig, PeacoQcMethod, PeacoQcRunner, masks_path as peacoqc_masks_path,
    parse_channel_list as parse_peacoqc_channel_list,
};
use cytiq_provenance::{RunRecorder, files_under, record_path_for_dir, record_path_for_file};
use cytiq_transform::{
    BiexponentialParameters, LogicleParameters, TransformConfig, TransformFormat, TransformMethod,
    TransformParameters, TransformRunResult, TransformRunner,
    parse_channel_list as parse_transform_channel_list,
};
use log::{LevelFilter, Log, Metadata, Record, SetLoggerError, debug, info, warn};

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
        /// Clean controls before building spectra: drop raw detector-margin
        /// events and gate FSC/SSC with the same KDE gate as `autospill`.
        #[arg(long)]
        clean_controls: bool,
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
        #[arg(long)]
        debug_dir: Option<PathBuf>,
        /// Worker thread limit; defaults to all available logical CPUs.
        #[arg(long, value_parser = parse_threads)]
        threads: Option<usize>,
        /// Fail instead of warning when a file has no spillover metadata or a
        /// selected channel is not in it.
        #[arg(long)]
        strict: bool,
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
        /// Logicle/hyperlog A parameter, additional negative decades (-W <= A <= M - 2W).
        #[arg(long, default_value_t = 0.0, value_parser = parse_finite_f64, allow_negative_numbers = true)]
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
        /// Transform inputs that `cytiq transform` already transformed.
        #[arg(long)]
        allow_retransform: bool,
    },
    /// Apply a Gating-ML 2.0 gating strategy to an FCS file and report per-gate counts.
    Gate {
        /// Input FCS file.
        #[arg(long)]
        input: PathBuf,
        /// Gating-ML 2.0 XML file with the gate definitions.
        #[arg(long)]
        gating_ml: PathBuf,
        /// Output path for the per-gate counts report.
        #[arg(long)]
        output: PathBuf,
        #[arg(long, value_enum, default_value_t = CliGateOutputFormat::Csv)]
        format: CliGateOutputFormat,
        /// Worker thread limit; defaults to all available logical CPUs.
        #[arg(long, value_parser = parse_threads)]
        threads: Option<usize>,
        /// Event mask Parquet (`masks.parquet`) restricting the gated events;
        /// repeat to combine several masks with AND.
        #[arg(long = "mask")]
        masks: Vec<PathBuf>,
        /// Optional Parquet path for per-event gate membership.
        #[arg(long)]
        membership: Option<PathBuf>,
    },
    /// Work with per-event masks (`masks.parquet`) written by cytiq stages.
    #[command(subcommand)]
    Mask(MaskCommand),
    /// Dump FCS metadata (file hash, parameters, all TEXT keywords) as JSON.
    Meta {
        /// Input FCS file.
        #[arg(long)]
        input: PathBuf,
        /// Output JSON path; omit or pass `-` to write to stdout.
        #[arg(long)]
        output: Option<PathBuf>,
        /// Replace an existing output file.
        #[arg(long)]
        overwrite: bool,
    },
    /// Apply a spillover or compensation matrix to FCS events; also writes a
    /// raw detector-margin mask next to the output.
    Compensate {
        /// Input FCS file with raw (uncompensated, untransformed) values.
        #[arg(long)]
        input: PathBuf,
        /// Output FCS file; `<output stem>.masks.parquet` is written beside it.
        #[arg(long)]
        output: PathBuf,
        /// Square matrix CSV/TSV as written by `cytiq autospill`; defaults to
        /// the input's `$SPILLOVER` keyword.
        #[arg(long, requires = "matrix_kind")]
        matrix: Option<PathBuf>,
        /// What `--matrix` holds.
        #[arg(long, value_enum, requires = "matrix")]
        matrix_kind: Option<CliMatrixKind>,
        /// Replace existing output files.
        #[arg(long)]
        overwrite: bool,
    },
    /// Remove debris, doublets and dead cells; writes per-event masks,
    /// metrics, Gating-ML gates and an HTML report.
    Clean(CleanArgs),
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
        /// Event mask Parquet (`masks.parquet`); only kept events are
        /// clustered. Repeat to combine several masks with AND.
        /// SOM grid width.
        #[arg(long, default_value_t = FLOWSOM_XDIM, value_parser = parse_positive_usize)]
        xdim: usize,
        /// SOM grid height.
        #[arg(long, default_value_t = FLOWSOM_YDIM, value_parser = parse_positive_usize)]
        ydim: usize,
        /// Number of metaclusters (at most xdim × ydim).
        #[arg(long, default_value_t = FLOWSOM_METACLUSTERS, value_parser = parse_positive_usize)]
        metaclusters: usize,
        /// Training seed.
        #[arg(long, default_value_t = FLOWSOM_RANDOM_SEED)]
        seed: u64,
        /// Cluster channel values as given instead of z-scoring them.
        #[arg(long)]
        no_scale: bool,
        /// Train the SOM on this many events (a deterministic sample), then
        /// assign every event; default trains on all events.
        #[arg(long, value_parser = parse_positive_usize)]
        training_events: Option<usize>,
        #[arg(long = "mask")]
        masks: Vec<PathBuf>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum CliFilterKind {
    Debris,
    Singlets,
    Viability,
}

impl From<CliFilterKind> for FilterKind {
    fn from(value: CliFilterKind) -> Self {
        match value {
            CliFilterKind::Debris => Self::Debris,
            CliFilterKind::Singlets => Self::Singlets,
            CliFilterKind::Viability => Self::Viability,
        }
    }
}

impl From<CliMatrixKind> for CompensateMatrixKind {
    fn from(value: CliMatrixKind) -> Self {
        match value {
            CliMatrixKind::Spillover => Self::Spillover,
            CliMatrixKind::Compensation => Self::Compensation,
        }
    }
}

#[derive(Debug, Subcommand)]
enum MaskCommand {
    /// Combine masks of one acquisition by `source_event_id`; an event is kept
    /// only when every mask keeps it.
    Combine {
        /// Event mask Parquet file; repeat for each mask to combine.
        #[arg(long = "mask", required = true)]
        masks: Vec<PathBuf>,
        /// Output Parquet path.
        #[arg(long)]
        output: PathBuf,
        /// Replace an existing output file.
        #[arg(long)]
        overwrite: bool,
    },
}

#[derive(Debug, Args)]
struct CleanArgs {
    /// Input FCS file.
    #[arg(long)]
    input: PathBuf,
    /// Output directory for masks.parquet, metrics.json, gates.gatingml.xml
    /// and report.html.
    #[arg(long)]
    output: PathBuf,
    /// Event mask Parquet from an earlier stage; filters are fitted on kept
    /// events only and `keep` is false outside it. Repeat to combine masks.
    #[arg(long = "mask")]
    masks: Vec<PathBuf>,
    /// Comma-separated filters that must be applied (`debris`, `singlets`,
    /// `viability`); otherwise the run fails without writing outputs.
    #[arg(long, value_enum, value_delimiter = ',')]
    require: Vec<CliFilterKind>,
    /// Re-apply these Gating-ML gates (e.g. an edited gates.gatingml.xml)
    /// instead of fitting new ones.
    #[arg(long)]
    gates: Option<PathBuf>,
    #[arg(long, default_value = DEFAULT_FSC_A)]
    fsc: String,
    #[arg(long, default_value = DEFAULT_FSC_H)]
    fsc_h: String,
    #[arg(long, default_value = DEFAULT_FSC_W)]
    fsc_w: String,
    #[arg(long, default_value = DEFAULT_SSC_A)]
    ssc: String,
    #[arg(long, value_enum, default_value_t = CliDebrisMode::Adaptive)]
    debris_mode: CliDebrisMode,
    /// Minimum FSC for `--debris-mode fixed`.
    #[arg(long, value_parser = parse_finite_f64)]
    debris_fsc_min: Option<f64>,
    /// Minimum SSC for `--debris-mode fixed`.
    #[arg(long, value_parser = parse_finite_f64)]
    debris_ssc_min: Option<f64>,
    /// Singlet band half-width, in MADs of the FSC-A/FSC-H ratio (or FSC-W).
    #[arg(long, value_parser = parse_positive_f64, default_value_t = DEFAULT_SINGLET_MAD)]
    singlet_mad: f64,
    /// Viability dye channel; auto-detected from $PnN/$PnS when omitted.
    #[arg(long, conflicts_with = "no_viability")]
    viability_channel: Option<String>,
    /// Skip the viability filter.
    #[arg(long)]
    no_viability: bool,
    /// Unstained/FMO control FCS; its upper quantile sets the live/dead threshold.
    #[arg(long, conflicts_with = "viability_threshold")]
    viability_control: Option<PathBuf>,
    #[arg(long, value_parser = parse_positive_f64, default_value_t = DEFAULT_VIABILITY_CONTROL_QUANTILE)]
    viability_control_quantile: f64,
    /// Explicit live/dead threshold in compensated intensity units.
    #[arg(long, value_parser = parse_finite_f64)]
    viability_threshold: Option<f64>,
    /// Arcsinh cofactor for the viability channel.
    #[arg(long, value_parser = parse_positive_f64, default_value_t = DEFAULT_VIABILITY_COFACTOR)]
    viability_cofactor: f64,
    /// Fit gates on at most this many evenly spaced events.
    #[arg(long, default_value_t = DEFAULT_MAX_FIT_EVENTS)]
    max_fit_events: usize,
    /// Worker thread limit; defaults to all available logical CPUs.
    #[arg(long, value_parser = parse_threads)]
    threads: Option<usize>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum CliDebrisMode {
    /// FSC-A threshold at the density valley above the debris peak.
    Adaptive,
    /// FSC/SSC auto-gate around the dominant population (beads, cell lines).
    Population,
    /// Thresholds from --debris-fsc-min / --debris-ssc-min.
    Fixed,
}

impl From<CliDebrisMode> for DebrisMode {
    fn from(value: CliDebrisMode) -> Self {
        match value {
            CliDebrisMode::Adaptive => Self::Adaptive,
            CliDebrisMode::Population => Self::Population,
            CliDebrisMode::Fixed => Self::Fixed,
        }
    }
}

impl From<CleanArgs> for CleanConfig {
    fn from(args: CleanArgs) -> Self {
        Self {
            input: args.input,
            output: args.output,
            channels: ChannelOverrides {
                fsc_a: args.fsc,
                fsc_h: args.fsc_h,
                fsc_w: args.fsc_w,
                ssc_a: args.ssc,
                viability: args.viability_channel,
                no_viability: args.no_viability,
            },
            debris: DebrisConfig {
                mode: args.debris_mode.into(),
                fsc_min: args.debris_fsc_min,
                ssc_min: args.debris_ssc_min,
            },
            singlet_mad: args.singlet_mad,
            viability: ViabilityConfig {
                cofactor: args.viability_cofactor,
                control: args.viability_control,
                control_quantile: args.viability_control_quantile,
                threshold: args.viability_threshold,
            },
            gates: args.gates,
            max_fit_events: args.max_fit_events,
            threads: args.threads,
            masks: args.masks,
            required: args.require.into_iter().map(FilterKind::from).collect(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum CliGateOutputFormat {
    Csv,
    Json,
}

impl From<CliGateOutputFormat> for GateOutputFormat {
    fn from(value: CliGateOutputFormat) -> Self {
        match value {
            CliGateOutputFormat::Csv => Self::Csv,
            CliGateOutputFormat::Json => Self::Json,
        }
    }
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
    let recorder = RunRecorder::start(
        format!("cytiq {}", command_name(&cli.command)),
        env!("CARGO_PKG_VERSION"),
        env::args().skip(1).collect(),
    );
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
            let mut inputs = vec![controls.clone()];
            inputs.extend(detectors.clone());
            inputs.extend(transform_file.clone());
            inputs.extend(manifest.iter().map(|row| input_dir.join(&row.filename)));
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
            record_provenance(
                recorder,
                &inputs,
                std::slice::from_ref(&output),
                &record_path_for_file(&output),
            )
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
            let mut inputs = vec![controls.clone()];
            inputs.extend(detectors.clone());
            inputs.extend(manifest.iter().map(|row| input_dir.join(&row.filename)));
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
            record_provenance(
                recorder,
                &inputs,
                std::slice::from_ref(&output),
                &record_path_for_file(&output),
            )
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
            clean_controls,
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
            let mut inputs = vec![controls.clone(), samples.clone()];
            inputs.extend(detectors.clone());
            inputs.extend(
                control_manifest
                    .iter()
                    .map(|row| input_dir.join(&row.filename))
                    .chain(
                        sample_manifest
                            .iter()
                            .map(|row| input_dir.join(&row.filename)),
                    ),
            );
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
                clean_controls,
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
            record_provenance(
                recorder,
                &inputs,
                &files_under(&output),
                &record_path_for_dir(&output),
            )
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
            let outputs = [output.clone(), peacoqc_masks_path(&output)];
            info!(
                "peacoqc retained {} of {} events",
                result.summary.metrics.events_after, result.summary.metrics.events_before
            );
            if let Some(debug_dir) = config.debug_dir.as_ref() {
                info!("wrote peacoqc debug artifacts to {}", debug_dir.display());
            }
            info!("peacoqc completed successfully");
            record_provenance(recorder, &[input], &outputs, &record_path_for_file(&output))
        }
        Commands::Cytoscan {
            input,
            reference,
            output,
            channels,
            channel,
            method,
            flag_method,
            debug_dir,
            threads,
            strict,
        } => {
            info!(
                "starting cytoscan: inputs={}, references={}, output={}, method={:?}, flag_method={:?}",
                input.len(),
                reference.len(),
                output.display(),
                method,
                flag_method
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
                debug_dir,
                threads,
                strict,
            };
            let result = CytoScanRunner::run(&config)?;
            for warning in &result.diagnostics.warnings {
                warn!("cytoscan: {warning}");
            }
            info!(
                "cytoscan wrote {} summary row(s) and {} feature column(s)",
                result.summary_rows.len(),
                result.test_features.feature_names.len()
            );
            if let Some(debug_dir) = config.debug_dir.as_ref() {
                info!("wrote cytoscan debug artifacts to {}", debug_dir.display());
            }
            info!("cytoscan completed successfully");
            let parameters = &result.parameters;
            let inputs = parameters
                .resolved_test_inputs
                .iter()
                .chain(&parameters.resolved_reference_inputs)
                .cloned()
                .collect::<Vec<_>>();
            record_provenance(
                recorder,
                &inputs,
                &files_under(&config.output_dir),
                &record_path_for_dir(&config.output_dir),
            )
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
            allow_retransform,
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
                allow_retransform,
            };
            let result = TransformRunner::run(&config)?;
            if result.dry_run {
                print_transform_dry_run(&result);
            }
            if let Some(debug_dir) = config.debug_dir.as_ref() {
                info!("wrote transform debug artifacts to {}", debug_dir.display());
            }
            info!("transform completed for {} file(s)", result.files.len());
            if result.dry_run {
                return Ok(());
            }
            let inputs = result
                .files
                .iter()
                .map(|file| file.input_path.clone())
                .collect::<Vec<_>>();
            let mut outputs = result
                .files
                .iter()
                .map(|file| file.output_path.clone())
                .collect::<Vec<_>>();
            outputs.extend(result.metadata_path.clone());
            let record_path = if config.output.is_dir() {
                record_path_for_dir(&config.output)
            } else {
                record_path_for_file(&config.output)
            };
            record_provenance(recorder, &inputs, &outputs, &record_path)
        }
        Commands::Gate {
            input,
            gating_ml,
            output,
            format,
            threads,
            masks,
            membership,
        } => {
            info!(
                "starting gate: input={}, gating_ml={}, output={}",
                input.display(),
                gating_ml.display(),
                output.display()
            );
            let config = GatingConfig {
                input,
                gating_ml,
                output,
                format: format.into(),
                threads,
                masks,
                membership,
            };
            let result = GatingRunner::run(&config)?;
            let mut inputs = vec![config.input.clone(), config.gating_ml.clone()];
            inputs.extend(config.masks.iter().cloned());
            let mut outputs = vec![config.output.clone()];
            outputs.extend(config.membership.clone());
            for gate in &result.gates {
                debug!(
                    "gate `{}`: {} event(s), {:.2}% of parent",
                    gate.gate_path, gate.count, gate.percent_of_parent
                );
            }
            info!(
                "gate evaluated {} gate(s) over {} event(s); wrote {}",
                result.gates.len(),
                result.event_count,
                result.output.display()
            );
            record_provenance(
                recorder,
                &inputs,
                &outputs,
                &record_path_for_file(&config.output),
            )
        }
        Commands::Compensate {
            input,
            output,
            matrix,
            matrix_kind,
            overwrite,
        } => {
            info!(
                "starting compensate: input={}, output={}",
                input.display(),
                output.display()
            );
            let config = CompensateConfig {
                input,
                output,
                matrix: matrix.zip(matrix_kind.map(CompensateMatrixKind::from)),
                overwrite,
            };
            let result = CompensateRunner::run(&config)?;
            let mut inputs = vec![config.input.clone()];
            inputs.extend(config.matrix.as_ref().map(|(path, _)| path.clone()));
            info!(
                "compensated {} channel(s) over {} event(s); {} on a raw detector margin; wrote {} and {}",
                result.channels.len(),
                result.event_count,
                result.margin_event_count,
                result.output.display(),
                result.masks.display()
            );
            record_provenance(
                recorder,
                &inputs,
                &[result.output.clone(), result.masks.clone()],
                &record_path_for_file(&result.output),
            )
        }
        Commands::Mask(MaskCommand::Combine {
            masks,
            output,
            overwrite,
        }) => {
            info!("starting mask combine: masks={}", masks.len());
            let config = MaskCombineConfig {
                masks,
                output,
                overwrite,
            };
            let combined = MaskCombineRunner::run(&config)?;
            info!(
                "mask combine kept {} of {} event(s); wrote {}",
                combined.kept(),
                combined.len(),
                config.output.display()
            );
            record_provenance(
                recorder,
                &config.masks,
                std::slice::from_ref(&config.output),
                &record_path_for_file(&config.output),
            )
        }
        Commands::Meta {
            input,
            output,
            overwrite,
        } => {
            let output = output.filter(|path| path.as_os_str() != "-");
            debug!("starting meta: input={}", input.display());
            let config = MetaConfig {
                input,
                output,
                overwrite,
            };
            // The metadata document already records the input's hash.
            drop(recorder);
            let result = MetaRunner::run(&config)?;
            debug!(
                "meta read {} TEXT keyword(s) and {} parameter(s)",
                result.metadata.text.len(),
                result.metadata.parameter_count
            );
            Ok(())
        }
        Commands::Clean(args) => {
            info!(
                "starting clean: input={}, output={}",
                args.input.display(),
                args.output.display()
            );
            let mut inputs = vec![args.input.clone()];
            inputs.extend(args.masks.iter().cloned());
            inputs.extend(args.gates.clone());
            inputs.extend(args.viability_control.clone());
            let result = CleanRunner::run(&args.into())?;
            let metrics = &result.metrics;
            for (name, report) in [
                ("debris", &metrics.filters.debris),
                ("singlets", &metrics.filters.singlets),
                ("viability", &metrics.filters.viability),
            ] {
                match report.status {
                    FilterStatus::Applied => info!(
                        "{name}: removed {} event(s)",
                        report.removed.unwrap_or_default()
                    ),
                    _ => info!(
                        "{name}: {:?} ({})",
                        report.status,
                        report.reason.as_deref().unwrap_or_default()
                    ),
                }
            }
            info!(
                "clean kept {} of {} event(s); wrote {}",
                metrics.kept,
                metrics.event_count,
                result.output.display()
            );
            record_provenance(
                recorder,
                &inputs,
                &files_under(&result.output),
                &record_path_for_dir(&result.output),
            )
        }
        Commands::Flowsom {
            input,
            output,
            channels,
            channel,
            debug_dir,
            threads,
            xdim,
            ydim,
            metaclusters,
            seed,
            no_scale,
            training_events,
            masks,
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
                xdim,
                ydim,
                metaclusters,
                seed,
                scale: !no_scale,
                training_events,
                masks,
            };
            let result = FlowSomRunner::run(&input, &output, &config)?;
            let mut inputs = vec![input.clone()];
            inputs.extend(config.masks.iter().cloned());
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
            record_provenance(
                recorder,
                &inputs,
                std::slice::from_ref(&output),
                &record_path_for_file(&output),
            )
        }
    }
}

const fn command_name(command: &Commands) -> &'static str {
    match command {
        Commands::Autospill { .. } => "autospill",
        Commands::Autospread { .. } => "autospread",
        Commands::Autospectral { .. } => "autospectral",
        Commands::Peacoqc { .. } => "peacoqc",
        Commands::Cytoscan { .. } => "cytoscan",
        Commands::Transform { .. } => "transform",
        Commands::Gate { .. } => "gate",
        Commands::Mask(MaskCommand::Combine { .. }) => "mask combine",
        Commands::Meta { .. } => "meta",
        Commands::Compensate { .. } => "compensate",
        Commands::Clean(_) => "clean",
        Commands::Flowsom { .. } => "flowsom",
    }
}

/// Write the run's `provenance.json`: tool, arguments, and the hashes of the
/// files it read and wrote.
fn record_provenance(
    recorder: RunRecorder,
    inputs: &[PathBuf],
    outputs: &[PathBuf],
    record_path: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    recorder.finish(inputs, outputs, record_path)?;
    debug!("wrote provenance record {}", record_path.display());
    Ok(())
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

fn parse_positive_usize(raw: &str) -> Result<usize, String> {
    match raw.parse::<usize>() {
        Ok(value) if value > 0 => Ok(value),
        _ => Err(format!("expected a positive integer, got `{raw}`")),
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

fn parse_finite_f64(raw: &str) -> Result<f64, String> {
    let value = raw
        .parse::<f64>()
        .map_err(|error| format!("invalid number `{raw}`: {error}"))?;
    if value.is_finite() {
        Ok(value)
    } else {
        Err("value must be finite".to_owned())
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
