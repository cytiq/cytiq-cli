//! Gaussian mixtures fitted the way `mclust::Mclust` (mclust 6.1.3) fits them
//! with its defaults, which `CytoScan` uses for novelty detection.
//!
//! - Every covariance structure below is fitted for `G = 1..9` and the best
//!   BIC (`2 loglik - npar ln n`) wins; ties go to the smaller `G`, then to
//!   the alphabetically first model, as in `pickBIC`.
//! - `G = 1` fits are closed form (`mvnXII`, `mvnXXI`, `mvnXXX`).
//! - `G >= 2` fits run EM (`me*`) from the partition of model-based
//!   hierarchical clustering: `hc(modelName = "EII", use = "SVD")`, which is
//!   Ward's method on the scaled, SVD-whitened data.
//! - With no more rows than columns mclust only tries the spherical and
//!   diagonal models (`EII`, `VII`, `EEI`, `VEI`, `EVI`, `VVI`); so does this
//!   port. With more rows it also tries `EEE` and `VVV` here, but not mclust's
//!   seven other full-covariance structures, and it initialises from `hcEII`
//!   where mclust uses `hcVVV`.
//!
//! Singular fits are rejected with mclust's own thresholds (`eps` is the
//! machine epsilon, `rteps` its square root), so a model mclust reports as
//! `NA` is skipped here too. One exception: a one-component full covariance
//! that is singular up to rounding, which mclust keeps, is rejected with the
//! rule mclust applies to more components.

#![allow(
    clippy::cast_precision_loss,
    clippy::many_single_char_names,
    clippy::needless_range_loop
)]

use nalgebra::{DMatrix, DVector};
use serde::Serialize;

/// `emControl()$eps`.
const EPS: f64 = f64::EPSILON;
/// `emControl()$tol[1]`: relative log-likelihood change that ends EM.
const TOLERANCE: f64 = 1.0e-5;
/// mclust iterates up to `.Machine$integer.max` times; EM converges long
/// before, and the cap only guards against a cycle.
const MAX_ITERATIONS: usize = 100_000;
/// `exp` arguments below this are zero (Fortran `SMALOG`).
const SMALL_LOG: f64 = -708.0;
/// Geometric means of `exp` above this overflow (Fortran `BIGLOG`).
const BIG_LOG: f64 = 709.0;
/// `log(2 pi)` as mclust writes it (`pi2log`).
const LOG_TWO_PI: f64 = 1.837_877_066_409_345;
/// `G = 1..9`, mclust's default search range.
const MAX_COMPONENTS: usize = 9;

fn root_eps() -> f64 {
    EPS.sqrt()
}

/// mclust's covariance structures: volume, shape and orientation each equal
/// (`E`) or variable (`V`) across components, or the identity (`I`). `X..`
/// names the single-component fits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum MclustModel {
    /// Spherical, one component.
    Xii,
    /// Diagonal, one component.
    Xxi,
    /// Full covariance, one component.
    Xxx,
    Eii,
    Vii,
    Eei,
    Vei,
    Evi,
    Vvi,
    Eee,
    Vvv,
}

impl MclustModel {
    /// mclust's name, `VVI` or `XXX`.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Xii => "XII",
            Self::Xxi => "XXI",
            Self::Xxx => "XXX",
            Self::Eii => "EII",
            Self::Vii => "VII",
            Self::Eei => "EEI",
            Self::Vei => "VEI",
            Self::Evi => "EVI",
            Self::Vvi => "VVI",
            Self::Eee => "EEE",
            Self::Vvv => "VVV",
        }
    }

    /// The name of this structure with one component.
    const fn single(self) -> Self {
        match self {
            Self::Xii | Self::Eii | Self::Vii => Self::Xii,
            Self::Xxi | Self::Eei | Self::Vei | Self::Evi | Self::Vvi => Self::Xxi,
            Self::Xxx | Self::Eee | Self::Vvv => Self::Xxx,
        }
    }

    /// `nVarParams`.
    const fn variance_parameters(self, d: usize, g: usize) -> usize {
        match self {
            Self::Xii | Self::Eii => 1,
            Self::Vii => g,
            Self::Xxi | Self::Eei => d,
            Self::Vei => g + d - 1,
            Self::Evi => 1 + g * (d - 1),
            Self::Vvi => g * d,
            Self::Xxx | Self::Eee => d * (d + 1) / 2,
            Self::Vvv => g * d * (d + 1) / 2,
        }
    }

    /// `nMclustParams`: means, variances and `G - 1` mixing proportions.
    const fn parameters(self, d: usize, g: usize) -> usize {
        g * d + self.variance_parameters(d, g) + (g - 1)
    }
}

