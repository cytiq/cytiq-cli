use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use cytiq_fcs::{FcsWriteOptions, FcsWriteParameter};
use serde_json::Value;
use sha2::{Digest, Sha256};

const CUSTOM_KEYWORDS: [(&str, &str); 3] = [
    ("$CYT", "Test Cytometer"),
    ("$SPILLOVER", "2,CH1,CH2,1,0.1,0.05,1"),
    ("CYTIQ_TRANSFORM", "{\"method\":\"none\"}"),
];

#[test]
fn meta_writes_json_to_stdout() {
    let directory = create_temp_dir("cytiq-meta-stdout");
    let input = directory.join("input.fcs");
    write_fixture(&input);

    let output = run_meta(&["--input", utf8(&input)]);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let document: Value = serde_json::from_slice(&output.stdout).expect("stdout should be JSON");

    assert_eq!(document["event_count"], 3);
    assert_eq!(document["parameter_count"], 12);
    assert_eq!(document["fcs_version"], "FCS3.1");
    assert_eq!(document["file"]["sha256"], sha256_hex(&input));
    assert_eq!(document["parameters"][0]["name"], "CH1");
    assert!(
        document["parameters"][0]["keywords"]
            .as_object()
            .expect("keywords should be an object")
            .keys()
            .all(|key| !key.starts_with("$P10") && !key.starts_with("$P11"))
    );

    let text = document["text"]
        .as_array()
        .expect("text should be an array of pairs");
    let positions = CUSTOM_KEYWORDS
        .iter()
        .map(|(key, value)| {
            text.iter()
                .position(|pair| pair[0] == *key && pair[1] == *value)
                .unwrap_or_else(|| panic!("missing keyword {key}"))
        })
        .collect::<Vec<_>>();
    assert!(positions.windows(2).all(|pair| pair[0] < pair[1]));
}

#[test]
fn meta_dash_output_writes_to_stdout() {
    let directory = create_temp_dir("cytiq-meta-dash");
    let input = directory.join("input.fcs");
    write_fixture(&input);

    let output = run_meta(&["--input", utf8(&input), "--output", "-"]);
    assert!(output.status.success());
    serde_json::from_slice::<Value>(&output.stdout).expect("stdout should be JSON");
    assert!(!directory.join("-").exists());
}

#[test]
fn meta_output_file_requires_overwrite() {
    let directory = create_temp_dir("cytiq-meta-file");
    let input = directory.join("input.fcs");
    let json = directory.join("nested/input.meta.json");
    write_fixture(&input);

    let first = run_meta(&["--input", utf8(&input), "--output", utf8(&json)]);
    assert!(
        first.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&first.stderr)
    );
    assert_eq!(first.stdout, b"");
    let document: Value =
        serde_json::from_slice(&fs::read(&json).expect("metadata file should exist"))
            .expect("metadata file should be JSON");
    assert_eq!(document["file"]["name"], "input.fcs");

    let second = run_meta(&["--input", utf8(&input), "--output", utf8(&json)]);
    assert!(!second.status.success());
    assert!(String::from_utf8_lossy(&second.stderr).contains("--overwrite"));

    let third = run_meta(&[
        "--input",
        utf8(&input),
        "--output",
        utf8(&json),
        "--overwrite",
    ]);
    assert!(third.status.success());
}

#[test]
fn meta_reports_missing_input() {
    let directory = create_temp_dir("cytiq-meta-missing");
    let output = run_meta(&["--input", utf8(&directory.join("missing.fcs"))]);
    assert!(!output.status.success());
    assert_eq!(output.stdout, b"");
    assert_ne!(output.stderr, b"");
}

fn run_meta(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_cytiq"))
        .arg("meta")
        .args(args)
        .output()
        .expect("cytiq should run")
}

fn write_fixture(path: &Path) {
    let parameters = (1..=12)
        .map(|index| FcsWriteParameter::new(format!("CH{index}")))
        .collect::<Vec<_>>();
    let columns = vec![vec![1.0_f32, 2.0, 3.0]; parameters.len()];
    let options = CUSTOM_KEYWORDS
        .iter()
        .fold(FcsWriteOptions::new(), |options, (key, value)| {
            options.with_text_keyword(*key, *value)
        });
    cytiq_fcs::write_fcs3_1_f32_with_options(path, &parameters, &columns, &options)
        .expect("fixture FCS should write");
}

fn sha256_hex(path: &Path) -> String {
    Sha256::digest(fs::read(path).expect("fixture should be readable"))
        .iter()
        .fold(String::new(), |mut hex, byte| {
            write!(hex, "{byte:02x}").expect("writing to a String cannot fail");
            hex
        })
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
