// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.breakiter.LanguageBreakEngine`.

use std::any::Any;
use std::fmt::Debug;

use rucene_analysis_common::jdk::character_iterator::CharacterIterator;

use crate::r#impl::breakiter::dictionary_break_engine::DequeI;

/// The interface of the language-specific break engines a
/// `RuleBasedBreakIterator` hands its dictionary ranges to.
///
/// Equivalent to the interface
/// `com.ibm.icu.impl.breakiter.LanguageBreakEngine`. Engines are shared by
/// every iterator (`RuleBasedBreakIterator.gAllBreakEngines`), so both
/// methods take `&self`.
pub trait LanguageBreakEngine: Debug + Send + Sync + Any {
    /// `handles(int c)`: whether this engine breaks text containing `c`.
    fn handles(&self, c: i32) -> bool;

    /// `findBreaks(CharacterIterator text, int startPos, int endPos, DequeI
    /// foundBreaks, boolean isPhraseBreaking)`: finds the breaks of the run
    /// of characters this engine handles starting at the text's index,
    /// pushes them onto `found_breaks` and returns their number; the text is
    /// left at the end of the run.
    fn find_breaks(
        &self,
        text: &mut dyn CharacterIterator,
        start_pos: i32,
        end_pos: i32,
        found_breaks: &mut DequeI,
        is_phrase_breaking: bool,
    ) -> i32;

    /// The engine as `Any`, for the identity and `instanceof` tests of
    /// `RuleBasedBreakIterator.getLanguageBreakEngine`.
    fn as_any(&self) -> &dyn Any;
}
