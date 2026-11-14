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

//! Factory for `WhitespaceTokenizer`, ported from
//! `org.apache.lucene.analysis.core.WhitespaceTokenizerFactory`.

use std::collections::HashMap;
use std::sync::Arc;

use rucene::analysis::factories::{
    AbstractAnalysisFactory, AnalysisFactoryClass, FactoryTokenizer, SPINameField, TokenizerFactory,
};
use rucene::analysis::{Tokenizer, MAX_TOKEN_LENGTH_LIMIT};
use rucene::error::{LuceneError, Result};
use rucene::util::attribute::AttributeFactory;

use super::unicode_whitespace_tokenizer::UnicodeWhitespaceTokenizerLogic;
use super::whitespace_tokenizer::WhitespaceTokenizerLogic;
use crate::analysis::util::char_tokenizer::{
    char_tokenizer_source, CharTokenizerAdapter, DEFAULT_MAX_WORD_LEN,
};
use crate::spi::check_unknown_parameters;

/// Factory for `WhitespaceTokenizer`.
///
/// ```text
/// <fieldType name="text_ws" class="solr.TextField" positionIncrementGap="100">
///   <analyzer>
///     <tokenizer class="solr.WhitespaceTokenizerFactory" rule="unicode"  maxTokenLen="256"/>
///   </analyzer>
/// </fieldType>
/// ```
///
/// Options:
///
/// * rule: either "java" for `WhitespaceTokenizer` or "unicode" for
///   `UnicodeWhitespaceTokenizer`
/// * maxTokenLen: max token length, should be greater than 0 and less than
///   MAX_TOKEN_LENGTH_LIMIT (1024*1024). It is rare to need to change this
///   else `CharTokenizer`::DEFAULT_MAX_TOKEN_LEN
///
/// Equivalent to `org.apache.lucene.analysis.core.WhitespaceTokenizerFactory`
/// (since 3.1).
#[derive(Debug)]
pub struct WhitespaceTokenizerFactory {
    base: AbstractAnalysisFactory,
    rule: String,
    max_token_len: i32,
}

impl WhitespaceTokenizerFactory {
    /// SPI name.
    ///
    /// Equivalent to `WhitespaceTokenizerFactory.NAME`.
    pub const NAME: &'static str = "whitespace";

    /// Equivalent to `WhitespaceTokenizerFactory.RULE_JAVA`.
    pub const RULE_JAVA: &'static str = "java";

    /// Equivalent to `WhitespaceTokenizerFactory.RULE_UNICODE`.
    pub const RULE_UNICODE: &'static str = "unicode";

    /// Equivalent to the private `RULE_NAMES`, `Arrays.asList(RULE_JAVA,
    /// RULE_UNICODE)`.
    const RULE_NAMES: [&'static str; 2] = [Self::RULE_JAVA, Self::RULE_UNICODE];

    /// Creates a new WhitespaceTokenizerFactory.
    ///
    /// Equivalent to `WhitespaceTokenizerFactory(Map<String, String>)`.
    ///
    /// # Errors
    ///
    /// Returns what [`AbstractAnalysisFactory::new`] returns, and
    /// `IllegalArgumentException` for an invalid argument (with Java's
    /// messages) or a leftover one.
    pub fn new(args: &mut HashMap<String, String>) -> Result<Self> {
        let base = AbstractAnalysisFactory::new(args)?;

        let rule = AbstractAnalysisFactory::get_one_of(
            args,
            "rule",
            &Self::RULE_NAMES,
            Some(Self::RULE_JAVA),
            true,
        )?
        .expect("INVARIANT: the default rule is not null");
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
            rule,
            max_token_len,
        })
    }

    /// Default ctor for compatibility with SPI.
    ///
    /// Equivalent to `WhitespaceTokenizerFactory()`.
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
    /// names it: `WhitespaceTokenizerFactory.class`, with its `NAME` field and its
    /// `Map<String, String>` constructor.
    pub const fn class() -> AnalysisFactoryClass<dyn TokenizerFactory> {
        AnalysisFactoryClass::new(
            "org.apache.lucene.analysis.core.WhitespaceTokenizerFactory",
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

impl TokenizerFactory for WhitespaceTokenizerFactory {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.analysis.core.WhitespaceTokenizerFactory"
    }

    /// Equivalent to `WhitespaceTokenizerFactory.create(AttributeFactory)`: a
    /// `WhitespaceTokenizer` for the rule `java`, a
    /// `UnicodeWhitespaceTokenizer` for the rule `unicode`, over `factory`
    /// and with `maxTokenLen`.
    fn create_with_factory(&self, factory: Arc<dyn AttributeFactory>) -> Result<FactoryTokenizer> {
        let source = char_tokenizer_source(factory)?;
        match self.rule.as_str() {
            Self::RULE_JAVA => Ok(Tokenizer::with_source(
                source,
                Box::new(CharTokenizerAdapter::with_max_token_len(
                    WhitespaceTokenizerLogic,
                    self.max_token_len,
                )?),
            )),
            Self::RULE_UNICODE => Ok(Tokenizer::with_source(
                source,
                Box::new(CharTokenizerAdapter::with_max_token_len(
                    UnicodeWhitespaceTokenizerLogic,
                    self.max_token_len,
                )?),
            )),
            _ => rucene::store::java_nio_file::throw_unchecked("java.lang.AssertionError", None),
        }
    }
}
