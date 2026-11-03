use std::collections::HashSet;
use std::io::Write as _;
use std::path::PathBuf;

use cytiq_publish::{Existing, Publication};
use log::debug;

use crate::compose::{PlotStats, compose};
use crate::config::{OutputFormat, PlotConfig, PlotKind};
use crate::error::PlotError;
use crate::sample::Sample;
use crate::{plotly_json, svg};

/// What one plot showed and where it was written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlotSummary {
    pub name: String,
    pub kind: PlotKind,
    pub x: String,
    pub y: String,
    pub outputs: Vec<PathBuf>,
    pub stats: PlotStats,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlotRunSummary {
    pub plots: Vec<PlotSummary>,
}

impl PlotRunSummary {
    /// Every file written, in plot order.
    #[must_use]
    pub fn outputs(&self) -> Vec<PathBuf> {
        self.plots
            .iter()
            .flat_map(|plot| plot.outputs.iter().cloned())
            .collect()
    }
}

pub struct PlotRunner;

impl PlotRunner {
    /// Read the sample once, compose every plot, render each requested
    /// format and publish all files together: a failing plot publishes none.
    ///
    /// # Errors
    ///
    /// Returns an error when an input cannot be read or does not match the
    /// FCS file, a plot request is invalid, or an output exists without
    /// `overwrite`.
    pub fn run(config: &PlotConfig) -> Result<PlotRunSummary, PlotError> {
        if config.jobs.is_empty() {
            return Err(PlotError::InvalidOption {
                plot: String::new(),
                field: "plots",
                message: "must name at least one plot".to_owned(),
            });
        }
        let mut publication = Publication::new(Existing::overwrite(config.overwrite));
        let mut targets = HashSet::new();
        for job in &config.jobs {
            job.request.validate()?;
            for (_, path) in &job.outputs {
                if !targets.insert(path.clone()) {
                    return Err(PlotError::InvalidOption {
                        plot: job.request.name.clone(),
                        field: "output",
                        message: format!("`{}` is written twice", path.display()),
                    });
                }
                // Fail before reading the sample when an output exists.
                publication.check(path)?;
            }
        }

        let sample = Sample::load(config)?;
        let mut plots = Vec::with_capacity(config.jobs.len());
        for job in &config.jobs {
            let (spec, stats) = compose(&job.request, &sample)?;
            for (format, path) in &job.outputs {
                let text = match format {
                    OutputFormat::Svg => {
                        let text = svg::render(&spec);
                        svg::check(&spec.name, &text)?;
                        text
                    }
                    OutputFormat::Plotly => plotly_json::render(&spec),
                };
                publication.stage_file(path, |writer| {
                    writer
                        .write_all(text.as_bytes())
                        .map_err(|source| PlotError::Write {
                            path: path.clone(),
                            source,
                        })
                })?;
                debug!("rendered plot {} to {}", spec.name, path.display());
            }
            plots.push(PlotSummary {
                name: job.request.name.clone(),
                kind: job.request.kind,
                x: spec.x.title.clone(),
                y: spec.y.title.clone(),
                outputs: job.outputs.iter().map(|(_, path)| path.clone()).collect(),
                stats,
            });
        }
        publication.commit()?;
        Ok(PlotRunSummary { plots })
    }
}
