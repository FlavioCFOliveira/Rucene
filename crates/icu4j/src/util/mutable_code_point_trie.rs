// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.util.MutableCodePointTrie`.

use rucene::error::{LuceneError, Result};

use crate::util::code_point_map::{
    CodePointMap, Range, RangeIterator, StringIterator, StringIteratorKind, ValueFilter,
};
use crate::util::code_point_trie::{
    CodePointTrie, Data, Type, ValueWidth, CP_PER_INDEX_2_ENTRY, FAST_DATA_BLOCK_LENGTH,
    FAST_SHIFT, INDEX_2_BLOCK_LENGTH, INDEX_2_MASK, INDEX_3_BLOCK_LENGTH, NO_DATA_NULL_OFFSET,
    NO_INDEX3_NULL_OFFSET, SHIFT_1_2, SHIFT_2_3, SHIFT_3, SMALL_DATA_BLOCK_LENGTH, SMALL_DATA_MASK,
    SMALL_LIMIT,
};

/// `MAX_UNICODE`.
const MAX_UNICODE: i32 = 0x10ffff;
/// `UNICODE_LIMIT`.
const UNICODE_LIMIT: i32 = 0x110000;
/// `BMP_LIMIT`.
const BMP_LIMIT: i32 = 0x10000;
/// `ASCII_LIMIT`.
const ASCII_LIMIT: i32 = 0x80;
/// `I_LIMIT`.
const I_LIMIT: i32 = UNICODE_LIMIT >> SHIFT_3;
/// `BMP_I_LIMIT`.
const BMP_I_LIMIT: i32 = BMP_LIMIT >> SHIFT_3;
/// `ASCII_I_LIMIT`.
const ASCII_I_LIMIT: i32 = ASCII_LIMIT >> SHIFT_3;
/// `SMALL_DATA_BLOCKS_PER_BMP_BLOCK`.
const SMALL_DATA_BLOCKS_PER_BMP_BLOCK: i32 = 1 << (FAST_SHIFT - SHIFT_3);
/// `ALL_SAME`: flag of an index entry holding the value of the whole block.
const ALL_SAME: i8 = 0;
/// `MIXED`: flag of an index entry holding a data block offset.
const MIXED: i8 = 1;
/// `SAME_AS`: flag of an index entry holding the index of an equal block.
const SAME_AS: i8 = 2;
/// `INITIAL_DATA_LENGTH`: start with allocation of 16k data entries.
const INITIAL_DATA_LENGTH: i32 = 1 << 14;
/// `MEDIUM_DATA_LENGTH`: grow about 8x each time.
const MEDIUM_DATA_LENGTH: i32 = 1 << 17;
/// `MAX_DATA_LENGTH`.
const MAX_DATA_LENGTH: i32 = UNICODE_LIMIT;
/// `I3_NULL`.
const I3_NULL: i8 = 0;
/// `I3_BMP`.
const I3_BMP: i8 = 1;
/// `I3_16`.
const I3_16: i8 = 2;
/// `I3_18`.
const I3_18: i8 = 3;
/// `INDEX_3_18BIT_BLOCK_LENGTH`.
const INDEX_3_18BIT_BLOCK_LENGTH: i32 = INDEX_3_BLOCK_LENGTH + INDEX_3_BLOCK_LENGTH / 8;

/// Mutable Unicode code point trie: builds a [`CodePointTrie`] by setting
/// values for code points and ranges.
///
/// Equivalent to `com.ibm.icu.util.MutableCodePointTrie`, which extends
/// `CodePointMap` and implements `Cloneable` ([`Clone`], with Java's
/// `clone()` logic).
#[derive(Debug)]
pub struct MutableCodePointTrie {
    index: Vec<i32>,
    index3_null_offset: i32,
    data: Vec<i32>,
    data_length: i32,
    data_null_offset: i32,
    orig_initial_value: i32,
    initial_value: i32,
    error_value: i32,
    high_start: i32,
    high_value: i32,
    /// Temporary array while building the final data.
    index16: Option<Vec<u16>>,
    flags: Vec<i8>,
}

impl Clone for MutableCodePointTrie {
    /// `clone()`: a copy with a fresh index of the current capacity class
    /// and the flags of the used part.
    fn clone(&self) -> Self {
        let i_capacity = if self.high_start <= BMP_LIMIT {
            BMP_I_LIMIT
        } else {
            I_LIMIT
        };
        let mut index = vec![0i32; i_capacity as usize];
        let mut flags = vec![0i8; (UNICODE_LIMIT >> SHIFT_3) as usize];
        let i_limit = (self.high_start >> SHIFT_3) as usize;
        index[..i_limit].copy_from_slice(&self.index[..i_limit]);
        flags[..i_limit].copy_from_slice(&self.flags[..i_limit]);
        debug_assert!(self.index16.is_none());
        Self {
            index,
            index3_null_offset: self.index3_null_offset,
            data: self.data.clone(),
            data_length: self.data_length,
            data_null_offset: self.data_null_offset,
            orig_initial_value: self.orig_initial_value,
            initial_value: self.initial_value,
            error_value: self.error_value,
            high_start: self.high_start,
            high_value: self.high_value,
            index16: None,
            flags,
        }
    }
}

impl MutableCodePointTrie {
    /// `MutableCodePointTrie(int initialValue, int errorValue)`: every code
    /// point has `initial_value`; `error_value` is returned for
    /// out-of-range code points.
    pub fn new(initial_value: i32, error_value: i32) -> Self {
        Self {
            index: vec![0; BMP_I_LIMIT as usize],
            index3_null_offset: -1,
            data: vec![0; INITIAL_DATA_LENGTH as usize],
            data_length: 0,
            data_null_offset: -1,
            orig_initial_value: initial_value,
            initial_value,
            error_value,
            high_start: 0,
            high_value: initial_value,
            index16: None,
            flags: vec![0; (UNICODE_LIMIT >> SHIFT_3) as usize],
        }
    }

    /// `fromCodePointMap(CodePointMap map)`: a mutable trie with the values
    /// of `map`.
    pub fn from_code_point_map(map: &dyn CodePointMap) -> Self {
        // TODO: Consider special code branch for map instanceof CodePointTrie?
        // Use the highValue as the initialValue to reduce the highStart.
        let error_value = map.get(-1);
        let initial_value = map.get(MAX_UNICODE);
        let mut mutable_trie = MutableCodePointTrie::new(initial_value, error_value);
        let mut range = Range::new();
        let mut start = 0;
        while map.get_range(start, None, &mut range) {
            let end = range.get_end();
            let value = range.get_value();
            if value != initial_value {
                if start == end {
                    mutable_trie
                        .set(start, value)
                        .expect("a range start is a code point");
                } else {
                    mutable_trie
                        .set_range(start, end, value)
                        .expect("a map range is a valid code point range");
                }
            }
            start = end + 1;
        }
        mutable_trie
    }

    /// `clear()` (private).
    fn clear(&mut self) {
        self.index3_null_offset = -1;
        self.data_null_offset = -1;
        self.data_length = 0;
        self.initial_value = self.orig_initial_value;
        self.high_value = self.orig_initial_value;
        self.high_start = 0;
        self.index16 = None;
    }

    /// `maybeFilterValue(int value, int initialValue, int nullValue,
    /// ValueFilter filter)` (private).
    fn maybe_filter_value(
        mut value: i32,
        initial_value: i32,
        null_value: i32,
        filter: Option<ValueFilter<'_>>,
    ) -> i32 {
        if value == initial_value {
            value = null_value;
        } else if let Some(f) = filter {
            value = f(value);
        }
        value
    }

    /// `writeBlock(int block, int value)` (private).
    fn write_block(&mut self, block: i32, value: i32) {
        let limit = block + SMALL_DATA_BLOCK_LENGTH;
        self.data[block as usize..limit as usize].fill(value);
    }

