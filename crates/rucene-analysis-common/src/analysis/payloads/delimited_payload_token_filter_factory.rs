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

//! Factory for `DelimitedPayloadTokenFilter`, ported from
//! `org.apache.lucene.analysis.payloads.DelimitedPayloadTokenFilterFactory`.

use std::collections::HashMap;
use std::sync::Arc;

use rucene::analysis::factories::{
    AbstractAnalysisFactory, AnalysisFactoryClass, SPINameField, TokenFilterFactory,
};
use rucene::analysis::TokenStream;
use rucene::error::Result;
use rucene::util::resource_loader::{ResourceLoader, ResourceLoaderAware};

use super::delimited_payload_token_filter::new_delimited_payload_token_filter;
use super::float_encoder::FloatEncoder;
use super::identity_encoder::IdentityEncoder;
use super::integer_encoder::IntegerEncoder;
use super::payload_encoder::PayloadEncoder;
use crate::spi::{check_unknown_parameters, payload_encoder_classes};

/// Factory for `DelimitedPayloadTokenFilter`.
///
/// ```text
/// <fieldType name="text_dlmtd" class="solr.TextField" positionIncrementGap="100">
///   <analyzer>
///     <tokenizer class="solr.WhitespaceTokenizerFactory"/>
///     <filter class="solr.DelimitedPayloadTokenFilterFactory" encoder="float" delimiter="|"/>
///   </analyzer>
/// </fieldType>
/// ```
///
/// Equivalent to `org.apache.lucene.analysis.payloads.DelimitedPayloadTokenFilterFactory`
/// (since 3.1). An encoder named by class is what
/// `ResourceLoader.newInstance(encoderClass, PayloadEncoder.class)` finds:
/// Rust has no runtime class loading, so the class is the constructor
/// [`payload_encoder_classes`] registers under that name, whose
/// [`ClassRegistry::new_instance`](rucene::util::resource_loader::ClassRegistry::new_instance)
/// reproduces `newInstance`'s failure.
#[derive(Debug)]
pub struct DelimitedPayloadTokenFilterFactory {
    base: AbstractAnalysisFactory,
    encoder_class: String,
    delimiter: u16,
    encoder: Option<Arc<dyn PayloadEncoder>>,
}

impl DelimitedPayloadTokenFilterFactory {
    /// SPI name.
    ///
    /// Equivalent to `DelimitedPayloadTokenFilterFactory.NAME`.
    pub const NAME: &'static str = "delimitedPayload";

    /// Equivalent to `DelimitedPayloadTokenFilterFactory.ENCODER_ATTR`.
    pub const ENCODER_ATTR: &'static str = "encoder";

    /// Equivalent to `DelimitedPayloadTokenFilterFactory.DELIMITER_ATTR`.
    pub const DELIMITER_ATTR: &'static str = "delimiter";

    /// Creates a new DelimitedPayloadTokenFilterFactory.
    ///
    /// Equivalent to `DelimitedPayloadTokenFilterFactory(Map<String, String>)`.
    ///
    /// # Errors
    ///
    /// Returns what [`AbstractAnalysisFactory::new`],
    /// [`AbstractAnalysisFactory::require`] and
    /// [`AbstractAnalysisFactory::get_char`] return, and
    /// `IllegalArgumentException` for a leftover argument.
    pub fn new(args: &mut HashMap<String, String>) -> Result<Self> {
        let base = AbstractAnalysisFactory::new(args)?;
        let encoder_class = AbstractAnalysisFactory::require(args, Self::ENCODER_ATTR)?;
        // `getChar` accepts one UTF-16 unit, a character of the BMP.
        let delimiter = AbstractAnalysisFactory::get_char(args, Self::DELIMITER_ATTR, '|')?;
        check_unknown_parameters(args)?;
        Ok(Self {
            base,
            encoder_class,
            delimiter: delimiter as u32 as u16,
            encoder: None,
        })
    }

    /// Default ctor for compatibility with SPI.
    ///
    /// Equivalent to `DelimitedPayloadTokenFilterFactory()`.
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
    /// names it: `DelimitedPayloadTokenFilterFactory.class`, with its `NAME`
    /// field and its `Map<String, String>` constructor.
    pub const fn class() -> AnalysisFactoryClass<dyn TokenFilterFactory> {
        AnalysisFactoryClass::new(
            "org.apache.lucene.analysis.payloads.DelimitedPayloadTokenFilterFactory",
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

impl ResourceLoaderAware for DelimitedPayloadTokenFilterFactory {
    /// Equivalent to `DelimitedPayloadTokenFilterFactory.inform(ResourceLoader)`.
    ///
    /// # Errors
    ///
    /// What `ResourceLoader.newInstance` throws for a class it cannot
    /// instantiate.
    fn inform(&mut self, _loader: &dyn ResourceLoader) -> Result<()> {
        self.encoder = Some(match self.encoder_class.as_str() {
            "float" => Arc::new(FloatEncoder::new()),
            "integer" => Arc::new(IntegerEncoder::new()),
            "identity" => Arc::new(IdentityEncoder::new()),
            encoder_class => payload_encoder_classes()
                .read()
                .expect("INVARIANT: no thread panics while holding the encoder class table")
                .new_instance(encoder_class)?,
        });
        Ok(())
    }
}

impl TokenFilterFactory for DelimitedPayloadTokenFilterFactory {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.analysis.payloads.DelimitedPayloadTokenFilterFactory"
    }

    /// Equivalent to `DelimitedPayloadTokenFilterFactory.create(TokenStream)`,
    /// `new DelimitedPayloadTokenFilter(input, delimiter, encoder)`.
    ///
    /// # Errors
    ///
    /// Java's `NullPointerException` "encoder" when the factory was not
    /// informed, and what the filter's construction returns.
    fn create(&self, input: Box<dyn TokenStream>) -> Result<Box<dyn TokenStream>> {
        let Some(encoder) = &self.encoder else {
            return Err(rucene::error::LuceneError::null_pointer_with_message(
                "encoder",
            ));
        };
        Ok(Box::new(new_delimited_payload_token_filter(
            input,
            self.delimiter,
            Arc::clone(encoder),
        )?))
    }

    fn as_resource_loader_aware(&mut self) -> Option<&mut dyn ResourceLoaderAware> {
        Some(self)
    }
}
