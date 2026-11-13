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

//! Factory for `StopFilter`, ported from
//! `org.apache.lucene.analysis.core.StopFilterFactory`.

use std::collections::HashMap;
use std::sync::Arc;

use rucene::analysis::factories::{
    AbstractAnalysisFactory, AnalysisFactoryClass, SPINameField, TokenFilterFactory,
};
use rucene::analysis::{CharArraySet, TokenStream};
use rucene::error::Result;
use rucene::util::resource_loader::{ResourceLoader, ResourceLoaderAware};

use super::stop_filter::StopFilter;
use crate::analysis::en::abstract_words_file_filter_factory::AbstractWordsFileFilterFactory;
use crate::analysis::en::english_analyzer::ENGLISH_STOP_WORDS_SET;

/// Factory for `StopFilter`.
///
/// ```text
/// <fieldType name="text_stop" class="solr.TextField" positionIncrementGap="100" autoGeneratePhraseQueries="true">
///   <analyzer>
///     <tokenizer class="solr.WhitespaceTokenizerFactory"/>
///     <filter class="solr.StopFilterFactory" ignoreCase="true"
///             words="stopwords.txt" format="wordset"
///   </analyzer>
/// </fieldType>
/// ```
///
/// All attributes are optional:
///
/// * `ignoreCase` defaults to `false`
/// * `words` should be the name of a stopwords file to parse, if not
///   specified the factory will use `EnglishAnalyzer.ENGLISH_STOP_WORDS_SET`
/// * `format` defines how the `words` file will be parsed, and defaults to
///   `wordset`. If `words` is not specified, then `format` must not be
///   specified.
///
/// The valid values for the `format` option are:
///
/// * `wordset` - This is the default format, which supports one word per
///   line (including any intra-word whitespace) and allows whole line
///   comments beginning with the "#" character. Blank lines are ignored. See
///   `WordlistLoader.getLines` for details.
/// * `snowball` - This format allows for multiple words specified on each
///   line, and trailing comments may be specified using the vertical line
///   ("|"). Blank lines are ignored. See `WordlistLoader.getSnowballWordSet`
///   for details.
///
/// Equivalent to `org.apache.lucene.analysis.core.StopFilterFactory` (since
/// 3.1), which extends `AbstractWordsFileFilterFactory`: the factory holds
/// the [`AbstractWordsFileFilterFactory`] it extends.
#[derive(Debug)]
pub struct StopFilterFactory {
    base: AbstractWordsFileFilterFactory,
}

impl StopFilterFactory {
    /// SPI name.
    ///
    /// Equivalent to `StopFilterFactory.NAME`.
    pub const NAME: &'static str = "stop";

    /// Creates a new StopFilterFactory.
    ///
    /// Equivalent to `StopFilterFactory(Map<String, String>)`, `super(args)`.
    ///
    /// # Errors
    ///
    /// Returns what [`AbstractWordsFileFilterFactory::new`] returns.
    pub fn new(args: &mut HashMap<String, String>) -> Result<Self> {
        Ok(Self {
            base: AbstractWordsFileFilterFactory::new(args)?,
        })
    }

    /// Default ctor for compatibility with SPI.
    ///
    /// Equivalent to `StopFilterFactory()`.
    ///
    /// # Errors
    ///
    /// Always: [`AbstractAnalysisFactory::default_ctor_exception`].
    pub fn new_without_args() -> Result<Self> {
        Err(AbstractAnalysisFactory::default_ctor_exception())
    }

    /// The members this factory inherits from `AbstractWordsFileFilterFactory`.
    pub fn base(&self) -> &AbstractWordsFileFilterFactory {
        &self.base
    }

    /// Equivalent to `getStopWords()`, `getWords()`.
    pub fn get_stop_words(&self) -> Option<&CharArraySet> {
        self.base.get_words()
    }

    /// Equivalent to the protected `createDefaultWords()`: `new
    /// CharArraySet(EnglishAnalyzer.ENGLISH_STOP_WORDS_SET, isIgnoreCase())`.
    fn create_default_words(base: &AbstractWordsFileFilterFactory) -> CharArraySet {
        let mut words = CharArraySet::new(ENGLISH_STOP_WORDS_SET.size(), base.is_ignore_case());
        for word in ENGLISH_STOP_WORDS_SET.iter() {
            let mut word = word.to_vec();
            let len = word.len();
            words.add_chars(&mut word, 0, len);
        }
        words
    }

    /// The factory's class, as `META-INF/services/org.apache.lucene.analysis.TokenFilterFactory`
    /// names it: `StopFilterFactory.class`, with its `NAME` field and its
    /// `Map<String, String>` constructor.
    pub const fn class() -> AnalysisFactoryClass<dyn TokenFilterFactory> {
        AnalysisFactoryClass::new(
            "org.apache.lucene.analysis.core.StopFilterFactory",
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

impl ResourceLoaderAware for StopFilterFactory {
    /// Equivalent to the inherited `AbstractWordsFileFilterFactory.inform`,
    /// over this factory's `createDefaultWords()`.
    fn inform(&mut self, loader: &dyn ResourceLoader) -> Result<()> {
        self.base.inform(loader, Self::create_default_words)
    }
}

impl TokenFilterFactory for StopFilterFactory {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.analysis.core.StopFilterFactory"
    }

    /// Equivalent to `StopFilterFactory.create(TokenStream)`, `new
    /// StopFilter(input, getWords())`.
    ///
    /// # Panics
    ///
    /// When the words are not set (Java's `getWords()` is then `null`), with
    /// the `NullPointerException` `StopFilter`'s
    /// `Objects.requireNonNull(stopWords, "stopWords")` throws.
    fn create(&self, input: Box<dyn TokenStream>) -> Result<Box<dyn TokenStream>> {
        let words = self.base.get_words().cloned().unwrap_or_else(|| {
            rucene::store::java_nio_file::throw_unchecked(
                "java.lang.NullPointerException",
                Some("stopWords"),
            )
        });
        let stop_filter = StopFilter::new(input, words)?;
        Ok(Box::new(stop_filter))
    }

    fn as_resource_loader_aware(&mut self) -> Option<&mut dyn ResourceLoaderAware> {
        Some(self)
    }
}
