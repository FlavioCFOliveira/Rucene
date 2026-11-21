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

//! Factory for `FixedShingleFilter`, ported from
//! `org.apache.lucene.analysis.shingle.FixedShingleFilterFactory`.

use std::collections::HashMap;
use std::sync::Arc;

use rucene::analysis::factories::{
    AbstractAnalysisFactory, AnalysisFactoryClass, SPINameField, TokenFilterFactory,
};
use rucene::analysis::TokenStream;
use rucene::error::Result;

use super::fixed_shingle_filter::FixedShingleFilter;

/// Factory for `FixedShingleFilter`
///
/// Parameters are:
///
/// * shingleSize - how many tokens should be combined into each shingle
///   (default: 2)
/// * tokenSeparator - how tokens should be joined together in the shingle
///   (default: space)
/// * fillerToken - what should be added in place of stop words (default: _ )
///
/// Equivalent to `org.apache.lucene.analysis.shingle.FixedShingleFilterFactory`
/// (since 7.4.0).
#[derive(Debug)]
pub struct FixedShingleFilterFactory {
    base: AbstractAnalysisFactory,
    shingle_size: i32,
    token_separator: String,
    filler_token: String,
}

impl FixedShingleFilterFactory {
    /// SPI name.
    ///
    /// Equivalent to `FixedShingleFilterFactory.NAME`.
    pub const NAME: &'static str = "fixedShingle";

    /// Equivalent to `FixedShingleFilterFactory(Map<String, String>)`.
    ///
    /// Java checks no leftover argument here, and neither does the port.
    ///
    /// # Errors
    ///
    /// Returns what [`AbstractAnalysisFactory::new`] and
    /// [`AbstractAnalysisFactory::get_int`] return.
    pub fn new(args: &mut HashMap<String, String>) -> Result<Self> {
        let base = AbstractAnalysisFactory::new(args)?;
        let shingle_size = AbstractAnalysisFactory::get_int(args, "shingleSize", 2)?;
        let token_separator = AbstractAnalysisFactory::get_or(args, "tokenSeparator", " ");
        let filler_token = AbstractAnalysisFactory::get_or(args, "fillerToken", "_");
        Ok(Self {
            base,
            shingle_size,
            token_separator,
            filler_token,
        })
    }

    /// Default ctor for compatibility with SPI.
    ///
    /// Equivalent to `FixedShingleFilterFactory()`.
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
    /// names it: `FixedShingleFilterFactory.class`, with its `NAME` field
    /// and its `Map<String, String>` constructor.
    pub const fn class() -> AnalysisFactoryClass<dyn TokenFilterFactory> {
        AnalysisFactoryClass::new(
            "org.apache.lucene.analysis.shingle.FixedShingleFilterFactory",
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

impl TokenFilterFactory for FixedShingleFilterFactory {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.analysis.shingle.FixedShingleFilterFactory"
    }

    /// Equivalent to `FixedShingleFilterFactory.create(TokenStream)`, `new
    /// FixedShingleFilter(input, shingleSize, tokenSeparator, fillerToken)`.
    fn create(&self, input: Box<dyn TokenStream>) -> Result<Box<dyn TokenStream>> {
        Ok(Box::new(FixedShingleFilter::with_separators(
            input,
            self.shingle_size,
            &self.token_separator,
            &self.filler_token,
        )?))
    }
}
