//! Novelty detection as in `CytoScan::Flag(flagMethod = "novelty")`:
//! a Gaussian mixture is fitted on the reference features, the threshold is the
//! 5% quantile of the reference log-densities, and test files at or below it
//! are novelties.
//!
//! Upstream uses `mclust::Mclust`, which searches 14 covariance structures for
//! `G = 1..9` and keeps the best BIC. Here the mixtures come from
//! `linfa-clustering`, which fits full covariances only (mclust's `VVV`; `XXX`
//! for `G = 1`). The BIC is mclust's `2 loglik - npar ln n`, and a `G` is
//! skipped when a component has no more members than features (its covariance
//! is singular, where mclust reports `NA`). When no `G` qualifies (fewer
//! reference files than features), a single diagonal Gaussian (mclust's `XXI`)
//! is used instead and reported in the diagnostics.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]

use linfa::DatasetBase;
use linfa::traits::Fit;
use linfa_clustering::GaussianMixtureModel;
use nalgebra::{DMatrix, DVector};
use ndarray::Array2;
use rand::SeedableRng;
use rand_xoshiro::Xoshiro256Plus;
use serde::Serialize;

use crate::error::CytoScanError;

/// `G = 1..9`, mclust's default search range.
const MAX_COMPONENTS: usize = 9;
const EM_RUNS: u64 = 10;
const EM_MAX_ITERATIONS: u64 = 1_000;
const EM_TOLERANCE: f64 = 1.0e-8;
/// Ridge linfa adds to every covariance diagonal; kept tiny because singular
/// fits are rejected explicitly rather than regularised.
const COVARIANCE_RIDGE: f64 = 1.0e-12;
const DIAGONAL_VARIANCE_FLOOR: f64 = 1.0e-12;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NoveltyModelKind {
    /// Full-covariance mixture (`VVV`, or `XXX` when `G = 1`).
    FullCovariance,
    /// Single diagonal Gaussian (`XXI`) used when no full-covariance fit exists.
    DiagonalFallback,
}

#[derive(Clone, Debug)]
pub(crate) struct NoveltyModel {
    components: Vec<Component>,
    pub(crate) kind: NoveltyModelKind,
    pub(crate) bic: Option<f64>,
    pub(crate) threshold: f64,
}

#[derive(Clone, Debug)]
struct Component {
    log_weight: f64,
    mean: DVector<f64>,
    /// Lower Cholesky factor `L` of the covariance (`Σ = L Lᵀ`).
    cholesky: DMatrix<f64>,
    log_determinant: f64,
}

impl NoveltyModel {
    pub(crate) fn components(&self) -> usize {
        self.components.len()
    }

    /// Log-density of one feature row under the mixture (`log(mclust::dens)`).
    pub(crate) fn log_density(&self, row: &[f64]) -> f64 {
        let dimensions = row.len() as f64;
        let point = DVector::from_column_slice(row);
        let terms = self
            .components
            .iter()
            .map(|component| {
                let delta = &point - &component.mean;
                let solved = component
                    .cholesky
                    .solve_lower_triangular(&delta)
                    .expect("Cholesky factor is non-singular");
                component.log_weight
                    - 0.5
                        * (dimensions * std::f64::consts::TAU.ln()
                            + component.log_determinant
                            + solved.norm_squared())
            })
            .collect::<Vec<_>>();
        log_sum_exp(&terms)
    }
}

