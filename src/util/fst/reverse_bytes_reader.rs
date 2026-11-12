//! Port of `org.apache.lucene.util.fst.ReverseBytesReader`.

use crate::error::{LuceneError, LuceneErrorKind, Result};
use crate::store::DataInput;

use super::fst::BytesReader;

/// Reads in reverse from a single byte slice.
///
/// Equivalent to the package-private `ReverseBytesReader`. The FST writes every
/// node backwards, so the natural read direction is decreasing addresses.
///
/// # Java to Rust adaptations
///
/// * The position is an `i64` rather than an `int`. Reading the byte at
///   position `0` leaves the position at `-1`, which is legal; reading again
///   reports [`LuceneErrorKind::CorruptIndex`] where Lucene raises an
///   `ArrayIndexOutOfBoundsException`.
pub struct ReverseBytesReader<B> {
    bytes: B,
    pos: i64,
}

impl<B: AsRef<[u8]>> std::fmt::Debug for ReverseBytesReader<B> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReverseBytesReader")
            .field("len", &self.bytes.as_ref().len())
            .field("pos", &self.pos)
            .finish()
    }
}

impl<B: AsRef<[u8]>> ReverseBytesReader<B> {
    /// Creates a reader over `bytes`, positioned at `0`.
    ///
    /// Equivalent to `new ReverseBytesReader(byte[])`. `bytes` is a borrowed
    /// slice, or an owner of the store's array when the reader must outlive
    /// the borrow (see [`FSTReader::get_owned_reverse_bytes_reader`](super::FSTReader::get_owned_reverse_bytes_reader)).
    pub fn new(bytes: B) -> Self {
        Self { bytes, pos: 0 }
    }

    fn out_of_bounds(&self, pos: i64) -> LuceneError {
        LuceneError::from(LuceneErrorKind::corrupt_index_message(format!(
            "FST read position {pos} is outside the {} available bytes",
            self.bytes.as_ref().len()
        )))
    }

    fn next(&mut self) -> Result<u8> {
        let pos = self.pos;
        if pos < 0 || pos as u64 >= self.bytes.as_ref().len() as u64 {
            return Err(self.out_of_bounds(pos));
        }
        self.pos = pos - 1;
        Ok(self.bytes.as_ref()[pos as usize])
    }
}

impl<B: AsRef<[u8]>> DataInput for ReverseBytesReader<B> {
    /// `Object.toString()` of `org.apache.lucene.util.fst.ReverseBytesReader`, which declares no `toString()`.
    fn to_java_string(&self) -> String {
        crate::index::index_reader::java_object_to_string(
            "org.apache.lucene.util.fst.ReverseBytesReader",
            self,
        )
    }

    fn read_byte(&mut self) -> Result<u8> {
        self.next()
    }

    fn read_bytes(&mut self, b: Option<&mut [u8]>, offset: usize, len: usize) -> Result<()> {
        let b = crate::store::require_array(b)?;
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

impl<B: AsRef<[u8]>> BytesReader for ReverseBytesReader<B> {
    fn position(&self) -> i64 {
        self.pos
    }

    fn set_position(&mut self, pos: i64) {
        self.pos = pos;
    }
}
