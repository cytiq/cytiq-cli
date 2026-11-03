//! A [`PlotRequest`] and the loaded [`Sample`] → a renderer-neutral
//! [`PlotSpec`]: axes, selected events, layers, gate outlines and text.

use std::collections::BTreeSet;
use std::fmt::Write as _;

use cytiq_density::sample_indices;
use cytiq_fcs::{FcsDataset, TransformContract};
use cytiq_gating::{CompensationRef, Dimension, Gate, GateKind, GatingResults, GatingStrategy};
use cytiq_transform::ValueTransform;
use cytiq_transform::scales::Logicle;

use crate::axis::{Axis, AxisInput, DataTransform, Display};
use crate::bin::Histogram;
use crate::classes::Classes;
use crate::config::{
    AxisOptions, DENSITY_LEVELS, PlotKind, PlotRequest, RemovedLayer, ScaleChoice,
};
use crate::contour;
use crate::error::PlotError;
use crate::geometry::{self, AxisDimensions, GeometryError};
use crate::palette::{
    self, CATEGORICAL, GATE_COLOR, OTHER_COLOR, POINT_COLOR, REMOVED_COLOR, UNLABELLED_COLOR,
};
use crate::sample::Sample;
use crate::spec::{AxisSpec, Frame, LEGEND_ROW_HEIGHT, Label, Layer, LegendEntry, PlotSpec};

/// Logicle defaults of `cytiq transform` (and FlowJo): `W`, `M`, `A`.
const LOGICLE_W: f64 = 0.5;
const LOGICLE_M: f64 = 4.5;
const LOGICLE_A: f64 = 0.0;
/// `T` when neither `$PnR` nor the data give one.
const LOGICLE_DEFAULT_T: f64 = 262_144.0;
const GATE_LINE_WIDTH: f64 = 1.5;
const CONTOUR_LINE_WIDTH: f64 = 1.0;

/// Event counts behind one plot.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PlotStats {
    /// Events in the FCS file.
    pub events: usize,
    /// Events plotted (kept by the mask, in the population, finite).
    pub shown: usize,
    /// Events in the population the mask removed.
    pub removed: usize,
    /// Events drawn as dots, after sampling.
    pub drawn: usize,
    /// Plotted or removed events outside the axis range, drawn on its edge.
    pub off_scale: usize,
    /// Events in the population with a non-finite value on an axis.
    pub non_finite: usize,
}

/// Where an axis takes its values from.
#[derive(Clone, Copy)]
enum Source<'a> {
    Channel(usize),
    Dimension(&'a Dimension),
}

struct ResolvedAxis<'a> {
    axis: Axis,
    values: Vec<f64>,
    /// `$PnN` or gate dimension name, for the title.
    short: String,
    dimension: Option<&'a Dimension>,
}

struct Gating<'a> {
    strategy: &'a GatingStrategy,
    results: &'a GatingResults,
}

