// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.UBiDiProps`.

use std::sync::LazyLock;

use rucene::error::{LuceneError, Result};

use crate::jdk::ByteBuffer;
use crate::lang::u_character::JoiningGroup;
use crate::lang::u_property::UProperty;
use crate::r#impl::icu_binary::ICUBinary;
use crate::r#impl::trie2_16::Trie2_16;
use crate::text::unicode_set::UnicodeSet;

fn io_exception(message: String) -> LuceneError {
    LuceneError::from(std::io::Error::other(message))
}

/// Low-level Unicode bidi/shaping properties access.
///
/// Equivalent to `com.ibm.icu.impl.UBiDiProps` (reads `ubidi.icu`).
#[derive(Debug)]
pub struct UBiDiProps {
    indexes: Vec<i32>,
    mirrors: Vec<i32>,
    jg_array: Vec<i8>,
    jg_array2: Vec<i8>,
    trie: Trie2_16,
}

/// `INSTANCE`. A failure of Java's static initializer surfaces as
/// `ExceptionInInitializerError`, here a panic.
static INSTANCE: LazyLock<UBiDiProps> = LazyLock::new(|| match UBiDiProps::new() {
    Ok(p) => p,
    Err(e) => panic!("java.lang.ExceptionInInitializerError: {e}"),
});

const DATA_FILE_NAME: &str = "ubidi.icu";
/* format "BiDi" */
const FMT: i32 = 0x42694469;

/* indexes into indexes[] */
//private static final int IX_INDEX_TOP=0;
//private static final int IX_LENGTH=1;
const IX_TRIE_SIZE: usize = 2;
const IX_MIRROR_LENGTH: usize = 3;

const IX_JG_START: usize = 4;
const IX_JG_LIMIT: usize = 5;
const IX_JG_START2: usize = 6; /* new in format version 2.2, ICU 54 */
const IX_JG_LIMIT2: usize = 7;

const IX_MAX_VALUES: usize = 15;
const IX_TOP: i32 = 16;

// definitions for 16-bit bidi/shaping properties word ----------------------

/* CLASS_SHIFT=0, */ /* bidi class: 5 bits (4..0) */
const JT_SHIFT: i32 = 5; /* joining type: 3 bits (7..5) */

const BPT_SHIFT: i32 = 8; /* Bidi_Paired_Bracket_Type(bpt): 2 bits (9..8) */

const JOIN_CONTROL_SHIFT: i32 = 10;
const BIDI_CONTROL_SHIFT: i32 = 11;

const IS_MIRRORED_SHIFT: i32 = 12; /* 'is mirrored' */
const MIRROR_DELTA_SHIFT: i32 = 13; /* bidi mirroring delta: 3 bits (15..13) */

const MAX_JG_SHIFT: i32 = 16; /* max JG value in indexes[MAX_VALUES_INDEX] bits 23..16 */

const CLASS_MASK: i32 = 0x0000001f;
const JT_MASK: i32 = 0x000000e0;
const BPT_MASK: i32 = 0x00000300;

const MAX_JG_MASK: i32 = 0x00ff0000;

const ESC_MIRROR_DELTA: i32 = -4;
//private static final int MIN_MIRROR_DELTA=-3;
//private static final int MAX_MIRROR_DELTA=3;

/* the source Unicode code point takes 21 bits (20..0) */
const MIRROR_INDEX_SHIFT: u32 = 21;
//private static final int MAX_MIRROR_INDEX=0x7ff;

fn get_class_from_props(props: i32) -> i32 {
    props & CLASS_MASK
}
fn get_flag_from_props(props: i32, shift: i32) -> bool {
    ((props >> shift) & 1) != 0
}
fn get_mirror_delta_from_props(props: i32) -> i32 {
    (props as i16 as i32) >> MIRROR_DELTA_SHIFT
}
fn get_mirror_code_point(m: i32) -> i32 {
    m & 0x1fffff
}
fn get_mirror_index(m: i32) -> i32 {
    ((m as u32) >> MIRROR_INDEX_SHIFT) as i32
}

fn is_acceptable(version: &[u8; 4]) -> bool {
    version[0] == 2
}

