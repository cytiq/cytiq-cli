use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::PlotError;

/// Cells per axis of the density grid.
pub const DEFAULT_BINS: usize = 256;
pub const MIN_BINS: usize = 16;
pub const MAX_BINS: usize = 1024;
/// Events drawn per points layer; more are sampled evenly.
pub const DEFAULT_MAX_POINTS: usize = 10_000;
pub const MAX_POINTS_LIMIT: usize = 1_000_000;
/// Event mass between two equal-probability contours.
pub const DEFAULT_CONTOUR_STEP: f64 = 0.1;
pub const MAX_CONTOUR_STEP: f64 = 0.5;
/// Plot size in pixels (SVG user units).
pub const DEFAULT_WIDTH: u32 = 600;
pub const DEFAULT_HEIGHT: u32 = 560;
pub const MIN_SIZE: u32 = 200;
pub const MAX_SIZE: u32 = 4000;
/// Colour levels of a density plot.
pub const DENSITY_LEVELS: u16 = 64;
/// `asinh(x / cofactor)` display scale; the `cytiq transform` default.
pub const ARCSINH_COFACTOR: f64 = 150.0;
/// `FlowSOM` CSV column read when `--class-column` is omitted.
pub const DEFAULT_CLASS_COLUMN: &str = "flowsom_metacluster";
/// Longest plot name; names are file stems in batch mode.
pub const MAX_NAME_LENGTH: usize = 64;

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PlotKind {
    /// Pseudocolor: events binned on a grid, coloured by log count.
    #[default]
    Density,
    /// Dots, sampled to `max_points`.
    Scatter,
    /// Equal-probability contours with outlier dots.
    Contour,
}

impl PlotKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Density => "density",
            Self::Scatter => "scatter",
            Self::Contour => "contour",
        }
    }
}

/// Display scale of an axis. `Auto` is linear for scatter, time and
/// already-transformed channels and logicle for other raw channels.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ScaleChoice {
    #[default]
    Auto,
    Linear,
    Logicle,
    Arcsinh,
}

impl ScaleChoice {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Linear => "linear",
            Self::Logicle => "logicle",
            Self::Arcsinh => "arcsinh",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Palette {
    /// FlowJo-style blue → green → yellow → red.
    #[default]
    Pseudocolor,
    /// Perceptually uniform viridis.
    Viridis,
}

/// Whether events a mask removes are drawn over the plot.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RemovedLayer {
    #[default]
    Overlay,
    Hide,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum OutputFormat {
    /// Vector SVG.
    Svg,
    /// Plotly figure JSON (`{data, layout}`) of the same plot.
    Plotly,
}

impl OutputFormat {
    /// File extension used for batch outputs.
    #[must_use]
    pub const fn extension(self) -> &'static str {
        match self {
            Self::Svg => "svg",
            Self::Plotly => "plotly.json",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct AxisOptions {
    /// Channel (`$PnN`, `$PnS` or `"$PnN [$PnS]"`) or gate dimension name;
    /// `None` takes the axis from the first gate.
    pub channel: Option<String>,
    pub scale: ScaleChoice,
    /// Visible range in data units (`lo < hi`); `None` is automatic.
    pub range: Option<(f64, f64)>,
}

/// One plot of the input sample.
#[derive(Clone, Debug, PartialEq)]
pub struct PlotRequest {
    /// `[A-Za-z0-9_-]{1,64}`; the file stem in batch mode and the prefix of
    /// SVG element ids.
    pub name: String,
    pub kind: PlotKind,
    pub x: AxisOptions,
    pub y: AxisOptions,
    /// Gates drawn on the plot; their dimensions define the axes they use.
    pub gates: Vec<String>,
    /// Only events in this gate are plotted.
    pub population: Option<String>,
    /// Colour events by the class labels of [`PlotConfig::color_by`].
    pub color_by: bool,
    pub bins: usize,
    pub max_points: usize,
    pub palette: Palette,
    pub removed: RemovedLayer,
    pub contour_step: f64,
    pub width: u32,
    pub height: u32,
    pub title: Option<String>,
}

impl PlotRequest {
    /// A density plot with every option at its default.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            kind: PlotKind::default(),
            x: AxisOptions::default(),
            y: AxisOptions::default(),
            gates: Vec::new(),
            population: None,
            color_by: false,
            bins: DEFAULT_BINS,
            max_points: DEFAULT_MAX_POINTS,
            palette: Palette::default(),
            removed: RemovedLayer::default(),
            contour_step: DEFAULT_CONTOUR_STEP,
            width: DEFAULT_WIDTH,
            height: DEFAULT_HEIGHT,
            title: None,
        }
    }

