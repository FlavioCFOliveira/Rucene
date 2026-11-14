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

//! Abstract parent class for analysis factories that accept a stopwords file
//! as input, ported from
//! `org.apache.lucene.analysis.en.AbstractWordsFileFilterFactory`.

use std::collections::HashMap;

use rucene::analysis::factories::{java_equals_ignore_case, AbstractAnalysisFactory};
use rucene::analysis::CharArraySet;
use rucene::error::{LuceneError, Result};
use rucene::util::resource_loader::ResourceLoader;

use crate::spi::check_unknown_parameters;

/// Abstract parent class for analysis factories that accept a stopwords file
/// as input.
///
/// Concrete implementations can leverage the following input attributes. All
/// attributes are optional:
///
/// * `ignoreCase` defaults to `false`
/// * `words` should be the name of a stopwords file to parse, if not
///   specified the factory will use the value provided by the concrete
///   factory's `createDefaultWords()`.
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
/// Equivalent to `org.apache.lucene.analysis.en.AbstractWordsFileFilterFactory`,
/// which extends `TokenFilterFactory` and implements `ResourceLoaderAware`:
/// the concrete factory holds this state, implements those two traits, and
/// supplies the abstract `createDefaultWords()` to
/// [`inform`](Self::inform), its only caller.
#[derive(Debug)]
pub struct AbstractWordsFileFilterFactory {
    base: AbstractAnalysisFactory,
    words: Option<CharArraySet>,
    word_files: Option<String>,
    format: Option<String>,
    ignore_case: bool,
}

impl AbstractWordsFileFilterFactory {
    /// Equivalent to `AbstractWordsFileFilterFactory.FORMAT_WORDSET`.
    pub const FORMAT_WORDSET: &'static str = "wordset";

    /// Equivalent to `AbstractWordsFileFilterFactory.FORMAT_SNOWBALL`.
    pub const FORMAT_SNOWBALL: &'static str = "snowball";

    /// Default ctor for compatibility with SPI.
    ///
    /// Equivalent to the protected `AbstractWordsFileFilterFactory()`.
    ///
    /// # Errors
    ///
    /// Always: [`AbstractAnalysisFactory::default_ctor_exception`].
    pub fn new_without_args() -> Result<Self> {
        Err(AbstractAnalysisFactory::default_ctor_exception())
    }

    /// Initialize this factory via a set of key-value pairs.
    ///
    /// Equivalent to `AbstractWordsFileFilterFactory(Map<String, String>)`.
    ///
    /// # Errors
    ///
    /// Returns what [`AbstractAnalysisFactory::new`] returns, and
    /// `IllegalArgumentException` for a leftover argument.
    pub fn new(args: &mut HashMap<String, String>) -> Result<Self> {
        let base = AbstractAnalysisFactory::new(args)?;
        let word_files = AbstractAnalysisFactory::get(args, "words");
        let format = AbstractAnalysisFactory::get(args, "format").or_else(|| {
            if word_files.is_none() {
                None
            } else {
                Some(Self::FORMAT_WORDSET.to_string())
            }
        });
        let ignore_case = AbstractAnalysisFactory::get_boolean(args, "ignoreCase", false);
        check_unknown_parameters(args)?;
        Ok(Self {
            base,
            words: None,
            word_files,
            format,
            ignore_case,
        })
    }

    /// The members this factory inherits from `AbstractAnalysisFactory`.
    pub fn base(&self) -> &AbstractAnalysisFactory {
        &self.base
    }

    /// Initialize the set of stopwords provided via ResourceLoader, or using
    /// defaults.
    ///
    /// Equivalent to `AbstractWordsFileFilterFactory.inform(ResourceLoader)`;
    /// `create_default_words` is the concrete factory's
    /// `createDefaultWords()`, the default word set (`None` for Java's
    /// `null`), called when no `words` file was named.
    ///
    /// # Errors
    ///
    /// Returns what reading the word files returns, and
    /// `IllegalArgumentException` with Java's messages for an unknown
    /// `format` or a `format` given without `words`.
    pub fn inform(
        &mut self,
        loader: &dyn ResourceLoader,
        create_default_words: impl FnOnce(&Self) -> Option<CharArraySet>,
    ) -> Result<()> {
        if let Some(word_files) = &self.word_files {
            let format = self.format.as_deref();
            if format.is_some_and(|f| java_equals_ignore_case(Self::FORMAT_WORDSET, f)) {
                self.words = AbstractAnalysisFactory::get_word_set(
                    loader,
                    Some(word_files.as_str()),
                    self.ignore_case,
                )?;
            } else if format.is_some_and(|f| java_equals_ignore_case(Self::FORMAT_SNOWBALL, f)) {
                self.words = AbstractAnalysisFactory::get_snowball_word_set(
                    loader,
                    Some(word_files.as_str()),
                    self.ignore_case,
                )?;
            } else {
                return Err(LuceneError::illegal_argument(format!(
                    "Unknown 'format' specified for 'words' file: {}",
                    format.unwrap_or("null")
                )));
            }
        } else {
            if let Some(format) = &self.format {
                return Err(LuceneError::illegal_argument(format!(
                    "'format' can not be specified w/o an explicit 'words' file: {format}"
                )));
            }
            self.words = create_default_words(self);
        }
        Ok(())
    }

    /// Equivalent to `getWords()`: `None` until [`inform`](Self::inform)
    /// sets them (Java's `null`), or when the named files name no file.
    pub fn get_words(&self) -> Option<&CharArraySet> {
        self.words.as_ref()
    }

    /// Equivalent to `getWordFiles()`.
    pub fn get_word_files(&self) -> Option<&str> {
        self.word_files.as_deref()
    }

    /// Equivalent to `getFormat()`.
    pub fn get_format(&self) -> Option<&str> {
        self.format.as_deref()
    }

    /// Equivalent to `isIgnoreCase()`.
    pub fn is_ignore_case(&self) -> bool {
        self.ignore_case
    }
}
