use cytiq_population::{
    ArtifactRef, EventAnnotations, Measure, ObservationStatus, PopulationObservation,
    annotations::KnowledgeRef, knowledge::KnowledgeArtifact,
};
use cytiq_specimen::{
    SpecimenLayout, SpecimenPlan,
    layout::{ReconciliationRule, SourceRef, Strategy},
    plan::{Acquisition, PanelSelection},
    reconcile::{SourceObservations, reconcile},
};
use serde_json::json;
use std::path::{Path, PathBuf};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../data/fixtures/specimen")
        .join(name)
}

fn plan() -> SpecimenPlan {
    let definition = ArtifactRef::of(&fixture("gates.xml")).unwrap();
    let mut panels = serde_json::Map::new();
    let mut selections = Vec::new();
    for (name, events) in [("P1", 10000_u64), ("P2", 20000_u64)] {
        let projection =
            KnowledgeArtifact::load(&fixture(&format!("{name}.projection.json"))).unwrap();
        panels.insert(name.into(),json!({"projection":KnowledgeRef {path:fixture(&format!("{name}.projection.json")),sha256:projection.sha256().into()},"sources":{"gates":{
            "format":"gate_membership","definition":definition,"bindings":[
                {"label":"All","target":{"kind":"population","id":"cells","scope":"cells"}},
                {"label":"T","target":{"kind":"population","id":"t_cells","scope":"cells"},"denominator":"All"}
            ]}}}));
        let id = format!("{name}-A");
        selections.push(PanelSelection {
            panel_id: name.into(),
            tree_sha256: projection.value()["tree_sha256"].as_str().unwrap().into(),
            panel_profile_sha256: projection.value()["panel_sha256"].as_str().unwrap().into(),
            projection_sha256: projection.sha256().into(),
            acquisitions: vec![Acquisition {
                id: id.clone(),
                file: ArtifactRef {
                    path: PathBuf::from("unused.fcs"),
                    sha256: "a".repeat(64),
                },
                raw_events: events,
            }],
            selected: Some(id),
            excluded: std::collections::BTreeMap::default(),
        });
    }
    let mut layout: SpecimenLayout =
        serde_json::from_value(json!({"schema_version":1,"protocol_id":"test","panels":panels}))
            .unwrap();
    layout.validate().unwrap();
    SpecimenPlan {
        schema_version: 1,
        specimen_id: "S01".into(),
        manifest: definition.clone(),
        layout_file: definition,
        cytometry: cytiq_manifest::Cytometry::Conventional,
        layout,
        selections,
    }
}

fn source(plan: &SpecimenPlan, panel: &str, population: u64, anchor: u64) -> SourceObservations {
    let context = plan.context(&format!("{panel}-A"), "gates").unwrap();
    let sha = "b".repeat(64);
    let reference = ArtifactRef {
        path: Path::new("unused.parquet").into(),
        sha256: sha.clone(),
    };
    let descriptor = EventAnnotations {
        schema_version: 1,
        context: context.clone(),
        data: reference.clone(),
        producer_provenance: reference.clone(),
        analysis_mask: reference.clone(),
        technical_qc: reference.clone(),
        technical_qc_provenance: reference,
        technical_status: "pass".into(),
        assessed_events: context.raw_events,
        unknown_events: 0,
    };
    let mut observations = Vec::new();
    let selection = plan
        .selections
        .iter()
        .find(|s| s.panel_id == panel)
        .unwrap();
    for (index, binding) in context.source.bindings.iter().enumerate() {
        let count = if index == 0 { anchor } else { population };
        let denominator = if index == 0 {
            context.raw_events
        } else {
            anchor
        };
        for measure in ["observed_count", "fraction"] {
            let fraction = measure == "fraction";
            #[allow(clippy::cast_precision_loss)]
            let value =
                (fraction && denominator > 0).then_some(count as f64 / denominator.max(1) as f64);
            let mut row: PopulationObservation = serde_json::from_value(json!({
                "schema_version":1,"observation_id":"","specimen_id":"S01","panel_id":panel,"acquisition_id":context.acquisition_id,"source_id":"gates","target":binding.target,
                "measure":measure,"unit":if fraction {"fraction"} else {"events"},"status":if fraction && denominator == 0 {"not_applicable"} else {"measured"},
                "value_u64":if fraction {None} else {Some(count)},"value_f64":value,
                "denominator":{"target":if index == 0 {None} else {Some(&context.source.bindings[0].target)},"scope":"cells","observed_count":denominator},
                "method":{"name":"gating","version":"1","definition_sha256":context.source.definition.sha256,"experimental":false,"source_observation_ids":[]},
                "lineage":{"acquisition_sha256":context.acquisition_sha256,"tree_sha256":selection.tree_sha256,"panel_profile_sha256":selection.panel_profile_sha256,"projection_sha256":selection.projection_sha256,
                    "annotations_sha256":sha,"producer_provenance_sha256":sha,"analysis_mask_sha256":sha},"n_events":context.raw_events,"supporting_count":null,
                "reason":if fraction && denominator == 0 {Some("zero_denominator")} else {None}
            })).unwrap();
            row.seal().unwrap();
            observations.push(row);
        }
    }
    SourceObservations {
        annotations: descriptor,
        observations,
    }
}

fn rule(plan: &SpecimenPlan, strategy: Strategy) -> ReconciliationRule {
    ReconciliationRule {
        target: plan.layout.panels["P1"].sources["gates"].bindings[1]
            .target
            .clone(),
        strategy,
    }
}

