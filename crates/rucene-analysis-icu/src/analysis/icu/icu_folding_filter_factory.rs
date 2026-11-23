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

//! Factory for `ICUFoldingFilter`, ported from
//! `org.apache.lucene.analysis.icu.ICUFoldingFilterFactory`.

use std::collections::HashMap;
use std::sync::Arc;

use icu4j::text::filtered_normalizer2::FilteredNormalizer2;
use icu4j::text::normalizer2::Normalizer2;
use icu4j::text::unicode_set::UnicodeSet;
use rucene::analysis::factories::{
    AbstractAnalysisFactory, AnalysisFactoryClass, SPINameField, TokenFilterFactory,
};
use rucene::analysis::TokenStream;
use rucene::error::Result;

use super::icu_folding_filter::{ICUFoldingFilter, NORMALIZER};
use crate::spi::check_unknown_parameters;

/// Factory for `ICUFoldingFilter`.
///
/// ```text
/// <fieldType name="text_folded" class="solr.TextField" positionIncrementGap="100">
///   <analyzer>
///     <tokenizer class="solr.WhitespaceTokenizerFactory"/>
///     <filter class="solr.ICUFoldingFilterFactory"/>
///   </analyzer>
/// </fieldType>
/// ```
///
/// Equivalent to `org.apache.lucene.analysis.icu.ICUFoldingFilterFactory`
/// (since 3.1.0, `@lucene.spi icuFolding`).
#[derive(Debug)]
pub struct ICUFoldingFilterFactory {
    base: AbstractAnalysisFactory,
    normalizer: Arc<dyn Normalizer2>,
}

impl ICUFoldingFilterFactory {
    /// SPI name.
    ///
    /// Equivalent to `ICUFoldingFilterFactory.NAME`.
    pub const NAME: &'static str = "icuFolding";

    /// Creates a new ICUFoldingFilterFactory.
    ///
    /// Equivalent to `ICUFoldingFilterFactory(Map<String, String>)`.
    ///
    /// # Errors
    ///
    /// Returns what [`AbstractAnalysisFactory::new`] returns; the
    /// `IllegalArgumentException` of a malformed `filter` pattern; and
    /// `IllegalArgumentException` for a leftover argument.
    pub fn new(args: &mut HashMap<String, String>) -> Result<Self> {
        let base = AbstractAnalysisFactory::new(args)?;
        let mut normalizer: Arc<dyn Normalizer2> = Arc::clone(&NORMALIZER);
        let filter = AbstractAnalysisFactory::get(args, "filter");
        if let Some(filter) = filter {
            let units: Vec<u16> = filter.encode_utf16().collect();
            let mut set = UnicodeSet::from_pattern(&units)?;
            if !set.is_empty() {
                set.freeze();
                normalizer = Arc::new(FilteredNormalizer2::new(normalizer, Arc::new(set)));
            }
        }
        check_unknown_parameters(args)?;
        Ok(Self { base, normalizer })
    }

    /// Default ctor for compatibility with SPI.
    ///
    /// Equivalent to `ICUFoldingFilterFactory()`.
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

    /// The normalizer the filters of this factory apply (Java's private
    /// field `normalizer`).
    pub fn normalizer(&self) -> &Arc<dyn Normalizer2> {
        &self.normalizer
    }

    /// The factory's class, as `META-INF/services/org.apache.lucene.analysis.TokenFilterFactory`
    /// names it: `ICUFoldingFilterFactory.class`, with its `NAME` field and
    /// its `Map<String, String>` constructor.
    pub const fn class() -> AnalysisFactoryClass<dyn TokenFilterFactory> {
        AnalysisFactoryClass::new(
            "org.apache.lucene.analysis.icu.ICUFoldingFilterFactory",
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

impl TokenFilterFactory for ICUFoldingFilterFactory {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.analysis.icu.ICUFoldingFilterFactory"
    }

    /// Equivalent to `ICUFoldingFilterFactory.create(TokenStream)`, `new
    /// ICUFoldingFilter(input, normalizer)`.
    fn create(&self, input: Box<dyn TokenStream>) -> Result<Box<dyn TokenStream>> {
        Ok(Box::new(ICUFoldingFilter::with_normalizer(
            input,
            Arc::clone(&self.normalizer),
        )?))
    }

    /// Equivalent to `ICUFoldingFilterFactory.normalize(TokenStream)`,
    /// `create(input)`.
    fn normalize(&self, input: Box<dyn TokenStream>) -> Result<Box<dyn TokenStream>> {
        self.create(input)
    }
}
