use cytiq_population::{
    PopulationObservation,
    artifact::{verify_package, write_package},
    knowledge::KnowledgeArtifact,
    serialization, table,
};
use cytiq_provenance::{RunRecorder, outcome::OutcomeReason};
use serde_json::{Value, json};
use std::path::PathBuf;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../data/fixtures/specimen")
        .join(name)
}

fn count(value: u64) -> PopulationObservation {
    let sha = "a".repeat(64);
    let mut observation: PopulationObservation = serde_json::from_value(json!({
        "schema_version":1,"observation_id":"","specimen_id":"S01","panel_id":"P1","acquisition_id":"A01","source_id":"gates",
        "target":{"kind":"population","id":"t_cells","scope":"cells","state":null},
        "measure":"observed_count","unit":"events","status":"measured","value_u64":value,"value_f64":null,
        "denominator":{"target":null,"scope":"cells","observed_count":value},
        "method":{"name":"gating","version":"1","definition_sha256":sha,"experimental":false,"source_observation_ids":[]},
        "lineage":{"acquisition_sha256":sha,"tree_sha256":sha,"panel_profile_sha256":sha,"projection_sha256":sha,"annotations_sha256":sha,
            "producer_provenance_sha256":sha,"analysis_mask_sha256":sha},"n_events":value,"supporting_count":null,"reason":null
    })).unwrap();
    observation.seal().unwrap();
    observation
}

#[test]
fn observed_counts_above_float_precision_round_trip_exactly() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("counts.parquet");
    let original = vec![count(0), count((1_u64 << 53) + 1), count(u64::MAX)];
    table::write(&path, &original).unwrap();
    assert_eq!(table::read(&path).unwrap(), original);
    let encoded = serde_json::to_vec(&original).unwrap();
    assert_eq!(
        serde_json::from_slice::<Vec<PopulationObservation>>(&encoded).unwrap(),
        original
    );
}

#[test]
fn unavailable_states_cannot_carry_a_usable_numeric_value() {
    use cytiq_population::ObservationStatus as S;
    for status in [
        S::NotMeasured,
        S::QcRejected,
        S::BelowDetection,
        S::NotApplicable,
    ] {
        let mut observation = count(0);
        observation.status = status;
        assert!(observation.seal().is_err());
        observation.value_u64 = None;
        observation.reason = Some("explicit_reason".into());
        observation.seal().unwrap();
    }
    let mut observation = count(1);
    observation.value_f64 = Some(f64::NAN);
    assert!(observation.seal().is_err());
    observation = count(1);
    observation.value_u64 = Some(2);
    assert!(observation.seal().is_err());
}

#[test]
fn external_knowledge_is_validated_and_digest_pinned() {
    for name in [
        "P1.projection.json",
        "P2.projection.json",
        "facscanto2-p1.projection.json",
        "flowcyt-bm.projection.json",
    ] {
        KnowledgeArtifact::load(&fixture(name)).unwrap_or_else(|e| panic!("{name}: {e}"));
    }
    let mut value = serialization::read(&fixture("P1.projection.json")).unwrap();
    value["tree"]["cells"][1]["parent"] = json!("missing");
    assert!(KnowledgeArtifact::from_value(value).is_err());
    let mut value = serialization::read(&fixture("P1.projection.json")).unwrap();
    value["panel"]["cofactor"] = json!(0);
    assert!(KnowledgeArtifact::from_value(value).is_err());
    let mut value = serialization::read(&fixture("P1.projection.json")).unwrap();
    value["tree"]["unknown"] = json!(true);
    assert!(KnowledgeArtifact::from_value(value).is_err());
    let mut value = serialization::read(&fixture("P1.projection.json")).unwrap();
    value["populations"][0]["status"] = json!("ambiguous");
    assert!(KnowledgeArtifact::from_value(value).is_err());
    let mut value = serialization::read(&fixture("tree.json")).unwrap();
    let evidence = value["cells"][0]["evidence"].clone();
    value["cells"][0]["marker_signature"] = json!([
        {"marker":"CD3","state":"positive","inherited":true,"evidence":evidence}
    ]);
    value["cells"][1]["marker_signature"] = json!([
        {"marker":"CD3","state":"negative","evidence":evidence}
    ]);
    assert!(KnowledgeArtifact::from_value(value).is_err());
    let mut value = serialization::read(&fixture("tree.json")).unwrap();
    value["ontology_edges"] =
        json!([{"child":"CL:A","parent":"CL:B"},{"child":"CL:B","parent":"CL:A"}]);
    assert!(KnowledgeArtifact::from_value(value).is_err());
}

#[test]
fn omitted_factory_defaults_have_the_same_upstream_artifact_identity() {
    let complete = serialization::read(&fixture("tree.json")).unwrap();
    let expected = KnowledgeArtifact::from_value(complete.clone()).unwrap();
    let mut compact = complete;
    for key in [
        "phenotypes",
        "ontology_edges",
        "tasks",
        "mappings",
        "decisions",
        "snapshots",
    ] {
        compact.as_object_mut().unwrap().remove(key);
    }
    for cell in compact["cells"].as_array_mut().unwrap() {
        for key in ["aliases", "ontology_ids", "marker_signature"] {
            cell.as_object_mut().unwrap().remove(key);
        }
    }
    assert_eq!(
        KnowledgeArtifact::from_value(compact).unwrap().sha256(),
        expected.sha256()
    );
}

#[test]
fn duplicate_json_keys_and_excessive_nesting_are_rejected() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("bad.json");
    for text in [
        "{\"a\":1,\"a\":2}".into(),
        format!("{{\"a\":{}0{}}}", "[".repeat(66), "]".repeat(66)),
    ] {
        std::fs::write(&path, text).unwrap();
        assert!(serialization::read(&path).is_err());
    }
}

#[test]
fn failed_replacement_preserves_the_entire_previous_package() {
    let directory = tempfile::tempdir().unwrap();
    let output = directory.path().join("result");
    let record = || RunRecorder::start("test", "1", vec![]);
    write_package(
        &output,
        false,
        record(),
        "test",
        OutcomeReason::Success,
        |staged| {
            std::fs::write(staged.join("data.json"), b"old")?;
            Ok(vec!["data.json".into()])
        },
    )
    .unwrap();
    let original = std::fs::read(output.join("outcome.json")).unwrap();
    let error = write_package(
        &output,
        true,
        record(),
        "test",
        OutcomeReason::Success,
        |staged| {
            std::fs::write(staged.join("data.json"), b"new")?;
            Err(cytiq_population::PopulationError::Invalid(
                "injected writer failure".into(),
            ))
        },
    );
    assert!(error.is_err());
    assert_eq!(std::fs::read(output.join("data.json")).unwrap(), b"old");
    assert_eq!(
        std::fs::read(output.join("outcome.json")).unwrap(),
        original
    );
    verify_package(&output).unwrap();
    std::fs::write(output.join("data.json"), b"corrupt").unwrap();
    assert!(verify_package(&output).is_err());
    let _value: Value = serde_json::from_slice(&original).unwrap();
}
