use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use arrow_array::{
    ArrayRef, BooleanArray, Float64Array, Int64Array, RecordBatch, StringArray, UInt8Array,
};
use arrow_schema::{DataType, Field, Schema};
use cytiq_fcs::{ColumnData, FcsReader, Parameter};
use ndarray::{Array2, Array4};
use num_traits::ToPrimitive;
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
}

#[derive(Debug, Clone, PartialEq)]
struct ResolvedAxis {
    name: String,
    values: Vec<f64>,
}

#[derive(Debug, Clone, PartialEq)]
struct PreparedInput {
    events: Vec<PreparedEvent>,
    tensor: Array4<f32>,
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
    #[error("failed to build parquet batch: {0}")]
    Arrow(#[from] arrow_schema::ArrowError),
    #[error("failed to write parquet file: {0}")]
    Parquet(#[from] parquet::errors::ParquetError),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("model metadata is missing required keys: {missing:?}")]
    MissingMetadata { missing: Vec<&'static str> },
    #[error("sequential UNITO gating is not supported yet (`cytiq.parent_gate={parent_gate}`)")]
    UnsupportedParentGate { parent_gate: String },
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
    let dataset = FcsReader::open(&command.fcs_path)?.read()?;

    let mut session = ort::session::Session::builder()?.commit_from_file(&command.model_path)?;
    let metadata = GateMetadata::from_session(&session)?;
    validate_model_io(&session)?;

    let x_axis = resolve_axis(&dataset.parameters, &dataset.data.columns, &metadata.x_axis)?;
    let y_axis = resolve_axis(&dataset.parameters, &dataset.data.columns, &metadata.y_axis)?;
    let prepared = prepare_input(&x_axis, &y_axis)?;
    let mask = run_inference(&mut session, &prepared.tensor)?;
    let rows = project_predictions(&metadata, &prepared.events, &mask);
    write_predictions(&command.output_path, &rows)?;

    Ok(GateRunSummary {
        event_count: rows.len(),
        gate_name: metadata.gate_name,
    })
}

impl GateMetadata {
    fn from_session(session: &ort::session::Session) -> Result<Self, GateError> {
        let metadata = session.metadata()?;
        let missing = REQUIRED_METADATA_KEYS
            .into_iter()
            .filter(|key| metadata.custom(key).is_none())
            .collect::<Vec<_>>();
        if !missing.is_empty() {
            return Err(GateError::MissingMetadata { missing });
        }

        if let Some(parent_gate) = metadata.custom("cytiq.parent_gate")
            && !parent_gate.trim().is_empty()
        {
            return Err(GateError::UnsupportedParentGate { parent_gate });
        }

        Ok(Self {
            gate_name: metadata.custom("cytiq.gate_name").expect("validated above"),
            x_axis: metadata.custom("cytiq.x_axis").expect("validated above"),
            y_axis: metadata.custom("cytiq.y_axis").expect("validated above"),
        })
    }
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

fn resolve_axis(
    parameters: &[Parameter],
    columns: &[ColumnData],
    axis: &str,
) -> Result<ResolvedAxis, GateError> {
    if let Some((index, parameter)) = parameters
        .iter()
        .enumerate()
        .find(|(_, parameter)| parameter.name == axis)
    {
        return Ok(ResolvedAxis {
            name: parameter.name.clone(),
            values: numeric_values(columns.get(index), axis)?,
        });
    }

    let label_matches = parameters
        .iter()
        .enumerate()
        .filter(|(_, parameter)| parameter.label.as_deref() == Some(axis))
        .collect::<Vec<_>>();

    match label_matches.as_slice() {
        [] => Err(GateError::MissingAxis {
            axis: axis.to_owned(),
        }),
        [(index, _parameter)] => Ok(ResolvedAxis {
            name: axis.to_owned(),
            values: numeric_values(columns.get(*index), axis)?,
        }),
        _ => Err(GateError::AmbiguousAxis {
            axis: axis.to_owned(),
        }),
    }
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

fn prepare_input(x_axis: &ResolvedAxis, y_axis: &ResolvedAxis) -> Result<PreparedInput, GateError> {
    if x_axis.values.is_empty() || y_axis.values.is_empty() {
        return Err(GateError::EmptyDataset);
    }
    if x_axis.values.len() != y_axis.values.len() {
        return Err(GateError::MismatchedEventCount {
            x_count: x_axis.values.len(),
            y_count: y_axis.values.len(),
        });
    }

    let x_norm = normalize_axis(&x_axis.values, &x_axis.name)?;
    let y_norm = normalize_axis(&y_axis.values, &y_axis.name)?;

    let mut grid = Array2::<f32>::zeros((GRID_SIZE, GRID_SIZE));
    let mut events = Vec::with_capacity(x_axis.values.len());
    for (index, ((x_value, x_bin), (y_value, y_bin))) in x_axis
        .values
        .iter()
        .zip(&x_norm)
        .zip(y_axis.values.iter().zip(&y_norm))
        .enumerate()
    {
        grid[[GRID_SIZE - 1 - usize::from(*x_bin), usize::from(*y_bin)]] += 1.0;
        events.push(PreparedEvent {
            event_index: i64::try_from(index).expect("event count exceeds i64"),
            x_value: *x_value,
            y_value: *y_value,
            x_bin: *x_bin,
            y_bin: *y_bin,
        });
    }

    let max_count = grid.iter().copied().fold(0.0_f32, f32::max);
    let normalized = grid.mapv(|value| value / max_count);
    let tensor = normalized
        .into_shape_with_order((1, 1, GRID_SIZE, GRID_SIZE))
        .expect("fixed grid shape should always reshape");

    Ok(PreparedInput { events, tensor })
}

fn normalize_axis(values: &[f64], axis: &str) -> Result<Vec<u8>, GateError> {
    if values.is_empty() {
        return Err(GateError::EmptyDataset);
    }
    if values.iter().any(|value| !value.is_finite()) {
        return Err(GateError::NonFiniteAxis {
            axis: axis.to_owned(),
        });
    }

    let min = values.iter().copied().fold(f64::INFINITY, f64::min);
    let max = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
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
    mask: &Array2<bool>,
) -> Vec<PredictionRow> {
    events
        .iter()
        .map(|event| PredictionRow {
            event_index: event.event_index,
            gate_name: metadata.gate_name.clone(),
            x_axis: metadata.x_axis.clone(),
            y_axis: metadata.y_axis.clone(),
            x_value: event.x_value,
            y_value: event.y_value,
            x_bin: event.x_bin,
            y_bin: event.y_bin,
            prediction: mask[[
                GRID_SIZE - 1 - usize::from(event.x_bin),
                usize::from(event.y_bin),
            ]],
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

    #[test]
    fn normalizes_with_bankers_rounding() {
        let values = [0.0, 0.5, 1.0, 2.5];
        let normalized = normalize_axis(&values, "FSC-A").expect("normalization should succeed");

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

        let prepared = prepare_input(&x_axis, &y_axis).expect("input should prepare");
        assert_eq!(prepared.tensor.shape(), &[1, 1, GRID_SIZE, GRID_SIZE]);

        let mut mask = Array2::<bool>::from_elem((GRID_SIZE, GRID_SIZE), false);
        mask[[GRID_SIZE - 1, 0]] = true;
        mask[[0, 50]] = true;

        let metadata = GateMetadata {
            gate_name: "ExampleGate".to_owned(),
            x_axis: "X".to_owned(),
            y_axis: "Y".to_owned(),
        };
        let rows = project_predictions(&metadata, &prepared.events, &mask);

        assert_eq!(rows.len(), 3);
        assert!(rows[0].prediction);
        assert!(rows[1].prediction);
        assert!(!rows[2].prediction);
    }
}
