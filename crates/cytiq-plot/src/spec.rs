//! The renderer-neutral description of one plot. Coordinates are in the
//! unit square (`[0, 1]²`, `y` up); renderers map them to their own space.

use crate::axis::Tick;

pub(crate) const MARGIN_LEFT: f64 = 78.0;
pub(crate) const MARGIN_RIGHT: f64 = 24.0;
pub(crate) const MARGIN_TOP: f64 = 62.0;
pub(crate) const MARGIN_BOTTOM: f64 = 54.0;
/// Extra right margin holding the legend.
pub(crate) const LEGEND_WIDTH: f64 = 170.0;
/// Height of one legend row.
pub(crate) const LEGEND_ROW_HEIGHT: f64 = 18.0;
/// Smallest plot area, in pixels, left after margins and legend.
pub(crate) const MIN_PLOT_AREA: f64 = 80.0;

/// Pixel rectangle of the plot area inside the image.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Frame {
    pub left: f64,
    pub top: f64,
    pub width: f64,
    pub height: f64,
}

impl Frame {
    /// The plot area of a `width × height` image; `None` when the margins
    /// (and legend) leave less than [`MIN_PLOT_AREA`] pixels.
    pub(crate) fn new(width: u32, height: u32, legend: bool) -> Option<Self> {
        let right = MARGIN_RIGHT + if legend { LEGEND_WIDTH } else { 0.0 };
        let frame = Self {
            left: MARGIN_LEFT,
            top: MARGIN_TOP,
            width: f64::from(width) - MARGIN_LEFT - right,
            height: f64::from(height) - MARGIN_TOP - MARGIN_BOTTOM,
        };
        (frame.width >= MIN_PLOT_AREA && frame.height >= MIN_PLOT_AREA).then_some(frame)
    }

    pub(crate) fn x(&self, unit: f64) -> f64 {
        self.left + unit * self.width
    }

    pub(crate) fn y(&self, unit: f64) -> f64 {
        self.top + (1.0 - unit) * self.height
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct AxisSpec {
    pub title: String,
    pub ticks: Vec<Tick>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Polyline {
    pub points: Vec<[f64; 2]>,
    pub closed: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Layer {
    /// Colour level per cell (`0` = empty), row-major from the bottom row;
    /// `colors[level - 1]` is the colour of a level.
    Density {
        bins: usize,
        levels: Vec<u16>,
        colors: Vec<String>,
    },
    Points {
        name: String,
        color: String,
        points: Vec<[f64; 2]>,
    },
    Lines {
        name: String,
        color: String,
        width: f64,
        paths: Vec<Polyline>,
    },
}

/// Text placed at a point of the unit square (gate names and percentages).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Label {
    pub position: [f64; 2],
    pub text: String,
}

#[derive(Clone, Debug, PartialEq)]
/// A square colour swatch and its text.
pub(crate) struct LegendEntry {
    pub color: String,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PlotSpec {
    /// Plot name; prefixes SVG element ids.
    pub name: String,
    pub title: String,
    pub subtitle: String,
    /// Warnings shown under the subtitle (sampling, off-scale, empty plot).
    pub notes: Vec<String>,
    pub width: u32,
    pub height: u32,
    pub frame: Frame,
    pub x: AxisSpec,
    pub y: AxisSpec,
    /// Drawn in order, first at the bottom.
    pub layers: Vec<Layer>,
    pub labels: Vec<Label>,
    pub legend: Vec<LegendEntry>,
}
