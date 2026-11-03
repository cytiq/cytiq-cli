//! Axes: how plotted values map to `[0, 1]` and where ticks go.
//!
//! A plotted value `v` lives in a *data space*: raw linear intensities, or
//! intensities already transformed by `cytiq transform` or a Gating-ML
//! transformation (`g`). The display scale `f` maps `v` onto the axis; the
//! visible range is kept in `f`-space, so only forward functions are needed.
//! Ticks on log-like axes sit at raw decades `r`, placed at `f(g(r))`.

use cytiq_transform::ValueTransform;
use cytiq_transform::scales::Logicle;

use crate::config::ARCSINH_COFACTOR;

/// Least distance between two tick labels, in pixels.
const MIN_LABEL_GAP: f64 = 22.0;
/// Least distance between two tick marks, in pixels.
const MIN_TICK_GAP: f64 = 4.0;
/// Least length of a decade, in pixels, for its minor ticks to be drawn
/// (decades squeezed into the linear region near zero get none).
const MIN_MINOR_DECADE: f64 = 30.0;
/// Quantiles of the automatic range; the outer 0.05% of events pile on the
/// axis edges.
const RANGE_QUANTILES: (f64, f64) = (0.0005, 0.9995);
/// Padding of the automatic range, as a fraction of its span.
const RANGE_PADDING: f64 = 0.05;
/// Raw decades considered for log-like ticks: `±10^0 ..= ±10^9`.
const MAX_DECADE: i32 = 9;
/// Pixels per linear tick.
const LINEAR_TICK_SPACING: f64 = 80.0;

/// The transform that turned raw intensities into stored values.
#[derive(Clone, Debug)]
pub(crate) enum DataTransform {
    /// `CYTIQ_TRANSFORM` of a `cytiq transform` output.
    File(ValueTransform),
    /// A Gating-ML `transformation-ref` of a gate dimension.
    Gating(cytiq_gating::Transform),
}

impl DataTransform {
    fn apply(&self, raw: f64) -> f64 {
        match self {
            Self::File(transform) => transform.apply(raw),
            Self::Gating(transform) => transform.apply(raw),
        }
    }

    fn is_log_like(&self) -> bool {
        match self {
            // Every `cytiq transform` method except `none`, which writes no keyword.
            Self::File(_) => true,
            Self::Gating(transform) => !matches!(transform, cytiq_gating::Transform::Linear { .. }),
        }
    }
}

/// Display scale `f` of an axis.
#[derive(Clone, Debug)]
pub(crate) enum Display {
    Linear,
    Logicle(Box<Logicle>),
    Arcsinh { cofactor: f64 },
}

impl Display {
    pub(crate) fn arcsinh() -> Self {
        Self::Arcsinh {
            cofactor: ARCSINH_COFACTOR,
        }
    }

    fn apply(&self, value: f64) -> f64 {
        match self {
            Self::Linear => value,
            Self::Logicle(logicle) => logicle.scale(value),
            Self::Arcsinh { cofactor } => (value / cofactor).asinh(),
        }
    }

    pub(crate) const fn is_linear(&self) -> bool {
        matches!(self, Self::Linear)
    }
}

/// A tick at `position` in `[0, 1]`; labelled ticks are major.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Tick {
    pub position: f64,
    pub label: Option<String>,
    pub major: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct Axis {
    pub title: String,
    data: Option<DataTransform>,
    display: Display,
    /// Visible range in `f`-space.
    lo: f64,
    hi: f64,
}

/// What an axis is built from.
pub(crate) struct AxisInput<'a> {
    pub title: String,
    pub data: Option<DataTransform>,
    pub display: Display,
    /// Every event's value in data space (non-finite values are ignored).
    pub values: &'a [f64],
    /// The values of the population a plot shows (empty without one): the
    /// automatic range widens to cover them.
    pub population: &'a [f64],
    /// Explicit range in data units.
    pub range: Option<(f64, f64)>,
    /// A raw linear scatter channel: its axis starts at 0 when no value is
    /// negative, as on the cytometer.
    pub starts_at_zero: bool,
}

