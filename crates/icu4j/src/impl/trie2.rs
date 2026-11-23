// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.Trie2`.

use std::sync::atomic::{AtomicI32, Ordering};

use rucene::error::{LuceneError, Result};

use crate::jdk::{ByteBuffer, ByteOrder, DataOutputStream, InputStream};
use crate::r#impl::icu_binary::ICUBinary;

/// `ValueWidth` (package-private): 16- or 32-bit data values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ValueWidth {
    /// `BITS_16`: [`Trie2_16`](crate::r#impl::trie2_16::Trie2_16).
    Bits16,
    /// `BITS_32`: [`Trie2_32`](crate::r#impl::trie2_32::Trie2_32).
    Bits32,
}

/// `UTrie2Header` (package-private): the trie header as it is serialized.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct UTrie2Header {
    /// "Tri2" in big-endian US-ASCII (0x54726932)
    pub(crate) signature: i32,
    /// options bit field (16 bits): 15.. 4 reserved (0), 3..0 UTrie2ValueBits
    pub(crate) options: i32,
    /// UTRIE2_INDEX_1_OFFSET..UTRIE2_MAX_INDEX_LENGTH
    pub(crate) index_length: i32,
    /// (UTRIE2_DATA_START_OFFSET..UTRIE2_MAX_DATA_LENGTH)>>UTRIE2_INDEX_SHIFT
    pub(crate) shifted_data_length: i32,
    /// Null index and data blocks, not shifted.
    pub(crate) index2_null_offset: i32,
    /// Null index and data blocks, not shifted.
    pub(crate) data_null_offset: i32,
    /// U+10000..highStart-1 are stored in the trie; highStart>>UTRIE2_SHIFT_1.
    pub(crate) shifted_high_start: i32,
}

/// A range of code points with one trie value, as the trie iterators
/// return them.
///
/// Equivalent to `com.ibm.icu.impl.Trie2.Range`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Range {
    /// `startCodePoint`.
    pub start_code_point: i32,
    /// `endCodePoint`: inclusive.
    pub end_code_point: i32,
    /// `value`.
    pub value: i32,
    /// `leadSurrogate`: whether the range is of lead-surrogate code units
    /// (values of `getFromU16SingleLead`).
    pub lead_surrogate: bool,
}

impl Range {
    /// `hashCode()`: the FNV-like hash of the four fields.
    pub fn hash_code(&self) -> i32 {
        let mut h = init_hash();
        h = hash_u_char32(h, self.start_code_point);
        h = hash_u_char32(h, self.end_code_point);
        h = hash_int(h, self.value);
        h = hash_byte(h, if self.lead_surrogate { 1 } else { 0 });
        h
    }
}

/// `ValueMapper`: maps trie values while iterating, so that ranges of
/// different values with the same mapped value merge.
///
/// Equivalent to the interface `com.ibm.icu.impl.Trie2.ValueMapper`.
pub type ValueMapper<'m> = &'m dyn Fn(i32) -> i32;

/// The identity mapper (`defaultValueMapper`).
fn default_value_mapper(v: i32) -> i32 {
    v
}

/// Results of a [`CharSequenceIterator`] step.
///
/// Equivalent to `com.ibm.icu.impl.Trie2.CharSequenceValues`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CharSequenceValues {
    /// `index`: the UTF-16 index of the code point.
    pub index: i32,
    /// `codePoint`.
    pub code_point: i32,
    /// `value`: the trie value of the code point.
    pub value: i32,
}

