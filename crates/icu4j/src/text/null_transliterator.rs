// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.text.NullTransliterator`.

use rucene::error::Result;

use crate::jdk::units;
use crate::text::replaceable::Replaceable;
use crate::text::transliterator::{
    transliterator_boilerplate, Position, Transliterator, TransliteratorBase,
};
use crate::text::unicode_set::UnicodeSet;

/// A transliterator that leaves text unchanged.
///
/// Equivalent to the package-private class
/// `com.ibm.icu.text.NullTransliterator`.
#[derive(Debug)]
pub struct NullTransliterator {
    base: TransliteratorBase,
}

impl Default for NullTransliterator {
    fn default() -> Self {
        Self::new()
    }
}

impl NullTransliterator {
    /// `SHORT_ID`: package accessible IDs for this transliterator.
    pub const SHORT_ID: &'static str = "Null";
    /// `_ID`.
    pub const ID: &'static str = "Any-Null";

    /// `NullTransliterator()`: constructs a transliterator.
    pub fn new() -> Self {
        Self {
            base: TransliteratorBase::with_id(units(Self::ID)),
        }
    }
}

impl Transliterator for NullTransliterator {
    transliterator_boilerplate!();

    /// `handleTransliterate(Replaceable text, Position offsets, boolean
    /// incremental)`.
    fn handle_transliterate(
        &self,
        _text: &dyn Replaceable,
        offsets: &mut Position,
        _incremental: bool,
    ) -> Result<()> {
        offsets.start = offsets.limit;
        Ok(())
    }

    /// `addSourceTargetSet(UnicodeSet inputFilter, UnicodeSet sourceSet,
    /// UnicodeSet targetSet)`.
    fn add_source_target_set(
        &self,
        _input_filter: &UnicodeSet,
        _source_set: &mut UnicodeSet,
        _target_set: &mut UnicodeSet,
    ) -> Result<()> {
        // do nothing
        Ok(())
    }
}