    /// `set(int c, int value)`: sets the value for a code point.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("invalid code point")`.
    pub fn set(&mut self, c: i32, value: i32) -> Result<()> {
        if !(0..=MAX_UNICODE).contains(&c) {
            return Err(LuceneError::illegal_argument("invalid code point"));
        }

        self.ensure_high_start(c);
        let block = self.get_data_block(c >> SHIFT_3);
        self.data[(block + (c & SMALL_DATA_MASK)) as usize] = value;
        Ok(())
    }

    /// `fillBlock(int block, int start, int limit, int value)` (private).
    fn fill_block(&mut self, block: i32, start: i32, limit: i32, value: i32) {
        self.data[(block + start) as usize..(block + limit) as usize].fill(value);
    }

    /// `setRange(int start, int end, int value)`: sets the value for the
    /// code points `start..=end`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("invalid code point range")`.
    pub fn set_range(&mut self, mut start: i32, end: i32, value: i32) -> Result<()> {
        if !(0..=MAX_UNICODE).contains(&start) || !(0..=MAX_UNICODE).contains(&end) || start > end {
            return Err(LuceneError::illegal_argument("invalid code point range"));
        }
        self.ensure_high_start(end);

        let mut limit = end + 1;
        if (start & SMALL_DATA_MASK) != 0 {
            // Set partial block at [start..following block boundary[.
            let block = self.get_data_block(start >> SHIFT_3);
            let next_start = (start + SMALL_DATA_MASK) & !SMALL_DATA_MASK;
            if next_start <= limit {
                self.fill_block(
                    block,
                    start & SMALL_DATA_MASK,
                    SMALL_DATA_BLOCK_LENGTH,
                    value,
                );
                start = next_start;
            } else {
                self.fill_block(
                    block,
                    start & SMALL_DATA_MASK,
                    limit & SMALL_DATA_MASK,
                    value,
                );
                return Ok(());
            }
        }

        // Number of positions in the last, partial block.
        let rest = limit & SMALL_DATA_MASK;

        // Round down limit to a block boundary.
        limit &= !SMALL_DATA_MASK;

        // Iterate over all-value blocks.
        while start < limit {
            let i = start >> SHIFT_3;
            if self.flags[i as usize] == ALL_SAME {
                self.index[i as usize] = value;
            } else {
                let b = self.index[i as usize];
                self.fill_block(b, 0, SMALL_DATA_BLOCK_LENGTH, value);
            }
            start += SMALL_DATA_BLOCK_LENGTH;
        }

        if rest > 0 {
            // Set partial block at [last block boundary..limit[.
            let block = self.get_data_block(start >> SHIFT_3);
            self.fill_block(block, 0, rest, value);
        }
        Ok(())
    }

