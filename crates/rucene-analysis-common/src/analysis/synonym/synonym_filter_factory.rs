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

//! Factory for `SynonymFilter`, ported from
//! `org.apache.lucene.analysis.synonym.SynonymFilterFactory`.

use std::collections::HashMap;
use std::sync::Arc;

use rucene::analysis::factories::{
    AbstractAnalysisFactory, AnalysisFactoryClass, SPINameField, TokenFilterFactory,
};
use rucene::analysis::{Analyzer, TokenStream};
use rucene::error::Result;
use rucene::util::resource_loader::{ResourceLoader, ResourceLoaderAware};

use super::synonym_filter::new_synonym_filter;
use super::synonym_graph_filter_factory::SynonymFactoryBase;
use super::synonym_map::SynonymMap;

/// Factory for `SynonymFilter`.
///
/// ```text
/// <fieldType name="text_synonym" class="solr.TextField" positionIncrementGap="100">
///   <analyzer>
///     <tokenizer class="solr.WhitespaceTokenizerFactory"/>
///     <filter class="solr.SynonymFilterFactory" synonyms="synonyms.txt"
///             format="solr" ignoreCase="false" expand="true"
///             tokenizerFactory="solr.WhitespaceTokenizerFactory"
///             [optional tokenizer factory parameters]/>
///   </analyzer>
/// </fieldType>
/// ```
///
/// An optional param name prefix of "tokenizerFactory." may be used for any
/// init params that the SynonymFilterFactory needs to pass to the
/// specified TokenizerFactory. If the TokenizerFactory expects an init
/// parameters with the same name as an init param used by the
/// SynonymFilterFactory, the prefix is mandatory.
///
/// The optional `format` parameter controls how the synonyms will be
/// parsed: It supports the short names of `solr` for `SolrSynonymParser`
/// and `wordnet` for and `WordnetSynonymParser`, or your own
/// `SynonymMap.Parser` class name. The default is `solr`. A custom
/// `SynonymMap.Parser` is expected to have a constructor taking:
///
/// * `boolean dedup` - true if duplicates should be ignored, false otherwise
/// * `boolean expand` - true if conflation groups should be expanded, false
///   if they are one-directional
/// * `Analyzer analyzer` - an analyzer used for each raw synonym
///
/// See `SolrSynonymParser` (the default format).
///
/// Equivalent to `org.apache.lucene.analysis.synonym.SynonymFilterFactory`
/// (since 3.1, deprecated: use `SynonymGraphFilterFactory` instead, but be
/// sure to also use `FlattenGraphFilterFactory` at index time (not at search
/// time) as well), which implements
/// `ResourceLoaderAware`. The classes `tokenizerFactory`, `analyzer` and
/// `format` name are found with the module's class tables
/// ([`tokenizer_factory_classes`](crate::spi::tokenizer_factory_classes),
/// [`analyzer_classes`](crate::spi::analyzer_classes),
/// [`synonym_parser_classes`](crate::spi::synonym_parser_classes)), Rust's
/// stand-in for `ResourceLoader.findClass`.
#[derive(Debug)]
pub struct SynonymFilterFactory {
    inner: SynonymFactoryBase,
}

impl SynonymFilterFactory {
    /// SPI name.
    ///
    /// Equivalent to `SynonymFilterFactory.NAME`.
    pub const NAME: &'static str = "synonym";

    /// Equivalent to `SynonymFilterFactory(Map<String, String>)`.
    ///
    /// # Errors
    ///
    /// Returns what [`AbstractAnalysisFactory::new`] returns,
    /// `IllegalArgumentException` for a missing `synonyms` argument, for an
    /// `analyzer` given with a `tokenizerFactory`, and for a leftover
    /// argument.
    pub fn new(args: &mut HashMap<String, String>) -> Result<Self> {
        Ok(Self {
            inner: SynonymFactoryBase::new(
                args,
                "org.apache.lucene.analysis.synonym.SynonymFilterFactory$1",
            )?,
        })
    }

    /// Default ctor for compatibility with SPI.
    ///
    /// Equivalent to `SynonymFilterFactory()`.
    ///
    /// # Errors
    ///
    /// Always: [`AbstractAnalysisFactory::default_ctor_exception`].
    pub fn new_without_args() -> Result<Self> {
        Err(AbstractAnalysisFactory::default_ctor_exception())
    }

    /// The members this factory inherits from `AbstractAnalysisFactory`.
    pub fn base(&self) -> &AbstractAnalysisFactory {
        self.inner.base()
    }

    /// Load synonyms with the given `SynonymMap.Parser` class.
    ///
    /// Equivalent to the protected `loadSynonyms(ResourceLoader, String,
    /// boolean, Analyzer)`; Java closes each file's reader with
    /// try-with-resources after `parse`, which closed it already.
    ///
    /// # Errors
    ///
    /// What finding the parser class, reading the files and parsing them
    /// raise.
    pub fn load_synonyms(
        &self,
        loader: &dyn ResourceLoader,
        cname: &str,
        dedup: bool,
        analyzer: Arc<dyn Analyzer>,
    ) -> Result<SynonymMap> {
        self.inner.load_synonyms(loader, cname, dedup, analyzer)
    }

    /// The factory's class, as `META-INF/services/org.apache.lucene.analysis.TokenFilterFactory`
    /// names it: `SynonymFilterFactory.class`, with its `NAME` field and its
    /// `Map<String, String>` constructor.
    pub const fn class() -> AnalysisFactoryClass<dyn TokenFilterFactory> {
        AnalysisFactoryClass::new(
            "org.apache.lucene.analysis.synonym.SynonymFilterFactory",
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

impl ResourceLoaderAware for SynonymFilterFactory {
    /// Equivalent to `SynonymFilterFactory.inform(ResourceLoader)`: the
    /// `java.text.ParseException` of the parser becomes `IOException("Error
    /// parsing synonyms file:", e)`.
    fn inform(&mut self, loader: &dyn ResourceLoader) -> Result<()> {
        self.inner.inform(loader)
    }
}

impl TokenFilterFactory for SynonymFilterFactory {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.analysis.synonym.SynonymFilterFactory"
    }

    /// Equivalent to `SynonymFilterFactory.create(TokenStream)`: if the
    /// fst is null, it means there's actually no synonyms... just return the
    /// original stream as there is nothing to do here.
    ///
    /// # Panics
    ///
    /// With the `NullPointerException` Java throws before `inform`.
    fn create(&self, input: Box<dyn TokenStream>) -> Result<Box<dyn TokenStream>> {
        let Some(map) = self.inner.map() else {
            rucene::store::java_nio_file::throw_unchecked("java.lang.NullPointerException", None)
        };
        if map.fst.is_none() {
            Ok(input)
        } else {
            Ok(Box::new(new_synonym_filter(
                input,
                Arc::clone(map),
                self.inner.ignore_case(),
            )?))
        }
    }

    fn as_resource_loader_aware(&mut self) -> Option<&mut dyn ResourceLoaderAware> {
        Some(self)
    }
}
