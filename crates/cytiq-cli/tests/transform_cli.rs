use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use cytiq_fcs::{ColumnData, FcsReader, FcsWriteParameter};

#[test]
fn cli_runs_transform_default_arcsinh_with_metadata() {
    let directory = create_temp_dir("cytiq-cli-transform-default");
    let input = directory.join("sample.fcs");
    let output_path = directory.join("sample.transformed.fcs");
    let metadata_path = directory.join("metadata.json");
    let debug_dir = directory.join("debug");
    write_transform_fixture_fcs(&input, 0.0);

    let output = Command::new(env!("CARGO_BIN_EXE_cytiq"))
        .args([
            "transform",
            "--input",
            utf8(&input),
            "--output",
            utf8(&output_path),
            "--metadata-json",
            utf8(&metadata_path),
            "--debug-dir",
            utf8(&debug_dir),
        ])
        .output()
        .expect("command should run");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let dataset = FcsReader::open(&output_path)
        .and_then(FcsReader::read)
        .expect("transformed FCS should be readable");
    assert!(dataset.text.get("CYTIQ_TRANSFORM").is_some());
    assert_f32_column(dataset.data.column(0), &[0.0, 1.0, 2.0]);
    assert_f32_column(dataset.data.column(1), &[10.0, 11.0, 12.0]);
    assert_f32_column_close(
        dataset.data.column(3),
        &[
            (0.0_f64 / 150.0).asinh(),
            (150.0_f64 / 150.0).asinh(),
            (300.0_f64 / 150.0).asinh(),
        ],
    );

    let metadata = fs::read_to_string(metadata_path).expect("metadata should exist");
    assert!(metadata.contains("\"method\": \"arcsinh\""));
    assert!(metadata.contains("\"channels\""));
    assert!(debug_dir.join("summary.json").is_file());
    assert!(debug_dir.join("metrics.csv").is_file());
    let report = fs::read_to_string(debug_dir.join("report.html")).expect("report should exist");
    assert!(report.contains("plotly"));
    let metrics = fs::read_to_string(debug_dir.join("metrics.csv")).expect("metrics should exist");
    assert!(metrics.contains("raw_mean"));
    assert!(metrics.contains("transformed_mean"));
}

#[test]
fn cli_transforms_repeated_inputs_to_output_directory() {
    let directory = create_temp_dir("cytiq-cli-transform-repeated");
    let input_a = directory.join("sample-a.fcs");
    let input_b = directory.join("sample-b.fcs");
    let output_dir = directory.join("out");
    write_transform_fixture_fcs(&input_a, 0.0);
    write_transform_fixture_fcs(&input_b, 10.0);

    let output = Command::new(env!("CARGO_BIN_EXE_cytiq"))
        .args([
            "transform",
            "--input",
            utf8(&input_a),
            "--input",
            utf8(&input_b),
            "--output",
            utf8(&output_dir),
            "--method",
            "none",
        ])
        .output()
        .expect("command should run");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output_dir.join("sample-a.transformed.fcs").is_file());
    assert!(output_dir.join("sample-b.transformed.fcs").is_file());
}

#[test]
fn cli_transforms_glob_inputs_to_output_directory() {
    let directory = create_temp_dir("cytiq-cli-transform-glob");
    write_transform_fixture_fcs(&directory.join("sample-1.fcs"), 0.0);
    write_transform_fixture_fcs(&directory.join("sample-2.fcs"), 10.0);
    fs::write(directory.join("notes.txt"), "ignored").expect("notes should write");
    let output_dir = directory.join("out");
    let glob = directory.join("sample-*.fcs");

    let output = Command::new(env!("CARGO_BIN_EXE_cytiq"))
        .args([
            "transform",
            "--input",
            utf8(&glob),
            "--output",
            utf8(&output_dir),
            "--method",
            "none",
        ])
        .output()
        .expect("command should run");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output_dir.join("sample-1.transformed.fcs").is_file());
    assert!(output_dir.join("sample-2.transformed.fcs").is_file());
}

#[test]
fn cli_applies_channels_and_exclude_channels() {
    let directory = create_temp_dir("cytiq-cli-transform-channels");
    let input = directory.join("sample.fcs");
    let output_path = directory.join("sample.transformed.fcs");
    write_transform_fixture_fcs(&input, 0.0);

    let output = Command::new(env!("CARGO_BIN_EXE_cytiq"))
        .args([
            "transform",
            "--input",
            utf8(&input),
            "--output",
            utf8(&output_path),
            "--channels",
            "CD3,CD4",
            "--exclude-channels",
            "CD4",
        ])
        .output()
        .expect("command should run");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let dataset = FcsReader::open(&output_path)
        .and_then(FcsReader::read)
        .expect("transformed FCS should be readable");
    assert_f32_column_close(
        dataset.data.column(3),
        &[
            (0.0_f64 / 150.0).asinh(),
            (150.0_f64 / 150.0).asinh(),
            (300.0_f64 / 150.0).asinh(),
        ],
    );
    assert_f32_column(dataset.data.column(4), &[30.0, 180.0, 330.0]);
}

