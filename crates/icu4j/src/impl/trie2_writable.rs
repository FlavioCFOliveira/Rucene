// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.Trie2Writable`.

use rucene::error::{LuceneError, Result};

use crate::r#impl::trie2::{
    Range, Trie2, UTrie2Header, ValueMapper, ValueWidth, UNEWTRIE2_INDEX_1_LENGTH,
    UNEWTRIE2_INDEX_GAP_LENGTH, UNEWTRIE2_INDEX_GAP_OFFSET, UNEWTRIE2_MAX_DATA_LENGTH,
    UNEWTRIE2_MAX_INDEX_2_LENGTH, UTRIE2_BAD_UTF8_DATA_OFFSET, UTRIE2_CP_PER_INDEX_1_ENTRY,
    UTRIE2_DATA_BLOCK_LENGTH, UTRIE2_DATA_GRANULARITY, UTRIE2_DATA_MASK, UTRIE2_DATA_START_OFFSET,
    UTRIE2_INDEX_1_OFFSET, UTRIE2_INDEX_2_BLOCK_LENGTH, UTRIE2_INDEX_2_BMP_LENGTH,
    UTRIE2_INDEX_2_MASK, UTRIE2_INDEX_2_OFFSET, UTRIE2_INDEX_SHIFT, UTRIE2_LSCP_INDEX_2_LENGTH,
    UTRIE2_LSCP_INDEX_2_OFFSET, UTRIE2_OMITTED_BMP_INDEX_1_LENGTH, UTRIE2_SHIFT_1,
    UTRIE2_SHIFT_1_2, UTRIE2_SHIFT_2, UTRIE2_UTF8_2B_INDEX_2_LENGTH,
};
use crate::r#impl::trie2_16::Trie2_16;
use crate::r#impl::trie2_32::Trie2_32;

const UNEWTRIE2_INITIAL_DATA_LENGTH: i32 = 1 << 14;
const UNEWTRIE2_MEDIUM_DATA_LENGTH: i32 = 1 << 17;
const UNEWTRIE2_INDEX_2_NULL_OFFSET: i32 = UNEWTRIE2_INDEX_GAP_OFFSET + UNEWTRIE2_INDEX_GAP_LENGTH;
const UNEWTRIE2_INDEX_2_START_OFFSET: i32 =
    UNEWTRIE2_INDEX_2_NULL_OFFSET + UTRIE2_INDEX_2_BLOCK_LENGTH;
const UNEWTRIE2_DATA_NULL_OFFSET: i32 = UTRIE2_DATA_START_OFFSET;
const UNEWTRIE2_DATA_START_OFFSET: i32 = UNEWTRIE2_DATA_NULL_OFFSET + 0x40;
const UNEWTRIE2_DATA_0800_OFFSET: i32 = UNEWTRIE2_DATA_START_OFFSET + 0x780;

const UTRIE2_MAX_INDEX_LENGTH: i32 = 0xffff;
const UTRIE2_MAX_DATA_LENGTH: i32 = 0xffff << UTRIE2_INDEX_SHIFT;

/// The identity value mapper (`Trie2.defaultValueMapper`).
fn identity(v: i32) -> i32 {
    v
}

/// A Trie2Writable is a modifiable, or build-time Trie2. Functions for
/// reading data from the Trie are all from class Trie2.
///
/// Equivalent to `com.ibm.icu.impl.Trie2Writable` (a subclass of
/// `Trie2`). The inherited `Trie2` members the builder uses (`get`,
/// `getFromU16SingleLead`, the range iterators, `rangeEnd`,
/// `hashCode`) are members of this type; Java's `Trie2` fields the
/// writable trie uses are fields here.
#[derive(Debug, Clone)]
pub struct Trie2Writable {
    // Members from Trie2.
    initial_value: i32,
    error_value: i32,
    high_start: i32,
    data_length: i32,
    data_null_offset: i32,
    f_hash: i32,
    // Private data members. From struct UNewTrie2 in ICU4C.
    index1: Vec<i32>,
    index2: Vec<i32>,
    data: Vec<i32>,
    index2_length: i32,
    data_capacity: i32,
    first_free_block: i32,
    index2_null_offset: i32,
    is_compacted: bool,
    map: Vec<i32>,
}

impl Trie2Writable {
    /// Create a new, empty, writable Trie2. 32-bit data values are used.
    ///
    /// Equivalent to `Trie2Writable(int initialValueP, int errorValueP)`.
    pub fn new(initial_value_p: i32, error_value_p: i32) -> Self {
        // This constructor corresponds to utrie2_open() in ICU4C.
        let mut t = Self::blank();
        t.init(initial_value_p, error_value_p);
        t
    }

    fn blank() -> Self {
        Self {
            initial_value: 0,
            error_value: 0,
            high_start: 0,
            data_length: 0,
            data_null_offset: 0,
            f_hash: 0,
            index1: vec![0; UNEWTRIE2_INDEX_1_LENGTH as usize],
            index2: vec![0; UNEWTRIE2_MAX_INDEX_2_LENGTH as usize],
            data: Vec::new(),
            index2_length: 0,
            data_capacity: 0,
            first_free_block: 0,
            index2_null_offset: 0,
            is_compacted: false,
            map: vec![0; (UNEWTRIE2_MAX_DATA_LENGTH >> UTRIE2_SHIFT_2) as usize],
        }
    }