impl Axis {
    /// Build an axis. The automatic range covers all events given, so plots
    /// of one file share axes whatever mask they show. A population beyond
    /// that range widens it: a rare bright population (`FlowCyt`'s mast cells,
    /// past the file's 99.95% quantile) would otherwise pile on an edge.
    ///
    /// Returns `None` when an explicit range maps to an empty or non-finite
    /// span on this scale.
    pub(crate) fn new(input: AxisInput<'_>) -> Option<Self> {
        let AxisInput {
            title,
            data,
            display,
            values,
            population,
            range,
            starts_at_zero,
        } = input;
        let mut axis = Self {
            title,
            data,
            display,
            lo: 0.0,
            hi: 1.0,
        };
        if let Some((lo, hi)) = range {
            let (lo, hi) = (axis.display.apply(lo), axis.display.apply(hi));
            if !(lo.is_finite() && hi.is_finite() && lo < hi) {
                return None;
            }
            axis.lo = lo;
            axis.hi = hi;
            return Some(axis);
        }
        let zero = axis.position_of_raw(0.0);
        let Some([low, high]) = quantiles(values, &axis.display) else {
            let center = zero.filter(|value| value.is_finite()).unwrap_or(0.0);
            axis.lo = center - 0.5;
            axis.hi = center + 0.5;
            return Some(axis);
        };
        let padding = RANGE_PADDING * (high - low);
        let (mut lo, mut hi) = (low - padding, high + padding);
        if let Some([population_low, population_high]) = quantiles(population, &axis.display) {
            lo = lo.min(population_low - padding);
            hi = hi.max(population_high + padding);
        }
        if axis.is_log_like()
            && let Some(zero) = zero.filter(|value| value.is_finite())
        {
            lo = lo.min(zero);
            hi = hi.max(zero);
        }
        if starts_at_zero && low >= 0.0 {
            lo = 0.0;
        }
        if hi - lo <= 1e-12 * lo.abs().max(1.0) {
            lo -= 0.5;
            hi += 0.5;
        }
        axis.lo = lo;
        axis.hi = hi;
        Some(axis)
    }

    /// Position in `[0, 1]` of a value in data space (outside the range for
    /// off-scale values, non-finite for invalid ones).
    pub(crate) fn unit(&self, value: f64) -> f64 {
        (self.display.apply(value) - self.lo) / (self.hi - self.lo)
    }

    /// Whether straight lines in data space stay straight on this axis.
    pub(crate) const fn is_linear(&self) -> bool {
        self.display.is_linear()
    }

    fn is_log_like(&self) -> bool {
        !self.display.is_linear() || self.data.as_ref().is_some_and(DataTransform::is_log_like)
    }

    /// `f(g(raw))`: where a raw intensity sits in `f`-space.
    fn position_of_raw(&self, raw: f64) -> Option<f64> {
        let value = match &self.data {
            Some(transform) => transform.apply(raw),
            None => raw,
        };
        let mapped = self.display.apply(value);
        mapped.is_finite().then_some(mapped)
    }

    /// Ticks for an axis `pixels` long: raw decades on log-like axes, "nice"
    /// 1-2-5 steps on linear ones. Labels closer than [`MIN_LABEL_GAP`] are
    /// dropped, larger decades first.
    pub(crate) fn ticks(&self, pixels: f64) -> Vec<Tick> {
        let mut ticks = if self.is_log_like() {
            self.decade_ticks(pixels)
        } else {
            self.linear_ticks(pixels)
        };
        ticks.sort_by(|a, b| a.position.total_cmp(&b.position));
        ticks
    }

    fn to_unit(&self, mapped: f64) -> f64 {
        (mapped - self.lo) / (self.hi - self.lo)
    }