    /// `buildImmutable(CodePointTrie.Type type, CodePointTrie.ValueWidth
    /// valueWidth)`: compacts the data and builds an immutable trie; this
    /// object is then cleared (back to its initial value), also when the
    /// build fails.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("The type and valueWidth must be
    /// specified.")` for a `None` argument.
    ///
    /// # Panics
    ///
    /// `IndexOutOfBoundsException("The trie data exceeds limitations of the
    /// data structure.")`.
    pub fn build_immutable(
        &mut self,
        type_: Option<Type>,
        value_width: Option<ValueWidth>,
    ) -> Result<CodePointTrie> {
        let (Some(type_), Some(value_width)) = (type_, value_width) else {
            return Err(LuceneError::illegal_argument(
                "The type and valueWidth must be specified.",
            ));
        };

        // try { return build(type, valueWidth); } finally { clear(); }
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.build(type_, value_width)
        }));
        self.clear();
        match result {
            Ok(trie) => Ok(trie),
            Err(payload) => std::panic::resume_unwind(payload),
        }
    }

    /// `ensureHighStart(int c)` (private).
    fn ensure_high_start(&mut self, mut c: i32) {
        if c >= self.high_start {
            // Round up to a CodePointTrie.CP_PER_INDEX_2_ENTRY boundary to simplify compaction.
            c = (c + CP_PER_INDEX_2_ENTRY) & !(CP_PER_INDEX_2_ENTRY - 1);
            let mut i = self.high_start >> SHIFT_3;
            let i_limit = c >> SHIFT_3;
            if i_limit > self.index.len() as i32 {
                let mut new_index = vec![0i32; I_LIMIT as usize];
                new_index[..i as usize].copy_from_slice(&self.index[..i as usize]);
                self.index = new_index;
            }
            loop {
                self.flags[i as usize] = ALL_SAME;
                self.index[i as usize] = self.initial_value;
                i += 1;
                if i >= i_limit {
                    break;
                }
            }
            self.high_start = c;
        }
    }

    /// `allocDataBlock(int blockLength)` (private).
    fn alloc_data_block(&mut self, block_length: i32) -> i32 {
        let new_block = self.data_length;
        let new_top = new_block + block_length;
        if new_top > self.data.len() as i32 {
            let capacity = if (self.data.len() as i32) < MEDIUM_DATA_LENGTH {
                MEDIUM_DATA_LENGTH
            } else if (self.data.len() as i32) < MAX_DATA_LENGTH {
                MAX_DATA_LENGTH
            } else {
                // Should never occur.
                // Either MAX_DATA_LENGTH is incorrect,
                // or the code writes more values than should be possible.
                panic!("java.lang.AssertionError");
            };
            let mut new_data = vec![0i32; capacity as usize];
            new_data[..self.data_length as usize]
                .copy_from_slice(&self.data[..self.data_length as usize]);
            self.data = new_data;
        }
        self.data_length = new_top;
        new_block
    }

    /// `getDataBlock(int i)` (private): the data block of index entry `i`,
    /// allocating one (a whole fast block in the BMP) for an all-same entry.
    fn get_data_block(&mut self, i: i32) -> i32 {
        if self.flags[i as usize] == MIXED {
            return self.index[i as usize];
        }
        if i < BMP_I_LIMIT {
            let mut new_block = self.alloc_data_block(FAST_DATA_BLOCK_LENGTH);
            let mut i_start = i & !(SMALL_DATA_BLOCKS_PER_BMP_BLOCK - 1);
            let i_limit = i_start + SMALL_DATA_BLOCKS_PER_BMP_BLOCK;
            loop {
                debug_assert!(self.flags[i_start as usize] == ALL_SAME);
                let v = self.index[i_start as usize];
                self.write_block(new_block, v);
                self.flags[i_start as usize] = MIXED;
                self.index[i_start as usize] = new_block;
                i_start += 1;
                new_block += SMALL_DATA_BLOCK_LENGTH;
                if i_start >= i_limit {
                    break;
                }
            }
            self.index[i as usize]
        } else {
            let new_block = self.alloc_data_block(SMALL_DATA_BLOCK_LENGTH);
            if new_block < 0 {
                return new_block;
            }
            let v = self.index[i as usize];
            self.write_block(new_block, v);
            self.flags[i as usize] = MIXED;
            self.index[i as usize] = new_block;
            new_block
        }
    }

    /// `maskValues(int mask)` (private).
    fn mask_values(&mut self, mask: i32) {
        self.initial_value &= mask;
        self.error_value &= mask;
        self.high_value &= mask;
        let i_limit = self.high_start >> SHIFT_3;
        for i in 0..i_limit as usize {
            if self.flags[i] == ALL_SAME {
                self.index[i] &= mask;
            }
        }
        for v in &mut self.data[..self.data_length as usize] {
            *v &= mask;
        }
    }

    /// `findHighStart()` (private): the start of the last range of the high
    /// value, rounded to a small block.
    fn find_high_start(&self) -> i32 {
        let mut i = self.high_start >> SHIFT_3;
        while i > 0 {
            i -= 1;
            let matched;
            if self.flags[i as usize] == ALL_SAME {
                matched = self.index[i as usize] == self.high_value;
            } else {
                let p = self.index[i as usize];
                let mut j = 0;
                loop {
                    if j == SMALL_DATA_BLOCK_LENGTH {
                        matched = true;
                        break;
                    }
                    if self.data[(p + j) as usize] != self.high_value {
                        matched = false;
                        break;
                    }
                    j += 1;
                }
            }
            if !matched {
                return (i + 1) << SHIFT_3;
            }
        }
        0
    }

    /// `compactWholeDataBlocks(int fastILimit, AllSameBlocks allSameBlocks)`
    /// (private).
    fn compact_whole_data_blocks(
        &mut self,
        fast_i_limit: i32,
        all_same_blocks: &mut AllSameBlocks,
    ) -> i32 {
        // ASCII data will be stored as a linear table, even if the following code
        // does not yet count it that way.
        let mut new_data_capacity = ASCII_LIMIT;
        // Add room for a small data null block in case it would match the start of
        // a fast data block where dataNullOffset must not be set in that case.
        new_data_capacity += SMALL_DATA_BLOCK_LENGTH;
        // Add room for special values (errorValue, highValue) and padding.
        new_data_capacity += 4;
        let i_limit = self.high_start >> SHIFT_3;
        let mut block_length = FAST_DATA_BLOCK_LENGTH;
        let mut inc = SMALL_DATA_BLOCKS_PER_BMP_BLOCK;
        let mut i = 0;
        while i < i_limit {
            'iteration: {
                if i == fast_i_limit {
                    block_length = SMALL_DATA_BLOCK_LENGTH;
                    inc = 1;
                }
                let mut value = self.index[i as usize];
                if self.flags[i as usize] == MIXED {
                    // Really mixed?
                    let p = value;
                    value = self.data[p as usize];
                    if all_values_same_as(&self.data, p + 1, block_length - 1, value) {
                        self.flags[i as usize] = ALL_SAME;
                        self.index[i as usize] = value;
                        // Fall through to ALL_SAME handling.
                    } else {
                        new_data_capacity += block_length;
                        break 'iteration;
                    }
                } else {
                    debug_assert!(self.flags[i as usize] == ALL_SAME);
                    if inc > 1 {
                        // Do all of the fast-range data block's ALL_SAME parts have the same value?
                        let mut all_same = true;
                        let next_i = i + inc;
                        for j in (i + 1)..next_i {
                            debug_assert!(self.flags[j as usize] == ALL_SAME);
                            if self.index[j as usize] != value {
                                all_same = false;
                                break;
                            }
                        }
                        if !all_same {
                            // Turn it into a MIXED block.
                            if self.get_data_block(i) < 0 {
                                return -1;
                            }
                            new_data_capacity += block_length;
                            break 'iteration;
                        }
                    }
                }
                // Is there another ALL_SAME block with the same value?
                let mut other = all_same_blocks.find_or_add(i, inc, value);
                if other == AllSameBlocks::OVERFLOW {
                    // The fixed-size array overflowed. Slow check for a duplicate block.
                    let mut j_inc = SMALL_DATA_BLOCKS_PER_BMP_BLOCK;
                    let mut j = 0;
                    loop {
                        if j == i {
                            all_same_blocks.add(i, inc, value);
                            break;
                        }
                        if j == fast_i_limit {
                            j_inc = 1;
                        }
                        if self.flags[j as usize] == ALL_SAME && self.index[j as usize] == value {
                            all_same_blocks.add(j, j_inc + inc, value);
                            other = j;
                            break;
                            // We could keep counting blocks with the same value
                            // before we add the first one, which may improve compaction in rare cases,
                            // but it would make it slower.
                        }
                        j += j_inc;
                    }
                }
                if other >= 0 {
                    self.flags[i as usize] = SAME_AS;
                    self.index[i as usize] = other;
                } else {
                    // New unique same-value block.
                    new_data_capacity += block_length;
                }
            }
            i += inc;
        }
        new_data_capacity
    }

    /// `compactData(int fastILimit, int[] newData, int dataNullIndex,
    /// MixedBlocks mixedBlocks)` (private).
    fn compact_data(
        &mut self,
        fast_i_limit: i32,
        new_data: &mut [i32],
        data_null_index: i32,
        mixed_blocks: &mut MixedBlocks,
    ) -> i32 {
        // The linear ASCII data has been copied into newData already.
        let mut new_data_length = 0;
        let mut i = 0;
        while new_data_length < ASCII_LIMIT {
            self.index[i as usize] = new_data_length;
            new_data_length += FAST_DATA_BLOCK_LENGTH;
            i += SMALL_DATA_BLOCKS_PER_BMP_BLOCK;
        }

        let mut block_length = FAST_DATA_BLOCK_LENGTH;
        mixed_blocks.init(new_data.len() as i32, block_length);
        mixed_blocks.extend(BlockData::Ints(new_data), 0, 0, new_data_length);

        let i_limit = self.high_start >> SHIFT_3;
        let mut inc = SMALL_DATA_BLOCKS_PER_BMP_BLOCK;
        let mut fast_length = 0;
        let mut i = ASCII_I_LIMIT;
        while i < i_limit {
            if i == fast_i_limit {
                block_length = SMALL_DATA_BLOCK_LENGTH;
                inc = 1;
                fast_length = new_data_length;
                mixed_blocks.init(new_data.len() as i32, block_length);
                mixed_blocks.extend(BlockData::Ints(new_data), 0, 0, new_data_length);
            }
            if self.flags[i as usize] == ALL_SAME {
                let value = self.index[i as usize];
                // Find an earlier part of the data array of length blockLength
                // that is filled with this value.
                let mut n = mixed_blocks.find_all_same_block(new_data, value);
                // If we find a match, and the current block is the data null block,
                // and it is not a fast block but matches the start of a fast block,
                // then we need to continue looking.
                // This is because this small block is shorter than the fast block,
                // and not all of the rest of the fast block is filled with this value.
                // Otherwise trie.getRange() would detect that the fast block starts at
                // dataNullOffset and assume incorrectly that it is filled with the null value.
                while n >= 0
                    && i == data_null_index
                    && i >= fast_i_limit
                    && n < fast_length
                    && is_start_of_some_fast_block(n, &self.index, fast_i_limit)
                {
                    n = find_all_same_block(new_data, n + 1, new_data_length, value, block_length);
                }
                if n >= 0 {
                    self.index[i as usize] = n;
                } else {
                    n = get_all_same_overlap(new_data, new_data_length, value, block_length);
                    self.index[i as usize] = new_data_length - n;
                    let prev_data_length = new_data_length;
                    while n < block_length {
                        new_data[new_data_length as usize] = value;
                        new_data_length += 1;
                        n += 1;
                    }
                    mixed_blocks.extend(
                        BlockData::Ints(new_data),
                        0,
                        prev_data_length,
                        new_data_length,
                    );
                }
            } else if self.flags[i as usize] == MIXED {
                // Find an earlier part of the data array of length blockLength
                // that is filled with this value.
                let block = self.index[i as usize];
                let mut n = mixed_blocks.find_block(
                    BlockData::Ints(new_data),
                    BlockData::Ints(&self.data),
                    block,
                );
                if n >= 0 {
                    self.index[i as usize] = n;
                } else {
                    n = get_overlap(
                        BlockData::Ints(new_data),
                        new_data_length,
                        BlockData::Ints(&self.data),
                        block,
                        block_length,
                    );
                    self.index[i as usize] = new_data_length - n;
                    let prev_data_length = new_data_length;
                    while n < block_length {
                        new_data[new_data_length as usize] = self.data[(block + n) as usize];
                        new_data_length += 1;
                        n += 1;
                    }
                    mixed_blocks.extend(
                        BlockData::Ints(new_data),
                        0,
                        prev_data_length,
                        new_data_length,
                    );
                }
            } else {
                // SAME_AS
                let j = self.index[i as usize];
                self.index[i as usize] = self.index[j as usize];
            }
            i += inc;
        }

        new_data_length
    }

    /// `compactIndex(int fastILimit, MixedBlocks mixedBlocks)` (private).
    ///
    /// # Panics
    ///
    /// `IndexOutOfBoundsException("The trie data exceeds limitations of the
    /// data structure.")`.
    fn compact_index(&mut self, fast_i_limit: i32, mixed_blocks: &mut MixedBlocks) -> i32 {
        let fast_index_length = fast_i_limit >> (FAST_SHIFT - SHIFT_3);
        if (self.high_start >> FAST_SHIFT) <= fast_index_length {
            // Only the linear fast index, no multi-stage index tables.
            self.index3_null_offset = NO_INDEX3_NULL_OFFSET;
            return fast_index_length;
        }

        // Condense the fast index table.
        // Also, does it contain an index-3 block with all dataNullOffset?
        let mut fast_index = vec![0u16; fast_index_length as usize];
        let mut i3_first_null = -1;
        let mut i = 0;
        let mut j = 0;
        while i < fast_i_limit {
            let mut i3 = self.index[i as usize];
            fast_index[j as usize] = i3 as u16;
            if i3 == self.data_null_offset {
                if i3_first_null < 0 {
                    i3_first_null = j;
                } else if self.index3_null_offset < 0
                    && (j - i3_first_null + 1) == INDEX_3_BLOCK_LENGTH
                {
                    self.index3_null_offset = i3_first_null;
                }
            } else {
                i3_first_null = -1;
            }
            // Set the index entries that compactData() skipped.
            // Needed when the multi-stage index covers the fast index range as well.
            let i_next = i + SMALL_DATA_BLOCKS_PER_BMP_BLOCK;
            loop {
                i += 1;
                if i >= i_next {
                    break;
                }
                i3 += SMALL_DATA_BLOCK_LENGTH;
                self.index[i as usize] = i3;
            }
            j += 1;
        }

        mixed_blocks.init(fast_index_length, INDEX_3_BLOCK_LENGTH);
        mixed_blocks.extend(BlockData::Chars(&fast_index), 0, 0, fast_index_length);

        // Examine index-3 blocks. For each determine one of:
        // - same as the index-3 null block
        // - same as a fast-index block
        // - 16-bit indexes
        // - 18-bit indexes
        // We store this in the first flags entry for the index-3 block.
        //
        // Also determine an upper limit for the index-3 table length.
        let mut index3_capacity = 0;
        i3_first_null = self.index3_null_offset;
        let mut has_long_i3_blocks = false;
        // If the fast index covers the whole BMP, then
        // the multi-stage index is only for supplementary code points.
        // Otherwise, the multi-stage index covers all of Unicode.
        let i_start = if fast_i_limit < BMP_I_LIMIT {
            0
        } else {
            BMP_I_LIMIT
        };
        let i_limit = self.high_start >> SHIFT_3;
        let mut i = i_start;
        while i < i_limit {
            let mut j = i;
            let j_limit = i + INDEX_3_BLOCK_LENGTH;
            let mut ored_i3 = 0;
            let mut is_null = true;
            loop {
                let i3 = self.index[j as usize];
                ored_i3 |= i3;
                if i3 != self.data_null_offset {
                    is_null = false;
                }
                j += 1;
                if j >= j_limit {
                    break;
                }
            }
            if is_null {
                self.flags[i as usize] = I3_NULL;
                if i3_first_null < 0 {
                    if ored_i3 <= 0xffff {
                        index3_capacity += INDEX_3_BLOCK_LENGTH;
                    } else {
                        index3_capacity += INDEX_3_18BIT_BLOCK_LENGTH;
                        has_long_i3_blocks = true;
                    }
                    i3_first_null = 0;
                }
            } else if ored_i3 <= 0xffff {
                let n = mixed_blocks.find_block(
                    BlockData::Chars(&fast_index),
                    BlockData::Ints(&self.index),
                    i,
                );
                if n >= 0 {
                    self.flags[i as usize] = I3_BMP;
                    self.index[i as usize] = n;
                } else {
                    self.flags[i as usize] = I3_16;
                    index3_capacity += INDEX_3_BLOCK_LENGTH;
                }
            } else {
                self.flags[i as usize] = I3_18;
                index3_capacity += INDEX_3_18BIT_BLOCK_LENGTH;
                has_long_i3_blocks = true;
            }
            i = j;
        }

        let index2_capacity = (i_limit - i_start) >> SHIFT_2_3;

        // Length of the index-1 table, rounded up.
        let index1_length = (index2_capacity + INDEX_2_MASK) >> SHIFT_1_2;

        // Index table: Fast index, index-1, index-3, index-2.
        // +1 for possible index table padding.
        let index16_capacity =
            fast_index_length + index1_length + index3_capacity + index2_capacity + 1;
        let mut index16 = fast_index.clone();
        index16.resize(index16_capacity as usize, 0);

        mixed_blocks.init(index16_capacity, INDEX_3_BLOCK_LENGTH);
        let mut long_i3_blocks: Option<MixedBlocks> = None;
        if has_long_i3_blocks {
            let mut m = MixedBlocks::new();
            m.init(index16_capacity, INDEX_3_18BIT_BLOCK_LENGTH);
            long_i3_blocks = Some(m);
        }

        // Compact the index-3 table and write an uncompacted version of the index-2 table.
        let mut index2 = vec![0u16; index2_capacity as usize]; // index2Capacity <= index2Length
        let mut i2_length = 0;
        i3_first_null = self.index3_null_offset;
        let index3_start = fast_index_length + index1_length;
        let mut index_length = index3_start;
        let mut i = i_start;
        while i < i_limit {
            let i3;
            let mut f = self.flags[i as usize];
            if f == I3_NULL && i3_first_null < 0 {
                // First index-3 null block. Write & overlap it like a normal block, then remember it.
                f = if self.data_null_offset <= 0xffff {
                    I3_16
                } else {
                    I3_18
                };
                i3_first_null = 0;
            }
            if f == I3_NULL {
                i3 = self.index3_null_offset;
            } else if f == I3_BMP {
                i3 = self.index[i as usize];
            } else if f == I3_16 {
                let mut n = mixed_blocks.find_block(
                    BlockData::Chars(&index16),
                    BlockData::Ints(&self.index),
                    i,
                );
                if n >= 0 {
                    i3 = n;
                } else {
                    if index_length == index3_start {
                        // No overlap at the boundary between the index-1 and index-3 tables.
                        n = 0;
                    } else {
                        n = get_overlap(
                            BlockData::Chars(&index16),
                            index_length,
                            BlockData::Ints(&self.index),
                            i,
                            INDEX_3_BLOCK_LENGTH,
                        );
                    }
                    i3 = index_length - n;
                    let prev_index_length = index_length;
                    while n < INDEX_3_BLOCK_LENGTH {
                        index16[index_length as usize] = self.index[(i + n) as usize] as u16;
                        index_length += 1;
                        n += 1;
                    }
                    mixed_blocks.extend(
                        BlockData::Chars(&index16),
                        index3_start,
                        prev_index_length,
                        index_length,
                    );
                    if has_long_i3_blocks {
                        long_i3_blocks
                            .as_mut()
                            .expect("created when needed")
                            .extend(
                                BlockData::Chars(&index16),
                                index3_start,
                                prev_index_length,
                                index_length,
                            );
                    }
                }
            } else {
                debug_assert!(f == I3_18);
                debug_assert!(has_long_i3_blocks);
                // Encode an index-3 block that contains one or more data indexes exceeding 16 bits.
                let mut j = i;
                let j_limit = i + INDEX_3_BLOCK_LENGTH;
                let mut k = index_length;
                loop {
                    k += 1;
                    let mut v = self.index[j as usize];
                    j += 1;
                    let mut upper_bits = (v & 0x30000) >> 2;
                    index16[k as usize] = v as u16;
                    k += 1;
                    v = self.index[j as usize];
                    j += 1;
                    upper_bits |= (v & 0x30000) >> 4;
                    index16[k as usize] = v as u16;
                    k += 1;
                    v = self.index[j as usize];
                    j += 1;
                    upper_bits |= (v & 0x30000) >> 6;
                    index16[k as usize] = v as u16;
                    k += 1;
                    v = self.index[j as usize];
                    j += 1;
                    upper_bits |= (v & 0x30000) >> 8;
                    index16[k as usize] = v as u16;
                    k += 1;
                    v = self.index[j as usize];
                    j += 1;
                    upper_bits |= (v & 0x30000) >> 10;
                    index16[k as usize] = v as u16;
                    k += 1;
                    v = self.index[j as usize];
                    j += 1;
                    upper_bits |= (v & 0x30000) >> 12;
                    index16[k as usize] = v as u16;
                    k += 1;
                    v = self.index[j as usize];
                    j += 1;
                    upper_bits |= (v & 0x30000) >> 14;
                    index16[k as usize] = v as u16;
                    k += 1;
                    v = self.index[j as usize];
                    j += 1;
                    upper_bits |= (v & 0x30000) >> 16;
                    index16[k as usize] = v as u16;
                    k += 1;
                    index16[(k - 9) as usize] = upper_bits as u16;
                    if j >= j_limit {
                        break;
                    }
                }
                let long = long_i3_blocks.as_mut().expect("created when needed");
                let mut n = long.find_block(
                    BlockData::Chars(&index16),
                    BlockData::Chars(&index16),
                    index_length,
                );
                if n >= 0 {
                    i3 = n | 0x8000;
                } else {
                    if index_length == index3_start {
                        // No overlap at the boundary between the index-1 and index-3 tables.
                        n = 0;
                    } else {
                        n = get_overlap(
                            BlockData::Chars(&index16),
                            index_length,
                            BlockData::Chars(&index16),
                            index_length,
                            INDEX_3_18BIT_BLOCK_LENGTH,
                        );
                    }
                    i3 = (index_length - n) | 0x8000;
                    let prev_index_length = index_length;
                    if n > 0 {
                        let start = index_length;
                        while n < INDEX_3_18BIT_BLOCK_LENGTH {
                            index16[index_length as usize] = index16[(start + n) as usize];
                            index_length += 1;
                            n += 1;
                        }
                    } else {
                        index_length += INDEX_3_18BIT_BLOCK_LENGTH;
                    }
                    mixed_blocks.extend(
                        BlockData::Chars(&index16),
                        index3_start,
                        prev_index_length,
                        index_length,
                    );
                    if has_long_i3_blocks {
                        long_i3_blocks
                            .as_mut()
                            .expect("created when needed")
                            .extend(
                                BlockData::Chars(&index16),
                                index3_start,
                                prev_index_length,
                                index_length,
                            );
                    }
                }
            }
            if self.index3_null_offset < 0 && i3_first_null >= 0 {
                self.index3_null_offset = i3;
            }
            // Set the index-2 table entry.
            index2[i2_length as usize] = i3 as u16;
            i2_length += 1;
            i += INDEX_3_BLOCK_LENGTH;
        }
        debug_assert!(i2_length == index2_capacity);
        debug_assert!(index_length <= index3_start + index3_capacity);

        if self.index3_null_offset < 0 {
            self.index3_null_offset = NO_INDEX3_NULL_OFFSET;
        }
        if index_length >= (NO_INDEX3_NULL_OFFSET + INDEX_3_BLOCK_LENGTH) {
            // The index-3 offsets exceed 15 bits, or
            // the last one cannot be distinguished from the no-null-block value.
            crate::jdk::index_out_of_bounds(Some(
                "The trie data exceeds limitations of the data structure.".to_string(),
            ));
        }

        // Compact the index-2 table and write the index-1 table.
        // assert(CodePointTrie.INDEX_2_BLOCK_LENGTH == CodePointTrie.INDEX_3_BLOCK_LENGTH) :
        //     "must re-init mixedBlocks";
        let mut block_length = INDEX_2_BLOCK_LENGTH;
        let mut i1 = fast_index_length;
        let mut i = 0;
        while i < i2_length {
            let mut n;
            if (i2_length - i) >= block_length {
                // normal block
                debug_assert!(block_length == INDEX_2_BLOCK_LENGTH);
                n = mixed_blocks.find_block(
                    BlockData::Chars(&index16),
                    BlockData::Chars(&index2),
                    i,
                );
            } else {
                // highStart is inside the last index-2 block. Shorten it.
                block_length = i2_length - i;
                n = find_same_block(
                    &index16,
                    index3_start,
                    index_length,
                    &index2,
                    i,
                    block_length,
                );
            }
            let i2;
            if n >= 0 {
                i2 = n;
            } else {
                if index_length == index3_start {
                    // No overlap at the boundary between the index-1 and index-3/2 tables.
                    n = 0;
                } else {
                    n = get_overlap(
                        BlockData::Chars(&index16),
                        index_length,
                        BlockData::Chars(&index2),
                        i,
                        block_length,
                    );
                }
                i2 = index_length - n;
                let prev_index_length = index_length;
                while n < block_length {
                    index16[index_length as usize] = index2[(i + n) as usize];
                    index_length += 1;
                    n += 1;
                }
                mixed_blocks.extend(
                    BlockData::Chars(&index16),
                    index3_start,
                    prev_index_length,
                    index_length,
                );
            }
            // Set the index-1 table entry.
            index16[i1 as usize] = i2 as u16;
            i1 += 1;
            i += block_length;
        }
        debug_assert!(i1 == index3_start);
        debug_assert!(index_length <= index16_capacity);

        self.index16 = Some(index16);
        index_length
    }

    /// `compactTrie(int fastILimit)` (private).
    ///
    /// # Panics
    ///
    /// `IndexOutOfBoundsException("The trie data exceeds limitations of the
    /// data structure.")`.
    fn compact_trie(&mut self, fast_i_limit: i32) -> i32 {
        // Find the real highStart and round it up.
        debug_assert!((self.high_start & (CP_PER_INDEX_2_ENTRY - 1)) == 0);
        self.high_value = self.get(MAX_UNICODE);
        let mut real_high_start = self.find_high_start();
        real_high_start =
            (real_high_start + (CP_PER_INDEX_2_ENTRY - 1)) & !(CP_PER_INDEX_2_ENTRY - 1);
        if real_high_start == UNICODE_LIMIT {
            self.high_value = self.initial_value;
        }

        // We always store indexes and data values for the fast range.
        // Pin highStart to the top of that range while building.
        let fast_limit = fast_i_limit << SHIFT_3;
        if real_high_start < fast_limit {
            for i in (real_high_start >> SHIFT_3)..fast_i_limit {
                self.flags[i as usize] = ALL_SAME;
                self.index[i as usize] = self.high_value;
            }
            self.high_start = fast_limit;
        } else {
            self.high_start = real_high_start;
        }

        let mut ascii_data = vec![0i32; ASCII_LIMIT as usize];
        for (i, a) in ascii_data.iter_mut().enumerate() {
            *a = self.get(i as i32);
        }

        // First we look for which data blocks have the same value repeated over the whole block,
        // deduplicate such blocks, find a good null data block (for faster enumeration),
        // and get an upper bound for the necessary data array length.
        let mut all_same_blocks = AllSameBlocks::new();
        let new_data_capacity = self.compact_whole_data_blocks(fast_i_limit, &mut all_same_blocks);
        let mut new_data = ascii_data;
        new_data.resize(new_data_capacity as usize, 0);

        let data_null_index = all_same_blocks.find_most_used();

        let mut mixed_blocks = MixedBlocks::new();
        let new_data_length = self.compact_data(
            fast_i_limit,
            &mut new_data,
            data_null_index,
            &mut mixed_blocks,
        );
        debug_assert!(new_data_length <= new_data_capacity);
        self.data = new_data;
        self.data_length = new_data_length;
        if self.data_length > (0x3ffff + SMALL_DATA_BLOCK_LENGTH) {
            // The offset of the last data block is too high to be stored in the index table.
            crate::jdk::index_out_of_bounds(Some(
                "The trie data exceeds limitations of the data structure.".to_string(),
            ));
        }

        if data_null_index >= 0 {
            self.data_null_offset = self.index[data_null_index as usize];
            self.initial_value = self.data[self.data_null_offset as usize];
        } else {
            self.data_null_offset = NO_DATA_NULL_OFFSET;
        }

        let index_length = self.compact_index(fast_i_limit, &mut mixed_blocks);
        self.high_start = real_high_start;
        index_length
    }

    /// `build(CodePointTrie.Type type, CodePointTrie.ValueWidth valueWidth)`
    /// (private).
    fn build(&mut self, type_: Type, value_width: ValueWidth) -> CodePointTrie {
        // The mutable trie always stores 32-bit values.
        // When we build a UCPTrie for a smaller value width, we first mask off unused bits
        // before compacting the data.
        match value_width {
            ValueWidth::Bits32 => {}
            ValueWidth::Bits16 => self.mask_values(0xffff),
            ValueWidth::Bits8 => self.mask_values(0xff),
        }

        let fast_limit = if type_ == Type::Fast {
            BMP_LIMIT
        } else {
            SMALL_LIMIT
        };
        let mut index_length = self.compact_trie(fast_limit >> SHIFT_3);

        // Ensure data table alignment: The index length must be even for uint32_t data.
        if value_width == ValueWidth::Bits32 && (index_length & 1) != 0 {
            self.index16.as_mut().expect("compactIndex built index16")[index_length as usize] =
                0xffee; // arbitrary value
            index_length += 1;
        }

        // Make the total trie structure length a multiple of 4 bytes by padding the data table,
        // and store special values as the last two data values.
        let mut length = index_length * 2;
        let dl = |s: &Self, k: i32| s.data[(s.data_length - k) as usize];
        if value_width == ValueWidth::Bits16 {
            if ((index_length ^ self.data_length) & 1) != 0 {
                // padding
                self.data[self.data_length as usize] = self.error_value;
                self.data_length += 1;
            }
            if dl(self, 1) != self.error_value || dl(self, 2) != self.high_value {
                self.data[self.data_length as usize] = self.high_value;
                self.data_length += 1;
                self.data[self.data_length as usize] = self.error_value;
                self.data_length += 1;
            }
            length += self.data_length * 2;
        } else if value_width == ValueWidth::Bits32 {
            // 32-bit data words never need padding to a multiple of 4 bytes.
            if dl(self, 1) != self.error_value || dl(self, 2) != self.high_value {
                if dl(self, 1) != self.high_value {
                    self.data[self.data_length as usize] = self.high_value;
                    self.data_length += 1;
                }
                self.data[self.data_length as usize] = self.error_value;
                self.data_length += 1;
            }
            length += self.data_length * 4;
        } else {
            let mut and3 = (length + self.data_length) & 3;
            if and3 == 0 && dl(self, 1) == self.error_value && dl(self, 2) == self.high_value {
                // all set
            } else if and3 == 3 && dl(self, 1) == self.high_value {
                self.data[self.data_length as usize] = self.error_value;
                self.data_length += 1;
            } else {
                while and3 != 2 {
                    self.data[self.data_length as usize] = self.high_value;
                    self.data_length += 1;
                    and3 = (and3 + 1) & 3;
                }
                self.data[self.data_length as usize] = self.high_value;
                self.data_length += 1;
                self.data[self.data_length as usize] = self.error_value;
                self.data_length += 1;
            }
            length += self.data_length;
        }
        debug_assert!((length & 3) == 0);

        // Fill the index and data arrays.
        let trie_index: Vec<u16> = if self.high_start <= fast_limit {
            // Condense only the fast index from the mutable-trie index.
            let mut t = vec![0u16; index_length as usize];
            let mut i = 0;
            for v in t.iter_mut() {
                *v = self.index[i as usize] as u16;
                i += SMALL_DATA_BLOCKS_PER_BMP_BLOCK;
            }
            t
        } else {
            let index16 = self.index16.as_ref().expect("compactIndex built index16");
            if index_length as usize == index16.len() {
                self.index16.take().expect("checked above")
            } else {
                index16[..index_length as usize].to_vec()
            }
        };

        // Write the data array.
        let dlen = self.data_length as usize;
        let data = match value_width {
            ValueWidth::Bits16 => {
                Data::Data16(self.data[..dlen].iter().map(|&v| v as u16).collect())
            }
            ValueWidth::Bits32 => Data::Data32(self.data[..dlen].to_vec()),
            ValueWidth::Bits8 => Data::Data8(self.data[..dlen].iter().map(|&v| v as i8).collect()),
        };
        CodePointTrie::from_parts(
            type_,
            trie_index,
            data,
            self.high_start,
            self.index3_null_offset,
            self.data_null_offset,
        )
    }
}

