#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use cytiq_fcs::{
    ColumnData, FcsDataset, FcsReader, FcsWriteParameter, MARGINS_KEYWORD, Parameter, SignalState,
    UNMIXING_KEYWORD, UnmixingState,
};
use cytiq_mask::EventMask;
use log::{debug, info, warn};
use nalgebra::{DMatrix, DVector};
use rayon::{ThreadPoolBuilder, prelude::*};
use serde::Serialize;

use crate::error::AutoSpectralError;
use crate::export::write_outputs;
use crate::manifest::{
    ControlManifestRow, DetectorSpec, SampleManifestRow, is_autofluorescence_dye,
    normalize_identifier,
};

pub const SOLVER: &str = "ols";
/// How the OLS solution is computed: a Householder QR solve, which avoids the
/// squared condition number of the normal equations `(AᵀA)⁻¹Aᵀ`.
pub const SOLVER_METHOD: &str = "householder_qr";
pub const AF_COMPONENT_COUNT: usize = 1;
pub const DEFAULT_FSC_CHANNEL: &str = "FSC-A";
pub const DEFAULT_SSC_CHANNEL: &str = "SSC-A";
pub const SCATTER_GATE_LOWER_QUANTILE: f64 = 0.01;
pub const SCATTER_GATE_UPPER_QUANTILE: f64 = 0.99;
/// `asp$positivity.threshold`: positives lie above this quantile of the
/// negative (autofluorescence) control's primary channel.
pub const POSITIVITY_THRESHOLD: f64 = 0.99;
/// `asp$positive.n`: the brightest positives used for each spectrum.
pub const POSITIVE_EVENT_LIMIT: usize = 500;
pub const MIN_RETAINED_EVENTS: usize = 20;
/// `asp$min.cell.stop.n`.
pub const MIN_POSITIVE_EVENTS: usize = 20;
pub const CONDITION_NUMBER_LIMIT: f64 = 1.0e8;
/// Never inferred as detectors: acquisition time and the event lineage.
pub const AUTO_EXCLUDED_CHANNELS: &[&str] = &["Time", cytiq_fcs::ORIGINAL_ID_PARAMETER];

const SPECTRUM_EPSILON: f64 = 1e-12;
const UNMIX_EVENT_CHUNK_SIZE: usize = 8_192;

#[derive(Clone, Debug, PartialEq)]
pub struct AutoSpectralConfig {
    pub fsc: String,
    pub ssc: String,
    pub detectors: Vec<DetectorSpec>,
    pub output_dir: PathBuf,
    pub debug_dir: Option<PathBuf>,
    pub threads: Option<usize>,
    /// Clean controls before building spectra: drop events on raw detector
    /// margins and gate FSC/SSC with the `AutoSpill` KDE gate instead of the
    /// 1-99% quantile box.
    pub clean_controls: bool,
}

/// Control gate names recorded in `run.json`.
const CONTROL_GATE_QUANTILE: &str = "fsc_ssc_quantile_box";
const CONTROL_GATE_CLEAN: &str = "detector_margins+autospill_kde_gate";

