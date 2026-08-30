//! Port of `org.apache.lucene.util.fst.ByteBlockPoolReverseBytesReader`.

use crate::error::{LuceneError, Result};
use crate::store::DataInput;
use crate::util::byte_block_pool::{
    ByteBlockPool, BYTE_BLOCK_MASK, BYTE_BLOCK_SHIFT, BYTE_BLOCK_SIZE,
};

use super::fst::BytesReader;

/// Reads in reverse from a [`ByteBlockPool`].
///
/// Equivalent to the package-private `ByteBlockPoolReverseBytesReader`, used by
/// `NodeHash` to compare a candidate node against the copy of an already frozen
/// node that the suffix cache keeps.
///
/// The pool holds a copy of the node bytes at an address of its own, while the
/// rest of the FST addresses that node by its address in the FST byte stream.
/// [`ByteBlockPoolReverseBytesReader::set_pos_delta`] records the difference so
/// that the reader can be driven with FST addresses.
///
/// # Java to Rust adaptations
///
/// * Lucene 10.5.0 gives `ByteBlockPool` the `append`, `readByte(long)`,
///   `readBytes(long, ...)` and `getPosition()` methods this reader and
///   `NodeHash` rely on. This crate's `ByteBlockPool` does not expose them yet,
///   so they are provided here as free functions built on the pool's public
///   block API. The block layout, and therefore every address, is the same:
///   blocks are filled completely and a value may straddle two of them.
pub struct ByteBlockPoolReverseBytesReader<'a> {
    buf: &'a ByteBlockPool,
    /// Difference between the FST node address and the address of the copy
    /// held by the hash table.
    pos_delta: i64,
    pos: i64,
}

impl<'a> ByteBlockPoolReverseBytesReader<'a> {
    /// Creates a reader over `buf`, positioned at `0` with a zero delta.
    ///
    /// Equivalent to `new ByteBlockPoolReverseBytesReader(ByteBlockPool)`.
    pub fn new(buf: &'a ByteBlockPool) -> Self {
        Self {
            buf,
            pos_delta: 0,
            pos: 0,
        }
    }

    /// Sets the difference between FST addresses and pool addresses.
    ///
    /// Equivalent to `ByteBlockPoolReverseBytesReader.setPosDelta`.
    pub fn set_pos_delta(&mut self, pos_delta: i64) {
        self.pos_delta = pos_delta;
    }

    fn next(&mut self) -> Result<u8> {
        let pos = self.pos;
        self.pos = pos - 1;
        pool_read_byte(self.buf, pos)
    }
}

impl std::fmt::Debug for ByteBlockPoolReverseBytesReader<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ByteBlockPoolReverseBytesReader")
            .field("pos", &self.pos)
            .field("posDelta", &self.pos_delta)
            .finish()
    }
}

impl DataInput for ByteBlockPoolReverseBytesReader<'_> {
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
        self.pos -= num_bytes;
        Ok(())
    }
}

impl BytesReader for ByteBlockPoolReverseBytesReader<'_> {
    fn position(&self) -> i64 {
        self.pos + self.pos_delta
    }

    fn set_position(&mut self, pos: i64) {
        self.pos = pos - self.pos_delta;
    }

    fn as_data_input(&mut self) -> &mut dyn DataInput {
        self
    }
}

/// Returns the global write position of `pool`.
///
/// Equivalent to `ByteBlockPool.getPosition()` in Lucene 10.5.0. The position
/// is derived from the head block index rather than from the pool's own
/// `byteOffset` field, which this crate stores as an `i32`.
pub(crate) fn pool_position(pool: &ByteBlockPool) -> i64 {
    let buffer_upto = pool.buffer_upto();
    if buffer_upto < 0 {
        0
    } else {
        i64::from(buffer_upto) * BYTE_BLOCK_SIZE as i64 + pool.byte_upto() as i64
    }
}

/// Reads the byte at the given global offset.
///
/// Equivalent to `ByteBlockPool.readByte(long)`.
///
/// # Errors
///
/// Returns [`LuceneError::CorruptIndex`] when the offset addresses a block that
/// was never allocated, which is Lucene's `ArrayIndexOutOfBoundsException`.
pub(crate) fn pool_read_byte(pool: &ByteBlockPool, offset: i64) -> Result<u8> {
    if offset < 0 {
        return Err(LuceneError::CorruptIndex(format!(
            "negative ByteBlockPool offset {offset}"
        )));
    }
    let buffer_index = (offset >> BYTE_BLOCK_SHIFT) as usize;
    if pool.buffer_upto() < 0 || buffer_index > pool.buffer_upto() as usize {
        return Err(LuceneError::CorruptIndex(format!(
            "ByteBlockPool offset {offset} is outside the allocated blocks"
        )));
    }
    Ok(pool.buffer(buffer_index)[(offset as usize) & BYTE_BLOCK_MASK])
}

/// Reads `length` bytes starting at the given global offset.
///
/// Equivalent to `ByteBlockPool.readBytes(long, byte[], int, int)`.
///
/// # Errors
///
/// Propagates the errors of [`pool_read_byte`].
pub(crate) fn pool_read_bytes(
    pool: &ByteBlockPool,
    offset: i64,
    bytes: &mut [u8],
    bytes_offset: usize,
    bytes_length: usize,
) -> Result<()> {
    for i in 0..bytes_length {
        bytes[bytes_offset + i] = pool_read_byte(pool, offset + i as i64)?;
    }
    Ok(())
}

/// Appends `bytes[offset..offset + length]` to the pool, filling every block
/// completely before moving on to the next one.
///
/// Equivalent to `ByteBlockPool.append(byte[], int, int)`.
///
/// # Errors
///
/// Propagates the [`crate::error::LuceneError::ResourceLimit`] this crate's
/// pool raises once its blocks no longer fit the addressable space.
pub(crate) fn pool_append(
    pool: &mut ByteBlockPool,
    bytes: &[u8],
    mut offset: usize,
    length: usize,
) -> Result<()> {
    let mut bytes_left = length;
    while bytes_left > 0 {
        if pool.byte_upto() == BYTE_BLOCK_SIZE {
            pool.next_buffer()?;
        }
        let buffer_left = BYTE_BLOCK_SIZE - pool.byte_upto();
        let n = buffer_left.min(bytes_left);
        let buffer_index = pool.buffer_upto() as usize;
        let start = pool.byte_upto();
        pool.buffer_mut(buffer_index)[start..start + n].copy_from_slice(&bytes[offset..offset + n]);
        pool.advance(n);
        offset += n;
        bytes_left -= n;
    }
    Ok(())
}

/// Appends `length` bytes read from `src` at `src_offset` to `dst`.
///
/// Equivalent to `ByteBlockPool.append(ByteBlockPool, long, int)`.
///
/// # Errors
///
/// Propagates the errors of [`pool_read_bytes`] and [`pool_append`].
pub(crate) fn pool_append_from_pool(
    dst: &mut ByteBlockPool,
    src: &ByteBlockPool,
    src_offset: i64,
    length: usize,
) -> Result<()> {
    let mut scratch = vec![0u8; length];
    pool_read_bytes(src, src_offset, &mut scratch, 0, length)?;
    pool_append(dst, &scratch, 0, length)
}
