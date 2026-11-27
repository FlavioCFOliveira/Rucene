// © 2017 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.number.parse.NumberParseMatcher`.

use std::any::Any;
use std::fmt::Debug;

use rucene::error::Result;

use crate::r#impl::number::parse::parsed_number::ParsedNumber;
use crate::r#impl::string_segment::StringSegment;

/// Given a string, there should NOT be more than one way to consume the
/// string with the same matcher applied multiple times. If there is, the
/// non-greedy parsing algorithm will be unhappy and may enter an exponential
/// or infinite loop.
///
/// Equivalent to the interface
/// `com.ibm.icu.impl.number.parse.NumberParseMatcher`; the marker interface
/// `NumberParseMatcher.Flexible` is [`is_flexible`](Self::is_flexible), and
/// `toString()` is [`to_java_string`](Self::to_java_string).
pub trait NumberParseMatcher: Debug + Send + Sync + Any {
    /// Runs this matcher starting at the beginning of the given
    /// StringSegment. If this matcher finds something interesting in the
    /// StringSegment, it should update the offset of the StringSegment
    /// corresponding to how many chars were matched. Returns whether
    /// more characters could be consumed (Java `match`).
    ///
    /// # Errors
    ///
    /// The errors of the matcher (the unchecked exceptions Java lets
    /// propagate).
    fn match_(&self, segment: &mut StringSegment, result: &mut ParsedNumber) -> Result<bool>;

    /// Performs a fast "smoke check" for whether or not this matcher could
    /// possibly match against the given string segment. Equivalent to
    /// `smokeTest(StringSegment)`.
    fn smoke_test(&self, segment: &StringSegment) -> bool;

    /// Method called at the end of a parse, after all matchers have failed
    /// to consume any more chars. Equivalent to `postProcess(ParsedNumber)`.
    ///
    /// # Errors
    ///
    /// The errors of the matcher.
    fn post_process(&self, result: &mut ParsedNumber) -> Result<()>;

    /// `instanceof NumberParseMatcher.Flexible`: matchers that are allowed to
    /// match multiple times in a series (`SeriesMatcher`).
    fn is_flexible(&self) -> bool {
        false
    }

    /// Equivalent to `toString()`.
    fn to_java_string(&self) -> String;

    /// The matcher as `Any`, for `instanceof` tests.
    fn as_any(&self) -> &dyn Any;
}

/// `List.toString()` of a list of matchers: `[a, b]`.
pub(crate) fn matchers_to_string(matchers: &[std::sync::Arc<dyn NumberParseMatcher>]) -> String {
    let mut sb = String::from("[");
    for (i, m) in matchers.iter().enumerate() {
        if i > 0 {
            sb.push_str(", ");
        }
        sb.push_str(&m.to_java_string());
    }
    sb.push(']');
    sb
}
