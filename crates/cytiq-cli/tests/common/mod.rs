//! Helpers shared by the CLI integration tests.

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
