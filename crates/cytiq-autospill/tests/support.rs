#![allow(
    dead_code,
    clippy::cast_lossless,
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::format_push_string,
    clippy::missing_panics_doc,
    clippy::must_use_candidate,
    clippy::needless_range_loop
)]

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const DELIMITER: u8 = b'|';

const DEFAULT_DYES: [&str; 2] = ["FITC", "PE"];
const DEFAULT_PRIMARY_CHANNELS: [&str; 2] = ["FL1-A", "FL2-A"];
const DEFAULT_DETECTORS: [&str; 2] = ["FL1-A", "FL2-A"];

const RECTANGULAR_DYES: [&str; 2] = ["FITC", "PE"];
const RECTANGULAR_PRIMARY_CHANNELS: [&str; 2] = ["FL1-A", "FL2-A"];
const RECTANGULAR_DETECTORS: [&str; 3] = ["FL1-A", "FL2-A", "FL3-A"];
const AUTOFLUORESCENCE_DETECTORS: [&str; 3] = ["FL1-A", "FL2-A", "AF-A"];
const AUTOFLUORESCENCE_REAL_DETECTORS: [&str; 2] = ["FL1-A", "FL2-A"];

pub fn autospill_fixture_dir() -> PathBuf {
    fixture_dir("autospill")
}

pub fn autospread_fixture_dir() -> PathBuf {
    fixture_dir("autospread")
}

pub fn create_temp_dir(label: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should be after unix epoch")
        .as_nanos();
    let directory = std::env::temp_dir().join(format!("cytiq-{label}-{unique}"));
    fs::create_dir_all(&directory).expect("temporary directory should be created");
    directory
}

pub fn copy_autospill_fixture_set(label: &str) -> PathBuf {
    let directory = create_temp_dir(label);
    let _ = write_two_control_fixture(&directory, [[1.0, 0.2], [0.1, 1.0]]);
    directory
}

pub fn copy_autospread_fixture_set(label: &str) -> PathBuf {
    let directory = create_temp_dir(label);
    let _ = write_autospread_fixture(&directory);
    directory
}

pub fn write_two_control_fixture(directory: &Path, spillover: [[f64; 2]; 2]) -> PathBuf {
    let detector_labels =
        detector_labels(&DEFAULT_DYES, &DEFAULT_PRIMARY_CHANNELS, &DEFAULT_DETECTORS);
    fs::write(
        directory.join("control_a.fcs"),
        build_control_bytes(0, &spillover[0], None, &DEFAULT_DETECTORS, &detector_labels),
    )
    .expect("first control should be written");
    fs::write(
        directory.join("control_b.fcs"),
        build_control_bytes(1, &spillover[1], None, &DEFAULT_DETECTORS, &detector_labels),
    )
    .expect("second control should be written");

    write_manifest_rows(
        directory,
        &[("control_a.fcs", "FITC"), ("control_b.fcs", "PE")],
    )
}

pub fn write_rectangular_autospill_fixture(directory: &Path) -> (PathBuf, PathBuf) {
    let manifest = write_control_fixture_set(
        directory,
        &RECTANGULAR_DYES,
        &RECTANGULAR_PRIMARY_CHANNELS,
        &RECTANGULAR_DETECTORS,
        &[vec![1.0, 0.2, 0.05], vec![0.1, 1.0, 0.03]],
        None,
    );
    let detectors = write_detector_manifest(directory, &RECTANGULAR_DETECTORS);
    (manifest, detectors)
}

pub fn write_autospread_fixture(directory: &Path) -> (PathBuf, PathBuf) {
    let manifest = write_control_fixture_set(
        directory,
        &RECTANGULAR_DYES,
        &RECTANGULAR_PRIMARY_CHANNELS,
        &RECTANGULAR_DETECTORS,
        &[vec![1.0, 0.2, 0.05], vec![0.1, 1.0, 0.03]],
        Some(&[vec![0.0, 0.08, 0.05], vec![0.06, 0.0, 0.04]]),
    );
    let detectors = write_detector_manifest(directory, &RECTANGULAR_DETECTORS);
    (manifest, detectors)
}

