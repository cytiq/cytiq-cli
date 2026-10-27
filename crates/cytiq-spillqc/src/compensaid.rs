//! Line-for-line port of `CompensAID::CompensAID(ff, segment.value,
//! events.value)` (`Olsman/CompensAID@db74803`, version 0.99.7) on marker
//! columns that are already compensated and transformed.
//!
//! Upstream function names are kept in the comments so each step can be
//! checked against the R source. Deliberate divergences, all on inputs where R
//! errors or propagates `NA` rows, are listed in this crate's CONTRIBUTING.md.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::float_cmp,
    clippy::too_many_lines
)]

use cytiq_density::{DEFAULT_TINYPEAK_REMOVAL, DeGate, degate_upper_cuts};
use rayon::prelude::*;
use serde::Serialize;

/// `segment.value`: segments of the positive population.
pub const DEFAULT_SEGMENTS: usize = 4;
/// `events.value`: minimum events per population and per segment.
pub const DEFAULT_MIN_EVENTS: usize = 50;
/// `|SSI| >= 1` is flagged, as `CompensAID::PlotMatrix` colours it.
pub const FLAG_THRESHOLD: f64 = 1.0;
/// `.WithinLimit(min = 10, max = 90)`: population shares (percent) that
/// trigger a second cut-off search.
const WITHIN_LIMIT: (f64, f64) = (10.0, 90.0);
/// `.WithinLimit`'s `deGate(tinypeak.removal = 0.0001)`.
const ADJUSTED_TINYPEAK_REMOVAL: f64 = 1e-4;
/// `.DetermineCenter`: bins at 5% of the tallest count still count.
const CENTER_COUNT_FRACTION: f64 = 0.05;
/// `.DetermineCenter`: `separation.distance <- center.plot * 0.10`.
const SEPARATION_FRACTION: f64 = 0.10;
const NO_POPULATION: &str = "No positive/negative population";
const PASS: &str = "PASS";

/// Settings of one run, `CompensAID(segment.value, events.value)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct CompensaidParameters {
    pub segments: usize,
    pub min_events: usize,
}

impl Default for CompensaidParameters {
    fn default() -> Self {
        Self {
            segments: DEFAULT_SEGMENTS,
            min_events: DEFAULT_MIN_EVENTS,
        }
    }
}

/// A marker channel: its `$PnN` and the marker name `CompensAID` reports.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct MarkerChannel {
    pub channel: String,
    pub marker: String,
}

/// Verdict on one primary/secondary pair or segment.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SsiStatus {
    Pass,
    /// `SSI <= -1`: the positive population is pushed below the negative one
    /// in the secondary channel, the signature of over-compensation.
    Over,
    /// `SSI >= 1`: residual signal in the secondary channel,
    /// under-compensation or co-expression below the secondary cut-off.
    Under,
    /// No SSI: too few positive or negative events.
    NotAssessed,
}

impl SsiStatus {
    #[must_use]
    pub fn of(ssi: Option<f64>) -> Self {
        match ssi {
            Some(value) if value <= -FLAG_THRESHOLD => Self::Over,
            Some(value) if value >= FLAG_THRESHOLD => Self::Under,
            Some(_) => Self::Pass,
            None => Self::NotAssessed,
        }
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pass => "pass",
            Self::Over => "over",
            Self::Under => "under",
            Self::NotAssessed => "not_assessed",
        }
    }

    #[must_use]
    pub const fn is_flag(self) -> bool {
        matches!(self, Self::Over | Self::Under)
    }
}

/// The density cut-off of one channel (`.DensityGating`).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ChannelCutoff {
    /// Every cut-off `deGate` returned (`-inf` for fewer than 20 events,
    /// NaN for `NA`).
    pub candidates: Vec<f64>,
    /// The candidate closest to the center (`opt`); NaN when there is none.
    pub cutoff: f64,
}

