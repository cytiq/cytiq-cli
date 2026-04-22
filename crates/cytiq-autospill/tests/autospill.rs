#[path = "support.rs"]
mod support;

use std::fs;

use cytiq_autospill::{
    AutospillConfig, AutospillError, AutospillRunner, AutospreadConfig, AutospreadRunner,
    MatrixKind, export_matrix, read_controls_manifest, read_detectors_manifest,
    read_transform_manifest,
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
    ) || matches!(
        error,
        AutospillError::MissingDetector { ref channel } if channel == "FL9-A"
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

#[test]
fn runner_supports_rectangular_detector_sets() {
    let directory = support::create_temp_dir("autospill-rectangular");
    let (manifest_path, detectors_path) = support::write_rectangular_autospill_fixture(&directory);
    let manifest = read_controls_manifest(&manifest_path).expect("manifest should load");
    let detectors = read_detectors_manifest(&detectors_path).expect("detectors should load");
    let config = AutospillConfig {
        detectors,
        ..AutospillConfig::default()
    };

    let result = AutospillRunner::run(&manifest, &directory, &config)
        .expect("rectangular autospill should succeed");

    assert_eq!(result.dyes, vec!["FITC", "PE"]);
    assert_eq!(result.detectors, vec!["FL1-A", "FL2-A", "FL3-A"]);
    assert_eq!(result.spillover_matrix.len(), 2);
    assert_eq!(result.spillover_matrix[0].len(), 3);
    assert_eq!(result.compensation_matrix.len(), 3);
    assert_eq!(result.compensation_matrix[0].len(), 2);
    assert_close(result.spillover_matrix[0][1], 0.2, 1e-6);
    assert_close(result.spillover_matrix[0][2], 0.05, 1e-5);
    assert_close(result.spillover_matrix[1][0], 0.1, 1e-6);
    assert_close(result.spillover_matrix[1][2], 0.03, 1e-5);
}

#[test]
fn runner_supports_autofluorescence_controls() {
    let directory = support::create_temp_dir("autospill-autofluorescence");
    let (manifest_path, detectors_path) = support::write_autofluorescence_fixture(&directory);
    let manifest = read_controls_manifest(&manifest_path).expect("manifest should load");
    let detectors = read_detectors_manifest(&detectors_path).expect("detectors should load");
    let config = AutospillConfig {
        detectors,
        ..AutospillConfig::default()
    };

    let result = AutospillRunner::run(&manifest, &directory, &config)
        .expect("autospill with autofluorescence should succeed");

    assert_eq!(result.dyes, vec!["FITC", "PE", "Autofluorescence"]);
    assert_eq!(result.detectors, vec!["FL1-A", "FL2-A", "AF-A"]);
    assert_eq!(result.spillover_matrix.len(), 3);
    assert_eq!(result.spillover_matrix[2].len(), 3);
    assert_eq!(result.spillover_matrix[2][2], 1.0);
    assert!(result.spillover_matrix[2][0] > 0.0);
    assert!(result.spillover_matrix[2][1] > 0.0);
}

#[test]
fn runner_writes_debug_artifacts() {
    let directory = support::create_temp_dir("autospill-debug");
    let debug_dir = directory.join("debug");
    let (manifest_path, detectors_path) = support::write_rectangular_autospill_fixture(&directory);
    let transform_path = support::write_transform_manifest(&directory, &["FITC", "PE"]);
    let manifest = read_controls_manifest(&manifest_path).expect("manifest should load");
    let detectors = read_detectors_manifest(&detectors_path).expect("detectors should load");
    let transforms = read_transform_manifest(&transform_path).expect("transforms should load");
    let config = AutospillConfig {
        detectors,
        transforms,
        debug_dir: Some(debug_dir.clone()),
        ..AutospillConfig::default()
    };

    let _ = AutospillRunner::run(&manifest, &directory, &config)
        .expect("autospill with debug output should succeed");

    for name in [
        "summary.json",
        "convergence.html",
        "spillover_matrix.html",
        "compensation_matrix.html",
        "gate_fitc.html",
        "gate_pe.html",
        "scatter_fitc_00.html",
        "scatter_pe_00.html",
    ] {
        assert!(debug_dir.join(name).is_file(), "missing debug artifact {name}");
    }
}

#[test]
fn autospread_estimates_positive_spread_coefficients() {
    let directory = support::create_temp_dir("autospread");
    let (manifest_path, detectors_path) = support::write_autospread_fixture(&directory);
    let manifest = read_controls_manifest(&manifest_path).expect("manifest should load");
    let detectors = read_detectors_manifest(&detectors_path).expect("detectors should load");
    let config = AutospreadConfig {
        detectors,
        ..AutospreadConfig::default()
    };

    let result = AutospreadRunner::run(&manifest, &directory, &config)
        .expect("autospread should succeed");

    assert_eq!(result.spread_matrix.len(), 2);
    assert_eq!(result.spread_matrix[0].len(), 3);
    assert_eq!(result.spread_matrix[0][0], 0.0);
    assert_eq!(result.spread_matrix[1][1], 0.0);
    assert!(result.spread_matrix[0][1] > 0.0);
    assert!(result.spread_matrix[0][2] > 0.0);
    assert!(result.spread_matrix[1][0] > 0.0);
    assert!(result.spread_matrix[1][2] > 0.0);
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
