// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.text.TitlecaseTransliterator`.

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

/// A transliterator that converts all letters (as defined by `UCharacter.isLetter()`) to lower case, except for those letters preceded by non-letters. The latter are converted to title case using `UCharacter.toTitleCase()`.
///
/// Equivalent to the package-private class `com.ibm.icu.text.TitlecaseTransliterator`.
/// Java's `iter` and `result` fields, reset at the start of every
/// `synchronized` call, are locals of the call here (the iterator borrows
/// the text for the call only).
#[derive(Debug)]
pub struct TitlecaseTransliterator {
    base: TransliteratorBase,
    locale: ULocale,
    csp: &'static UCaseProps,
    case_locale: i32,
    /// `sourceTargetUtility`, created lazily under `synchronized (this)`.
    source_target_utility: Mutex<Option<Arc<SourceTargetUtility>>>,
}

impl TitlecaseTransliterator {
    /// `_ID`: package accessible ID.
    pub(crate) const ID: &'static str = "Any-Title";

    /// `register()`: system registration hook (on the registry under
    /// construction).
    pub(crate) fn register(registry: &TransliteratorRegistry) -> Result<()> {
        register_factory_in(
            registry,
            &units(Self::ID),
            factory(|_id| {
                Ok(Some(Arc::new(TitlecaseTransliterator::new((*US).clone())?)
                    as Arc<dyn Transliterator>))
            }),
        )?;
        register_special_inverse(&units("Title"), &units("Lower"), false);
        Ok(())
    }

    /// `TitlecaseTransliterator(ULocale loc)`: constructs a transliterator.
    ///
    /// # Errors
    ///
    /// None in practice (`setMaximumContextLength` of a constant).
    pub fn new(loc: ULocale) -> Result<Self> {
        let base = TransliteratorBase::with_id(units(Self::ID));
        // Need to look back 2 characters in the case of "can't"
        base.set_maximum_context_length(2)?;
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

impl Transliterator for TitlecaseTransliterator {
    transliterator_boilerplate!();

    /// `handleTransliterate(Replaceable text, Position offsets, boolean
    /// isIncremental)`.
    fn handle_transliterate(
        &self,
        text: &dyn Replaceable,
        offsets: &mut Position,
        is_incremental: bool,
    ) -> Result<()> {
        // TODO reimplement, see ustrcase.c
        // using a real word break iterator
        //   instead of just looking for a transition between cased and uncased characters
        // call CaseMapTransliterator::handleTransliterate() for lowercasing? (set fMap)
        // needs to take isIncremental into account because case mappings are context-sensitive
        //   also detect when lowercasing function did not finish because of context

        if offsets.start >= offsets.limit {
            return Ok(());
        }

        // case type: >0 cased (UCaseProps.LOWER etc.)  ==0 uncased  <0 case-ignorable
        let mut type_;

        // Our mode; we are either converting letter toTitle or
        // toLower.
        let mut do_title = true;

        // Determine if there is a preceding context of cased case-ignorable*,
        // in which case we want to start in toLower mode.  If the
        // prior context is anything else (including empty) then start
        // in toTitle mode.
        let mut c;
        let mut start = offsets.start - 1;
        while start >= offsets.context_start {
            c = text.char32_at(start);
            type_ = self.csp.get_type_or_ignorable(c);
            if type_ > 0 {
                // cased
                do_title = false;
                break;
            } else if type_ == 0 {
                // uncased but not ignorable
                break;
            }
            // else (type<0) case-ignorable: continue
            start -= UTF16::get_char_count(c);
        }

        // Convert things after a cased character toLower; things
        // after a uncased, non-case-ignorable character toTitle.  Case-ignorable
        // characters are copied directly and do not change the mode.

        let mut iter = ReplaceableContextIterator::new();
        iter.set_text(text);
        iter.set_index(offsets.start);
        iter.set_limit(offsets.limit);
        iter.set_context_limits(offsets.context_start, offsets.context_limit);

        let mut result: Vec<u16> = Vec::new();

        // Walk through original string
        // If there is a case change, modify corresponding position in replaceable
        let mut delta;

        loop {
            c = iter.next_case_map_cp();
            if c < 0 {
                break;
            }
            type_ = self.csp.get_type_or_ignorable(c);
            if type_ >= 0 {
                // not case-ignorable
                if do_title {
                    c = self
                        .csp
                        .to_full_title(c, Some(&mut iter), &mut result, self.case_locale);
                } else {
                    c = self
                        .csp
                        .to_full_lower(c, Some(&mut iter), &mut result, self.case_locale);
                }
                do_title = type_ == 0; // doTitle=isUncased

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
                            UCharacter::to_title_case_ulocale(&locale, source, None)
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
