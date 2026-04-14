mod support;

use std::fs::File;

use arrow_array::{Array, BooleanArray, Int64Array};
use cytiq_gate::{GateCommand, run_gate};
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
