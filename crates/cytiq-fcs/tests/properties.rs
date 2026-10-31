//! Properties over generated inputs. The reader must never panic on any bytes,
//! and whatever it accepts must be internally consistent; the writer must
//! round-trip every finite dataset it accepts.

use std::io::Cursor;

use cytiq_fcs::{ColumnData, FcsDataset, FcsReader, FcsWriteOptions, FcsWriteParameter, Spillover};
use proptest::prelude::*;

mod common;

use common::{build_fcs, encode_u16_rows};

fn fixtures() -> Vec<Vec<u8>> {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../../data/fixtures/fcs");
    let mut files = [
        "fcs3_0_integer.fcs",
        "fcs3_1_variable_integer.fcs",
        "fcs3_2_mixed.fcs",
    ]
    .iter()
    .map(|name| std::fs::read(format!("{root}/{name}")).expect("fixture exists"))
    .collect::<Vec<_>>();
    files.push(build_fcs(
        "FCS3.1",
        vec![
            ("$BYTEORD", "1,2,3,4"),
            ("$DATATYPE", "I"),
            ("$MODE", "L"),
            ("$NEXTDATA", "0"),
            ("$PAR", "1"),
            ("$TOT", "3"),
            ("$P1B", "16"),
            ("$P1E", "0,0"),
            ("$P1N", "Time"),
            ("$P1R", "1024"),
            ("$SPILLOVER", "1,Time,1"),
        ],
        Some(vec![("NOTE", "supplemental")]),
        encode_u16_rows(&[[1], [2], [3]], true),
        Some(b"analysis".to_vec()),
    ));
    files
}

/// What every dataset the reader returns must satisfy.
fn assert_consistent(dataset: &FcsDataset, file_len: usize) {
    assert_eq!(dataset.data.columns.len(), dataset.parameters.len());
    let events = dataset.event_count();
    assert!(
        dataset
            .data
            .columns
            .iter()
            .all(|column| column.len() == events)
    );
    assert!(
        events <= file_len,
        "{events} events in a {file_len}-byte file"
    );
    for (position, parameter) in dataset.parameters.iter().enumerate() {
        assert_eq!(usize::from(parameter.index), position + 1);
    }
}

fn mutated(bytes: &[u8], edits: &[(usize, u8)]) -> Vec<u8> {
    let mut bytes = bytes.to_vec();
    for &(position, value) in edits {
        let index = position % bytes.len();
        bytes[index] = value;
    }
    bytes
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn mutated_files_never_panic_and_accepted_files_are_consistent(
        fixture in 0..4_usize,
        edits in prop::collection::vec((any::<usize>(), any::<u8>()), 1..8),
        truncate in prop::option::of(any::<usize>()),
    ) {
        let mut bytes = mutated(&fixtures()[fixture], &edits);
        if let Some(length) = truncate {
            bytes.truncate(length % (bytes.len() + 1));
        }
        let file_len = bytes.len();
        if let Ok(dataset) = FcsReader::new(Cursor::new(bytes)).read() {
            assert_consistent(&dataset, file_len);
        }
    }

    #[test]
    fn arbitrary_bytes_never_panic(bytes in prop::collection::vec(any::<u8>(), 0..512)) {
        let file_len = bytes.len();
        if let Ok(dataset) = FcsReader::new(Cursor::new(bytes)).read() {
            assert_consistent(&dataset, file_len);
        }
    }

    #[test]
    fn finite_f32_datasets_round_trip(
        columns in (1..6_usize, 1..48_usize).prop_flat_map(|(parameters, rows)| {
            prop::collection::vec(
                prop::collection::vec(
                    any::<f32>().prop_filter("finite", |value| value.is_finite()),
                    rows,
                ),
                parameters,
            )
        }),
        note in "[A-Za-z0-9 ,.][A-Za-z0-9 ,.|]{0,23}",
    ) {
        let parameters = (0..columns.len())
            .map(|index| FcsWriteParameter::with_label(format!("P{index}"), format!("label {index}")))
            .collect::<Vec<_>>();
        let options = FcsWriteOptions::new()
            .with_text_keywords(vec![("NOTE".to_owned(), note.clone())]);
        let mut bytes = Vec::new();
        cytiq_fcs::write_fcs3_1_f32_to_writer_with_options(&mut bytes, &parameters, &columns, &options)
            .expect("finite data is writable");
        let dataset = FcsReader::new(Cursor::new(bytes)).read().expect("written file reads back");
        prop_assert_eq!(dataset.text.get("NOTE"), Some(note.as_str()));
        for (index, column) in columns.iter().enumerate() {
            prop_assert_eq!(&dataset.parameters[index].name, &format!("P{index}"));
            match dataset.data.column(index) {
                Some(ColumnData::F32(values)) => {
                    let read = values.iter().map(|value| value.to_bits()).collect::<Vec<_>>();
                    let written = column.iter().map(|value| value.to_bits()).collect::<Vec<_>>();
                    prop_assert_eq!(read, written);
                }
                other => prop_assert!(false, "unexpected column {:?}", other),
            }
        }
    }

    #[test]
    fn spillover_parsing_never_panics_and_accepts_only_square_finite_matrices(
        raw in "[0-9]{0,22}(,[-+0-9.eEA-Za-z ]{0,8}){0,12}",
    ) {
        if let Ok(spillover) = Spillover::parse_keyword(&raw) {
            let size = spillover.size();
            prop_assert!(size > 0);
            prop_assert_eq!(spillover.channels().len(), size);
            prop_assert_eq!(spillover.matrix().len(), size * size);
            prop_assert!(spillover.matrix().iter().all(|value| value.is_finite()));
        }
    }

    #[test]
    fn writer_refuses_text_values_it_cannot_represent(value in "(\\|[A-Za-z|]{0,8})?") {
        let options = FcsWriteOptions::new().with_text_keywords(vec![("NOTE".to_owned(), value)]);
        let error = cytiq_fcs::write_fcs3_1_f32_to_writer_with_options(
            &mut Vec::new(),
            &[FcsWriteParameter::new("P0")],
            &[vec![1.0]],
            &options,
        )
        .expect_err("an empty or delimiter-led value cannot be written");
        prop_assert!(error.to_string().contains("must not start with"));
    }
}
