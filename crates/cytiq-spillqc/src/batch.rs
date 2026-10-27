//! Batch view across files (a cytiq extension; `CompensAID` assesses one file).
//!
//! A wrong spillover coefficient skews every file compensated with that
//! matrix, while biology-driven skew is sample-specific. Files are grouped by
//! the identity of the matrix applied to them, and each primary/secondary pair
//! is called recurrent when enough of the group flags it in the same
//! direction.

#![allow(clippy::cast_precision_loss)]

use std::collections::HashMap;

use serde::Serialize;

use crate::compensaid::{CompensaidResult, SsiStatus};

pub const DEFAULT_BATCH_MIN_FILES: usize = 3;
pub const DEFAULT_BATCH_MIN_FRACTION: f64 = 0.5;
/// Group of files with no known matrix (`--preprocessed` without one).
pub const UNKNOWN_MATRIX: &str = "unknown";

/// When a pair counts as recurrent within a matrix group.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct BatchSettings {
    /// Fewer assessed files make the pair `insufficient`.
    pub min_files: usize,
    /// Share of assessed files flagging the pair in one direction.
    pub min_fraction: f64,
}

impl Default for BatchSettings {
    fn default() -> Self {
        Self {
            min_files: DEFAULT_BATCH_MIN_FILES,
            min_fraction: DEFAULT_BATCH_MIN_FRACTION,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BatchStatus {
    RecurrentOver,
    RecurrentUnder,
    /// Flagged in some files, below the recurrence share.
    Sporadic,
    Pass,
    /// Assessed in fewer than `min_files` files.
    Insufficient,
}

impl BatchStatus {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::RecurrentOver => "recurrent_over",
            Self::RecurrentUnder => "recurrent_under",
            Self::Sporadic => "sporadic",
            Self::Pass => "pass",
            Self::Insufficient => "insufficient",
        }
    }

    #[must_use]
    pub const fn is_recurrent(self) -> bool {
        matches!(self, Self::RecurrentOver | Self::RecurrentUnder)
    }
}

/// One primary/secondary pair within a matrix group.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct BatchRow {
    pub matrix_id: String,
    pub primary_channel: String,
    pub primary_marker: String,
    pub secondary_channel: String,
    pub secondary_marker: String,
    /// Files of the group that hold both channels.
    pub files: usize,
    /// Of those, files with an SSI for the pair.
    pub files_assessed: usize,
    pub files_over: usize,
    pub files_under: usize,
    pub median_ssi: Option<f64>,
    pub status: BatchStatus,
}

#[derive(Default)]
struct Tally {
    primary_marker: String,
    secondary_marker: String,
    files: usize,
    ssi: Vec<f64>,
    over: usize,
    under: usize,
}

/// Summarize assessed files, each with the id of its matrix group. Rows keep
/// the order in which groups and pairs first appear.
#[must_use]
pub fn summarize(files: &[(&str, &CompensaidResult)], settings: &BatchSettings) -> Vec<BatchRow> {
    let mut positions: HashMap<(String, String, String), usize> = HashMap::new();
    let mut tallies: Vec<((String, String, String), Tally)> = Vec::new();
    for (matrix_id, result) in files {
        for pair in &result.pairs {
            let primary = &result.channels[pair.primary];
            let secondary = &result.channels[pair.secondary];
            let key = (
                (*matrix_id).to_owned(),
                primary.channel.clone(),
                secondary.channel.clone(),
            );
            let position = *positions.entry(key.clone()).or_insert_with(|| {
                tallies.push((
                    key,
                    Tally {
                        primary_marker: primary.marker.clone(),
                        secondary_marker: secondary.marker.clone(),
                        ..Tally::default()
                    },
                ));
                tallies.len() - 1
            });
            let tally = &mut tallies[position].1;
            tally.files += 1;
            if let Some(ssi) = pair.ssi {
                tally.ssi.push(ssi);
            }
            match pair.status {
                SsiStatus::Over => tally.over += 1,
                SsiStatus::Under => tally.under += 1,
                SsiStatus::Pass | SsiStatus::NotAssessed => {}
            }
        }
    }
    tallies
        .into_iter()
        .map(|((matrix_id, primary_channel, secondary_channel), tally)| {
            let assessed = tally.ssi.len();
            BatchRow {
                status: status(assessed, tally.over, tally.under, settings),
                matrix_id,
                primary_channel,
                primary_marker: tally.primary_marker,
                secondary_channel,
                secondary_marker: tally.secondary_marker,
                files: tally.files,
                files_assessed: assessed,
                files_over: tally.over,
                files_under: tally.under,
                median_ssi: median(tally.ssi),
            }
        })
        .collect()
}

