use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use cytiq_fcs::{FcsReader, FcsWriteParameter};

const PEACOQC_FIXTURE_EVENTS: &str = "sample_events.csv";

#[test]
fn cli_runs_peacoqc_with_debug_artifacts() {
    let directory = create_temp_dir("cytiq-cli-peacoqc");
    let input = directory.join("sample.fcs");
    let output_path = directory.join("sample.peacoqc.fcs");
    let debug_dir = directory.join("debug");
    write_peacoqc_fixture_fcs(&input);

    let output = Command::new(env!("CARGO_BIN_EXE_cytiq"))
        .args([
            "peacoqc",
            "--input",
            utf8(&input),
            "--output",
            utf8(&output_path),
            "--channel",
            "355_378/28_uH-A [CD3 BUV395]",
            "--channel",
            "405_750/40_vB-A [CD4 BV750]",
            "--channel",
            "355_515/20_uF-A [CD8 BUV496]",
            "--debug-dir",
            utf8(&debug_dir),
            "--threads",
            "1",
        ])
        .output()
        .expect("command should run");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(debug_dir.join("summary.json").is_file());
    assert!(debug_dir.join("mask.csv").is_file());
    let report = fs::read_to_string(debug_dir.join("report.html")).expect("report should exist");
    assert!(report.contains("plotly"));

    let dataset = FcsReader::open(&output_path)
        .and_then(FcsReader::read)
        .expect("cleaned FCS should be readable");
    let names = dataset
        .parameters
        .iter()
        .map(|parameter| parameter.name.as_str())
        .collect::<Vec<_>>();
    assert!(names.contains(&"Original_ID"));
}

#[test]
fn cli_fails_when_peacoqc_channel_is_missing() {
    let directory = create_temp_dir("cytiq-cli-peacoqc-missing-channel");
    let input = directory.join("sample.fcs");
    write_peacoqc_fixture_fcs(&input);

    let output = Command::new(env!("CARGO_BIN_EXE_cytiq"))
        .args([
            "peacoqc",
            "--input",
            utf8(&input),
            "--output",
            utf8(&directory.join("sample.peacoqc.fcs")),
            "--channels",
            "CD45",
        ])
        .output()
        .expect("command should run");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("CD45"));
}

#[test]
fn cli_rejects_non_fcs_peacoqc_output() {
    let directory = create_temp_dir("cytiq-cli-peacoqc-output-format");
    let input = directory.join("sample.fcs");
    write_peacoqc_fixture_fcs(&input);

    let output = Command::new(env!("CARGO_BIN_EXE_cytiq"))
        .args([
            "peacoqc",
            "--input",
            utf8(&input),
            "--output",
            utf8(&directory.join("sample.csv")),
            "--channels",
            "355_378/28_uH-A",
        ])
        .output()
        .expect("command should run");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("MVP supports `.fcs`"));
}

fn write_peacoqc_fixture_fcs(path: &Path) {
    let columns = read_peacoqc_fixture_columns();
    let parameters = vec![
        FcsWriteParameter::new("Time"),
        FcsWriteParameter::new("FSC-A"),
        FcsWriteParameter::new("SSC-A"),
        FcsWriteParameter::with_label("355_378/28_uH-A", "CD3 BUV395"),
        FcsWriteParameter::with_label("405_750/40_vB-A", "CD4 BV750"),
        FcsWriteParameter::with_label("355_515/20_uF-A", "CD8 BUV496"),
    ];
    cytiq_fcs::write_fcs3_1_f32(path, &parameters, &columns).expect("fixture FCS should write");
}

fn read_peacoqc_fixture_columns() -> Vec<Vec<f32>> {
    let path = peacoqc_fixture_dir().join(PEACOQC_FIXTURE_EVENTS);
    let mut reader = csv::Reader::from_path(&path).expect("PeacoQC fixture CSV should open");
    let header_len = reader
        .headers()
        .expect("PeacoQC fixture header should parse")
        .len();
    let mut columns = vec![Vec::new(); header_len];
    for record in reader.records() {
        let record = record.expect("PeacoQC fixture row should parse");
        assert_eq!(record.len(), columns.len());
        for (index, column) in columns.iter_mut().enumerate() {
            column.push(
                record[index]
                    .parse::<f32>()
                    .expect("PeacoQC fixture value should be numeric"),
            );
        }
    }
    columns
}

fn peacoqc_fixture_dir() -> PathBuf {
    fixture_dir("peacoqc")
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

fn create_temp_dir(label: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should be after unix epoch")
        .as_nanos();
    let directory = std::env::temp_dir().join(format!("cytiq-{label}-{unique}"));
    fs::create_dir_all(&directory).expect("temporary directory should be created");
    directory
}

fn utf8(path: &Path) -> &str {
    path.to_str().expect("path should be valid utf-8")
}
