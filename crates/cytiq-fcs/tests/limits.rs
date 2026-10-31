//! Crafted files whose metadata lies about sizes. Each must come back as a
//! specific `Err` before the reader allocates or loops on the lie, never as a
//! panic, an abort or a hang.

use std::io::Cursor;
use std::time::{Duration, Instant};

use cytiq_fcs::{Error, FcsReader, Spillover};

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
