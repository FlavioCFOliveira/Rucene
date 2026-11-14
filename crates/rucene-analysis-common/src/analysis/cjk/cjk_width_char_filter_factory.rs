/*
 * Licensed to the Apache Software Foundation (ASF) under one or more
 * contributor license agreements.  See the NOTICE file distributed with
 * this work for additional information regarding copyright ownership.
 * The ASF licenses this file to You under the Apache License, Version 2.0
 * (the "License"); you may not use this file except in compliance with
 * the License.  You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

//! Factory for `CJKWidthCharFilter`, ported from
//! `org.apache.lucene.analysis.cjk.CJKWidthCharFilterFactory`.

use std::collections::HashMap;
use std::sync::Arc;

use rucene::analysis::factories::{
    AbstractAnalysisFactory, AnalysisFactoryClass, CharFilterFactory, SPINameField,
};
use rucene::analysis::CharReader;
use rucene::error::Result;

use super::cjk_width_char_filter::CJKWidthCharFilter;
use crate::spi::check_unknown_parameters;

/// Factory for [`CJKWidthCharFilter`].
///
/// Equivalent to `org.apache.lucene.analysis.cjk.CJKWidthCharFilterFactory`
/// (SPI name `cjkWidth`).
#[derive(Debug)]
pub struct CJKWidthCharFilterFactory {
    base: AbstractAnalysisFactory,
}

impl CJKWidthCharFilterFactory {
    /// SPI name: `CJKWidthCharFilterFactory.NAME`.
    pub const NAME: &'static str = "cjkWidth";

    /// Creates a new CJKWidthCharFilterFactory
    ///
    /// Equivalent to `CJKWidthCharFilterFactory(Map<String, String>)`.
    ///
    /// # Errors
    ///
    /// What `AbstractAnalysisFactory` raises, and
    /// `IllegalArgumentException("Unknown parameters: ...")` for arguments
    /// left over.
    pub fn new(args: &mut HashMap<String, String>) -> Result<Self> {
        let base = AbstractAnalysisFactory::new(args)?;
        check_unknown_parameters(args)?;
        Ok(Self { base })
    }

    /// Default ctor for compatibility with SPI
    ///
    /// Equivalent to `CJKWidthCharFilterFactory()`.
    ///
    /// # Errors
    ///
    /// Always: `defaultCtorException()`.
    pub fn new_without_args() -> Result<Self> {
        Err(AbstractAnalysisFactory::default_ctor_exception())
    }

    /// The inherited `AbstractAnalysisFactory` state.
    pub fn base(&self) -> &AbstractAnalysisFactory {
        &self.base
    }

    /// The class of this factory, as the SPI loader registers it.
    pub const fn class() -> AnalysisFactoryClass<dyn CharFilterFactory> {
        AnalysisFactoryClass::new(
            "org.apache.lucene.analysis.cjk.CJKWidthCharFilterFactory",
            SPINameField::Declared(Self::NAME),
            Some(Self::new_from_map),
        )
    }

    fn new_from_map(args: &mut HashMap<String, String>) -> Result<Arc<dyn CharFilterFactory>> {
        Ok(Arc::new(Self::new(args)?))
    }
}

impl CharFilterFactory for CJKWidthCharFilterFactory {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.analysis.cjk.CJKWidthCharFilterFactory"
    }

    /// Equivalent to `CJKWidthCharFilterFactory.create(Reader)`.
    fn create(&self, input: Box<dyn CharReader>) -> Result<Box<dyn CharReader>> {
        Ok(Box::new(CJKWidthCharFilter::new(input)))
    }

    /// Equivalent to `CJKWidthCharFilterFactory.normalize(Reader)`,
    /// `create(input)`.
    fn normalize(&self, input: Box<dyn CharReader>) -> Result<Box<dyn CharReader>> {
        self.create(input)
    }
}