/// A read-only trie of 16- or 32-bit values for all code points, in the
/// serialized format of ICU's `UTrie2`.
///
/// Equivalent to the abstract class `com.ibm.icu.impl.Trie2`. Its two
/// subclasses `Trie2_16` and `Trie2_32` are the data widths of this one
/// struct ([`get`](Self::get), [`get_from_u16_single_lead`](Self::get_from_u16_single_lead)
/// and `rangeEnd` dispatch on the width, as Java's overrides do) and the
/// typed wrappers [`Trie2_16`](crate::r#impl::trie2_16::Trie2_16) and
/// [`Trie2_32`](crate::r#impl::trie2_32::Trie2_32). The builder subclass
/// `Trie2Writable` is outside the scope of this crate.
///
/// **Divergence.** Java's range iterators return one mutable `Range` object
/// and its `CharSequenceIterator` one mutable `CharSequenceValues` object,
/// overwritten on each step; here each step returns a copy, so that a
/// caller who keeps an earlier result keeps its values (Rust forbids
/// handing out a reference the iterator then mutates).
#[derive(Debug)]
pub struct Trie2 {
    pub(crate) width: ValueWidth,
    pub(crate) header: UTrie2Header,
    /// Index array. Includes data for 16 bit Tries.
    pub(crate) index: Vec<u16>,
    /// Offset to data portion of the index array, if 16 bit data. Zero if 32 bit data.
    pub(crate) data16: i32,
    /// NULL if 16b data is used via index.
    pub(crate) data32: Option<Vec<i32>>,
    #[allow(dead_code)] // set as in Java; read by the builder subclass `Trie2Writable`
    pub(crate) index_length: i32,
    pub(crate) data_length: i32,
    /// 0xffff if there is no dedicated index-2 null block.
    pub(crate) index2_null_offset: i32,
    pub(crate) initial_value: i32,
    /// Value returned for out-of-range code points and illegal UTF-8.
    pub(crate) error_value: i32,
    /// Start of the last range which ends at U+10ffff, and its value.
    pub(crate) high_start: i32,
    pub(crate) high_value_index: i32,
    /// 0xfffff if there is no dedicated data null block.
    pub(crate) data_null_offset: i32,
    /// `fHash`: the cached hash code, 0 until computed.
    pub(crate) f_hash: AtomicI32,
}

/// `UTRIE2_OPTIONS_VALUE_BITS_MASK`.
pub const UTRIE2_OPTIONS_VALUE_BITS_MASK: i32 = 0x000f;
/// `UTRIE2_SHIFT_1`: shift size for getting the index-1 table offset.
pub const UTRIE2_SHIFT_1: i32 = 6 + 5;
/// `UTRIE2_SHIFT_2`: shift size for getting the index-2 table offset.
pub const UTRIE2_SHIFT_2: i32 = 5;
/// `UTRIE2_SHIFT_1_2`.
pub const UTRIE2_SHIFT_1_2: i32 = UTRIE2_SHIFT_1 - UTRIE2_SHIFT_2;
/// `UTRIE2_OMITTED_BMP_INDEX_1_LENGTH`.
pub const UTRIE2_OMITTED_BMP_INDEX_1_LENGTH: i32 = 0x10000 >> UTRIE2_SHIFT_1;
/// `UTRIE2_CP_PER_INDEX_1_ENTRY`.
pub const UTRIE2_CP_PER_INDEX_1_ENTRY: i32 = 1 << UTRIE2_SHIFT_1;
/// `UTRIE2_INDEX_2_BLOCK_LENGTH`.
pub const UTRIE2_INDEX_2_BLOCK_LENGTH: i32 = 1 << UTRIE2_SHIFT_1_2;
/// `UTRIE2_INDEX_2_MASK`.
pub const UTRIE2_INDEX_2_MASK: i32 = UTRIE2_INDEX_2_BLOCK_LENGTH - 1;
/// `UTRIE2_DATA_BLOCK_LENGTH`.
pub const UTRIE2_DATA_BLOCK_LENGTH: i32 = 1 << UTRIE2_SHIFT_2;
/// `UTRIE2_DATA_MASK`.
pub const UTRIE2_DATA_MASK: i32 = UTRIE2_DATA_BLOCK_LENGTH - 1;
/// `UTRIE2_INDEX_SHIFT`.
pub const UTRIE2_INDEX_SHIFT: i32 = 2;
/// `UTRIE2_DATA_GRANULARITY`.
pub const UTRIE2_DATA_GRANULARITY: i32 = 1 << UTRIE2_INDEX_SHIFT;
/// `UTRIE2_INDEX_2_OFFSET`.
pub const UTRIE2_INDEX_2_OFFSET: i32 = 0;
/// `UTRIE2_LSCP_INDEX_2_OFFSET`.
pub const UTRIE2_LSCP_INDEX_2_OFFSET: i32 = 0x10000 >> UTRIE2_SHIFT_2;
/// `UTRIE2_LSCP_INDEX_2_LENGTH`.
pub const UTRIE2_LSCP_INDEX_2_LENGTH: i32 = 0x400 >> UTRIE2_SHIFT_2;
/// `UTRIE2_INDEX_2_BMP_LENGTH`.
pub const UTRIE2_INDEX_2_BMP_LENGTH: i32 = UTRIE2_LSCP_INDEX_2_OFFSET + UTRIE2_LSCP_INDEX_2_LENGTH;
/// `UTRIE2_UTF8_2B_INDEX_2_OFFSET`.
pub const UTRIE2_UTF8_2B_INDEX_2_OFFSET: i32 = UTRIE2_INDEX_2_BMP_LENGTH;
/// `UTRIE2_UTF8_2B_INDEX_2_LENGTH`.
pub const UTRIE2_UTF8_2B_INDEX_2_LENGTH: i32 = 0x800 >> 6;
/// `UTRIE2_INDEX_1_OFFSET`.
pub const UTRIE2_INDEX_1_OFFSET: i32 = UTRIE2_UTF8_2B_INDEX_2_OFFSET + UTRIE2_UTF8_2B_INDEX_2_LENGTH;
/// `UTRIE2_MAX_INDEX_1_LENGTH`.
pub const UTRIE2_MAX_INDEX_1_LENGTH: i32 = 0x100000 >> UTRIE2_SHIFT_1;
/// `UTRIE2_BAD_UTF8_DATA_OFFSET`.
pub const UTRIE2_BAD_UTF8_DATA_OFFSET: i32 = 0x80;
/// `UTRIE2_DATA_START_OFFSET`.
pub const UTRIE2_DATA_START_OFFSET: i32 = 0xc0;
/// `UNEWTRIE2_INDEX_GAP_OFFSET`.
pub const UNEWTRIE2_INDEX_GAP_OFFSET: i32 = UTRIE2_INDEX_2_BMP_LENGTH;
/// `UNEWTRIE2_INDEX_GAP_LENGTH`.
pub const UNEWTRIE2_INDEX_GAP_LENGTH: i32 =
    ((UTRIE2_UTF8_2B_INDEX_2_LENGTH + UTRIE2_MAX_INDEX_1_LENGTH) + UTRIE2_INDEX_2_MASK) & !UTRIE2_INDEX_2_MASK;