/// Fit the novelty model on reference features and set the 5% threshold.
pub(crate) fn fit(features: &[Vec<f64>], quantile: f64) -> Result<NoveltyModel, CytoScanError> {
    let rows = features.len();
    let dimensions = features.first().map_or(0, Vec::len);
    if rows == 0 || dimensions == 0 {
        return Err(CytoScanError::EmptyFeatureMatrix);
    }

    let mut best: Option<(f64, Vec<Component>)> = None;
    for components in 1..=MAX_COMPONENTS.min(rows) {
        let Some((log_likelihood, fitted)) = fit_full(features, components)? else {
            continue;
        };
        let parameters = components * dimensions
            + components * dimensions * (dimensions + 1) / 2
            + (components - 1);
        let bic = 2.0 * log_likelihood - parameters as f64 * (rows as f64).ln();
        if best.as_ref().is_none_or(|(current, _)| bic > *current) {
            best = Some((bic, fitted));
        }
    }

    let mut model = match best {
        Some((bic, components)) => NoveltyModel {
            components,
            kind: NoveltyModelKind::FullCovariance,
            bic: Some(bic),
            threshold: f64::NAN,
        },
        None => NoveltyModel {
            components: vec![diagonal_gaussian(features)],
            kind: NoveltyModelKind::DiagonalFallback,
            bic: None,
            threshold: f64::NAN,
        },
    };
    let mut reference = features
        .iter()
        .map(|row| model.log_density(row))
        .collect::<Vec<_>>();
    model.threshold = quantile_type7(&mut reference, quantile);
    Ok(model)
}

/// One linfa fit with `components` full-covariance Gaussians; `None` when the
/// fit fails or any component is singular (no more members than features).
fn fit_full(
    features: &[Vec<f64>],
    components: usize,
) -> Result<Option<(f64, Vec<Component>)>, CytoScanError> {
    let rows = features.len();
    let dimensions = features[0].len();
    let observations =
        Array2::from_shape_fn((rows, dimensions), |(row, column)| features[row][column]);
    let dataset = DatasetBase::from(observations);
    let rng = Xoshiro256Plus::seed_from_u64(crate::runner::DEFAULT_RANDOM_SEED);
    let Ok(gmm) = GaussianMixtureModel::params_with_rng(components, rng)
        .n_runs(EM_RUNS)
        .max_n_iterations(EM_MAX_ITERATIONS)
        .tolerance(EM_TOLERANCE)
        .reg_covariance(COVARIANCE_RIDGE)
        .fit(&dataset)
    else {
        return Ok(None);
    };

    let mut fitted = Vec::with_capacity(components);
    for component in 0..components {
        let weight = gmm.weights()[component];
        if weight * rows as f64 <= dimensions as f64 {
            return Ok(None);
        }
        let mean = DVector::from_iterator(dimensions, gmm.means().row(component).iter().copied());
        let covariance = DMatrix::from_fn(dimensions, dimensions, |row, column| {
            gmm.covariances()[[component, row, column]]
        });
        let Some(cholesky) = covariance.cholesky() else {
            return Ok(None);
        };
        let lower = cholesky.l();
        let log_determinant = 2.0 * lower.diagonal().iter().map(|value| value.ln()).sum::<f64>();
        if !log_determinant.is_finite() {
            return Ok(None);
        }
        fitted.push(Component {
            log_weight: weight.ln(),
            mean,
            cholesky: lower,
            log_determinant,
        });
    }

    let model = NoveltyModel {
        components: fitted,
        kind: NoveltyModelKind::FullCovariance,
        bic: None,
        threshold: f64::NAN,
    };
    let log_likelihood = features
        .iter()
        .map(|row| model.log_density(row))
        .sum::<f64>();
    if !log_likelihood.is_finite() {
        return Err(CytoScanError::NoveltyFit {
            reason: format!("non-finite log-likelihood for G = {components}"),
        });
    }
    Ok(Some((log_likelihood, model.components)))
}