impl CodePointMap for MutableCodePointTrie {
    fn get(&self, c: i32) -> i32 {
        if !(0..=MAX_UNICODE).contains(&c) {
            return self.error_value;
        }
        if c >= self.high_start {
            return self.high_value;
        }
        let i = (c >> SHIFT_3) as usize;
        if self.flags[i] == ALL_SAME {
            self.index[i]
        } else {
            self.data[(self.index[i] + (c & SMALL_DATA_MASK)) as usize]
        }
    }

    fn get_range(&self, start: i32, filter: Option<ValueFilter<'_>>, range: &mut Range) -> bool {
        if !(0..=MAX_UNICODE).contains(&start) {
            return false;
        }
        if start >= self.high_start {
            let mut value = self.high_value;
            if let Some(f) = filter {
                value = f(value);
            }
            range.set(start, MAX_UNICODE, value);
            return true;
        }
        let mut null_value = self.initial_value;
        if let Some(f) = filter {
            null_value = f(null_value);
        }
        let mut c = start;
        // Initialize to make compiler happy. Real value when haveValue is true.
        let mut trie_value = 0;
        let mut value = 0;
        let mut have_value = false;
        let mut i = c >> SHIFT_3;
        loop {
            if self.flags[i as usize] == ALL_SAME {
                let trie_value2 = self.index[i as usize];
                if have_value {
                    if trie_value2 != trie_value {
                        if filter.is_none()
                            || Self::maybe_filter_value(
                                trie_value2,
                                self.initial_value,
                                null_value,
                                filter,
                            ) != value
                        {
                            range.set(start, c - 1, value);
                            return true;
                        }
                        trie_value = trie_value2; // may or may not help
                    }
                } else {
                    trie_value = trie_value2;
                    value = Self::maybe_filter_value(
                        trie_value2,
                        self.initial_value,
                        null_value,
                        filter,
                    );
                    have_value = true;
                }
                c = (c + SMALL_DATA_BLOCK_LENGTH) & !SMALL_DATA_MASK;
            } else {
                let mut di = self.index[i as usize] + (c & SMALL_DATA_MASK);
                let mut trie_value2 = self.data[di as usize];
                if have_value {
                    if trie_value2 != trie_value {
                        if filter.is_none()
                            || Self::maybe_filter_value(
                                trie_value2,
                                self.initial_value,
                                null_value,
                                filter,
                            ) != value
                        {
                            range.set(start, c - 1, value);
                            return true;
                        }
                        trie_value = trie_value2; // may or may not help
                    }
                } else {
                    trie_value = trie_value2;
                    value = Self::maybe_filter_value(
                        trie_value2,
                        self.initial_value,
                        null_value,
                        filter,
                    );
                    have_value = true;
                }
                loop {
                    c += 1;
                    if (c & SMALL_DATA_MASK) == 0 {
                        break;
                    }
                    di += 1;
                    trie_value2 = self.data[di as usize];
                    if trie_value2 != trie_value {
                        if filter.is_none()
                            || Self::maybe_filter_value(
                                trie_value2,
                                self.initial_value,
                                null_value,
                                filter,
                            ) != value
                        {
                            range.set(start, c - 1, value);
                            return true;
                        }
                        trie_value = trie_value2; // may or may not help
                    }
                }
            }
            i += 1;
            if c >= self.high_start {
                break;
            }
        }
        debug_assert!(have_value);
        if Self::maybe_filter_value(self.high_value, self.initial_value, null_value, filter)
            != value
        {
            range.set(start, c - 1, value);
        } else {
            range.set(start, MAX_UNICODE, value);
        }
        true
    }

