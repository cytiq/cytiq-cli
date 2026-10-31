//! Pins the exact bytes of every prediction and summary across the fixture
//! bundles, batch sizes and masks, so classify refactors can prove they are
//! bit-identical. Run with `CYTIQ_BLESS=1` to rewrite
//! `data/fixtures/characterization/classify.json` after an intended change.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs::{self, File};
use std::path::{Path, PathBuf};

use arrow_array::{Array, BooleanArray, Float32Array, StringArray, UInt64Array};
use cytiq_classify::{ClassifyConfig, ClassifyRunner};
use cytiq_fcs::FcsReader;
use cytiq_mask::EventMask;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use sha2::{Digest, Sha256};

/// (case name, bundle directory, input FCS), relative to the classify fixtures.
const CASES: &[(&str, &str, &str)] = &[
    ("v1-catboost", "catboost", "raw.fcs"),
    ("v1-lightgbm", "lightgbm", "raw.fcs"),
    ("v2-binary", "v2/binary", "raw.fcs"),
    ("v2-catboost", "v2/catboost", "v2/input.fcs"),
    ("v2-lightgbm", "v2/lightgbm", "v2/input.fcs"),
    ("cytof-lightgbm", "cytof/lightgbm", "cytof/input.fcs"),
    ("cytof-xgboost", "cytof/xgboost", "cytof/input.fcs"),
];
const BATCH_SIZES: &[usize] = &[1, 17, 65_536];

#[test]
fn predictions_match_characterization_digests() {
    let temp = tempfile::tempdir().unwrap();
    let mut actual = BTreeMap::new();
    for &(name, bundle, input) in CASES {
        for masked in [false, true] {
            let mut digests = Vec::new();
            for &batch_size in BATCH_SIZES {
                let output = temp.path().join(format!("{name}-{masked}-{batch_size}"));
                let config = ClassifyConfig {
                    input: fixtures().join(input),
                    model: fixtures().join(bundle),
                    output: output.clone(),
                    masks: if masked {
                        vec![alternating_mask(&fixtures().join(input), temp.path(), name)]
                    } else {
                        vec![]
                    },
                    threads: Some(1),
                    batch_size,
                    abstain_margin: 0.0,
                    overwrite: false,
                };
                ClassifyRunner::run(&config).unwrap();
                digests.push(digest_outputs(&output));
            }
            assert!(
                digests.windows(2).all(|pair| pair[0] == pair[1]),
                "{name} (masked: {masked}) depends on the batch size"
            );
            let key = format!("{name}{}", if masked { "-masked" } else { "" });
            actual.insert(key, digests.swap_remove(0));
        }
    }

    let golden = fixtures().join("../characterization/classify.json");
    if std::env::var_os("CYTIQ_BLESS").is_some() {
        let mut json = serde_json::to_string_pretty(&actual).unwrap();
        json.push('\n');
        fs::write(&golden, json).unwrap();
        return;
    }
    let expected: BTreeMap<String, String> = serde_json::from_str(
        &fs::read_to_string(&golden)
            .unwrap_or_else(|error| panic!("read {golden:?} (bless with CYTIQ_BLESS=1): {error}")),
    )
    .unwrap();
    assert_eq!(
        actual, expected,
        "classify output changed; if intended, rerun with CYTIQ_BLESS=1"
    );
}

fn alternating_mask(input: &Path, directory: &Path, name: &str) -> PathBuf {
    let path = directory.join(format!("{name}.mask.parquet"));
    if !path.exists() {
        let dataset = FcsReader::open(input).unwrap().read().unwrap();
        let acquisition = cytiq_fcs::acquisition_id(input, &dataset.text).unwrap();
        EventMask::new(
            acquisition,
            (0..dataset.event_count()).map(|i| i % 3 != 1).collect(),
            None,
        )
        .write_parquet(&path)
        .unwrap();
    }
    path
}

/// Digest of every decoded prediction value plus the summary bytes. Floats are
/// hashed by their bit patterns, so any change in arithmetic shows up.
fn digest_outputs(output: &Path) -> String {
    let mut hasher = Sha256::new();
    let reader = ParquetRecordBatchReaderBuilder::try_new(
        File::open(output.join("predictions.parquet")).unwrap(),
    )
    .unwrap();
    let schema = reader.schema().clone();
    let mut metadata = schema.metadata().iter().collect::<Vec<_>>();
    metadata.sort();
    hasher.update(format!("{metadata:?}{:?}", schema.fields()).as_bytes());
    for batch in reader.build().unwrap() {
        let batch = batch.unwrap();
        for column in batch.columns() {
            let any = column.as_any();
            for row in 0..column.len() {
                if column.is_null(row) {
                    hasher.update(b"null");
                } else if let Some(values) = any.downcast_ref::<UInt64Array>() {
                    hasher.update(values.value(row).to_le_bytes());
                } else if let Some(values) = any.downcast_ref::<Float32Array>() {
                    hasher.update(values.value(row).to_bits().to_le_bytes());
                } else if let Some(values) = any.downcast_ref::<BooleanArray>() {
                    hasher.update([u8::from(values.value(row))]);
                } else if let Some(values) = any.downcast_ref::<StringArray>() {
                    hasher.update(values.value(row).as_bytes());
                    hasher.update([0]);
                } else {
                    panic!("unexpected prediction column type {:?}", column.data_type());
                }
            }
        }
    }
    hasher.update(fs::read(output.join("summary.json")).unwrap());
    hasher
        .finalize()
        .iter()
        .fold(String::with_capacity(64), |mut hex, byte| {
            let _ = write!(hex, "{byte:02x}");
            hex
        })
}

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data/fixtures/classify")
}
