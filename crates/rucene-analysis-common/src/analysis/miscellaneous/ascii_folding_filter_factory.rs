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
//! Factory for `ASCIIFoldingFilter`, ported from
//! `org.apache.lucene.analysis.miscellaneous.ASCIIFoldingFilterFactory`.

use std::collections::HashMap;
use std::sync::Arc;

use rucene::analysis::factories::{
    AbstractAnalysisFactory, AnalysisFactoryClass, SPINameField, TokenFilterFactory,
};
use rucene::analysis::TokenStream;
use rucene::error::Result;

use super::ascii_folding_filter::new_ascii_folding_filter_preserving;
use crate::spi::check_unknown_parameters;

/// Factory for `ASCIIFoldingFilter`.
///
/// ```text
/// <fieldType name="text_ascii" class="solr.TextField" positionIncrementGap="100">
///   <analyzer>
///     <tokenizer class="solr.WhitespaceTokenizerFactory"/>
///     <filter class="solr.ASCIIFoldingFilterFactory" preserveOriginal="false"/>
///   </analyzer>
/// </fieldType>
/// ```
///
/// Equivalent to `org.apache.lucene.analysis.miscellaneous.ASCIIFoldingFilterFactory` (since 3.1).
#[derive(Debug)]
pub struct ASCIIFoldingFilterFactory {
    base: AbstractAnalysisFactory,
    preserve_original: bool,
}

impl ASCIIFoldingFilterFactory {
    /// SPI name.
    ///
    /// Equivalent to `ASCIIFoldingFilterFactory.NAME`.
    pub const NAME: &'static str = "asciiFolding";

    /// Equivalent to the private `ASCIIFoldingFilterFactory.PRESERVE_ORIGINAL`.
    const PRESERVE_ORIGINAL: &'static str = "preserveOriginal";

    /// Creates a new ASCIIFoldingFilterFactory.
    ///
    /// Equivalent to `ASCIIFoldingFilterFactory(Map<String, String>)`.
    ///
    /// # Errors
    ///
    /// Returns what [`AbstractAnalysisFactory::new`] returns, and `IllegalArgumentException` for a leftover argument.
    pub fn new(args: &mut HashMap<String, String>) -> Result<Self> {
        let base = AbstractAnalysisFactory::new(args)?;
        let preserve_original =
            AbstractAnalysisFactory::get_boolean(args, Self::PRESERVE_ORIGINAL, false);
        check_unknown_parameters(args)?;
        Ok(Self {
            base,
            preserve_original,
        })
    }

    /// Default ctor for compatibility with SPI.
    ///
    /// Equivalent to `ASCIIFoldingFilterFactory()`.
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
    /// names it: `ASCIIFoldingFilterFactory.class`, with its `NAME` field and its
    /// `Map<String, String>` constructor.
    pub const fn class() -> AnalysisFactoryClass<dyn TokenFilterFactory> {
        AnalysisFactoryClass::new(
            "org.apache.lucene.analysis.miscellaneous.ASCIIFoldingFilterFactory",
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

impl TokenFilterFactory for ASCIIFoldingFilterFactory {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.analysis.miscellaneous.ASCIIFoldingFilterFactory"
    }

    /// Equivalent to `ASCIIFoldingFilterFactory.create(TokenStream)`, `new
    /// ASCIIFoldingFilter(input, preserveOriginal)`.
    fn create(&self, input: Box<dyn TokenStream>) -> Result<Box<dyn TokenStream>> {
        Ok(Box::new(new_ascii_folding_filter_preserving(
            input,
            self.preserve_original,
        )?))
    }

    /// Equivalent to `ASCIIFoldingFilterFactory.normalize(TokenStream)`: the
    /// main use-case for using preserveOriginal is to match regardless of case
    /// and to give better scores to exact matches. Since most multi-term queries
    /// return constant scores anyway, for normalization we emit only the folded
    /// token, `new ASCIIFoldingFilter(input, false)`.
    fn normalize(&self, input: Box<dyn TokenStream>) -> Result<Box<dyn TokenStream>> {
        Ok(Box::new(new_ascii_folding_filter_preserving(input, false)?))
    }
}
