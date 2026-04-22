use std::collections::BTreeSet;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use arrow_array::{
    ArrayRef, BooleanArray, Float64Array, Int64Array, RecordBatch, StringArray, UInt8Array,
};
use arrow_schema::{DataType, Field, Schema};
use cytiq_fcs::{ColumnData, FcsReader, Parameter};
use ndarray::{Array2, Array4};
use num_traits::ToPrimitive;
use onnx_protobuf::{
    AttributeProto, GraphProto, Message, ModelProto, SparseTensorProto, StringStringEntryProto,
    TensorProto, tensor_proto,
};
use ort::value::TensorRef;
use parquet::arrow::ArrowWriter;
use thiserror::Error;

const GRID_SIZE: usize = 101;
const REQUIRED_METADATA_KEYS: [&str; 3] = ["cytiq.gate_name", "cytiq.x_axis", "cytiq.y_axis"];

#[derive(Debug, Clone)]
pub struct GateCommand {
    pub model_path: PathBuf,
    pub fcs_path: PathBuf,
    pub output_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateRunSummary {
    pub event_count: usize,
    pub gate_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GateMetadata {
    gate_name: String,
    x_axis: String,
    y_axis: String,
    parent_gate: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GateModel {
    path: PathBuf,
    metadata: GateMetadata,
}

#[derive(Debug, Clone, PartialEq)]
struct ResolvedAxis {
    name: String,
    values: Vec<f64>,
}

#[derive(Debug, Clone, PartialEq)]
struct PreparedInput {
    events: Vec<PreparedEvent>,
    selected_events: Vec<bool>,
    tensor: Option<Array4<f32>>,
}

#[derive(Debug, Clone, PartialEq)]
struct PreparedEvent {
    event_index: i64,
    x_value: f64,
    y_value: f64,
    x_bin: u8,
    y_bin: u8,
}

#[derive(Debug, Clone, PartialEq)]
struct PredictionRow {
    event_index: i64,
    gate_name: String,
    x_axis: String,
    y_axis: String,
    x_value: f64,
    y_value: f64,
    x_bin: u8,
    y_bin: u8,
    prediction: bool,
}

#[derive(Debug, Error)]
pub enum GateError {
    #[error("failed to read FCS file: {0}")]
    Fcs(#[from] cytiq_fcs::Error),
    #[error("failed to initialize ONNX runtime: {0}")]
    Ort(#[from] ort::Error),
    #[error("failed to parse ONNX model `{model_path}`: {reason}")]
    InvalidOnnxModel { model_path: PathBuf, reason: String },
    #[error("failed to build parquet batch: {0}")]
    Arrow(#[from] arrow_schema::ArrowError),
    #[error("failed to write parquet file: {0}")]
    Parquet(#[from] parquet::errors::ParquetError),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("model `{model_path}` references missing external data file(s): {missing}")]
    MissingExternalData {
        model_path: PathBuf,
        missing: String,
    },
    #[error(
        "model `{model_path}` uses external tensor data without an `external_data.location` entry for tensor(s): {tensors}"
    )]
    MissingExternalDataLocation {
        model_path: PathBuf,
        tensors: String,
    },
    #[error("model metadata is missing required keys: {missing:?}")]
    MissingMetadata { missing: Vec<&'static str> },
    #[error("could not locate parent gate model `{parent_gate}` next to `{model_path}`")]
    MissingParentModel {
        model_path: PathBuf,
        parent_gate: String,
    },
    #[error(
        "found multiple usable parent gate models for `{parent_gate}` next to `{model_path}`: {candidates}"
    )]
    AmbiguousParentModel {
        model_path: PathBuf,
        parent_gate: String,
        candidates: String,
    },
    #[error("detected cyclic UNITO parent gate chain at `{gate_name}`")]
    CyclicParentGate { gate_name: String },
    #[error(
        "FCS file is incompatible with the model chain; missing parameters: {missing}. Available parameters: {available}"
    )]
    IncompatibleFcsAxes { missing: String, available: String },
    #[error("FCS parameter `{axis}` was not found")]
    MissingAxis { axis: String },
    #[error("FCS parameter `{axis}` matched multiple labels")]
    AmbiguousAxis { axis: String },
    #[error("FCS parameter `{axis}` is not numeric")]
    NonNumericAxis { axis: String },
    #[error(
        "FCS parameter `{axis}` contains integer values above 2^53 and cannot be converted to f64 without losing precision"
    )]
    IntegerAxisPrecisionLoss { axis: String },
    #[error("selected axes have different event counts ({x_count} vs {y_count})")]
    MismatchedEventCount { x_count: usize, y_count: usize },
    #[error(
        "parent gate selection contains {parent_count} events, but the current FCS axes contain {event_count}"
    )]
    MismatchedParentSelection {
        parent_count: usize,
        event_count: usize,
    },
    #[error("FCS file contains no events")]
    EmptyDataset,
    #[error("axis `{axis}` cannot be normalized because all values are identical")]
    DegenerateAxis { axis: String },
    #[error("axis `{axis}` contains non-finite values")]
    NonFiniteAxis { axis: String },
    #[error("model must expose exactly one input, found {found}")]
    UnsupportedInputCount { found: usize },
    #[error("model must expose exactly one output, found {found}")]
    UnsupportedOutputCount { found: usize },
    #[error("unsupported model output shape {shape:?}; expected [1, 1, 101, 101] or [1, 101, 101]")]
    UnsupportedOutputShape { shape: Vec<usize> },
    #[error("model did not return an output tensor")]
    MissingOutputTensor,
}

