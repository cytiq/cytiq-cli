use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../data/fixtures/gating")
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

fn run_gate(output: &Path, format: &str) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_cytiq"))
        .args([
            "gate",
            "--input",
            utf8(&fixture("sample.fcs")),
            "--gating-ml",
            utf8(&fixture("gates.xml")),
            "--output",
            utf8(output),
            "--format",
            format,
            "--threads",
            "2",
        ])
        .output()
        .expect("command should run")
}

#[test]
fn cli_writes_gate_counts_csv() {
    let directory = create_temp_dir("cytiq-cli-gate-csv");
    let output_path = directory.join("nested").join("counts.csv");

    let output = run_gate(&output_path, "csv");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let mut reader = csv::Reader::from_path(&output_path).expect("counts CSV should exist");
    let headers = reader.headers().expect("CSV should have headers").clone();
    assert_eq!(
        headers.iter().collect::<Vec<_>>(),
        [
            "gate_id",
            "gate_path",
            "parent_id",
            "gate_type",
            "count",
            "parent_count",
            "percent_of_parent",
            "percent_of_total"
        ]
    );
    let rows = reader
        .records()
        .collect::<Result<Vec<_>, _>>()
        .expect("CSV rows should parse");
    assert_eq!(rows.len(), 11);
    let cd3 = rows
        .iter()
        .find(|row| &row[0] == "CD3pos")
        .expect("CD3pos row should exist");
    assert_eq!(&cd3[1], "Cells/Singlets/CD3pos");
    assert_eq!(&cd3[2], "Singlets");
    assert_eq!(&cd3[4], "1637");
    assert_eq!(&cd3[5], "2762");

    fs::remove_dir_all(directory).ok();
}

#[test]
fn cli_writes_gate_counts_json() {
    let directory = create_temp_dir("cytiq-cli-gate-json");
    let output_path = directory.join("counts.json");

    let output = run_gate(&output_path, "json");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json = fs::read_to_string(&output_path).expect("counts JSON should exist");
    assert!(json.trim_start().starts_with('['));
    assert!(json.contains(r#""gate_id": "Ellipse_or_log""#));
    assert!(json.contains(r#""parent_id": null"#));

    fs::remove_dir_all(directory).ok();
}

#[test]
fn cli_fails_when_gating_ml_is_missing() {
    let directory = create_temp_dir("cytiq-cli-gate-missing");
    let output = Command::new(env!("CARGO_BIN_EXE_cytiq"))
        .args([
            "gate",
            "--input",
            utf8(&fixture("sample.fcs")),
            "--gating-ml",
            utf8(&directory.join("missing.xml")),
            "--output",
            utf8(&directory.join("counts.csv")),
        ])
        .output()
        .expect("command should run");

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("missing.xml"));

    fs::remove_dir_all(directory).ok();
}
