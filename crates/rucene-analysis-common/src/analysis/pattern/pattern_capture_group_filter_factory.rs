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

//! Factory for `PatternCaptureGroupTokenFilter`, ported from
//! `org.apache.lucene.analysis.pattern.PatternCaptureGroupFilterFactory`.

use std::collections::HashMap;
use std::sync::Arc;

use rucene::analysis::factories::{
    AbstractAnalysisFactory, AnalysisFactoryClass, SPINameField, TokenFilterFactory,
};
use rucene::analysis::java_regex::JavaPattern;
use rucene::analysis::TokenStream;
use rucene::error::Result;

use super::pattern_capture_group_token_filter::new_pattern_capture_group_token_filter;

/// Factory for `PatternCaptureGroupTokenFilter`.
///
/// ```text
/// <fieldType name="text_ptncapturegroup" class="solr.TextField" positionIncrementGap="100">
///   <analyzer>
///     <tokenizer class="solr.KeywordTokenizerFactory"/>
///     <filter class="solr.PatternCaptureGroupFilterFactory" pattern="([^a-z])" preserve_original="true"/>
///   </analyzer>
/// </fieldType>
/// ```
///
/// Equivalent to `org.apache.lucene.analysis.pattern.PatternCaptureGroupFilterFactory`
/// (since 4.4.0).
#[derive(Debug)]
pub struct PatternCaptureGroupFilterFactory {
    base: AbstractAnalysisFactory,
    pattern: JavaPattern,
    preserve_original: bool,
}

impl PatternCaptureGroupFilterFactory {
    /// SPI name.
    ///
    /// Equivalent to `PatternCaptureGroupFilterFactory.NAME`.
    pub const NAME: &'static str = "patternCaptureGroup";

    /// Equivalent to `PatternCaptureGroupFilterFactory(Map<String, String>)`.
    ///
    /// Java checks no leftover argument here, and neither does the port.
    ///
    /// # Errors
    ///
    /// Returns what [`AbstractAnalysisFactory::new`] and
    /// [`AbstractAnalysisFactory::get_java_pattern`] return.
    pub fn new(args: &mut HashMap<String, String>) -> Result<Self> {
        let base = AbstractAnalysisFactory::new(args)?;
        let pattern = AbstractAnalysisFactory::get_java_pattern(
            args,
            "pattern",
            "PatternCaptureGroupFilterFactory",
        )?;
        // `args.containsKey(..) ? Boolean.parseBoolean(args.get(..)) : true`:
        // the argument is read, not consumed.
        let preserve_original = match args.get("preserve_original") {
            Some(value) => value.eq_ignore_ascii_case("true"),
            None => true,
        };
        Ok(Self {
            base,
            pattern,
            preserve_original,
        })
    }

    /// Default ctor for compatibility with SPI.
    ///
    /// Equivalent to `PatternCaptureGroupFilterFactory()`.
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
    /// names it: `PatternCaptureGroupFilterFactory.class`, with its `NAME`
    /// field and its `Map<String, String>` constructor.
    pub const fn class() -> AnalysisFactoryClass<dyn TokenFilterFactory> {
        AnalysisFactoryClass::new(
            "org.apache.lucene.analysis.pattern.PatternCaptureGroupFilterFactory",
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

impl TokenFilterFactory for PatternCaptureGroupFilterFactory {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.analysis.pattern.PatternCaptureGroupFilterFactory"
    }

    /// Equivalent to `PatternCaptureGroupFilterFactory.create(TokenStream)`,
    /// `new PatternCaptureGroupTokenFilter(input, preserveOriginal, pattern)`.
    fn create(&self, input: Box<dyn TokenStream>) -> Result<Box<dyn TokenStream>> {
        Ok(Box::new(new_pattern_capture_group_token_filter(
            input,
            self.preserve_original,
            std::slice::from_ref(&self.pattern),
        )?))
    }
}
