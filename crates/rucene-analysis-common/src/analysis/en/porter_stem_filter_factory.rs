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

//! Factory for `PorterStemFilter`, ported from
//! `org.apache.lucene.analysis.en.PorterStemFilterFactory`.

use std::collections::HashMap;
use std::sync::Arc;

use rucene::analysis::factories::{
    AbstractAnalysisFactory, AnalysisFactoryClass, SPINameField, TokenFilterFactory,
};
use rucene::analysis::TokenStream;
use rucene::error::Result;

use super::porter_stem_filter::new_porter_stem_filter;
use crate::spi::check_unknown_parameters;

/// Factory for `PorterStemFilter`.
///
/// ```text
/// <fieldType name="text_porterstem" class="solr.TextField" positionIncrementGap="100">
///   <analyzer>
///     <tokenizer class="solr.StandardTokenizerFactory"/>
///     <filter class="solr.LowerCaseFilterFactory"/>
///     <filter class="solr.PorterStemFilterFactory"/>
///   </analyzer>
/// </fieldType>
/// ```
///
/// Equivalent to `org.apache.lucene.analysis.en.PorterStemFilterFactory` (since 3.1).
#[derive(Debug)]
pub struct PorterStemFilterFactory {
    base: AbstractAnalysisFactory,
}

impl PorterStemFilterFactory {
    /// SPI name.
    ///
    /// Equivalent to `PorterStemFilterFactory.NAME`.
    pub const NAME: &'static str = "porterStem";

    /// Creates a new PorterStemFilterFactory.
    ///
    /// Equivalent to `PorterStemFilterFactory(Map<String, String>)`.
    ///
    /// # Errors
    ///
    /// Returns what [`AbstractAnalysisFactory::new`] returns, and `IllegalArgumentException` for a leftover argument.
    pub fn new(args: &mut HashMap<String, String>) -> Result<Self> {
        let base = AbstractAnalysisFactory::new(args)?;
        check_unknown_parameters(args)?;
        Ok(Self { base })
    }

    /// Default ctor for compatibility with SPI.
    ///
    /// Equivalent to `PorterStemFilterFactory()`.
    ///
    /// # Errors
    ///
    /// Always: [`AbstractAnalysisFactory::default_ctor_exception`].
    pub fn new_without_args() -> Result<Self> {
        Err(AbstractAnalysisFactory::default_ctor_exception())
    }

    /// The members this factory inherits from `AbstractAnalysisFactory`.
    pub fn base(&self) -> &AbstractAnalysisFactory {
        &self.base
    }

    /// The factory's class, as `META-INF/services/org.apache.lucene.analysis.TokenFilterFactory`
    /// names it: `PorterStemFilterFactory.class`, with its `NAME` field and its
    /// `Map<String, String>` constructor.
    pub const fn class() -> AnalysisFactoryClass<dyn TokenFilterFactory> {
        AnalysisFactoryClass::new(
            "org.apache.lucene.analysis.en.PorterStemFilterFactory",
            SPINameField::Declared(Self::NAME),
            Some(Self::new_from_map),
        )
    }

    /// The `Map<String, String>` constructor, as [`class`](Self::class)
    /// exposes it.
    fn new_from_map(args: &mut HashMap<String, String>) -> Result<Arc<dyn TokenFilterFactory>> {
        Ok(Arc::new(Self::new(args)?))
    }
}

impl TokenFilterFactory for PorterStemFilterFactory {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.analysis.en.PorterStemFilterFactory"
    }

    /// Equivalent to `PorterStemFilterFactory.create(TokenStream)`, `new PorterStemFilter(input)`.
    fn create(&self, input: Box<dyn TokenStream>) -> Result<Box<dyn TokenStream>> {
        Ok(Box::new(new_porter_stem_filter(input)?))
    }
}
