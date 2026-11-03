//! `PlotSpec` → Plotly figure JSON (`{"data": [...], "layout": {...}}`),
//! for interactive viewers such as cytiq-web.
//!
//! Coordinates stay in the unit square; axis ticks are given explicitly
//! (`tickmode: "array"`), so logicle and transformed axes read the same as in
//! the SVG. Density is a heatmap of colour levels: in a browser that is fine,
//! but Plotly's own static export would rasterise it, which is why the SVG
//! comes from [`crate::svg`] instead.

use plotly::common::{
    Anchor, ColorScale, ColorScaleElement, Font, Line, Marker, MarkerSymbol, Mode, TickMode, Title,
};
use plotly::layout::{Annotation, Axis as PlotlyAxis, Margin};
use plotly::{HeatMap, Layout, Plot, Scatter};

use crate::axis::Tick;
use crate::spec::{Layer, PlotSpec, Polyline};
use crate::svg::escape;

/// Marker diameter of event dots, in pixels.
const MARKER_SIZE: usize = 3;

pub(crate) fn render(spec: &PlotSpec) -> String {
    let mut plot = Plot::new();
    for layer in &spec.layers {
        match layer {
            Layer::Density {
                bins,
                levels,
                colors,
            } => plot.add_trace(density(*bins, levels, colors)),
            Layer::Points {
                name,
                color,
                points,
            } => {
                let (x, y): (Vec<f64>, Vec<f64>) =
                    points.iter().map(|[x, y]| (round(*x), round(*y))).unzip();
                plot.add_trace(
                    Scatter::new(x, y)
                        .name(escape(name))
                        .mode(Mode::Markers)
                        .marker(
                            Marker::new()
                                .size(MARKER_SIZE)
                                .symbol(MarkerSymbol::Square)
                                .color(color.clone()),
                        ),
                );
            }
            Layer::Lines {
                name,
                color,
                width,
                paths,
            } => {
                let (x, y) = joined(paths);
                plot.add_trace(
                    Scatter::new(x, y)
                        .name(escape(name))
                        .mode(Mode::Lines)
                        .connect_gaps(false)
                        .line(Line::new().color(color.clone()).width(*width)),
                );
            }
        }
    }
    plot.set_layout(layout(spec));
    plot.to_json()
}

