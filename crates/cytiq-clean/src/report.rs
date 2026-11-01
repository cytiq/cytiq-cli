//! `report.html`: one panel per gate, plotted on the gate's own dimensions.

use std::fmt::Write as _;

use cytiq_fcs::FcsDataset;
use cytiq_gating::{Gate, GateKind, GatingResults, GatingStrategy};
use plotly::common::{Line, Marker, Mode, Title};
use plotly::layout::{Axis, BarMode};
use plotly::{Histogram, Layout, Plot, Scatter};

use crate::error::CleanError;
use crate::filter::{CLEAN_GATE, FilterReport, sample_indices};
use crate::runner::CleanMetrics;

/// Events drawn per scatter plot; enough to show the shape, small enough to
/// keep the HTML light.
const PLOT_EVENTS: usize = 10_000;
const PASS_COLOR: &str = "#4E79A7";
const FAIL_COLOR: &str = "#E15759";
const GATE_COLOR: &str = "#1F2933";

pub(crate) fn report_html(
    metrics: &CleanMetrics,
    strategy: &GatingStrategy,
    dataset: &FcsDataset,
    results: &GatingResults,
) -> Result<String, CleanError> {
    let sample = sample_indices(&vec![true; dataset.event_count()], PLOT_EVENTS);
    let mut panels = String::new();
    for gate in strategy.gates().iter().filter(|gate| gate.id != CLEAN_GATE) {
        let Some(plot) = gate_plot(gate, strategy, dataset, results, &sample)? else {
            continue;
        };
        let _ = write!(
            panels,
            "<section class=\"plot-panel\"><h2>Gate <code>{}</code></h2>{}</section>",
            escape_html(&gate.id),
            plot.to_inline_html(Some(&format!("gate-{}", gate.id)))
        );
    }

    let rows = [
        ("Debris", &metrics.filters.debris),
        ("Singlets", &metrics.filters.singlets),
        ("Viability", &metrics.filters.viability),
    ]
    .into_iter()
    .map(|(name, report)| filter_row(name, report))
    .collect::<String>();
    let overlap = if metrics.removal_overlap.is_empty() {
        "<tr><td colspan=\"2\">No events removed</td></tr>".to_owned()
    } else {
        metrics
            .removal_overlap
            .iter()
            .fold(String::new(), |mut rows, (filters, count)| {
                let _ = write!(
                    rows,
                    "<tr><td>{}</td><td>{count}</td></tr>",
                    escape_html(filters)
                );
                rows
            })
    };

    let html = format!(
        r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>Clean Report</title>
{plotly_js}
<style>
body {{ font-family: system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif; margin: 2rem; color: #1f2933; }}
table {{ border-collapse: collapse; margin-top: 1rem; }}
td, th {{ border: 1px solid #d9e2ec; padding: 0.45rem 0.65rem; text-align: left; vertical-align: top; }}
th {{ background: #f0f4f8; }}
code {{ background: #f0f4f8; padding: 0.1rem 0.25rem; }}
.status-applied {{ color: #1b7f3b; }}
.status-not_assessed {{ color: #b7791f; }}
.status-disabled {{ color: #616e7c; }}
.plot-panel {{ margin-top: 1.5rem; border: 1px solid #d9e2ec; padding: 1rem; }}
.plot-panel h2 {{ margin-top: 0; }}
.js-plotly-plot, .plotly, .plotly-graph-div {{ width: 100% !important; }}
</style>
</head>
<body>
<h1>Clean Report</h1>
<p><strong>Input:</strong> <code>{input}</code></p>
<p><strong>Gates:</strong> {source}</p>
<p><strong>Kept:</strong> {kept} of {events} events ({kept_percent:.2}%), rule: <code>{rule}</code></p>
<h2>Filters</h2>
<table>
<tr><th>Filter</th><th>Status</th><th>Method</th><th>Channels</th><th>Removed</th><th>Notes</th></tr>
{rows}
</table>
<h2>Removed events by filter combination</h2>
<table>
<tr><th>Removed by</th><th>Events</th></tr>
{overlap}
</table>
{panels}
</body>
</html>
"#,
        plotly_js = Plot::online_cdn_js(),
        input = escape_html(&metrics.input),
        source = escape_html(&metrics.gates_source),
        kept = metrics.kept,
        events = metrics.event_count,
        kept_percent = metrics.kept_fraction * 100.0,
        rule = escape_html(&metrics.keep_rule),
    );
    Ok(html)
}

fn filter_row(name: &str, report: &FilterReport) -> String {
    let status = serde_json::to_value(report.status)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_default();
    let removed = match (report.removed, report.removed_fraction) {
        (Some(count), Some(fraction)) => format!("{count} ({:.2}%)", fraction * 100.0),
        _ => "&mdash;".to_owned(),
    };
    let notes = report.reason.clone().unwrap_or_else(|| {
        if report.parameters.is_null() {
            String::new()
        } else {
            report.parameters.to_string()
        }
    });
    format!(
        "<tr><td>{name}</td><td class=\"status-{status}\">{status}</td><td>{}</td>\
         <td>{}</td><td>{removed}</td><td><code>{}</code></td></tr>",
        report.method.unwrap_or("&mdash;"),
        escape_html(&report.channels.join(", ")),
        escape_html(&notes)
    )
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

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