/// Run UNITO-style single-gate inference for one FCS file and persist event-level
/// predictions as a Parquet file.
///
/// # Errors
///
/// Returns an error when the model metadata is incomplete, the selected FCS axes
/// are missing or not representable as numeric input, ONNX inference fails, or
/// the output Parquet file cannot be created.
pub fn run_gate(command: &GateCommand) -> Result<GateRunSummary, GateError> {
    let models = resolve_model_chain(&command.model_path)?;
    let dataset = FcsReader::open(&command.fcs_path)?.read()?;
    validate_chain_axes(&dataset.parameters, &models)?;
    let mut parent_predictions: Option<Vec<bool>> = None;
    let mut rows = Vec::new();
    let mut gate_name = String::new();

    for model in models {
        let mut session = ort::session::Session::builder()?.commit_from_file(&model.path)?;
        validate_model_io(&session)?;

        let x_axis = resolve_axis(
            &dataset.parameters,
            &dataset.data.columns,
            &model.metadata.x_axis,
        )?;
        let y_axis = resolve_axis(
            &dataset.parameters,
            &dataset.data.columns,
            &model.metadata.y_axis,
        )?;
        let prepared = prepare_input(&x_axis, &y_axis, parent_predictions.as_deref())?;
        let mask = prepared
            .tensor
            .as_ref()
            .map(|tensor| run_inference(&mut session, tensor))
            .transpose()?;
        rows = project_predictions(
            &model.metadata,
            &prepared.events,
            &prepared.selected_events,
            mask.as_ref(),
        );
        parent_predictions = Some(rows.iter().map(|row| row.prediction).collect());
        gate_name = model.metadata.gate_name;
    }

    write_predictions(&command.output_path, &rows)?;

    Ok(GateRunSummary {
        event_count: rows.len(),
        gate_name,
    })
}

impl GateMetadata {
    fn from_model(model: &ModelProto) -> Result<Self, GateError> {
        let missing = REQUIRED_METADATA_KEYS
            .iter()
            .copied()
            .filter(|key| metadata_value(&model.metadata_props, key).is_none())
            .collect::<Vec<_>>();
        if !missing.is_empty() {
            return Err(GateError::MissingMetadata { missing });
        }

        Ok(Self {
            gate_name: metadata_value(&model.metadata_props, "cytiq.gate_name")
                .expect("validated above")
                .to_owned(),
            x_axis: metadata_value(&model.metadata_props, "cytiq.x_axis")
                .expect("validated above")
                .to_owned(),
            y_axis: metadata_value(&model.metadata_props, "cytiq.y_axis")
                .expect("validated above")
                .to_owned(),
            parent_gate: metadata_value(&model.metadata_props, "cytiq.parent_gate")
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned),
        })
    }
}

fn load_model(model_path: &Path) -> Result<ModelProto, GateError> {
    let model = parse_model(model_path)?;
    validate_external_data_files(&model, model_path)?;
    Ok(model)
}

fn parse_model(model_path: &Path) -> Result<ModelProto, GateError> {
    let mut file = File::open(model_path)?;
    ModelProto::parse_from_reader(&mut file).map_err(|error| GateError::InvalidOnnxModel {
        model_path: model_path.to_path_buf(),
        reason: error.to_string(),
    })
}

fn resolve_model_chain(model_path: &Path) -> Result<Vec<GateModel>, GateError> {
    let mut models = Vec::new();
    let mut active_gates = BTreeSet::new();
    resolve_model_chain_inner(model_path, &mut active_gates, &mut models)?;
    Ok(models)
}

fn resolve_model_chain_inner(
    model_path: &Path,
    active_gates: &mut BTreeSet<String>,
    models: &mut Vec<GateModel>,
) -> Result<(), GateError> {
    let model = load_model(model_path)?;
    let metadata = GateMetadata::from_model(&model)?;

    if !active_gates.insert(metadata.gate_name.clone()) {
        return Err(GateError::CyclicParentGate {
            gate_name: metadata.gate_name,
        });
    }

    if let Some(parent_gate) = metadata.parent_gate.as_deref() {
        let parent_model_path = find_parent_model_path(model_path, parent_gate)?;
        resolve_model_chain_inner(&parent_model_path, active_gates, models)?;
    }

    active_gates.remove(&metadata.gate_name);
    models.push(GateModel {
        path: model_path.to_path_buf(),
        metadata,
    });
    Ok(())
}

