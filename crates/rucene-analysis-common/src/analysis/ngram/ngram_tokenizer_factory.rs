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

//! Factory for `NGramTokenizer`, ported from
//! `org.apache.lucene.analysis.ngram.NGramTokenizerFactory`.

use std::collections::HashMap;
use std::sync::Arc;

use rucene::analysis::factories::{
    AbstractAnalysisFactory, AnalysisFactoryClass, FactoryTokenizer, SPINameField, TokenizerFactory,
};
use rucene::analysis::Tokenizer;
use rucene::error::Result;
use rucene::util::attribute::AttributeFactory;

use super::ngram_tokenizer::ngram_tokenizer_parts;
use super::ngram_tokenizer::{DEFAULT_MAX_NGRAM_SIZE, DEFAULT_MIN_NGRAM_SIZE};
use crate::spi::check_unknown_parameters;

/// Factory for `NGramTokenizer`.
///
/// ```text
/// <fieldType name="text_ngrm" class="solr.TextField" positionIncrementGap="100">
///   <analyzer>
///     <tokenizer class="solr.NGramTokenizerFactory" minGramSize="1" maxGramSize="2"/>
///   </analyzer>
/// </fieldType>
/// ```
///
/// Equivalent to `org.apache.lucene.analysis.ngram.NGramTokenizerFactory`
/// (since 3.1).
#[derive(Debug)]
pub struct NGramTokenizerFactory {
    base: AbstractAnalysisFactory,
    max_gram_size: i32,
    min_gram_size: i32,
}

impl NGramTokenizerFactory {
    /// SPI name.
    ///
    /// Equivalent to `NGramTokenizerFactory.NAME`.
    pub const NAME: &'static str = "nGram";

    /// Creates a new NGramTokenizerFactory.
    ///
    /// Equivalent to `NGramTokenizerFactory(Map<String, String>)`.
    ///
    /// # Errors
    ///
    /// Returns what [`AbstractAnalysisFactory::new`] and
    /// [`AbstractAnalysisFactory::get_int`] return, and
    /// `IllegalArgumentException` for a leftover argument.
    pub fn new(args: &mut HashMap<String, String>) -> Result<Self> {
        let base = AbstractAnalysisFactory::new(args)?;
        let min_gram_size =
            AbstractAnalysisFactory::get_int(args, "minGramSize", DEFAULT_MIN_NGRAM_SIZE)?;
        let max_gram_size =
            AbstractAnalysisFactory::get_int(args, "maxGramSize", DEFAULT_MAX_NGRAM_SIZE)?;
        check_unknown_parameters(args)?;
        Ok(Self {
            base,
            max_gram_size,
            min_gram_size,
        })
    }

    /// Default ctor for compatibility with SPI.
    ///
    /// Equivalent to `NGramTokenizerFactory()`.
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
    /// names it: `NGramTokenizerFactory.class`, with its `NAME` field and its
    /// `Map<String, String>` constructor.
    pub const fn class() -> AnalysisFactoryClass<dyn TokenizerFactory> {
        AnalysisFactoryClass::new(
            "org.apache.lucene.analysis.ngram.NGramTokenizerFactory",
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

impl TokenizerFactory for NGramTokenizerFactory {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.analysis.ngram.NGramTokenizerFactory"
    }

    /// Creates the `TokenStream` of n-grams from the given `Reader` and
    /// `AttributeFactory`.
    ///
    /// Equivalent to `NGramTokenizerFactory.create(AttributeFactory)`, `new
    /// NGramTokenizer(factory, minGramSize, maxGramSize)`.
    fn create_with_factory(&self, factory: Arc<dyn AttributeFactory>) -> Result<FactoryTokenizer> {
        let (source, logic) = ngram_tokenizer_parts(
            factory,
            self.min_gram_size,
            self.max_gram_size,
            false,
            Box::new(|_| true),
        )?;
        Ok(Tokenizer::with_source(source, Box::new(logic)))
    }
}