    fn init(&mut self, initial_value_p: i32, error_value_p: i32) {
        self.initial_value = initial_value_p;
        self.error_value = error_value_p;
        self.high_start = 0x110000;

        self.data = vec![0; UNEWTRIE2_INITIAL_DATA_LENGTH as usize];
        self.data_capacity = UNEWTRIE2_INITIAL_DATA_LENGTH;
        self.initial_value = initial_value_p;
        self.error_value = error_value_p;
        self.high_start = 0x110000;
        self.first_free_block = 0; /* no free block in the list */
        self.is_compacted = false;

        // preallocate and reset
        // - ASCII
        // - the bad-UTF-8-data block
        // - the null data block
        let mut i: i32 = 0;
        while i < 0x80 {
            self.data[i as usize] = self.initial_value;
            i += 1;
        }
        while i < 0xc0 {
            self.data[i as usize] = self.error_value;
            i += 1;
        }
        i = UNEWTRIE2_DATA_NULL_OFFSET;
        while i < UNEWTRIE2_DATA_START_OFFSET {
            self.data[i as usize] = self.initial_value;
            i += 1;
        }
        self.data_null_offset = UNEWTRIE2_DATA_NULL_OFFSET;
        self.data_length = UNEWTRIE2_DATA_START_OFFSET;

        // set the index-2 indexes for the 2=0x80>>UTRIE2_SHIFT_2 ASCII data blocks
        i = 0;
        let mut j: i32 = 0;
        while j < 0x80 {
            self.index2[i as usize] = j;
            self.map[i as usize] = 1;
            i += 1;
            j += UTRIE2_DATA_BLOCK_LENGTH;
        }

        // reference counts for the bad-UTF-8-data block
        while j < 0xc0 {
            self.map[i as usize] = 0;
            i += 1;
            j += UTRIE2_DATA_BLOCK_LENGTH;
        }

        // Reference counts for the null data block: all blocks except for the ASCII blocks.
        // Plus 1 so that we don't drop this block during compaction.
        // Plus as many as needed for lead surrogate code points.
        // i==newTrie->dataNullOffset
        self.map[i as usize] = (0x110000 >> UTRIE2_SHIFT_2) - (0x80 >> UTRIE2_SHIFT_2)
            + 1
            + UTRIE2_LSCP_INDEX_2_LENGTH;
        i += 1;
        j += UTRIE2_DATA_BLOCK_LENGTH;
        while j < UNEWTRIE2_DATA_START_OFFSET {
            self.map[i as usize] = 0;
            i += 1;
            j += UTRIE2_DATA_BLOCK_LENGTH;
        }

        // set the remaining indexes in the BMP index-2 block
        // to the null data block
        i = 0x80 >> UTRIE2_SHIFT_2;
        while i < UTRIE2_INDEX_2_BMP_LENGTH {
            self.index2[i as usize] = UNEWTRIE2_DATA_NULL_OFFSET;
            i += 1;
        }

        // Fill the index gap with impossible values so that compaction
        // does not overlap other index-2 blocks with the gap.
        for i in 0..UNEWTRIE2_INDEX_GAP_LENGTH {
            self.index2[(UNEWTRIE2_INDEX_GAP_OFFSET + i) as usize] = -1;
        }

        // set the indexes in the null index-2 block
        for i in 0..UTRIE2_INDEX_2_BLOCK_LENGTH {
            self.index2[(UNEWTRIE2_INDEX_2_NULL_OFFSET + i) as usize] = UNEWTRIE2_DATA_NULL_OFFSET;
        }
        self.index2_null_offset = UNEWTRIE2_INDEX_2_NULL_OFFSET;
        self.index2_length = UNEWTRIE2_INDEX_2_START_OFFSET;

        // set the index-1 indexes for the linear index-2 block
        i = 0;
        j = 0;
        while i < UTRIE2_OMITTED_BMP_INDEX_1_LENGTH {
            self.index1[i as usize] = j;
            i += 1;
            j += UTRIE2_INDEX_2_BLOCK_LENGTH;
        }

        // set the remaining index-1 indexes to the null index-2 block
        while i < UNEWTRIE2_INDEX_1_LENGTH {
            self.index1[i as usize] = UNEWTRIE2_INDEX_2_NULL_OFFSET;
            i += 1;
        }

        // Preallocate and reset data for U+0080..U+07ff,
        // for 2-byte UTF-8 which will be compacted in 64-blocks
        // even if UTRIE2_SHIFT_2 is smaller.
        i = 0x80;
        while i < 0x800 {
            let iv = self.initial_value;
            // Java's constructor propagates the internal IllegalStateException.
            self.set_lscp(i, true, iv).unwrap_or_else(|e| panic!("{e}"));
            i += UTRIE2_DATA_BLOCK_LENGTH;
        }
    }

    /// Create a new build time (modifiable) Trie2 whose contents are the
    /// same as the source Trie2.
    ///
    /// Equivalent to `Trie2Writable(Trie2 source)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException`/`IllegalStateException` of the range
    /// setters (not raised for valid tries).
    pub fn from_trie2(source: &Trie2) -> Result<Self> {
        let mut t = Self::blank();
        t.init(source.initial_value, source.error_value);
        for r in source.iterator() {
            t.set_range_r(&r, true)?;
        }
        Ok(t)
    }

    /// `Trie2Writable(Trie2 source)` with a writable trie as the source.
    ///
    /// # Errors
    ///
    /// As [`from_trie2`](Self::from_trie2).
    pub fn from_writable(source: &Trie2Writable) -> Result<Self> {
        let mut t = Self::blank();
        t.init(source.initial_value, source.error_value);
        let ranges: Vec<Range> = source.iterator().collect();
        for r in ranges {
            t.set_range_r(&r, true)?;
        }
        Ok(t)
    }

    fn is_in_null_block(&self, c: i32, for_lscp: bool) -> bool {
        // Java tests Character.isHighSurrogate((char)c): the low 16 bits of c.
        let i2 = if crate::jdk::is_high_surrogate(c as u16) && for_lscp {
            (UTRIE2_LSCP_INDEX_2_OFFSET - (0xd800 >> UTRIE2_SHIFT_2)) + (c >> UTRIE2_SHIFT_2)
        } else {
            self.index1[(c >> UTRIE2_SHIFT_1) as usize]
                + ((c >> UTRIE2_SHIFT_2) & UTRIE2_INDEX_2_MASK)
        };
        let block = self.index2[i2 as usize];
        block == self.data_null_offset
    }

    fn alloc_index2_block(&mut self) -> Result<i32> {
        let new_block = self.index2_length;
        let new_top = new_block + UTRIE2_INDEX_2_BLOCK_LENGTH;
        if new_top > self.index2.len() as i32 {
            // Should never occur.
            // Either UTRIE2_MAX_BUILD_TIME_INDEX_LENGTH is incorrect,
            // or the code writes more values than should be possible.
            return Err(LuceneError::illegal_state(
                "Internal error in Trie2 creation.",
            ));
        }
        self.index2_length = new_top;
        let src = self.index2_null_offset as usize;
        self.index2.copy_within(
            src..src + UTRIE2_INDEX_2_BLOCK_LENGTH as usize,
            new_block as usize,
        );
        Ok(new_block)
    }

    fn get_index2_block(&mut self, c: i32, for_lscp: bool) -> Result<i32> {
        if (0xd800..0xdc00).contains(&c) && for_lscp {
            return Ok(UTRIE2_LSCP_INDEX_2_OFFSET);
        }
        let i1 = c >> UTRIE2_SHIFT_1;
        let mut i2 = self.index1[i1 as usize];
        if i2 == self.index2_null_offset {
            i2 = self.alloc_index2_block()?;
            self.index1[i1 as usize] = i2;
        }
        Ok(i2)
    }

