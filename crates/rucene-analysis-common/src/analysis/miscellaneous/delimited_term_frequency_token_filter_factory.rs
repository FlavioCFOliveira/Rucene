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
//! Factory for `DelimitedTermFrequencyTokenFilter`, ported from
//! `org.apache.lucene.analysis.miscellaneous.DelimitedTermFrequencyTokenFilterFactory`.

use std::collections::HashMap;
use std::sync::Arc;

use rucene::analysis::factories::{
    AbstractAnalysisFactory, AnalysisFactoryClass, SPINameField, TokenFilterFactory,
};
use rucene::analysis::TokenStream;
use rucene::error::Result;

use super::delimited_term_frequency_token_filter::{
    new_delimited_term_frequency_token_filter_with_delimiter, DEFAULT_DELIMITER,
};
use crate::spi::check_unknown_parameters;

/// Factory for `DelimitedTermFrequencyTokenFilter`. The field must have
/// `omitPositions=true`.
///
/// Equivalent to `org.apache.lucene.analysis.miscellaneous.DelimitedTermFrequencyTokenFilterFactory` (since 7.0.0).
#[derive(Debug)]
pub struct DelimitedTermFrequencyTokenFilterFactory {
    base: AbstractAnalysisFactory,
    delimiter: char,
}

impl DelimitedTermFrequencyTokenFilterFactory {
    /// SPI name.
    ///
    /// Equivalent to `DelimitedTermFrequencyTokenFilterFactory.NAME`.
    pub const NAME: &'static str = "delimitedTermFrequency";

    /// Equivalent to `DelimitedTermFrequencyTokenFilterFactory.DELIMITER_ATTR`.
    pub const DELIMITER_ATTR: &'static str = "delimiter";

    /// Creates a new DelimitedTermFrequencyTokenFilterFactory.
    ///
    /// Equivalent to `DelimitedTermFrequencyTokenFilterFactory(Map<String, String>)`.
    ///
    /// # Errors
    ///
    /// Returns what [`AbstractAnalysisFactory::new`] returns, `IllegalArgumentException` for a `delimiter` that is not one char, and for a leftover argument.
    pub fn new(args: &mut HashMap<String, String>) -> Result<Self> {
        let base = AbstractAnalysisFactory::new(args)?;
        let delimiter =
            AbstractAnalysisFactory::get_char(args, Self::DELIMITER_ATTR, DEFAULT_DELIMITER)?;
        check_unknown_parameters(args)?;
        Ok(Self { base, delimiter })
    }

    /// Default ctor for compatibility with SPI.
    ///
    /// Equivalent to `DelimitedTermFrequencyTokenFilterFactory()`.
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
    /// names it: `DelimitedTermFrequencyTokenFilterFactory.class`, with its `NAME` field and its
    /// `Map<String, String>` constructor.
    pub const fn class() -> AnalysisFactoryClass<dyn TokenFilterFactory> {
        AnalysisFactoryClass::new(
            "org.apache.lucene.analysis.miscellaneous.DelimitedTermFrequencyTokenFilterFactory",
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

impl TokenFilterFactory for DelimitedTermFrequencyTokenFilterFactory {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.analysis.miscellaneous.DelimitedTermFrequencyTokenFilterFactory"
    }

    /// Equivalent to `DelimitedTermFrequencyTokenFilterFactory.create(TokenStream)`.
    fn create(&self, input: Box<dyn TokenStream>) -> Result<Box<dyn TokenStream>> {
        Ok(Box::new(
            new_delimited_term_frequency_token_filter_with_delimiter(input, self.delimiter)?,
        ))
    }
}
