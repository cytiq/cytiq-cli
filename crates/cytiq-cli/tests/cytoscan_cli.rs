use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use cytiq_fcs::{FcsWriteOptions, FcsWriteParameter};

#[test]
fn cli_runs_cytoscan_outlier_with_debug_artifacts() {
    let directory = create_temp_dir("cytiq-cli-cytoscan-outlier");
    let input_dir = directory.join("test");
    let output_dir = directory.join("out");
    let debug_dir = directory.join("debug");
    write_cytoscan_fixture_set(&input_dir, &[0.0, 4.0, 8.0, 280.0]);

    let output = Command::new(env!("CARGO_BIN_EXE_cytiq"))
        .args([
            "cytoscan",
            "--input",
            utf8(&input_dir),
            "--output",
            utf8(&output_dir),
            "--channels",
            "CD3,CD4,CD8",
            "--method",
            "quantiles",
            "--flag-method",
            "outlier",
            "--debug-dir",
            utf8(&debug_dir),
            "--threads",
            "1",
        ])
        .output()
        .expect("command should run");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output_dir.join("cytoscan_summary.csv").is_file());
    assert!(output_dir.join("cytoscan_features.csv").is_file());
    assert!(output_dir.join("cytoscan_config.json").is_file());
    assert!(debug_dir.join("pca_coordinates.csv").is_file());
    assert!(debug_dir.join("pca_loadings.csv").is_file());
    let report = fs::read_to_string(debug_dir.join("report.html")).expect("report should exist");
    assert!(report.contains("plotly"));

    let rows = read_csv_rows(&output_dir.join("cytoscan_summary.csv"));
    assert_eq!(rows.len(), 4);
    let feature_header = read_csv_header(&output_dir.join("cytoscan_features.csv"));
    assert!(
        feature_header
            .iter()
            .any(|header| header == "FL1-A [CD3]_0.1")
    );
}

#[test]
fn cli_runs_cytoscan_novelty_with_reference() {
    let directory = create_temp_dir("cytiq-cli-cytoscan-novelty");
    let input_dir = directory.join("test");
    let reference_dir = directory.join("reference");
    let output_dir = directory.join("out");
    write_cytoscan_fixture_set(&reference_dir, &[0.0, 3.0, 6.0, 9.0]);
    write_cytoscan_fixture_set(&input_dir, &[2.0, 260.0]);

    let output = Command::new(env!("CARGO_BIN_EXE_cytiq"))
        .args([
            "cytoscan",
            "--input",
            utf8(&input_dir),
            "--reference",
            utf8(&reference_dir),
            "--output",
            utf8(&output_dir),
            "--channel",
            "FL1-A [CD3]",
            "--channel",
            "FL2-A [CD4]",
            "--channel",
            "FL3-A [CD8]",
            "--method",
            "binning",
            "--flag-method",
            "novelty",
            "--threads",
            "1",
        ])
        .output()
        .expect("command should run");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let header = read_csv_header(&output_dir.join("cytoscan_summary.csv"));
    assert!(
        header
            .iter()
            .any(|column| column == "novelty_log_likelihood")
    );
    let config = fs::read_to_string(output_dir.join("cytoscan_config.json"))
        .expect("config json should exist");
    assert!(config.contains("novelty_threshold"));
}

#[test]
fn cli_requires_reference_for_cytoscan_novelty() {
    let directory = create_temp_dir("cytiq-cli-cytoscan-reference-required");
    let input_dir = directory.join("test");
    write_cytoscan_fixture_set(&input_dir, &[0.0, 5.0]);

    let output = Command::new(env!("CARGO_BIN_EXE_cytiq"))
        .args([
            "cytoscan",
            "--input",
            utf8(&input_dir),
            "--output",
            utf8(&directory.join("out")),
            "--channels",
            "CD3",
            "--flag-method",
            "novelty",
        ])
        .output()
        .expect("command should run");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("--reference"));
}