    /// Check every option the command line would have checked; batch files
    /// bypass the command-line parsers.
    ///
    /// # Errors
    ///
    /// Returns [`PlotError::InvalidOption`] naming the first invalid field.
    pub fn validate(&self) -> Result<(), PlotError> {
        let invalid = |field: &'static str, message: String| PlotError::InvalidOption {
            plot: self.name.clone(),
            field,
            message,
        };
        validate_name(&self.name).map_err(|message| invalid("name", message))?;
        if !(MIN_BINS..=MAX_BINS).contains(&self.bins) {
            return Err(invalid(
                "bins",
                format!("must be between {MIN_BINS} and {MAX_BINS}"),
            ));
        }
        if !(1..=MAX_POINTS_LIMIT).contains(&self.max_points) {
            return Err(invalid(
                "max_points",
                format!("must be between 1 and {MAX_POINTS_LIMIT}"),
            ));
        }
        if !(self.contour_step > 0.0 && self.contour_step <= MAX_CONTOUR_STEP) {
            return Err(invalid(
                "contour_step",
                format!("must be in (0, {MAX_CONTOUR_STEP}]"),
            ));
        }
        for (field, size) in [("width", self.width), ("height", self.height)] {
            if !(MIN_SIZE..=MAX_SIZE).contains(&size) {
                return Err(invalid(
                    field,
                    format!("must be between {MIN_SIZE} and {MAX_SIZE} pixels"),
                ));
            }
        }
        for (field, range) in [("x_range", self.x.range), ("y_range", self.y.range)] {
            if let Some((lo, hi)) = range
                && !(lo.is_finite() && hi.is_finite() && lo < hi)
            {
                return Err(invalid(
                    field,
                    "must be two finite values with lo < hi".into(),
                ));
            }
        }
        if self.color_by && self.kind != PlotKind::Scatter {
            return Err(PlotError::ColorByNeedsScatter {
                plot: self.name.clone(),
            });
        }
        Ok(())
    }
}

/// A plot and the files it is written to.
#[derive(Clone, Debug, PartialEq)]
pub struct PlotJob {
    pub request: PlotRequest,
    pub outputs: Vec<(OutputFormat, PathBuf)>,
}

/// Per-event class labels: classify `predictions.parquet` (`label`) or a
/// `FlowSOM` CSV (`column`, default [`DEFAULT_CLASS_COLUMN`]).
#[derive(Clone, Debug, PartialEq)]
pub struct ClassSource {
    pub path: PathBuf,
    pub column: Option<String>,
}

/// One `cytiq plot` run: one sample, one or more plots.
#[derive(Clone, Debug, PartialEq)]
pub struct PlotConfig {
    pub input: PathBuf,
    /// Event masks, combined with AND; removed events form the overlay.
    pub masks: Vec<PathBuf>,
    pub gating_ml: Option<PathBuf>,
    pub color_by: Option<ClassSource>,
    pub jobs: Vec<PlotJob>,
    pub overwrite: bool,
}

/// A valid plot name derived from an output file name: the stem before the
/// first dot, with other characters replaced by `_`.
#[must_use]
pub fn plot_name_for(path: &Path) -> String {
    let stem = path
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.split('.').next())
        .unwrap_or_default();
    let name = stem
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '_' || character == '-' {
                character
            } else {
                '_'
            }
        })
        .take(MAX_NAME_LENGTH)
        .collect::<String>();
    if name.is_empty() {
        "plot".to_owned()
    } else {
        name
    }
}

pub(crate) fn validate_name(name: &str) -> Result<(), String> {
    if name.is_empty() || name.len() > MAX_NAME_LENGTH {
        return Err(format!("must be 1 to {MAX_NAME_LENGTH} characters"));
    }
    if !name
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || character == '_' || character == '-')
    {
        return Err(format!(
            "`{name}` may only contain ASCII letters, digits, `_` and `-`"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plot_names_come_from_the_output_file_stem() {
        assert_eq!(plot_name_for(Path::new("out/fsc_ssc.svg")), "fsc_ssc");
        assert_eq!(
            plot_name_for(Path::new("CD4 vs CD8.plotly.json")),
            "CD4_vs_CD8"
        );
        assert_eq!(plot_name_for(Path::new(".svg")), "plot");
    }

    #[test]
    fn invalid_options_are_named() {
        let mut request = PlotRequest::new("ok");
        assert!(request.validate().is_ok());
        request.bins = 8;
        assert!(request.validate().unwrap_err().to_string().contains("bins"));
        let mut request = PlotRequest::new("../escape");
        assert!(request.validate().unwrap_err().to_string().contains("name"));
        request.name = "ok".into();
        request.x.range = Some((5.0, 1.0));
        assert!(
            request
                .validate()
                .unwrap_err()
                .to_string()
                .contains("x_range")
        );
    }

    #[test]
    fn colour_by_class_needs_a_scatter_plot() {
        let mut request = PlotRequest::new("classes");
        request.color_by = true;
        assert!(matches!(
            request.validate(),
            Err(PlotError::ColorByNeedsScatter { .. })
        ));
        request.kind = PlotKind::Scatter;
        assert!(request.validate().is_ok());
    }
}