    #[allow(clippy::needless_late_init)] // declare-then-assign mirrors the Java source
    fn alloc_data_block(&mut self, copy_block: i32) -> Result<i32> {
        let new_block;
        if self.first_free_block != 0 {
            // get the first free block
            new_block = self.first_free_block;
            self.first_free_block = -self.map[(new_block >> UTRIE2_SHIFT_2) as usize];
        } else {
            // get a new block from the high end
            new_block = self.data_length;
            let new_top = new_block + UTRIE2_DATA_BLOCK_LENGTH;
            if new_top > self.data_capacity {
                // out of memory in the data array
                let capacity;
                if self.data_capacity < UNEWTRIE2_MEDIUM_DATA_LENGTH {
                    capacity = UNEWTRIE2_MEDIUM_DATA_LENGTH;
                } else if self.data_capacity < UNEWTRIE2_MAX_DATA_LENGTH {
                    capacity = UNEWTRIE2_MAX_DATA_LENGTH;
                } else {
                    // Should never occur.
                    // Either UNEWTRIE2_MAX_DATA_LENGTH is incorrect,
                    // or the code writes more values than should be possible.
                    return Err(LuceneError::illegal_state(
                        "Internal error in Trie2 creation.",
                    ));
                }
                let mut new_data = vec![0i32; capacity as usize];
                new_data[..self.data_length as usize]
                    .copy_from_slice(&self.data[..self.data_length as usize]);
                self.data = new_data;
                self.data_capacity = capacity;
            }
            self.data_length = new_top;
        }
        let src = copy_block as usize;
        self.data.copy_within(
            src..src + UTRIE2_DATA_BLOCK_LENGTH as usize,
            new_block as usize,
        );
        self.map[(new_block >> UTRIE2_SHIFT_2) as usize] = 0;
        Ok(new_block)
    }

    // call when the block's reference counter reaches 0
    fn release_data_block(&mut self, block: i32) {
        // put this block at the front of the free-block chain
        self.map[(block >> UTRIE2_SHIFT_2) as usize] = -self.first_free_block;
        self.first_free_block = block;
    }

    fn is_writable_block(&self, block: i32) -> bool {
        block != self.data_null_offset && 1 == self.map[(block >> UTRIE2_SHIFT_2) as usize]
    }

    fn set_index2_entry(&mut self, i2: i32, block: i32) {
        self.map[(block >> UTRIE2_SHIFT_2) as usize] += 1; // increment first, in case block==oldBlock!
        let old_block = self.index2[i2 as usize];
        self.map[(old_block >> UTRIE2_SHIFT_2) as usize] -= 1;
        if 0 == self.map[(old_block >> UTRIE2_SHIFT_2) as usize] {
            self.release_data_block(old_block);
        }
        self.index2[i2 as usize] = block;
    }

    /// No error checking for illegal arguments.
    fn get_data_block(&mut self, c: i32, for_lscp: bool) -> Result<i32> {
        let mut i2 = self.get_index2_block(c, for_lscp)?;

        i2 += (c >> UTRIE2_SHIFT_2) & UTRIE2_INDEX_2_MASK;
        let old_block = self.index2[i2 as usize];
        if self.is_writable_block(old_block) {
            return Ok(old_block);
        }

        // allocate a new data block
        let new_block = self.alloc_data_block(old_block)?;
        self.set_index2_entry(i2, new_block);
        Ok(new_block)
    }

    /// Set a 32 bit data value for a code point.
    ///
    /// Equivalent to `set(int c, int value)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for an invalid code point.
    pub fn set(&mut self, c: i32, value: i32) -> Result<&mut Self> {
        if !(0..=0x10ffff).contains(&c) {
            return Err(LuceneError::illegal_argument("Invalid code point."));
        }
        self.set_lscp(c, true, value)?;
        self.f_hash = 0;
        Ok(self)
    }

    fn set_lscp(&mut self, c: i32, for_lscp: bool, value: i32) -> Result<&mut Self> {
        if self.is_compacted {
            self.uncompact()?;
        }
        let block = self.get_data_block(c, for_lscp)?;
        self.data[(block + (c & UTRIE2_DATA_MASK)) as usize] = value;
        Ok(self)
    }

    /// Uncompact a compacted Trie2Writable. This is needed if a the
    /// Trie2Writable was compacted in preparation for creating a read-only
    /// Trie2, and then is subsequently altered.
    fn uncompact(&mut self) -> Result<()> {
        let temp_trie = Trie2Writable::from_writable(self)?;

        // Members from Trie2Writable
        self.index1 = temp_trie.index1;
        self.index2 = temp_trie.index2;
        self.data = temp_trie.data;
        self.index2_length = temp_trie.index2_length;
        self.data_capacity = temp_trie.data_capacity;
        self.is_compacted = temp_trie.is_compacted;

        // Members From Trie2
        self.data_length = temp_trie.data_length;
        self.index2_null_offset = temp_trie.index2_null_offset;
        self.initial_value = temp_trie.initial_value;
        self.error_value = temp_trie.error_value;
        self.high_start = temp_trie.high_start;
        self.data_null_offset = temp_trie.data_null_offset;
        Ok(())
    }

    fn write_block(&mut self, mut block: i32, value: i32) {
        let limit = block + UTRIE2_DATA_BLOCK_LENGTH;
        while block < limit {
            self.data[block as usize] = value;
            block += 1;
        }
    }

    /// `initialValue` is ignored if `overwrite`=true
    fn fill_block(
        &mut self,
        block: i32,
        start: i32,
        limit: i32,
        value: i32,
        initial_value: i32,
        overwrite: bool,
    ) {
        let p_limit = block + limit;
        if overwrite {
            for i in block + start..p_limit {
                self.data[i as usize] = value;
            }
        } else {
            for i in block + start..p_limit {
                if self.data[i as usize] == initial_value {
                    self.data[i as usize] = value;
                }
            }
        }
    }