fn find_parent_model_path(model_path: &Path, parent_gate: &str) -> Result<PathBuf, GateError> {
    let model_dir = model_path.parent().unwrap_or_else(|| Path::new("."));
    let expected_file_name = format!("{parent_gate}_model.onnx");
    let mut valid_candidates = Vec::new();
    let mut invalid_candidates = Vec::new();

    for entry in fs::read_dir(model_dir)? {
        let path = entry?.path();
        if !is_onnx_file(&path) || path == model_path {
            continue;
        }

        let Ok(model) = parse_model(&path) else {
            continue;
        };
        let Ok(metadata) = GateMetadata::from_model(&model) else {
            continue;
        };
        if metadata.gate_name != parent_gate {
            continue;
        }

        match validate_external_data_files(&model, &path) {
            Ok(()) => valid_candidates.push(path),
            Err(error) => invalid_candidates.push((path, error)),
        }
    }

    if let Some(path) = choose_candidate(
        valid_candidates,
        model_path,
        parent_gate,
        &expected_file_name,
    )? {
        return Ok(path);
    }
    if let Some(error) = choose_invalid_candidate_error(
        invalid_candidates,
        model_path,
        parent_gate,
        &expected_file_name,
    )? {
        return Err(error);
    }

    Err(GateError::MissingParentModel {
        model_path: model_path.to_path_buf(),
        parent_gate: parent_gate.to_owned(),
    })
}

fn choose_candidate(
    candidates: Vec<PathBuf>,
    model_path: &Path,
    parent_gate: &str,
    expected_file_name: &str,
) -> Result<Option<PathBuf>, GateError> {
    match candidates.as_slice() {
        [] => Ok(None),
        [path] => Ok(Some(path.clone())),
        _ => {
            let exact_matches = candidates
                .iter()
                .filter(|path| file_name_matches(path, expected_file_name))
                .cloned()
                .collect::<Vec<_>>();
            match exact_matches.as_slice() {
                [path] => Ok(Some(path.clone())),
                _ => Err(GateError::AmbiguousParentModel {
                    model_path: model_path.to_path_buf(),
                    parent_gate: parent_gate.to_owned(),
                    candidates: join_paths(&candidates),
                }),
            }
        }
    }
}

fn choose_invalid_candidate_error(
    candidates: Vec<(PathBuf, GateError)>,
    model_path: &Path,
    parent_gate: &str,
    expected_file_name: &str,
) -> Result<Option<GateError>, GateError> {
    let candidate_paths = candidates
        .iter()
        .map(|(path, _error)| path.clone())
        .collect::<Vec<_>>();
    match candidates.as_slice() {
        [] => Ok(None),
        [_single_candidate] => Ok(Some(
            candidates.into_iter().next().expect("single candidate").1,
        )),
        _ => {
            let exact_matches = candidates
                .into_iter()
                .filter(|(path, _error)| file_name_matches(path, expected_file_name))
                .collect::<Vec<_>>();
            match exact_matches.as_slice() {
                [_single_candidate] => Ok(Some(
                    exact_matches
                        .into_iter()
                        .next()
                        .expect("single candidate")
                        .1,
                )),
                _ => Err(GateError::AmbiguousParentModel {
                    model_path: model_path.to_path_buf(),
                    parent_gate: parent_gate.to_owned(),
                    candidates: join_paths(&candidate_paths),
                }),
            }
        }
    }
}

fn is_onnx_file(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("onnx"))
}

fn file_name_matches(path: &Path, expected_file_name: &str) -> bool {
    path.file_name()
        .and_then(|file_name| file_name.to_str())
        .is_some_and(|file_name| file_name == expected_file_name)
}

