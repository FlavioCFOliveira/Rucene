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
//! Factory for `KeywordMarkerFilter`, ported from
//! `org.apache.lucene.analysis.miscellaneous.KeywordMarkerFilterFactory`.

use std::collections::HashMap;
use std::sync::Arc;

use rucene::analysis::factories::{
    AbstractAnalysisFactory, AnalysisFactoryClass, SPINameField, TokenFilterFactory,
};
use rucene::analysis::java_regex::{JavaPattern, CASE_INSENSITIVE, UNICODE_CASE};
use rucene::analysis::{CharArraySet, TokenStream};
use rucene::error::{LuceneError, Result};
use rucene::util::resource_loader::{ResourceLoader, ResourceLoaderAware};

use super::pattern_keyword_marker_filter::new_pattern_keyword_marker_filter;
use super::set_keyword_marker_filter::new_set_keyword_marker_filter;
use crate::spi::check_unknown_parameters;

/// Factory for `KeywordMarkerFilter`.
///
/// ```text
/// <fieldType name="text_keyword" class="solr.TextField" positionIncrementGap="100">
///   <analyzer>
///     <tokenizer class="solr.WhitespaceTokenizerFactory"/>
///     <filter class="solr.KeywordMarkerFilterFactory" protected="protectedkeyword.txt" pattern="^.+er$" ignoreCase="false"/>
///   </analyzer>
/// </fieldType>
/// ```
///
/// Equivalent to `org.apache.lucene.analysis.miscellaneous.KeywordMarkerFilterFactory`
/// (since 3.1), which implements `ResourceLoaderAware`.
#[derive(Debug)]
pub struct KeywordMarkerFilterFactory {
    base: AbstractAnalysisFactory,
    word_files: Option<String>,
    string_pattern: Option<String>,
    ignore_case: bool,
    pattern: Option<JavaPattern>,
    protected_words: Option<CharArraySet>,
}

impl KeywordMarkerFilterFactory {
    /// SPI name.
    ///
    /// Equivalent to `KeywordMarkerFilterFactory.NAME`.
    pub const NAME: &'static str = "keywordMarker";

    /// Equivalent to `KeywordMarkerFilterFactory.PROTECTED_TOKENS`.
    pub const PROTECTED_TOKENS: &'static str = "protected";

    /// Equivalent to `KeywordMarkerFilterFactory.PATTERN`.
    pub const PATTERN: &'static str = "pattern";

    /// Creates a new KeywordMarkerFilterFactory.
    ///
    /// Equivalent to `KeywordMarkerFilterFactory(Map<String, String>)`.
    ///
    /// # Errors
    ///
    /// Returns what [`AbstractAnalysisFactory::new`] returns, and
    /// `IllegalArgumentException` for a leftover argument.
    pub fn new(args: &mut HashMap<String, String>) -> Result<Self> {
        let base = AbstractAnalysisFactory::new(args)?;
        let word_files = AbstractAnalysisFactory::get(args, Self::PROTECTED_TOKENS);
        let string_pattern = AbstractAnalysisFactory::get(args, Self::PATTERN);
        let ignore_case = AbstractAnalysisFactory::get_boolean(args, "ignoreCase", false);
        check_unknown_parameters(args)?;
        Ok(Self {
            base,
            word_files,
            string_pattern,
            ignore_case,
            pattern: None,
            protected_words: None,
        })
    }

    /// Default ctor for compatibility with SPI.
    ///
    /// Equivalent to `KeywordMarkerFilterFactory()`.
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

    /// Equivalent to `isIgnoreCase()`.
    pub fn is_ignore_case(&self) -> bool {
        self.ignore_case
    }

    /// The factory's class, as `META-INF/services/org.apache.lucene.analysis.TokenFilterFactory`
    /// names it: `KeywordMarkerFilterFactory.class`, with its `NAME` field
    /// and its `Map<String, String>` constructor.
    pub const fn class() -> AnalysisFactoryClass<dyn TokenFilterFactory> {
        AnalysisFactoryClass::new(
            "org.apache.lucene.analysis.miscellaneous.KeywordMarkerFilterFactory",
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

impl ResourceLoaderAware for KeywordMarkerFilterFactory {
    /// Equivalent to `KeywordMarkerFilterFactory.inform(ResourceLoader)`: the
    /// protected words, and the pattern compiled with `Pattern.compile`,
    /// with `CASE_INSENSITIVE | UNICODE_CASE` when ignoring case.
    ///
    /// The `PatternSyntaxException` Java throws (an
    /// `IllegalArgumentException`) is returned as one, with its message.
    fn inform(&mut self, loader: &dyn ResourceLoader) -> Result<()> {
        if let Some(word_files) = &self.word_files {
            self.protected_words = AbstractAnalysisFactory::get_word_set(
                loader,
                Some(word_files.as_str()),
                self.ignore_case,
            )?;
        }
        if let Some(string_pattern) = &self.string_pattern {
            let flags = if self.ignore_case {
                CASE_INSENSITIVE | UNICODE_CASE
            } else {
                0
            };
            self.pattern = Some(
                JavaPattern::compile(string_pattern, flags)
                    .map_err(|syntax| LuceneError::illegal_argument(syntax.to_string()))?,
            );
        }
        Ok(())
    }
}

impl TokenFilterFactory for KeywordMarkerFilterFactory {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.analysis.miscellaneous.KeywordMarkerFilterFactory"
    }

    /// Equivalent to `KeywordMarkerFilterFactory.create(TokenStream)`: a
    /// `PatternKeywordMarkerFilter` when a pattern is set, then a
    /// `SetKeywordMarkerFilter` when protected words are set.
    fn create(&self, input: Box<dyn TokenStream>) -> Result<Box<dyn TokenStream>> {
        let mut input = input;
        if let Some(pattern) = &self.pattern {
            input = Box::new(new_pattern_keyword_marker_filter(input, pattern.clone())?);
        }
        if let Some(protected_words) = &self.protected_words {
            input = Box::new(new_set_keyword_marker_filter(
                input,
                protected_words.clone(),
            )?);
        }
        Ok(input)
    }

    fn as_resource_loader_aware(&mut self) -> Option<&mut dyn ResourceLoaderAware> {
        Some(self)
    }
}
