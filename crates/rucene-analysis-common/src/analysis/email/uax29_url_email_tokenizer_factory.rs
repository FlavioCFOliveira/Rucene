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

//! Factory for `UAX29URLEmailTokenizer`, ported from
//! `org.apache.lucene.analysis.email.UAX29URLEmailTokenizerFactory`.

use std::collections::HashMap;
use std::sync::Arc;

use rucene::analysis::factories::{
    AbstractAnalysisFactory, AnalysisFactoryClass, FactoryTokenizer, SPINameField, TokenizerFactory,
};
use rucene::analysis::{Tokenizer, DEFAULT_MAX_TOKEN_LENGTH};
use rucene::error::Result;
use rucene::util::attribute::AttributeFactory;

use super::uax29_url_email_tokenizer::{
    uax29_url_email_tokenizer_source, UAX29URLEmailTokenizerLogic,
};
use crate::spi::check_unknown_parameters;

/// Factory for `UAX29URLEmailTokenizer`.
///
/// ```text
/// <fieldType name="text_urlemail" class="solr.TextField" positionIncrementGap="100">
///   <analyzer>
///     <tokenizer class="solr.UAX29URLEmailTokenizerFactory" maxTokenLength="255"/>
///   </analyzer>
/// </fieldType>
/// ```
///
/// Equivalent to `org.apache.lucene.analysis.email.UAX29URLEmailTokenizerFactory`
/// (since 3.1).
#[derive(Debug)]
pub struct UAX29URLEmailTokenizerFactory {
    base: AbstractAnalysisFactory,
    max_token_length: i32,
}

impl UAX29URLEmailTokenizerFactory {
    /// SPI name
    ///
    /// Equivalent to `UAX29URLEmailTokenizerFactory.NAME`.
    pub const NAME: &'static str = "uax29UrlEmail";

    /// Creates a new UAX29URLEmailTokenizerFactory
    ///
    /// Equivalent to `UAX29URLEmailTokenizerFactory(Map<String, String>)`:
    /// `maxTokenLength` defaults to `StandardAnalyzer.DEFAULT_MAX_TOKEN_LENGTH`.
    ///
    /// # Errors
    ///
    /// Returns what [`AbstractAnalysisFactory::new`] and
    /// [`AbstractAnalysisFactory::get_int`] return, and
    /// `IllegalArgumentException` for a leftover argument.
    pub fn new(args: &mut HashMap<String, String>) -> Result<Self> {
        let base = AbstractAnalysisFactory::new(args)?;
        let max_token_length = AbstractAnalysisFactory::get_int(
            args,
            "maxTokenLength",
            DEFAULT_MAX_TOKEN_LENGTH as i32,
        )?;
        check_unknown_parameters(args)?;
        Ok(Self {
            base,
            max_token_length,
        })
    }

    /// Default ctor for compatibility with SPI
    ///
    /// Equivalent to `UAX29URLEmailTokenizerFactory()`.
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
    /// names it: `UAX29URLEmailTokenizerFactory.class`, with its `NAME` field
    /// and its `Map<String, String>` constructor.
    pub const fn class() -> AnalysisFactoryClass<dyn TokenizerFactory> {
        AnalysisFactoryClass::new(
            "org.apache.lucene.analysis.email.UAX29URLEmailTokenizerFactory",
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

impl TokenizerFactory for UAX29URLEmailTokenizerFactory {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.analysis.email.UAX29URLEmailTokenizerFactory"
    }

    /// Equivalent to `UAX29URLEmailTokenizerFactory.create(AttributeFactory)`:
    /// a `new UAX29URLEmailTokenizer(factory)` whose maximum token length is
    /// set.
    fn create_with_factory(&self, factory: Arc<dyn AttributeFactory>) -> Result<FactoryTokenizer> {
        let source = uax29_url_email_tokenizer_source(factory)?;
        let mut logic = UAX29URLEmailTokenizerLogic::new();
        logic.set_max_token_length(self.max_token_length)?;
        Ok(Tokenizer::with_source(source, Box::new(logic)))
    }
}