    fn decade_ticks(&self, pixels: f64) -> Vec<Tick> {
        let visible = |position: f64| (-1e-9..=1.0 + 1e-9).contains(&position);
        // Label candidates in priority order: zero, then decades from the
        // largest down, positive before negative.
        let mut candidates = vec![(0.0, "0".to_owned())];
        for exponent in (0..=MAX_DECADE).rev() {
            let magnitude = 10_f64.powi(exponent);
            candidates.push((magnitude, decade_label(exponent, false)));
            candidates.push((-magnitude, decade_label(exponent, true)));
        }
        let near = |ticks: &[Tick], position: f64, gap: f64| {
            ticks
                .iter()
                .any(|tick| (tick.position - position).abs() * pixels < gap)
        };
        let mut ticks: Vec<Tick> = Vec::new();
        for (raw, label) in candidates {
            let Some(position) = self.raw_unit(raw) else {
                continue;
            };
            if !visible(position) {
                continue;
            }
            if !near(&ticks, position, MIN_LABEL_GAP) {
                ticks.push(Tick {
                    position,
                    label: Some(label),
                    major: true,
                });
            } else if !near(&ticks, position, MIN_TICK_GAP) {
                ticks.push(Tick {
                    position,
                    label: None,
                    major: true,
                });
            }
        }
        for exponent in 0..MAX_DECADE {
            let magnitude = 10_f64.powi(exponent);
            for sign in [1.0, -1.0] {
                let span = match (
                    self.raw_unit(sign * magnitude),
                    self.raw_unit(sign * magnitude * 10.0),
                ) {
                    (Some(start), Some(end)) => (end - start).abs() * pixels,
                    _ => 0.0,
                };
                if span < MIN_MINOR_DECADE {
                    continue;
                }
                for multiple in 2..=9 {
                    let Some(position) = self.raw_unit(sign * f64::from(multiple) * magnitude)
                    else {
                        continue;
                    };
                    if visible(position) && !near(&ticks, position, MIN_TICK_GAP) {
                        ticks.push(Tick {
                            position,
                            label: None,
                            major: false,
                        });
                    }
                }
            }
        }
        ticks
    }

    /// Unit position of a raw intensity, when it maps to a finite one.
    fn raw_unit(&self, raw: f64) -> Option<f64> {
        self.position_of_raw(raw).map(|value| self.to_unit(value))
    }

    fn linear_ticks(&self, pixels: f64) -> Vec<Tick> {
        // On a linear axis without a log-like data transform, `f` is the
        // identity (or affine `flin`), so ticks are in plotted units.
        let (lo, hi) = (self.lo, self.hi);
        let target = (pixels / LINEAR_TICK_SPACING).max(2.0);
        let step = nice_step((hi - lo) / target);
        let first = (lo / step).ceil();
        let last = (hi / step).floor();
        let mut ticks = Vec::new();
        let mut index = first;
        while index <= last {
            let value = index * step;
            let value = if value.abs() < step * 1e-9 {
                0.0
            } else {
                value
            };
            ticks.push(Tick {
                position: self.to_unit(value),
                label: Some(linear_label(value, step)),
                major: true,
            });
            index += 1.0;
        }
        ticks
    }
}

/// The 1-2-5 step at or above `raw`.
/// The [`RANGE_QUANTILES`] of `values` on `display`, ignoring non-finite
/// values; `None` when none is finite.
fn quantiles(values: &[f64], display: &Display) -> Option<[f64; 2]> {
    let mut mapped = values
        .iter()
        .map(|&value| display.apply(value))
        .filter(|value| value.is_finite())
        .collect::<Vec<_>>();
    if mapped.is_empty() {
        return None;
    }
    mapped.sort_unstable_by(f64::total_cmp);
    Some([
        cytiq_density::quantile_type7(&mapped, RANGE_QUANTILES.0),
        cytiq_density::quantile_type7(&mapped, RANGE_QUANTILES.1),
    ])
}

fn nice_step(raw: f64) -> f64 {
    if !(raw.is_finite() && raw > 0.0) {
        return 1.0;
    }
    let magnitude = 10_f64.powf(raw.log10().floor());
    let fraction = raw / magnitude;
    let nice = if fraction <= 1.0 {
        1.0
    } else if fraction <= 2.0 {
        2.0
    } else if fraction <= 5.0 {
        5.0
    } else {
        10.0
    };
    nice * magnitude
}

/// `value` with as many decimals as `step` needs, using `K`/`M` for large
/// steps (`50K`, `1.5M`), as `FlowJo` labels scatter axes.
fn linear_label(value: f64, step: f64) -> String {
    let (scale, suffix) = if step >= 100_000.0 && value.abs() >= 1e6 {
        (1e6, "M")
    } else if step >= 1000.0 {
        (1e3, "K")
    } else {
        (1.0, "")
    };
    let decimals = decimals_for(step / scale);
    let scaled = value / scale;
    let mut label = format!("{scaled:.decimals$}");
    if label.starts_with('-') && label.trim_start_matches(['-', '0', '.']).is_empty() {
        label.remove(0);
    }
    if scaled != 0.0 {
        label.push_str(suffix);
    }
    label.replace('-', "\u{2212}")
}

