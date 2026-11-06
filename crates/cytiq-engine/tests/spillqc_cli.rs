mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

/// Three tubes whose `$SPILLOVER` under-compensates PE into PerCP-Cy5.5 and
/// over-compensates APC into APC-Cy7 (`parity/gen/spillqc.py`).
fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data/fixtures/parity/spillqc")
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

fn cytiq(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_cytiq"))
        .args(args)
        .output()
        .expect("command should run")
}

fn run(args: &[&str]) -> Output {
    let output = cytiq(args);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn summary(output: &Path) -> Value {
    serde_json::from_str(&fs::read_to_string(output.join("summary.json")).expect("summary"))
        .expect("summary should be JSON")
}

/// The SSI status of a primary/secondary pair in a file's summary flags, or
/// `pass` when it is not flagged.
fn status(file: &Value, primary: &str, secondary: &str) -> String {
    file["flags"]
        .as_array()
        .expect("flags")
        .iter()
        .find(|flag| flag["primary_channel"] == primary && flag["secondary_channel"] == secondary)
        .map_or_else(
            || "pass".to_owned(),
            |flag| flag["status"].as_str().expect("status").to_owned(),
        )
}

fn tube(name: &str) -> PathBuf {
    fixture().join(format!("{name}.fcs"))
}

#[test]
fn flags_the_injected_spillover_errors() {
    let directory = create_temp_dir("cytiq-engine-spillqc");
    let output = directory.join("qc");
    run(&[
        "spillqc",
        "--input",
        utf8(&tube("tube_1")),
        "--output",
        utf8(&output),
    ]);

    let summary = summary(&output);
    assert_eq!(summary["counts"]["assessed"], 1);
    let file = &summary["files"][0];
    assert_eq!(file["status"], "assessed");
    assert_eq!(file["matrix"]["source"], "fcs");
    assert_eq!(file["matrix"]["applied"], true);
    assert_eq!(status(file, "PE-A", "PerCP-Cy5-5-A"), "under");
    assert_eq!(status(file, "APC-A", "APC-Cy7-A"), "over");
    assert_eq!(file["channels"].as_array().expect("channels").len(), 6);
    assert_eq!(file["channels"][0]["marker"], "CD3");

    let tables = output.join("tube_1");
    let outputs = [
        tables.join("ssi_matrix.csv"),
        tables.join("ssi_segments.csv"),
        tables.join("cutoffs.csv"),
        output.join("batch.csv"),
        output.join("report.html"),
        output.join("summary.json"),
    ];
    for path in &outputs {
        assert!(path.is_file(), "{} should exist", path.display());
    }
    let report = fs::read_to_string(output.join("report.html")).expect("report");
    assert!(report.contains("SSI matrix: tube_1"));
    assert!(report.contains("need review") || report.contains("needs review"));
    common::assert_provenance(
        &output.join("provenance.json"),
        "cytiq spillqc",
        &[&tube("tube_1")],
        &outputs.iter().map(PathBuf::as_path).collect::<Vec<_>>(),
    );
}

#[test]
fn the_true_matrix_clears_the_injected_pairs() {
    let directory = create_temp_dir("cytiq-engine-spillqc-truth");
    let output = directory.join("qc");
    let matrix = fixture().join("truth/spillover.csv");
    run(&[
        "spillqc",
        "--input",
        utf8(&tube("tube_1")),
        "--output",
        utf8(&output),
        "--matrix",
        utf8(&matrix),
        "--matrix-kind",
        "spillover",
    ]);
    let file = &summary(&output)["files"][0];
    assert_eq!(file["matrix"]["source"], "file");
    assert_eq!(status(file, "PE-A", "PerCP-Cy5-5-A"), "pass");
    assert_eq!(status(file, "APC-A", "APC-Cy7-A"), "pass");
}

#[test]
fn errors_shared_by_a_batch_recur() {
    let directory = create_temp_dir("cytiq-engine-spillqc-batch");
    let output = directory.join("qc");
    run(&[
        "spillqc",
        "--input",
        utf8(&fixture()),
        "--output",
        utf8(&output),
    ]);

    let summary = summary(&output);
    assert_eq!(summary["counts"]["assessed"], 3);
    let ids = summary["files"]
        .as_array()
        .expect("files")
        .iter()
        .map(|file| file["matrix"]["id"].clone())
        .collect::<Vec<_>>();
    assert!(ids.iter().all(|id| *id == ids[0]), "tubes share one matrix");
    let batch = summary["batch"].as_array().expect("batch");
    let row = |primary: &str, secondary: &str| {
        batch
            .iter()
            .find(|row| row["primary_channel"] == primary && row["secondary_channel"] == secondary)
            .expect("pair in batch")
    };
    assert_eq!(row("PE-A", "PerCP-Cy5-5-A")["status"], "recurrent_under");
    assert_eq!(row("APC-A", "APC-Cy7-A")["status"], "recurrent_over");
    assert_eq!(row("APC-A", "APC-Cy7-A")["files_over"], 3);
}

#[test]
fn skips_files_without_a_spillover_matrix() {
    let directory = create_temp_dir("cytiq-engine-spillqc-skip");
    let plain = directory.join("plain.fcs");
    without_spillover(&tube("tube_2"), &plain);
    let output = directory.join("qc");
    run(&[
        "spillqc",
        "--input",
        utf8(&plain),
        "--input",
        utf8(&tube("tube_1")),
        "--output",
        utf8(&output),
    ]);
    let summary = summary(&output);
    assert_eq!(summary["counts"]["skipped"], 1);
    let skipped = summary["files"]
        .as_array()
        .expect("files")
        .iter()
        .find(|file| file["name"] == "plain")
        .expect("plain is reported");
    assert_eq!(skipped["status"], "skipped");
    assert!(
        skipped["reason"]
            .as_str()
            .is_some_and(|reason| reason.contains("--matrix")),
        "{skipped}"
    );
    assert!(!output.join("plain").exists());
}

#[test]
fn refuses_input_already_compensated_by_cytiq() {
    let directory = create_temp_dir("cytiq-engine-spillqc-compensated");
    let compensated = directory.join("compensated.fcs");
    run(&[
        "compensate",
        "--input",
        utf8(&tube("tube_1")),
        "--output",
        utf8(&compensated),
    ]);
    let output = cytiq(&[
        "spillqc",
        "--input",
        utf8(&compensated),
        "--output",
        utf8(&directory.join("qc")),
    ]);
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("already compensated"), "{stderr}");
    assert!(stderr.contains("--preprocessed"), "{stderr}");
}