/// `UNEWTRIE2_MAX_INDEX_2_LENGTH`.
pub const UNEWTRIE2_MAX_INDEX_2_LENGTH: i32 = (0x110000 >> UTRIE2_SHIFT_2)
    + UTRIE2_LSCP_INDEX_2_LENGTH
    + UNEWTRIE2_INDEX_GAP_LENGTH
    + UTRIE2_INDEX_2_BLOCK_LENGTH;
/// `UNEWTRIE2_INDEX_1_LENGTH`.
pub const UNEWTRIE2_INDEX_1_LENGTH: i32 = 0x110000 >> UTRIE2_SHIFT_1;
/// `UNEWTRIE2_MAX_DATA_LENGTH`.
pub const UNEWTRIE2_MAX_DATA_LENGTH: i32 = 0x110000 + 0x40 + 0x40 + 0x400;

impl Trie2 {
    /// `createFromSerialized(ByteBuffer bytes)`: a `Trie2_16` or `Trie2_32`
    /// from its serialized form at the buffer's position, which moves past
    /// it; the buffer's byte order is restored.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("Buffer does not contain a serialized
    /// UTrie2")` or `("UTrie2 serialized format error.")`;
    /// `BufferUnderflowException` for truncated data.
    pub fn create_from_serialized(bytes: &mut ByteBuffer) -> Result<Trie2> {
        //    From ICU4C utrie2_impl.h
        //    * Trie2 data structure in serialized form:
        //    *
        //    * UTrie2Header header;
        //    * uint16_t index[header.index2Length];
        //    * uint16_t data[header.shiftedDataLength<<2];  -- or uint32_t data[...]
        //    * @internal
        //    */
        //    typedef struct UTrie2Header {
        //        /** "Tri2" in big-endian US-ASCII (0x54726932) */
        //        uint32_t signature;

        //       /**
        //         * options bit field:
        //         * 15.. 4   reserved (0)
        //         *  3.. 0   UTrie2ValueBits valueBits
        //         */
        //        uint16_t options;
        //
        //        /** UTRIE2_INDEX_1_OFFSET..UTRIE2_MAX_INDEX_LENGTH */
        //        uint16_t indexLength;
        //
        //        /** (UTRIE2_DATA_START_OFFSET..UTRIE2_MAX_DATA_LENGTH)>>UTRIE2_INDEX_SHIFT */
        //        uint16_t shiftedDataLength;
        //
        //        /** Null index and data blocks, not shifted. */
        //        uint16_t index2NullOffset, dataNullOffset;
        //
        //        /**
        //         * First code point of the single-value range ending with U+10ffff,
        //         * rounded up and then shifted right by UTRIE2_SHIFT_1.
        //         */
        //        uint16_t shiftedHighStart;
        //    } UTrie2Header;

        let outer_byte_order = bytes.order();
        let result = Self::create_from_serialized_inner(bytes, outer_byte_order);
        bytes.set_order(outer_byte_order);
        result
    }