    fn iterator(&self) -> RangeIterator<'_> {
        RangeIterator::new(self)
    }

    fn string_iterator<'a>(&'a self, s: &'a [u16], s_index: i32) -> StringIterator<'a> {
        StringIterator::new(StringIteratorKind::Map(self), s, s_index)
    }
}

/// A data array to compare blocks in: `int[]` or `char[]`.
#[derive(Clone, Copy)]
enum BlockData<'a> {
    Ints(&'a [i32]),
    Chars(&'a [u16]),
}

impl BlockData<'_> {
    #[inline]
    fn at(&self, i: i32) -> i32 {
        match self {
            BlockData::Ints(a) => a[i as usize],
            BlockData::Chars(a) => a[i as usize] as i32,
        }
    }
}

/// `equalBlocks(int[]|char[] s, int si, int[]|char[] t, int ti, int
/// length)` (private).
fn equal_blocks(
    s: BlockData<'_>,
    mut si: i32,
    t: BlockData<'_>,
    mut ti: i32,
    mut length: i32,
) -> bool {
    while length > 0 && s.at(si) == t.at(ti) {
        si += 1;
        ti += 1;
        length -= 1;
    }
    length == 0
}

/// `allValuesSameAs(int[] p, int pi, int length, int value)` (private).
fn all_values_same_as(p: &[i32], mut pi: i32, length: i32, value: i32) -> bool {
    let p_limit = pi + length;
    while pi < p_limit && p[pi as usize] == value {
        pi += 1;
    }
    pi == p_limit
}

