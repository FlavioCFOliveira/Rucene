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
//! Factory for `LengthFilter`, ported from
//! `org.apache.lucene.analysis.miscellaneous.LengthFilterFactory`.

use std::collections::HashMap;
use std::sync::Arc;

use rucene::analysis::factories::{
    AbstractAnalysisFactory, AnalysisFactoryClass, SPINameField, TokenFilterFactory,
};
use rucene::analysis::TokenStream;
use rucene::error::Result;

use super::length_filter::new_length_filter;
use crate::spi::check_unknown_parameters;

/// Factory for `LengthFilter`.
///
/// Equivalent to `org.apache.lucene.analysis.miscellaneous.LengthFilterFactory` (since 3.1).
#[derive(Debug)]
pub struct LengthFilterFactory {
    base: AbstractAnalysisFactory,
    min: i32,
    max: i32,
}

impl LengthFilterFactory {
    /// SPI name.
    ///
    /// Equivalent to `LengthFilterFactory.NAME`.
    pub const NAME: &'static str = "length";

    /// Equivalent to `LengthFilterFactory.MIN_KEY`.
    pub const MIN_KEY: &'static str = "min";
    /// Equivalent to `LengthFilterFactory.MAX_KEY`.
    pub const MAX_KEY: &'static str = "max";

    /// Creates a new LengthFilterFactory.
    ///
    /// Equivalent to `LengthFilterFactory(Map<String, String>)`.
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
    /// Equivalent to `LengthFilterFactory()`.
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
    /// names it: `LengthFilterFactory.class`, with its `NAME` field and its
    /// `Map<String, String>` constructor.
    pub const fn class() -> AnalysisFactoryClass<dyn TokenFilterFactory> {
        AnalysisFactoryClass::new(
            "org.apache.lucene.analysis.miscellaneous.LengthFilterFactory",
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

impl TokenFilterFactory for LengthFilterFactory {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.analysis.miscellaneous.LengthFilterFactory"
    }

    /// Equivalent to `LengthFilterFactory.create(TokenStream)`, `new LengthFilter(input, min, max)`.
    fn create(&self, input: Box<dyn TokenStream>) -> Result<Box<dyn TokenStream>> {
        Ok(Box::new(new_length_filter(input, self.min, self.max)?))
    }
}
