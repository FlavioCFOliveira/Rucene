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

//! Factory for `ICUTransformFilter`, ported from
//! `org.apache.lucene.analysis.icu.ICUTransformFilterFactory`.

use std::collections::HashMap;
use std::sync::Arc;

use icu4j::text::transliterator::{self, Transliterator, FORWARD, REVERSE};
use rucene::analysis::factories::{
    AbstractAnalysisFactory, AnalysisFactoryClass, SPINameField, TokenFilterFactory,
};
use rucene::analysis::TokenStream;
use rucene::error::Result;

use super::icu_transform_filter::new_icu_transform_filter;
use crate::spi::check_unknown_parameters;

/// Factory for `ICUTransformFilter`.
///
/// Supports the following attributes:
///
/// * id (mandatory): A Transliterator ID, one from
///   `Transliterator.getAvailableIDs()`
/// * direction (optional): Either 'forward' or 'reverse'. Default is
///   forward.
///
/// See `Transliterator`.
///
/// Equivalent to `org.apache.lucene.analysis.icu.ICUTransformFilterFactory`
/// (since 3.1.0, `@lucene.spi icuTransform`).
#[derive(Debug)]
pub struct ICUTransformFilterFactory {
    base: AbstractAnalysisFactory,
    transliterator: Arc<dyn Transliterator>,
}

impl ICUTransformFilterFactory {
    /// SPI name.
    ///
    /// Equivalent to `ICUTransformFilterFactory.NAME`.
    pub const NAME: &'static str = "icuTransform";

    // TODO: add support for custom rules
    /// Creates a new ICUTransformFilterFactory.
    ///
    /// Equivalent to `ICUTransformFilterFactory(Map<String, String>)`: the
    /// `direction` is checked case-insensitively, and then compared with
    /// `"forward".equals(direction)`, so that any other spelling of
    /// `forward` selects `REVERSE`, as in Java.
    ///
    /// # Errors
    ///
    /// Returns what [`AbstractAnalysisFactory::new`] returns; the
    /// `IllegalArgumentException` of a missing `id` or of a `direction`
    /// outside `forward`/`reverse`; the errors of
    /// `Transliterator.getInstance(id, dir)`; and `IllegalArgumentException`
    /// for a leftover argument.
    pub fn new(args: &mut HashMap<String, String>) -> Result<Self> {
        let base = AbstractAnalysisFactory::new(args)?;
        let id = AbstractAnalysisFactory::require(args, "id")?;
        let direction = AbstractAnalysisFactory::get_one_of(
            args,
            "direction",
            &["forward", "reverse"],
            Some("forward"),
            false,
        )?
        .expect("INVARIANT: the argument has a default value");
        let dir = if "forward" == direction { FORWARD } else { REVERSE };
        let id_units: Vec<u16> = id.encode_utf16().collect();
        let transliterator = transliterator::get_instance_dir(&id_units, dir)?;
        check_unknown_parameters(args)?;
        Ok(Self {
            base,
            transliterator,
        })
    }

    /// Default ctor for compatibility with SPI.
    ///
    /// Equivalent to `ICUTransformFilterFactory()`.
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

    /// The transliterator the filters of this factory apply (Java's private
    /// field `transliterator`).
    pub fn transliterator(&self) -> &Arc<dyn Transliterator> {
        &self.transliterator
    }

    /// The factory's class, as `META-INF/services/org.apache.lucene.analysis.TokenFilterFactory`
    /// names it: `ICUTransformFilterFactory.class`, with its `NAME` field and
    /// its `Map<String, String>` constructor.
    pub const fn class() -> AnalysisFactoryClass<dyn TokenFilterFactory> {
        AnalysisFactoryClass::new(
            "org.apache.lucene.analysis.icu.ICUTransformFilterFactory",
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

impl TokenFilterFactory for ICUTransformFilterFactory {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.analysis.icu.ICUTransformFilterFactory"
    }

    /// Equivalent to `ICUTransformFilterFactory.create(TokenStream)`, `new
    /// ICUTransformFilter(input, transliterator)`.
    fn create(&self, input: Box<dyn TokenStream>) -> Result<Box<dyn TokenStream>> {
        Ok(Box::new(new_icu_transform_filter(
            input,
            Arc::clone(&self.transliterator),
        )?))
    }

    /// Equivalent to `ICUTransformFilterFactory.normalize(TokenStream)`,
    /// `create(input)`.
    fn normalize(&self, input: Box<dyn TokenStream>) -> Result<Box<dyn TokenStream>> {
        self.create(input)
    }
}
