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

//! Factory for `ThaiTokenizer`, ported from
//! `org.apache.lucene.analysis.th.ThaiTokenizerFactory`.

use std::collections::HashMap;
use std::sync::Arc;

use rucene::analysis::factories::{
    AbstractAnalysisFactory, AnalysisFactoryClass, FactoryTokenizer, SPINameField, TokenizerFactory,
};
use rucene::analysis::{Tokenizer, TokenizerLogic};
use rucene::error::Result;
use rucene::util::attribute::AttributeFactory;

use super::thai_tokenizer::thai_tokenizer_parts;
use crate::spi::check_unknown_parameters;

/// Factory for [`ThaiTokenizer`](super::thai_tokenizer::ThaiTokenizer).
///
/// ```xml
/// <fieldType name="text_thai" class="solr.TextField" positionIncrementGap="100">
///   <analyzer>
///     <tokenizer class="solr.ThaiTokenizerFactory"/>
///   </analyzer>
/// </fieldType>
/// ```
///
/// Equivalent to `org.apache.lucene.analysis.th.ThaiTokenizerFactory`
/// (since 4.10.0, SPI name `thai`).
#[derive(Debug)]
pub struct ThaiTokenizerFactory {
    base: AbstractAnalysisFactory,
}

impl ThaiTokenizerFactory {
    /// SPI name: `ThaiTokenizerFactory.NAME`.
    pub const NAME: &'static str = "thai";

    /// Creates a new ThaiTokenizerFactory
    ///
    /// Equivalent to `ThaiTokenizerFactory(Map<String, String>)`.
    ///
    /// # Errors
    ///
    /// What `AbstractAnalysisFactory` raises, and
    /// `IllegalArgumentException("Unknown parameters: ...")` for arguments
    /// left over.
    pub fn new(args: &mut HashMap<String, String>) -> Result<Self> {
        let base = AbstractAnalysisFactory::new(args)?;
        check_unknown_parameters(args)?;
        Ok(Self { base })
    }

    /// Default ctor for compatibility with SPI
    ///
    /// Equivalent to `ThaiTokenizerFactory()`.
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
    pub const fn class() -> AnalysisFactoryClass<dyn TokenizerFactory> {
        AnalysisFactoryClass::new(
            "org.apache.lucene.analysis.th.ThaiTokenizerFactory",
            SPINameField::Declared(Self::NAME),
            Some(Self::new_from_map),
        )
    }

    fn new_from_map(args: &mut HashMap<String, String>) -> Result<Arc<dyn TokenizerFactory>> {
        Ok(Arc::new(Self::new(args)?))
    }
}

impl TokenizerFactory for ThaiTokenizerFactory {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.analysis.th.ThaiTokenizerFactory"
    }

    /// Equivalent to `ThaiTokenizerFactory.create(AttributeFactory)`, `new
    /// ThaiTokenizer(factory)`.
    fn create_with_factory(&self, factory: Arc<dyn AttributeFactory>) -> Result<FactoryTokenizer> {
        let (source, adapter) = thai_tokenizer_parts(factory)?;
        let logic: Box<dyn TokenizerLogic> = Box::new(adapter);
        Ok(Tokenizer::with_source(source, logic))
    }
}
