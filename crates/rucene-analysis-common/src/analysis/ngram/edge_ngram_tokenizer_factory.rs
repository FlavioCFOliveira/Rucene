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

//! Factory for `EdgeNGramTokenizer`, ported from
//! `org.apache.lucene.analysis.ngram.EdgeNGramTokenizerFactory`.

use std::collections::HashMap;
use std::sync::Arc;

use rucene::analysis::factories::{
    AbstractAnalysisFactory, AnalysisFactoryClass, FactoryTokenizer, SPINameField, TokenizerFactory,
};
use rucene::analysis::Tokenizer;
use rucene::error::Result;
use rucene::util::attribute::AttributeFactory;

use super::edge_ngram_tokenizer::{DEFAULT_MAX_GRAM_SIZE, DEFAULT_MIN_GRAM_SIZE};
use super::ngram_tokenizer::ngram_tokenizer_parts;
use crate::spi::check_unknown_parameters;

/// Factory for `EdgeNGramTokenizer`.
///
/// ```text
/// <fieldType name="text_edgngrm" class="solr.TextField" positionIncrementGap="100">
///   <analyzer>
///     <tokenizer class="solr.EdgeNGramTokenizerFactory" minGramSize="1" maxGramSize="1"/>
///   </analyzer>
/// </fieldType>
/// ```
///
/// Equivalent to `org.apache.lucene.analysis.ngram.EdgeNGramTokenizerFactory`
/// (since 3.1).
#[derive(Debug)]
pub struct EdgeNGramTokenizerFactory {
    base: AbstractAnalysisFactory,
    max_gram_size: i32,
    min_gram_size: i32,
}

impl EdgeNGramTokenizerFactory {
    /// SPI name.
    ///
    /// Equivalent to `EdgeNGramTokenizerFactory.NAME`.
    pub const NAME: &'static str = "edgeNGram";

    /// Creates a new EdgeNGramTokenizerFactory.
    ///
    /// Equivalent to `EdgeNGramTokenizerFactory(Map<String, String>)`.
    ///
    /// # Errors
    ///
    /// Returns what [`AbstractAnalysisFactory::new`] and
    /// [`AbstractAnalysisFactory::get_int`] return, and
    /// `IllegalArgumentException` for a leftover argument.
    pub fn new(args: &mut HashMap<String, String>) -> Result<Self> {
        let base = AbstractAnalysisFactory::new(args)?;
        let min_gram_size =
            AbstractAnalysisFactory::get_int(args, "minGramSize", DEFAULT_MIN_GRAM_SIZE)?;
        let max_gram_size =
            AbstractAnalysisFactory::get_int(args, "maxGramSize", DEFAULT_MAX_GRAM_SIZE)?;
        check_unknown_parameters(args)?;
        Ok(Self {
            base,
            max_gram_size,
            min_gram_size,
        })
    }

    /// Default ctor for compatibility with SPI.
    ///
    /// Equivalent to `EdgeNGramTokenizerFactory()`.
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

    /// The factory's class, as `META-INF/services/org.apache.lucene.analysis.TokenizerFactory`
    /// names it: `EdgeNGramTokenizerFactory.class`, with its `NAME` field and its
    /// `Map<String, String>` constructor.
    pub const fn class() -> AnalysisFactoryClass<dyn TokenizerFactory> {
        AnalysisFactoryClass::new(
            "org.apache.lucene.analysis.ngram.EdgeNGramTokenizerFactory",
            SPINameField::Declared(Self::NAME),
            Some(Self::new_from_map),
        )
    }

    /// The `Map<String, String>` constructor, as [`class`](Self::class)
    /// exposes it.
    fn new_from_map(args: &mut HashMap<String, String>) -> Result<Arc<dyn TokenizerFactory>> {
        Ok(Arc::new(Self::new(args)?))
    }
}

impl TokenizerFactory for EdgeNGramTokenizerFactory {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.analysis.ngram.EdgeNGramTokenizerFactory"
    }

    /// Equivalent to `EdgeNGramTokenizerFactory.create(AttributeFactory)`, `new
    /// EdgeNGramTokenizer(factory, minGramSize, maxGramSize)`.
    fn create_with_factory(&self, factory: Arc<dyn AttributeFactory>) -> Result<FactoryTokenizer> {
        let (source, logic) = ngram_tokenizer_parts(
            factory,
            self.min_gram_size,
            self.max_gram_size,
            true,
            Box::new(|_| true),
        )?;
        Ok(Tokenizer::with_source(source, Box::new(logic)))
    }
}