#[test]
fn assesses_stored_values_with_preprocessed() {
    let directory = create_temp_dir("cytiq-engine-spillqc-preprocessed");
    let compensated = directory.join("compensated.fcs");
    let transformed = directory.join("transformed.fcs");
    run(&[
        "compensate",
        "--input",
        utf8(&tube("tube_1")),
        "--output",
        utf8(&compensated),
    ]);
    run(&[
        "transform",
        "--input",
        utf8(&compensated),
        "--output",
        utf8(&transformed),
        "--method",
        "logicle",
        "--channels",
        "FITC-A,PE-A,PerCP-Cy5-5-A,PE-Cy7-A,APC-A,APC-Cy7-A",
    ]);
    let output = directory.join("qc");
    run(&[
        "spillqc",
        "--input",
        utf8(&transformed),
        "--output",
        utf8(&output),
        "--preprocessed",
    ]);
    let file = &summary(&output)["files"][0];
    assert_eq!(file["status"], "assessed");
    // the matrix moved aside by `cytiq compensate` still groups the file
    assert_eq!(file["matrix"]["applied"], false);
    assert_eq!(file["channels"].as_array().expect("channels").len(), 6);
    assert_eq!(status(file, "APC-A", "APC-Cy7-A"), "over");
}

#[test]
fn fail_on_flag_exits_with_three_after_writing_outputs() {
    let directory = create_temp_dir("cytiq-engine-spillqc-fail");
    let output = directory.join("qc");
    let result = cytiq(&[
        "spillqc",
        "--input",
        utf8(&tube("tube_1")),
        "--output",
        utf8(&output),
        "--fail-on-flag",
    ]);
    assert_eq!(result.status.code(), Some(3));
    assert!(String::from_utf8_lossy(&result.stderr).contains("flagged"));
    assert!(output.join("report.html").is_file());
    assert!(output.join("provenance.json").is_file());
}