/// One row of `CompensAID`'s `matrixInfo` for a pair. `None` is R's `NA`.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SsiSegment {
    /// 1-based segment number, lowest primary intensity first.
    pub segment: usize,
    pub segment_min: Option<f64>,
    pub segment_max: Option<f64>,
    pub event_count: Option<usize>,
    pub event_count_merge: Option<usize>,
    pub mfi_neg: Option<f64>,
    pub sd_neg: Option<f64>,
    pub mfi_pos: Option<f64>,
    pub ssi: Option<f64>,
    pub message: Option<String>,
}

/// Assessment of one ordered marker pair.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PairAssessment {
    /// Index into [`CompensaidResult::channels`].
    pub primary: usize,
    pub secondary: usize,
    pub secondary_cutoff: f64,
    pub primary_cutoff_neg: f64,
    pub primary_cutoff_pos: f64,
    pub segments: Vec<SsiSegment>,
    /// The SSI matrix cell: the highest segment with an SSI.
    pub ssi: Option<f64>,
    pub status: SsiStatus,
}

/// `CompensAID`'s `matrix` and `matrixInfo` plus the intermediate values.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CompensaidResult {
    pub channels: Vec<MarkerChannel>,
    /// Events assessed, without the zero event `CompensAID` appends.
    pub events: usize,
    pub center: f64,
    pub separation: f64,
    pub cutoffs: Vec<ChannelCutoff>,
    /// Pairs in `CompensAID`'s order: secondary outer, primary inner.
    pub pairs: Vec<PairAssessment>,
}

impl CompensaidResult {
    /// SSI matrix cell for a (secondary, primary) pair.
    #[must_use]
    pub fn cell(&self, secondary: usize, primary: usize) -> Option<&PairAssessment> {
        self.pairs
            .iter()
            .find(|pair| pair.secondary == secondary && pair.primary == primary)
    }
}

/// Run `CompensAID` on `columns` (one per marker, compensated and transformed,
/// all the same length).
///
/// # Panics
///
/// Panics when `columns` and `channels` differ in length or the columns
/// differ in length.
#[must_use]
pub fn compensaid(
    channels: &[MarkerChannel],
    columns: &[Vec<f64>],
    parameters: &CompensaidParameters,
) -> CompensaidResult {
    assert_eq!(channels.len(), columns.len(), "one column per marker");
    let events = columns.first().map_or(0, Vec::len);
    assert!(
        columns.iter().all(|column| column.len() == events),
        "marker columns differ in length"
    );
    // "Add a rows of zero's to prevent errors during the density-based
    // cut-off detection"
    let columns = columns
        .iter()
        .map(|column| {
            let mut column = column.clone();
            column.push(0.0);
            column
        })
        .collect::<Vec<_>>();

    let (center, separation) = determine_center(&columns);
    let gates = columns
        .par_iter()
        .map(|column| ChannelGates {
            default: degate_upper_cuts(column, DEFAULT_TINYPEAK_REMOVAL),
            adjusted: degate_upper_cuts(column, ADJUSTED_TINYPEAK_REMOVAL),
        })
        .collect::<Vec<_>>();
    let cutoffs = gates
        .iter()
        .map(|gate| {
            let candidates = gate.default.values();
            let cutoff = closest_to_center(&candidates, center).unwrap_or(f64::NAN);
            ChannelCutoff { candidates, cutoff }
        })
        .collect::<Vec<_>>();
    let opt = cutoffs.iter().map(|cut| cut.cutoff).collect::<Vec<_>>();

    // expand.grid(primary, secondary): primary varies fastest
    let combinations = (0..channels.len())
        .flat_map(|secondary| {
            (0..channels.len())
                .filter(move |primary| *primary != secondary)
                .map(move |primary| (primary, secondary))
        })
        .collect::<Vec<_>>();
    let context = PairContext {
        columns: &columns,
        gates: &gates,
        opt: &opt,
        center,
        separation,
        parameters,
    };
    let pairs = combinations
        .par_iter()
        .map(|&(primary, secondary)| context.assess(primary, secondary))
        .collect();

    CompensaidResult {
        channels: channels.to_vec(),
        events,
        center,
        separation,
        cutoffs,
        pairs,
    }
}

