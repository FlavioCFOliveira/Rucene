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

//! The type declared in `HyphenationCompoundWordTokenFilterFactory.java`, of
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
use super::hyphenation::hyphenation_tree::HyphenationTree;
use super::hyphenation_compound_word_token_filter::{get_hyphenation_tree_from_source, new_filter};
use crate::jdk::sax::InputSource;
use crate::spi::check_unknown_parameters;

/// Factory for `HyphenationCompoundWordTokenFilter`.
///
/// This factory accepts the following parameters:
///
/// * `hyphenator` (mandatory): path to the FOP xml hyphenation pattern. See
///   <http://offo.sourceforge.net/hyphenation/>.
/// * `encoding` (optional): encoding of the xml hyphenation file. defaults to
///   UTF-8.
/// * `dictionary` (optional): dictionary of words. defaults to no dictionary.
/// * `minWordSize` (optional): minimal word length that gets decomposed.
///   defaults to 5.
/// * `minSubwordSize` (optional): minimum length of subwords. defaults to 2.
/// * `maxSubwordSize` (optional): maximum length of subwords. defaults to 15.
/// * `onlyLongestMatch` (optional): if true, adds only the longest matching
///   subword to the stream. defaults to false.
///
/// ```text
/// <fieldType name="text_hyphncomp" class="solr.TextField" positionIncrementGap="100">
///   <analyzer>
///     <tokenizer class="solr.WhitespaceTokenizerFactory"/>
///     <filter class="solr.HyphenationCompoundWordTokenFilterFactory" hyphenator="hyphenator.xml" encoding="UTF-8"
///         dictionary="dictionary.txt" minWordSize="5" minSubwordSize="2" maxSubwordSize="15" onlyLongestMatch="false"/>
///   </analyzer>
/// </fieldType>
/// ```
///
/// Equivalent to `org.apache.lucene.analysis.compound.HyphenationCompoundWordTokenFilterFactory`
/// (since 3.1.0); the hyphenation tree it loads is shared with the filters
/// it creates through an [`Arc`].
#[derive(Debug)]
pub struct HyphenationCompoundWordTokenFilterFactory {
    base: AbstractAnalysisFactory,
    dictionary: Option<CharArraySet>,
    hyphenator: Option<Arc<HyphenationTree>>,
    dict_file: Option<String>,
    hyp_file: String,
    encoding: Option<String>,
    min_word_size: i32,
    min_subword_size: i32,
    max_subword_size: i32,
    only_longest_match: bool,
    no_sub_matches: bool,
    no_overlapping_matches: bool,
}

impl HyphenationCompoundWordTokenFilterFactory {
    /// SPI name
    ///
    /// Equivalent to `HyphenationCompoundWordTokenFilterFactory.NAME`.
    pub const NAME: &'static str = "hyphenationCompoundWord";

    /// Creates a new HyphenationCompoundWordTokenFilterFactory
    ///
    /// Equivalent to `HyphenationCompoundWordTokenFilterFactory(Map<String, String>)`.
    ///
    /// # Errors
    ///
    /// What [`AbstractAnalysisFactory::new`] returns, `IllegalArgumentException`
    /// for a missing `hyphenator`, a size that is not an `int`, or a leftover
    /// argument (`Unknown parameters: …`).
    pub fn new(args: &mut HashMap<String, String>) -> Result<Self> {
        let base = AbstractAnalysisFactory::new(args)?;
        let dict_file = AbstractAnalysisFactory::get(args, "dictionary");
        let encoding = AbstractAnalysisFactory::get(args, "encoding");
        let hyp_file = AbstractAnalysisFactory::require(args, "hyphenator")?;
        let min_word_size =
            AbstractAnalysisFactory::get_int(args, "minWordSize", DEFAULT_MIN_WORD_SIZE)?;
        let min_subword_size =
            AbstractAnalysisFactory::get_int(args, "minSubwordSize", DEFAULT_MIN_SUBWORD_SIZE)?;
        let max_subword_size =
            AbstractAnalysisFactory::get_int(args, "maxSubwordSize", DEFAULT_MAX_SUBWORD_SIZE)?;
        let only_longest_match =
            AbstractAnalysisFactory::get_boolean(args, "onlyLongestMatch", false);
        let no_sub_matches = AbstractAnalysisFactory::get_boolean(args, "noSubMatches", false);
        let no_overlapping_matches =
            AbstractAnalysisFactory::get_boolean(args, "noOverlappingMatches", false);
        check_unknown_parameters(args)?;
        Ok(Self {
            base,
            dictionary: None,
            hyphenator: None,
            dict_file,
            hyp_file,
            encoding,
            min_word_size,
            min_subword_size,
            max_subword_size,
            only_longest_match,
            no_sub_matches,
            no_overlapping_matches,
        })
    }

    /// Default ctor for compatibility with SPI
    ///
    /// Equivalent to `HyphenationCompoundWordTokenFilterFactory()`.
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
            "org.apache.lucene.analysis.compound.HyphenationCompoundWordTokenFilterFactory",
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

impl ResourceLoaderAware for HyphenationCompoundWordTokenFilterFactory {
    /// Equivalent to `HyphenationCompoundWordTokenFilterFactory.inform(ResourceLoader)`:
    /// the optional dictionary, then the hyphenation tree read from
    /// `loader.openResource(hypFile)` with the configured encoding (the XML
    /// parser decides when there is none) and `hypFile` as system id. The
    /// stream is closed in any case, as `IOUtils.closeWhileHandlingException`
    /// closes it.
    fn inform(&mut self, loader: &dyn ResourceLoader) -> Result<()> {
        if let Some(dict_file) = &self.dict_file {
            // the dictionary can be empty.
            self.dictionary =
                AbstractAnalysisFactory::get_word_set(loader, Some(dict_file.as_str()), false)?;
        }
        // TODO: Broken, because we cannot resolve real system id
        // ResourceLoader should also supply method like ClassLoader to get resource URL
        let stream = loader.open_resource(&self.hyp_file)?;
        let mut is = InputSource::from_byte_stream(stream);
        is.set_encoding(self.encoding.clone()); // if it's null let xml parser decide
        is.set_system_id(Some(self.hyp_file.clone()));
        self.hyphenator = Some(Arc::new(get_hyphenation_tree_from_source(is)?));
        Ok(())
    }
}

impl TokenFilterFactory for HyphenationCompoundWordTokenFilterFactory {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.analysis.compound.HyphenationCompoundWordTokenFilterFactory"
    }

    /// Equivalent to `HyphenationCompoundWordTokenFilterFactory.create(TokenStream)`;
    /// before `inform`, the filter's `requireNonNull(hyphenator,
    /// "hyphenator")` throws `NullPointerException`.
    fn create(&self, input: Box<dyn TokenStream>) -> Result<Box<dyn TokenStream>> {
        Ok(Box::new(new_filter(
            input,
            self.hyphenator.clone(),
            self.dictionary.clone(),
            self.min_word_size,
            self.min_subword_size,
            self.max_subword_size,
            self.only_longest_match,
            self.no_sub_matches,
            self.no_overlapping_matches,
        )?))
    }

    fn as_resource_loader_aware(&mut self) -> Option<&mut dyn ResourceLoaderAware> {
        Some(self)
    }
}
