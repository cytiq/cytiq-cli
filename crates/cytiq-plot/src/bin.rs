//! 2-D histogram on the unit square and its colour levels.

/// Event counts on a `bins × bins` grid over `[0, 1]²`, row-major with row 0
/// at the bottom (`y` near 0).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Histogram {
    pub bins: usize,
    pub counts: Vec<u32>,
}

impl Histogram {
    /// Bin points given in unit coordinates. Points outside `[0, 1]` are
    /// clamped into the edge cells (`FlowJo` piles off-scale events on the
    /// axis); callers count them.
    pub(crate) fn new(bins: usize, points: impl IntoIterator<Item = [f64; 2]>) -> Self {
        let mut counts = vec![0_u32; bins * bins];
        for [x, y] in points {
            let cell = cell_index(y, bins) * bins + cell_index(x, bins);
            counts[cell] = counts[cell].saturating_add(1);
        }
        Self { bins, counts }
    }

    pub(crate) fn max(&self) -> u32 {
        self.counts.iter().copied().max().unwrap_or(0)
    }

    /// Cell index of a point in unit coordinates.
    pub(crate) fn cell_of(&self, [x, y]: [f64; 2]) -> usize {
        cell_index(y, self.bins) * self.bins + cell_index(x, self.bins)
    }

    /// Colour level of every cell: `0` for an empty cell, otherwise
    /// `1 + floor((levels − 1) · ln c / ln c_max)`, so a single event is level
    /// 1 and the densest cell is the top level.
    pub(crate) fn levels(&self, levels: u16) -> Vec<u16> {
        let max = self.max();
        let top = f64::from(levels.saturating_sub(1));
        let log_max = f64::from(max).ln();
        self.counts
            .iter()
            .map(|&count| match count {
                0 => 0,
                _ if max <= 1 => 1,
                _ => 1 + level_floor(top * f64::from(count).ln() / log_max, levels),
            })
            .collect()
    }
}

/// Cell of a unit coordinate; non-finite and out-of-range values are clamped.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    reason = "the scaled value is clamped to [0, bins - 1] before the cast"
)]
fn cell_index(unit: f64, bins: usize) -> usize {
    let scaled = (unit * bins as f64).floor();
    if scaled.is_nan() {
        return 0;
    }
    scaled.clamp(0.0, (bins - 1) as f64) as usize
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the value is clamped to [0, levels - 1] before the cast"
)]
fn level_floor(value: f64, levels: u16) -> u16 {
    value
        .floor()
        .clamp(0.0, f64::from(levels.saturating_sub(1))) as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn off_scale_events_pile_on_edge_bins() {
        let histogram = Histogram::new(4, [[-0.5, 0.5], [1.5, 0.5], [0.5, 2.0], [f64::NAN, 0.0]]);
        assert_eq!(histogram.counts[2 * 4], 1, "left edge, row 2");
        assert_eq!(histogram.counts[2 * 4 + 3], 1, "right edge, row 2");
        assert_eq!(histogram.counts[3 * 4 + 2], 1, "top row");
        assert_eq!(histogram.counts.iter().sum::<u32>(), 4);
    }

    #[test]
    fn highest_count_gets_the_top_level() {
        let points =
            std::iter::repeat_n([0.1, 0.1], 100).chain(std::iter::repeat_n([0.9, 0.9], 10));
        let histogram = Histogram::new(2, points);
        let levels = histogram.levels(64);
        assert_eq!(levels[0], 64);
        assert_eq!(levels[3], 1 + 31, "10 events: 63 · ln 10 / ln 100 = 31.5");
        assert_eq!(levels[1], 0, "empty cells have no level");
        let lone = Histogram::new(2, [[0.1, 0.1], [0.1, 0.1], [0.9, 0.9]]);
        assert_eq!(lone.levels(64)[3], 1);
        assert_eq!(lone.levels(64)[1], 0);
    }
}