/// Colour levels as a heatmap over cell centres; empty cells are gaps.
fn density(
    bins: usize,
    levels: &[u16],
    colors: &[String],
) -> Box<HeatMap<f64, f64, Vec<Option<f64>>>> {
    #[allow(clippy::cast_precision_loss, reason = "bins is at most 1024")]
    let centres = (0..bins)
        .map(|cell| round((cell as f64 + 0.5) / bins as f64))
        .collect::<Vec<_>>();
    let z = levels
        .chunks(bins)
        .map(|row| {
            row.iter()
                .map(|&level| (level > 0).then_some(f64::from(level)))
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let span = colors.len().saturating_sub(1).max(1);
    #[allow(clippy::cast_precision_loss, reason = "few colour levels")]
    let scale = colors
        .iter()
        .enumerate()
        .map(|(index, color)| ColorScaleElement(index as f64 / span as f64, color.clone()))
        .collect();
    #[allow(clippy::cast_precision_loss, reason = "few colour levels")]
    let top = colors.len() as f64;
    HeatMap::new(centres.clone(), centres, z)
        .name("density")
        .color_scale(ColorScale::Vector(scale))
        .zmin(1.0)
        .zmax(top)
        .show_scale(false)
        .hover_on_gaps(false)
}

fn layout(spec: &PlotSpec) -> Layout {
    let annotations = spec
        .labels
        .iter()
        .map(|label| {
            Annotation::new()
                .x(round(label.position[0].clamp(0.0, 1.0)))
                .y(round(label.position[1].clamp(0.0, 1.0)))
                .text(escape(&label.text))
                .show_arrow(false)
                .x_anchor(Anchor::Left)
                .y_anchor(Anchor::Bottom)
                .font(Font::new().size(11))
        })
        .collect::<Vec<_>>();
    // Plotly reads a few HTML tags in text; everything from the data is
    // escaped so labels cannot inject markup.
    let mut subtitle = escape(&spec.subtitle);
    if !spec.notes.is_empty() {
        subtitle.push_str("<br>");
        subtitle.push_str(&escape(&spec.notes.join(" · ")));
    }
    Layout::new()
        .width(usize::try_from(spec.width).unwrap_or(usize::MAX))
        .height(usize::try_from(spec.height).unwrap_or(usize::MAX))
        .title(Title::with_text(format!(
            "{}<br><sub>{subtitle}</sub>",
            escape(&spec.title)
        )))
        .show_legend(!spec.legend.is_empty())
        .margin(Margin::new().left(78).right(24).top(80).bottom(54))
        .plot_background_color("#FFFFFF")
        .paper_background_color("#FFFFFF")
        .x_axis(axis(&spec.x.title, &spec.x.ticks))
        .y_axis(axis(&spec.y.title, &spec.y.ticks))
        .annotations(annotations)
}

fn axis(title: &str, ticks: &[Tick]) -> PlotlyAxis {
    let (values, text): (Vec<f64>, Vec<String>) = ticks
        .iter()
        .filter_map(|tick| {
            tick.label
                .as_ref()
                .map(|label| (round(tick.position), escape(label)))
        })
        .unzip();
    PlotlyAxis::new()
        .title(Title::with_text(escape(title)))
        .range(vec![0.0, 1.0])
        .tick_mode(TickMode::Array)
        .tick_values(values)
        .tick_text(text)
        .show_grid(false)
        .zero_line(false)
        .show_line(true)
        .mirror(true)
}

/// Polylines as one trace, separated by `null` gaps; closed ones repeat
/// their first point.
fn joined(paths: &[Polyline]) -> (Vec<Option<f64>>, Vec<Option<f64>>) {
    let mut x = Vec::new();
    let mut y = Vec::new();
    for path in paths {
        if !x.is_empty() {
            x.push(None);
            y.push(None);
        }
        let closing = path.closed.then(|| path.points.first()).flatten();
        for [px, py] in path.points.iter().chain(closing) {
            x.push(Some(round(*px)));
            y.push(Some(round(*py)));
        }
    }
    (x, y)
}

/// Unit coordinates rounded to 1e-5 (well under a pixel) to keep files small
/// and stable.
fn round(value: f64) -> f64 {
    (value * 1e5).round() / 1e5
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::{AxisSpec, Frame, Label};

    #[test]
    fn plotly_json_has_heatmap_and_array_ticks() {
        let spec = PlotSpec {
            name: "p".into(),
            title: "title".into(),
            subtitle: "subtitle".into(),
            notes: Vec::new(),
            width: 600,
            height: 560,
            frame: Frame::new(600, 560, false).unwrap(),
            x: AxisSpec {
                title: "FSC-A".into(),
                ticks: vec![Tick {
                    position: 0.5,
                    label: Some("100K".into()),
                    major: true,
                }],
            },
            y: AxisSpec {
                title: "SSC-A".into(),
                ticks: Vec::new(),
            },
            layers: vec![
                Layer::Density {
                    bins: 2,
                    levels: vec![0, 1, 2, 0],
                    colors: vec!["#000001".into(), "#000002".into()],
                },
                Layer::Lines {
                    name: "gate".into(),
                    color: "#1F2933".into(),
                    width: 1.5,
                    paths: vec![
                        Polyline {
                            points: vec![[0.1, 0.1], [0.2, 0.2]],
                            closed: true,
                        },
                        Polyline {
                            points: vec![[0.5, 0.5], [0.6, 0.6]],
                            closed: false,
                        },
                    ],
                },
            ],
            labels: vec![Label {
                position: [0.1, 0.2],
                text: "gate 10.0%".into(),
            }],
            legend: Vec::new(),
        };
        let json: serde_json::Value = serde_json::from_str(&render(&spec)).unwrap();
        assert_eq!(json["data"][0]["type"], "heatmap");
        assert_eq!(json["data"][0]["z"][0], serde_json::json!([null, 1.0]));
        assert_eq!(
            json["data"][1]["x"],
            serde_json::json!([0.1, 0.2, 0.1, null, 0.5, 0.6])
        );
        assert_eq!(json["layout"]["xaxis"]["tickmode"], "array");
        assert_eq!(
            json["layout"]["xaxis"]["tickvals"],
            serde_json::json!([0.5])
        );
        assert_eq!(
            json["layout"]["xaxis"]["ticktext"],
            serde_json::json!(["100K"])
        );
        assert_eq!(json["layout"]["annotations"][0]["text"], "gate 10.0%");
    }
}
