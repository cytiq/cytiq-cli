use std::error::Error;
use std::path::PathBuf;

use cytiq_fcs::{ColumnData, FcsReader, FcsVersion, FcsWriteOptions, FcsWriteParameter};

mod common;

use common::{build_fcs, encode_u16_rows, overwrite_header_range};

#[test]
fn reads_fcs3_0_integer_fixture() -> Result<(), Box<dyn Error>> {
    let dataset = FcsReader::open(fixture_path("fcs3_0_integer.fcs"))?.read()?;

    assert_eq!(dataset.version, FcsVersion::V3_0);
    assert_eq!(dataset.parameters.len(), 2);
    assert_eq!(dataset.parameters[0].name, "FSC-A");
    assert_eq!(dataset.parameters[1].name, "SSC-A");

    assert_uint_column(dataset.data.column(0), &[1, 2, 3]);
    assert_uint_column(dataset.data.column(1), &[10, 20, 30]);

    Ok(())
}

#[test]
fn reads_fcs3_1_variable_integer_fixture() -> Result<(), Box<dyn Error>> {
    let dataset = FcsReader::open(fixture_path("fcs3_1_variable_integer.fcs"))?.read()?;

    assert_eq!(dataset.version, FcsVersion::V3_1);
    assert_eq!(dataset.parameters.len(), 2);
    assert_uint_column(dataset.data.column(0), &[7, 8]);
    assert_uint_column(dataset.data.column(1), &[1000, 2000]);

    Ok(())
}

#[test]
fn reads_fcs3_2_mixed_fixture() -> Result<(), Box<dyn Error>> {
    let dataset = FcsReader::open(fixture_path("fcs3_2_mixed.fcs"))?.read()?;

    assert_eq!(dataset.version, FcsVersion::V3_2);
    assert_eq!(dataset.parameters.len(), 3);
    assert_uint_column(dataset.data.column(0), &[1, 2]);
    assert_f32_column(dataset.data.column(1), &[1.5, 3.5]);
    assert_f64_column(dataset.data.column(2), &[2.5, 4.5]);
    assert!(
        dataset.parameters[1]
            .keywords
            .iter()
            .any(|(key, value)| key == "$P2DATATYPE" && value == "F")
    );
    assert!(
        dataset.parameters[2]
            .keywords
            .iter()
            .any(|(key, value)| key == "$P3DATATYPE" && value == "D")
    );

    Ok(())
}

#[test]
fn merges_supplemental_text_keywords() -> Result<(), Box<dyn Error>> {
    let bytes = build_fcs(
        "FCS3.1",
        vec![
            ("$BYTEORD", "1,2,3,4"),
            ("$DATATYPE", "I"),
            ("$MODE", "L"),
            ("$NEXTDATA", "00000000"),
            ("$PAR", "1"),
            ("$TOT", "1"),
            ("$P1B", "16"),
            ("$P1E", "0,0"),
            ("$P1N", "Time"),
            ("$P1R", "1024"),
        ],
        Some(vec![("EXTRA", "value")]),
        encode_u16_rows(&[[42]], true),
        None,
    );

    let dataset = FcsReader::new(std::io::Cursor::new(bytes)).read()?;
    assert_eq!(dataset.text.get("EXTRA"), Some("value"));

    Ok(())
}

#[test]
fn reads_analysis_segment_bytes() -> Result<(), Box<dyn Error>> {
    let bytes = build_fcs(
        "FCS3.0",
        vec![
            ("$BYTEORD", "1,2"),
            ("$DATATYPE", "I"),
            ("$MODE", "L"),
            ("$NEXTDATA", "00000000"),
            ("$PAR", "1"),
            ("$TOT", "1"),
            ("$P1B", "16"),
            ("$P1E", "0,0"),
            ("$P1N", "Time"),
            ("$P1R", "1024"),
        ],
        None,
        encode_u16_rows(&[[7]], true),
        Some(vec![0x41, 0x4e, 0x41, 0x4c]),
    );

    let dataset = FcsReader::new(std::io::Cursor::new(bytes)).read()?;
    assert_eq!(dataset.analysis, Some(b"ANAL".to_vec()));

    Ok(())
}

