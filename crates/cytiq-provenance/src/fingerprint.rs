//! Versioned execution identity. Bump the version when these semantics change.
use crate::RunRecord;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::fmt::Write as _;

pub const FINGERPRINT_VERSION: u32 = 1;
pub const NON_SEMANTIC_OPTIONS: &[&str] = &["overwrite", "debug_dir", "dry_run", "threads"];

#[must_use]
pub fn semantic_config(mut config: Value) -> Value {
    match &mut config {
        Value::Object(map) => {
            map.retain(|key, _| !NON_SEMANTIC_OPTIONS.contains(&key.as_str()));
            for value in map.values_mut() {
                *value = semantic_config(value.take());
            }
        }
        Value::Array(values) => {
            for value in values {
                *value = semantic_config(value.take());
            }
        }
        _ => {}
    }
    config
}

#[must_use]
pub fn compute(record: &RunRecord) -> String {
    let components = json!({
        "fingerprint_version": FINGERPRINT_VERSION,
        "tool": record.tool, "version": record.version, "build": record.build,
        "config": semantic_config(record.config.clone()), "constants": record.constants,
        "algorithms": record.algorithms, "inputs": record.inputs, "parallelism": record.parallelism,
    });
    // serde_json uses BTreeMap without preserve_order; the test pins this contract.
    Sha256::digest(components.to_string().as_bytes())
        .iter()
        .fold(String::with_capacity(64), |mut hex, byte| {
            write!(hex, "{byte:02x}").expect("write to String");
            hex
        })
}

#[must_use]
pub fn reusable_build(commit: &str) -> bool {
    let (sha, dirty) = commit
        .split_once("-dirty.")
        .map_or((commit, None), |(sha, digest)| (sha, Some(digest)));
    (sha.len() == 40 || sha.len() == 64)
        && sha.bytes().all(|b| b.is_ascii_hexdigit())
        && dirty.is_none_or(|digest| {
            digest.len() == 12 && digest.bytes().all(|b| b.is_ascii_hexdigit())
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn json_keys_are_sorted_including_nested_objects() {
        let value: Value = serde_json::from_str(r#"{"z":{"y":1,"a":2},"a":3}"#).unwrap();
        assert_eq!(
            serde_json::to_string(&value).unwrap(),
            r#"{"a":3,"z":{"a":2,"y":1}}"#
        );
    }
    #[test]
    fn unknown_and_unidentified_dirty_builds_are_not_reusable() {
        assert!(!reusable_build("unknown"));
        assert!(!reusable_build(&format!("{}-dirty", "a".repeat(40))));
        assert!(reusable_build(&"a".repeat(40)));
        assert!(reusable_build(&format!(
            "{}-dirty.012345abcdef",
            "a".repeat(40)
        )));
    }
}
