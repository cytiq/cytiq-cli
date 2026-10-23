mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

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

/// A conventional manifest over the clean fixtures; `samples` and `markers`
/// are YAML list items.
fn manifest(samples: &str, markers: &str) -> String {
    format!(
        "cytometry: conventional\n\
         panel:\n  name: test\n  markers:\n{markers}\
         samples:\n{samples}\
         controls:\n  - {{ file: {control}, dye: Autofluorescence }}\n",
        control = utf8(&fixture("control.fcs")),
    )
}

const MARKERS: &str =
    "    - { marker: CD3 }\n    - { marker: CD19, channel: FL2-A }\n    - { marker: Live/Dead }\n";

fn sample(id: &str, path: &Path) -> String {
    format!("  - {{ id: {id}, file: {} }}\n", utf8(path))
}

fn preflight(directory: &Path, name: &str, text: &str) -> (std::process::Output, Value, PathBuf) {
    let manifest_path = directory.join(name);
    fs::write(&manifest_path, text).expect("manifest should write");
    let report_path = directory.join(format!("{name}.report.json"));
    let output = Command::new(env!("CARGO_BIN_EXE_cytiq"))
        .args([
            "preflight",
            "--manifest",
            utf8(&manifest_path),
            "--output",
            utf8(&report_path),
        ])
        .output()
        .expect("command should run");
    let report = serde_json::from_str(
        &fs::read_to_string(&report_path).expect("the report is written even on failure"),
    )
    .expect("report should be JSON");
    (output, report, manifest_path)
}

fn failed_checks(report: &Value) -> Vec<String> {
    report["checks"]
        .as_array()
        .expect("checks")
        .iter()
        .filter(|check| check["status"] == "fail")
        .map(|check| format!("{} {}", check["scope"], check["check"]))
        .collect()
}

#[test]
fn passes_a_consistent_run() {
    let directory = create_temp_dir("cytiq-cli-preflight-pass");
    let (output, report, manifest_path) = preflight(
        &directory,
        "run.yaml",
        &manifest(&sample("S01", &fixture("sample.fcs")), MARKERS),
    );

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(report["failures"], 0, "{:?}", failed_checks(&report));
    // The fixtures have no `$CYT`/`$DATE`.
    assert_eq!(report["status"], "warn");
    let checks = report["checks"].as_array().expect("checks");
    assert!(checks.iter().any(|check| {
        check["check"] == "panel_marker" && check["message"] == "marker `CD3` → `FL1-A`"
    }));
    assert!(
        checks
            .iter()
            .any(|check| check["check"] == "spillover" && check["status"] == "pass")
    );
    common::assert_provenance(
        &directory.join("run.yaml.report.json.provenance.json"),
        "cytiq preflight",
        &[
            &manifest_path,
            &fixture("sample.fcs"),
            &fixture("control.fcs"),
        ],
        &[&directory.join("run.yaml.report.json")],
    );
}

#[test]
fn yaml_and_json_manifests_give_the_same_checks() {
    let directory = create_temp_dir("cytiq-cli-preflight-json");
    let yaml = manifest(&sample("S01", &fixture("sample.fcs")), MARKERS);
    let (_, from_yaml, _) = preflight(&directory, "run.yaml", &yaml);
    let json = format!(
        r#"{{"cytometry":"conventional",
            "panel":{{"name":"test","markers":[{{"marker":"CD3"}},{{"marker":"CD19","channel":"FL2-A"}},{{"marker":"Live/Dead"}}]}},
            "samples":[{{"id":"S01","file":"{sample}"}}],
            "controls":[{{"file":"{control}","dye":"Autofluorescence"}}]}}"#,
        sample = utf8(&fixture("sample.fcs")),
        control = utf8(&fixture("control.fcs")),
    );
    let (_, from_json, _) = preflight(&directory, "run.json", &json);
    assert_eq!(from_yaml["checks"], from_json["checks"]);
}

#[test]
fn fails_on_a_missing_required_marker() {
    let directory = create_temp_dir("cytiq-cli-preflight-marker");
    let markers =
        format!("{MARKERS}    - {{ marker: CD8 }}\n    - {{ marker: CD45, required: false }}\n");
    let (output, report, _) = preflight(
        &directory,
        "run.yaml",
        &manifest(&sample("S01", &fixture("sample.fcs")), &markers),
    );

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(failed_checks(&report), [r#""sample S01" "panel_marker""#]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("marker `CD8`"), "stderr: {stderr}");
}

#[test]
fn fails_on_mismatched_detectors_and_processed_files() {
    let directory = create_temp_dir("cytiq-cli-preflight-mismatch");
    let transformed = directory.join("sample.t.fcs");
    let transform = Command::new(env!("CARGO_BIN_EXE_cytiq"))
        .args([
            "transform",
            "--input",
            utf8(&fixture("sample.fcs")),
            "--output",
            utf8(&transformed),
        ])
        .output()
        .expect("command should run");
    assert!(transform.status.success());
    let samples = [
        sample("S01", &fixture("sample.fcs")),
        sample("S02", &fixture("no_viability.fcs")),
        sample("S03", &transformed),
    ]
    .concat();

    let (output, report, _) = preflight(&directory, "run.yaml", &manifest(&samples, MARKERS));

    assert!(!output.status.success());
    let failed = failed_checks(&report);
    for expected in [
        r#""sample S02" "panel_marker""#,
        r#""sample S02" "detector_configuration""#,
        r#""sample S03" "raw_signal""#,
    ] {
        assert!(failed.iter().any(|check| check == expected), "{failed:?}");
    }
}

#[test]
fn rejects_an_invalid_manifest() {
    let directory = create_temp_dir("cytiq-cli-preflight-invalid");
    let manifest_path = directory.join("run.yaml");
    fs::write(&manifest_path, "cytometry: conventional\nsampels: []\n").expect("write");
    let output = Command::new(env!("CARGO_BIN_EXE_cytiq"))
        .args([
            "preflight",
            "--manifest",
            utf8(&manifest_path),
            "--output",
            utf8(&directory.join("report.json")),
        ])
        .output()
        .expect("command should run");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("unknown field `sampels`"),
        "stderr: {stderr}"
    );
}
