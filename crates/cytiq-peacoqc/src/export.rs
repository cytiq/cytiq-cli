use std::fs;
use std::path::Path;

use cytiq_mask::EventMask;
use plotly::common::{Mode, Title};
use plotly::{Bar, Layout, Plot, Scatter};

use crate::error::PeacoQcError;
use crate::runner::{EventMaskRecord, PeacoQcSummary};

/// Write `PeacoQC` debug artifacts to `debug_dir`.
///
/// # Errors
///
/// Returns an error if the directory cannot be created or any artifact cannot
/// be written.
pub fn write_debug_artifacts(
    summary: &PeacoQcSummary,
    mask: &[EventMaskRecord],
    debug_dir: &Path,
) -> Result<(), PeacoQcError> {
    fs::create_dir_all(debug_dir).map_err(|source| PeacoQcError::DebugDirectoryCreate {
        path: debug_dir.to_path_buf(),
        source,
    })?;
    write_summary_json(&debug_dir.join("summary.json"), summary)?;
    write_mask_csv(&debug_dir.join("mask.csv"), mask)?;
    write_report_html(&debug_dir.join("report.html"), summary, mask)
}

/// One row per input event, in the schema of `cytiq clean`'s masks: `event_id`
/// is the 0-based input row, `source_event_id` the 0-based raw-file row (from
/// `Original_ID` when the input has one). A null filter column means the
/// filter did not assess the event, never that it passed.
pub(crate) fn write_masks_parquet(
    path: &Path,
    acquisition: &str,
    mask: &[EventMaskRecord],
    source_event_ids: Option<&[u64]>,
    margins_assessed: bool,
) -> Result<(), PeacoQcError> {
    let margin_pass = mask
        .iter()
        .map(|record| margins_assessed.then_some(!record.removed_by_margin))
        .collect();
    let peacoqc_pass = mask
        .iter()
        .map(|record| {
            (!record.removed_by_margin).then_some(
                !(record.removed_by_mad
                    || record.removed_by_it
                    || record.removed_by_consecutive_bins),
            )
        })
        .collect();
    EventMask::new(
        acquisition,
        mask.iter().map(|record| record.is_good).collect(),
        source_event_ids.map(<[u64]>::to_vec),
    )
    .with_filter("margin_pass", margin_pass)
    .with_filter("peacoqc_pass", peacoqc_pass)
    .write_parquet(path)
    .map_err(|error| PeacoQcError::ParquetWrite {
        path: path.to_path_buf(),
        message: error.to_string(),
    })
}

fn write_summary_json(path: &Path, summary: &PeacoQcSummary) -> Result<(), PeacoQcError> {
    let bytes =
        serde_json::to_vec_pretty(summary).map_err(|source| PeacoQcError::JsonSerialize {
            path: path.to_path_buf(),
            source,
        })?;
    fs::write(path, bytes).map_err(|source| PeacoQcError::JsonWrite {
        path: path.to_path_buf(),
        source,
    })
}

fn write_mask_csv(path: &Path, mask: &[EventMaskRecord]) -> Result<(), PeacoQcError> {
    let mut writer = csv::Writer::from_path(path).map_err(|source| PeacoQcError::CsvWrite {
        path: path.to_path_buf(),
        source,
    })?;
    for record in mask {
        writer
            .serialize(record)
            .map_err(|source| PeacoQcError::CsvWrite {
                path: path.to_path_buf(),
                source,
            })?;
    }
    writer.flush().map_err(|source| PeacoQcError::CsvFlush {
        path: path.to_path_buf(),
        source,
    })
}

