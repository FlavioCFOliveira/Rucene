// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.text.FunctionReplacer`.

use std::sync::Arc;

use rucene::error::Result;

use crate::text::replaceable::Replaceable;
use crate::text::transliterator::Transliterator;
use crate::text::unicode_replacer::UnicodeReplacer;
use crate::text::unicode_set::UnicodeSet;

/// A replacer that calls a transliterator to generate its output text.
/// The input text to the transliterator is the output of another
/// `UnicodeReplacer` object. That is, this replacer wraps another replacer
/// with a transliterator.
///
/// Equivalent to the package-private class `com.ibm.icu.text.FunctionReplacer`.
pub struct FunctionReplacer {
    /// The transliterator. Must not be null.
    translit: Arc<dyn Transliterator>,
    /// The replacer object. This generates text that is then processed by
    /// `translit`. Must not be null.
    replacer: Box<dyn UnicodeReplacer>,
}

impl std::fmt::Debug for FunctionReplacer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FunctionReplacer")
            .field("translit", &self.translit)
            .finish()
    }
}

impl FunctionReplacer {
    /// `FunctionReplacer(Transliterator theTranslit, UnicodeReplacer
    /// theReplacer)`.
    pub(crate) fn new(
        the_translit: Arc<dyn Transliterator>,
        the_replacer: Box<dyn UnicodeReplacer>,
    ) -> Self {
        Self {
            translit: the_translit,
            replacer: the_replacer,
        }
    }
}

impl UnicodeReplacer for FunctionReplacer {
    /// `replace(Replaceable text, int start, int limit, int[] cursor)`.
    fn replace(
        &self,
        text: &dyn Replaceable,
        start: i32,
        limit: i32,
        cursor: &mut [i32],
    ) -> Result<i32> {
        // First delegate to subordinate replacer
        let len = self.replacer.replace(text, start, limit, cursor)?;
        let limit = start + len;

        // Now transliterate
        let limit = self.translit.transliterate_range(text, start, limit)?;

        Ok(limit - start)
    }

    /// `toReplacerPattern(boolean escapeUnprintable)`.
    fn to_replacer_pattern(&self, escape_unprintable: bool) -> Result<Vec<u16>> {
        let mut rule = vec![b'&' as u16];
        rule.extend_from_slice(&self.translit.get_id());
        rule.extend_from_slice(&[b'(' as u16, b' ' as u16]);
        rule.extend_from_slice(&self.replacer.to_replacer_pattern(escape_unprintable)?);
        rule.extend_from_slice(&[b' ' as u16, b')' as u16]);
        Ok(rule)
    }

    /// `addReplacementSetTo(UnicodeSet toUnionTo)`.
    fn add_replacement_set_to(&self, to_union_to: &mut UnicodeSet) -> Result<()> {
        to_union_to.add_all(&self.translit.get_target_set()?)?;
        Ok(())
    }
}