struct ChannelGates {
    default: DeGate,
    adjusted: DeGate,
}

/// `.DetermineCenter`: a unit-width histogram (`hist`, right-closed, fuzzed
/// breaks) of every marker value; `center.plot` is half the upper edge of the
/// highest bin holding at least 5% of `max(counts)`.
fn determine_center(columns: &[Vec<f64>]) -> (f64, f64) {
    let values = columns.iter().flatten().copied();
    let (minimum, maximum) = values
        .clone()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), value| {
            (lo.min(value), hi.max(value))
        });
    // breaks = seq(round(min) - 1, round(max) + 1, by = 1)
    let first = minimum.round_ties_even() - 1.0;
    let last = maximum.round_ties_even() + 1.0;
    let bins = (last - first) as usize;
    let mut counts = vec![0_usize; bins];
    // hist's fuzz: 1e-7 times the bin width moves each break up, the first down
    let fuzz = 1e-7;
    for value in values {
        let position = value - first;
        let mut bin = (position - fuzz).ceil() as isize - 1;
        if bin < 0 && position >= -fuzz {
            bin = 0;
        }
        let bin = (bin.max(0) as usize).min(bins - 1);
        counts[bin] += 1;
    }
    // max() of the counts data.frame spans its `breaks` column (ceiling(mids))
    // as well as `counts`
    let upper_edges = (1..=bins).map(|index| first + index as f64);
    let tallest = counts
        .iter()
        .map(|count| *count as f64)
        .chain(upper_edges.clone())
        .fold(f64::NEG_INFINITY, f64::max);
    let threshold = CENTER_COUNT_FRACTION * tallest;
    let max_bin = counts
        .iter()
        .zip(upper_edges)
        .filter(|(count, _)| **count as f64 >= threshold)
        .map(|(_, edge)| edge)
        .fold(f64::NEG_INFINITY, f64::max);
    let center = max_bin / 2.0;
    (center, center * SEPARATION_FRACTION)
}

/// `.GetClosestCenter`: the first candidate nearest to `center`, ignoring
/// `NA`; `-Inf` is a candidate at infinite distance.
fn closest_to_center(candidates: &[f64], center: f64) -> Option<f64> {
    let mut best: Option<(f64, f64)> = None;
    for &candidate in candidates.iter().filter(|value| !value.is_nan()) {
        let distance = (candidate - center).abs();
        if best.is_none_or(|(_, nearest)| distance < nearest) {
            best = Some((candidate, distance));
        }
    }
    best.map(|(candidate, _)| candidate)
}

/// `.GetClosestLimit`: keep `old` only when strictly closer to the center.
fn closest_limit(old: f64, new: f64, center: f64) -> f64 {
    if (old - center).abs() < (new - center).abs() {
        old
    } else {
        new
    }
}

/// R's `round(x)` for the shares `.WithinLimit` compares.
fn rounded_percentage(part: usize, whole: usize) -> f64 {
    (part as f64 * 100.0 / whole as f64).round_ties_even()
}

/// R's `round(x, digits = 2)` for the SSI.
fn round_ssi(value: f64) -> f64 {
    if value.is_finite() {
        (value * 100.0).round_ties_even() / 100.0
    } else {
        value
    }
}

fn outside_limits(percentage: f64) -> bool {
    percentage <= WITHIN_LIMIT.0 || percentage >= WITHIN_LIMIT.1
}

/// Event indices of `.GetPopulations`: secondary-negative events, and among
/// them the primary-negative and primary-positive ones.
struct Populations {
    secondary_negative: Vec<usize>,
    primary_negative: Vec<usize>,
    primary_positive: Vec<usize>,
}

