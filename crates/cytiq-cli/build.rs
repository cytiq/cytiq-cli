//! Build information for provenance records: the commit, the compiler, target
//! and profile. The commit is `CYTIQ_BUILD_COMMIT` when set (the Docker build),
//! else the one cargo recorded in `.cargo_vcs_info.json` when packaging (a
//! crates.io install), else `git rev-parse HEAD`.

use std::env;
use std::fs;
use std::path::Path;
use std::process::Command;

/// Written by `cargo package` into the root of every packaged crate.
const VCS_INFO: &str = ".cargo_vcs_info.json";

fn main() {
    println!("cargo:rerun-if-env-changed=CYTIQ_BUILD_COMMIT");
    let commit = env::var("CYTIQ_BUILD_COMMIT")
        .ok()
        .filter(|commit| !commit.trim().is_empty())
        .or_else(packaged_commit)
        .or_else(git_commit)
        .unwrap_or_else(|| "unknown".to_owned());
    let rustc = env::var("RUSTC").unwrap_or_else(|_| "rustc".to_owned());
    let rustc_version =
        output(Command::new(rustc).arg("--version")).unwrap_or_else(|| "unknown".to_owned());
    println!("cargo:rustc-env=CYTIQ_GIT_COMMIT={}", commit.trim());
    println!("cargo:rustc-env=CYTIQ_RUSTC_VERSION={rustc_version}");
    println!(
        "cargo:rustc-env=CYTIQ_TARGET={}",
        env::var("TARGET").unwrap_or_default()
    );
    println!(
        "cargo:rustc-env=CYTIQ_PROFILE={}",
        env::var("PROFILE").unwrap_or_default()
    );
}

/// The commit a packaged crate was built from, with `-dirty` when cargo
/// packaged uncommitted changes. Unpacked registry sources have no `.git` of
/// their own, and `git` there would report whatever repository encloses the
/// cargo home.
fn packaged_commit() -> Option<String> {
    let path = Path::new(&env::var("CARGO_MANIFEST_DIR").ok()?).join(VCS_INFO);
    let text = fs::read_to_string(&path).ok()?;
    println!("cargo:rerun-if-changed={}", path.display());
    let sha1 = json_string(&text, "sha1")?;
    let dirty = text
        .split_once("\"dirty\"")
        .is_some_and(|(_, rest)| rest.trim_start_matches([':', ' ']).starts_with("true"));
    Some(if dirty { format!("{sha1}-dirty") } else { sha1 })
}

/// The string value of `key` in cargo's flat, pretty-printed JSON.
fn json_string(text: &str, key: &str) -> Option<String> {
    let (_, rest) = text.split_once(&format!("\"{key}\""))?;
    let (_, rest) = rest.split_once('"')?;
    let (value, _) = rest.split_once('"')?;
    (!value.is_empty()).then(|| value.to_owned())
}

fn git_commit() -> Option<String> {
    // Rebuild when HEAD moves to another commit, including a commit on the
    // current branch.
    for path in ["HEAD", "packed-refs"]
        .into_iter()
        .map(str::to_owned)
        .chain(git(&["symbolic-ref", "-q", "HEAD"]))
        .filter_map(|reference| git(&["rev-parse", "--git-path", &reference]))
    {
        println!("cargo:rerun-if-changed={path}");
    }
    git(&["rev-parse", "HEAD"])
}

fn git(args: &[&str]) -> Option<String> {
    output(Command::new("git").args(args))
}

fn output(command: &mut Command) -> Option<String> {
    let output = command.output().ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        .filter(|text| !text.is_empty())
}
