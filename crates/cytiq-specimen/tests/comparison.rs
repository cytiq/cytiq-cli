mod common;
use common::{package, recorder, reseal, result};
use cytiq_population::{Measure, ObservationStatus, artifact::verify_package};
use cytiq_provenance::outcome::OutcomeReason;
use cytiq_qc::CheckStatus;
use cytiq_specimen::{
    ComparisonPolicy, SpecimenComparison, compare,
    compare::{Delta, Direction, RowVerdict, Tolerance, ToleranceOverride},
    reconcile::SpecimenCheck,
    runner::{CompareConfig, write_comparison},
};

#[test]
fn a_failed_check_alone_fails_an_otherwise_equal_comparison() {
    let mut a = result(3);
    a.checks.push(SpecimenCheck {
        category: "technical".into(),
        scope: "P1".into(),
        check: "existing_failure".into(),
        status: CheckStatus::Fail,
        message: "already failed".into(),
    });
    a.status = CheckStatus::Fail;
    let mut b = a.clone();
    b.checks.push(SpecimenCheck {
        category: "technical".into(),
        scope: "P1".into(),
        check: "acquisition_qc".into(),
        status: CheckStatus::Fail,
        message: "failed".into(),
    });
    b.status = CheckStatus::Fail;
    let diff = compare(&a, &b, &ComparisonPolicy::default()).unwrap();
    assert_eq!(diff.summary.failed_rows, 0);
    assert!(diff.summary.failed_checks > 0);
    assert_eq!(diff.outcome_reason(), OutcomeReason::ChecksFailed);
}

#[test]
fn counts_above_float_precision_use_exact_deltas_and_inclusive_limits() {
    let a = result((1 << 53) + 1);
    let b = result((1 << 53) + 2);
    let mut policy = ComparisonPolicy::default();
    let diff = compare(&a, &b, &policy).unwrap();
    assert_eq!(diff.outcome_reason(), OutcomeReason::ChecksFailed);
    assert_eq!(
        diff.rows[0].delta,
        Some(Delta::Count {
            absolute: 1,
            direction: Direction::Increased
        })
    );
    policy.defaults.observed_count = 1;
    assert_eq!(
        compare(&a, &b, &policy).unwrap().outcome_reason(),
        OutcomeReason::Success
    );
    assert_eq!(
        compare(&result(u64::MAX), &result(0), &policy)
            .unwrap()
            .rows[0]
            .delta,
        Some(Delta::Count {
            absolute: u64::MAX,
            direction: Direction::Decreased
        })
    );
}

#[test]
fn permutation_and_context_changes_do_not_break_semantic_pairing() {
    let mut a = result(3);
    let mut extra = a.observations[0].clone();
    extra.target.id = "other".into();
    extra.seal().unwrap();
    a.observations.push(extra.clone());
    a.results.push(extra);
    let mut b = a.clone();
    b.observations.reverse();
    b.results.reverse();
    for row in b.observations.iter_mut().chain(&mut b.results) {
        row.acquisition_id = Some("retake".into());
        row.method.definition_sha256 = "3".repeat(64);
        row.lineage.analysis_mask_sha256 = Some("4".repeat(64));
    }
    reseal(&mut b);
    let diff = compare(&a, &b, &ComparisonPolicy::default()).unwrap();
    assert_eq!(diff.summary.numeric_pairs, 4);
    assert_eq!(diff.outcome_reason(), OutcomeReason::Success);
    assert!(
        diff.context
            .iter()
            .any(|c| c.field == "lineage.analysis_mask_sha256")
    );
    assert_ne!(diff.rows[0].baseline_id, diff.rows[0].candidate_id);
    diff.validate().unwrap();
}

#[test]
fn additions_removals_and_ambiguous_rows_are_explicit() {
    let a = result(0);
    let mut b = a.clone();
    b.results[0].target.id = "other".into();
    reseal(&mut b);
    let diff = compare(&a, &b, &ComparisonPolicy::default()).unwrap();
    assert!(diff.rows.iter().any(|r| r.verdict == RowVerdict::Added));
    assert!(diff.rows.iter().any(|r| r.verdict == RowVerdict::Removed));
    b.results.push(b.results[0].clone());
    assert!(compare(&a, &b, &ComparisonPolicy::default()).is_err());
    b = a.clone();
    b.specimen_id = "different".into();
    assert!(compare(&a, &b, &ComparisonPolicy::default()).is_err());
}

