// © 2017 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.number.parse.ValidationMatcher`.

use rucene::error::Result;

use crate::r#impl::number::parse::parsed_number::ParsedNumber;

/// A Matcher used only for post-process validation, not for consuming
/// characters at runtime.
///
/// Equivalent to the abstract class
/// `com.ibm.icu.impl.number.parse.ValidationMatcher`: its `match` and
/// `smokeTest` answer `false`; a subclass implements this trait (its
/// `postProcess`) and `impl_validation_matcher!`.
pub trait ValidationMatcher {
    /// The subclass's `postProcess(ParsedNumber)`.
    ///
    /// # Errors
    ///
    /// The errors of the subclass.
    fn validate(&self, result: &mut ParsedNumber) -> Result<()>;
}

/// Implements `NumberParseMatcher` for a `ValidationMatcher` subclass.
macro_rules! impl_validation_matcher {
    ($ty:ty, $to_string:expr) => {
        impl $crate::r#impl::number::parse::number_parse_matcher::NumberParseMatcher for $ty {
            fn match_(
                &self,
                _segment: &mut $crate::r#impl::string_segment::StringSegment,
                _result: &mut $crate::r#impl::number::parse::parsed_number::ParsedNumber,
            ) -> rucene::error::Result<bool> {
                Ok(false)
            }

            fn smoke_test(&self, _segment: &$crate::r#impl::string_segment::StringSegment) -> bool {
                false
            }

            fn post_process(
                &self,
                result: &mut $crate::r#impl::number::parse::parsed_number::ParsedNumber,
            ) -> rucene::error::Result<()> {
                $crate::r#impl::number::parse::validation_matcher::ValidationMatcher::validate(self, result)
            }

            fn to_java_string(&self) -> String {
                let f: fn(&$ty) -> String = $to_string;
                f(self)
            }

            fn as_any(&self) -> &dyn std::any::Any {
                self
            }
        }
    };
}
pub(crate) use impl_validation_matcher;

