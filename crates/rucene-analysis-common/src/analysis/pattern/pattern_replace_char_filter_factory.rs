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

//! Factory for `PatternReplaceCharFilter`, ported from
//! `org.apache.lucene.analysis.pattern.PatternReplaceCharFilterFactory`.

use std::collections::HashMap;
use std::sync::Arc;

use rucene::analysis::factories::{
    AbstractAnalysisFactory, AnalysisFactoryClass, CharFilterFactory, SPINameField,
};
use rucene::analysis::java_regex::JavaPattern;
use rucene::analysis::CharReader;
use rucene::error::Result;

use super::pattern_replace_char_filter::PatternReplaceCharFilter;
use crate::spi::check_unknown_parameters;

/// Factory for `PatternReplaceCharFilter`.
///
/// ```text
/// <fieldType name="text_ptnreplace" class="solr.TextField" positionIncrementGap="100">
///   <analyzer>
///     <charFilter class="solr.PatternReplaceCharFilterFactory"
///                    pattern="([^a-z])" replacement=""/>
///     <tokenizer class="solr.KeywordTokenizerFactory"/>
///   </analyzer>
/// </fieldType>
/// ```
///
/// Equivalent to `org.apache.lucene.analysis.pattern.PatternReplaceCharFilterFactory`
/// (since Solr 3.1).
#[derive(Debug)]
pub struct PatternReplaceCharFilterFactory {
    base: AbstractAnalysisFactory,
    pattern: JavaPattern,
    replacement: String,
}

impl PatternReplaceCharFilterFactory {
    /// SPI name.
    ///
    /// Equivalent to `PatternReplaceCharFilterFactory.NAME`.
    pub const NAME: &'static str = "patternReplace";

    /// Creates a new PatternReplaceCharFilterFactory.
    ///
    /// Equivalent to `PatternReplaceCharFilterFactory(Map<String, String>)`.
    ///
    /// # Errors
    ///
    /// Returns what [`AbstractAnalysisFactory::new`] and
    /// [`AbstractAnalysisFactory::get_java_pattern`] return, and
    /// `IllegalArgumentException` for a leftover argument.
    pub fn new(args: &mut HashMap<String, String>) -> Result<Self> {
        let base = AbstractAnalysisFactory::new(args)?;
        let pattern = AbstractAnalysisFactory::get_java_pattern(
            args,
            "pattern",
            "PatternReplaceCharFilterFactory",
        )?;
        let replacement = AbstractAnalysisFactory::get_or(args, "replacement", "");
        check_unknown_parameters(args)?;
        Ok(Self {
            base,
            pattern,
            replacement,
        })
    }

    /// Default ctor for compatibility with SPI.
    ///
    /// Equivalent to `PatternReplaceCharFilterFactory()`.
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
    /// names it: `PatternReplaceCharFilterFactory.class`, with its `NAME`
    /// field and its `Map<String, String>` constructor.
    pub const fn class() -> AnalysisFactoryClass<dyn CharFilterFactory> {
        AnalysisFactoryClass::new(
            "org.apache.lucene.analysis.pattern.PatternReplaceCharFilterFactory",
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

impl CharFilterFactory for PatternReplaceCharFilterFactory {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.analysis.pattern.PatternReplaceCharFilterFactory"
    }

    /// Equivalent to `PatternReplaceCharFilterFactory.create(Reader)`, `new
    /// PatternReplaceCharFilter(pattern, replacement, input)`.
    fn create(&self, input: Box<dyn CharReader>) -> Result<Box<dyn CharReader>> {
        Ok(Box::new(PatternReplaceCharFilter::new(
            self.pattern.clone(),
            self.replacement.clone(),
            input,
        )))
    }

    /// Equivalent to `PatternReplaceCharFilterFactory.normalize(Reader)`,
    /// `create(input)`.
    fn normalize(&self, input: Box<dyn CharReader>) -> Result<Box<dyn CharReader>> {
        self.create(input)
    }
}