impl UBiDiProps {
    // constructors etc. --------------------------------------------------- ***

    // port of ubidi_openProps()
    fn new() -> Result<Self> {
        let mut bytes = ICUBinary::get_data(DATA_FILE_NAME)?
            .ok_or_else(|| LuceneError::null_pointer_with_message("ubidi.icu"))?;
        Self::read_data(&mut bytes)
    }

    fn read_data(bytes: &mut ByteBuffer) -> Result<Self> {
        // read the header
        ICUBinary::read_header(bytes, FMT, Some(&is_acceptable as &dyn Fn(&[u8; 4]) -> bool))?;

        // read indexes[]
        let count = bytes.get_int()?;
        if count < IX_TOP {
            return Err(io_exception(format!("indexes[0] too small in {DATA_FILE_NAME}")));
        }
        let mut indexes = vec![0i32; count as usize];

        indexes[0] = count;
        for i in 1..count as usize {
            indexes[i] = bytes.get_int()?;
        }

        // read the trie
        let trie = Trie2_16::create_from_serialized(bytes)?;
        let expected_trie_length = indexes[IX_TRIE_SIZE];
        let trie_length = trie.get_serialized_length();
        if trie_length > expected_trie_length {
            return Err(io_exception(format!("{DATA_FILE_NAME}: not enough bytes for the trie")));
        }
        // skip padding after trie bytes
        ICUBinary::skip_bytes(bytes, expected_trie_length - trie_length)?;

        // read mirrors[]
        let count = indexes[IX_MIRROR_LENGTH];
        let mirrors = if count > 0 { ICUBinary::get_ints(bytes, count, 0)? } else { Vec::new() };

        // read jgArray[]
        let count = indexes[IX_JG_LIMIT] - indexes[IX_JG_START];
        let mut b = vec![0u8; count as usize];
        bytes.get_bytes(&mut b)?;
        let jg_array: Vec<i8> = b.into_iter().map(|x| x as i8).collect();

        // read jgArray2[]
        let count = indexes[IX_JG_LIMIT2] - indexes[IX_JG_START2];
        let mut b = vec![0u8; count as usize];
        bytes.get_bytes(&mut b)?;
        let jg_array2: Vec<i8> = b.into_iter().map(|x| x as i8).collect();

        Ok(Self { indexes, mirrors, jg_array, jg_array2, trie })
    }

