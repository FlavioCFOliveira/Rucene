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

//! Factory for `KeywordTokenizer`, ported from
//! `org.apache.lucene.analysis.core.KeywordTokenizerFactory`.

use std::collections::HashMap;
use std::sync::Arc;

use rucene::analysis::factories::{
    AbstractAnalysisFactory, AnalysisFactoryClass, FactoryTokenizer, SPINameField, TokenizerFactory,
};
use rucene::analysis::{Tokenizer, MAX_TOKEN_LENGTH_LIMIT};
use rucene::error::{LuceneError, Result};
use rucene::util::attribute::AttributeFactory;

use super::keyword_tokenizer::{keyword_tokenizer_parts, DEFAULT_BUFFER_SIZE};
use crate::spi::check_unknown_parameters;

/// Factory for `KeywordTokenizer`.
///
/// ```text
/// <fieldType name="text_keyword" class="solr.TextField" positionIncrementGap="100">
///   <analyzer>
///     <tokenizer class="solr.KeywordTokenizerFactory" maxTokenLen="256"/>
///   </analyzer>
/// </fieldType>
/// ```
///
/// Options:
///
/// * maxTokenLen: max token length, should be greater than 0 and less than
///   MAX_TOKEN_LENGTH_LIMIT (1024*1024). It is rare to need to change this
///   else `KeywordTokenizer`::DEFAULT_BUFFER_SIZE
///
/// Equivalent to `org.apache.lucene.analysis.core.KeywordTokenizerFactory`
/// (since 3.1).
#[derive(Debug)]
pub struct KeywordTokenizerFactory {
    base: AbstractAnalysisFactory,
    max_token_len: i32,
}

impl KeywordTokenizerFactory {
    /// SPI name.
    ///
    /// Equivalent to `KeywordTokenizerFactory.NAME`.
    pub const NAME: &'static str = "keyword";

    /// Creates a new KeywordTokenizerFactory.
    ///
    /// Equivalent to `KeywordTokenizerFactory(Map<String, String>)`.
    ///
    /// # Errors
    ///
    /// Returns what [`AbstractAnalysisFactory::new`] returns, and
    /// `IllegalArgumentException` for an invalid argument (with Java's
    /// messages) or a leftover one.
    pub fn new(args: &mut HashMap<String, String>) -> Result<Self> {
        let base = AbstractAnalysisFactory::new(args)?;
        let max_token_len =
            AbstractAnalysisFactory::get_int(args, "maxTokenLen", DEFAULT_BUFFER_SIZE)?;
        if max_token_len as i64 > MAX_TOKEN_LENGTH_LIMIT as i64 || max_token_len <= 0 {
            return Err(LuceneError::illegal_argument(format!(
                "maxTokenLen must be greater than 0 and less than {MAX_TOKEN_LENGTH_LIMIT} passed: {max_token_len}"
            )));
        }
        check_unknown_parameters(args)?;
        Ok(Self {
            base,
            max_token_len,
        })
    }

    /// Default ctor for compatibility with SPI.
    ///
    /// Equivalent to `KeywordTokenizerFactory()`.
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
    /// names it: `KeywordTokenizerFactory.class`, with its `NAME` field and its
    /// `Map<String, String>` constructor.
    pub const fn class() -> AnalysisFactoryClass<dyn TokenizerFactory> {
        AnalysisFactoryClass::new(
            "org.apache.lucene.analysis.core.KeywordTokenizerFactory",
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

impl TokenizerFactory for KeywordTokenizerFactory {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.analysis.core.KeywordTokenizerFactory"
    }

    /// Equivalent to `KeywordTokenizerFactory.create(AttributeFactory)`, `new
    /// KeywordTokenizer(factory, maxTokenLen)`.
    fn create_with_factory(&self, factory: Arc<dyn AttributeFactory>) -> Result<FactoryTokenizer> {
        let (source, logic) = keyword_tokenizer_parts(factory, self.max_token_len)?;
        Ok(Tokenizer::with_source(source, Box::new(logic)))
    }
}
