//! `FcsReader::stream` checks a file as `read` does and decodes the same
//! events, one batch at a time.

use std::cell::Cell;
use std::io::{Cursor, Read, Seek, SeekFrom};
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use cytiq_fcs::{ColumnData, Error, FcsReadLimits, FcsReader};

mod common;

use common::{assert_stream_matches_read, build_fcs, encode_u16_rows};

fn fcs_files_under(directory: &Path, paths: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(directory).expect("read fixture directory") {
        let path = entry.expect("read fixture entry").path();
        if path.is_dir() {
            fcs_files_under(&path, paths);
        } else if path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("fcs"))
        {
            paths.push(path);
        }
    }
}

fn batch(size: usize) -> NonZeroUsize {
    NonZeroUsize::new(size).expect("batch size is positive")
}

fn one_parameter(rows: usize) -> Vec<(String, String)> {
    vec![
        ("$BYTEORD".into(), "1,2,3,4".into()),
        ("$DATATYPE".into(), "A".into()),
        ("$MODE".into(), "L".into()),
        ("$NEXTDATA".into(), "0".into()),
        ("$PAR".into(), "1".into()),
        ("$TOT".into(), rows.to_string()),
        ("$P1B".into(), "3".into()),
        ("$P1E".into(), "0,0".into()),
        ("$P1N".into(), "P1".into()),
        ("$P1R".into(), "1024".into()),
    ]
}

fn ascii_file(values: &[&str], analysis: Option<Vec<u8>>) -> Vec<u8> {
    let pairs = one_parameter(values.len());
    let data = values
        .iter()
        .flat_map(|value| format!("{value:>3}").into_bytes())
        .collect();
    build_fcs(
        "FCS3.1",
        pairs
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect(),
        None,
        data,
        analysis,
    )
}

#[test]
fn streams_every_fixture_like_read() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/fixtures");
    let mut paths = Vec::new();
    fcs_files_under(&root, &mut paths);
    assert!(paths.len() > 20, "found only {} fixtures", paths.len());
    for path in paths {
        let bytes = std::fs::read(&path).expect("read fixture");
        for size in [1, 7, 65_536] {
            assert_stream_matches_read(&bytes, size);
        }
    }
}

#[test]
fn batches_hold_batch_size_events_and_the_last_the_rest() {
    let bytes = ascii_file(&["1", "2", "3", "4", "5", "6", "7"], None);
    let stream = FcsReader::new(Cursor::new(bytes))
        .stream(batch(3), FcsReadLimits::default())
        .expect("valid file");
    assert_eq!(stream.metadata.event_count, 7);
    let batches = stream
        .batches
        .map(
            |batch| match batch.expect("valid batch").columns.as_slice() {
                [ColumnData::UInt(values)] => values.clone(),
                other => panic!("unexpected columns {other:?}"),
            },
        )
        .collect::<Vec<_>>();
    assert_eq!(batches, [vec![1, 2, 3], vec![4, 5, 6], vec![7]]);
}

#[test]
fn stops_after_the_first_error() {
    let bytes = ascii_file(&["1", "x", "3"], None);
    let mut batches = FcsReader::new(Cursor::new(bytes))
        .stream(batch(1), FcsReadLimits::default())
        .expect("TEXT and layout are valid")
        .batches;
    assert!(batches.next().is_some_and(|batch| batch.is_ok()));
    assert!(matches!(
        batches.next(),
        Some(Err(Error::EventValueParse { parameter: 1, .. }))
    ));
    assert!(batches.next().is_none());
}

/// A reader that records how far into the file anything was read.
#[derive(Debug)]
struct Watched {
    inner: Cursor<Vec<u8>>,
    furthest: Rc<Cell<u64>>,
}

impl Read for Watched {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let read = self.inner.read(buffer)?;
        self.furthest
            .set(self.furthest.get().max(self.inner.position()));
        Ok(read)
    }
}

impl Seek for Watched {
    fn seek(&mut self, position: SeekFrom) -> std::io::Result<u64> {
        self.inner.seek(position)
    }
}

fn watched(bytes: Vec<u8>) -> (Watched, Rc<Cell<u64>>) {
    let furthest = Rc::new(Cell::new(0));
    let reader = Watched {
        inner: Cursor::new(bytes),
        furthest: Rc::clone(&furthest),
    };
    (reader, furthest)
}

fn header_offset(bytes: &[u8], at: usize) -> u64 {
    std::str::from_utf8(&bytes[at..at + 8])
        .expect("HEADER offsets are ASCII")
        .trim()
        .parse()
        .expect("HEADER offsets are numbers")
}

#[test]
fn stream_never_reads_analysis() {
    let bytes = ascii_file(&["1", "2"], Some(b"ANALYSIS".to_vec()));
    let data_end = header_offset(&bytes, 34);
    let tight = FcsReadLimits {
        analysis_bytes_max: 0,
        ..FcsReadLimits::default()
    };
    assert!(matches!(
        FcsReader::new(Cursor::new(bytes.clone())).read_with_limits(tight),
        Err(Error::LimitExceeded {
            what: "ANALYSIS bytes",
            ..
        })
    ));

    let (reader, furthest) = watched(bytes);
    let stream = FcsReader::new(reader)
        .stream(batch(1), tight)
        .expect("ANALYSIS limits do not apply to a stream");
    assert_eq!(stream.batches.map(Result::unwrap).count(), 2);
    assert!(
        furthest.get() <= data_end + 1,
        "read up to {}, DATA ends at {data_end}",
        furthest.get()
    );
}

#[test]
fn rejects_oversized_data_before_streaming_it() {
    let bytes = build_fcs(
        "FCS3.1",
        vec![
            ("$BYTEORD", "1,2,3,4"),
            ("$DATATYPE", "I"),
            ("$MODE", "L"),
            ("$NEXTDATA", "0"),
            ("$PAR", "1"),
            ("$TOT", "2"),
            ("$P1B", "16"),
            ("$P1E", "0,0"),
            ("$P1N", "Time"),
            ("$P1R", "1024"),
        ],
        None,
        encode_u16_rows(&[[1], [2]], true),
        None,
    );
    let data_start = header_offset(&bytes, 26);
    let (reader, furthest) = watched(bytes);
    let limits = FcsReadLimits {
        decoded_bytes_max: 15,
        ..FcsReadLimits::default()
    };
    let error = FcsReader::new(reader)
        .stream(batch(1), limits)
        .expect_err("16 decoded bytes over a limit of 15");
    assert!(
        matches!(
            error,
            Error::LimitExceeded {
                what: "decoded DATA bytes",
                value: 16,
                limit: 15
            }
        ),
        "{error}"
    );
    assert!(furthest.get() <= data_start);
}

#[test]
fn empty_batch_has_the_columns_of_every_batch() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/fixtures/fcs");
    let bytes = std::fs::read(root.join("fcs3_2_mixed.fcs")).expect("fixture exists");
    let stream = FcsReader::new(Cursor::new(bytes))
        .stream(batch(4), FcsReadLimits::default())
        .expect("fixture streams");
    let kind = |column: &ColumnData| std::mem::discriminant(column);
    let empty = stream.batches.empty_batch();
    assert!(empty.columns.iter().all(|column| column.len() == 0));
    for batch in stream.batches {
        let batch = batch.expect("fixture batch");
        let kinds = batch.columns.iter().map(kind).collect::<Vec<_>>();
        assert_eq!(kinds, empty.columns.iter().map(kind).collect::<Vec<_>>());
    }
}
