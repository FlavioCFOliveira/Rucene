// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.UCharacterName` and `com.ibm.icu.impl.UCharacterNameReader`.

use std::sync::{LazyLock, Mutex};

use rucene::error::{LuceneError, Result};

use crate::jdk::ByteBuffer;
use crate::lang::u_character::UCharacter;
use crate::lang::u_character_enums::ECharacterCategory;
use crate::r#impl::icu_binary::ICUBinary;
use crate::r#impl::u_character_name_choice::UCharacterNameChoice;
use crate::r#impl::u_character_utility::UCharacterUtility;
use crate::r#impl::utility::Utility;
use crate::text::unicode_set::UnicodeSet;
use crate::text::utf16::UTF16;
use crate::util::u_resource_bundle::missing_resource;

/// `LINES_PER_GROUP_`: number of lines per group (`1 << GROUP_SHIFT_`).
pub const LINES_PER_GROUP: i32 = 1 << 5;

const FILE_NAME: &str = "unames.icu";
/// Shift count to retrieve group information.
const GROUP_SHIFT: i32 = 5;
/// Mask to retrieve the offset for a particular character within a group.
const GROUP_MASK: i32 = LINES_PER_GROUP - 1;
/// Position of offsethigh in group information array.
const OFFSET_HIGH_OFFSET: usize = 1;
/// Position of offsetlow in group information array.
const OFFSET_LOW_OFFSET: usize = 2;
/// Double nibble indicator, any nibble > this number has to be combined
/// with its following nibble.
const SINGLE_NIBBLE_MAX: i8 = 11;

static TYPE_NAMES: [&str; 33] = [
    "unassigned",
    "uppercase letter",
    "lowercase letter",
    "titlecase letter",
    "modifier letter",
    "other letter",
    "non spacing mark",
    "enclosing mark",
    "combining spacing mark",
    "decimal digit number",
    "letter number",
    "other number",
    "space separator",
    "line separator",
    "paragraph separator",
    "control",
    "format",
    "private use area",
    "surrogate",
    "dash punctuation",
    "start punctuation",
    "end punctuation",
    "connector punctuation",
    "other punctuation",
    "math symbol",
    "currency symbol",
    "modifier symbol",
    "other symbol",
    "initial punctuation",
    "final punctuation",
    "noncharacter",
    "lead surrogate",
    "trail surrogate",
];
const UNKNOWN_TYPE_NAME: &str = "unknown";
const NON_CHARACTER: i32 = ECharacterCategory::CHAR_CATEGORY_COUNT as i32;
const LEAD_SURROGATE: i32 = ECharacterCategory::CHAR_CATEGORY_COUNT as i32 + 1;
const TRAIL_SURROGATE: i32 = ECharacterCategory::CHAR_CATEGORY_COUNT as i32 + 2;
/// `EXTENDED_CATEGORY_` (package-private): extended category count.
#[allow(dead_code)] // Java member with no caller in the closure
pub(crate) const EXTENDED_CATEGORY: i32 = ECharacterCategory::CHAR_CATEGORY_COUNT as i32 + 3;

