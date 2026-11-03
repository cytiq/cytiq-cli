//! Deterministic event subsampling for fits and plots.

/// Evenly spaced subset of the events selected by `mask`, at most `limit`.
///
/// Deterministic (no randomness), so repeated runs fit identical gates and
/// draw identical plots. `limit == 0` means no limit. Indices are returned in
/// ascending order.
#[must_use]
pub fn sample_indices(mask: &[bool], limit: usize) -> Vec<usize> {
    let selected = mask
        .iter()
        .enumerate()
        .filter_map(|(index, keep)| keep.then_some(index))
        .collect::<Vec<_>>();
    if selected.len() <= limit || limit == 0 {
        return selected;
    }
    (0..limit)
        .map(|position| selected[position * selected.len() / limit])
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn samples_evenly_from_selected_events() {
        let mask = (0..1000).map(|index| index % 2 == 0).collect::<Vec<_>>();
        let sample = sample_indices(&mask, 100);
        assert_eq!(sample.len(), 100);
        assert!(sample.iter().all(|index| index % 2 == 0));
        assert_eq!(sample[0], 0);
        assert!(*sample.last().unwrap() > 900);
    }

    #[test]
    fn keeps_everything_under_the_limit() {
        assert_eq!(sample_indices(&[true, false, true], 10), [0, 2]);
    }
}
