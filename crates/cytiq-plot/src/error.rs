use std::path::PathBuf;

use thiserror::Error;

/// Every failure of `cytiq plot`. A plot never fails because QC failed or no
/// event is left: those render as an annotated plot.
#[derive(Debug, Error)]
pub enum PlotError {
    #[error("failed to read FCS input `{path}`: {source}")]
    FcsRead {
        path: PathBuf,
        #[source]
        source: cytiq_fcs::Error,
    },
    #[error("invalid FCS input `{path}`: {source}")]
    Fcs {
        path: PathBuf,
        #[source]
        source: cytiq_fcs::Error,
    },
    #[error(transparent)]
    Mask(#[from] cytiq_mask::MaskError),
    #[error(transparent)]
    Gating(#[from] cytiq_gating::GatingError),
    #[error("failed to read Gating-ML `{path}`: {source}")]
    GatingMlRead {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("channel `{channel}`: {source}")]
    Transform {
        channel: String,
        #[source]
        source: cytiq_transform::TransformError,
    },
    #[error("channel `{channel}` is not in the FCS file; available: {available}")]
    MissingChannel { channel: String, available: String },
    #[error(
        "plot `{plot}` has no {axis} axis; pass --{axis} or a two-dimensional gate whose dimensions it uses"
    )]
    MissingAxis { plot: String, axis: &'static str },
    #[error("plot `{plot}`: gate `{gate}` is not in the Gating-ML file")]
    UnknownGate { plot: String, gate: String },
    #[error("plot `{plot}` uses gates or a population; pass --gating-ml")]
    MissingGatingMl { plot: String },
    #[error("plot `{plot}`: gate `{gate}` {reason}")]
    GateAxisMismatch {
        plot: String,
        gate: String,
        reason: String,
    },
    #[error("plot `{plot}`: gate `{gate}` cannot be drawn: {reason}")]
    UnsupportedGateGeometry {
        plot: String,
        gate: String,
        reason: String,
    },
    #[error(
        "plot `{plot}`: channel `{channel}` already holds transformed values; use the `auto` or `linear` scale instead of `{scale}`"
    )]
    ScaleOnTransformedData {
        plot: String,
        channel: String,
        scale: &'static str,
    },
    #[error("plot `{plot}`: {field} {message}")]
    InvalidOption {
        plot: String,
        field: &'static str,
        message: String,
    },
    #[error("class labels `{path}`: {message}")]
    Classes { path: PathBuf, message: String },
    #[error(
        "class labels `{path}` were computed on acquisition {found}, but the FCS input is acquisition {expected}"
    )]
    ClassesAcquisition {
        path: PathBuf,
        expected: String,
        found: String,
    },
    #[error("plot `{plot}` colours events by class; pass --color-by")]
    ColorByWithoutClasses { plot: String },
    #[error("plot `{plot}` colours events by class, which needs the `scatter` type")]
    ColorByNeedsScatter { plot: String },
    #[error("plot config `{path}`: {message}")]
    Batch { path: PathBuf, message: String },
    #[error("plot `{plot}` produced an invalid SVG: {message}")]
    InvalidSvg { plot: String, message: String },
    #[error("plot `{plot}` failed to serialize: {message}")]
    Serialize { plot: String, message: String },
    #[error("{0}")]
    Publish(#[from] cytiq_publish::PublishError),
    #[error("failed to write `{path}`: {source}")]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}