pub(crate) fn compose(
    request: &PlotRequest,
    sample: &Sample,
) -> Result<(PlotSpec, PlotStats), PlotError> {
    request.validate()?;
    let plot = request.name.as_str();
    if request.color_by && sample.classes.is_none() {
        return Err(PlotError::ColorByWithoutClasses {
            plot: plot.to_owned(),
        });
    }
    let gating = gating(request, sample)?;
    let gates = overlay_gates(request, gating.as_ref())?;
    let population = match (&request.population, &gating) {
        (Some(id), Some(gating)) => {
            Some(
                gating
                    .results
                    .membership(id)
                    .ok_or_else(|| PlotError::UnknownGate {
                        plot: plot.to_owned(),
                        gate: id.clone(),
                    })?,
            )
        }
        _ => None,
    };
    let (x_source, y_source) = resolve_sources(request, &sample.dataset, &gates)?;
    let strategy = gating.as_ref().map(|gating| gating.strategy);
    let x = build_axis(x_source, &request.x, "x", request, sample, strategy)?;
    let y = build_axis(y_source, &request.y, "y", request, sample, strategy)?;

    let keep = sample.keep.as_deref();
    let selection = Selection::new(&x, &y, population, keep);
    let mut drawing = Drawing {
        layers: Vec::new(),
        legend: Vec::new(),
        notes: Vec::new(),
        labels: Vec::new(),
        drawn: 0,
        max_points: request.max_points,
        selection: &selection,
    };
    drawing.base(request, sample)?;
    // Removed events go over the plotted ones. Under a density cell they
    // would vanish: on real data most removed events share cells with kept
    // ones, and the legend would count dots nobody can see.
    if keep.is_some() && request.removed == RemovedLayer::Overlay {
        drawing.points("removed by mask", REMOVED_COLOR, &selection.removed);
        drawing.legend.push(LegendEntry {
            color: REMOVED_COLOR.to_owned(),
            text: format!("removed by mask ({})", count(selection.stats.removed)),
        });
    }
    if let Some(gating) = &gating {
        drawing.gates(plot, &gates, gating.results, keep, [&x, &y])?;
    }
    let mut stats = selection.stats.clone();
    stats.drawn = drawing.drawn;
    drawing
        .notes
        .extend(selection_notes(&stats, keep.is_some()));
    let Drawing {
        layers,
        mut legend,
        notes,
        labels,
        ..
    } = drawing;

    let frame = Frame::new(request.width, request.height, !legend.is_empty()).ok_or_else(|| {
        PlotError::InvalidOption {
            plot: plot.to_owned(),
            field: "width",
            message: "leaves too little room for the plot area".to_owned(),
        }
    })?;
    fit_legend(&mut legend, frame);
    let spec = PlotSpec {
        name: plot.to_owned(),
        title: request
            .title
            .clone()
            .unwrap_or_else(|| format!("{}: {} vs {}", sample.name, y.short, x.short)),
        subtitle: subtitle(request, &stats, keep.is_some(), sample),
        notes,
        width: request.width,
        height: request.height,
        frame,
        x: AxisSpec {
            ticks: x.axis.ticks(frame.width),
            title: x.axis.title.clone(),
        },
        y: AxisSpec {
            ticks: y.axis.ticks(frame.height),
            title: y.axis.title.clone(),
        },
        layers,
        labels,
        legend,
    };
    Ok((spec, stats))
}

/// Which events a plot shows, and where they sit.
struct Selection {
    /// Unit position of every event (non-finite when a value is).
    units: Vec<[f64; 2]>,
    /// Kept by the mask, in the population, finite.
    shown: Vec<bool>,
    /// Removed by the mask, in the population, finite.
    removed: Vec<bool>,
    stats: PlotStats,
}

impl Selection {
    fn new(
        x: &ResolvedAxis<'_>,
        y: &ResolvedAxis<'_>,
        population: Option<&[bool]>,
        keep: Option<&[bool]>,
    ) -> Self {
        let events = x.values.len();
        let units = x
            .values
            .iter()
            .zip(&y.values)
            .map(|(vx, vy)| [x.axis.unit(*vx), y.axis.unit(*vy)])
            .collect::<Vec<_>>();
        let mut stats = PlotStats {
            events,
            ..PlotStats::default()
        };
        let mut shown = vec![false; events];
        let mut removed = vec![false; events];
        for (event, [ux, uy]) in units.iter().enumerate() {
            if population.is_some_and(|members| !members[event]) {
                continue;
            }
            if !(ux.is_finite() && uy.is_finite()) {
                stats.non_finite += 1;
                continue;
            }
            if keep.is_none_or(|keep| keep[event]) {
                shown[event] = true;
                stats.shown += 1;
            } else {
                removed[event] = true;
                stats.removed += 1;
            }
            if !((0.0..=1.0).contains(ux) && (0.0..=1.0).contains(uy)) {
                stats.off_scale += 1;
            }
        }
        Self {
            units,
            shown,
            removed,
            stats,
        }
    }

    /// Unit position of an event, clamped onto the plot.
    fn clamped(&self, event: usize) -> [f64; 2] {
        let [ux, uy] = self.units[event];
        [ux.clamp(0.0, 1.0), uy.clamp(0.0, 1.0)]
    }

