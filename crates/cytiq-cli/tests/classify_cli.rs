use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use arrow_array::Float32Array;
use cytiq_fcs::FcsReader;
use cytiq_mask::EventMask;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data/fixtures/classify")
}

fn run(input: &Path, model: &Path, output: &Path, extra: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_cytiq"))
        .args(["classify", "--input"])
        .arg(input)
        .arg("--model")
        .arg(model)
        .arg("--output")
        .arg(output)
        .args(extra)
        .output()
        .unwrap()
}

fn probabilities(output: &Path) -> Vec<Option<f32>> {
    let reader = ParquetRecordBatchReaderBuilder::try_new(
        File::open(output.join("predictions.parquet")).unwrap(),
    )
    .unwrap()
    .build()
    .unwrap();
    let mut values = Vec::new();
    for batch in reader {
        let batch = batch.unwrap();
        let array = batch
            .column(4)
            .as_any()
            .downcast_ref::<Float32Array>()
            .unwrap();
        values.extend(array.iter());
    }
    values
}

#[test]
fn both_models_match_python_and_record_provenance() {
    let temp = tempfile::tempdir().unwrap();
    let expected: serde_json::Value =
        serde_json::from_reader(File::open(fixtures().join("expected.json")).unwrap()).unwrap();
    for kind in ["catboost", "lightgbm"] {
        let output = temp.path().join(kind);
        let result = run(
            &fixtures().join("raw.fcs"),
            &fixtures().join(kind),
            &output,
            &["--batch-size", "17", "--threads", "1"],
        );
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let actual = probabilities(&output);
        for (actual, expected) in actual
            .iter()
            .zip(expected["probabilities"][kind].as_array().unwrap())
        {
            assert!((f64::from(actual.unwrap()) - expected.as_f64().unwrap()).abs() < 1e-5);
        }
        assert_eq!(actual.len(), 128);
        let provenance: serde_json::Value =
            serde_json::from_reader(File::open(output.join("provenance.json")).unwrap()).unwrap();
        assert_eq!(provenance["tool"], "cytiq classify");
        let repeated = run(
            &fixtures().join("raw.fcs"),
            &fixtures().join(kind),
            &output,
            &[],
        );
        assert!(!repeated.status.success());
    }
}

#[test]
fn masks_keep_event_rows_and_unknown_is_optional() {
    let temp = tempfile::tempdir().unwrap();
    let input = fixtures().join("raw.fcs");
    let dataset = FcsReader::open(&input).unwrap().read().unwrap();
    let acquisition = cytiq_fcs::acquisition_id(&input, &dataset.text).unwrap();
    let mask = temp.path().join("mask.parquet");
    EventMask::new(acquisition, (0..128).map(|i| i % 2 == 0).collect(), None)
        .write_parquet(&mask)
        .unwrap();
    let output = temp.path().join("masked");
    let result = run(
        &input,
        &fixtures().join("catboost"),
        &output,
        &["--mask", mask.to_str().unwrap(), "--abstain-margin", "0.49"],
    );
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let values = probabilities(&output);
    assert_eq!(values.len(), 128);
    for (i, value) in values.iter().enumerate() {
        assert_eq!(value.is_some(), i % 2 == 0);
    }
    let summary: serde_json::Value =
        serde_json::from_reader(File::open(output.join("summary.json")).unwrap()).unwrap();
    assert_eq!(summary["assessed"], 64);
    assert_eq!(summary["unknown"], 64);

    EventMask::new(
        summary["acquisition_sha256"].as_str().unwrap(),
        vec![false; 128],
        None,
    )
    .write_parquet(&mask)
    .unwrap();
    let empty = temp.path().join("empty");
    assert!(
        run(
            &input,
            &fixtures().join("catboost"),
            &empty,
            &["--mask", mask.to_str().unwrap()]
        )
        .status
        .success()
    );
    assert!(probabilities(&empty).iter().all(Option::is_none));
}