/// The structures mclust tries, in `mclust.options("emModelNames")` order.
const SEARCHED: [MclustModel; 8] = [
    MclustModel::Eii,
    MclustModel::Vii,
    MclustModel::Eei,
    MclustModel::Vei,
    MclustModel::Evi,
    MclustModel::Vvi,
    MclustModel::Eee,
    MclustModel::Vvv,
];
/// Spherical and diagonal structures: all mclust tries when `n <= d`.
const DIAGONAL: usize = 6;

/// One Gaussian of a mixture.
#[derive(Clone, Debug)]
pub(crate) struct Component {
    pub(crate) log_weight: f64,
    mean: Vec<f64>,
    spread: Spread,
    /// `log |Sigma|`.
    log_determinant: f64,
}

#[derive(Clone, Debug)]
enum Spread {
    /// Variances of a diagonal covariance.
    Diagonal(Vec<f64>),
    /// Lower Cholesky factor `L` of a full covariance (`Sigma = L L'`).
    Full(DMatrix<f64>),
}

impl Component {
    fn diagonal(weight: f64, mean: Vec<f64>, variances: Vec<f64>) -> Self {
        Self {
            log_weight: weight.ln(),
            mean,
            log_determinant: variances.iter().map(|variance| variance.ln()).sum(),
            spread: Spread::Diagonal(variances),
        }
    }

    fn full(weight: f64, mean: Vec<f64>, cholesky: DMatrix<f64>) -> Self {
        Self {
            log_weight: weight.ln(),
            mean,
            log_determinant: 2.0
                * cholesky
                    .diagonal()
                    .iter()
                    .map(|value| value.ln())
                    .sum::<f64>(),
            spread: Spread::Full(cholesky),
        }
    }

    /// `log N(row; mean, Sigma)`, without the mixing weight.
    pub(crate) fn log_kernel(&self, row: &[f64]) -> f64 {
        let squared = match &self.spread {
            Spread::Diagonal(variances) => row
                .iter()
                .zip(&self.mean)
                .zip(variances)
                .map(|((value, mean), variance)| (value - mean).powi(2) / variance)
                .sum::<f64>(),
            Spread::Full(cholesky) => {
                let delta = DVector::from_iterator(
                    row.len(),
                    row.iter().zip(&self.mean).map(|(value, mean)| value - mean),
                );
                cholesky
                    .solve_lower_triangular(&delta)
                    .map_or(f64::INFINITY, |solved| solved.norm_squared())
            }
        };
        -0.5 * (row.len() as f64 * LOG_TWO_PI + self.log_determinant + squared)
    }
}

/// The selected mixture.
#[derive(Clone, Debug)]
pub(crate) struct Mixture {
    /// The structure as mclust reports it (`XXI` for a one-component `VVI`).
    pub(crate) model: MclustModel,
    pub(crate) components: Vec<Component>,
    /// Compared with mclust's in the unit tests.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) log_likelihood: f64,
    pub(crate) bic: f64,
}

impl Mixture {
    /// `log(mclust::dens)` of one row.
    pub(crate) fn log_density(&self, row: &[f64]) -> f64 {
        log_sum_exp(
            self.components
                .iter()
                .map(|component| component.log_weight + component.log_kernel(row)),
        )
    }
}

/// One cell of mclust's BIC table; `None` where the fit is singular (`NA`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct BicEntry {
    pub(crate) model: MclustModel,
    pub(crate) components: usize,
    pub(crate) bic: Option<f64>,
}

#[derive(Clone, Debug)]
pub(crate) struct Search {
    /// `None` when no model could be fitted, where `Mclust` returns `NULL`.
    pub(crate) best: Option<Mixture>,
    /// Compared with mclust's in the unit tests.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) table: Vec<BicEntry>,
}

/// A converged fit of one structure and `G`.
struct Fit {
    log_likelihood: f64,
    components: Vec<Component>,
    /// Conditional probabilities of the last E-step, `n x G`.
    z: Vec<Vec<f64>>,
}

