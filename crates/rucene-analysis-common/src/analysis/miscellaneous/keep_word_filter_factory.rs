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
//! Factory for `KeepWordFilter`, ported from
//! `org.apache.lucene.analysis.miscellaneous.KeepWordFilterFactory`.

use std::collections::HashMap;
use std::sync::Arc;

use rucene::analysis::factories::{
    AbstractAnalysisFactory, AnalysisFactoryClass, SPINameField, TokenFilterFactory,
};
use rucene::analysis::{CharArraySet, TokenStream};
use rucene::error::Result;
use rucene::util::resource_loader::{ResourceLoader, ResourceLoaderAware};

use super::keep_word_filter::new_keep_word_filter;
use crate::analysis::en::abstract_words_file_filter_factory::AbstractWordsFileFilterFactory;

/// Factory for `KeepWordFilter`.
///
/// ```text
/// <fieldType name="text_keepword" class="solr.TextField" positionIncrementGap="100">
///   <analyzer>
///     <tokenizer class="solr.WhitespaceTokenizerFactory"/>
///     <filter class="solr.KeepWordFilterFactory" words="keepwords.txt" ignoreCase="false"/>
///   </analyzer>
/// </fieldType>
/// ```
///
/// Equivalent to `org.apache.lucene.analysis.miscellaneous.KeepWordFilterFactory`
/// (since 3.1), which extends `AbstractWordsFileFilterFactory`: the factory
/// holds the [`AbstractWordsFileFilterFactory`] it extends.
#[derive(Debug)]
pub struct KeepWordFilterFactory {
    base: AbstractWordsFileFilterFactory,
}

impl KeepWordFilterFactory {
    /// SPI name.
    ///
    /// Equivalent to `KeepWordFilterFactory.NAME`.
    pub const NAME: &'static str = "keepWord";

    /// Creates a new KeepWordFilterFactory.
    ///
    /// Equivalent to `KeepWordFilterFactory(Map<String, String>)`, `super(args)`.
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
    /// Equivalent to `KeepWordFilterFactory()`.
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

    /// Equivalent to the protected `createDefaultWords()`: `null`, no
    /// default words.
    fn create_default_words(_base: &AbstractWordsFileFilterFactory) -> Option<CharArraySet> {
        None
    }

    /// The factory's class, as `META-INF/services/org.apache.lucene.analysis.TokenFilterFactory`
    /// names it: `KeepWordFilterFactory.class`, with its `NAME` field and its
    /// `Map<String, String>` constructor.
    pub const fn class() -> AnalysisFactoryClass<dyn TokenFilterFactory> {
        AnalysisFactoryClass::new(
            "org.apache.lucene.analysis.miscellaneous.KeepWordFilterFactory",
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

impl ResourceLoaderAware for KeepWordFilterFactory {
    /// Equivalent to the inherited `AbstractWordsFileFilterFactory.inform`,
    /// over this factory's `createDefaultWords()`.
    fn inform(&mut self, loader: &dyn ResourceLoader) -> Result<()> {
        self.base.inform(loader, Self::create_default_words)
    }
}

impl TokenFilterFactory for KeepWordFilterFactory {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.analysis.miscellaneous.KeepWordFilterFactory"
    }

    /// Equivalent to `KeepWordFilterFactory.create(TokenStream)`: if the set is
    /// null, it means it was empty, and `input` is returned.
    fn create(&self, input: Box<dyn TokenStream>) -> Result<Box<dyn TokenStream>> {
        match self.base.get_words() {
            None => Ok(input),
            Some(words) => Ok(Box::new(new_keep_word_filter(input, words.clone())?)),
        }
    }

    fn as_resource_loader_aware(&mut self) -> Option<&mut dyn ResourceLoaderAware> {
        Some(self)
    }
}
