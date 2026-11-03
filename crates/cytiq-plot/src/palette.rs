//! Colours. Sequential palettes are interpolated in sRGB between fixed stops,
//! so the same level always gets the same hex value.

use crate::config::Palette;

/// Events drawn as dots on a scatter plot (and contour outliers).
pub(crate) const POINT_COLOR: &str = "#4E79A7";
/// Events a mask removed. Kept out of [`CATEGORICAL`].
pub(crate) const REMOVED_COLOR: &str = "#E15759";
/// Gate outlines and labels.
pub(crate) const GATE_COLOR: &str = "#1F2933";
/// Events without a class label.
pub(crate) const UNLABELLED_COLOR: &str = "#C9C4C0";
/// Classes beyond the categorical palette.
pub(crate) const OTHER_COLOR: &str = "#7F7F7F";
pub(crate) const TEXT_COLOR: &str = "#1F2933";
pub(crate) const MUTED_TEXT_COLOR: &str = "#52606D";
pub(crate) const NOTE_COLOR: &str = "#9A3412";

/// Distinct class colours: Tableau 10 without its red, then Tableau 20 and D3
/// hues that stay apart from the removed-event red.
pub(crate) const CATEGORICAL: [&str; 20] = [
    "#4E79A7", "#F28E2B", "#59A14F", "#B07AA1", "#76B7B2", "#EDC948", "#9C755F", "#FF9DA7",
    "#17BECF", "#BCBD22", "#1F77B4", "#FF7F0E", "#2CA02C", "#9467BD", "#8C564B", "#E377C2",
    "#A0CBE8", "#FFBE7D", "#8CD17D", "#D4A6C8",
];

/// FlowJo-like pseudocolor stops (position, RGB).
const PSEUDOCOLOR: [(f64, [u8; 3]); 6] = [
    (0.0, [0x1F, 0x3F, 0xBF]),
    (0.25, [0x00, 0xB3, 0xFF]),
    (0.5, [0x2F, 0xD1, 0x2F]),
    (0.75, [0xFF, 0xE0, 0x00]),
    (0.9, [0xFF, 0x7F, 0x00]),
    (1.0, [0xE0, 0x00, 0x00]),
];

/// Viridis (matplotlib) at nine evenly spaced positions.
const VIRIDIS: [(f64, [u8; 3]); 9] = [
    (0.0, [0x44, 0x01, 0x54]),
    (0.125, [0x47, 0x2D, 0x7B]),
    (0.25, [0x3B, 0x52, 0x8B]),
    (0.375, [0x2C, 0x72, 0x8E]),
    (0.5, [0x21, 0x91, 0x8C]),
    (0.625, [0x28, 0xAE, 0x80]),
    (0.75, [0x5E, 0xC9, 0x62]),
    (0.875, [0xAD, 0xDC, 0x30]),
    (1.0, [0xFD, 0xE7, 0x25]),
];

/// Colour at `t` in `[0, 1]` (clamped) as `#rrggbb`.
pub(crate) fn sequential(palette: Palette, t: f64) -> String {
    let stops: &[(f64, [u8; 3])] = match palette {
        Palette::Pseudocolor => &PSEUDOCOLOR,
        Palette::Viridis => &VIRIDIS,
    };
    let t = if t.is_finite() {
        t.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let upper = stops
        .iter()
        .position(|(position, _)| *position >= t)
        .unwrap_or(stops.len() - 1)
        .max(1);
    let (start, from) = stops[upper - 1];
    let (end, to) = stops[upper];
    let fraction = if end > start {
        (t - start) / (end - start)
    } else {
        0.0
    };
    let channel = |index: usize| {
        let value =
            f64::from(from[index]) + (f64::from(to[index]) - f64::from(from[index])) * fraction;
        unit_to_byte(value / 255.0)
    };
    format!("#{:02X}{:02X}{:02X}", channel(0), channel(1), channel(2))
}

/// Colours of density levels `1..=levels`, lowest first.
pub(crate) fn levels(palette: Palette, levels: u16) -> Vec<String> {
    let span = f64::from(levels.saturating_sub(1).max(1));
    (0..levels)
        .map(|level| sequential(palette, f64::from(level) / span))
        .collect()
}

/// `value` in `[0, 1]` to `0..=255`, rounded.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the value is clamped to [0, 255] before the cast"
)]
fn unit_to_byte(value: f64) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palette_ends_match_their_stops() {
        assert_eq!(sequential(Palette::Pseudocolor, 0.0), "#1F3FBF");
        assert_eq!(sequential(Palette::Pseudocolor, 1.0), "#E00000");
        assert_eq!(sequential(Palette::Viridis, 0.0), "#440154");
        assert_eq!(sequential(Palette::Viridis, 1.0), "#FDE725");
        assert_eq!(sequential(Palette::Viridis, 0.5), "#21918C");
    }

    #[test]
    fn level_tables_have_one_colour_per_level() {
        let colours = levels(Palette::Pseudocolor, 64);
        assert_eq!(colours.len(), 64);
        assert_eq!(colours[0], "#1F3FBF");
        assert_eq!(colours[63], "#E00000");
    }

    #[test]
    fn categorical_palette_has_distinct_colours_without_the_removed_red() {
        let mut colours = CATEGORICAL.to_vec();
        colours.sort_unstable();
        colours.dedup();
        assert_eq!(colours.len(), CATEGORICAL.len());
        assert!(!CATEGORICAL.contains(&REMOVED_COLOR));
    }
}