/// `Mclust(data)` with mclust's defaults: the BIC table and the best model.
pub(crate) fn mclust(data: &[Vec<f64>]) -> Search {
    let n = data.len();
    let d = data.first().map_or(0, Vec::len);
    let models = if n <= d {
        &SEARCHED[..DIAGONAL]
    } else {
        &SEARCHED[..]
    };
    let max_components = MAX_COMPONENTS.min(n);
    let ln_n = (n as f64).ln();
    let bic = |model: MclustModel, g: usize, log_likelihood: f64| {
        2.0 * log_likelihood - model.parameters(d, g) as f64 * ln_n
    };

    let mut table = Vec::new();
    let mut fits = Vec::<(MclustModel, usize, f64, Fit)>::new();
    for &model in models {
        let fit = single(model.single(), data);
        table.push(BicEntry {
            model,
            components: 1,
            bic: fit.as_ref().map(|fit| bic(model, 1, fit.log_likelihood)),
        });
        if let Some(fit) = fit {
            fits.push((model, 1, bic(model, 1, fit.log_likelihood), fit));
        }
    }
    let partitions = if max_components >= 2 {
        hc_partitions(data, 2..=max_components)
    } else {
        Vec::new()
    };
    for (g, partition) in (2..=max_components).zip(partitions) {
        for &model in models {
            let fit = partition
                .as_ref()
                .and_then(|labels| em(model, data, unmap(labels, g)));
            table.push(BicEntry {
                model,
                components: g,
                bic: fit.as_ref().map(|fit| bic(model, g, fit.log_likelihood)),
            });
            if let Some(fit) = fit {
                fits.push((model, g, bic(model, g, fit.log_likelihood), fit));
            }
        }
    }

    // `pickBIC`: highest BIC, then the smaller G, then the model name.
    let best = fits
        .into_iter()
        .filter(|(_, _, bic, _)| bic.is_finite())
        .min_by(|(model_a, g_a, bic_a, _), (model_b, g_b, bic_b, _)| {
            bic_b
                .total_cmp(bic_a)
                .then(g_a.cmp(g_b))
                .then(model_a.name().cmp(model_b.name()))
        })
        .map(|(model, g, bic, fit)| {
            let components = if g == 1 {
                fit.components
            } else {
                refine(model, data, fit.components, &fit.z)
            };
            Mixture {
                model: if g == 1 { model.single() } else { model },
                components,
                log_likelihood: fit.log_likelihood,
                bic,
            }
        });
    Search { best, table }
}

/// `summaryMclustBIC` ends with one more M-step when the mixing proportions
/// of the last EM iteration differ from the mean conditional probabilities.
fn refine(
    model: MclustModel,
    data: &[Vec<f64>],
    components: Vec<Component>,
    z: &[Vec<f64>],
) -> Vec<Component> {
    let n = data.len() as f64;
    let shift = components
        .iter()
        .enumerate()
        .map(|(k, component)| {
            let mean_z = z.iter().map(|row| row[k]).sum::<f64>() / n;
            (component.log_weight.exp() - mean_z).powi(2)
        })
        .sum::<f64>();
    if shift <= root_eps() {
        return components;
    }
    m_step(
        model,
        data,
        z,
        &mut VeiState::new(data[0].len(), components.len()),
    )
    .unwrap_or(components)
}

/// The closed-form one-component fits `mvnXII`, `mvnXXI` and `mvnXXX`.
fn single(model: MclustModel, data: &[Vec<f64>]) -> Option<Fit> {
    let n = data.len();
    let d = data[0].len();
    let rows = n as f64;
    let mean = (0..d)
        .map(|j| data.iter().map(|row| row[j]).sum::<f64>() / rows)
        .collect::<Vec<_>>();
    let z = vec![vec![1.0]; n];
    match model {
        MclustModel::Xii => {
            let sigma_squared = data
                .iter()
                .map(|row| squared_distance(row, &mean))
                .sum::<f64>()
                / (rows * d as f64);
            if sigma_squared == 0.0 {
                return None;
            }
            Some(Fit {
                log_likelihood: -(rows * d as f64) * (LOG_TWO_PI + 1.0 + sigma_squared.ln()) / 2.0,
                components: vec![Component::diagonal(1.0, mean, vec![sigma_squared; d])],
                z,
            })
        }
        MclustModel::Xxi => {
            let shape = (0..d)
                .map(|j| {
                    data.iter()
                        .map(|row| (row[j] - mean[j]).powi(2))
                        .sum::<f64>()
                })
                .collect::<Vec<_>>();
            if minimum(&shape) <= 0.0 {
                return None;
            }
            let log_geometric = shape.iter().map(|value| value.ln()).sum::<f64>() / d as f64;
            if !(SMALL_LOG..=BIG_LOG).contains(&log_geometric) {
                return None;
            }
            let scale = log_geometric.exp() / rows;
            Some(Fit {
                log_likelihood: -(rows * d as f64) * (1.0 + LOG_TWO_PI + scale.ln()) / 2.0,
                components: vec![Component::diagonal(
                    1.0,
                    mean,
                    shape.iter().map(|value| value / rows).collect(),
                )],
                z,
            })
        }
        MclustModel::Xxx => {
            // `mvnXXX` only rejects an exactly zero pivot, so mclust keeps a
            // covariance that is singular up to rounding (features summing
            // to 1, as fingerprint bins do) and its BIC is then rounding
            // noise. The `G >= 2` rule (`rc <= rteps`) rejects it here.
            let covariance = scatter(data, &mean, |_| 1.0) / rows;
            let cholesky = covariance.cholesky()?.l();
            if reciprocal_condition(&cholesky) <= root_eps() {
                return None;
            }
            let log_determinant = cholesky
                .diagonal()
                .iter()
                .map(|value| value.ln())
                .sum::<f64>();
            Some(Fit {
                log_likelihood: -rows * (log_determinant + d as f64 * (LOG_TWO_PI + 1.0) / 2.0),
                components: vec![Component::full(1.0, mean, cholesky)],
                z,
            })
        }
        _ => unreachable!("one-component fits are named XII, XXI or XXX"),
    }
}