pub fn write_autofluorescence_fixture(directory: &Path) -> (PathBuf, PathBuf) {
    let detector_labels = detector_labels(
        &DEFAULT_DYES,
        &DEFAULT_PRIMARY_CHANNELS,
        &AUTOFLUORESCENCE_REAL_DETECTORS,
    );
    fs::write(
        directory.join("control_fitc.fcs"),
        build_control_bytes(
            0,
            &[1.0, 0.2],
            None,
            &AUTOFLUORESCENCE_REAL_DETECTORS,
            &detector_labels,
        ),
    )
    .expect("fitc control should be written");
    fs::write(
        directory.join("control_pe.fcs"),
        build_control_bytes(
            1,
            &[0.1, 1.0],
            None,
            &AUTOFLUORESCENCE_REAL_DETECTORS,
            &detector_labels,
        ),
    )
    .expect("pe control should be written");
    fs::write(
        directory.join("unstained.fcs"),
        build_autofluorescence_control_bytes(
            &AUTOFLUORESCENCE_REAL_DETECTORS,
            &[0.6, 0.4],
            &detector_labels,
        ),
    )
    .expect("autofluorescence control should be written");

    let manifest_path = write_manifest_rows(
        directory,
        &[
            ("control_fitc.fcs", "FITC"),
            ("control_pe.fcs", "PE"),
            ("unstained.fcs", "Autofluorescence"),
        ],
    );
    let detectors = write_detector_manifest(directory, &AUTOFLUORESCENCE_DETECTORS);
    (manifest_path, detectors)
}

pub fn write_transform_manifest(directory: &Path, channels: &[&str]) -> PathBuf {
    let path = directory.join("transforms.csv");
    let mut text = String::from("channel,length,max_range,pos,neg,width\n");
    for channel in channels {
        text.push_str(&format!("{channel},256,262144,4.418539922,0,-100\n"));
    }
    write_text(&path, &text);
    path
}

pub fn write_detector_manifest(directory: &Path, detectors: &[&str]) -> PathBuf {
    let path = directory.join("detectors.csv");
    let mut text = String::from("channel,label,wavelength\n");
    for (index, detector) in detectors.iter().enumerate() {
        text.push_str(&format!(
            "{detector},{detector},{}\n",
            500 + (index as u32) * 10
        ));
    }
    write_text(&path, &text);
    path
}

pub fn write_control_fixture_set(
    directory: &Path,
    dyes: &[&str],
    primary_channels: &[&str],
    detectors: &[&str],
    spillover_rows: &[Vec<f64>],
    spread_rows: Option<&[Vec<f64>]>,
) -> PathBuf {
    assert_eq!(dyes.len(), primary_channels.len());
    assert_eq!(dyes.len(), spillover_rows.len());
    if let Some(spread_rows) = spread_rows {
        assert_eq!(spread_rows.len(), dyes.len());
    }
    let detector_labels = detector_labels(dyes, primary_channels, detectors);

    for (control_index, dye) in dyes.iter().enumerate() {
        let primary_detector_index = detectors
            .iter()
            .position(|detector| detector == &primary_channels[control_index])
            .expect("primary channel should exist in detectors");
        let file_name = format!("control_{}.fcs", slug(dye));
        let bytes = build_control_bytes(
            primary_detector_index,
            &spillover_rows[control_index],
            spread_rows.map(|rows| rows[control_index].as_slice()),
            detectors,
            &detector_labels,
        );
        fs::write(directory.join(file_name), bytes).expect("control should be written");
    }

    let rows = dyes
        .iter()
        .map(|dye| (format!("control_{}.fcs", slug(dye)), (*dye).to_owned()))
        .collect::<Vec<_>>();
    let borrowed_rows = rows
        .iter()
        .map(|(filename, dye)| (filename.as_str(), dye.as_str()))
        .collect::<Vec<_>>();
    write_manifest_rows(directory, &borrowed_rows)
}

pub fn write_manifest_rows(directory: &Path, rows: &[(&str, &str)]) -> PathBuf {
    let manifest_path = directory.join("controls.csv");
    let mut manifest = String::from("filename,dye\n");
    for (filename, dye) in rows {
        manifest.push_str(&format!("{filename},{dye}\n"));
    }
    write_text(&manifest_path, &manifest);
    manifest_path
}

pub fn write_control_file(
    directory: &Path,
    file_name: &str,
    primary_detector_index: usize,
    spillover_row: &[f64],
    detectors: &[&str],
    detector_labels: &[Option<String>],
) {
    fs::write(
        directory.join(file_name),
        build_control_bytes(
            primary_detector_index,
            spillover_row,
            None,
            detectors,
            detector_labels,
        ),
    )
    .expect("control should be written");
}

