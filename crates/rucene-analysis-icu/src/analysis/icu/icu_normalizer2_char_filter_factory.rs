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

//! Factory for `ICUNormalizer2CharFilter`, ported from
//! `org.apache.lucene.analysis.icu.ICUNormalizer2CharFilterFactory`.

use std::collections::HashMap;
use std::sync::Arc;

use icu4j::text::filtered_normalizer2::FilteredNormalizer2;
use icu4j::text::normalizer2::{self, Mode, Normalizer2};
use icu4j::text::unicode_set::UnicodeSet;
use rucene::analysis::factories::{
    AbstractAnalysisFactory, AnalysisFactoryClass, CharFilterFactory, SPINameField,
};
use rucene::analysis::CharReader;
use rucene::error::Result;

use super::icu_normalizer2_char_filter::ICUNormalizer2CharFilter;
use crate::spi::check_unknown_parameters;

/// Factory for `ICUNormalizer2CharFilter`.
///
/// Supports the following attributes:
///
/// * form: A [Unicode Normalization Form](http://unicode.org/reports/tr15/),
///   one of 'nfc','nfkc', 'nfkc_cf'. Default is nfkc_cf.
/// * mode: Either 'compose' or 'decompose'. Default is compose. Use
///   "decompose" with nfc or nfkc, to get nfd or nfkd, respectively.
/// * filter: A `UnicodeSet` pattern. Codepoints outside the set are always
///   left unchanged. Default is [] (the null set, no filtering).
///
/// See `ICUNormalizer2CharFilter`, `Normalizer2` and `FilteredNormalizer2`.
///
/// Equivalent to `org.apache.lucene.analysis.icu.ICUNormalizer2CharFilterFactory`
/// (since 4.10.0, `@lucene.spi icuNormalizer2`).
#[derive(Debug)]
pub struct ICUNormalizer2CharFilterFactory {
    base: AbstractAnalysisFactory,
    normalizer: Arc<dyn Normalizer2>,
}

impl ICUNormalizer2CharFilterFactory {
    /// SPI name.
    ///
    /// Equivalent to `ICUNormalizer2CharFilterFactory.NAME`.
    pub const NAME: &'static str = "icuNormalizer2";

    /// Creates a new ICUNormalizer2CharFilterFactory.
    ///
    /// Equivalent to `ICUNormalizer2CharFilterFactory(Map<String, String>)`.
    ///
    /// # Errors
    ///
    /// Returns what [`AbstractAnalysisFactory::new`] returns; the
    /// `IllegalArgumentException` of a `mode` outside `compose`/`decompose`;
    /// the errors of `Normalizer2.getInstance` for the `form` (a missing
    /// data file); the `IllegalArgumentException` of a malformed `filter`
    /// pattern; and `IllegalArgumentException` for a leftover argument.
    pub fn new(args: &mut HashMap<String, String>) -> Result<Self> {
        let base = AbstractAnalysisFactory::new(args)?;
        let form = AbstractAnalysisFactory::get_or(args, "form", "nfkc_cf");
        let mode = AbstractAnalysisFactory::get_one_of(
            args,
            "mode",
            &["compose", "decompose"],
            Some("compose"),
            true,
        )?
        .expect("INVARIANT: the argument has a default value");
        let mut normalizer: Arc<dyn Normalizer2> = normalizer2::get_instance(
            None,
            &form,
            if "compose" == mode {
                Mode::Compose
            } else {
                Mode::Decompose
            },
        )?;

        let filter = AbstractAnalysisFactory::get(args, "filter");
        if let Some(filter) = filter {
            let units: Vec<u16> = filter.encode_utf16().collect();
            let mut set = UnicodeSet::from_pattern(&units)?;
            if !set.is_empty() {
                set.freeze();
                normalizer = Arc::new(FilteredNormalizer2::new(normalizer, Arc::new(set)));
            }
        }
        check_unknown_parameters(args)?;
        Ok(Self { base, normalizer })
    }

    /// Default ctor for compatibility with SPI.
    ///
    /// Equivalent to `ICUNormalizer2CharFilterFactory()`.
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

    /// The normalizer the char filters of this factory apply (Java's
    /// private field `normalizer`).
    pub fn normalizer(&self) -> &Arc<dyn Normalizer2> {
        &self.normalizer
    }

    /// The factory's class, as `META-INF/services/org.apache.lucene.analysis.CharFilterFactory`
    /// names it: `ICUNormalizer2CharFilterFactory.class`, with its `NAME`
    /// field and its `Map<String, String>` constructor.
    pub const fn class() -> AnalysisFactoryClass<dyn CharFilterFactory> {
        AnalysisFactoryClass::new(
            "org.apache.lucene.analysis.icu.ICUNormalizer2CharFilterFactory",
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

impl CharFilterFactory for ICUNormalizer2CharFilterFactory {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.analysis.icu.ICUNormalizer2CharFilterFactory"
    }

    /// Equivalent to `ICUNormalizer2CharFilterFactory.create(Reader)`, `new
    /// ICUNormalizer2CharFilter(input, normalizer)`.
    fn create(&self, input: Box<dyn CharReader>) -> Result<Box<dyn CharReader>> {
        Ok(Box::new(ICUNormalizer2CharFilter::with_normalizer(
            input,
            Arc::clone(&self.normalizer),
        )?))
    }

    /// Equivalent to `ICUNormalizer2CharFilterFactory.normalize(Reader)`,
    /// `create(input)`.
    fn normalize(&self, input: Box<dyn CharReader>) -> Result<Box<dyn CharReader>> {
        self.create(input)
    }
}
