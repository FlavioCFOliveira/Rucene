// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.EmojiProps`.

use std::sync::{Arc, LazyLock};

use rucene::error::Result;

use crate::lang::u_property::UProperty;
use crate::r#impl::icu_binary::ICUBinary;
use crate::text::unicode_set::UnicodeSet;
use crate::util::chars_trie::CharsTrie;
use crate::util::code_point_map::{CodePointMap, Range};
use crate::util::code_point_trie::{Fast, ValueWidth};
use crate::util::icu_unchecked_io_exception::ICUUncheckedIOException;

const DATA_FORMAT: i32 = 0x456d6f6a; // "Emoj"

/// Whether the data file's format version is supported.
///
/// Equivalent to the private class `com.ibm.icu.impl.EmojiProps.IsAcceptable`,
/// whose `IsAcceptable.isDataVersionAcceptable(byte[])` is this function
/// (an `ICUBinary.Authenticate`).
fn is_acceptable(version: &[u8; 4]) -> bool {
    version[0] == 1
}

// Byte offsets from the start of the data, after the generic header,
// in ascending order.
// UCPTrie=CodePointTrie, follows the indexes
const IX_CPTRIE_OFFSET: usize = 0;

// Note: ICU-internal string tries are not ported to ICU4J.
const IX_BASIC_EMOJI_TRIE_OFFSET: usize = 4;
//private static final int IX_EMOJI_KEYCAP_SEQUENCE_TRIE_OFFSET = 5;
//private static final int IX_RGI_EMOJI_MODIFIER_SEQUENCE_TRIE_OFFSET = 6;
//private static final int IX_RGI_EMOJI_FLAG_SEQUENCE_TRIE_OFFSET = 7;
//private static final int IX_RGI_EMOJI_TAG_SEQUENCE_TRIE_OFFSET = 8;
const IX_RGI_EMOJI_ZWJ_SEQUENCE_TRIE_OFFSET: usize = 9;

// Properties in the code point trie.
const BIT_EMOJI: i8 = 0;
const BIT_EMOJI_PRESENTATION: i8 = 1;
const BIT_EMOJI_MODIFIER: i8 = 2;
const BIT_EMOJI_MODIFIER_BASE: i8 = 3;
const BIT_EMOJI_COMPONENT: i8 = 4;
const BIT_EXTENDED_PICTOGRAPHIC: i8 = 5;
// https://www.unicode.org/reports/tr51/#Emoji_Sets
const BIT_BASIC_EMOJI: i8 = 6;

/// Emoji properties (code point trie and string tries of `uemoji.icu`).
///
/// Equivalent to `com.ibm.icu.impl.EmojiProps`.
#[derive(Debug)]
pub struct EmojiProps {
    cp_trie: Fast,
    string_tries: [Option<Arc<[u16]>>; 6],
}

/// `INSTANCE`. A failure of Java's static initializer surfaces as
/// `ExceptionInInitializerError`, here a panic.
static INSTANCE: LazyLock<EmojiProps> = LazyLock::new(|| match EmojiProps::new() {
    Ok(p) => p,
    Err(e) => panic!("java.lang.ExceptionInInitializerError: {e}"),
});

fn get_string_trie_index(i: usize) -> usize {
    i - IX_BASIC_EMOJI_TRIE_OFFSET
}

static BIT_FLAGS: [i8; 15] = [
    BIT_EMOJI,                 // UCHAR_EMOJI=57
    BIT_EMOJI_PRESENTATION,    // UCHAR_EMOJI_PRESENTATION=58
    BIT_EMOJI_MODIFIER,        // UCHAR_EMOJI_MODIFIER=59
    BIT_EMOJI_MODIFIER_BASE,   // UCHAR_EMOJI_MODIFIER_BASE=60
    BIT_EMOJI_COMPONENT,       // UCHAR_EMOJI_COMPONENT=61
    -1,                        // UCHAR_REGIONAL_INDICATOR=62
    -1,                        // UCHAR_PREPENDED_CONCATENATION_MARK=63
    BIT_EXTENDED_PICTOGRAPHIC, // UCHAR_EXTENDED_PICTOGRAPHIC=64
    BIT_BASIC_EMOJI,           // UCHAR_BASIC_EMOJI=65
    -1,                        // UCHAR_EMOJI_KEYCAP_SEQUENCE=66
    -1,                        // UCHAR_RGI_EMOJI_MODIFIER_SEQUENCE=67
    -1,                        // UCHAR_RGI_EMOJI_FLAG_SEQUENCE=68
    -1,                        // UCHAR_RGI_EMOJI_TAG_SEQUENCE=69
    -1,                        // UCHAR_RGI_EMOJI_ZWJ_SEQUENCE=70
    BIT_BASIC_EMOJI,           // UCHAR_RGI_EMOJI=71
];

