#[path = "../../cytiq-autospill/tests/support.rs"]
mod support;

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

#[test]
fn cli_writes_csv_tsv_and_parquet_outputs() {
    let directory = support::copy_autospill_fixture_set("cytiq-cli-success");
    let manifest_path = directory.join("controls.csv");

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
            utf8(&manifest_path),
            "--input-dir",
            utf8(&directory),
            "--output",
            utf8(&directory.join("matrix.csv")),
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
            utf8(&manifest_path),
            "--input-dir",
            utf8(&directory),
            "--output",
            utf8(&directory.join("matrix.csv")),
        ])
        .output()
        .expect("command should run");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("duplicate `primary_channel`"));
}

#[test]
fn cli_supports_detectors_transform_file_and_debug_dir() {
    let directory = support::create_temp_dir("cytiq-cli-rectangular");
    let debug_dir = directory.join("debug");
    let (manifest_path, detectors_path) = support::write_rectangular_autospill_fixture(&directory);
    let transform_path = support::write_transform_manifest(&directory, &["FITC", "PE"]);
    let output_path = directory.join("spillover.csv");

    let output = Command::new(env!("CARGO_BIN_EXE_cytiq"))
        .args([
            "autospill",
            "--controls",
            utf8(&manifest_path),
            "--input-dir",
            utf8(&directory),
            "--output",
            utf8(&output_path),
            "--detectors",
            utf8(&detectors_path),
            "--transform-file",
            utf8(&transform_path),
            "--debug-dir",
            utf8(&debug_dir),
            "--matrix-kind",
            "spillover",
        ])
        .output()
        .expect("command should run");

    assert!(output.status.success(), "stderr: {}", String::from_utf8_lossy(&output.stderr));
    let csv_output = fs::read_to_string(&output_path).expect("spillover output should exist");
    assert!(csv_output.starts_with("dye,FL1-A,FL2-A,FL3-A"));
    assert!(debug_dir.join("summary.json").is_file());
    assert!(debug_dir.join("convergence.html").is_file());
}

#[test]
fn cli_runs_autospread() {
    let directory = support::create_temp_dir("cytiq-cli-autospread");
    let debug_dir = directory.join("debug");
    let (manifest_path, detectors_path) = support::write_autospread_fixture(&directory);
    let output_path = directory.join("spread.csv");

    let output = Command::new(env!("CARGO_BIN_EXE_cytiq"))
        .args([
            "autospread",
            "--controls",
            utf8(&manifest_path),
            "--input-dir",
            utf8(&directory),
            "--output",
            utf8(&output_path),
            "--detectors",
            utf8(&detectors_path),
            "--debug-dir",
            utf8(&debug_dir),
        ])
        .output()
        .expect("command should run");

    assert!(output.status.success(), "stderr: {}", String::from_utf8_lossy(&output.stderr));
    let csv_output = fs::read_to_string(&output_path).expect("spread output should exist");
    assert!(csv_output.starts_with("dye,FL1-A,FL2-A,FL3-A"));
    assert!(debug_dir.join("spread_matrix.html").is_file());
    assert!(debug_dir.join("summary.json").is_file());
}

fn run_success(manifest_path: &Path, input_dir: &Path, output_path: &Path) {
    let status = Command::new(env!("CARGO_BIN_EXE_cytiq"))
        .args([
            "autospill",
            "--controls",
            utf8(manifest_path),
            "--input-dir",
            utf8(input_dir),
            "--output",
            utf8(output_path),
        ])
        .status()
        .expect("command should run");

    assert!(status.success());
}

fn utf8(path: &Path) -> &str {
    path.to_str().expect("path should be valid utf-8")
}

#[allow(dead_code)]
fn run_output(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_cytiq"))
        .args(args)
        .output()
        .expect("command should run")
}
