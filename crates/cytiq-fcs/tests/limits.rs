//! Crafted files whose metadata lies about sizes. Each must come back as a
//! specific `Err` before the reader allocates or loops on the lie, never as a
//! panic, an abort or a hang.

use std::cell::Cell;
use std::io::{Cursor, Read, Seek, SeekFrom};
use std::rc::Rc;
use std::time::{Duration, Instant};

use cytiq_fcs::{Error, FcsReadLimits, FcsReader, Spillover};

mod common;

use common::{build_fcs, build_fcs_overriding, encode_u16_rows, overwrite_header_range};

const U64_MAX: &str = "18446744073709551615";

fn one_parameter() -> Vec<(&'static str, &'static str)> {
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
    ]
}

fn read_overriding(overrides: &[(&str, &str)]) -> Result<cytiq_fcs::FcsDataset, Error> {
    let bytes = build_fcs_overriding(
        "FCS3.1",
        one_parameter(),
        None,
        encode_u16_rows(&[[1], [2]], true),
        None,
        overrides,
    );
    FcsReader::new(Cursor::new(bytes)).read()
}

#[test]
fn rejects_supplemental_text_ending_at_u64_max() {
    let error = read_overriding(&[("$BEGINSTEXT", "1"), ("$ENDSTEXT", U64_MAX)])
        .expect_err("supplemental TEXT past the end of the file must fail");
    assert!(
        matches!(
            error,
            Error::SegmentOutOfBounds {
                name: "supplemental TEXT",
                ..
            }
        ),
        "{error}"
    );
}

#[test]
fn rejects_supplemental_text_spanning_the_whole_offset_range() {
    let error = read_overriding(&[("$BEGINSTEXT", "0"), ("$ENDSTEXT", U64_MAX)])
        .expect_err("supplemental TEXT past the end of the file must fail");
    assert!(
        matches!(
            error,
            Error::SegmentOutOfBounds {
                name: "supplemental TEXT",
                ..
            }
        ),
        "{error}"
    );
}

#[test]
fn rejects_parameter_count_beyond_the_format_limit() {
    let error = read_overriding(&[("$PAR", "1000000000000000")])
        .expect_err("a $PAR beyond 65535 must fail");
    assert!(
        matches!(error, Error::LimitExceeded { what: "$PAR", .. }),
        "{error}"
    );
}

#[test]
fn rejects_zero_parameters() {
    let error = read_overriding(&[("$PAR", "0")]).expect_err("$PAR=0 must fail");
    assert!(
        matches!(&error, Error::InvalidKeywordValue { key, .. } if key == "$PAR"),
        "{error}"
    );
}

#[test]
fn rejects_primary_text_outside_the_file() {
    let mut bytes = build_fcs(
        "FCS3.1",
        one_parameter(),
        None,
        encode_u16_rows(&[[1], [2]], true),
        None,
    );
    overwrite_header_range(&mut bytes, 10, 18, 58, 99_999_999);
    let error = FcsReader::new(Cursor::new(bytes))
        .read()
        .expect_err("TEXT past the end of the file must fail");
    assert!(
        matches!(error, Error::SegmentOutOfBounds { name: "TEXT", .. }),
        "{error}"
    );
}

#[test]
fn rejects_data_overlapping_text() {
    let mut bytes = build_fcs_overriding(
        "FCS3.1",
        one_parameter(),
        None,
        encode_u16_rows(&[[1], [2]], true),
        None,
        &[("$BEGINDATA", "00000058"), ("$ENDDATA", "00000061")],
    );
    overwrite_header_range(&mut bytes, 26, 34, 0, 0);
    let error = FcsReader::new(Cursor::new(bytes))
        .read()
        .expect_err("DATA inside TEXT must fail");
    assert!(matches!(error, Error::SegmentOverlap { .. }), "{error}");
}

#[test]
fn rejects_event_count_that_disagrees_with_data_length() {
    let error = read_overriding(&[("$TOT", "3")]).expect_err("$TOT=3 over 2 rows must fail");
    assert!(
        matches!(
            error,
            Error::DataLengthMismatch {
                expected: 6,
                actual: 4
            }
        ),
        "{error}"
    );
}