#[test]
fn reference_ratio_keeps_both_measured_counts_and_emits_a_distinct_estimate() {
    let mut plan = plan();
    plan.layout.reconciliation = vec![rule(
        &plan,
        Strategy::ReferenceRatio {
            source: SourceRef {
                panel: "P2".into(),
                source: "gates".into(),
            },
            reference: SourceRef {
                panel: "P1".into(),
                source: "gates".into(),
            },
            anchor: plan.layout.panels["P1"].sources["gates"].bindings[0]
                .target
                .clone(),
        },
    )];
    let sources = vec![
        source(&plan, "P1", 3000, 10000),
        source(&plan, "P2", 6400, 20000),
    ];
    let result = reconcile(&plan, &sources).unwrap();
    assert!(result.complete);
    let measured = result
        .observations
        .iter()
        .filter(|o| o.target.id == "t_cells" && o.measure == Measure::ObservedCount)
        .map(|o| o.value_u64.unwrap())
        .collect::<Vec<_>>();
    assert_eq!(measured, vec![3000, 6400]);
    let estimate = result
        .results
        .iter()
        .find(|o| o.measure == Measure::EstimatedCount)
        .unwrap();
    assert_eq!(estimate.value_f64, Some(3200.0));
    assert_eq!(estimate.status, ObservationStatus::Estimated);
    assert!(estimate.method.experimental);
    assert_eq!(estimate.method.source_observation_ids.len(), 3);
}

#[test]
fn missing_required_panel_is_incomplete_while_optional_panel_remains_explicitly_missing() {
    let mut plan = plan();
    let sources = vec![source(&plan, "P1", 3000, 10000)];
    let result = reconcile(&plan, &sources).unwrap();
    assert!(!result.complete);
    assert!(
        result
            .observations
            .iter()
            .filter(|o| o.panel_id == "P2")
            .all(|o| o.status == ObservationStatus::NotMeasured && o.value_u64.is_none())
    );
    plan.layout.panels.get_mut("P2").unwrap().required = false;
    plan.selections[1].selected = None;
    assert!(reconcile(&plan, &sources).unwrap().complete);
}

#[test]
fn prefer_reference_has_no_fallback_and_never_adds_children() {
    let mut plan = plan();
    plan.layout.reconciliation = vec![rule(
        &plan,
        Strategy::PreferReference {
            reference: SourceRef {
                panel: "P2".into(),
                source: "gates".into(),
            },
        },
    )];
    let result = reconcile(
        &plan,
        &[
            source(&plan, "P1", 3000, 10000),
            source(&plan, "P2", 6400, 20000),
        ],
    )
    .unwrap();
    assert_eq!(
        result
            .results
            .iter()
            .find(|o| o.target.id == "t_cells" && o.measure == Measure::ObservedCount)
            .unwrap()
            .value_u64,
        Some(6400)
    );
    let missing = reconcile(&plan, &[source(&plan, "P1", 3000, 10000)]).unwrap();
    assert!(
        missing
            .results
            .iter()
            .filter(|o| o.target.id == "t_cells")
            .all(|o| o.status == ObservationStatus::NotMeasured)
    );
}

#[test]
fn zero_anchors_reject_only_the_derived_estimate() {
    let mut plan = plan();
    plan.layout.reconciliation = vec![rule(
        &plan,
        Strategy::ReferenceRatio {
            source: SourceRef {
                panel: "P2".into(),
                source: "gates".into(),
            },
            reference: SourceRef {
                panel: "P1".into(),
                source: "gates".into(),
            },
            anchor: plan.layout.panels["P1"].sources["gates"].bindings[0]
                .target
                .clone(),
        },
    )];
    let result = reconcile(
        &plan,
        &[source(&plan, "P1", 3000, 10000), source(&plan, "P2", 0, 0)],
    )
    .unwrap();
    let estimate = result
        .results
        .iter()
        .find(|o| o.measure == Measure::EstimatedCount)
        .unwrap();
    assert_eq!(estimate.status, ObservationStatus::QcRejected);
    assert_eq!(estimate.value_f64, None);
    assert!(result.observations.iter().any(|o| o.panel_id == "P2"
        && o.value_u64 == Some(0)
        && o.status == ObservationStatus::Measured));
}

#[test]
fn biological_disagreement_does_not_delete_or_reject_measurements() {
    let mut plan = plan();
    plan.layout
        .consistency
        .push(cytiq_specimen::layout::ConsistencyPolicy {
            target: plan.layout.panels["P1"].sources["gates"].bindings[1]
                .target
                .clone(),
            max_abs_fraction_difference: 0.01,
            severity: cytiq_qc::CheckStatus::Fail,
        });
    let result = reconcile(
        &plan,
        &[
            source(&plan, "P1", 100, 10000),
            source(&plan, "P2", 19900, 20000),
        ],
    )
    .unwrap();
    assert_eq!(result.status, cytiq_qc::CheckStatus::Fail);
    assert!(result.complete);
    assert!(
        result
            .observations
            .iter()
            .all(|o| o.status == ObservationStatus::Measured)
    );
    let duplicate = vec![
        source(&plan, "P1", 100, 10000),
        source(&plan, "P1", 100, 10000),
    ];
    assert!(reconcile(&plan, &duplicate).is_err());
}