/// `me(modelName, data, z)`: EM from the conditional probabilities `z`,
/// until the log-likelihood changes by at most `TOLERANCE` relative.
fn em(model: MclustModel, data: &[Vec<f64>], mut z: Vec<Vec<f64>>) -> Option<Fit> {
    let g = z.first().map_or(0, Vec::len);
    let mut vei = VeiState::new(data[0].len(), g);
    let mut previous = f64::MAX / 2.0;
    for iteration in 1..=MAX_ITERATIONS {
        let components = m_step(model, data, &z, &mut vei)?;
        let (log_likelihood, next) = e_step(data, &components);
        if !log_likelihood.is_finite() {
            return None;
        }
        z = next;
        let change = (previous - log_likelihood).abs() / (1.0 + log_likelihood.abs());
        previous = log_likelihood;
        if change <= TOLERANCE || iteration == MAX_ITERATIONS {
            return Some(Fit {
                log_likelihood,
                components,
                z,
            });
        }
    }
    None
}

/// `VEI` alternates scale and shape inside each M-step, starting from the
/// previous M-step's values (both 1 at first).
struct VeiState {
    scale: Vec<f64>,
    shape: Vec<f64>,
}

impl VeiState {
    fn new(d: usize, g: usize) -> Self {
        Self {
            scale: vec![1.0; g],
            shape: vec![1.0; d],
        }
    }
}

/// One M-step: weights, means and covariances from `z`; `None` when a
/// component is empty or a covariance is singular by mclust's rules.
fn m_step(
    model: MclustModel,
    data: &[Vec<f64>],
    z: &[Vec<f64>],
    vei: &mut VeiState,
) -> Option<Vec<Component>> {
    let moments = Moments::of(data, z)?;
    match model {
        MclustModel::Eee | MclustModel::Vvv => full_components(model, data, z, &moments),
        MclustModel::Xii | MclustModel::Xxi | MclustModel::Xxx => {
            unreachable!("EM runs the G >= 2 structures")
        }
        _ => {
            let squares = moments.squares(data, z);
            let variances =
                diagonal_variances(model, &squares, &moments.sizes, data.len() as f64, vei)?;
            Some(
                moments
                    .weights
                    .iter()
                    .zip(&moments.means)
                    .zip(variances)
                    .map(|((weight, mean), variances)| {
                        Component::diagonal(*weight, mean.clone(), variances)
                    })
                    .collect(),
            )
        }
    }
}

/// Component sizes (`sum_i z_ik`), mixing weights and means.
struct Moments {
    sizes: Vec<f64>,
    weights: Vec<f64>,
    means: Vec<Vec<f64>>,
}

impl Moments {
    /// `None` when a component's size is at most `rteps`, where mclust stops.
    fn of(data: &[Vec<f64>], z: &[Vec<f64>]) -> Option<Self> {
        let d = data[0].len();
        let g = z[0].len();
        let sizes = (0..g)
            .map(|k| z.iter().map(|row| row[k]).sum::<f64>())
            .collect::<Vec<_>>();
        if minimum(&sizes) <= root_eps() {
            return None;
        }
        let weights = sizes.iter().map(|size| size / data.len() as f64).collect();
        let means = (0..g)
            .map(|k| {
                (0..d)
                    .map(|j| {
                        data.iter()
                            .zip(z)
                            .map(|(row, z)| z[k] * row[j])
                            .sum::<f64>()
                            / sizes[k]
                    })
                    .collect()
            })
            .collect();
        Some(Self {
            sizes,
            weights,
            means,
        })
    }

