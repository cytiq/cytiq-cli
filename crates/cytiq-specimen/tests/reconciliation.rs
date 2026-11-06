use cytiq_population::{
    ArtifactRef, EventAnnotations, Measure, ObservationStatus, PopulationObservation,
    annotations::KnowledgeRef, knowledge::KnowledgeArtifact,
};
use cytiq_qc::CheckStatus;
use cytiq_specimen::{
    SpecimenLayout, SpecimenPlan, SpecimenResult,
    layout::{AnchorLimits, ReconciliationRule, SourceRef, Strategy},
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

fn limits(
    min_events: u64,
    max_scale: Option<f64>,
    max_frequency_ratio: Option<f64>,
) -> AnchorLimits {
    AnchorLimits {
        min_events,
        max_scale,
        max_frequency_ratio,
    }
}

/// P2's T cells scaled by the `cells` anchor of P1 over that of P2.
fn ratio_rule(plan: &SpecimenPlan, anchor_limits: AnchorLimits) -> ReconciliationRule {
    rule(
        plan,
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
            anchor_limits,
        },
    )
}

/// Reconcile P1 (`p1` T cells of `p1_anchor`) and P2 under one ratio rule.
fn ratio(
    anchor_limits: AnchorLimits,
    (p1, p1_anchor): (u64, u64),
    (p2, p2_anchor): (u64, u64),
) -> SpecimenResult {
    let mut plan = plan();
    plan.layout.reconciliation = vec![ratio_rule(&plan, anchor_limits)];
    let sources = [
        source(&plan, "P1", p1, p1_anchor),
        source(&plan, "P2", p2, p2_anchor),
    ];
    reconcile(&plan, &sources).unwrap()
}

fn estimate(result: &SpecimenResult) -> &PopulationObservation {
    result
        .results
        .iter()
        .find(|o| o.measure == Measure::EstimatedCount)
        .unwrap()
}

fn check<'a>(
    result: &'a SpecimenResult,
    name: &str,
) -> Option<&'a cytiq_specimen::reconcile::SpecimenCheck> {
    result.checks.iter().find(|c| c.check == name)
}

fn assert_measured_counts_unchanged(result: &SpecimenResult, p1: u64, p2: u64) {
    let measured = result
        .observations
        .iter()
        .filter(|o| o.target.id == "t_cells" && o.measure == Measure::ObservedCount)
        .map(|o| (o.status, o.value_u64))
        .collect::<Vec<_>>();
    assert_eq!(
        measured,
        [
            (ObservationStatus::Measured, Some(p1)),
            (ObservationStatus::Measured, Some(p2))
        ]
    );
}