impl AutoSpectralConfig {
    #[must_use]
    pub fn new(output_dir: PathBuf) -> Self {
        Self {
            fsc: DEFAULT_FSC_CHANNEL.to_owned(),
            ssc: DEFAULT_SSC_CHANNEL.to_owned(),
            detectors: Vec::new(),
            output_dir,
            debug_dir: None,
            threads: None,
            clean_controls: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct AutoSpectralSummary {
    pub run_id: String,
    pub solver: String,
    pub solver_method: String,
    /// How control events were selected before building spectra.
    pub control_gate: String,
    pub spectral_channels: Vec<String>,
    pub components: Vec<String>,
    pub condition_number: f64,
    pub controls: Vec<ControlQc>,
    pub samples: Vec<SampleQc>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ControlQc {
    pub dye: String,
    pub filename: String,
    pub primary_channel: Option<String>,
    pub total_events: usize,
    pub retained_events: usize,
    pub positive_events: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SampleQc {
    pub filename: String,
    pub events_processed: usize,
    pub residual_mean: f64,
    pub residual_median: f64,
    pub residual_p95: f64,
    pub negative_fraction: f64,
    pub output_path: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SpectralReferenceSet {
    pub detectors: Vec<String>,
    pub components: Vec<SpectralComponent>,
    pub condition_number: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SpectralComponent {
    pub name: String,
    pub kind: ComponentKind,
    pub spectrum: Vec<f64>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ComponentKind {
    Fluorophore,
    Autofluorescence,
}

#[derive(Clone, Debug)]
pub struct AutoSpectralResult {
    pub references: SpectralReferenceSet,
    pub samples: Vec<UnmixedSample>,
    pub summary: AutoSpectralSummary,
}

#[derive(Clone, Debug)]
pub struct UnmixedSample {
    pub output_path: PathBuf,
    pub parameters: Vec<FcsWriteParameter>,
    /// Passed-through columns hold their exact source values; unmixed
    /// components are rounded to f32.
    pub columns: Vec<Vec<f64>>,
    pub text_keywords: Vec<(String, String)>,
    /// Raw detector-margin flags per event (`margin_pass`), written next to
    /// the unmixed FCS.
    pub mask: EventMask,
}

pub struct AutoSpectralRunner;

#[derive(Clone, Debug)]
struct LoadedControl {
    row: ControlManifestRow,
    path: PathBuf,
    dataset: FcsDataset,
    primary_channel: Option<String>,
    is_autofluorescence: bool,
}

#[derive(Clone, Debug)]
struct LoadedSample {
    row: SampleManifestRow,
    path: PathBuf,
    dataset: FcsDataset,
}

#[derive(Clone, Debug)]
struct BuiltReferences {
    set: SpectralReferenceSet,
    matrix: DMatrix<f64>,
    unmixing: DMatrix<f64>,
    controls: Vec<ControlQc>,
}

#[derive(Clone, Debug)]
struct BuiltControlReference {
    component: SpectralComponent,
    control_qc: ControlQc,
}

#[derive(Clone, Debug)]
struct UnmixedChunk {
    component_columns: Vec<Vec<f32>>,
    residuals: Vec<f64>,
    negative_count: usize,
}

impl AutoSpectralRunner {
    /// Run the `AutoSpectral` MVP pipeline and write all configured outputs.
    ///
    /// # Errors
    ///
    /// Returns an error if FCS files cannot be loaded, references cannot be
    /// built, samples cannot be unmixed, or outputs cannot be written.
    pub fn run(
        controls: &[ControlManifestRow],
        samples: &[SampleManifestRow],
        input_dir: impl AsRef<Path>,
        config: &AutoSpectralConfig,
    ) -> Result<AutoSpectralSummary, AutoSpectralError> {
        let input_dir = input_dir.as_ref();
        with_thread_pool(config.threads, || {
            Self::run_inner(controls, samples, input_dir, config)
        })
    }

    fn run_inner(
        controls: &[ControlManifestRow],
        samples: &[SampleManifestRow],
        input_dir: &Path,
        config: &AutoSpectralConfig,
    ) -> Result<AutoSpectralSummary, AutoSpectralError> {
        info!(
            "autospectral pipeline: loading {} controls and {} samples from {}",
            controls.len(),
            samples.len(),
            input_dir.display()
        );
        let loaded_controls = load_controls(controls, input_dir)?;
        let loaded_samples = load_samples(samples, input_dir)?;
        let detectors = resolve_detectors(&loaded_controls, &loaded_samples, config)?;
        info!(
            "autospectral pipeline: using {} spectral detectors",
            detectors.len()
        );
        let references = build_references(&loaded_controls, &detectors, config)?;
        let output_paths = sample_output_paths(samples, &config.output_dir);
        let unmixed_samples = unmix_samples(
            &loaded_samples,
            &detectors,
            &references.set.components,
            &references.matrix,
            &references.unmixing,
            &output_paths,
        )?;
        let sample_qc = unmixed_samples
            .iter()
            .map(|sample| sample.summary.clone())
            .collect::<Vec<_>>();
        let summary = AutoSpectralSummary {
            run_id: run_id(),
            solver: SOLVER.to_owned(),
            solver_method: SOLVER_METHOD.to_owned(),
            control_gate: if config.clean_controls {
                CONTROL_GATE_CLEAN
            } else {
                CONTROL_GATE_QUANTILE
            }
            .to_owned(),
            spectral_channels: detectors,
            components: references
                .set
                .components
                .iter()
                .map(|component| component.name.clone())
                .collect(),
            condition_number: references.set.condition_number,
            controls: references.controls,
            samples: sample_qc,
        };
        let result = AutoSpectralResult {
            references: references.set,
            samples: unmixed_samples
                .into_iter()
                .map(|sample| UnmixedSample {
                    output_path: sample.summary.output_path,
                    parameters: sample.parameters,
                    columns: sample.columns,
                    text_keywords: sample.text_keywords,
                    mask: sample.mask,
                })
                .collect(),
            summary,
        };
        write_outputs(&result, &config.output_dir, config.debug_dir.as_deref())?;
        info!("autospectral pipeline: completed successfully");
        Ok(result.summary)
    }
}

#[derive(Clone, Debug)]
struct WorkingUnmixedSample {
    parameters: Vec<FcsWriteParameter>,
    columns: Vec<Vec<f64>>,
    text_keywords: Vec<(String, String)>,
    mask: EventMask,
    summary: SampleQc,
}

fn load_controls(
    rows: &[ControlManifestRow],
    input_dir: &Path,
) -> Result<Vec<LoadedControl>, AutoSpectralError> {
    parallel_try_map(rows, "control loading", |_, row| {
        let path = input_dir.join(&row.filename);
        let dataset = load_dataset(&path)?;
        let is_autofluorescence = is_autofluorescence_dye(&row.dye);
        let primary_channel = if is_autofluorescence {
            None
        } else if let Some(primary_channel) = row.primary_channel.as_deref() {
            Some(primary_channel.trim().to_owned())
        } else {
            Some(infer_primary_channel(&row.dye, &path, &dataset.parameters)?)
        };
        Ok(LoadedControl {
            row: row.clone(),
            path,
            dataset,
            primary_channel,
            is_autofluorescence,
        })
    })
}

fn load_samples(
    rows: &[SampleManifestRow],
    input_dir: &Path,
) -> Result<Vec<LoadedSample>, AutoSpectralError> {
    parallel_try_map(rows, "sample loading", |_, row| {
        let path = input_dir.join(&row.filename);
        let dataset = load_dataset(&path)?;
        Ok(LoadedSample {
            row: row.clone(),
            path,
            dataset,
        })
    })
}

fn load_dataset(path: &Path) -> Result<FcsDataset, AutoSpectralError> {
    if !path.is_file() {
        return Err(AutoSpectralError::MissingFcsFile {
            path: path.to_path_buf(),
        });
    }
    debug!("loading FCS file {}", path.display());
    let dataset = FcsReader::open(path)
        .and_then(FcsReader::read)
        .map_err(|source| AutoSpectralError::FcsRead {
            path: path.to_path_buf(),
            source,
        })?;
    if event_count(&dataset) == 0 {
        return Err(AutoSpectralError::EmptyFcs {
            path: path.to_path_buf(),
        });
    }
    // Unmixing fits raw detector values, and the output keeps no earlier
    // `CYTIQ_*` state, so a processed file would lose its history silently.
    let state = SignalState::of(&dataset.text).map_err(|source| AutoSpectralError::FcsRead {
        path: path.to_path_buf(),
        source,
    })?;
    if !state.is_raw() {
        return Err(AutoSpectralError::ProcessedInput {
            path: path.to_path_buf(),
            state: state.to_string(),
        });
    }

    Ok(dataset)
}

fn resolve_detectors(
    controls: &[LoadedControl],
    samples: &[LoadedSample],
    config: &AutoSpectralConfig,
) -> Result<Vec<String>, AutoSpectralError> {
    let detectors = if config.detectors.is_empty() {
        infer_detectors(controls, samples, config)?
    } else {
        config
            .detectors
            .iter()
            .map(|detector| detector.channel.clone())
            .collect::<Vec<_>>()
    };
    if let Some(channel) = detectors
        .iter()
        .find(|channel| channel.as_str() == cytiq_fcs::ORIGINAL_ID_PARAMETER)
    {
        return Err(AutoSpectralError::ReservedDetectorChannel {
            value: channel.clone(),
        });
    }

    if detectors.is_empty() {
        return Err(AutoSpectralError::DetectorInferenceFailed);
    }
    for control in controls {
        validate_detector_presence(&control.path, &control.dataset, &detectors)?;
    }
    for sample in samples {
        validate_detector_presence(&sample.path, &sample.dataset, &detectors)?;
    }

    Ok(detectors)
}

fn infer_detectors(
    controls: &[LoadedControl],
    samples: &[LoadedSample],
    config: &AutoSpectralConfig,
) -> Result<Vec<String>, AutoSpectralError> {
    let Some(first) = controls.first() else {
        return Err(AutoSpectralError::DetectorInferenceFailed);
    };
    let all_datasets = controls
        .iter()
        .map(|control| (&control.path, &control.dataset))
        .chain(samples.iter().map(|sample| (&sample.path, &sample.dataset)))
        .collect::<Vec<_>>();
    let detectors = first
        .dataset
        .parameters
        .iter()
        .filter(|parameter| !is_auto_excluded_channel(&parameter.name, config))
        .filter(|parameter| {
            all_datasets.iter().all(|(_, dataset)| {
                find_parameter_index(&dataset.parameters, &parameter.name).is_some()
            })
        })
        .map(|parameter| parameter.name.clone())
        .collect::<Vec<_>>();

    if detectors.is_empty() {
        return Err(AutoSpectralError::DetectorInferenceFailed);
    }

    Ok(detectors)
}

fn validate_detector_presence(
    path: &Path,
    dataset: &FcsDataset,
    detectors: &[String],
) -> Result<(), AutoSpectralError> {
    for detector in detectors {
        if find_parameter_index(&dataset.parameters, detector).is_none() {
            return Err(AutoSpectralError::MissingDetectorChannel {
                path: path.to_path_buf(),
                channel: detector.clone(),
            });
        }
    }

    Ok(())
}

fn build_references(
    controls: &[LoadedControl],
    detectors: &[String],
    config: &AutoSpectralConfig,
) -> Result<BuiltReferences, AutoSpectralError> {
    let component_count = controls.len();
    if detectors.len() < component_count {
        return Err(AutoSpectralError::TooFewDetectors {
            detectors: detectors.len(),
            components: component_count,
        });
    }

    let autofluorescence = controls
        .iter()
        .find(|control| control.is_autofluorescence)
        .ok_or(AutoSpectralError::MissingAutofluorescenceControl)?;
    let af_columns =
        detector_columns(&autofluorescence.path, &autofluorescence.dataset, detectors)?;
    let af_gate = retained_indices(
        &autofluorescence.path,
        &autofluorescence.dataset,
        detectors,
        &af_columns,
        config,
    )?;
    let af_raw = mean_spectrum(&af_columns, &af_gate);
    let af_spectrum = normalize_spectrum(&af_raw, &autofluorescence.path, PeakNorm::Absolute)?;

    let single_stain_controls = controls
        .iter()
        .filter(|control| !control.is_autofluorescence)
        .collect::<Vec<_>>();
    let built_single_stains = parallel_try_map(
        &single_stain_controls,
        "single-stain reference spectra",
        |_, control| {
            build_single_stain_reference(control, detectors, config, &af_raw, &af_columns, &af_gate)
        },
    )?;
    let mut components = built_single_stains
        .iter()
        .map(|reference| reference.component.clone())
        .collect::<Vec<_>>();
    let mut controls_qc = built_single_stains
        .iter()
        .map(|reference| reference.control_qc.clone())
        .collect::<Vec<_>>();
    controls_qc.push(ControlQc {
        dye: autofluorescence.row.dye.clone(),
        filename: autofluorescence.row.filename.clone(),
        primary_channel: None,
        total_events: event_count(&autofluorescence.dataset),
        retained_events: af_gate.len(),
        positive_events: None,
    });
    components.push(SpectralComponent {
        name: "Autofluorescence".to_owned(),
        kind: ComponentKind::Autofluorescence,
        spectrum: af_spectrum,
    });

    // calculate.condition.number: fluorophores only, warn when kappa > n.
    let fluorophore_count = components.len() - AF_COMPONENT_COUNT;
    let fluorophore_matrix = reference_matrix(detectors.len(), &components[..fluorophore_count]);
    let fluorophore_condition = condition_number(&fluorophore_matrix)?;
    let matrix = reference_matrix(detectors.len(), &components);
    let condition_number = condition_number(&matrix)?;
    if fluorophore_condition > fluorophore_count as f64 {
        warn!(
            "fluorophore mixing matrix condition number {fluorophore_condition:.2} exceeds the number of fluorophores ({fluorophore_count})"
        );
    }
    if condition_number > CONDITION_NUMBER_LIMIT {
        return Err(AutoSpectralError::IllConditionedReferenceMatrix {
            condition: condition_number,
            limit: CONDITION_NUMBER_LIMIT,
        });
    }
    let unmixing = unmixing_matrix(&matrix)?;

    Ok(BuiltReferences {
        set: SpectralReferenceSet {
            detectors: detectors.to_vec(),
            components,
            condition_number,
        },
        matrix,
        unmixing,
        controls: controls_qc,
    })
}

fn build_single_stain_reference(
    control: &LoadedControl,
    detectors: &[String],
    config: &AutoSpectralConfig,
    af_raw: &[f64],
    af_columns: &[Vec<f64>],
    af_gate: &[usize],
) -> Result<BuiltControlReference, AutoSpectralError> {
    let primary_channel = control
        .primary_channel
        .as_deref()
        .expect("single-stain controls have primary channels");
    let primary_detector_index = detectors
        .iter()
        .position(|channel| channel == primary_channel)
        .ok_or_else(|| AutoSpectralError::DetectorMismatch {
            path: control.path.clone(),
            channel: primary_channel.to_owned(),
        })?;
    let columns = detector_columns(&control.path, &control.dataset, detectors)?;
    let retained = retained_indices(&control.path, &control.dataset, detectors, &columns, config)?;
    let negative_primary = af_gate
        .iter()
        .map(|&index| af_columns[primary_detector_index][index])
        .collect::<Vec<_>>();
    let positive = positive_indices(
        &control.path,
        &columns,
        &retained,
        primary_detector_index,
        negative_primary,
    )?;
    let raw = subtract_background(&mean_spectrum(&columns, &positive), af_raw);
    let spectrum = normalize_spectrum(&raw, &control.path, PeakNorm::Signed)?;

    Ok(BuiltControlReference {
        component: SpectralComponent {
            name: control.row.dye.clone(),
            kind: ComponentKind::Fluorophore,
            spectrum,
        },
        control_qc: ControlQc {
            dye: control.row.dye.clone(),
            filename: control.row.filename.clone(),
            primary_channel: Some(primary_channel.to_owned()),
            total_events: event_count(&control.dataset),
            retained_events: retained.len(),
            positive_events: Some(positive.len()),
        },
    })
}

fn retained_indices(
    path: &Path,
    dataset: &FcsDataset,
    detectors: &[String],
    detector_columns: &[Vec<f64>],
    config: &AutoSpectralConfig,
) -> Result<Vec<usize>, AutoSpectralError> {
    let fsc = channel_column(path, dataset, &config.fsc)?;
    let ssc = channel_column(path, dataset, &config.ssc)?;
    let mut valid = (0..event_count(dataset))
        .filter(|&index| {
            fsc[index].is_finite()
                && ssc[index].is_finite()
                && detector_columns
                    .iter()
                    .all(|column| column[index].is_finite())
        })
        .collect::<Vec<_>>();
    let retained = if config.clean_controls {
        clean_control_indices(path, dataset, detectors, &fsc, &ssc, valid, config)?
    } else {
        let fsc_values = valid.iter().map(|&index| fsc[index]).collect::<Vec<_>>();
        let ssc_values = valid.iter().map(|&index| ssc[index]).collect::<Vec<_>>();
        let fsc_low = quantile(fsc_values.clone(), SCATTER_GATE_LOWER_QUANTILE);
        let fsc_high = quantile(fsc_values, SCATTER_GATE_UPPER_QUANTILE);
        let ssc_low = quantile(ssc_values.clone(), SCATTER_GATE_LOWER_QUANTILE);
        let ssc_high = quantile(ssc_values, SCATTER_GATE_UPPER_QUANTILE);
        valid.retain(|&index| {
            (fsc_low..=fsc_high).contains(&fsc[index]) && (ssc_low..=ssc_high).contains(&ssc[index])
        });
        valid
    };
    if retained.len() < MIN_RETAINED_EVENTS {
        return Err(AutoSpectralError::TooFewRetainedEvents {
            path: path.to_path_buf(),
            retained: retained.len(),
            minimum: MIN_RETAINED_EVENTS,
        });
    }

    Ok(retained)
}

/// `--clean-controls`: drop events on a raw detector margin, then keep the
/// dense FSC/SSC core found by the `AutoSpill` KDE gate.
fn clean_control_indices(
    path: &Path,
    dataset: &FcsDataset,
    detectors: &[String],
    fsc: &[f64],
    ssc: &[f64],
    mut valid: Vec<usize>,
    config: &AutoSpectralConfig,
) -> Result<Vec<usize>, AutoSpectralError> {
    let detector_indices = detectors
        .iter()
        .filter_map(|detector| dataset.find_parameter(detector))
        .collect::<Vec<_>>();
    let margin_pass = cytiq_fcs::raw_margin_pass(dataset, &detector_indices).map_err(|source| {
        AutoSpectralError::FcsRead {
            path: path.to_path_buf(),
            source,
        }
    })?;
    valid.retain(|&index| margin_pass[index]);
    let points = valid
        .iter()
        .map(|&index| [fsc[index], ssc[index]])
        .collect::<Vec<_>>();
    // AutoSpill clamps scatter to the FSC range of the file.
    let data_max = find_parameter_index(&dataset.parameters, &config.fsc).and_then(|index| {
        let parameter = &dataset.parameters[index];
        let key = format!("$P{}R", parameter.index);
        parameter
            .keywords
            .iter()
            .find_map(|(candidate, value)| (*candidate == key).then_some(value))
            .and_then(|value| value.trim().parse::<f64>().ok())
    });
    let gate = cytiq_density::gate_events(&points, data_max).map_err(|reason| {
        AutoSpectralError::DegenerateScatterGate {
            path: path.to_path_buf(),
            reason,
        }
    })?;
    Ok(gate
        .gated_indices
        .into_iter()
        .map(|position| valid[position])
        .collect())
}

/// `get.universal.negative`: events above the 99th percentile of the negative
/// control's primary channel, keeping the brightest `positive.n` (500).
fn positive_indices(
    path: &Path,
    detector_columns: &[Vec<f64>],
    retained: &[usize],
    primary_detector_index: usize,
    negative_primary_values: Vec<f64>,
) -> Result<Vec<usize>, AutoSpectralError> {
    let primary = &detector_columns[primary_detector_index];
    let threshold = quantile(negative_primary_values, POSITIVITY_THRESHOLD);
    let mut positive = retained
        .iter()
        .copied()
        .filter(|&index| primary[index] > threshold)
        .collect::<Vec<_>>();
    positive.sort_by(|left, right| primary[*right].total_cmp(&primary[*left]));
    positive.truncate(POSITIVE_EVENT_LIMIT);
    positive.sort_unstable();
    if positive.len() < MIN_POSITIVE_EVENTS {
        return Err(AutoSpectralError::TooFewPositiveEvents {
            path: path.to_path_buf(),
            positive: positive.len(),
            minimum: MIN_POSITIVE_EVENTS,
        });
    }

    Ok(positive)
}

fn unmix_samples(
    samples: &[LoadedSample],
    detectors: &[String],
    components: &[SpectralComponent],
    reference_matrix: &DMatrix<f64>,
    unmixing: &DMatrix<f64>,
    output_paths: &[PathBuf],
) -> Result<Vec<WorkingUnmixedSample>, AutoSpectralError> {
    parallel_try_map(samples, "sample unmixing", |index, sample| {
        unmix_sample(
            sample,
            detectors,
            components,
            reference_matrix,
            unmixing,
            &output_paths[index],
        )
    })
}

fn unmix_sample(
    sample: &LoadedSample,
    detectors: &[String],
    components: &[SpectralComponent],
    reference_matrix: &DMatrix<f64>,
    unmixing: &DMatrix<f64>,
    output_path: &Path,
) -> Result<WorkingUnmixedSample, AutoSpectralError> {
    let detector_columns = detector_columns(&sample.path, &sample.dataset, detectors)?;
    if detector_columns
        .iter()
        .flat_map(|column| column.iter())
        .any(|value| !value.is_finite())
    {
        return Err(AutoSpectralError::NonFiniteSampleValues {
            path: sample.path.clone(),
        });
    }

    let event_total = event_count(&sample.dataset);
    let ranges = event_ranges(event_total);
    let chunks = ranges
        .par_iter()
        .map(|&(start, end)| {
            unmix_event_range(
                &detector_columns,
                components.len(),
                detectors.len(),
                reference_matrix,
                unmixing,
                start,
                end,
            )
        })
        .collect::<Vec<_>>();
    let (component_columns, residuals, negative_count) =
        merge_unmixed_chunks(chunks, components.len(), event_total);

    let read_error = |source| AutoSpectralError::FcsRead {
        path: sample.path.clone(),
        source,
    };
    // Unmixing mixes the detectors: judge raw detector margins now.
    let detector_indices = detectors
        .iter()
        .filter_map(|detector| sample.dataset.find_parameter(detector))
        .collect::<Vec<_>>();
    let margin_pass =
        cytiq_fcs::raw_margin_pass(&sample.dataset, &detector_indices).map_err(read_error)?;
    let acquisition =
        cytiq_fcs::acquisition_id(&sample.path, &sample.dataset.text).map_err(read_error)?;
    let mask = EventMask::new(
        acquisition.clone(),
        margin_pass.clone(),
        cytiq_fcs::source_event_ids(&sample.dataset).map_err(read_error)?,
    )
    .with_filter("margin_pass", margin_pass.into_iter().map(Some).collect());
    let mut text_keywords =
        output_keywords(&sample.dataset, detectors, components).map_err(read_error)?;
    cytiq_fcs::set_acquisition(&mut text_keywords, &acquisition);

    let (mut parameters, mut columns) = preserved_columns(&sample.dataset, detectors);
    for (component, column) in components.iter().zip(component_columns) {
        parameters.push(FcsWriteParameter::new(component.name.clone()));
        columns.push(column.into_iter().map(f64::from).collect());
    }
    let negative_fraction = if event_total == 0 || components.is_empty() {
        0.0
    } else {
        negative_count as f64 / ((event_total * components.len()) as f64)
    };
    Ok(WorkingUnmixedSample {
        parameters,
        columns,
        text_keywords,
        mask,
        summary: SampleQc {
            filename: sample.row.filename.clone(),
            events_processed: event_total,
            residual_mean: mean(&residuals),
            residual_median: quantile(residuals.clone(), 0.5),
            residual_p95: quantile(residuals, 0.95),
            negative_fraction,
            output_path: output_path.to_path_buf(),
        },
    })
}

fn unmix_event_range(
    detector_columns: &[Vec<f64>],
    component_count: usize,
    detector_count: usize,
    reference_matrix: &DMatrix<f64>,
    unmixing: &DMatrix<f64>,
    start: usize,
    end: usize,
) -> UnmixedChunk {
    let chunk_len = end - start;
    let mut component_columns = vec![Vec::with_capacity(chunk_len); component_count];
    let mut residuals = Vec::with_capacity(chunk_len);
    let mut negative_count = 0_usize;

    for row_index in start..end {
        let y = DVector::from_iterator(
            detector_count,
            detector_columns.iter().map(|column| column[row_index]),
        );
        let x = unmixing * &y;
        let residual = (reference_matrix * &x - y).norm();
        residuals.push(residual);
        for (component_index, value) in x.iter().enumerate() {
            if *value < 0.0 {
                negative_count += 1;
            }
            component_columns[component_index].push(*value as f32);
        }
    }

    UnmixedChunk {
        component_columns,
        residuals,
        negative_count,
    }
}

fn merge_unmixed_chunks(
    chunks: Vec<UnmixedChunk>,
    component_count: usize,
    event_total: usize,
) -> (Vec<Vec<f32>>, Vec<f64>, usize) {
    let mut component_columns = vec![Vec::with_capacity(event_total); component_count];
    let mut residuals = Vec::with_capacity(event_total);
    let mut negative_count = 0_usize;

    for chunk in chunks {
        for (target, source) in component_columns.iter_mut().zip(chunk.component_columns) {
            target.extend(source);
        }
        residuals.extend(chunk.residuals);
        negative_count += chunk.negative_count;
    }

    (component_columns, residuals, negative_count)
}

fn event_ranges(event_total: usize) -> Vec<(usize, usize)> {
    (0..event_total)
        .step_by(UNMIX_EVENT_CHUNK_SIZE)
        .map(|start| (start, (start + UNMIX_EVENT_CHUNK_SIZE).min(event_total)))
        .collect()
}

fn sample_output_paths(samples: &[SampleManifestRow], output_dir: &Path) -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    samples
        .iter()
        .enumerate()
        .map(|(index, sample)| {
            let stem = Path::new(&sample.filename)
                .file_stem()
                .and_then(|value| value.to_str())
                .filter(|value| !value.trim().is_empty())
                .map_or_else(|| format!("sample_{}", index + 1), ToOwned::to_owned);
            let mut candidate = format!("{stem}.unmixed.fcs");
            if !seen.insert(candidate.clone()) {
                candidate = format!("{}-{}.unmixed.fcs", stem, index + 1);
                seen.insert(candidate.clone());
            }
            output_dir.join("unmixed").join(candidate)
        })
        .collect()
}

fn preserved_columns(
    dataset: &FcsDataset,
    detectors: &[String],
) -> (Vec<FcsWriteParameter>, Vec<Vec<f64>>) {
    let detector_set = detectors.iter().collect::<HashSet<_>>();
    let mut parameters = Vec::new();
    let mut columns = Vec::new();
    for (index, parameter) in dataset.parameters.iter().enumerate() {
        if detector_set.contains(&parameter.name) {
            continue;
        }
        parameters.push(FcsWriteParameter::from_source(parameter));
        columns.push(column_to_f64(
            dataset
                .data
                .column(index)
                .expect("parameter index should have data column"),
        ));
    }

    (parameters, columns)
}

/// Source keywords that still describe the unmixed file. The spillover matrix
/// and cytiq stage keywords refer to detector channels that were replaced.
/// Source keywords that still describe the unmixed file, plus
/// `CYTIQ_UNMIXING` and the margin bounds of the passed-through channels. The
/// spillover matrix and earlier `CYTIQ_*` state refer to the replaced
/// detectors.
fn output_keywords(
    dataset: &FcsDataset,
    detectors: &[String],
    components: &[SpectralComponent],
) -> Result<Vec<(String, String)>, cytiq_fcs::Error> {
    let mut keywords = cytiq_fcs::carried_keywords(&dataset.text)
        .into_iter()
        .filter(|(key, _)| !cytiq_fcs::is_spillover_keyword(key) && !key.starts_with("CYTIQ_"))
        .collect::<Vec<_>>();
    let mut bounds = cytiq_fcs::dataset_margin_bounds(&dataset.text, &dataset.parameters)?;
    bounds.retain(|name, _| !detectors.contains(name));
    keywords.push((
        MARGINS_KEYWORD.to_owned(),
        cytiq_fcs::encode_margin_bounds(&bounds),
    ));
    let state = UnmixingState {
        components: components
            .iter()
            .map(|component| component.name.clone())
            .collect(),
        detectors: detectors.to_vec(),
    };
    keywords.push((UNMIXING_KEYWORD.to_owned(), state.to_keyword_value()));
    Ok(keywords)
}

fn reference_matrix(detector_count: usize, components: &[SpectralComponent]) -> DMatrix<f64> {
    let values = components
        .iter()
        .flat_map(|component| component.spectrum.iter().copied())
        .collect::<Vec<_>>();
    DMatrix::from_column_slice(detector_count, components.len(), &values)
}

fn condition_number(matrix: &DMatrix<f64>) -> Result<f64, AutoSpectralError> {
    let svd = matrix.clone().svd(false, false);
    let max = svd.singular_values.iter().copied().fold(0.0, f64::max);
    let min = svd
        .singular_values
        .iter()
        .copied()
        .fold(f64::INFINITY, f64::min);
    if min <= SPECTRUM_EPSILON || !min.is_finite() || !max.is_finite() {
        return Err(AutoSpectralError::SingularReferenceMatrix);
    }

    Ok(max / min)
}

/// Least-squares unmixing matrix `A⁺ = R⁻¹Qᵀ` from the thin QR decomposition
/// `A = QR`, so that `abundances = A⁺ · detectors`. For a full-rank `A` this
/// equals `(AᵀA)⁻¹Aᵀ`, but forming `AᵀA` squares the condition number and
/// loses precision for similar spectra. (nalgebra's SVD was measured less
/// accurate than QR here.)
fn unmixing_matrix(reference_matrix: &DMatrix<f64>) -> Result<DMatrix<f64>, AutoSpectralError> {
    let qr = reference_matrix.clone().qr();
    qr.r()
        .solve_upper_triangular(&qr.q().transpose())
        .filter(|unmixing| unmixing.iter().all(|value| value.is_finite()))
        .ok_or(AutoSpectralError::SingularReferenceMatrix)
}

fn detector_columns(
    path: &Path,
    dataset: &FcsDataset,
    detectors: &[String],
) -> Result<Vec<Vec<f64>>, AutoSpectralError> {
    detectors
        .iter()
        .map(|detector| {
            let index = find_parameter_index(&dataset.parameters, detector).ok_or_else(|| {
                AutoSpectralError::MissingChannel {
                    path: path.to_path_buf(),
                    channel: detector.clone(),
                }
            })?;
            Ok(column_to_f64(
                dataset
                    .data
                    .column(index)
                    .expect("parameter index should have data column"),
            ))
        })
        .collect()
}

fn channel_column(
    path: &Path,
    dataset: &FcsDataset,
    channel: &str,
) -> Result<Vec<f64>, AutoSpectralError> {
    let index = find_parameter_index(&dataset.parameters, channel).ok_or_else(|| {
        AutoSpectralError::MissingChannel {
            path: path.to_path_buf(),
            channel: channel.to_owned(),
        }
    })?;
    Ok(column_to_f64(
        dataset
            .data
            .column(index)
            .expect("parameter index should have data column"),
    ))
}

fn mean_spectrum(columns: &[Vec<f64>], indices: &[usize]) -> Vec<f64> {
    columns
        .iter()
        .map(|column| {
            indices.iter().map(|&index| column[index]).sum::<f64>() / (indices.len() as f64)
        })
        .collect()
}

fn subtract_background(values: &[f64], background: &[f64]) -> Vec<f64> {
    values
        .iter()
        .zip(background)
        .map(|(value, background)| value - background)
        .collect()
}

#[derive(Clone, Copy, Debug)]
enum PeakNorm {
    /// `spectrum / max(spectrum)` (`get.fluorophore.spectra`).
    Signed,
    /// `spectrum / max(abs(spectrum))` (autofluorescence spectra).
    Absolute,
}

/// Peak (L-infinity) normalisation as in upstream `AutoSpectral`, so unmixed
/// abundances are expressed in units of the peak detector.
fn normalize_spectrum(
    values: &[f64],
    path: &Path,
    norm: PeakNorm,
) -> Result<Vec<f64>, AutoSpectralError> {
    let peak = match norm {
        PeakNorm::Signed => values.iter().copied().fold(f64::NEG_INFINITY, f64::max),
        PeakNorm::Absolute => values
            .iter()
            .fold(0.0_f64, |peak, value| peak.max(value.abs())),
    };
    if peak <= SPECTRUM_EPSILON || !peak.is_finite() {
        return Err(AutoSpectralError::ZeroSpectrum {
            path: path.to_path_buf(),
        });
    }

    Ok(values.iter().map(|value| value / peak).collect())
}

fn infer_primary_channel(
    dye: &str,
    path: &Path,
    parameters: &[Parameter],
) -> Result<String, AutoSpectralError> {
    let normalized_dye = normalize_identifier(dye);
    let name_matches = parameters
        .iter()
        .filter(|parameter| normalize_identifier(&parameter.name) == normalized_dye)
        .collect::<Vec<_>>();
    if let Some(channel) = resolve_unique_primary_channel_match(path, dye, &name_matches)? {
        return Ok(channel);
    }

    let label_matches = parameters
        .iter()
        .filter(|parameter| {
            parameter
                .label
                .as_deref()
                .is_some_and(|label| normalize_identifier(label) == normalized_dye)
        })
        .collect::<Vec<_>>();
    if let Some(channel) = resolve_unique_primary_channel_match(path, dye, &label_matches)? {
        return Ok(channel);
    }

    Err(AutoSpectralError::PrimaryChannelNotFound {
        path: path.to_path_buf(),
        dye: dye.to_owned(),
        available_channels: parameters
            .iter()
            .map(display_parameter_identifier)
            .collect(),
    })
}

fn resolve_unique_primary_channel_match(
    path: &Path,
    dye: &str,
    matches: &[&Parameter],
) -> Result<Option<String>, AutoSpectralError> {
    match matches {
        [] => Ok(None),
        [parameter] => Ok(Some(parameter.name.clone())),
        _ => Err(AutoSpectralError::AmbiguousPrimaryChannel {
            path: path.to_path_buf(),
            dye: dye.to_owned(),
            matches: matches
                .iter()
                .map(|parameter| display_parameter_identifier(parameter))
                .collect(),
        }),
    }
}

fn display_parameter_identifier(parameter: &Parameter) -> String {
    match parameter.label.as_deref() {
        Some(label) if label != parameter.name => format!("{} [{}]", parameter.name, label),
        _ => parameter.name.clone(),
    }
}

fn is_auto_excluded_channel(channel: &str, config: &AutoSpectralConfig) -> bool {
    let normalized = normalize_identifier(channel);
    normalized == normalize_identifier(&config.fsc)
        || normalized == normalize_identifier(&config.ssc)
        || is_scatter_channel(&normalized)
        || AUTO_EXCLUDED_CHANNELS
            .iter()
            .any(|excluded| normalized == normalize_identifier(excluded))
}

fn is_scatter_channel(normalized_channel: &str) -> bool {
    normalized_channel.starts_with("fsc") || normalized_channel.starts_with("ssc")
}

fn find_parameter_index(parameters: &[Parameter], channel: &str) -> Option<usize> {
    parameters
        .iter()
        .position(|parameter| parameter.name == channel)
}

fn parallel_try_map<T, U, E, F>(items: &[T], stage: &str, worker: F) -> Result<Vec<U>, E>
where
    T: Sync,
    U: Send,
    E: Send,
    F: Fn(usize, &T) -> Result<U, E> + Sync + Send,
{
    if items.len() < 2 {
        return items
            .iter()
            .enumerate()
            .map(|(index, item)| worker(index, item))
            .collect();
    }

    debug!("parallelizing {stage} with rayon for {} tasks", items.len());
    items
        .par_iter()
        .enumerate()
        .map(|(index, item)| worker(index, item))
        .collect::<Vec<_>>()
        .into_iter()
        .collect()
}

fn with_thread_pool<T, F>(threads: Option<usize>, operation: F) -> Result<T, AutoSpectralError>
where
    T: Send,
    F: FnOnce() -> Result<T, AutoSpectralError> + Send,
{
    let Some(threads) = threads else {
        return operation();
    };

    debug!("using local rayon thread pool with {threads} threads");
    ThreadPoolBuilder::new()
        .num_threads(threads)
        .build()
        .map_err(|source| AutoSpectralError::ThreadPoolBuild { threads, source })?
        .install(operation)
}

fn event_count(dataset: &FcsDataset) -> usize {
    dataset.data.columns.first().map_or(0, ColumnData::len)
}

fn column_to_f64(column: &ColumnData) -> Vec<f64> {
    match column {
        ColumnData::UInt(values) => values.iter().map(|value| *value as f64).collect(),
        ColumnData::F32(values) => values.iter().map(|value| f64::from(*value)).collect(),
        ColumnData::F64(values) => values.clone(),
    }
}

/// R's default (type 7) quantile.
fn quantile(mut values: Vec<f64>, q: f64) -> f64 {
    values.sort_by(f64::total_cmp);
    if values.is_empty() {
        return 0.0;
    }
    let position = (values.len() - 1) as f64 * q;
    let lower = position.floor() as usize;
    let upper = position.ceil() as usize;
    values[lower] + (position - lower as f64) * (values[upper] - values[lower])
}

fn mean(values: &[f64]) -> f64 {
    if values.is_empty() {
        0.0
    } else {
        values.iter().sum::<f64>() / (values.len() as f64)
    }
}

fn run_id() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs());
    format!("autospectral-{seconds}")
}

#[cfg(test)]
mod tests {
    use nalgebra::DMatrix;

    use super::{AutoSpectralConfig, ComponentKind, SpectralComponent, parallel_try_map};

    #[test]
    fn reference_matrix_uses_components_as_columns() {
        let components = vec![
            SpectralComponent {
                name: "FITC".to_owned(),
                kind: ComponentKind::Fluorophore,
                spectrum: vec![1.0, 0.0, 0.0],
            },
            SpectralComponent {
                name: "PE".to_owned(),
                kind: ComponentKind::Fluorophore,
                spectrum: vec![0.0, 1.0, 0.0],
            },
        ];
        let matrix = super::reference_matrix(3, &components);
        assert_eq!(
            matrix,
            DMatrix::from_row_slice(3, 2, &[1.0, 0.0, 0.0, 1.0, 0.0, 0.0])
        );
    }

    #[test]
    fn qr_solver_matches_normal_equations_when_well_conditioned() {
        let reference = DMatrix::from_row_slice(
            4,
            3,
            &[1.0, 0.1, 0.2, 0.3, 1.0, 0.2, 0.0, 0.4, 1.0, 0.2, 0.1, 0.3],
        );
        let normal = (reference.transpose() * &reference)
            .try_inverse()
            .expect("gram should invert")
            * reference.transpose();
        let qr = super::unmixing_matrix(&reference).expect("matrix should invert");
        assert!((normal - qr).abs().max() < 1e-12);
    }

    #[test]
    fn qr_solver_keeps_precision_for_similar_spectra() {
        // Two spectra that differ by 1e-6: κ ≈ 2.5e6. The normal equations
        // work with κ² and recover the coefficients to ~5e-5; QR to ~1e-9.
        let reference = DMatrix::from_row_slice(3, 2, &[1.0, 1.0, 0.5, 0.5 + 1.0e-6, 0.25, 0.25]);
        let coefficients = nalgebra::DVector::from_vec(vec![3.0, 7.0]);
        let observed = &reference * &coefficients;
        let recovered =
            super::unmixing_matrix(&reference).expect("matrix should invert") * observed;
        for (actual, expected) in recovered.iter().zip(coefficients.iter()) {
            assert!((actual - expected).abs() < 1e-7, "{actual} vs {expected}");
        }
    }

    #[test]
    fn ols_solver_recovers_known_coefficients() {
        let reference =
            DMatrix::from_row_slice(3, 3, &[1.0, 0.1, 0.2, 0.0, 1.0, 0.2, 0.0, 0.1, 1.0]);
        let unmixing = super::unmixing_matrix(&reference).expect("matrix should invert");
        let coefficients = nalgebra::DVector::from_vec(vec![10.0, 20.0, 3.0]);
        let observed = &reference * &coefficients;
        let recovered = unmixing * observed;
        for (actual, expected) in recovered.iter().zip(coefficients.iter()) {
            assert!((actual - expected).abs() < 1e-9);
        }
    }

    #[test]
    fn detector_inference_excludes_scatter_channel_variants() {
        let config = AutoSpectralConfig::new("out".into());

        assert!(super::is_auto_excluded_channel("FSC-A", &config));
        assert!(super::is_auto_excluded_channel("FSC-H", &config));
        assert!(super::is_auto_excluded_channel("SSC-A", &config));
        assert!(super::is_auto_excluded_channel("SSC-B-A", &config));
        assert!(super::is_auto_excluded_channel("SSC-B-H", &config));
        assert!(!super::is_auto_excluded_channel("R1-A", &config));
        assert!(!super::is_auto_excluded_channel("YG9-A", &config));
    }

    #[test]
    fn parallel_try_map_preserves_input_order() {
        let values = [7, 3, 9, 1];
        let mapped = parallel_try_map(&values, "test", |index, value| {
            Ok::<_, &'static str>(format!("{index}:{value}"))
        })
        .expect("parallel map should succeed");

        assert_eq!(mapped, vec!["0:7", "1:3", "2:9", "3:1"]);
    }

    #[test]
    fn parallel_try_map_collects_errors_in_input_order() {
        let values = [0, 1, 2];
        let error = parallel_try_map(&values, "test", |index, value| {
            if *value == 1 {
                Err("expected error")
            } else if index == 2 {
                Err("later error")
            } else {
                Ok(*value)
            }
        })
        .expect_err("parallel map should surface the first ordered error");

        assert_eq!(error, "expected error");
    }
}
