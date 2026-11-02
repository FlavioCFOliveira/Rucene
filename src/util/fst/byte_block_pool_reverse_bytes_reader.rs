//! Port of `org.apache.lucene.util.fst.ByteBlockPoolReverseBytesReader`.

use crate::error::Result;
use crate::store::DataInput;
use crate::util::byte_block_pool::ByteBlockPool;

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
/// Every byte is read with [`ByteBlockPool::read_byte`], Java's
/// `ByteBlockPool.readByte(long)` (`ByteBlockPoolReverseBytesReader.java:35-44`).
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

    /// `buf.readByte(pos--)`; a position outside the pool panics where Java
    /// throws `ArrayIndexOutOfBoundsException`.
    fn next(&mut self) -> u8 {
        let pos = self.pos;
        self.pos = pos - 1;
        self.buf.read_byte(pos)
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
    /// `Object.toString()` of `org.apache.lucene.util.fst.ByteBlockPoolReverseBytesReader`, which declares no `toString()`.
    fn to_java_string(&self) -> String {
        crate::index::index_reader::java_object_to_string(
            "org.apache.lucene.util.fst.ByteBlockPoolReverseBytesReader",
            self,
        )
    }

    fn read_byte(&mut self) -> Result<u8> {
        Ok(self.next())
    }

    fn read_bytes(&mut self, b: &mut [u8], offset: usize, len: usize) -> Result<()> {
        for i in 0..len {
            b[offset + i] = self.next();
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