fn units(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

/// The name sets and maximum lengths `initNameSetsLengths()` computes
/// lazily (`m_nameSet_`, `m_ISOCommentSet_`, `m_maxNameLength_`,
/// `m_maxISOCommentLength_`).
#[derive(Debug, Default)]
struct NameSets {
    name_set: [i32; 8],
    iso_comment_set: [i32; 8],
    max_iso_comment_length: i32,
    max_name_length: i32,
}

/// Internal class to manage character names.
///
/// Equivalent to `com.ibm.icu.impl.UCharacterName`. Java's synchronized
/// scratch buffers (`m_utilStringBuffer_`, `m_groupoffsets_`, ...) are
/// local buffers per call; the lazily computed name sets are behind a
/// `Mutex`.
#[derive(Debug)]
pub struct UCharacterName {
    /// `m_groupcount_`: number of groups.
    pub m_groupcount: i32,
    /// Size of each group.
    m_groupsize: i32,
    /// Token table.
    m_tokentable: Vec<u16>,
    /// Token string.
    m_tokenstring: Vec<i8>,
    /// Group information.
    m_groupinfo: Vec<u16>,
    /// Group string.
    m_groupstring: Vec<i8>,
    /// Algorithmic names.
    m_algorithm: Vec<AlgorithmName>,
    sets: Mutex<NameSets>,
}

/// `INSTANCE`. A failure of Java's static initializer surfaces as
/// `ExceptionInInitializerError` (its `MissingResourceException`), here a
/// panic.
static INSTANCE: LazyLock<UCharacterName> = LazyLock::new(|| match UCharacterName::new() {
    Ok(n) => n,
    Err(_) => panic!(
        "java.lang.ExceptionInInitializerError: {}",
        missing_resource(
            "Could not construct UCharacterName. Missing unames.icu",
            "",
            ""
        )
    ),
});

impl UCharacterName {
    /// `INSTANCE`.
    ///
    /// # Panics
    ///
    /// When the data cannot be read (Java's `ExceptionInInitializerError`).
    pub fn instance() -> &'static UCharacterName {
        &INSTANCE
    }

    fn new() -> Result<Self> {
        let b = ICUBinary::get_required_data(FILE_NAME)?;
        let mut reader = UCharacterNameReader::new(b)?;
        let mut data = Self {
            m_groupcount: 0,
            m_groupsize: 0,
            m_tokentable: Vec::new(),
            m_tokenstring: Vec::new(),
            m_groupinfo: Vec::new(),
            m_groupstring: Vec::new(),
            m_algorithm: Vec::new(),
            sets: Mutex::new(NameSets::default()),
        };
        reader.read(&mut data)?;
        Ok(data)
    }

    // public methods ---------------------------------------------------

    /// `getName(int ch, int choice)`: the name of a code point, `None`
    /// when it has none.
    pub fn get_name(&self, ch: i32, choice: i32) -> Option<Vec<u16>> {
        if !(UCharacter::MIN_VALUE..=UCharacter::MAX_VALUE).contains(&ch)
            || choice > UCharacterNameChoice::CHAR_NAME_CHOICE_COUNT
        {
            return None;
        }

        // try algorithmic names first, if fails then try group names
        let mut result = self.get_alg_name(ch, choice);

        // getting normal character name
        if result.as_ref().is_none_or(|r| r.is_empty()) {
            if choice == UCharacterNameChoice::EXTENDED_CHAR_NAME {
                result = self.get_extended_name(ch);
            } else {
                result = self.get_group_name_cp(ch, choice);
            }
        }

        result
    }

    /// `getCharFromName(int choice, String name)`: the code point of a
    /// name, -1 when none.
    pub fn get_char_from_name(&self, choice: i32, name: Option<&[u16]>) -> i32 {
        // checks for illegal arguments
        let Some(name) = name else { return -1 };
        if choice >= UCharacterNameChoice::CHAR_NAME_CHOICE_COUNT || name.is_empty() {
            return -1;
        }

        // try extended names first
        let lower = english_lower(name);
        let mut result = Self::get_extended_char(&lower, choice);
        if result >= -1 {
            return result;
        }

        let upper_case_name = english_upper(name);
        // try algorithmic names first, if fails then try group names
        // int result = getAlgorithmChar(choice, uppercasename);

        if choice == UCharacterNameChoice::UNICODE_CHAR_NAME
            || choice == UCharacterNameChoice::EXTENDED_CHAR_NAME
        {
            let mut count = self.m_algorithm.len() as i32;
            count -= 1;
            while count >= 0 {
                result = self.m_algorithm[count as usize].get_char(&upper_case_name);
                if result >= 0 {
                    return result;
                }
                count -= 1;
            }
        }

        if choice == UCharacterNameChoice::EXTENDED_CHAR_NAME {
            result =
                self.get_group_char_name(&upper_case_name, UCharacterNameChoice::UNICODE_CHAR_NAME);
            if result == -1 {
                result = self
                    .get_group_char_name(&upper_case_name, UCharacterNameChoice::CHAR_NAME_ALIAS);
            }
        } else {
            result = self.get_group_char_name(&upper_case_name, choice);
        }
        result
    }

    // these are all UCharacterNameIterator use methods -------------------

    /// `getGroupLengths(int index, char offsets[], char lengths[])`: reads
    /// the offsets and lengths of the 32 lines of group `index`, returns
    /// the offset of the group's strings.
    pub fn get_group_lengths(
        &self,
        mut index: i32,
        offsets: &mut [u16],
        lengths: &mut [u16],
    ) -> i32 {
        let mut length: u16 = 0xffff;
        let mut n: i8;
        index *= self.m_groupsize;
        let mut stringoffset = UCharacterUtility::to_int(
            self.m_groupinfo[index as usize + OFFSET_HIGH_OFFSET],
            self.m_groupinfo[index as usize + OFFSET_LOW_OFFSET],
        );

        offsets[0] = 0;

        // all 32 lengths must be read to get the offset of the first group
        // string
        let mut i = 0;
        while i < LINES_PER_GROUP {
            let b = self.m_groupstring[stringoffset as usize];
            let mut shift = 4;

            while shift >= 0 {
                // getting nibble
                n = (b >> shift) & 0x0F;
                if length == 0xffff && n > SINGLE_NIBBLE_MAX {
                    length = ((n as i32 - 12) << 4) as u16;
                } else {
                    if length != 0xffff {
                        lengths[i as usize] = ((length as i32 | n as i32) + 12) as u16;
                    } else {
                        lengths[i as usize] = n as u16;
                    }

                    if i < LINES_PER_GROUP {
                        offsets[i as usize + 1] =
                            offsets[i as usize].wrapping_add(lengths[i as usize]);
                    }

                    length = 0xffff;
                    i += 1;
                }

                shift -= 4;
            }
            stringoffset += 1;
        }
        stringoffset
    }

    /// `getGroupName(int index, int length, int choice)`: the name at
    /// `index` in the group strings, `None` when empty.
    pub fn get_group_name(&self, mut index: i32, mut length: i32, choice: i32) -> Option<Vec<u16>> {
        if choice != UCharacterNameChoice::UNICODE_CHAR_NAME
            && choice != UCharacterNameChoice::EXTENDED_CHAR_NAME
        {
            if (b';' as usize) >= self.m_tokentable.len()
                || self.m_tokentable[b';' as usize] == 0xFFFF
            {
                /*
                 * skip the modern name if it is not requested _and_
                 * if the semicolon byte value is a character, not a token number
                 */
                let mut field_index = if choice == UCharacterNameChoice::ISO_COMMENT {
                    2
                } else {
                    choice
                };
                loop {
                    let oldindex = index;
                    index += UCharacterUtility::skip_byte_sub_string(
                        &self.m_groupstring,
                        index,
                        length,
                        b';' as i8,
                    );
                    length -= index - oldindex;
                    field_index -= 1;
                    if field_index <= 0 {
                        break;
                    }
                }
            } else {
                // the semicolon byte is a token number, therefore only modern
                // names are stored in unames.dat and there is no such
                // requested alternate name here
                length = 0;
            }
        }

        let mut buf: Vec<u16> = Vec::new();
        let tlen = self.m_tokentable.len() as i32;
        let mut i = 0;
        while i < length {
            let b = self.m_groupstring[(index + i) as usize];
            i += 1;

            if b as i32 >= tlen {
                if b == b';' as i8 {
                    break;
                }
                // StringBuffer.append(byte) appends the decimal value.
                buf.extend(units(&(b as i32).to_string()));
            } else {
                let mut token = self.m_tokentable[(b as i32 & 0x00ff) as usize];
                if token == 0xFFFE {
                    // this is a lead byte for a double-byte token
                    let idx = ((b as i32) << 8)
                        | (self.m_groupstring[(index + i) as usize] as i32 & 0x00ff);
                    token = self.m_tokentable[idx as usize];
                    i += 1;
                }
                if token == 0xFFFF {
                    if b == b';' as i8 {
                        // skip the semicolon if we are seeking extended
                        // names and there was no 2.0 name but there
                        // is a 1.0 name.
                        if buf.is_empty() && choice == UCharacterNameChoice::EXTENDED_CHAR_NAME {
                            continue;
                        }
                        break;
                    }
                    // explicit letter
                    buf.push((b as i32 & 0x00ff) as u16);
                } else {
                    // write token word
                    UCharacterUtility::get_null_term_byte_sub_string(
                        &mut buf,
                        &self.m_tokenstring,
                        token as i32,
                    );
                }
            }
        }

        if !buf.is_empty() {
            return Some(buf);
        }
        None
    }

    /// `getExtendedName(int ch)`: the extended name of a code point.
    pub fn get_extended_name(&self, ch: i32) -> Option<Vec<u16>> {
        let mut result = self.get_name(ch, UCharacterNameChoice::UNICODE_CHAR_NAME);
        if result.is_none() {
            // TODO: Return Name_Alias/control names for control codes 0..1F & 7F..9F.
            result = Some(self.get_extended_or10_name(ch));
        }
        result
    }

    /// `getGroup(int codepoint)`: the group index of a code point.
    pub fn get_group(&self, codepoint: i32) -> i32 {
        let mut end_group = self.m_groupcount;
        let msb = Self::get_codepoint_msb(codepoint);
        let mut result = 0;
        // binary search for the group of names that contains the one for
        // code
        // find the group that contains codepoint, or the highest before it
        while result < end_group - 1 {
            let gindex = (result + end_group) >> 1;
            if msb < self.get_group_msb(gindex) {
                end_group = gindex;
            } else {
                result = gindex;
            }
        }
        result
    }

    /// `getExtendedOr10Name(int ch)`: `<type-XXXX>`.
    pub fn get_extended_or10_name(&self, ch: i32) -> Vec<u16> {
        // TODO: Return Name_Alias/control names for control codes 0..1F & 7F..9F.
        let ty = Self::get_type(ch);
        // getting the type name
        let result = if ty >= TYPE_NAMES.len() as i32 {
            UNKNOWN_TYPE_NAME
        } else {
            TYPE_NAMES[ty as usize]
        };
        let mut buf = units("<");
        buf.extend(units(result));
        buf.push(b'-' as u16);
        let ch_str = format!("{:x}", ch as u32).to_ascii_uppercase();
        let mut zeros = 4 - ch_str.len() as i32;
        while zeros > 0 {
            buf.push(b'0' as u16);
            zeros -= 1;
        }
        buf.extend(units(&ch_str));
        buf.push(b'>' as u16);
        buf
    }

    /// `getGroupMSB(int gindex)`: the most significant bits of a group, -1
    /// past the groups.
    pub fn get_group_msb(&self, gindex: i32) -> i32 {
        if gindex >= self.m_groupcount {
            return -1;
        }
        self.m_groupinfo[(gindex * self.m_groupsize) as usize] as i32
    }

    /// `getCodepointMSB(int codepoint)`.
    pub fn get_codepoint_msb(codepoint: i32) -> i32 {
        codepoint >> GROUP_SHIFT
    }

    /// `getGroupLimit(int msb)`.
    pub fn get_group_limit(msb: i32) -> i32 {
        (msb << GROUP_SHIFT) + LINES_PER_GROUP
    }

    /// `getGroupMin(int msb)`.
    pub fn get_group_min(msb: i32) -> i32 {
        msb << GROUP_SHIFT
    }

    /// `getGroupOffset(int codepoint)`.
    pub fn get_group_offset(codepoint: i32) -> i32 {
        codepoint & GROUP_MASK
    }

    /// `getGroupMinFromCodepoint(int codepoint)`.
    pub fn get_group_min_from_codepoint(codepoint: i32) -> i32 {
        codepoint & !GROUP_MASK
    }

    /// `getAlgorithmLength()`.
    pub fn get_algorithm_length(&self) -> i32 {
        self.m_algorithm.len() as i32
    }

    /// `getAlgorithmStart(int index)`.
    pub fn get_algorithm_start(&self, index: i32) -> i32 {
        self.m_algorithm[index as usize].m_rangestart
    }

    /// `getAlgorithmEnd(int index)`.
    pub fn get_algorithm_end(&self, index: i32) -> i32 {
        self.m_algorithm[index as usize].m_rangeend
    }

    /// `getAlgorithmName(int index, int codepoint)`.
    pub fn get_algorithm_name(&self, index: i32, codepoint: i32) -> Vec<u16> {
        let mut buf = Vec::new();
        self.m_algorithm[index as usize].append_name(codepoint, &mut buf);
        buf
    }

    /// `getGroupName(int ch, int choice)` (synchronized): the group name of
    /// a code point.
    pub fn get_group_name_cp(&self, ch: i32, choice: i32) -> Option<Vec<u16>> {
        // gets the msb
        let msb = Self::get_codepoint_msb(ch);
        let group = self.get_group(ch);

        // return this if it is an exact match
        if msb == self.m_groupinfo[(group * self.m_groupsize) as usize] as i32 {
            let mut offsets = [0u16; LINES_PER_GROUP as usize + 1];
            let mut lengths = [0u16; LINES_PER_GROUP as usize + 1];
            let index = self.get_group_lengths(group, &mut offsets, &mut lengths);
            let offset = (ch & GROUP_MASK) as usize;
            return self.get_group_name(
                index + offsets[offset] as i32,
                lengths[offset] as i32,
                choice,
            );
        }

        None
    }

    // these are transliterator use methods ---------------------------------

    /// `getMaxCharNameLength()`.
    pub fn get_max_char_name_length(&self) -> i32 {
        let mut s = self.sets.lock().unwrap_or_else(|e| e.into_inner());
        if self.init_name_sets_lengths(&mut s) {
            s.max_name_length
        } else {
            0
        }
    }

    /// `getMaxISOCommentLength()` (deprecated).
    pub fn get_max_iso_comment_length(&self) -> i32 {
        let mut s = self.sets.lock().unwrap_or_else(|e| e.into_inner());
        if self.init_name_sets_lengths(&mut s) {
            s.max_iso_comment_length
        } else {
            0
        }
    }

    /// `getCharNameCharacters(UnicodeSet set)`: the characters of all names.
    ///
    /// # Errors
    ///
    /// The errors of [`UnicodeSet::clear`]/[`UnicodeSet::add`] (a frozen set).
    pub fn get_char_name_characters(&self, set: &mut UnicodeSet) -> Result<()> {
        self.convert(false, set)
    }

    /// `getISOCommentCharacters(UnicodeSet set)` (deprecated).
    ///
    /// # Errors
    ///
    /// As [`get_char_name_characters`](Self::get_char_name_characters).
    pub fn get_iso_comment_characters(&self, set: &mut UnicodeSet) -> Result<()> {
        self.convert(true, set)
    }

    // package methods --------------------------------------------------

    /// `setToken(char token[], byte tokenstring[])` (package-private).
    fn set_token(&mut self, token: Vec<u16>, tokenstring: Vec<i8>) -> bool {
        if !token.is_empty() && !tokenstring.is_empty() {
            self.m_tokentable = token;
            self.m_tokenstring = tokenstring;
            return true;
        }
        false
    }

    /// `setAlgorithm(AlgorithmName alg[])` (package-private).
    fn set_algorithm(&mut self, alg: Vec<AlgorithmName>) -> bool {
        if !alg.is_empty() {
            self.m_algorithm = alg;
            return true;
        }
        false
    }

    /// `setGroupCountSize(int count, int size)` (package-private).
    fn set_group_count_size(&mut self, count: i32, size: i32) -> bool {
        if count <= 0 || size <= 0 {
            return false;
        }
        self.m_groupcount = count;
        self.m_groupsize = size;
        true
    }

    /// `setGroup(char group[], byte groupstring[])` (package-private).
    fn set_group(&mut self, group: Vec<u16>, groupstring: Vec<i8>) -> bool {
        if !group.is_empty() && !groupstring.is_empty() {
            self.m_groupinfo = group;
            self.m_groupstring = groupstring;
            return true;
        }
        false
    }

    // private methods ---------------------------------------------------

    /// Gets the algorithmic name for the argument codepoint.
    fn get_alg_name(&self, ch: i32, choice: i32) -> Option<Vec<u16>> {
        /* Only the normative character name can be algorithmic. */
        if choice == UCharacterNameChoice::UNICODE_CHAR_NAME
            || choice == UCharacterNameChoice::EXTENDED_CHAR_NAME
        {
            // index in terms integer index
            let mut buf = Vec::new();
            for index in (0..self.m_algorithm.len()).rev() {
                if self.m_algorithm[index].contains(ch) {
                    self.m_algorithm[index].append_name(ch, &mut buf);
                    return Some(buf);
                }
            }
        }
        None
    }

    /// Getting the character with the tokenized argument name (synchronized).
    fn get_group_char_name(&self, name: &[u16], choice: i32) -> i32 {
        let mut offsets = [0u16; LINES_PER_GROUP as usize + 1];
        let mut lengths = [0u16; LINES_PER_GROUP as usize + 1];
        for i in 0..self.m_groupcount {
            // populating the data set of grouptable

            let startgpstrindex = self.get_group_lengths(i, &mut offsets, &mut lengths);

            // shift out to function
            let result = self.get_group_char(startgpstrindex, &lengths, name, choice);
            if result != -1 {
                return ((self.m_groupinfo[(i * self.m_groupsize) as usize] as i32) << GROUP_SHIFT)
                    | result;
            }
        }
        -1
    }

    /// Compares and retrieve character if name is found within the argument
    /// group.
    fn get_group_char(&self, mut index: i32, length: &[u16], name: &[u16], choice: i32) -> i32 {
        let mut b: i8;
        let mut token: u16;
        let mut len;
        let namelen = name.len() as i32;
        let mut nindex;
        let mut count;
        let tlen = self.m_tokentable.len() as i32;

        for result in 0..=LINES_PER_GROUP {
            nindex = 0;
            len = length[result as usize] as i32;

            if choice != UCharacterNameChoice::UNICODE_CHAR_NAME
                && choice != UCharacterNameChoice::EXTENDED_CHAR_NAME
            {
                /*
                 * skip the modern name if it is not requested _and_
                 * if the semicolon byte value is a character, not a token number
                 */
                let mut field_index = if choice == UCharacterNameChoice::ISO_COMMENT {
                    2
                } else {
                    choice
                };
                loop {
                    let oldindex = index;
                    index += UCharacterUtility::skip_byte_sub_string(
                        &self.m_groupstring,
                        index,
                        len,
                        b';' as i8,
                    );
                    len -= index - oldindex;
                    field_index -= 1;
                    if field_index <= 0 {
                        break;
                    }
                }
            }

            // number of tokens is > the length of the name
            // write each letter directly, and write a token word per token
            count = 0;
            while count < len && nindex != -1 && nindex < namelen {
                b = self.m_groupstring[(index + count) as usize];
                count += 1;

                if b as i32 >= tlen {
                    let ch = name[nindex as usize];
                    nindex += 1;
                    if ch as i32 != (b as i32 & 0xFF) {
                        nindex = -1;
                    }
                } else {
                    token = self.m_tokentable[(b as i32 & 0xFF) as usize];
                    if token == 0xFFFE {
                        // this is a lead byte for a double-byte token
                        let idx = ((b as i32) << 8)
                            | (self.m_groupstring[(index + count) as usize] as i32 & 0x00ff);
                        token = self.m_tokentable[idx as usize];
                        count += 1;
                    }
                    if token == 0xFFFF {
                        let ch = name[nindex as usize];
                        nindex += 1;
                        if ch as i32 != (b as i32 & 0xFF) {
                            nindex = -1;
                        }
                    } else {
                        // compare token with name
                        nindex = UCharacterUtility::compare_null_term_byte_sub_string(
                            name,
                            &self.m_tokenstring,
                            nindex,
                            token as i32,
                        );
                    }
                }
            }

            if namelen == nindex
                && (count == len || self.m_groupstring[(index + count) as usize] == b';' as i8)
            {
                return result;
            }

            index += len;
        }
        -1
    }

    /// Gets the character extended type.
    fn get_type(ch: i32) -> i32 {
        if UCharacterUtility::is_non_character(ch) {
            // not a character we return a invalid category count
            return NON_CHARACTER;
        }
        let mut result = UCharacter::get_type(ch);
        if result == ECharacterCategory::SURROGATE as i32 {
            if ch <= UTF16::LEAD_SURROGATE_MAX_VALUE {
                result = LEAD_SURROGATE;
            } else {
                result = TRAIL_SURROGATE;
            }
        }
        result
    }

    /// Getting the character with extended name of the form `<....>`; -1
    /// when not found, -2 when the name is not extended.
    fn get_extended_char(name: &[u16], choice: i32) -> i32 {
        if name[0] == b'<' as u16 {
            if choice == UCharacterNameChoice::EXTENDED_CHAR_NAME {
                let end_index = name.len() as i32 - 1;
                if name[end_index as usize] == b'>' as u16 {
                    let mut start_index =
                        crate::jdk::last_index_of_unit(name, b'-' as u16, name.len() as i32 - 1);
                    if start_index >= 0 {
                        // We've got a category.
                        start_index += 1;

                        // There should be 1 to 8 hex digits.
                        let hex_length = end_index - start_index;
                        if !(1..=8).contains(&hex_length) {
                            return -1;
                        }
                        let result = match crate::jdk::parse_int(
                            &name[start_index as usize..end_index as usize],
                            16,
                        ) {
                            Ok(r) => r,
                            Err(_) => return -1,
                        };
                        if !(0..=0x10ffff).contains(&result) {
                            return -1;
                        }
                        // Now validate the category name. We could use a
                        // binary search, or a trie, if we really wanted to.
                        let char_type = Self::get_type(result);
                        let ty = &name[1..(start_index - 1) as usize];
                        for (i, tn) in TYPE_NAMES.iter().enumerate() {
                            if ty == units(tn).as_slice() {
                                if char_type == i as i32 {
                                    return result;
                                }
                                break;
                            }
                        }
                    }
                }
            }
            return -1;
        }
        -2
    }

    // sets of name characters, maximum name lengths -----------------------

    fn add_char(set: &mut [i32; 8], ch: u16) {
        set[(ch >> 5) as usize] |= 1 << (ch & 0x1f);
    }

    fn contains_char(set: &[i32; 8], ch: u16) -> bool {
        (set[(ch >> 5) as usize] & (1 << (ch & 0x1f))) != 0
    }

    fn add_str(set: &mut [i32; 8], str: &[u16]) -> i32 {
        let result = str.len() as i32;
        for i in (0..str.len()).rev() {
            Self::add_char(set, str[i]);
        }
        result
    }

    fn add_algorithm_name(&self, s: &mut NameSets, mut maxlength: i32) -> i32 {
        for i in (0..self.m_algorithm.len()).rev() {
            let result = self.m_algorithm[i].add(&mut s.name_set, maxlength);
            if result > maxlength {
                maxlength = result;
            }
        }
        maxlength
    }

    fn add_extended_name(s: &mut NameSets, mut maxlength: i32) -> i32 {
        for i in (0..TYPE_NAMES.len()).rev() {
            // for each category, count the length of the category name
            // plus 9 =
            // 2 for <>
            // 1 for -
            // 6 for most hex digits per code point
            let length = 9 + Self::add_str(&mut s.name_set, &units(TYPE_NAMES[i]));
            if length > maxlength {
                maxlength = length;
            }
        }
        maxlength
    }

    /// Adds names of a group to the argument set; returns
    /// `[name length, parsed length]`.
    fn add_group_name_line(
        &self,
        offset: i32,
        length: i32,
        tokenlength: &mut [i8],
        set: &mut [i32; 8],
    ) -> [i32; 2] {
        let mut resultnlength = 0;
        let mut resultplength = 0;
        while resultplength < length {
            let mut b =
                (self.m_groupstring[(offset + resultplength) as usize] as i32 & 0xff) as u16;
            resultplength += 1;
            if b == b';' as u16 {
                break;
            }

            if b as usize >= self.m_tokentable.len() {
                Self::add_char(set, b); // implicit letter
                resultnlength += 1;
            } else {
                let mut token = self.m_tokentable[(b & 0x00ff) as usize];
                if token == 0xFFFE {
                    // this is a lead byte for a double-byte token
                    b = (b << 8)
                        | (self.m_groupstring[(offset + resultplength) as usize] as i32 & 0x00ff)
                            as u16;
                    token = self.m_tokentable[b as usize];
                    resultplength += 1;
                }
                if token == 0xFFFF {
                    Self::add_char(set, b);
                    resultnlength += 1;
                } else {
                    // count token word
                    // use cached token length
                    let mut tlength = tokenlength[b as usize];
                    if tlength == 0 {
                        let mut buf = Vec::new();
                        UCharacterUtility::get_null_term_byte_sub_string(
                            &mut buf,
                            &self.m_tokenstring,
                            token as i32,
                        );
                        tlength = Self::add_str(set, &buf) as i8;
                        tokenlength[b as usize] = tlength;
                    }
                    resultnlength += tlength as i32;
                }
            }
        }
        [resultnlength, resultplength]
    }

    /// Adds names of all group to the argument set.
    fn add_group_name(&self, s: &mut NameSets, mut maxlength: i32) {
        let mut maxisolength = 0;
        let mut offsets = [0u16; LINES_PER_GROUP as usize + 2];
        let mut lengths = [0u16; LINES_PER_GROUP as usize + 2];
        let mut tokenlengths = vec![0i8; self.m_tokentable.len()];

        // enumerate all groups
        // for (int i = m_groupcount_ - 1; i >= 0; i --) {
        for i in 0..self.m_groupcount {
            let offset = self.get_group_lengths(i, &mut offsets, &mut lengths);
            // enumerate all lines in each group
            // for (int linenumber = LINES_PER_GROUP_ - 1; linenumber >= 0;
            //    linenumber --) {
            for linenumber in 0..LINES_PER_GROUP as usize {
                let mut lineoffset = offset + offsets[linenumber] as i32;
                let mut length = lengths[linenumber] as i32;
                if length == 0 {
                    continue;
                }

                // read regular name
                let mut parsed = self.add_group_name_line(
                    lineoffset,
                    length,
                    &mut tokenlengths,
                    &mut s.name_set,
                );
                if parsed[0] > maxlength {
                    // 0 for name length
                    maxlength = parsed[0];
                }
                lineoffset += parsed[1];
                if parsed[1] >= length {
                    // 1 for parsed group string length
                    continue;
                }
                length -= parsed[1];
                // read Unicode 1.0 name
                parsed = self.add_group_name_line(
                    lineoffset,
                    length,
                    &mut tokenlengths,
                    &mut s.name_set,
                );
                if parsed[0] > maxlength {
                    // 0 for name length
                    maxlength = parsed[0];
                }
                lineoffset += parsed[1];
                if parsed[1] >= length {
                    // 1 for parsed group string length
                    continue;
                }
                length -= parsed[1];
                // read ISO comment
                parsed = self.add_group_name_line(
                    lineoffset,
                    length,
                    &mut tokenlengths,
                    &mut s.iso_comment_set,
                );
                if parsed[1] > maxisolength {
                    maxisolength = length;
                }
            }
        }

        // set gMax... - name length last for threading
        s.max_iso_comment_length = maxisolength;
        s.max_name_length = maxlength;
    }

    /// Sets up the name sets and the calculation of the maximum lengths.
    fn init_name_sets_lengths(&self, s: &mut NameSets) -> bool {
        if s.max_name_length > 0 {
            return true;
        }

        let extra = units("0123456789ABCDEF<>-");
        // set hex digits, used in various names, and <>-, used in extended
        // names
        for i in (0..extra.len()).rev() {
            Self::add_char(&mut s.name_set, extra[i]);
        }

        // set sets and lengths from algorithmic names
        s.max_name_length = self.add_algorithm_name(s, 0);
        // set sets and lengths from extended names
        s.max_name_length = Self::add_extended_name(s, s.max_name_length);
        // set sets and lengths from group names, set global maximum values
        let m = s.max_name_length;
        self.add_group_name(s, m);
        true
    }

    /// Converts the char set cset into a Unicode set uset.
    fn convert(&self, iso: bool, uset: &mut UnicodeSet) -> Result<()> {
        uset.clear()?;
        let mut s = self.sets.lock().unwrap_or_else(|e| e.into_inner());
        if !self.init_name_sets_lengths(&mut s) {
            return Ok(());
        }
        let set = if iso { s.iso_comment_set } else { s.name_set };

        // build a char string with all chars that are used in character names
        let mut c: u16 = 255;
        while c > 0 {
            if Self::contains_char(&set, c) {
                uset.add(c as i32)?;
            }
            c -= 1;
        }
        Ok(())
    }
}

