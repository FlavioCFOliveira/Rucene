// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.text.NameUnicodeTransliterator`.

use std::sync::Arc;

use rucene::error::Result;

use crate::jdk::units;
use crate::lang::u_character::UCharacter;
use crate::r#impl::pattern_props::PatternProps;
use crate::r#impl::u_character_name::UCharacterName;
use crate::r#impl::utility::Utility;
use crate::text::replaceable::Replaceable;
use crate::text::transliterator::{factory, register_factory_in, transliterator_boilerplate, Position, Transliterator, TransliteratorBase};
use crate::text::transliterator_registry::TransliteratorRegistry;
use crate::text::unicode_filter::UnicodeFilter;
use crate::text::unicode_name_transliterator::UnicodeNameTransliterator;
use crate::text::unicode_set::UnicodeSet;
use crate::text::utf16::UTF16;

/// A transliterator that performs name to character mapping.
///
/// Equivalent to the package-private class
/// `com.ibm.icu.text.NameUnicodeTransliterator`.
#[derive(Debug)]
pub struct NameUnicodeTransliterator {
    base: TransliteratorBase,
}

impl NameUnicodeTransliterator {
    /// `_ID`.
    pub(crate) const ID: &'static str = "Name-Any";

    /// `OPEN_PAT`.
    pub(crate) const OPEN_PAT: &'static str = "\\N~{~";
    /// `OPEN_DELIM`.
    pub(crate) const OPEN_DELIM: u16 = b'\\' as u16; // first char of OPEN_PAT
    /// `CLOSE_DELIM`.
    pub(crate) const CLOSE_DELIM: u16 = b'}' as u16;
    /// `SPACE`.
    pub(crate) const SPACE: u16 = b' ' as u16;

    /// `register()`: system registration hook (on the registry under
    /// construction).
    pub(crate) fn register(registry: &TransliteratorRegistry) -> Result<()> {
        register_factory_in(
            registry,
            &units(Self::ID),
            factory(|_id| Ok(Some(Arc::new(NameUnicodeTransliterator::new(None)?) as Arc<dyn Transliterator>))),
        )
    }

    /// `NameUnicodeTransliterator(UnicodeFilter filter)`: constructs a
    /// transliterator.
    ///
    /// # Errors
    ///
    /// The errors of `setFilter`.
    pub fn new(filter: Option<&dyn UnicodeFilter>) -> Result<Self> {
        Ok(Self {
            base: TransliteratorBase::new(units(Self::ID), filter)?,
        })
    }
}

impl Transliterator for NameUnicodeTransliterator {
    transliterator_boilerplate!();

