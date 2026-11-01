//! What the fuzz targets check. `tests/fuzz_regressions.rs` includes this file
//! too, so `cargo test` replays every saved input against the same checks.

use std::io::Cursor;

use cytiq_fcs::{ColumnData, Error, FcsDataset, FcsReadLimits, FcsReader, Spillover};

/// Limits small enough that fuzzed files reach them.
const TIGHT_LIMITS: FcsReadLimits = FcsReadLimits {
    decoded_bytes_max: 4096,
    analysis_bytes_max: 64,
};

/// Any bytes: the reader returns `Ok` or `Err`, never panics, aborts or hangs,
/// and what it accepts is consistent. Read limits only ever refuse a file:
/// a read within tight limits returns what an unlimited read returns, or
/// `LimitExceeded`.
pub fn read_fcs(bytes: &[u8]) {
    let read = FcsReader::new(Cursor::new(bytes)).read();
    if let Ok(dataset) = &read {
        let events = dataset.event_count();
        assert_eq!(dataset.data.columns.len(), dataset.parameters.len());
        assert!(
            dataset
                .data
                .columns
                .iter()
                .all(|column| column.len() == events)
        );
        assert!(events <= bytes.len());
    }
    match (
        FcsReader::new(Cursor::new(bytes)).read_with_limits(TIGHT_LIMITS),
        &read,
    ) {
        (Ok(limited), Ok(dataset)) => assert_same(&limited, dataset),
        (Ok(_), Err(error)) => panic!("only the limited read accepted the file: {error}"),
        (Err(Error::LimitExceeded { .. }), _) => {}
        (Err(limited), Ok(_)) => panic!("only the unlimited read accepted the file: {limited}"),
        (Err(limited), Err(error)) => assert_eq!(limited.to_string(), error.to_string()),
    }
}

/// Derived equality fails on the NaN values DATA may hold, so columns are
/// compared bit for bit.
fn assert_same(left: &FcsDataset, right: &FcsDataset) {
    assert_eq!(left.version, right.version);
    assert_eq!(left.header, right.header);
    assert_eq!(left.text, right.text);
    assert_eq!(left.parameters, right.parameters);
    assert_eq!(left.analysis, right.analysis);
    let bits = |dataset: &FcsDataset| {
        dataset
            .data
            .columns
            .iter()
            .map(column_bits)
            .collect::<Vec<_>>()
    };
    assert_eq!(bits(left), bits(right));
}

fn column_bits(column: &ColumnData) -> (&'static str, Vec<u64>) {
    match column {
        ColumnData::UInt(values) => ("u64", values.clone()),
        ColumnData::F32(values) => (
            "f32",
            values
                .iter()
                .map(|value| u64::from(value.to_bits()))
                .collect(),
        ),
        ColumnData::F64(values) => ("f64", values.iter().map(|value| value.to_bits()).collect()),
    }
}

/// Any keyword value: parsing never panics, and an accepted matrix is square
/// over its channels and finite. Values that are not UTF-8 cannot be TEXT
/// values and are skipped.
pub fn spillover_keyword(bytes: &[u8]) {
    let Ok(raw) = std::str::from_utf8(bytes) else {
        return;
    };
    if let Ok(spillover) = Spillover::parse_keyword(raw) {
        let size = spillover.size();
        assert_eq!(spillover.channels().len(), size);
        assert_eq!(spillover.matrix().len(), size * size);
        assert!(spillover.matrix().iter().all(|value| value.is_finite()));
    }
}
