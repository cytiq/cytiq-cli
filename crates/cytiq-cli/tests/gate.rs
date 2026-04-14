#[path = "../../cytiq-gate/tests/support.rs"]
mod support;

use std::process::Command;

use tempfile::tempdir;

#[test]
fn runs_gate_subcommand() -> Result<(), Box<dyn std::error::Error>> {
    let temp_dir = tempdir()?;
    let model_path = temp_dir.path().join("model.onnx");
    let output_path = temp_dir.path().join("predictions.parquet");
    support::write_identity_unito_model(&model_path)?;

    let output = Command::new(env!("CARGO_BIN_EXE_cytiq"))
        .arg("gate")
        .arg("--model")
        .arg(&model_path)
        .arg("--fcs")
        .arg(fixture_fcs_path())
        .arg("--output")
        .arg(&output_path)
        .output()?;

    assert!(
        output.status.success(),
        "command failed: stdout={:?} stderr={:?}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output_path.exists());
    assert!(String::from_utf8_lossy(&output.stdout).contains("wrote 3 predictions"));

    Ok(())
}

fn fixture_fcs_path() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../data/fixtures/fcs/fcs3_0_integer.fcs")
}
