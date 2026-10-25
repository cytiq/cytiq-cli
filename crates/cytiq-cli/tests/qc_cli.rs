mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

use cytiq_mask::EventMask;
use serde_json::Value;

const EVENTS: usize = 10_000;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../data/fixtures/clean")
        .join(name)
}

fn create_temp_dir(prefix: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be after epoch")
        .as_nanos();
    let directory = std::env::temp_dir().join(format!("{prefix}-{}-{nanos}", std::process::id()));
    fs::create_dir_all(&directory).expect("temp dir should be created");
    directory
}

fn utf8(path: &Path) -> &str {
    path.to_str().expect("path should be UTF-8")
}

/// A manifest for `fixture("sample.fcs")` whose panel has `qc` (YAML lines
/// indented under `panel`).
fn write_manifest(directory: &Path, qc: &str) -> PathBuf {
    let path = directory.join("run.yaml");
    fs::write(
        &path,
        format!(
            "cytometry: conventional\n\
             panel:\n  name: test\n  markers:\n    - {{ marker: CD3 }}\n{qc}\
             samples:\n  - {{ id: S01, file: {} }}\n",
            utf8(&fixture("sample.fcs"))
        ),
    )
    .expect("manifest should write");
    path
}

/// A mask over every raw event: `peacoqc_pass` removes the first
/// `peacoqc_removed` events, `live` assessed nothing.
fn write_mask(directory: &Path, acquisition: String, peacoqc_removed: usize) -> PathBuf {
    let path = directory.join("analysis.parquet");
    let pass = (0..EVENTS)
        .map(|row| row >= peacoqc_removed)
        .collect::<Vec<_>>();
    EventMask::new(acquisition, pass.clone(), None)
        .with_filter("peacoqc_pass", pass.into_iter().map(Some).collect())
        .with_filter("live", vec![None; EVENTS])
        .write_parquet(&path)
        .expect("mask should write");
    path
}

fn qc(directory: &Path, manifest: &Path, mask: &Path) -> (Output, Option<Value>) {
    let report = directory.join("qc.json");
    let output = Command::new(env!("CARGO_BIN_EXE_cytiq"))
        .args([
            "qc",
            "--manifest",
            utf8(manifest),
            "--sample",
            "S01",
            "--mask",
            utf8(mask),
            "--output",
            utf8(&report),
        ])
        .output()
        .expect("command should run");
    let report = fs::read_to_string(report)
        .ok()
        .map(|text| serde_json::from_str(&text).expect("report should be JSON"));
    (output, report)
}

fn failed(report: &Value) -> Vec<String> {
    report["checks"]
        .as_array()
        .expect("checks")
        .iter()
        .filter(|check| check["status"] == "fail")
        .map(|check| format!("{} {}", check["scope"], check["check"]))
        .collect()
}

#[test]
fn passes_a_mask_within_the_policy() {
    let directory = create_temp_dir("cytiq-cli-qc-pass");
    let manifest = write_manifest(
        &directory,
        "  qc:\n    min_events: 5000\n    max_removed_fraction: 0.5\n    filters:\n      peacoqc_pass: { required: true, max_removed_fraction: 0.3 }\n",
    );
    let mask = write_mask(
        &directory,
        common::acquisition_of(&fixture("sample.fcs")),
        1_000,
    );

    let (output, report) = qc(&directory, &manifest, &mask);

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report = report.expect("report");
    assert_eq!(report["status"], "pass");
    assert_eq!(report["panel"], "test");
    assert_eq!(report["metrics"]["kept"], 9_000);
    assert_eq!(report["metrics"]["filters"][0]["removed"], 1_000);
    common::assert_provenance(
        &directory.join("qc.json.provenance.json"),
        "cytiq qc",
        &[&manifest, &fixture("sample.fcs"), &mask],
        &[&directory.join("qc.json")],
    );
}

#[test]
fn fails_when_peacoqc_removes_too_much() {
    let directory = create_temp_dir("cytiq-cli-qc-peacoqc");
    let manifest = write_manifest(
        &directory,
        "  qc:\n    filters:\n      peacoqc_pass: { max_removed_fraction: 0.3 }\n",
    );
    let mask = write_mask(
        &directory,
        common::acquisition_of(&fixture("sample.fcs")),
        5_000,
    );

    let (output, report) = qc(&directory, &manifest, &mask);

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("qc failed"));
    let report = report.expect("the report is written even on failure");
    assert_eq!(
        failed(&report),
        [r#""filter peacoqc_pass" "filter_removed_fraction""#]
    );
    let check = &report["checks"].as_array().expect("checks")[1];
    assert_eq!(
        (check["value"].as_f64(), check["limit"].as_f64()),
        (Some(0.5), Some(0.3))
    );
}