fn status(assessed: usize, over: usize, under: usize, settings: &BatchSettings) -> BatchStatus {
    if assessed < settings.min_files {
        return BatchStatus::Insufficient;
    }
    let share = |count: usize| count as f64 / assessed as f64;
    let over_recurs = share(over) >= settings.min_fraction;
    let under_recurs = share(under) >= settings.min_fraction;
    if over_recurs && (!under_recurs || over >= under) {
        BatchStatus::RecurrentOver
    } else if under_recurs {
        BatchStatus::RecurrentUnder
    } else if over + under > 0 {
        BatchStatus::Sporadic
    } else {
        BatchStatus::Pass
    }
}

fn median(mut values: Vec<f64>) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    values.sort_by(f64::total_cmp);
    let middle = values.len() / 2;
    if values.len().is_multiple_of(2) {
        // SSIs have 2 decimals, so a midpoint has 3: drop the binary residue
        let midpoint = f64::midpoint(values[middle - 1], values[middle]);
        Some((midpoint * 1000.0).round() / 1000.0)
    } else {
        Some(values[middle])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compensaid::{MarkerChannel, PairAssessment};

    fn result(ssi: Option<f64>) -> CompensaidResult {
        let channel = |name: &str| MarkerChannel {
            channel: name.to_owned(),
            marker: name.to_lowercase(),
        };
        CompensaidResult {
            channels: vec![channel("A"), channel("B")],
            events: 100,
            center: 2.0,
            separation: 0.2,
            cutoffs: Vec::new(),
            pairs: vec![PairAssessment {
                primary: 0,
                secondary: 1,
                secondary_cutoff: 2.0,
                primary_cutoff_neg: 1.8,
                primary_cutoff_pos: 2.2,
                segments: Vec::new(),
                ssi,
                status: SsiStatus::of(ssi),
            }],
        }
    }

    #[test]
    fn pairs_flagged_in_most_files_of_a_matrix_recur() {
        let files = [result(Some(-1.5)), result(Some(-2.0)), result(Some(0.2))];
        let input = files.iter().map(|file| ("m1", file)).collect::<Vec<_>>();
        let rows = summarize(&input, &BatchSettings::default());
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].status, BatchStatus::RecurrentOver);
        assert_eq!((rows[0].files_assessed, rows[0].files_over), (3, 2));
        assert_eq!(rows[0].median_ssi, Some(-1.5));
        assert_eq!(median(vec![1.13, 1.15]), Some(1.14));
        assert_eq!(rows[0].primary_marker, "a");
    }

    #[test]
    fn groups_are_kept_apart_by_matrix() {
        let files = [
            result(Some(1.5)),
            result(Some(1.2)),
            result(Some(1.1)),
            result(Some(0.0)),
        ];
        let input = vec![
            ("m1", &files[0]),
            ("m1", &files[1]),
            ("m1", &files[2]),
            ("m2", &files[3]),
        ];
        let rows = summarize(&input, &BatchSettings::default());
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].status, BatchStatus::RecurrentUnder);
        assert_eq!(rows[1].status, BatchStatus::Insufficient);
    }

    #[test]
    fn rare_flags_are_sporadic_and_unassessed_files_do_not_count() {
        let files = [
            result(Some(-1.5)),
            result(Some(0.1)),
            result(Some(0.3)),
            result(Some(-0.2)),
            result(None),
        ];
        let input = files.iter().map(|file| ("m1", file)).collect::<Vec<_>>();
        let rows = summarize(&input, &BatchSettings::default());
        assert_eq!(rows[0].status, BatchStatus::Sporadic);
        assert_eq!((rows[0].files, rows[0].files_assessed), (5, 4));
    }
}