    fn create_from_serialized_inner(bytes: &mut ByteBuffer, outer_byte_order: ByteOrder) -> Result<Trie2> {
        let mut header = UTrie2Header::default();

        /* check the signature */
        header.signature = bytes.get_int()?;
        match header.signature {
            0x54726932 => {
                // The buffer is already set to the trie data byte order.
            }
            0x32697254 => {
                // Temporarily reverse the byte order.
                let is_big_endian = outer_byte_order == ByteOrder::BigEndian;
                bytes.set_order(if is_big_endian {
                    ByteOrder::LittleEndian
                } else {
                    ByteOrder::BigEndian
                });
                header.signature = 0x54726932;
            }
            _ => {
                return Err(LuceneError::illegal_argument(
                    "Buffer does not contain a serialized UTrie2",
                ));
            }
        }

        header.options = bytes.get_char()? as i32;
        header.index_length = bytes.get_char()? as i32;
        header.shifted_data_length = bytes.get_char()? as i32;
        header.index2_null_offset = bytes.get_char()? as i32;
        header.data_null_offset = bytes.get_char()? as i32;
        header.shifted_high_start = bytes.get_char()? as i32;

        // Trie2 data width - 0: 16 bits
        //                    1: 32 bits
        if (header.options & UTRIE2_OPTIONS_VALUE_BITS_MASK) > 1 {
            return Err(LuceneError::illegal_argument("UTrie2 serialized format error."));
        }
        let width = if (header.options & UTRIE2_OPTIONS_VALUE_BITS_MASK) == 0 {
            ValueWidth::Bits16
        } else {
            ValueWidth::Bits32
        };

        /* get the length values and offsets */
        let index_length = header.index_length;
        let data_length = header.shifted_data_length << UTRIE2_INDEX_SHIFT;
        let index2_null_offset = header.index2_null_offset;
        let data_null_offset = header.data_null_offset;
        let high_start = header.shifted_high_start << UTRIE2_SHIFT_1;
        let mut high_value_index = data_length - UTRIE2_DATA_GRANULARITY;
        if width == ValueWidth::Bits16 {
            high_value_index += index_length;
        }

        // Allocate the Trie2 index array. If the data width is 16 bits, the array also
        // includes the space for the data.

        let mut index_array_size = index_length;
        if width == ValueWidth::Bits16 {
            index_array_size += data_length;
        }

        /* Read in the index */
        let index = ICUBinary::get_chars(bytes, index_array_size, 0)?;

        /* Read in the data. 16 bit data goes in the same array as the index.
         * 32 bit data goes in its own separate data array.
         */
        let (data16, data32) = if width == ValueWidth::Bits16 {
            (index_length, None)
        } else {
            (0, Some(ICUBinary::get_ints(bytes, data_length, 0)?))
        };

        let (initial_value, error_value) = match width {
            ValueWidth::Bits16 => (
                index[data_null_offset as usize] as i32,
                index[(data16 + UTRIE2_BAD_UTF8_DATA_OFFSET) as usize] as i32,
            ),
            ValueWidth::Bits32 => {
                let d = data32.as_ref().expect("32-bit data was read");
                (d[data_null_offset as usize], d[UTRIE2_BAD_UTF8_DATA_OFFSET as usize])
            }
        };

        Ok(Trie2 {
            width,
            header,
            index,
            data16,
            data32,
            index_length,
            data_length,
            index2_null_offset,
            initial_value,
            error_value,
            high_start,
            high_value_index,
            data_null_offset,
            f_hash: AtomicI32::new(0),
        })
    }

