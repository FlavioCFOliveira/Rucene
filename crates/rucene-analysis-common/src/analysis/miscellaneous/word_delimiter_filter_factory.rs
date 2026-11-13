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
//! Factory for `WordDelimiterFilter`, ported from
//! `org.apache.lucene.analysis.miscellaneous.WordDelimiterFilterFactory`.

use std::collections::HashMap;
use std::sync::Arc;

use rucene::analysis::factories::{
    AbstractAnalysisFactory, AnalysisFactoryClass, SPINameField, TokenFilterFactory,
};
use rucene::analysis::{CharArraySet, TokenStream};
use rucene::error::Result;
use rucene::util::resource_loader::{ResourceLoader, ResourceLoaderAware};

#[allow(deprecated)]
use super::word_delimiter_filter::{
    new_word_delimiter_filter_with_table, CATENATE_ALL, CATENATE_NUMBERS, CATENATE_WORDS,
    GENERATE_NUMBER_PARTS, GENERATE_WORD_PARTS, PRESERVE_ORIGINAL, SPLIT_ON_CASE_CHANGE,
    SPLIT_ON_NUMERICS, STEM_ENGLISH_POSSESSIVE,
};
use super::word_delimiter_graph_filter_factory::parse_types;
use super::word_delimiter_iterator::DEFAULT_WORD_DELIM_TABLE;
use crate::spi::check_unknown_parameters;

/// Factory for `WordDelimiterFilter`.
///
/// ```text
/// <fieldType name="text_wd" class="solr.TextField" positionIncrementGap="100">
///   <analyzer>
///     <tokenizer class="solr.WhitespaceTokenizerFactory"/>
///     <filter class="solr.WordDelimiterFilterFactory" protected="protectedword.txt"
///             preserveOriginal="0" splitOnNumerics="1" splitOnCaseChange="1"
///             catenateWords="0" catenateNumbers="0" catenateAll="0"
///             generateWordParts="1" generateNumberParts="1" stemEnglishPossessive="1"
///             types="wdfftypes.txt" />
///   </analyzer>
/// </fieldType>
/// ```
///
/// Equivalent to `org.apache.lucene.analysis.miscellaneous.WordDelimiterFilterFactory`
/// (since 3.1), which implements `ResourceLoaderAware`.
///
/// **Deprecated** in Lucene: use `WordDelimiterGraphFilterFactory` instead:
/// it produces a correct token graph so that e.g. `PhraseQuery` works
/// correctly when it's used in the search time analyzer.
#[derive(Debug)]
pub struct WordDelimiterFilterFactory {
    base: AbstractAnalysisFactory,
    word_files: Option<String>,
    types: Option<String>,
    flags: i32,
    type_table: Option<Arc<[u8]>>,
    protected_words: Option<CharArraySet>,
}

impl WordDelimiterFilterFactory {
    /// SPI name.
    ///
    /// Equivalent to `WordDelimiterFilterFactory.NAME`.
    pub const NAME: &'static str = "wordDelimiter";

    /// Equivalent to `WordDelimiterFilterFactory.PROTECTED_TOKENS`.
    pub const PROTECTED_TOKENS: &'static str = "protected";

    /// Equivalent to `WordDelimiterFilterFactory.TYPES`.
    pub const TYPES: &'static str = "types";

    /// Creates a new WordDelimiterFilterFactory.
    ///
    /// Equivalent to `WordDelimiterFilterFactory(Map<String, String>)`.
    ///
    /// # Errors
    ///
    /// Returns what [`AbstractAnalysisFactory::new`] returns, what an
    /// unparsable flag raises, and `IllegalArgumentException` for a leftover
    /// argument.
    pub fn new(args: &mut HashMap<String, String>) -> Result<Self> {
        let base = AbstractAnalysisFactory::new(args)?;
        let mut flags = 0;
        let mut flag = |name: &str, default_val: i32, bit: i32| -> Result<()> {
            if AbstractAnalysisFactory::get_int(args, name, default_val)? != 0 {
                flags |= bit;
            }
            Ok(())
        };
        flag("generateWordParts", 1, GENERATE_WORD_PARTS)?;
        flag("generateNumberParts", 1, GENERATE_NUMBER_PARTS)?;
        flag("catenateWords", 0, CATENATE_WORDS)?;
        flag("catenateNumbers", 0, CATENATE_NUMBERS)?;
        flag("catenateAll", 0, CATENATE_ALL)?;
        flag("splitOnCaseChange", 1, SPLIT_ON_CASE_CHANGE)?;
        flag("splitOnNumerics", 1, SPLIT_ON_NUMERICS)?;
        flag("preserveOriginal", 0, PRESERVE_ORIGINAL)?;
        flag("stemEnglishPossessive", 1, STEM_ENGLISH_POSSESSIVE)?;
        let word_files = AbstractAnalysisFactory::get(args, Self::PROTECTED_TOKENS);
        let types = AbstractAnalysisFactory::get(args, Self::TYPES);
        check_unknown_parameters(args)?;
        Ok(Self {
            base,
            word_files,
            types,
            flags,
            type_table: None,
            protected_words: None,
        })
    }

    /// Default ctor for compatibility with SPI.
    ///
    /// Equivalent to `WordDelimiterFilterFactory()`.
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
    /// names it: `WordDelimiterFilterFactory.class`, with its `NAME`
    /// field and its `Map<String, String>` constructor.
    pub const fn class() -> AnalysisFactoryClass<dyn TokenFilterFactory> {
        AnalysisFactoryClass::new(
            "org.apache.lucene.analysis.miscellaneous.WordDelimiterFilterFactory",
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

impl ResourceLoaderAware for WordDelimiterFilterFactory {
    /// Equivalent to `WordDelimiterFilterFactory.inform(ResourceLoader)`.
    fn inform(&mut self, loader: &dyn ResourceLoader) -> Result<()> {
        if let Some(word_files) = &self.word_files {
            self.protected_words =
                AbstractAnalysisFactory::get_word_set(loader, Some(word_files), false)?;
        }
        if let Some(types) = &self.types {
            let files = AbstractAnalysisFactory::split_file_names(Some(types));
            let mut wlist = Vec::new();
            for file in &files {
                let lines = AbstractAnalysisFactory::get_lines(loader, file.trim())?;
                wlist.extend(lines);
            }
            self.type_table = Some(parse_types(&wlist)?);
        }
        Ok(())
    }
}

impl TokenFilterFactory for WordDelimiterFilterFactory {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.analysis.miscellaneous.WordDelimiterFilterFactory"
    }

    /// Equivalent to `WordDelimiterFilterFactory.create(TokenStream)`.
    #[allow(deprecated)]
    fn create(&self, input: Box<dyn TokenStream>) -> Result<Box<dyn TokenStream>> {
        let type_table = self
            .type_table
            .clone()
            .unwrap_or_else(|| Arc::clone(&DEFAULT_WORD_DELIM_TABLE));
        Ok(Box::new(new_word_delimiter_filter_with_table(
            input,
            type_table,
            self.flags,
            self.protected_words.clone(),
        )?))
    }

    fn as_resource_loader_aware(&mut self) -> Option<&mut dyn ResourceLoaderAware> {
        Some(self)
    }
}
