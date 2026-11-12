//! Port of `org.apache.lucene.util.fst.ReverseBytesReader`.

use crate::error::Result;
use crate::store::DataInput;

use super::fst::BytesReader;

/// Reads in reverse from a single byte slice.
///
/// Equivalent to the package-private `ReverseBytesReader`. The FST writes every
/// node backwards, so the natural read direction is decreasing addresses.
///
/// The position is held in an `i64` but follows Java's `int pos`: every
/// update narrows it to an `int`. Reading the byte at position `0` leaves the
/// position at `-1`, which is legal; reading again raises the JVM's
/// `ArrayIndexOutOfBoundsException`, as Lucene does.
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

    /// `bytes[pos--]`: the position moves before the array access fails.
    fn next(&mut self) -> Result<u8> {
        let pos = self.pos as i32;
        self.pos = i64::from(pos.wrapping_sub(1));
        let bytes = self.bytes.as_ref();
        if pos < 0 || pos as usize >= bytes.len() {
            return Err(crate::store::array_index_out_of_bounds(
                i64::from(pos),
                bytes.len(),
            ));
        }
        Ok(bytes[pos as usize])
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
            // `b[offset + i] = bytes[pos--]`: the byte is read before the
            // store is checked (JLS 15.26.1).
            let value = self.next()?;
            let Some(slot) = b.get_mut(offset + i) else {
                return Err(crate::store::array_index_out_of_bounds(
                    (offset + i) as i64,
                    b.len(),
                ));
            };
            *slot = value;
        }
        Ok(())
    }

    /// `pos -= count`, narrowed to an `int`.
    fn skip_bytes(&mut self, num_bytes: i64) -> Result<()> {
        self.pos = i64::from(self.pos.wrapping_sub(num_bytes) as i32);
        Ok(())
    }
}

impl<B: AsRef<[u8]>> BytesReader for ReverseBytesReader<B> {
    fn position(&self) -> i64 {
        self.pos
    }

    /// `this.pos = (int) pos`.
    fn set_position(&mut self, pos: i64) {
        self.pos = i64::from(pos as i32);
    }
}