struct PairContext<'a> {
    columns: &'a [Vec<f64>],
    gates: &'a [ChannelGates],
    opt: &'a [f64],
    center: f64,
    separation: f64,
    parameters: &'a CompensaidParameters,
}

impl PairContext<'_> {
    /// One iteration of `CompensAID()`'s loop over marker combinations.
    fn assess(&self, primary: usize, secondary: usize) -> PairAssessment {
        let segments_count = self.parameters.segments;
        let mut secondary_cutoff = self.opt[secondary];
        let mut primary_cutoff = self.opt[primary];
        let mut primary_cutoff_neg = primary_cutoff - self.separation;
        let mut primary_cutoff_pos = primary_cutoff + self.separation;

        // .WithinLimit; the primary check reads the populations from before
        // the secondary adjustment, as upstream does
        let population = self.populations(primary, secondary, primary_cutoff, secondary_cutoff);
        let total = self.columns[secondary].len();
        if outside_limits(rounded_percentage(
            population.secondary_negative.len(),
            total,
        )) && let Some(new) =
            closest_to_center(&self.gates[secondary].adjusted.values(), self.center)
        {
            secondary_cutoff = closest_limit(secondary_cutoff, new, self.center);
        }
        let negatives = population.secondary_negative.len();
        if negatives > 0
            && (outside_limits(rounded_percentage(
                population.primary_positive.len(),
                negatives,
            )) || outside_limits(rounded_percentage(
                population.primary_negative.len(),
                negatives,
            )))
            && let Some(new) =
                closest_to_center(&self.gates[primary].adjusted.values(), self.center)
        {
            primary_cutoff = closest_limit(primary_cutoff, new, self.center);
            primary_cutoff_neg = primary_cutoff - self.separation;
            primary_cutoff_pos = primary_cutoff + self.separation;
        }
        let population = self.populations(primary, secondary, primary_cutoff, secondary_cutoff);

        let mut segments = (1..=segments_count)
            .map(|segment| SsiSegment {
                segment,
                segment_min: None,
                segment_max: None,
                event_count: None,
                event_count_merge: None,
                mfi_neg: None,
                sd_neg: None,
                mfi_pos: None,
                ssi: None,
                message: None,
            })
            .collect::<Vec<_>>();
        let min_events = self.parameters.min_events;
        if population.primary_negative.len() < min_events
            || population.primary_positive.len() < min_events
        {
            for row in &mut segments {
                row.message = Some(NO_POPULATION.to_owned());
            }
        } else {
            self.fill_segments(
                &mut segments,
                &population,
                primary,
                secondary,
                primary_cutoff_pos,
            );
        }

        let ssi = segments.iter().rev().find_map(|row| row.ssi);
        PairAssessment {
            primary,
            secondary,
            secondary_cutoff,
            primary_cutoff_neg,
            primary_cutoff_pos,
            segments,
            ssi,
            status: SsiStatus::of(ssi),
        }
    }

    /// `.GetPopulations`. Comparisons with a NaN cut-off select no events
    /// (R would select rows of `NA`).
    fn populations(
        &self,
        primary: usize,
        secondary: usize,
        primary_cutoff: f64,
        secondary_cutoff: f64,
    ) -> Populations {
        let p = &self.columns[primary];
        let s = &self.columns[secondary];
        let negative_limit = primary_cutoff - self.separation;
        let positive_limit = primary_cutoff + self.separation;
        let secondary_negative = (0..s.len())
            .filter(|&event| s[event] <= secondary_cutoff)
            .collect::<Vec<_>>();
        let primary_negative = secondary_negative
            .iter()
            .copied()
            .filter(|&event| p[event] <= negative_limit)
            .collect();
        let primary_positive = secondary_negative
            .iter()
            .copied()
            .filter(|&event| p[event] >= positive_limit)
            .collect();
        Populations {
            secondary_negative,
            primary_negative,
            primary_positive,
        }
    }

    /// `.UpdateMatrixInfo(output = "PASS")`, `.UpdateSegments` and
    /// `.UpdateSSI`.
    fn fill_segments(
        &self,
        rows: &mut [SsiSegment],
        population: &Populations,
        primary: usize,
        secondary: usize,
        primary_cutoff_pos: f64,
    ) {
        let p = &self.columns[primary];
        let s = &self.columns[secondary];
        let positive_p = population
            .primary_positive
            .iter()
            .map(|&event| p[event])
            .collect::<Vec<_>>();
        let positive_s = population
            .primary_positive
            .iter()
            .map(|&event| s[event])
            .collect::<Vec<_>>();
        // range <- (max(pop$primary.positive) - cutoff.pos) / segment.value,
        // where max() spans both columns of the population matrix
        let maximum = positive_p
            .iter()
            .chain(&positive_s)
            .copied()
            .fold(f64::NEG_INFINITY, f64::max);
        let range = (maximum - primary_cutoff_pos) / rows.len() as f64;

        let negative_s = population
            .primary_negative
            .iter()
            .map(|&event| s[event])
            .collect::<Vec<_>>();
        let mfi_neg = median(&negative_s);
        let sd_neg = sample_sd(&negative_s);
        let ssi_of = |mfi_pos: f64| round_ssi((mfi_pos - mfi_neg) / (2.0 * sd_neg));

        // .GetSegment
        let lowest = positive_p.iter().copied().fold(f64::INFINITY, f64::min);
        let highest_below = |bound: f64| {
            positive_p
                .iter()
                .copied()
                .filter(|value| *value <= bound)
                .fold(f64::NEG_INFINITY, f64::max)
        };
        let in_range = |low: f64, high: f64| {
            positive_p
                .iter()
                .zip(&positive_s)
                .filter(|(value, _)| **value >= low && **value <= high)
                .map(|(_, secondary)| *secondary)
                .collect::<Vec<_>>()
        };
        for row in rows.iter_mut() {
            let segment = row.segment as f64;
            let high = highest_below(lowest + segment * range);
            let low = if row.segment == 1 {
                lowest
            } else {
                highest_below(lowest + (segment - 1.0) * range)
            };
            let selected = in_range(low, high);
            let mfi_pos = median(&selected);
            row.segment_min = Some(low);
            row.segment_max = Some(high);
            row.event_count = Some(selected.len());
            row.mfi_pos = Some(mfi_pos);
            row.mfi_neg = Some(mfi_neg);
            row.sd_neg = Some(sd_neg);
            row.ssi = finite_or_none(ssi_of(mfi_pos));
            row.message = Some(PASS.to_owned());
        }

        // .UpdateSegments: .MergeSegments, .AdjustSegments, .CleanSegments
        let counts = rows
            .iter()
            .map(|row| row.event_count.unwrap_or(0))
            .collect::<Vec<_>>();
        let groups = merge_segments(&counts, self.parameters.min_events);
        let merged_any = groups.iter().any(|group| group.len() > 1);
        for group in &groups {
            let first = group[0];
            let min = group
                .iter()
                .filter_map(|&index| rows[index].segment_min)
                .fold(f64::INFINITY, f64::min);
            let max = group
                .iter()
                .filter_map(|&index| rows[index].segment_max)
                .fold(f64::NEG_INFINITY, f64::max);
            let sum = group.iter().map(|&index| counts[index]).sum::<usize>();
            let label = group
                .iter()
                .map(|&index| rows[index].segment.to_string())
                .collect::<Vec<_>>()
                .join("+");
            for &index in &group[1..] {
                let row = &mut rows[index];
                row.segment_min = None;
                row.segment_max = None;
                row.event_count_merge = None;
                row.message = None;
                row.mfi_neg = None;
                row.mfi_pos = None;
                row.sd_neg = None;
                row.ssi = None;
            }
            let row = &mut rows[first];
            row.segment_min = Some(min);
            row.segment_max = Some(max);
            row.event_count_merge = Some(sum);
            if merged_any && sum != counts[first] {
                // .UpdateSSI recomputes merged segments
                row.message = Some(format!("Merged segments: {label}"));
                let mfi_pos = median(&in_range(min, max));
                row.mfi_pos = Some(mfi_pos);
                row.ssi = finite_or_none(ssi_of(mfi_pos));
            }
        }
    }
}

