//! Port of R's `stats::smooth.spline(seq_along(y), y, spar = spar)` as used by
//! `PeacoQC::MADOutliers`.
//!
//! The fit follows R's `sbart.c`/`sgram.f`/`stxwx.f`/`sslvrg.f`: cubic B-splines
//! on knots chosen by `.nknots.smspl`, penalty Gram matrix integrated with R's
//! `0.3330` coefficient, `lambda = r * 256^(3 * spar - 1)` with `r` the partial
//! trace ratio of `X'WX` to the penalty, and fitted values at the inputs.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::many_single_char_names,
    clippy::needless_range_loop
)]

/// Fitted values of `smooth.spline(seq_along(values), values, spar = spar)`.
///
/// Inputs shorter than four points are returned unchanged.
#[must_use]
pub fn smooth_spline_fitted(values: &[f64], spar: f64) -> Vec<f64> {
    let count = values.len();
    if count < 4 {
        return values.to_vec();
    }
    // x = 1..n scaled to [0, 1] as in smooth.spline's xbar
    let xbar = (0..count)
        .map(|index| index as f64 / (count - 1) as f64)
        .collect::<Vec<_>>();
    let knots = knot_sequence(&xbar);
    let coefficient_count = knots.len() - 4;

    let (xwy, hessian) = weighted_cross_products(&xbar, values, &knots, coefficient_count);
    let penalty = penalty_gram(&knots, coefficient_count);

    let (trace_hessian, trace_penalty) = (2..coefficient_count - 3).fold((0.0, 0.0), |acc, i| {
        (acc.0 + hessian[i][i], acc.1 + penalty[i][i])
    });
    let lambda = trace_hessian / trace_penalty * 256.0_f64.powf(3.0 * spar - 1.0);

    let system = (0..coefficient_count)
        .map(|row| {
            (0..coefficient_count)
                .map(|column| hessian[row][column] + lambda * penalty[row][column])
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let coefficients = cholesky_solve(system, xwy);

    xbar.iter()
        .map(|x| {
            let piece = knot_piece(&knots, *x);
            let basis = cubic_basis(&knots, piece, *x);
            (0..4).map(|k| basis[k] * coefficients[piece - 3 + k]).sum()
        })
        .collect()
}

/// R's `.nknots.smspl(n)`.
fn knot_count(n: usize) -> usize {
    if n < 50 {
        return n;
    }
    let (a1, a2, a3, a4) = (
        50_f64.log2(),
        100_f64.log2(),
        140_f64.log2(),
        200_f64.log2(),
    );
    let n_f = n as f64;
    let value = if n < 200 {
        2.0_f64.powf(a1 + (a2 - a1) * (n_f - 50.0) / 150.0)
    } else if n < 800 {
        2.0_f64.powf(a2 + (a3 - a2) * (n_f - 200.0) / 600.0)
    } else if n < 3200 {
        2.0_f64.powf(a3 + (a4 - a3) * (n_f - 800.0) / 2400.0)
    } else {
        200.0 + (n_f - 3200.0).powf(0.2)
    };
    value.trunc() as usize
}

/// `c(rep(xbar[1], 3), xbar[seq.int(1, n, length.out = nknots)], rep(xbar[n], 3))`.
fn knot_sequence(xbar: &[f64]) -> Vec<f64> {
    let n = xbar.len();
    let inner = knot_count(n);
    let step = (n - 1) as f64 / (inner - 1) as f64;
    let mut knots = vec![xbar[0]; 3];
    for index in 0..inner {
        // R indexes with the truncated double from seq.int; the last is exactly n
        let position = if index + 1 == inner {
            n
        } else {
            (1.0 + index as f64 * step).trunc() as usize
        };
        knots.push(xbar[position - 1]);
    }
    knots.extend([xbar[n - 1]; 3]);
    knots
}

/// Index `l` of the knot interval `[t_l, t_{l+1})` containing `x`, restricted
/// to non-degenerate intervals; the right boundary uses the last interval.
fn knot_piece(knots: &[f64], x: f64) -> usize {
    let last = knots.len() - 5;
    let mut piece = 3;
    while piece < last && knots[piece + 1] <= x {
        piece += 1;
    }
    piece
}

/// The four non-zero cubic B-splines `B_{l-3..=l}` on piece `l`, evaluated with
/// the piece's polynomial (so `x` may sit on the piece's right end).
fn cubic_basis(knots: &[f64], piece: usize, x: f64) -> [f64; 4] {
    let mut values = [1.0, 0.0, 0.0, 0.0];
    let mut left = [0.0; 4];
    let mut right = [0.0; 4];
    for j in 1..4 {
        left[j] = x - knots[piece + 1 - j];
        right[j] = knots[piece + j] - x;
        let mut saved = 0.0;
        for r in 0..j {
            let term = values[r] / (right[r + 1] + left[j - r]);
            values[r] = saved + right[r + 1] * term;
            saved = left[j - r] * term;
        }
        values[j] = saved;
    }
    values
}

/// Second derivatives of `B_{l-3..=l}` at `x` using piece `l`'s polynomial.
fn cubic_second_derivatives(knots: &[f64], piece: usize, x: f64) -> [f64; 4] {
    let ratio = |numerator: f64, denominator: f64| {
        if denominator == 0.0 {
            0.0
        } else {
            numerator / denominator
        }
    };
    let width = knots[piece + 1] - knots[piece];
    // order-2 (linear) B-splines active on the piece: N_{l-1}, N_l
    let mut linear = vec![0.0; knots.len()];
    linear[piece - 1] = (knots[piece + 1] - x) / width;
    linear[piece] = (x - knots[piece]) / width;
    let order3_derivative = |m: usize| {
        // d/dx B_{m,3} = 2 (N_{m,2}/(t_{m+2}-t_m) - N_{m+1,2}/(t_{m+3}-t_{m+1}))
        2.0 * (ratio(linear[m], knots[m + 2] - knots[m])
            - ratio(linear[m + 1], knots[m + 3] - knots[m + 1]))
    };
    let mut output = [0.0; 4];
    for (k, value) in output.iter_mut().enumerate() {
        let j = piece - 3 + k;
        *value = 3.0
            * (ratio(order3_derivative(j), knots[j + 3] - knots[j])
                - ratio(order3_derivative(j + 1), knots[j + 4] - knots[j + 1]));
    }
    output
}

/// `stxwx`: `X'W^2 y` and the banded `X'W^2 X`, here with unit weights.
fn weighted_cross_products(
    xbar: &[f64],
    values: &[f64],
    knots: &[f64],
    coefficient_count: usize,
) -> (Vec<f64>, Vec<Vec<f64>>) {
    let mut xwy = vec![0.0; coefficient_count];
    let mut hessian = vec![vec![0.0; coefficient_count]; coefficient_count];
    for (x, y) in xbar.iter().zip(values) {
        let piece = knot_piece(knots, *x);
        let basis = cubic_basis(knots, piece, *x);
        for a in 0..4 {
            let row = piece - 3 + a;
            xwy[row] += y * basis[a];
            for b in a..4 {
                let column = piece - 3 + b;
                hessian[row][column] += basis[a] * basis[b];
                hessian[column][row] = hessian[row][column];
            }
        }
    }
    (xwy, hessian)
}

/// `sgram`: integral of products of B-spline second derivatives, using R's
/// linear-in-interval formula with the `0.3330` coefficient.
fn penalty_gram(knots: &[f64], coefficient_count: usize) -> Vec<Vec<f64>> {
    let mut gram = vec![vec![0.0; coefficient_count]; coefficient_count];
    for piece in 3..coefficient_count {
        let width = knots[piece + 1] - knots[piece];
        if width <= 0.0 {
            continue;
        }
        let start = cubic_second_derivatives(knots, piece, knots[piece]);
        let end = cubic_second_derivatives(knots, piece, knots[piece + 1]);
        let slope = [
            end[0] - start[0],
            end[1] - start[1],
            end[2] - start[2],
            end[3] - start[3],
        ];
        for a in 0..4 {
            for b in a..4 {
                let contribution = width
                    * (start[a] * start[b]
                        + (slope[a] * start[b] + slope[b] * start[a]) * 0.5
                        + slope[a] * slope[b] * 0.3330);
                let (row, column) = (piece - 3 + a, piece - 3 + b);
                gram[row][column] += contribution;
                if row != column {
                    gram[column][row] += contribution;
                }
            }
        }
    }
    gram
}

fn cholesky_solve(mut matrix: Vec<Vec<f64>>, mut rhs: Vec<f64>) -> Vec<f64> {
    let n = rhs.len();
    for j in 0..n {
        let mut diagonal = matrix[j][j];
        for k in 0..j {
            diagonal -= matrix[j][k] * matrix[j][k];
        }
        let diagonal = diagonal.max(f64::MIN_POSITIVE).sqrt();
        matrix[j][j] = diagonal;
        for i in (j + 1)..n {
            let mut value = matrix[i][j];
            for k in 0..j {
                value -= matrix[i][k] * matrix[j][k];
            }
            matrix[i][j] = value / diagonal;
        }
    }
    for i in 0..n {
        for k in 0..i {
            rhs[i] -= matrix[i][k] * rhs[k];
        }
        rhs[i] /= matrix[i][i];
    }
    for i in (0..n).rev() {
        for k in (i + 1)..n {
            rhs[i] -= matrix[k][i] * rhs[k];
        }
        rhs[i] /= matrix[i][i];
    }
    rhs
}

#[cfg(test)]
#[allow(clippy::unreadable_literal, clippy::excessive_precision)]
mod tests {
    use super::*;

    #[test]
    fn knot_count_matches_r() {
        // .nknots.smspl(c(10, 60, 250, 1000, 5000))
        for (n, expected) in [(10, 10), (60, 52), (250, 102), (1000, 144), (5000, 204)] {
            assert_eq!(knot_count(n), expected, "n = {n}");
        }
    }

    #[test]
    fn fitted_values_match_r_smooth_spline() {
        for (values, expected) in [(&SHORT_Y[..], &SHORT_FIT[..]), (&LONG_Y[..], &LONG_FIT[..])] {
            let fitted = smooth_spline_fitted(values, 0.5);
            for (index, (actual, wanted)) in fitted.iter().zip(expected).enumerate() {
                assert!(
                    (actual - wanted).abs() < 1e-6,
                    "index {index}: {actual} vs {wanted}"
                );
            }
        }
    }

    include!("../../../data/fixtures/peacoqc/smooth_spline_reference.rs");
}