/// `String.toLowerCase(Locale.ENGLISH)`.
fn english_lower(name: &[u16]) -> Vec<u16> {
    rucene_analysis_common::jdk::string::to_lower_case(
        name,
        &rucene_analysis_common::jdk::locale::Locale::english(),
    )
}

/// `String.toUpperCase(Locale.ENGLISH)`.
fn english_upper(name: &[u16]) -> Vec<u16> {
    rucene_analysis_common::jdk::character::to_upper_case_root(name)
}

/// Algorithmic name class.
///
/// Equivalent to `UCharacterName.AlgorithmName` (package-private). Java's
/// synchronized scratch buffers are local per call.
#[derive(Debug, Clone, Default)]
pub(crate) struct AlgorithmName {
    m_rangestart: i32,
    m_rangeend: i32,
    m_type: i8,
    m_variant: i8,
    m_factor: Vec<u16>,
    m_prefix: Vec<u16>,
    m_factorstring: Vec<i8>,
}

impl AlgorithmName {
    /// `TYPE_0_`.
    pub(crate) const TYPE_0: i8 = 0;
    /// `TYPE_1_`.
    pub(crate) const TYPE_1: i8 = 1;

    /// Sets the information for accessing the algorithmic names.
    fn set_info(&mut self, rangestart: i32, rangeend: i32, ty: i8, variant: i8) -> bool {
        if rangestart >= UCharacter::MIN_VALUE
            && rangestart <= rangeend
            && rangeend <= UCharacter::MAX_VALUE
            && (ty == Self::TYPE_0 || ty == Self::TYPE_1)
        {
            self.m_rangestart = rangestart;
            self.m_rangeend = rangeend;
            self.m_type = ty;
            self.m_variant = variant;
            return true;
        }
        false
    }