fn join_paths(paths: &[PathBuf]) -> String {
    paths
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

fn validate_external_data_files(model: &ModelProto, model_path: &Path) -> Result<(), GateError> {
    let mut missing_paths = BTreeSet::new();
    let mut missing_locations = BTreeSet::new();

    if let Some(graph) = model.graph.as_ref() {
        collect_external_data_issues_in_graph(
            graph,
            model_path,
            &mut missing_paths,
            &mut missing_locations,
        );
    }

    if !missing_locations.is_empty() {
        return Err(GateError::MissingExternalDataLocation {
            model_path: model_path.to_path_buf(),
            tensors: missing_locations.into_iter().collect::<Vec<_>>().join(", "),
        });
    }
    if !missing_paths.is_empty() {
        return Err(GateError::MissingExternalData {
            model_path: model_path.to_path_buf(),
            missing: missing_paths
                .into_iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(", "),
        });
    }

    Ok(())
}

fn collect_external_data_issues_in_graph(
    graph: &GraphProto,
    model_path: &Path,
    missing_paths: &mut BTreeSet<PathBuf>,
    missing_locations: &mut BTreeSet<String>,
) {
    for tensor in &graph.initializer {
        collect_external_data_issues_in_tensor(
            tensor,
            model_path,
            missing_paths,
            missing_locations,
        );
    }
    for sparse_tensor in &graph.sparse_initializer {
        collect_external_data_issues_in_sparse_tensor(
            sparse_tensor,
            model_path,
            missing_paths,
            missing_locations,
        );
    }
    for node in &graph.node {
        for attribute in &node.attribute {
            collect_external_data_issues_in_attribute(
                attribute,
                model_path,
                missing_paths,
                missing_locations,
            );
        }
    }
}

fn collect_external_data_issues_in_attribute(
    attribute: &AttributeProto,
    model_path: &Path,
    missing_paths: &mut BTreeSet<PathBuf>,
    missing_locations: &mut BTreeSet<String>,
) {
    if let Some(tensor) = attribute.t.as_ref() {
        collect_external_data_issues_in_tensor(
            tensor,
            model_path,
            missing_paths,
            missing_locations,
        );
    }
    for tensor in &attribute.tensors {
        collect_external_data_issues_in_tensor(
            tensor,
            model_path,
            missing_paths,
            missing_locations,
        );
    }
    if let Some(graph) = attribute.g.as_ref() {
        collect_external_data_issues_in_graph(graph, model_path, missing_paths, missing_locations);
    }
    for graph in &attribute.graphs {
        collect_external_data_issues_in_graph(graph, model_path, missing_paths, missing_locations);
    }
    if let Some(sparse_tensor) = attribute.sparse_tensor.as_ref() {
        collect_external_data_issues_in_sparse_tensor(
            sparse_tensor,
            model_path,
            missing_paths,
            missing_locations,
        );
    }
    for sparse_tensor in &attribute.sparse_tensors {
        collect_external_data_issues_in_sparse_tensor(
            sparse_tensor,
            model_path,
            missing_paths,
            missing_locations,
        );
    }
}

fn collect_external_data_issues_in_sparse_tensor(
    sparse_tensor: &SparseTensorProto,
    model_path: &Path,
    missing_paths: &mut BTreeSet<PathBuf>,
    missing_locations: &mut BTreeSet<String>,
) {
    if let Some(values) = sparse_tensor.values.as_ref() {
        collect_external_data_issues_in_tensor(
            values,
            model_path,
            missing_paths,
            missing_locations,
        );
    }
    if let Some(indices) = sparse_tensor.indices.as_ref() {
        collect_external_data_issues_in_tensor(
            indices,
            model_path,
            missing_paths,
            missing_locations,
        );
    }
}

fn collect_external_data_issues_in_tensor(
    tensor: &TensorProto,
    model_path: &Path,
    missing_paths: &mut BTreeSet<PathBuf>,
    missing_locations: &mut BTreeSet<String>,
) {
    let uses_external_data = tensor.data_location.enum_value_or_default()
        == tensor_proto::DataLocation::EXTERNAL
        || !tensor.external_data.is_empty();
    if !uses_external_data {
        return;
    }

    let Some(location) = external_data_location(tensor) else {
        missing_locations.insert(tensor_name(tensor));
        return;
    };

    let path = resolve_external_data_path(model_path, location);
    if !path.exists() {
        missing_paths.insert(path);
    }
}

fn external_data_location<'a>(tensor: &'a TensorProto) -> Option<&'a str> {
    tensor.external_data.iter().find_map(|entry| {
        (entry.key == "location" && !entry.value.trim().is_empty()).then_some(entry.value.as_str())
    })
}

fn resolve_external_data_path(model_path: &Path, location: &str) -> PathBuf {
    let location = Path::new(location);
    if location.is_absolute() {
        return location.to_path_buf();
    }

    model_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(location)
}

fn tensor_name(tensor: &TensorProto) -> String {
    if tensor.name.trim().is_empty() {
        return "<unnamed>".to_owned();
    }
    tensor.name.clone()
}

fn metadata_value<'a>(metadata: &'a [StringStringEntryProto], key: &str) -> Option<&'a str> {
    metadata
        .iter()
        .find_map(|entry| (entry.key == key).then_some(entry.value.as_str()))
}

fn validate_model_io(session: &ort::session::Session) -> Result<(), GateError> {
    if session.inputs().len() != 1 {
        return Err(GateError::UnsupportedInputCount {
            found: session.inputs().len(),
        });
    }
    if session.outputs().len() != 1 {
        return Err(GateError::UnsupportedOutputCount {
            found: session.outputs().len(),
        });
    }
    Ok(())
}

