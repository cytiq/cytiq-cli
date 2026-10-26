//! Preserve seeded event selections and isolation trees across rand upgrades.
//!
//! Integer sampling and algorithm selection follow rand 0.8.8, adapted from
//! <https://github.com/rust-random/rand/tree/0.8.8/src/seq/index.rs> and
//! src/distributions/uniform.rs. Copyright 2018 Developers of the Rand project;
//! adapted under MIT (see LICENSE-rand). We sort samples, so Floyd's final shuffle
//! is unnecessary. Do not substitute rand's version-dependent range samplers.

#![allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]

use std::collections::HashSet;

use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

fn bounded_u32(rng: &mut StdRng, range: u32, zone: u32) -> u32 {
    loop {
        let product = u64::from(rng.next_u32()) * u64::from(range);
        if product as u32 <= zone {
            return (product >> 32) as u32;
        }
    }
}

fn bounded_u64(rng: &mut StdRng, range: u64, zone: u64) -> u64 {
    loop {
        let product = u128::from(rng.next_u64()) * u128::from(range);
        if product as u64 <= zone {
            return (product >> 64) as u64;
        }
    }
}

fn single_u32(rng: &mut StdRng, range: u32) -> u32 {
    bounded_u32(rng, range, (range << range.leading_zeros()).wrapping_sub(1))
}

pub(crate) fn random_index(rng: &mut StdRng, length: usize) -> usize {
    assert!(length > 0);
    if usize::BITS == 32 {
        return single_u32(rng, length as u32) as usize;
    }
    let range = length as u64;
    bounded_u64(rng, range, (range << range.leading_zeros()).wrapping_sub(1)) as usize
}

pub(crate) fn sample_indices(length: usize, limit: usize, seed: u64) -> Vec<usize> {
    if length <= limit {
        return (0..length).collect();
    }
    let mut rng = StdRng::seed_from_u64(seed);
    let mut indices = if let Ok(length) = u32::try_from(length) {
        let amount = limit as u32;
        let inplace = if amount < 163 {
            let j = usize::from(length >= 500_000);
            let c = [[1.6, 8.0 / 45.0], [10.0, 70.0 / 9.0]];
            amount > 11 && (length as f32) < (c[1][j] + c[0][j] * amount as f32) * amount as f32
        } else {
            let c = if length < 500_000 { 270.0 } else { 330.0 / 9.0 };
            (length as f32) < c * amount as f32
        };
        let indices = if inplace {
            let mut indices = (0..length).collect::<Vec<_>>();
            for i in 0..amount {
                let j = i + single_u32(&mut rng, length - i);
                indices.swap(i as usize, j as usize);
            }
            indices.truncate(limit);
            indices
        } else if amount < 163 {
            let mut indices = Vec::with_capacity(limit);
            for j in length - amount..length {
                let index = single_u32(&mut rng, j + 1);
                indices.push(if indices.contains(&index) { j } else { index });
            }
            indices
        } else {
            let zone = u32::MAX - length.wrapping_neg() % length;
            let mut cache = HashSet::with_capacity(limit);
            let mut indices = Vec::with_capacity(limit);
            while indices.len() < limit {
                let index = bounded_u32(&mut rng, length, zone);
                if cache.insert(index) {
                    indices.push(index);
                }
            }
            indices
        };
        indices.into_iter().map(|index| index as usize).collect()
    } else {
        let range = length as u64;
        let zone = u64::MAX - range.wrapping_neg() % range;
        let mut cache = HashSet::with_capacity(limit);
        let mut indices = Vec::with_capacity(limit);
        while indices.len() < limit {
            let index = bounded_u64(&mut rng, range, zone) as usize;
            if cache.insert(index) {
                indices.push(index);
            }
        }
        indices
    };
    indices.sort_unstable();
    indices
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_selections_match_rand_0_8_for_every_sampling_algorithm() {
        // Fingerprints of sorted selections produced independently with rand 0.8.8.
        for (length, limit, expected) in [
            (100, 10, 0xfe24_ffa5_8676_a4a1),
            (100, 20, 0x629c_d063_1e1f_b443),
            (20_000, 1000, 0x6b0d_162f_c25e_c808),
            (20_000, 100, 0xaa50_bf2d_d5d5_a173),
            (1_000_000, 1000, 0x56b4_c6ef_5b13_4716),
        ] {
            let indices = sample_indices(length, limit, 42);
            assert_eq!(indices.len(), limit);
            assert!(indices.windows(2).all(|pair| pair[0] < pair[1]));
            assert!(indices.iter().all(|index| *index < length));
            let fingerprint = indices.iter().fold(0_u64, |hash, index| {
                hash.wrapping_mul(0x0100_0000_01b3) ^ *index as u64
            });
            assert_eq!(fingerprint, expected, "length={length}, limit={limit}");
        }
    }

    #[test]
    #[cfg(target_pointer_width = "64")]
    fn large_event_selections_match_rand_0_8() {
        let indices = sample_indices(5_000_000_000, 1000, 42);
        let fingerprint = indices.iter().fold(0_u64, |hash, index| {
            hash.wrapping_mul(0x0100_0000_01b3) ^ *index as u64
        });
        assert_eq!(fingerprint, 0x827e_2cf1_6daf_e039);
    }

    #[test]
    #[cfg(target_pointer_width = "64")]
    fn tree_feature_choices_match_rand_0_8() {
        let mut rng = StdRng::seed_from_u64(42);
        let choices = (0..10)
            .map(|_| random_index(&mut rng, 6))
            .collect::<Vec<_>>();
        assert_eq!(choices, [3, 3, 2, 0, 2, 4, 5, 0, 5, 3]);
    }

    #[test]
    fn sampling_handles_empty_and_complete_selections() {
        assert_eq!(sample_indices(100, 0, 42), Vec::<usize>::new());
        assert_eq!(sample_indices(0, 100, 42), Vec::<usize>::new());
        assert_eq!(sample_indices(3, 3, 42), [0, 1, 2]);
        assert_eq!(sample_indices(3, 100, 42), [0, 1, 2]);
    }
}
