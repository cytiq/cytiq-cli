#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::too_many_lines
)]

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use cytiq_fcs::{
    ColumnData, FcsDataset, FcsReader, FcsWriteParameter, Parameter, ParameterDataType,
};
use log::{debug, info};
use rayon::{ThreadPoolBuilder, prelude::*};
use serde::Serialize;

use crate::error::PeacoQcError;
use crate::export::write_debug_artifacts;

pub const DEFAULT_MAD_THRESHOLD: f64 = 6.0;
pub const DEFAULT_IT_LIMIT: f64 = 0.6;
pub const DEFAULT_CONSECUTIVE_BINS: usize = 5;
pub const DEFAULT_MIN_CELLS: usize = 150;
pub const DEFAULT_MAX_BINS: usize = 500;
pub const DEFAULT_STEP: usize = 500;
pub const DEFAULT_FORCE_IT_BINS: usize = 150;
pub const DEFAULT_PEAK_REMOVAL: f64 = 1.0 / 3.0;
pub const DEFAULT_MIN_PEAK_BIN_PERCENT: f64 = 10.0;
pub const DEFAULT_TIME_CHANNEL: &str = "Time";

const PEAK_HISTOGRAM_MIN_BINS: usize = 16;
const PEAK_HISTOGRAM_MAX_BINS: usize = 256;
const MEDIAN_ABSOLUTE_DEVIATION_SCALE: f64 = 1.4826;
const EQUALITY_EPSILON: f64 = 1.0e-12;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PeacoQcMethod {
    #[default]
    All,
    Mad,
    It,
    PeaksOnly,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PeacoQcConfig {
    pub channels: Vec<String>,
    pub method: PeacoQcMethod,
    pub remove_margins: bool,
    pub remove_zeros: bool,
    pub debug_dir: Option<PathBuf>,
    pub threads: Option<usize>,
}

impl PeacoQcConfig {
    #[must_use]
    pub fn new(channels: Vec<String>) -> Self {
        Self {
            channels,
            method: PeacoQcMethod::All,
            remove_margins: false,
            remove_zeros: false,
            debug_dir: None,
            threads: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PeacoQcResult {
    pub summary: PeacoQcSummary,
    pub mask: Vec<EventMaskRecord>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PeacoQcSummary {
    pub tool: String,
    pub version: String,
    pub algorithm: String,
    pub input: String,
    pub output: String,
    pub channels: Vec<String>,
    pub parameters: PeacoQcParameters,
    pub metrics: PeacoQcMetrics,
    pub warnings: Vec<String>,
    pub status: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PeacoQcParameters {
    pub method: PeacoQcMethod,
    pub mad: f64,
    pub it_limit: f64,
    pub consecutive_bins: usize,
    pub min_cells: usize,
    pub max_bins: usize,
    pub step: usize,
    pub force_it: usize,
    pub peak_removal: f64,
    pub min_peak_bin_percent: f64,
    pub remove_zeros: bool,
    pub remove_margins: bool,
    pub time_channel: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PeacoQcMetrics {
    pub events_before: usize,
    pub events_after: usize,
    pub removed_total: usize,
    pub removed_total_fraction: f64,
    pub margin_removed_count: usize,
    pub margin_removed_fraction: f64,
    pub per_channel_margin_counts: Vec<ChannelMarginCounts>,
    pub mad_outlier_bins: Vec<usize>,
    pub mad_removed_event_count: usize,
    pub mad_removed_fraction: f64,
    pub mad_channel_contributions: Vec<MadContribution>,
    pub it_outlier_bins: Vec<usize>,
    pub it_removed_event_count: usize,
    pub it_removed_fraction: f64,
    pub consecutive_removed_bins: Vec<usize>,
    pub consecutive_removed_event_count: usize,
    pub consecutive_removed_fraction: f64,
    pub n_bins: usize,
    pub events_per_bin: usize,
    pub peak_feature_count: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ChannelMarginCounts {
    pub channel: String,
    pub min_count: usize,
    pub max_count: usize,
    pub removed_count: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct MadContribution {
    pub feature: String,
    pub removed_event_count: usize,
    pub removed_fraction: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[allow(clippy::struct_excessive_bools)]
pub struct EventMaskRecord {
    pub event_id: usize,
    pub original_event_id: usize,
    pub bin_id: Option<usize>,
    pub is_good: bool,
    pub removed_by_margin: bool,
    pub removed_by_mad: bool,
    pub removed_by_it: bool,
    pub removed_by_consecutive_bins: bool,
}

#[derive(Clone, Debug)]
struct ResolvedChannel {
    display_name: String,
    parameter: Parameter,
    values: Vec<f64>,
}

#[derive(Clone, Debug)]
struct WorkingData {
    original_indices: Vec<usize>,
    channel_values: Vec<Vec<f64>>,
    margin_removed: Vec<bool>,
    margin_counts: Vec<ChannelMarginCounts>,
}

#[derive(Clone, Debug)]
struct PeakMatrix {
    values: Vec<Vec<f64>>,
    feature_names: Vec<String>,
}

#[derive(Clone, Debug)]
struct MadResult {
    bad_bins: Vec<bool>,
    contributions: Vec<MadContribution>,
}

#[derive(Clone, Debug)]
struct ItNode {
    selection: Vec<bool>,
    depth: usize,
    to_split: bool,
    path_length: Option<f64>,
}

pub struct PeacoQcRunner;

impl PeacoQcRunner {
    /// Run the PeacoQC-compatible pipeline and write the cleaned FCS output.
    ///
    /// # Errors
    ///
    /// Returns an error if the input cannot be read, channels cannot be
    /// resolved, the algorithm cannot produce peak features, or outputs cannot
    /// be written.
    pub fn run(
        input: impl AsRef<Path>,
        output: impl AsRef<Path>,
        config: &PeacoQcConfig,
    ) -> Result<PeacoQcResult, PeacoQcError> {
        let input = input.as_ref();
        let output = output.as_ref();
        with_thread_pool(config.threads, || Self::run_inner(input, output, config))
    }

    fn run_inner(
        input: &Path,
        output: &Path,
        config: &PeacoQcConfig,
    ) -> Result<PeacoQcResult, PeacoQcError> {
        validate_output_path(output)?;
        let dataset = load_dataset(input)?;
        let events_before = event_count(&dataset);
        let channels = resolve_channels(input, &dataset, &config.channels)?;
        let mut warnings = input_warnings(&channels, &dataset);

        info!(
            "peacoqc pipeline: loaded {} events and {} selected channels from {}",
            events_before,
            channels.len(),
            input.display()
        );
        let working = prepare_working_data(&channels, events_before, config.remove_margins);
        if working.original_indices.is_empty() {
            return Err(PeacoQcError::AllEventsRemoved {
                path: input.to_path_buf(),
            });
        }

        let events_per_bin = find_events_per_bin(
            config.remove_zeros,
            &working.channel_values,
            working.original_indices.len(),
        );
        let bins = make_bins(events_per_bin, working.original_indices.len());
        let peak_matrix =
            build_peak_matrix(input, &channels, &working, &bins, config.remove_zeros)?;
        info!(
            "peacoqc pipeline: built {} bins and {} peak features",
            bins.len(),
            peak_matrix.feature_names.len()
        );

        let mut good_bins = vec![true; bins.len()];
        let mut it_bad_bins = vec![false; bins.len()];
        if matches!(config.method, PeacoQcMethod::All | PeacoQcMethod::It) {
            if bins.len() >= DEFAULT_FORCE_IT_BINS {
                let it_good_bins = isolation_tree_good_bins(&peak_matrix.values, DEFAULT_IT_LIMIT);
                for (index, good) in it_good_bins.iter().copied().enumerate() {
                    if !good {
                        it_bad_bins[index] = true;
                    }
                    good_bins[index] = good;
                }
            } else {
                warnings.push(format!(
                    "Isolation Tree skipped: {} bins is below the default force_IT threshold of {}",
                    bins.len(),
                    DEFAULT_FORCE_IT_BINS
                ));
            }
        }

        let mut mad_bad_bins = vec![false; bins.len()];
        let mut mad_contributions = Vec::new();
        if matches!(config.method, PeacoQcMethod::All | PeacoQcMethod::Mad) {
            let mad = mad_bad_bins_for_features(
                &peak_matrix,
                &good_bins,
                &bins,
                working.original_indices.len(),
                events_before,
            );
            for (index, bad) in mad.bad_bins.iter().copied().enumerate() {
                if bad {
                    mad_bad_bins[index] = true;
                    good_bins[index] = false;
                }
            }
            mad_contributions = mad.contributions;
        }

        let pre_consecutive_good_bins = good_bins.clone();
        let mut consecutive_bad_bins = vec![false; bins.len()];
        if !matches!(config.method, PeacoQcMethod::PeaksOnly) {
            good_bins = remove_short_good_regions(&good_bins, DEFAULT_CONSECUTIVE_BINS);
            for (index, (before, after)) in pre_consecutive_good_bins
                .iter()
                .copied()
                .zip(good_bins.iter().copied())
                .enumerate()
            {
                consecutive_bad_bins[index] = before && !after;
            }
        }

        let removed_by_it_working =
            working_event_mask_from_bins(&it_bad_bins, &bins, working.original_indices.len());
        let removed_by_mad_working =
            working_event_mask_from_bins(&mad_bad_bins, &bins, working.original_indices.len());
        let removed_by_consecutive_working = working_event_mask_from_bins(
            &consecutive_bad_bins,
            &bins,
            working.original_indices.len(),
        );

        let mask = build_event_mask(
            events_before,
            &working,
            &bins,
            &removed_by_mad_working,
            &removed_by_it_working,
            &removed_by_consecutive_working,
        );
        write_cleaned_fcs(output, &dataset, &mask)?;

        let summary = build_summary(
            input,
            output,
            config,
            &channels,
            &mask,
            &working,
            &bins,
            events_per_bin,
            &peak_matrix,
            &mad_bad_bins,
            &it_bad_bins,
            &consecutive_bad_bins,
            mad_contributions,
            warnings,
        );
        if let Some(debug_dir) = config.debug_dir.as_deref() {
            write_debug_artifacts(&summary, &mask, debug_dir)?;
        }
        info!(
            "peacoqc pipeline: retained {} of {} events",
            summary.metrics.events_after, summary.metrics.events_before
        );

        Ok(PeacoQcResult { summary, mask })
    }
}

/// Parse a comma-separated list of channel names.
///
/// # Errors
///
/// Returns an error when the list is empty after trimming.
pub fn parse_channel_list(raw: &str) -> Result<Vec<String>, String> {
    let channels = raw
        .split(',')
        .map(str::trim)
        .filter(|channel| !channel.is_empty())
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    if channels.is_empty() {
        Err("channel list must contain at least one channel".to_owned())
    } else {
        Ok(channels)
    }
}

fn validate_output_path(output: &Path) -> Result<(), PeacoQcError> {
    let is_fcs = output
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("fcs"));
    if is_fcs {
        Ok(())
    } else {
        Err(PeacoQcError::UnsupportedOutputFormat {
            path: output.to_path_buf(),
        })
    }
}

fn load_dataset(path: &Path) -> Result<FcsDataset, PeacoQcError> {
    if !path.is_file() {
        return Err(PeacoQcError::MissingInputFile {
            path: path.to_path_buf(),
        });
    }
    debug!("loading FCS file {}", path.display());
    let dataset = FcsReader::open(path)
        .and_then(FcsReader::read)
        .map_err(|source| PeacoQcError::FcsRead {
            path: path.to_path_buf(),
            source,
        })?;
    if event_count(&dataset) == 0 {
        return Err(PeacoQcError::EmptyInput {
            path: path.to_path_buf(),
        });
    }

    Ok(dataset)
}

fn resolve_channels(
    path: &Path,
    dataset: &FcsDataset,
    requested_channels: &[String],
) -> Result<Vec<ResolvedChannel>, PeacoQcError> {
    if requested_channels.is_empty() {
        return Err(PeacoQcError::EmptyChannels);
    }

    let mut seen = HashSet::new();
    let mut channels = Vec::with_capacity(requested_channels.len());
    for channel in requested_channels {
        let Some(parameter_index) = find_parameter_index(&dataset.parameters, channel) else {
            return Err(PeacoQcError::MissingChannel {
                path: path.to_path_buf(),
                channel: channel.clone(),
                available_channels: dataset
                    .parameters
                    .iter()
                    .map(display_parameter_identifier)
                    .collect(),
            });
        };
        let parameter = dataset.parameters[parameter_index].clone();
        if !seen.insert(parameter.name.clone()) {
            return Err(PeacoQcError::DuplicateChannel {
                channel: parameter.name,
            });
        }
        let values = column_to_f64(
            dataset
                .data
                .column(parameter_index)
                .expect("parameter index should have a data column"),
        );
        if values.iter().any(|value| !value.is_finite()) {
            return Err(PeacoQcError::NonFiniteChannel {
                path: path.to_path_buf(),
                channel: channel.clone(),
            });
        }
        channels.push(ResolvedChannel {
            display_name: parameter.name.clone(),
            parameter,
            values,
        });
    }

    Ok(channels)
}

fn input_warnings(channels: &[ResolvedChannel], dataset: &FcsDataset) -> Vec<String> {
    let has_spill_keyword = dataset
        .text
        .iter()
        .any(|(key, _)| key.eq_ignore_ascii_case("$SPILL") || key.eq_ignore_ascii_case("SPILL"));
    let integer_selected = channels
        .iter()
        .any(|channel| channel.parameter.data_type == ParameterDataType::Integer);
    if has_spill_keyword && integer_selected {
        vec![
            "Selected channels look like integer-scale FCS data with spillover metadata; PeacoQC expects compensated/transformed input".to_owned(),
        ]
    } else {
        Vec::new()
    }
}

fn prepare_working_data(
    channels: &[ResolvedChannel],
    event_count: usize,
    remove_margins: bool,
) -> WorkingData {
    let mut margin_removed = vec![false; event_count];
    let mut margin_counts = Vec::new();

    if remove_margins {
        for channel in channels {
            let (channel_removed, counts) = margin_events_for_channel(channel);
            for (target, removed) in margin_removed.iter_mut().zip(channel_removed) {
                *target = *target || removed;
            }
            margin_counts.push(counts);
        }
    }

    let original_indices = margin_removed
        .iter()
        .enumerate()
        .filter_map(|(index, removed)| (!removed).then_some(index))
        .collect::<Vec<_>>();
    let channel_values = channels
        .iter()
        .map(|channel| {
            original_indices
                .iter()
                .map(|&index| channel.values[index])
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();

    WorkingData {
        original_indices,
        channel_values,
        margin_removed,
        margin_counts,
    }
}

fn margin_events_for_channel(channel: &ResolvedChannel) -> (Vec<bool>, ChannelMarginCounts) {
    let minimum = channel.values.iter().copied().fold(f64::INFINITY, f64::min);
    let maximum = channel
        .values
        .iter()
        .copied()
        .fold(f64::NEG_INFINITY, f64::max);
    let min_range = parameter_min_range(&channel.parameter).unwrap_or(0.0);
    let max_range = parameter_max_range(&channel.parameter).unwrap_or(maximum);
    let min_threshold = min_range.max(0.0).max(minimum);
    let max_threshold = max_range.min(maximum);

    let min_flags = channel
        .values
        .iter()
        .map(|value| *value <= min_threshold)
        .collect::<Vec<_>>();
    let max_flags = channel
        .values
        .iter()
        .map(|value| *value >= max_threshold)
        .collect::<Vec<_>>();
    let removed = min_flags
        .iter()
        .zip(&max_flags)
        .map(|(min_removed, max_removed)| *min_removed || *max_removed)
        .collect::<Vec<_>>();
    let counts = ChannelMarginCounts {
        channel: channel.display_name.clone(),
        min_count: min_flags.iter().filter(|removed| **removed).count(),
        max_count: max_flags.iter().filter(|removed| **removed).count(),
        removed_count: removed.iter().filter(|removed| **removed).count(),
    };

    (removed, counts)
}

fn parameter_min_range(parameter: &Parameter) -> Option<f64> {
    keyword_value(parameter, "minRange")
        .or_else(|| keyword_value(parameter, "min"))
        .and_then(|value| value.trim().parse::<f64>().ok())
}

fn parameter_max_range(parameter: &Parameter) -> Option<f64> {
    keyword_value(parameter, &format!("$P{}R", parameter.index))
        .or_else(|| keyword_value(parameter, "maxRange"))
        .or_else(|| keyword_value(parameter, "max"))
        .and_then(|value| value.trim().parse::<f64>().ok())
}

fn keyword_value<'a>(parameter: &'a Parameter, key: &str) -> Option<&'a str> {
    parameter
        .keywords
        .iter()
        .find_map(|(candidate, value)| (candidate == key).then_some(value.as_str()))
}

fn find_events_per_bin(
    remove_zeros: bool,
    channel_values: &[Vec<f64>],
    event_count: usize,
) -> usize {
    let mut max_bins = DEFAULT_MAX_BINS.max(1);
    if remove_zeros {
        let min_non_zero = channel_values
            .iter()
            .map(|values| {
                values
                    .iter()
                    .filter(|value| value.abs() > EQUALITY_EPSILON)
                    .count()
            })
            .min()
            .unwrap_or(event_count);
        max_bins = max_bins.min((min_non_zero / DEFAULT_MIN_CELLS).max(1));
    }

    let raw_max_cells = ((event_count as f64 / max_bins as f64) * 2.0).ceil() as usize;
    let rounded_max_cells = ((raw_max_cells / DEFAULT_STEP) * DEFAULT_STEP) + DEFAULT_STEP;
    DEFAULT_MIN_CELLS.max(rounded_max_cells)
}

fn make_bins(events_per_bin: usize, event_count: usize) -> Vec<Vec<usize>> {
    if event_count == 0 {
        return Vec::new();
    }
    let overlap = events_per_bin.div_ceil(2);
    let step = events_per_bin.saturating_sub(overlap).max(1);
    let mut bins = Vec::new();
    let mut start = 0;
    while start < event_count {
        let end = start.saturating_add(events_per_bin).min(event_count);
        bins.push((start..end).collect());
        if end == event_count {
            break;
        }
        start = start.saturating_add(step);
    }

    bins
}

fn build_peak_matrix(
    input: &Path,
    channels: &[ResolvedChannel],
    working: &WorkingData,
    bins: &[Vec<usize>],
    remove_zeros: bool,
) -> Result<PeakMatrix, PeacoQcError> {
    let channel_features = channels
        .par_iter()
        .enumerate()
        .filter_map(|(index, channel)| {
            build_channel_peak_features(
                &channel.display_name,
                &working.channel_values[index],
                bins,
                remove_zeros,
            )
        })
        .collect::<Vec<_>>();

    let feature_count = channel_features
        .iter()
        .map(|features| features.names.len())
        .sum::<usize>();
    if feature_count == 0 {
        return Err(PeacoQcError::NoPeakFeatures {
            path: input.to_path_buf(),
        });
    }

    let mut values = vec![Vec::with_capacity(feature_count); bins.len()];
    let mut feature_names = Vec::with_capacity(feature_count);
    for features in channel_features {
        feature_names.extend(features.names);
        for (row, row_values) in values.iter_mut().zip(features.values) {
            row.extend(row_values);
        }
    }

    Ok(PeakMatrix {
        values,
        feature_names,
    })
}

#[derive(Clone, Debug)]
struct ChannelPeakFeatures {
    names: Vec<String>,
    values: Vec<Vec<f64>>,
}

fn build_channel_peak_features(
    channel_name: &str,
    values: &[f64],
    bins: &[Vec<usize>],
    remove_zeros: bool,
) -> Option<ChannelPeakFeatures> {
    let full_peaks = find_peaks(values, remove_zeros, DEFAULT_PEAK_REMOVAL);
    if full_peaks.is_empty() {
        return None;
    }

    let peaks_by_bin = bins
        .iter()
        .map(|bin| {
            let bin_values = bin.iter().map(|&index| values[index]).collect::<Vec<_>>();
            find_peaks(&bin_values, remove_zeros, DEFAULT_PEAK_REMOVAL)
        })
        .collect::<Vec<_>>();
    let non_empty = peaks_by_bin
        .iter()
        .filter(|peaks| !peaks.is_empty())
        .collect::<Vec<_>>();
    if non_empty.is_empty() {
        return None;
    }

    let mut count_frequencies = HashMap::<usize, usize>::new();
    for peaks in &non_empty {
        *count_frequencies.entry(peaks.len()).or_default() += 1;
    }
    let min_bins = (DEFAULT_MIN_PEAK_BIN_PERCENT / 100.0) * (non_empty.len() as f64);
    let target_peak_count = count_frequencies
        .into_iter()
        .filter(|(_, frequency)| (*frequency as f64) > min_bins)
        .map(|(count, _)| count)
        .max()
        .unwrap_or(1);
    if target_peak_count == 0 {
        return None;
    }

    let mut centers = Vec::with_capacity(target_peak_count);
    for peak_index in 0..target_peak_count {
        let values_for_peak = non_empty
            .iter()
            .filter(|peaks| peaks.len() == target_peak_count)
            .map(|peaks| peaks[peak_index])
            .collect::<Vec<_>>();
        centers.push(median(values_for_peak));
    }

    let assigned = assign_peak_clusters(&peaks_by_bin, &centers);
    let kept_clusters = kept_peak_clusters(&assigned, bins.len(), centers.len());
    if kept_clusters.is_empty() {
        return None;
    }

    let mut names = Vec::with_capacity(kept_clusters.len());
    let mut columns = Vec::with_capacity(kept_clusters.len());
    for cluster in kept_clusters {
        let observed = assigned
            .iter()
            .filter_map(|bin| {
                bin.iter()
                    .find_map(|(candidate, value)| (*candidate == cluster).then_some(*value))
            })
            .collect::<Vec<_>>();
        let fallback = median(observed);
        let column = assigned
            .iter()
            .map(|bin| {
                bin.iter()
                    .find_map(|(candidate, value)| (*candidate == cluster).then_some(*value))
                    .unwrap_or(fallback)
            })
            .collect::<Vec<_>>();
        names.push(format!("{}:peak{}", channel_name, cluster + 1));
        columns.push(column);
    }

    let mut rows = vec![Vec::with_capacity(columns.len()); bins.len()];
    for column in columns {
        for (row, value) in rows.iter_mut().zip(column) {
            row.push(value);
        }
    }

    Some(ChannelPeakFeatures {
        names,
        values: rows,
    })
}

fn find_peaks(values: &[f64], remove_zeros: bool, peak_removal: f64) -> Vec<f64> {
    let mut data = values
        .iter()
        .copied()
        .filter(|value| value.is_finite())
        .filter(|value| !remove_zeros || value.abs() > EQUALITY_EPSILON)
        .collect::<Vec<_>>();
    if data.len() < 3 {
        return Vec::new();
    }
    data.sort_by(f64::total_cmp);
    let minimum = data[0];
    let maximum = data[data.len() - 1];
    if (maximum - minimum).abs() <= EQUALITY_EPSILON {
        return vec![minimum];
    }

    let bin_count = (((data.len() as f64).sqrt().ceil() as usize) * 4)
        .clamp(PEAK_HISTOGRAM_MIN_BINS, PEAK_HISTOGRAM_MAX_BINS);
    let width = (maximum - minimum) / bin_count as f64;
    let mut histogram = vec![0.0_f64; bin_count];
    for value in data {
        let mut index = ((value - minimum) / width).floor() as usize;
        if index >= bin_count {
            index = bin_count - 1;
        }
        histogram[index] += 1.0;
    }
    let smoothed = smooth_histogram(&histogram);
    let max_height = smoothed.iter().copied().fold(0.0, f64::max);
    if max_height <= EQUALITY_EPSILON {
        return Vec::new();
    }

    let mut peaks = Vec::new();
    for index in 1..(smoothed.len() - 1) {
        if smoothed[index] > smoothed[index - 1]
            && smoothed[index] > smoothed[index + 1]
            && smoothed[index] > peak_removal * max_height
        {
            peaks.push(minimum + (index as f64 + 0.5) * width);
        }
    }
    if peaks.is_empty() {
        let max_index = smoothed
            .iter()
            .enumerate()
            .max_by(|(_, left), (_, right)| left.total_cmp(right))
            .map_or(0, |(index, _)| index);
        peaks.push(minimum + (max_index as f64 + 0.5) * width);
    }

    peaks
}

fn smooth_histogram(histogram: &[f64]) -> Vec<f64> {
    let mut current = histogram.to_vec();
    for _ in 0..2 {
        let mut next = current.clone();
        for index in 0..current.len() {
            let previous = index.checked_sub(1).map_or(current[index], |i| current[i]);
            let following = current.get(index + 1).copied().unwrap_or(current[index]);
            next[index] = (previous + (2.0 * current[index]) + following) / 4.0;
        }
        current = next;
    }

    current
}

fn assign_peak_clusters(peaks_by_bin: &[Vec<f64>], centers: &[f64]) -> Vec<Vec<(usize, f64)>> {
    peaks_by_bin
        .iter()
        .map(|peaks| {
            let mut by_cluster = HashMap::<usize, f64>::new();
            for peak in peaks {
                let cluster = centers
                    .iter()
                    .enumerate()
                    .min_by(|(_, left), (_, right)| {
                        ((*peak - **left).abs()).total_cmp(&(*peak - **right).abs())
                    })
                    .map_or(0, |(index, _)| index);
                by_cluster
                    .entry(cluster)
                    .and_modify(|existing| {
                        let current_distance = (*peak - centers[cluster]).abs();
                        let existing_distance = (*existing - centers[cluster]).abs();
                        if current_distance < existing_distance {
                            *existing = *peak;
                        }
                    })
                    .or_insert(*peak);
            }
            let mut assigned = by_cluster.into_iter().collect::<Vec<_>>();
            assigned.sort_by_key(|(cluster, _)| *cluster);
            assigned
        })
        .collect()
}

fn kept_peak_clusters(
    assigned: &[Vec<(usize, f64)>],
    bin_count: usize,
    cluster_count: usize,
) -> Vec<usize> {
    let mut counts = vec![0_usize; cluster_count];
    for bin in assigned {
        for (cluster, _) in bin {
            counts[*cluster] += 1;
        }
    }
    counts
        .into_iter()
        .enumerate()
        .filter_map(|(cluster, count)| (count * 2 >= bin_count).then_some(cluster))
        .collect()
}

fn isolation_tree_good_bins(matrix: &[Vec<f64>], gain_limit: f64) -> Vec<bool> {
    if matrix.is_empty() {
        return Vec::new();
    }
    let max_depth = (matrix.len() as f64).log2().ceil() as usize;
    let mut current_gain_limit = gain_limit;
    let mut nodes = vec![ItNode {
        selection: vec![true; matrix.len()],
        depth: 0,
        to_split: true,
        path_length: None,
    }];

    while let Some(node_index) = nodes.iter().position(|node| node.to_split) {
        let rows = nodes[node_index]
            .selection
            .iter()
            .enumerate()
            .filter_map(|(index, selected)| selected.then_some(index))
            .collect::<Vec<_>>();
        if rows.len() > 3 && nodes[node_index].depth < max_depth {
            if let Some(split) = best_isolation_split(matrix, &rows, current_gain_limit) {
                let left = nodes[node_index]
                    .selection
                    .iter()
                    .enumerate()
                    .map(|(row, selected)| *selected && matrix[row][split.column] <= split.value)
                    .collect::<Vec<_>>();
                let right = nodes[node_index]
                    .selection
                    .iter()
                    .enumerate()
                    .map(|(row, selected)| *selected && matrix[row][split.column] > split.value)
                    .collect::<Vec<_>>();
                let left_count = left.iter().filter(|selected| **selected).count();
                let right_count = right.iter().filter(|selected| **selected).count();
                if left_count == 0 || right_count == 0 {
                    close_it_node(&mut nodes[node_index], rows.len());
                    continue;
                }

                nodes[node_index].to_split = false;
                current_gain_limit = split.gain;
                let child_depth = nodes[node_index].depth + 1;
                nodes.push(ItNode {
                    selection: left,
                    depth: child_depth,
                    to_split: true,
                    path_length: None,
                });
                nodes.push(ItNode {
                    selection: right,
                    depth: child_depth,
                    to_split: true,
                    path_length: None,
                });
            } else {
                close_it_node(&mut nodes[node_index], rows.len());
            }
        } else {
            close_it_node(&mut nodes[node_index], rows.len());
        }
    }

    nodes
        .iter()
        .filter(|node| node.path_length.is_some())
        .max_by_key(|node| node.selection.iter().filter(|selected| **selected).count())
        .map_or_else(|| vec![true; matrix.len()], |node| node.selection.clone())
}

#[derive(Clone, Copy, Debug)]
struct IsolationSplit {
    column: usize,
    value: f64,
    gain: f64,
}

#[allow(clippy::needless_range_loop)]
fn best_isolation_split(
    matrix: &[Vec<f64>],
    rows: &[usize],
    gain_limit: f64,
) -> Option<IsolationSplit> {
    let column_count = matrix.first().map_or(0, Vec::len);
    let mut best = None;
    for column in 0..column_count {
        let mut values = rows
            .iter()
            .map(|&row| matrix[row][column])
            .collect::<Vec<_>>();
        values.sort_by(f64::total_cmp);
        let base_sd = sample_standard_deviation(&values);
        if base_sd <= EQUALITY_EPSILON {
            continue;
        }

        let mut best_col_gain = 0.0;
        let mut best_col_value = None;
        for split_index in 1..values.len() {
            let left_sd = if split_index == 1 {
                0.0
            } else {
                sample_standard_deviation(&values[..split_index])
            };
            let right_sd = if split_index == values.len() - 1 {
                0.0
            } else {
                sample_standard_deviation(&values[split_index..])
            };
            let gain = (base_sd - f64::midpoint(left_sd, right_sd)) / base_sd;
            if gain.is_finite() && gain >= best_col_gain {
                best_col_gain = gain;
                best_col_value = Some(values[split_index - 1]);
            }
        }
        if best_col_gain > gain_limit {
            best = Some(IsolationSplit {
                column,
                value: best_col_value.unwrap_or(values[0]),
                gain: best_col_gain,
            });
        }
    }

    best
}

fn close_it_node(node: &mut ItNode, selected_count: usize) {
    node.to_split = false;
    node.path_length = Some(avg_path_length(selected_count) + node.depth as f64);
}

fn avg_path_length(n_datapoints: usize) -> f64 {
    if n_datapoints <= 1 {
        0.0
    } else {
        let n = n_datapoints as f64;
        2.0 * ((n - 1.0).ln() + 0.577_215_664_9) - (2.0 * (n - 1.0)) / n
    }
}

fn mad_bad_bins_for_features(
    matrix: &PeakMatrix,
    candidate_good_bins: &[bool],
    bins: &[Vec<usize>],
    working_event_count: usize,
    denominator_event_count: usize,
) -> MadResult {
    let candidate_rows = candidate_good_bins
        .iter()
        .enumerate()
        .filter_map(|(index, good)| good.then_some(index))
        .collect::<Vec<_>>();
    let mut bad_bins = vec![false; matrix.values.len()];
    let mut contributions = Vec::new();
    if candidate_rows.len() < 3 {
        return MadResult {
            bad_bins,
            contributions,
        };
    }

    for feature_index in 0..matrix.feature_names.len() {
        let series = candidate_rows
            .iter()
            .map(|&row| matrix.values[row][feature_index])
            .collect::<Vec<_>>();
        let smoothed = smooth_series(&series);
        let median_value = median(smoothed.clone());
        let mad_value = median_absolute_deviation(&smoothed);
        let mut feature_bad_bins = vec![false; matrix.values.len()];
        let lower = median_value - (DEFAULT_MAD_THRESHOLD * mad_value);
        let upper = median_value + (DEFAULT_MAD_THRESHOLD * mad_value);
        for (&row, value) in candidate_rows.iter().zip(smoothed) {
            let outlier = if mad_value <= EQUALITY_EPSILON {
                (value - median_value).abs() > EQUALITY_EPSILON
            } else {
                value < lower || value > upper
            };
            if outlier {
                bad_bins[row] = true;
                feature_bad_bins[row] = true;
            }
        }

        let removed_event_count =
            count_working_events_from_bins(&feature_bad_bins, bins, working_event_count);
        if removed_event_count > 0 {
            contributions.push(MadContribution {
                feature: matrix.feature_names[feature_index].clone(),
                removed_event_count,
                removed_fraction: fraction(removed_event_count, denominator_event_count),
            });
        }
    }

    MadResult {
        bad_bins,
        contributions,
    }
}

fn smooth_series(values: &[f64]) -> Vec<f64> {
    (0..values.len())
        .map(|index| {
            let start = index.saturating_sub(2);
            let end = (index + 3).min(values.len());
            mean(&values[start..end])
        })
        .collect()
}

fn remove_short_good_regions(good_bins: &[bool], consecutive_bins: usize) -> Vec<bool> {
    let mut output = good_bins.to_vec();
    let mut start = 0;
    while start < good_bins.len() {
        let value = good_bins[start];
        let mut end = start + 1;
        while end < good_bins.len() && good_bins[end] == value {
            end += 1;
        }
        let bounded_by_bad_bins =
            start > 0 && end < good_bins.len() && !good_bins[start - 1] && !good_bins[end];
        if value && end - start < consecutive_bins && bounded_by_bad_bins {
            output[start..end].fill(false);
        }
        start = end;
    }

    output
}

fn working_event_mask_from_bins(
    bad_bins: &[bool],
    bins: &[Vec<usize>],
    working_event_count: usize,
) -> Vec<bool> {
    let mut mask = vec![false; working_event_count];
    for (bad, bin) in bad_bins.iter().zip(bins) {
        if *bad {
            for &event_index in bin {
                mask[event_index] = true;
            }
        }
    }

    mask
}

fn build_event_mask(
    original_event_count: usize,
    working: &WorkingData,
    bins: &[Vec<usize>],
    removed_by_mad_working: &[bool],
    removed_by_it_working: &[bool],
    removed_by_consecutive_working: &[bool],
) -> Vec<EventMaskRecord> {
    let primary_bin_ids = primary_bin_ids(bins, working.original_indices.len());
    let mut records = (0..original_event_count)
        .map(|index| EventMaskRecord {
            event_id: index + 1,
            original_event_id: index + 1,
            bin_id: None,
            is_good: !working.margin_removed[index],
            removed_by_margin: working.margin_removed[index],
            removed_by_mad: false,
            removed_by_it: false,
            removed_by_consecutive_bins: false,
        })
        .collect::<Vec<_>>();

    for (working_index, &original_index) in working.original_indices.iter().enumerate() {
        let record = &mut records[original_index];
        record.bin_id = primary_bin_ids[working_index];
        record.removed_by_mad = removed_by_mad_working[working_index];
        record.removed_by_it = removed_by_it_working[working_index];
        record.removed_by_consecutive_bins = removed_by_consecutive_working[working_index];
        record.is_good = !(record.removed_by_margin
            || record.removed_by_mad
            || record.removed_by_it
            || record.removed_by_consecutive_bins);
    }

    records
}

fn primary_bin_ids(bins: &[Vec<usize>], event_count: usize) -> Vec<Option<usize>> {
    let mut ids = vec![None; event_count];
    for (bin_index, bin) in bins.iter().enumerate() {
        for &event_index in bin {
            ids[event_index].get_or_insert(bin_index + 1);
        }
    }

    ids
}

fn write_cleaned_fcs(
    output: &Path,
    dataset: &FcsDataset,
    mask: &[EventMaskRecord],
) -> Result<(), PeacoQcError> {
    let retained = mask
        .iter()
        .enumerate()
        .filter_map(|(index, record)| record.is_good.then_some(index))
        .collect::<Vec<_>>();
    if retained.is_empty() {
        return Err(PeacoQcError::AllEventsRemoved {
            path: output.to_path_buf(),
        });
    }

    let mut parameters = dataset
        .parameters
        .iter()
        .map(|parameter| match parameter.label.as_ref() {
            Some(label) => FcsWriteParameter::with_label(parameter.name.clone(), label.clone()),
            None => FcsWriteParameter::new(parameter.name.clone()),
        })
        .collect::<Vec<_>>();
    let mut columns = dataset
        .data
        .columns
        .iter()
        .map(|column| {
            retained
                .iter()
                .map(|&index| column_value_to_f32(column, index))
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();

    if !dataset
        .parameters
        .iter()
        .any(|parameter| parameter.name == "Original_ID")
    {
        parameters.push(FcsWriteParameter::new("Original_ID"));
        columns.push(
            retained
                .iter()
                .map(|&index| (index + 1) as f32)
                .collect::<Vec<_>>(),
        );
    }
    if columns
        .iter()
        .flat_map(|column| column.iter())
        .any(|value| !value.is_finite())
    {
        return Err(PeacoQcError::NonFiniteOutputValues {
            path: output.to_path_buf(),
        });
    }

    cytiq_fcs::write_fcs3_1_f32(output, &parameters, &columns).map_err(|source| {
        PeacoQcError::FcsWrite {
            path: output.to_path_buf(),
            source,
        }
    })
}

#[allow(clippy::too_many_arguments)]
fn build_summary(
    input: &Path,
    output: &Path,
    config: &PeacoQcConfig,
    channels: &[ResolvedChannel],
    mask: &[EventMaskRecord],
    working: &WorkingData,
    bins: &[Vec<usize>],
    events_per_bin: usize,
    peak_matrix: &PeakMatrix,
    mad_bad_bins: &[bool],
    it_bad_bins: &[bool],
    consecutive_bad_bins: &[bool],
    mad_contributions: Vec<MadContribution>,
    warnings: Vec<String>,
) -> PeacoQcSummary {
    let events_before = mask.len();
    let events_after = mask.iter().filter(|record| record.is_good).count();
    let removed_total = events_before - events_after;
    let margin_removed_count = mask
        .iter()
        .filter(|record| record.removed_by_margin)
        .count();
    let mad_removed_event_count = mask.iter().filter(|record| record.removed_by_mad).count();
    let it_removed_event_count = mask.iter().filter(|record| record.removed_by_it).count();
    let consecutive_removed_event_count = mask
        .iter()
        .filter(|record| record.removed_by_consecutive_bins)
        .count();
    let metrics = PeacoQcMetrics {
        events_before,
        events_after,
        removed_total,
        removed_total_fraction: fraction(removed_total, events_before),
        margin_removed_count,
        margin_removed_fraction: fraction(margin_removed_count, events_before),
        per_channel_margin_counts: working.margin_counts.clone(),
        mad_outlier_bins: one_based_bin_indexes(mad_bad_bins),
        mad_removed_event_count,
        mad_removed_fraction: fraction(mad_removed_event_count, events_before),
        mad_channel_contributions: mad_contributions,
        it_outlier_bins: one_based_bin_indexes(it_bad_bins),
        it_removed_event_count,
        it_removed_fraction: fraction(it_removed_event_count, events_before),
        consecutive_removed_bins: one_based_bin_indexes(consecutive_bad_bins),
        consecutive_removed_event_count,
        consecutive_removed_fraction: fraction(consecutive_removed_event_count, events_before),
        n_bins: bins.len(),
        events_per_bin,
        peak_feature_count: peak_matrix.feature_names.len(),
    };

    PeacoQcSummary {
        tool: "cytiq peacoqc".to_owned(),
        version: env!("CARGO_PKG_VERSION").to_owned(),
        algorithm: "PeacoQC-compatible MVP".to_owned(),
        input: input.display().to_string(),
        output: output.display().to_string(),
        channels: channels
            .iter()
            .map(|channel| channel.display_name.clone())
            .collect(),
        parameters: PeacoQcParameters {
            method: config.method,
            mad: DEFAULT_MAD_THRESHOLD,
            it_limit: DEFAULT_IT_LIMIT,
            consecutive_bins: DEFAULT_CONSECUTIVE_BINS,
            min_cells: DEFAULT_MIN_CELLS,
            max_bins: DEFAULT_MAX_BINS,
            step: DEFAULT_STEP,
            force_it: DEFAULT_FORCE_IT_BINS,
            peak_removal: DEFAULT_PEAK_REMOVAL,
            min_peak_bin_percent: DEFAULT_MIN_PEAK_BIN_PERCENT,
            remove_zeros: config.remove_zeros,
            remove_margins: config.remove_margins,
            time_channel: DEFAULT_TIME_CHANNEL.to_owned(),
        },
        metrics,
        warnings,
        status: "PASS".to_owned(),
    }
}

fn one_based_bin_indexes(bad_bins: &[bool]) -> Vec<usize> {
    bad_bins
        .iter()
        .enumerate()
        .filter_map(|(index, bad)| bad.then_some(index + 1))
        .collect()
}

fn count_working_events_from_bins(
    bad_bins: &[bool],
    bins: &[Vec<usize>],
    working_event_count: usize,
) -> usize {
    working_event_mask_from_bins(bad_bins, bins, working_event_count)
        .iter()
        .filter(|removed| **removed)
        .count()
}

fn find_parameter_index(parameters: &[Parameter], channel: &str) -> Option<usize> {
    parameters
        .iter()
        .position(|parameter| parameter.name == channel)
        .or_else(|| {
            parameters.iter().position(|parameter| {
                parameter
                    .label
                    .as_deref()
                    .is_some_and(|label| label == channel)
            })
        })
        .or_else(|| {
            parameters
                .iter()
                .position(|parameter| display_parameter_identifier(parameter) == channel)
        })
}

fn display_parameter_identifier(parameter: &Parameter) -> String {
    match parameter.label.as_deref() {
        Some(label) if label != parameter.name => format!("{} [{}]", parameter.name, label),
        _ => parameter.name.clone(),
    }
}

fn event_count(dataset: &FcsDataset) -> usize {
    dataset.data.columns.first().map_or(0, ColumnData::len)
}

fn column_to_f64(column: &ColumnData) -> Vec<f64> {
    match column {
        ColumnData::UInt(values) => values.iter().map(|value| *value as f64).collect(),
        ColumnData::F32(values) => values.iter().map(|value| f64::from(*value)).collect(),
        ColumnData::F64(values) => values.clone(),
    }
}

fn column_value_to_f32(column: &ColumnData, index: usize) -> f32 {
    match column {
        ColumnData::UInt(values) => values[index] as f32,
        ColumnData::F32(values) => values[index],
        ColumnData::F64(values) => values[index] as f32,
    }
}

fn sample_standard_deviation(values: &[f64]) -> f64 {
    if values.len() < 2 {
        return 0.0;
    }
    let average = mean(values);
    let variance = values
        .iter()
        .map(|value| {
            let delta = value - average;
            delta * delta
        })
        .sum::<f64>()
        / ((values.len() - 1) as f64);
    variance.sqrt()
}

fn mean(values: &[f64]) -> f64 {
    if values.is_empty() {
        0.0
    } else {
        values.iter().sum::<f64>() / values.len() as f64
    }
}

fn median(mut values: Vec<f64>) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.sort_by(f64::total_cmp);
    let middle = values.len() / 2;
    if values.len().is_multiple_of(2) {
        f64::midpoint(values[middle - 1], values[middle])
    } else {
        values[middle]
    }
}

fn median_absolute_deviation(values: &[f64]) -> f64 {
    let median_value = median(values.to_vec());
    let deviations = values
        .iter()
        .map(|value| (value - median_value).abs())
        .collect::<Vec<_>>();
    median(deviations) * MEDIAN_ABSOLUTE_DEVIATION_SCALE
}

fn fraction(count: usize, denominator: usize) -> f64 {
    if denominator == 0 {
        0.0
    } else {
        count as f64 / denominator as f64
    }
}

fn with_thread_pool<T, F>(threads: Option<usize>, operation: F) -> Result<T, PeacoQcError>
where
    T: Send,
    F: FnOnce() -> Result<T, PeacoQcError> + Send,
{
    let Some(threads) = threads else {
        return operation();
    };

    debug!("using local rayon thread pool with {threads} threads");
    ThreadPoolBuilder::new()
        .num_threads(threads)
        .build()
        .map_err(|source| PeacoQcError::ThreadPoolBuild { threads, source })?
        .install(operation)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_comma_separated_channels() {
        let channels = parse_channel_list("CD3, CD4,,CD8").expect("channels should parse");
        assert_eq!(channels, vec!["CD3", "CD4", "CD8"]);
    }

    #[test]
    fn computes_overlapping_bins() {
        let bins = make_bins(6, 14);
        assert_eq!(
            bins,
            vec![
                vec![0, 1, 2, 3, 4, 5],
                vec![3, 4, 5, 6, 7, 8],
                vec![6, 7, 8, 9, 10, 11],
                vec![9, 10, 11, 12, 13]
            ]
        );
    }

    #[test]
    fn removes_short_good_regions() {
        let cleaned = remove_short_good_regions(
            &[
                false, false, true, true, false, true, true, true, true, true,
            ],
            5,
        );
        assert_eq!(
            cleaned,
            vec![
                false, false, false, false, false, true, true, true, true, true
            ]
        );
    }

    #[test]
    fn detects_histogram_peaks() {
        let values = [1.0, 1.1, 1.2, 1.1, 10.0, 10.1, 10.2, 10.1, 10.0];
        let peaks = find_peaks(&values, false, DEFAULT_PEAK_REMOVAL);
        assert!(!peaks.is_empty());
    }

    #[test]
    fn mad_detects_feature_outlier() {
        let matrix = PeakMatrix {
            values: (0..13)
                .map(|index| if index == 6 { vec![1_000.0] } else { vec![1.0] })
                .collect(),
            feature_names: vec!["CD3:peak1".to_owned()],
        };
        let bins = (0..13).map(|index| vec![index]).collect::<Vec<_>>();
        let result = mad_bad_bins_for_features(&matrix, &[true; 13], &bins, 13, 13);
        assert!(result.bad_bins[6]);
    }
}
