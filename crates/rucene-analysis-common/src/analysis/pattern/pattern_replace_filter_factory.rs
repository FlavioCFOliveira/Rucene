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

//! Factory for `PatternReplaceFilter`, ported from
//! `org.apache.lucene.analysis.pattern.PatternReplaceFilterFactory`.

use std::collections::HashMap;
use std::sync::Arc;

use rucene::analysis::factories::{
    AbstractAnalysisFactory, AnalysisFactoryClass, SPINameField, TokenFilterFactory,
};
use rucene::analysis::java_regex::JavaPattern;
use rucene::analysis::TokenStream;
use rucene::error::Result;

use super::pattern_replace_filter::new_pattern_replace_filter;
use crate::spi::check_unknown_parameters;

/// Factory for `PatternReplaceFilter`.
///
/// ```text
/// <fieldType name="text_ptnreplace" class="solr.TextField" positionIncrementGap="100">
///   <analyzer>
///     <tokenizer class="solr.KeywordTokenizerFactory"/>
///     <filter class="solr.PatternReplaceFilterFactory" pattern="([^a-z])" replacement=""
///             replace="all"/>
///   </analyzer>
/// </fieldType>
/// ```
///
/// Equivalent to `org.apache.lucene.analysis.pattern.PatternReplaceFilterFactory`
/// (since 3.1).
#[derive(Debug)]
pub struct PatternReplaceFilterFactory {
    base: AbstractAnalysisFactory,
    pub(crate) pattern: JavaPattern,
    pub(crate) replacement: Option<String>,
    pub(crate) replace_all: bool,
}

impl PatternReplaceFilterFactory {
    /// SPI name.
    ///
    /// Equivalent to `PatternReplaceFilterFactory.NAME`.
    pub const NAME: &'static str = "patternReplace";

    /// Creates a new PatternReplaceFilterFactory.
    ///
    /// Equivalent to `PatternReplaceFilterFactory(Map<String, String>)`.
    ///
    /// # Errors
    ///
    /// Returns what [`AbstractAnalysisFactory::new`] returns, what
    /// [`AbstractAnalysisFactory::get_java_pattern`] and
    /// [`AbstractAnalysisFactory::get_one_of`] return, and
    /// `IllegalArgumentException` for a leftover argument.
    pub fn new(args: &mut HashMap<String, String>) -> Result<Self> {
        let base = AbstractAnalysisFactory::new(args)?;
        let pattern = AbstractAnalysisFactory::get_java_pattern(
            args,
            "pattern",
            "PatternReplaceFilterFactory",
        )?;
        let replacement = AbstractAnalysisFactory::get(args, "replacement");
        let replace_all = AbstractAnalysisFactory::get_one_of(
            args,
            "replace",
            &["all", "first"],
            Some("all"),
            true,
        )?
        .as_deref()
            == Some("all");
        check_unknown_parameters(args)?;
        Ok(Self {
            base,
            pattern,
            replacement,
            replace_all,
        })
    }

    /// Default ctor for compatibility with SPI.
    ///
    /// Equivalent to `PatternReplaceFilterFactory()`.
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
    /// names it: `PatternReplaceFilterFactory.class`, with its `NAME` field
    /// and its `Map<String, String>` constructor.
    pub const fn class() -> AnalysisFactoryClass<dyn TokenFilterFactory> {
        AnalysisFactoryClass::new(
            "org.apache.lucene.analysis.pattern.PatternReplaceFilterFactory",
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

impl TokenFilterFactory for PatternReplaceFilterFactory {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.analysis.pattern.PatternReplaceFilterFactory"
    }

    /// Equivalent to `PatternReplaceFilterFactory.create(TokenStream)`, `new
    /// PatternReplaceFilter(input, pattern, replacement, replaceAll)`.
    fn create(&self, input: Box<dyn TokenStream>) -> Result<Box<dyn TokenStream>> {
        Ok(Box::new(new_pattern_replace_filter(
            input,
            &self.pattern,
            self.replacement.as_deref(),
            self.replace_all,
        )?))
    }
}
