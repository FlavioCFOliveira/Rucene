// © 2018 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.number.parse.CodePointMatcher`.

use std::any::Any;
use std::sync::Arc;

use rucene::error::Result;

use crate::r#impl::number::parse::number_parse_matcher::NumberParseMatcher;
use crate::r#impl::number::parse::parsed_number::ParsedNumber;
use crate::r#impl::string_segment::StringSegment;

/// Matches a single code point, performing no other logic.
///
/// Equivalent to `com.ibm.icu.impl.number.parse.CodePointMatcher`.
#[derive(Debug)]
pub struct CodePointMatcher {
    cp: i32,
}

impl CodePointMatcher {
    /// Equivalent to `getInstance(int)`.
    pub fn get_instance(cp: i32) -> Arc<CodePointMatcher> {
        // TODO: Cache certain popular instances?
        Arc::new(CodePointMatcher { cp })
    }
}

impl NumberParseMatcher for CodePointMatcher {
    fn match_(&self, segment: &mut StringSegment, result: &mut ParsedNumber) -> Result<bool> {
        if segment.starts_with_code_point(self.cp) {
            segment.adjust_offset_by_code_point();
            result.set_chars_consumed(segment);
        }
        Ok(false)
    }

    fn smoke_test(&self, segment: &StringSegment) -> bool {
        segment.starts_with_code_point(self.cp)
    }

    fn post_process(&self, _result: &mut ParsedNumber) -> Result<()> {
        // No-op
        Ok(())
    }

    fn to_java_string(&self) -> String {
        format!("<CodePointMatcher U+{:x}>", self.cp as u32)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
