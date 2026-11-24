// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.text.LowercaseTransliterator`.

use std::sync::{Arc, Mutex};

use rucene::error::Result;

use crate::jdk::units;
use crate::lang::u_character::UCharacter;
use crate::r#impl::u_case_props::UCaseProps;
use crate::text::replaceable::Replaceable;
use crate::text::replaceable_context_iterator::ReplaceableContextIterator;
use crate::text::source_target_utility::{FnTransform, SourceTargetUtility};
use crate::text::transliterator::{
    factory, register_factory_in, register_special_inverse, transliterator_boilerplate, Position,
    Transliterator, TransliteratorBase,
};
use crate::text::transliterator_registry::TransliteratorRegistry;
use crate::text::unicode_set::UnicodeSet;
use crate::text::utf16::UTF16;
use crate::util::u_locale::{ULocale, US};

/// A transliterator that performs locale-sensitive toLower() case mapping.
///
/// Equivalent to the package-private class `com.ibm.icu.text.LowercaseTransliterator`.
/// Java's `iter` and `result` fields, reset at the start of every
/// `synchronized` call, are locals of the call here (the iterator borrows
/// the text for the call only).
#[derive(Debug)]
pub struct LowercaseTransliterator {
    base: TransliteratorBase,
    locale: ULocale,
    csp: &'static UCaseProps,
    case_locale: i32,
    /// `sourceTargetUtility`, created lazily under `synchronized (this)`.
    source_target_utility: Mutex<Option<Arc<SourceTargetUtility>>>,
}

impl LowercaseTransliterator {
    /// `_ID`: package accessible ID.
    pub(crate) const ID: &'static str = "Any-Lower";

    /// `register()`: system registration hook (on the registry under
    /// construction).
    pub(crate) fn register(registry: &TransliteratorRegistry) -> Result<()> {
        register_factory_in(
            registry,
            &units(Self::ID),
            factory(|_id| {
                Ok(Some(Arc::new(LowercaseTransliterator::new((*US).clone())?)
                    as Arc<dyn Transliterator>))
            }),
        )?;
        register_special_inverse(&units("Lower"), &units("Upper"), true);
        Ok(())
    }

    /// `LowercaseTransliterator(ULocale loc)`: constructs a transliterator.
    ///
    /// # Errors
    ///
    /// None in practice (`setMaximumContextLength` of a constant).
    pub fn new(loc: ULocale) -> Result<Self> {
        let base = TransliteratorBase::with_id(units(Self::ID));
        let case_locale = UCaseProps::get_case_locale_ulocale(&loc);
        Ok(Self {
            base,
            locale: loc,
            csp: UCaseProps::instance(),
            case_locale,
            source_target_utility: Mutex::new(None),
        })
    }
}

impl Transliterator for LowercaseTransliterator {
    transliterator_boilerplate!();

    /// `handleTransliterate(Replaceable text, Position offsets, boolean
    /// isIncremental)`.
    fn handle_transliterate(
        &self,
        text: &dyn Replaceable,
        offsets: &mut Position,
        is_incremental: bool,
    ) -> Result<()> {
        // (csp is never null: UCaseProps.INSTANCE)
        if offsets.start >= offsets.limit {
            return Ok(());
        }

        let mut iter = ReplaceableContextIterator::new();
        let mut result: Vec<u16> = Vec::new();
        iter.set_text(text);
        result.clear();

        let mut c;
        let mut delta;

        // Walk through original string
        // If there is a case change, modify corresponding position in replaceable

        iter.set_index(offsets.start);
        iter.set_limit(offsets.limit);
        iter.set_context_limits(offsets.context_start, offsets.context_limit);
        loop {
            c = iter.next_case_map_cp();
            if c < 0 {
                break;
            }
            c = self
                .csp
                .to_full_lower(c, Some(&mut iter), &mut result, self.case_locale);

            if iter.did_reach_limit() && is_incremental {
                // the case mapping function tried to look beyond the context limit
                // wait for more input
                offsets.start = iter.get_case_map_cp_start();
                return Ok(());
            }

            /* decode the result */
            if c < 0 {
                /* c mapped to itself, no change */
                continue;
            } else if c <= UCaseProps::MAX_STRING_LENGTH {
                /* replace by the mapping string */
                delta = iter.replace(&result);
                result.clear();
            } else {
                /* replace by single-code point mapping */
                delta = iter.replace(&UTF16::value_of(c)?);
            }

            if delta != 0 {
                offsets.limit += delta;
                offsets.context_limit += delta;
            }
        }
        offsets.start = offsets.limit;
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
        let utility = {
            let mut guard = self
                .source_target_utility
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            match &*guard {
                Some(u) => u.clone(),
                None => {
                    let locale = self.locale.clone();
                    let u = Arc::new(SourceTargetUtility::new(Box::new(FnTransform(
                        move |source: &[u16]| {
                            Ok(UCharacter::to_lower_case_ulocale(&locale, source))
                        },
                    )))?);
                    *guard = Some(u.clone());
                    u
                }
            }
        };
        utility.add_source_target_set(self, input_filter, source_set, target_set)
    }
}
