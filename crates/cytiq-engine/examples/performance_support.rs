//! Fixture generation and strict output comparison for `performance/run.py`.
//! This example is a development tool, not a cytiq subcommand.

use std::error::Error;
use std::fmt::Write as _;
use std::fs;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand};
use cytiq_fcs::{FcsReader, FcsWriteOptions, FcsWriteParameter};
use cytiq_mask::EventMask;
use cytiq_provenance::{FileRecord, RunRecord};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

#[derive(Parser)]
struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Generate {
        #[arg(long)]
        output: PathBuf,
        #[arg(long)]
        events: usize,
        #[arg(long, default_value_t = 42)]
        seed: u64,
    },
    Inspect {
        #[arg(long)]
        input: PathBuf,
    },
    Compare {
        #[arg(long)]
        baseline: PathBuf,
        #[arg(long)]
        candidate: PathBuf,
    },
}

fn main() -> Result<()> {
    match Args::parse().command {
        Command::Generate {
            output,
            events,
            seed,
        } => generate(&output, events, seed),
        Command::Inspect { input } => {
            let stream = FcsReader::open(input)?.stream(
                NonZeroUsize::new(8192).unwrap(),
                cytiq_fcs::FcsReadLimits::default(),
            )?;
            println!(
                "{}",
                serde_json::json!({"events": stream.metadata.event_count})
            );
            Ok(())
        }
        Command::Compare {
            baseline,
            candidate,
        } => compare(&baseline, &candidate),
    }
}

