#![no_main]

use std::io::Cursor;

use cytiq_fcs::FcsReader;
use libfuzzer_sys::fuzz_target;

// Any bytes: the reader returns `Ok` or `Err`, never panics, aborts or hangs,
// and what it accepts is consistent.
fuzz_target!(|bytes: &[u8]| {
    if let Ok(dataset) = FcsReader::new(Cursor::new(bytes)).read() {
        let events = dataset.event_count();
        assert_eq!(dataset.data.columns.len(), dataset.parameters.len());
        assert!(dataset.data.columns.iter().all(|column| column.len() == events));
        assert!(events <= bytes.len());
    }
});