    /// Sets the factor data.
    fn set_factor(&mut self, factor: Vec<u16>) -> bool {
        if factor.len() == self.m_variant as usize {
            self.m_factor = factor;
            return true;
        }
        false
    }

    /// Sets the name prefix.
    fn set_prefix(&mut self, prefix: Vec<u16>) -> bool {
        if !prefix.is_empty() {
            self.m_prefix = prefix;
            return true;
        }
        false
    }

    /// Sets the variant factorized name data.
    fn set_factor_string(&mut self, string: Vec<i8>) -> bool {
        // factor and variant string can be empty for things like
        // hanggul code points
        self.m_factorstring = string;
        true
    }

    /// Checks if code point lies in Algorithm object at index.
    fn contains(&self, ch: i32) -> bool {
        self.m_rangestart <= ch && ch <= self.m_rangeend
    }

    /// Appends algorithm name of code point into StringBuffer.
    fn append_name(&self, ch: i32, str: &mut Vec<u16>) {
        str.extend_from_slice(&self.m_prefix);
        match self.m_type {
            Self::TYPE_0 => {
                // prefix followed by hex digits indicating variants
                str.extend(units(&Utility::hex_places(
                    ch as i64,
                    self.m_variant as i32,
                )));
            }
            Self::TYPE_1 => {
                // prefix followed by factorized-elements
                let mut offset = ch - self.m_rangestart;
                let mut indexes = vec![0i32; 256];

                // write elements according to the factors
                // the factorized elements are determined by modulo
                // arithmetic
                let mut i = self.m_variant as i32 - 1;
                while i > 0 {
                    let factor = self.m_factor[i as usize] as i32 & 0x00FF;
                    indexes[i as usize] = offset % factor;
                    offset /= factor;
                    i -= 1;
                }

                // we don't need to calculate the last modulus because
                // start <= code <= end guarantees here that
                // code <= factors[0]
                indexes[0] = offset;

                // joining up the factorized strings
                if let Some(s) = self.get_factor_string(&indexes, self.m_variant as i32) {
                    str.extend_from_slice(&s);
                } else {
                    // StringBuffer.append((String) null) appends "null".
                    str.extend(units("null"));
                }
            }
            _ => {}
        }
    }

