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
//! Factory for `LimitTokenOffsetFilter`, ported from
//! `org.apache.lucene.analysis.miscellaneous.LimitTokenOffsetFilterFactory`.

use std::collections::HashMap;
use std::sync::Arc;

use rucene::analysis::factories::{
    AbstractAnalysisFactory, AnalysisFactoryClass, SPINameField, TokenFilterFactory,
};
use rucene::analysis::TokenStream;
use rucene::error::Result;

use super::limit_token_offset_filter::new_limit_token_offset_filter_consuming;
use crate::spi::check_unknown_parameters;

/// Factory for `LimitTokenOffsetFilter`.
///
/// Equivalent to `org.apache.lucene.analysis.miscellaneous.LimitTokenOffsetFilterFactory` (since 5.2.0).
#[derive(Debug)]
pub struct LimitTokenOffsetFilterFactory {
    base: AbstractAnalysisFactory,
    max_start_offset: i32,
    consume_all_tokens: bool,
}

impl LimitTokenOffsetFilterFactory {
    /// SPI name.
    ///
    /// Equivalent to `LimitTokenOffsetFilterFactory.NAME`.
    pub const NAME: &'static str = "limitTokenOffset";

    /// Equivalent to `LimitTokenOffsetFilterFactory.MAX_START_OFFSET`.
    pub const MAX_START_OFFSET: &'static str = "maxStartOffset";
    /// Equivalent to `LimitTokenOffsetFilterFactory.CONSUME_ALL_TOKENS_KEY`.
    pub const CONSUME_ALL_TOKENS_KEY: &'static str = "consumeAllTokens";

    /// Creates a new LimitTokenOffsetFilterFactory.
    ///
    /// Equivalent to `LimitTokenOffsetFilterFactory(Map<String, String>)`.
    ///
    /// # Errors
    ///
    /// Returns what [`AbstractAnalysisFactory::new`] returns, what a missing or unparsable `maxStartOffset` raises, and `IllegalArgumentException` for a leftover argument.
    pub fn new(args: &mut HashMap<String, String>) -> Result<Self> {
        let base = AbstractAnalysisFactory::new(args)?;
        let max_start_offset = AbstractAnalysisFactory::require_int(args, Self::MAX_START_OFFSET)?;
        let consume_all_tokens =
            AbstractAnalysisFactory::get_boolean(args, Self::CONSUME_ALL_TOKENS_KEY, false);
        check_unknown_parameters(args)?;
        Ok(Self {
            base,
            max_start_offset,
            consume_all_tokens,
        })
    }

    /// Default ctor for compatibility with SPI.
    ///
    /// Equivalent to `LimitTokenOffsetFilterFactory()`.
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
    /// names it: `LimitTokenOffsetFilterFactory.class`, with its `NAME` field and its
    /// `Map<String, String>` constructor.
    pub const fn class() -> AnalysisFactoryClass<dyn TokenFilterFactory> {
        AnalysisFactoryClass::new(
            "org.apache.lucene.analysis.miscellaneous.LimitTokenOffsetFilterFactory",
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

impl TokenFilterFactory for LimitTokenOffsetFilterFactory {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.analysis.miscellaneous.LimitTokenOffsetFilterFactory"
    }

    /// Equivalent to `LimitTokenOffsetFilterFactory.create(TokenStream)`.
    fn create(&self, input: Box<dyn TokenStream>) -> Result<Box<dyn TokenStream>> {
        Ok(Box::new(new_limit_token_offset_filter_consuming(
            input,
            self.max_start_offset,
            self.consume_all_tokens,
        )?))
    }
}
