//! Replays saved fuzz inputs through the check the fuzz target runs, so a
//! crash found once stays fixed without nightly or cargo-fuzz. Hand-written
//! seeds also pin a verdict: `valid*` inputs must parse and validate, other
//! named inputs must not; `crash-*` inputs found by the fuzzer only replay.

use std::fs;
use std::path::{Path, PathBuf};

#[path = "../fuzz/checks.rs"]
mod checks;

fn files_in(directory: &Path) -> Vec<PathBuf> {
    let mut paths = fs::read_dir(directory)
        .unwrap_or_else(|error| panic!("read {}: {error}", directory.display()))
        .map(|entry| entry.expect("read directory entry").path())
        .filter(|path| path.is_file())
        .collect::<Vec<_>>();
    paths.sort();
    paths
}

#[test]
fn replays_every_fuzz_regression() {
    let directory =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fuzz_regressions/specimen_documents");
    let inputs = files_in(&directory);
    let mut accepted = 0;
    for input in inputs {
        let bytes = fs::read(&input).expect("read saved input");
        eprintln!("specimen_documents: {}", input.display());
        checks::specimen_documents(&bytes);
        let name = input
            .file_name()
            .and_then(|name| name.to_str())
            .expect("UTF-8 file name");
        if !name.starts_with("crash-") {
            let valid = checks::documents(&bytes).is_some();
            assert_eq!(valid, name.starts_with("valid"), "{name}");
            accepted += usize::from(valid);
        }
    }
    assert!(accepted > 0, "no accepted seed");
}
