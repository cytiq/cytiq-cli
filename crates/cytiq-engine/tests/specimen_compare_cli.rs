use cytiq_population::{
    PopulationObservation,
    artifact::{verify_package, write_json, write_package},
    table,
};
use cytiq_provenance::RunRecorder;
use cytiq_qc::CheckStatus;
use cytiq_specimen::SpecimenResult;
use serde_json::{Value, json};
use std::{path::Path, process::Command};

fn package(path: &Path, count: u64) {
    let mut row: PopulationObservation = serde_json::from_value(json!({
        "schema_version":1,"observation_id":"","specimen_id":"S01","panel_id":"P1","acquisition_id":"A01","source_id":"gates",
        "target":{"kind":"population","id":"t_cells","scope":"cells"},"measure":"observed_count","unit":"events","status":"measured",
        "value_u64":count,"value_f64":null,"n_events":u64::MAX,"supporting_count":null,"reason":null,"denominator":null,
        "method":{"name":"gating","version":"1","definition_sha256":"a".repeat(64),"experimental":false,"source_observation_ids":[]},
        "lineage":{"acquisition_sha256":"b".repeat(64),"tree_sha256":"c".repeat(64),"panel_profile_sha256":"d".repeat(64),"projection_sha256":"e".repeat(64),
        "annotations_sha256":null,"producer_provenance_sha256":null,"analysis_mask_sha256":null}
    })).unwrap();
    row.seal().unwrap();
    let result = SpecimenResult {
        schema_version: 1,
        specimen_id: "S01".into(),
        protocol_id: "cli".into(),
        complete: true,
        status: CheckStatus::Pass,
        observations: vec![row.clone()],
        results: vec![row],
        checks: vec![],
    };
    write_package(
        path,
        false,
        RunRecorder::start("cytiq specimen run", "test", vec![]),
        "cytiq specimen run",
        result.outcome_reason(),
        |staged| {
            write_json(&staged.join("result.json"), &result)?;
            table::write(&staged.join("results.parquet"), &result.results)?;
            Ok(vec!["result.json".into(), "results.parquet".into()])
        },
    )
    .unwrap();
}

fn invoke(args: &[&str], code: i32) -> Value {
    let output = Command::new(env!("CARGO_BIN_EXE_cytiq"))
        .args(args)
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(code),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["exit_code"], code);
    value
}

fn text(path: &Path) -> &str {
    path.to_str().unwrap()
}

#[test]
fn comparison_cli_publishes_failed_checks_and_respects_policy_and_atomic_outputs() {
    let tmp = tempfile::tempdir().unwrap();
    let a = tmp.path().join("a");
    let b = tmp.path().join("b");
    let diff = tmp.path().join("diff");
    let policy = tmp.path().join("policy.yaml");
    package(&a, (1 << 53) + 1);
    package(&b, (1 << 53) + 2);
    let args = [
        "specimen",
        "compare",
        "--baseline",
        text(&a),
        "--candidate",
        text(&b),
        "--output",
        text(&diff),
    ];
    assert_eq!(invoke(&args, 3)["completed"], true);
    cytiq_specimen::SpecimenComparison::load(&diff).unwrap();
    let before = std::fs::read(diff.join("comparison.json")).unwrap();
    invoke(&args, 4);
    assert_eq!(before, std::fs::read(diff.join("comparison.json")).unwrap());
    std::fs::write(
        &policy,
        "schema_version: 1\ndefaults: {observed_count: 1}\n",
    )
    .unwrap();
    let mut with_policy = args.to_vec();
    with_policy.extend(["--overwrite", "--policy", text(&policy)]);
    invoke(&with_policy, 0);
    verify_package(&diff).unwrap();
    std::fs::write(&policy, "schema_version: 99").unwrap();
    assert_eq!(invoke(&with_policy, 2)["completed"], false);
    verify_package(&diff).unwrap();
    std::fs::remove_dir_all(&b).unwrap();
    invoke(&args, 4);
    std::fs::write(a.join("result.json"), b"{}").unwrap();
    invoke(
        &[
            "specimen",
            "compare",
            "--baseline",
            text(&a),
            "--candidate",
            text(&a),
            "--output",
            text(&diff),
            "--overwrite",
        ],
        2,
    );
    verify_package(&diff).unwrap();
}
