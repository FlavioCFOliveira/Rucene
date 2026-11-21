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

//! Factory for `CJKWidthFilter`, ported from
//! `org.apache.lucene.analysis.cjk.CJKWidthFilterFactory`.

use std::collections::HashMap;
use std::sync::Arc;

use rucene::analysis::factories::{
    AbstractAnalysisFactory, AnalysisFactoryClass, SPINameField, TokenFilterFactory,
};
use rucene::analysis::TokenStream;
use rucene::error::Result;

use super::cjk_width_filter::new_cjk_width_filter;
use crate::spi::check_unknown_parameters;

/// Factory for [`CJKWidthFilter`](super::cjk_width_filter::CJKWidthFilter).
///
/// ```xml
/// <fieldType name="text_cjk" class="solr.TextField">
///   <analyzer>
///     <tokenizer class="solr.StandardTokenizerFactory"/>
///     <filter class="solr.CJKWidthFilterFactory"/>
///     <filter class="solr.LowerCaseFilterFactory"/>
///     <filter class="solr.CJKBigramFilterFactory"/>
///   </analyzer>
/// </fieldType>
/// ```
///
/// Equivalent to `org.apache.lucene.analysis.cjk.CJKWidthFilterFactory`
/// (since 3.6.0, SPI name `cjkWidth`).
#[derive(Debug)]
pub struct CJKWidthFilterFactory {
    base: AbstractAnalysisFactory,
}

impl CJKWidthFilterFactory {
    /// SPI name: `CJKWidthFilterFactory.NAME`.
    pub const NAME: &'static str = "cjkWidth";

    /// Creates a new CJKWidthFilterFactory
    ///
    /// Equivalent to `CJKWidthFilterFactory(Map<String, String>)`.
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
    /// Equivalent to `CJKWidthFilterFactory()`.
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
    pub const fn class() -> AnalysisFactoryClass<dyn TokenFilterFactory> {
        AnalysisFactoryClass::new(
            "org.apache.lucene.analysis.cjk.CJKWidthFilterFactory",
            SPINameField::Declared(Self::NAME),
            Some(Self::new_from_map),
        )
    }

    fn new_from_map(args: &mut HashMap<String, String>) -> Result<Arc<dyn TokenFilterFactory>> {
        Ok(Arc::new(Self::new(args)?))
    }
}

impl TokenFilterFactory for CJKWidthFilterFactory {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.analysis.cjk.CJKWidthFilterFactory"
    }

    /// Equivalent to `CJKWidthFilterFactory.create(TokenStream)`.
    fn create(&self, input: Box<dyn TokenStream>) -> Result<Box<dyn TokenStream>> {
        Ok(Box::new(new_cjk_width_filter(input)?))
    }

    /// Equivalent to `CJKWidthFilterFactory.normalize(TokenStream)`,
    /// `create(input)`.
    fn normalize(&self, input: Box<dyn TokenStream>) -> Result<Box<dyn TokenStream>> {
        self.create(input)
    }
}