    /// Set a value in a range of code points [start..end]. All code points
    /// c with start<=c<=end will get the value if overwrite is true or if
    /// the old value is the initial value.
    ///
    /// Equivalent to `setRange(int, int, int, boolean)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for an invalid code point range.
    pub fn set_range(
        &mut self,
        mut start: i32,
        end: i32,
        value: i32,
        overwrite: bool,
    ) -> Result<&mut Self> {
        // repeat value in [start..end]
        // mark index values for repeat-data blocks by setting bit 31 of the index values
        // fill around existing values if any, if(overwrite)

        // valid, uncompacted trie and valid arguments?
        if !(0..=0x10ffff).contains(&start) || !(0..=0x10ffff).contains(&end) || start > end {
            return Err(LuceneError::illegal_argument("Invalid code point range."));
        }
        if !overwrite && value == self.initial_value {
            return Ok(self); // nothing to do
        }
        self.f_hash = 0;
        if self.is_compacted {
            self.uncompact()?;
        }

        let mut limit = end + 1;
        if (start & UTRIE2_DATA_MASK) != 0 {
            // set partial block at [start..following block boundary[
            let block = self.get_data_block(start, true)?;

            let next_start = (start + UTRIE2_DATA_BLOCK_LENGTH) & !UTRIE2_DATA_MASK;
            if next_start <= limit {
                let iv = self.initial_value;
                self.fill_block(
                    block,
                    start & UTRIE2_DATA_MASK,
                    UTRIE2_DATA_BLOCK_LENGTH,
                    value,
                    iv,
                    overwrite,
                );
                start = next_start;
            } else {
                let iv = self.initial_value;
                self.fill_block(
                    block,
                    start & UTRIE2_DATA_MASK,
                    limit & UTRIE2_DATA_MASK,
                    value,
                    iv,
                    overwrite,
                );
                return Ok(self);
            }
        }

        // number of positions in the last, partial block
        let rest = limit & UTRIE2_DATA_MASK;

        // round down limit to a block boundary
        limit &= !UTRIE2_DATA_MASK;

        // iterate over all-value blocks
        let mut repeat_block = if value == self.initial_value {
            self.data_null_offset
        } else {
            -1
        };

        while start < limit {
            let mut set_repeat_block = false;

            if value == self.initial_value && self.is_in_null_block(start, true) {
                start += UTRIE2_DATA_BLOCK_LENGTH; // nothing to do
                continue;
            }

            // get index value
            let mut i2 = self.get_index2_block(start, true)?;
            i2 += (start >> UTRIE2_SHIFT_2) & UTRIE2_INDEX_2_MASK;
            let block = self.index2[i2 as usize];
            if self.is_writable_block(block) {
                // already allocated
                if overwrite && block >= UNEWTRIE2_DATA_0800_OFFSET {
                    // We overwrite all values, and it's not a
                    // protected (ASCII-linear or 2-byte UTF-8) block:
                    // replace with the repeatBlock.
                    set_repeat_block = true;
                } else {
                    // !overwrite, or protected block: just write the values into this block
                    let iv = self.initial_value;
                    self.fill_block(block, 0, UTRIE2_DATA_BLOCK_LENGTH, value, iv, overwrite);
                }
            } else if self.data[block as usize] != value
                && (overwrite || block == self.data_null_offset)
            {
                // Set the repeatBlock instead of the null block or previous repeat block:
                //
                // If !isWritableBlock() then all entries in the block have the same value
                // because it's the null block or a range block (the repeatBlock from a previous
                // call to utrie2_setRange32()).
                // No other blocks are used multiple times before compacting.
                //
                // The null block is the only non-writable block with the initialValue because
                // of the repeatBlock initialization above. (If value==initialValue, then
                // the repeatBlock will be the null data block.)
                //
                // We set our repeatBlock if the desired value differs from the block's value,
                // and if we overwrite any data or if the data is all initial values
                // (which is the same as the block being the null block, see above).
                set_repeat_block = true;
            }
            if set_repeat_block {
                if repeat_block >= 0 {
                    self.set_index2_entry(i2, repeat_block);
                } else {
                    // create and set and fill the repeatBlock
                    repeat_block = self.get_data_block(start, true)?;
                    self.write_block(repeat_block, value);
                }
            }

            start += UTRIE2_DATA_BLOCK_LENGTH;
        }

        if rest > 0 {
            // set partial block at [last block boundary..limit[
            let block = self.get_data_block(start, true)?;
            let iv = self.initial_value;
            self.fill_block(block, 0, rest, value, iv, overwrite);
        }

        Ok(self)
    }

    /// Set the values from a Trie2.Range.
    ///
    /// Equivalent to `setRange(Trie2.Range, boolean)`.
    ///
    /// # Errors
    ///
    /// As [`set_range`](Self::set_range).
    pub fn set_range_r(&mut self, range: &Range, overwrite: bool) -> Result<&mut Self> {
        self.f_hash = 0;
        if range.lead_surrogate {
            for c in range.start_code_point..=range.end_code_point {
                if overwrite || self.get_from_u16_single_lead(c as u16) == self.initial_value {
                    self.set_for_lead_surrogate_code_unit(c as u16, range.value)?;
                }
            }
        } else {
            self.set_range(
                range.start_code_point,
                range.end_code_point,
                range.value,
                overwrite,
            )?;
        }
        Ok(self)
    }

    /// Set a value for a UTF-16 code unit. Note that a Trie2 stores
    /// separate values for supplementary code points in the lead surrogate
    /// range (accessed via the plain set() and get() interfaces) and for
    /// lead surrogate code units.
    ///
    /// Equivalent to `setForLeadSurrogateCodeUnit(char, int)`.
    ///
    /// # Errors
    ///
    /// `IllegalStateException` for an internal capacity error.
    pub fn set_for_lead_surrogate_code_unit(
        &mut self,
        code_unit: u16,
        value: i32,
    ) -> Result<&mut Self> {
        self.f_hash = 0;
        self.set_lscp(code_unit as i32, false, value)?;
        Ok(self)
    }

    /// Get the value for a code point as stored in the Trie2.
    ///
    /// Equivalent to `get(int codePoint)`.
    pub fn get(&self, code_point: i32) -> i32 {
        if !(0..=0x10ffff).contains(&code_point) {
            self.error_value
        } else {
            self.get_lscp(code_point, true)
        }
    }

    fn get_lscp(&self, c: i32, from_lscp: bool) -> i32 {
        if c >= self.high_start && (!((0xd800..0xdc00).contains(&c)) || from_lscp) {
            return self.data[(self.data_length - UTRIE2_DATA_GRANULARITY) as usize];
        }

        let i2 = if (0xd800..0xdc00).contains(&c) && from_lscp {
            (UTRIE2_LSCP_INDEX_2_OFFSET - (0xd800 >> UTRIE2_SHIFT_2)) + (c >> UTRIE2_SHIFT_2)
        } else {
            self.index1[(c >> UTRIE2_SHIFT_1) as usize]
                + ((c >> UTRIE2_SHIFT_2) & UTRIE2_INDEX_2_MASK)
        };
        let block = self.index2[i2 as usize];
        self.data[(block + (c & UTRIE2_DATA_MASK)) as usize]
    }