/// `.MergeSegments`: walk from the top segment down, closing a group once it
/// holds `min_events`; leftover low segments join the last group. Each group
/// lists 0-based segment indices in ascending order.
fn merge_segments(counts: &[usize], min_events: usize) -> Vec<Vec<usize>> {
    let mut groups: Vec<Vec<usize>> = Vec::new();
    let mut buffer = 0;
    let mut current = Vec::new();
    for index in (0..counts.len()).rev() {
        buffer += counts[index];
        current.push(index);
        if buffer >= min_events {
            groups.push(std::mem::take(&mut current));
            buffer = 0;
        }
    }
    if !current.is_empty() {
        match groups.last_mut() {
            Some(last) => last.extend(current),
            None => groups.push(current),
        }
    }
    for group in &mut groups {
        group.sort_unstable();
    }
    groups
}

/// R's `median`; `NA` (NaN) for no values.
fn median(values: &[f64]) -> f64 {
    if values.is_empty() {
        f64::NAN
    } else {
        cytiq_density::median(values)
    }
}

/// R's `sd`: the sample standard deviation; `NA` (NaN) below two values.
fn sample_sd(values: &[f64]) -> f64 {
    if values.len() < 2 {
        return f64::NAN;
    }
    let count = values.len() as f64;
    let mean = values.iter().sum::<f64>() / count;
    let squares = values
        .iter()
        .map(|value| (value - mean) * (value - mean))
        .sum::<f64>();
    (squares / (count - 1.0)).sqrt()
}