#[test]
fn incompatible_trees_denominators_and_statuses_have_no_numeric_delta() {
    let a = result(0);
    for field in ["tree", "denominator", "status"] {
        let mut b = a.clone();
        for row in b.observations.iter_mut().chain(&mut b.results) {
            match field {
                "tree" => row.lineage.tree_sha256 = "7".repeat(64),
                "denominator" => row.denominator.as_mut().unwrap().scope = "other".into(),
                _ => {
                    row.status = ObservationStatus::NotMeasured;
                    row.value_u64 = None;
                    row.reason = Some("missing".into());
                }
            }
        }
        reseal(&mut b);
        let diff = compare(&a, &b, &ComparisonPolicy::default()).unwrap();
        assert_eq!(diff.outcome_reason(), OutcomeReason::ChecksFailed);
        assert!(diff.rows.iter().all(|r| r.delta.is_none()));
        let expected = if field == "status" {
            RowVerdict::StatusChanged
        } else {
            RowVerdict::Incompatible
        };
        assert!(diff.rows.iter().all(|r| r.verdict == expected));
    }
    let mut absent = a;
    for row in absent.observations.iter_mut().chain(&mut absent.results) {
        row.status = ObservationStatus::QcRejected;
        row.value_u64 = None;
        row.supporting_count = Some(0);
        row.reason = Some("qc_failed".into());
    }
    reseal(&mut absent);
    let diff = compare(&absent, &absent, &ComparisonPolicy::default()).unwrap();
    assert_eq!(diff.outcome_reason(), OutcomeReason::Incomplete);
    assert!(
        diff.rows
            .iter()
            .all(|r| r.verdict == RowVerdict::Unavailable)
    );
}

#[test]
fn float_limits_and_exact_key_overrides_are_validated() {
    let mut a = result(2);
    for row in a.observations.iter_mut().chain(&mut a.results) {
        row.measure = Measure::Fraction;
        row.unit = "fraction".into();
        row.value_u64 = None;
        row.value_f64 = Some(0.25);
    }
    reseal(&mut a);
    let mut b = a.clone();
    for row in b.observations.iter_mut().chain(&mut b.results) {
        row.value_f64 = Some(0.5);
    }
    reseal(&mut b);
    let mut policy = ComparisonPolicy::default();
    policy.defaults.fraction = 0.25;
    assert_eq!(
        compare(&a, &b, &policy).unwrap().outcome_reason(),
        OutcomeReason::Success
    );
    let key = compare(&a, &b, &policy).unwrap().rows[0].key.clone();
    policy.overrides.push(ToleranceOverride {
        key,
        limit: Tolerance::Float(0.0),
    });
    assert_eq!(compare(&a, &b, &policy).unwrap().summary.failed_rows, 1);
    policy.overrides.push(policy.overrides[0].clone());
    assert!(compare(&a, &b, &policy).is_err());
    policy.overrides.pop();
    policy.overrides[0].key.source_id = "unknown".into();
    assert!(compare(&a, &b, &policy).is_err());
    policy.overrides.clear();
    policy.defaults.fraction = f64::NAN;
    assert!(compare(&a, &b, &policy).is_err());
    policy.defaults.fraction = -1.0;
    assert!(compare(&a, &b, &policy).is_err());
}

#[test]
fn qc_multisets_and_completeness_are_gates_but_messages_are_descriptive() {
    let mut a = result(1);
    a.checks.push(SpecimenCheck {
        category: "technical".into(),
        scope: "P1".into(),
        check: "qc".into(),
        status: CheckStatus::Pass,
        message: "before".into(),
    });
    let mut b = a.clone();
    b.checks[0].message = "after".into();
    assert_eq!(
        compare(&a, &b, &ComparisonPolicy::default())
            .unwrap()
            .outcome_reason(),
        OutcomeReason::Success
    );
    b.checks.push(b.checks[0].clone());
    assert_eq!(
        compare(&a, &b, &ComparisonPolicy::default())
            .unwrap()
            .summary
            .failed_checks,
        1
    );
    b = a.clone();
    b.complete = false;
    assert!(
        compare(&a, &b, &ComparisonPolicy::default())
            .unwrap()
            .summary
            .specimen_changed
    );
    b = a.clone();
    b.checks[0].status = CheckStatus::Fail;
    b.status = CheckStatus::Fail;
    assert_eq!(
        compare(&a, &b, &ComparisonPolicy::default())
            .unwrap()
            .outcome_reason(),
        OutcomeReason::ChecksFailed
    );
}

#[test]
fn comparison_packages_are_standalone_recomputed_and_atomic() {
    let tmp = tempfile::tempdir().unwrap();
    let a = tmp.path().join("baseline");
    let b = tmp.path().join("candidate");
    let output = tmp.path().join("comparison");
    package(&a, &result(1));
    package(&b, &result(2));
    let mut config = CompareConfig {
        baseline: a.clone(),
        candidate: b.clone(),
        output: output.clone(),
        policy: None,
        overwrite: false,
    };
    let outcome = write_comparison(&config, recorder("cytiq specimen compare")).unwrap();
    assert_eq!(outcome.exit_code, 3);
    let diff = SpecimenComparison::load(&output).unwrap();
    assert!(diff.baseline_package.is_some());
    let before = std::fs::read(output.join("comparison.json")).unwrap();
    assert!(write_comparison(&config, recorder("compare")).is_err());
    assert_eq!(
        std::fs::read(output.join("comparison.json")).unwrap(),
        before
    );
    config.output = a.clone();
    config.overwrite = true;
    assert!(write_comparison(&config, recorder("compare")).is_err());
    cytiq_specimen::SpecimenResult::load(&a).unwrap();
    std::fs::remove_dir_all(a).unwrap();
    std::fs::remove_dir_all(b).unwrap();
    SpecimenComparison::load(&output).unwrap();
    let mut corrupt = diff;
    corrupt.rows[0].delta = None;
    assert!(corrupt.validate().is_err());
    std::fs::write(output.join("comparison.json"), b"{}").unwrap();
    assert!(verify_package(&output).is_err());
}