#[test]
fn fails_when_a_required_filter_was_not_assessed() {
    let directory = create_temp_dir("cytiq-cli-qc-required");
    let manifest = write_manifest(
        &directory,
        "  qc:\n    filters:\n      live: { required: true }\n      singlets: { required: true, severity: warn }\n",
    );
    let mask = write_mask(
        &directory,
        common::acquisition_of(&fixture("sample.fcs")),
        0,
    );

    let (output, report) = qc(&directory, &manifest, &mask);

    assert!(!output.status.success());
    let report = report.expect("report");
    assert_eq!(failed(&report), [r#""filter live" "filter_assessed""#]);
    assert_eq!(
        report["warnings"], 1,
        "singlets is missing, with severity warn"
    );
}

#[test]
fn fails_when_a_required_filter_assessed_few_events() {
    let directory = create_temp_dir("cytiq-cli-qc-assessed");
    let manifest = write_manifest(
        &directory,
        "  qc:\n    filters:\n      live: { required: true, min_assessed_fraction: 0.95 }\n",
    );
    // `live` ran on 100 of 10,000 events: enough for `required`, not for
    // the policy.
    let mask = directory.join("analysis.parquet");
    EventMask::new(
        common::acquisition_of(&fixture("sample.fcs")),
        vec![true; EVENTS],
        None,
    )
    .with_filter(
        "live",
        (0..EVENTS).map(|row| (row < 100).then_some(true)).collect(),
    )
    .write_parquet(&mask)
    .expect("mask should write");

    let (output, report) = qc(&directory, &manifest, &mask);

    assert!(!output.status.success());
    let report = report.expect("report");
    assert_eq!(
        failed(&report),
        [r#""filter live" "filter_assessed_fraction""#]
    );
    assert_eq!(report["metrics"]["filters"][0]["assessed_fraction"], 0.01);
}

#[test]
fn fails_when_the_mask_has_rows_beyond_the_raw_file() {
    let directory = create_temp_dir("cytiq-cli-qc-extra-rows");
    let manifest = write_manifest(
        &directory,
        "  qc:\n    filters:\n      live: { max_removed_fraction: 0.3 }\n",
    );
    // Every raw event is there, plus 5,000 rows `live` removed that belong to
    // no raw event: counted, they would double `live`'s removed fraction.
    let rows = EVENTS + 5_000;
    let mask = directory.join("analysis.parquet");
    EventMask::new(
        common::acquisition_of(&fixture("sample.fcs")),
        vec![true; rows],
        None,
    )
    .with_filter("live", (0..rows).map(|row| Some(row < 7_500)).collect())
    .write_parquet(&mask)
    .expect("mask should write");

    let (output, report) = qc(&directory, &manifest, &mask);

    assert!(!output.status.success());
    let report = report.expect("report");
    assert_eq!(failed(&report), [r#""sample S01" "mask_coverage""#]);
    assert!(report["metrics"].is_null());
    assert!(
        report["checks"][0]["message"]
            .as_str()
            .is_some_and(
                |message| message.contains("5000 row(s)") && message.contains("for example 10000")
            ),
        "{}",
        report["checks"][0]
    );
}

#[test]
fn fails_when_the_mask_belongs_to_another_sample() {
    let directory = create_temp_dir("cytiq-cli-qc-acquisition");
    let manifest = write_manifest(&directory, "");
    let mask = write_mask(
        &directory,
        common::acquisition_of(&fixture("control.fcs")),
        0,
    );

    let (output, report) = qc(&directory, &manifest, &mask);

    assert!(!output.status.success());
    let report = report.expect("report");
    assert_eq!(failed(&report), [r#""sample S01" "mask_acquisition""#]);
    assert!(report["metrics"].is_null());
}

#[test]
fn passes_without_a_policy_and_rejects_an_unknown_sample() {
    let directory = create_temp_dir("cytiq-cli-qc-no-policy");
    let manifest = write_manifest(&directory, "");
    let mask = write_mask(
        &directory,
        common::acquisition_of(&fixture("sample.fcs")),
        0,
    );

    let (output, report) = qc(&directory, &manifest, &mask);
    assert!(output.status.success());
    assert_eq!(report.expect("report")["checks"][1]["check"], "policy");

    let unknown = Command::new(env!("CARGO_BIN_EXE_cytiq"))
        .args([
            "qc",
            "--manifest",
            utf8(&manifest),
            "--sample",
            "S99",
            "--mask",
            utf8(&mask),
            "--output",
            utf8(&directory.join("unknown.json")),
        ])
        .output()
        .expect("command should run");
    assert!(!unknown.status.success());
    assert!(String::from_utf8_lossy(&unknown.stderr).contains("sample `S99` is not in manifest"));
    assert!(!directory.join("unknown.json").exists());
}
