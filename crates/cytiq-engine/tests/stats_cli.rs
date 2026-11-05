mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use cytiq_fcs::{FcsWriteOptions, FcsWriteParameter};
use serde_json::Value;

fn fixture(area: &str, name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../data/fixtures")
        .join(area)
        .join(name)
}

fn utf8(path: &Path) -> &str {
    path.to_str().expect("path should be UTF-8")
}

fn stats(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_cytiq"))
        .arg("stats")
        .args(args)
        .output()
        .expect("cytiq should run")
}

fn read_json(path: &Path) -> Value {
    serde_json::from_slice(&fs::read(path).expect("report should exist"))
        .expect("report should be JSON")
}

fn channel<'a>(report: &'a Value, name: &str) -> &'a Value {
    report["channels"]
        .as_array()
        .expect("channels")
        .iter()
        .find(|channel| channel["name"] == name)
        .unwrap_or_else(|| panic!("no channel {name}"))
}

/// Three channels with a `$PnR` of 1024: `FSC-A` has `saturated` events at 1023.
fn write_saturated(path: &Path, saturated: usize) {
    let parameters = ["FSC-A", "FL1-A", "Time"]
        .into_iter()
        .map(|name| FcsWriteParameter::new(name).with_range(1024.0))
        .collect::<Vec<_>>();
    let events = 200;
    #[allow(clippy::cast_precision_loss)]
    let fsc = (0..events)
        .map(|event| {
            if event < saturated {
                1023.0
            } else {
                100.0 + event as f32
            }
        })
        .collect::<Vec<f32>>();
    #[allow(clippy::cast_precision_loss)]
    let fl1 = (0..events)
        .map(|event| event as f32 - 20.0)
        .collect::<Vec<f32>>();
    #[allow(clippy::cast_precision_loss)]
    let time = (0..events)
        .map(|event| (event % 150) as f32)
        .collect::<Vec<f32>>();
    let options = FcsWriteOptions::new().with_text_keyword("$TIMESTEP", "0.5");
    cytiq_fcs::write_fcs3_1_f32_with_options(path, &parameters, &[fsc, fl1, time], &options)
        .expect("fixture FCS should write");
}

#[test]
fn stats_summarize_every_channel_with_provenance() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let input = fixture("gating", "sample.fcs");
    let output = directory.path().join("stats.json");

    let result = stats(&["--input", utf8(&input), "--output", utf8(&output)]);

    assert!(
        result.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    let report = read_json(&output);
    assert_eq!(report["schema_version"], 1);
    assert_eq!(report["event_count"], 4000);
    assert_eq!(report["file"]["sha256"], common::sha256_hex(&input));
    assert_eq!(channel(&report, "FSC-A")["kind"], "scatter");
    assert_eq!(channel(&report, "FL1-A")["kind"], "fluorescence");
    assert_eq!(channel(&report, "FL1-A")["histogram"]["scale"], "arcsinh");
    assert_eq!(channel(&report, "Time")["kind"], "time");
    for item in report["channels"].as_array().expect("channels") {
        let counts = item["histogram"]["counts"].as_array().expect("counts");
        assert_eq!(counts.len(), 256);
        assert_eq!(
            counts
                .iter()
                .map(|count| count.as_u64().expect("count"))
                .sum::<u64>(),
            4000
        );
    }
    assert_eq!(report["time"]["timestep"], 0.01);
    assert_eq!(
        report["time"]["counts"]
            .as_array()
            .expect("time bins")
            .len(),
        100
    );
    common::assert_provenance(
        &directory.path().join("stats.json.provenance.json"),
        "cytiq stats",
        &[&input],
        &[&output],
    );
}

#[test]
fn stats_count_detector_limit_and_clock_resets() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let input = directory.path().join("saturated.fcs");
    let output = directory.path().join("stats.json");
    write_saturated(&input, 7);

    let result = stats(&[
        "--input",
        utf8(&input),
        "--output",
        utf8(&output),
        "--bins",
        "16",
    ]);

    assert!(
        result.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    let report = read_json(&output);
    let fsc = channel(&report, "FSC-A");
    assert_eq!(fsc["detector_limit"], 1023.0);
    assert_eq!(fsc["at_detector_limit"], 7);
    assert_eq!(fsc["max"], 1023.0);
    assert_eq!(channel(&report, "FL1-A")["at_or_below_zero"], 21);
    assert_eq!(
        channel(&report, "FL1-A")["histogram"]["counts"]
            .as_array()
            .expect("bins")
            .len(),
        16
    );
    assert_eq!(report["time"]["monotonic"], false);
    assert_eq!(report["time"]["decreases"], 1);
    assert_eq!(report["time"]["duration_seconds"], 24.5);
}

#[test]
fn stats_refuse_to_overwrite_and_reject_bad_bins() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let input = fixture("gating", "sample.fcs");
    let output = directory.path().join("stats.json");
    assert!(
        stats(&["--input", utf8(&input), "--output", utf8(&output)])
            .status
            .success()
    );

    let again = stats(&["--input", utf8(&input), "--output", utf8(&output)]);
    let bins = stats(&[
        "--input",
        utf8(&input),
        "--output",
        utf8(&directory.path().join("b.json")),
        "--bins",
        "1",
    ]);
    let replaced = stats(&[
        "--input",
        utf8(&input),
        "--output",
        utf8(&output),
        "--overwrite",
    ]);

    assert_eq!(again.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&again.stderr).contains("--overwrite"));
    assert_eq!(bins.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&bins.stderr).contains("--bins"));
    assert!(replaced.status.success());
}

#[test]
fn stats_report_an_unreadable_input() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let input = directory.path().join("broken.fcs");
    fs::write(&input, b"FCS3.1    garbage").expect("write");

    let result = stats(&[
        "--input",
        utf8(&input),
        "--output",
        utf8(&directory.path().join("s.json")),
    ]);

    assert_eq!(result.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&result.stderr).contains("failed to read FCS input"));
    assert!(!directory.path().join("s.json").exists());
}
