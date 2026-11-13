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
//! Factory for `FixBrokenOffsetsFilter`, ported from
//! `org.apache.lucene.analysis.miscellaneous.FixBrokenOffsetsFilterFactory`.

use std::collections::HashMap;
use std::sync::Arc;

use rucene::analysis::factories::{
    AbstractAnalysisFactory, AnalysisFactoryClass, SPINameField, TokenFilterFactory,
};
use rucene::analysis::TokenStream;
use rucene::error::Result;

#[allow(deprecated)]
use super::fix_broken_offsets_filter::new_fix_broken_offsets_filter;

/// Factory for `FixBrokenOffsetsFilter`.
///
/// Like Java's constructor, this one does not refuse leftover arguments.
///
/// Equivalent to `org.apache.lucene.analysis.miscellaneous.FixBrokenOffsetsFilterFactory` (since 7.0.0; deprecated).
#[derive(Debug)]
pub struct FixBrokenOffsetsFilterFactory {
    base: AbstractAnalysisFactory,
}

impl FixBrokenOffsetsFilterFactory {
    /// SPI name.
    ///
    /// Equivalent to `FixBrokenOffsetsFilterFactory.NAME`.
    pub const NAME: &'static str = "fixBrokenOffsets";

    /// Creates a new FixBrokenOffsetsFilterFactory.
    ///
    /// Equivalent to `FixBrokenOffsetsFilterFactory(Map<String, String>)`.
    ///
    /// # Errors
    ///
    /// Returns what [`AbstractAnalysisFactory::new`] returns.
    pub fn new(args: &mut HashMap<String, String>) -> Result<Self> {
        let base = AbstractAnalysisFactory::new(args)?;
        Ok(Self { base })
    }

    /// Default ctor for compatibility with SPI.
    ///
    /// Equivalent to `FixBrokenOffsetsFilterFactory()`.
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
    /// names it: `FixBrokenOffsetsFilterFactory.class`, with its `NAME` field and its
    /// `Map<String, String>` constructor.
    pub const fn class() -> AnalysisFactoryClass<dyn TokenFilterFactory> {
        AnalysisFactoryClass::new(
            "org.apache.lucene.analysis.miscellaneous.FixBrokenOffsetsFilterFactory",
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

impl TokenFilterFactory for FixBrokenOffsetsFilterFactory {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.analysis.miscellaneous.FixBrokenOffsetsFilterFactory"
    }

    /// Equivalent to `FixBrokenOffsetsFilterFactory.create(TokenStream)`.
    #[allow(deprecated)]
    fn create(&self, input: Box<dyn TokenStream>) -> Result<Box<dyn TokenStream>> {
        Ok(Box::new(new_fix_broken_offsets_filter(input)?))
    }
}
