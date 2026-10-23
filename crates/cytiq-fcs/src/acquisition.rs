//! Identity of the raw acquisition a dataset and its event masks come from.

use std::fmt::Write as _;
use std::fs::File;
use std::io;
use std::path::Path;

use sha2::{Digest, Sha256};

use crate::error::FcsError;
use crate::types::FcsText;

/// SHA-256 of the raw FCS file a derived file comes from. Every cytiq stage
/// that writes FCS records it, so masks from any stage can be matched to the
/// acquisition whose rows their `source_event_id`s count.
pub const ACQUISITION_KEYWORD: &str = "CYTIQ_ACQUISITION";

/// The acquisition a dataset read from `path` belongs to: the recorded
/// `CYTIQ_ACQUISITION`, or the SHA-256 of `path` itself when the file is raw.
///
/// # Errors
///
/// Returns an error when the recorded value is not a lowercase hex SHA-256 or
/// the file cannot be read.
pub fn acquisition_id(path: &Path, text: &FcsText) -> Result<String, FcsError> {
    match text.get(ACQUISITION_KEYWORD) {
        Some(value) if is_sha256(value) => Ok(value.to_owned()),
        Some(value) => Err(FcsError::InvalidKeywordValue {
            key: ACQUISITION_KEYWORD.to_owned(),
            value: value.to_owned(),
        }),
        None => sha256_file(path).map_err(FcsError::Io),
    }
}

/// Record `id` as the acquisition of a file about to be written, replacing any
/// carried value.
pub fn set_acquisition(keywords: &mut Vec<(String, String)>, id: &str) {
    keywords.retain(|(key, _)| key != ACQUISITION_KEYWORD);
    keywords.push((ACQUISITION_KEYWORD.to_owned(), id.to_owned()));
}

/// Whether `value` is a lowercase hex SHA-256.
#[must_use]
pub fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn sha256_file(path: &Path) -> io::Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    io::copy(&mut file, &mut hasher)?;
    Ok(hasher
        .finalize()
        .iter()
        .fold(String::with_capacity(64), |mut hex, byte| {
            let _ = write!(hex, "{byte:02x}");
            hex
        }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_acquisition_replaces_a_carried_value() {
        let mut keywords = vec![
            ("$CYT".to_owned(), "Aurora".to_owned()),
            (ACQUISITION_KEYWORD.to_owned(), "a".repeat(64)),
        ];
        set_acquisition(&mut keywords, &"b".repeat(64));
        assert_eq!(
            keywords,
            vec![
                ("$CYT".to_owned(), "Aurora".to_owned()),
                (ACQUISITION_KEYWORD.to_owned(), "b".repeat(64)),
            ]
        );
    }

    #[test]
    fn recognises_lowercase_sha256() {
        assert!(is_sha256(&"0f".repeat(32)));
        assert!(!is_sha256(&"0F".repeat(32)));
        assert!(!is_sha256("abc"));
    }
}