    /// Weighted sums of squares per component and column: the diagonals of
    /// the scatter matrices `W_k`.
    fn squares(&self, data: &[Vec<f64>], z: &[Vec<f64>]) -> Vec<Vec<f64>> {
        self.means
            .iter()
            .enumerate()
            .map(|(k, mean)| {
                (0..mean.len())
                    .map(|j| {
                        data.iter()
                            .zip(z)
                            .map(|(row, z)| z[k] * (row[j] - mean[j]).powi(2))
                            .sum::<f64>()
                    })
                    .collect()
            })
            .collect()
    }
}

/// Per-component variances of the spherical and diagonal structures, from
/// the sums of squares `squares[k][j]` (`mstepEII` ... `mstepVVI`).
fn diagonal_variances(
    model: MclustModel,
    squares: &[Vec<f64>],
    sizes: &[f64],
    rows: f64,
    vei: &mut VeiState,
) -> Option<Vec<Vec<f64>>> {
    let g = squares.len();
    let d = squares[0].len();
    let scaled = |scale: f64, shape: &[f64]| shape.iter().map(|value| scale * value).collect();
    match model {
        MclustModel::Eii => {
            let sigma_squared = squares.iter().flatten().sum::<f64>() / (rows * d as f64);
            (sigma_squared > EPS).then(|| vec![vec![sigma_squared; d]; g])
        }
        MclustModel::Vii => {
            let variances = squares
                .iter()
                .zip(sizes)
                .map(|(squares, size)| squares.iter().sum::<f64>() / size / d as f64)
                .collect::<Vec<_>>();
            (minimum(&variances) > EPS)
                .then(|| variances.iter().map(|value| vec![*value; d]).collect())
        }
        MclustModel::Eei => {
            let pooled = (0..d)
                .map(|j| squares.iter().map(|row| row[j]).sum::<f64>())
                .collect::<Vec<_>>();
            let (geometric, shape) = normalise(&pooled)?;
            let scale = geometric / rows;
            (minimum(&shape) > EPS && scale > EPS).then(|| vec![scaled(scale, &shape); g])
        }
        MclustModel::Vvi => squares
            .iter()
            .zip(sizes)
            .map(|(squares, size)| {
                if minimum(squares) <= EPS {
                    return None;
                }
                let (geometric, shape) = normalise(squares)?;
                let scale = geometric / size;
                (scale > EPS && minimum(&shape) > EPS).then(|| scaled(scale, &shape))
            })
            .collect(),
        MclustModel::Evi => {
            let mut volume = 0.0;
            let mut shapes = Vec::with_capacity(g);
            for squares in squares {
                if minimum(squares) <= 0.0 {
                    return None;
                }
                let (geometric, shape) = normalise(squares)?;
                volume += geometric;
                shapes.push(shape);
            }
            let scale = volume / rows;
            (scale > EPS && shapes.iter().all(|shape| minimum(shape) > EPS))
                .then(|| shapes.iter().map(|shape| scaled(scale, shape)).collect())
        }
        MclustModel::Vei => {
            vei_scale_shape(squares, sizes, vei)?;
            Some(
                vei.scale
                    .iter()
                    .map(|scale| scaled(*scale, &vei.shape))
                    .collect(),
            )
        }
        _ => unreachable!("full-covariance structures have no variance vector"),
    }
}

/// `VEI`'s inner iteration: per-component scales and a shared shape (with
/// geometric mean 1), alternated until both change by at most `rteps`.
fn vei_scale_shape(squares: &[Vec<f64>], sizes: &[f64], vei: &mut VeiState) -> Option<()> {
    let g = squares.len();
    let d = squares[0].len();
    let counts = sizes.iter().map(|size| size * d as f64).collect::<Vec<_>>();
    let VeiState { scale, shape } = vei;
    for _ in 0..MAX_ITERATIONS {
        if minimum(shape) <= EPS {
            return None;
        }
        let previous_scale = scale.clone();
        for k in 0..g {
            scale[k] = (0..d).map(|j| squares[k][j] / shape[j]).sum::<f64>() / counts[k];
        }
        if minimum(scale) <= EPS {
            return None;
        }
        let previous_shape = shape.clone();
        for j in 0..d {
            shape[j] = (0..g).map(|k| squares[k][j] / scale[k]).sum::<f64>();
        }
        if minimum(shape) <= EPS {
            return None;
        }
        *shape = normalise(shape)?.1;
        let change = previous_scale
            .iter()
            .zip(scale.iter())
            .chain(previous_shape.iter().zip(shape.iter()))
            .map(|(old, new)| (old - new).abs() / (1.0 + new))
            .fold(0.0, f64::max);
        if change <= root_eps() {
            break;
        }
    }
    (minimum(scale) > EPS && minimum(shape) > EPS).then_some(())
}

