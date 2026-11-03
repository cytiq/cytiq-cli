//! `cytiq-plot` renders FlowJo-style 2-D plots of one FCS file — pseudocolor
//! density, scatter and equal-probability contour — as vector SVG, with
//! optional Plotly figure JSON of the same plot.
//!
//! A plot can show the events an event mask keeps with the removed ones
//! overlaid, restrict to a Gating-ML population, draw Gating-ML gates with
//! their percentage of parent, and colour events by class labels from
//! `cytiq classify` or `cytiq flowsom`. Plots never depend on QC passing: a
//! sample whose mask removes every event still renders, annotated.
//!
//! The SVG is pure vector (no raster images, scripts or external
//! references, checked before publishing) and byte-deterministic.

mod axis;
mod batch;
mod bin;
mod classes;
mod compose;
mod config;
mod contour;
mod error;
mod geometry;
mod palette;
mod plotly_json;
mod runner;
mod sample;
mod spec;
mod svg;

pub use crate::batch::{BATCH_VERSION, load_batch};
pub use crate::compose::PlotStats;
pub use crate::config::{
    ARCSINH_COFACTOR, AxisOptions, ClassSource, DEFAULT_BINS, DEFAULT_CLASS_COLUMN,
    DEFAULT_CONTOUR_STEP, DEFAULT_HEIGHT, DEFAULT_MAX_POINTS, DEFAULT_WIDTH, DENSITY_LEVELS,
    MAX_BINS, MAX_CONTOUR_STEP, MAX_NAME_LENGTH, MAX_POINTS_LIMIT, MAX_SIZE, MIN_BINS, MIN_SIZE,
    OutputFormat, Palette, PlotConfig, PlotJob, PlotKind, PlotRequest, RemovedLayer, ScaleChoice,
    plot_name_for,
};
pub use crate::error::PlotError;
pub use crate::runner::{PlotRunSummary, PlotRunner, PlotSummary};
