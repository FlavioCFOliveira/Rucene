//! Port of `org.apache.lucene.util.fst.ReverseBytesReader`.

use crate::error::{LuceneError, Result};
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
///   reports [`LuceneError::CorruptIndex`] where Lucene raises an
///   `ArrayIndexOutOfBoundsException`.
#[derive(Debug)]
pub struct ReverseBytesReader<'a> {
    bytes: &'a [u8],
    pos: i64,
}

impl<'a> ReverseBytesReader<'a> {
    /// Creates a reader over `bytes`, positioned at `0`.
    ///
    /// Equivalent to `new ReverseBytesReader(byte[])`.
    pub fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    fn out_of_bounds(&self, pos: i64) -> LuceneError {
        LuceneError::CorruptIndex(format!(
            "FST read position {pos} is outside the {} available bytes",
            self.bytes.len()
        ))
    }

    fn next(&mut self) -> Result<u8> {
        let pos = self.pos;
        if pos < 0 || pos as u64 >= self.bytes.len() as u64 {
            return Err(self.out_of_bounds(pos));
        }
        self.pos = pos - 1;
        Ok(self.bytes[pos as usize])
    }
}

impl DataInput for ReverseBytesReader<'_> {
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

impl BytesReader for ReverseBytesReader<'_> {
    fn position(&self) -> i64 {
        self.pos
    }

    fn set_position(&mut self, pos: i64) {
        self.pos = pos;
    }

    fn as_data_input(&mut self) -> &mut dyn DataInput {
        self
    }
}