    /// Get a trie value for a UTF-16 code unit.
    ///
    /// Equivalent to `getFromU16SingleLead(char c)`.
    pub fn get_from_u16_single_lead(&self, c: u16) -> i32 {
        self.get_lscp(c as i32, false)
    }

    fn equal_int(a: &[i32], s: i32, t: i32, length: i32) -> bool {
        for i in 0..length {
            if a[(s + i) as usize] != a[(t + i) as usize] {
                return false;
            }
        }
        true
    }

    fn find_same_index2_block(&self, mut index2_length: i32, other_block: i32) -> i32 {
        // ensure that we do not even partially get past index2Length
        index2_length -= UTRIE2_INDEX_2_BLOCK_LENGTH;

        let mut block = 0;
        while block <= index2_length {
            if Self::equal_int(
                &self.index2,
                block,
                other_block,
                UTRIE2_INDEX_2_BLOCK_LENGTH,
            ) {
                return block;
            }
            block += 1;
        }
        -1
    }

    fn find_same_data_block(
        &self,
        mut data_length: i32,
        other_block: i32,
        block_length: i32,
    ) -> i32 {
        // ensure that we do not even partially get past dataLength
        data_length -= block_length;

        let mut block = 0;
        while block <= data_length {
            if Self::equal_int(&self.data, block, other_block, block_length) {
                return block;
            }
            block += UTRIE2_DATA_GRANULARITY;
        }
        -1
    }

    /// Find the start of the last range in the trie by enumerating backward.
    /// Indexes for supplementary code points higher than this will be
    /// omitted.
    fn find_high_start(&self, high_value: i32) -> i32 {
        // set variables for previous range
        let mut prev_i2_block;
        let mut prev_block;
        if high_value == self.initial_value {
            prev_i2_block = self.index2_null_offset;
            prev_block = self.data_null_offset;
        } else {
            prev_i2_block = -1;
            prev_block = -1;
        }
        let prev = 0x110000;

        // enumerate index-2 blocks
        let mut i1 = UNEWTRIE2_INDEX_1_LENGTH;
        let mut c = prev;
        while c > 0 {
            i1 -= 1;
            let i2_block = self.index1[i1 as usize];
            if i2_block == prev_i2_block {
                // the index-2 block is the same as the previous one, and filled with highValue
                c -= UTRIE2_CP_PER_INDEX_1_ENTRY;
                continue;
            }
            prev_i2_block = i2_block;
            if i2_block == self.index2_null_offset {
                // this is the null index-2 block
                if high_value != self.initial_value {
                    return c;
                }
                c -= UTRIE2_CP_PER_INDEX_1_ENTRY;
            } else {
                // enumerate data blocks for one index-2 block
                let mut i2 = UTRIE2_INDEX_2_BLOCK_LENGTH;
                while i2 > 0 {
                    i2 -= 1;
                    let block = self.index2[(i2_block + i2) as usize];
                    if block == prev_block {
                        // the block is the same as the previous one, and filled with highValue
                        c -= UTRIE2_DATA_BLOCK_LENGTH;
                        continue;
                    }
                    prev_block = block;
                    if block == self.data_null_offset {
                        // this is the null data block
                        if high_value != self.initial_value {
                            return c;
                        }
                        c -= UTRIE2_DATA_BLOCK_LENGTH;
                    } else {
                        let mut j = UTRIE2_DATA_BLOCK_LENGTH;
                        while j > 0 {
                            j -= 1;
                            let value = self.data[(block + j) as usize];
                            if value != high_value {
                                return c;
                            }
                            c -= 1;
                        }
                    }
                }
            }
        }

        // deliver last range
        0
    }

    /// Compact a build-time trie.
    #[allow(clippy::explicit_counter_loop)] // counter mirrors the Java for-loop
    fn compact_data(&mut self) {
        // do not compact linear-ASCII data
        let mut new_start = UTRIE2_DATA_START_OFFSET;
        let mut start = 0;
        let mut i = 0;
        while start < new_start {
            self.map[i as usize] = start;
            start += UTRIE2_DATA_BLOCK_LENGTH;
            i += 1;
        }

        // Start with a block length of 64 for 2-byte UTF-8,
        // then switch to UTRIE2_DATA_BLOCK_LENGTH.
        let mut block_length = 64;
        let mut block_count = block_length >> UTRIE2_SHIFT_2;
        start = new_start;
        while start < self.data_length {
            // start: index of first entry of current block
            // newStart: index where the current block is to be moved
            //           (right after current end of already-compacted data)
            if start == UNEWTRIE2_DATA_0800_OFFSET {
                block_length = UTRIE2_DATA_BLOCK_LENGTH;
                block_count = 1;
            }

            // skip blocks that are not used
            if self.map[(start >> UTRIE2_SHIFT_2) as usize] <= 0 {
                // advance start to the next block
                start += block_length;

                // leave newStart with the previous block!
                continue;
            }

            // search for an identical block
            let mut moved_start = self.find_same_data_block(new_start, start, block_length);
            if moved_start >= 0 {
                // found an identical block, set the other block's index value for the current block
                let mut map_index = start >> UTRIE2_SHIFT_2;
                for _ in 0..block_count {
                    self.map[map_index as usize] = moved_start;
                    map_index += 1;
                    moved_start += UTRIE2_DATA_BLOCK_LENGTH;
                }

                // advance start to the next block
                start += block_length;

                // leave newStart with the previous block!
                continue;
            }

            // see if the beginning of this block can be overlapped with the end of the previous block
            // look for maximum overlap (modulo granularity) with the previous, adjacent block
            let mut overlap = block_length - UTRIE2_DATA_GRANULARITY;
            while overlap > 0 && !Self::equal_int(&self.data, new_start - overlap, start, overlap) {
                overlap -= UTRIE2_DATA_GRANULARITY;
            }

            if overlap > 0 || new_start < start {
                // some overlap, or just move the whole block
                moved_start = new_start - overlap;
                let mut map_index = start >> UTRIE2_SHIFT_2;
                for _ in 0..block_count {
                    self.map[map_index as usize] = moved_start;
                    map_index += 1;
                    moved_start += UTRIE2_DATA_BLOCK_LENGTH;
                }

                // move the non-overlapping indexes to their new positions
                start += overlap;
                for _ in 0..block_length - overlap {
                    self.data[new_start as usize] = self.data[start as usize];
                    new_start += 1;
                    start += 1;
                }
            } else {
                // no overlap && newStart==start
                let mut map_index = start >> UTRIE2_SHIFT_2;
                for _ in 0..block_count {
                    self.map[map_index as usize] = start;
                    map_index += 1;
                    start += UTRIE2_DATA_BLOCK_LENGTH;
                }
                new_start = start;
            }
        }

        // now adjust the index-2 table
        let mut i = 0;
        while i < self.index2_length {
            if i == UNEWTRIE2_INDEX_GAP_OFFSET {
                // Gap indexes are invalid (-1). Skip over the gap.
                i += UNEWTRIE2_INDEX_GAP_LENGTH;
            }
            self.index2[i as usize] =
                self.map[(self.index2[i as usize] >> UTRIE2_SHIFT_2) as usize];
            i += 1;
        }
        self.data_null_offset = self.map[(self.data_null_offset >> UTRIE2_SHIFT_2) as usize];

        // ensure dataLength alignment
        while (new_start & (UTRIE2_DATA_GRANULARITY - 1)) != 0 {
            self.data[new_start as usize] = self.initial_value;
            new_start += 1;
        }

        // UTRIE2_DEBUG is false: no diagnostic output.
        self.data_length = new_start;
    }

