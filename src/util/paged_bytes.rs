//! A paged byte arena ported from `org.apache.lucene.util.PagedBytes`.
//!
//! | Rucene | Apache Lucene Core 10.5.0 |
//! | --- | --- |
//! | [`PagedBytes`] | `PagedBytes` |
//! | [`PagedBytesReader`] | `PagedBytes.Reader` |
//! | [`PagedBytesDataInput`] | `PagedBytes.PagedBytesDataInput` |
//! | [`PagedBytesDataOutput`] | `PagedBytes.PagedBytesDataOutput` |
//!
//! **Divergence from Lucene 10.5.0.** Java's blocks are `byte[]` references
//! shared between the writer and every `Reader`; `Reader` copies only the array
//! of references. This port holds the blocks as `Arc<Vec<u8>>`, which shares
//! them the same way, so freezing stays O(number of blocks). The one place the
//! sharing is not observable is [`PagedBytesReader::fill_slice`]: Java returns a
//! `BytesRef` pointing straight into a block when the slice does not span two
//! of them, whereas Rucene's [`BytesRef`] owns its buffer (see
//! [`crate::util::BytesRef`]) and must copy. The bytes returned are identical.

#![deny(unsafe_code)]

use std::sync::Arc;

use crate::error::{LuceneError, Result};
use crate::store::{DataInput, DataOutput, IndexInput};
use crate::util::{Accountable, BytesRef, RamUsageEstimator};

/// An empty block, used when nothing was ever written.
/// `PagedBytes.EMPTY_BYTES`.
const EMPTY_BYTES: &[u8] = &[];

/// A growable arena of fixed-size byte pages.
///
/// Port of `org.apache.lucene.util.PagedBytes`.
#[derive(Debug)]
pub struct PagedBytes {
    blocks: Vec<Arc<Vec<u8>>>,
    block_size: usize,
    block_bits: u32,
    block_mask: usize,
    did_skip_bytes: bool,
    frozen: bool,
    upto: usize,
    current_block: Option<Vec<u8>>,
    bytes_used_per_block: i64,
}

impl PagedBytes {
    /// Creates an arena whose pages hold `1 << block_bits` bytes.
    ///
    /// # Panics
    ///
    /// Panics unless `0 < block_bits <= 31`, which is Java's assertion.
    pub fn new(block_bits: u32) -> Self {
        assert!(block_bits > 0 && block_bits <= 31, "{block_bits}");
        let block_size = 1usize << block_bits;
        Self {
            blocks: Vec::new(),
            block_size,
            block_bits,
            block_mask: block_size - 1,
            did_skip_bytes: false,
            frozen: false,
            upto: block_size,
            current_block: None,
            bytes_used_per_block: RamUsageEstimator::align_object_size(
                block_size as i64 + RamUsageEstimator::NUM_BYTES_ARRAY_HEADER,
            ),
        }
    }

    /// Returns the page size in bytes.
    pub fn block_size(&self) -> usize {
        self.block_size
    }

    /// Equivalent to the private `PagedBytes.addBlock`.
    fn add_block(&mut self, block: Vec<u8>) {
        self.blocks.push(Arc::new(block));
    }

    /// Reads `byte_count` bytes from `input` into this arena.
    ///
    /// Equivalent to `PagedBytes.copy(IndexInput, long)`.
    ///
    /// # Errors
    ///
    /// Propagates the input's read errors.
    pub fn copy_from_input(&mut self, input: &mut dyn IndexInput, byte_count: i64) -> Result<()> {
        let mut byte_count = byte_count;
        while byte_count > 0 {
            let mut left = self.block_size - self.upto;
            if left == 0 {
                if let Some(block) = self.current_block.take() {
                    self.add_block(block);
                }
                self.current_block = Some(vec![0u8; self.block_size]);
                self.upto = 0;
                left = self.block_size;
            }
            let block = self
                .current_block
                .as_mut()
                .expect("INVARIANT: a block was just allocated");
            if (left as i64) < byte_count {
                input.read_bytes_buffered(block, self.upto, left, false)?;
                self.upto = self.block_size;
                byte_count -= left as i64;
            } else {
                let n = byte_count as usize;
                input.read_bytes_buffered(block, self.upto, n, false)?;
                self.upto += n;
                break;
            }
        }
        Ok(())
    }

