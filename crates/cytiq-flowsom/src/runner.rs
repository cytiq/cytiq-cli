#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::module_name_repetitions,
    clippy::similar_names,
    clippy::too_many_lines
)]

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use cytiq_fcs::{ColumnData, FcsDataset, FcsReader, Parameter};
use log::{debug, info};
use rayon::{ThreadPoolBuilder, prelude::*};
use serde::Serialize;

use crate::error::FlowSomError;
use crate::export::{write_assignments_csv, write_debug_artifacts};

pub const FLOWSOM_XDIM: usize = 10;
pub const FLOWSOM_YDIM: usize = 10;
pub const FLOWSOM_METACLUSTERS: usize = 20;
pub const FLOWSOM_TRAINING_ITERATIONS: usize = 10;
pub const FLOWSOM_LEARNING_RATE_START: f64 = 0.05;
pub const FLOWSOM_LEARNING_RATE_END: f64 = 0.01;
/// Quantile of the Chebyshev grid-distance matrix used as the starting radius,
/// matching `FlowSOM::SOM(radius = stats::quantile(nhbrdist, 0.67) * c(1, 0))`.
pub const FLOWSOM_RADIUS_START_QUANTILE: f64 = 0.67;
pub const FLOWSOM_RADIUS_END: f64 = 0.0;
pub const FLOWSOM_RANDOM_SEED: u64 = 1;
pub const FLOWSOM_SCALE_INPUT: bool = true;

const EQUALITY_EPSILON: f64 = 1.0e-12;

#[derive(Clone, Debug, PartialEq)]
pub struct FlowSomConfig {
    pub channels: Vec<String>,
    pub debug_dir: Option<PathBuf>,
    pub threads: Option<usize>,
    /// Event masks (`masks.parquet`); only events every mask keeps are
    /// scaled, trained on and assigned.
    pub masks: Vec<PathBuf>,
}