    fn positions(&self, mask: &[bool]) -> Vec<[f64; 2]> {
        mask.iter()
            .enumerate()
            .filter(|(_, selected)| **selected)
            .map(|(event, _)| self.clamped(event))
            .collect()
    }
}

fn selection_notes(stats: &PlotStats, masked: bool) -> Vec<String> {
    let mut notes = Vec::new();
    if stats.shown == 0 {
        notes.push(if masked {
            "no events left after the mask".to_owned()
        } else {
            "no events to show".to_owned()
        });
    }
    if stats.off_scale > 0 {
        notes.push(format!(
            "{} off-scale events drawn on the axes",
            count(stats.off_scale)
        ));
    }
    if stats.non_finite > 0 {
        notes.push(format!(
            "{} events with non-finite values not shown",
            count(stats.non_finite)
        ));
    }
    notes
}

/// Layers, legend and notes of a plot being composed.
struct Drawing<'c> {
    layers: Vec<Layer>,
    legend: Vec<LegendEntry>,
    notes: Vec<String>,
    labels: Vec<Label>,
    /// Events drawn as dots so far.
    drawn: usize,
    max_points: usize,
    selection: &'c Selection,
}

impl Drawing<'_> {
    /// The layers of the plot kind: density cells, dots or contours.
    fn base(&mut self, request: &PlotRequest, sample: &Sample) -> Result<(), PlotError> {
        let selection = self.selection;
        match request.kind {
            PlotKind::Density => {
                let histogram = Histogram::new(request.bins, selection.positions(&selection.shown));
                self.layers.push(Layer::Density {
                    bins: request.bins,
                    levels: histogram.levels(DENSITY_LEVELS),
                    colors: palette::levels(request.palette, DENSITY_LEVELS),
                });
            }
            PlotKind::Scatter if request.color_by => {
                let classes =
                    sample
                        .classes
                        .as_ref()
                        .ok_or_else(|| PlotError::ColorByWithoutClasses {
                            plot: request.name.clone(),
                        })?;
                self.classes(classes, &selection.shown);
            }
            PlotKind::Scatter => self.points("events", POINT_COLOR, &selection.shown),
            PlotKind::Contour => self.contours(request),
        }
        Ok(())
    }

    fn contours(&mut self, request: &PlotRequest) {
        let selection = self.selection;
        let positions = selection.positions(&selection.shown);
        let histogram = Histogram::new(request.bins, positions.iter().copied());
        let surface = contour::smooth(&histogram, contour::bandwidth(&positions, request.bins));
        let thresholds = contour::probability_thresholds(&surface, request.contour_step);
        if let Some(&outer) = thresholds.first() {
            let outliers = selection
                .shown
                .iter()
                .enumerate()
                .map(|(event, shown)| {
                    *shown && surface.values[histogram.cell_of(selection.clamped(event))] < outer
                })
                .collect::<Vec<_>>();
            self.points("outliers", POINT_COLOR, &outliers);
        }
        let span = thresholds.len().saturating_sub(1).max(1);
        for (index, threshold) in thresholds.iter().enumerate() {
            #[allow(clippy::cast_precision_loss, reason = "few contour levels")]
            let (position, enclosed) = (
                index as f64 / span as f64,
                1.0 - request.contour_step * (index + 1) as f64,
            );
            self.layers.push(Layer::Lines {
                name: format!("{:.0}%", enclosed * 100.0),
                color: palette::sequential(request.palette, position),
                width: CONTOUR_LINE_WIDTH,
                paths: contour::contour_lines(&surface, *threshold),
            });
        }
    }

    /// Outlines and `id xx.x%` labels of the overlaid gates.
    fn gates(
        &mut self,
        plot: &str,
        gates: &[&Gate],
        results: &GatingResults,
        keep: Option<&[bool]>,
        [x, y]: [&ResolvedAxis<'_>; 2],
    ) -> Result<(), PlotError> {
        let dimensions = AxisDimensions {
            x: x.dimension,
            y: y.dimension,
        };
        for gate in gates {
            let paths = geometry::outline(gate, &dimensions, &x.axis, &y.axis).map_err(
                |error| match error {
                    GeometryError::Mismatch(reason) => PlotError::GateAxisMismatch {
                        plot: plot.to_owned(),
                        gate: gate.id.clone(),
                        reason,
                    },
                    GeometryError::Unsupported(reason) => PlotError::UnsupportedGateGeometry {
                        plot: plot.to_owned(),
                        gate: gate.id.clone(),
                        reason,
                    },
                },
            )?;
            let anchor = geometry::label_anchor(gate, &dimensions, &x.axis, &y.axis, &paths);
            if let Some(position) = anchor {
                let events = self.selection.units.len();
                self.labels.push(Label {
                    position,
                    text: format!(
                        "{} {}",
                        gate.id,
                        percent_of_parent(results, &gate.id, keep, events)
                    ),
                });
                self.layers.push(Layer::Lines {
                    name: gate.id.clone(),
                    color: GATE_COLOR.to_owned(),
                    width: GATE_LINE_WIDTH,
                    paths,
                });
            }
        }
        Ok(())
    }

    /// A dots layer of the events in `mask`, sampled evenly to `max_points`.
    fn points(&mut self, name: &str, color: &str, mask: &[bool]) {
        let eligible = mask.iter().filter(|selected| **selected).count();
        let indices = sample_indices(mask, self.max_points);
        self.note_sampling(name, indices.len(), eligible);
        if !indices.is_empty() {
            let selection = self.selection;
            self.layers.push(Layer::Points {
                name: name.to_owned(),
                color: color.to_owned(),
                points: indices
                    .into_iter()
                    .map(|event| selection.clamped(event))
                    .collect(),
            });
        }
    }

    fn note_sampling(&mut self, name: &str, drawn: usize, eligible: usize) {
        self.drawn += drawn;
        if drawn < eligible {
            self.notes.push(format!(
                "{name}: {} of {} drawn (evenly sampled)",
                count(drawn),
                count(eligible)
            ));
        }
    }

    /// Dots coloured by class. Colours follow class order, so a class keeps
    /// its colour across samples; classes past the palette share the
    /// "other" grey and unlabelled events are drawn first, in light grey.
    fn classes(&mut self, classes: &Classes, shown: &[bool]) {
        let colored = classes.names.len().min(CATEGORICAL.len());
        let group = |event: usize| match classes.labels[event] {
            Some(class) if class < colored => Group::Class(class),
            Some(_) => Group::Other,
            None => Group::Unlabelled,
        };
        let mut counts = vec![0_usize; colored];
        let mut other_classes = BTreeSet::new();
        let (mut other, mut unlabelled) = (0_usize, 0_usize);
        for (event, _) in shown.iter().enumerate().filter(|(_, shown)| **shown) {
            match group(event) {
                Group::Class(class) => counts[class] += 1,
                Group::Other => {
                    other += 1;
                    other_classes.insert(classes.labels[event]);
                }
                Group::Unlabelled => unlabelled += 1,
            }
        }
        let eligible = shown.iter().filter(|shown| **shown).count();
        let drawn = sample_indices(shown, self.max_points);
        self.note_sampling("events", drawn.len(), eligible);
        let groups = [
            ("unlabelled".to_owned(), UNLABELLED_COLOR, Group::Unlabelled),
            ("other".to_owned(), OTHER_COLOR, Group::Other),
        ]
        .into_iter()
        .chain(
            classes
                .names
                .iter()
                .take(colored)
                .enumerate()
                .map(|(class, name)| (name.clone(), CATEGORICAL[class], Group::Class(class))),
        );
        for (name, color, wanted) in groups {
            let points = drawn
                .iter()
                .copied()
                .filter(|&event| group(event) == wanted)
                .map(|event| self.selection.clamped(event))
                .collect::<Vec<_>>();
            if !points.is_empty() {
                self.layers.push(Layer::Points {
                    name,
                    color: color.to_owned(),
                    points,
                });
            }
        }
        for (class, name) in classes.names.iter().take(colored).enumerate() {
            if counts[class] > 0 {
                self.legend.push(LegendEntry {
                    color: CATEGORICAL[class].to_owned(),
                    text: format!("{name} ({})", count(counts[class])),
                });
            }
        }
        if other > 0 {
            self.legend.push(LegendEntry {
                color: OTHER_COLOR.to_owned(),
                text: format!("other: {} classes ({})", other_classes.len(), count(other)),
            });
        }
        if unlabelled > 0 {
            self.legend.push(LegendEntry {
                color: UNLABELLED_COLOR.to_owned(),
                text: format!("unlabelled ({})", count(unlabelled)),
            });
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Group {
    Class(usize),
    Other,
    Unlabelled,
}

fn gating<'a>(request: &PlotRequest, sample: &'a Sample) -> Result<Option<Gating<'a>>, PlotError> {
    if request.gates.is_empty() && request.population.is_none() {
        return Ok(None);
    }
    match (&sample.strategy, &sample.gating) {
        (Some(strategy), Some(results)) => Ok(Some(Gating { strategy, results })),
        _ => Err(PlotError::MissingGatingMl {
            plot: request.name.clone(),
        }),
    }
}

fn overlay_gates<'a>(
    request: &PlotRequest,
    gating: Option<&Gating<'a>>,
) -> Result<Vec<&'a Gate>, PlotError> {
    let Some(gating) = gating else {
        return Ok(Vec::new());
    };
    request
        .gates
        .iter()
        .map(|id| {
            let gate = gating
                .strategy
                .gate(id)
                .ok_or_else(|| PlotError::UnknownGate {
                    plot: request.name.clone(),
                    gate: id.clone(),
                })?;
            if matches!(gate.kind, GateKind::Boolean { .. }) {
                return Err(PlotError::UnsupportedGateGeometry {
                    plot: request.name.clone(),
                    gate: id.clone(),
                    reason: "is a Boolean gate without geometry; use it as the population instead"
                        .to_owned(),
                });
            }
            Ok(gate)
        })
        .collect()
}

