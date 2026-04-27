#![allow(dead_code, clippy::missing_panics_doc, clippy::must_use_candidate)]

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub fn autospectral_fixture_dir() -> PathBuf {
    fixture_dir("autospectral")
}

pub fn create_temp_dir(label: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should be after unix epoch")
        .as_nanos();
    let directory = std::env::temp_dir().join(format!("cytiq-{label}-{unique}"));
    fs::create_dir_all(&directory).expect("temporary directory should be created");
    directory
}

pub fn copy_autospectral_fixture_set(label: &str) -> PathBuf {
    let source = autospectral_fixture_dir();
    let target = create_temp_dir(label);
    for entry in fs::read_dir(&source).expect("autospectral fixture dir should be readable") {
        let entry = entry.expect("fixture entry should be readable");
        let source_path = entry.path();
        if source_path.is_file() {
            fs::copy(&source_path, target.join(entry.file_name()))
                .expect("fixture file should be copied");
        }
    }

    target
}

fn fixture_dir(name: &str) -> PathBuf {
    for start in fixture_search_roots() {
        for candidate in start
            .ancestors()
            .map(|ancestor| ancestor.join("data").join("fixtures").join(name))
        {
            if candidate.is_dir() {
                return candidate;
            }
        }
    }

    panic!("fixture directory should exist: data/fixtures/{name}");
}

fn fixture_search_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();

    if let Ok(current_dir) = std::env::current_dir() {
        roots.push(current_dir);
    }

    if let Ok(current_exe) = std::env::current_exe() {
        roots.push(current_exe);
    }

    roots.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")));
    roots
}

#[allow(dead_code)]
pub fn utf8(path: &Path) -> &str {
    path.to_str().expect("path should be valid utf-8")
}