    /// `getVersion(InputStream is, boolean littleEndianOk)`: 1 for a
    /// serialized `UTrie` (version 1), 2 for a `UTrie2`, 0 otherwise; the
    /// stream is reset to its start.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("Input stream must support mark().")`;
    /// the `IOException` of the stream.
    pub fn get_version(is: &mut dyn InputStream, little_endian_ok: bool) -> Result<i32> {
        if !is.mark_supported() {
            return Err(LuceneError::illegal_argument("Input stream must support mark()."));
        }
        is.mark(4);
        let mut sig = [0u8; 4];
        let read = is.read(&mut sig, 0, 4)?;
        is.reset()?;

        if read != sig.len() as i32 {
            return Ok(0);
        }

        if &sig == b"Trie" {
            return Ok(1);
        }
        if &sig == b"Tri2" {
            return Ok(2);
        }
        if little_endian_ok {
            if &sig == b"eirT" {
                return Ok(1);
            }
            if &sig == b"2irT" {
                return Ok(2);
            }
        }
        Ok(0)
    }

    /// `get(int codePoint)`: the value for a code point; the error value
    /// outside `0..=0x10ffff`. A lead surrogate code point gets the value
    /// of the code point, not of the code unit.
    pub fn get(&self, code_point: i32) -> i32 {
        match self.width {
            ValueWidth::Bits16 => self.get16(code_point),
            ValueWidth::Bits32 => self.get32(code_point),
        }
    }

    /// `getFromU16SingleLead(char c)`: the value for a UTF-16 code unit; a
    /// lead surrogate code unit gets its separate lead-unit value.
    pub fn get_from_u16_single_lead(&self, c: u16) -> i32 {
        match self.width {
            ValueWidth::Bits16 => self.get_from_u16_single_lead16(c),
            ValueWidth::Bits32 => self.get_from_u16_single_lead32(c),
        }
    }

    /// `equals(Object other)` (final): the same ranges, error value and
    /// initial value.
    pub fn equals(&self, other: &Trie2) -> bool {
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

    /// `hashCode()`: the hash of the ranges, computed once.
    pub fn hash_code(&self) -> i32 {
        let cached = self.f_hash.load(Ordering::Relaxed);
        if cached == 0 {
            let mut hash = init_hash();
            for r in self.iterator() {
                hash = hash_int(hash, r.hash_code());
            }
            if hash == 0 {
                hash = 1;
            }
            self.f_hash.store(hash, Ordering::Relaxed);
            return hash;
        }
        cached
    }

    /// `iterator()`: the ranges of equal values over all code points, then
    /// over the lead surrogate code units.
    pub fn iterator(&self) -> Trie2Iterator<'_, 'static> {
        self.iterator_with(&default_value_mapper)
    }

    /// `iterator(ValueMapper mapper)`: as [`iterator`](Self::iterator), the
    /// values mapped first.
    pub fn iterator_with<'m>(&self, mapper: ValueMapper<'m>) -> Trie2Iterator<'_, 'm> {
        Trie2Iterator::new(self, mapper)
    }

