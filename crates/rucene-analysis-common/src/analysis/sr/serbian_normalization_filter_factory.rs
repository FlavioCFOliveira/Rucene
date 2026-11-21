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

//! Factory for `SerbianNormalizationFilter`, ported from
//! `org.apache.lucene.analysis.sr.SerbianNormalizationFilterFactory`.

use std::collections::HashMap;
use std::sync::Arc;

use rucene::analysis::factories::{
    AbstractAnalysisFactory, AnalysisFactoryClass, SPINameField, TokenFilterFactory,
};
use rucene::analysis::TokenStream;
use rucene::error::Result;

use super::serbian_normalization_filter::new_serbian_normalization_filter;
use super::serbian_normalization_regular_filter::new_serbian_normalization_regular_filter;
use crate::spi::check_unknown_parameters;

/// Factory for `SerbianNormalizationFilter`.
///
/// ```text
/// <fieldType name="text_srnorm" class="solr.TextField" positionIncrementGap="100">
///   <analyzer>
///     <tokenizer class="solr.StandardTokenizerFactory"/>
///     <filter class="solr.LowerCaseFilterFactory"/>
///     <filter class="solr.SerbianNormalizationFilterFactory"
///       haircut="bald"/>
///   </analyzer>
/// </fieldType>
/// ```
///
/// Equivalent to `org.apache.lucene.analysis.sr.SerbianNormalizationFilterFactory` (since 5.0.0).
#[derive(Debug)]
pub struct SerbianNormalizationFilterFactory {
    base: AbstractAnalysisFactory,
    /// Equivalent to the package-private field `haircut`.
    pub(crate) haircut: String,
}

impl SerbianNormalizationFilterFactory {
    /// SPI name.
    ///
    /// Equivalent to `SerbianNormalizationFilterFactory.NAME`.
    pub const NAME: &'static str = "serbianNormalization";

    /// Creates a new SerbianNormalizationFilterFactory
    ///
    /// Equivalent to `SerbianNormalizationFilterFactory(Map<String, String>)`:
    /// `haircut` is `get(args, "haircut", Arrays.asList("bald", "regular"),
    /// "bald")`.
    ///
    /// # Errors
    ///
    /// Returns what [`AbstractAnalysisFactory::new`] returns, the
    /// `IllegalArgumentException` of [`AbstractAnalysisFactory::get_one_of`]
    /// for a `haircut` that is neither `bald` nor `regular`, and
    /// `IllegalArgumentException` for a leftover argument.
    pub fn new(args: &mut HashMap<String, String>) -> Result<Self> {
        let base = AbstractAnalysisFactory::new(args)?;

        let haircut = AbstractAnalysisFactory::get_one_of(
            args,
            "haircut",
            &["bald", "regular"],
            Some("bald"),
            true,
        )?
        .expect("INVARIANT: the default haircut is not null");
        check_unknown_parameters(args)?;
        Ok(Self { base, haircut })
    }

    /// Default ctor for compatibility with SPI.
    ///
    /// Equivalent to `SerbianNormalizationFilterFactory()`.
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
    /// names it: `SerbianNormalizationFilterFactory.class`, with its `NAME` field and its
    /// `Map<String, String>` constructor.
    pub const fn class() -> AnalysisFactoryClass<dyn TokenFilterFactory> {
        AnalysisFactoryClass::new(
            "org.apache.lucene.analysis.sr.SerbianNormalizationFilterFactory",
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

impl TokenFilterFactory for SerbianNormalizationFilterFactory {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.analysis.sr.SerbianNormalizationFilterFactory"
    }

    /// Equivalent to `SerbianNormalizationFilterFactory.create(TokenStream)`:
    /// a `SerbianNormalizationRegularFilter` when `haircut` is `regular`, a
    /// `SerbianNormalizationFilter` otherwise.
    fn create(&self, input: Box<dyn TokenStream>) -> Result<Box<dyn TokenStream>> {
        if self.haircut == "regular" {
            Ok(Box::new(new_serbian_normalization_regular_filter(input)?))
        } else {
            Ok(Box::new(new_serbian_normalization_filter(input)?))
        }
    }

    /// Equivalent to `SerbianNormalizationFilterFactory.normalize(TokenStream)`, `create(input)`.
    fn normalize(&self, input: Box<dyn TokenStream>) -> Result<Box<dyn TokenStream>> {
        self.create(input)
    }
}
