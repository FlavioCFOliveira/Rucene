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
//! Factory for `CapitalizationFilter`, ported from
//! `org.apache.lucene.analysis.miscellaneous.CapitalizationFilterFactory`.

use std::collections::HashMap;
use std::sync::Arc;

use rucene::analysis::factories::{
    AbstractAnalysisFactory, AnalysisFactoryClass, SPINameField, TokenFilterFactory,
};
use rucene::analysis::CharArraySet;
use rucene::analysis::TokenStream;
use rucene::error::Result;

use super::capitalization_filter::{
    new_capitalization_filter_with, DEFAULT_MAX_TOKEN_LENGTH, DEFAULT_MAX_WORD_COUNT,
};
use crate::spi::check_unknown_parameters;

/// Factory for `CapitalizationFilter`.
///
/// The factory takes parameters:
///
/// * "onlyFirstWord" - should each word be capitalized or all of the words?
/// * "keep" - a keep word list. Each word that should be kept separated by
///   whitespace.
/// * "keepIgnoreCase - true or false. If true, the keep list will be considered
///   case-insensitive.
/// * "forceFirstLetter" - Force the first letter to be capitalized even if it is
///   in the keep list
/// * "okPrefix" - do not change word capitalization if a word begins with
///   something in this list. for example if "McK" is on the okPrefix list, the
///   word "McKinley" should not be changed to "Mckinley"
/// * "minWordLength" - how long the word needs to be to get capitalization
///   applied. If the minWordLength is 3, "and" > "And" but "or" stays "or"
/// * "maxWordCount" - if the token contains more then maxWordCount words, the
///   capitalization is assumed to be correct.
///
/// ```text
/// <fieldType name="text_cptlztn" class="solr.TextField" positionIncrementGap="100">
///   <analyzer>
///     <tokenizer class="solr.WhitespaceTokenizerFactory"/>
///     <filter class="solr.CapitalizationFilterFactory" onlyFirstWord="true"
///           keep="java solr lucene" keepIgnoreCase="false"
///           okPrefix="McK McD McA"/>
///   </analyzer>
/// </fieldType>
/// ```
///
/// Equivalent to `org.apache.lucene.analysis.miscellaneous.CapitalizationFilterFactory` (since solr 1.3).
#[derive(Debug)]
pub struct CapitalizationFilterFactory {
    base: AbstractAnalysisFactory,
    keep: Option<CharArraySet>,
    /// Java's `Collection<char[]> okPrefix`, `Collections.emptyList()` by
    /// default (for Example: McK), each prefix as UTF-16 units.
    ok_prefix: Vec<Vec<u16>>,
    /// don't modify capitalization for words shorter then this
    min_word_length: i32,
    max_word_count: i32,
    max_token_length: i32,
    only_first_word: bool,
    /// make sure the first letter is capital even if it is in the keep list
    force_first_letter: bool,
}

impl CapitalizationFilterFactory {
    /// SPI name.
    ///
    /// Equivalent to `CapitalizationFilterFactory.NAME`.
    pub const NAME: &'static str = "capitalization";

    /// Equivalent to `CapitalizationFilterFactory.KEEP`.
    pub const KEEP: &'static str = "keep";
    /// Equivalent to `CapitalizationFilterFactory.KEEP_IGNORE_CASE`.
    pub const KEEP_IGNORE_CASE: &'static str = "keepIgnoreCase";
    /// Equivalent to `CapitalizationFilterFactory.OK_PREFIX`.
    pub const OK_PREFIX: &'static str = "okPrefix";
    /// Equivalent to `CapitalizationFilterFactory.MIN_WORD_LENGTH`.
    pub const MIN_WORD_LENGTH: &'static str = "minWordLength";
    /// Equivalent to `CapitalizationFilterFactory.MAX_WORD_COUNT`.
    pub const MAX_WORD_COUNT: &'static str = "maxWordCount";
    /// Equivalent to `CapitalizationFilterFactory.MAX_TOKEN_LENGTH`.
    pub const MAX_TOKEN_LENGTH: &'static str = "maxTokenLength";
    /// Equivalent to `CapitalizationFilterFactory.ONLY_FIRST_WORD`.
    pub const ONLY_FIRST_WORD: &'static str = "onlyFirstWord";
    /// Equivalent to `CapitalizationFilterFactory.FORCE_FIRST_LETTER`.
    pub const FORCE_FIRST_LETTER: &'static str = "forceFirstLetter";

    /// Creates a new CapitalizationFilterFactory.
    ///
    /// Equivalent to `CapitalizationFilterFactory(Map<String, String>)`.
    ///
    /// # Errors
    ///
    /// Returns what [`AbstractAnalysisFactory::new`] returns, what an unparsable integer argument raises, and `IllegalArgumentException` for a leftover argument.
    pub fn new(args: &mut HashMap<String, String>) -> Result<Self> {
        let base = AbstractAnalysisFactory::new(args)?;
        let ignore_case = AbstractAnalysisFactory::get_boolean(args, Self::KEEP_IGNORE_CASE, false);
        let mut keep = None;
        if let Some(k) = AbstractAnalysisFactory::get_set(args, Self::KEEP) {
            let mut set = CharArraySet::new(10, ignore_case);
            set.add_all(k.iter());
            keep = Some(set);
        }

        let mut ok_prefix = Vec::new();
        if let Some(k) = AbstractAnalysisFactory::get_set(args, Self::OK_PREFIX) {
            for item in k.iter() {
                ok_prefix.push(item.encode_utf16().collect());
            }
        }

        let min_word_length = AbstractAnalysisFactory::get_int(args, Self::MIN_WORD_LENGTH, 0)?;
        let max_word_count =
            AbstractAnalysisFactory::get_int(args, Self::MAX_WORD_COUNT, DEFAULT_MAX_WORD_COUNT)?;
        let max_token_length = AbstractAnalysisFactory::get_int(
            args,
            Self::MAX_TOKEN_LENGTH,
            DEFAULT_MAX_TOKEN_LENGTH,
        )?;
        let only_first_word =
            AbstractAnalysisFactory::get_boolean(args, Self::ONLY_FIRST_WORD, true);
        let force_first_letter =
            AbstractAnalysisFactory::get_boolean(args, Self::FORCE_FIRST_LETTER, true);
        check_unknown_parameters(args)?;
        Ok(Self {
            base,
            keep,
            ok_prefix,
            min_word_length,
            max_word_count,
            max_token_length,
            only_first_word,
            force_first_letter,
        })
    }

    /// Default ctor for compatibility with SPI.
    ///
    /// Equivalent to `CapitalizationFilterFactory()`.
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
    /// names it: `CapitalizationFilterFactory.class`, with its `NAME` field and its
    /// `Map<String, String>` constructor.
    pub const fn class() -> AnalysisFactoryClass<dyn TokenFilterFactory> {
        AnalysisFactoryClass::new(
            "org.apache.lucene.analysis.miscellaneous.CapitalizationFilterFactory",
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

impl TokenFilterFactory for CapitalizationFilterFactory {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.analysis.miscellaneous.CapitalizationFilterFactory"
    }

    /// Equivalent to `CapitalizationFilterFactory.create(TokenStream)`.
    fn create(&self, input: Box<dyn TokenStream>) -> Result<Box<dyn TokenStream>> {
        Ok(Box::new(new_capitalization_filter_with(
            input,
            self.only_first_word,
            self.keep.clone(),
            self.force_first_letter,
            Some(self.ok_prefix.clone()),
            self.min_word_length,
            self.max_word_count,
            self.max_token_length,
        )?))
    }
}