    /// `iteratorForLeadSurrogate(char lead, ValueMapper mapper)`: the ranges
    /// of the code points with the lead surrogate `lead`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("Bad lead surrogate value.")`.
    pub fn iterator_for_lead_surrogate_with<'m>(&self, lead: u16, mapper: ValueMapper<'m>) -> Result<Trie2Iterator<'_, 'm>> {
        Trie2Iterator::for_lead_surrogate(self, lead, mapper)
    }

    /// `iteratorForLeadSurrogate(char lead)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("Bad lead surrogate value.")`.
    pub fn iterator_for_lead_surrogate(&self, lead: u16) -> Result<Trie2Iterator<'_, 'static>> {
        Trie2Iterator::for_lead_surrogate(self, lead, &default_value_mapper)
    }

    /// `serializeHeader(DataOutputStream dos)` (protected): writes the
    /// header and the index; returns the byte count ICU computes (16 plus
    /// the index length in units).
    ///
    /// # Errors
    ///
    /// The `IOException` of the stream.
    pub(crate) fn serialize_header(&self, dos: &mut DataOutputStream<'_>) -> Result<i32> {
        // Write the header.  It is already set and ready to use, having been
        //  created when the Trie2 was unserialized or when it was frozen.
        let mut bytes_written = 0;

        dos.write_int(self.header.signature)?;
        dos.write_short(self.header.options)?;
        dos.write_short(self.header.index_length)?;
        dos.write_short(self.header.shifted_data_length)?;
        dos.write_short(self.header.index2_null_offset)?;
        dos.write_short(self.header.data_null_offset)?;
        dos.write_short(self.header.shifted_high_start)?;
        bytes_written += 16;

        // Write the index
        for i in 0..self.header.index_length {
            dos.write_char(self.index[i as usize] as i32)?;
        }
        bytes_written += self.header.index_length;
        Ok(bytes_written)
    }

    /// `charSequenceIterator(CharSequence text, int index)`: an iterator over
    /// the code points of `text` from `index`, with their trie values.
    ///
    /// # Panics
    ///
    /// `IndexOutOfBoundsException` when `index` is outside `0..=length`.
    pub fn char_sequence_iterator<'t>(&'t self, text: &'t [u16], index: i32) -> CharSequenceIterator<'t> {
        CharSequenceIterator::new(self, text, index)
    }

    /// `rangeEnd(int start, int limitp, int val)` (package-private): the
    /// last code point of the range of `val` from `start`. Dispatches to the
    /// overrides of `Trie2_16` and `Trie2_32`.
    pub(crate) fn range_end(&self, start: i32, limitp: i32, val: i32) -> i32 {
        match self.width {
            ValueWidth::Bits16 => self.range_end16(start, limitp, val),
            ValueWidth::Bits32 => self.range_end32(start, limitp, val),
        }
    }

    /// `Trie2.rangeEnd` itself, the implementation of the base class, which
    /// both subclasses of this crate override.
    #[allow(dead_code)]
    pub(crate) fn range_end_default(&self, start: i32, limitp: i32, val: i32) -> i32 {
        let mut c = start + 1;
        let limit = self.high_start.min(limitp);
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
}

/// An iterator over the ranges of equal (mapped) values of a [`Trie2`]:
/// the code points, then (for a whole-trie iterator) the lead surrogate
/// code units.
///
/// Equivalent to `Trie2.Trie2Iterator` (package-private inner class).
pub struct Trie2Iterator<'a, 'm> {
    trie: &'a Trie2,
    /// The mapping function to be applied to values as they are returned.
    mapper: ValueMapper<'m>,
    /// The first code point for the next range to be returned.
    next_start: i32,
    /// The upper limit for the last normal range to be returned.  Normally 0x110000, but
    /// may be lower when iterating over the code points for a single lead surrogate.
    limit_cp: i32,
    /// True while iterating over the Trie2 values for code points.
    /// False while iterating over the alternate values for lead surrogates.
    doing_code_points: bool,
    /// True if the iterator should iterate the special values for lead surrogates in
    /// addition to the normal values for code points.
    do_lead_surrogates: bool,
}

impl<'a, 'm> Trie2Iterator<'a, 'm> {
    /// `Trie2Iterator(ValueMapper vm)`: the whole trie.
    fn new(trie: &'a Trie2, vm: ValueMapper<'m>) -> Self {
        Self {
            trie,
            mapper: vm,
            next_start: 0,
            limit_cp: 0x110000,
            doing_code_points: true,
            do_lead_surrogates: true,
        }
    }

    /// `Trie2Iterator(char leadSurrogate, ValueMapper vm)`: the code points
    /// of one lead surrogate.
    fn for_lead_surrogate(trie: &'a Trie2, lead_surrogate: u16, vm: ValueMapper<'m>) -> Result<Self> {
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
            do_lead_surrogates: false, // Do not iterate over lead the special lead surrogate
                                       //   values after completing iteration over code points.
        })
    }

    /// `hasNext()`.
    pub fn has_next(&self) -> bool {
        (self.doing_code_points && (self.do_lead_surrogates || self.next_start < self.limit_cp))
            || self.next_start < 0xdc00
    }

    /// `remove()`: unsupported.
    ///
    /// # Errors
    ///
    /// Always `UnsupportedOperationException`.
    pub fn remove(&mut self) -> Result<()> {
        Err(crate::jdk::unchecked_no_message("java.lang.UnsupportedOperationException"))
    }

    /// `rangeEndLS(char startingLS)` (private): the last lead surrogate of
    /// the range of equal lead-unit values from `startingLS`.
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

impl Iterator for Trie2Iterator<'_, '_> {
    type Item = Range;

