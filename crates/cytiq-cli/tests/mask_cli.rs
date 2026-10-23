mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use cytiq_mask::EventMask;

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

fn run(args: &[&str]) -> std::process::Output {
    let output = Command::new(env!("CARGO_BIN_EXE_cytiq"))
        .args(args)
        .output()
        .expect("command should run");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

#[test]
fn combines_peacoqc_and_clean_masks_by_source_event() {
    let directory = create_temp_dir("cytiq-cli-mask-combine");
    let raw = fixture("sample.fcs");
    let peacoqc = directory.join("sample.peacoqc.fcs");
    let clean = directory.join("clean");
    let combined_path = directory.join("analysis.parquet");
    run(&[
        "peacoqc",
        "--input",
        utf8(&raw),
        "--output",
        utf8(&peacoqc),
        "--channels",
        "FL1-A,FL2-A",
        "--remove-margins",
    ]);
    // clean on the PeacoQC output: its masks cover only the retained events.
    run(&["clean", "--input", utf8(&peacoqc), "--output", utf8(&clean)]);
    let peacoqc_masks = directory.join("sample.peacoqc.masks.parquet");
    let clean_masks = clean.join("masks.parquet");

    run(&[
        "mask",
        "combine",
        "--mask",
        utf8(&peacoqc_masks),
        "--mask",
        utf8(&clean_masks),
        "--output",
        utf8(&combined_path),
    ]);

    let peacoqc_mask = EventMask::read_parquet(&peacoqc_masks).expect("peacoqc mask");
    let clean_mask = EventMask::read_parquet(&clean_masks).expect("clean mask");
    let combined = EventMask::read_parquet(&combined_path).expect("combined mask");
    assert_eq!(combined.source_event_ids, peacoqc_mask.source_event_ids);
    let clean_kept = clean_mask
        .source_event_ids
        .iter()
        .zip(&clean_mask.keep)
        .filter_map(|(id, keep)| keep.then_some(*id))
        .collect::<std::collections::HashSet<_>>();
    let expected = peacoqc_mask
        .source_event_ids
        .iter()
        .zip(&peacoqc_mask.keep)
        .map(|(id, keep)| *keep && clean_kept.contains(id))
        .collect::<Vec<_>>();
    assert_eq!(combined.keep, expected);
    assert!(combined.kept() <= clean_mask.kept());
    let names = combined
        .filters
        .iter()
        .map(|(name, _)| name.as_str())
        .collect::<Vec<_>>();
    common::assert_provenance(
        &directory.join("analysis.parquet.provenance.json"),
        "cytiq mask combine",
        &[&peacoqc_masks, &clean_masks],
        &[&combined_path],
    );
    common::assert_provenance(
        &clean.join("provenance.json"),
        "cytiq clean",
        &[&peacoqc],
        &[&clean_masks, &clean.join("metrics.json")],
    );
    assert_eq!(
        names,
        [
            "margin_pass",
            "peacoqc_pass",
            "non_debris",
            "singlets",
            "live"
        ]
    );
}

#[test]
fn refuses_to_replace_combined_mask_without_overwrite() {
    let directory = create_temp_dir("cytiq-cli-mask-overwrite");
    let mask = directory.join("mask.parquet");
    EventMask::new(vec![true, false], None)
        .write_parquet(&mask)
        .expect("mask should write");
    let output = directory.join("combined.parquet");
    fs::write(&output, b"existing").expect("placeholder should write");

    let result = Command::new(env!("CARGO_BIN_EXE_cytiq"))
        .args([
            "mask",
            "combine",
            "--mask",
            utf8(&mask),
            "--output",
            utf8(&output),
        ])
        .output()
        .expect("command should run");

    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("--overwrite"));
}
