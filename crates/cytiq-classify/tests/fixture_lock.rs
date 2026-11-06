//! The cytiq-ml bundles under `data/fixtures/classify` are pinned by
//! `cytiq-ml.lock.json`: a changed, added or removed fixture must update the
//! lock, which records where the fixtures came from.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;
use sha2::{Digest, Sha256};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/fixtures/classify")
}

fn sha256(path: &Path) -> String {
    Sha256::digest(fs::read(path).expect("read fixture"))
        .iter()
        .fold(String::new(), |mut text, byte| {
            use std::fmt::Write as _;
            write!(text, "{byte:02x}").expect("write to a String");
            text
        })
}

fn fixture_files(directory: &Path, root: &Path, files: &mut BTreeSet<String>) {
    for entry in fs::read_dir(directory).expect("read fixture directory") {
        let path = entry.expect("read fixture entry").path();
        if path.is_dir() {
            fixture_files(&path, root, files);
        } else if path.file_name().is_some_and(|name| name != "README.md") {
            let relative = path.strip_prefix(root).expect("under the root");
            files.insert(relative.to_str().expect("UTF-8 path").replace('\\', "/"));
        }
    }
}

#[test]
fn cytiq_ml_fixtures_match_their_lock() {
    let root = root();
    let lock: Value =
        serde_json::from_slice(&fs::read(root.join("cytiq-ml.lock.json")).expect("read lock"))
            .expect("lock is JSON");
    assert_eq!(lock["repository"], "cytiq/cytiq-ml");
    assert!(
        lock["revision"].is_null()
            || lock["revision"]
                .as_str()
                .is_some_and(|revision| revision.len() == 40
                    && revision.bytes().all(|byte| byte.is_ascii_hexdigit())),
        "revision must be a full commit hash or null"
    );
    let mut locked = BTreeSet::new();
    for file in lock["files"].as_array().expect("files") {
        let path = file["path"].as_str().expect("path");
        assert_eq!(
            sha256(&root.join(path)),
            file["sha256"].as_str().expect("sha256"),
            "{path} changed; regenerate the fixture lock"
        );
        assert!(locked.insert(path.to_owned()), "{path} is locked twice");
    }
    let mut present = BTreeSet::new();
    fixture_files(&root, &root, &mut present);
    present.remove("cytiq-ml.lock.json");
    assert_eq!(
        present, locked,
        "every fixture is locked, and only fixtures"
    );
}
