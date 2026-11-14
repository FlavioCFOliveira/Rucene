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

//! Factory for `DelimitedBoostTokenFilter`, ported from
//! `org.apache.lucene.analysis.boost.DelimitedBoostTokenFilterFactory`.

use std::collections::HashMap;
use std::sync::Arc;

use rucene::analysis::factories::{
    AbstractAnalysisFactory, AnalysisFactoryClass, SPINameField, TokenFilterFactory,
};
use rucene::analysis::TokenStream;
use rucene::error::Result;

use super::delimited_boost_token_filter::new_delimited_boost_token_filter;
use crate::spi::check_unknown_parameters;

/// Factory for `DelimitedBoostTokenFilter`.
///
/// ```text
/// <fieldType name="text_dlmtd" class="solr.TextField" positionIncrementGap="100">
///   <analyzer>
///     <tokenizer class="solr.WhitespaceTokenizerFactory"/>
///     <filter class="solr.DelimitedBoostTokenFilterFactory" delimiter="|"/>
///   </analyzer>
/// </fieldType>
/// ```
///
/// Equivalent to `org.apache.lucene.analysis.boost.DelimitedBoostTokenFilterFactory`.
#[derive(Debug)]
pub struct DelimitedBoostTokenFilterFactory {
    base: AbstractAnalysisFactory,
    delimiter: u16,
}

impl DelimitedBoostTokenFilterFactory {
    /// SPI name.
    ///
    /// Equivalent to `DelimitedBoostTokenFilterFactory.NAME`.
    pub const NAME: &'static str = "delimitedBoost";

    /// Equivalent to `DelimitedBoostTokenFilterFactory.DELIMITER_ATTR`.
    pub const DELIMITER_ATTR: &'static str = "delimiter";

    /// Equivalent to `DelimitedBoostTokenFilterFactory.DEFAULT_DELIMITER`.
    pub const DEFAULT_DELIMITER: u16 = b'|' as u16;

    /// Creates a new DelimitedPayloadTokenFilterFactory
    ///
    /// Equivalent to `DelimitedBoostTokenFilterFactory(Map<String, String>)`.
    ///
    /// # Errors
    ///
    /// Returns what [`AbstractAnalysisFactory::new`] and
    /// [`AbstractAnalysisFactory::get_char`] return, and
    /// `IllegalArgumentException` for a leftover argument.
    pub fn new(args: &mut HashMap<String, String>) -> Result<Self> {
        let base = AbstractAnalysisFactory::new(args)?;
        // `getChar` accepts one UTF-16 unit, a character of the BMP.
        let delimiter = AbstractAnalysisFactory::get_char(args, Self::DELIMITER_ATTR, '|')?;
        check_unknown_parameters(args)?;
        Ok(Self {
            base,
            delimiter: delimiter as u32 as u16,
        })
    }

    /// Default ctor for compatibility with SPI.
    ///
    /// Equivalent to `DelimitedBoostTokenFilterFactory()`.
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
    /// names it: `DelimitedBoostTokenFilterFactory.class`, with its `NAME`
    /// field and its `Map<String, String>` constructor.
    pub const fn class() -> AnalysisFactoryClass<dyn TokenFilterFactory> {
        AnalysisFactoryClass::new(
            "org.apache.lucene.analysis.boost.DelimitedBoostTokenFilterFactory",
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

impl TokenFilterFactory for DelimitedBoostTokenFilterFactory {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.analysis.boost.DelimitedBoostTokenFilterFactory"
    }

    /// Equivalent to `DelimitedBoostTokenFilterFactory.create(TokenStream)`,
    /// `new DelimitedBoostTokenFilter(input, delimiter)`.
    fn create(&self, input: Box<dyn TokenStream>) -> Result<Box<dyn TokenStream>> {
        Ok(Box::new(new_delimited_boost_token_filter(
            input,
            self.delimiter,
        )?))
    }
}
