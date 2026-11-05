use std::fs;

use cytiq_fcs::{FcsReader, FcsWriteOptions, FcsWriteParameter};
use cytiq_transform::{TransformConfig, TransformError, TransformRunner};

#[test]
fn non_finite_input_publishes_neither_fcs_nor_metadata() {
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("input.fcs");
    let output = directory.path().join("output.fcs");
    let metadata = directory.path().join("metrics.json");
    cytiq_fcs::write_fcs3_1_f64_with_options(
        &input,
        &[FcsWriteParameter::new("FL1-A")],
        &[vec![1.0, 2.0]],
        &FcsWriteOptions::new(),
    )
    .unwrap();
    let dataset = FcsReader::open(&input).unwrap().read().unwrap();
    let start = usize::try_from(dataset.header.data.unwrap().start).unwrap();
    let mut bytes = fs::read(&input).unwrap();
    bytes[start..start + 8].copy_from_slice(&f64::NAN.to_le_bytes());
    fs::write(&input, bytes).unwrap();
    let mut config = TransformConfig::new(vec![input], output.clone());
    config.metadata_json = Some(metadata.clone());
    assert!(matches!(
        TransformRunner::run(&config),
        Err(TransformError::NonFiniteChannel { .. })
    ));
    assert!(!output.exists());
    assert!(!metadata.exists());
}
