// © 2017 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.number.parse.RequireCurrencyValidator`.

use rucene::error::Result;

use crate::r#impl::number::parse::parsed_number::{self, ParsedNumber};
use crate::r#impl::number::parse::validation_matcher::{impl_validation_matcher, ValidationMatcher};

/// Equivalent to `com.ibm.icu.impl.number.parse.RequireCurrencyValidator`.
#[derive(Debug, Default)]
pub struct RequireCurrencyValidator;

impl ValidationMatcher for RequireCurrencyValidator {
    fn validate(&self, result: &mut ParsedNumber) -> Result<()> {
        if result.currency_code.is_none() {
            result.flags |= parsed_number::FLAG_FAIL;
        }
        Ok(())
    }
}

impl_validation_matcher!(RequireCurrencyValidator, |_| "<RequireCurrency>".to_string());