#[test]
fn parses_escaped_text_delimiter() -> Result<(), Box<dyn Error>> {
    let bytes = build_fcs(
        "FCS3.0",
        vec![
            ("$BYTEORD", "1,2"),
            ("$DATATYPE", "I"),
            ("$MODE", "L"),
            ("$NEXTDATA", "00000000"),
            ("$PAR", "1"),
            ("$TOT", "1"),
            ("$P1B", "16"),
            ("$P1E", "0,0"),
            ("$P1N", "CD4"),
            ("$P1S", "CD4|CD8"),
            ("$P1R", "1024"),
        ],
        None,
        encode_u16_rows(&[[9]], true),
        None,
    );

    let dataset = FcsReader::new(std::io::Cursor::new(bytes)).read()?;
    assert_eq!(dataset.parameters[0].label.as_deref(), Some("CD4|CD8"));

    Ok(())
}

#[test]
fn parses_whitespace_padded_layout_keywords() -> Result<(), Box<dyn Error>> {
    let bytes = build_fcs(
        "FCS3.1",
        vec![
            ("$BYTEORD", "1,2,3,4   "),
            ("$DATATYPE", "I   "),
            ("$MODE", "L   "),
            ("$NEXTDATA", "00000000"),
            ("$PAR", "1   "),
            ("$TOT", "1               "),
            ("$P1B", "16   "),
            ("$P1E", "0,0"),
            ("$P1N", "Time"),
            ("$P1R", "1024"),
        ],
        None,
        encode_u16_rows(&[[42]], true),
        None,
    );

    let dataset = FcsReader::new(std::io::Cursor::new(bytes)).read()?;
    assert_uint_column(dataset.data.column(0), &[42]);

    Ok(())
}

#[test]
fn rejects_duplicate_keywords() {
    let bytes = build_fcs(
        "FCS3.1",
        vec![
            ("$BYTEORD", "1,2,3,4"),
            ("$DATATYPE", "I"),
            ("$MODE", "L"),
            ("$NEXTDATA", "00000000"),
            ("$PAR", "1"),
            ("$TOT", "1"),
            ("$TOT", "2"),
            ("$P1B", "16"),
            ("$P1E", "0,0"),
            ("$P1N", "Time"),
            ("$P1R", "1024"),
        ],
        None,
        encode_u16_rows(&[[1]], true),
        None,
    );

    let error = FcsReader::new(std::io::Cursor::new(bytes))
        .read()
        .expect_err("duplicate keyword should fail");
    assert!(error.to_string().contains("duplicate keyword"));
}

#[test]
fn rejects_header_text_offset_mismatch() {
    let mut bytes = build_fcs(
        "FCS3.1",
        vec![
            ("$BYTEORD", "1,2,3,4"),
            ("$DATATYPE", "I"),
            ("$MODE", "L"),
            ("$NEXTDATA", "00000000"),
            ("$PAR", "1"),
            ("$TOT", "1"),
            ("$P1B", "16"),
            ("$P1E", "0,0"),
            ("$P1N", "Time"),
            ("$P1R", "1024"),
        ],
        None,
        encode_u16_rows(&[[99]], true),
        None,
    );
    overwrite_header_range(&mut bytes, 26, 34, 1, 2);

    let error = FcsReader::new(std::io::Cursor::new(bytes))
        .read()
        .expect_err("mismatched DATA offsets should fail");
    assert!(error.to_string().contains("offsets do not match"));
}

