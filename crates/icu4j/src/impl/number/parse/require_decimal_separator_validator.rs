// © 2017 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.number.parse.RequireDecimalSeparatorValidator`.

use rucene::error::Result;

use crate::r#impl::number::parse::parsed_number::{self, ParsedNumber};
use crate::r#impl::number::parse::validation_matcher::{impl_validation_matcher, ValidationMatcher};
use std::sync::{Arc, LazyLock};

static A: LazyLock<Arc<RequireDecimalSeparatorValidator>> =
    LazyLock::new(|| Arc::new(RequireDecimalSeparatorValidator { pattern_has_decimal_separator: true }));
static B: LazyLock<Arc<RequireDecimalSeparatorValidator>> =
    LazyLock::new(|| Arc::new(RequireDecimalSeparatorValidator { pattern_has_decimal_separator: false }));

/// Equivalent to `com.ibm.icu.impl.number.parse.RequireDecimalSeparatorValidator`.
#[derive(Debug)]
pub struct RequireDecimalSeparatorValidator {
    pattern_has_decimal_separator: bool,
}

impl RequireDecimalSeparatorValidator {
    /// Equivalent to `getInstance(boolean)`.
    pub fn get_instance(pattern_has_decimal_separator: bool) -> Arc<RequireDecimalSeparatorValidator> {
        if pattern_has_decimal_separator {
            A.clone()
        } else {
            B.clone()
        }
    }
}

impl ValidationMatcher for RequireDecimalSeparatorValidator {
    fn validate(&self, result: &mut ParsedNumber) -> Result<()> {
        let parse_has_decimal_separator = 0 != (result.flags & parsed_number::FLAG_HAS_DECIMAL_SEPARATOR);
        if parse_has_decimal_separator != self.pattern_has_decimal_separator {
            result.flags |= parsed_number::FLAG_FAIL;
        }
        Ok(())
    }
}

impl_validation_matcher!(RequireDecimalSeparatorValidator, |_| "<RequireDecimalSeparator>".to_string());
