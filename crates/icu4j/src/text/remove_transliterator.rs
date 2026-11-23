// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.text.RemoveTransliterator`.

use std::sync::Arc;

use rucene::error::Result;

use crate::jdk::units;
use crate::text::replaceable::Replaceable;
use crate::text::transliterator::{
    factory, register_factory_in, register_special_inverse, transliterator_boilerplate, Position, Transliterator, TransliteratorBase,
};
use crate::text::transliterator_registry::TransliteratorRegistry;
use crate::text::unicode_set::UnicodeSet;

/// A transliterator that removes characters. This is useful in conjunction
/// with a filter.
///
/// Equivalent to the package-private class
/// `com.ibm.icu.text.RemoveTransliterator`.
#[derive(Debug)]
pub struct RemoveTransliterator {
    base: TransliteratorBase,
}

impl Default for RemoveTransliterator {
    fn default() -> Self {
        Self::new()
    }
}

impl RemoveTransliterator {
    /// `_ID`: ID for this transliterator.
    const ID: &'static str = "Any-Remove";

    /// `register()`: system registration hook (on the registry under
    /// construction).
    pub(crate) fn register(registry: &TransliteratorRegistry) -> Result<()> {
        register_factory_in(
            registry,
            &units(Self::ID),
            factory(|_id| Ok(Some(Arc::new(RemoveTransliterator::new()) as Arc<dyn Transliterator>))),
        )?;
        register_special_inverse(&units("Remove"), &units("Null"), false);
        Ok(())
    }

    /// `RemoveTransliterator()`.
    pub fn new() -> Self {
        Self {
            base: TransliteratorBase::with_id(units(Self::ID)),
        }
    }
}

impl Transliterator for RemoveTransliterator {
    transliterator_boilerplate!();

    /// `handleTransliterate(Replaceable text, Position index, boolean
    /// incremental)`.
    fn handle_transliterate(&self, text: &dyn Replaceable, index: &mut Position, _incremental: bool) -> Result<()> {
        // Our caller (filteredTransliterate) has already narrowed us
        // to an unfiltered run.  Delete it.
        text.replace(index.start, index.limit, &[]);
        let len = index.limit - index.start;
        index.context_limit -= len;
        index.limit -= len;
        Ok(())
    }

    /// `addSourceTargetSet(UnicodeSet inputFilter, UnicodeSet sourceSet,
    /// UnicodeSet targetSet)`.
    fn add_source_target_set(&self, input_filter: &UnicodeSet, source_set: &mut UnicodeSet, _target_set: &mut UnicodeSet) -> Result<()> {
        // intersect myFilter with the input filter
        let my_filter = self.get_filter_as_unicode_set(input_filter)?;
        source_set.add_all(&my_filter)?;
        // do nothing with the target
        Ok(())
    }
}
