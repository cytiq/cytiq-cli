//! Batched decoding of DATA, for commands that process one batch of events at
//! a time instead of holding a whole file.

use std::io::{Read, Seek, SeekFrom};
use std::num::NonZeroUsize;

use crate::data::{new_column, read_rows};
use crate::error::FcsError;
use crate::layout::ResolvedLayout;
use crate::types::{ByteRange, EventColumns, FcsText, FcsVersion, Header, Parameter};

/// Everything an [`crate::FcsDataset`] holds except its events and ANALYSIS.
#[derive(Clone, Debug, PartialEq)]
pub struct FcsMetadata {
    pub version: FcsVersion,
    pub header: Header,
    pub text: FcsText,
    pub parameters: Vec<Parameter>,
    /// `$TOT`: the events the batches hold in all.
    pub event_count: usize,
}

/// A checked file's metadata and its events, batch by batch.
#[derive(Debug)]
pub struct FcsStream<R> {
    pub metadata: FcsMetadata,
    pub batches: EventBatches<R>,
}

/// Decodes DATA in batches of `batch_size` events, in file order; the last
/// batch holds the rest. Each batch has one column per parameter, typed as
/// [`crate::FcsReader::read`] types it. After an `Err` the iterator ends.
#[derive(Debug)]
pub struct EventBatches<R> {
    reader: R,
    layout: ResolvedLayout,
    batch_size: usize,
    remaining: usize,
    failed: bool,
}

impl<R: Read + Seek> EventBatches<R> {
    pub(crate) fn new(
        mut reader: R,
        data: ByteRange,
        layout: ResolvedLayout,
        batch_size: NonZeroUsize,
    ) -> Result<Self, FcsError> {
        reader
            .seek(SeekFrom::Start(data.start))
            .map_err(FcsError::Io)?;
        Ok(Self {
            reader,
            remaining: layout.row_count(),
            layout,
            batch_size: batch_size.get(),
            failed: false,
        })
    }
}

impl<R> EventBatches<R> {
    /// A batch of no events, with the columns and types every batch has.
    #[must_use]
    pub fn empty_batch(&self) -> EventColumns {
        self.columns(0)
    }

    fn columns(&self, rows: usize) -> EventColumns {
        EventColumns {
            columns: self
                .layout
                .parameters()
                .iter()
                .map(|parameter| new_column(parameter, rows))
                .collect(),
        }
    }
}

impl<R: Read> Iterator for EventBatches<R> {
    type Item = Result<EventColumns, FcsError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.failed || self.remaining == 0 {
            return None;
        }
        let rows = self.remaining.min(self.batch_size);
        let mut batch = self.columns(rows);
        match read_rows(&mut self.reader, &self.layout, rows, &mut batch.columns) {
            Ok(()) => {
                self.remaining -= rows;
                Some(Ok(batch))
            }
            Err(error) => {
                self.failed = true;
                Some(Err(error))
            }
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        if self.failed {
            return (0, Some(0));
        }
        let batches = self.remaining.div_ceil(self.batch_size);
        (0, Some(batches))
    }
}