#[test]
fn writes_and_reads_fcs3_1_float32() -> Result<(), Box<dyn Error>> {
    let parameters = vec![
        FcsWriteParameter::new("FSC-A"),
        FcsWriteParameter::with_label("FITC", "FITC-A"),
    ];
    let columns = vec![vec![1.0_f32, 2.0, 3.0], vec![10.5_f32, 20.5, 30.5]];
    let mut bytes = Vec::new();
    cytiq_fcs::write_fcs3_1_f32_to_writer(&mut bytes, &parameters, &columns)?;

    let dataset = FcsReader::new(std::io::Cursor::new(bytes)).read()?;
    assert_eq!(dataset.version, FcsVersion::V3_1);
    assert_eq!(dataset.parameters.len(), 2);
    assert_eq!(dataset.parameters[0].name, "FSC-A");
    assert_eq!(dataset.parameters[1].label.as_deref(), Some("FITC-A"));
    assert_f32_column(dataset.data.column(0), &[1.0, 2.0, 3.0]);
    assert_f32_column(dataset.data.column(1), &[10.5, 20.5, 30.5]);

    Ok(())
}

#[test]
fn writes_and_reads_custom_text_keywords() -> Result<(), Box<dyn Error>> {
    let parameters = vec![FcsWriteParameter::new("FL1-A")];
    let columns = vec![vec![1.0_f32, 2.0, 3.0]];
    let options =
        FcsWriteOptions::new().with_text_keyword("CYTIQ_TRANSFORM", "{\"method\":\"none\"}");
    let mut bytes = Vec::new();
    cytiq_fcs::write_fcs3_1_f32_to_writer_with_options(
        &mut bytes,
        &parameters,
        &columns,
        &options,
    )?;

    let dataset = FcsReader::new(std::io::Cursor::new(bytes)).read()?;
    assert_eq!(
        dataset.text.get("CYTIQ_TRANSFORM"),
        Some("{\"method\":\"none\"}")
    );

    Ok(())
}

#[test]
fn parameter_keywords_do_not_leak_across_indexes() -> Result<(), Box<dyn Error>> {
    let parameters = (1..=12)
        .map(|index| FcsWriteParameter::new(format!("CH{index}")))
        .collect::<Vec<_>>();
    let columns = vec![vec![1.0_f32, 2.0]; parameters.len()];
    let mut bytes = Vec::new();
    cytiq_fcs::write_fcs3_1_f32_to_writer(&mut bytes, &parameters, &columns)?;

    let dataset = FcsReader::new(std::io::Cursor::new(bytes)).read()?;
    let first = &dataset.parameters[0].keywords;
    assert!(first.iter().any(|(key, _)| key == "$P1N"));
    assert!(
        first
            .iter()
            .all(|(key, _)| !key.starts_with("$P10") && !key.starts_with("$P11")),
        "unexpected keywords for $P1: {first:?}"
    );

    Ok(())
}

fn fixture_path(name: &str) -> PathBuf {
    fixture_dir().join(name)
}

fn fixture_dir() -> PathBuf {
    for start in fixture_search_roots() {
        for candidate in start
            .ancestors()
            .map(|ancestor| ancestor.join("data").join("fixtures").join("fcs"))
        {
            if candidate.is_dir() {
                return candidate;
            }
        }
    }

    panic!("fixture directory should exist: data/fixtures/fcs");
}

fn fixture_search_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();

    if let Ok(current_dir) = std::env::current_dir() {
        roots.push(current_dir);
    }

    if let Ok(current_exe) = std::env::current_exe() {
        roots.push(current_exe);
    }

    roots.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")));
    roots
}

fn assert_uint_column(column: Option<&ColumnData>, expected: &[u64]) {
    match column {
        Some(ColumnData::UInt(values)) => assert_eq!(values, expected),
        other => panic!("unexpected uint column: {other:?}"),
    }
}

fn assert_f32_column(column: Option<&ColumnData>, expected: &[f32]) {
    match column {
        Some(ColumnData::F32(values)) => assert_eq!(values, expected),
        other => panic!("unexpected f32 column: {other:?}"),
    }
}

fn assert_f64_column(column: Option<&ColumnData>, expected: &[f64]) {
    match column {
        Some(ColumnData::F64(values)) => assert_eq!(values, expected),
        other => panic!("unexpected f64 column: {other:?}"),
    }
}

