//! Linear-scale event values, the input of compensation and Gating-ML.

use crate::error::FcsError;
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

    let (decades, log_zero) = log_amplification(dataset, number);
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

/// `$PnN` of every parameter stored on a log scale (`$PnE` with non-zero
/// decades). Only [`linear_values`] turns such values into intensities.
#[must_use]
pub fn log_amplified_parameters(dataset: &FcsDataset) -> Vec<&str> {
    dataset
        .parameters
        .iter()
        .filter(|parameter| log_amplification(dataset, parameter.index).0 > 0.0)
        .map(|parameter| parameter.name.as_str())
        .collect()
}

/// Commands that read stored values directly call this first: on a
/// log-amplified parameter they would treat log channel numbers as
/// intensities, and the FCS they write declares `$PnE=0,0`.
///
/// # Errors
///
/// Returns [`FcsError::LogAmplified`] naming every log-amplified parameter.
pub fn ensure_linear_scale(dataset: &FcsDataset) -> Result<(), FcsError> {
    let parameters = log_amplified_parameters(dataset);
    if parameters.is_empty() {
        Ok(())
    } else {
        Err(FcsError::LogAmplified {
            parameters: parameters
                .iter()
                .map(|name| format!("`{name}`"))
                .collect::<Vec<_>>()
                .join(", "),
        })
    }
}

/// `$PnE` of parameter `number` as (decades, log zero); `(0, 0)` for linear
/// or unreadable values.
fn log_amplification(dataset: &FcsDataset, number: u16) -> (f64, f64) {
    keyword(dataset, &format!("$P{number}E"))
        .and_then(|value| {
            let (decades, log_zero) = value.split_once(',')?;
            Some((
                decades.trim().parse::<f64>().ok()?,
                log_zero.trim().parse::<f64>().ok()?,
            ))
        })
        .unwrap_or((0.0, 0.0))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FcsReader, FcsWriteParameter};

    /// One 1024-channel parameter per `$PnE` value. The writer only emits
    /// `0,0`, so the TEXT is patched in place (same length, same offsets).
    fn dataset(scales: &[&str]) -> FcsDataset {
        let parameters = (1..=scales.len())
            .map(|number| FcsWriteParameter::new(format!("FL{number}-H")).with_range(1024.0))
            .collect::<Vec<_>>();
        let mut bytes = Vec::new();
        crate::write_fcs3_1_f32_to_writer(
            &mut bytes,
            &parameters,
            &vec![vec![0.0_f32, 512.0]; scales.len()],
        )
        .unwrap();
        for (number, scale) in (1..).zip(scales) {
            let linear = format!("$P{number}E|0,0|").into_bytes();
            let start = bytes
                .windows(linear.len())
                .position(|window| window == linear)
                .unwrap();
            bytes[start..start + linear.len()]
                .copy_from_slice(format!("$P{number}E|{scale}|").as_bytes());
        }
        FcsReader::new(std::io::Cursor::new(bytes)).read().unwrap()
    }

    #[test]
    fn finds_log_amplified_parameters() {
        let dataset = dataset(&["0,0", "4,1", "4,0"]);
        assert_eq!(log_amplified_parameters(&dataset), ["FL2-H", "FL3-H"]);
        let error = ensure_linear_scale(&dataset).unwrap_err().to_string();
        assert!(error.starts_with("`FL2-H`, `FL3-H` store"), "{error}");
        // Channel 512 of 1024 over four decades is 10^2.
        assert_eq!(linear_values(&dataset, 1), [1.0, 100.0]);
        assert!(ensure_linear_scale(&self::dataset(&["0,0"])).is_ok());
    }
}