fn validate_chain_axes(parameters: &[Parameter], models: &[GateModel]) -> Result<(), GateError> {
    let mut missing = Vec::new();
    let mut seen = BTreeSet::new();

    for model in models {
        for axis in [&model.metadata.x_axis, &model.metadata.y_axis] {
            match resolve_axis_index(parameters, axis) {
                Ok(_index) => {}
                Err(GateError::MissingAxis { axis }) => {
                    if seen.insert(axis.clone()) {
                        missing.push(axis);
                    }
                }
                Err(error) => return Err(error),
            }
        }
    }

    if missing.is_empty() {
        return Ok(());
    }

    Err(GateError::IncompatibleFcsAxes {
        missing: missing.join(", "),
        available: available_axes(parameters),
    })
}

fn available_axes(parameters: &[Parameter]) -> String {
    parameters
        .iter()
        .map(|parameter| match parameter.label.as_deref() {
            Some(label) if label != parameter.name => {
                format!("{} (label: {label})", parameter.name)
            }
            _ => parameter.name.clone(),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn resolve_axis(
    parameters: &[Parameter],
    columns: &[ColumnData],
    axis: &str,
) -> Result<ResolvedAxis, GateError> {
    let index = resolve_axis_index(parameters, axis)?;
    let parameter = &parameters[index];
    Ok(ResolvedAxis {
        name: parameter
            .label
            .clone()
            .filter(|label| label.eq_ignore_ascii_case(axis))
            .unwrap_or_else(|| parameter.name.clone()),
        values: numeric_values(columns.get(index), axis)?,
    })
}

fn resolve_axis_index(parameters: &[Parameter], axis: &str) -> Result<usize, GateError> {
    let matches = parameters
        .iter()
        .enumerate()
        .filter_map(|(index, parameter)| axis_match_kind(axis, parameter).map(|kind| (index, kind)))
        .collect::<Vec<_>>();

    let Some(best_kind) = matches.iter().map(|(_index, kind)| *kind).min() else {
        return Err(GateError::MissingAxis {
            axis: axis.to_owned(),
        });
    };

    let best_matches = matches
        .into_iter()
        .filter_map(|(index, kind)| (kind == best_kind).then_some(index))
        .collect::<Vec<_>>();

    match best_matches.as_slice() {
        [index] => Ok(*index),
        _ => Err(GateError::AmbiguousAxis {
            axis: axis.to_owned(),
        }),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum AxisMatchKind {
    ExactName,
    ExactLabel,
    NormalizedName,
    NormalizedLabel,
    MarkerAreaLabel,
    MarkerLabel,
}

fn axis_match_kind(axis: &str, parameter: &Parameter) -> Option<AxisMatchKind> {
    if parameter.name == axis {
        return Some(AxisMatchKind::ExactName);
    }
    if parameter
        .label
        .as_deref()
        .is_some_and(|label| label == axis)
    {
        return Some(AxisMatchKind::ExactLabel);
    }

    let normalized_axis = normalize_axis_name(axis);
    if normalize_axis_name(&parameter.name) == normalized_axis {
        return Some(AxisMatchKind::NormalizedName);
    }
    if parameter
        .label
        .as_deref()
        .is_some_and(|label| normalize_axis_name(label) == normalized_axis)
    {
        return Some(AxisMatchKind::NormalizedLabel);
    }

    let Some(label) = parameter.label.as_deref() else {
        return None;
    };
    if !label_has_marker_prefix(label, axis) {
        return None;
    }

    if is_area_parameter(parameter) {
        Some(AxisMatchKind::MarkerAreaLabel)
    } else {
        Some(AxisMatchKind::MarkerLabel)
    }
}

fn normalize_axis_name(value: &str) -> String {
    value
        .chars()
        .filter(|char| char.is_ascii_alphanumeric())
        .flat_map(char::to_uppercase)
        .collect()
}

fn label_has_marker_prefix(label: &str, axis: &str) -> bool {
    let label = label.trim();
    let Some(prefix) = label.get(..axis.len()) else {
        return false;
    };
    if !prefix.eq_ignore_ascii_case(axis) {
        return false;
    }

    label
        .get(axis.len()..)
        .unwrap_or_default()
        .chars()
        .next()
        .is_none_or(|char| !char.is_ascii_alphanumeric())
}

fn is_area_parameter(parameter: &Parameter) -> bool {
    parameter.name.to_ascii_uppercase().ends_with("-A")
        || parameter
            .label
            .as_deref()
            .is_some_and(|label| label.to_ascii_uppercase().ends_with("-A"))
}

fn numeric_values(column: Option<&ColumnData>, axis: &str) -> Result<Vec<f64>, GateError> {
    match column {
        Some(ColumnData::UInt(values)) => values
            .iter()
            .map(|value| {
                if *value > (1_u64 << 53) {
                    return Err(GateError::IntegerAxisPrecisionLoss {
                        axis: axis.to_owned(),
                    });
                }
                value
                    .to_f64()
                    .ok_or_else(|| GateError::IntegerAxisPrecisionLoss {
                        axis: axis.to_owned(),
                    })
            })
            .collect(),
        Some(ColumnData::F32(values)) => Ok(values.iter().map(|value| f64::from(*value)).collect()),
        Some(ColumnData::F64(values)) => Ok(values.clone()),
        None => Err(GateError::MissingAxis {
            axis: axis.to_owned(),
        }),
    }
}

fn prepare_input(
    x_axis: &ResolvedAxis,
    y_axis: &ResolvedAxis,
    parent_selection: Option<&[bool]>,
) -> Result<PreparedInput, GateError> {
    if x_axis.values.is_empty() || y_axis.values.is_empty() {
        return Err(GateError::EmptyDataset);
    }
    if x_axis.values.len() != y_axis.values.len() {
        return Err(GateError::MismatchedEventCount {
            x_count: x_axis.values.len(),
            y_count: y_axis.values.len(),
        });
    }

    let selected_events = match parent_selection {
        Some(parent_selection) => {
            if parent_selection.len() != x_axis.values.len() {
                return Err(GateError::MismatchedParentSelection {
                    parent_count: parent_selection.len(),
                    event_count: x_axis.values.len(),
                });
            }
            parent_selection.to_vec()
        }
        None => vec![true; x_axis.values.len()],
    };

    let x_norm = normalize_axis(&x_axis.values, &x_axis.name, &selected_events)?;
    let y_norm = normalize_axis(&y_axis.values, &y_axis.name, &selected_events)?;

    let mut grid = Array2::<f32>::zeros((GRID_SIZE, GRID_SIZE));
    let mut events = Vec::with_capacity(x_axis.values.len());
    let mut selected_count = 0_usize;
    for (index, (((x_value, x_bin), (y_value, y_bin)), is_selected)) in x_axis
        .values
        .iter()
        .zip(&x_norm)
        .zip(y_axis.values.iter().zip(&y_norm))
        .zip(&selected_events)
        .enumerate()
    {
        if *is_selected {
            grid[[GRID_SIZE - 1 - usize::from(*x_bin), usize::from(*y_bin)]] += 1.0;
            selected_count += 1;
        }
        events.push(PreparedEvent {
            event_index: i64::try_from(index).expect("event count exceeds i64"),
            x_value: *x_value,
            y_value: *y_value,
            x_bin: *x_bin,
            y_bin: *y_bin,
        });
    }

    let tensor = if selected_count == 0 {
        None
    } else {
        let max_count = grid.iter().copied().fold(0.0_f32, f32::max);
        let normalized = grid.mapv(|value| value / max_count);
        Some(
            normalized
                .into_shape_with_order((1, 1, GRID_SIZE, GRID_SIZE))
                .expect("fixed grid shape should always reshape"),
        )
    };

    Ok(PreparedInput {
        events,
        selected_events,
        tensor,
    })
}

fn normalize_axis(
    values: &[f64],
    axis: &str,
    selected_events: &[bool],
) -> Result<Vec<u8>, GateError> {
    if values.is_empty() {
        return Err(GateError::EmptyDataset);
    }
    if values.iter().any(|value| !value.is_finite()) {
        return Err(GateError::NonFiniteAxis {
            axis: axis.to_owned(),
        });
    }

    let (min, max) = normalization_bounds(values, selected_events);
    let scale = max - min;
    if scale.abs() <= f64::EPSILON {
        return Err(GateError::DegenerateAxis {
            axis: axis.to_owned(),
        });
    }

    Ok(values
        .iter()
        .map(|value| normalize_to_bin(*value, min, scale))
        .collect())
}

fn normalization_bounds(values: &[f64], selected_events: &[bool]) -> (f64, f64) {
    let use_selected = selected_events.iter().any(|selected| *selected);
    values
        .iter()
        .zip(selected_events)
        .filter_map(|(value, selected)| (!use_selected || *selected).then_some(*value))
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(min, max), value| {
            (min.min(value), max.max(value))
        })
}

fn run_inference(
    session: &mut ort::session::Session,
    tensor: &Array4<f32>,
) -> Result<Array2<bool>, GateError> {
    let output_name = session.outputs()[0].name().to_owned();
    let outputs = session.run(ort::inputs![TensorRef::from_array_view(tensor.view())?])?;
    let output = outputs
        .get(output_name)
        .ok_or(GateError::MissingOutputTensor)?
        .try_extract_array::<f32>()?;

    let shape = output.shape().to_vec();
    match shape.as_slice() {
        [1, 1, GRID_SIZE, GRID_SIZE] => Ok(Array2::from_shape_fn(
            (GRID_SIZE, GRID_SIZE),
            |(row, col)| sigmoid(output[[0, 0, row, col]]) > 0.5,
        )),
        [1, GRID_SIZE, GRID_SIZE] => Ok(Array2::from_shape_fn(
            (GRID_SIZE, GRID_SIZE),
            |(row, col)| sigmoid(output[[0, row, col]]) > 0.5,
        )),
        _ => Err(GateError::UnsupportedOutputShape { shape }),
    }
}

fn project_predictions(
    metadata: &GateMetadata,
    events: &[PreparedEvent],
    selected_events: &[bool],
    mask: Option<&Array2<bool>>,
) -> Vec<PredictionRow> {
    events
        .iter()
        .zip(selected_events)
        .map(|(event, is_selected)| PredictionRow {
            event_index: event.event_index,
            gate_name: metadata.gate_name.clone(),
            x_axis: metadata.x_axis.clone(),
            y_axis: metadata.y_axis.clone(),
            x_value: event.x_value,
            y_value: event.y_value,
            x_bin: event.x_bin,
            y_bin: event.y_bin,
            prediction: *is_selected
                && mask.is_some_and(|mask| {
                    mask[[
                        GRID_SIZE - 1 - usize::from(event.x_bin),
                        usize::from(event.y_bin),
                    ]]
                }),
        })
        .collect()
}

fn write_predictions(path: &Path, rows: &[PredictionRow]) -> Result<(), GateError> {
    let schema = Arc::new(Schema::new(vec![
        Field::new("event_index", DataType::Int64, false),
        Field::new("gate_name", DataType::Utf8, false),
        Field::new("x_axis", DataType::Utf8, false),
        Field::new("y_axis", DataType::Utf8, false),
        Field::new("x_value", DataType::Float64, false),
        Field::new("y_value", DataType::Float64, false),
        Field::new("x_bin", DataType::UInt8, false),
        Field::new("y_bin", DataType::UInt8, false),
        Field::new("prediction", DataType::Boolean, false),
    ]));

    let batch = RecordBatch::try_new(
        Arc::clone(&schema),
        vec![
            Arc::new(Int64Array::from(
                rows.iter().map(|row| row.event_index).collect::<Vec<_>>(),
            )) as ArrayRef,
            Arc::new(StringArray::from(
                rows.iter()
                    .map(|row| row.gate_name.as_str())
                    .collect::<Vec<_>>(),
            )),
            Arc::new(StringArray::from(
                rows.iter()
                    .map(|row| row.x_axis.as_str())
                    .collect::<Vec<_>>(),
            )),
            Arc::new(StringArray::from(
                rows.iter()
                    .map(|row| row.y_axis.as_str())
                    .collect::<Vec<_>>(),
            )),
            Arc::new(Float64Array::from(
                rows.iter().map(|row| row.x_value).collect::<Vec<_>>(),
            )),
            Arc::new(Float64Array::from(
                rows.iter().map(|row| row.y_value).collect::<Vec<_>>(),
            )),
            Arc::new(UInt8Array::from(
                rows.iter().map(|row| row.x_bin).collect::<Vec<_>>(),
            )),
            Arc::new(UInt8Array::from(
                rows.iter().map(|row| row.y_bin).collect::<Vec<_>>(),
            )),
            Arc::new(BooleanArray::from(
                rows.iter().map(|row| row.prediction).collect::<Vec<_>>(),
            )),
        ],
    )?;

    let file = File::create(path)?;
    let mut writer = ArrowWriter::try_new(file, schema, None)?;
    writer.write(&batch)?;
    writer.close()?;
    Ok(())
}

fn sigmoid(value: f32) -> f32 {
    1.0 / (1.0 + (-value).exp())
}

fn normalize_to_bin(value: f64, min: f64, scale: f64) -> u8 {
    (((value - min) / scale) * 100.0)
        .clamp(0.0, 100.0)
        .round_ties_even()
        .to_u8()
        .expect("normalized values are clamped to 0..=100")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    mod support {
        #![allow(dead_code)]
        include!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/support.rs"));
    }

    #[test]
    fn normalizes_with_bankers_rounding() {
        let values = [0.0, 0.5, 1.0, 2.5];
        let normalized =
            normalize_axis(&values, "FSC-A", &[true; 4]).expect("normalization should succeed");

        assert_eq!(normalized, vec![0, 20, 40, 100]);
    }

    #[test]
    fn builds_density_grid_and_projects_predictions() {
        let x_axis = ResolvedAxis {
            name: "X".to_owned(),
            values: vec![0.0, 10.0, 10.0],
        };
        let y_axis = ResolvedAxis {
            name: "Y".to_owned(),
            values: vec![0.0, 5.0, 10.0],
        };

        let prepared = prepare_input(&x_axis, &y_axis, None).expect("input should prepare");
        assert_eq!(
            prepared
                .tensor
                .as_ref()
                .expect("root gate should produce a tensor")
                .shape(),
            &[1, 1, GRID_SIZE, GRID_SIZE]
        );

        let mut mask = Array2::<bool>::from_elem((GRID_SIZE, GRID_SIZE), false);
        mask[[GRID_SIZE - 1, 0]] = true;
        mask[[0, 50]] = true;

        let metadata = GateMetadata {
            gate_name: "ExampleGate".to_owned(),
            x_axis: "X".to_owned(),
            y_axis: "Y".to_owned(),
            parent_gate: None,
        };
        let rows = project_predictions(
            &metadata,
            &prepared.events,
            &prepared.selected_events,
            Some(&mask),
        );

        assert_eq!(rows.len(), 3);
        assert!(rows[0].prediction);
        assert!(rows[1].prediction);
        assert!(!rows[2].prediction);
    }

    #[test]
    fn parent_selection_filters_the_density_grid() {
        let x_axis = ResolvedAxis {
            name: "X".to_owned(),
            values: vec![0.0, 50.0, 100.0],
        };
        let y_axis = ResolvedAxis {
            name: "Y".to_owned(),
            values: vec![0.0, 50.0, 100.0],
        };
        let parent_selection = [true, false, true];

        let prepared =
            prepare_input(&x_axis, &y_axis, Some(&parent_selection)).expect("input should prepare");
        let tensor = prepared
            .tensor
            .as_ref()
            .expect("selected events should produce a tensor");

        assert_eq!(prepared.selected_events, parent_selection);
        assert_eq!(tensor[[0, 0, GRID_SIZE - 1, 0]], 1.0);
        assert_eq!(tensor[[0, 0, GRID_SIZE - 1 - 50, 50]], 0.0);
        assert_eq!(tensor[[0, 0, 0, 100]], 1.0);

        let mask = Array2::<bool>::from_elem((GRID_SIZE, GRID_SIZE), true);
        let metadata = GateMetadata {
            gate_name: "Child".to_owned(),
            x_axis: "X".to_owned(),
            y_axis: "Y".to_owned(),
            parent_gate: Some("Parent".to_owned()),
        };
        let rows = project_predictions(
            &metadata,
            &prepared.events,
            &prepared.selected_events,
            Some(&mask),
        );

        assert_eq!(
            rows.iter().map(|row| row.prediction).collect::<Vec<_>>(),
            vec![true, false, true]
        );
    }

    #[test]
    fn resolves_marker_labels_to_area_channel() {
        let parameters = vec![
            Parameter {
                index: 1,
                name: "FITC-A".to_owned(),
                label: Some("CD3 FITC-A".to_owned()),
                width: cytiq_fcs::ParameterWidth::Bits(32),
                data_type: cytiq_fcs::ParameterDataType::Float,
                keywords: Vec::new(),
            },
            Parameter {
                index: 2,
                name: "FITC-W".to_owned(),
                label: Some("CD3 FITC-W".to_owned()),
                width: cytiq_fcs::ParameterWidth::Bits(32),
                data_type: cytiq_fcs::ParameterDataType::Float,
                keywords: Vec::new(),
            },
            Parameter {
                index: 3,
                name: "FITC-H".to_owned(),
                label: Some("CD3 FITC-H".to_owned()),
                width: cytiq_fcs::ParameterWidth::Bits(32),
                data_type: cytiq_fcs::ParameterDataType::Float,
                keywords: Vec::new(),
            },
        ];

        let index = resolve_axis_index(&parameters, "CD3").expect("marker should resolve");
        assert_eq!(index, 0);
    }

    #[test]
    fn resolves_parent_model_from_valid_duplicate() -> Result<(), Box<dyn std::error::Error>> {
        let temp_dir = tempdir()?;
        let root_path = temp_dir.path().join("Single_cell_gate_1_model.onnx");
        let leaf_path = temp_dir.path().join("Neutrophil_model.onnx");
        support::write_identity_unito_model_with_metadata(
            &root_path,
            "Single_cell_gate_1",
            "FSC-A",
            "SSC-A",
            None,
        )?;
        support::write_unito_model_with_external_data_and_metadata(
            &temp_dir.path().join("Granulocyte_model.onnx"),
            "Granulocyte_model.onnx.data",
            "Granulocyte",
            "FSC-A",
            "SSC-A",
            Some("Single_cell_gate_1"),
        )?;
        support::write_identity_unito_model_with_metadata(
            &temp_dir.path().join("Granulocyte_model(2).onnx"),
            "Granulocyte",
            "FSC-A",
            "SSC-A",
            Some("Single_cell_gate_1"),
        )?;
        support::write_identity_unito_model_with_metadata(
            &leaf_path,
            "Neutrophil",
            "FSC-A",
            "SSC-A",
            Some("Granulocyte"),
        )?;

        let models = resolve_model_chain(&leaf_path)?;
        assert_eq!(
            models
                .iter()
                .map(|model| model.metadata.gate_name.as_str())
                .collect::<Vec<_>>(),
            vec!["Single_cell_gate_1", "Granulocyte", "Neutrophil"]
        );
        assert!(
            models[1].path.ends_with("Granulocyte_model(2).onnx"),
            "expected valid duplicate model, got {}",
            models[1].path.display()
        );

        Ok(())
    }
}