/// Pick each axis's source: a named channel, a gate dimension with that name,
/// or (when omitted) the first gate's dimensions.
fn resolve_sources<'a>(
    request: &PlotRequest,
    dataset: &FcsDataset,
    gates: &[&'a Gate],
) -> Result<(Source<'a>, Source<'a>), PlotError> {
    let dimensions = gates
        .iter()
        .flat_map(|gate| gate.dimensions())
        .collect::<Vec<_>>();
    let first = gates.first().map_or(&[][..], |gate| gate.dimensions());
    let named = |name: &str| -> Result<Source<'a>, PlotError> {
        if let Some(dimension) = dimensions
            .iter()
            .find(|dimension| names_match(dataset, name, &dimension.name))
        {
            return Ok(Source::Dimension(dimension));
        }
        dataset
            .find_parameter(name)
            .map(Source::Channel)
            .ok_or_else(|| PlotError::MissingChannel {
                channel: name.to_owned(),
                available: available_channels(dataset),
            })
    };
    let other = |taken: Source<'a>, axis: &'static str| {
        first
            .iter()
            .find(|dimension| !is_source(dataset, taken, dimension))
            .map(Source::Dimension)
            .ok_or_else(|| PlotError::MissingAxis {
                plot: request.name.clone(),
                axis,
            })
    };
    match (request.x.channel.as_deref(), request.y.channel.as_deref()) {
        (Some(x), Some(y)) => Ok((named(x)?, named(y)?)),
        (Some(x), None) => {
            let x = named(x)?;
            Ok((x, other(x, "y")?))
        }
        (None, Some(y)) => {
            let y = named(y)?;
            Ok((other(y, "x")?, y))
        }
        (None, None) => match first {
            [x, y, ..] => Ok((Source::Dimension(x), Source::Dimension(y))),
            [_] => Err(PlotError::MissingAxis {
                plot: request.name.clone(),
                axis: "y",
            }),
            [] => Err(PlotError::MissingAxis {
                plot: request.name.clone(),
                axis: "x",
            }),
        },
    }
}

