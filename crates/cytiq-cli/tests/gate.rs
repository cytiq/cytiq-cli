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

#[test]
fn reports_missing_external_data_from_gate_subcommand() -> Result<(), Box<dyn std::error::Error>> {
    let temp_dir = tempdir()?;
    let model_path = temp_dir.path().join("model.onnx");
    support::write_unito_model_with_external_data(&model_path, "model.onnx.data")?;

    let output = Command::new(env!("CARGO_BIN_EXE_cytiq"))
        .arg("gate")
        .arg("--model")
        .arg(&model_path)
        .arg("--fcs")
        .arg(temp_dir.path().join("missing.fcs"))
        .arg("--output")
        .arg(temp_dir.path().join("predictions.parquet"))
        .output()?;

    assert!(
        !output.status.success(),
        "command unexpectedly succeeded: stdout={:?} stderr={:?}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("references missing external data file(s)"));
    assert!(stderr.contains("model.onnx.data"));

    Ok(())
}

#[test]
fn runs_leaf_gate_subcommand_with_parent_chain() -> Result<(), Box<dyn std::error::Error>> {
    let temp_dir = tempdir()?;
    let root_path = temp_dir.path().join("Single_cell_gate_1_model.onnx");
    let leaf_path = temp_dir.path().join("Neutrophil_model.onnx");
    let output_path = temp_dir.path().join("predictions.parquet");

    support::write_identity_unito_model_with_metadata(
        &root_path,
        "Single_cell_gate_1",
        "FSC-A",
        "SSC-A",
        None,
    )?;
    support::write_identity_unito_model_with_metadata(
        &temp_dir.path().join("Granulocyte_model.onnx"),
        "Granulocyte",
        "FSC-A",
        "SSC-A",
        Some("Single_cell_gate_1"),
    )?;
    support::write_identity_unito_model_with_metadata(
        &leaf_path,
        "Neutrophil",
        "FSC-A",
        "SSC-A",
        Some("Granulocyte"),
    )?;

    let output = Command::new(env!("CARGO_BIN_EXE_cytiq"))
        .arg("gate")
        .arg("--model")
        .arg(&leaf_path)
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
    assert!(String::from_utf8_lossy(&output.stdout).contains("wrote 3 predictions for Neutrophil"));

    Ok(())
}

fn fixture_fcs_path() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../data/fixtures/fcs/fcs3_0_integer.fcs")
}
