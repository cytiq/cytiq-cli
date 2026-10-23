//! Helpers shared by the CLI integration tests.

// Each test crate uses a different subset of these helpers.
#![allow(dead_code)]

use std::collections::HashMap;
use std::fmt::Write as _;
use std::fs;
use std::path::Path;

use sha2::{Digest, Sha256};

/// Assert that `record` is a provenance record of `tool` that lists every
/// path in `inputs` and `outputs` with its current SHA-256.
pub fn assert_provenance(record: &Path, tool: &str, inputs: &[&Path], outputs: &[&Path]) {
    let text = fs::read_to_string(record)
        .unwrap_or_else(|error| panic!("provenance {} should exist: {error}", record.display()));
    let record: serde_json::Value = serde_json::from_str(&text).expect("provenance should be JSON");
    assert_eq!(record["tool"], tool);
    assert!(
        record["arguments"]
            .as_array()
            .is_some_and(|arguments| !arguments.is_empty())
    );
    for (key, paths) in [("inputs", inputs), ("outputs", outputs)] {
        let listed = record[key].as_array().expect("file list");
        for path in paths {
            let bytes = fs::read(path).expect("listed file should exist");
            let sha256 = Sha256::digest(&bytes)
                .iter()
                .fold(String::new(), |mut hex, byte| {
                    let _ = write!(hex, "{byte:02x}");
                    hex
                });
            let entry = listed
                .iter()
                .find(|entry| entry["path"] == path.display().to_string())
                .unwrap_or_else(|| panic!("{key} should list {}: {listed:?}", path.display()));
            assert_eq!(entry["sha256"], sha256, "{}", path.display());
        }
    }
}

/// Adjusted Rand index between two labelings of the same events.
#[allow(clippy::cast_precision_loss)]
pub fn adjusted_rand_index(left: &[u64], right: &[u64]) -> f64 {
    assert_eq!(left.len(), right.len());
    let pairs = |count: usize| (count * count.saturating_sub(1) / 2) as f64;
    let mut table = HashMap::<(u64, u64), usize>::new();
    let mut left_counts = HashMap::<u64, usize>::new();
    let mut right_counts = HashMap::<u64, usize>::new();
    for (a, b) in left.iter().zip(right) {
        *table.entry((*a, *b)).or_default() += 1;
        *left_counts.entry(*a).or_default() += 1;
        *right_counts.entry(*b).or_default() += 1;
    }
    let index = table.values().map(|count| pairs(*count)).sum::<f64>();
    let left_sum = left_counts.values().map(|count| pairs(*count)).sum::<f64>();
    let right_sum = right_counts
        .values()
        .map(|count| pairs(*count))
        .sum::<f64>();
    let expected = left_sum * right_sum / pairs(left.len());
    let maximum = f64::midpoint(left_sum, right_sum);
    if (maximum - expected).abs() < f64::EPSILON {
        1.0
    } else {
        (index - expected) / (maximum - expected)
    }
}

/// Every `*.provenance.json` under `directory`: a file read by one stage has
/// the SHA-256 another stage recorded when it wrote that file.
pub fn assert_provenance_chain(directory: &Path) {
    let records = walk(directory)
        .into_iter()
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.ends_with("provenance.json"))
        })
        .map(|path| {
            serde_json::from_str::<serde_json::Value>(&fs::read_to_string(&path).expect("record"))
                .expect("record should be JSON")
        })
        .collect::<Vec<_>>();
    assert!(
        !records.is_empty(),
        "no provenance records under {}",
        directory.display()
    );
    let canonical = |path: &str| {
        let path = Path::new(path);
        let path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            directory.join(path)
        };
        fs::canonicalize(&path).unwrap_or(path)
    };
    let written = records
        .iter()
        .flat_map(|record| record["outputs"].as_array().cloned().unwrap_or_default())
        .map(|output| {
            (
                canonical(output["path"].as_str().expect("path")),
                output["sha256"].clone(),
            )
        })
        .collect::<HashMap<_, _>>();
    let mut linked = 0;
    for record in &records {
        for input in record["inputs"].as_array().into_iter().flatten() {
            let path = canonical(input["path"].as_str().expect("path"));
            if let Some(sha256) = written.get(&path) {
                assert_eq!(
                    &input["sha256"],
                    sha256,
                    "{} changed between stages",
                    path.display()
                );
                linked += 1;
            }
        }
    }
    assert!(linked > 0, "no stage read another stage's output");
}

fn walk(directory: &Path) -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();
    for entry in fs::read_dir(directory).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            files.extend(walk(&path));
        } else {
            files.push(path);
        }
    }
    files
}
