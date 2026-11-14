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

//! Factory for `LetterTokenizer`, ported from
//! `org.apache.lucene.analysis.core.LetterTokenizerFactory`.

use std::collections::HashMap;
use std::sync::Arc;

use rucene::analysis::factories::{
    AbstractAnalysisFactory, AnalysisFactoryClass, FactoryTokenizer, SPINameField, TokenizerFactory,
};
use rucene::analysis::{Tokenizer, MAX_TOKEN_LENGTH_LIMIT};
use rucene::error::{LuceneError, Result};
use rucene::util::attribute::AttributeFactory;

use super::letter_tokenizer::LetterTokenizerLogic;
use crate::analysis::util::char_tokenizer::{
    char_tokenizer_source, CharTokenizerAdapter, DEFAULT_MAX_WORD_LEN,
};
use crate::spi::check_unknown_parameters;

/// Factory for `LetterTokenizer`.
///
/// ```text
/// <fieldType name="text_letter" class="solr.TextField" positionIncrementGap="100">
///   <analyzer>
///     <tokenizer class="solr.LetterTokenizerFactory" maxTokenLen="256"/>
///   </analyzer>
/// </fieldType>
/// ```
///
/// Options:
///
/// * maxTokenLen: max token length, must be greater than 0 and less than
///   MAX_TOKEN_LENGTH_LIMIT (1024*1024). It is rare to need to change this
///   else `CharTokenizer`::DEFAULT_MAX_TOKEN_LEN
///
/// Equivalent to `org.apache.lucene.analysis.core.LetterTokenizerFactory`
/// (since 3.1).
#[derive(Debug)]
pub struct LetterTokenizerFactory {
    base: AbstractAnalysisFactory,
    max_token_len: i32,
}

impl LetterTokenizerFactory {
    /// SPI name.
    ///
    /// Equivalent to `LetterTokenizerFactory.NAME`.
    pub const NAME: &'static str = "letter";

    /// Creates a new LetterTokenizerFactory.
    ///
    /// Equivalent to `LetterTokenizerFactory(Map<String, String>)`.
    ///
    /// # Errors
    ///
    /// Returns what [`AbstractAnalysisFactory::new`] returns, and
    /// `IllegalArgumentException` for an invalid argument (with Java's
    /// messages) or a leftover one.
    pub fn new(args: &mut HashMap<String, String>) -> Result<Self> {
        let base = AbstractAnalysisFactory::new(args)?;
        let max_token_len =
            AbstractAnalysisFactory::get_int(args, "maxTokenLen", DEFAULT_MAX_WORD_LEN)?;
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
    /// Equivalent to `LetterTokenizerFactory()`.
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
    /// names it: `LetterTokenizerFactory.class`, with its `NAME` field and its
    /// `Map<String, String>` constructor.
    pub const fn class() -> AnalysisFactoryClass<dyn TokenizerFactory> {
        AnalysisFactoryClass::new(
            "org.apache.lucene.analysis.core.LetterTokenizerFactory",
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

impl TokenizerFactory for LetterTokenizerFactory {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.analysis.core.LetterTokenizerFactory"
    }

    /// Equivalent to `LetterTokenizerFactory.create(AttributeFactory)`, `new
    /// LetterTokenizer(factory, maxTokenLen)`.
    fn create_with_factory(&self, factory: Arc<dyn AttributeFactory>) -> Result<FactoryTokenizer> {
        let source = char_tokenizer_source(factory)?;
        let logic =
            CharTokenizerAdapter::with_max_token_len(LetterTokenizerLogic, self.max_token_len)?;
        Ok(Tokenizer::with_source(source, Box::new(logic)))
    }
}