    /// Copies `bytes` into this arena and returns a reference to the copy.
    ///
    /// Equivalent to `PagedBytes.copy(BytesRef, BytesRef)`; Java fills the
    /// caller's `out` reference, which Rucene's owning [`BytesRef`] returns
    /// instead.
    ///
    /// Using this method forbids a later [`PagedBytes::freeze`], exactly as in
    /// Java, because a value may be dropped at a page boundary.
    pub fn copy(&mut self, bytes: &BytesRef) -> BytesRef {
        let mut left = self.block_size - self.upto;
        if bytes.length > left || self.current_block.is_none() {
            if let Some(block) = self.current_block.take() {
                self.add_block(block);
                self.did_skip_bytes = true;
            }
            self.current_block = Some(vec![0u8; self.block_size]);
            self.upto = 0;
            left = self.block_size;
            debug_assert!(bytes.length <= self.block_size);
        }
        let _ = left;

        let start = self.upto;
        let block = self
            .current_block
            .as_mut()
            .expect("INVARIANT: a block is allocated at this point");
        block[start..start + bytes.length].copy_from_slice(bytes.slice());
        self.upto += bytes.length;
        BytesRef::new(bytes.slice().to_vec())
    }

    /// Freezes the arena and returns a reader over it.
    ///
    /// When `trim` is set, the last page is shrunk to the bytes actually used.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] when the arena was already frozen,
    /// or when [`PagedBytes::copy`] was used — both reproducing Lucene's
    /// `IllegalStateException`s.
    pub fn freeze(&mut self, trim: bool) -> Result<PagedBytesReader> {
        if self.frozen {
            return Err(LuceneError::IllegalState("already frozen".to_string()));
        }
        if self.did_skip_bytes {
            return Err(LuceneError::IllegalState(
                "cannot freeze when copy(BytesRef, BytesRef) was used".to_string(),
            ));
        }
        let mut current_block = self.current_block.take();
        if trim {
            if let Some(block) = current_block.as_mut() {
                if self.upto < self.block_size {
                    block.truncate(self.upto);
                }
            }
        }
        let block = current_block.unwrap_or_else(|| EMPTY_BYTES.to_vec());
        self.add_block(block);
        self.frozen = true;
        Ok(PagedBytesReader {
            blocks: self.blocks.clone(),
            block_bits: self.block_bits,
            block_mask: self.block_mask,
            block_size: self.block_size,
            bytes_used_per_block: self.bytes_used_per_block,
        })
    }

    /// Returns the offset of the next byte that would be written.
    ///
    /// Equivalent to `PagedBytes.getPointer()`.
    pub fn get_pointer(&self) -> i64 {
        if self.current_block.is_none() {
            0
        } else {
            (self.blocks.len() as i64) * (self.block_size as i64) + self.upto as i64
        }
    }

