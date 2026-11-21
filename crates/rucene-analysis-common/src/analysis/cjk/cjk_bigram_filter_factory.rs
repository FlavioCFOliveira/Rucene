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

//! Factory for `CJKBigramFilter`, ported from
//! `org.apache.lucene.analysis.cjk.CJKBigramFilterFactory`.

use std::collections::HashMap;
use std::sync::Arc;

use rucene::analysis::factories::{
    AbstractAnalysisFactory, AnalysisFactoryClass, SPINameField, TokenFilterFactory,
};
use rucene::analysis::TokenStream;
use rucene::error::Result;

use super::cjk_bigram_filter::{
    new_cjk_bigram_filter_with_unigrams, HAN, HANGUL, HIRAGANA, KATAKANA,
};
use crate::spi::check_unknown_parameters;

/// Factory for [`CJKBigramFilter`](super::cjk_bigram_filter::CJKBigramFilter).
///
/// ```xml
/// <fieldType name="text_cjk" class="solr.TextField">
///   <analyzer>
///     <tokenizer class="solr.StandardTokenizerFactory"/>
///     <filter class="solr.CJKWidthFilterFactory"/>
///     <filter class="solr.LowerCaseFilterFactory"/>
///     <filter class="solr.CJKBigramFilterFactory"
///       han="true" hiragana="true"
///       katakana="true" hangul="true" outputUnigrams="false" />
///   </analyzer>
/// </fieldType>
/// ```
///
/// Equivalent to `org.apache.lucene.analysis.cjk.CJKBigramFilterFactory`
/// (since 3.6.0, SPI name `cjkBigram`).
#[derive(Debug)]
pub struct CJKBigramFilterFactory {
    base: AbstractAnalysisFactory,
    pub(crate) flags: i32,
    pub(crate) output_unigrams: bool,
}

impl CJKBigramFilterFactory {
    /// SPI name: `CJKBigramFilterFactory.NAME`.
    pub const NAME: &'static str = "cjkBigram";

    /// Creates a new CJKBigramFilterFactory
    ///
    /// Equivalent to `CJKBigramFilterFactory(Map<String, String>)`.
    ///
    /// # Errors
    ///
    /// What `AbstractAnalysisFactory` raises, and
    /// `IllegalArgumentException("Unknown parameters: ...")` for arguments
    /// left over.
    pub fn new(args: &mut HashMap<String, String>) -> Result<Self> {
        let base = AbstractAnalysisFactory::new(args)?;
        let mut flags = 0;
        if AbstractAnalysisFactory::get_boolean(args, "han", true) {
            flags |= HAN;
        }
        if AbstractAnalysisFactory::get_boolean(args, "hiragana", true) {
            flags |= HIRAGANA;
        }
        if AbstractAnalysisFactory::get_boolean(args, "katakana", true) {
            flags |= KATAKANA;
        }
        if AbstractAnalysisFactory::get_boolean(args, "hangul", true) {
            flags |= HANGUL;
        }
        let output_unigrams = AbstractAnalysisFactory::get_boolean(args, "outputUnigrams", false);
        check_unknown_parameters(args)?;
        Ok(Self {
            base,
            flags,
            output_unigrams,
        })
    }

    /// Default ctor for compatibility with SPI
    ///
    /// Equivalent to `CJKBigramFilterFactory()`.
    ///
    /// # Errors
    ///
    /// Always: `defaultCtorException()`.
    pub fn new_without_args() -> Result<Self> {
        Err(AbstractAnalysisFactory::default_ctor_exception())
    }

    /// The inherited `AbstractAnalysisFactory` state.
    pub fn base(&self) -> &AbstractAnalysisFactory {
        &self.base
    }

    /// The class of this factory, as the SPI loader registers it.
    pub const fn class() -> AnalysisFactoryClass<dyn TokenFilterFactory> {
        AnalysisFactoryClass::new(
            "org.apache.lucene.analysis.cjk.CJKBigramFilterFactory",
            SPINameField::Declared(Self::NAME),
            Some(Self::new_from_map),
        )
    }

    fn new_from_map(args: &mut HashMap<String, String>) -> Result<Arc<dyn TokenFilterFactory>> {
        Ok(Arc::new(Self::new(args)?))
    }
}

impl TokenFilterFactory for CJKBigramFilterFactory {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.analysis.cjk.CJKBigramFilterFactory"
    }

    /// Equivalent to `CJKBigramFilterFactory.create(TokenStream)`.
    fn create(&self, input: Box<dyn TokenStream>) -> Result<Box<dyn TokenStream>> {
        Ok(Box::new(new_cjk_bigram_filter_with_unigrams(
            input,
            self.flags,
            self.output_unigrams,
        )?))
    }
}