    /// `next()`: the next range; `None` where Java throws
    /// `NoSuchElementException`.
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
                val = self.trie.get_from_u16_single_lead((end_of_range + 1) as u16);
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

/// An iterator over the code points of a text with their trie values,
/// forwards and backwards.
///
/// Equivalent to `com.ibm.icu.impl.Trie2.CharSequenceIterator` (an inner
/// class of the trie).
pub struct CharSequenceIterator<'t> {
    trie: &'t Trie2,
    text: &'t [u16],
    text_length: i32,
    index: i32,
}

impl<'t> CharSequenceIterator<'t> {
    /// `CharSequenceIterator(CharSequence t, int index)`.
    fn new(trie: &'t Trie2, t: &'t [u16], index: i32) -> Self {
        let mut it = Self {
            trie,
            text: t,
            text_length: t.len() as i32,
            index: 0,
        };
        it.set(index);
        it
    }

    /// `set(int i)`: moves to index `i`.
    ///
    /// # Panics
    ///
    /// `IndexOutOfBoundsException` when `i` is outside `0..=length`.
    pub fn set(&mut self, i: i32) {
        if i < 0 || i > self.text_length {
            crate::jdk::index_out_of_bounds(None);
        }
        self.index = i;
    }

    /// `hasNext()` (final).
    pub fn has_next(&self) -> bool {
        self.index < self.text_length
    }

    /// `hasPrevious()` (final).
    pub fn has_previous(&self) -> bool {
        self.index > 0
    }

    /// `next()`: the code point at the index and its value; the index moves
    /// past it.
    ///
    /// # Panics
    ///
    /// `IndexOutOfBoundsException` at the end of the text.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> CharSequenceValues {
        let c = crate::jdk::code_point_at(self.text, self.index);
        let val = self.trie.get(c);

        let results = CharSequenceValues {
            index: self.index,
            code_point: c,
            value: val,
        };
        self.index += 1;
        if c >= 0x10000 {
            self.index += 1;
        }
        results
    }

    /// `previous()`: moves back over one code point and returns it with its
    /// value.
    ///
    /// # Panics
    ///
    /// `IndexOutOfBoundsException` at the start of the text.
    pub fn previous(&mut self) -> CharSequenceValues {
        let c = code_point_before(self.text, self.index);
        let val = self.trie.get(c);
        self.index -= 1;
        if c >= 0x10000 {
            self.index -= 1;
        }
        CharSequenceValues {
            index: self.index,
            code_point: c,
            value: val,
        }
    }

    /// `remove()`: unsupported.
    ///
    /// # Errors
    ///
    /// Always `UnsupportedOperationException("Trie2.CharSequenceIterator
    /// does not support remove().")`.
    pub fn remove(&mut self) -> Result<()> {
        Err(LuceneError::unsupported_operation(
            "Trie2.CharSequenceIterator does not support remove().",
        ))
    }
}

/// `Character.codePointBefore(CharSequence seq, int index)`.
///
/// # Panics
///
/// `IndexOutOfBoundsException` when `index` is outside `1..=length`.
fn code_point_before(seq: &[u16], index: i32) -> i32 {
    if index < 1 || index as usize > seq.len() {
        crate::jdk::index_out_of_bounds(Some(format!(
            "Index {} out of bounds for length {}",
            index - 1,
            seq.len()
        )));
    }
    let mut i = index as usize - 1;
    let c2 = seq[i];
    if crate::jdk::is_low_surrogate(c2) && i > 0 {
        i -= 1;
        let c1 = seq[i];
        if crate::jdk::is_high_surrogate(c1) {
            return crate::jdk::to_code_point(c1, c2);
        }
    }
    c2 as i32
}

/// `initHash()` (private).
fn init_hash() -> i32 {
    0x811c9DC5_u32 as i32
}

/// `hashByte(int h, int b)` (private).
fn hash_byte(mut h: i32, b: i32) -> i32 {
    h = h.wrapping_mul(16777619);
    h ^= b;
    h
}

/// `hashUChar32(int h, int c)` (private).
fn hash_u_char32(mut h: i32, c: i32) -> i32 {
    h = hash_byte(h, c & 255);
    h = hash_byte(h, (c >> 8) & 255);
    h = hash_byte(h, c >> 16);
    h
}

/// `hashInt(int h, int i)` (private).
fn hash_int(mut h: i32, i: i32) -> i32 {
    h = hash_byte(h, i & 255);
    h = hash_byte(h, (i >> 8) & 255);
    h = hash_byte(h, (i >> 16) & 255);
    h = hash_byte(h, (i >> 24) & 255);
    h
}
