//! `report.html`: SSI heatmaps coloured as `CompensAID::PlotMatrix` (orange at
//! `SSI <= -1`, blue at `SSI >= 1`), the batch recurrence view and dot plots
//! of the strongest flags, in the spirit of `CompensAID::PlotDotSSI`.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::too_many_lines
)]

use std::fmt::Write as _;
use std::path::Path;

use plotly::common::{ColorScale, ColorScaleElement, DashType, Line, Marker, Mode, Title};
use plotly::layout::Axis;
use plotly::{HeatMap, Layout, Plot, Scatter};

use crate::batch::{BatchRow, BatchStatus};
use crate::compensaid::{CompensaidResult, FLAG_THRESHOLD, PairAssessment, SsiStatus};
use crate::error::SpillQcError;
use crate::export::write_bytes;

/// Dot plots are drawn for runs of at most this many assessed files.
pub(crate) const DOT_PLOT_MAX_FILES: usize = 25;
const DOT_PLOTS_PER_FILE: usize = 4;
const DOT_PLOT_MAX_EVENTS: usize = 5_000;
const OVER_COLOUR: &str = "#ef8a62";
const UNDER_COLOUR: &str = "#67a9cf";

const REPORT_CSS: &str = r"
body { margin: 0; background: #f4f6f9; color: #1d2b3a; font-family: 'Helvetica Neue', Arial, sans-serif; }
.shell { max-width: 1200px; margin: 0 auto; padding: 24px 16px; }
header { background: #1c4f78; color: #f8fbff; border-radius: 16px; padding: 24px 28px; }
header h1 { margin: 6px 0; font-size: 30px; }
header p { margin: 4px 0; opacity: 0.9; }
nav a { color: #f8fbff; margin-right: 14px; font-size: 14px; }
section { background: #fff; border-radius: 16px; padding: 20px 24px; margin-top: 20px; box-shadow: 0 6px 18px rgba(28, 79, 120, 0.08); }
h2 { margin-top: 0; }
table { border-collapse: collapse; width: 100%; font-size: 14px; margin: 12px 0; }
th, td { text-align: left; padding: 6px 8px; border-bottom: 1px solid #e3e8ef; }
.over { color: #b44d22; font-weight: 600; }
.under { color: #2c6e97; font-weight: 600; }
.note { background: #fff6e5; border-left: 4px solid #e0a133; padding: 10px 14px; border-radius: 6px; }
.panels { display: grid; grid-template-columns: repeat(auto-fit, minmax(420px, 1fr)); gap: 12px; }
";

/// A flagged pair as the report lists it.
pub(crate) struct FlagRow {
    pub primary: String,
    pub secondary: String,
    pub ssi: Option<f64>,
    pub status: SsiStatus,
}

/// The plots of one assessed file, rendered to HTML while its data is still
/// in memory (Plotly plots are not `Send`).
pub(crate) struct FileReport {
    pub name: String,
    pub heatmap: String,
    pub flags: Vec<FlagRow>,
    pub dot_plots: Vec<(String, String)>,
}

/// Build the plots of one assessed file. `columns` are the assessed marker
/// values; dot plots are skipped when `with_dot_plots` is false.
pub(crate) fn file_report(
    name: &str,
    result: &CompensaidResult,
    columns: &[Vec<f64>],
    with_dot_plots: bool,
) -> FileReport {
    let labels = result
        .channels
        .iter()
        .map(|channel| format!("{} ({})", channel.marker, channel.channel))
        .collect::<Vec<_>>();
    let z = (0..result.channels.len())
        .map(|secondary| {
            (0..result.channels.len())
                .map(|primary| result.cell(secondary, primary).and_then(|pair| pair.ssi))
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let plot_id = |index: usize| format!("{}-plot-{index}", anchor(name));
    let heatmap = ssi_heatmap(
        &format!("SSI matrix: {name}"),
        &labels,
        &labels,
        z,
        "SSI %{z}<extra>primary %{x}<br>secondary %{y}</extra>",
    )
    .to_inline_html(Some(&plot_id(0)));

    let mut dot_plots = Vec::new();
    if with_dot_plots {
        let mut flagged = result
            .pairs
            .iter()
            .filter(|pair| pair.status.is_flag())
            .collect::<Vec<_>>();
        flagged.sort_by(|a, b| {
            let strength = |pair: &PairAssessment| pair.ssi.map_or(0.0, f64::abs);
            strength(b).total_cmp(&strength(a))
        });
        for pair in flagged.into_iter().take(DOT_PLOTS_PER_FILE) {
            let title = format!(
                "{} -> {} (SSI {})",
                result.channels[pair.primary].marker,
                result.channels[pair.secondary].marker,
                pair.ssi
                    .map_or_else(|| "NA".to_owned(), |ssi| format!("{ssi:.2}")),
            );
            let html =
                dot_plot(result, pair, columns).to_inline_html(Some(&plot_id(dot_plots.len() + 1)));
            dot_plots.push((title, html));
        }
    }
    let flags = result
        .pairs
        .iter()
        .filter(|pair| pair.status.is_flag())
        .map(|pair| FlagRow {
            primary: result.channels[pair.primary].marker.clone(),
            secondary: result.channels[pair.secondary].marker.clone(),
            ssi: pair.ssi,
            status: pair.status,
        })
        .collect();
    FileReport {
        name: name.to_owned(),
        heatmap,
        flags,
        dot_plots,
    }
}

/// Write the report. `skipped` pairs a file name with the reason.
pub(crate) fn write_report(
    path: &Path,
    summary_line: &str,
    files: &[&FileReport],
    batch: &[BatchRow],
    skipped: &[(String, String)],
) -> Result<(), SpillQcError> {
    let mut html = String::new();
    let _ = write!(
        html,
        "<!doctype html><html><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"><title>Spillover QC</title><style>{REPORT_CSS}</style>{}</head><body><div class=\"shell\"><header><div>cytiq spillqc · CompensAID</div><h1>Spillover QC</h1><p>{}</p><nav><a href=\"#batch\">Batch</a>",
        Plot::online_cdn_js(),
        escape(summary_line),
    );
    for file in files {
        let _ = write!(
            html,
            "<a href=\"#{}\">{}</a>",
            anchor(&file.name),
            escape(&file.name)
        );
    }
    html.push_str("</nav></header>");
    html.push_str(
        "<section><p class=\"note\">Flags mark marker pairs whose positive population is skewed in the secondary channel (|SSI| &ge; 1). CompensAID reports high sensitivity but low precision (0.16&ndash;0.27 against expert review): every flag needs review on a dot plot before a spillover value is changed. Pairs with real co-expression can be flagged as under-compensated.</p></section>",
    );

    html.push_str("<section id=\"batch\"><h2>Batch</h2>");
    let recurrent = batch
        .iter()
        .filter(|row| row.status.is_recurrent())
        .collect::<Vec<_>>();
    if recurrent.is_empty() {
        html.push_str("<p>No pair is flagged recurrently in files sharing a matrix.</p>");
    } else {
        html.push_str("<table><tr><th>Matrix</th><th>Primary</th><th>Secondary</th><th>Flagged / assessed</th><th>Median SSI</th><th>Status</th></tr>");
        for row in recurrent {
            let flagged = if row.status == BatchStatus::RecurrentOver {
                row.files_over
            } else {
                row.files_under
            };
            let _ = write!(
                html,
                "<tr><td>{}</td><td>{}</td><td>{}</td><td>{flagged} / {}</td><td>{}</td><td class=\"{}\">{}</td></tr>",
                escape(short_id(&row.matrix_id)),
                escape(&row.primary_marker),
                escape(&row.secondary_marker),
                row.files_assessed,
                row.median_ssi
                    .map_or_else(|| "NA".to_owned(), |ssi| format!("{ssi:.2}")),
                if row.status == BatchStatus::RecurrentOver {
                    "over"
                } else {
                    "under"
                },
                row.status.as_str(),
            );
        }
        html.push_str("</table>");
    }
    for (index, (matrix_id, plot)) in batch_heatmaps(batch).into_iter().enumerate() {
        let _ = write!(
            html,
            "<h3>Matrix {}</h3>{}",
            escape(short_id(&matrix_id)),
            plot.to_inline_html(Some(&format!("batch-plot-{index}")))
        );
    }
    html.push_str("</section>");

    for file in files {
        let _ = write!(
            html,
            "<section id=\"{}\"><h2>{}</h2>{}",
            anchor(&file.name),
            escape(&file.name),
            file.heatmap
        );
        if file.flags.is_empty() {
            html.push_str("<p>No pair is flagged.</p>");
        } else {
            html.push_str(
                "<table><tr><th>Primary</th><th>Secondary</th><th>SSI</th><th>Status</th></tr>",
            );
            for flag in &file.flags {
                let _ = write!(
                    html,
                    "<tr><td>{}</td><td>{}</td><td>{}</td><td class=\"{}\">{}</td></tr>",
                    escape(&flag.primary),
                    escape(&flag.secondary),
                    flag.ssi
                        .map_or_else(|| "NA".to_owned(), |ssi| format!("{ssi:.2}")),
                    flag.status.as_str(),
                    flag.status.as_str(),
                );
            }
            html.push_str("</table>");
        }
        if !file.dot_plots.is_empty() {
            html.push_str("<div class=\"panels\">");
            for (title, plot) in &file.dot_plots {
                let _ = write!(html, "<div><h3>{}</h3>{plot}</div>", escape(title));
            }
            html.push_str("</div>");
        }
        html.push_str("</section>");
    }

    if !skipped.is_empty() {
        html.push_str(
            "<section><h2>Skipped files</h2><table><tr><th>File</th><th>Reason</th></tr>",
        );
        for (name, reason) in skipped {
            let _ = write!(
                html,
                "<tr><td>{}</td><td>{}</td></tr>",
                escape(name),
                escape(reason)
            );
        }
        html.push_str("</table></section>");
    }
    html.push_str("</div></body></html>\n");
    write_bytes(path, html.as_bytes())
}

/// Heatmaps of the share of assessed files flagging each pair, signed
/// (negative for over-compensation), one per matrix group with assessments.
fn batch_heatmaps(batch: &[BatchRow]) -> Vec<(String, Plot)> {
    let mut groups: Vec<&str> = Vec::new();
    for row in batch {
        if !groups.contains(&row.matrix_id.as_str()) {
            groups.push(&row.matrix_id);
        }
    }
    groups
        .into_iter()
        .filter_map(|matrix_id| {
            let rows = batch
                .iter()
                .filter(|row| row.matrix_id == matrix_id)
                .collect::<Vec<_>>();
            let files = rows.iter().map(|row| row.files_assessed).max().unwrap_or(0);
            if files < 2 {
                return None;
            }
            let mut channels: Vec<(&str, &str)> = Vec::new();
            for row in &rows {
                for channel in [
                    (row.primary_channel.as_str(), row.primary_marker.as_str()),
                    (
                        row.secondary_channel.as_str(),
                        row.secondary_marker.as_str(),
                    ),
                ] {
                    if !channels.contains(&channel) {
                        channels.push(channel);
                    }
                }
            }
            let labels = channels
                .iter()
                .map(|(channel, marker)| format!("{marker} ({channel})"))
                .collect::<Vec<_>>();
            let z = channels
                .iter()
                .map(|(secondary, _)| {
                    channels
                        .iter()
                        .map(|(primary, _)| {
                            rows.iter()
                                .find(|row| {
                                    row.primary_channel == *primary
                                        && row.secondary_channel == *secondary
                                })
                                .filter(|row| row.files_assessed > 0)
                                .map(|row| {
                                    2.0 * (row.files_under as f64 - row.files_over as f64)
                                        / row.files_assessed as f64
                                })
                        })
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>();
            let title =
                format!("Flag share, {files} files (orange: over, blue: under; ±2 = every file)");
            Some((
                matrix_id.to_owned(),
                ssi_heatmap(
                    &title,
                    &labels,
                    &labels,
                    z,
                    "%{z}<extra>primary %{x}<br>secondary %{y}</extra>",
                ),
            ))
        })
        .collect()
}

/// A heatmap white within `(-1, 1)` and shading to orange below and blue
/// above, as `PlotMatrix` does.
fn ssi_heatmap(
    title: &str,
    columns: &[String],
    rows: &[String],
    z: Vec<Vec<Option<f64>>>,
    hover: &str,
) -> Plot {
    let extreme = z
        .iter()
        .flatten()
        .flatten()
        .filter(|value| value.is_finite())
        .fold(2.0_f64, |acc, value| acc.max(value.abs()));
    let position = |value: f64| (value + extreme) / (2.0 * extreme);
    let scale = ColorScale::Vector(vec![
        ColorScaleElement(0.0, OVER_COLOUR.to_owned()),
        ColorScaleElement(position(-FLAG_THRESHOLD), "#fbe0d3".to_owned()),
        ColorScaleElement(position(-FLAG_THRESHOLD) + 1e-6, "#ffffff".to_owned()),
        ColorScaleElement(position(FLAG_THRESHOLD) - 1e-6, "#ffffff".to_owned()),
        ColorScaleElement(position(FLAG_THRESHOLD), "#d9ebf5".to_owned()),
        ColorScaleElement(1.0, UNDER_COLOUR.to_owned()),
    ]);
    // Plotly draws the first row at the bottom; list the first secondary on top
    let rows = rows.iter().rev().cloned().collect::<Vec<_>>();
    let z = z.into_iter().rev().collect::<Vec<_>>();
    let mut plot = Plot::new();
    plot.add_trace(
        HeatMap::new(columns.to_vec(), rows, z)
            .color_scale(scale)
            .zmin(-extreme)
            .zmax(extreme)
            .hover_template(hover),
    );
    let size = 160.0 + 34.0 * columns.len() as f64;
    plot.set_layout(
        Layout::new()
            .title(Title::with_text(title))
            .height(size.min(900.0) as usize)
            .x_axis(Axis::new().title(Title::with_text("primary")))
            .y_axis(Axis::new().title(Title::with_text("secondary"))),
    );
    plot
}

/// Primary against secondary for every assessed event (evenly thinned), with
/// the cut-offs and the segment edges.
fn dot_plot(result: &CompensaidResult, pair: &PairAssessment, columns: &[Vec<f64>]) -> Plot {
    let x = &columns[pair.primary];
    let y = &columns[pair.secondary];
    let step = x.len().div_ceil(DOT_PLOT_MAX_EVENTS).max(1);
    let (mut negative, mut positive, mut other) = (
        (Vec::new(), Vec::new()),
        (Vec::new(), Vec::new()),
        (Vec::new(), Vec::new()),
    );
    for event in (0..x.len()).step_by(step) {
        let target = if y[event] > pair.secondary_cutoff {
            &mut other
        } else if x[event] <= pair.primary_cutoff_neg {
            &mut negative
        } else if x[event] >= pair.primary_cutoff_pos {
            &mut positive
        } else {
            &mut other
        };
        target.0.push(x[event]);
        target.1.push(y[event]);
    }
    let mut plot = Plot::new();
    for ((xs, ys), name, colour) in [
        (other, "other", "#c3cbd6"),
        (negative, "primary negative", "#4b5d73"),
        (positive, "primary positive", OVER_COLOUR),
    ] {
        plot.add_trace(
            Scatter::new(xs, ys)
                .mode(Mode::Markers)
                .name(name)
                .marker(Marker::new().size(3).color(colour)),
        );
    }
    let (x_low, x_high) = extremes(x);
    let (y_low, y_high) = extremes(y);
    let mut line = |xs: Vec<f64>, ys: Vec<f64>, name: &str, dash: DashType| {
        plot.add_trace(
            Scatter::new(xs, ys)
                .mode(Mode::Lines)
                .name(name)
                .line(Line::new().color("#1d2b3a").width(1.0).dash(dash)),
        );
    };
    line(
        vec![x_low, x_high],
        vec![pair.secondary_cutoff, pair.secondary_cutoff],
        "secondary cut-off",
        DashType::Solid,
    );
    for cut in [pair.primary_cutoff_neg, pair.primary_cutoff_pos] {
        line(
            vec![cut, cut],
            vec![y_low, y_high],
            "primary cut-off",
            DashType::Solid,
        );
    }
    for edge in pair
        .segments
        .iter()
        .filter_map(|segment| segment.segment_max)
    {
        line(
            vec![edge, edge],
            vec![y_low, y_high],
            "segment edge",
            DashType::Dot,
        );
    }
    plot.set_layout(
        Layout::new()
            .height(420)
            .show_legend(false)
            .x_axis(Axis::new().title(Title::with_text(format!(
                "{} (primary)",
                result.channels[pair.primary].marker
            ))))
            .y_axis(Axis::new().title(Title::with_text(format!(
                "{} (secondary)",
                result.channels[pair.secondary].marker
            )))),
    );
    plot
}

fn extremes(values: &[f64]) -> (f64, f64) {
    values
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(low, high), value| {
            (low.min(*value), high.max(*value))
        })
}

fn short_id(id: &str) -> &str {
    &id[..id.len().min(12)]
}

fn anchor(name: &str) -> String {
    let cleaned = name
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character
            } else {
                '-'
            }
        })
        .collect::<String>();
    format!("file-{cleaned}")
}

fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