    /// Copies `bytes` behind a one- or two-byte length prefix and returns the
    /// offset it was written at.
    ///
    /// Equivalent to `PagedBytes.copyUsingLengthPrefix(BytesRef)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when the value is 32767 bytes
    /// or longer, or when the page size cannot hold it — both reproducing
    /// Lucene's messages.
    pub fn copy_using_length_prefix(&mut self, bytes: &BytesRef) -> Result<i64> {
        if bytes.length >= 32768 {
            return Err(LuceneError::IllegalArgument(format!(
                "max length is 32767 (got {})",
                bytes.length
            )));
        }

        if self.upto + bytes.length + 2 > self.block_size {
            if bytes.length + 2 > self.block_size {
                return Err(LuceneError::IllegalArgument(format!(
                    "block size {} is too small to store length {} bytes",
                    self.block_size, bytes.length
                )));
            }
            if let Some(block) = self.current_block.take() {
                self.add_block(block);
            }
            self.current_block = Some(vec![0u8; self.block_size]);
            self.upto = 0;
        }

        let pointer = self.get_pointer();

        let upto = self.upto;
        let length = bytes.length;
        let block = self
            .current_block
            .as_mut()
            .expect("INVARIANT: a block is allocated at this point");
        if length < 128 {
            block[upto] = length as u8;
            self.upto += 1;
        } else {
            // `BitUtil.VH_BE_SHORT.set(currentBlock, upto, (short) (length | 0x8000))`.
            let encoded = (length | 0x8000) as u16;
            block[upto] = (encoded >> 8) as u8;
            block[upto + 1] = encoded as u8;
            self.upto += 2;
        }
        let upto = self.upto;
        let block = self
            .current_block
            .as_mut()
            .expect("INVARIANT: a block is allocated at this point");
        block[upto..upto + length].copy_from_slice(bytes.slice());
        self.upto += length;

        Ok(pointer)
    }

    /// Returns a reader over the frozen content.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] when the arena is not frozen,
    /// reproducing Lucene's "must call freeze() before getDataInput".
    pub fn get_data_input(&self) -> Result<PagedBytesDataInput<'_>> {
        if !self.frozen {
            return Err(LuceneError::IllegalState(
                "must call freeze() before getDataInput".to_string(),
            ));
        }
        Ok(PagedBytesDataInput {
            blocks: &self.blocks,
            block_size: self.block_size,
            block_bits: self.block_bits,
            block_mask: self.block_mask,
            current_block_index: 0,
            current_block_upto: 0,
        })
    }

    /// Returns a writer appending to this arena.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] when the arena is frozen,
    /// reproducing Lucene's "cannot get DataOutput after freeze()".
    pub fn get_data_output(&mut self) -> Result<PagedBytesDataOutput<'_>> {
        if self.frozen {
            return Err(LuceneError::IllegalState(
                "cannot get DataOutput after freeze()".to_string(),
            ));
        }
        Ok(PagedBytesDataOutput { owner: self })
    }
}

impl Accountable for PagedBytes {
    fn ram_bytes_used(&self) -> i64 {
        let base = RamUsageEstimator::align_object_size(
            RamUsageEstimator::NUM_BYTES_OBJECT_HEADER
                + 4 * RamUsageEstimator::NUM_BYTES_OBJECT_REF,
        );
        let mut size = base + RamUsageEstimator::shallow_size_of(&self.blocks);
        if !self.blocks.is_empty() {
            size += (self.blocks.len() as i64 - 1) * self.bytes_used_per_block;
            size += RamUsageEstimator::size_of(self.blocks[self.blocks.len() - 1].as_slice());
        }
        if let Some(block) = self.current_block.as_ref() {
            size += RamUsageEstimator::size_of(block);
        }
        size
    }
}

/// A frozen, read-only view over a [`PagedBytes`].
///
/// Port of the nested class `PagedBytes.Reader`.
#[derive(Debug, Clone)]
pub struct PagedBytesReader {
    blocks: Vec<Arc<Vec<u8>>>,
    block_bits: u32,
    block_mask: usize,
    block_size: usize,
    bytes_used_per_block: i64,
}

