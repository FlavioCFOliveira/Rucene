//! Port of `org.apache.lucene.internal.vectorization.MemorySegmentPostingDecodingUtil`.

#![deny(unsafe_code)]

use crate::error::Result;
use crate::internal::vectorization::{PanamaVectorConstants, PostingDecodingUtil};
use crate::store::{IndexInput, MemorySegment};
use crate::util::BitUtil;

/// The Panama subclass of [`PostingDecodingUtil`], which decodes straight out
/// of a memory-mapped segment.
///
/// Equivalent to
/// `org.apache.lucene.internal.vectorization.MemorySegmentPostingDecodingUtil`.
/// Rust has no subclassing, so the subclass is a form of the one
/// [`PostingDecodingUtil`] type (declared there): [`new`](Self::new) is the
/// subclass constructor, and [`PostingDecodingUtil::split_ints`] runs this
/// class's override when the decoder carries a segment.
///
/// The override reads `INT_SPECIES` (the preferred `int` species,
/// [`PanamaVectorConstants::int_lanes`]) lanes of little-endian ints at a time
/// from the segment at the input's file pointer, shifts and masks every lane,
/// handles the tail with one vector aligned on the right end of the block,
/// and advances the input to the end of the block with a single seek.
#[derive(Debug, Clone, Copy)]
pub struct MemorySegmentPostingDecodingUtil;

impl MemorySegmentPostingDecodingUtil {
    /// Creates the memory-segment form of [`PostingDecodingUtil`] over `input`,
    /// whose bytes `memory_segment` maps from offset 0.
    ///
    /// Equivalent to the package-private
    /// `MemorySegmentPostingDecodingUtil(IndexInput, MemorySegment)`.
    #[allow(clippy::new_ret_no_self)]
    pub fn new(input: Box<dyn IndexInput>, memory_segment: MemorySegment) -> PostingDecodingUtil {
        let mut util = PostingDecodingUtil::new(input);
        util.memory_segment = Some(memory_segment);
        util
    }
}

/// `MemorySegmentPostingDecodingUtil.shift(IntVector, int, int, int, int,
/// int[], int, int)`: lane `k` of `vector` lands at `b[count * j + i + k]`.
#[allow(clippy::too_many_arguments)]
#[inline]
fn shift(
    vector: &[i32],
    b_shift: i32,
    dec: i32,
    max_iter: i32,
    b_mask: i32,
    b: &mut [i32],
    count: usize,
    i: usize,
) {
    for j in 0..=max_iter {
        // `lanewise(LSHR, bShift - j * dec)`: an unsigned shift whose
        // distance is reduced modulo the lane width.
        let distance = (b_shift - j * dec) as u32;
        let base = count * (j as usize) + i;
        for (k, &lane) in vector.iter().enumerate() {
            b[base + k] = ((lane as u32).wrapping_shr(distance) as i32) & b_mask;
        }
    }
}

/// `IntVector.fromMemorySegment(INT_SPECIES, memorySegment, offset,
/// LITTLE_ENDIAN)` into `lanes`.
#[inline]
fn load(segment: &[u8], offset: usize, lanes: &mut [i32]) {
    for (k, lane) in lanes.iter_mut().enumerate() {
        *lane = BitUtil::read_le_int(segment, offset + 4 * k);
    }
}

/// `MemorySegmentPostingDecodingUtil.splitInts(int, int[], int, int, int,
/// int[], int, int)` (`MemorySegmentPostingDecodingUtil.java:46-81`).
#[allow(clippy::too_many_arguments)]
pub(crate) fn split_ints(
    input: &mut dyn IndexInput,
    memory_segment: &MemorySegment,
    count: i32,
    b: &mut [i32],
    b_shift: i32,
    dec: i32,
    b_mask: i32,
    c: &mut [i32],
    c_index: i32,
    c_mask: i32,
) -> Result<()> {
    let lanes = PanamaVectorConstants::int_lanes();
    if count < lanes as i32 {
        // Not enough data to vectorize without going out-of-bounds.
        return PostingDecodingUtil::split_ints_base(
            input, count, b, b_shift, dec, b_mask, c, c_index, c_mask,
        );
    }
    let count = count as usize;
    let c_index = c_index as usize;
    let data = memory_segment.bytes();
    let mut vector = [0i32; 64];
    let vector = &mut vector[..lanes];

    let max_iter = (b_shift - 1) / dec;
    let mut offset = input.file_pointer()?;
    let end_offset = offset + (count as i64) * 4;
    // INT_SPECIES.loopBound(count - 1)
    let loop_bound = (count - 1) - (count - 1) % lanes;
    let mut i = 0;
    while i < loop_bound {
        load(data, offset as usize, vector);
        shift(vector, b_shift, dec, max_iter, b_mask, b, count, i);
        for (k, &lane) in vector.iter().enumerate() {
            c[c_index + i + k] = lane & c_mask;
        }
        i += lanes;
        offset += (lanes * 4) as i64;
    }

    // Handle the tail by reading a vector that is aligned with `count` on the
    // right side.
    let i = count - lanes;
    let offset = end_offset - (lanes * 4) as i64;
    load(data, offset as usize, vector);
    shift(vector, b_shift, dec, max_iter, b_mask, b, count, i);
    for (k, &lane) in vector.iter().enumerate() {
        c[c_index + i + k] = lane & c_mask;
    }

    input.seek(end_offset)
}