fn write_report_html(
    path: &Path,
    summary: &PeacoQcSummary,
    mask: &[EventMaskRecord],
) -> Result<(), PeacoQcError> {
    let plots = report_plots(summary, mask);
    let metrics = &summary.metrics;
    let warnings = if summary.warnings.is_empty() {
        "<li>None</li>".to_owned()
    } else {
        summary
            .warnings
            .iter()
            .map(|warning| format!("<li>{}</li>", escape_html(warning)))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let html = format!(
        r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>PeacoQC Report</title>
{plotly_js}
<style>
body {{ font-family: system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif; margin: 2rem; color: #1f2933; }}
table {{ border-collapse: collapse; margin-top: 1rem; }}
td, th {{ border: 1px solid #d9e2ec; padding: 0.45rem 0.65rem; text-align: left; }}
th {{ background: #f0f4f8; }}
code {{ background: #f0f4f8; padding: 0.1rem 0.25rem; }}
.plot-panel {{ margin-top: 1.5rem; border: 1px solid #d9e2ec; padding: 1rem; }}
.plot-panel h2 {{ margin-top: 0; }}
.js-plotly-plot, .plotly, .plotly-graph-div {{ width: 100% !important; }}
</style>
</head>
<body>
<h1>PeacoQC Report</h1>
<p><strong>Status:</strong> {status}</p>
<p><strong>Input:</strong> <code>{input}</code></p>
<p><strong>Output:</strong> <code>{output}</code></p>
<p><strong>Channels:</strong> {channels}</p>
<table>
<tr><th>Metric</th><th>Value</th></tr>
<tr><td>Events before</td><td>{events_before}</td></tr>
<tr><td>Events after</td><td>{events_after}</td></tr>
<tr><td>Removed total</td><td>{removed_total} ({removed_fraction:.4})</td></tr>
<tr><td>Removed by margins</td><td>{margin_count} ({margin_fraction:.4})</td></tr>
<tr><td>Removed by MAD</td><td>{mad_count} ({mad_fraction:.4})</td></tr>
<tr><td>Removed by IT</td><td>{it_count} ({it_fraction:.4})</td></tr>
<tr><td>Removed by consecutive bins</td><td>{consecutive_count} ({consecutive_fraction:.4})</td></tr>
<tr><td>Bins</td><td>{bins}</td></tr>
<tr><td>Events per bin</td><td>{events_per_bin}</td></tr>
<tr><td>Peak features</td><td>{peak_features}</td></tr>
</table>
<h2>Warnings</h2>
<ul>
{warnings}
</ul>
<h2>Plots</h2>
{plots}
</body>
</html>
"#,
        plotly_js = Plot::online_cdn_js(),
        status = escape_html(&summary.status),
        input = escape_html(&summary.input),
        output = escape_html(&summary.output),
        channels = escape_html(&summary.channels.join(", ")),
        events_before = metrics.events_before,
        events_after = metrics.events_after,
        removed_total = metrics.removed_total,
        removed_fraction = metrics.removed_total_fraction,
        margin_count = metrics.margin_removed_count,
        margin_fraction = metrics.margin_removed_fraction,
        mad_count = metrics.mad_removed_event_count,
        mad_fraction = metrics.mad_removed_fraction,
        it_count = metrics.it_removed_event_count,
        it_fraction = metrics.it_removed_fraction,
        consecutive_count = metrics.consecutive_removed_event_count,
        consecutive_fraction = metrics.consecutive_removed_fraction,
        bins = metrics.n_bins,
        events_per_bin = metrics.events_per_bin,
        peak_features = metrics.peak_feature_count,
        warnings = warnings,
        plots = plots
    );
    fs::write(path, html).map_err(|source| PeacoQcError::HtmlWrite {
        path: path.to_path_buf(),
        source,
    })
}

fn report_plots(summary: &PeacoQcSummary, mask: &[EventMaskRecord]) -> String {
    let mut panels = Vec::new();
    panels.push(plot_panel(
        "Removal summary",
        &removal_summary_plot(summary).to_inline_html(Some("removal-summary")),
    ));
    panels.push(plot_panel(
        "Removed bins",
        &removed_bins_plot(summary).to_inline_html(Some("removed-bins")),
    ));
    if !mask.is_empty() {
        panels.push(plot_panel(
            "Event mask",
            &event_mask_plot(mask).to_inline_html(Some("event-mask")),
        ));
    }
    panels.join("\n")
}

fn plot_panel(title: &str, plot_html: &str) -> String {
    format!(
        "<section class=\"plot-panel\"><h2>{}</h2>{}</section>",
        escape_html(title),
        plot_html
    )
}

fn removal_summary_plot(summary: &PeacoQcSummary) -> Plot {
    let metrics = &summary.metrics;
    let x = vec![
        "margins".to_owned(),
        "MAD".to_owned(),
        "IT".to_owned(),
        "consecutive".to_owned(),
    ];
    let y = vec![
        metrics.margin_removed_count,
        metrics.mad_removed_event_count,
        metrics.it_removed_event_count,
        metrics.consecutive_removed_event_count,
    ];
    let mut plot = Plot::new();
    plot.add_trace(Bar::new(x, y).name("removed events"));
    plot.set_layout(Layout::new().title(Title::from("Removed events by reason")));
    plot
}

fn removed_bins_plot(summary: &PeacoQcSummary) -> Plot {
    let metrics = &summary.metrics;
    let bin_ids = (1..=metrics.n_bins).collect::<Vec<_>>();
    let mad = bin_indicator(&bin_ids, &metrics.mad_outlier_bins);
    let it = bin_indicator(&bin_ids, &metrics.it_outlier_bins);
    let consecutive = bin_indicator(&bin_ids, &metrics.consecutive_removed_bins);

    let mut plot = Plot::new();
    plot.add_trace(
        Scatter::new(bin_ids.clone(), mad)
            .mode(Mode::LinesMarkers)
            .name("MAD"),
    );
    plot.add_trace(
        Scatter::new(bin_ids.clone(), it)
            .mode(Mode::LinesMarkers)
            .name("IT"),
    );
    plot.add_trace(
        Scatter::new(bin_ids, consecutive)
            .mode(Mode::LinesMarkers)
            .name("consecutive"),
    );
    plot.set_layout(Layout::new().title(Title::from("Removed bins over acquisition order")));
    plot
}

fn bin_indicator(bin_ids: &[usize], removed_bins: &[usize]) -> Vec<usize> {
    bin_ids
        .iter()
        .map(|bin_id| usize::from(removed_bins.contains(bin_id)))
        .collect()
}

fn event_mask_plot(mask: &[EventMaskRecord]) -> Plot {
    let sampled = sampled_mask(mask);
    let event_ids = sampled
        .iter()
        .map(|record| record.event_id)
        .collect::<Vec<_>>();
    let good = sampled
        .iter()
        .map(|record| usize::from(record.is_good))
        .collect::<Vec<_>>();
    let removed = sampled
        .iter()
        .map(|record| usize::from(!record.is_good))
        .collect::<Vec<_>>();

    let mut plot = Plot::new();
    plot.add_trace(
        Scatter::new(event_ids.clone(), good)
            .mode(Mode::Markers)
            .name("good"),
    );
    plot.add_trace(
        Scatter::new(event_ids, removed)
            .mode(Mode::Markers)
            .name("removed"),
    );
    plot.set_layout(Layout::new().title(Title::from("Sampled event mask")));
    plot
}

fn sampled_mask(mask: &[EventMaskRecord]) -> Vec<EventMaskRecord> {
    const MAX_POINTS: usize = 5_000;
    if mask.len() <= MAX_POINTS {
        return mask.to_vec();
    }
    let step = mask.len().div_ceil(MAX_POINTS);
    mask.iter().step_by(step).cloned().collect()
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
