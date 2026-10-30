use std::fs::File;
use std::path::PathBuf;

use arrow_array::{Array, Float32Array, StringArray};
use cytiq_classify::{Bundle, ClassifyConfig, ClassifyRunner, feature_columns};
use cytiq_fcs::FcsReader;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use serde_json::Value;

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data/fixtures/classify/cytof")
}

fn expected() -> Value {
    serde_json::from_reader(File::open(fixtures().join("expected.json")).unwrap()).unwrap()
}

fn dataset(name: &str) -> cytiq_fcs::FcsDataset {
    FcsReader::open(fixtures().join(name))
        .unwrap()
        .read()
        .unwrap()
}

#[test]
fn cytof_features_match_the_python_recipe() {
    let bundle = Bundle::load(&fixtures().join("lightgbm"), 0.0).unwrap();
    assert_eq!(bundle.profile.recipe, "cytiq-cytof-arcsinh-v1");
    let columns = feature_columns(&dataset("input.fcs"), &bundle.profile).unwrap();
    let expected = expected();
    let rows = expected["features"].as_array().unwrap();
    assert_eq!(rows.len(), 192);
    for (row, values) in rows.iter().enumerate() {
        for (column, value) in values.as_array().unwrap().iter().enumerate() {
            assert!((f64::from(columns[column][row]) - value.as_f64().unwrap()).abs() < 1e-5);
        }
    }
}

#[test]
fn lightgbm_and_xgboost_cytof_bundles_match_native_python() {
    let temp = tempfile::tempdir().unwrap();
    let expected = expected();
    for kind in ["lightgbm", "xgboost"] {
        let cfg = ClassifyConfig {
            input: fixtures().join("input.fcs"),
            model: fixtures().join(kind),
            output: temp.path().join(kind),
            masks: vec![],
            threads: Some(1),
            batch_size: 17,
            abstain_margin: 0.0,
            overwrite: false,
        };
        let result = ClassifyRunner::run(&cfg).unwrap();
        assert_eq!(result.assessed, 192);
        let bundle = Bundle::load(&cfg.model, 0.0).unwrap();
        let probabilities = expected["probabilities"][kind].as_array().unwrap();
        let reader = ParquetRecordBatchReaderBuilder::try_new(
            File::open(cfg.output.join("predictions.parquet")).unwrap(),
        )
        .unwrap()
        .build()
        .unwrap();
        let mut row = 0;
        for batch in reader {
            let batch = batch.unwrap();
            let labels = batch
                .column_by_name("label")
                .unwrap()
                .as_any()
                .downcast_ref::<StringArray>()
                .unwrap();
            for index in 0..batch.num_rows() {
                let python = probabilities[row].as_array().unwrap();
                let mut best = 0;
                for (column, id) in bundle.classes.iter().enumerate() {
                    let values = batch
                        .column_by_name(&format!("p_{id}"))
                        .unwrap()
                        .as_any()
                        .downcast_ref::<Float32Array>()
                        .unwrap();
                    let reference = python[column].as_f64().unwrap();
                    assert!((f64::from(values.value(index)) - reference).abs() <= 1e-5);
                    if reference > python[best].as_f64().unwrap() {
                        best = column;
                    }
                }
                assert_eq!(labels.value(index), bundle.classes[best]);
                row += 1;
            }
        }
        assert_eq!(row, 192, "{kind}");
    }
}

#[test]
fn cytof_recipe_rejects_spillover_and_table_profiles() {
    let bundle = Bundle::load(&fixtures().join("lightgbm"), 0.0).unwrap();
    let error = feature_columns(&dataset("spill.fcs"), &bundle.profile).unwrap_err();
    assert!(error.to_string().contains("spillover"), "{error}");
    let mut table = bundle.profile.clone();
    table.recipe = "cytiq-table-stored-v1".into();
    let error = feature_columns(&dataset("input.fcs"), &table).unwrap_err();
    assert!(error.to_string().contains("training-only"), "{error}");
}

#[test]
fn cytof_profiles_match_the_instrument_family_not_the_exact_software_string() {
    let bundle = Bundle::load(&fixtures().join("lightgbm"), 0.0).unwrap();
    let mut family = bundle.profile.clone();
    family.instrument = "helio".into();
    assert!(feature_columns(&dataset("input.fcs"), &family).is_ok());
    family.instrument = "CyTOF".into();
    let error = feature_columns(&dataset("input.fcs"), &family).unwrap_err();
    assert!(error.to_string().contains("instrument"), "{error}");
}
