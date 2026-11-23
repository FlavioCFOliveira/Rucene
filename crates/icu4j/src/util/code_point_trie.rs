// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.util.CodePointTrie`.

use std::ops::Deref;

use rucene::error::{LuceneError, Result};

use crate::jdk::{ByteBuffer, ByteOrder, DataOutputStream, OutputStream};
use crate::r#impl::icu_binary::ICUBinary;
use crate::r#impl::normalizer2_impl::UTF16Plus;
use crate::util::code_point_map::{CodePointMap, Range, RangeIterator, StringIterator, StringIteratorKind, ValueFilter};
use crate::util::icu_unchecked_io_exception::ICUUncheckedIOException;

/// Selectors for the type of a `CodePointTrie`.
///
/// Equivalent to the enum `com.ibm.icu.util.CodePointTrie.Type`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Type {
    /// `FAST`: fast/simple/larger BMP data structure.
    Fast,
    /// `SMALL`: small/slower BMP data structure.
    Small,
}

impl Type {
    /// `ordinal()`.
    pub fn ordinal(self) -> i32 {
        match self {
            Type::Fast => 0,
            Type::Small => 1,
        }
    }
}

/// Selectors for the number of bits in a `CodePointTrie` data value.
///
/// Equivalent to the enum `com.ibm.icu.util.CodePointTrie.ValueWidth`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueWidth {
    /// `BITS_16`.
    Bits16,
    /// `BITS_32`.
    Bits32,
    /// `BITS_8`.
    Bits8,
}

impl ValueWidth {
    /// `ordinal()`.
    pub fn ordinal(self) -> i32 {
        match self {
            ValueWidth::Bits16 => 0,
            ValueWidth::Bits32 => 1,
            ValueWidth::Bits8 => 2,
        }
    }
}

/// The data array of a trie.
///
/// Equivalent to `CodePointTrie.Data` (private abstract class) with its
/// subclasses `Data16`, `Data32` and `Data8`.
#[derive(Debug, Clone)]
pub(crate) enum Data {
    /// `Data16`.
    Data16(Vec<u16>),
    /// `Data32`.
    Data32(Vec<i32>),
    /// `Data8`.
    Data8(Vec<i8>),
}

impl Data {
    /// `getValueWidth()`.
    fn get_value_width(&self) -> ValueWidth {
        match self {
            Data::Data16(_) => ValueWidth::Bits16,
            Data::Data32(_) => ValueWidth::Bits32,
            Data::Data8(_) => ValueWidth::Bits8,
        }
    }

    /// `getDataLength()`.
    fn get_data_length(&self) -> i32 {
        match self {
            Data::Data16(a) => a.len() as i32,
            Data::Data32(a) => a.len() as i32,
            Data::Data8(a) => a.len() as i32,
        }
    }

    /// `getFromIndex(int index)`.
    #[inline]
    pub(crate) fn get_from_index(&self, index: i32) -> i32 {
        match self {
            Data::Data16(a) => a[index as usize] as i32,
            Data::Data32(a) => a[index as usize],
            Data::Data8(a) => a[index as usize] as i32 & 0xff,
        }
    }

    /// `write(DataOutputStream dos)`.
    fn write(&self, dos: &mut DataOutputStream<'_>) -> Result<i32> {
        match self {
            Data::Data16(a) => {
                for &v in a {
                    dos.write_char(v as i32)?;
                }
                Ok(a.len() as i32 * 2)
            }
            Data::Data32(a) => {
                for &v in a {
                    dos.write_int(v)?;
                }
                Ok(a.len() as i32 * 4)
            }
            Data::Data8(a) => {
                for &v in a {
                    dos.write_byte(v as i32)?;
                }
                Ok(a.len() as i32)
            }
        }
    }
}

