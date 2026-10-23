//! Linear-scale event values, the input of compensation and Gating-ML.

use crate::types::FcsDataset;

/// Stored values of one parameter on a linear scale: `$PnE` anti-log scaling,
/// `$PnG` gain and `$TIMESTEP`, applied as `flowio` (and `FlowKit`) do.
///
/// # Panics
///
/// Panics when `parameter_index` is out of range.
#[must_use]
pub fn linear_values(dataset: &FcsDataset, parameter_index: usize) -> Vec<f64> {
    let parameter = &dataset.parameters[parameter_index];
    let mut values = dataset
        .data
        .column(parameter_index)
        .expect("every parameter has a data column")
        .to_f64_vec();
    let number = parameter.index;
    let keyword_number = |suffix: &str| {
        keyword(dataset, &format!("$P{number}{suffix}"))
            .and_then(|value| value.trim().parse::<f64>().ok())
    };

    let is_time = parameter.name.eq_ignore_ascii_case("time");
    if is_time
        && let Some(step) =
            keyword(dataset, "$TIMESTEP").and_then(|value| value.trim().parse::<f64>().ok())
    {
        for value in &mut values {
            *value *= step;
        }
    }

    let (decades, log_zero) = keyword(dataset, &format!("$P{number}E"))
        .and_then(|value| {
            let (decades, log_zero) = value.split_once(',')?;
            Some((
                decades.trim().parse::<f64>().ok()?,
                log_zero.trim().parse::<f64>().ok()?,
            ))
        })
        .unwrap_or((0.0, 0.0));
    if decades > 0.0
        && let Some(range) = keyword_number("R")
    {
        // The FCS standard says to use 1 for an invalid log(0) value of 0.
        let log_zero = if log_zero > 0.0 { log_zero } else { 1.0 };
        for value in &mut values {
            *value = 10_f64.powf(decades * *value / range) * log_zero;
        }
    }

    // flowio ignores gain on the time channel; a gain of 0 means "no gain".
    let gain = if is_time {
        None
    } else {
        keyword_number("G").filter(|gain| (*gain - 1.0).abs() > 0.0 && gain.abs() > 0.0)
    };
    if let Some(gain) = gain {
        for value in &mut values {
            *value /= gain;
        }
    }
    values
}

/// A TEXT keyword, matched exactly first and case-insensitively otherwise.
fn keyword<'a>(dataset: &'a FcsDataset, key: &str) -> Option<&'a str> {
    dataset.text.get(key).or_else(|| {
        dataset
            .text
            .iter()
            .find_map(|(candidate, value)| candidate.eq_ignore_ascii_case(key).then_some(value))
    })
}
