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

//! Factory for `HTMLStripCharFilter`, ported from
//! `org.apache.lucene.analysis.charfilter.HTMLStripCharFilterFactory`.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, LazyLock};

use rucene::analysis::factories::{
    AbstractAnalysisFactory, AnalysisFactoryClass, CharFilterFactory, SPINameField,
};
use rucene::analysis::java_regex::JavaPattern;
use rucene::analysis::CharReader;
use rucene::error::Result;

use super::html_strip_char_filter::HTMLStripCharFilter;
use crate::spi::check_unknown_parameters;

/// Equivalent to the package-private `HTMLStripCharFilterFactory.TAG_NAME_PATTERN`.
pub(crate) static TAG_NAME_PATTERN: LazyLock<JavaPattern> =
    LazyLock::new(|| JavaPattern::compile("[^\\s,]+", 0).expect("INVARIANT: the pattern is valid"));

/// Factory for `HTMLStripCharFilter`.
///
/// ```text
/// <fieldType name="text_html" class="solr.TextField" positionIncrementGap="100">
///   <analyzer>
///     <charFilter class="solr.HTMLStripCharFilterFactory" escapedTags="a, title" />
///     <tokenizer class="solr.WhitespaceTokenizerFactory"/>
///   </analyzer>
/// </fieldType>
/// ```
///
/// Equivalent to `org.apache.lucene.analysis.charfilter.HTMLStripCharFilterFactory`
/// (since 3.1).
#[derive(Debug)]
pub struct HTMLStripCharFilterFactory {
    base: AbstractAnalysisFactory,
    /// Java's package-private `escapedTags`, `null` as `None`.
    pub(crate) escaped_tags: Option<HashSet<String>>,
}

impl HTMLStripCharFilterFactory {
    /// SPI name
    ///
    /// Equivalent to `HTMLStripCharFilterFactory.NAME`.
    pub const NAME: &'static str = "htmlStrip";

    /// Creates a new HTMLStripCharFilterFactory
    ///
    /// Equivalent to `HTMLStripCharFilterFactory(Map<String, String>)`.
    ///
    /// # Errors
    ///
    /// Returns what [`AbstractAnalysisFactory::new`] returns, and
    /// `IllegalArgumentException` for a leftover argument.
    pub fn new(args: &mut HashMap<String, String>) -> Result<Self> {
        let base = AbstractAnalysisFactory::new(args)?;
        let escaped_tags = AbstractAnalysisFactory::get_set(args, "escapedTags")
            .map(|set| set.iter().cloned().collect());
        check_unknown_parameters(args)?;
        Ok(Self { base, escaped_tags })
    }

    /// Default ctor for compatibility with SPI
    ///
    /// Equivalent to `HTMLStripCharFilterFactory()`.
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

    /// The factory's class, as `META-INF/services/org.apache.lucene.analysis.CharFilterFactory`
    /// names it: `HTMLStripCharFilterFactory.class`, with its `NAME` field and
    /// its `Map<String, String>` constructor.
    pub const fn class() -> AnalysisFactoryClass<dyn CharFilterFactory> {
        AnalysisFactoryClass::new(
            "org.apache.lucene.analysis.charfilter.HTMLStripCharFilterFactory",
            SPINameField::Declared(Self::NAME),
            Some(Self::new_from_map),
        )
    }

    /// The `Map<String, String>` constructor, as [`class`](Self::class)
    /// exposes it.
    fn new_from_map(args: &mut HashMap<String, String>) -> Result<Arc<dyn CharFilterFactory>> {
        Ok(Arc::new(Self::new(args)?))
    }
}

impl CharFilterFactory for HTMLStripCharFilterFactory {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.analysis.charfilter.HTMLStripCharFilterFactory"
    }

    /// Equivalent to `HTMLStripCharFilterFactory.create(Reader)`: a `new
    /// HTMLStripCharFilter(input)`, or `new HTMLStripCharFilter(input,
    /// escapedTags)` when there are escaped tags.
    fn create(&self, input: Box<dyn CharReader>) -> Result<Box<dyn CharReader>> {
        let char_filter = match &self.escaped_tags {
            None => HTMLStripCharFilter::new(input),
            Some(escaped_tags) => HTMLStripCharFilter::with_escaped_tags(input, Some(escaped_tags)),
        };
        Ok(Box::new(char_filter))
    }
}
