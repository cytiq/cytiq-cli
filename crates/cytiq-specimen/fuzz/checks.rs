//! What the fuzz target checks. `tests/fuzz_regressions.rs` includes this file
//! too, so `cargo test` replays every saved input against the same checks.

use cytiq_population::serialization::{self, DocumentFormat};
use cytiq_specimen::SpecimenResult;
use cytiq_specimen::compare::{ComparisonPolicy, compare};
use serde_json::Value;

/// The specimen documents in one input: `{"baseline", "candidate", "policy"}`,
/// with the default (exact) policy when `policy` is absent.
pub struct Documents {
    pub baseline: SpecimenResult,
    pub candidate: SpecimenResult,
    pub policy: ComparisonPolicy,
}

/// `None` unless every document parses and validates.
#[must_use]
pub fn documents(bytes: &[u8]) -> Option<Documents> {
    let document = serialization::parse(bytes, DocumentFormat::Json).ok()?;
    let result = |key: &str| {
        let result: SpecimenResult = serde_json::from_value(document.get(key)?.clone()).ok()?;
        result.validate().ok().map(|()| result)
    };
    let policy = match document.get("policy") {
        Some(policy) => ComparisonPolicy::from_value(policy.clone()).ok()?,
        None => ComparisonPolicy::default(),
    };
    Some(Documents {
        baseline: result("baseline")?,
        candidate: result("candidate")?,
        policy,
    })
}

/// Any bytes: parsing and comparison never panic. A comparison of valid
/// results revalidates from its own snapshots, and a result compared with
/// itself never fails a check or changes the specimen.
pub fn specimen_documents(bytes: &[u8]) {
    let Some(Documents {
        baseline,
        candidate,
        policy,
    }) = documents(bytes)
    else {
        return;
    };
    if let Ok(comparison) = compare(&baseline, &candidate, &policy) {
        comparison
            .validate()
            .expect("a comparison revalidates from its snapshots");
        let encoded: Value = serde_json::to_value(&comparison).expect("serializable comparison");
        assert!(encoded.is_object());
    }
    for result in [&baseline, &candidate] {
        let same = compare(result, result, &policy).expect("a result compares with itself");
        assert_eq!(same.summary.failed_rows, 0);
        assert_eq!(same.summary.failed_checks, 0);
        assert!(!same.summary.specimen_changed);
    }
}