#[test]
fn rejects_event_count_whose_data_length_overflows() {
    let error = read_overriding(&[("$TOT", U64_MAX)]).expect_err("an overflowing $TOT must fail");
    assert!(matches!(error, Error::UnsupportedLayout(_)), "{error}");
}

#[test]
fn rejects_unsupported_byte_order() {
    let error = read_overriding(&[("$BYTEORD", "2,1")]).expect_err("BYTEORD 2,1 is not FCS 3.1");
    assert!(matches!(error, Error::UnsupportedByteOrder(_)), "{error}");
}

#[test]
fn rejects_unsupported_integer_width() {
    let error = read_overriding(&[("$P1B", "12")]).expect_err("12-bit integers are unsupported");
    assert!(
        matches!(error, Error::UnsupportedWidth { parameter: 1, .. }),
        "{error}"
    );
}

#[test]
fn reads_sixty_thousand_parameters_in_linear_time() {
    const PARAMETERS: usize = 60_000;
    let names = (1..=PARAMETERS)
        .map(|index| {
            [
                (format!("$P{index}B"), "8".to_owned()),
                (format!("$P{index}E"), "0,0".to_owned()),
                (format!("$P{index}N"), format!("P{index}")),
                (format!("$P{index}R"), "256".to_owned()),
            ]
        })
        .collect::<Vec<_>>();
    let count = PARAMETERS.to_string();
    let mut pairs = vec![
        ("$BYTEORD", "1,2,3,4"),
        ("$DATATYPE", "I"),
        ("$MODE", "L"),
        ("$NEXTDATA", "0"),
        ("$PAR", count.as_str()),
        ("$TOT", "1"),
    ];
    pairs.extend(
        names
            .iter()
            .flatten()
            .map(|(key, value)| (key.as_str(), value.as_str())),
    );
    let bytes = build_fcs("FCS3.1", pairs, None, vec![7; PARAMETERS], None);

    let started = Instant::now();
    let dataset = FcsReader::new(Cursor::new(bytes))
        .read()
        .expect("60,000 parameters are valid");
    let elapsed = started.elapsed();

    assert_eq!(dataset.parameters.len(), PARAMETERS);
    assert_eq!(
        dataset.parameters[PARAMETERS - 1].keywords.len(),
        4,
        "keywords of $P60000 only"
    );
    // Grouping `$Pn*` keywords per parameter by scanning all keywords each
    // time is quadratic: about 10^10 comparisons here, minutes in debug.
    assert!(elapsed < Duration::from_secs(20), "took {elapsed:?}");
}

#[test]
fn rejects_spillover_channel_count_that_overflows() {
    for raw in [U64_MAX, "4294967296,A,B", "3,A,B,C,1,0"] {
        let error = Spillover::parse_keyword(raw).expect_err("malformed spillover must fail");
        assert!(
            matches!(error, Error::InvalidSpillover(_)),
            "{raw}: {error}"
        );
    }
}

/// Two 16-bit events of one parameter: 4 bytes stored, 16 decoded.
fn two_u16_events(analysis: Option<Vec<u8>>) -> Vec<u8> {
    build_fcs(
        "FCS3.1",
        one_parameter(),
        None,
        encode_u16_rows(&[[1], [2]], true),
        analysis,
    )
}

fn limits(decoded_bytes_max: u64, analysis_bytes_max: u64) -> FcsReadLimits {
    FcsReadLimits {
        decoded_bytes_max,
        analysis_bytes_max,
    }
}

#[test]
fn rejects_decoded_data_beyond_the_limit() {
    let error = FcsReader::new(Cursor::new(two_u16_events(None)))
        .read_with_limits(limits(15, 0))
        .expect_err("16 decoded bytes over a limit of 15 must fail");
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
}

#[test]
fn reads_decoded_data_exactly_at_the_limit() {
    let dataset = FcsReader::new(Cursor::new(two_u16_events(None)))
        .read_with_limits(limits(16, 0))
        .expect("16 decoded bytes fit a limit of 16");
    assert_eq!(dataset.data.columns[0].len(), 2);
}

