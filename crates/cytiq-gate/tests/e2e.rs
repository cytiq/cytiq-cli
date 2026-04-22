mod support;

use std::fs::File;

use arrow_array::{Array, BooleanArray, Int64Array};
use cytiq_gate::{GateCommand, GateError, run_gate};
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use tempfile::tempdir;

#[test]
fn runs_identity_model_and_writes_event_predictions() -> Result<(), Box<dyn std::error::Error>> {
    let temp_dir = tempdir()?;
    let model_path = temp_dir.path().join("model.onnx");
    let output_path = temp_dir.path().join("predictions.parquet");
    support::write_identity_unito_model(&model_path)?;

    let summary = run_gate(&GateCommand {
        model_path,
        fcs_path: fixture_fcs_path(),
        output_path: output_path.clone(),
    })?;

    assert_eq!(summary.event_count, 3);
    assert_eq!(summary.gate_name, "Single_cell_gate_1");

    let file = File::open(output_path)?;
    let mut reader = ParquetRecordBatchReaderBuilder::try_new(file)?
        .with_batch_size(32)
        .build()?;
    let batch = reader
        .next()
        .transpose()?
        .expect("parquet file should contain a batch");

    assert_eq!(batch.num_rows(), 3);
    assert_eq!(
        int64_values(batch.column(0).as_ref()),
        vec![0_i64, 1_i64, 2_i64]
    );
    assert_eq!(
        bool_values(batch.column(8).as_ref()),
        vec![true, true, true]
    );

    Ok(())
}

#[test]
fn reports_missing_external_data_before_reading_fcs() -> Result<(), Box<dyn std::error::Error>> {
    let temp_dir = tempdir()?;
    let model_path = temp_dir.path().join("model.onnx");
    support::write_unito_model_with_external_data(&model_path, "model.onnx.data")?;

    let error = run_gate(&GateCommand {
        model_path: model_path.clone(),
        fcs_path: temp_dir.path().join("missing.fcs"),
        output_path: temp_dir.path().join("predictions.parquet"),
    })
    .expect_err("model preflight should fail");

    let expected_missing = model_path.with_file_name("model.onnx.data");
    match error {
        GateError::MissingExternalData {
            model_path: actual_model_path,
            missing,
        } => {
            assert_eq!(actual_model_path, model_path);
            assert_eq!(missing, expected_missing.display().to_string());
        }
        other => panic!("expected missing external data error, got {other}"),
    }

    Ok(())
}

#[test]
fn reports_missing_metadata_before_reading_fcs() -> Result<(), Box<dyn std::error::Error>> {
    let temp_dir = tempdir()?;
    let model_path = temp_dir.path().join("model.onnx");
    support::write_unito_model_without_metadata(&model_path)?;

    let error = run_gate(&GateCommand {
        model_path,
        fcs_path: temp_dir.path().join("missing.fcs"),
        output_path: temp_dir.path().join("predictions.parquet"),
    })
    .expect_err("metadata preflight should fail");

    match error {
        GateError::MissingMetadata { missing } => {
            assert_eq!(
                missing,
                vec!["cytiq.gate_name", "cytiq.x_axis", "cytiq.y_axis"]
            );
        }
        other => panic!("expected missing metadata error, got {other}"),
    }

    Ok(())
}

#[test]
fn runs_leaf_model_with_parent_chain() -> Result<(), Box<dyn std::error::Error>> {
    let temp_dir = tempdir()?;
    let output_path = temp_dir.path().join("predictions.parquet");
    let root_path = temp_dir.path().join("Single_cell_gate_1_model.onnx");
    let leaf_path = temp_dir.path().join("Neutrophil_model.onnx");

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

    let summary = run_gate(&GateCommand {
        model_path: leaf_path,
        fcs_path: fixture_fcs_path(),
        output_path: output_path.clone(),
    })?;

    assert_eq!(summary.event_count, 3);
    assert_eq!(summary.gate_name, "Neutrophil");

    let file = File::open(output_path)?;
    let mut reader = ParquetRecordBatchReaderBuilder::try_new(file)?
        .with_batch_size(32)
        .build()?;
    let batch = reader
        .next()
        .transpose()?
        .expect("parquet file should contain a batch");

    assert_eq!(batch.num_rows(), 3);
    assert_eq!(
        bool_values(batch.column(8).as_ref()),
        vec![true, true, true]
    );

    Ok(())
}

#[test]
fn reports_full_chain_axis_mismatch() -> Result<(), Box<dyn std::error::Error>> {
    let temp_dir = tempdir()?;
    let root_path = temp_dir.path().join("Single_cell_gate_1_model.onnx");
    let leaf_path = temp_dir.path().join("Neutrophil_model.onnx");

    support::write_identity_unito_model_with_metadata(
        &root_path,
        "Single_cell_gate_1",
        "DNA1",
        "Event_length",
        None,
    )?;
    support::write_identity_unito_model_with_metadata(
        &temp_dir.path().join("Granulocyte_model.onnx"),
        "Granulocyte",
        "CD66b",
        "CD45",
        Some("Single_cell_gate_1"),
    )?;
    support::write_identity_unito_model_with_metadata(
        &leaf_path,
        "Neutrophil",
        "CD16",
        "CD294",
        Some("Granulocyte"),
    )?;

    let error = run_gate(&GateCommand {
        model_path: leaf_path,
        fcs_path: fixture_fcs_path(),
        output_path: temp_dir.path().join("predictions.parquet"),
    })
    .expect_err("axis preflight should fail");

    match error {
        GateError::IncompatibleFcsAxes { missing, available } => {
            assert_eq!(missing, "DNA1, Event_length, CD66b, CD45, CD16, CD294");
            assert!(available.contains("FSC-A"));
            assert!(available.contains("SSC-A"));
        }
        other => panic!("expected incompatible axis error, got {other}"),
    }

    Ok(())
}

fn fixture_fcs_path() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../data/fixtures/fcs/fcs3_0_integer.fcs")
}

fn int64_values(array: &dyn Array) -> Vec<i64> {
    array
        .as_any()
        .downcast_ref::<Int64Array>()
        .expect("expected Int64Array")
        .values()
        .to_vec()
}

fn bool_values(array: &dyn Array) -> Vec<bool> {
    let array = array
        .as_any()
        .downcast_ref::<BooleanArray>()
        .expect("expected BooleanArray");
    (0..array.len()).map(|index| array.value(index)).collect()
}
