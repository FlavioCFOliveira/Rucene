//! Port of `org.apache.lucene.util.fst.ForwardBytesReader`.

use crate::error::{LuceneError, Result};
use crate::store::DataInput;

use super::fst::BytesReader;

/// Reads forwards from a single byte slice.
///
/// Equivalent to the package-private `ForwardBytesReader`.
///
/// # Java to Rust adaptations
///
/// * The position is an `i64` rather than an `int`, so that a position outside
///   the slice is reported as [`LuceneError::CorruptIndex`] instead of the
///   `ArrayIndexOutOfBoundsException` Lucene raises.
#[derive(Debug)]
pub struct ForwardBytesReader<'a> {
    bytes: &'a [u8],
    pos: i64,
}

impl<'a> ForwardBytesReader<'a> {
    /// Creates a reader over `bytes`, positioned at `0`.
    ///
    /// Equivalent to `new ForwardBytesReader(byte[])`.
    pub fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    fn out_of_bounds(&self, pos: i64) -> LuceneError {
        LuceneError::CorruptIndex(format!(
            "FST read position {pos} is outside the {} available bytes",
            self.bytes.len()
        ))
    }
}

impl DataInput for ForwardBytesReader<'_> {
    fn read_byte(&mut self) -> Result<u8> {
        let pos = self.pos;
        if pos < 0 || pos as u64 >= self.bytes.len() as u64 {
            return Err(self.out_of_bounds(pos));
        }
        self.pos = pos + 1;
        Ok(self.bytes[pos as usize])
    }

    fn read_bytes(&mut self, b: &mut [u8], offset: usize, len: usize) -> Result<()> {
        let pos = self.pos;
        let end = pos
            .checked_add(len as i64)
            .ok_or_else(|| self.out_of_bounds(pos))?;
        if pos < 0 || end as u64 > self.bytes.len() as u64 {
            return Err(self.out_of_bounds(pos));
        }
        b[offset..offset + len].copy_from_slice(&self.bytes[pos as usize..end as usize]);
        self.pos = end;
        Ok(())
    }

    fn skip_bytes(&mut self, num_bytes: i64) -> Result<()> {
        self.pos += num_bytes;
        Ok(())
    }
}

impl BytesReader for ForwardBytesReader<'_> {
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
