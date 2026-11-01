//! Properties of masks over generated inputs: Parquet round-trips exactly, and
//! `combine` matches rows by `source_event_id`, never by position.

use std::collections::HashMap;

use cytiq_mask::EventMask;
use proptest::prelude::*;

const ACQUISITION: &str = "0000000000000000000000000000000000000000000000000000000000000000";

/// Distinct source ids in a shuffled order, with a keep flag and one optional
/// filter value each.
fn rows() -> impl Strategy<Value = Vec<(u64, bool, Option<bool>)>> {
    prop::collection::hash_set(0..1_000_u64, 1..64)
        .prop_map(|ids| ids.into_iter().collect::<Vec<_>>())
        .prop_flat_map(|ids| {
            let count = ids.len();
            (
                Just(ids).prop_shuffle(),
                prop::collection::vec(any::<bool>(), count),
                prop::collection::vec(prop::option::of(any::<bool>()), count),
            )
        })
        .prop_map(|(ids, keep, filter)| {
            ids.into_iter()
                .zip(keep)
                .zip(filter)
                .map(|((id, keep), filter)| (id, keep, filter))
                .collect()
        })
}

fn mask(rows: &[(u64, bool, Option<bool>)]) -> EventMask {
    EventMask::new(
        ACQUISITION,
        rows.iter().map(|row| row.1).collect(),
        Some(rows.iter().map(|row| row.0).collect()),
    )
    .with_filter("check", rows.iter().map(|row| row.2).collect())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn parquet_round_trips_every_mask(rows in rows()) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("mask.parquet");
        let original = mask(&rows);
        original.write_parquet(&path).unwrap();
        prop_assert_eq!(EventMask::read_parquet(&path).unwrap(), original);
    }

    #[test]
    fn combine_matches_rows_by_source_id(first in rows(), second in rows()) {
        let combined = EventMask::combine(&[mask(&first), mask(&second)]).unwrap();
        let other = second
            .iter()
            .map(|(id, keep, filter)| (*id, (*keep, *filter)))
            .collect::<HashMap<_, _>>();
        prop_assert_eq!(combined.len(), first.len());
        prop_assert_eq!(
            &combined.source_event_ids,
            &first.iter().map(|row| row.0).collect::<Vec<_>>()
        );
        for (index, (id, keep, filter)) in first.iter().enumerate() {
            let expected = *keep && other.get(id).is_some_and(|(keep, _)| *keep);
            prop_assert_eq!(combined.keep[index], expected);
            prop_assert_eq!(combined.filters[0].1[index], *filter);
            prop_assert_eq!(
                combined.filters[1].1[index],
                other.get(id).and_then(|(_, filter)| *filter)
            );
        }
        let rejected = combined.keep.iter().filter(|keep| !**keep).count();
        prop_assert_eq!(combined.kept() + rejected, combined.len());
    }
}