/// Single Gaussian with per-feature maximum-likelihood variances (`XXI`).
fn diagonal_gaussian(features: &[Vec<f64>]) -> Component {
    let rows = features.len() as f64;
    let dimensions = features[0].len();
    let mean = DVector::from_fn(dimensions, |column, _| {
        features.iter().map(|row| row[column]).sum::<f64>() / rows
    });
    let variances = (0..dimensions)
        .map(|column| {
            let variance = features
                .iter()
                .map(|row| (row[column] - mean[column]).powi(2))
                .sum::<f64>()
                / rows;
            variance.max(DIAGONAL_VARIANCE_FLOOR)
        })
        .collect::<Vec<_>>();
    let cholesky = DMatrix::from_diagonal(&DVector::from_iterator(
        dimensions,
        variances.iter().map(|variance| variance.sqrt()),
    ));
    Component {
        log_weight: 0.0,
        mean,
        cholesky,
        log_determinant: variances.iter().map(|variance| variance.ln()).sum(),
    }
}

fn quantile_type7(values: &mut [f64], probability: f64) -> f64 {
    values.sort_by(f64::total_cmp);
    let position = probability * (values.len() - 1) as f64;
    let lower = position.floor() as usize;
    let upper = position.ceil() as usize;
    values[lower] + (values[upper] - values[lower]) * (position - lower as f64)
}

fn log_sum_exp(values: &[f64]) -> f64 {
    let maximum = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    if !maximum.is_finite() {
        return maximum;
    }
    maximum
        + values
            .iter()
            .map(|value| (value - maximum).exp())
            .sum::<f64>()
            .ln()
}

#[cfg(test)]
#[allow(
    dead_code,
    clippy::approx_constant,
    clippy::unreadable_literal,
    clippy::excessive_precision
)]
mod tests {
    use super::*;

    include!("../../../data/fixtures/cytoscan/reference.rs");

    fn points(range: std::ops::Range<usize>) -> Vec<Vec<f64>> {
        GMM_POINTS[range].iter().map(|row| row.to_vec()).collect()
    }

    #[test]
    fn two_cluster_fit_matches_mclust_vvv() {
        let features = points(0..GMM_POINTS.len());
        let model = fit(&features, 0.05).expect("fit");
        assert_eq!(model.kind, NoveltyModelKind::FullCovariance);
        assert_eq!(model.components(), GMM_TWO_G);
        let bic = model.bic.expect("bic");
        assert!((bic - GMM_TWO_BIC).abs() < 1e-3, "{bic} vs {GMM_TWO_BIC}");
        let log_likelihood = features
            .iter()
            .map(|row| model.log_density(row))
            .sum::<f64>();
        assert!((log_likelihood - GMM_TWO_LOGLIK).abs() < 1e-3);
        for (row, expected) in features.iter().zip(GMM_TWO_LOG_DENSITY) {
            assert!((model.log_density(row) - expected).abs() < 1e-3);
        }
    }

    #[test]
    fn single_cluster_fit_matches_mclust_xxx() {
        let features = points(0..120);
        let model = fit(&features, 0.05).expect("fit");
        assert_eq!(model.components(), GMM_ONE_G);
        let log_likelihood = features
            .iter()
            .map(|row| model.log_density(row))
            .sum::<f64>();
        assert!((log_likelihood - GMM_ONE_LOGLIK).abs() < 1e-6);
        for (row, expected) in features.iter().zip(GMM_ONE_LOG_DENSITY) {
            assert!((model.log_density(row) - expected).abs() < 1e-8);
        }
    }

    #[test]
    fn fewer_rows_than_features_falls_back_to_diagonal_gaussian() {
        let features = (0..5)
            .map(|index| (0..8).map(|column| f64::from(index * column % 7)).collect())
            .collect::<Vec<Vec<f64>>>();
        let model = fit(&features, 0.05).expect("fit");
        assert_eq!(model.kind, NoveltyModelKind::DiagonalFallback);
        assert!(model.threshold.is_finite());
    }

    #[test]
    fn fit_is_deterministic() {
        let features = points(0..GMM_POINTS.len());
        let first = fit(&features, 0.05).expect("fit");
        let second = fit(&features, 0.05).expect("fit");
        assert_eq!(first.threshold.to_bits(), second.threshold.to_bits());
    }
}
