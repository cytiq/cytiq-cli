//! Supervised FCS event classification using a portable model bundle.
mod features;

use std::collections::HashMap;
use std::fs::{self, File};
use std::path::PathBuf;
use std::sync::Arc;

use arrow_array::{ArrayRef, BooleanArray, Float32Array, RecordBatch, StringArray, UInt64Array};
use arrow_schema::{DataType, Field, Schema};
use cytiq_fcs::FcsReader;
use ort::{session::Session, value::Tensor};
use parquet::arrow::ArrowWriter;
use serde::{Deserialize, Serialize};

pub use features::{Feature, Profile, feature_columns};

pub const DEFAULT_BATCH_SIZE: usize = 65_536;
pub const UPSTREAM: &[(&str, &str)] = &[("ONNX Runtime", "1.28.0")];

#[derive(Debug, thiserror::Error)]
pub enum ClassifyError {
    #[error("{0}")]
    Invalid(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Fcs(#[from] cytiq_fcs::Error),
    #[error(transparent)]
    Mask(#[from] cytiq_mask::MaskError),
    #[error(transparent)]
    Arrow(#[from] arrow_schema::ArrowError),
    #[error(transparent)]
    Parquet(#[from] parquet::errors::ParquetError),
    #[error(transparent)]
    Ort(#[from] ort::Error),
}

#[derive(Clone, Debug, Serialize)]
pub struct ClassifyConfig {
    pub input: PathBuf,
    pub model: PathBuf,
    pub output: PathBuf,
    pub masks: Vec<PathBuf>,
    pub threads: Option<usize>,
    pub batch_size: usize,
    pub abstain_margin: f32,
    pub overwrite: bool,
}

#[derive(Debug, Deserialize)]
pub struct Bundle {
    pub schema_version: u32,
    pub profile: Profile,
    pub threshold: f32,
    pub classes: Vec<String>,
    pub input: String,
    pub output: String,
    pub model_sha256: String,
    pub run_id: String,
}

#[derive(Debug, Serialize)]
pub struct ClassifyRunResult {
    pub schema_version: u32,
    pub acquisition_sha256: String,
    pub model_sha256: String,
    pub run_id: String,
    pub events: usize,
    pub assessed: usize,
    pub blast: usize,
    pub non_blast: usize,
    pub unknown: usize,
    pub blast_fraction: Option<f64>,
    pub non_blast_fraction: Option<f64>,
    pub unknown_fraction: Option<f64>,
    pub threshold: f32,
    pub abstain_margin: f32,
}

pub struct ClassifyRunner;

impl ClassifyRunner {
    /// Classify one FCS, preserving input event order and lineage.
    ///
    /// # Errors
    /// Returns a typed error for incompatible inputs, invalid bundles or output failures.
    #[allow(
        clippy::too_many_lines,
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation
    )]
    pub fn run(config: &ClassifyConfig) -> Result<ClassifyRunResult, ClassifyError> {
        let invalid = |s: &str| ClassifyError::Invalid(s.to_owned());
        if config.batch_size == 0
            || config.threads == Some(0)
            || !config.abstain_margin.is_finite()
            || config.abstain_margin < 0.0
        {
            return Err(invalid("invalid batch size, threads or abstain margin"));
        }
        let bundle: Bundle =
            serde_json::from_reader(File::open(config.model.join("manifest.json"))?)?;
        if bundle.schema_version != 1
            || bundle.classes != ["non_blast", "blast"]
            || bundle.input != "features"
            || bundle.output != "probabilities"
            || !bundle.threshold.is_finite()
            || !(0.0..=1.0).contains(&bundle.threshold)
            || config.abstain_margin > bundle.threshold.min(1.0 - bundle.threshold)
        {
            return Err(invalid(
                "incompatible bundle or abstain margin around threshold",
            ));
        }
        let model = config.model.join("model.onnx");
        if cytiq_provenance::hash_file(&model)?.1 != bundle.model_sha256 {
            return Err(invalid("model checksum mismatch"));
        }
        for name in ["predictions.parquet", "summary.json", "provenance.json"] {
            if config.output.join(name).exists() && !config.overwrite {
                return Err(invalid("output exists; pass --overwrite to replace it"));
            }
        }
        let dataset = FcsReader::open(&config.input)?.read()?;
        let acquisition = cytiq_fcs::acquisition_id(&config.input, &dataset.text)?;
        let keep = cytiq_mask::read_combined(&config.masks)?
            .map(|m| m.keep_for(&dataset, &acquisition))
            .transpose()?
            .unwrap_or_else(|| vec![true; dataset.event_count()]);
        let source = cytiq_fcs::source_event_ids(&dataset)?
            .unwrap_or_else(|| (0..dataset.event_count() as u64).collect());
        let columns = feature_columns(&dataset, &bundle.profile)?;
        let mut builder = Session::builder()?;
        if let Some(threads) = config.threads {
            builder = builder
                .with_intra_threads(threads)
                .map_err(|e| invalid(&e.to_string()))?;
        }
        let mut session = builder.commit_from_file(model)?;
        if session.inputs().len() != 1 || session.inputs()[0].name() != bundle.input {
            return Err(invalid("invalid ONNX feature input"));
        }
        let schema = Arc::new(Schema::new_with_metadata(
            vec![
                Field::new("event_id", DataType::UInt64, false),
                Field::new("source_event_id", DataType::UInt64, false),
                Field::new("assessed", DataType::Boolean, false),
                Field::new("p_non_blast", DataType::Float32, true),
                Field::new("p_blast", DataType::Float32, true),
                Field::new("label", DataType::Utf8, true),
            ],
            HashMap::from([
                ("cytiq.classify.schema_version".into(), "1".into()),
                ("cytiq.acquisition_sha256".into(), acquisition.clone()),
                ("cytiq.event_id_base".into(), "0".into()),
            ]),
        ));
        fs::create_dir_all(&config.output)?;
        let temp = tempfile::tempdir_in(&config.output)?;
        let mut writer = ArrowWriter::try_new(
            File::create(temp.path().join("predictions.parquet"))?,
            schema.clone(),
            None,
        )?;
        let mut result = ClassifyRunResult {
            schema_version: 1,
            acquisition_sha256: acquisition,
            model_sha256: bundle.model_sha256,
            run_id: bundle.run_id,
            events: dataset.event_count(),
            assessed: 0,
            blast: 0,
            non_blast: 0,
            unknown: 0,
            blast_fraction: None,
            non_blast_fraction: None,
            unknown_fraction: None,
            threshold: bundle.threshold,
            abstain_margin: config.abstain_margin,
        };
        for start in (0..dataset.event_count()).step_by(config.batch_size) {
            let end = (start + config.batch_size).min(dataset.event_count());
            let selected = (start..end).filter(|i| keep[*i]).collect::<Vec<_>>();
            let mut probabilities = vec![None; end - start];
            if !selected.is_empty() {
                let values = selected
                    .iter()
                    .flat_map(|i| columns.iter().map(move |c| c[*i]))
                    .collect::<Vec<_>>();
                let tensor = Tensor::from_array(([selected.len(), columns.len()], values))?;
                let outputs = session.run(ort::inputs!["features" => tensor])?;
                let probabilities_output = outputs
                    .get("probabilities")
                    .ok_or_else(|| invalid("ONNX probability output missing"))?;
                let (shape, data) = probabilities_output.try_extract_tensor::<f32>()?;
                let rows = i64::try_from(selected.len()).map_err(|_| invalid("batch too large"))?;
                if shape.as_ref() != [rows, 2]
                    || data
                        .iter()
                        .any(|p| !p.is_finite() || !(0.0..=1.0).contains(p))
                {
                    return Err(invalid("invalid ONNX probabilities"));
                }
                for (i, pair) in selected.iter().zip(data.as_chunks::<2>().0) {
                    if (pair[0] + pair[1] - 1.0).abs() > 1e-5 {
                        return Err(invalid("ONNX probabilities do not sum to one"));
                    }
                    probabilities[*i - start] = Some(pair[1]);
                }
            }
            let labels = probabilities
                .iter()
                .map(|p| {
                    p.map(|p| {
                        result.assessed += 1;
                        if config.abstain_margin > 0.0
                            && (p - bundle.threshold).abs() < config.abstain_margin
                        {
                            result.unknown += 1;
                            "unknown"
                        } else if p >= bundle.threshold {
                            result.blast += 1;
                            "blast"
                        } else {
                            result.non_blast += 1;
                            "non_blast"
                        }
                    })
                })
                .collect::<Vec<_>>();
            let arrays: Vec<ArrayRef> = vec![
                Arc::new(UInt64Array::from_iter_values(start as u64..end as u64)),
                Arc::new(UInt64Array::from(source[start..end].to_vec())),
                Arc::new(BooleanArray::from(keep[start..end].to_vec())),
                Arc::new(Float32Array::from(
                    probabilities
                        .iter()
                        .map(|p| p.map(|v| 1.0 - v))
                        .collect::<Vec<_>>(),
                )),
                Arc::new(Float32Array::from(probabilities)),
                Arc::new(StringArray::from(labels)),
            ];
            writer.write(&RecordBatch::try_new(schema.clone(), arrays)?)?;
        }
        writer.close()?;
        if result.assessed > 0 {
            result.blast_fraction = Some(result.blast as f64 / result.assessed as f64);
            result.non_blast_fraction = Some(result.non_blast as f64 / result.assessed as f64);
            result.unknown_fraction = Some(result.unknown as f64 / result.assessed as f64);
        }
        serde_json::to_writer_pretty(File::create(temp.path().join("summary.json"))?, &result)?;
        for name in ["predictions.parquet", "summary.json"] {
            fs::rename(temp.path().join(name), config.output.join(name))?;
        }
        Ok(result)
    }
}
