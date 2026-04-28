#[path = "../../cytiq-autospectral/tests/support.rs"]
mod support;

use std::fs;
use std::path::Path;
use std::process::Command;

use cytiq_fcs::FcsReader;

#[test]
fn cli_runs_autospectral() {
    let directory = support::copy_autospectral_fixture_set("cytiq-cli-autospectral");
    let output_dir = directory.join("out");
    let debug_dir = directory.join("debug");

    let output = Command::new(env!("CARGO_BIN_EXE_cytiq"))
        .args([
            "autospectral",
            "--controls",
            utf8(&directory.join("controls.csv")),
            "--samples",
            utf8(&directory.join("samples.csv")),
            "--input-dir",
            utf8(&directory),
            "--output",
            utf8(&output_dir),
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
    assert!(
        output_dir
            .join("references/fluorophore_spectra.csv")
            .is_file()
    );
    assert!(
        output_dir
            .join("references/autospectral-references.json")
            .is_file()
    );
    assert!(output_dir.join("diagnostics/run.json").is_file());
    assert!(debug_dir.join("summary.json").is_file());
    let unmixed = output_dir.join("unmixed/sample.unmixed.fcs");
    let dataset = FcsReader::open(&unmixed)
        .and_then(FcsReader::read)
        .expect("unmixed FCS should be readable");
    let names = dataset
        .parameters
        .iter()
        .map(|parameter| parameter.name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        names,
        vec!["FSC-A", "SSC-A", "FITC", "PE", "Autofluorescence"]
    );
}

#[test]
fn cli_fails_when_autospectral_manifest_has_no_autofluorescence() {
    let directory = support::create_temp_dir("cytiq-cli-autospectral-no-af");
    fs::write(
        directory.join("controls.csv"),
        "filename,dye\nfitc.fcs,FITC\n",
    )
    .expect("manifest should write");
    fs::write(directory.join("samples.csv"), "filename\nsample.fcs\n")
        .expect("manifest should write");

    let output = Command::new(env!("CARGO_BIN_EXE_cytiq"))
        .args([
            "autospectral",
            "--controls",
            utf8(&directory.join("controls.csv")),
            "--samples",
            utf8(&directory.join("samples.csv")),
            "--input-dir",
            utf8(&directory),
            "--output",
            utf8(&directory.join("out")),
        ])
        .output()
        .expect("command should run");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Autofluorescence"));
}

#[test]
fn cli_rejects_zero_threads_for_autospectral() {
    let directory = support::create_temp_dir("cytiq-cli-autospectral-zero-threads");
    fs::write(
        directory.join("controls.csv"),
        "filename,dye\nfitc.fcs,FITC\n",
    )
    .expect("manifest should write");
    fs::write(directory.join("samples.csv"), "filename\nsample.fcs\n")
        .expect("manifest should write");

    let output = Command::new(env!("CARGO_BIN_EXE_cytiq"))
        .args([
            "autospectral",
            "--controls",
            utf8(&directory.join("controls.csv")),
            "--samples",
            utf8(&directory.join("samples.csv")),
            "--input-dir",
            utf8(&directory),
            "--output",
            utf8(&directory.join("out")),
            "--threads",
            "0",
        ])
        .output()
        .expect("command should run");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("thread count must be greater"));
}

fn utf8(path: &Path) -> &str {
    path.to_str().expect("path should be valid utf-8")
}
