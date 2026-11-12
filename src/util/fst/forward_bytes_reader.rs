//! Port of `org.apache.lucene.util.fst.ForwardBytesReader`.

use crate::error::Result;
use crate::store::DataInput;

use super::fst::BytesReader;

/// Reads forwards from a single byte slice.
///
/// Equivalent to the package-private `ForwardBytesReader`. The position is
/// held in an `i64` but follows Java's `int pos`: every update narrows it to
/// an `int`, and a read outside the slice raises the JVM's
/// `ArrayIndexOutOfBoundsException`.
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
}

impl DataInput for ForwardBytesReader<'_> {
    /// `Object.toString()` of `org.apache.lucene.util.fst.ForwardBytesReader`, which declares no `toString()`.
    fn to_java_string(&self) -> String {
        crate::index::index_reader::java_object_to_string(
            "org.apache.lucene.util.fst.ForwardBytesReader",
            self,
        )
    }

    /// `return bytes[pos++]`: the position moves before the array access
    /// fails.
    fn read_byte(&mut self) -> Result<u8> {
        let pos = self.pos as i32;
        self.pos = i64::from(pos.wrapping_add(1));
        if pos < 0 || pos as usize >= self.bytes.len() {
            return Err(crate::store::array_index_out_of_bounds(
                i64::from(pos),
                self.bytes.len(),
            ));
        }
        Ok(self.bytes[pos as usize])
    }

    /// `System.arraycopy(bytes, pos, b, offset, len)`, then `pos += len`.
    fn read_bytes(&mut self, b: Option<&mut [u8]>, offset: usize, len: usize) -> Result<()> {
        let b = crate::store::require_array(b)?;
        let pos = self.pos as i32;
        crate::util::java_byte_arraycopy(self.bytes, pos, b, offset as i32, len as i32)?;
        self.pos = i64::from(pos.wrapping_add(len as i32));
        Ok(())
    }

    /// `pos += count`, narrowed to an `int`.
    fn skip_bytes(&mut self, num_bytes: i64) -> Result<()> {
        self.pos = i64::from(self.pos.wrapping_add(num_bytes) as i32);
        Ok(())
    }
}

impl BytesReader for ForwardBytesReader<'_> {
    fn position(&self) -> i64 {
        self.pos
    }

    /// `this.pos = (int) pos`.
    fn set_position(&mut self, pos: i64) {
        self.pos = i64::from(pos as i32);
    }
}