/// `SplitMix64`: fixed integer operations make the input independent of Rust's
/// random-library version. Generator version 1; record seed and file hashes.
fn next_random(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut value = *state;
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

#[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
fn generate(output: &Path, events: usize, seed: u64) -> Result<()> {
    if events == 0 {
        return Err("events must be positive".into());
    }
    let mut names = ["Time", "FSC-A", "FSC-H", "SSC-A"]
        .map(str::to_owned)
        .to_vec();
    names.extend((1..=26).map(|index| format!("FL{index}-A")));
    let parameters = names
        .iter()
        .map(|name| FcsWriteParameter::new(name).with_range(262_144.0))
        .collect::<Vec<_>>();
    let mut state = seed;
    let columns = (0..names.len())
        .map(|channel| {
            (0..events)
                .map(|event| {
                    if channel == 0 {
                        return event as f32;
                    }
                    let noise = (next_random(&mut state) >> 40) as f32 / 256.0;
                    if channel < 4 {
                        noise + 100.0
                    } else if event % 9973 == 0 {
                        200_000.0 + noise / 10.0
                    } else {
                        noise - 500.0
                    }
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let mut spillover = format!("26,{}", names[4..].join(","));
    for row in 0..26 {
        for column in 0..26 {
            let coefficient = if row == column {
                1.0
            } else if column == (row + 1) % 26 {
                0.02
            } else {
                0.0
            };
            write!(spillover, ",{coefficient}")?;
        }
    }
    let options = FcsWriteOptions::new()
        .with_text_keyword("$SPILLOVER", spillover)
        .with_text_keyword("$TIMESTEP", "0.01");
    cytiq_fcs::write_fcs3_1_f32_with_options(output, &parameters, &columns, &options)?;
    Ok(())
}

fn files(root: &Path) -> Result<Vec<PathBuf>> {
    fn visit(root: &Path, directory: &Path, found: &mut Vec<PathBuf>) -> Result<()> {
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            let kind = entry.file_type()?;
            if kind.is_dir() {
                visit(root, &entry.path(), found)?;
            } else if kind.is_file() {
                found.push(entry.path().strip_prefix(root)?.to_path_buf());
            } else {
                return Err(format!("unexpected output type: {}", entry.path().display()).into());
            }
        }
        Ok(())
    }
    let mut found = Vec::new();
    visit(root, root, &mut found)?;
    found.sort();
    if found.is_empty() {
        return Err(format!("no output files in {}", root.display()).into());
    }
    Ok(found)
}

fn normalized_provenance(path: &Path) -> Result<serde_json::Value> {
    // Deserialise the contract before removing exactly the three fields that
    // may vary. Missing metadata or changes to config/build/lineage still fail.
    let mut value: serde_json::Value = serde_json::from_slice(&fs::read(path)?)?;
    let _: RunRecord = serde_json::from_value(value.clone())?;
    let object = value
        .as_object_mut()
        .ok_or("provenance must be an object")?;
    object.remove("started_at_unix_ms");
    object.remove("finished_at_unix_ms");
    object
        .get_mut("build")
        .and_then(serde_json::Value::as_object_mut)
        .ok_or("provenance must contain build")?
        .remove("git_commit");
    Ok(value)
}

fn compare(baseline: &Path, candidate: &Path) -> Result<()> {
    let paths = files(baseline)?;
    if paths != files(candidate)? {
        return Err("output file sets differ".into());
    }
    for relative in paths {
        let left = baseline.join(&relative);
        let right = candidate.join(&relative);
        let name = relative
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        let equal = if name == "provenance.json" || name.ends_with(".provenance.json") {
            normalized_provenance(&left)? == normalized_provenance(&right)?
        } else if relative.extension().is_some_and(|ext| ext == "parquet") {
            EventMask::read_parquet(&left)? == EventMask::read_parquet(&right)?
        } else {
            // FCS includes its TEXT metadata and numeric metrics. Comparing its
            // bytes catches signed zero and the last bit of every stored value.
            let left = FileRecord::of(&left)?;
            let right = FileRecord::of(&right)?;
            left.bytes == right.bytes && left.sha256 == right.sha256
        };
        if !equal {
            return Err(format!("outputs differ: {}", relative.display()).into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generator_is_repeatable_and_compensable() {
        let directory = tempfile::tempdir().unwrap();
        let left = directory.path().join("a.fcs");
        let right = directory.path().join("b.fcs");
        generate(&left, 8193, 42).unwrap();
        generate(&right, 8193, 42).unwrap();
        assert_eq!(fs::read(&left).unwrap(), fs::read(&right).unwrap());
        let dataset = FcsReader::open(&left).unwrap().read().unwrap();
        assert_eq!(dataset.event_count(), 8193);
        assert_eq!(dataset.parameters.len(), 30);
        assert!(
            cytiq_fcs::Spillover::from_dataset(&dataset)
                .unwrap()
                .unwrap()
                .compensation_matrix()
                .is_ok()
        );
    }

    #[test]
    fn comparison_rejects_empty_missing_and_changed_outputs() {
        let directory = tempfile::tempdir().unwrap();
        let left = directory.path().join("left");
        let right = directory.path().join("right");
        fs::create_dir(&left).unwrap();
        fs::create_dir(&right).unwrap();
        assert!(compare(&left, &right).is_err());
        fs::write(left.join("metrics.json"), "{\"mean\":-0.0}").unwrap();
        assert!(compare(&left, &right).is_err());
        fs::write(right.join("metrics.json"), "{\"mean\":-0.0}").unwrap();
        compare(&left, &right).unwrap();
        fs::write(right.join("metrics.json"), "{\"mean\":0.0}").unwrap();
        assert!(compare(&left, &right).is_err());
    }

    #[test]
    fn provenance_ignores_only_clock_and_commit() {
        let directory = tempfile::tempdir().unwrap();
        let left = directory.path().join("left");
        let right = directory.path().join("right");
        fs::create_dir(&left).unwrap();
        fs::create_dir(&right).unwrap();
        let mut record = serde_json::json!({
            "schema_version": 2, "tool": "cytiq compensate", "version": "0.2.1",
            "build": {"git_commit": "baseline", "rustc": "same", "target": "same", "profile": "release"},
            "platform": {"os": "same", "arch": "same", "cpus": 4},
            "arguments": [], "config": {"threads": 4}, "constants": {}, "algorithms": {},
            "started_at_unix_ms": 1, "finished_at_unix_ms": 2, "inputs": [], "outputs": []
        });
        fs::write(
            left.join("provenance.json"),
            serde_json::to_vec(&record).unwrap(),
        )
        .unwrap();
        record["build"]["git_commit"] = "candidate".into();
        record["started_at_unix_ms"] = 3.into();
        record["finished_at_unix_ms"] = 4.into();
        fs::write(
            right.join("provenance.json"),
            serde_json::to_vec(&record).unwrap(),
        )
        .unwrap();
        compare(&left, &right).unwrap();
        record["config"]["threads"] = 1.into();
        fs::write(
            right.join("provenance.json"),
            serde_json::to_vec(&record).unwrap(),
        )
        .unwrap();
        assert!(compare(&left, &right).is_err());
        record["config"]["threads"] = 4.into();
        record["build"]["profile"] = "debug".into();
        fs::write(
            right.join("provenance.json"),
            serde_json::to_vec(&record).unwrap(),
        )
        .unwrap();
        assert!(compare(&left, &right).is_err());
    }
}