/// `EEE` (one pooled covariance) and `VVV` (one per component).
fn full_components(
    model: MclustModel,
    data: &[Vec<f64>],
    z: &[Vec<f64>],
    moments: &Moments,
) -> Option<Vec<Component>> {
    let g = moments.means.len();
    let d = data[0].len();
    let factor = |covariance: DMatrix<f64>| {
        let cholesky = covariance.cholesky()?.l();
        (reciprocal_condition(&cholesky) > root_eps()).then_some(cholesky)
    };
    let scatter_of = |k: usize| scatter(data, &moments.means[k], |row| z[row][k]);
    let pooled = if model == MclustModel::Eee {
        let mut pooled = DMatrix::zeros(d, d);
        for k in 0..g {
            pooled += scatter_of(k);
        }
        Some(factor(pooled / data.len() as f64)?)
    } else {
        None
    };
    (0..g)
        .map(|k| {
            let cholesky = match &pooled {
                Some(cholesky) => cholesky.clone(),
                None => factor(scatter_of(k) / moments.sizes[k])?,
            };
            Some(Component::full(
                moments.weights[k],
                moments.means[k].clone(),
                cholesky,
            ))
        })
        .collect()
}

/// The geometric mean of positive values and the values divided by it;
/// `None` where mclust's `exp` would overflow or the mean is below `eps`.
fn normalise(values: &[f64]) -> Option<(f64, Vec<f64>)> {
    let log_geometric = values.iter().map(|value| value.ln()).sum::<f64>() / values.len() as f64;
    if log_geometric > BIG_LOG {
        return None;
    }
    let geometric = if log_geometric > SMALL_LOG {
        log_geometric.exp()
    } else {
        0.0
    };
    if geometric <= EPS {
        return None;
    }
    Some((
        geometric,
        values.iter().map(|value| value / geometric).collect(),
    ))
}

/// mclust's singularity measure of a triangular factor:
/// `min |U_jj| / (1 + max |U_jj|)`.
fn reciprocal_condition(cholesky: &DMatrix<f64>) -> f64 {
    let diagonal = cholesky.diagonal();
    let smallest = diagonal
        .iter()
        .map(|value| value.abs())
        .fold(f64::INFINITY, f64::min);
    let largest = diagonal.iter().map(|value| value.abs()).fold(0.0, f64::max);
    smallest / (1.0 + largest)
}

/// `sum_i w_i (x_i - mean)(x_i - mean)'`.
fn scatter(data: &[Vec<f64>], mean: &[f64], weight: impl Fn(usize) -> f64) -> DMatrix<f64> {
    let d = mean.len();
    let mut matrix = DMatrix::zeros(d, d);
    for (index, row) in data.iter().enumerate() {
        let w = weight(index);
        let delta =
            DVector::from_iterator(d, row.iter().zip(mean).map(|(value, mean)| value - mean));
        matrix.ger(w, &delta, &delta, 1.0);
    }
    matrix
}

/// E-step: the log-likelihood and the conditional probabilities. Terms more
/// than `SMALL_LOG` below a row's largest are dropped, as in mclust.
fn e_step(data: &[Vec<f64>], components: &[Component]) -> (f64, Vec<Vec<f64>>) {
    let mut log_likelihood = 0.0;
    let z = data
        .iter()
        .map(|row| {
            let terms = components
                .iter()
                .map(|component| component.log_weight + component.log_kernel(row))
                .collect::<Vec<_>>();
            let largest = terms.iter().copied().fold(f64::NEG_INFINITY, f64::max);
            let scaled = terms
                .iter()
                .map(|term| {
                    let shifted = term - largest;
                    if shifted >= SMALL_LOG {
                        shifted.exp()
                    } else {
                        0.0
                    }
                })
                .collect::<Vec<_>>();
            let sum = scaled.iter().sum::<f64>();
            log_likelihood += sum.ln() + largest;
            scaled.iter().map(|value| value / sum).collect()
        })
        .collect();
    (log_likelihood, z)
}

/// `unmap`: a 0/1 matrix from 0-based labels.
fn unmap(labels: &[usize], g: usize) -> Vec<Vec<f64>> {
    labels
        .iter()
        .map(|label| (0..g).map(|k| f64::from(u8::from(k == *label))).collect())
        .collect()
}

