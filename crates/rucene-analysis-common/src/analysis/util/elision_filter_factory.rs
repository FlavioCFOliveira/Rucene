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

//! Factory for `ElisionFilter`, ported from
//! `org.apache.lucene.analysis.util.ElisionFilterFactory`.

use std::collections::HashMap;
use std::sync::Arc;

use rucene::analysis::factories::{
    AbstractAnalysisFactory, AnalysisFactoryClass, SPINameField, TokenFilterFactory,
};
use rucene::analysis::{CharArraySet, TokenStream};
use rucene::error::Result;
use rucene::util::resource_loader::{ResourceLoader, ResourceLoaderAware};

use super::elision_filter::new_elision_filter;
use crate::analysis::fr::french_analyzer::DEFAULT_ARTICLES;
use crate::spi::check_unknown_parameters;

/// Factory for `ElisionFilter`.
///
/// ```text
/// <fieldType name="text_elsn" class="solr.TextField" positionIncrementGap="100">
///   <analyzer>
///     <tokenizer class="solr.StandardTokenizerFactory"/>
///     <filter class="solr.LowerCaseFilterFactory"/>
///     <filter class="solr.ElisionFilterFactory"
///       articles="stopwordarticles.txt" ignoreCase="true"/>
///   </analyzer>
/// </fieldType>
/// ```
///
/// Equivalent to `org.apache.lucene.analysis.util.ElisionFilterFactory`
/// (since 3.1), which extends `TokenFilterFactory` and implements
/// `ResourceLoaderAware`.
#[derive(Debug)]
pub struct ElisionFilterFactory {
    base: AbstractAnalysisFactory,
    articles_file: Option<String>,
    ignore_case: bool,
    articles: Option<CharArraySet>,
}

impl ElisionFilterFactory {
    /// SPI name.
    ///
    /// Equivalent to `ElisionFilterFactory.NAME`.
    pub const NAME: &'static str = "elision";

    /// Creates a new ElisionFilterFactory.
    ///
    /// Equivalent to `ElisionFilterFactory(Map<String, String>)`.
    ///
    /// # Errors
    ///
    /// Returns what [`AbstractAnalysisFactory::new`] returns, and
    /// `IllegalArgumentException` for a leftover argument.
    pub fn new(args: &mut HashMap<String, String>) -> Result<Self> {
        let base = AbstractAnalysisFactory::new(args)?;
        let articles_file = AbstractAnalysisFactory::get(args, "articles");
        let ignore_case = AbstractAnalysisFactory::get_boolean(args, "ignoreCase", false);
        check_unknown_parameters(args)?;
        Ok(Self {
            base,
            articles_file,
            ignore_case,
            articles: None,
        })
    }

    /// Default ctor for compatibility with SPI.
    ///
    /// Equivalent to `ElisionFilterFactory()`.
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
    /// names it: `ElisionFilterFactory.class`, with its `NAME` field and its
    /// `Map<String, String>` constructor.
    pub const fn class() -> AnalysisFactoryClass<dyn TokenFilterFactory> {
        AnalysisFactoryClass::new(
            "org.apache.lucene.analysis.util.ElisionFilterFactory",
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

impl ResourceLoaderAware for ElisionFilterFactory {
    /// Equivalent to `ElisionFilterFactory.inform(ResourceLoader)`: the
    /// articles are `FrenchAnalyzer.DEFAULT_ARTICLES` when no `articles`
    /// file was named, and the word set of the named files otherwise.
    fn inform(&mut self, loader: &dyn ResourceLoader) -> Result<()> {
        self.articles = match &self.articles_file {
            None => Some(DEFAULT_ARTICLES.clone()),
            Some(articles_file) => AbstractAnalysisFactory::get_word_set(
                loader,
                Some(articles_file.as_str()),
                self.ignore_case,
            )?,
        };
        Ok(())
    }
}

impl TokenFilterFactory for ElisionFilterFactory {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.analysis.util.ElisionFilterFactory"
    }

    /// Equivalent to `ElisionFilterFactory.create(TokenStream)`, `new
    /// ElisionFilter(input, articles)`.
    ///
    /// # Panics
    ///
    /// When [`inform`](ResourceLoaderAware::inform) has not set the articles
    /// (Java's `articles` is then `null`), with the `NullPointerException`
    /// `ElisionFilter`'s `Objects.requireNonNull(articles, "articles")`
    /// throws.
    fn create(&self, input: Box<dyn TokenStream>) -> Result<Box<dyn TokenStream>> {
        let articles = self.articles.clone().unwrap_or_else(|| {
            rucene::store::java_nio_file::throw_unchecked(
                "java.lang.NullPointerException",
                Some("articles"),
            )
        });
        Ok(Box::new(new_elision_filter(input, articles)?))
    }

    /// Equivalent to `ElisionFilterFactory.normalize(TokenStream)`,
    /// `create(input)`.
    fn normalize(&self, input: Box<dyn TokenStream>) -> Result<Box<dyn TokenStream>> {
        self.create(input)
    }

    fn as_resource_loader_aware(&mut self) -> Option<&mut dyn ResourceLoaderAware> {
        Some(self)
    }
}
