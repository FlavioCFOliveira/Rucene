// © 2017 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.number.MultiplierFormatHandler`.

use std::sync::Arc;

use rucene::error::Result;

use crate::number::scale::Scale;
use crate::r#impl::number::decimal_quantity::DecimalQuantity;
use crate::r#impl::number::micro_props::MicroProps;
use crate::r#impl::number::micro_props_generator::MicroPropsGenerator;

/// Wraps a [`Scale`] for use in the number formatting pipeline.
///
/// Equivalent to `com.ibm.icu.impl.number.MultiplierFormatHandler`.
#[derive(Debug)]
pub struct MultiplierFormatHandler {
    /// `multiplier`.
    multiplier: Scale,
    /// `parent`.
    parent: Arc<dyn MicroPropsGenerator>,
}

impl MultiplierFormatHandler {
    /// Equivalent to `MultiplierFormatHandler(Scale, MicroPropsGenerator)`.
    pub fn new(multiplier: Scale, parent: Arc<dyn MicroPropsGenerator>) -> Self {
        MultiplierFormatHandler { multiplier, parent }
    }
}

impl MicroPropsGenerator for MultiplierFormatHandler {
    fn process_quantity(&self, quantity: &mut dyn DecimalQuantity) -> Result<MicroProps> {
        let micros = self.parent.process_quantity(quantity)?;
        self.multiplier.apply_to(quantity)?;
        Ok(micros)
    }
}