    fn compact_index2(&mut self) {
        // do not compact linear-BMP index-2 blocks
        let mut new_start = UTRIE2_INDEX_2_BMP_LENGTH;
        let mut start = 0;
        let mut i = 0;
        while start < new_start {
            self.map[i as usize] = start;
            start += UTRIE2_INDEX_2_BLOCK_LENGTH;
            i += 1;
        }

        // Reduce the index table gap to what will be needed at runtime.
        new_start +=
            UTRIE2_UTF8_2B_INDEX_2_LENGTH + ((self.high_start - 0x10000) >> UTRIE2_SHIFT_1);

        start = UNEWTRIE2_INDEX_2_NULL_OFFSET;
        while start < self.index2_length {
            // start: index of first entry of current block
            // newStart: index where the current block is to be moved
            //           (right after current end of already-compacted data)

            // search for an identical block
            let moved_start = self.find_same_index2_block(new_start, start);
            if moved_start >= 0 {
                // found an identical block, set the other block's index value for the current block
                self.map[(start >> UTRIE2_SHIFT_1_2) as usize] = moved_start;

                // advance start to the next block
                start += UTRIE2_INDEX_2_BLOCK_LENGTH;

                // leave newStart with the previous block!
                continue;
            }

            // see if the beginning of this block can be overlapped with the end of the previous block
            // look for maximum overlap with the previous, adjacent block
            let mut overlap = UTRIE2_INDEX_2_BLOCK_LENGTH - 1;
            while overlap > 0 && !Self::equal_int(&self.index2, new_start - overlap, start, overlap)
            {
                overlap -= 1;
            }

            if overlap > 0 || new_start < start {
                // some overlap, or just move the whole block
                self.map[(start >> UTRIE2_SHIFT_1_2) as usize] = new_start - overlap;

                // move the non-overlapping indexes to their new positions
                start += overlap;
                for _ in 0..UTRIE2_INDEX_2_BLOCK_LENGTH - overlap {
                    self.index2[new_start as usize] = self.index2[start as usize];
                    new_start += 1;
                    start += 1;
                }
            } else {
                // no overlap && newStart==start
                self.map[(start >> UTRIE2_SHIFT_1_2) as usize] = start;
                start += UTRIE2_INDEX_2_BLOCK_LENGTH;
                new_start = start;
            }
        }

        // now adjust the index-1 table
        for i in 0..UNEWTRIE2_INDEX_1_LENGTH {
            self.index1[i as usize] =
                self.map[(self.index1[i as usize] >> UTRIE2_SHIFT_1_2) as usize];
        }
        self.index2_null_offset = self.map[(self.index2_null_offset >> UTRIE2_SHIFT_1_2) as usize];

        // Ensure data table alignment:
        // Needs to be granularity-aligned for 16-bit trie
        // (so that dataMove will be down-shiftable),
        // and 2-aligned for uint32_t data.

        // Arbitrary value: 0x3fffc not possible for real data.
        while (new_start & ((UTRIE2_DATA_GRANULARITY - 1) | 1)) != 0 {
            self.index2[new_start as usize] = 0x0000ffff << UTRIE2_INDEX_SHIFT;
            new_start += 1;
        }

        // UTRIE2_DEBUG is false: no diagnostic output.
        self.index2_length = new_start;
    }

    fn compact_trie(&mut self) -> Result<()> {
        // find highStart and round it up
        let mut high_value = self.get(0x10ffff);
        let mut local_high_start = self.find_high_start(high_value);
        local_high_start = (local_high_start + (UTRIE2_CP_PER_INDEX_1_ENTRY - 1))
            & !(UTRIE2_CP_PER_INDEX_1_ENTRY - 1);
        if local_high_start == 0x110000 {
            high_value = self.error_value;
        }

        // Set trie->highStart only after utrie2_get32(trie, highStart).
        // Otherwise utrie2_get32(trie, highStart) would try to read the highValue.
        self.high_start = local_high_start;

        // UTRIE2_DEBUG is false: no diagnostic output.

        if self.high_start < 0x110000 {
            // Blank out [highStart..10ffff] to release associated data blocks.
            let supp_high_start = if self.high_start <= 0x10000 {
                0x10000
            } else {
                self.high_start
            };
            let iv = self.initial_value;
            self.set_range(supp_high_start, 0x10ffff, iv, true)?;
        }

        self.compact_data();
        if self.high_start > 0x10000 {
            self.compact_index2();
        } else {
            // UTRIE2_DEBUG is false: no diagnostic output.
        }

        // Store the highValue in the data array and round up the dataLength.
        // Must be done after compactData() because that assumes that dataLength
        // is a multiple of UTRIE2_DATA_BLOCK_LENGTH.
        self.data[self.data_length as usize] = high_value;
        self.data_length += 1;
        while (self.data_length & (UTRIE2_DATA_GRANULARITY - 1)) != 0 {
            self.data[self.data_length as usize] = self.initial_value;
            self.data_length += 1;
        }

        self.is_compacted = true;
        Ok(())
    }

    /// Produce an optimized, read-only Trie2_16 from this writable Trie.
    /// The data values outside of the range that will fit in a 16 bit
    /// unsigned value will be truncated.
    ///
    /// Equivalent to `toTrie2_16()`.
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException` when the data is too large.
    pub fn to_trie2_16(&mut self) -> Result<Trie2_16> {
        let mut frozen_trie = Trie2_16::new();
        self.freeze(frozen_trie.trie_mut(), ValueWidth::Bits16)?;
        Ok(frozen_trie)
    }

    /// Produce an optimized, read-only Trie2_32 from this writable Trie.
    ///
    /// Equivalent to `toTrie2_32()`.
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException` when the data is too large.
    pub fn to_trie2_32(&mut self) -> Result<Trie2_32> {
        let mut frozen_trie = Trie2_32::new();
        self.freeze(frozen_trie.trie_mut(), ValueWidth::Bits32)?;
        Ok(frozen_trie)
    }

