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

//! Factory for `CommonGramsFilter`, ported from
//! `org.apache.lucene.analysis.commongrams.CommonGramsFilterFactory`.

use std::collections::HashMap;
use std::sync::Arc;

use rucene::analysis::factories::{
    AbstractAnalysisFactory, AnalysisFactoryClass, SPINameField, TokenFilterFactory,
};
use rucene::analysis::{CharArraySet, TokenStream};
use rucene::error::Result;
use rucene::util::resource_loader::{ResourceLoader, ResourceLoaderAware};

use super::common_grams_filter::{new_common_grams_filter, CommonGramsFilter};
use crate::analysis::en::abstract_words_file_filter_factory::AbstractWordsFileFilterFactory;
use crate::analysis::en::english_analyzer::ENGLISH_STOP_WORDS_SET;

/// Constructs a `CommonGramsFilter`.
///
/// ```text
/// <fieldType name="text_cmmngrms" class="solr.TextField" positionIncrementGap="100">
///   <analyzer>
///     <tokenizer class="solr.WhitespaceTokenizerFactory"/>
///     <filter class="solr.CommonGramsFilterFactory" words="commongramsstopwords.txt" ignoreCase="false"/>
///   </analyzer>
/// </fieldType>
/// ```
///
/// Equivalent to `org.apache.lucene.analysis.commongrams.CommonGramsFilterFactory`
/// (since 3.1), which extends `AbstractWordsFileFilterFactory`: the factory
/// holds the [`AbstractWordsFileFilterFactory`] it extends.
#[derive(Debug)]
pub struct CommonGramsFilterFactory {
    base: AbstractWordsFileFilterFactory,
}

impl CommonGramsFilterFactory {
    /// SPI name.
    ///
    /// Equivalent to `CommonGramsFilterFactory.NAME`.
    pub const NAME: &'static str = "commonGrams";

    /// Creates a new CommonGramsFilterFactory.
    ///
    /// Equivalent to `CommonGramsFilterFactory(Map<String, String>)`,
    /// `super(args)`.
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
    /// Equivalent to `CommonGramsFilterFactory()`.
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

    /// Equivalent to `CommonGramsFilterFactory.getCommonWords()`,
    /// `getWords()`.
    pub fn get_common_words(&self) -> Option<&CharArraySet> {
        self.base.get_words()
    }

    /// Equivalent to the protected `createDefaultWords()`: `new
    /// CharArraySet(EnglishAnalyzer.ENGLISH_STOP_WORDS_SET, isIgnoreCase())`.
    pub(crate) fn create_default_words(base: &AbstractWordsFileFilterFactory) -> CharArraySet {
        let mut words = CharArraySet::new(ENGLISH_STOP_WORDS_SET.size(), base.is_ignore_case());
        for word in ENGLISH_STOP_WORDS_SET.iter() {
            let mut word = word.to_vec();
            let len = word.len();
            words.add_chars(&mut word, 0, len);
        }
        words
    }

    /// Equivalent to `CommonGramsFilterFactory.create(TokenStream)`, `new
    /// CommonGramsFilter(input, getWords())`, as its concrete type, which
    /// `CommonGramsQueryFilterFactory.create` casts it to.
    ///
    /// # Errors
    ///
    /// What the filter's construction returns.
    pub fn create_common_grams_filter(
        &self,
        input: Box<dyn TokenStream>,
    ) -> Result<CommonGramsFilter> {
        new_common_grams_filter(input, self.base.get_words().cloned())
    }

    /// The factory's class, as `META-INF/services/org.apache.lucene.analysis.TokenFilterFactory`
    /// names it: `CommonGramsFilterFactory.class`, with its `NAME` field and
    /// its `Map<String, String>` constructor.
    pub const fn class() -> AnalysisFactoryClass<dyn TokenFilterFactory> {
        AnalysisFactoryClass::new(
            "org.apache.lucene.analysis.commongrams.CommonGramsFilterFactory",
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

impl ResourceLoaderAware for CommonGramsFilterFactory {
    /// Equivalent to the inherited `AbstractWordsFileFilterFactory.inform`,
    /// over this factory's `createDefaultWords()`.
    fn inform(&mut self, loader: &dyn ResourceLoader) -> Result<()> {
        self.base
            .inform(loader, |base| Some(Self::create_default_words(base)))
    }
}

impl TokenFilterFactory for CommonGramsFilterFactory {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.analysis.commongrams.CommonGramsFilterFactory"
    }

    /// Equivalent to `CommonGramsFilterFactory.create(TokenStream)`.
    fn create(&self, input: Box<dyn TokenStream>) -> Result<Box<dyn TokenStream>> {
        let common_grams = self.create_common_grams_filter(input)?;
        Ok(Box::new(common_grams))
    }

    fn as_resource_loader_aware(&mut self) -> Option<&mut dyn ResourceLoaderAware> {
        Some(self)
    }
}
