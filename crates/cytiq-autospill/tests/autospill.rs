#[path = "support.rs"]
mod support;

use std::fs;

use cytiq_autospill::{
    AutospillConfig, AutospillError, AutospillRunner, MatrixKind, export_matrix,
    read_controls_manifest,
};

#[test]
fn runner_matches_synthetic_spillover_fixture() {
    let directory = support::create_temp_dir("autospill-runner");
    let manifest_path = support::write_two_control_fixture(&directory, [[1.0, 0.2], [0.1, 1.0]]);
    let manifest = read_controls_manifest(&manifest_path).expect("manifest should load");

    let result = AutospillRunner::run(&manifest, &directory, &AutospillConfig::default())
        .expect("autospill should succeed");

    assert_eq!(result.channels, vec!["FL1-A", "FL2-A"]);
    assert_close(result.spillover_matrix[0][1], 0.2, 1e-6);
    assert_close(result.spillover_matrix[1][0], 0.1, 1e-6);
    assert_close(
        result.compensation_matrix[0][0],
        1.020_408_163_265_306,
        1e-9,
    );
    assert_close(
        result.compensation_matrix[0][1],
        -0.204_081_632_653_061_2,
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
    let directory = support::create_temp_dir("autospill-parquet");
    let manifest_path = support::write_two_control_fixture(&directory, [[1.0, 0.2], [0.1, 1.0]]);
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
