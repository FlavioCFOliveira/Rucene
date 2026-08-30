//! Port of `org.apache.lucene.internal.vectorization.PostingDecodingUtil`.

#![deny(unsafe_code)]

use crate::error::Result;
use crate::store::IndexInput;

/// Utility class to decode postings.
///
/// Equivalent to `org.apache.lucene.internal.vectorization.PostingDecodingUtil`.
///
/// Instances are created through
/// [`VectorizationProvider::new_posting_decoding_util`](super::VectorizationProvider::new_posting_decoding_util),
/// never directly by the postings reader, so that a vectorized provider can
/// substitute a faster decoder.
///
/// # Divergences from Lucene 10.5.0
///
/// * **Concrete type, not a base class.** Java declares this class non-final so
///   that `MemorySegmentPostingDecodingUtil` can override
///   [`split_ints`](Self::split_ints). Rust has no implementation inheritance,
///   so this is a plain struct and
///   [`MemorySegmentPostingDecodingUtil`](super::MemorySegmentPostingDecodingUtil)
///   *contains* one and shadows the method; see its own divergence note.
/// * **Owned input.** Java exposes the wrapped input as a `public final
///   IndexInput in` field, which callers read from directly. Rust cannot hand
///   out a shared mutable field, so the input is owned and reached through
///   [`input`](Self::input) / [`input_mut`](Self::input_mut).
pub struct PostingDecodingUtil {
    input: Box<dyn IndexInput>,
}

impl std::fmt::Debug for PostingDecodingUtil {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PostingDecodingUtil")
            .field("in", &self.input.resource_description())
            .finish()
    }
}

impl PostingDecodingUtil {
    /// Sole constructor.
    ///
    /// Equivalent to the protected `PostingDecodingUtil(IndexInput)`.
    pub fn new(input: Box<dyn IndexInput>) -> Self {
        Self { input }
    }

    /// Returns the wrapped input.
    ///
    /// Equivalent to reading the `public final IndexInput in` field.
    pub fn input(&self) -> &dyn IndexInput {
        self.input.as_ref()
    }

    /// Returns the wrapped input for reading.
    ///
    /// Equivalent to reading the `public final IndexInput in` field; Lucene's
    /// callers use it to pull values that are not part of a split block.
    pub fn input_mut(&mut self) -> &mut dyn IndexInput {
        self.input.as_mut()
    }

    /// Gives the wrapped input back to the caller, consuming this decoder.
    ///
    /// Java relies on the garbage collector and on the caller keeping its own
    /// reference to the input; Rust needs an explicit way to recover ownership.
    pub fn into_input(self) -> Box<dyn IndexInput> {
        self.input
    }

    /// Core method for decoding blocks of docs / freqs / positions / offsets.
    ///
    /// Equivalent to
    /// `PostingDecodingUtil.splitInts(int, int[], int, int, int, int[], int, int)`:
    ///
    /// * Read `count` ints.
    /// * For all `i >= 0` such that `b_shift - i * dec > 0`, apply shift
    ///   `b_shift - i * dec` and store the result in `b` at offset `count * i`.
    /// * Apply mask `c_mask` and store the result in `c` starting at offset
    ///   `c_index`.
    ///
    /// The shift is an unsigned (`>>>`) shift whose distance Java reduces
    /// modulo 32; [`u32::wrapping_shr`] reproduces that exactly.
    ///
    /// # Errors
    ///
    /// Returns any error raised while reading `count` ints from the input.
    ///
    /// # Panics
    ///
    /// Panics when `b` or `c` are too short for the requested offsets, standing
    /// in for Java's `ArrayIndexOutOfBoundsException`, and when `dec` is zero,
    /// standing in for Java's `ArithmeticException`.
    #[allow(clippy::too_many_arguments)]
    pub fn split_ints(
        &mut self,
        count: usize,
        b: &mut [i32],
        b_shift: i32,
        dec: i32,
        b_mask: i32,
        c: &mut [i32],
        c_index: usize,
        c_mask: i32,
    ) -> Result<()> {
        self.input.read_ints(c, c_index, count)?;
        let max_iter = (b_shift - 1) / dec;

        // Process each shift level across all elements (better for vectorization)
        let mut j = 0i32;
        while j <= max_iter {
            let shift = b_shift - j * dec;
            let b_offset = count * (j as usize);
            // Vectorizable loop: contiguous memory access with simple operations
            for i in 0..count {
                b[b_offset + i] =
                    ((c[c_index + i] as u32).wrapping_shr(shift as u32) as i32) & b_mask;
            }
            j += 1;
        }

        // Apply mask to c array (vectorizable)
        for i in 0..count {
            c[c_index + i] &= c_mask;
        }
        Ok(())
    }
}
