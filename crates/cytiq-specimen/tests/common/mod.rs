#![allow(dead_code)] // Shared by comparison and report integration suites.
use cytiq_population::{
    PopulationObservation,
    artifact::{write_json, write_package},
    table,
};
use cytiq_provenance::RunRecorder;
use cytiq_qc::CheckStatus;
use cytiq_specimen::SpecimenResult;
use serde_json::json;
use std::path::Path;

pub fn result(count: u64) -> SpecimenResult {
    let mut row: PopulationObservation = serde_json::from_value(json!({
        "schema_version":1,"observation_id":"","specimen_id":"S01","panel_id":"P1","acquisition_id":"A01","source_id":"gates",
        "target":{"kind":"population","id":"t_cells","scope":"cells"},"measure":"observed_count","unit":"events","status":"measured",
        "value_u64":count,"value_f64":null,"n_events":u64::MAX,"supporting_count":null,"reason":null,
        "denominator":{"target":null,"scope":"cells","observed_count":u64::MAX},
        "method":{"name":"gating","version":"1","definition_sha256":"a".repeat(64),"experimental":false,"source_observation_ids":[]},
        "lineage":{"acquisition_sha256":"b".repeat(64),"tree_sha256":"c".repeat(64),"panel_profile_sha256":"d".repeat(64),"projection_sha256":"e".repeat(64),
        "annotations_sha256":"f".repeat(64),"producer_provenance_sha256":"1".repeat(64),"analysis_mask_sha256":"2".repeat(64)}
    })).unwrap();
    row.seal().unwrap();
    SpecimenResult {
        schema_version: 1,
        specimen_id: "S01".into(),
        protocol_id: "test".into(),
        complete: true,
        status: CheckStatus::Pass,
        observations: vec![row.clone()],
        results: vec![row],
        checks: vec![],
    }
}

pub fn recorder(command: &str) -> RunRecorder {
    RunRecorder::start(command, "test", vec![])
}

pub fn package(directory: &Path, result: &SpecimenResult) {
    write_package(
        directory,
        false,
        recorder("cytiq specimen run"),
        "cytiq specimen run",
        result.outcome_reason(),
        |staged| {
            write_json(&staged.join("result.json"), result)?;
            table::write(&staged.join("results.parquet"), &result.results)?;
            Ok(vec!["result.json".into(), "results.parquet".into()])
        },
    )
    .unwrap();
}

pub fn reseal(result: &mut SpecimenResult) {
    for row in result.observations.iter_mut().chain(&mut result.results) {
        row.seal().unwrap();
    }
}
