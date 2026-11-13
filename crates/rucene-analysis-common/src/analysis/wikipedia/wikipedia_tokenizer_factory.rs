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

//! Factory for `WikipediaTokenizer`, ported from
//! `org.apache.lucene.analysis.wikipedia.WikipediaTokenizerFactory`.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use rucene::analysis::factories::{
    AbstractAnalysisFactory, AnalysisFactoryClass, FactoryTokenizer, SPINameField, TokenizerFactory,
};
use rucene::analysis::Tokenizer;
use rucene::error::Result;
use rucene::util::attribute::AttributeFactory;

use super::wikipedia_tokenizer::{
    wikipedia_tokenizer_source, WikipediaTokenizerLogic, TOKENS_ONLY,
};
use crate::spi::check_unknown_parameters;

/// Factory for `WikipediaTokenizer`.
///
/// ```text
/// <fieldType name="text_wiki" class="solr.TextField" positionIncrementGap="100">
///   <analyzer>
///     <tokenizer class="solr.WikipediaTokenizerFactory"/>
///   </analyzer>
/// </fieldType>
/// ```
///
/// Equivalent to `org.apache.lucene.analysis.wikipedia.WikipediaTokenizerFactory`
/// (since 3.1).
#[derive(Debug)]
pub struct WikipediaTokenizerFactory {
    base: AbstractAnalysisFactory,
    token_output: i32,
    untokenized_types: HashSet<String>,
}

impl WikipediaTokenizerFactory {
    /// SPI name
    ///
    /// Equivalent to `WikipediaTokenizerFactory.NAME`.
    pub const NAME: &'static str = "wikipedia";

    /// Equivalent to `WikipediaTokenizerFactory.TOKEN_OUTPUT`.
    pub const TOKEN_OUTPUT: &'static str = "tokenOutput";
    /// Equivalent to `WikipediaTokenizerFactory.UNTOKENIZED_TYPES`.
    pub const UNTOKENIZED_TYPES: &'static str = "untokenizedTypes";

    /// Creates a new WikipediaTokenizerFactory
    ///
    /// Equivalent to `WikipediaTokenizerFactory(Map<String, String>)`: an
    /// absent `untokenizedTypes` is `Collections.emptySet()`.
    ///
    /// # Errors
    ///
    /// Returns what [`AbstractAnalysisFactory::new`] and
    /// [`AbstractAnalysisFactory::get_int`] return, and
    /// `IllegalArgumentException` for a leftover argument.
    pub fn new(args: &mut HashMap<String, String>) -> Result<Self> {
        let base = AbstractAnalysisFactory::new(args)?;
        let token_output = AbstractAnalysisFactory::get_int(args, Self::TOKEN_OUTPUT, TOKENS_ONLY)?;
        let untokenized_types: HashSet<String> =
            match AbstractAnalysisFactory::get_set(args, Self::UNTOKENIZED_TYPES) {
                Some(set) => set.iter().cloned().collect(),
                None => HashSet::new(),
            };
        check_unknown_parameters(args)?;
        Ok(Self {
            base,
            token_output,
            untokenized_types,
        })
    }

    /// Default ctor for compatibility with SPI
    ///
    /// Equivalent to `WikipediaTokenizerFactory()`.
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

    /// Equivalent to the protected field `WikipediaTokenizerFactory.tokenOutput`.
    pub fn token_output(&self) -> i32 {
        self.token_output
    }

    /// Equivalent to the protected field `WikipediaTokenizerFactory.untokenizedTypes`.
    pub fn untokenized_types(&self) -> &HashSet<String> {
        &self.untokenized_types
    }

    /// The factory's class, as `META-INF/services/org.apache.lucene.analysis.TokenizerFactory`
    /// names it: `WikipediaTokenizerFactory.class`, with its `NAME` field and
    /// its `Map<String, String>` constructor.
    pub const fn class() -> AnalysisFactoryClass<dyn TokenizerFactory> {
        AnalysisFactoryClass::new(
            "org.apache.lucene.analysis.wikipedia.WikipediaTokenizerFactory",
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

impl TokenizerFactory for WikipediaTokenizerFactory {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.analysis.wikipedia.WikipediaTokenizerFactory"
    }

    /// Equivalent to `WikipediaTokenizerFactory.create(AttributeFactory)`,
    /// `new WikipediaTokenizer(factory, tokenOutput, untokenizedTypes)`.
    fn create_with_factory(&self, factory: Arc<dyn AttributeFactory>) -> Result<FactoryTokenizer> {
        let source = wikipedia_tokenizer_source(factory)?;
        let logic =
            WikipediaTokenizerLogic::new(self.token_output, self.untokenized_types.clone())?;
        Ok(Tokenizer::with_source(source, Box::new(logic)))
    }
}
