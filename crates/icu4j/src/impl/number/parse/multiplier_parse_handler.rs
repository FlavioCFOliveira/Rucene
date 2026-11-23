// © 2017 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.number.parse.MultiplierParseHandler`.

use rucene::error::Result;

use crate::r#impl::number::parse::parsed_number::ParsedNumber;
use crate::r#impl::number::parse::validation_matcher::{impl_validation_matcher, ValidationMatcher};
use crate::number::scale::Scale;

/// Wraps a `Scale` for use in the number parsing pipeline.
///
/// Equivalent to `com.ibm.icu.impl.number.parse.MultiplierParseHandler`.
#[derive(Debug)]
pub struct MultiplierParseHandler {
    multiplier: Scale,
}

impl MultiplierParseHandler {
    /// Equivalent to `MultiplierParseHandler(Scale)`.
    pub fn new(multiplier: Scale) -> Self {
        MultiplierParseHandler { multiplier }
    }
}

impl ValidationMatcher for MultiplierParseHandler {
    fn validate(&self, result: &mut ParsedNumber) -> Result<()> {
        if let Some(quantity) = result.quantity.as_mut() {
            self.multiplier.apply_reciprocal_to(quantity)?;
        }
        Ok(())
    }
}

impl_validation_matcher!(MultiplierParseHandler, |m| format!("<MultiplierHandler com.ibm.icu.number.Scale@{:x}>", m.multiplier.hash_code()));
