mod common;
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
    common::assert_provenance(
        &directory.join("matrix.csv.provenance.json"),
        "cytiq autospill",
        &[&manifest_path, &directory.join("control_a.fcs")],
        &[&csv_path],
    );

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
    support::write_text(&manifest_path, "filename,dye\nmissing.fcs,FITC\n");

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
fn cli_fails_on_duplicate_inferred_primary_channel() {
    let directory = support::create_temp_dir("cytiq-cli-duplicate-channel");
    support::write_control_file(
        &directory,
        "control_fitc.fcs",
        0,
        &[1.0, 0.2],
        &["FL1-A", "FL2-A"],
        &[Some("FITC".to_owned()), None],
    );
    support::write_control_file(
        &directory,
        "control_pe.fcs",
        0,
        &[1.0, 0.2],
        &["FL1-A", "FL2-A"],
        &[Some("PE".to_owned()), None],
    );
    let manifest_path = support::write_manifest_rows(
        &directory,
        &[("control_fitc.fcs", "FITC"), ("control_pe.fcs", "PE")],
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
    assert!(stderr.contains("duplicate inferred `primary_channel`"));
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

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let csv_output = fs::read_to_string(&output_path).expect("spillover output should exist");
    assert!(csv_output.starts_with("dye,FL1-A,FL2-A,FL3-A"));
    assert!(debug_dir.join("summary.json").is_file());
    let report_path = debug_dir.join("report.html");
    assert!(report_path.is_file());
    let report = fs::read_to_string(report_path).expect("report should exist");
    assert!(report.contains("cytiq AutoSpill Report"));
    assert!(report.contains("Spillover Matrix"));
    assert!(report.contains("Compensation Matrix"));
}

#[test]
fn cli_runs_autospread() {
    let directory = support::copy_autospread_fixture_set("cytiq-cli-autospread");
    let debug_dir = directory.join("debug");
    let manifest_path = directory.join("controls.csv");
    let detectors_path = directory.join("detectors.csv");
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

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let csv_output = fs::read_to_string(&output_path).expect("spread output should exist");
    assert!(csv_output.starts_with("dye,FL1-A,FL2-A,FL3-A"));
    assert!(debug_dir.join("summary.json").is_file());
    let report_path = debug_dir.join("report.html");
    assert!(report_path.is_file());
    let report = fs::read_to_string(report_path).expect("report should exist");
    assert!(report.contains("cytiq AutoSpread Report"));
    assert!(report.contains("Spread Matrix"));
}

#[test]
fn cli_writes_autospread_csv_tsv_and_parquet_outputs() {
    let directory = support::copy_autospread_fixture_set("cytiq-cli-autospread-formats");
    let manifest_path = directory.join("controls.csv");
    let detectors_path = directory.join("detectors.csv");

    let csv_path = directory.join("spread.csv");
    run_autospread_success(&manifest_path, &detectors_path, &directory, &csv_path);
    let csv_output = fs::read_to_string(&csv_path).expect("csv output should exist");
    assert_eq!(
        csv_output,
        support::read_autospread_fixture_text("expected_spread.csv")
    );

    let tsv_path = directory.join("spread.tsv");
    run_autospread_success(&manifest_path, &detectors_path, &directory, &tsv_path);
    let tsv_output = fs::read_to_string(&tsv_path).expect("tsv output should exist");
    assert!(tsv_output.starts_with("dye\tFL1-A\tFL2-A\tFL3-A"));

    let parquet_path = directory.join("spread.parquet");
    run_autospread_success(&manifest_path, &detectors_path, &directory, &parquet_path);
    let parquet = fs::read(&parquet_path).expect("parquet output should exist");
    assert_eq!(&parquet[..4], b"PAR1");
}

#[test]
fn cli_rejects_processed_controls() {
    let directory = support::copy_autospill_fixture_set("cytiq-cli-processed-control");
    common::add_keyword(
        &directory.join("control_a.fcs"),
        cytiq_fcs::TRANSFORM_KEYWORD,
        &common::transform_keyword(&["FL1-A", "FL2-A"]),
    );

    let output = Command::new(env!("CARGO_BIN_EXE_cytiq"))
        .args([
            "autospill",
            "--controls",
            utf8(&directory.join("controls.csv")),
            "--input-dir",
            utf8(&directory),
            "--output",
            utf8(&directory.join("matrix.csv")),
        ])
        .output()
        .expect("command should run");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("transformed (arcsinh)") && stderr.contains("must be raw"),
        "stderr: {stderr}"
    );
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
            "--threads",
            "1",
        ])
        .status()
        .expect("command should run");

    assert!(status.success());
}

fn run_autospread_success(
    manifest_path: &Path,
    detectors_path: &Path,
    input_dir: &Path,
    output_path: &Path,
) {
    let status = Command::new(env!("CARGO_BIN_EXE_cytiq"))
        .args([
            "autospread",
            "--controls",
            utf8(manifest_path),
            "--input-dir",
            utf8(input_dir),
            "--output",
            utf8(output_path),
            "--detectors",
            utf8(detectors_path),
            "--threads",
            "1",
        ])
        .status()
        .expect("command should run");

    assert!(status.success());
}

#[test]
fn cli_rejects_zero_threads_for_autospill_and_autospread() {
    let directory = support::create_temp_dir("cytiq-cli-zero-threads");
    let manifest_path = support::write_manifest_rows(&directory, &[("missing.fcs", "FITC")]);
    let detectors_path = directory.join("detectors.csv");
    support::write_text(
        &detectors_path,
        "channel,label,wavelength\nFL1-A,FITC detector,530\n",
    );

    let autospill_output = Command::new(env!("CARGO_BIN_EXE_cytiq"))
        .args([
            "autospill",
            "--controls",
            utf8(&manifest_path),
            "--input-dir",
            utf8(&directory),
            "--output",
            utf8(&directory.join("matrix.csv")),
            "--threads",
            "0",
        ])
        .output()
        .expect("command should run");
    assert!(!autospill_output.status.success());
    assert!(
        String::from_utf8_lossy(&autospill_output.stderr).contains("thread count must be greater")
    );

    let autospread_output = Command::new(env!("CARGO_BIN_EXE_cytiq"))
        .args([
            "autospread",
            "--controls",
            utf8(&manifest_path),
            "--input-dir",
            utf8(&directory),
            "--output",
            utf8(&directory.join("spread.csv")),
            "--detectors",
            utf8(&detectors_path),
            "--threads",
            "0",
        ])
        .output()
        .expect("command should run");
    assert!(!autospread_output.status.success());
    assert!(
        String::from_utf8_lossy(&autospread_output.stderr).contains("thread count must be greater")
    );
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

#[test]
fn cli_rejects_log_amplified_controls() {
    let directory = support::copy_autospill_fixture_set("cytiq-cli-log-control");
    common::set_log_amplified(&directory.join("control_a.fcs"), "FL1-A");

    let output = Command::new(env!("CARGO_BIN_EXE_cytiq"))
        .args([
            "autospill",
            "--controls",
            utf8(&directory.join("controls.csv")),
            "--input-dir",
            utf8(&directory),
            "--output",
            utf8(&directory.join("matrix.csv")),
        ])
        .output()
        .expect("command should run");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("control_a.fcs") && stderr.contains("`FL1-A` store log-amplified values"),
        "stderr: {stderr}"
    );
}
