//! Port of `org.apache.lucene.util.fst.OffHeapFSTStore`.

use crate::error::{LuceneError, Result};
use crate::store::{DataOutput, IndexInput};
use crate::util::Accountable;

use super::fst::{BytesReader, FSTMetadata};
use super::fst_reader::FSTReader;
use super::outputs::Outputs;
use super::reverse_random_access_reader::ReverseRandomAccessReader;

/// Shallow size of the instance, mirroring
/// `RamUsageEstimator.shallowSizeOfInstance(OffHeapFSTStore.class)`.
const BASE_RAM_BYTES_USED: i64 = 32;

/// Stores an FST off heap, reading it from the underlying [`IndexInput`]
/// instead of from a byte store on the heap.
///
/// Equivalent to `org.apache.lucene.util.fst.OffHeapFSTStore`.
pub struct OffHeapFSTStore {
    input: Box<dyn IndexInput>,
    offset: i64,
    num_bytes: i64,
}

impl OffHeapFSTStore {
    /// Creates a store reading `metadata.num_bytes()` bytes from `input`
    /// starting at `offset`.
    ///
    /// Equivalent to `new OffHeapFSTStore(IndexInput, long, FST.FSTMetadata)`.
    pub fn new<O: Outputs>(
        input: Box<dyn IndexInput>,
        offset: i64,
        metadata: &FSTMetadata<O>,
    ) -> Self {
        Self {
            input,
            offset,
            num_bytes: metadata.num_bytes(),
        }
    }

    /// Returns the number of FST bytes held off heap.
    ///
    /// Equivalent to `OffHeapFSTStore.size`.
    pub fn size(&self) -> i64 {
        self.num_bytes
    }
}

impl std::fmt::Debug for OffHeapFSTStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OffHeapFSTStore")
            .field("offset", &self.offset)
            .field("numBytes", &self.num_bytes)
            .finish()
    }
}

impl Accountable for OffHeapFSTStore {
    fn ram_bytes_used(&self) -> i64 {
        BASE_RAM_BYTES_USED
    }
}

impl FSTReader for OffHeapFSTStore {
    fn get_reverse_bytes_reader(&self) -> Result<Box<dyn BytesReader + '_>> {
        let slice = self
            .input
            .random_access_slice(self.offset, self.num_bytes)?;
        Ok(Box::new(ReverseRandomAccessReader::new(slice)))
    }

    fn write_to(&self, _out: &mut dyn DataOutput) -> Result<()> {
        Err(LuceneError::UnsupportedOperation(
            "writeToOutput operation is not supported for OffHeapFSTStore".to_string(),
        ))
    }
}
