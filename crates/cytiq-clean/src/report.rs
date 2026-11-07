//! `report.html`: one panel per gate, plotted on the gate's own dimensions.

use askama::Template;

use cytiq_density::sample_indices;
use cytiq_fcs::FcsDataset;
use cytiq_gating::{Gate, GateKind, GatingResults, GatingStrategy};
use plotly::common::{Line, Marker, Mode, Title};
use plotly::layout::{Axis, BarMode};
use plotly::{Histogram, Layout, Plot, Scatter};

use crate::error::CleanError;
use crate::filter::{CLEAN_GATE, FilterReport};
use crate::runner::CleanMetrics;

/// Events drawn per scatter plot; enough to show the shape, small enough to
/// keep the HTML light.
const PLOT_EVENTS: usize = 10_000;
const PASS_COLOR: &str = "#4E79A7";
const FAIL_COLOR: &str = "#E15759";
const GATE_COLOR: &str = "#1F2933";

pub(crate) fn report_html(
    report_path: &std::path::Path,
    metrics: &CleanMetrics,
    strategy: &GatingStrategy,
    dataset: &FcsDataset,
    results: &GatingResults,
) -> Result<String, CleanError> {
    let sample = sample_indices(&vec![true; dataset.event_count()], PLOT_EVENTS);
    let mut panels = Vec::new();
    for gate in strategy.gates().iter().filter(|gate| gate.id != CLEAN_GATE) {
        if let Some(plot) = gate_plot(gate, strategy, dataset, results, &sample)? {
            panels.push(GatePanel {
                id: &gate.id,
                plot: PlotHtml(plot.to_inline_html(Some(&format!("gate-{}", gate.id)))),
            });
        }
    }
    let rows = [
        ("Debris", &metrics.filters.debris),
        ("Singlets", &metrics.filters.singlets),
        ("Viability", &metrics.filters.viability),
    ]
    .map(|(name, report)| FilterRow {
        name,
        report,
        channels: report.channels.join(", "),
        removed: match (report.removed, report.removed_fraction) {
            (Some(count), Some(fraction)) => format!("{count} ({:.2}%)", fraction * 100.0),
            _ => "—".into(),
        },
        notes: report.reason.clone().unwrap_or_else(|| {
            if report.parameters.is_null() {
                String::new()
            } else {
                report.parameters.to_string()
            }
        }),
    });
    CleanReport {
        metrics,
        rows,
        panels,
        plotly_js: PlotHtml(Plot::online_cdn_js()),
    }
    .render()
    .map_err(|error| CleanError::OutputWrite {
        path: report_path.to_path_buf(),
        source: std::io::Error::other(error),
    })
}

#[derive(Template)]
#[template(path = "report.html")]
struct CleanReport<'a> {
    metrics: &'a CleanMetrics,
    rows: [FilterRow<'a>; 3],
    panels: Vec<GatePanel<'a>>,
    plotly_js: PlotHtml,
}
struct FilterRow<'a> {
    name: &'static str,
    report: &'a FilterReport,
    channels: String,
    removed: String,
    notes: String,
}
struct GatePanel<'a> {
    id: &'a str,
    plot: PlotHtml,
}
// Only generated Plotly fragments use this private wrapper and template |safe.
struct PlotHtml(String);
impl std::fmt::Display for PlotHtml {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Scatter (2-D gates) or overlaid histograms (1-D gates) of the sampled
/// events, coloured by gate membership, with the gate boundary drawn on top.
fn gate_plot(
    gate: &Gate,
    strategy: &GatingStrategy,
    dataset: &FcsDataset,
    results: &GatingResults,
    sample: &[usize],
) -> Result<Option<Plot>, CleanError> {
    let Some(membership) = results.membership(&gate.id) else {
        return Ok(None);
    };
    let dimensions = gate.dimensions();
    let columns = dimensions
        .iter()
        .take(2)
        .map(|dimension| {
            strategy.dimension_values(dataset, dimension).map(|values| {
                sample
                    .iter()
                    .map(|&index| values[index])
                    .collect::<Vec<_>>()
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let inside = sample
        .iter()
        .map(|&index| membership[index])
        .collect::<Vec<_>>();
    let split = |values: &[f64], keep: bool| {
        values
            .iter()
            .zip(&inside)
            .filter_map(|(value, flag)| (*flag == keep).then_some(*value))
            .collect::<Vec<_>>()
    };
    let axis_title = |index: usize| {
        let dimension = &dimensions[index];
        dimension.transform.as_ref().map_or_else(
            || dimension.name.clone(),
            |transform| format!("{} ({transform})", dimension.name),
        )
    };

    let mut plot = Plot::new();
    match columns.as_slice() {
        [x] => {
            plot.add_trace(
                Histogram::new(split(x, true))
                    .name("inside")
                    .opacity(0.6)
                    .marker(Marker::new().color(PASS_COLOR)),
            );
            plot.add_trace(
                Histogram::new(split(x, false))
                    .name("outside")
                    .opacity(0.6)
                    .marker(Marker::new().color(FAIL_COLOR)),
            );
            let mut layout = Layout::new()
                .bar_mode(BarMode::Overlay)
                .x_axis(Axis::new().title(Title::with_text(axis_title(0))))
                .y_axis(Axis::new().title(Title::with_text("events")));
            for bound in [dimensions[0].min, dimensions[0].max].into_iter().flatten() {
                layout.add_shape(threshold_line(bound));
            }
            plot.set_layout(layout);
        }
        [x, y] => {
            for (keep, name, color) in
                [(true, "inside", PASS_COLOR), (false, "outside", FAIL_COLOR)]
            {
                plot.add_trace(
                    Scatter::new(split(x, keep), split(y, keep))
                        .mode(Mode::Markers)
                        .name(name)
                        .marker(Marker::new().color(color).size(3).opacity(0.5)),
                );
            }
            if let Some(outline) = outline(gate) {
                plot.add_trace(
                    Scatter::new(
                        outline.iter().map(|point| point[0]).collect(),
                        outline.iter().map(|point| point[1]).collect(),
                    )
                    .mode(Mode::Lines)
                    .name("gate")
                    .line(Line::new().color(GATE_COLOR).width(2.0)),
                );
            }
            plot.set_layout(
                Layout::new()
                    .x_axis(Axis::new().title(Title::with_text(axis_title(0))))
                    .y_axis(Axis::new().title(Title::with_text(axis_title(1)))),
            );
        }
        _ => return Ok(None),
    }
    Ok(Some(plot))
}

/// Closed outline of a 2-D polygon gate.
fn outline(gate: &Gate) -> Option<Vec<[f64; 2]>> {
    let GateKind::Polygon { vertices, .. } = &gate.kind else {
        return None;
    };
    let mut closed = vertices.clone();
    closed.push(*vertices.first()?);
    Some(closed)
}

fn threshold_line(x: f64) -> plotly::layout::Shape {
    use plotly::layout::{Shape, ShapeLine, ShapeType};
    Shape::new()
        .shape_type(ShapeType::Line)
        .x0(x)
        .x1(x)
        .y0(0)
        .y1(1)
        .y_ref("paper")
        .line(ShapeLine::new().color(GATE_COLOR).width(2.0))
}