    /// `handleTransliterate(Replaceable text, Position offsets, boolean
    /// isIncremental)`.
    fn handle_transliterate(&self, text: &dyn Replaceable, offsets: &mut Position, is_incremental: bool) -> Result<()> {
        let max_len = UCharacterName::instance().get_max_char_name_length() + 1; // allow for temporary trailing space

        let mut name: Vec<u16> = Vec::with_capacity(max_len.max(0) as usize);

        // Get the legal character set
        let mut legal = UnicodeSet::new();
        UCharacterName::instance().get_char_name_characters(&mut legal)?;

        let mut cursor = offsets.start;
        let mut limit = offsets.limit;

        // Modes:
        // 0 - looking for open delimiter
        // 1 - after open delimiter
        let mut mode = 0;
        let mut open_pos = -1; // open delim candidate pos

        let open_pat = units(Self::OPEN_PAT);
        let mut c;
        while cursor < limit {
            c = text.char32_at(cursor);

            match mode {
                0 => {
                    // looking for open delimiter
                    if c == Self::OPEN_DELIM as i32 {
                        // quick check first
                        open_pos = cursor;
                        let i = Utility::parse_pattern_replaceable(&open_pat, text, cursor, limit);
                        if i >= 0 && i < limit {
                            mode = 1;
                            name.clear();
                            cursor = i;
                            continue; // *** reprocess char32At(cursor)
                        }
                    }
                }
                1 => {
                    // after open delimiter
                    // Look for legal chars.  If \s+ is found, convert it
                    // to a single space.  If closeDelimiter is found, exit
                    // the loop.  If any other character is found, exit the
                    // loop.  If the limit is reached, exit the loop.

                    // Convert \s+ => SPACE.  This assumes there are no
                    // runs of >1 space characters in names.
                    if PatternProps::is_white_space(c) {
                        // Ignore leading whitespace
                        if !name.is_empty() && name[name.len() - 1] != Self::SPACE {
                            name.push(Self::SPACE);
                            // If we are too long then abort.  maxLen includes
                            // temporary trailing space, so use '>'.
                            if name.len() as i32 > max_len {
                                mode = 0;
                            }
                        }
                    } else if c == Self::CLOSE_DELIM as i32 {
                        let mut len = name.len();

                        // Delete trailing space, if any
                        if len > 0 && name[len - 1] == Self::SPACE {
                            len -= 1;
                            name.truncate(len);
                        }

                        c = UCharacter::get_char_from_extended_name(&name)?;
                        if c != -1 {
                            // Lookup succeeded

                            // assert(UTF16.getCharCount(CLOSE_DELIM) == 1);
                            cursor += 1; // advance over CLOSE_DELIM

                            let str = UTF16::value_of(c)?;
                            text.replace(open_pos, cursor, &str);

                            // Adjust indices for the change in the length of
                            // the string.  Do not assume that str.length() ==
                            // 1, in case of surrogates.
                            let delta = cursor - open_pos - str.len() as i32;
                            cursor -= delta;
                            limit -= delta;
                            // assert(cursor == openPos + str.length());
                        }
                        // If the lookup failed, we leave things as-is and
                        // still switch to mode 0 and continue.
                        mode = 0;
                        open_pos = -1; // close off candidate
                        continue; // *** reprocess char32At(cursor)
                    } else if legal.contains_valid(c) {
                        UTF16::append(&mut name, c)?;
                        // If we go past the longest possible name then abort.
                        // maxLen includes temporary trailing space, so use '>='.
                        if name.len() as i32 >= max_len {
                            mode = 0;
                        }
                    }
                    // Invalid character
                    else {
                        cursor -= 1; // Backup and reprocess this character
                        mode = 0;
                    }
                }
                _ => {}
            }

            cursor += UTF16::get_char_count(c);
        }

        offsets.context_limit += limit - offsets.limit;
        offsets.limit = limit;
        // In incremental mode, only advance the cursor up to the last
        // open delimiter candidate.
        offsets.start = if is_incremental && open_pos >= 0 { open_pos } else { cursor };
        Ok(())
    }

    /// `addSourceTargetSet(UnicodeSet inputFilter, UnicodeSet sourceSet,
    /// UnicodeSet targetSet)`.
    fn add_source_target_set(&self, input_filter: &UnicodeSet, source_set: &mut UnicodeSet, target_set: &mut UnicodeSet) -> Result<()> {
        let my_filter = self.get_filter_as_unicode_set(input_filter)?;
        if !my_filter.contains_all_str(&units(UnicodeNameTransliterator::OPEN_DELIM)) || !my_filter.contains(Self::CLOSE_DELIM as i32)? {
            return Ok(()); // we have to contain both prefix and suffix
        }
        let mut items = UnicodeSet::new();
        items
            .add_all_range(b'0' as i32, b'9' as i32)?
            .add_all_range(b'A' as i32, b'F' as i32)?
            .add_all_range(b'a' as i32, b'z' as i32)? // for controls
            .add(b'<' as i32)?
            .add(b'>' as i32)? // for controls
            .add(b'(' as i32)?
            .add(b')' as i32)? // for controls
            .add(b'-' as i32)?
            .add(b' ' as i32)?
            .add_all_str(&units(UnicodeNameTransliterator::OPEN_DELIM))?
            .add(Self::CLOSE_DELIM as i32)?;
        items.retain_all(&my_filter)?;
        if items.size() > 0 {
            source_set.add_all(&items)?;
            // could produce any character
            target_set.add_all_range(0, 0x10FFFF)?;
        }
        Ok(())
    }
}