impl EmojiProps {
    #[allow(clippy::needless_range_loop)] // indexed loop mirrors the Java for-loop
    #[allow(unused_assignments)] // initial value mirrors the Java declaration
    fn new() -> Result<Self> {
        let mut bytes = ICUBinary::get_required_data("uemoji.icu")?;
        ICUBinary::read_header_and_data_version(
            &mut bytes,
            DATA_FORMAT,
            Some(&is_acceptable as &dyn Fn(&[u8; 4]) -> bool),
        )?;
        let start_pos = bytes.position();

        let cp_trie_offset = bytes.get_int()?; // inIndexes[IX_CPTRIE_OFFSET]
        let indexes_length = cp_trie_offset / 4;
        if indexes_length <= IX_RGI_EMOJI_ZWJ_SEQUENCE_TRIE_OFFSET as i32 {
            return Err(ICUUncheckedIOException::with_message(
                "Emoji properties data: not enough indexes",
            )
            .error());
        }
        let mut in_indexes = vec![0i32; indexes_length as usize];
        in_indexes[0] = cp_trie_offset;
        for i in 1..indexes_length as usize {
            in_indexes[i] = bytes.get_int()?;
        }

        let mut i = IX_CPTRIE_OFFSET;
        let mut offset = in_indexes[i];
        i += 1;
        let mut next_offset = in_indexes[i];
        let cp_trie = Fast::from_binary(Some(ValueWidth::Bits8), &mut bytes)?;
        let pos = bytes.position() - start_pos;
        debug_assert!(next_offset >= pos);
        ICUBinary::skip_bytes(&mut bytes, next_offset - pos)?; // skip padding after trie bytes

        offset = next_offset;
        next_offset = in_indexes[IX_BASIC_EMOJI_TRIE_OFFSET];
        ICUBinary::skip_bytes(&mut bytes, next_offset - offset)?; // skip unknown bytes

        let mut string_tries: [Option<Arc<[u16]>>; 6] = Default::default();
        for i in IX_BASIC_EMOJI_TRIE_OFFSET..=IX_RGI_EMOJI_ZWJ_SEQUENCE_TRIE_OFFSET {
            let offset = in_indexes[i];
            let next_offset = in_indexes[i + 1];
            if next_offset > offset {
                string_tries[get_string_trie_index(i)] = Some(Arc::from(ICUBinary::get_string(
                    &mut bytes,
                    (next_offset - offset) / 2,
                    0,
                )?));
            }
        }
        let _ = offset;
        Ok(Self {
            cp_trie,
            string_tries,
        })
    }

    /// `INSTANCE`.
    ///
    /// # Panics
    ///
    /// When the data cannot be read (Java's `ExceptionInInitializerError`).
    pub fn instance() -> &'static EmojiProps {
        &INSTANCE
    }

    /// `addPropertyStarts(UnicodeSet set)`.
    ///
    /// # Errors
    ///
    /// The errors of [`UnicodeSet::add`] (a frozen set).
    pub fn add_property_starts<'s>(&self, set: &'s mut UnicodeSet) -> Result<&'s mut UnicodeSet> {
        // Add the start code point of each same-value range of the trie.
        let mut range = Range::new();
        let mut start = 0;
        while self.cp_trie.get_range(start, None, &mut range) {
            set.add(start)?;
            start = range.get_end() + 1;
        }
        Ok(set)
    }

    /// `hasBinaryProperty(int c, int which)`.
    pub fn has_binary_property(&self, c: i32, which: i32) -> bool {
        if !(UProperty::EMOJI..=UProperty::RGI_EMOJI).contains(&which) {
            return false;
        }
        // Note: UCHAR_REGIONAL_INDICATOR is a single, hardcoded range implemented elsewhere.
        let bit = BIT_FLAGS[(which - UProperty::EMOJI) as usize];
        if bit < 0 {
            return false;
        }
        let bits = self.cp_trie.get(c);
        ((bits >> bit) & 1) != 0
    }

    /// `hasBinaryProperty(CharSequence s, int which)`.
    pub fn has_binary_property_str(&self, s: &[u16], which: i32) -> bool {
        let length = s.len() as i32;
        if length == 0 {
            return false;
        } // empty string
          // The caller should have delegated single code points to hasBinaryProperty(c, which).
        if !(UProperty::BASIC_EMOJI..=UProperty::RGI_EMOJI).contains(&which) {
            return false;
        }
        let mut first_prop = which;
        let mut last_prop = which;
        if which == UProperty::RGI_EMOJI {
            // RGI_Emoji is the union of the other emoji properties of strings.
            first_prop = UProperty::BASIC_EMOJI;
            last_prop = UProperty::RGI_EMOJI_ZWJ_SEQUENCE;
        }
        for prop in first_prop..=last_prop {
            if let Some(trie_u_chars) = &self.string_tries[(prop - UProperty::BASIC_EMOJI) as usize]
            {
                let mut trie = CharsTrie::new(trie_u_chars.clone(), 0);
                let result = trie.next_chars(s, 0, length);
                if result.has_value() {
                    return true;
                }
            }
        }
        false
    }

    /// `addStrings(int which, UnicodeSet set)`.
    ///
    /// # Errors
    ///
    /// The errors of [`UnicodeSet::add_str`] (a frozen set).
    pub fn add_strings(&self, which: i32, set: &mut UnicodeSet) -> Result<()> {
        if !(UProperty::BASIC_EMOJI..=UProperty::RGI_EMOJI).contains(&which) {
            return Ok(());
        }
        let mut first_prop = which;
        let mut last_prop = which;
        if which == UProperty::RGI_EMOJI {
            // RGI_Emoji is the union of the other emoji properties of strings.
            first_prop = UProperty::BASIC_EMOJI;
            last_prop = UProperty::RGI_EMOJI_ZWJ_SEQUENCE;
        }
        for prop in first_prop..=last_prop {
            if let Some(trie_u_chars) = &self.string_tries[(prop - UProperty::BASIC_EMOJI) as usize]
            {
                let trie = CharsTrie::new(trie_u_chars.clone(), 0);
                for entry in trie.iterator() {
                    set.add_str(&entry.chars)?;
                }
            }
        }
        Ok(())
    }
}
