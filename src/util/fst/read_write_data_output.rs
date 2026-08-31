//! Port of `org.apache.lucene.util.fst.ReadWriteDataOutput`.

use crate::error::Result;
use crate::store::{DataInput, DataOutput};
use crate::util::Accountable;

use super::fst::BytesReader;
use super::fst_reader::FSTReader;
use super::reverse_bytes_reader::ReverseBytesReader;

/// A [`DataOutput`] that doubles as an [`FSTReader`], so that the FST is
/// readable immediately after being written.
///
/// Equivalent to the package-private `ReadWriteDataOutput`, which adapts
/// `ByteBuffersDataOutput(blockBits, blockBits, ALLOCATE_BB_ON_HEAP, NO_REUSE)`.
///
/// # Java to Rust adaptations
///
/// * Lucene delegates the storage to `ByteBuffersDataOutput` configured so that
///   every block is exactly `1 << blockBits` bytes; the reverse reader's
///   `pos >> blockBits` addressing depends on that. This crate's
///   `ByteBuffersDataOutput` allocates each block with `Vec::with_capacity`,
///   which is only guaranteed to reserve *at least* the requested size, so this
///   port keeps its own list of uniformly sized blocks. The `DataOutput`
///   contract and the bytes produced are unchanged.
/// * Lucene caches the buffer list in `freeze()` because
///   `toWriteableBufferList()` is costly. Here the blocks are already owned, so
///   `freeze()` only marks the output read-only.
pub struct ReadWriteDataOutput {
    blocks: Vec<Vec<u8>>,
    block_bits: u32,
    block_size: usize,
    block_mask: usize,
    /// Whether this output has been frozen and is now read-only.
    frozen: bool,
}

impl ReadWriteDataOutput {
    /// Creates an output whose blocks are `1 << block_bits` bytes each.
    ///
    /// Equivalent to `new ReadWriteDataOutput(int)`.
    ///
    /// # Errors
    ///
    /// Returns [`crate::error::LuceneError::IllegalArgument`] when `block_bits`
    /// is outside `1 ..= 31`, the range `ByteBuffersDataOutput` accepts.
    pub fn new(block_bits: i32) -> Result<Self> {
        if !(1..=31).contains(&block_bits) {
            return Err(crate::error::LuceneError::IllegalArgument(format!(
                "blockBits must be 1 .. 31; got {block_bits}"
            )));
        }
        let block_size = 1usize << block_bits;
        Ok(Self {
            blocks: Vec::new(),
            block_bits: block_bits as u32,
            block_size,
            block_mask: block_size - 1,
            frozen: false,
        })
    }

    /// Marks this output read-only so that it can be read back.
    ///
    /// Equivalent to `ReadWriteDataOutput.freeze`.
    pub fn freeze(&mut self) {
        self.frozen = true;
    }

    /// Returns whether [`ReadWriteDataOutput::freeze`] has been called.
    pub fn is_frozen(&self) -> bool {
        self.frozen
    }

    /// Returns the number of bytes written.
    pub fn size(&self) -> i64 {
        match self.blocks.last() {
            None => 0,
            Some(last) => {
                (self.blocks.len() as i64 - 1) * self.block_size as i64 + last.len() as i64
            }
        }
    }

    /// Returns the reverse [`BytesReader`], as [`FSTReader`] does, without
    /// going through the trait.
    ///
    /// Equivalent to `ReadWriteDataOutput.getReverseBytesReader`.
    ///
    /// # Errors
    ///
    /// This implementation never fails; the [`Result`] matches the
    /// [`FSTReader`] contract.
    pub fn get_reverse_bytes_reader(&self) -> Result<Box<dyn BytesReader + '_>> {
        debug_assert!(self.frozen, "freeze() must be called first");
        if self.blocks.len() <= 1 {
            // Use a faster implementation for the single-block case.
            let single: &[u8] = self.blocks.first().map_or(&[], |b| b.as_slice());
            Ok(Box::new(ReverseBytesReader::new(single)))
        } else {
            Ok(Box::new(BlockReverseBytesReader::new(
                &self.blocks,
                self.block_bits,
                self.block_size,
                self.block_mask,
            )))
        }
    }

    fn ensure_block(&mut self) {
        if self
            .blocks
            .last()
            .map_or(true, |block| block.len() == self.block_size)
        {
            self.blocks.push(Vec::with_capacity(self.block_size));
        }
    }
}

impl std::fmt::Debug for ReadWriteDataOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReadWriteDataOutput")
            .field("blockBits", &self.block_bits)
            .field("size", &self.size())
            .field("frozen", &self.frozen)
            .finish()
    }
}

