//! Port of `org.apache.lucene.util.fst.ReverseRandomAccessReader`.

use crate::error::Result;
use crate::store::{DataInput, RandomAccessInput};

use super::fst::BytesReader;

/// Implements reverse reads from a [`RandomAccessInput`].
///
/// Equivalent to the package-private `ReverseRandomAccessReader`, used by
/// [`super::off_heap_fst_store::OffHeapFSTStore`].
///
/// # Java to Rust adaptations
///
/// * The input is owned rather than shared: this crate's
///   [`RandomAccessInput::read_byte_at`] takes `&mut self`, so the reader keeps
///   the slice it reads from. Lucene obtains the same exclusivity by creating a
///   fresh slice for every reader.
pub struct ReverseRandomAccessReader {
    input: Box<dyn RandomAccessInput>,
    pos: i64,
}

impl ReverseRandomAccessReader {
    /// Creates a reader over `input`, positioned at `0`.
    ///
    /// Equivalent to `new ReverseRandomAccessReader(RandomAccessInput)`.
    pub fn new(input: Box<dyn RandomAccessInput>) -> Self {
        Self { input, pos: 0 }
    }
}

impl std::fmt::Debug for ReverseRandomAccessReader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReverseRandomAccessReader")
            .field("pos", &self.pos)
            .finish()
    }
}

impl DataInput for ReverseRandomAccessReader {
    fn read_byte(&mut self) -> Result<u8> {
        let pos = self.pos;
        self.pos = pos - 1;
        self.input.read_byte_at(pos)
    }

    fn read_bytes(&mut self, b: &mut [u8], offset: usize, len: usize) -> Result<()> {
        for i in 0..len {
            let pos = self.pos;
            self.pos = pos - 1;
            b[offset + i] = self.input.read_byte_at(pos)?;
        }
        Ok(())
    }

    fn skip_bytes(&mut self, num_bytes: i64) -> Result<()> {
        self.pos -= num_bytes;
        Ok(())
    }
}

impl BytesReader for ReverseRandomAccessReader {
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