/// `findSameBlock(char[] p, int pStart, int length, char[] q, int qStart,
/// int blockLength)` (private): the start of a block of `p` equal to `q`
/// at `q_start`, or -1.
fn find_same_block(
    p: &[u16],
    mut p_start: i32,
    mut length: i32,
    q: &[u16],
    q_start: i32,
    block_length: i32,
) -> i32 {
    // Ensure that we do not even partially get past length.
    length -= block_length;

    while p_start <= length {
        if equal_blocks(
            BlockData::Chars(p),
            p_start,
            BlockData::Chars(q),
            q_start,
            block_length,
        ) {
            return p_start;
        }
        p_start += 1;
    }
    -1
}

/// `findAllSameBlock(int[] p, int start, int limit, int value, int
/// blockLength)` (private).
fn find_all_same_block(
    p: &[i32],
    start: i32,
    mut limit: i32,
    value: i32,
    block_length: i32,
) -> i32 {
    // Ensure that we do not even partially get past limit.
    limit -= block_length;

    let mut block = start;
    while block <= limit {
        if p[block as usize] == value {
            let mut i = 1;
            loop {
                if i == block_length {
                    return block;
                }
                if p[(block + i) as usize] != value {
                    block += i;
                    break;
                }
                i += 1;
            }
        }
        block += 1;
    }
    -1
}