/// `MAX_UNICODE`.
const MAX_UNICODE: i32 = 0x10ffff;
/// `ASCII_LIMIT`.
const ASCII_LIMIT: i32 = 0x80;
/// `FAST_SHIFT` (package-private).
pub const FAST_SHIFT: i32 = 6;
/// `FAST_DATA_BLOCK_LENGTH` (package-private).
pub const FAST_DATA_BLOCK_LENGTH: i32 = 1 << FAST_SHIFT;
/// `FAST_DATA_MASK`.
const FAST_DATA_MASK: i32 = FAST_DATA_BLOCK_LENGTH - 1;
/// `SMALL_MAX`.
const SMALL_MAX: i32 = 0xfff;
/// `ERROR_VALUE_NEG_DATA_OFFSET`.
const ERROR_VALUE_NEG_DATA_OFFSET: i32 = 1;
/// `HIGH_VALUE_NEG_DATA_OFFSET`.
const HIGH_VALUE_NEG_DATA_OFFSET: i32 = 2;
/// `BMP_INDEX_LENGTH`.
const BMP_INDEX_LENGTH: i32 = 0x10000 >> FAST_SHIFT;
/// `SMALL_LIMIT` (package-private).
pub const SMALL_LIMIT: i32 = 0x1000;
/// `SMALL_INDEX_LENGTH`.
const SMALL_INDEX_LENGTH: i32 = SMALL_LIMIT >> FAST_SHIFT;
/// `SHIFT_3` (package-private).
pub const SHIFT_3: i32 = 4;
/// `SHIFT_2`.
const SHIFT_2: i32 = 5 + SHIFT_3;
/// `SHIFT_1`.
const SHIFT_1: i32 = 5 + SHIFT_2;
/// `SHIFT_2_3` (package-private).
pub const SHIFT_2_3: i32 = SHIFT_2 - SHIFT_3;
/// `SHIFT_1_2` (package-private).
pub const SHIFT_1_2: i32 = SHIFT_1 - SHIFT_2;
/// `OMITTED_BMP_INDEX_1_LENGTH`.
const OMITTED_BMP_INDEX_1_LENGTH: i32 = 0x10000 >> SHIFT_1;
/// `INDEX_2_BLOCK_LENGTH` (package-private).
pub const INDEX_2_BLOCK_LENGTH: i32 = 1 << SHIFT_1_2;
/// `INDEX_2_MASK` (package-private).
pub const INDEX_2_MASK: i32 = INDEX_2_BLOCK_LENGTH - 1;
/// `CP_PER_INDEX_2_ENTRY` (package-private).
pub const CP_PER_INDEX_2_ENTRY: i32 = 1 << SHIFT_2;
/// `INDEX_3_BLOCK_LENGTH` (package-private).
pub const INDEX_3_BLOCK_LENGTH: i32 = 1 << SHIFT_2_3;
/// `INDEX_3_MASK`.
const INDEX_3_MASK: i32 = INDEX_3_BLOCK_LENGTH - 1;
/// `SMALL_DATA_BLOCK_LENGTH` (package-private).
pub const SMALL_DATA_BLOCK_LENGTH: i32 = 1 << SHIFT_3;
/// `SMALL_DATA_MASK` (package-private).
pub const SMALL_DATA_MASK: i32 = SMALL_DATA_BLOCK_LENGTH - 1;
/// `OPTIONS_DATA_LENGTH_MASK`.
const OPTIONS_DATA_LENGTH_MASK: i32 = 0xf000;
/// `OPTIONS_DATA_NULL_OFFSET_MASK`.
const OPTIONS_DATA_NULL_OFFSET_MASK: i32 = 0xf00;
/// `OPTIONS_RESERVED_MASK`.
const OPTIONS_RESERVED_MASK: i32 = 0x38;
/// `OPTIONS_VALUE_BITS_MASK`.
const OPTIONS_VALUE_BITS_MASK: i32 = 7;
/// `NO_INDEX3_NULL_OFFSET` (package-private).
pub const NO_INDEX3_NULL_OFFSET: i32 = 0x7fff;
/// `NO_DATA_NULL_OFFSET` (package-private).
pub const NO_DATA_NULL_OFFSET: i32 = 0xfffff;

/// Immutable Unicode code point trie: fast lookup of a 16-, 32- or 8-bit
/// value for each code point.
///
/// Equivalent to the abstract class `com.ibm.icu.util.CodePointTrie`. Its
/// subclass tree (`Fast`, `Small`, and `Fast16`, `Fast32`, `Fast8`,
/// `Small16`, `Small32`, `Small8`) is this one struct, whose
/// [`get_type`](Self::get_type) and value width select the behaviour of
/// the overrides (`cpIndex`, `stringIterator`), plus typed wrappers that
/// dereference to it ([`Fast`], [`Small`], [`Fast16`], ...) and give the
/// subclass-only methods (`bmpGet`, `suppGet`) and the typed `fromBinary`
/// factories.
#[derive(Debug, Clone)]
pub struct CodePointTrie {
    type_: Type,
    ascii: Vec<i32>,
    index: Vec<u16>,
    /// `data` (deprecated, protected).
    pub(crate) data: Data,
    /// `dataLength` (deprecated, protected).
    pub(crate) data_length: i32,
    /// Start of the last range which ends at U+10FFFF (deprecated, protected).
    pub(crate) high_start: i32,
    /// Internal index-3 null block offset.
    index3_null_offset: i32,
    /// Internal data null block offset, not shifted.
    data_null_offset: i32,
    null_value: i32,
}

impl CodePointTrie {
    /// `CodePointTrie(char[] index, Data data, int highStart, int
    /// index3NullOffset, int dataNullOffset)` (private).
    fn new(type_: Type, index: Vec<u16>, data: Data, high_start: i32, index3_null_offset: i32, data_null_offset: i32) -> Self {
        let data_length = data.get_data_length();
        let mut ascii = vec![0i32; ASCII_LIMIT as usize];
        for (c, a) in ascii.iter_mut().enumerate() {
            *a = data.get_from_index(c as i32);
        }

        let mut null_value_offset = data_null_offset;
        if null_value_offset >= data_length {
            null_value_offset = data_length - HIGH_VALUE_NEG_DATA_OFFSET;
        }
        let null_value = data.get_from_index(null_value_offset);
        Self {
            type_,
            ascii,
            index,
            data,
            data_length,
            high_start,
            index3_null_offset,
            data_null_offset,
            null_value,
        }
    }

