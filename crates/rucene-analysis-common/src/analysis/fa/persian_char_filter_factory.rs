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

//! Factory for `PersianCharFilter`, ported from
//! `org.apache.lucene.analysis.fa.PersianCharFilterFactory`.

use std::collections::HashMap;
use std::sync::Arc;

use rucene::analysis::factories::{
    AbstractAnalysisFactory, AnalysisFactoryClass, CharFilterFactory, SPINameField,
};
use rucene::analysis::CharReader;
use rucene::error::Result;

use super::persian_char_filter::PersianCharFilter;
use crate::spi::check_unknown_parameters;

/// Factory for `PersianCharFilter`.
///
/// ```text
/// <fieldType name="text_fa" class="solr.TextField" positionIncrementGap="100">
///   <analyzer>
///     <charFilter class="solr.PersianCharFilterFactory"/>
///     <tokenizer class="solr.StandardTokenizerFactory"/>
///   </analyzer>
/// </fieldType>
/// ```
///
/// Equivalent to `org.apache.lucene.analysis.fa.PersianCharFilterFactory` (since 3.1).
#[derive(Debug)]
pub struct PersianCharFilterFactory {
    base: AbstractAnalysisFactory,
}

impl PersianCharFilterFactory {
    /// SPI name.
    ///
    /// Equivalent to `PersianCharFilterFactory.NAME`.
    pub const NAME: &'static str = "persian";

    /// Creates a new PersianCharFilterFactory
    ///
    /// Equivalent to `PersianCharFilterFactory(Map<String, String>)`.
    ///
    /// # Errors
    ///
    /// Returns what [`AbstractAnalysisFactory::new`] returns, and `IllegalArgumentException` for a leftover argument.
    pub fn new(args: &mut HashMap<String, String>) -> Result<Self> {
        let base = AbstractAnalysisFactory::new(args)?;
        check_unknown_parameters(args)?;
        Ok(Self { base })
    }

    /// Default ctor for compatibility with SPI.
    ///
    /// Equivalent to `PersianCharFilterFactory()`.
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

    /// The factory's class, as `META-INF/services/org.apache.lucene.analysis.CharFilterFactory`
    /// names it: `PersianCharFilterFactory.class`, with its `NAME` field and its
    /// `Map<String, String>` constructor.
    pub const fn class() -> AnalysisFactoryClass<dyn CharFilterFactory> {
        AnalysisFactoryClass::new(
            "org.apache.lucene.analysis.fa.PersianCharFilterFactory",
            SPINameField::Declared(Self::NAME),
            Some(Self::new_from_map),
        )
    }

    /// The `Map<String, String>` constructor, as [`class`](Self::class)
    /// exposes it.
    fn new_from_map(args: &mut HashMap<String, String>) -> Result<Arc<dyn CharFilterFactory>> {
        Ok(Arc::new(Self::new(args)?))
    }
}

impl CharFilterFactory for PersianCharFilterFactory {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.analysis.fa.PersianCharFilterFactory"
    }

    /// Equivalent to `PersianCharFilterFactory.create(Reader)`, `new PersianCharFilter(input)`.
    fn create(&self, input: Box<dyn CharReader>) -> Result<Box<dyn CharReader>> {
        Ok(Box::new(PersianCharFilter::new(input)))
    }

    /// Equivalent to `PersianCharFilterFactory.normalize(Reader)`, `create(input)`.
    fn normalize(&self, input: Box<dyn CharReader>) -> Result<Box<dyn CharReader>> {
        self.create(input)
    }
}