/// `hclass(hc(data, modelName = "EII", use = "SVD"), G)` for every `G`:
/// labels numbered by first appearance, or `None` when there are fewer
/// distinct rows than `G`.
fn hc_partitions(
    data: &[Vec<f64>],
    groups: std::ops::RangeInclusive<usize>,
) -> Vec<Option<Vec<usize>>> {
    let points = svd_whiten(data);
    let n = data.len();
    // `dupPartition`: identical rows start in one group.
    let mut cluster_of = (0..n).collect::<Vec<_>>();
    for row in 0..n {
        if let Some(first) = (0..row).find(|other| data[*other] == data[row]) {
            cluster_of[row] = cluster_of[first];
        }
    }
    let starts = (0..n)
        .filter(|row| cluster_of[*row] == *row)
        .collect::<Vec<_>>();
    let mut sizes = vec![0.0; n];
    let mut centres = vec![vec![0.0; points.first().map_or(0, Vec::len)]; n];
    for row in 0..n {
        let cluster = cluster_of[row];
        sizes[cluster] += 1.0;
        for (centre, value) in centres[cluster].iter_mut().zip(&points[row]) {
            *centre += value;
        }
    }
    for &cluster in &starts {
        let size = sizes[cluster];
        centres[cluster].iter_mut().for_each(|value| *value /= size);
    }
    // Ward's cost of merging two clusters: the rise in the within-cluster
    // sum of squares, updated by Lance-Williams as `hceii` does.
    let mut cost = vec![vec![0.0; n]; n];
    for (position, &i) in starts.iter().enumerate() {
        for &j in &starts[..position] {
            let value = sizes[i] * sizes[j] / (sizes[i] + sizes[j])
                * squared_distance(&centres[i], &centres[j]);
            cost[i][j] = value;
            cost[j][i] = value;
        }
    }
    let mut active = starts;
    let mut snapshots = vec![None; groups.end().saturating_sub(*groups.start()) + 1];
    let snapshot = |active: &[usize], cluster_of: &[usize]| {
        let mut labels = vec![usize::MAX; n];
        let mut order = Vec::<usize>::new();
        for row in 0..n {
            let cluster = cluster_of[row];
            let label = order
                .iter()
                .position(|seen| *seen == cluster)
                .unwrap_or_else(|| {
                    order.push(cluster);
                    order.len() - 1
                });
            labels[row] = label;
        }
        debug_assert_eq!(order.len(), active.len());
        labels
    };
    loop {
        if groups.contains(&active.len()) {
            snapshots[active.len() - groups.start()] = Some(snapshot(&active, &cluster_of));
        }
        if active.len() <= *groups.start() {
            break;
        }
        let mut best = (f64::INFINITY, 0, 0);
        for (position, &j) in active.iter().enumerate() {
            for &i in &active[..position] {
                if cost[i][j] <= best.0 {
                    best = (cost[i][j], i, j);
                }
            }
        }
        let (merged_cost, keep, gone) = best;
        let (size_keep, size_gone) = (sizes[keep], sizes[gone]);
        for &other in &active {
            if other == keep || other == gone {
                continue;
            }
            let size = sizes[other];
            let value = ((size + size_keep) * cost[other][keep]
                + (size + size_gone) * cost[other][gone]
                - size * merged_cost)
                / (size + size_keep + size_gone);
            cost[other][keep] = value;
            cost[keep][other] = value;
        }
        sizes[keep] += size_gone;
        for cluster in &mut cluster_of {
            if *cluster == gone {
                *cluster = keep;
            }
        }
        active.retain(|cluster| *cluster != gone);
    }
    snapshots
}

/// `hc(use = "SVD")`'s transform: columns centred and scaled to unit
/// standard deviation (constant ones dropped), then `U sqrt(D)` of their
/// SVD, which is `X V D^(-1/2)`.
fn svd_whiten(data: &[Vec<f64>]) -> Vec<Vec<f64>> {
    let n = data.len();
    let d = data[0].len();
    let rows = n as f64;
    let mut columns = Vec::new();
    for j in 0..d {
        let mean = data.iter().map(|row| row[j]).sum::<f64>() / rows;
        let sd =
            (data.iter().map(|row| (row[j] - mean).powi(2)).sum::<f64>() / (rows - 1.0)).sqrt();
        let scaled = data
            .iter()
            .map(|row| (row[j] - mean) / sd)
            .collect::<Vec<_>>();
        if scaled.iter().all(|value| value.is_finite()) {
            columns.push(scaled);
        }
    }
    if columns.is_empty() {
        return vec![Vec::new(); n];
    }
    let matrix = DMatrix::from_fn(n, columns.len(), |row, column| columns[column][row]);
    let svd = matrix.svd(true, false);
    let u = svd.u.expect("U was requested");
    (0..n)
        .map(|row| {
            svd.singular_values
                .iter()
                .enumerate()
                .map(|(k, value)| u[(row, k)] * value.sqrt())
                .collect()
        })
        .collect()
}

fn squared_distance(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b).map(|(a, b)| (a - b).powi(2)).sum()
}