/// `getOverlap(int[]|char[] p, int length, int[]|char[] q, int qStart, int
/// blockLength)` (private): the length of the longest suffix of
/// `p[..length]` that is a prefix of the block of `q`, below the block
/// length.
fn get_overlap(
    p: BlockData<'_>,
    length: i32,
    q: BlockData<'_>,
    q_start: i32,
    block_length: i32,
) -> i32 {
    let mut overlap = block_length - 1;
    debug_assert!(overlap <= length);
    while overlap > 0 && !equal_blocks(p, length - overlap, q, q_start, overlap) {
        overlap -= 1;
    }
    overlap
}

/// `getAllSameOverlap(int[] p, int length, int value, int blockLength)`
/// (private).
fn get_all_same_overlap(p: &[i32], length: i32, value: i32, block_length: i32) -> i32 {
    let min = length - (block_length - 1);
    let mut i = length;
    while min < i && p[(i - 1) as usize] == value {
        i -= 1;
    }
    length - i
}

/// `isStartOfSomeFastBlock(int dataOffset, int[] index, int fastILimit)`
/// (private).
fn is_start_of_some_fast_block(data_offset: i32, index: &[i32], fast_i_limit: i32) -> bool {
    let mut i = 0;
    while i < fast_i_limit {
        if index[i as usize] == data_offset {
            return true;
        }
        i += SMALL_DATA_BLOCKS_PER_BMP_BLOCK;
    }
    false
}

/// Finds same-value blocks, keeping a small table of the most used ones.
///
/// Equivalent to `MutableCodePointTrie.AllSameBlocks` (private).
struct AllSameBlocks {
    length: i32,
    most_recent: i32,
    indexes: [i32; AllSameBlocks::CAPACITY],
    values: [i32; AllSameBlocks::CAPACITY],
    ref_counts: [i32; AllSameBlocks::CAPACITY],
}

impl AllSameBlocks {
    /// `NEW_UNIQUE`.
    const NEW_UNIQUE: i32 = -1;
    /// `OVERFLOW`.
    const OVERFLOW: i32 = -2;
    /// `CAPACITY`.
    const CAPACITY: usize = 32;

    /// `AllSameBlocks()`.
    fn new() -> Self {
        Self {
            length: 0,
            most_recent: -1,
            indexes: [0; Self::CAPACITY],
            values: [0; Self::CAPACITY],
            ref_counts: [0; Self::CAPACITY],
        }
    }

    /// `findOrAdd(int index, int count, int value)`.
    fn find_or_add(&mut self, index: i32, count: i32, value: i32) -> i32 {
        if self.most_recent >= 0 && self.values[self.most_recent as usize] == value {
            self.ref_counts[self.most_recent as usize] += count;
            return self.indexes[self.most_recent as usize];
        }
        for i in 0..self.length as usize {
            if self.values[i] == value {
                self.most_recent = i as i32;
                self.ref_counts[i] += count;
                return self.indexes[i];
            }
        }
        if self.length as usize == Self::CAPACITY {
            return Self::OVERFLOW;
        }
        self.most_recent = self.length;
        let l = self.length as usize;
        self.indexes[l] = index;
        self.values[l] = value;
        self.ref_counts[l] = count;
        self.length += 1;
        Self::NEW_UNIQUE
    }

    /// `add(int index, int count, int value)`: replaces the least-used
    /// entry, after an overflow.
    fn add(&mut self, index: i32, count: i32, value: i32) {
        debug_assert!(self.length as usize == Self::CAPACITY);
        let mut least = -1i32;
        let mut least_count = I_LIMIT;
        for i in 0..self.length as usize {
            debug_assert!(self.values[i] != value);
            if self.ref_counts[i] < least_count {
                least = i as i32;
                least_count = self.ref_counts[i];
            }
        }
        debug_assert!(least >= 0);
        self.most_recent = least;
        let l = least as usize;
        self.indexes[l] = index;
        self.values[l] = value;
        self.ref_counts[l] = count;
    }