impl FlowSomConfig {
    #[must_use]
    pub fn new(channels: Vec<String>) -> Self {
        Self {
            channels,
            debug_dir: None,
            threads: None,
            masks: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct FlowSomResult {
    pub summary: FlowSomSummary,
    pub assignments: Vec<FlowSomAssignment>,
    pub nodes: Vec<FlowSomNode>,
    pub metaclusters: Vec<FlowSomMetacluster>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct FlowSomSummary {
    pub tool: String,
    pub version: String,
    pub algorithm: String,
    pub input: String,
    pub output: String,
    pub selected_channels: Vec<String>,
    pub resolved_channels: Vec<String>,
    pub parameters: FlowSomParameters,
    pub metrics: FlowSomMetrics,
    pub scale: Vec<FlowSomScaleSummary>,
    pub status: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct FlowSomParameters {
    pub xdim: usize,
    pub ydim: usize,
    pub metaclusters: usize,
    pub training_iterations: usize,
    pub learning_rate_start: f64,
    pub learning_rate_end: f64,
    pub radius_start: f64,
    pub radius_end: f64,
    pub random_seed: u64,
    pub scale_input: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct FlowSomMetrics {
    pub events: usize,
    pub channels: usize,
    pub som_nodes: usize,
    pub metaclusters: usize,
    pub empty_nodes: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct FlowSomScaleSummary {
    pub channel: String,
    pub mean: f64,
    pub standard_deviation: f64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct FlowSomAssignment {
    /// 0-based row in the input FCS.
    pub event_index: usize,
    /// 0-based row in the raw acquisition (`Original_ID - 1` when present).
    pub source_event_id: u64,
    pub flowsom_node: usize,
    pub flowsom_metacluster: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct FlowSomNode {
    pub node_id: usize,
    pub x: usize,
    pub y: usize,
    pub metacluster: usize,
    pub event_count: usize,
    pub codebook: Vec<f64>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct FlowSomMetacluster {
    pub metacluster_id: usize,
    pub event_count: usize,
    pub node_count: usize,
}

#[derive(Clone, Debug)]
struct ResolvedChannel {
    requested_name: String,
    parameter_name: String,
    display_name: String,
    values: Vec<f64>,
}

#[derive(Clone, Debug)]
struct ScaledMatrix {
    values: Vec<Vec<f64>>,
    scale: Vec<FlowSomScaleSummary>,
}

#[derive(Clone, Debug)]
struct SomTrainingResult {
    codebook: Vec<Vec<f64>>,
    assignments: Vec<usize>,
    node_counts: Vec<usize>,
}

#[derive(Clone, Debug)]
struct Cluster {
    members: Vec<usize>,
}

#[derive(Clone, Debug)]
struct DeterministicRng {
    state: u64,
}

impl DeterministicRng {
    const fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next_u64(&mut self) -> u64 {
        self.state = self
            .state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.state
    }

    /// Uniform index in `0..upper_bound` drawn from the high 53 bits, which are
    /// far better distributed than the low bits of an LCG.
    fn next_index(&mut self, upper_bound: usize) -> usize {
        let unit = (self.next_u64() >> 11) as f64 / (1_u64 << 53) as f64;
        ((unit * upper_bound as f64) as usize).min(upper_bound - 1)
    }
}

pub struct FlowSomRunner;

impl FlowSomRunner {
    /// Run the FlowSOM-style pipeline and write per-event assignments as CSV.
    ///
    /// # Errors
    ///
    /// Returns an error if the input cannot be read, channels cannot be
    /// resolved, SOM assignment cannot be written, or debug artifacts cannot be
    /// produced.
    pub fn run(
        input: impl AsRef<Path>,
        output: impl AsRef<Path>,
        config: &FlowSomConfig,
    ) -> Result<FlowSomResult, FlowSomError> {
        let input = input.as_ref();
        let output = output.as_ref();
        with_thread_pool(config.threads, || Self::run_inner(input, output, config))
    }

    fn run_inner(
        input: &Path,
        output: &Path,
        config: &FlowSomConfig,
    ) -> Result<FlowSomResult, FlowSomError> {
        let dataset = load_dataset(input)?;
        let rows = selected_rows(input, &dataset, &config.masks)?;
        let events = rows.len();
        let channels = resolve_channels(input, &dataset, &config.channels, &rows)?;
        let matrix = channel_matrix(&channels, events);
        let scaled = scale_matrix(matrix, &channels);
        info!(
            "flowsom pipeline: loaded {} events and {} selected channels from {}",
            events,
            channels.len(),
            input.display()
        );

        let trained = train_som(&scaled.values);
        let node_metaclusters = hierarchical_metaclusters(&trained.codebook, FLOWSOM_METACLUSTERS);
        let source_event_ids =
            cytiq_fcs::source_event_ids(&dataset).map_err(|source| FlowSomError::FcsRead {
                path: input.to_path_buf(),
                source,
            })?;
        let assignments = build_assignments(
            &trained.assignments,
            &node_metaclusters,
            &rows,
            source_event_ids.as_deref(),
        );
        let nodes = build_nodes(&trained.codebook, &trained.node_counts, &node_metaclusters);
        let metaclusters = build_metaclusters(&node_metaclusters, &trained.node_counts);
        let summary = build_summary(
            input,
            output,
            config,
            &channels,
            &scaled,
            nodes.len(),
            metaclusters.len(),
            trained
                .node_counts
                .iter()
                .filter(|count| **count == 0)
                .count(),
        );

        write_assignments_csv(output, &assignments)?;
        if let Some(debug_dir) = config.debug_dir.as_deref() {
            write_debug_artifacts(&summary, &assignments, &nodes, &metaclusters, debug_dir)?;
        }
        info!(
            "flowsom pipeline: assigned {} events across {} SOM nodes and {} metaclusters",
            assignments.len(),
            nodes.len(),
            metaclusters.len()
        );

        Ok(FlowSomResult {
            summary,
            assignments,
            nodes,
            metaclusters,
        })
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

fn with_thread_pool<T: Send>(
    threads: Option<usize>,
    operation: impl FnOnce() -> Result<T, FlowSomError> + Send,
) -> Result<T, FlowSomError> {
    let Some(threads) = threads else {
        return operation();
    };

    debug!("using local rayon thread pool with {threads} threads");
    ThreadPoolBuilder::new()
        .num_threads(threads)
        .build()
        .map_err(|source| FlowSomError::ThreadPoolBuild { threads, source })?
        .install(operation)
}

fn load_dataset(path: &Path) -> Result<FcsDataset, FlowSomError> {
    if !path.is_file() {
        return Err(FlowSomError::MissingInputFile {
            path: path.to_path_buf(),
        });
    }
    debug!("loading FCS file {}", path.display());
    let dataset = FcsReader::open(path)
        .and_then(FcsReader::read)
        .map_err(|source| FlowSomError::FcsRead {
            path: path.to_path_buf(),
            source,
        })?;
    if event_count(&dataset) == 0 {
        return Err(FlowSomError::EmptyInput {
            path: path.to_path_buf(),
        });
    }

    Ok(dataset)
}

/// Input rows to cluster: every row, or those every event mask keeps.
fn selected_rows(
    path: &Path,
    dataset: &FcsDataset,
    masks: &[PathBuf],
) -> Result<Vec<usize>, FlowSomError> {
    let Some(mask) = cytiq_mask::read_combined(masks)? else {
        return Ok((0..event_count(dataset)).collect());
    };
    let rows = mask
        .keep_for(dataset)?
        .iter()
        .enumerate()
        .filter_map(|(row, keep)| keep.then_some(row))
        .collect::<Vec<_>>();
    info!(
        "flowsom: event mask keeps {} of {} event(s)",
        rows.len(),
        event_count(dataset)
    );
    if rows.is_empty() {
        return Err(FlowSomError::NoMaskedEvents {
            path: path.to_path_buf(),
        });
    }
    Ok(rows)
}

fn resolve_channels(
    path: &Path,
    dataset: &FcsDataset,
    requested_channels: &[String],
    rows: &[usize],
) -> Result<Vec<ResolvedChannel>, FlowSomError> {
    if requested_channels.is_empty() {
        return Err(FlowSomError::EmptyChannels);
    }

    let channels = parallel_try_map(
        requested_channels,
        "FlowSOM channel extraction",
        |_, channel| {
            let Some(parameter_index) = find_parameter_index(&dataset.parameters, channel) else {
                return Err(FlowSomError::MissingChannel {
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
            let column = column_to_f64(
                dataset
                    .data
                    .column(parameter_index)
                    .expect("parameter index should have a data column"),
            );
            let values = rows.iter().map(|row| column[*row]).collect::<Vec<_>>();
            if values.iter().any(|value| !value.is_finite()) {
                return Err(FlowSomError::NonFiniteChannel {
                    path: path.to_path_buf(),
                    channel: channel.clone(),
                });
            }
            Ok(ResolvedChannel {
                requested_name: channel.clone(),
                parameter_name: parameter.name.clone(),
                display_name: display_parameter_identifier(&parameter),
                values,
            })
        },
    )?;

    let mut seen = HashSet::new();
    for channel in &channels {
        if !seen.insert(channel.parameter_name.clone()) {
            return Err(FlowSomError::DuplicateChannel {
                channel: channel.parameter_name.clone(),
            });
        }
    }

    Ok(channels)
}

fn channel_matrix(channels: &[ResolvedChannel], events: usize) -> Vec<Vec<f64>> {
    if events < 2 {
        return (0..events)
            .map(|event_index| {
                channels
                    .iter()
                    .map(|channel| channel.values[event_index])
                    .collect()
            })
            .collect();
    }

    debug!("parallelizing FlowSOM event matrix assembly with rayon for {events} events");
    (0..events)
        .into_par_iter()
        .map(|event_index| {
            channels
                .iter()
                .map(|channel| channel.values[event_index])
                .collect()
        })
        .collect()
}

fn scale_matrix(mut values: Vec<Vec<f64>>, channels: &[ResolvedChannel]) -> ScaledMatrix {
    let events = values.len();
    let scale = channels
        .iter()
        .enumerate()
        .map(|(channel_index, channel)| {
            let mean = values.iter().map(|event| event[channel_index]).sum::<f64>() / events as f64;
            let variance = values
                .iter()
                .map(|event| {
                    let centered = event[channel_index] - mean;
                    centered * centered
                })
                .sum::<f64>()
                / events.saturating_sub(1).max(1) as f64;
            let standard_deviation = variance.sqrt();
            FlowSomScaleSummary {
                channel: channel.display_name.clone(),
                mean,
                standard_deviation,
            }
        })
        .collect::<Vec<_>>();

    if FLOWSOM_SCALE_INPUT {
        if values.len() >= 2 {
            debug!(
                "parallelizing FlowSOM input scaling with rayon for {} events",
                values.len()
            );
            values.par_iter_mut().for_each(|event| {
                for (channel_index, scale) in scale.iter().enumerate() {
                    if scale.standard_deviation <= EQUALITY_EPSILON {
                        event[channel_index] = 0.0;
                    } else {
                        event[channel_index] =
                            (event[channel_index] - scale.mean) / scale.standard_deviation;
                    }
                }
            });
        } else {
            for event in &mut values {
                for (channel_index, scale) in scale.iter().enumerate() {
                    if scale.standard_deviation <= EQUALITY_EPSILON {
                        event[channel_index] = 0.0;
                    } else {
                        event[channel_index] =
                            (event[channel_index] - scale.mean) / scale.standard_deviation;
                    }
                }
            }
        }
    }

    ScaledMatrix { values, scale }
}

fn parallel_try_map<T, U, E, F>(items: &[T], stage: &str, worker: F) -> Result<Vec<U>, E>
where
    T: Sync,
    U: Send,
    E: Send,
    F: Fn(usize, &T) -> Result<U, E> + Sync + Send,
{
    if items.len() < 2 {
        return items
            .iter()
            .enumerate()
            .map(|(index, item)| worker(index, item))
            .collect();
    }

    debug!("parallelizing {stage} with rayon for {} tasks", items.len());
    items
        .par_iter()
        .enumerate()
        .map(|(index, item)| worker(index, item))
        .collect::<Vec<_>>()
        .into_iter()
        .collect()
}

/// Online SOM training following `FlowSOM`'s `C_SOM` (`src/som.c`): `rlen * n`
/// single-event updates with events drawn at random, linearly decaying learning
/// rate and radius per update, a bubble neighbourhood on the Chebyshev grid
/// distance, and only the best-matching node updated once the radius drops
/// below one.
fn train_som(events: &[Vec<f64>]) -> SomTrainingResult {
    let mut rng = DeterministicRng::new(FLOWSOM_RANDOM_SEED);
    let mut codebook = initialize_codebook(events, &mut rng);
    let node_count = codebook.len();
    let grid_distances = (0..node_count)
        .map(|left| {
            (0..node_count)
                .map(|right| grid_distance_chebyshev(left, right))
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let radius_start = default_radius_start();
    let total_updates = FLOWSOM_TRAINING_ITERATIONS * events.len();
    let radius_step = (radius_start - FLOWSOM_RADIUS_END) / total_updates as f64;
    let mut threshold = radius_start;
    for update in 0..total_updates {
        let event = &events[rng.next_index(events.len())];
        let best_node = best_matching_node(&codebook, event);
        if threshold < 1.0 {
            threshold = 0.5;
        }
        let learning_rate = interpolate(
            FLOWSOM_LEARNING_RATE_START,
            FLOWSOM_LEARNING_RATE_END,
            update as f64 / total_updates as f64,
        );
        for (prototype, distance) in codebook.iter_mut().zip(&grid_distances[best_node]) {
            if *distance > threshold {
                continue;
            }
            for (prototype_value, event_value) in prototype.iter_mut().zip(event) {
                *prototype_value += learning_rate * (*event_value - *prototype_value);
            }
        }
        threshold -= radius_step;
    }
    let assignments = assign_events(events, &codebook);
    let node_counts = count_nodes(&assignments, codebook.len());

    SomTrainingResult {
        codebook,
        assignments,
        node_counts,
    }
}

/// Initial codes are distinct events sampled without replacement, as in
/// `FlowSOM::SOM(init = FALSE)`; small inputs repeat events once exhausted.
fn initialize_codebook(events: &[Vec<f64>], rng: &mut DeterministicRng) -> Vec<Vec<f64>> {
    let node_count = FLOWSOM_XDIM * FLOWSOM_YDIM;
    let mut indices = (0..events.len()).collect::<Vec<_>>();
    let distinct = node_count.min(events.len());
    for position in 0..distinct {
        let swap_with = position + rng.next_index(events.len() - position);
        indices.swap(position, swap_with);
    }
    (0..node_count)
        .map(|node| events[indices[node % distinct]].clone())
        .collect()
}

/// Type-7 quantile of the full Chebyshev grid-distance matrix (diagonal
/// included), which is 6 for the default 10x10 grid.
fn default_radius_start() -> f64 {
    let node_count = FLOWSOM_XDIM * FLOWSOM_YDIM;
    let mut distances = (0..node_count)
        .flat_map(|left| (0..node_count).map(move |right| grid_distance_chebyshev(left, right)))
        .collect::<Vec<_>>();
    distances.sort_by(f64::total_cmp);
    let position = FLOWSOM_RADIUS_START_QUANTILE * (distances.len() - 1) as f64;
    let lower = position.floor() as usize;
    let upper = position.ceil() as usize;
    distances[lower] + (distances[upper] - distances[lower]) * (position - lower as f64)
}

fn assign_events(events: &[Vec<f64>], codebook: &[Vec<f64>]) -> Vec<usize> {
    events
        .par_iter()
        .map(|event| best_matching_node(codebook, event))
        .collect()
}

fn count_nodes(assignments: &[usize], node_count: usize) -> Vec<usize> {
    let mut counts = vec![0_usize; node_count];
    for node in assignments {
        counts[*node] += 1;
    }
    counts
}

fn best_matching_node(codebook: &[Vec<f64>], event: &[f64]) -> usize {
    let mut best_index = 0;
    let mut best_distance = squared_distance(&codebook[0], event);
    for (index, prototype) in codebook.iter().enumerate().skip(1) {
        let distance = squared_distance(prototype, event);
        if distance < best_distance {
            best_index = index;
            best_distance = distance;
        }
    }
    best_index
}

fn hierarchical_metaclusters(codebook: &[Vec<f64>], target_count: usize) -> Vec<usize> {
    let node_count = codebook.len();
    let target_count = target_count.clamp(1, node_count);
    let distances = node_pair_distances(codebook);
    let mut clusters = (0..node_count)
        .map(|node| Cluster {
            members: vec![node],
        })
        .collect::<Vec<_>>();

    while clusters.len() > target_count {
        let (left, right) = closest_cluster_pair(&clusters, &distances);
        let mut right_members = clusters[right].members.clone();
        clusters[left].members.append(&mut right_members);
        clusters[left].members.sort_unstable();
        clusters.remove(right);
    }

    clusters.sort_by_key(|cluster| cluster.members[0]);
    let mut metaclusters = vec![0_usize; node_count];
    for (metacluster_index, cluster) in clusters.iter().enumerate() {
        for node in &cluster.members {
            metaclusters[*node] = metacluster_index + 1;
        }
    }
    metaclusters
}

fn closest_cluster_pair(clusters: &[Cluster], distances: &[Vec<f64>]) -> (usize, usize) {
    let mut best_pair = (0, 1);
    let mut best_distance = average_linkage(&clusters[0], &clusters[1], distances);
    for left in 0..clusters.len() {
        for right in (left + 1)..clusters.len() {
            let distance = average_linkage(&clusters[left], &clusters[right], distances);
            if distance < best_distance {
                best_pair = (left, right);
                best_distance = distance;
            }
        }
    }
    best_pair
}

fn average_linkage(left: &Cluster, right: &Cluster, distances: &[Vec<f64>]) -> f64 {
    let mut total = 0.0;
    let mut count = 0_usize;
    for left_node in &left.members {
        for right_node in &right.members {
            total += distances[*left_node][*right_node];
            count += 1;
        }
    }
    total / count as f64
}

fn node_pair_distances(codebook: &[Vec<f64>]) -> Vec<Vec<f64>> {
    if codebook.len() < 2 {
        return vec![vec![0.0; codebook.len()]; codebook.len()];
    }

    debug!(
        "parallelizing FlowSOM node distance matrix with rayon for {} nodes",
        codebook.len()
    );
    (0..codebook.len())
        .into_par_iter()
        .map(|left| {
            let mut row = vec![0.0; codebook.len()];
            for right in 0..codebook.len() {
                if left != right {
                    row[right] = squared_distance(&codebook[left], &codebook[right]).sqrt();
                }
            }
            row
        })
        .collect()
}

/// `assignments[i]` is the node of input row `rows[i]`.
fn build_assignments(
    assignments: &[usize],
    node_metaclusters: &[usize],
    rows: &[usize],
    source_event_ids: Option<&[u64]>,
) -> Vec<FlowSomAssignment> {
    assignments
        .par_iter()
        .zip(rows)
        .map(|(node, &event_index)| FlowSomAssignment {
            event_index,
            source_event_id: source_event_ids.map_or(event_index as u64, |ids| ids[event_index]),
            flowsom_node: node + 1,
            flowsom_metacluster: node_metaclusters[*node],
        })
        .collect()
}

fn build_nodes(
    codebook: &[Vec<f64>],
    node_counts: &[usize],
    node_metaclusters: &[usize],
) -> Vec<FlowSomNode> {
    codebook
        .par_iter()
        .enumerate()
        .map(|(node_index, prototype)| FlowSomNode {
            node_id: node_index + 1,
            x: node_index % FLOWSOM_XDIM + 1,
            y: node_index / FLOWSOM_XDIM + 1,
            metacluster: node_metaclusters[node_index],
            event_count: node_counts[node_index],
            codebook: prototype.clone(),
        })
        .collect()
}

fn build_metaclusters(
    node_metaclusters: &[usize],
    node_counts: &[usize],
) -> Vec<FlowSomMetacluster> {
    let metacluster_count = node_metaclusters.iter().copied().max().unwrap_or(0);
    (1..=metacluster_count)
        .into_par_iter()
        .map(|metacluster_id| FlowSomMetacluster {
            metacluster_id,
            event_count: node_metaclusters
                .iter()
                .enumerate()
                .filter_map(|(node_index, candidate)| {
                    (*candidate == metacluster_id).then_some(node_counts[node_index])
                })
                .sum(),
            node_count: node_metaclusters
                .iter()
                .filter(|candidate| **candidate == metacluster_id)
                .count(),
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn build_summary(
    input: &Path,
    output: &Path,
    config: &FlowSomConfig,
    channels: &[ResolvedChannel],
    scaled: &ScaledMatrix,
    node_count: usize,
    metacluster_count: usize,
    empty_nodes: usize,
) -> FlowSomSummary {
    FlowSomSummary {
        tool: "cytiq flowsom".to_owned(),
        version: env!("CARGO_PKG_VERSION").to_owned(),
        algorithm: "FlowSOM-style MVP".to_owned(),
        input: input.display().to_string(),
        output: output.display().to_string(),
        selected_channels: channels
            .iter()
            .map(|channel| channel.requested_name.clone())
            .collect(),
        resolved_channels: channels
            .iter()
            .map(|channel| channel.display_name.clone())
            .collect(),
        parameters: FlowSomParameters {
            xdim: FLOWSOM_XDIM,
            ydim: FLOWSOM_YDIM,
            metaclusters: FLOWSOM_METACLUSTERS,
            training_iterations: FLOWSOM_TRAINING_ITERATIONS,
            learning_rate_start: FLOWSOM_LEARNING_RATE_START,
            learning_rate_end: FLOWSOM_LEARNING_RATE_END,
            radius_start: default_radius_start(),
            radius_end: FLOWSOM_RADIUS_END,
            random_seed: FLOWSOM_RANDOM_SEED,
            scale_input: FLOWSOM_SCALE_INPUT,
        },
        metrics: FlowSomMetrics {
            events: scaled.values.len(),
            channels: config.channels.len(),
            som_nodes: node_count,
            metaclusters: metacluster_count,
            empty_nodes,
        },
        scale: scaled.scale.clone(),
        status: "PASS".to_owned(),
    }
}

fn interpolate(start: f64, end: f64, progress: f64) -> f64 {
    start + (end - start) * progress
}

fn squared_distance(left: &[f64], right: &[f64]) -> f64 {
    left.iter()
        .zip(right)
        .map(|(left_value, right_value)| {
            let difference = left_value - right_value;
            difference * difference
        })
        .sum()
}

/// Chebyshev ("maximum") distance between SOM grid nodes, as
/// `stats::dist(grid, method = "maximum")` in `FlowSOM::SOM`.
fn grid_distance_chebyshev(left: usize, right: usize) -> f64 {
    let dx = (left % FLOWSOM_XDIM).abs_diff(right % FLOWSOM_XDIM);
    let dy = (left / FLOWSOM_XDIM).abs_diff(right / FLOWSOM_XDIM);
    dx.max(dy) as f64
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_comma_separated_channels() {
        let channels = parse_channel_list("CD3, CD4,,CD8").expect("channels should parse");
        assert_eq!(channels, vec!["CD3", "CD4", "CD8"]);
    }

    #[test]
    fn rejects_empty_comma_separated_channels() {
        let error = parse_channel_list(" , ,, ").expect_err("empty list should fail");
        assert!(error.contains("at least one channel"));
    }

    #[test]
    fn trains_som_deterministically() {
        let events = synthetic_events();
        let first = train_som(&events);
        let second = train_som(&events);
        assert_eq!(first.assignments, second.assignments);
        assert_eq!(first.node_counts, second.node_counts);
        assert_eq!(first.codebook, second.codebook);
    }

    #[test]
    fn default_radius_matches_flowsom_quantile() {
        // stats::quantile(as.matrix(dist(expand.grid(1:10, 1:10), "maximum")), 0.67) == 6
        assert!((default_radius_start() - 6.0).abs() < 1e-12);
    }

    #[test]
    fn chebyshev_grid_distance_uses_maximum_axis_offset() {
        assert!((grid_distance_chebyshev(0, 0) - 0.0).abs() < f64::EPSILON);
        assert!((grid_distance_chebyshev(0, 11) - 1.0).abs() < f64::EPSILON);
        assert!((grid_distance_chebyshev(0, 23) - 3.0).abs() < f64::EPSILON);
    }

    #[test]
    fn codebook_initialization_samples_distinct_events() {
        let events = (0..200)
            .map(|index| vec![f64::from(index)])
            .collect::<Vec<_>>();
        let mut rng = DeterministicRng::new(FLOWSOM_RANDOM_SEED);
        let codebook = initialize_codebook(&events, &mut rng);
        let distinct = codebook
            .iter()
            .map(|code| code[0].to_bits())
            .collect::<HashSet<_>>();
        assert_eq!(distinct.len(), FLOWSOM_XDIM * FLOWSOM_YDIM);
    }

    #[test]
    fn hierarchical_metaclustering_uses_requested_cluster_count() {
        let codebook = vec![
            vec![0.0, 0.0],
            vec![0.1, 0.0],
            vec![10.0, 10.0],
            vec![10.1, 10.0],
        ];
        let metaclusters = hierarchical_metaclusters(&codebook, 2);
        assert_eq!(metaclusters.len(), 4);
        assert_eq!(metaclusters.iter().copied().max(), Some(2));
        assert_eq!(metaclusters[0], metaclusters[1]);
        assert_eq!(metaclusters[2], metaclusters[3]);
        assert_ne!(metaclusters[0], metaclusters[2]);
    }

    fn synthetic_events() -> Vec<Vec<f64>> {
        (0..40)
            .map(|index| {
                let value = f64::from(index);
                if index < 20 {
                    vec![value * 0.01, value * 0.02]
                } else {
                    vec![5.0 + value * 0.01, 6.0 + value * 0.02]
                }
            })
            .collect()
    }
}