impl DataOutput for ReadWriteDataOutput {
    fn write_byte(&mut self, b: u8) -> Result<()> {
        debug_assert!(!self.frozen);
        self.ensure_block();
        self.blocks
            .last_mut()
            .expect("INVARIANT: ensure_block pushed a block")
            .push(b);
        Ok(())
    }

    fn write_bytes(&mut self, b: &[u8], offset: usize, len: usize) -> Result<()> {
        debug_assert!(!self.frozen);
        let mut src = offset;
        let mut remaining = len;
        while remaining > 0 {
            self.ensure_block();
            let block_size = self.block_size;
            let block = self
                .blocks
                .last_mut()
                .expect("INVARIANT: ensure_block pushed a block");
            // `Vec::with_capacity` may reserve more than asked for, so the
            // block limit is the configured size, never the real capacity:
            // every block has to be exactly `1 << block_bits` bytes for the
            // reverse reader's `pos >> block_bits` addressing to hold.
            let space = block_size - block.len();
            let chunk = remaining.min(space);
            block.extend_from_slice(&b[src..src + chunk]);
            src += chunk;
            remaining -= chunk;
        }
        Ok(())
    }
}

impl Accountable for ReadWriteDataOutput {
    fn ram_bytes_used(&self) -> i64 {
        self.blocks.len() as i64 * self.block_size as i64
    }
}

impl FSTReader for ReadWriteDataOutput {
    fn get_reverse_bytes_reader(&self) -> Result<Box<dyn BytesReader + '_>> {
        ReadWriteDataOutput::get_reverse_bytes_reader(self)
    }

    fn write_to(&self, out: &mut dyn DataOutput) -> Result<()> {
        for block in &self.blocks {
            out.write_bytes(block, 0, block.len())?;
        }
        Ok(())
    }
}

/// Reverse reader over the block list of a [`ReadWriteDataOutput`].
///
/// Equivalent to the anonymous `FST.BytesReader` subclass returned by
/// `ReadWriteDataOutput.getReverseBytesReader` for the multi-block case.
struct BlockReverseBytesReader<'a> {
    blocks: &'a [Vec<u8>],
    block_bits: u32,
    block_size: usize,
    block_mask: usize,
    current: usize,
    next_buffer: i64,
    next_read: i64,
}

impl<'a> BlockReverseBytesReader<'a> {
    fn new(blocks: &'a [Vec<u8>], block_bits: u32, block_size: usize, block_mask: usize) -> Self {
        Self {
            blocks,
            block_bits,
            block_size,
            block_mask,
            current: 0,
            next_buffer: -1,
            next_read: 0,
        }
    }

    fn next(&mut self) -> Result<u8> {
        if self.next_read == -1 {
            let index = self.next_buffer;
            self.next_buffer = index - 1;
            if index < 0 || index as usize >= self.blocks.len() {
                return Err(crate::error::LuceneError::CorruptIndex(format!(
                    "FST block index {index} is outside the {} available blocks",
                    self.blocks.len()
                )));
            }
            self.current = index as usize;
            self.next_read = self.block_size as i64 - 1;
        }
        let pos = self.next_read;
        self.next_read = pos - 1;
        let block = &self.blocks[self.current];
        if pos < 0 || pos as usize >= block.len() {
            return Err(crate::error::LuceneError::CorruptIndex(format!(
                "FST read position {pos} is outside the {} bytes of block {}",
                block.len(),
                self.current
            )));
        }
        Ok(block[pos as usize])
    }
}

impl DataInput for BlockReverseBytesReader<'_> {
    fn read_byte(&mut self) -> Result<u8> {
        self.next()
    }

    fn read_bytes(&mut self, b: &mut [u8], offset: usize, len: usize) -> Result<()> {
        for i in 0..len {
            b[offset + i] = self.next()?;
        }
        Ok(())
    }

    fn skip_bytes(&mut self, num_bytes: i64) -> Result<()> {
        let pos = self.position() - num_bytes;
        self.set_position(pos);
        Ok(())
    }
}

impl BytesReader for BlockReverseBytesReader<'_> {
    fn position(&self) -> i64 {
        (self.next_buffer + 1) * self.block_size as i64 + self.next_read
    }

    fn set_position(&mut self, pos: i64) {
        let buffer_index = pos >> self.block_bits;
        if self.next_buffer != buffer_index - 1 {
            self.next_buffer = buffer_index - 1;
            if buffer_index >= 0 && (buffer_index as usize) < self.blocks.len() {
                self.current = buffer_index as usize;
            }
        }
        self.next_read = (pos as usize & self.block_mask) as i64;
    }

    fn as_data_input(&mut self) -> &mut dyn DataInput {
        self
    }
}
