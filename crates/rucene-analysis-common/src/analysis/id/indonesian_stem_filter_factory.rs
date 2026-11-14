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

//! Factory for `IndonesianStemFilter`, ported from
//! `org.apache.lucene.analysis.id.IndonesianStemFilterFactory`.

use std::collections::HashMap;
use std::sync::Arc;

use rucene::analysis::factories::{
    AbstractAnalysisFactory, AnalysisFactoryClass, SPINameField, TokenFilterFactory,
};
use rucene::analysis::TokenStream;
use rucene::error::Result;

use super::indonesian_stem_filter::new_indonesian_stem_filter_with_derivational;
use crate::spi::check_unknown_parameters;

/// Factory for `IndonesianStemFilter`.
///
/// ```text
/// <fieldType name="text_idstem" class="solr.TextField" positionIncrementGap="100">
///   <analyzer>
///     <tokenizer class="solr.StandardTokenizerFactory"/>
///     <filter class="solr.LowerCaseFilterFactory"/>
///     <filter class="solr.IndonesianStemFilterFactory" stemDerivational="true"/>
///   </analyzer>
/// </fieldType>
/// ```
///
/// Equivalent to `org.apache.lucene.analysis.id.IndonesianStemFilterFactory` (since 3.1.0).
#[derive(Debug)]
pub struct IndonesianStemFilterFactory {
    base: AbstractAnalysisFactory,
    stem_derivational: bool,
}

impl IndonesianStemFilterFactory {
    /// SPI name.
    ///
    /// Equivalent to `IndonesianStemFilterFactory.NAME`.
    pub const NAME: &'static str = "indonesianStem";

    /// Creates a new IndonesianStemFilterFactory
    ///
    /// Equivalent to `IndonesianStemFilterFactory(Map<String, String>)`:
    /// `stemDerivational` defaults to `true`.
    ///
    /// # Errors
    ///
    /// Returns what [`AbstractAnalysisFactory::new`] returns, and `IllegalArgumentException` for a leftover argument.
    pub fn new(args: &mut HashMap<String, String>) -> Result<Self> {
        let base = AbstractAnalysisFactory::new(args)?;
        let stem_derivational =
            AbstractAnalysisFactory::get_boolean(args, "stemDerivational", true);
        check_unknown_parameters(args)?;
        Ok(Self {
            base,
            stem_derivational,
        })
    }

    /// Default ctor for compatibility with SPI.
    ///
    /// Equivalent to `IndonesianStemFilterFactory()`.
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
    /// names it: `IndonesianStemFilterFactory.class`, with its `NAME` field and its
    /// `Map<String, String>` constructor.
    pub const fn class() -> AnalysisFactoryClass<dyn TokenFilterFactory> {
        AnalysisFactoryClass::new(
            "org.apache.lucene.analysis.id.IndonesianStemFilterFactory",
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

impl TokenFilterFactory for IndonesianStemFilterFactory {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.analysis.id.IndonesianStemFilterFactory"
    }

    /// Equivalent to `IndonesianStemFilterFactory.create(TokenStream)`, `new
    /// IndonesianStemFilter(input, stemDerivational)`.
    fn create(&self, input: Box<dyn TokenStream>) -> Result<Box<dyn TokenStream>> {
        Ok(Box::new(new_indonesian_stem_filter_with_derivational(
            input,
            self.stem_derivational,
        )?))
    }
}
