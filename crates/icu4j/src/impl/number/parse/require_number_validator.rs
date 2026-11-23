// © 2017 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.number.parse.RequireNumberValidator`.

use rucene::error::Result;

use crate::r#impl::number::parse::parsed_number::{self, ParsedNumber};
use crate::r#impl::number::parse::validation_matcher::{impl_validation_matcher, ValidationMatcher};

/// Equivalent to `com.ibm.icu.impl.number.parse.RequireNumberValidator`.
#[derive(Debug, Default)]
pub struct RequireNumberValidator;

impl ValidationMatcher for RequireNumberValidator {
    fn validate(&self, result: &mut ParsedNumber) -> Result<()> {
        // Require that a number is matched.
        if !result.seen_number() {
            result.flags |= parsed_number::FLAG_FAIL;
        }
        Ok(())
    }
}

impl_validation_matcher!(RequireNumberValidator, |_| "<RequireNumber>".to_string());