/// Edits a control written by these builders in place. Parameter `number`
/// (1-based) gets `$PnR = range`; with `cap_data` it keeps its `$PnR` and its
/// values above `range` become `range` instead. `range` has five digits like
/// the builders' `65535`, so every offset stays valid.
pub fn limit_parameter(path: &Path, number: usize, range: u16, cap_data: bool) {
    let mut bytes = fs::read(path).expect("control should be readable");
    let offset = |start: usize| -> usize {
        std::str::from_utf8(&bytes[start..start + 8])
            .expect("header offsets are ASCII")
            .trim()
            .parse()
            .expect("header offsets are numbers")
    };
    let (data_start, data_end) = (offset(26), offset(34));
    if cap_data {
        let text = String::from_utf8_lossy(&bytes[58..data_start]).into_owned();
        let fields = text.split(char::from(DELIMITER)).collect::<Vec<_>>();
        let parameters = fields
            .iter()
            .position(|field| *field == "$PAR")
            .map(|index| {
                fields[index + 1]
                    .parse::<usize>()
                    .expect("$PAR is a number")
            })
            .expect("controls carry $PAR");
        let first = data_start + 2 * (number - 1);
        for at in (first..data_end).step_by(2 * parameters) {
            if u16::from_le_bytes([bytes[at], bytes[at + 1]]) > range {
                bytes[at..at + 2].copy_from_slice(&range.to_le_bytes());
            }
        }
    } else {
        let delimiter = char::from(DELIMITER);
        let from = format!("{delimiter}$P{number}R{delimiter}65535{delimiter}");
        let to = format!("{delimiter}$P{number}R{delimiter}{range:05}{delimiter}");
        let at = bytes
            .windows(from.len())
            .position(|window| window == from.as_bytes())
            .expect("controls carry $PnR = 65535");
        bytes[at..at + to.len()].copy_from_slice(to.as_bytes());
    }
    fs::write(path, bytes).expect("control should be written");
}

pub fn write_text(path: &Path, contents: &str) {
    fs::write(path, contents).expect("text fixture should be written");
}

pub fn read_fixture_text(name: &str) -> String {
    fs::read_to_string(autospill_fixture_dir().join(name)).expect("fixture text should exist")
}

pub fn read_autospread_fixture_text(name: &str) -> String {
    fs::read_to_string(autospread_fixture_dir().join(name)).expect("fixture text should exist")
}

