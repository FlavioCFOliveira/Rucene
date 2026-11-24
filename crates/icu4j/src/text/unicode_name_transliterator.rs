// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.text.UnicodeNameTransliterator`.

use std::sync::Arc;

use rucene::error::Result;

use crate::jdk::units;
use crate::lang::u_character::UCharacter;
use crate::text::replaceable::Replaceable;
use crate::text::transliterator::{
    factory, register_factory_in, transliterator_boilerplate, Position, Transliterator,
    TransliteratorBase,
};
use crate::text::transliterator_registry::TransliteratorRegistry;
use crate::text::unicode_filter::UnicodeFilter;
use crate::text::unicode_set::UnicodeSet;
use crate::text::utf16::UTF16;

/// A transliterator that performs character to name mapping. It generates
/// the Perl syntax \N{name}.
///
/// Equivalent to the package-private class
/// `com.ibm.icu.text.UnicodeNameTransliterator`.
#[derive(Debug)]
pub struct UnicodeNameTransliterator {
    base: TransliteratorBase,
}

impl UnicodeNameTransliterator {
    /// `_ID`.
    pub(crate) const ID: &'static str = "Any-Name";

    /// `OPEN_DELIM`.
    pub(crate) const OPEN_DELIM: &'static str = "\\N{";
    /// `CLOSE_DELIM`.
    pub(crate) const CLOSE_DELIM: u16 = b'}' as u16;
    /// `OPEN_DELIM_LEN`.
    pub(crate) const OPEN_DELIM_LEN: usize = 3;

    /// `register()`: system registration hook (on the registry under
    /// construction).
    pub(crate) fn register(registry: &TransliteratorRegistry) -> Result<()> {
        register_factory_in(
            registry,
            &units(Self::ID),
            factory(|_id| {
                Ok(Some(
                    Arc::new(UnicodeNameTransliterator::new(None)?) as Arc<dyn Transliterator>
                ))
            }),
        )
    }

    /// `UnicodeNameTransliterator(UnicodeFilter filter)`: constructs a
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

impl Transliterator for UnicodeNameTransliterator {
    transliterator_boilerplate!();

    /// `handleTransliterate(Replaceable text, Position offsets, boolean
    /// isIncremental)`.
    fn handle_transliterate(
        &self,
        text: &dyn Replaceable,
        offsets: &mut Position,
        _is_incremental: bool,
    ) -> Result<()> {
        let mut cursor = offsets.start;
        let mut limit = offsets.limit;

        let mut str = units(Self::OPEN_DELIM);
        let mut len;

        while cursor < limit {
            let c = text.char32_at(cursor);
            if let Some(name) = UCharacter::get_extended_name(c) {
                str.truncate(Self::OPEN_DELIM_LEN);
                str.extend_from_slice(&name);
                str.push(Self::CLOSE_DELIM);

                let clen = UTF16::get_char_count(c);
                text.replace(cursor, cursor + clen, &str);
                len = str.len() as i32;
                cursor += len; // advance cursor by 1 and adjust for new text
                limit += len - clen; // change in length
            } else {
                cursor += 1;
            }
        }

        offsets.context_limit += limit - offsets.limit;
        offsets.limit = limit;
        offsets.start = cursor;
        Ok(())
    }

    /// `addSourceTargetSet(UnicodeSet inputFilter, UnicodeSet sourceSet,
    /// UnicodeSet targetSet)`.
    fn add_source_target_set(
        &self,
        input_filter: &UnicodeSet,
        source_set: &mut UnicodeSet,
        target_set: &mut UnicodeSet,
    ) -> Result<()> {
        let my_filter = self.get_filter_as_unicode_set(input_filter)?;
        if my_filter.size() > 0 {
            source_set.add_all(&my_filter)?;
            target_set
                .add_all_range(b'0' as i32, b'9' as i32)?
                .add_all_range(b'A' as i32, b'Z' as i32)?
                .add(b'-' as i32)?
                .add(b' ' as i32)?
                .add_all_str(&units(Self::OPEN_DELIM))?
                .add(Self::CLOSE_DELIM as i32)?
                .add_all_range(b'a' as i32, b'z' as i32)? // for controls
                .add(b'<' as i32)?
                .add(b'>' as i32)? // for controls
                .add(b'(' as i32)?
                .add(b')' as i32)?; // for controls
        }
        Ok(())
    }
}