fn minimum(values: &[f64]) -> f64 {
    values.iter().copied().fold(f64::INFINITY, f64::min)
}

pub(crate) fn log_sum_exp(values: impl Iterator<Item = f64> + Clone) -> f64 {
    let maximum = values.clone().fold(f64::NEG_INFINITY, f64::max);
    if !maximum.is_finite() {
        return maximum;
    }
    maximum
        + values
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

    fn rows<const D: usize>(points: &[[f64; D]]) -> Vec<Vec<f64>> {
        points.iter().map(|row| row.to_vec()).collect()
    }

    /// Every cell of `table` (rows `G = 1..`, columns `models`) equals R's.
    fn assert_bic_table<const M: usize>(
        table: &[BicEntry],
        expected: &[[Option<f64>; M]],
        models: &[MclustModel],
    ) {
        for (g, row) in expected.iter().enumerate() {
            for (model, expected) in models.iter().zip(row) {
                let entry = table
                    .iter()
                    .find(|entry| entry.model == *model && entry.components == g + 1)
                    .unwrap_or_else(|| panic!("{} G = {} missing", model.name(), g + 1));
                match (entry.bic, expected) {
                    (Some(bic), Some(expected)) => assert!(
                        (bic - expected).abs() < 1e-6 * expected.abs(),
                        "{} G = {}: {bic} vs mclust {expected}",
                        model.name(),
                        g + 1
                    ),
                    (bic, expected) => assert_eq!(
                        bic.is_some(),
                        expected.is_some(),
                        "{} G = {}: {bic:?} vs mclust {expected:?}",
                        model.name(),
                        g + 1
                    ),
                }
            }
        }
    }

    #[test]
    fn ward_partitions_match_mclust_hclass() {
        let partitions = hc_partitions(&rows(&WIDE_POINTS), 2..=9);
        for (column, partition) in partitions.iter().enumerate() {
            let expected = WIDE_HCLASS
                .iter()
                .map(|row| row[column])
                .collect::<Vec<_>>();
            assert_eq!(
                partition.as_deref(),
                Some(expected.as_slice()),
                "G = {}",
                column + 2
            );
        }
    }

    #[test]
    fn wide_data_matches_mclust_defaults() {
        let data = rows(&WIDE_POINTS);
        let search = mclust(&data);
        assert_bic_table(&search.table, &WIDE_BIC_TABLE, &SEARCHED[..DIAGONAL]);
        assert!(
            search
                .table
                .iter()
                .all(|entry| entry.model != MclustModel::Vvv),
            "n <= d: spherical and diagonal structures only"
        );
        let best = search.best.expect("a model fits");
        assert_eq!(
            (best.model.name(), best.components.len()),
            (WIDE_MODEL, WIDE_G)
        );
        assert!((best.bic - WIDE_BIC).abs() < 1e-6 * WIDE_BIC.abs());
        assert!((best.log_likelihood - WIDE_LOGLIK).abs() < 1e-6 * WIDE_LOGLIK.abs());
        for (row, expected) in data.iter().zip(WIDE_LOG_DENSITY) {
            assert!((best.log_density(row) - expected).abs() < 1e-5, "{row:?}");
        }
    }

    #[test]
    fn tall_data_matches_mclust_for_every_structure_it_fits() {
        let data = rows(&GMM_POINTS);
        let search = mclust(&data);
        assert_bic_table(&search.table, &GMM_DEFAULT_BIC_TABLE, &SEARCHED);
        // mclust's best (`VEV`) is a structure cytiq does not fit; cytiq picks
        // the best of the ones it does.
        let (g, column, bic) = GMM_DEFAULT_BIC_TABLE
            .iter()
            .enumerate()
            .flat_map(|(g, row)| {
                row.iter()
                    .enumerate()
                    .filter_map(move |(column, bic)| bic.map(|bic| (g + 1, column, bic)))
            })
            .max_by(|a, b| a.2.total_cmp(&b.2))
            .expect("a model fits");
        let best = search.best.expect("a model fits");
        assert_eq!((best.model, best.components.len()), (SEARCHED[column], g));
        assert!((best.bic - bic).abs() < 1e-6 * bic.abs());
        assert_eq!(GMM_DEFAULT_MODEL, "VEV");
    }

    #[test]
    fn one_component_fits_are_named_like_mclust() {
        assert_eq!(MclustModel::Vvi.single().name(), "XXI");
        assert_eq!(MclustModel::Vii.single().name(), "XII");
        assert_eq!(MclustModel::Eee.single().name(), "XXX");
        assert_eq!(MclustModel::Vei.parameters(20, 3), 3 * 20 + 3 + 19 + 2);
    }
}
