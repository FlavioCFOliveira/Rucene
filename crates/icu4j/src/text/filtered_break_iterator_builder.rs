// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.text.FilteredBreakIteratorBuilder`.

use rucene::error::Result;
use rucene_queryparser::jdk::locale::Locale;

use crate::r#impl::simple_filtered_sentence_break_iterator::Builder;
use crate::text::break_iterator::BreakIterator;
use crate::util::u_locale::ULocale;

/// The interface for instances that can wrap a sentence break iterator
/// with a filter suppressing breaks after given strings (abbreviations).
///
/// Equivalent to the abstract class
/// `com.ibm.icu.text.FilteredBreakIteratorBuilder`; its static factories
/// are the functions of this module.
pub trait FilteredBreakIteratorBuilder: Send + Sync {
    /// `suppressBreakAfter(CharSequence str)`: suppresses a certain string
    /// from being the end of a segment; `true` when it was not already
    /// suppressed.
    fn suppress_break_after(&mut self, str: &[u16]) -> bool;

    /// `unsuppressBreakAfter(CharSequence str)`: stops suppressing a
    /// string; `true` when it was suppressed.
    fn unsuppress_break_after(&mut self, str: &[u16]) -> bool;

    /// `wrapIteratorWithFilter(BreakIterator wrappedBreakIterator)`: wraps
    /// (adopts) the break iterator; returns it unchanged when there is
    /// nothing to suppress.
    ///
    /// # Errors
    ///
    /// The exceptions of the trie building.
    fn wrap_iterator_with_filter(
        &self,
        wrapped_break_iterator: Box<dyn BreakIterator>,
    ) -> Result<Box<dyn BreakIterator>>;
}

/// `getInstance(Locale where)`: a builder with the locale's suppressions.
///
/// # Errors
///
/// The exceptions of the bundle lookup.
pub fn get_instance_locale(
    where_: Option<&Locale>,
) -> Result<Box<dyn FilteredBreakIteratorBuilder>> {
    Ok(Box::new(Builder::new_locale(where_)?))
}

/// `getInstance(ULocale where)`: a builder with the locale's suppressions.
///
/// # Errors
///
/// The exceptions of the bundle lookup.
pub fn get_instance_ulocale(where_: &ULocale) -> Result<Box<dyn FilteredBreakIteratorBuilder>> {
    Ok(Box::new(Builder::new_ulocale(where_)?))
}

/// `getEmptyInstance()`: a builder with no suppressions.
pub fn get_empty_instance() -> Box<dyn FilteredBreakIteratorBuilder> {
    Box::new(Builder::new())
}