#[test]
fn refuses_to_overwrite_an_earlier_run() {
    let directory = create_temp_dir("cytiq-engine-spillqc-overwrite");
    let output = directory.join("qc");
    let input = tube("tube_1");
    let args = [
        "spillqc",
        "--input",
        utf8(&input),
        "--output",
        utf8(&output),
    ];
    run(&args);
    let again = cytiq(&args);
    assert_eq!(again.status.code(), Some(4));
    assert!(String::from_utf8_lossy(&again.stderr).contains("--overwrite"));
    run(&[&args[..], &["--overwrite"]].concat());
}

#[test]
fn a_failed_overwrite_keeps_the_earlier_run() {
    let directory = create_temp_dir("cytiq-engine-spillqc-failed-overwrite");
    let output = directory.join("qc");
    let first = tube("tube_1");
    run(&[
        "spillqc",
        "--input",
        utf8(&first),
        "--output",
        utf8(&output),
    ]);
    let before = fs::read(output.join("summary.json")).expect("summary");
    let matrix = fs::read(output.join("tube_1").join("ssi_matrix.csv")).expect("matrix");

    // An input already compensated by cytiq fails the run after the output
    // directory was checked, where earlier outputs used to be deleted.
    let compensated = directory.join("compensated.fcs");
    run(&[
        "compensate",
        "--input",
        utf8(&first),
        "--output",
        utf8(&compensated),
    ]);
    let failed = cytiq(&[
        "spillqc",
        "--input",
        utf8(&first),
        "--input",
        utf8(&compensated),
        "--output",
        utf8(&output),
        "--overwrite",
    ]);
    assert_eq!(failed.status.code(), Some(2));
    assert_eq!(
        fs::read(output.join("summary.json")).expect("summary"),
        before
    );
    assert_eq!(
        fs::read(output.join("tube_1").join("ssi_matrix.csv")).expect("matrix"),
        matrix
    );
    let hidden = fs::read_dir(&output)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with('.'))
        .collect::<Vec<_>>();
    assert!(hidden.is_empty(), "{hidden:?}");
}

#[test]
fn fails_when_input_is_missing() {
    let directory = create_temp_dir("cytiq-engine-spillqc-missing");
    let output = cytiq(&[
        "spillqc",
        "--input",
        utf8(&directory.join("absent.fcs")),
        "--output",
        utf8(&directory.join("qc")),
    ]);
    assert_eq!(output.status.code(), Some(4));
    assert!(String::from_utf8_lossy(&output.stderr).contains("does not exist"));
}

#[test]
fn rejects_invalid_flags() {
    let directory = create_temp_dir("cytiq-engine-spillqc-flags");
    let input = tube("tube_1");
    let output = directory.join("qc");
    for extra in [
        &["--batch-min-fraction", "1.5"][..],
        &["--segments", "0"],
        &["--preprocessed", "--no-peacoqc"],
        &["--matrix", "m.csv"],
    ] {
        let mut args = vec![
            "spillqc",
            "--input",
            utf8(&input),
            "--output",
            utf8(&output),
        ];
        args.extend_from_slice(extra);
        let result = cytiq(&args);
        assert_eq!(result.status.code(), Some(2), "{extra:?}");
    }
}

fn without_spillover(source: &Path, target: &Path) {
    let dataset = cytiq_fcs::FcsReader::open(source)
        .and_then(cytiq_fcs::FcsReader::read)
        .expect("fixture should be readable");
    let keywords = cytiq_fcs::carried_keywords(&dataset.text)
        .into_iter()
        .filter(|(key, _)| !cytiq_fcs::is_spillover_keyword(key))
        .collect::<Vec<_>>();
    cytiq_fcs::write_fcs3_1_exact_with_options(
        target,
        &dataset
            .parameters
            .iter()
            .map(cytiq_fcs::FcsWriteParameter::from_source)
            .collect::<Vec<_>>(),
        &dataset
            .data
            .columns
            .iter()
            .map(cytiq_fcs::ColumnData::to_f64_vec)
            .collect::<Vec<_>>(),
        &cytiq_fcs::FcsWriteOptions::new().with_text_keywords(keywords),
    )
    .expect("copy should write");
}
