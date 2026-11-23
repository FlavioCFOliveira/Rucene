// © 2017 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.number.parse.RequireAffixValidator`.

use rucene::error::Result;

use crate::r#impl::number::parse::parsed_number::{self, ParsedNumber};
use crate::r#impl::number::parse::validation_matcher::{impl_validation_matcher, ValidationMatcher};

/// Equivalent to `com.ibm.icu.impl.number.parse.RequireAffixValidator`.
#[derive(Debug, Default)]
pub struct RequireAffixValidator;

impl ValidationMatcher for RequireAffixValidator {
    fn validate(&self, result: &mut ParsedNumber) -> Result<()> {
        if result.prefix.is_none() || result.suffix.is_none() {
            // We saw a prefix or a suffix but not both. Fail the parse.
            result.flags |= parsed_number::FLAG_FAIL;
        }
        Ok(())
    }
}

impl_validation_matcher!(RequireAffixValidator, |_| "<RequireAffix>".to_string());