    /// `fromBinary(Type type, ValueWidth valueWidth, ByteBuffer bytes)`:
    /// a trie from its serialized form at the buffer's position, which moves
    /// past it; `None` for `type` or `value_width` accepts what the data
    /// says. The buffer's byte order is restored.
    ///
    /// # Errors
    ///
    /// `ICUUncheckedIOException` for a short buffer, a bad signature, an
    /// unsupported or unexpected type, width or options;
    /// `BufferUnderflowException` from the buffer.
    pub fn from_binary(type_: Option<Type>, value_width: Option<ValueWidth>, bytes: &mut ByteBuffer) -> Result<CodePointTrie> {
        let outer_byte_order = bytes.order();
        let result = Self::from_binary_inner(type_, value_width, bytes, outer_byte_order);
        bytes.set_order(outer_byte_order);
        result
    }

    fn from_binary_inner(
        type_: Option<Type>,
        value_width: Option<ValueWidth>,
        bytes: &mut ByteBuffer,
        outer_byte_order: ByteOrder,
    ) -> Result<CodePointTrie> {
        let io = |m: &str| ICUUncheckedIOException::with_message(m).error();
        // Enough data for a trie header?
        if bytes.remaining() < 16 {
            return Err(io("Buffer too short for a CodePointTrie header"));
        }

        // struct UCPTrieHeader
        /* "Tri3" in big-endian US-ASCII (0x54726933) */
        let signature = bytes.get_int()?;

        // Check the signature.
        match signature {
            0x54726933 => {
                // The buffer is already set to the trie data byte order.
            }
            0x33697254 => {
                // Temporarily reverse the byte order.
                let is_big_endian = outer_byte_order == ByteOrder::BigEndian;
                bytes.set_order(if is_big_endian {
                    ByteOrder::LittleEndian
                } else {
                    ByteOrder::BigEndian
                });
            }
            _ => return Err(io("Buffer does not contain a serialized CodePointTrie")),
        }

        // struct UCPTrieHeader continued
        /*
         * Options bit field:
         * Bits 15..12: Data length bits 19..16.
         * Bits 11..8: Data null block offset bits 19..16.
         * Bits 7..6: UCPTrieType
         * Bits 5..3: Reserved (0).
         * Bits 2..0: UCPTrieValueWidth
         */
        let options = bytes.get_char()? as i32;

        /* Total length of the index tables. */
        let index_length = bytes.get_char()? as i32;

        /* Data length bits 15..0. */
        let mut data_length = bytes.get_char()? as i32;

        /* Index-3 null block offset, 0x7fff or 0xffff if none. */
        let index3_null_offset = bytes.get_char()? as i32;

        /* Data null block offset bits 15..0, 0xfffff if none. */
        let mut data_null_offset = bytes.get_char()? as i32;

        /* First code point of the single-value range ending with U+10ffff, rounded up and then shifted right by SHIFT_2. */
        let shifted_high_start = bytes.get_char()? as i32;
        // struct UCPTrieHeader end

        let type_int = (options >> 6) & 3;
        let actual_type = match type_int {
            0 => Type::Fast,
            1 => Type::Small,
            _ => return Err(io("CodePointTrie data header has an unsupported type")),
        };

        let value_width_int = options & OPTIONS_VALUE_BITS_MASK;
        let actual_value_width = match value_width_int {
            0 => ValueWidth::Bits16,
            1 => ValueWidth::Bits32,
            2 => ValueWidth::Bits8,
            _ => return Err(io("CodePointTrie data header has an unsupported value width")),
        };

        if (options & OPTIONS_RESERVED_MASK) != 0 {
            return Err(io("CodePointTrie data header has unsupported options"));
        }

        let type_ = type_.unwrap_or(actual_type);
        let value_width = value_width.unwrap_or(actual_value_width);
        if type_ != actual_type || value_width != actual_value_width {
            return Err(io(
                "CodePointTrie data header has a different type or value width than required",
            ));
        }

        // Get the length values and offsets.
        data_length |= (options & OPTIONS_DATA_LENGTH_MASK) << 4;
        data_null_offset |= (options & OPTIONS_DATA_NULL_OFFSET_MASK) << 8;

        let high_start = shifted_high_start << SHIFT_2;

        // Calculate the actual length, minus the header.
        let mut actual_length = index_length * 2;
        if value_width == ValueWidth::Bits16 {
            actual_length += data_length * 2;
        } else if value_width == ValueWidth::Bits32 {
            actual_length += data_length * 4;
        } else {
            actual_length += data_length;
        }
        if bytes.remaining() < actual_length {
            return Err(io("Buffer too short for the CodePointTrie data"));
        }

        let index = ICUBinary::get_chars(bytes, index_length, 0)?;
        let data = match value_width {
            ValueWidth::Bits16 => Data::Data16(ICUBinary::get_chars(bytes, data_length, 0)?),
            ValueWidth::Bits32 => Data::Data32(ICUBinary::get_ints(bytes, data_length, 0)?),
            ValueWidth::Bits8 => Data::Data8(ICUBinary::get_bytes(bytes, data_length, 0)?),
        };
        Ok(CodePointTrie::new(type_, index, data, high_start, index3_null_offset, data_null_offset))
    }