fn names_match(dataset: &FcsDataset, requested: &str, dimension: &str) -> bool {
    let requested = requested.trim();
    requested == dimension
        || dataset
            .find_parameter(requested)
            .is_some_and(|index| dataset.find_parameter(dimension) == Some(index))
}

fn is_source(dataset: &FcsDataset, source: Source<'_>, dimension: &Dimension) -> bool {
    match source {
        Source::Dimension(taken) => geometry::same_dimension(taken, dimension),
        Source::Channel(index) => dataset.find_parameter(&dimension.name) == Some(index),
    }
}

fn available_channels(dataset: &FcsDataset) -> String {
    dataset
        .parameters
        .iter()
        .map(|parameter| match &parameter.label {
            Some(label) if !label.trim().is_empty() => {
                format!("{} [{}]", parameter.name, label.trim())
            }
            _ => parameter.name.clone(),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn build_axis<'a>(
    source: Source<'a>,
    options: &AxisOptions,
    axis: &'static str,
    request: &PlotRequest,
    sample: &Sample,
    strategy: Option<&GatingStrategy>,
) -> Result<ResolvedAxis<'a>, PlotError> {
    let dataset = &sample.dataset;
    let (values, channel, short, title, data, dimension) = match source {
        Source::Channel(index) => {
            let name = dataset.parameters[index].name.clone();
            (
                cytiq_fcs::linear_values(dataset, index),
                Some(index),
                short_name(dataset, index),
                channel_title(dataset, index),
                file_transform(sample, &name)?,
                None,
            )
        }
        Source::Dimension(dimension) => {
            let strategy = strategy.ok_or_else(|| PlotError::MissingGatingMl {
                plot: request.name.clone(),
            })?;
            let values = strategy.dimension_values(dataset, dimension)?;
            let channel = dataset.find_parameter(&dimension.name);
            let data = match (&dimension.transform, channel) {
                (Some(reference), _) => strategy
                    .transform(reference)
                    .cloned()
                    .map(DataTransform::Gating),
                (None, Some(index)) => file_transform(sample, &dataset.parameters[index].name)?,
                (None, None) => None,
            };
            (
                values,
                channel,
                channel.map_or_else(
                    || dimension.name.clone(),
                    |index| short_name(dataset, index),
                ),
                dimension_title(dataset, dimension, channel),
                data,
                Some(dimension),
            )
        }
    };
    // Scatter and time are recognised by `$PnN`, not by the marker label.
    let scatter = is_scatter_or_time(&channel.map_or_else(
        || short.clone(),
        |index| dataset.parameters[index].name.clone(),
    ));
    let display = match (options.scale, &data) {
        (ScaleChoice::Logicle | ScaleChoice::Arcsinh, Some(_)) => {
            return Err(PlotError::ScaleOnTransformedData {
                plot: request.name.clone(),
                channel: short,
                scale: options.scale.as_str(),
            });
        }
        (ScaleChoice::Linear, _) | (ScaleChoice::Auto, Some(_)) => Display::Linear,
        (ScaleChoice::Auto, None) if scatter => Display::Linear,
        (ScaleChoice::Auto | ScaleChoice::Logicle, None) => {
            Display::Logicle(Box::new(logicle_for(dataset, channel, &values)))
        }
        (ScaleChoice::Arcsinh, None) => Display::arcsinh(),
    };
    let starts_at_zero = display.is_linear() && data.is_none() && scatter;
    let built = Axis::new(AxisInput {
        title,
        data,
        display,
        values: &values,
        range: options.range,
        starts_at_zero,
    })
    .ok_or_else(|| PlotError::InvalidOption {
        plot: request.name.clone(),
        field: if axis == "x" { "x_range" } else { "y_range" },
        message: "is empty or not finite on this axis's scale".to_owned(),
    })?;
    Ok(ResolvedAxis {
        axis: built,
        values,
        short,
        dimension,
    })
}

/// The `cytiq transform` transform of a channel, when the file records one.
fn file_transform(sample: &Sample, channel: &str) -> Result<Option<DataTransform>, PlotError> {
    if !sample.signal.transformed(channel) {
        return Ok(None);
    }
    let contract =
        TransformContract::of(&sample.dataset.text, &[channel.to_owned()]).map_err(|source| {
            PlotError::Fcs {
                path: sample.input.clone(),
                source,
            }
        })?;
    let transform =
        ValueTransform::from_contract(&contract).map_err(|source| PlotError::Transform {
            channel: channel.to_owned(),
            source,
        })?;
    Ok(Some(DataTransform::File(transform)))
}

/// Logicle display with `T` from `$PnR`, else from the data.
fn logicle_for(dataset: &FcsDataset, channel: Option<usize>, values: &[f64]) -> Logicle {
    let range = channel.and_then(|index| {
        let parameter = &dataset.parameters[index];
        let key = format!("$P{}R", parameter.index);
        parameter
            .keywords
            .iter()
            .find(|(candidate, _)| candidate.eq_ignore_ascii_case(&key))
            .and_then(|(_, value)| value.trim().parse::<f64>().ok())
    });
    let data_max = values
        .iter()
        .filter(|value| value.is_finite())
        .fold(0.0_f64, |max, value| max.max(value.abs()));
    let top = range
        .filter(|range| range.is_finite() && *range > 0.0)
        .or((data_max > 0.0).then_some(data_max))
        .unwrap_or(LOGICLE_DEFAULT_T);
    Logicle::new(top, LOGICLE_W, LOGICLE_M, LOGICLE_A)
        .or_else(|_| Logicle::new(LOGICLE_DEFAULT_T, LOGICLE_W, LOGICLE_M, LOGICLE_A))
        .unwrap_or_else(|_| unreachable!("the default logicle parameters are valid"))
}

fn is_scatter_or_time(name: &str) -> bool {
    let upper = name.trim().to_ascii_uppercase();
    upper.starts_with("FSC") || upper.starts_with("SSC") || upper.starts_with("TIME")
}

/// The marker (`$PnS`) of a channel, else its `$PnN`.
fn short_name(dataset: &FcsDataset, index: usize) -> String {
    let parameter = &dataset.parameters[index];
    parameter
        .label
        .as_deref()
        .map(str::trim)
        .filter(|label| !label.is_empty())
        .unwrap_or(&parameter.name)
        .to_owned()
}

fn channel_title(dataset: &FcsDataset, index: usize) -> String {
    let parameter = &dataset.parameters[index];
    match parameter.label.as_deref().map(str::trim) {
        Some(label) if !label.is_empty() && label != parameter.name => {
            format!("{} :: {label}", parameter.name)
        }
        _ => parameter.name.clone(),
    }
}

fn dimension_title(dataset: &FcsDataset, dimension: &Dimension, channel: Option<usize>) -> String {
    let mut title = String::new();
    if dimension.compensation != CompensationRef::Uncompensated {
        title.push_str("Comp-");
    }
    title.push_str(&channel.map_or_else(
        || dimension.name.clone(),
        |index| channel_title(dataset, index),
    ));
    if let Some(reference) = &dimension.transform {
        let _ = write!(title, " [{reference}]");
    }
    title
}

/// `xx.x%` of the gate's parent (or of all kept events for a root gate), with
/// the mask applied: `cytiq gate`'s `percent_of_parent` for the same masks.
fn percent_of_parent(
    results: &GatingResults,
    gate: &str,
    keep: Option<&[bool]>,
    events: usize,
) -> String {
    let kept_members = |membership: &[bool]| {
        membership
            .iter()
            .enumerate()
            .filter(|(event, member)| **member && keep.is_none_or(|keep| keep[*event]))
            .count()
    };
    let Some(members) = results.membership(gate).map(kept_members) else {
        return String::new();
    };
    let parent = match results.parent(gate) {
        Some(parent) => results.membership(parent).map_or(0, kept_members),
        None => keep.map_or(events, |keep| keep.iter().filter(|kept| **kept).count()),
    };
    if parent == 0 {
        return "\u{2013}".to_owned();
    }
    #[allow(clippy::cast_precision_loss, reason = "event counts fit f64 exactly")]
    let percent = 100.0 * members as f64 / parent as f64;
    format!("{percent:.1}%")
}

fn subtitle(request: &PlotRequest, stats: &PlotStats, masked: bool, sample: &Sample) -> String {
    let mut parts = vec![format!(
        "{} · {} events shown",
        request.kind.as_str(),
        count(stats.shown)
    )];
    if masked {
        parts.push(format!("{} removed by mask", count(stats.removed)));
    }
    if let Some(population) = &request.population {
        parts.push(format!("population {population}"));
    }
    if request.color_by
        && let Some(classes) = &sample.classes
    {
        parts.push(format!("coloured by {}", classes.title));
    }
    parts.push(format!("{} in file", count(stats.events)));
    parts.join(" · ")
}

/// Drop legend rows that would run past the plot area, ending with a count
/// of the hidden ones.
fn fit_legend(legend: &mut Vec<LegendEntry>, frame: Frame) {
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "the frame height is at most a few thousand pixels"
    )]
    let rows = ((frame.height - 8.0) / LEGEND_ROW_HEIGHT).floor().max(1.0) as usize;
    if legend.len() > rows {
        let hidden = legend.len() - rows + 1;
        legend.truncate(rows - 1);
        legend.push(LegendEntry {
            color: OTHER_COLOR.to_owned(),
            text: format!("+{hidden} more"),
        });
    }
}