/// `NA`/`NaN` SSIs drop out of the matrix (`!is.na(ssi)`); `Inf` stays.
fn finite_or_none(value: f64) -> Option<f64> {
    (!value.is_nan()).then_some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_keeps_segments_that_reach_the_minimum() {
        assert_eq!(
            merge_segments(&[60, 70, 80, 90], 50),
            vec![vec![3], vec![2], vec![1], vec![0]]
        );
    }

    #[test]
    fn merge_closes_groups_from_the_top_segment_down() {
        // i = 4: 60 closes; then 10 + 30 + 100 closes at segment 1
        assert_eq!(
            merge_segments(&[100, 30, 10, 60], 50),
            vec![vec![3], vec![0, 1, 2]]
        );
    }

    #[test]
    fn merge_appends_leftover_low_segments_to_the_last_group() {
        assert_eq!(
            merge_segments(&[10, 100, 20, 20], 50),
            vec![vec![0, 1, 2, 3]]
        );
        assert_eq!(merge_segments(&[10, 5, 5, 5], 50), vec![vec![0, 1, 2, 3]]);
    }

    #[test]
    fn center_counts_break_values_as_upstream_max_does() {
        // two events: max(counts) is 1, but the ceiling(mids) column reaches 5,
        // so the threshold is 0.25 and every non-empty bin qualifies
        let (center, separation) = determine_center(&[vec![0.5, 3.5]]);
        assert_eq!(center, 2.0);
        assert!((separation - 0.2).abs() < 1e-15);
    }

    #[test]
    fn center_uses_right_closed_bins() {
        // breaks -1..5; 4.0 sits in (3, 4], whose upper edge is 4
        let column = vec![0.2; 100]
            .into_iter()
            .chain(vec![4.0; 40])
            .collect::<Vec<_>>();
        let (center, _) = determine_center(&[column]);
        assert_eq!(center, 2.0);
    }

    #[test]
    fn closest_to_center_ignores_na_and_prefers_the_first_tie() {
        assert_eq!(closest_to_center(&[1.0, 3.0, f64::NAN], 2.0), Some(1.0));
        assert_eq!(
            closest_to_center(&[f64::NEG_INFINITY], 2.0),
            Some(f64::NEG_INFINITY)
        );
        assert_eq!(closest_to_center(&[f64::NAN], 2.0), None);
    }

    #[test]
    fn closest_limit_prefers_the_new_cutoff_on_ties() {
        assert_eq!(closest_limit(1.0, 3.0, 2.0), 3.0);
        assert_eq!(closest_limit(1.9, 3.0, 2.0), 1.9);
    }

    #[test]
    fn ssi_status_flags_both_directions_at_one() {
        assert_eq!(SsiStatus::of(Some(-1.0)), SsiStatus::Over);
        assert_eq!(SsiStatus::of(Some(-0.99)), SsiStatus::Pass);
        assert_eq!(SsiStatus::of(Some(1.0)), SsiStatus::Under);
        assert_eq!(SsiStatus::of(None), SsiStatus::NotAssessed);
    }

    #[test]
    fn ssi_rounds_to_two_decimals() {
        assert_eq!(round_ssi(-1.234_567), -1.23);
        assert_eq!(round_ssi(0.125), 0.12);
        assert_eq!(round_ssi(0.135), 0.14);
    }

    #[test]
    fn sample_sd_matches_r() {
        // sd(c(1, 2, 3, 4)) = 1.2909944487358056
        assert!((sample_sd(&[1.0, 2.0, 3.0, 4.0]) - 1.290_994_448_735_805_6).abs() < 1e-15);
    }

    /// Two markers with a known spillover error: B leaks into nothing, A is
    /// over-compensated into B (A-positive events sit below B's negatives).
    fn over_compensated_pair() -> (Vec<MarkerChannel>, Vec<Vec<f64>>) {
        let mut a = Vec::new();
        let mut b = Vec::new();
        for index in 0..3000 {
            let jitter = f64::from((index * 37) % 101) / 101.0 - 0.5;
            let wobble = f64::from((index * 53) % 97) / 97.0 - 0.5;
            let a_positive = index % 3 == 0;
            let b_positive = index % 5 == 0;
            let a_value = if a_positive {
                3.0 + 0.6 * jitter
            } else {
                0.8 + 0.3 * jitter
            };
            let mut b_value = if b_positive {
                3.2 + 0.4 * wobble
            } else {
                0.9 + 0.25 * wobble
            };
            if a_positive && !b_positive {
                // pushed down in proportion to the A signal
                b_value -= 0.25 * (a_value - 0.8);
            }
            a.push(a_value);
            b.push(b_value);
        }
        let channels = vec![
            MarkerChannel {
                channel: "FL1-A".to_owned(),
                marker: "A".to_owned(),
            },
            MarkerChannel {
                channel: "FL2-A".to_owned(),
                marker: "B".to_owned(),
            },
        ];
        (channels, vec![a, b])
    }

    #[test]
    fn flags_over_compensation_into_the_secondary_channel() {
        let (channels, columns) = over_compensated_pair();
        let result = compensaid(&channels, &columns, &CompensaidParameters::default());
        assert_eq!(result.events, 3000);
        assert_eq!(result.pairs.len(), 2);
        // secondary-outer order: (primary B, secondary A) comes first
        assert_eq!((result.pairs[0].primary, result.pairs[0].secondary), (1, 0));
        let a_into_b = result.cell(1, 0).expect("pair exists");
        assert_eq!(a_into_b.status, SsiStatus::Over, "{a_into_b:?}");
        let b_into_a = result.cell(0, 1).expect("pair exists");
        assert_eq!(b_into_a.status, SsiStatus::Pass, "{b_into_a:?}");
    }
}
