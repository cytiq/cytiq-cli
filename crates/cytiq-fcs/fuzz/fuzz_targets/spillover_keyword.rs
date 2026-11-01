#![no_main]

use cytiq_fcs::Spillover;
use libfuzzer_sys::fuzz_target;

// Any keyword value: parsing never panics, and an accepted matrix is square
// over its channels and finite.
fuzz_target!(|raw: &str| {
    if let Ok(spillover) = Spillover::parse_keyword(raw) {
        let size = spillover.size();
        assert_eq!(spillover.channels().len(), size);
        assert_eq!(spillover.matrix().len(), size * size);
        assert!(spillover.matrix().iter().all(|value| value.is_finite()));
    }
});
