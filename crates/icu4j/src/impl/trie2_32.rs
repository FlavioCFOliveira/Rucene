// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.Trie2_32`.

use std::ops::Deref;

use rucene::error::{LuceneError, Result};

use crate::jdk::{ByteBuffer, DataOutputStream, OutputStream};
use crate::r#impl::trie2::{
    Trie2, ValueWidth, UTRIE2_CP_PER_INDEX_1_ENTRY, UTRIE2_DATA_BLOCK_LENGTH, UTRIE2_DATA_MASK,
    UTRIE2_INDEX_1_OFFSET, UTRIE2_INDEX_2_MASK, UTRIE2_INDEX_SHIFT, UTRIE2_LSCP_INDEX_2_OFFSET,
    UTRIE2_OMITTED_BMP_INDEX_1_LENGTH, UTRIE2_SHIFT_1, UTRIE2_SHIFT_2,
};

/// A read-only `Trie2` holding 32-bit data values.
///
/// Equivalent to `com.ibm.icu.impl.Trie2_32`, a subclass of `Trie2`:
/// a [`Trie2`] of width 32, which it dereferences to.
#[derive(Debug)]
pub struct Trie2_32(Trie2);

impl Deref for Trie2_32 {
    type Target = Trie2;

    fn deref(&self) -> &Trie2 {
        &self.0
    }
}

impl Trie2_32 {
    /// `Trie2_32()` (package-private, "Internal constructor, not for general
    /// use"): an unfilled trie, which `Trie2Writable.freeze` fills.
    pub(crate) fn new() -> Trie2_32 {
        Trie2_32(Trie2::new_unfilled(ValueWidth::Bits32))
    }

    /// The base-class fields, for `Trie2Writable.freeze` (Java writes the
    /// package-private fields of the `Trie2` directly).
    pub(crate) fn trie_mut(&mut self) -> &mut Trie2 {
        &mut self.0
    }

    /// `createFromSerialized(ByteBuffer bytes)`: `Trie2.createFromSerialized`
    /// cast to `Trie2_32`.
    ///
    /// # Errors
    ///
    /// The errors of [`Trie2::create_from_serialized`];
    /// `ClassCastException` when the data is a 16-bit trie.
    pub fn create_from_serialized(bytes: &mut ByteBuffer) -> Result<Trie2_32> {
        let trie = Trie2::create_from_serialized(bytes)?;
        if trie.width != ValueWidth::Bits32 {
            return Err(LuceneError::unchecked(
                "java.lang.ClassCastException",
                "class com.ibm.icu.impl.Trie2_16 cannot be cast to class com.ibm.icu.impl.Trie2_32 \
                 (com.ibm.icu.impl.Trie2_16 and com.ibm.icu.impl.Trie2_32 are in unnamed module of loader 'app')",
            ));
        }
        Ok(Trie2_32(trie))
    }

    /// The trie as its base class.
    pub fn as_trie2(&self) -> &Trie2 {
        &self.0
    }

    /// `get(int codePoint)` (final).
    pub fn get(&self, code_point: i32) -> i32 {
        self.0.get32(code_point)
    }

    /// `getFromU16SingleLead(char codeUnit)`.
    pub fn get_from_u16_single_lead(&self, code_unit: u16) -> i32 {
        self.0.get_from_u16_single_lead32(code_unit)
    }

    /// `serialize(OutputStream os)`: writes the trie in its serialized form
    /// and returns the byte count ICU computes.
    ///
    /// # Errors
    ///
    /// The `IOException` of the stream.
    pub fn serialize(&self, os: &mut dyn OutputStream) -> Result<i32> {
        let mut dos = DataOutputStream::new(os);
        let mut bytes_written = 0;

        bytes_written += self.0.serialize_header(&mut dos)?;
        for i in 0..self.0.data_length {
            dos.write_int(
                self.0
                    .data32
                    .as_ref()
                    .expect("a 32-bit trie has 32-bit data")[i as usize],
            )?;
        }
        bytes_written += self.0.data_length * 4;
        Ok(bytes_written)
    }

    /// `getSerializedLength()`.
    pub fn get_serialized_length(&self) -> i32 {
        16 + self.0.header.index_length * 2 + self.0.data_length * 4
    }
}

impl Trie2 {
    /// The value array of this width: `data32`.
    #[inline]
    fn value32(&self, ix: i32) -> i32 {
        self.data32.as_ref().expect("a 32-bit trie has 32-bit data")[ix as usize]
    }

