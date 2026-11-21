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

//! Factory for `EdgeNGramTokenFilter`, ported from
//! `org.apache.lucene.analysis.ngram.EdgeNGramFilterFactory`.

use std::collections::HashMap;
use std::sync::Arc;

use rucene::analysis::factories::{
    AbstractAnalysisFactory, AnalysisFactoryClass, SPINameField, TokenFilterFactory,
};
use rucene::analysis::TokenStream;
use rucene::error::Result;

use super::edge_ngram_token_filter::{new_edge_ngram_token_filter, DEFAULT_PRESERVE_ORIGINAL};
use crate::spi::check_unknown_parameters;

/// Factory for `EdgeNGramTokenFilter`.
///
/// ```text
/// <fieldType name="text_edgngrm" class="solr.TextField" positionIncrementGap="100">
///   <analyzer>
///     <tokenizer class="solr.WhitespaceTokenizerFactory"/>
///     <filter class="solr.EdgeNGramFilterFactory" minGramSize="1" maxGramSize="2" preserveOriginal="true"/>
///   </analyzer>
/// </fieldType>
/// ```
///
/// Equivalent to `org.apache.lucene.analysis.ngram.EdgeNGramFilterFactory`
/// (since 3.1).
#[derive(Debug)]
pub struct EdgeNGramFilterFactory {
    base: AbstractAnalysisFactory,
    max_gram_size: i32,
    min_gram_size: i32,
    preserve_original: bool,
}

impl EdgeNGramFilterFactory {
    /// SPI name.
    ///
    /// Equivalent to `EdgeNGramFilterFactory.NAME`.
    pub const NAME: &'static str = "edgeNGram";

    /// Creates a new EdgeNGramFilterFactory.
    ///
    /// Equivalent to `EdgeNGramFilterFactory(Map<String, String>)`.
    ///
    /// # Errors
    ///
    /// Returns what [`AbstractAnalysisFactory::new`] and
    /// [`AbstractAnalysisFactory::require_int`] return, and
    /// `IllegalArgumentException` for a leftover argument.
    pub fn new(args: &mut HashMap<String, String>) -> Result<Self> {
        let base = AbstractAnalysisFactory::new(args)?;
        let min_gram_size = AbstractAnalysisFactory::require_int(args, "minGramSize")?;
        let max_gram_size = AbstractAnalysisFactory::require_int(args, "maxGramSize")?;
        let preserve_original = AbstractAnalysisFactory::get_boolean(
            args,
            "preserveOriginal",
            DEFAULT_PRESERVE_ORIGINAL,
        );
        check_unknown_parameters(args)?;
        Ok(Self {
            base,
            max_gram_size,
            min_gram_size,
            preserve_original,
        })
    }

    /// Default ctor for compatibility with SPI.
    ///
    /// Equivalent to `EdgeNGramFilterFactory()`.
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
    /// names it: `EdgeNGramFilterFactory.class`, with its `NAME` field and its
    /// `Map<String, String>` constructor.
    pub const fn class() -> AnalysisFactoryClass<dyn TokenFilterFactory> {
        AnalysisFactoryClass::new(
            "org.apache.lucene.analysis.ngram.EdgeNGramFilterFactory",
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

impl TokenFilterFactory for EdgeNGramFilterFactory {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.analysis.ngram.EdgeNGramFilterFactory"
    }

    /// Equivalent to `EdgeNGramFilterFactory.create(TokenStream)`, `new
    /// EdgeNGramTokenFilter(input, minGramSize, maxGramSize, preserveOriginal)`.
    fn create(&self, input: Box<dyn TokenStream>) -> Result<Box<dyn TokenStream>> {
        Ok(Box::new(new_edge_ngram_token_filter(
            input,
            self.min_gram_size,
            self.max_gram_size,
            self.preserve_original,
        )?))
    }
}