    /// Gets the character for the argument algorithmic name.
    fn get_char(&self, name: &[u16]) -> i32 {
        let prefixlen = self.m_prefix.len();
        if name.len() < prefixlen || self.m_prefix.as_slice() != &name[..prefixlen] {
            return -1;
        }

        match self.m_type {
            Self::TYPE_0 => {
                match crate::jdk::parse_int(&name[prefixlen..], 16) {
                    Ok(result) => {
                        // does it fit into the range?
                        if self.m_rangestart <= result && result <= self.m_rangeend {
                            return result;
                        }
                    }
                    Err(_) => return -1,
                }
            }
            Self::TYPE_1 => {
                // repetitative suffix name comparison done here
                // offset is the character code - start
                let mut indexes = vec![0i32; 256];
                for ch in self.m_rangestart..=self.m_rangeend {
                    let mut offset = ch - self.m_rangestart;

                    let mut i = self.m_variant as i32 - 1;
                    while i > 0 {
                        let factor = self.m_factor[i as usize] as i32 & 0x00FF;
                        indexes[i as usize] = offset % factor;
                        offset /= factor;
                        i -= 1;
                    }

                    // we don't need to calculate the last modulus
                    // because start <= code <= end guarantees here that
                    // code <= factors[0]
                    indexes[0] = offset;

                    // joining up the factorized strings
                    if self.compare_factor_string(
                        &indexes,
                        self.m_variant as i32,
                        name,
                        prefixlen as i32,
                    ) {
                        return ch;
                    }
                }
            }
            _ => {}
        }

        -1
    }