    /// `INSTANCE`.
    ///
    /// # Panics
    ///
    /// When the data cannot be read (Java's `ExceptionInInitializerError`).
    pub fn instance() -> &'static UBiDiProps {
        &INSTANCE
    }

    // set of property starts for UnicodeSet ------------------------------- ***

    /// `addPropertyStarts(UnicodeSet set)`.
    ///
    /// # Errors
    ///
    /// The errors of [`UnicodeSet::add`] (a frozen set).
    pub fn add_property_starts(&self, set: &mut UnicodeSet) -> Result<()> {
        /* add the start code point of each same-value range of the trie */
        let mut trie_iterator = self.trie.iterator();
        while trie_iterator.has_next() {
            let range = trie_iterator.next().expect("has_next");
            if range.lead_surrogate {
                break;
            }
            set.add(range.start_code_point)?;
        }

        /* add the code points from the bidi mirroring table */
        let length = self.indexes[IX_MIRROR_LENGTH];
        for i in 0..length as usize {
            let c = get_mirror_code_point(self.mirrors[i]);
            set.add_range(c, c + 1)?;
        }

        /* add the code points from the Joining_Group array where the value changes */
        let mut start = self.indexes[IX_JG_START];
        let mut limit = self.indexes[IX_JG_LIMIT];
        let mut jga = &self.jg_array;
        loop {
            let length = limit - start;
            let mut prev: i8 = 0;
            for i in 0..length as usize {
                let jg = jga[i];
                if jg != prev {
                    set.add(start)?;
                    prev = jg;
                }
                start += 1;
            }
            if prev != 0 {
                /* add the limit code point if the last value was not 0 (it is now start==limit) */
                set.add(limit)?;
            }
            if limit == self.indexes[IX_JG_LIMIT] {
                /* switch to the second Joining_Group range */
                start = self.indexes[IX_JG_START2];
                limit = self.indexes[IX_JG_LIMIT2];
                jga = &self.jg_array2;
            } else {
                break;
            }
        }

        /* add code points with hardcoded properties, plus the ones following them */

        /* (none right now) */
        Ok(())
    }

    // property access functions ------------------------------------------- ***

    /// `getMaxValue(int which)`.
    pub fn get_max_value(&self, which: i32) -> i32 {
        let max = self.indexes[IX_MAX_VALUES];
        match which {
            UProperty::BIDI_CLASS => max & CLASS_MASK,
            UProperty::JOINING_GROUP => (max & MAX_JG_MASK) >> MAX_JG_SHIFT,
            UProperty::JOINING_TYPE => (max & JT_MASK) >> JT_SHIFT,
            UProperty::BIDI_PAIRED_BRACKET_TYPE => (max & BPT_MASK) >> BPT_SHIFT,
            _ => -1, /* undefined */
        }
    }

    /// `getClass(int c)`.
    pub fn get_class(&self, c: i32) -> i32 {
        get_class_from_props(self.trie.get(c))
    }

    /// `isMirrored(int c)`.
    pub fn is_mirrored(&self, c: i32) -> bool {
        get_flag_from_props(self.trie.get(c), IS_MIRRORED_SHIFT)
    }

    fn get_mirror_props(&self, c: i32, props: i32) -> i32 {
        let delta = get_mirror_delta_from_props(props);
        if delta != ESC_MIRROR_DELTA {
            c + delta
        } else {
            /* look for mirror code point in the mirrors[] table */
            let length = self.indexes[IX_MIRROR_LENGTH];

            /* linear search */
            for i in 0..length as usize {
                let m = self.mirrors[i];
                let c2 = get_mirror_code_point(m);
                if c == c2 {
                    /* found c, return its mirror code point using the index in m */
                    return get_mirror_code_point(self.mirrors[get_mirror_index(m) as usize]);
                } else if c < c2 {
                    break;
                }
            }

            /* c not found, return it itself */
            c
        }
    }

    /// `getMirror(int c)`.
    pub fn get_mirror(&self, c: i32) -> i32 {
        let props = self.trie.get(c);
        self.get_mirror_props(c, props)
    }

    /// `isBidiControl(int c)`.
    pub fn is_bidi_control(&self, c: i32) -> bool {
        get_flag_from_props(self.trie.get(c), BIDI_CONTROL_SHIFT)
    }

    /// `isJoinControl(int c)`.
    pub fn is_join_control(&self, c: i32) -> bool {
        get_flag_from_props(self.trie.get(c), JOIN_CONTROL_SHIFT)
    }

    /// `getJoiningType(int c)`.
    pub fn get_joining_type(&self, c: i32) -> i32 {
        (self.trie.get(c) & JT_MASK) >> JT_SHIFT
    }

    /// `getJoiningGroup(int c)`.
    pub fn get_joining_group(&self, c: i32) -> i32 {
        let start = self.indexes[IX_JG_START];
        let limit = self.indexes[IX_JG_LIMIT];
        if start <= c && c < limit {
            return self.jg_array[(c - start) as usize] as i32 & 0xff;
        }
        let start = self.indexes[IX_JG_START2];
        let limit = self.indexes[IX_JG_LIMIT2];
        if start <= c && c < limit {
            return self.jg_array2[(c - start) as usize] as i32 & 0xff;
        }
        JoiningGroup::NO_JOINING_GROUP
    }

    /// `getPairedBracketType(int c)`.
    pub fn get_paired_bracket_type(&self, c: i32) -> i32 {
        (self.trie.get(c) & BPT_MASK) >> BPT_SHIFT
    }

    /// `getPairedBracket(int c)`.
    pub fn get_paired_bracket(&self, c: i32) -> i32 {
        let props = self.trie.get(c);
        if (props & BPT_MASK) == 0 {
            c
        } else {
            self.get_mirror_props(c, props)
        }
    }
}
