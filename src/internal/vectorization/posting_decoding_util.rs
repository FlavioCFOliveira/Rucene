//! Port of `org.apache.lucene.internal.vectorization.PostingDecodingUtil`.

#![deny(unsafe_code)]

use crate::error::Result;
use crate::store::IndexInput;

/// Utility class to decode postings.
///
/// Equivalent to `org.apache.lucene.internal.vectorization.PostingDecodingUtil`
/// and its Panama subclass `MemorySegmentPostingDecodingUtil`: the one type the
/// postings reader, [`ForUtil`](crate::codecs::lucene104::ForUtil) and
/// [`PForUtil`](crate::codecs::lucene104::PForUtil) decode through. Instances
/// come from
/// [`VectorizationProvider::new_posting_decoding_util`](super::VectorizationProvider::new_posting_decoding_util),
/// so that the Panama provider can hand out the memory-segment form.
///
/// # Divergences from Lucene 10.5.0, declared
///
/// * **One type for the class and its subclass.** Java's
///   `MemorySegmentPostingDecodingUtil extends PostingDecodingUtil` and
///   overrides `splitInts`. Rust has no implementation inheritance, so the
///   subclass is the state this type carries when
///   [`MemorySegmentPostingDecodingUtil::new`](super::MemorySegmentPostingDecodingUtil::new)
///   built it — the mapped segment — and [`split_ints`](Self::split_ints)
///   dispatches on it exactly as Java's virtual call dispatches on the class.
/// * **The decoder owns its input.** Java's `public final IndexInput in` is a
///   second reference to an input the caller also keeps (the postings
///   reader's `docIn`, `posIn`, `payIn`). Rust cannot hold two mutable
///   references to one input, so the decoder owns it and the caller reaches
///   `docIn` as `docInUtil.in` ([`r#in`](Self::r#in)), the same object Java's
///   two references denote. Java's `final` (the field is never reassigned)
///   cannot be stated on a public Rust field; nothing in the crate assigns it.
pub struct PostingDecodingUtil {
    /// The wrapped [`IndexInput`]: Java's `public final IndexInput in`.
    pub r#in: Box<dyn IndexInput>,
    /// `MemorySegmentPostingDecodingUtil.memorySegment`: the mapped bytes of
    /// the whole input, present when the decoder is the memory-segment form.
    #[cfg(feature = "mmap")]
    pub(crate) memory_segment: Option<crate::store::MemorySegment>,
}

impl std::fmt::Debug for PostingDecodingUtil {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut s = f.debug_struct(self.class_name());
        s.field("in", &self.r#in.resource_description());
        #[cfg(feature = "mmap")]
        if let Some(segment) = &self.memory_segment {
            s.field("memorySegment", segment);
        }
        s.finish()
    }
}

impl PostingDecodingUtil {
    /// Sole constructor.
    ///
    /// Equivalent to the protected `PostingDecodingUtil(IndexInput)`.
    pub fn new(input: Box<dyn IndexInput>) -> Self {
        Self {
            r#in: input,
            #[cfg(feature = "mmap")]
            memory_segment: None,
        }
    }

    /// The simple name of the Java class this decoder stands for:
    /// `PostingDecodingUtil` or `MemorySegmentPostingDecodingUtil`.
    pub fn class_name(&self) -> &'static str {
        #[cfg(feature = "mmap")]
        if self.memory_segment.is_some() {
            return "MemorySegmentPostingDecodingUtil";
        }
        "PostingDecodingUtil"
    }

    /// Core method for decoding blocks of docs / freqs / positions / offsets.
    ///
    /// Equivalent to
    /// `PostingDecodingUtil.splitInts(int, int[], int, int, int, int[], int, int)`,
    /// or to its `MemorySegmentPostingDecodingUtil` override when this is the
    /// memory-segment form:
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
    #[inline]
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
        #[cfg(feature = "mmap")]
        if let Some(segment) = &self.memory_segment {
            return super::memory_segment_posting_decoding_util::split_ints(
                self.r#in.as_mut(),
                segment,
                count,
                b,
                b_shift,
                dec,
                b_mask,
                c,
                c_index,
                c_mask,
            );
        }
        Self::split_ints_base(
            self.r#in.as_mut(),
            count,
            b,
            b_shift,
            dec,
            b_mask,
            c,
            c_index,
            c_mask,
        )
    }

    /// The body of the base class's `splitInts`, which the memory-segment form
    /// calls as `super.splitInts` for short blocks.
    #[allow(clippy::too_many_arguments)]
    #[inline]
    pub(crate) fn split_ints_base(
        input: &mut dyn IndexInput,
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
        input.read_ints(Some(c), c_index, count)?;
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
