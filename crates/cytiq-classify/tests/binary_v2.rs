use std::fs::File;
use std::path::PathBuf;

use arrow_array::{Float32Array, StringArray};
use cytiq_classify::{Bundle, ClassifyConfig, ClassifyRunner};
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

#[test]
fn binary_v2_retains_saved_threshold_and_exposes_generic_class_counts() {
    let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data/fixtures/classify");
    let temp = tempfile::tempdir().unwrap();
    let cfg = ClassifyConfig {
        input: fixtures.join("raw.fcs"),
        model: fixtures.join("v2/binary"),
        output: temp.path().join("predictions"),
        masks: vec![],
        threads: Some(1),
        batch_size: 17,
        abstain_margin: 0.0,
        overwrite: false,
    };
    let bundle = Bundle::load(&cfg.model, 0.0).unwrap();
    let result = ClassifyRunner::run(&cfg).unwrap();
    assert_eq!(result.schema_version, 2);
    assert_eq!(result.task.as_deref(), Some("blast"));
    assert_eq!(result.assessed, 128);
    assert_eq!(result.class_counts.as_ref().unwrap()["blast"], result.blast);
    let expected: Vec<Vec<f64>> =
        serde_json::from_reader(File::open(fixtures.join("v2/binary.expected.json")).unwrap())
            .unwrap();
    let reader = ParquetRecordBatchReaderBuilder::try_new(
        File::open(cfg.output.join("predictions.parquet")).unwrap(),
    )
    .unwrap()
    .build()
    .unwrap();
    let mut row = 0;
    for batch in reader {
        let batch = batch.unwrap();
        let probabilities = batch
            .column(4)
            .as_any()
            .downcast_ref::<Float32Array>()
            .unwrap();
        let labels = batch
            .column(5)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        for index in 0..batch.num_rows() {
            assert!((f64::from(probabilities.value(index)) - expected[row][1]).abs() < 1e-5);
            let expected_label = if expected[row][1] >= f64::from(bundle.threshold) {
                "blast"
            } else {
                "non_blast"
            };
            assert_eq!(labels.value(index), expected_label);
            row += 1;
        }
    }
    assert_eq!(row, 128);
}