/// Decimals needed to print multiples of `step` exactly (at most 6).
fn decimals_for(step: f64) -> usize {
    let mut decimals = 0;
    while decimals < 6 {
        let scaled = step * 10_f64.powi(i32::try_from(decimals).unwrap_or(6));
        if (scaled - scaled.round()).abs() <= 1e-9 * scaled.abs().max(1.0) {
            break;
        }
        decimals += 1;
    }
    decimals
}

/// `10³`, `−10³`; `1` for `10⁰`.
fn decade_label(exponent: i32, negative: bool) -> String {
    let mut label = String::new();
    if negative {
        label.push('\u{2212}');
    }
    if exponent == 0 {
        label.push('1');
        return label;
    }
    label.push_str("10");
    let digits = exponent.to_string();
    for digit in digits.chars() {
        let superscript = match digit {
            '0' => '\u{2070}',
            '1' => '\u{00B9}',
            '2' => '\u{00B2}',
            '3' => '\u{00B3}',
            '4' => '\u{2074}',
            '5' => '\u{2075}',
            '6' => '\u{2076}',
            '7' => '\u{2077}',
            '8' => '\u{2078}',
            _ => '\u{2079}',
        };
        label.push(superscript);
    }
    label
}

#[cfg(test)]
mod tests {
    use super::*;

    fn axis(display: Display, data: Option<DataTransform>, values: &[f64]) -> Axis {
        Axis::new(AxisInput {
            title: "x".into(),
            data,
            display,
            values,
            population: &[],
            range: None,
            starts_at_zero: false,
        })
        .expect("valid axis")
    }

    fn labels(ticks: &[Tick]) -> Vec<&str> {
        ticks
            .iter()
            .filter_map(|tick| tick.label.as_deref())
            .collect()
    }

    #[test]
    fn linear_ticks_use_1_2_5_steps_and_k_suffix() {
        let axis = Axis::new(AxisInput {
            title: "FSC-A".into(),
            data: None,
            display: Display::Linear,
            values: &[0.0, 262_144.0],
            population: &[],
            range: Some((0.0, 262_144.0)),
            starts_at_zero: true,
        })
        .unwrap();
        let ticks = axis.ticks(480.0);
        assert_eq!(labels(&ticks), ["0", "50K", "100K", "150K", "200K", "250K"]);
        assert!((ticks[1].position - 50_000.0 / 262_144.0).abs() < 1e-12);
    }

    #[test]
    fn linear_labels_print_the_step_precision() {
        assert_eq!(linear_label(2.5, 0.5), "2.5");
        assert_eq!(linear_label(-1.0, 0.5), "\u{2212}1.0");
        assert_eq!(linear_label(1_500_000.0, 500_000.0), "1.5M");
        assert_eq!(linear_label(0.0, 0.1), "0.0");
        assert_eq!(nice_step(37.0), 50.0);
        assert_eq!(nice_step(0.012), 0.02);
    }

    #[test]
    fn decade_ticks_on_logicle_axis_include_zero_and_negative_decades() {
        let logicle = Logicle::new(262_144.0, 0.5, 4.5, 0.0).unwrap();
        let axis = Axis::new(AxisInput {
            title: "CD3".into(),
            data: None,
            display: Display::Logicle(Box::new(logicle)),
            values: &[],
            population: &[],
            range: Some((-5_000.0, 262_144.0)),
            starts_at_zero: false,
        })
        .unwrap();
        let ticks = axis.ticks(480.0);
        let labelled = labels(&ticks);
        for expected in ["0", "10³", "10⁴", "10⁵", "\u{2212}10³"] {
            assert!(labelled.contains(&expected), "{expected} in {labelled:?}");
        }
        assert!(ticks.iter().any(|tick| !tick.major));
        // Labels never crowd each other.
        let positions = ticks
            .iter()
            .filter(|tick| tick.label.is_some())
            .map(|tick| tick.position)
            .collect::<Vec<_>>();
        assert!(
            positions
                .windows(2)
                .all(|pair| (pair[1] - pair[0]) * 480.0 >= MIN_LABEL_GAP)
        );
    }