#[test]
fn reference_ratio_keeps_both_measured_counts_and_emits_a_distinct_estimate() {
    let mut plan = plan();
    plan.layout.reconciliation = vec![ratio_rule(&plan, limits(1, None, None))];
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
    assert_eq!(estimate.method.version, "2");
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
    plan.layout.reconciliation = vec![ratio_rule(&plan, limits(1, None, None))];
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

#[test]
fn anchor_below_min_events_rejects_only_the_derived_estimate() {
    // P2's anchor holds 49 events; the rule demands 50.
    let result = ratio(limits(50, None, None), (30, 100), (20, 49));
    let estimate = estimate(&result);
    assert_eq!(estimate.status, ObservationStatus::QcRejected);
    assert_eq!(estimate.reason.as_deref(), Some("anchor_below_min_events"));
    assert_eq!(estimate.value_f64, None);
    assert_eq!(estimate.supporting_count, Some(20));
    let events = check(&result, "reference_ratio_anchor_events").unwrap();
    assert_eq!(events.status, CheckStatus::Fail);
    assert!(
        events
            .message
            .contains("P2/gates anchor `cells`: 49 event(s) (minimum 50")
    );
    assert_eq!(
        check(&result, "reference_ratio_anchor").unwrap().status,
        CheckStatus::Pass
    );
    assert_eq!(result.status, CheckStatus::Fail);
    assert_measured_counts_unchanged(&result, 30, 20);
}

#[test]
fn anchor_count_equal_to_min_events_is_accepted() {
    let result = ratio(limits(50, None, None), (30, 100), (20, 50));
    let estimate = estimate(&result);
    assert_eq!(estimate.status, ObservationStatus::Estimated);
    assert_eq!(estimate.value_f64, Some(40.0));
    assert_eq!(
        check(&result, "reference_ratio_anchor_events")
            .unwrap()
            .status,
        CheckStatus::Pass
    );
    let reference_low = ratio(limits(101, None, None), (30, 100), (20, 200));
    assert_eq!(
        estimate_status(&reference_low),
        ObservationStatus::QcRejected,
        "the reference anchor is checked too"
    );
}

fn estimate_status(result: &SpecimenResult) -> ObservationStatus {
    estimate(result).status
}

#[test]
fn scale_beyond_bound_rejects_estimate_in_both_directions() {
    // Reference (P1) over source (P2) anchor: scale 0.5, then 2.
    for (p1_anchor, p2_anchor) in [(5000, 10000), (10000, 5000)] {
        let result = ratio(
            limits(1, Some(1.5), None),
            (3000, p1_anchor),
            (2000, p2_anchor),
        );
        let estimate = estimate(&result);
        assert_eq!(estimate.status, ObservationStatus::QcRejected);
        assert_eq!(
            estimate.reason.as_deref(),
            Some("anchor_scale_out_of_bounds")
        );
        assert_eq!(
            check(&result, "reference_ratio_scale").unwrap().status,
            CheckStatus::Fail
        );
        assert_measured_counts_unchanged(&result, 3000, 2000);
    }
}

#[test]
fn scale_at_bound_is_accepted() {
    for (p1_anchor, p2_anchor) in [(5000, 10000), (10000, 5000)] {
        let result = ratio(
            limits(1, Some(2.0), None),
            (3000, p1_anchor),
            (2000, p2_anchor),
        );
        assert_eq!(estimate_status(&result), ObservationStatus::Estimated);
        assert_eq!(
            check(&result, "reference_ratio_scale").unwrap().status,
            CheckStatus::Pass
        );
    }
}

#[test]
fn events_are_reported_before_scale_when_both_fail() {
    let result = ratio(limits(50, Some(1.5), None), (30, 100), (5, 10));
    assert_eq!(
        estimate(&result).reason.as_deref(),
        Some("anchor_below_min_events")
    );
    assert_eq!(
        check(&result, "reference_ratio_scale").unwrap().status,
        CheckStatus::Fail
    );
}

#[test]
fn divergent_anchor_frequencies_warn_without_rejecting() {
    // Anchor frequencies 4000/10000 = 0.4 in P1 and 16000/20000 = 0.8 in P2.
    let result = ratio(limits(1, None, Some(1.5)), (1000, 4000), (2000, 16000));
    let estimate = estimate(&result);
    assert_eq!(estimate.status, ObservationStatus::Estimated);
    assert_eq!(estimate.value_f64, Some(500.0));
    let frequency = check(&result, "reference_ratio_anchor_frequency").unwrap();
    assert_eq!(frequency.status, CheckStatus::Warn);
    assert!(
        frequency.message.contains("ratio 2"),
        "{}",
        frequency.message
    );
    assert_eq!(result.status, CheckStatus::Warn);

    let at_limit = ratio(limits(1, None, Some(2.0)), (1000, 4000), (2000, 16000));
    assert_eq!(
        check(&at_limit, "reference_ratio_anchor_frequency")
            .unwrap()
            .status,
        CheckStatus::Pass
    );
}

#[test]
fn optional_checks_are_absent_unless_configured_or_anchors_are_unavailable() {
    let result = ratio(limits(1, None, None), (3000, 10000), (6400, 20000));
    assert!(check(&result, "reference_ratio_scale").is_none());
    assert!(check(&result, "reference_ratio_anchor_frequency").is_none());
    let unavailable = ratio(limits(1, Some(2.0), Some(2.0)), (3000, 10000), (0, 0));
    assert_eq!(
        estimate(&unavailable).reason.as_deref(),
        Some("invalid_or_unavailable_anchor")
    );
    assert!(check(&unavailable, "reference_ratio_scale").is_none());
    assert!(check(&unavailable, "reference_ratio_anchor_frequency").is_none());
    let events = check(&unavailable, "reference_ratio_anchor_events").unwrap();
    assert_eq!(events.status, CheckStatus::Fail);
    assert!(events.message.contains("unavailable"), "{}", events.message);
}

#[test]
fn anchor_check_messages_report_poisson_cv() {
    let result = ratio(limits(1, None, None), (100, 400), (100, 100));
    let events = check(&result, "reference_ratio_anchor_events").unwrap();
    assert!(
        events.message.contains("Poisson CV 0.100") && events.message.contains("Poisson CV 0.050"),
        "{}",
        events.message
    );
    // √(1/100 + 1/100 + 1/400) = 0.15
    let anchor = check(&result, "reference_ratio_anchor").unwrap();
    assert!(
        anchor.message.ends_with("estimate Poisson CV 0.150"),
        "{}",
        anchor.message
    );
    let empty = ratio(limits(1, None, None), (100, 400), (0, 100));
    assert!(
        check(&empty, "reference_ratio_anchor")
            .unwrap()
            .message
            .ends_with("estimate Poisson CV n/a")
    );
}

#[test]
fn reference_ratio_without_anchor_limits_is_rejected() {
    let plan = plan();
    let mut rule = serde_json::to_value(ratio_rule(&plan, limits(1, None, None))).unwrap();
    rule["strategy"]
        .as_object_mut()
        .unwrap()
        .remove("anchor_limits");
    assert!(serde_json::from_value::<ReconciliationRule>(rule).is_err());
}

#[test]
fn invalid_anchor_limits_are_rejected() {
    for invalid in [
        limits(0, None, None),
        limits(1, Some(f64::NAN), None),
        limits(1, Some(0.5), None),
        limits(1, None, Some(f64::INFINITY)),
        limits(1, None, Some(0.99)),
    ] {
        let mut plan = plan();
        plan.layout.reconciliation = vec![ratio_rule(&plan, invalid.clone())];
        assert!(plan.layout.validate().is_err(), "{invalid:?}");
    }
    let mut plan = plan();
    plan.layout.reconciliation = vec![ratio_rule(&plan, limits(1, Some(1.0), Some(1.0)))];
    assert!(plan.layout.validate().is_ok());
}

#[test]
fn optional_anchor_limits_are_omitted_from_the_layout_digest() {
    let plan = plan();
    let rule = serde_json::to_value(ratio_rule(&plan, limits(5, None, None))).unwrap();
    assert_eq!(rule["strategy"]["anchor_limits"], json!({"min_events": 5}));
}