    /// Adds all chars in the set of algorithmic names into the set.
    fn add(&self, set: &mut [i32; 8], maxlength: i32) -> i32 {
        // prefix length
        let mut length = UCharacterName::add_str(set, &self.m_prefix);
        match self.m_type {
            Self::TYPE_0 => {
                // name = prefix + (range->variant)-digit hex number
                // the hex digits have already been added to the set
                length += self.m_variant as i32;
            }
            Self::TYPE_1 => {
                // name = prefix factorized-elements
                // get the set and maximum factor suffix length for each
                // factor
                let mut i = self.m_variant as i32 - 1;
                while i > 0 {
                    let mut maxfactorlength = 0;
                    let mut count = 0;
                    let mut factor = self.m_factor[i as usize] as i32;
                    while factor > 0 {
                        let mut buf = Vec::new();
                        count = UCharacterUtility::get_null_term_byte_sub_string(
                            &mut buf,
                            &self.m_factorstring,
                            count,
                        );
                        UCharacterName::add_str(set, &buf);
                        if buf.len() as i32 > maxfactorlength {
                            maxfactorlength = buf.len() as i32;
                        }
                        factor -= 1;
                    }
                    length += maxfactorlength;
                    i -= 1;
                }
            }
            _ => {}
        }
        if length > maxlength {
            return length;
        }
        maxlength
    }

