use std::error::Error;
use std::path::PathBuf;

use cytiq_fcs::{ColumnData, FcsReader, FcsVersion};

const DELIMITER: u8 = b'|';

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

fn build_fcs(
    version: &str,
    primary_pairs: Vec<(&str, &str)>,
    supplemental_pairs: Option<Vec<(&str, &str)>>,
    data: Vec<u8>,
    analysis: Option<Vec<u8>>,
) -> Vec<u8> {
    let mut primary = primary_pairs
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .collect::<Vec<_>>();
    set_pair(&mut primary, "$BEGINSTEXT", "00000000");
    set_pair(&mut primary, "$ENDSTEXT", "00000000");
    set_pair(&mut primary, "$BEGINDATA", "00000000");
    set_pair(&mut primary, "$ENDDATA", "00000000");
    set_pair(&mut primary, "$BEGINANALYSIS", "00000000");
    set_pair(&mut primary, "$ENDANALYSIS", "00000000");

    let placeholder_primary = encode_text(&primary);
    let text_start = 58_u64;
    let text_end =
        text_start + u64::try_from(placeholder_primary.len()).expect("text length fits") - 1;

    let supplemental = supplemental_pairs.map(|pairs| {
        pairs
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value.to_owned()))
            .collect::<Vec<_>>()
    });
    let supplemental_bytes = supplemental.as_ref().map(|pairs| encode_text(pairs));
    let supplemental_range = supplemental_bytes.as_ref().map(|bytes| {
        let start = text_end + 1;
        let end = start + u64::try_from(bytes.len()).expect("supplemental TEXT length fits") - 1;
        (start, end)
    });

    let data_start = supplemental_range.map_or(text_end + 1, |(_, end)| end + 1);
    let data_end = data_start + u64::try_from(data.len()).expect("data length fits") - 1;
    let analysis_range = analysis.as_ref().map(|bytes| {
        let start = data_end + 1;
        let end = start + u64::try_from(bytes.len()).expect("analysis length fits") - 1;
        (start, end)
    });

    if let Some((start, end)) = supplemental_range {
        set_pair(&mut primary, "$BEGINSTEXT", &format!("{start:08}"));
        set_pair(&mut primary, "$ENDSTEXT", &format!("{end:08}"));
    }
    set_pair(&mut primary, "$BEGINDATA", &format!("{data_start:08}"));
    set_pair(&mut primary, "$ENDDATA", &format!("{data_end:08}"));
    if let Some((start, end)) = analysis_range {
        set_pair(&mut primary, "$BEGINANALYSIS", &format!("{start:08}"));
        set_pair(&mut primary, "$ENDANALYSIS", &format!("{end:08}"));
    }

    let primary_bytes = encode_text(&primary);
    let header = format_header(
        version,
        text_start,
        text_start + u64::try_from(primary_bytes.len()).expect("text length fits") - 1,
        data_start,
        data_end,
        analysis_range.map_or(0, |(start, _)| start),
        analysis_range.map_or(0, |(_, end)| end),
    );

    let mut output = header.into_bytes();
    output.extend(primary_bytes);
    if let Some(bytes) = supplemental_bytes {
        output.extend(bytes);
    }
    output.extend(data);
    if let Some(bytes) = analysis {
        output.extend(bytes);
    }

    output
}

fn format_header(
    version: &str,
    text_start: u64,
    text_end: u64,
    data_start: u64,
    data_end: u64,
    analysis_start: u64,
    analysis_end: u64,
) -> String {
    format!(
        "{version}    {text_start:>8}{text_end:>8}{data_start:>8}{data_end:>8}{analysis_start:>8}{analysis_end:>8}"
    )
}

fn encode_text(pairs: &[(String, String)]) -> Vec<u8> {
    let mut bytes = vec![DELIMITER];
    for (key, value) in pairs {
        push_escaped(&mut bytes, key);
        bytes.push(DELIMITER);
        push_escaped(&mut bytes, value);
        bytes.push(DELIMITER);
    }
    bytes
}

fn push_escaped(bytes: &mut Vec<u8>, value: &str) {
    for byte in value.as_bytes() {
        bytes.push(*byte);
        if *byte == DELIMITER {
            bytes.push(DELIMITER);
        }
    }
}

fn set_pair(pairs: &mut Vec<(String, String)>, key: &str, value: &str) {
    if let Some((_, existing)) = pairs.iter_mut().find(|(current_key, _)| current_key == key) {
        value.clone_into(existing);
    } else {
        pairs.push((key.to_owned(), value.to_owned()));
    }
}

fn encode_u16_rows(rows: &[[u16; 1]], little_endian: bool) -> Vec<u8> {
    let mut bytes = Vec::new();
    for [value] in rows {
        let raw = if little_endian {
            value.to_le_bytes()
        } else {
            value.to_be_bytes()
        };
        bytes.extend(raw);
    }
    bytes
}

fn overwrite_header_range(
    bytes: &mut [u8],
    start_offset: usize,
    end_offset: usize,
    start: u64,
    end: u64,
) {
    let begin = format!("{start:>8}");
    let finish = format!("{end:>8}");
    bytes[start_offset..start_offset + 8].copy_from_slice(begin.as_bytes());
    bytes[end_offset..end_offset + 8].copy_from_slice(finish.as_bytes());
}
