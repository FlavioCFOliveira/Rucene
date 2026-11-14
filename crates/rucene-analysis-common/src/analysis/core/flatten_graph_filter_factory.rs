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

//! Factory for `FlattenGraphFilter`, ported from
//! `org.apache.lucene.analysis.core.FlattenGraphFilterFactory`.

use std::collections::HashMap;
use std::sync::Arc;

use rucene::analysis::factories::{
    AbstractAnalysisFactory, AnalysisFactoryClass, SPINameField, TokenFilterFactory,
};
use rucene::analysis::TokenStream;
use rucene::error::Result;

use super::flatten_graph_filter::new_flatten_graph_filter;
use crate::spi::check_unknown_parameters;

/// Factory for `FlattenGraphFilter`.
///
/// Equivalent to `org.apache.lucene.analysis.core.FlattenGraphFilterFactory`
/// (`@lucene.experimental`, since 6.4.0).
#[derive(Debug)]
pub struct FlattenGraphFilterFactory {
    base: AbstractAnalysisFactory,
}

impl FlattenGraphFilterFactory {
    /// SPI name.
    ///
    /// Equivalent to `FlattenGraphFilterFactory.NAME`.
    pub const NAME: &'static str = "flattenGraph";

    /// Creates a new FlattenGraphFilterFactory.
    ///
    /// Equivalent to `FlattenGraphFilterFactory(Map<String, String>)`.
    ///
    /// # Errors
    ///
    /// Returns what [`AbstractAnalysisFactory::new`] returns, and
    /// `IllegalArgumentException` for a leftover argument.
    pub fn new(args: &mut HashMap<String, String>) -> Result<Self> {
        let base = AbstractAnalysisFactory::new(args)?;
        check_unknown_parameters(args)?;
        Ok(Self { base })
    }

    /// Default ctor for compatibility with SPI.
    ///
    /// Equivalent to `FlattenGraphFilterFactory()`.
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
    /// names it: `FlattenGraphFilterFactory.class`, with its `NAME` field and its
    /// `Map<String, String>` constructor.
    pub const fn class() -> AnalysisFactoryClass<dyn TokenFilterFactory> {
        AnalysisFactoryClass::new(
            "org.apache.lucene.analysis.core.FlattenGraphFilterFactory",
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

impl TokenFilterFactory for FlattenGraphFilterFactory {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.analysis.core.FlattenGraphFilterFactory"
    }

    /// Equivalent to `FlattenGraphFilterFactory.create(TokenStream)`, `new FlattenGraphFilter(input)`.
    fn create(&self, input: Box<dyn TokenStream>) -> Result<Box<dyn TokenStream>> {
        Ok(Box::new(new_flatten_graph_filter(input)?))
    }
}
