//! Port of `org.apache.lucene.util.fst.GrowableByteArrayDataOutput`.

use crate::error::Result;
use crate::store::DataOutput;
use crate::util::{Accountable, ArrayUtil, RamUsageEstimator};

/// Initial size of the backing array.
///
/// Equivalent to `GrowableByteArrayDataOutput.INITIAL_SIZE`.
const INITIAL_SIZE: usize = 1 << 8;

/// Shallow size of the instance, mirroring
/// `RamUsageEstimator.shallowSizeOfInstance(GrowableByteArrayDataOutput.class)`.
const BASE_RAM_BYTES_USED: i64 = 24;

/// Holds a single contiguous byte array for the node of the FST currently being
/// written. The array only ever grows.
///
/// Equivalent to the package-private `GrowableByteArrayDataOutput`. It is only
/// safe for usage bounded in the number of bytes written; general callers
/// should use `ByteBuffersDataOutput` instead.
#[derive(Debug)]
pub struct GrowableByteArrayDataOutput {
    bytes: Vec<u8>,
    next_write: usize,
}

impl GrowableByteArrayDataOutput {
    /// Creates an output with the initial 256-byte array.
    ///
    /// Equivalent to `new GrowableByteArrayDataOutput()`.
    pub fn new() -> Self {
        Self {
            bytes: vec![0u8; INITIAL_SIZE],
            next_write: 0,
        }
    }

    /// Returns the current write position.
    ///
    /// Equivalent to `GrowableByteArrayDataOutput.getPosition`.
    pub fn position(&self) -> usize {
        self.next_write
    }

    /// Returns the whole backing array, including the bytes past the current
    /// position.
    ///
    /// Equivalent to `GrowableByteArrayDataOutput.getBytes`.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Returns the whole backing array for mutation.
    ///
    /// Equivalent to reading `GrowableByteArrayDataOutput.getBytes()` and
    /// writing into the returned array, which `FSTCompiler` does when it
    /// expands variable-length arcs into fixed-length ones.
    pub fn bytes_mut(&mut self) -> &mut [u8] {
        &mut self.bytes
    }

    /// Sets the write position, growing the array when needed.
    ///
    /// Equivalent to `GrowableByteArrayDataOutput.setPosition`.
    pub fn set_position(&mut self, new_len: usize) {
        if new_len > self.next_write {
            self.ensure_capacity(new_len - self.next_write);
        }
        self.next_write = new_len;
    }

    /// Ensures the array can hold `capacity_to_write` more bytes.
    ///
    /// Equivalent to the private `GrowableByteArrayDataOutput.ensureCapacity`.
    /// Lucene calls `ArrayUtil.grow`, which allocates a new array and copies;
    /// this port grows the `Vec` in place with the same
    /// [`ArrayUtil::oversize`] policy, so the resulting capacities match.
    fn ensure_capacity(&mut self, capacity_to_write: usize) {
        debug_assert!(capacity_to_write > 0);
        let min_size = self.next_write + capacity_to_write;
        if self.bytes.len() < min_size {
            let target = ArrayUtil::oversize(min_size, 1).max(min_size);
            self.bytes.resize(target, 0);
        }
    }

    /// Writes every byte written so far to `out`.
    ///
    /// Equivalent to `GrowableByteArrayDataOutput.writeTo(DataOutput)`.
    ///
    /// # Errors
    ///
    /// Propagates any error raised by the target output.
    pub fn write_to(&self, out: &mut dyn DataOutput) -> Result<()> {
        out.write_bytes(&self.bytes, 0, self.next_write)
    }

    /// Copies `len` bytes from this store, starting at `src_offset`, into
    /// `dest` at `dest_offset`.
    ///
    /// Equivalent to `GrowableByteArrayDataOutput.writeTo(int, byte[], int, int)`.
    pub fn copy_to_slice(
        &self,
        src_offset: usize,
        dest: &mut [u8],
        dest_offset: usize,
        len: usize,
    ) {
        debug_assert!(src_offset + len <= self.next_write);
        dest[dest_offset..dest_offset + len]
            .copy_from_slice(&self.bytes[src_offset..src_offset + len]);
    }

    /// Copies `len` bytes inside the backing array.
    ///
    /// This is the `System.arraycopy` that `FSTCompiler.writeScratchBytes`
    /// performs on `scratchBytes.getBytes()`; it is spelled out here because
    /// Rust cannot borrow the same array twice.
    pub fn copy_within(&mut self, src_offset: usize, dest_offset: usize, len: usize) {
        self.bytes
            .copy_within(src_offset..src_offset + len, dest_offset);
    }

    /// Reverses the first `position()` bytes of the backing array in place.
    ///
    /// This is `FSTCompiler.reverseScratchBytes`, kept next to the array it
    /// operates on. The write position is unchanged.
    pub fn reverse_written_bytes(&mut self) {
        let pos = self.next_write;
        self.bytes[..pos].reverse();
    }
}

impl Default for GrowableByteArrayDataOutput {
    fn default() -> Self {
        Self::new()
    }
}

impl DataOutput for GrowableByteArrayDataOutput {
    fn write_byte(&mut self, b: u8) -> Result<()> {
        self.ensure_capacity(1);
        self.bytes[self.next_write] = b;
        self.next_write += 1;
        Ok(())
    }

    fn write_bytes(&mut self, b: &[u8], offset: usize, len: usize) -> Result<()> {
        if len == 0 {
            return Ok(());
        }
        self.ensure_capacity(len);
        self.bytes[self.next_write..self.next_write + len]
            .copy_from_slice(&b[offset..offset + len]);
        self.next_write += len;
        Ok(())
    }
}

impl Accountable for GrowableByteArrayDataOutput {
    fn ram_bytes_used(&self) -> i64 {
        BASE_RAM_BYTES_USED + RamUsageEstimator::size_of(&self.bytes)
    }
}