#[test]
fn processed_cytiq_input_matches_raw_input() {
    let temp = tempfile::tempdir().unwrap();
    let compensated = temp.path().join("comp.fcs");
    let transformed = temp.path().join("transformed.fcs");
    let result = Command::new(env!("CARGO_BIN_EXE_cytiq"))
        .args(["compensate", "--input"])
        .arg(fixtures().join("raw.fcs"))
        .arg("--output")
        .arg(&compensated)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let result = Command::new(env!("CARGO_BIN_EXE_cytiq"))
        .args(["transform", "--input"])
        .arg(&compensated)
        .arg("--output")
        .arg(&transformed)
        .args([
            "--channels",
            "FITC-A,PE-A,PerCP-A,PC7-A,APC-A,APC-H7-A,Horizon V450-A,Horizon V500-A",
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let a = temp.path().join("a");
    let b = temp.path().join("b");
    assert!(
        run(
            &fixtures().join("raw.fcs"),
            &fixtures().join("catboost"),
            &a,
            &[]
        )
        .status
        .success()
    );
    let result = run(&transformed, &fixtures().join("catboost"), &b, &[]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(probabilities(&a), probabilities(&b));
}

#[test]
fn missing_input_and_invalid_flags_fail() {
    let temp = tempfile::tempdir().unwrap();
    assert!(
        !run(
            &temp.path().join("missing.fcs"),
            &fixtures().join("catboost"),
            temp.path(),
            &[]
        )
        .status
        .success()
    );
    let result = run(
        &fixtures().join("raw.fcs"),
        &fixtures().join("catboost"),
        temp.path(),
        &["--batch-size", "0"],
    );
    assert_eq!(result.status.code(), Some(2));
}

#[test]
fn incompatible_bundles_fail_without_prediction_files() {
    let temp = tempfile::tempdir().unwrap();
    let model = temp.path().join("model");
    std::fs::create_dir(&model).unwrap();
    std::fs::copy(
        fixtures().join("catboost/model.onnx"),
        model.join("model.onnx"),
    )
    .unwrap();
    let original: serde_json::Value =
        serde_json::from_reader(File::open(fixtures().join("catboost/manifest.json")).unwrap())
            .unwrap();
    for variant in 0..4 {
        let mut changed = original.clone();
        match variant {
            0 => changed["schema_version"] = 2.into(),
            1 => changed["model_sha256"] = "invalid".into(),
            2 => changed["profile"]["instrument"] = "other instrument".into(),
            _ => changed["profile"]["features"][0]["channel"] = "missing".into(),
        }
        serde_json::to_writer(File::create(model.join("manifest.json")).unwrap(), &changed)
            .unwrap();
        let output = temp.path().join(format!("invalid-{variant}"));
        assert!(
            !run(&fixtures().join("raw.fcs"), &model, &output, &[])
                .status
                .success()
        );
        assert!(!output.join("predictions.parquet").exists());
    }
}

#[test]
fn relocated_binary_uses_bundled_runtime_without_environment() {
    let temp = tempfile::tempdir().unwrap();
    let binary = PathBuf::from(env!("CARGO_BIN_EXE_cytiq"));
    let relocated = temp.path().join("cytiq");
    std::fs::copy(&binary, &relocated).unwrap();
    let library = temp.path().join("lib");
    std::fs::create_dir(&library).unwrap();
    for entry in std::fs::read_dir(binary.parent().unwrap().join("lib")).unwrap() {
        let entry = entry.unwrap();
        std::fs::copy(entry.path(), library.join(entry.file_name())).unwrap();
    }
    for kind in ["catboost", "lightgbm"] {
        let output = temp.path().join(kind);
        let result = Command::new(&relocated)
            .env_remove("ORT_DYLIB_PATH")
            .env_remove("DYLD_LIBRARY_PATH")
            .env_remove("LD_LIBRARY_PATH")
            .current_dir(temp.path())
            .arg("classify")
            .arg("--input")
            .arg(fixtures().join("raw.fcs"))
            .arg("--model")
            .arg(fixtures().join(kind))
            .arg("--output")
            .arg(&output)
            .args(["--threads", "1"])
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(probabilities(&output).len(), 128);
    }
    std::fs::remove_dir_all(library).unwrap();
    let missing = Command::new(&relocated)
        .env_remove("ORT_DYLIB_PATH")
        .env_remove("DYLD_LIBRARY_PATH")
        .env_remove("LD_LIBRARY_PATH")
        .arg("classify")
        .arg("--input")
        .arg(fixtures().join("raw.fcs"))
        .arg("--model")
        .arg(fixtures().join("lightgbm"))
        .arg("--output")
        .arg(temp.path().join("missing"))
        .output()
        .unwrap();
    assert!(!missing.status.success());
    assert!(
        String::from_utf8_lossy(&missing.stderr)
            .contains("keep the built lib/ directory beside cytiq")
    );
}