#[test]
fn round_trips_f64_dataset() -> Result<(), Box<dyn Error>> {
    let parameters = vec![FcsWriteParameter::new("Original_ID")];
    // 2^24 + 1 is the first integer f32 cannot represent.
    let ids = vec![16_777_217.0_f64, 16_777_218.0, 16_777_219.0];
    let mut bytes = Vec::new();
    cytiq_fcs::write_fcs3_1_f64_to_writer_with_options(
        &mut bytes,
        &parameters,
        std::slice::from_ref(&ids),
        &FcsWriteOptions::new(),
    )?;

    let dataset = FcsReader::new(std::io::Cursor::new(bytes)).read()?;
    assert_eq!(dataset.text.get("$DATATYPE"), Some("D"));
    assert_eq!(dataset.text.get("$P1B"), Some("64"));
    assert_f64_column(dataset.data.column(0), &ids);

    Ok(())
}

#[test]
fn carries_source_keywords_into_derived_file() -> Result<(), Box<dyn Error>> {
    let source_parameters = vec![
        FcsWriteParameter::new("FSC-A")
            .with_range(262_144.0)
            .with_keyword("V", "350"),
        FcsWriteParameter::with_label("FL1-A", "CD3").with_keyword("G", "1.5"),
    ];
    let options = FcsWriteOptions::new()
        .with_text_keyword("$CYT", "Aurora")
        .with_text_keyword("$PROJ", "study")
        .with_text_keyword("$SPILLOVER", "1,FL1-A,1");
    let mut bytes = Vec::new();
    cytiq_fcs::write_fcs3_1_f32_to_writer_with_options(
        &mut bytes,
        &source_parameters,
        &[vec![1.0_f32, 2.0], vec![3.0_f32, 4.0]],
        &options,
    )?;
    let source = FcsReader::new(std::io::Cursor::new(bytes)).read()?;

    let parameters = source
        .parameters
        .iter()
        .map(FcsWriteParameter::from_source)
        .collect::<Vec<_>>();
    let options =
        FcsWriteOptions::new().with_text_keywords(cytiq_fcs::carried_keywords(&source.text));
    let mut bytes = Vec::new();
    cytiq_fcs::write_fcs3_1_f32_to_writer_with_options(
        &mut bytes,
        &parameters,
        &[vec![1.0_f32], vec![3.0_f32]],
        &options,
    )?;
    let derived = FcsReader::new(std::io::Cursor::new(bytes)).read()?;

    assert_eq!(derived.text.get("$CYT"), Some("Aurora"));
    assert_eq!(derived.text.get("$PROJ"), Some("study"));
    assert_eq!(derived.text.get("$SPILLOVER"), Some("1,FL1-A,1"));
    assert_eq!(derived.text.get("$TOT"), Some("1"));
    assert_eq!(derived.text.get("$P1R"), Some("262144"));
    assert_eq!(derived.text.get("$P1V"), Some("350"));
    assert_eq!(derived.text.get("$P2S"), Some("CD3"));
    assert_eq!(derived.text.get("$P2G"), Some("1.5"));

    Ok(())
}

#[test]
#[ignore = "writes a ~120 MB file; run with --ignored"]
fn writes_and_reads_dataset_over_100mb() -> Result<(), Box<dyn Error>> {
    let events = 30_000_000_usize;
    let parameters = vec![FcsWriteParameter::new("FL1-A")];
    #[allow(clippy::cast_precision_loss)]
    let column = (0..events)
        .map(|index| (index % 1024) as f32)
        .collect::<Vec<_>>();
    let directory = std::env::temp_dir().join(format!("cytiq-large-{}", std::process::id()));
    std::fs::create_dir_all(&directory)?;
    let path = directory.join("large.fcs");
    cytiq_fcs::write_fcs3_1_f32(&path, &parameters, &[column])?;

    let dataset = FcsReader::open(&path)?.read()?;
    std::fs::remove_dir_all(&directory)?;
    assert_eq!(dataset.data.column(0).map(ColumnData::len), Some(events));
    match dataset.data.column(0) {
        Some(ColumnData::F32(values)) => assert_eq!(values[events - 1], 895.0),
        other => panic!("expected F32 column, got {other:?}"),
    }

    Ok(())
}
