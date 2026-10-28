use std::fs::{self, File};
use std::path::{Path, PathBuf};

use arrow_array::{Array, Float32Array, StringArray};
use cytiq_classify::{Bundle, ClassifyConfig, ClassifyRunner};
use cytiq_fcs::FcsReader;
use cytiq_mask::EventMask;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use serde_json::Value;

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data/fixtures/classify/v2")
}

fn config(kind: &str, output: PathBuf) -> ClassifyConfig {
    ClassifyConfig {
        input: fixtures().join("input.fcs"),
        model: fixtures().join(kind),
        output,
        masks: vec![],
        threads: Some(1),
        batch_size: 17,
        abstain_margin: 0.0,
        overwrite: false,
    }
}

#[test]
fn both_multiclass_models_match_native_python_and_stable_column_order() {
    let temp = tempfile::tempdir().unwrap();
    for kind in ["catboost", "lightgbm"] {
        let cfg = config(kind, temp.path().join(kind));
        let result = ClassifyRunner::run(&cfg).unwrap();
        assert_eq!(result.schema_version, 2);
        assert_eq!(result.assessed, 192);
        assert_eq!(
            result
                .class_counts
                .as_ref()
                .unwrap()
                .values()
                .sum::<usize>(),
            192
        );
        let expected: Vec<Vec<f64>> = serde_json::from_reader(
            File::open(fixtures().join(format!("{kind}.expected.json"))).unwrap(),
        )
        .unwrap();
        let bundle = Bundle::load(&cfg.model, 0.0).unwrap();
        let reader = ParquetRecordBatchReaderBuilder::try_new(
            File::open(cfg.output.join("predictions.parquet")).unwrap(),
        )
        .unwrap()
        .build()
        .unwrap();
        let mut row = 0;
        for batch in reader {
            let batch = batch.unwrap();
            for index in 0..batch.num_rows() {
                let mut best = 0;
                for (column, id) in bundle.classes.iter().enumerate() {
                    assert_eq!(batch.schema().field(3 + column).name(), &format!("p_{id}"));
                    let values = batch
                        .column(3 + column)
                        .as_any()
                        .downcast_ref::<Float32Array>()
                        .unwrap();
                    assert!((f64::from(values.value(index)) - expected[row][column]).abs() <= 1e-5);
                    if expected[row][column] > expected[row][best] {
                        best = column;
                    }
                }
                let labels = batch
                    .column(9)
                    .as_any()
                    .downcast_ref::<StringArray>()
                    .unwrap();
                assert_eq!(labels.value(index), bundle.classes[best]);
                row += 1;
            }
        }
        assert_eq!(row, 192);
        let summary: Value =
            serde_json::from_reader(File::open(cfg.output.join("summary.json")).unwrap()).unwrap();
        assert!(summary.get("blast_fraction").is_none());
        assert_eq!(summary["class_counts"].as_object().unwrap().len(), 6);
    }
}

#[test]
fn masks_preserve_all_multiclass_rows_and_null_every_probability() {
    let temp = tempfile::tempdir().unwrap();
    let mut cfg = config("catboost", temp.path().join("masked"));
    let dataset = FcsReader::open(&cfg.input).unwrap().read().unwrap();
    let acquisition = cytiq_fcs::acquisition_id(&cfg.input, &dataset.text).unwrap();
    let path = temp.path().join("mask.parquet");
    EventMask::new(acquisition, (0..192).map(|i| i % 2 == 0).collect(), None)
        .write_parquet(&path)
        .unwrap();
    cfg.masks.push(path);
    let result = ClassifyRunner::run(&cfg).unwrap();
    assert_eq!(result.assessed, 96);
    let reader = ParquetRecordBatchReaderBuilder::try_new(
        File::open(cfg.output.join("predictions.parquet")).unwrap(),
    )
    .unwrap()
    .build()
    .unwrap();
    let mut row = 0;
    for batch in reader {
        let batch = batch.unwrap();
        for index in 0..batch.num_rows() {
            for column in 3..10 {
                assert_eq!(batch.column(column).is_null(index), row % 2 != 0);
            }
            row += 1;
        }
    }
    assert_eq!(row, 192);
}

fn copy_bundle(destination: &Path) {
    fs::create_dir_all(destination).unwrap();
    for file in fs::read_dir(fixtures().join("catboost")).unwrap() {
        let file = file.unwrap();
        fs::copy(file.path(), destination.join(file.file_name())).unwrap();
    }
}

#[test]
fn rejects_edited_knowledge_before_creating_output() {
    let temp = tempfile::tempdir().unwrap();
    let mut cfg = config("catboost", temp.path().join("output"));
    cfg.model = temp.path().join("bundle");
    copy_bundle(&cfg.model);
    fs::write(cfg.model.join("classes.json"), "{}").unwrap();
    assert!(
        ClassifyRunner::run(&cfg)
            .unwrap_err()
            .to_string()
            .contains("checksum")
    );
    assert!(!cfg.output.exists());
}

#[test]
fn rejects_wrong_onnx_class_dimension_without_publishing_predictions() {
    let temp = tempfile::tempdir().unwrap();
    let mut cfg = config("catboost", temp.path().join("output"));
    cfg.model = temp.path().join("bundle");
    copy_bundle(&cfg.model);
    fs::copy(
        fixtures().join("../catboost/model.onnx"),
        cfg.model.join("model.onnx"),
    )
    .unwrap();
    let path = cfg.model.join("manifest.json");
    let mut manifest: Value = serde_json::from_reader(File::open(&path).unwrap()).unwrap();
    manifest["model_sha256"] = cytiq_provenance::hash_file(&cfg.model.join("model.onnx"))
        .unwrap()
        .1
        .into();
    serde_json::to_writer(File::create(path).unwrap(), &manifest).unwrap();
    assert!(
        ClassifyRunner::run(&cfg)
            .unwrap_err()
            .to_string()
            .contains("probabilities")
    );
    assert!(!cfg.output.join("predictions.parquet").exists());
}

#[test]
fn rejects_binary_margin_for_multiclass() {
    let temp = tempfile::tempdir().unwrap();
    let mut cfg = config("catboost", temp.path().join("output"));
    cfg.abstain_margin = 0.05;
    assert!(
        ClassifyRunner::run(&cfg)
            .unwrap_err()
            .to_string()
            .contains("multiclass")
    );
    assert!(!cfg.output.exists());
}
