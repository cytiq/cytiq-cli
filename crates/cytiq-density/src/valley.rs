use crate::kde::Density;

/// Peaks lower than this fraction of the tallest peak are treated as noise.
pub const DEFAULT_MIN_PEAK_FRACTION: f64 = 0.05;

/// Two neighbouring peaks are separate populations only when the density
/// between them dips to at most this fraction of the lower peak.
pub const DEFAULT_MAX_VALLEY_RATIO: f64 = 0.5;

/// Density minimum separating two neighbouring populations.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Valley {
    pub x: f64,
    pub density: f64,
    pub left_peak_x: f64,
    pub right_peak_x: f64,
    /// `density / min(left peak, right peak)`; lower means better separated.
    pub ratio: f64,
}

/// Valleys between the well-separated populations of `estimate`, ordered by
/// `x`.
///
/// Peaks below `min_peak_fraction` of the tallest peak are dropped. Then,
/// while two neighbouring peaks are joined by a valley shallower than
/// `max_valley_ratio`, the lower of the two is merged into the other. An
/// empty result means the distribution is not clearly multimodal.
#[must_use]
pub fn find_valleys(
    estimate: &Density,
    min_peak_fraction: f64,
    max_valley_ratio: f64,
) -> Vec<Valley> {
    let y = &estimate.y;
    let maxima = local_maxima(y);
    let Some(tallest) = maxima.iter().map(|&index| y[index]).reduce(f64::max) else {
        return Vec::new();
    };
    let mut peaks = maxima
        .into_iter()
        .filter(|&index| y[index] >= tallest * min_peak_fraction)
        .collect::<Vec<_>>();

    loop {
        let shallowest = peaks
            .windows(2)
            .enumerate()
            .map(|(position, pair)| (position, valley_between(y, pair[0], pair[1]).1))
            .max_by(|left, right| left.1.total_cmp(&right.1));
        match shallowest {
            Some((position, ratio)) if ratio > max_valley_ratio => {
                let lower = if y[peaks[position]] < y[peaks[position + 1]] {
                    position
                } else {
                    position + 1
                };
                peaks.remove(lower);
            }
            _ => break,
        }
    }

    peaks
        .windows(2)
        .map(|pair| {
            let (index, ratio) = valley_between(y, pair[0], pair[1]);
            Valley {
                x: estimate.x[index],
                density: y[index],
                left_peak_x: estimate.x[pair[0]],
                right_peak_x: estimate.x[pair[1]],
                ratio,
            }
        })
        .collect()
}

/// Index of the density minimum between two peaks and its depth ratio.
fn valley_between(y: &[f64], left: usize, right: usize) -> (usize, f64) {
    let (offset, low) = y[left..=right]
        .iter()
        .enumerate()
        .min_by(|a, b| a.1.total_cmp(b.1))
        .expect("peak range is never empty");
    (left + offset, low / y[left].min(y[right]))
}

fn local_maxima(values: &[f64]) -> Vec<usize> {
    (0..values.len())
        .filter(|&index| {
            let left = index
                .checked_sub(1)
                .map_or(f64::NEG_INFINITY, |previous| values[previous]);
            let right = values.get(index + 1).copied().unwrap_or(f64::NEG_INFINITY);
            values[index] > left && values[index] >= right
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kde::density;

    fn normal_sample(center: f64, spread: f64, count: usize) -> Vec<f64> {
        // Deterministic quantiles of a normal via the logistic approximation.
        (1..=count)
            .map(|index| {
                #[allow(clippy::cast_precision_loss)]
                let p = index as f64 / (count + 1) as f64;
                center + spread * (p / (1.0 - p)).ln() / 1.702
            })
            .collect()
    }

    fn valleys(values: &[f64]) -> Vec<Valley> {
        find_valleys(
            &density(values),
            DEFAULT_MIN_PEAK_FRACTION,
            DEFAULT_MAX_VALLEY_RATIO,
        )
    }

    #[test]
    fn finds_valley_between_live_and_dead_peaks() {
        let mut values = normal_sample(1.0, 0.4, 850);
        values.extend(normal_sample(5.0, 0.6, 150));
        let found = valleys(&values);
        assert_eq!(found.len(), 1);
        let valley = found[0];
        assert!(valley.x > 2.0 && valley.x < 4.0, "valley at {}", valley.x);
        assert!((valley.left_peak_x - 1.0).abs() < 0.3);
        assert!((valley.right_peak_x - 5.0).abs() < 0.5);
    }

    #[test]
    fn finds_valley_when_right_population_dominates() {
        let mut values = normal_sample(1.0, 0.4, 200);
        values.extend(normal_sample(5.0, 0.6, 800));
        let found = valleys(&values);
        assert_eq!(found.len(), 1);
        assert!(found[0].x > 2.0 && found[0].x < 4.0);
    }

    #[test]
    fn orders_three_populations_by_position() {
        let mut values = normal_sample(0.0, 0.3, 400);
        values.extend(normal_sample(4.0, 0.3, 400));
        values.extend(normal_sample(8.0, 0.3, 400));
        let found = valleys(&values);
        assert_eq!(found.len(), 2);
        assert!(found[0].x < found[1].x);
    }

    #[test]
    fn reports_no_valley_for_unimodal_sample() {
        assert_eq!(valleys(&normal_sample(2.0, 0.5, 1000)), Vec::new());
    }
}
