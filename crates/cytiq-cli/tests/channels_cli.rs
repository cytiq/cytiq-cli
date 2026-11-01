//! Every command with channel flags rejects empty ones with the same messages,
//! before it reads any input.

use std::process::Command;

/// Commands whose `--channels` / `--channel` pair is required.
const REQUIRED: &[&str] = &["peacoqc", "flowsom", "cytoscan"];

fn run(command: &str, channels: &[&str]) -> (bool, String) {
    let temp = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_cytiq"))
        .arg(command)
        .arg("--input")
        .arg(temp.path().join("missing.fcs"))
        .arg("--output")
        .arg(temp.path().join("out.fcs"))
        .args(channels)
        .output()
        .unwrap();
    (
        output.status.success(),
        String::from_utf8(output.stderr).unwrap(),
    )
}

fn assert_fails_with(command: &str, channels: &[&str], message: &str) {
    let (success, stderr) = run(command, channels);
    assert!(!success, "{command} {channels:?} succeeded");
    assert_eq!(
        stderr.lines().last(),
        Some(message),
        "{command} {channels:?}: {stderr}"
    );
}

#[test]
fn empty_channel_flags_fail_with_the_same_messages_in_every_command() {
    for command in REQUIRED {
        assert_fails_with(
            command,
            &[],
            "at least one `--channels` or `--channel` value is required",
        );
        assert_fails_with(
            command,
            &["--channels", " , ,"],
            "channel list must contain at least one channel",
        );
        assert_fails_with(command, &["--channel", " "], "channel must not be empty");
    }
    // transform's channels are optional, but an empty list is still an error.
    for flag in ["--channels", "--exclude-channels"] {
        assert_fails_with(
            "transform",
            &[flag, ","],
            "channel list must contain at least one channel",
        );
    }
}
