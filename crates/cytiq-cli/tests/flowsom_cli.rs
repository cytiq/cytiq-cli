use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use cytiq_fcs::FcsWriteParameter;

#[test]
fn cli_runs_flowsom_with_debug_artifacts() {
    let directory = create_temp_dir("cytiq-cli-flowsom");
    let input = directory.join("sample.fcs");
    let output_path = directory.join("sample.flowsom.csv");
    let debug_dir = directory.join("debug");
    write_flowsom_fixture_fcs(&input);

    let output = Command::new(env!("CARGO_BIN_EXE_cytiq"))
        .args([
            "flowsom",
            "--input",
            utf8(&input),
            "--output",
            utf8(&output_path),
            "--channel",
            "FL1-A [CD3]",
            "--channel",
            "FL2-A [CD4]",
            "--channel",
            "FL3-A [CD8]",
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
    assert!(debug_dir.join("summary.json").is_file());
    assert!(debug_dir.join("nodes.csv").is_file());
    assert!(debug_dir.join("metaclusters.csv").is_file());
    let report = fs::read_to_string(debug_dir.join("report.html")).expect("report should exist");
    assert!(report.contains("plotly"));

    let mut reader = csv::Reader::from_path(&output_path).expect("output CSV should open");
    assert_eq!(
        reader.headers().expect("headers should parse"),
        ["event_index", "flowsom_node", "flowsom_metacluster"].as_slice()
    );
    let rows = reader
        .records()
        .collect::<Result<Vec<_>, _>>()
        .expect("output rows should parse");
    assert_eq!(rows.len(), 60);
}

#[test]
fn cli_runs_flowsom_with_compact_channels_deterministically() {
    let directory = create_temp_dir("cytiq-cli-flowsom-deterministic");
    let input = directory.join("sample.fcs");
    let first_output = directory.join("first.flowsom.csv");
    let second_output = directory.join("second.flowsom.csv");
    write_flowsom_fixture_fcs(&input);

    for (output_path, threads) in [(&first_output, "1"), (&second_output, "2")] {
        let output = Command::new(env!("CARGO_BIN_EXE_cytiq"))
            .args([
                "flowsom",
                "--input",
                utf8(&input),
                "--output",
                utf8(output_path),
                "--channels",
                "CD3,CD4,CD8",
                "--threads",
                threads,
            ])
            .output()
            .expect("command should run");

        assert!(
            output.status.success(),
            "stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let first = fs::read(&first_output).expect("first output should exist");
    let second = fs::read(&second_output).expect("second output should exist");
    assert_eq!(first, second);
}

#[test]
fn cli_fails_when_flowsom_channel_is_missing() {
    let directory = create_temp_dir("cytiq-cli-flowsom-missing-channel");
    let input = directory.join("sample.fcs");
    write_flowsom_fixture_fcs(&input);

    let output = Command::new(env!("CARGO_BIN_EXE_cytiq"))
        .args([
            "flowsom",
            "--input",
            utf8(&input),
            "--output",
            utf8(&directory.join("sample.flowsom.csv")),
            "--channels",
            "CD45",
        ])
        .output()
        .expect("command should run");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("CD45"));
}

#[test]
fn cli_fails_when_flowsom_input_file_is_missing() {
    let directory = create_temp_dir("cytiq-cli-flowsom-missing-input");
    let input = directory.join("missing.fcs");

    let output = Command::new(env!("CARGO_BIN_EXE_cytiq"))
        .args([
            "flowsom",
            "--input",
            utf8(&input),
            "--output",
            utf8(&directory.join("sample.flowsom.csv")),
            "--channels",
            "CD3",
        ])
        .output()
        .expect("command should run");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("does not exist"));
}

#[test]
fn cli_requires_flowsom_input() {
    let directory = create_temp_dir("cytiq-cli-flowsom-requires-input");

    let output = Command::new(env!("CARGO_BIN_EXE_cytiq"))
        .args([
            "flowsom",
            "--output",
            utf8(&directory.join("sample.flowsom.csv")),
            "--channels",
            "CD3",
        ])
        .output()
        .expect("command should run");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("--input"));
}

#[test]
fn cli_requires_flowsom_output() {
    let directory = create_temp_dir("cytiq-cli-flowsom-requires-output");
    let input = directory.join("sample.fcs");
    write_flowsom_fixture_fcs(&input);

    let output = Command::new(env!("CARGO_BIN_EXE_cytiq"))
        .args(["flowsom", "--input", utf8(&input), "--channels", "CD3"])
        .output()
        .expect("command should run");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("--output"));
}

#[test]
fn cli_requires_flowsom_channels() {
    let directory = create_temp_dir("cytiq-cli-flowsom-empty-channels");
    let input = directory.join("sample.fcs");
    write_flowsom_fixture_fcs(&input);

    let output = Command::new(env!("CARGO_BIN_EXE_cytiq"))
        .args([
            "flowsom",
            "--input",
            utf8(&input),
            "--output",
            utf8(&directory.join("sample.flowsom.csv")),
        ])
        .output()
        .expect("command should run");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("at least one"));
}

#[test]
fn cli_rejects_zero_threads_for_flowsom() {
    let directory = create_temp_dir("cytiq-cli-flowsom-zero-threads");
    let input = directory.join("sample.fcs");
    write_flowsom_fixture_fcs(&input);

    let output = Command::new(env!("CARGO_BIN_EXE_cytiq"))
        .args([
            "flowsom",
            "--input",
            utf8(&input),
            "--output",
            utf8(&directory.join("sample.flowsom.csv")),
            "--channels",
            "CD3",
            "--threads",
            "0",
        ])
        .output()
        .expect("command should run");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("thread count must be greater"));
}

fn write_flowsom_fixture_fcs(path: &Path) {
    let mut columns = vec![Vec::new(); 6];
    for raw_index in 0_u16..60 {
        let cluster_offset = if raw_index < 30 { 0.0 } else { 350.0 };
        let index = f32::from(raw_index);
        columns[0].push(index);
        columns[1].push(50.0 + index * 0.5);
        columns[2].push(70.0 + index * 0.25);
        columns[3].push(100.0 + cluster_offset + index * 1.5);
        columns[4].push(180.0 + cluster_offset * 0.5 + index);
        columns[5].push(80.0 + cluster_offset * 0.25 + index * 0.75);
    }
    let parameters = vec![
        FcsWriteParameter::new("Time"),
        FcsWriteParameter::new("FSC-A"),
        FcsWriteParameter::new("SSC-A"),
        FcsWriteParameter::with_label("FL1-A", "CD3"),
        FcsWriteParameter::with_label("FL2-A", "CD4"),
        FcsWriteParameter::with_label("FL3-A", "CD8"),
    ];
    cytiq_fcs::write_fcs3_1_f32(path, &parameters, &columns).expect("fixture FCS should write");
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
