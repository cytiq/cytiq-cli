//! Replays saved fuzz inputs through the checks the fuzz targets run, so a
//! crash found once stays fixed without nightly or cargo-fuzz.

use std::fs;
use std::path::{Path, PathBuf};

use cytiq_fcs::{FcsReader, is_spillover_keyword};

#[path = "../fuzz/checks.rs"]
mod checks;

/// Each target's inputs live in `tests/fuzz_regressions/<target>/`.
const TARGETS: [(&str, fn(&[u8])); 2] = [
    ("read_fcs", checks::read_fcs),
    ("spillover_keyword", checks::spillover_keyword),
];

fn files_in(directory: &Path) -> Vec<PathBuf> {
    let mut paths = fs::read_dir(directory)
        .unwrap_or_else(|error| panic!("read {}: {error}", directory.display()))
        .map(|entry| entry.expect("read directory entry").path())
        .filter(|path| path.is_file())
        .collect::<Vec<_>>();
    paths.sort();
    paths
}

fn fcs_files_under(directory: &Path, paths: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(directory).expect("read fixture directory") {
        let path = entry.expect("read fixture entry").path();
        if path.is_dir() {
            fcs_files_under(&path, paths);
        } else if path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("fcs"))
        {
            paths.push(path);
        }
    }
}

#[test]
fn replays_every_fuzz_regression() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fuzz_regressions");
    for (target, check) in TARGETS {
        let inputs = files_in(&root.join(target));
        assert!(!inputs.is_empty(), "no saved inputs for {target}");
        for input in inputs {
            let bytes = fs::read(&input).expect("read saved input");
            eprintln!("{target}: {}", input.display());
            check(&bytes);
        }
    }
}

#[test]
fn fuzz_checks_accept_every_fixture() {
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/fixtures");
    let mut paths = Vec::new();
    fcs_files_under(&fixtures, &mut paths);
    assert!(
        !paths.is_empty(),
        "no FCS fixtures under {}",
        fixtures.display()
    );
    let mut keywords = 0;
    for path in paths {
        let bytes = fs::read(&path).expect("read fixture");
        checks::read_fcs(&bytes);
        let Ok(dataset) = FcsReader::new(std::io::Cursor::new(&bytes)).read() else {
            continue;
        };
        for (_, value) in dataset
            .text
            .iter()
            .filter(|(key, _)| is_spillover_keyword(key))
        {
            checks::spillover_keyword(value.as_bytes());
            keywords += 1;
        }
    }
    assert!(keywords > 0, "no fixture has a spillover keyword");
}
