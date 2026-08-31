//! Port of `org.apache.lucene.internal.vectorization.MemorySegmentPostingDecodingUtil`.

#![deny(unsafe_code)]

use crate::error::Result;
use crate::internal::vectorization::PostingDecodingUtil;
use crate::store::{IndexInput, MemorySegment};
use crate::util::BitUtil;

/// A [`PostingDecodingUtil`] that decodes straight out of a memory-mapped
/// segment.
///
/// Equivalent to
/// `org.apache.lucene.internal.vectorization.MemorySegmentPostingDecodingUtil`.
///
/// Where the base decoder pulls `count` ints through
/// [`IndexInput::read_ints`](crate::store::DataInput::read_ints) into the
/// caller's buffer and then post-processes them, this one reads each value from
/// the mapped bytes at the input's current file pointer and advances the input
/// to the end of the block in a single seek. That is a real structural
/// difference — no copy through an intermediate buffer — and it is the reason
/// the class exists over and above its use of SIMD.
///
/// # Divergences from Lucene 10.5.0
///
/// * **Composition, not inheritance.** Java extends `PostingDecodingUtil` and
///   overrides `splitInts`. Rust has no implementation inheritance, so this
///   type *contains* the base decoder and shadows the method. The same shape is
///   used elsewhere in this crate, for example
///   [`LockValidatingDirectoryWrapper`](crate::store::LockValidatingDirectoryWrapper)
///   over `FilterDirectory`.
/// * **Scalar element loop.** Java loads `INT_SPECIES.length()` ints per
///   iteration from the segment, shifts and masks them lane-wise, and reads a
///   final vector right-aligned with `count` to cover the tail; when `count` is
///   smaller than one vector it calls `super.splitInts` instead. Stable Rust
///   has no portable SIMD (see [the module docs](super)), so the port reads one
///   int at a time. The values written to `b` and `c` are integers and are
///   therefore identical on both paths — only the instruction count differs.
pub struct MemorySegmentPostingDecodingUtil {
    base: PostingDecodingUtil,
    memory_segment: MemorySegment,
}

impl std::fmt::Debug for MemorySegmentPostingDecodingUtil {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MemorySegmentPostingDecodingUtil")
            .field("in", &self.base)
            .field("memorySegment", &self.memory_segment)
            .finish()
    }
}

impl MemorySegmentPostingDecodingUtil {
    /// Sole constructor.
    ///
    /// Equivalent to the package-private
    /// `MemorySegmentPostingDecodingUtil(IndexInput, MemorySegment)`. The
    /// segment must cover the whole input, as Lucene obtains it with
    /// `segmentSliceOrNull(0, input.length())`.
    pub fn new(input: Box<dyn IndexInput>, memory_segment: MemorySegment) -> Self {
        Self {
            base: PostingDecodingUtil::new(input),
            memory_segment,
        }
    }

    /// Returns the wrapped input.
    ///
    /// Equivalent to reading the inherited `public final IndexInput in` field.
    pub fn input(&self) -> &dyn IndexInput {
        self.base.input()
    }

    /// Returns the wrapped input for reading.
    ///
    /// Equivalent to reading the inherited `public final IndexInput in` field.
    pub fn input_mut(&mut self) -> &mut dyn IndexInput {
        self.base.input_mut()
    }

    /// Returns the segment this decoder reads from.
    ///
    /// Equivalent to reading the private `memorySegment` field, which Java
    /// keeps to itself; it is exposed here because Rust has no package
    /// visibility.
    pub fn memory_segment(&self) -> &MemorySegment {
        &self.memory_segment
    }

    /// Gives the base decoder back to the caller, consuming this one.
    ///
    /// Rust needs an explicit way to recover the wrapped decoder; Java relies
    /// on the object simply being a `PostingDecodingUtil`.
    pub fn into_base(self) -> PostingDecodingUtil {
        self.base
    }

    /// Applies every shift level of one decoded value to `b`.
    ///
    /// Equivalent to the private static
    /// `MemorySegmentPostingDecodingUtil.shift(IntVector, int, int, int, int, int[], int, int)`,
    /// with a single element in place of a vector.
    #[allow(clippy::too_many_arguments)]
    fn shift(
        value: i32,
        b_shift: i32,
        dec: i32,
        max_iter: i32,
        b_mask: i32,
        b: &mut [i32],
        count: usize,
        i: usize,
    ) {
        let mut j = 0i32;
        while j <= max_iter {
            let shift = b_shift - j * dec;
            b[count * (j as usize) + i] =
                ((value as u32).wrapping_shr(shift as u32) as i32) & b_mask;
            j += 1;
        }
    }

    /// Core method for decoding blocks of docs / freqs / positions / offsets.
    ///
    /// Equivalent to `MemorySegmentPostingDecodingUtil.splitInts(...)`, which
    /// overrides [`PostingDecodingUtil::split_ints`]. It reads the same
    /// `count` little-endian ints the base decoder would read, but takes them
    /// from the mapped segment at the input's current file pointer and then
    /// seeks the input past them.
    ///
    /// # Errors
    ///
    /// Returns any error raised while seeking the input to the end of the
    /// block.
    ///
    /// # Panics
    ///
    /// Panics when the block runs past the end of the segment, or when `b` or
    /// `c` are too short, standing in for Java's index-out-of-bounds
    /// exceptions, and when `dec` is zero, standing in for Java's
    /// `ArithmeticException`.
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
        let max_iter = (b_shift - 1) / dec;
        let offset = self.base.input().file_pointer();
        let end_offset = offset + (count as i64) * (std::mem::size_of::<i32>() as i64);

        let data = self.memory_segment.bytes();
        for i in 0..count {
            let at = (offset as usize) + i * std::mem::size_of::<i32>();
            let value = BitUtil::read_le_int(data, at);
            Self::shift(value, b_shift, dec, max_iter, b_mask, b, count, i);
            c[c_index + i] = value & c_mask;
        }

        self.base.input_mut().seek(end_offset)
    }
}
