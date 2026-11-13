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
//! Factory for `FingerprintFilter`, ported from
//! `org.apache.lucene.analysis.miscellaneous.FingerprintFilterFactory`.

use std::collections::HashMap;
use std::sync::Arc;

use rucene::analysis::factories::{
    AbstractAnalysisFactory, AnalysisFactoryClass, SPINameField, TokenFilterFactory,
};
use rucene::analysis::TokenStream;
use rucene::error::Result;

use super::fingerprint_filter::{
    new_fingerprint_filter_with, DEFAULT_MAX_OUTPUT_TOKEN_SIZE, DEFAULT_SEPARATOR,
};
use crate::spi::check_unknown_parameters;

/// Factory for `FingerprintFilter`.
///
/// The `maxOutputTokenSize` property is the maximum length of the fingerprint
/// token output (if exceeded, no token is emitted); `separator` is the
/// character used to separate tokens combined into the single output token.
///
/// ```text
/// <fieldType name="text_fingerprint" class="solr.TextField" positionIncrementGap="100">
///   <analyzer>
///     <tokenizer class="solr.WhitespaceTokenizerFactory"/>
///     <filter class="solr.FingerprintFilterFactory" maxOutputTokenSize="1024" separator=" "/>
///   </analyzer>
/// </fieldType>
/// ```
///
/// Equivalent to `org.apache.lucene.analysis.miscellaneous.FingerprintFilterFactory` (since 5.4.0).
#[derive(Debug)]
pub struct FingerprintFilterFactory {
    base: AbstractAnalysisFactory,
    max_output_token_size: i32,
    separator: char,
}

impl FingerprintFilterFactory {
    /// SPI name.
    ///
    /// Equivalent to `FingerprintFilterFactory.NAME`.
    pub const NAME: &'static str = "fingerprint";

    /// Equivalent to `FingerprintFilterFactory.MAX_OUTPUT_TOKEN_SIZE_KEY`.
    pub const MAX_OUTPUT_TOKEN_SIZE_KEY: &'static str = "maxOutputTokenSize";

    /// Equivalent to `FingerprintFilterFactory.SEPARATOR_KEY`.
    pub const SEPARATOR_KEY: &'static str = "separator";

    /// Creates a new FingerprintFilterFactory.
    ///
    /// Equivalent to `FingerprintFilterFactory(Map<String, String>)`.
    ///
    /// # Errors
    ///
    /// Returns what [`AbstractAnalysisFactory::new`] returns, what an unparsable `maxOutputTokenSize` or a `separator` that is not one char raises, and `IllegalArgumentException` for a leftover argument.
    pub fn new(args: &mut HashMap<String, String>) -> Result<Self> {
        let base = AbstractAnalysisFactory::new(args)?;
        let max_output_token_size = AbstractAnalysisFactory::get_int(
            args,
            Self::MAX_OUTPUT_TOKEN_SIZE_KEY,
            DEFAULT_MAX_OUTPUT_TOKEN_SIZE,
        )?;
        let separator =
            AbstractAnalysisFactory::get_char(args, Self::SEPARATOR_KEY, DEFAULT_SEPARATOR)?;
        check_unknown_parameters(args)?;
        Ok(Self {
            base,
            max_output_token_size,
            separator,
        })
    }

    /// Default ctor for compatibility with SPI.
    ///
    /// Equivalent to `FingerprintFilterFactory()`.
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
    /// names it: `FingerprintFilterFactory.class`, with its `NAME` field and its
    /// `Map<String, String>` constructor.
    pub const fn class() -> AnalysisFactoryClass<dyn TokenFilterFactory> {
        AnalysisFactoryClass::new(
            "org.apache.lucene.analysis.miscellaneous.FingerprintFilterFactory",
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

impl TokenFilterFactory for FingerprintFilterFactory {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.analysis.miscellaneous.FingerprintFilterFactory"
    }

    /// Equivalent to `FingerprintFilterFactory.create(TokenStream)`.
    fn create(&self, input: Box<dyn TokenStream>) -> Result<Box<dyn TokenStream>> {
        Ok(Box::new(new_fingerprint_filter_with(
            input,
            self.max_output_token_size,
            self.separator,
        )?))
    }
}
