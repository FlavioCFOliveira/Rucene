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
//! Factory for `CodepointCountFilter`, ported from
//! `org.apache.lucene.analysis.miscellaneous.CodepointCountFilterFactory`.

use std::collections::HashMap;
use std::sync::Arc;

use rucene::analysis::factories::{
    AbstractAnalysisFactory, AnalysisFactoryClass, SPINameField, TokenFilterFactory,
};
use rucene::analysis::TokenStream;
use rucene::error::Result;

use super::codepoint_count_filter::new_codepoint_count_filter;
use crate::spi::check_unknown_parameters;

/// Factory for `CodepointCountFilter`.
///
/// Equivalent to `org.apache.lucene.analysis.miscellaneous.CodepointCountFilterFactory` (since 4.5.1).
#[derive(Debug)]
pub struct CodepointCountFilterFactory {
    base: AbstractAnalysisFactory,
    min: i32,
    max: i32,
}

impl CodepointCountFilterFactory {
    /// SPI name.
    ///
    /// Equivalent to `CodepointCountFilterFactory.NAME`.
    pub const NAME: &'static str = "codepointCount";

    /// Equivalent to `CodepointCountFilterFactory.MIN_KEY`.
    pub const MIN_KEY: &'static str = "min";
    /// Equivalent to `CodepointCountFilterFactory.MAX_KEY`.
    pub const MAX_KEY: &'static str = "max";

    /// Creates a new CodepointCountFilterFactory.
    ///
    /// Equivalent to `CodepointCountFilterFactory(Map<String, String>)`.
    ///
    /// # Errors
    ///
    /// Returns what [`AbstractAnalysisFactory::new`] returns, what a missing or unparsable `min` or `max` raises, and `IllegalArgumentException` for a leftover argument.
    pub fn new(args: &mut HashMap<String, String>) -> Result<Self> {
        let base = AbstractAnalysisFactory::new(args)?;
        let min = AbstractAnalysisFactory::require_int(args, Self::MIN_KEY)?;
        let max = AbstractAnalysisFactory::require_int(args, Self::MAX_KEY)?;
        check_unknown_parameters(args)?;
        Ok(Self { base, min, max })
    }

    /// Default ctor for compatibility with SPI.
    ///
    /// Equivalent to `CodepointCountFilterFactory()`.
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
    /// names it: `CodepointCountFilterFactory.class`, with its `NAME` field and its
    /// `Map<String, String>` constructor.
    pub const fn class() -> AnalysisFactoryClass<dyn TokenFilterFactory> {
        AnalysisFactoryClass::new(
            "org.apache.lucene.analysis.miscellaneous.CodepointCountFilterFactory",
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

impl TokenFilterFactory for CodepointCountFilterFactory {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.analysis.miscellaneous.CodepointCountFilterFactory"
    }

    /// Equivalent to `CodepointCountFilterFactory.create(TokenStream)`, `new CodepointCountFilter(input, min, max)`.
    fn create(&self, input: Box<dyn TokenStream>) -> Result<Box<dyn TokenStream>> {
        Ok(Box::new(new_codepoint_count_filter(
            input, self.min, self.max,
        )?))
    }
}