impl PagedBytesReader {
    /// Returns the `length` bytes stored at `start`.
    ///
    /// Equivalent to `PagedBytes.Reader.fillSlice(BytesRef, long, int)`.
    ///
    /// # Panics
    ///
    /// Panics when `length` exceeds `block_size + 1`, which is Java's
    /// assertion.
    pub fn fill_slice(&self, start: i64, length: usize) -> BytesRef {
        debug_assert!(length <= self.block_size + 1, "length={length}");
        if length == 0 {
            return BytesRef::new(Vec::new());
        }
        let index = (start >> self.block_bits) as usize;
        let offset = (start as usize) & self.block_mask;
        if self.block_size - offset >= length {
            // Within one block.
            BytesRef::new(self.blocks[index][offset..offset + length].to_vec())
        } else {
            // Split across two blocks.
            let first = self.block_size - offset;
            let mut bytes = Vec::with_capacity(length);
            bytes.extend_from_slice(&self.blocks[index][offset..offset + first]);
            bytes.extend_from_slice(&self.blocks[index + 1][..length - first]);
            BytesRef::new(bytes)
        }
    }

    /// Returns the byte stored at `o`.
    ///
    /// Equivalent to `PagedBytes.Reader.getByte(long)`.
    pub fn get_byte(&self, o: i64) -> u8 {
        let index = (o >> self.block_bits) as usize;
        let offset = (o as usize) & self.block_mask;
        self.blocks[index][offset]
    }

    /// Returns the length-prefixed value stored at `start`.
    ///
    /// Equivalent to `PagedBytes.Reader.fill(BytesRef, long)`, the counterpart
    /// of [`PagedBytes::copy_using_length_prefix`].
    pub fn fill(&self, start: i64) -> BytesRef {
        let index = (start >> self.block_bits) as usize;
        let offset = (start as usize) & self.block_mask;
        let block = &self.blocks[index];

        if (block[offset] & 128) == 0 {
            let length = block[offset] as usize;
            BytesRef::new(block[offset + 1..offset + 1 + length].to_vec())
        } else {
            let encoded = ((block[offset] as u16) << 8) | block[offset + 1] as u16;
            let length = (encoded & 0x7FFF) as usize;
            debug_assert!(length > 0);
            BytesRef::new(block[offset + 2..offset + 2 + length].to_vec())
        }
    }

    /// Returns the page size in bytes.
    pub fn block_size(&self) -> usize {
        self.block_size
    }
}

impl Accountable for PagedBytesReader {
    fn ram_bytes_used(&self) -> i64 {
        let base = RamUsageEstimator::align_object_size(
            RamUsageEstimator::NUM_BYTES_OBJECT_HEADER
                + 4 * RamUsageEstimator::NUM_BYTES_OBJECT_REF,
        );
        let mut size = base + RamUsageEstimator::shallow_size_of(&self.blocks);
        if !self.blocks.is_empty() {
            size += (self.blocks.len() as i64 - 1) * self.bytes_used_per_block;
            size += RamUsageEstimator::size_of(self.blocks[self.blocks.len() - 1].as_slice());
        }
        size
    }
}

impl std::fmt::Display for PagedBytesReader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PagedBytes(blocksize={})", self.block_size)
    }
}

/// A [`DataInput`] over the frozen pages of a [`PagedBytes`].
///
/// Port of the nested class `PagedBytes.PagedBytesDataInput`.
#[derive(Debug, Clone)]
pub struct PagedBytesDataInput<'a> {
    blocks: &'a [Arc<Vec<u8>>],
    block_size: usize,
    block_bits: u32,
    block_mask: usize,
    current_block_index: usize,
    current_block_upto: usize,
}

impl PagedBytesDataInput<'_> {
    /// Returns the current position.
    pub fn get_position(&self) -> i64 {
        (self.current_block_index as i64) * (self.block_size as i64)
            + self.current_block_upto as i64
    }

    /// Moves to `pos`.
    pub fn set_position(&mut self, pos: i64) {
        self.current_block_index = (pos >> self.block_bits) as usize;
        self.current_block_upto = (pos as usize) & self.block_mask;
    }

    fn next_block(&mut self) {
        self.current_block_index += 1;
        self.current_block_upto = 0;
    }
}