    /// `Trie2_32.get(int codePoint)`.
    pub(crate) fn get32(&self, code_point: i32) -> i32 {
        let mut ix;

        if code_point >= 0 {
            if code_point < 0x0d800 || (code_point > 0x0dbff && code_point <= 0x0ffff) {
                // Ordinary BMP code point, excluding leading surrogates.
                // BMP uses a single level lookup.  BMP index starts at offset 0 in the Trie2 index.
                // 16 bit data is stored in the index array itself.
                ix = self.index[(code_point >> UTRIE2_SHIFT_2) as usize] as i32;
                ix = (ix << UTRIE2_INDEX_SHIFT) + (code_point & UTRIE2_DATA_MASK);
                return self.value32(ix);
            }
            if code_point <= 0xffff {
                // Lead Surrogate Code Point.  A Separate index section is stored for
                // lead surrogate code units and code points.
                //   The main index has the code unit data.
                //   For this function, we need the code point data.
                // Note: this expression could be refactored for slightly improved efficiency, but
                //       surrogate code points will be so rare in practice that it's not worth it.
                ix = self.index[(UTRIE2_LSCP_INDEX_2_OFFSET
                    + ((code_point - 0xd800) >> UTRIE2_SHIFT_2))
                    as usize] as i32;
                ix = (ix << UTRIE2_INDEX_SHIFT) + (code_point & UTRIE2_DATA_MASK);
                return self.value32(ix);
            }
            if code_point < self.high_start {
                // Supplemental code point, use two-level lookup.
                ix = (UTRIE2_INDEX_1_OFFSET - UTRIE2_OMITTED_BMP_INDEX_1_LENGTH)
                    + (code_point >> UTRIE2_SHIFT_1);
                ix = self.index[ix as usize] as i32;
                ix += (code_point >> UTRIE2_SHIFT_2) & UTRIE2_INDEX_2_MASK;
                ix = self.index[ix as usize] as i32;
                ix = (ix << UTRIE2_INDEX_SHIFT) + (code_point & UTRIE2_DATA_MASK);
                return self.value32(ix);
            }
            if code_point <= 0x10ffff {
                return self.value32(self.high_value_index);
            }
        }

        // Fall through.  The code point is outside of the legal range of 0..0x10ffff.
        self.error_value
    }

    /// `Trie2_32.getFromU16SingleLead(char codeUnit)`.
    pub(crate) fn get_from_u16_single_lead32(&self, code_unit: u16) -> i32 {
        // Because the input is a 16 bit char, we can skip the tests for it being in
        // the BMP range.  It is.
        let mut ix = self.index[(code_unit as i32 >> UTRIE2_SHIFT_2) as usize] as i32;
        ix = (ix << UTRIE2_INDEX_SHIFT) + (code_unit as i32 & UTRIE2_DATA_MASK);
        self.value32(ix)
    }

    /// `Trie2_32.rangeEnd(int startingCP, int limit, int value)`: the last
    /// code point of the range of `value` from `starting_cp`, walking whole
    /// null blocks at once.
    pub(crate) fn range_end32(&self, starting_cp: i32, limit: i32, value: i32) -> i32 {
        let mut cp = starting_cp;
        let mut block;
        let mut index2_block;

        // Loop runs once for each of
        //   - a partial data block
        //   - a reference to the null (default) data block.
        //   - a reference to the index2 null block

        'outer_loop: loop {
            if cp >= limit {
                break;
            }
            if cp < 0x0d800 || (cp > 0x0dbff && cp <= 0x0ffff) {
                // Ordinary BMP code point, excluding leading surrogates.
                // BMP uses a single level lookup.  BMP index starts at offset 0 in the Trie2 index.
                // 16 bit data is stored in the index array itself.
                index2_block = 0;
                block = (self.index[(cp >> UTRIE2_SHIFT_2) as usize] as i32) << UTRIE2_INDEX_SHIFT;
            } else if cp < 0xffff {
                // Lead Surrogate Code Point, 0xd800 <= cp < 0xdc00
                index2_block = UTRIE2_LSCP_INDEX_2_OFFSET;
                block = (self.index[(index2_block + ((cp - 0xd800) >> UTRIE2_SHIFT_2)) as usize]
                    as i32)
                    << UTRIE2_INDEX_SHIFT;
            } else if cp < self.high_start {
                // Supplemental code point, use two-level lookup.
                let ix = (UTRIE2_INDEX_1_OFFSET - UTRIE2_OMITTED_BMP_INDEX_1_LENGTH)
                    + (cp >> UTRIE2_SHIFT_1);
                index2_block = self.index[ix as usize] as i32;
                block = (self.index
                    [(index2_block + ((cp >> UTRIE2_SHIFT_2) & UTRIE2_INDEX_2_MASK)) as usize]
                    as i32)
                    << UTRIE2_INDEX_SHIFT;
            } else {
                // Code point above highStart.
                if value == self.value32(self.high_value_index) {
                    cp = limit;
                }
                break;
            }

            if index2_block == self.index2_null_offset {
                if value != self.initial_value {
                    break;
                }
                cp += UTRIE2_CP_PER_INDEX_1_ENTRY;
            } else if block == self.data_null_offset {
                // The block at dataNullOffset has all values == initialValue.
                // Because Trie2 iteration always proceeds in ascending order, we will always
                //   encounter a null block at its beginning, and can skip over
                //   a number of code points equal to the length of the block.
                if value != self.initial_value {
                    break;
                }
                cp += UTRIE2_DATA_BLOCK_LENGTH;
            } else {
                // Current position refers to an ordinary data block.
                // Walk over the data entries, checking the values.
                let start_ix = block + (cp & UTRIE2_DATA_MASK);
                let limit_ix = block + UTRIE2_DATA_BLOCK_LENGTH;
                for ix in start_ix..limit_ix {
                    if self.value32(ix) != value {
                        // We came to an entry with a different value.
                        //   We are done.
                        cp += ix - start_ix;
                        break 'outer_loop;
                    }
                }
                // The ordinary data block contained our value until its end.
                //  Advance the current code point, and continue the outer loop.
                cp += limit_ix - start_ix;
            }
        }
        if cp > limit {
            cp = limit;
        }

        cp - 1
    }
}