    /// Gets the indexth string in each of the argument factor block.
    fn get_factor_string(&self, index: &[i32], length: i32) -> Option<Vec<u16>> {
        let mut size = self.m_factor.len() as i32;
        if length != size {
            return None;
        }

        let mut buf = Vec::new();
        let mut count = 0;
        size -= 1;
        for i in 0..=size {
            let factor = self.m_factor[i as usize] as i32;
            count = UCharacterUtility::skip_null_term_byte_sub_string(
                &self.m_factorstring,
                count,
                index[i as usize],
            );
            count = UCharacterUtility::get_null_term_byte_sub_string(
                &mut buf,
                &self.m_factorstring,
                count,
            );
            if i != size {
                count = UCharacterUtility::skip_null_term_byte_sub_string(
                    &self.m_factorstring,
                    count,
                    factor - index[i as usize] - 1,
                );
            }
        }
        Some(buf)
    }

    /// Compares the indexth string in each of the argument factor block
    /// with the argument string.
    fn compare_factor_string(&self, index: &[i32], length: i32, str: &[u16], offset: i32) -> bool {
        let mut size = self.m_factor.len() as i32;
        if length != size {
            return false;
        }

        let mut count = 0;
        let mut strcount = offset;
        size -= 1;
        for i in 0..=size {
            let factor = self.m_factor[i as usize] as i32;
            count = UCharacterUtility::skip_null_term_byte_sub_string(
                &self.m_factorstring,
                count,
                index[i as usize],
            );
            strcount = UCharacterUtility::compare_null_term_byte_sub_string(
                str,
                &self.m_factorstring,
                strcount,
                count,
            );
            if strcount < 0 {
                return false;
            }

            if i != size {
                count = UCharacterUtility::skip_null_term_byte_sub_string(
                    &self.m_factorstring,
                    count,
                    factor - index[i as usize],
                );
            }
        }
        strcount == str.len() as i32
    }
}

