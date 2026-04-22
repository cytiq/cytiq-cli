#[path = "../../cytiq-autospill/tests/support.rs"]
mod support;

use std::fs;
use std::process::Command;

#[test]
fn cli_writes_csv_tsv_and_parquet_outputs() {
    let directory = support::create_temp_dir("cytiq-cli-success");
    let manifest_path = support::write_two_control_fixture(&directory, [[1.0, 0.2], [0.1, 1.0]]);

    let csv_path = directory.join("matrix.csv");
    run_success(&manifest_path, &directory, &csv_path);
    let csv_output = fs::read_to_string(&csv_path).expect("csv output should exist");
    assert!(csv_output.starts_with("channel,FL1-A,FL2-A"));

    let tsv_path = directory.join("matrix.tsv");
    run_success(&manifest_path, &directory, &tsv_path);
    let tsv_output = fs::read_to_string(&tsv_path).expect("tsv output should exist");
    assert!(tsv_output.starts_with("channel\tFL1-A\tFL2-A"));

    let parquet_path = directory.join("matrix.parquet");
    run_success(&manifest_path, &directory, &parquet_path);
    let parquet = fs::read(&parquet_path).expect("parquet output should exist");
    assert_eq!(&parquet[..4], b"PAR1");
}

#[test]
fn cli_fails_when_manifest_references_missing_control() {
    let directory = support::create_temp_dir("cytiq-cli-missing-file");
    let manifest_path = directory.join("controls.csv");
    support::write_text(
        &manifest_path,
        "filename,dye,primary_channel\nmissing.fcs,FITC,FL1-A\n",
    );

    let output = Command::new(env!("CARGO_BIN_EXE_cytiq"))
        .args([
            "autospill",
            "--controls",
            manifest_path
                .to_str()
                .expect("manifest path should be valid utf-8"),
            "--input-dir",
            directory
                .to_str()
                .expect("directory path should be valid utf-8"),
            "--output",
            directory
                .join("matrix.csv")
                .to_str()
                .expect("output path should be valid utf-8"),
        ])
        .output()
        .expect("command should run");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("does not exist"));
}

#[test]
fn cli_fails_on_duplicate_primary_channel() {
    let directory = support::create_temp_dir("cytiq-cli-duplicate-channel");
    let manifest_path = directory.join("controls.csv");
    support::write_text(
        &manifest_path,
        "filename,dye,primary_channel\ncontrol_a.fcs,FITC,FL1-A\ncontrol_b.fcs,PE,FL1-A\n",
    );

    let output = Command::new(env!("CARGO_BIN_EXE_cytiq"))
        .args([
            "autospill",
            "--controls",
            manifest_path
                .to_str()
                .expect("manifest path should be valid utf-8"),
            "--input-dir",
            directory
                .to_str()
                .expect("directory path should be valid utf-8"),
            "--output",
            directory
                .join("matrix.csv")
                .to_str()
                .expect("output path should be valid utf-8"),
        ])
        .output()
        .expect("command should run");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("duplicate `primary_channel`"));
}

fn run_success(
    manifest_path: &std::path::Path,
    input_dir: &std::path::Path,
    output_path: &std::path::Path,
) {
    let status = Command::new(env!("CARGO_BIN_EXE_cytiq"))
        .args([
            "autospill",
            "--controls",
            manifest_path
                .to_str()
                .expect("manifest path should be valid utf-8"),
            "--input-dir",
            input_dir.to_str().expect("input dir should be valid utf-8"),
            "--output",
            output_path
                .to_str()
                .expect("output path should be valid utf-8"),
        ])
        .status()
        .expect("command should run");

    assert!(status.success());
}