impl DataInput for PagedBytesDataInput<'_> {
    fn read_byte(&mut self) -> Result<u8> {
        if self.current_block_upto == self.block_size {
            self.next_block();
        }
        let b = self.blocks[self.current_block_index][self.current_block_upto];
        self.current_block_upto += 1;
        Ok(b)
    }

    fn read_bytes(&mut self, b: &mut [u8], offset: usize, len: usize) -> Result<()> {
        debug_assert!(b.len() >= offset + len);
        let offset_end = offset + len;
        let mut offset = offset;
        loop {
            let block_left = self.block_size - self.current_block_upto;
            let left = offset_end - offset;
            if block_left < left {
                b[offset..offset + block_left].copy_from_slice(
                    &self.blocks[self.current_block_index]
                        [self.current_block_upto..self.current_block_upto + block_left],
                );
                self.next_block();
                offset += block_left;
            } else {
                // Last block.
                b[offset..offset + left].copy_from_slice(
                    &self.blocks[self.current_block_index]
                        [self.current_block_upto..self.current_block_upto + left],
                );
                self.current_block_upto += left;
                break;
            }
        }
        Ok(())
    }

    fn skip_bytes(&mut self, num_bytes: i64) -> Result<()> {
        if num_bytes < 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "numBytes must be >= 0, got {num_bytes}"
            )));
        }
        let skip_to = self.get_position() + num_bytes;
        self.set_position(skip_to);
        Ok(())
    }
}

/// A [`DataOutput`] appending to a [`PagedBytes`].
///
/// Port of the nested class `PagedBytes.PagedBytesDataOutput`.
#[derive(Debug)]
pub struct PagedBytesDataOutput<'a> {
    owner: &'a mut PagedBytes,
}

impl PagedBytesDataOutput<'_> {
    /// Returns the position the next byte will be written at.
    pub fn get_position(&self) -> i64 {
        self.owner.get_pointer()
    }
}

impl DataOutput for PagedBytesDataOutput<'_> {
    fn write_byte(&mut self, b: u8) -> Result<()> {
        if self.owner.upto == self.owner.block_size {
            if let Some(block) = self.owner.current_block.take() {
                self.owner.add_block(block);
            }
            self.owner.current_block = Some(vec![0u8; self.owner.block_size]);
            self.owner.upto = 0;
        }
        let upto = self.owner.upto;
        let block = self
            .owner
            .current_block
            .as_mut()
            .expect("INVARIANT: a block is allocated at this point");
        block[upto] = b;
        self.owner.upto += 1;
        Ok(())
    }

    fn write_bytes(&mut self, b: &[u8], offset: usize, length: usize) -> Result<()> {
        debug_assert!(
            b.len() >= offset + length,
            "b.length={} offset={offset} length={length}",
            b.len()
        );
        if length == 0 {
            return Ok(());
        }

        if self.owner.upto == self.owner.block_size {
            if let Some(block) = self.owner.current_block.take() {
                self.owner.add_block(block);
            }
            self.owner.current_block = Some(vec![0u8; self.owner.block_size]);
            self.owner.upto = 0;
        }

        let offset_end = offset + length;
        let mut offset = offset;
        loop {
            let left = offset_end - offset;
            let block_left = self.owner.block_size - self.owner.upto;
            let upto = self.owner.upto;
            let block = self
                .owner
                .current_block
                .as_mut()
                .expect("INVARIANT: a block is allocated at this point");
            if block_left < left {
                block[upto..upto + block_left].copy_from_slice(&b[offset..offset + block_left]);
                let full = self
                    .owner
                    .current_block
                    .take()
                    .expect("INVARIANT: a block is allocated at this point");
                self.owner.add_block(full);
                self.owner.current_block = Some(vec![0u8; self.owner.block_size]);
                self.owner.upto = 0;
                offset += block_left;
            } else {
                // Last block.
                block[upto..upto + left].copy_from_slice(&b[offset..offset + left]);
                self.owner.upto += left;
                break;
            }
        }
        Ok(())
    }
}
