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

//! The type declared in `DictionaryCompoundWordTokenFilterFactory.java`, of
//! the Java package `org.apache.lucene.analysis.compound`.

use std::collections::HashMap;
use std::sync::Arc;

use rucene::analysis::factories::{
    AbstractAnalysisFactory, AnalysisFactoryClass, SPINameField, TokenFilterFactory,
};
use rucene::analysis::{CharArraySet, TokenStream};
use rucene::error::Result;
use rucene::util::resource_loader::{ResourceLoader, ResourceLoaderAware};

use super::compound_word_token_filter_base::{
    DEFAULT_MAX_SUBWORD_SIZE, DEFAULT_MIN_SUBWORD_SIZE, DEFAULT_MIN_WORD_SIZE,
};
use super::dictionary_compound_word_token_filter::new_dictionary_compound_word_token_filter_with_only_longest_match;
use crate::spi::check_unknown_parameters;

/// Factory for `DictionaryCompoundWordTokenFilter`.
///
/// ```text
/// <fieldType name="text_dictcomp" class="solr.TextField" positionIncrementGap="100">
///   <analyzer>
///     <tokenizer class="solr.WhitespaceTokenizerFactory"/>
///     <filter class="solr.DictionaryCompoundWordTokenFilterFactory" dictionary="dictionary.txt"
///         minWordSize="5" minSubwordSize="2" maxSubwordSize="15" onlyLongestMatch="true"/>
///   </analyzer>
/// </fieldType>
/// ```
///
/// Equivalent to `org.apache.lucene.analysis.compound.DictionaryCompoundWordTokenFilterFactory`
/// (since 3.1).
#[derive(Debug)]
pub struct DictionaryCompoundWordTokenFilterFactory {
    base: AbstractAnalysisFactory,
    dictionary: Option<CharArraySet>,
    dict_file: String,
    min_word_size: i32,
    min_subword_size: i32,
    max_subword_size: i32,
    only_longest_match: bool,
    only_longest_match_ignore_subwords: bool,
}

impl DictionaryCompoundWordTokenFilterFactory {
    /// SPI name
    ///
    /// Equivalent to `DictionaryCompoundWordTokenFilterFactory.NAME`.
    pub const NAME: &'static str = "dictionaryCompoundWord";

    /// Creates a new DictionaryCompoundWordTokenFilterFactory
    ///
    /// Equivalent to `DictionaryCompoundWordTokenFilterFactory(Map<String, String>)`.
    ///
    /// # Errors
    ///
    /// What [`AbstractAnalysisFactory::new`] returns, `IllegalArgumentException`
    /// for a missing `dictionary`, a size that is not an `int`, or a leftover
    /// argument (`Unknown parameters: …`).
    pub fn new(args: &mut HashMap<String, String>) -> Result<Self> {
        let base = AbstractAnalysisFactory::new(args)?;
        let dict_file = AbstractAnalysisFactory::require(args, "dictionary")?;
        let min_word_size =
            AbstractAnalysisFactory::get_int(args, "minWordSize", DEFAULT_MIN_WORD_SIZE)?;
        let min_subword_size =
            AbstractAnalysisFactory::get_int(args, "minSubwordSize", DEFAULT_MIN_SUBWORD_SIZE)?;
        let max_subword_size =
            AbstractAnalysisFactory::get_int(args, "maxSubwordSize", DEFAULT_MAX_SUBWORD_SIZE)?;
        let only_longest_match =
            AbstractAnalysisFactory::get_boolean(args, "onlyLongestMatch", true);
        let only_longest_match_ignore_subwords =
            AbstractAnalysisFactory::get_boolean(args, "onlyLongestMatchIgnoreSubwords", true);
        check_unknown_parameters(args)?;
        Ok(Self {
            base,
            dictionary: None,
            dict_file,
            min_word_size,
            min_subword_size,
            max_subword_size,
            only_longest_match,
            only_longest_match_ignore_subwords,
        })
    }

    /// Default ctor for compatibility with SPI
    ///
    /// Equivalent to `DictionaryCompoundWordTokenFilterFactory()`.
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
    /// names it, with its `NAME` field and its `Map<String, String>` constructor.
    pub const fn class() -> AnalysisFactoryClass<dyn TokenFilterFactory> {
        AnalysisFactoryClass::new(
            "org.apache.lucene.analysis.compound.DictionaryCompoundWordTokenFilterFactory",
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

impl ResourceLoaderAware for DictionaryCompoundWordTokenFilterFactory {
    /// Equivalent to `DictionaryCompoundWordTokenFilterFactory.inform(ResourceLoader)`:
    /// `getWordSet(loader, dictFile, false)`.
    fn inform(&mut self, loader: &dyn ResourceLoader) -> Result<()> {
        self.dictionary =
            AbstractAnalysisFactory::get_word_set(loader, Some(self.dict_file.as_str()), false)?;
        Ok(())
    }
}

impl TokenFilterFactory for DictionaryCompoundWordTokenFilterFactory {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.analysis.compound.DictionaryCompoundWordTokenFilterFactory"
    }

    /// Equivalent to `DictionaryCompoundWordTokenFilterFactory.create(TokenStream)`:
    /// the input itself when the dictionary is `null` (not informed).
    fn create(&self, input: Box<dyn TokenStream>) -> Result<Box<dyn TokenStream>> {
        // if the dictionary is null, it means it was empty
        let Some(dictionary) = &self.dictionary else {
            return Ok(input);
        };
        Ok(Box::new(
            new_dictionary_compound_word_token_filter_with_only_longest_match(
                input,
                Some(dictionary.clone()),
                self.min_word_size,
                self.min_subword_size,
                self.max_subword_size,
                self.only_longest_match,
                self.only_longest_match_ignore_subwords,
            )?,
        ))
    }

    fn as_resource_loader_aware(&mut self) -> Option<&mut dyn ResourceLoaderAware> {
        Some(self)
    }
}