/// `15000` → `15,000`.
pub(crate) fn count(value: usize) -> String {
    let digits = value.to_string();
    let mut grouped = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    grouped
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use cytiq_fcs::FcsReader;

    use super::*;

    /// Gate labels count `unmasked membership ∧ keep`; that must equal what
    /// `gate_sample_masked` (and so `cytiq gate --mask`) reports, for every
    /// gate kind including Boolean complements.
    #[test]
    fn masked_membership_equals_unmasked_and_keep() {
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/fixtures/gating");
        let dataset = FcsReader::open(fixtures.join("sample.fcs"))
            .and_then(FcsReader::read)
            .unwrap();
        let xml = std::fs::read_to_string(fixtures.join("gates.xml")).unwrap();
        let strategy = cytiq_gating::parse_gatingml(&xml).unwrap();
        let keep = (0..dataset.event_count())
            .map(|event| event % 4 != 1)
            .collect::<Vec<_>>();
        let unmasked = strategy.gate_sample_masked(&dataset, None).unwrap();
        let masked = strategy.gate_sample_masked(&dataset, Some(&keep)).unwrap();
        for gate in strategy.gates() {
            let derived = unmasked
                .membership(&gate.id)
                .unwrap()
                .iter()
                .zip(&keep)
                .map(|(member, kept)| *member && *kept)
                .collect::<Vec<_>>();
            assert_eq!(masked.membership(&gate.id).unwrap(), derived, "{}", gate.id);
            let count = masked.count(&gate.id).unwrap();
            let parent = masked.parent(&gate.id).map_or_else(
                || keep.iter().filter(|kept| **kept).count(),
                |parent| masked.count(parent).unwrap(),
            );
            #[allow(clippy::cast_precision_loss, reason = "test counts")]
            let expected = format!("{:.1}%", 100.0 * count as f64 / parent as f64);
            assert_eq!(
                percent_of_parent(&unmasked, &gate.id, Some(&keep), keep.len()),
                expected,
                "{}",
                gate.id
            );
        }
    }

    #[test]
    fn counts_have_thousands_separators() {
        assert_eq!(count(0), "0");
        assert_eq!(count(999), "999");
        assert_eq!(count(15_000), "15,000");
        assert_eq!(count(1_234_567), "1,234,567");
    }

    #[test]
    fn scatter_and_time_channels_are_recognised() {
        assert!(is_scatter_or_time("FSC-A"));
        assert!(is_scatter_or_time("ssc-h"));
        assert!(is_scatter_or_time("Time"));
        assert!(!is_scatter_or_time("FL1-A"));
    }
}