    /// `getType()`.
    pub fn get_type(&self) -> Type {
        self.type_
    }

    /// `getValueWidth()` (final).
    pub fn get_value_width(&self) -> ValueWidth {
        self.data.get_value_width()
    }

    /// `asciiGet(int c)` (final): the value for an ASCII code point.
    ///
    /// # Panics
    ///
    /// `ArrayIndexOutOfBoundsException` outside `0..0x80`.
    pub fn ascii_get(&self, c: i32) -> i32 {
        self.ascii[c as usize]
    }

    /// `maybeFilterValue(int value, int trieNullValue, int nullValue,
    /// ValueFilter filter)` (private).
    fn maybe_filter_value(mut value: i32, trie_null_value: i32, null_value: i32, filter: Option<ValueFilter<'_>>) -> i32 {
        if value == trie_null_value {
            value = null_value;
        } else if let Some(f) = filter {
            value = f(value);
        }
        value
    }

    /// `toBinary(OutputStream os)` (final): writes the serialized trie and
    /// returns the number of bytes written.
    ///
    /// # Errors
    ///
    /// `ICUUncheckedIOException` wrapping the stream's `IOException`.
    pub fn to_binary(&self, os: &mut dyn OutputStream) -> Result<i32> {
        let result = (|| -> Result<i32> {
            let mut dos = DataOutputStream::new(os);

            // Write the UCPTrieHeader
            dos.write_int(0x54726933)?; // signature="Tri3"
            dos.write_char(
                ((self.data_length & 0xf0000) >> 4)
                    | ((self.data_null_offset & 0xf0000) >> 8)
                    | (self.get_type().ordinal() << 6)
                    | self.get_value_width().ordinal(),
            )?;
            dos.write_char(self.index.len() as i32)?;
            dos.write_char(self.data_length)?;
            dos.write_char(self.index3_null_offset)?;
            dos.write_char(self.data_null_offset)?;
            dos.write_char(self.high_start >> SHIFT_2)?; // shiftedHighStart
            let mut length = 16; // sizeof(UCPTrieHeader)

            for &i in &self.index {
                dos.write_char(i as i32)?;
            }
            length += self.index.len() as i32 * 2;
            length += self.data.write(&mut dos)?;
            Ok(length)
        })();
        result.map_err(|e| {
            if e.is_io_exception() {
                ICUUncheckedIOException::with_cause(e).error()
            } else {
                e
            }
        })
    }

    /// `fastIndex(int c)` (final, deprecated, protected): the data index of
    /// a BMP code point (or one below the small limit).
    pub(crate) fn fast_index(&self, c: i32) -> i32 {
        self.index[(c >> FAST_SHIFT) as usize] as i32 + (c & FAST_DATA_MASK)
    }

    /// `smallIndex(Type type, int c)` (final, deprecated, protected).
    pub(crate) fn small_index(&self, type_: Type, c: i32) -> i32 {
        // Split into two methods to make this part inline-friendly.
        // In C, this part is a macro.
        if c >= self.high_start {
            return self.data_length - HIGH_VALUE_NEG_DATA_OFFSET;
        }
        self.internal_small_index(type_, c)
    }

    /// `internalSmallIndex(Type type, int c)` (private).
    fn internal_small_index(&self, type_: Type, c: i32) -> i32 {
        let mut i1 = c >> SHIFT_1;
        if type_ == Type::Fast {
            debug_assert!(0xffff < c && c < self.high_start);
            i1 += BMP_INDEX_LENGTH - OMITTED_BMP_INDEX_1_LENGTH;
        } else {
            debug_assert!(0 <= c && c < self.high_start && self.high_start > SMALL_LIMIT);
            i1 += SMALL_INDEX_LENGTH;
        }
        let mut i3_block = self.index[(self.index[i1 as usize] as i32 + ((c >> SHIFT_2) & INDEX_2_MASK)) as usize] as i32;
        let mut i3 = (c >> SHIFT_3) & INDEX_3_MASK;
        let mut data_block;
        if (i3_block & 0x8000) == 0 {
            // 16-bit indexes
            data_block = self.index[(i3_block + i3) as usize] as i32;
        } else {
            // 18-bit indexes stored in groups of 9 entries per 8 indexes.
            i3_block = (i3_block & 0x7fff) + (i3 & !7) + (i3 >> 3);
            i3 &= 7;
            data_block = ((self.index[i3_block as usize] as i32) << (2 + (2 * i3))) & 0x30000;
            i3_block += 1;
            data_block |= self.index[(i3_block + i3) as usize] as i32;
        }
        data_block + (c & SMALL_DATA_MASK)
    }

