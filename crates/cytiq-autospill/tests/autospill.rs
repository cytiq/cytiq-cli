#[path = "support.rs"]
mod support;

use std::fs;

use cytiq_autospill::{
    AutospillConfig, AutospillError, AutospillRunner, MatrixKind, export_matrix,
    read_controls_manifest,
};

#[test]
fn runner_matches_synthetic_spillover_fixture() {
    let directory = support::autospill_fixture_dir();
    let manifest_path = directory.join("controls.csv");
    let manifest = read_controls_manifest(&manifest_path).expect("manifest should load");

    let result = AutospillRunner::run(&manifest, &directory, &AutospillConfig::default())
        .expect("autospill should succeed");

    assert_eq!(result.channels, vec!["FL1-A", "FL2-A"]);
    assert_close(result.spillover_matrix[0][1], 0.2, 1e-6);
    assert_close(result.spillover_matrix[1][0], 0.1, 1e-6);
    assert_matrix_csv(
        &result.compensation_matrix,
        &support::read_fixture_text("expected_compensation.csv"),
        1e-9,
    );
    assert_matrix_csv(
        &result.spillover_matrix,
        &support::read_fixture_text("expected_spillover.csv"),
        1e-9,
    );
    assert!(
        result
            .gate_stats
            .iter()
            .all(|stats| stats.gated_events >= 60)
    );
    assert!(
        result
            .gate_stats
            .iter()
            .all(|stats| stats.bound_maxima >= 1)
    );
    assert!(
        result
            .gate_stats
            .iter()
            .all(|stats| stats.region_maxima >= 1)
    );
    assert!(result.convergence.converged);
}

#[test]
fn export_matrix_writes_parquet_magic_bytes() {
    let directory = support::copy_autospill_fixture_set("autospill-parquet");
    let manifest_path = directory.join("controls.csv");
    let manifest = read_controls_manifest(&manifest_path).expect("manifest should load");
    let result = AutospillRunner::run(&manifest, &directory, &AutospillConfig::default())
        .expect("autospill should succeed");

    let output_path = directory.join("matrix.parquet");
    export_matrix(&result, MatrixKind::Compensation, &output_path)
        .expect("parquet export should succeed");

    let bytes = fs::read(output_path).expect("parquet output should exist");
    assert!(bytes.len() > 8);
    assert_eq!(&bytes[..4], b"PAR1");
    assert_eq!(&bytes[bytes.len() - 4..], b"PAR1");
}

#[test]
fn runner_reports_missing_primary_channel() {
    let directory = support::create_temp_dir("autospill-missing-channel");
    let manifest_path = support::write_two_control_fixture(&directory, [[1.0, 0.2], [0.1, 1.0]]);
    support::write_text(
        &manifest_path,
        "filename,dye,primary_channel\ncontrol_a.fcs,FITC,FL1-A\ncontrol_b.fcs,PE,FL9-A\n",
    );
    let manifest = read_controls_manifest(&manifest_path).expect("manifest should load");

    let error = AutospillRunner::run(&manifest, &directory, &AutospillConfig::default())
        .expect_err("missing channel should fail");
    assert!(matches!(
        error,
        AutospillError::MissingChannel { ref channel, .. } if channel == "FL9-A"
    ));
}

#[test]
fn runner_reports_singular_matrix() {
    let directory = support::create_temp_dir("autospill-singular");
    let manifest_path = support::write_two_control_fixture(&directory, [[1.0, 1.0], [1.0, 1.0]]);
    let manifest = read_controls_manifest(&manifest_path).expect("manifest should load");

    let error = AutospillRunner::run(&manifest, &directory, &AutospillConfig::default())
        .expect_err("singular spillover matrix should fail");
    assert!(matches!(error, AutospillError::SingularMatrix));
}

fn assert_close(actual: f64, expected: f64, tolerance: f64) {
    let delta = (actual - expected).abs();
    assert!(
        delta <= tolerance,
        "expected {expected} +/- {tolerance}, got {actual} (|delta|={delta})"
    );
}

fn assert_matrix_csv(actual: &[Vec<f64>], expected_csv: &str, tolerance: f64) {
    let rows = expected_csv
        .lines()
        .skip(1)
        .map(|line| {
            line.split(',')
                .skip(1)
                .map(|value| {
                    value
                        .parse::<f64>()
                        .expect("fixture matrix value should parse")
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    assert_eq!(actual.len(), rows.len());
    for (actual_row, expected_row) in actual.iter().zip(rows) {
        assert_eq!(actual_row.len(), expected_row.len());
        for (actual_value, expected_value) in actual_row.iter().zip(expected_row) {
            assert_close(*actual_value, expected_value, tolerance);
        }
    }
}