    #[test]
    fn transformed_axis_places_decades_through_the_file_transform() {
        let contract = cytiq_fcs::TransformContract {
            method: "arcsinh".into(),
            parameters: serde_json::json!({"cofactor": 150.0}),
        };
        let transform = ValueTransform::from_contract(&contract).unwrap();
        let stored = [
            0.0,
            (1000.0_f64 / 150.0).asinh(),
            (100_000.0_f64 / 150.0).asinh(),
        ];
        let axis = axis(
            Display::Linear,
            Some(DataTransform::File(transform)),
            &stored,
        );
        let ticks = axis.ticks(480.0);
        let thousand = ticks
            .iter()
            .find(|tick| tick.label.as_deref() == Some("10³"))
            .expect("10³ is labelled");
        assert!((thousand.position - axis.unit((1000.0_f64 / 150.0).asinh())).abs() < 1e-12);
    }

    #[test]
    fn auto_range_covers_the_values_with_padding() {
        let values = (0..=1000).map(f64::from).collect::<Vec<_>>();
        let axis = axis(Display::Linear, None, &values);
        assert!(axis.unit(0.0) > 0.0 && axis.unit(0.0) < 0.06);
        assert!(axis.unit(1000.0) < 1.0 && axis.unit(1000.0) > 0.94);
    }

    #[test]
    fn a_population_beyond_the_file_range_widens_it() {
        // Four bright events among 10,000: past the file's 99.95% quantile,
        // they would pile on the edge of the plot that shows only them.
        let mut values = (0..10_000)
            .map(|event| f64::from(event % 1000))
            .collect::<Vec<_>>();
        let bright = [5_000.0, 5_500.0, 6_000.0, 7_000.0];
        values[..4].copy_from_slice(&bright);
        let file = axis(Display::Linear, None, &values);
        assert!(file.unit(7_000.0) > 1.0, "off-scale without the population");
        let widened = Axis::new(AxisInput {
            title: "x".into(),
            data: None,
            display: Display::Linear,
            values: &values,
            population: &bright,
            range: None,
            starts_at_zero: false,
        })
        .unwrap();
        assert!(
            bright
                .iter()
                .all(|value| (0.0..1.0).contains(&widened.unit(*value)))
        );
        assert!(
            (widened.unit(0.0) - file.unit(0.0)).abs() < 0.2,
            "the low end stays"
        );
        // A population inside the file range changes nothing.
        let inside = Axis::new(AxisInput {
            title: "x".into(),
            data: None,
            display: Display::Linear,
            values: &values,
            population: &values[100..200],
            range: None,
            starts_at_zero: false,
        })
        .unwrap();
        assert_eq!((inside.lo, inside.hi), (file.lo, file.hi));
    }

    #[test]
    fn scatter_axes_start_at_zero() {
        let values = (100..=1000).map(f64::from).collect::<Vec<_>>();
        let axis = Axis::new(AxisInput {
            title: "FSC-A".into(),
            data: None,
            display: Display::Linear,
            values: &values,
            population: &[],
            range: None,
            starts_at_zero: true,
        })
        .unwrap();
        assert_eq!(axis.unit(0.0), 0.0);
    }

    #[test]
    fn single_valued_channel_gets_a_nonempty_range() {
        let axis = axis(Display::Linear, None, &[7.0; 50]);
        assert!((axis.unit(7.0) - 0.5).abs() < 1e-12);
        let empty = axis_without_values();
        assert!(empty.unit(0.0).is_finite());
    }

    fn axis_without_values() -> Axis {
        axis(Display::arcsinh(), None, &[f64::NAN, f64::INFINITY])
    }

    #[test]
    fn explicit_range_must_be_valid_on_the_scale() {
        let gating = cytiq_gating::Transform::Log {
            t: 262_144.0,
            m: 4.5,
        };
        let built = Axis::new(AxisInput {
            title: "x".into(),
            data: Some(DataTransform::Gating(gating)),
            display: Display::Linear,
            values: &[],
            population: &[],
            range: Some((1.0, 0.5)),
            starts_at_zero: false,
        });
        assert!(built.is_none());
    }
}