fn fixture_dir(name: &str) -> PathBuf {
    for start in fixture_search_roots() {
        for candidate in start
            .ancestors()
            .map(|ancestor| ancestor.join("data").join("fixtures").join(name))
        {
            if candidate.is_dir() {
                return candidate;
            }
        }
    }

    panic!("fixture directory should exist: data/fixtures/{name}");
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

fn build_control_bytes(
    primary_detector_index: usize,
    spillover_row: &[f64],
    spread_row: Option<&[f64]>,
    detectors: &[&str],
    detector_labels: &[Option<String>],
) -> Vec<u8> {
    assert_eq!(spillover_row.len(), detectors.len());
    assert_eq!(detector_labels.len(), detectors.len());
    if let Some(spread_row) = spread_row {
        assert_eq!(spread_row.len(), detectors.len());
    }

    let mut rows = Vec::<Vec<u16>>::new();
    if let Some(spread_row) = spread_row {
        let noise_pattern = [-3.0, -2.0, -1.0, 0.0, 0.0, 1.0, 2.0, 3.0];
        for level in 0..48 {
            let primary = 1_200.0 + (level as f64) * 900.0;
            for (replicate, noise) in noise_pattern.iter().enumerate() {
                let fsc = 900.0 + ((level % 12) as f64) * 15.0 + replicate as f64;
                let ssc = 980.0 + ((level % 10) as f64) * 11.0 + (replicate as f64) * 0.5;
                let mut row = vec![fsc.round() as u16, ssc.round() as u16];
                for detector_index in 0..detectors.len() {
                    let base = if detector_index == primary_detector_index {
                        primary
                    } else {
                        spillover_row[detector_index] * primary
                    };
                    let spread = spread_row[detector_index] * primary.sqrt() * noise;
                    row.push((base + spread).max(0.0).round() as u16);
                }
                rows.push(row);
            }
        }
    } else {
        for index in 0..120 {
            let fsc = 900.0 + ((index % 12) as f64) * 15.0 + ((index / 12) as f64) * 2.0;
            let ssc = 980.0 + ((index % 10) as f64) * 11.0 + ((index / 10) as f64) * 3.0;
            let primary = 1_000.0 + (index as f64) * 230.0;
            let mut row = vec![fsc.round() as u16, ssc.round() as u16];
            for detector_index in 0..detectors.len() {
                let value = if detector_index == primary_detector_index {
                    primary
                } else {
                    spillover_row[detector_index] * primary
                };
                row.push(value.round() as u16);
            }
            rows.push(row);
        }
    }

    for index in 0..30 {
        let mut row = vec![20 + (index as u16), 30 + ((index % 7) as u16)];
        for detector_index in 0..detectors.len() {
            row.push(10 + (((index + detector_index) % 9) as u16));
        }
        rows.push(row);
    }

    build_fcs(
        "FCS3.1",
        build_parameter_pairs(detectors, detector_labels, rows.len()),
        encode_u16_rows(&rows),
    )
}

fn build_autofluorescence_control_bytes(
    detectors: &[&str],
    coefficients: &[f64],
    detector_labels: &[Option<String>],
) -> Vec<u8> {
    assert_eq!(detectors.len(), coefficients.len());
    assert_eq!(detector_labels.len(), detectors.len());

    let mut rows = Vec::<Vec<u16>>::new();
    for index in 0..120 {
        let fsc = 880.0 + ((index % 12) as f64) * 14.0 + ((index / 12) as f64) * 2.0;
        let ssc = 970.0 + ((index % 10) as f64) * 12.0 + ((index / 10) as f64) * 2.0;
        let latent = 500.0 + (index as f64) * 80.0;
        let mut row = vec![fsc.round() as u16, ssc.round() as u16];
        for coefficient in coefficients {
            row.push((latent * coefficient).round() as u16);
        }
        rows.push(row);
    }

    for index in 0..30 {
        let mut row = vec![18 + (index as u16), 28 + ((index % 7) as u16)];
        for detector_index in 0..detectors.len() {
            row.push(8 + (((index + detector_index) % 7) as u16));
        }
        rows.push(row);
    }

    build_fcs(
        "FCS3.1",
        build_parameter_pairs(detectors, detector_labels, rows.len()),
        encode_u16_rows(&rows),
    )
}

fn build_parameter_pairs(
    detectors: &[&str],
    detector_labels: &[Option<String>],
    row_count: usize,
) -> Vec<(String, String)> {
    let total_parameters = detectors.len() + 2;
    let mut pairs = vec![
        ("$BYTEORD".to_owned(), "1,2,3,4".to_owned()),
        ("$DATATYPE".to_owned(), "I".to_owned()),
        ("$MODE".to_owned(), "L".to_owned()),
        ("$NEXTDATA".to_owned(), "00000000".to_owned()),
        ("$PAR".to_owned(), total_parameters.to_string()),
        ("$TOT".to_owned(), row_count.to_string()),
    ];

    let names = std::iter::once("FSC-A")
        .chain(std::iter::once("SSC-A"))
        .chain(detectors.iter().copied())
        .collect::<Vec<_>>();
    for (index, name) in names.iter().enumerate() {
        let parameter_index = index + 1;
        pairs.push((format!("$P{parameter_index}B"), "16".to_owned()));
        pairs.push((format!("$P{parameter_index}E"), "0,0".to_owned()));
        pairs.push((format!("$P{parameter_index}N"), (*name).to_owned()));
        if let Some(label) = detector_labels
            .get(index.saturating_sub(2))
            .and_then(|label| label.as_deref())
            .filter(|_| index >= 2)
        {
            pairs.push((format!("$P{parameter_index}S"), label.to_owned()));
        }
        pairs.push((format!("$P{parameter_index}R"), "65535".to_owned()));
    }

    pairs
}

fn detector_labels(
    dyes: &[&str],
    primary_channels: &[&str],
    detectors: &[&str],
) -> Vec<Option<String>> {
    detectors
        .iter()
        .map(|detector| {
            primary_channels
                .iter()
                .position(|channel| channel == detector)
                .map(|index| dyes[index].to_owned())
        })
        .collect()
}

fn build_fcs(version: &str, primary_pairs: Vec<(String, String)>, data: Vec<u8>) -> Vec<u8> {
    let mut primary = primary_pairs;
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

fn encode_u16_rows(rows: &[Vec<u16>]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for row in rows {
        for value in row {
            bytes.extend(value.to_le_bytes());
        }
    }
    bytes
}

fn slug(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect()
}
