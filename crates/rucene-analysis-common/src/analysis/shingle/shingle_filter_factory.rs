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

//! Factory for `ShingleFilter`, ported from
//! `org.apache.lucene.analysis.shingle.ShingleFilterFactory`.

use std::collections::HashMap;
use std::sync::Arc;

use rucene::analysis::factories::{
    AbstractAnalysisFactory, AnalysisFactoryClass, SPINameField, TokenFilterFactory,
};
use rucene::analysis::TokenStream;
use rucene::error::{LuceneError, Result};

use super::shingle_filter::{
    new_shingle_filter, DEFAULT_FILLER_TOKEN, DEFAULT_MAX_SHINGLE_SIZE, DEFAULT_MIN_SHINGLE_SIZE,
    DEFAULT_TOKEN_SEPARATOR,
};
use crate::spi::check_unknown_parameters;

/// Factory for `ShingleFilter`.
///
/// ```text
/// <fieldType name="text_shingle" class="solr.TextField" positionIncrementGap="100">
///   <analyzer>
///     <tokenizer class="solr.WhitespaceTokenizerFactory"/>
///     <filter class="solr.ShingleFilterFactory" minShingleSize="2" maxShingleSize="2"
///             outputUnigrams="true" outputUnigramsIfNoShingles="false" tokenSeparator=" " fillerToken="_"/>
///   </analyzer>
/// </fieldType>
/// ```
///
/// Equivalent to `org.apache.lucene.analysis.shingle.ShingleFilterFactory`
/// (since 3.1).
#[derive(Debug)]
pub struct ShingleFilterFactory {
    base: AbstractAnalysisFactory,
    min_shingle_size: i32,
    max_shingle_size: i32,
    output_unigrams: bool,
    output_unigrams_if_no_shingles: bool,
    token_separator: String,
    filler_token: String,
}

impl ShingleFilterFactory {
    /// SPI name.
    ///
    /// Equivalent to `ShingleFilterFactory.NAME`.
    pub const NAME: &'static str = "shingle";

    /// Creates a new ShingleFilterFactory.
    ///
    /// Equivalent to `ShingleFilterFactory(Map<String, String>)`.
    ///
    /// # Errors
    ///
    /// Returns what [`AbstractAnalysisFactory::new`] and
    /// [`AbstractAnalysisFactory::get_int`] return, and
    /// `IllegalArgumentException` with Java's messages for invalid shingle
    /// sizes and a leftover argument.
    pub fn new(args: &mut HashMap<String, String>) -> Result<Self> {
        let base = AbstractAnalysisFactory::new(args)?;
        let max_shingle_size =
            AbstractAnalysisFactory::get_int(args, "maxShingleSize", DEFAULT_MAX_SHINGLE_SIZE)?;
        if max_shingle_size < 2 {
            return Err(LuceneError::illegal_argument(format!(
                "Invalid maxShingleSize ({max_shingle_size}) - must be at least 2"
            )));
        }
        let min_shingle_size =
            AbstractAnalysisFactory::get_int(args, "minShingleSize", DEFAULT_MIN_SHINGLE_SIZE)?;
        if min_shingle_size < 2 {
            return Err(LuceneError::illegal_argument(format!(
                "Invalid minShingleSize ({min_shingle_size}) - must be at least 2"
            )));
        }
        if min_shingle_size > max_shingle_size {
            return Err(LuceneError::illegal_argument(format!(
                "Invalid minShingleSize ({min_shingle_size}) - must be no greater than maxShingleSize ({max_shingle_size})"
            )));
        }
        let output_unigrams = AbstractAnalysisFactory::get_boolean(args, "outputUnigrams", true);
        let output_unigrams_if_no_shingles =
            AbstractAnalysisFactory::get_boolean(args, "outputUnigramsIfNoShingles", false);
        let token_separator =
            AbstractAnalysisFactory::get_or(args, "tokenSeparator", DEFAULT_TOKEN_SEPARATOR);
        let filler_token =
            AbstractAnalysisFactory::get_or(args, "fillerToken", DEFAULT_FILLER_TOKEN);
        check_unknown_parameters(args)?;
        Ok(Self {
            base,
            min_shingle_size,
            max_shingle_size,
            output_unigrams,
            output_unigrams_if_no_shingles,
            token_separator,
            filler_token,
        })
    }

    /// Default ctor for compatibility with SPI.
    ///
    /// Equivalent to `ShingleFilterFactory()`.
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
    /// names it: `ShingleFilterFactory.class`, with its `NAME` field and its
    /// `Map<String, String>` constructor.
    pub const fn class() -> AnalysisFactoryClass<dyn TokenFilterFactory> {
        AnalysisFactoryClass::new(
            "org.apache.lucene.analysis.shingle.ShingleFilterFactory",
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

impl TokenFilterFactory for ShingleFilterFactory {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.analysis.shingle.ShingleFilterFactory"
    }

    /// Equivalent to `ShingleFilterFactory.create(TokenStream)`.
    fn create(&self, input: Box<dyn TokenStream>) -> Result<Box<dyn TokenStream>> {
        let mut r = new_shingle_filter(input, self.min_shingle_size, self.max_shingle_size)?;
        let logic = r.logic_mut();
        logic.set_output_unigrams(self.output_unigrams);
        logic.set_output_unigrams_if_no_shingles(self.output_unigrams_if_no_shingles);
        logic.set_token_separator(Some(&self.token_separator));
        logic.set_filler_token(Some(&self.filler_token));
        Ok(Box::new(r))
    }
}