    /// `cpIndex(int c)` (deprecated, protected): the data index for a code
    /// point, per the overrides of `Fast` and `Small`.
    pub(crate) fn cp_index(&self, c: i32) -> i32 {
        match self.type_ {
            Type::Fast => {
                if c >= 0 {
                    if c <= 0xffff {
                        return self.fast_index(c);
                    } else if c <= 0x10ffff {
                        return self.small_index(Type::Fast, c);
                    }
                }
                self.data_length - ERROR_VALUE_NEG_DATA_OFFSET
            }
            Type::Small => {
                if c >= 0 {
                    if c <= SMALL_MAX {
                        return self.fast_index(c);
                    } else if c <= 0x10ffff {
                        return self.small_index(Type::Small, c);
                    }
                }
                self.data_length - ERROR_VALUE_NEG_DATA_OFFSET
            }
        }
    }

    /// `Fast.bmpGet(int c)` (of `Fast16`, `Fast32`, `Fast8`): the value of
    /// a BMP code point of a fast trie.
    pub fn bmp_get(&self, c: i32) -> i32 {
        debug_assert!((0..=0xffff).contains(&c));
        self.data.get_from_index(self.fast_index(c))
    }

    /// `Fast.suppGet(int c)`: the value of a supplementary code point of a
    /// fast trie.
    pub fn supp_get(&self, c: i32) -> i32 {
        debug_assert!((0x10000..=0x10ffff).contains(&c));
        self.data.get_from_index(self.small_index(Type::Fast, c))
    }

    /// `FastStringIterator.next()`.
    pub(crate) fn fast_string_iterator_next(&self, it: &mut StringIterator<'_>) -> bool {
        if it.s_index >= it.s.len() as i32 {
            return false;
        }
        let lead = it.s[it.s_index as usize];
        it.s_index += 1;
        it.c = lead as i32;
        let data_index;
        if !UTF16Plus::is_surrogate(lead as i32) {
            data_index = self.fast_index(it.c);
        } else if UTF16Plus::is_surrogate_lead(lead as i32)
            && it.s_index < it.s.len() as i32
            && crate::jdk::is_low_surrogate(it.s[it.s_index as usize])
        {
            let trail = it.s[it.s_index as usize];
            it.s_index += 1;
            it.c = crate::jdk::to_code_point(lead, trail);
            data_index = self.small_index(Type::Fast, it.c);
        } else {
            data_index = self.data_length - ERROR_VALUE_NEG_DATA_OFFSET;
        }
        it.value = self.data.get_from_index(data_index);
        true
    }

    /// `FastStringIterator.previous()`.
    pub(crate) fn fast_string_iterator_previous(&self, it: &mut StringIterator<'_>) -> bool {
        if it.s_index <= 0 {
            return false;
        }
        it.s_index -= 1;
        let trail = it.s[it.s_index as usize];
        it.c = trail as i32;
        let data_index;
        if !UTF16Plus::is_surrogate(trail as i32) {
            data_index = self.fast_index(it.c);
        } else if !UTF16Plus::is_surrogate_lead(trail as i32)
            && it.s_index > 0
            && crate::jdk::is_high_surrogate(it.s[(it.s_index - 1) as usize])
        {
            let lead = it.s[(it.s_index - 1) as usize];
            it.s_index -= 1;
            it.c = crate::jdk::to_code_point(lead, trail);
            data_index = self.small_index(Type::Fast, it.c);
        } else {
            data_index = self.data_length - ERROR_VALUE_NEG_DATA_OFFSET;
        }
        it.value = self.data.get_from_index(data_index);
        true
    }

    /// `SmallStringIterator.next()`.
    pub(crate) fn small_string_iterator_next(&self, it: &mut StringIterator<'_>) -> bool {
        if it.s_index >= it.s.len() as i32 {
            return false;
        }
        let lead = it.s[it.s_index as usize];
        it.s_index += 1;
        it.c = lead as i32;
        let data_index;
        if !UTF16Plus::is_surrogate(lead as i32) {
            data_index = self.cp_index(it.c);
        } else if UTF16Plus::is_surrogate_lead(lead as i32)
            && it.s_index < it.s.len() as i32
            && crate::jdk::is_low_surrogate(it.s[it.s_index as usize])
        {
            let trail = it.s[it.s_index as usize];
            it.s_index += 1;
            it.c = crate::jdk::to_code_point(lead, trail);
            data_index = self.small_index(Type::Small, it.c);
        } else {
            data_index = self.data_length - ERROR_VALUE_NEG_DATA_OFFSET;
        }
        it.value = self.data.get_from_index(data_index);
        true
    }