    /// `findMostUsed()`: the index of the most-used same-value block, -1
    /// when there is none.
    #[allow(clippy::needless_range_loop)] // indexed loop mirrors the Java for-loop
    fn find_most_used(&self) -> i32 {
        if self.length == 0 {
            return -1;
        }
        let mut max = -1i32;
        let mut max_count = 0;
        for i in 0..self.length as usize {
            if self.ref_counts[i] > max_count {
                max = i as i32;
                max_count = self.ref_counts[i];
            }
        }
        self.indexes[max as usize]
    }
}

/// Hash table of data blocks, for finding the same block again.
///
/// Equivalent to `MutableCodePointTrie.MixedBlocks` (private).
struct MixedBlocks {
    table: Option<Vec<i32>>,
    length: i32,
    shift: i32,
    mask: i32,
    block_length: i32,
}

impl MixedBlocks {
    fn new() -> Self {
        Self {
            table: None,
            length: 0,
            shift: 0,
            mask: 0,
            block_length: 0,
        }
    }

    fn table(&self) -> &[i32] {
        self.table.as_deref().expect("init() allocates the table")
    }

    /// `init(int maxLength, int newBlockLength)`.
    fn init(&mut self, max_length: i32, new_block_length: i32) {
        // We store actual data indexes + 1 to reserve 0 for empty entries.
        let max_data_index = max_length - new_block_length + 1;
        let new_length;
        if max_data_index <= 0xfff {
            // 4k
            new_length = 6007;
            self.shift = 12;
            self.mask = 0xfff;
        } else if max_data_index <= 0x7fff {
            // 32k
            new_length = 50021;
            self.shift = 15;
            self.mask = 0x7fff;
        } else if max_data_index <= 0x1ffff {
            // 128k
            new_length = 200003;
            self.shift = 17;
            self.mask = 0x1ffff;
        } else {
            // maxDataIndex up to around MAX_DATA_LENGTH, ca. 1.1M
            new_length = 1500007;
            self.shift = 21;
            self.mask = 0x1fffff;
        }
        match &mut self.table {
            Some(t) if new_length as usize <= t.len() => t[..new_length as usize].fill(0),
            _ => self.table = Some(vec![0; new_length as usize]),
        }
        self.length = new_length;

        self.block_length = new_block_length;
    }

    /// `extend(int[]|char[] data, int minStart, int prevDataLength, int
    /// newDataLength)`.
    fn extend(
        &mut self,
        data: BlockData<'_>,
        min_start: i32,
        prev_data_length: i32,
        new_data_length: i32,
    ) {
        let mut start = prev_data_length - self.block_length;
        if start >= min_start {
            start += 1; // Skip the last block that we added last time.
        } else {
            start = min_start; // Begin with the first full block.
        }
        let end = new_data_length - self.block_length;
        while start <= end {
            let hash_code = self.make_hash_code(data, start);
            self.add_entry(data, start, hash_code, start);
            start += 1;
        }
    }

    /// `findBlock(int[]|char[] data, int[]|char[] blockData, int
    /// blockStart)`.
    fn find_block(&self, data: BlockData<'_>, block_data: BlockData<'_>, block_start: i32) -> i32 {
        let hash_code = self.make_hash_code(block_data, block_start);
        let entry_index = self.find_entry(data, block_data, block_start, hash_code);
        if entry_index >= 0 {
            (self.table()[entry_index as usize] & self.mask) - 1
        } else {
            -1
        }
    }

    /// `findAllSameBlock(int[] data, int blockValue)`.
    fn find_all_same_block(&self, data: &[i32], block_value: i32) -> i32 {
        let hash_code = self.make_hash_code_value(block_value);
        let entry_index = self.find_entry_value(data, block_value, hash_code);
        if entry_index >= 0 {
            (self.table()[entry_index as usize] & self.mask) - 1
        } else {
            -1
        }
    }

    /// `makeHashCode(int[]|char[] blockData, int blockStart)` (private).
    fn make_hash_code(&self, block_data: BlockData<'_>, mut block_start: i32) -> i32 {
        let block_limit = block_start + self.block_length;
        let mut hash_code = block_data.at(block_start);
        block_start += 1;
        loop {
            hash_code = hash_code
                .wrapping_mul(37)
                .wrapping_add(block_data.at(block_start));
            block_start += 1;
            if block_start >= block_limit {
                break;
            }
        }
        hash_code
    }

    /// `makeHashCode(int blockValue)` (private).
    fn make_hash_code_value(&self, block_value: i32) -> i32 {
        let mut hash_code = block_value;
        for _ in 1..self.block_length {
            hash_code = hash_code.wrapping_mul(37).wrapping_add(block_value);
        }
        hash_code
    }

    /// `addEntry(int[] data32, char[] data16, int blockStart, int hashCode,
    /// int dataIndex)` (private).
    fn add_entry(
        &mut self,
        data: BlockData<'_>,
        block_start: i32,
        hash_code: i32,
        data_index: i32,
    ) {
        debug_assert!(0 <= data_index && data_index < self.mask);
        let entry_index = self.find_entry(data, data, block_start, hash_code);
        if entry_index < 0 {
            let shift = self.shift;
            self.table.as_mut().expect("init() allocates the table")[(!entry_index) as usize] =
                (hash_code << shift) | (data_index + 1);
        }
    }

    /// `findEntry(int[] data32, char[] data16, int[] blockData32, char[]
    /// blockData16, int blockStart, int hashCode)` (private).
    fn find_entry(
        &self,
        data: BlockData<'_>,
        block_data: BlockData<'_>,
        block_start: i32,
        hash_code: i32,
    ) -> i32 {
        let shifted_hash_code = hash_code << self.shift;
        let initial_entry_index = Self::modulo(hash_code, self.length - 1) + 1; // 1..length-1
        let table = self.table();
        let mut entry_index = initial_entry_index;
        loop {
            let entry = table[entry_index as usize];
            if entry == 0 {
                return !entry_index;
            }
            if (entry & !self.mask) == shifted_hash_code {
                let data_index = (entry & self.mask) - 1;
                if equal_blocks(data, data_index, block_data, block_start, self.block_length) {
                    return entry_index;
                }
            }
            entry_index = self.next_index(initial_entry_index, entry_index);
        }
    }

    /// `findEntry(int[] data, int blockValue, int hashCode)` (private).
    fn find_entry_value(&self, data: &[i32], block_value: i32, hash_code: i32) -> i32 {
        let shifted_hash_code = hash_code << self.shift;
        let initial_entry_index = Self::modulo(hash_code, self.length - 1) + 1; // 1..length-1
        let table = self.table();
        let mut entry_index = initial_entry_index;
        loop {
            let entry = table[entry_index as usize];
            if entry == 0 {
                return !entry_index;
            }
            if (entry & !self.mask) == shifted_hash_code {
                let data_index = (entry & self.mask) - 1;
                if all_values_same_as(data, data_index, self.block_length, block_value) {
                    return entry_index;
                }
            }
            entry_index = self.next_index(initial_entry_index, entry_index);
        }
    }

    /// `nextIndex(int initialEntryIndex, int entryIndex)` (private).
    fn next_index(&self, initial_entry_index: i32, entry_index: i32) -> i32 {
        // U_ASSERT(0 < initialEntryIndex && initialEntryIndex < length);
        (entry_index + initial_entry_index) % self.length
    }

    /// `modulo(int n, int m)` (private): `n % m`, non-negative.
    fn modulo(n: i32, m: i32) -> i32 {
        let mut i = n % m;
        if i < 0 {
            i += m;
        }
        i
    }
}