/// Internal reader class for ICU data file uname.dat containing Unicode
/// codepoint name data.
///
/// Equivalent to `com.ibm.icu.impl.UCharacterNameReader` (package-private).
struct UCharacterNameReader {
    m_byte_buffer: ByteBuffer,
    m_tokenstringindex: i32,
    m_groupindex: i32,
    m_groupstringindex: i32,
    m_algnamesindex: i32,
}

/// Size of the group information block.
const GROUP_INFO_SIZE: i32 = 3;
/// Size of an algorithmic name information group.
const ALG_INFO_SIZE: i32 = 12;
/// File format id that this class understands ("unam").
const DATA_FORMAT_ID: i32 = 0x756E616D;

fn name_data_acceptable(version: &[u8; 4]) -> bool {
    version[0] == 1
}

fn get_bytes(bb: &mut ByteBuffer, size: i32) -> Result<Vec<i8>> {
    let mut b = vec![0u8; size.max(0) as usize];
    if size < 0 {
        return Err(LuceneError::unchecked(
            "java.lang.NegativeArraySizeException",
            size.to_string(),
        ));
    }
    bb.get_bytes(&mut b)?;
    Ok(b.into_iter().map(|x| x as i8).collect())
}

impl UCharacterNameReader {
    /// `UCharacterNameReader(ByteBuffer bytes)`.
    fn new(mut bytes: ByteBuffer) -> Result<Self> {
        ICUBinary::read_header(
            &mut bytes,
            DATA_FORMAT_ID,
            Some(&name_data_acceptable as &dyn Fn(&[u8; 4]) -> bool),
        )?;
        Ok(Self {
            m_byte_buffer: bytes,
            m_tokenstringindex: 0,
            m_groupindex: 0,
            m_groupstringindex: 0,
            m_algnamesindex: 0,
        })
    }

    /// `isDataVersionAcceptable(byte version[])`.
    #[allow(dead_code)]
    fn is_data_version_acceptable(&self, version: &[u8; 4]) -> bool {
        name_data_acceptable(version)
    }

    /// `authenticate(byte dataformatid[], byte dataformatversion[])`.
    #[allow(dead_code)]
    fn authenticate(&self, dataformatid: &[i8], dataformatversion: &[u8; 4]) -> bool {
        ICUBinary::get_version_byte_array_from_compact_int(DATA_FORMAT_ID).as_slice()
            == dataformatid
            && name_data_acceptable(dataformatversion)
    }

    /// `read(UCharacterName data)`: reads the data into `data`.
    fn read(&mut self, data: &mut UCharacterName) -> Result<()> {
        let bb = &mut self.m_byte_buffer;
        // reading index
        self.m_tokenstringindex = bb.get_int()?;
        self.m_groupindex = bb.get_int()?;
        self.m_groupstringindex = bb.get_int()?;
        self.m_algnamesindex = bb.get_int()?;

        // reading tokens
        let count = bb.get_char()? as i32;
        let token = ICUBinary::get_chars(bb, count, 0)?;
        let size = self.m_groupindex - self.m_tokenstringindex;
        let tokenstr = get_bytes(bb, size)?;
        data.set_token(token, tokenstr);

        // reading the group information records
        let count = bb.get_char()? as i32;
        data.set_group_count_size(count, GROUP_INFO_SIZE);
        let count = count * GROUP_INFO_SIZE;
        let group = ICUBinary::get_chars(bb, count, 0)?;

        let size = self.m_algnamesindex - self.m_groupstringindex;
        let groupstring = get_bytes(bb, size)?;
        data.set_group(group, groupstring);

        let count = bb.get_int()?;
        let mut alg = Vec::with_capacity(count.max(0) as usize);

        for _ in 0..count {
            let Some(an) = self.read_alg()? else {
                return Err(LuceneError::from(std::io::Error::other(
                    "unames.icu read error: Algorithmic names creation error",
                )));
            };
            alg.push(an);
        }
        data.set_algorithm(alg);
        Ok(())
    }

    /// Reads an individual record of AlgorithmNames.
    fn read_alg(&mut self) -> Result<Option<AlgorithmName>> {
        let bb = &mut self.m_byte_buffer;
        let mut result = AlgorithmName::default();
        let rangestart = bb.get_int()?;
        let rangeend = bb.get_int()?;
        let ty = bb.get()?;
        let variant = bb.get()?;
        if !result.set_info(rangestart, rangeend, ty, variant) {
            return Ok(None);
        }

        let mut size = bb.get_char()? as i32;
        if ty == AlgorithmName::TYPE_1 {
            let factor = ICUBinary::get_chars(bb, variant as i32, 0)?;

            result.set_factor(factor);
            size -= (variant as i32) << 1;
        }

        let mut prefix: Vec<u16> = Vec::new();
        let mut c = (bb.get()? as i32 & 0x00FF) as u16;
        while c != 0 {
            prefix.push(c);
            c = (bb.get()? as i32 & 0x00FF) as u16;
        }

        let plen = prefix.len() as i32;
        result.set_prefix(prefix);

        size -= ALG_INFO_SIZE + plen + 1;

        if size > 0 {
            let string = get_bytes(bb, size)?;
            result.set_factor_string(string);
        }
        Ok(Some(result))
    }
}