    /// `SmallStringIterator.previous()`.
    pub(crate) fn small_string_iterator_previous(&self, it: &mut StringIterator<'_>) -> bool {
        if it.s_index <= 0 {
            return false;
        }
        it.s_index -= 1;
        let trail = it.s[it.s_index as usize];
        it.c = trail as i32;
        let data_index;
        if !UTF16Plus::is_surrogate(trail as i32) {
            data_index = self.cp_index(it.c);
        } else if !UTF16Plus::is_surrogate_lead(trail as i32)
            && it.s_index > 0
            && crate::jdk::is_high_surrogate(it.s[(it.s_index - 1) as usize])
        {
            let lead = it.s[(it.s_index - 1) as usize];
            it.s_index -= 1;
            it.c = crate::jdk::to_code_point(lead, trail);
            data_index = self.small_index(Type::Small, it.c);
        } else {
            data_index = self.data_length - ERROR_VALUE_NEG_DATA_OFFSET;
        }
        it.value = self.data.get_from_index(data_index);
        true
    }
}

impl CodePointMap for CodePointTrie {
    /// `get(int c)`: `data.getFromIndex(cpIndex(c))` (the `get` overrides of
    /// `Fast16`, `Fast32` and `Fast8` read the same array the same way).
    fn get(&self, c: i32) -> i32 {
        self.data.get_from_index(self.cp_index(c))
    }

