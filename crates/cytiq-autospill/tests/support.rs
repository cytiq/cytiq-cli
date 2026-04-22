#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::missing_panics_doc,
    clippy::must_use_candidate
)]

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const DELIMITER: u8 = b'|';

pub fn create_temp_dir(label: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should be after unix epoch")
        .as_nanos();
    let directory = std::env::temp_dir().join(format!("cytiq-{label}-{unique}"));
    fs::create_dir_all(&directory).expect("temporary directory should be created");
    directory
}

pub fn write_two_control_fixture(directory: &Path, spillover: [[f64; 2]; 2]) -> PathBuf {
    fs::write(
        directory.join("control_a.fcs"),
        build_control_bytes(0, spillover[0]),
    )
    .expect("first control should be written");
    fs::write(
        directory.join("control_b.fcs"),
        build_control_bytes(1, spillover[1]),
    )
    .expect("second control should be written");

    let manifest_path = directory.join("controls.csv");
    fs::write(
        &manifest_path,
        "filename,dye,primary_channel,antigen\ncontrol_a.fcs,FITC,FL1-A,CD3\ncontrol_b.fcs,PE,FL2-A,CD19\n",
    )
    .expect("manifest should be written");
    manifest_path
}

pub fn write_text(path: &Path, contents: &str) {
    fs::write(path, contents).expect("text fixture should be written");
}

fn build_control_bytes(primary_index: usize, spillover_row: [f64; 2]) -> Vec<u8> {
    let mut rows = Vec::<[u16; 4]>::new();
    for index in 0..120 {
        let fsc = 900 + ((index % 12) as u16) * 15 + ((index / 12) as u16) * 2;
        let ssc = 980 + ((index % 10) as u16) * 11 + ((index / 10) as u16) * 3;
        let primary = 1_000 + (index as u16) * 230;
        let fl1 = if primary_index == 0 {
            primary
        } else {
            (spillover_row[0] * f64::from(primary)).round() as u16
        };
        let fl2 = if primary_index == 1 {
            primary
        } else {
            (spillover_row[1] * f64::from(primary)).round() as u16
        };
        rows.push([fsc, ssc, fl1, fl2]);
    }

    for index in 0..30 {
        rows.push([
            20 + (index as u16),
            30 + ((index % 7) as u16),
            10 + ((index % 5) as u16),
            15 + ((index % 9) as u16),
        ]);
    }

    build_fcs(
        "FCS3.1",
        vec![
            ("$BYTEORD", "1,2,3,4"),
            ("$DATATYPE", "I"),
            ("$MODE", "L"),
            ("$NEXTDATA", "00000000"),
            ("$PAR", "4"),
            ("$TOT", &rows.len().to_string()),
            ("$P1B", "16"),
            ("$P1E", "0,0"),
            ("$P1N", "FSC-A"),
            ("$P1R", "65535"),
            ("$P2B", "16"),
            ("$P2E", "0,0"),
            ("$P2N", "SSC-A"),
            ("$P2R", "65535"),
            ("$P3B", "16"),
            ("$P3E", "0,0"),
            ("$P3N", "FL1-A"),
            ("$P3R", "65535"),
            ("$P4B", "16"),
            ("$P4E", "0,0"),
            ("$P4N", "FL2-A"),
            ("$P4R", "65535"),
        ],
        encode_u16_rows(&rows),
    )
}

fn build_fcs(version: &str, primary_pairs: Vec<(&str, &str)>, data: Vec<u8>) -> Vec<u8> {
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
    let data_start = text_end + 1;
    let data_end = data_start + u64::try_from(data.len()).expect("data length fits") - 1;

    set_pair(&mut primary, "$BEGINDATA", &format!("{data_start:08}"));
    set_pair(&mut primary, "$ENDDATA", &format!("{data_end:08}"));

    let primary_bytes = encode_text(&primary);
    let header = format_header(
        version,
        text_start,
        text_start + u64::try_from(primary_bytes.len()).expect("text length fits") - 1,
        data_start,
        data_end,
        0,
        0,
    );

    let mut output = header.into_bytes();
    output.extend(primary_bytes);
    output.extend(data);
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

fn encode_u16_rows(rows: &[[u16; 4]]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for row in rows {
        for value in row {
            bytes.extend(value.to_le_bytes());
        }
    }
    bytes
}
