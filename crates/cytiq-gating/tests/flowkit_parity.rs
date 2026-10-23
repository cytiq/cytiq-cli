use std::fs;
use std::path::PathBuf;

use cytiq_fcs::FcsReader;
use cytiq_gating::parse_gatingml;
use serde_json::Value;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../data/fixtures/gating")
        .join(name)
}

fn decode_membership(hex: &str, event_count: usize) -> Vec<bool> {
    let bytes = (0..hex.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&hex[index..index + 2], 16).expect("valid hex"))
        .collect::<Vec<_>>();
    (0..event_count)
        .map(|event| bytes[event / 8] & (0x80 >> (event % 8)) != 0)
        .collect()
}

#[test]
fn matches_flowkit_membership_for_every_gate() {
    let xml = fs::read_to_string(fixture("gates.xml")).expect("gates.xml should exist");
    let reference: Value = serde_json::from_str(
        &fs::read_to_string(fixture("flowkit_reference.json")).expect("reference should exist"),
    )
    .expect("reference should be JSON");
    let dataset = FcsReader::open(fixture("sample.fcs"))
        .and_then(FcsReader::read)
        .expect("sample.fcs should parse");

    let strategy = parse_gatingml(&xml).expect("gates.xml should parse");
    let results = strategy
        .gate_sample(&dataset)
        .expect("gating should succeed");

    let event_count = usize::try_from(reference["event_count"].as_u64().unwrap()).unwrap();
    assert_eq!(results.event_count(), event_count);
    let expected_gates = reference["gates"].as_object().unwrap();
    assert_eq!(results.gate_ids().count(), expected_gates.len());

    let report = results.report();
    for (gate_id, expected) in expected_gates {
        let expected_count = usize::try_from(expected["count"].as_u64().unwrap()).unwrap();
        assert_eq!(
            results.count(gate_id),
            Some(expected_count),
            "count mismatch for gate `{gate_id}`"
        );

        let membership = results.membership(gate_id).unwrap();
        let expected_membership =
            decode_membership(expected["membership_hex"].as_str().unwrap(), event_count);
        let mismatches = membership
            .iter()
            .zip(&expected_membership)
            .enumerate()
            .filter(|(_, (actual, expected))| actual != expected)
            .map(|(event, _)| event)
            .collect::<Vec<_>>();
        assert!(
            mismatches.is_empty(),
            "membership mismatch for gate `{gate_id}` at events {mismatches:?}"
        );

        let row = report.iter().find(|row| row.gate_id == *gate_id).unwrap();
        assert_eq!(row.gate_path, expected["gate_path"].as_str().unwrap());
    }
}

#[test]
fn child_gates_are_subsets_of_their_parents() {
    let xml = fs::read_to_string(fixture("gates.xml")).unwrap();
    let dataset = FcsReader::open(fixture("sample.fcs"))
        .and_then(FcsReader::read)
        .unwrap();
    let results = parse_gatingml(&xml).unwrap().gate_sample(&dataset).unwrap();

    for gate_id in results.gate_ids() {
        let Some(parent) = results.parent(gate_id) else {
            continue;
        };
        let child = results.membership(gate_id).unwrap();
        let parent = results.membership(parent).unwrap();
        assert!(
            child
                .iter()
                .zip(parent)
                .all(|(child, parent)| !child || *parent),
            "gate `{gate_id}` contains events outside its parent"
        );
    }
}
