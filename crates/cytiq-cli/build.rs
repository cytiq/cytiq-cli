//! Build information for provenance records: the commit (from git, or
//! `CYTIQ_BUILD_COMMIT` where the sources have no `.git`, as in the Docker
//! build), the compiler, target and profile.

use std::env;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=CYTIQ_BUILD_COMMIT");
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
    let commit = env::var("CYTIQ_BUILD_COMMIT")
        .ok()
        .filter(|commit| !commit.trim().is_empty())
        .or_else(|| git(&["rev-parse", "HEAD"]))
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