    /// Compact the data and then populate an optimized read-only Trie.
    fn freeze(&mut self, dest: &mut Trie2, value_bits: ValueWidth) -> Result<()> {
        // compact if necessary
        if !self.is_compacted {
            self.compact_trie()?;
        }

        let all_indexes_length = if self.high_start <= 0x10000 {
            UTRIE2_INDEX_1_OFFSET
        } else {
            self.index2_length
        };
        // >0 if the data is moved to the end of the index array
        let data_move = if value_bits == ValueWidth::Bits16 {
            all_indexes_length
        } else {
            0
        };

        // are indexLength and dataLength within limits?
        if all_indexes_length > UTRIE2_MAX_INDEX_LENGTH /* for unshifted indexLength */
            || (data_move + self.data_null_offset) > 0xffff /* for unshifted dataNullOffset */
            || (data_move + UNEWTRIE2_DATA_0800_OFFSET) > 0xffff /* for unshifted 2-byte UTF-8 index-2 values */
            || (data_move + self.data_length) > UTRIE2_MAX_DATA_LENGTH
        /* for shiftedDataLength */
        {
            return Err(LuceneError::unsupported_operation(
                "Trie2 data is too large.",
            ));
        }

        // calculate the sizes of, and allocate, the index and data arrays
        let mut index_length = all_indexes_length;
        if value_bits == ValueWidth::Bits16 {
            index_length += self.data_length;
        } else {
            dest.data32 = Some(vec![0i32; self.data_length as usize]);
        }
        dest.index = vec![0u16; index_length as usize];

        dest.index_length = all_indexes_length;
        dest.data_length = self.data_length;
        if self.high_start <= 0x10000 {
            dest.index2_null_offset = 0xffff;
        } else {
            dest.index2_null_offset = UTRIE2_INDEX_2_OFFSET + self.index2_null_offset;
        }
        dest.initial_value = self.initial_value;
        dest.error_value = self.error_value;
        dest.high_start = self.high_start;
        dest.high_value_index = data_move + self.data_length - UTRIE2_DATA_GRANULARITY;
        dest.data_null_offset = data_move + self.data_null_offset;

        // Create a header and set the its fields.
        //   (This is only used in the event that we serialize the Trie, but is
        //    convenient to do here.)
        dest.header = UTrie2Header {
            signature: 0x54726932, /* "Tri2" */
            options: if value_bits == ValueWidth::Bits16 {
                0
            } else {
                1
            },
            index_length: dest.index_length,
            shifted_data_length: dest.data_length >> UTRIE2_INDEX_SHIFT,
            index2_null_offset: dest.index2_null_offset,
            data_null_offset: dest.data_null_offset,
            shifted_high_start: dest.high_start >> UTRIE2_SHIFT_1,
        };

        // write the index-2 array values shifted right by UTRIE2_INDEX_SHIFT, after adding dataMove
        let mut dest_idx = 0usize;
        for i in 0..UTRIE2_INDEX_2_BMP_LENGTH {
            dest.index[dest_idx] =
                ((self.index2[i as usize] + data_move) >> UTRIE2_INDEX_SHIFT) as u16;
            dest_idx += 1;
        }

        // write UTF-8 2-byte index-2 values, not right-shifted
        let mut i = 0;
        while i < (0xc2 - 0xc0) {
            // C0..C1
            dest.index[dest_idx] = (data_move + UTRIE2_BAD_UTF8_DATA_OFFSET) as u16;
            dest_idx += 1;
            i += 1;
        }
        while i < (0xe0 - 0xc0) {
            // C2..DF
            dest.index[dest_idx] =
                (data_move + self.index2[(i << (6 - UTRIE2_SHIFT_2)) as usize]) as u16;
            dest_idx += 1;
            i += 1;
        }

        if self.high_start > 0x10000 {
            let index1_length = (self.high_start - 0x10000) >> UTRIE2_SHIFT_1;
            let index2_offset =
                UTRIE2_INDEX_2_BMP_LENGTH + UTRIE2_UTF8_2B_INDEX_2_LENGTH + index1_length;

            // write 16-bit index-1 values for supplementary code points
            for i in 0..index1_length {
                dest.index[dest_idx] = (UTRIE2_INDEX_2_OFFSET
                    + self.index1[(i + UTRIE2_OMITTED_BMP_INDEX_1_LENGTH) as usize])
                    as u16;
                dest_idx += 1;
            }

            // write the index-2 array values for supplementary code points,
            // shifted right by UTRIE2_INDEX_SHIFT, after adding dataMove
            for i in 0..self.index2_length - index2_offset {
                dest.index[dest_idx] = ((data_move + self.index2[(index2_offset + i) as usize])
                    >> UTRIE2_INDEX_SHIFT) as u16;
                dest_idx += 1;
            }
        }

        // write the 16/32-bit data array
        match value_bits {
            ValueWidth::Bits16 => {
                // write 16-bit data values
                debug_assert!(dest_idx as i32 == data_move);
                dest.data16 = dest_idx as i32;
                for i in 0..self.data_length {
                    dest.index[dest_idx] = self.data[i as usize] as u16;
                    dest_idx += 1;
                }
            }
            ValueWidth::Bits32 => {
                // write 32-bit data values
                let d = dest
                    .data32
                    .as_mut()
                    .expect("allocated above for 32-bit data");
                d.copy_from_slice(&self.data[..self.data_length as usize]);
            }
        }
        // The writable, but compressed, Trie2 stays around unless the caller drops its references to it.
        Ok(())
    }

    /// `equals(Object other)` (inherited from `Trie2`, final): the same
    /// ranges, error value and initial value.
    pub fn equals(&self, other: &Trie2Writable) -> bool {
        let mut other_iter = other.iterator();
        for range_from_this in self.iterator() {
            match other_iter.next() {
                None => return false,
                Some(range_from_other) => {
                    if range_from_this != range_from_other {
                        return false;
                    }
                }
            }
        }
        if other_iter.has_next() {
            return false;
        }
        if self.error_value != other.error_value || self.initial_value != other.initial_value {
            return false;
        }
        true
    }

