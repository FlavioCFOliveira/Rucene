//! Port of `org.apache.lucene.util.fst.OnHeapFSTStore`.

use crate::error::{LuceneError, Result};
use crate::store::{DataInput, DataOutput};
use crate::util::Accountable;

use super::fst::BytesReader;
use super::fst_compiler::get_on_heap_reader_writer;
use super::fst_reader::FSTReader;
use super::read_write_data_output::ReadWriteDataOutput;
use super::reverse_bytes_reader::ReverseBytesReader;

/// Shallow size of the instance, mirroring
/// `RamUsageEstimator.shallowSizeOfInstance(OnHeapFSTStore.class)`.
const BASE_RAM_BYTES_USED: i64 = 24;

/// Stores an FST in memory, either in one contiguous array or, for very large
/// FSTs, in a paged [`ReadWriteDataOutput`].
///
/// Equivalent to `org.apache.lucene.util.fst.OnHeapFSTStore`.
pub struct OnHeapFSTStore {
    /// Used when the FST is very large (more than one block); otherwise
    /// `bytes_array` is set instead.
    data_output: Option<ReadWriteDataOutput>,
    /// Used at read time when the FST fits into a single array.
    bytes_array: Option<Vec<u8>>,
}

impl OnHeapFSTStore {
    /// Reads `num_bytes` of FST data from `input`.
    ///
    /// Equivalent to `new OnHeapFSTStore(int, DataInput, long)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `max_block_bits` is
    /// outside `1 ..= 30` or `num_bytes` is negative, and propagates read
    /// errors.
    pub fn new(max_block_bits: i32, input: &mut dyn DataInput, num_bytes: i64) -> Result<Self> {
        if !(1..=30).contains(&max_block_bits) {
            return Err(LuceneError::IllegalArgument(format!(
                "maxBlockBits should be 1 .. 30; got {max_block_bits}"
            )));
        }
        if num_bytes < 0 {
            return Err(LuceneError::CorruptIndex(format!(
                "invalid FST length {num_bytes}"
            )));
        }

        if num_bytes > (1i64 << max_block_bits) {
            // The FST is big: several pages are needed.
            let mut data_output = get_on_heap_reader_writer(max_block_bits)?;
            data_output.copy_bytes(input, num_bytes)?;
            data_output.freeze();
            Ok(Self {
                data_output: Some(data_output),
                bytes_array: None,
            })
        } else {
            // The FST fits into a single block: use the cheaper reader.
            let len = usize::try_from(num_bytes).map_err(|_| {
                LuceneError::CorruptIndex(format!("FST length {num_bytes} does not fit in memory"))
            })?;
            let mut bytes_array = vec![0u8; len];
            input.read_bytes(&mut bytes_array, 0, len)?;
            Ok(Self {
                data_output: None,
                bytes_array: Some(bytes_array),
            })
        }
    }
}

impl std::fmt::Debug for OnHeapFSTStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OnHeapFSTStore")
            .field("ramBytesUsed", &self.ram_bytes_used())
            .finish()
    }
}

impl Accountable for OnHeapFSTStore {
    fn ram_bytes_used(&self) -> i64 {
        let mut size = BASE_RAM_BYTES_USED;
        if let Some(bytes) = &self.bytes_array {
            size += bytes.len() as i64;
        } else if let Some(data_output) = &self.data_output {
            size += data_output.ram_bytes_used();
        }
        size
    }
}

impl FSTReader for OnHeapFSTStore {
    fn get_reverse_bytes_reader(&self) -> Result<Box<dyn BytesReader + '_>> {
        if let Some(bytes) = &self.bytes_array {
            Ok(Box::new(ReverseBytesReader::new(bytes)))
        } else {
            self.data_output
                .as_ref()
                .expect("INVARIANT: the constructor sets exactly one of the two backings")
                .get_reverse_bytes_reader()
        }
    }

    fn write_to(&self, out: &mut dyn DataOutput) -> Result<()> {
        if let Some(data_output) = &self.data_output {
            data_output.write_to(out)
        } else {
            let bytes = self
                .bytes_array
                .as_ref()
                .expect("INVARIANT: the constructor sets exactly one of the two backings");
            out.write_bytes(bytes, 0, bytes.len())
        }
    }
}
