//! `PlotSpec` → vector SVG.
//!
//! Output is byte-deterministic: fixed number formatting, layers in spec
//! order, no timestamps. Density cells become one `<path>` per colour level;
//! points one `<path>` per colour. No raster images, scripts or external
//! references are ever written, and [`check`] enforces it before publishing.

use std::fmt::Write as _;

use crate::error::PlotError;
use crate::palette::{GATE_COLOR, MUTED_TEXT_COLOR, NOTE_COLOR, TEXT_COLOR};
use crate::spec::{Frame, LEGEND_ROW_HEIGHT, Label, Layer, LegendEntry, PlotSpec};

const FONT: &str = "Helvetica, Arial, sans-serif";
/// Side of the square drawn for one event, in pixels.
const POINT_SIZE: f64 = 1.6;
const MAJOR_TICK: f64 = 6.0;
const MINOR_TICK: f64 = 3.0;
/// Longest legend text before it is cut with an ellipsis.
const LEGEND_TEXT_CHARS: usize = 24;
/// Least distance from the frame top to a gate label's anchor: the label's
/// baseline sits 4 px above it, so 11 px glyphs stay clear of the frame line.
const LABEL_TOP_CLEARANCE: f64 = 16.0;
/// Width allowed per character of a bold 11 px gate label, for collisions.
const LABEL_CHAR_WIDTH: f64 = 7.0;
/// Vertical step between gate labels that would overlap.
const LABEL_LINE_HEIGHT: f64 = 14.0;

/// SVG elements a plot may contain.
const ALLOWED_ELEMENTS: [&str; 10] = [
    "svg", "title", "desc", "defs", "clipPath", "g", "rect", "path", "line", "text",
];