#[test]
fn cli_cytoscan_channels_and_repeated_channel_are_equivalent() {
    let directory = create_temp_dir("cytiq-cli-cytoscan-channel-equivalence");
    let input_dir = directory.join("test");
    let first_output = directory.join("first");
    let second_output = directory.join("second");
    write_cytoscan_fixture_set(&input_dir, &[0.0, 5.0, 10.0]);

    let first = Command::new(env!("CARGO_BIN_EXE_cytiq"))
        .args([
            "cytoscan",
            "--input",
            utf8(&input_dir),
            "--output",
            utf8(&first_output),
            "--channels",
            "CD3,CD4,CD8",
            "--method",
            "summary",
            "--threads",
            "1",
        ])
        .output()
        .expect("command should run");
    assert!(
        first.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&first.stderr)
    );

    let second = Command::new(env!("CARGO_BIN_EXE_cytiq"))
        .args([
            "cytoscan",
            "--input",
            utf8(&input_dir),
            "--output",
            utf8(&second_output),
            "--channel",
            "CD3",
            "--channel",
            "CD4",
            "--channel",
            "CD8",
            "--method",
            "summary",
            "--threads",
            "1",
        ])
        .output()
        .expect("command should run");
    assert!(
        second.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&second.stderr)
    );

    let first_features =
        fs::read(first_output.join("cytoscan_features.csv")).expect("features should exist");
    let second_features =
        fs::read(second_output.join("cytoscan_features.csv")).expect("features should exist");
    assert_eq!(first_features, second_features);
}

#[test]
fn cli_runs_cytoscan_all_feature_methods() {
    let directory = create_temp_dir("cytiq-cli-cytoscan-methods");
    let input_dir = directory.join("test");
    write_cytoscan_fixture_set(&input_dir, &[0.0, 6.0, 12.0, 260.0]);

    for method in ["summary", "quantiles", "emd", "binning", "fingerprint"] {
        let output_dir = directory.join(method);
        let output = Command::new(env!("CARGO_BIN_EXE_cytiq"))
            .args([
                "cytoscan",
                "--input",
                utf8(&input_dir),
                "--output",
                utf8(&output_dir),
                "--channels",
                "CD3,CD4,CD8",
                "--method",
                method,
                "--threads",
                "1",
            ])
            .output()
            .expect("command should run");

        assert!(
            output.status.success(),
            "{method} stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let rows = read_csv_rows(&output_dir.join("cytoscan_features.csv"));
        assert_eq!(rows.len(), 4);
    }
}

#[test]
fn cli_fails_when_cytoscan_channel_is_missing() {
    let directory = create_temp_dir("cytiq-cli-cytoscan-missing-channel");
    let input_dir = directory.join("test");
    write_cytoscan_fixture_set(&input_dir, &[0.0, 5.0]);

    let output = Command::new(env!("CARGO_BIN_EXE_cytiq"))
        .args([
            "cytoscan",
            "--input",
            utf8(&input_dir),
            "--output",
            utf8(&directory.join("out")),
            "--channels",
            "CD45",
        ])
        .output()
        .expect("command should run");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("CD45"));
}

fn write_cytoscan_fixture_set(directory: &Path, offsets: &[f32]) {
    fs::create_dir_all(directory).expect("fixture directory should be created");
    for (index, offset) in offsets.iter().enumerate() {
        write_cytoscan_fixture_fcs(&directory.join(format!("sample_{index:02}.fcs")), *offset);
    }
}

fn write_cytoscan_fixture_fcs(path: &Path, marker_offset: f32) {
    let mut columns = vec![Vec::new(); 6];
    for event_index in 0_u16..80 {
        let event = f32::from(event_index);
        columns[0].push(event);
        columns[1].push(20.0 + event * 0.2);
        columns[2].push(30.0 + event * 0.25);
        columns[3].push(100.0 + marker_offset + event * 1.2);
        columns[4].push(160.0 + marker_offset * 0.5 + event * 0.8);
        columns[5].push(90.0 + marker_offset * 0.25 + event * 0.6);
    }
    let parameters = vec![
        FcsWriteParameter::new("Time"),
        FcsWriteParameter::new("FSC-A"),
        FcsWriteParameter::new("SSC-A"),
        FcsWriteParameter::with_label("FL1-A", "CD3"),
        FcsWriteParameter::with_label("FL2-A", "CD4"),
        FcsWriteParameter::with_label("FL3-A", "CD8"),
    ];
    let options = FcsWriteOptions::new().with_text_keyword(
        "SPILL",
        "3,FL1-A,FL2-A,FL3-A,1,0.02,0.01,0.01,1,0.02,0.0,0.02,1",
    );
    cytiq_fcs::write_fcs3_1_f32_with_options(path, &parameters, &columns, &options)
        .expect("fixture FCS should write");
}

fn read_csv_header(path: &Path) -> Vec<String> {
    let mut reader = csv::Reader::from_path(path).expect("CSV should open");
    reader
        .headers()
        .expect("headers should parse")
        .iter()
        .map(ToOwned::to_owned)
        .collect()
}

fn read_csv_rows(path: &Path) -> Vec<csv::StringRecord> {
    let mut reader = csv::Reader::from_path(path).expect("CSV should open");
    reader
        .records()
        .collect::<Result<Vec<_>, _>>()
        .expect("CSV rows should parse")
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