#[test]
fn limits_decoded_size_not_stored_size() {
    // Integers widen to u64: 4 stored bytes are 16 in memory.
    let error = FcsReader::new(Cursor::new(two_u16_events(None)))
        .read_with_limits(limits(4, 0))
        .expect_err("the stored size is not what is allocated");
    assert!(
        matches!(error, Error::LimitExceeded { value: 16, .. }),
        "{error}"
    );

    // F stays 4 bytes a value.
    let mut pairs = one_parameter();
    pairs.retain(|(key, _)| !matches!(*key, "$DATATYPE" | "$P1B"));
    pairs.extend([("$DATATYPE", "F"), ("$P1B", "32")]);
    let data = [1.0_f32, 2.0]
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect();
    let bytes = build_fcs("FCS3.1", pairs, None, data, None);
    FcsReader::new(Cursor::new(bytes))
        .read_with_limits(limits(8, 0))
        .expect("two f32 values decode to 8 bytes");
}

#[test]
fn rejects_analysis_beyond_the_limit() {
    let error = FcsReader::new(Cursor::new(two_u16_events(Some(vec![b'A'; 5]))))
        .read_with_limits(limits(16, 4))
        .expect_err("5 ANALYSIS bytes over a limit of 4 must fail");
    assert!(
        matches!(
            error,
            Error::LimitExceeded {
                what: "ANALYSIS bytes",
                value: 5,
                limit: 4
            }
        ),
        "{error}"
    );
}

#[test]
fn reads_analysis_exactly_at_the_limit() {
    let dataset = FcsReader::new(Cursor::new(two_u16_events(Some(vec![b'A'; 4]))))
        .read_with_limits(limits(16, 4))
        .expect("4 ANALYSIS bytes fit a limit of 4");
    assert_eq!(dataset.analysis, Some(b"AAAA".to_vec()));
}

/// A reader that records how far into the file anything was read.
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

fn furthest_read(bytes: Vec<u8>, limits: FcsReadLimits) -> (Result<(), Error>, u64) {
    let furthest = Rc::new(Cell::new(0));
    let reader = Watched {
        inner: Cursor::new(bytes),
        furthest: Rc::clone(&furthest),
    };
    let result = FcsReader::new(reader).read_with_limits(limits).map(drop);
    (result, furthest.get())
}

fn data_start(bytes: &[u8]) -> u64 {
    std::str::from_utf8(&bytes[26..34])
        .expect("HEADER offsets are ASCII")
        .trim()
        .parse()
        .expect("HEADER offsets are numbers")
}

#[test]
fn rejects_oversized_data_before_reading_it() {
    let bytes = two_u16_events(None);
    let start = data_start(&bytes);

    let (result, furthest) = furthest_read(bytes, limits(15, 0));

    assert!(
        matches!(result, Err(Error::LimitExceeded { .. })),
        "{result:?}"
    );
    assert!(
        furthest <= start,
        "read up to {furthest}, DATA starts at {start}"
    );
}

#[test]
fn rejects_oversized_analysis_before_decoding_data() {
    let bytes = two_u16_events(Some(vec![b'A'; 5]));
    let start = data_start(&bytes);

    let (result, furthest) = furthest_read(bytes, limits(16, 4));

    assert!(
        matches!(
            result,
            Err(Error::LimitExceeded {
                what: "ANALYSIS bytes",
                ..
            })
        ),
        "{result:?}"
    );
    assert!(
        furthest <= start,
        "read up to {furthest}, DATA starts at {start}"
    );
}

#[test]
fn read_uses_the_default_limits() {
    let bytes = two_u16_events(Some(b"ANAL".to_vec()));
    let read = FcsReader::new(Cursor::new(bytes.clone()))
        .read()
        .expect("a small file reads");
    let limited = FcsReader::new(Cursor::new(bytes))
        .read_with_limits(FcsReadLimits::default())
        .expect("a small file reads within the defaults");

    assert_eq!(read, limited);
    assert_eq!(
        FcsReadLimits::default(),
        limits(8 << 30, 16 << 20),
        "README and limits.rs state these values"
    );
}
