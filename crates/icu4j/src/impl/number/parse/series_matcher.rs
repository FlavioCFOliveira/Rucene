// © 2018 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.number.parse.SeriesMatcher`.

use std::any::Any;
use std::sync::Arc;

use rucene::error::Result;

use crate::r#impl::number::parse::number_parse_matcher::{matchers_to_string, NumberParseMatcher};
use crate::r#impl::number::parse::parsed_number::ParsedNumber;
use crate::r#impl::string_segment::StringSegment;

/// Composes a number of matchers, running one after another. Matches the
/// input string only if all of the matchers in the series succeed.
/// Performs greedy matches within the context of the series.
///
/// Equivalent to `com.ibm.icu.impl.number.parse.SeriesMatcher`.
#[derive(Debug, Default)]
pub struct SeriesMatcher {
    /// `matchers` (Java `null` until the first matcher is added).
    pub(crate) matchers: Option<Vec<Arc<dyn NumberParseMatcher>>>,
    /// `frozen`.
    pub(crate) frozen: bool,
}

impl SeriesMatcher {
    /// Equivalent to `addMatcher(NumberParseMatcher)`.
    pub fn add_matcher(&mut self, matcher: Arc<dyn NumberParseMatcher>) {
        debug_assert!(!self.frozen);
        self.matchers.get_or_insert_with(Vec::new).push(matcher);
    }

    /// Equivalent to `freeze()`.
    pub fn freeze(&mut self) {
        self.frozen = true;
    }

    /// Equivalent to `length()`.
    pub fn length(&self) -> i32 {
        self.matchers.as_ref().map_or(0, |m| m.len() as i32)
    }

    /// `match(StringSegment, ParsedNumber)`.
    ///
    /// # Errors
    ///
    /// The errors of the matchers in the series.
    pub fn series_match(&self, segment: &mut StringSegment, result: &mut ParsedNumber) -> Result<bool> {
        debug_assert!(self.frozen);
        let Some(matchers) = self.matchers.as_ref() else {
            return Ok(false);
        };

        // TODO: Give a nice way to reset ParsedNumber to avoid the copy here.
        let mut backup = ParsedNumber::new();
        backup.copy_from(result);

        let initial_offset = segment.get_offset();
        let mut maybe_more = true;
        let mut i = 0;
        while i < matchers.len() {
            let matcher = &matchers[i];
            let matcher_offset = segment.get_offset();
            if segment.length() != 0 {
                maybe_more = matcher.match_(segment, result)?;
            } else {
                // Nothing for this matcher to match; ask for more.
                maybe_more = true;
            }

            let success = segment.get_offset() != matcher_offset;
            let is_flexible = matcher.is_flexible();
            if success && is_flexible {
                // Match succeeded, and this is a flexible matcher. Re-run it.
            } else if success {
                // Match succeeded, and this is NOT a flexible matcher. Proceed to the next matcher.
                i += 1;
                // Small hack: if there is another matcher coming, do not accept trailing weak chars.
                // Needed for proper handling of currency spacing.
                if i < matchers.len() && segment.get_offset() != result.char_end && result.char_end > matcher_offset {
                    segment.set_offset(result.char_end);
                }
            } else if is_flexible {
                // Match failed, and this is a flexible matcher. Try again with the next matcher.
                i += 1;
            } else {
                // Match failed, and this is NOT a flexible matcher. Exit.
                segment.set_offset(initial_offset);
                result.copy_from(&backup);
                return Ok(maybe_more);
            }
        }

        // All matchers in the series succeeded.
        Ok(maybe_more)
    }

    /// `smokeTest(StringSegment)`.
    pub fn series_smoke_test(&self, segment: &StringSegment) -> bool {
        debug_assert!(self.frozen);
        let Some(matchers) = self.matchers.as_ref() else {
            return false;
        };

        // SeriesMatchers are never allowed to start with a Flexible matcher.
        debug_assert!(!matchers[0].is_flexible());
        matchers[0].smoke_test(segment)
    }

    /// `postProcess(ParsedNumber)`.
    ///
    /// # Errors
    ///
    /// The errors of the matchers in the series.
    pub fn series_post_process(&self, result: &mut ParsedNumber) -> Result<()> {
        debug_assert!(self.frozen);
        let Some(matchers) = self.matchers.as_ref() else {
            return Ok(());
        };

        for matcher in matchers {
            matcher.post_process(result)?;
        }
        Ok(())
    }

    /// `toString()`: `<SeriesMatcher ` + `matchers` + `>`.
    pub fn series_to_string(&self) -> String {
        match self.matchers.as_ref() {
            Some(m) => format!("<SeriesMatcher {}>", matchers_to_string(m)),
            None => "<SeriesMatcher null>".to_string(),
        }
    }
}

impl NumberParseMatcher for SeriesMatcher {
    fn match_(&self, segment: &mut StringSegment, result: &mut ParsedNumber) -> Result<bool> {
        self.series_match(segment, result)
    }

    fn smoke_test(&self, segment: &StringSegment) -> bool {
        self.series_smoke_test(segment)
    }

    fn post_process(&self, result: &mut ParsedNumber) -> Result<()> {
        self.series_post_process(result)
    }

    fn to_java_string(&self) -> String {
        self.series_to_string()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
