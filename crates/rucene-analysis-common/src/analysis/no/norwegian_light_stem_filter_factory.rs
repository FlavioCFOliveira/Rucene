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

//! Factory for `NorwegianLightStemFilter`, ported from
//! `org.apache.lucene.analysis.no.NorwegianLightStemFilterFactory`.

use std::collections::HashMap;
use std::sync::Arc;

use rucene::analysis::factories::{
    AbstractAnalysisFactory, AnalysisFactoryClass, SPINameField, TokenFilterFactory,
};
use rucene::analysis::TokenStream;
use rucene::error::{LuceneError, Result};

use super::norwegian_light_stem_filter::new_norwegian_light_stem_filter_with_flags;
use super::norwegian_light_stemmer::{BOKMAAL, NYNORSK};
use crate::spi::check_unknown_parameters;

/// Factory for `NorwegianLightStemFilter`.
///
/// ```text
/// <fieldType name="text_svlgtstem" class="solr.TextField" positionIncrementGap="100">
///   <analyzer>
///     <tokenizer class="solr.StandardTokenizerFactory"/>
///     <filter class="solr.LowerCaseFilterFactory"/>
///     <filter class="solr.NorwegianLightStemFilterFactory" variant="nb"/>
///   </analyzer>
/// </fieldType>
/// ```
///
/// Equivalent to `org.apache.lucene.analysis.no.NorwegianLightStemFilterFactory` (since 3.6.0).
#[derive(Debug)]
pub struct NorwegianLightStemFilterFactory {
    base: AbstractAnalysisFactory,
    flags: i32,
}

impl NorwegianLightStemFilterFactory {
    /// SPI name.
    ///
    /// Equivalent to `NorwegianLightStemFilterFactory.NAME`.
    pub const NAME: &'static str = "norwegianLightStem";

    /// Creates a new NorwegianLightStemFilterFactory
    ///
    /// Equivalent to `NorwegianLightStemFilterFactory(Map<String, String>)`:
    /// `variant` `nb` (the default) selects `BOKMAAL`, `nn` `NYNORSK`, `no`
    /// both.
    ///
    /// # Errors
    ///
    /// Returns what [`AbstractAnalysisFactory::new`] returns,
    /// `IllegalArgumentException` "invalid variant: " for another variant, and
    /// `IllegalArgumentException` for a leftover argument.
    pub fn new(args: &mut HashMap<String, String>) -> Result<Self> {
        let base = AbstractAnalysisFactory::new(args)?;
        let variant = AbstractAnalysisFactory::get(args, "variant");
        let flags = match variant.as_deref() {
            None | Some("nb") => BOKMAAL,
            Some("nn") => NYNORSK,
            Some("no") => BOKMAAL | NYNORSK,
            Some(variant) => {
                return Err(LuceneError::illegal_argument(format!(
                    "invalid variant: {variant}"
                )))
            }
        };
        check_unknown_parameters(args)?;
        Ok(Self { base, flags })
    }

    /// Default ctor for compatibility with SPI.
    ///
    /// Equivalent to `NorwegianLightStemFilterFactory()`.
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
    /// names it: `NorwegianLightStemFilterFactory.class`, with its `NAME` field and its
    /// `Map<String, String>` constructor.
    pub const fn class() -> AnalysisFactoryClass<dyn TokenFilterFactory> {
        AnalysisFactoryClass::new(
            "org.apache.lucene.analysis.no.NorwegianLightStemFilterFactory",
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

impl TokenFilterFactory for NorwegianLightStemFilterFactory {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.analysis.no.NorwegianLightStemFilterFactory"
    }

    /// Equivalent to `NorwegianLightStemFilterFactory.create(TokenStream)`, `new
    /// NorwegianLightStemFilter(input, flags)`.
    fn create(&self, input: Box<dyn TokenStream>) -> Result<Box<dyn TokenStream>> {
        Ok(Box::new(new_norwegian_light_stem_filter_with_flags(
            input, self.flags,
        )?))
    }
}