    /// `hashCode()` (inherited from `Trie2`): the FNV hash of the ranges,
    /// cached in `fHash` until the next change.
    pub fn hash_code(&mut self) -> i32 {
        if self.f_hash == 0 {
            // initHash(), hashInt(): Trie2's FNV helpers.
            let hash_byte = |h: i32, b: i32| h.wrapping_mul(16777619) ^ b;
            let hash_int = |mut h: i32, i: i32| {
                h = hash_byte(h, i & 255);
                h = hash_byte(h, (i >> 8) & 255);
                h = hash_byte(h, (i >> 16) & 255);
                hash_byte(h, (i >> 24) & 255)
            };
            let mut hash = 0x811C9DC5_u32 as i32;
            for r in self.iterator() {
                hash = hash_int(hash, r.hash_code());
            }
            if hash == 0 {
                hash = 1;
            }
            self.f_hash = hash;
        }
        self.f_hash
    }

    /// `Trie2.rangeEnd(int, int, int)` (inherited): the last code point of
    /// the range from `start` with the same value `val`.
    pub fn range_end(&self, start: i32, limitp: i32, val: i32) -> i32 {
        let limit = self.high_start.min(limitp);
        let mut c = start + 1;
        while c < limit {
            if self.get(c) != val {
                break;
            }
            c += 1;
        }
        if c >= self.high_start {
            c = limitp;
        }
        c - 1
    }

    /// `iterator()` (inherited from `Trie2`).
    pub fn iterator(&self) -> Trie2WritableIterator<'_, 'static> {
        Trie2WritableIterator::new(self, &identity)
    }

    /// `iterator(ValueMapper)` (inherited from `Trie2`).
    pub fn iterator_with<'m>(&self, mapper: ValueMapper<'m>) -> Trie2WritableIterator<'_, 'm> {
        Trie2WritableIterator::new(self, mapper)
    }

    /// `iteratorForLeadSurrogate(char)` (inherited from `Trie2`).
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for a value that is not a lead surrogate.
    pub fn iterator_for_lead_surrogate(
        &self,
        lead: u16,
    ) -> Result<Trie2WritableIterator<'_, 'static>> {
        Trie2WritableIterator::for_lead_surrogate(self, lead, &identity)
    }
}

/// `Trie2.Trie2Iterator` over a [`Trie2Writable`] (the inherited iterator,
/// reading the writable trie's `get` and `getFromU16SingleLead`).
///
/// As the iterator of [`Trie2`], each step returns a copy of the range.
pub struct Trie2WritableIterator<'a, 'm> {
    trie: &'a Trie2Writable,
    mapper: ValueMapper<'m>,
    next_start: i32,
    limit_cp: i32,
    doing_code_points: bool,
    do_lead_surrogates: bool,
}

impl std::fmt::Debug for Trie2WritableIterator<'_, '_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Trie2WritableIterator")
            .field("next_start", &self.next_start)
            .finish_non_exhaustive()
    }
}

impl<'a, 'm> Trie2WritableIterator<'a, 'm> {
    fn new(trie: &'a Trie2Writable, vm: ValueMapper<'m>) -> Self {
        Self {
            trie,
            mapper: vm,
            next_start: 0,
            limit_cp: 0x110000,
            doing_code_points: true,
            do_lead_surrogates: true,
        }
    }

    fn for_lead_surrogate(
        trie: &'a Trie2Writable,
        lead_surrogate: u16,
        vm: ValueMapper<'m>,
    ) -> Result<Self> {
        if !(0xd800..=0xdbff).contains(&lead_surrogate) {
            return Err(LuceneError::illegal_argument("Bad lead surrogate value."));
        }
        let next_start = ((lead_surrogate as i32) - 0xd7c0) << 10;
        Ok(Self {
            trie,
            mapper: vm,
            next_start,
            limit_cp: next_start + 0x400,
            doing_code_points: true,
            do_lead_surrogates: false,
        })
    }

    /// `hasNext()`.
    pub fn has_next(&self) -> bool {
        (self.doing_code_points && (self.do_lead_surrogates || self.next_start < self.limit_cp))
            || self.next_start < 0xdc00
    }

    fn range_end_ls(&self, starting_ls: u16) -> i32 {
        if starting_ls >= 0xdbff {
            return 0xdbff;
        }
        let val = self.trie.get_from_u16_single_lead(starting_ls);
        let mut c = starting_ls as i32 + 1;
        while c <= 0x0dbff {
            if self.trie.get_from_u16_single_lead(c as u16) != val {
                break;
            }
            c += 1;
        }
        c - 1
    }
}

impl Iterator for Trie2WritableIterator<'_, '_> {
    type Item = Range;

    fn next(&mut self) -> Option<Range> {
        if !self.has_next() {
            return None;
        }
        if self.next_start >= self.limit_cp {
            // Switch over from iterating normal code point values to
            //   doing the alternate lead-surrogate values.
            self.doing_code_points = false;
            self.next_start = 0xd800;
        }
        let mut end_of_range;
        let mut val;
        let mapped_val;
        if self.doing_code_points {
            // Iteration over code point values.
            val = self.trie.get(self.next_start);
            mapped_val = (self.mapper)(val);
            end_of_range = self.trie.range_end(self.next_start, self.limit_cp, val);
            // Loop once for each range in the Trie2 with the same raw (unmapped) value.
            // Loop continues so long as the mapped values are the same.
            loop {
                if end_of_range >= self.limit_cp - 1 {
                    break;
                }
                val = self.trie.get(end_of_range + 1);
                if (self.mapper)(val) != mapped_val {
                    break;
                }
                end_of_range = self.trie.range_end(end_of_range + 1, self.limit_cp, val);
            }
        } else {
            // Iteration over the alternate lead surrogate values.
            val = self.trie.get_from_u16_single_lead(self.next_start as u16);
            mapped_val = (self.mapper)(val);
            end_of_range = self.range_end_ls(self.next_start as u16);
            // Loop once for each range in the Trie2 with the same raw (unmapped) value.
            // Loop continues so long as the mapped values are the same.
            loop {
                if end_of_range >= 0xdbff {
                    break;
                }
                val = self
                    .trie
                    .get_from_u16_single_lead((end_of_range + 1) as u16);
                if (self.mapper)(val) != mapped_val {
                    break;
                }
                end_of_range = self.range_end_ls((end_of_range + 1) as u16);
            }
        }
        let return_value = Range {
            start_code_point: self.next_start,
            end_code_point: end_of_range,
            value: mapped_val,
            lead_surrogate: !self.doing_code_points,
        };
        self.next_start = end_of_range + 1;
        Some(return_value)
    }
}
