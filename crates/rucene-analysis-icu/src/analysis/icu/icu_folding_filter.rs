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

//! A `TokenFilter` that applies search term folding to Unicode text,
//! applying foldings from UTR#30 Character Foldings, ported from
//! `org.apache.lucene.analysis.icu.ICUFoldingFilter`.

use std::sync::{Arc, LazyLock};

use icu4j::text::normalizer2::{self, Mode, Normalizer2};
use rucene::analysis::TokenStream;
use rucene::error::Result;
use rucene::store::java_nio_file::InputStream;
use rucene::util::attribute::{AttributeReflector, AttributeSource, Unwrappable};
use rucene::util::IOUtils;

use super::icu_normalizer2_filter::{new_icu_normalizer2_filter_with, ICUNormalizer2Filter};
use crate::resources::get_resource_as_stream;

/// A normalizer for search term folding to Unicode text, applying foldings
/// from UTR#30 Character Foldings.
///
/// Equivalent to the static field `ICUFoldingFilter.NORMALIZER`,
/// `Normalizer2.getInstance(ICUFoldingFilter.class.getResourceAsStream("utr30.nrm"),
/// "utr30", Normalizer2.Mode.COMPOSE)`, initialized on first use as the
/// class's static initializer runs on its first use.
///
/// # Panics
///
/// On first use, when the data cannot be loaded: Java's static initializer
/// then fails with an `ExceptionInInitializerError`. The data is the
/// embedded `utr30.nrm` of Lucene 10.5.0.
// TODO: if the wrong version of the ICU jar is used, loading these data files may give a
// strange error.
// maybe add an explicit check?
// http://icu-project.org/apiref/icu4j/com/ibm/icu/util/VersionInfo.html
pub static NORMALIZER: LazyLock<Arc<dyn Normalizer2>> = LazyLock::new(|| {
    let mut stream = get_resource_as_stream(
        "org.apache.lucene.analysis.icu.ICUFoldingFilter",
        "utr30.nrm",
    );
    let data = stream.as_mut().map(|s| s as &mut dyn InputStream);
    let normalizer: Arc<dyn Normalizer2> = normalizer2::get_instance(data, "utr30", Mode::Compose)
        .unwrap_or_else(|e| IOUtils::rethrow_always(e));
    normalizer
});

/// A `TokenFilter` that applies search term folding to Unicode text,
/// applying foldings from UTR#30 Character Foldings.
///
/// This filter applies the following foldings from the report to unicode
/// text:
///
/// * Accent removal
/// * Case folding
/// * Canonical duplicates folding
/// * Dashes folding
/// * Diacritic removal (including stroke, hook, descender)
/// * Greek letterforms folding
/// * Han Radical folding
/// * Hebrew Alternates folding
/// * Jamo folding
/// * Letterforms folding
/// * Math symbol folding
/// * Multigraph Expansions: All
/// * Native digit folding
/// * No-break folding
/// * Overline folding
/// * Positional forms folding
/// * Small forms folding
/// * Space folding
/// * Spacing Accents folding
/// * Subscript folding
/// * Superscript folding
/// * Suzhou Numeral folding
/// * Symbol folding
/// * Underline folding
/// * Vertical forms folding
/// * Width folding
///
/// Additionally, Default Ignorables are removed, and text is normalized to
/// NFKC. All foldings, case folding, and normalization mappings are applied
/// recursively to ensure a fully folded and normalized result.
///
/// A normalizer with additional settings such as a filter that lists
/// characters not to be normalized can be passed in the constructor.
///
/// Equivalent to `org.apache.lucene.analysis.icu.ICUFoldingFilter`, a final
/// subclass of `ICUNormalizer2Filter` that adds no state: the filter is the
/// [`ICUNormalizer2Filter`] it holds, every `TokenStream` method delegated
/// to it.
#[derive(Debug)]
pub struct ICUFoldingFilter {
    inner: ICUNormalizer2Filter,
}

impl ICUFoldingFilter {
    /// Create a new ICUFoldingFilter on the specified input.
    ///
    /// Equivalent to `ICUFoldingFilter(TokenStream)`, `super(input,
    /// NORMALIZER)`.
    ///
    /// # Errors
    ///
    /// What [`new_icu_normalizer2_filter_with`] raises.
    pub fn new(input: Box<dyn TokenStream>) -> Result<Self> {
        Self::with_normalizer(input, Arc::clone(&NORMALIZER))
    }

    /// Create a new ICUFoldingFilter on the specified input with the
    /// specified normalizer.
    ///
    /// Equivalent to `ICUFoldingFilter(TokenStream, Normalizer2)`.
    ///
    /// # Errors
    ///
    /// What [`new_icu_normalizer2_filter_with`] raises.
    pub fn with_normalizer(
        input: Box<dyn TokenStream>,
        normalizer: Arc<dyn Normalizer2>,
    ) -> Result<Self> {
        Ok(Self {
            inner: new_icu_normalizer2_filter_with(input, normalizer)?,
        })
    }

    /// The `ICUNormalizer2Filter` this filter is.
    pub fn as_icu_normalizer2_filter(&self) -> &ICUNormalizer2Filter {
        &self.inner
    }
}

impl TokenStream for ICUFoldingFilter {
    /// The inherited final `ICUNormalizer2Filter.incrementToken()`.
    fn increment_token(&mut self) -> Result<bool> {
        self.inner.increment_token()
    }

    fn end(&mut self) -> Result<()> {
        self.inner.end()
    }

    fn reset(&mut self) -> Result<()> {
        self.inner.reset()
    }

    fn close(&mut self) -> Result<()> {
        self.inner.close()
    }

    fn attribute_source(&self) -> &AttributeSource {
        self.inner.attribute_source()
    }

    fn attribute_source_mut(&mut self) -> &mut AttributeSource {
        self.inner.attribute_source_mut()
    }

    fn reflect_with(&self, reflector: &mut dyn AttributeReflector) {
        self.inner.reflect_with(reflector)
    }

    fn as_unwrappable(&self) -> Option<&dyn Unwrappable<dyn TokenStream>> {
        self.inner.as_unwrappable()
    }
}