/// Render a plot as a standalone SVG document.
pub(crate) fn render(spec: &PlotSpec) -> String {
    let mut out = String::with_capacity(64 * 1024);
    let frame = spec.frame;
    let clip = format!("{}-plot-area", spec.name);
    let _ = write!(
        out,
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="0 0 {w} {h}" role="img" font-family="{FONT}" font-size="11" fill="{TEXT_COLOR}">"#,
        w = spec.width,
        h = spec.height,
    );
    let _ = write!(
        out,
        "<title>{}</title><desc>{} — rendered by cytiq plot {}</desc>",
        escape(&spec.title),
        escape(&spec.subtitle),
        env!("CARGO_PKG_VERSION"),
    );
    let _ = write!(
        out,
        r#"<defs><clipPath id="{clip}"><rect x="{}" y="{}" width="{}" height="{}"/></clipPath></defs>"#,
        number(frame.left),
        number(frame.top),
        number(frame.width),
        number(frame.height),
    );
    let _ = write!(
        out,
        r##"<rect width="{}" height="{}" fill="#FFFFFF"/>"##,
        spec.width, spec.height
    );
    let _ = write!(out, r#"<g clip-path="url(#{clip})">"#);
    for layer in &spec.layers {
        match layer {
            Layer::Density {
                bins,
                levels,
                colors,
            } => density(&mut out, frame, *bins, levels, colors),
            Layer::Points { color, points, .. } => points_path(&mut out, frame, color, points),
            Layer::Lines {
                color,
                width,
                paths,
                ..
            } => lines(&mut out, frame, color, *width, paths),
        }
    }
    out.push_str("</g>");
    axes(&mut out, spec);
    gate_labels(&mut out, frame, &spec.labels);
    header(&mut out, spec);
    legend(&mut out, frame, &spec.legend);
    out.push_str("</svg>\n");
    out
}

fn density(out: &mut String, frame: Frame, bins: usize, levels: &[u16], colors: &[String]) {
    #[allow(clippy::cast_precision_loss, reason = "bins is at most 1024")]
    let cells = bins as f64;
    let _ = write!(
        out,
        r#"<g transform="translate({} {}) scale({:.6} {:.6})" shape-rendering="crispEdges">"#,
        number(frame.left),
        number(frame.top),
        frame.width / cells,
        frame.height / cells,
    );
    // One path per level: each maximal horizontal run of equal level is one
    // rectangle subpath, in integer cell coordinates (row 0 at the top).
    let mut paths = vec![String::new(); colors.len()];
    for row in 0..bins {
        let cells = &levels[row * bins..(row + 1) * bins];
        let top = bins - 1 - row;
        let mut column = 0;
        while column < bins {
            let level = cells[column];
            let start = column;
            while column < bins && cells[column] == level {
                column += 1;
            }
            if level > 0
                && let Some(path) = paths.get_mut(usize::from(level) - 1)
            {
                let _ = write!(
                    path,
                    "M{start} {top}h{}v1h-{}z",
                    column - start,
                    column - start
                );
            }
        }
    }
    for (color, path) in colors.iter().zip(&paths) {
        if !path.is_empty() {
            let _ = write!(out, r#"<path fill="{color}" d="{path}"/>"#);
        }
    }
    out.push_str("</g>");
}

fn points_path(out: &mut String, frame: Frame, color: &str, points: &[[f64; 2]]) {
    if points.is_empty() {
        return;
    }
    let half = POINT_SIZE / 2.0;
    let _ = write!(out, r#"<path fill="{color}" d=""#);
    for [x, y] in points {
        let _ = write!(
            out,
            "M{} {}h{size}v{size}h-{size}z",
            number(frame.x(*x) - half),
            number(frame.y(*y) - half),
            size = POINT_SIZE,
        );
    }
    out.push_str(r#""/>"#);
}

fn lines(out: &mut String, frame: Frame, color: &str, width: f64, paths: &[crate::spec::Polyline]) {
    let mut d = String::new();
    for path in paths {
        for (index, [x, y]) in path.points.iter().enumerate() {
            d.push(if index == 0 { 'M' } else { 'L' });
            let _ = write!(d, "{} {}", number(frame.x(*x)), number(frame.y(*y)));
        }
        if path.closed && !path.points.is_empty() {
            d.push('Z');
        }
    }
    if !d.is_empty() {
        let _ = write!(
            out,
            r#"<path fill="none" stroke="{color}" stroke-width="{}" stroke-linejoin="round" d="{d}"/>"#,
            number(width)
        );
    }
}

fn axes(out: &mut String, spec: &PlotSpec) {
    let frame = spec.frame;
    let bottom = frame.top + frame.height;
    let _ = write!(
        out,
        r#"<rect x="{}" y="{}" width="{}" height="{}" fill="none" stroke="{GATE_COLOR}" stroke-width="1"/>"#,
        number(frame.left),
        number(frame.top),
        number(frame.width),
        number(frame.height),
    );
    let mut marks = String::new();
    let mut labels = String::new();
    for tick in &spec.x.ticks {
        let x = frame.x(tick.position);
        let length = if tick.major { MAJOR_TICK } else { MINOR_TICK };
        let _ = write!(
            marks,
            "M{} {}v{}",
            number(x),
            number(bottom),
            number(length)
        );
        if let Some(label) = &tick.label {
            let _ = write!(
                labels,
                r#"<text x="{}" y="{}" text-anchor="middle">{}</text>"#,
                number(x),
                number(bottom + 18.0),
                escape(label)
            );
        }
    }
    for tick in &spec.y.ticks {
        let y = frame.y(tick.position);
        let length = if tick.major { MAJOR_TICK } else { MINOR_TICK };
        let _ = write!(
            marks,
            "M{} {}h-{}",
            number(frame.left),
            number(y),
            number(length)
        );
        if let Some(label) = &tick.label {
            let _ = write!(
                labels,
                r#"<text x="{}" y="{}" text-anchor="end">{}</text>"#,
                number(frame.left - 9.0),
                number(y + 4.0),
                escape(label)
            );
        }
    }
    if !marks.is_empty() {
        let _ = write!(
            out,
            r#"<path fill="none" stroke="{GATE_COLOR}" stroke-width="1" d="{marks}"/>"#
        );
    }
    out.push_str(&labels);
    let _ = write!(
        out,
        r#"<text x="{}" y="{}" text-anchor="middle" font-size="12">{}</text>"#,
        number(frame.left + frame.width / 2.0),
        number(bottom + 42.0),
        escape(&spec.x.title)
    );
    let _ = write!(
        out,
        r#"<text transform="translate(18 {}) rotate(-90)" text-anchor="middle" font-size="12">{}</text>"#,
        number(frame.top + frame.height / 2.0),
        escape(&spec.y.title)
    );
}

/// Gate labels just above their anchor, in order. A label at the top edge
/// keeps its glyphs below the frame line, and one that would overlap a label
/// already placed moves down a line, so no label hides another.
fn gate_labels(out: &mut String, frame: Frame, labels: &[Label]) {
    let lowest = frame.top + frame.height - 4.0;
    let mut placed: Vec<[f64; 3]> = Vec::with_capacity(labels.len());
    for label in labels {
        let x = frame
            .x(label.position[0].clamp(0.0, 1.0))
            .clamp(frame.left + 2.0, frame.left + frame.width - 2.0)
            + 3.0;
        let mut baseline = frame
            .y(label.position[1].clamp(0.0, 1.0))
            .clamp(frame.top + LABEL_TOP_CLEARANCE, lowest)
            - 4.0;
        #[allow(clippy::cast_precision_loss, reason = "labels are short")]
        let width = label.text.chars().count() as f64 * LABEL_CHAR_WIDTH;
        let overlaps = |baseline: f64| {
            placed
                .iter()
                .any(|&[other_x, other_baseline, other_width]| {
                    x < other_x + other_width
                        && other_x < x + width
                        && (baseline - other_baseline).abs() < LABEL_LINE_HEIGHT
                })
        };
        while overlaps(baseline) && baseline + LABEL_LINE_HEIGHT <= lowest - 4.0 {
            baseline += LABEL_LINE_HEIGHT;
        }
        placed.push([x, baseline, width]);
        let _ = write!(
            out,
            r##"<text x="{}" y="{}" font-weight="600" fill="{GATE_COLOR}" stroke="#FFFFFF" stroke-width="3" paint-order="stroke">{}</text>"##,
            number(x),
            number(baseline),
            escape(&label.text)
        );
    }
}

fn header(out: &mut String, spec: &PlotSpec) {
    let left = spec.frame.left;
    let _ = write!(
        out,
        r#"<text x="{}" y="22" font-size="14" font-weight="600">{}</text>"#,
        number(left),
        escape(&spec.title)
    );
    let _ = write!(
        out,
        r#"<text x="{}" y="38" fill="{MUTED_TEXT_COLOR}">{}</text>"#,
        number(left),
        escape(&spec.subtitle)
    );
    if !spec.notes.is_empty() {
        let _ = write!(
            out,
            r#"<text x="{}" y="53" fill="{NOTE_COLOR}">{}</text>"#,
            number(left),
            escape(&spec.notes.join(" · "))
        );
    }
}

fn legend(out: &mut String, frame: Frame, entries: &[LegendEntry]) {
    let x = frame.left + frame.width + 16.0;
    for (row, entry) in entries.iter().enumerate() {
        #[allow(clippy::cast_precision_loss, reason = "legend rows are few")]
        let y = frame.top + 8.0 + LEGEND_ROW_HEIGHT * row as f64;
        let _ = write!(
            out,
            r#"<rect x="{}" y="{}" width="10" height="10" fill="{}"/>"#,
            number(x),
            number(y - 9.0),
            entry.color
        );
        let text = shorten(&entry.text, LEGEND_TEXT_CHARS);
        let _ = write!(
            out,
            r#"<text x="{}" y="{}">{}</text>"#,
            number(x + 16.0),
            number(y),
            escape(&text)
        );
    }
}

fn shorten(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        text.to_owned()
    } else {
        let mut short = text.chars().take(limit - 1).collect::<String>();
        short.push('\u{2026}');
        short
    }
}

/// A coordinate with one decimal; `-0.0` prints as `0.0`.
fn number(value: f64) -> String {
    let rounded = (value * 10.0).round() / 10.0;
    let rounded = if rounded == 0.0 { 0.0 } else { rounded };
    format!("{rounded:.1}")
}

/// XML-escape text and drop characters XML 1.0 forbids, so labels from FCS
/// keywords, gate ids or class files can neither break nor inject markup.
pub(crate) fn escape(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&apos;"),
            '\t' | '\n' | '\r' => escaped.push(' '),
            character if u32::from(character) < 0x20 => {}
            '\u{FFFE}' | '\u{FFFF}' => {}
            character => escaped.push(character),
        }
    }
    escaped
}

/// Parse a rendered SVG and reject anything outside the plot vocabulary:
/// unknown elements (`image`, `script`, `foreignObject`, ...), `href`
/// attributes, `data:` URIs and non-local `url(...)` references.
pub(crate) fn check(plot: &str, svg: &str) -> Result<(), PlotError> {
    let invalid = |message: String| PlotError::InvalidSvg {
        plot: plot.to_owned(),
        message,
    };
    let document = roxmltree::Document::parse(svg).map_err(|error| invalid(error.to_string()))?;
    for node in document.descendants().filter(roxmltree::Node::is_element) {
        let name = node.tag_name();
        if name.namespace() != Some("http://www.w3.org/2000/svg")
            || !ALLOWED_ELEMENTS.contains(&name.name())
        {
            return Err(invalid(format!("element `{}` is not allowed", name.name())));
        }
        for attribute in node.attributes() {
            let value = attribute.value().to_ascii_lowercase();
            if attribute.name().eq_ignore_ascii_case("href")
                || value.contains("data:")
                || (value.contains("url(") && !value.starts_with("url(#"))
            {
                return Err(invalid(format!(
                    "attribute `{}` on `{}` is not allowed",
                    attribute.name(),
                    name.name()
                )));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::axis::Tick;
    use crate::spec::{AxisSpec, Polyline};

    fn spec(layers: Vec<Layer>) -> PlotSpec {
        PlotSpec {
            name: "fsc_ssc".into(),
            title: "sample <A&B>: SSC-A vs FSC-A".into(),
            subtitle: "15,000 events".into(),
            notes: vec!["showing 10 of 20 events".into()],
            width: 600,
            height: 560,
            frame: Frame::new(600, 560, true).unwrap(),
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
                ticks: vec![Tick {
                    position: 0.25,
                    label: None,
                    major: false,
                }],
            },
            layers,
            labels: vec![Label {
                position: [0.2, 0.8],
                text: "Cells 41.0%".into(),
            }],
            legend: vec![LegendEntry {
                color: "#E15759".into(),
                text: "removed (12)".into(),
            }],
        }
    }

    fn density_layer() -> Layer {
        // 4×4 grid: bottom row = [1, 1, 2, 0], others empty except one cell.
        let mut levels = vec![0; 16];
        levels[..4].copy_from_slice(&[1, 1, 2, 0]);
        levels[15] = 2;
        Layer::Density {
            bins: 4,
            levels,
            colors: vec!["#000001".into(), "#000002".into()],
        }
    }

    #[test]
    fn density_is_one_path_per_level_with_merged_runs() {
        let svg = render(&spec(vec![density_layer()]));
        assert!(
            svg.contains(r##"<path fill="#000001" d="M0 3h2v1h-2z"/>"##),
            "{svg}"
        );
        assert!(svg.contains(r##"<path fill="#000002" d="M2 3h1v1h-1zM3 0h1v1h-1z"/>"##));
        check("fsc_ssc", &svg).expect("valid SVG");
    }

    #[test]
    fn svg_never_contains_raster_or_script() {
        let layers = vec![
            density_layer(),
            Layer::Points {
                name: "events".into(),
                color: "#4E79A7".into(),
                points: vec![[0.5, 0.5], [0.0, 1.0]],
            },
            Layer::Lines {
                name: "gate".into(),
                color: "#1F2933".into(),
                width: 1.5,
                paths: vec![Polyline {
                    points: vec![[0.1, 0.1], [0.9, 0.1], [0.5, 0.9]],
                    closed: true,
                }],
            },
        ];
        let svg = render(&spec(layers));
        check("fsc_ssc", &svg).expect("valid SVG");
        for forbidden in ["<image", "<script", "foreignObject", "href", "data:"] {
            assert!(!svg.contains(forbidden), "{forbidden}");
        }
        assert!(check("x", r#"<svg xmlns="http://www.w3.org/2000/svg"><image href="data:image/png;base64,AA"/></svg>"#).is_err());
        assert!(
            check(
                "x",
                r#"<svg xmlns="http://www.w3.org/2000/svg"><rect fill="url(http://x/#a)"/></svg>"#
            )
            .is_err()
        );
    }

    #[test]
    fn labels_are_xml_escaped() {
        let svg = render(&spec(Vec::new()));
        assert!(svg.contains("sample &lt;A&amp;B&gt;: SSC-A vs FSC-A"));
        assert_eq!(escape("a\u{1}b\"c'"), "ab&quot;c&apos;");
        check("fsc_ssc", &svg).expect("valid SVG");
    }

    #[test]
    fn identical_specs_render_identical_bytes() {
        let first = render(&spec(vec![density_layer()]));
        let second = render(&spec(vec![density_layer()]));
        assert_eq!(first, second);
    }

    #[test]
    fn clip_ids_are_prefixed_with_the_plot_name() {
        let svg = render(&spec(Vec::new()));
        assert!(svg.contains(r#"<clipPath id="fsc_ssc-plot-area">"#));
        assert!(svg.contains(r#"clip-path="url(#fsc_ssc-plot-area)""#));
    }

    /// A million events, half uniform noise (every cell occupied, the worst
    /// case for run merging) and half a dense cluster, at 256² cells.
    #[test]
    fn million_event_density_svg_stays_under_2_mb() {
        let mut state = 0x9E37_79B9_7F4A_7C15_u64;
        let mut uniform = move || {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            #[allow(clippy::cast_precision_loss, reason = "53-bit mantissa")]
            let value = (state >> 11) as f64 / (1_u64 << 53) as f64;
            value
        };
        let points = (0..1_000_000)
            .map(|event| {
                let (a, b) = (uniform(), uniform());
                if event % 2 == 0 {
                    [a, b]
                } else {
                    [0.4 + 0.2 * a * b, 0.3 + 0.2 * a]
                }
            })
            .collect::<Vec<_>>();
        let histogram = crate::bin::Histogram::new(256, points);
        let layer = Layer::Density {
            bins: 256,
            levels: histogram.levels(crate::config::DENSITY_LEVELS),
            colors: crate::palette::levels(
                crate::config::Palette::Pseudocolor,
                crate::config::DENSITY_LEVELS,
            ),
        };
        let svg = render(&spec(vec![layer]));
        assert!(svg.len() < 2 * 1024 * 1024, "{} bytes", svg.len());
        check("fsc_ssc", &svg).expect("valid SVG");
    }

    #[test]
    fn coordinates_have_one_decimal_and_no_negative_zero() {
        assert_eq!(number(-0.01), "0.0");
        assert_eq!(number(12.345), "12.3");
        assert_eq!(shorten("abcdef", 4), "abc\u{2026}");
    }
}
