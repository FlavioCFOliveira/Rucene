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
//! Factory for `DropIfFlaggedFilter`, ported from
//! `org.apache.lucene.analysis.miscellaneous.DropIfFlaggedFilterFactory`.

use std::collections::HashMap;
use std::sync::Arc;

use rucene::analysis::factories::{
    AbstractAnalysisFactory, AnalysisFactoryClass, SPINameField, TokenFilterFactory,
};
use rucene::analysis::TokenStream;
use rucene::error::Result;

use super::drop_if_flagged_filter::new_drop_if_flagged_filter;

/// Provides a filter that will drop tokens matching a set of flags. This might
/// be used if you had both custom filters that identify tokens to be removed,
/// but need to run before other filters that want to see the token that will
/// eventually be dropped. Alternately you might have separate flag setting
/// filters and then remove tokens that match a particular combination of
/// those filters.
///
/// In Solr this might be configured such as
///
/// ```text
/// <analyzer type="index">
///   <tokenizer class="solr.WhitespaceTokenizerFactory"/>
///   <-- other filters -->
///   <filter class="solr.DropIfFlaggedFilterFactory" dropFlags="9"/>
/// </analyzer>
/// ```
///
/// The above would drop any token that had the first and fourth bit set.
///
/// Like Java's constructor, this one does not refuse leftover arguments.
///
/// Equivalent to `org.apache.lucene.analysis.miscellaneous.DropIfFlaggedFilterFactory` (since 8.8.0).
#[derive(Debug)]
pub struct DropIfFlaggedFilterFactory {
    base: AbstractAnalysisFactory,
    drop_flags: i32,
}

impl DropIfFlaggedFilterFactory {
    /// SPI name.
    ///
    /// Equivalent to `DropIfFlaggedFilterFactory.NAME`.
    pub const NAME: &'static str = "dropIfFlagged";

    /// Creates a new DropIfFlaggedFilterFactory.
    ///
    /// Equivalent to `DropIfFlaggedFilterFactory(Map<String, String>)`.
    ///
    /// # Errors
    ///
    /// Returns what [`AbstractAnalysisFactory::new`] returns, and what an unparsable `dropFlags` raises.
    pub fn new(args: &mut HashMap<String, String>) -> Result<Self> {
        let base = AbstractAnalysisFactory::new(args)?;
        let drop_flags = AbstractAnalysisFactory::get_int(args, "dropFlags", 2)?;
        Ok(Self { base, drop_flags })
    }

    /// Default ctor for compatibility with SPI.
    ///
    /// Equivalent to `DropIfFlaggedFilterFactory()`.
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
    /// names it: `DropIfFlaggedFilterFactory.class`, with its `NAME` field and its
    /// `Map<String, String>` constructor.
    pub const fn class() -> AnalysisFactoryClass<dyn TokenFilterFactory> {
        AnalysisFactoryClass::new(
            "org.apache.lucene.analysis.miscellaneous.DropIfFlaggedFilterFactory",
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

impl TokenFilterFactory for DropIfFlaggedFilterFactory {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.analysis.miscellaneous.DropIfFlaggedFilterFactory"
    }

    /// Equivalent to `DropIfFlaggedFilterFactory.create(TokenStream)`.
    fn create(&self, input: Box<dyn TokenStream>) -> Result<Box<dyn TokenStream>> {
        Ok(Box::new(new_drop_if_flagged_filter(
            input,
            self.drop_flags,
        )?))
    }
}