#[test]
fn cli_runs_logicle_biexponential_and_hyperlog_methods() {
    for method in ["logicle", "biexponential", "hyperlog"] {
        let directory = create_temp_dir(&format!("cytiq-cli-transform-{method}"));
        let input = directory.join("sample.fcs");
        let output_path = directory.join(format!("sample.{method}.fcs"));
        write_transform_fixture_fcs(&input, 0.0);

        let output = Command::new(env!("CARGO_BIN_EXE_cytiq"))
            .args([
                "transform",
                "--input",
                utf8(&input),
                "--output",
                utf8(&output_path),
                "--method",
                method,
                "--channels",
                "CD3,CD4",
                "--threads",
                "2",
            ])
            .output()
            .expect("command should run");

        assert!(
            output.status.success(),
            "method {method} stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let dataset = FcsReader::open(&output_path)
            .and_then(FcsReader::read)
            .expect("transformed FCS should be readable");
        let Some(ColumnData::F32(values)) = dataset.data.column(3) else {
            panic!("expected f32 column");
        };
        assert!(values.iter().all(|value| value.is_finite()));
        assert_ne!(values, &[0.0, 150.0, 300.0]);
    }
}

#[test]
fn cli_dry_run_writes_no_outputs() {
    let directory = create_temp_dir("cytiq-cli-transform-dry-run");
    let input = directory.join("sample.fcs");
    let output_path = directory.join("sample.transformed.fcs");
    let metadata_path = directory.join("metadata.json");
    write_transform_fixture_fcs(&input, 0.0);

    let output = Command::new(env!("CARGO_BIN_EXE_cytiq"))
        .args([
            "transform",
            "--input",
            utf8(&input),
            "--output",
            utf8(&output_path),
            "--metadata-json",
            utf8(&metadata_path),
            "--dry-run",
        ])
        .output()
        .expect("command should run");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("cytiq transform dry run"));
    assert!(stdout.contains("channels: FL1-A,FL2-A"));
    assert!(!output_path.exists());
    assert!(!metadata_path.exists());
}

#[test]
fn cli_rejects_existing_transform_output_without_overwrite() {
    let directory = create_temp_dir("cytiq-cli-transform-output-exists");
    let input = directory.join("sample.fcs");
    let output_path = directory.join("sample.transformed.fcs");
    write_transform_fixture_fcs(&input, 0.0);
    write_transform_fixture_fcs(&output_path, 0.0);

    let output = Command::new(env!("CARGO_BIN_EXE_cytiq"))
        .args([
            "transform",
            "--input",
            utf8(&input),
            "--output",
            utf8(&output_path),
        ])
        .output()
        .expect("command should run");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("already exists"));
}

#[test]
fn cli_rejects_existing_metadata_before_writing_transform_output() {
    let directory = create_temp_dir("cytiq-cli-transform-metadata-exists");
    let input = directory.join("sample.fcs");
    let output_path = directory.join("sample.transformed.fcs");
    let metadata_path = directory.join("metadata.json");
    write_transform_fixture_fcs(&input, 0.0);
    fs::write(&metadata_path, "{}").expect("metadata placeholder should write");

    let output = Command::new(env!("CARGO_BIN_EXE_cytiq"))
        .args([
            "transform",
            "--input",
            utf8(&input),
            "--output",
            utf8(&output_path),
            "--metadata-json",
            utf8(&metadata_path),
        ])
        .output()
        .expect("command should run");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("metadata output"));
    assert!(!output_path.exists());
}

#[test]
fn cli_requires_transform_input() {
    let directory = create_temp_dir("cytiq-cli-transform-requires-input");

    let output = Command::new(env!("CARGO_BIN_EXE_cytiq"))
        .args([
            "transform",
            "--output",
            utf8(&directory.join("sample.transformed.fcs")),
        ])
        .output()
        .expect("command should run");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("--input"));
}

#[test]
fn cli_rejects_unsupported_transform_format() {
    let directory = create_temp_dir("cytiq-cli-transform-format");
    let input = directory.join("sample.fcs");
    write_transform_fixture_fcs(&input, 0.0);

    let output = Command::new(env!("CARGO_BIN_EXE_cytiq"))
        .args([
            "transform",
            "--input",
            utf8(&input),
            "--output",
            utf8(&directory.join("sample.csv")),
            "--format",
            "csv",
        ])
        .output()
        .expect("command should run");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("not supported"));
}

fn write_transform_fixture_fcs(path: &Path, offset: f32) {
    let columns = vec![
        vec![0.0, 1.0, 2.0],
        vec![10.0 + offset, 11.0 + offset, 12.0 + offset],
        vec![20.0 + offset, 21.0 + offset, 22.0 + offset],
        vec![0.0 + offset, 150.0 + offset, 300.0 + offset],
        vec![30.0 + offset, 180.0 + offset, 330.0 + offset],
    ];
    let parameters = vec![
        FcsWriteParameter::new("Time"),
        FcsWriteParameter::new("FSC-A"),
        FcsWriteParameter::new("SSC-A"),
        FcsWriteParameter::with_label("FL1-A", "CD3"),
        FcsWriteParameter::with_label("FL2-A", "CD4"),
    ];
    cytiq_fcs::write_fcs3_1_f32(path, &parameters, &columns).expect("fixture FCS should write");
}

fn assert_f32_column(column: Option<&ColumnData>, expected: &[f32]) {
    let Some(ColumnData::F32(values)) = column else {
        panic!("expected f32 column");
    };
    assert_eq!(values, expected);
}

fn assert_f32_column_close(column: Option<&ColumnData>, expected: &[f64]) {
    let Some(ColumnData::F32(values)) = column else {
        panic!("expected f32 column");
    };
    assert_eq!(values.len(), expected.len());
    for (actual, expected) in values.iter().zip(expected) {
        assert!(
            (f64::from(*actual) - expected).abs() <= 1.0e-5,
            "expected {expected}, got {actual}"
        );
    }
}

fn create_temp_dir(label: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should be after unix epoch")
        .as_nanos();
    let directory = std::env::temp_dir().join(format!("cytiq-{label}-{unique}"));
    fs::create_dir_all(&directory).expect("temporary directory should be created");
    directory
}

fn utf8(path: &Path) -> &str {
    path.to_str().expect("path should be valid utf-8")
}