    /// `getRange(int start, ValueFilter filter, Range range)` (final).
    fn get_range(&self, start: i32, filter: Option<ValueFilter<'_>>, range: &mut Range) -> bool {
        if !(0..=MAX_UNICODE).contains(&start) {
            return false;
        }
        if start >= self.high_start {
            let di = self.data_length - HIGH_VALUE_NEG_DATA_OFFSET;
            let mut value = self.data.get_from_index(di);
            if let Some(f) = filter {
                value = f(value);
            }
            range.set(start, MAX_UNICODE, value);
            return true;
        }

        let mut null_value = self.null_value;
        if let Some(f) = filter {
            null_value = f(null_value);
        }
        let type_ = self.get_type();

        let mut prev_i3_block = -1;
        let mut prev_block = -1;
        let mut c = start;
        // Initialize to make compiler happy. Real value when haveValue is true.
        let mut trie_value = 0;
        let mut value = 0;
        let mut have_value = false;
        loop {
            let i3_block;
            let mut i3;
            let i3_block_length;
            let data_block_length;
            'block: {
                if c <= 0xffff && (type_ == Type::Fast || c <= SMALL_MAX) {
                    i3_block = 0;
                    i3 = c >> FAST_SHIFT;
                    i3_block_length = if type_ == Type::Fast { BMP_INDEX_LENGTH } else { SMALL_INDEX_LENGTH };
                    data_block_length = FAST_DATA_BLOCK_LENGTH;
                } else {
                    // Use the multi-stage index.
                    let mut i1 = c >> SHIFT_1;
                    if type_ == Type::Fast {
                        debug_assert!(0xffff < c && c < self.high_start);
                        i1 += BMP_INDEX_LENGTH - OMITTED_BMP_INDEX_1_LENGTH;
                    } else {
                        debug_assert!(c < self.high_start && self.high_start > SMALL_LIMIT);
                        i1 += SMALL_INDEX_LENGTH;
                    }
                    i3_block =
                        self.index[(self.index[i1 as usize] as i32 + ((c >> SHIFT_2) & INDEX_2_MASK)) as usize] as i32;
                    if i3_block == prev_i3_block && (c - start) >= CP_PER_INDEX_2_ENTRY {
                        // The index-3 block is the same as the previous one, and filled with value.
                        debug_assert!((c & (CP_PER_INDEX_2_ENTRY - 1)) == 0);
                        c += CP_PER_INDEX_2_ENTRY;
                        break 'block;
                    }
                    prev_i3_block = i3_block;
                    if i3_block == self.index3_null_offset {
                        // This is the index-3 null block.
                        if have_value {
                            if null_value != value {
                                range.set(start, c - 1, value);
                                return true;
                            }
                        } else {
                            trie_value = self.null_value;
                            value = null_value;
                            have_value = true;
                        }
                        prev_block = self.data_null_offset;
                        c = (c + CP_PER_INDEX_2_ENTRY) & !(CP_PER_INDEX_2_ENTRY - 1);
                        break 'block;
                    }
                    i3 = (c >> SHIFT_3) & INDEX_3_MASK;
                    i3_block_length = INDEX_3_BLOCK_LENGTH;
                    data_block_length = SMALL_DATA_BLOCK_LENGTH;
                }
                // Enumerate data blocks for one index-3 block.
                loop {
                    let mut block;
                    if (i3_block & 0x8000) == 0 {
                        block = self.index[(i3_block + i3) as usize] as i32;
                    } else {
                        // 18-bit indexes stored in groups of 9 entries per 8 indexes.
                        let mut group = (i3_block & 0x7fff) + (i3 & !7) + (i3 >> 3);
                        let gi = i3 & 7;
                        block = ((self.index[group as usize] as i32) << (2 + (2 * gi))) & 0x30000;
                        group += 1;
                        block |= self.index[(group + gi) as usize] as i32;
                    }
                    if block == prev_block && (c - start) >= data_block_length {
                        // The block is the same as the previous one, and filled with value.
                        debug_assert!((c & (data_block_length - 1)) == 0);
                        c += data_block_length;
                    } else {
                        let data_mask = data_block_length - 1;
                        prev_block = block;
                        if block == self.data_null_offset {
                            // This is the data null block.
                            if have_value {
                                if null_value != value {
                                    range.set(start, c - 1, value);
                                    return true;
                                }
                            } else {
                                trie_value = self.null_value;
                                value = null_value;
                                have_value = true;
                            }
                            c = (c + data_block_length) & !data_mask;
                        } else {
                            let mut di = block + (c & data_mask);
                            let mut trie_value2 = self.data.get_from_index(di);
                            if have_value {
                                if trie_value2 != trie_value {
                                    if filter.is_none()
                                        || Self::maybe_filter_value(trie_value2, self.null_value, null_value, filter)
                                            != value
                                    {
                                        range.set(start, c - 1, value);
                                        return true;
                                    }
                                    trie_value = trie_value2; // may or may not help
                                }
                            } else {
                                trie_value = trie_value2;
                                value = Self::maybe_filter_value(trie_value2, self.null_value, null_value, filter);
                                have_value = true;
                            }
                            loop {
                                c += 1;
                                if (c & data_mask) == 0 {
                                    break;
                                }
                                di += 1;
                                trie_value2 = self.data.get_from_index(di);
                                if trie_value2 != trie_value {
                                    if filter.is_none()
                                        || Self::maybe_filter_value(trie_value2, self.null_value, null_value, filter)
                                            != value
                                    {
                                        range.set(start, c - 1, value);
                                        return true;
                                    }
                                    trie_value = trie_value2; // may or may not help
                                }
                            }
                        }
                    }
                    i3 += 1;
                    if i3 >= i3_block_length {
                        break;
                    }
                }
            }
            if c >= self.high_start {
                break;
            }
        }
        debug_assert!(have_value);
        let di = self.data_length - HIGH_VALUE_NEG_DATA_OFFSET;
        let high_value = self.data.get_from_index(di);
        if Self::maybe_filter_value(high_value, self.null_value, null_value, filter) != value {
            c -= 1;
        } else {
            c = MAX_UNICODE;
        }
        range.set(start, c, value);
        true
    }

    fn iterator(&self) -> RangeIterator<'_> {
        RangeIterator::new(self)
    }

    /// `stringIterator(CharSequence s, int sIndex)` (final in `Fast` and
    /// `Small`): a `FastStringIterator` or `SmallStringIterator`.
    fn string_iterator<'a>(&'a self, s: &'a [u16], s_index: i32) -> StringIterator<'a> {
        let kind = match self.type_ {
            Type::Fast => StringIteratorKind::Fast(self),
            Type::Small => StringIteratorKind::Small(self),
        };
        StringIterator::new(kind, s, s_index)
    }
}

/// `new ClassCastException` for a cast of a trie to a subclass it is not.
fn class_cast(from: &CodePointTrie, to: &str) -> LuceneError {
    let from_name = match (from.get_type(), from.get_value_width()) {
        (Type::Fast, ValueWidth::Bits16) => "Fast16",
        (Type::Fast, ValueWidth::Bits32) => "Fast32",
        (Type::Fast, ValueWidth::Bits8) => "Fast8",
        (Type::Small, ValueWidth::Bits16) => "Small16",
        (Type::Small, ValueWidth::Bits32) => "Small32",
        (Type::Small, ValueWidth::Bits8) => "Small8",
    };
    LuceneError::unchecked(
        "java.lang.ClassCastException",
        format!(
            "class com.ibm.icu.util.CodePointTrie${from_name} cannot be cast to class \
             com.ibm.icu.util.CodePointTrie${to} (com.ibm.icu.util.CodePointTrie${from_name} and \
             com.ibm.icu.util.CodePointTrie${to} are in unnamed module of loader 'app')"
        ),
    )
}

