//! Scalar decoding helper used by [`ForUtil`].
//!
//! This is a Rust port of `org.apache.lucene.internal.vectorization.PostingDecodingUtil`
//! (default scalar implementation). It wraps an [`IndexInput`] and provides the
//! `split_ints` primitive used by the frame-of-reference decoders.

use crate::error::Result;
use crate::store::IndexInput;

/// Wrapper around an [`IndexInput`] that provides bulk integer splitting.
pub struct PostingDecodingUtil<'a> {
    /// The wrapped input stream.
    pub input: &'a mut dyn IndexInput,
}

impl<'a> PostingDecodingUtil<'a> {
    /// Creates a new utility wrapping the given input.
    pub fn new(input: &'a mut dyn IndexInput) -> Self {
        Self { input }
    }

    /// Reads `length` little-endian `i32` values into `dst[offset..]`.
    pub fn read_ints(&mut self, dst: &mut [i32], offset: i32, length: i32) -> Result<()> {
        self.input.read_ints(dst, offset as usize, length as usize)
    }

    /// Core bulk split primitive.
    ///
    /// Reads `count` ints into `c[c_index..]`, then for each shift level
    /// `b_shift - j * dec` writes the masked, shifted value to
    /// `b[count * j..]`. Finally applies `c_mask` to the `c` array.
    pub fn split_ints(
        &mut self,
        count: i32,
        b: &mut [i32],
        b_shift: i32,
        dec: i32,
        b_mask: i32,
        c: &mut [i32],
        c_index: i32,
        c_mask: i32,
    ) -> Result<()> {
        let count = count as usize;
        let c_index = c_index as usize;
        self.input.read_ints(c, c_index, count)?;

        let max_iter = ((b_shift - 1) / dec) as usize;
        for j in 0..=max_iter {
            let shift = b_shift - j as i32 * dec;
            let b_offset = count * j;
            for i in 0..count {
                let v = c[c_index + i] as u32;
                b[b_offset + i] = ((v >> shift) as i32) & b_mask;
            }
        }

        if c_mask != -1 {
            for i in 0..count {
                c[c_index + i] &= c_mask;
            }
        }
        Ok(())
    }
}