/// Declares a typed `CodePointTrie` subclass wrapper with its `fromBinary`.
macro_rules! trie_subclass {
    ($(#[$doc:meta])* $name:ident, $type_:expr, $width:expr, $java:literal) => {
        $(#[$doc])*
        #[derive(Debug, Clone)]
        pub struct $name(CodePointTrie);

        impl Deref for $name {
            type Target = CodePointTrie;

            fn deref(&self) -> &CodePointTrie {
                &self.0
            }
        }

        impl $name {
            /// `fromBinary(ByteBuffer bytes)`: `CodePointTrie.fromBinary` with
            /// this class's type and value width, cast to this class.
            ///
            /// # Errors
            ///
            /// As [`CodePointTrie::from_binary`].
            pub fn from_binary(bytes: &mut ByteBuffer) -> Result<$name> {
                let trie = CodePointTrie::from_binary(Some($type_), Some($width), bytes)?;
                Ok($name(trie))
            }

            /// The trie as its base class.
            pub fn into_inner(self) -> CodePointTrie {
                self.0
            }
        }

        impl TryFrom<CodePointTrie> for $name {
            type Error = LuceneError;

            /// The Java cast `(X) trie`.
            fn try_from(trie: CodePointTrie) -> Result<$name> {
                if trie.get_type() == $type_ && trie.get_value_width() == $width {
                    Ok($name(trie))
                } else {
                    Err(class_cast(&trie, $java))
                }
            }
        }
    };
}

/// A trie with a fast data structure (any value width).
///
/// Equivalent to the abstract class `com.ibm.icu.util.CodePointTrie.Fast`.
#[derive(Debug, Clone)]
pub struct Fast(CodePointTrie);

impl Deref for Fast {
    type Target = CodePointTrie;

    fn deref(&self) -> &CodePointTrie {
        &self.0
    }
}

impl Fast {
    /// `Fast.fromBinary(ValueWidth valueWidth, ByteBuffer bytes)`.
    ///
    /// # Errors
    ///
    /// As [`CodePointTrie::from_binary`].
    pub fn from_binary(value_width: Option<ValueWidth>, bytes: &mut ByteBuffer) -> Result<Fast> {
        Ok(Fast(CodePointTrie::from_binary(Some(Type::Fast), value_width, bytes)?))
    }
}

/// A trie with a small data structure (any value width).
///
/// Equivalent to the abstract class `com.ibm.icu.util.CodePointTrie.Small`.
#[derive(Debug, Clone)]
pub struct Small(CodePointTrie);

impl Deref for Small {
    type Target = CodePointTrie;

    fn deref(&self) -> &CodePointTrie {
        &self.0
    }
}

impl Small {
    /// `Small.fromBinary(ValueWidth valueWidth, ByteBuffer bytes)`.
    ///
    /// # Errors
    ///
    /// As [`CodePointTrie::from_binary`].
    pub fn from_binary(value_width: Option<ValueWidth>, bytes: &mut ByteBuffer) -> Result<Small> {
        Ok(Small(CodePointTrie::from_binary(Some(Type::Small), value_width, bytes)?))
    }
}

trie_subclass!(
    /// Fast trie with 16-bit data.
    ///
    /// Equivalent to `com.ibm.icu.util.CodePointTrie.Fast16`; `get`,
    /// `bmpGet` and `suppGet` read its 16-bit data array.
    Fast16, Type::Fast, ValueWidth::Bits16, "Fast16"
);
trie_subclass!(
    /// Fast trie with 32-bit data.
    ///
    /// Equivalent to `com.ibm.icu.util.CodePointTrie.Fast32`.
    Fast32, Type::Fast, ValueWidth::Bits32, "Fast32"
);
trie_subclass!(
    /// Fast trie with 8-bit data.
    ///
    /// Equivalent to `com.ibm.icu.util.CodePointTrie.Fast8`.
    Fast8, Type::Fast, ValueWidth::Bits8, "Fast8"
);
trie_subclass!(
    /// Small trie with 16-bit data.
    ///
    /// Equivalent to `com.ibm.icu.util.CodePointTrie.Small16`.
    Small16, Type::Small, ValueWidth::Bits16, "Small16"
);
trie_subclass!(
    /// Small trie with 32-bit data.
    ///
    /// Equivalent to `com.ibm.icu.util.CodePointTrie.Small32`.
    Small32, Type::Small, ValueWidth::Bits32, "Small32"
);
trie_subclass!(
    /// Small trie with 8-bit data.
    ///
    /// Equivalent to `com.ibm.icu.util.CodePointTrie.Small8`.
    Small8, Type::Small, ValueWidth::Bits8, "Small8"
);

impl CodePointTrie {
    /// The trie of one of the typed wrappers, from its parts (used by
    /// `MutableCodePointTrie.buildImmutable`).
    pub(crate) fn from_parts(
        type_: Type,
        index: Vec<u16>,
        data: Data,
        high_start: i32,
        index3_null_offset: i32,
        data_null_offset: i32,
    ) -> CodePointTrie {
        CodePointTrie::new(type_, index, data, high_start, index3_null_offset, data_null_offset)
    }
}
