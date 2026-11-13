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

//! An analyzer wrapper that adds a `ShingleFilter` to another analyzer,
//! ported from `org.apache.lucene.analysis.shingle.ShingleAnalyzerWrapper`.

use std::sync::Arc;

use rucene::analysis::{
    Analyzer, AnalyzerState, AnalyzerWrapper, AnalyzerWrapperLogic, CharReader, SharedTokenStream,
    StandardAnalyzer, TokenStream, TokenStreamComponents,
};
use rucene::error::{LuceneError, Result};
use rucene::util::atomic_ref_cell::AtomicRefCell;
use rucene::util::attribute::AttributeFactory;

use super::shingle_filter::{
    new_shingle_filter, DEFAULT_FILLER_TOKEN, DEFAULT_MAX_SHINGLE_SIZE, DEFAULT_MIN_SHINGLE_SIZE,
    DEFAULT_TOKEN_SEPARATOR,
};
use crate::analysis::miscellaneous::limit_token_count_analyzer::DelegateReuseStrategy;

/// What [`ShingleAnalyzerWrapper`] overrides of `AnalyzerWrapper`.
#[derive(Debug)]
struct ShingleAnalyzerWrapperLogic {
    delegate: Arc<dyn Analyzer>,
    max_shingle_size: i32,
    min_shingle_size: i32,
    token_separator: String,
    output_unigrams: bool,
    output_unigrams_if_no_shingles: bool,
    filler_token: Option<String>,
}

impl AnalyzerWrapperLogic for ShingleAnalyzerWrapperLogic {
    /// Equivalent to the final `ShingleAnalyzerWrapper.getWrappedAnalyzer(String)`.
    fn get_wrapped_analyzer(&self, _field_name: &str) -> &dyn Analyzer {
        self.delegate.as_ref()
    }

    /// Equivalent to `ShingleAnalyzerWrapper.wrapComponents(String,
    /// TokenStreamComponents)`: the delegate's source, and a configured
    /// `ShingleFilter` over its token stream.
    ///
    /// # Panics
    ///
    /// With the exception `ShingleFilter`'s construction throws (the hook
    /// returns no error, and Java's exceptions there are unchecked); the
    /// shingle sizes were checked by the constructor, so only an attribute
    /// the stream's factory cannot implement reaches it.
    fn wrap_components(
        &self,
        _field_name: &str,
        components: TokenStreamComponents,
    ) -> TokenStreamComponents {
        let input: Box<dyn TokenStream> =
            Box::new(SharedTokenStream::new(components.token_stream()));
        let built = new_shingle_filter(input, self.min_shingle_size, self.max_shingle_size)
            .and_then(|mut filter| {
                let logic = filter.logic_mut();
                logic.set_min_shingle_size(self.min_shingle_size)?;
                logic.set_max_shingle_size(self.max_shingle_size)?;
                logic.set_token_separator(Some(&self.token_separator));
                logic.set_output_unigrams(self.output_unigrams);
                logic.set_output_unigrams_if_no_shingles(self.output_unigrams_if_no_shingles);
                logic.set_filler_token(self.filler_token.as_deref());
                Ok(filter)
            });
        match built {
            Ok(filter) => components.with_sink(Arc::new(AtomicRefCell::new(filter))),
            Err(error) => rucene::store::java_nio_file::throw_unchecked(
                error.java_simple_name(),
                Some(&error.to_string()),
            ),
        }
    }
}

/// A ShingleAnalyzerWrapper wraps a `ShingleFilter` around another
/// `Analyzer`.
///
/// A shingle is another name for a token based n-gram.
///
/// Equivalent to `org.apache.lucene.analysis.shingle.ShingleAnalyzerWrapper`
/// (since 3.1), an `AnalyzerWrapper`.
#[derive(Debug)]
pub struct ShingleAnalyzerWrapper {
    inner: AnalyzerWrapper,
    delegate: Arc<dyn Analyzer>,
    max_shingle_size: i32,
    min_shingle_size: i32,
    token_separator: String,
    output_unigrams: bool,
    output_unigrams_if_no_shingles: bool,
    filler_token: Option<String>,
}

impl ShingleAnalyzerWrapper {
    /// Equivalent to `ShingleAnalyzerWrapper(Analyzer)`, `this(defaultAnalyzer,
    /// ShingleFilter.DEFAULT_MAX_SHINGLE_SIZE)`.
    ///
    /// # Errors
    ///
    /// As [`with_options`](Self::with_options).
    pub fn new(default_analyzer: Arc<dyn Analyzer>) -> Result<Self> {
        Self::with_max_shingle_size(default_analyzer, DEFAULT_MAX_SHINGLE_SIZE)
    }

    /// Equivalent to `ShingleAnalyzerWrapper(Analyzer, int)`, `this(defaultAnalyzer,
    /// ShingleFilter.DEFAULT_MIN_SHINGLE_SIZE, maxShingleSize)`.
    ///
    /// # Errors
    ///
    /// As [`with_options`](Self::with_options).
    pub fn with_max_shingle_size(
        default_analyzer: Arc<dyn Analyzer>,
        max_shingle_size: i32,
    ) -> Result<Self> {
        Self::with_shingle_sizes(default_analyzer, DEFAULT_MIN_SHINGLE_SIZE, max_shingle_size)
    }

    /// Equivalent to `ShingleAnalyzerWrapper(Analyzer, int, int)`.
    ///
    /// # Errors
    ///
    /// As [`with_options`](Self::with_options).
    pub fn with_shingle_sizes(
        default_analyzer: Arc<dyn Analyzer>,
        min_shingle_size: i32,
        max_shingle_size: i32,
    ) -> Result<Self> {
        Self::with_options(
            default_analyzer,
            min_shingle_size,
            max_shingle_size,
            Some(DEFAULT_TOKEN_SEPARATOR),
            true,
            false,
            Some(DEFAULT_FILLER_TOKEN),
        )
    }

    /// Creates a new ShingleAnalyzerWrapper
    ///
    /// * `delegate` Analyzer whose TokenStream is to be filtered
    /// * `min_shingle_size` Min shingle (token ngram) size
    /// * `max_shingle_size` Max shingle size
    /// * `token_separator` Used to separate input stream tokens in output
    ///   shingles (`None`, Java's `null`, is the empty string)
    /// * `output_unigrams` Whether or not the filter shall pass the original
    ///   tokens to the output stream
    /// * `output_unigrams_if_no_shingles` Overrides the behavior of
    ///   outputUnigrams==false for those times when no shingles are available
    ///   (because there are fewer than minShingleSize tokens in the input
    ///   stream)? Note that if outputUnigrams==true, then unigrams are always
    ///   output, regardless of whether any shingles are available.
    /// * `filler_token` filler token to use when positionIncrement is more
    ///   than 1
    ///
    /// Equivalent to `ShingleAnalyzerWrapper(Analyzer, int, int, String,
    /// boolean, boolean, String)`, whose `super(delegate.getReuseStrategy())`
    /// shares the delegate's strategy.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` "Max shingle size must be >= 2", "Min
    /// shingle size must be >= 2" or "Min shingle size must be <= max shingle
    /// size".
    pub fn with_options(
        delegate: Arc<dyn Analyzer>,
        min_shingle_size: i32,
        max_shingle_size: i32,
        token_separator: Option<&str>,
        output_unigrams: bool,
        output_unigrams_if_no_shingles: bool,
        filler_token: Option<&str>,
    ) -> Result<Self> {
        if max_shingle_size < 2 {
            return Err(LuceneError::illegal_argument(
                "Max shingle size must be >= 2",
            ));
        }
        if min_shingle_size < 2 {
            return Err(LuceneError::illegal_argument(
                "Min shingle size must be >= 2",
            ));
        }
        if min_shingle_size > max_shingle_size {
            return Err(LuceneError::illegal_argument(
                "Min shingle size must be <= max shingle size",
            ));
        }
        let token_separator = token_separator.unwrap_or("").to_string();
        let filler_token = filler_token.map(str::to_string);
        let inner = AnalyzerWrapper::new(
            Box::new(ShingleAnalyzerWrapperLogic {
                delegate: Arc::clone(&delegate),
                max_shingle_size,
                min_shingle_size,
                token_separator: token_separator.clone(),
                output_unigrams,
                output_unigrams_if_no_shingles,
                filler_token: filler_token.clone(),
            }),
            Box::new(DelegateReuseStrategy(Arc::clone(&delegate))),
        );
        Ok(Self {
            inner,
            delegate,
            max_shingle_size,
            min_shingle_size,
            token_separator,
            output_unigrams,
            output_unigrams_if_no_shingles,
            filler_token,
        })
    }

    /// Wraps `StandardAnalyzer`.
    ///
    /// Equivalent to `ShingleAnalyzerWrapper()`.
    pub fn new_standard() -> Self {
        Self::new_standard_with_shingle_sizes(DEFAULT_MIN_SHINGLE_SIZE, DEFAULT_MAX_SHINGLE_SIZE)
            .expect("INVARIANT: the default shingle sizes are valid")
    }

    /// Wraps `StandardAnalyzer`.
    ///
    /// Equivalent to `ShingleAnalyzerWrapper(int, int)`.
    ///
    /// # Errors
    ///
    /// As [`with_options`](Self::with_options).
    pub fn new_standard_with_shingle_sizes(
        min_shingle_size: i32,
        max_shingle_size: i32,
    ) -> Result<Self> {
        Self::with_shingle_sizes(
            Arc::new(StandardAnalyzer::new()),
            min_shingle_size,
            max_shingle_size,
        )
    }

    /// The max shingle (token ngram) size
    ///
    /// Equivalent to `ShingleAnalyzerWrapper.getMaxShingleSize()`.
    pub fn get_max_shingle_size(&self) -> i32 {
        self.max_shingle_size
    }

    /// The min shingle (token ngram) size
    ///
    /// Equivalent to `ShingleAnalyzerWrapper.getMinShingleSize()`.
    pub fn get_min_shingle_size(&self) -> i32 {
        self.min_shingle_size
    }

    /// Equivalent to `ShingleAnalyzerWrapper.getTokenSeparator()`.
    pub fn get_token_separator(&self) -> &str {
        &self.token_separator
    }

    /// Equivalent to `ShingleAnalyzerWrapper.isOutputUnigrams()`.
    pub fn is_output_unigrams(&self) -> bool {
        self.output_unigrams
    }

    /// Equivalent to `ShingleAnalyzerWrapper.isOutputUnigramsIfNoShingles()`.
    pub fn is_output_unigrams_if_no_shingles(&self) -> bool {
        self.output_unigrams_if_no_shingles
    }

    /// Equivalent to `ShingleAnalyzerWrapper.getFillerToken()`: `None` for
    /// Java's `null`.
    pub fn get_filler_token(&self) -> Option<&str> {
        self.filler_token.as_deref()
    }

    /// Equivalent to the final `ShingleAnalyzerWrapper.getWrappedAnalyzer(String)`.
    pub fn get_wrapped_analyzer(&self, _field_name: &str) -> &Arc<dyn Analyzer> {
        &self.delegate
    }
}

impl Default for ShingleAnalyzerWrapper {
    fn default() -> Self {
        Self::new_standard()
    }
}

impl Analyzer for ShingleAnalyzerWrapper {
    fn analyzer_state(&self) -> &AnalyzerState {
        self.inner.analyzer_state()
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.analysis.shingle.ShingleAnalyzerWrapper"
    }

    fn create_components(&self, field_name: &str) -> Result<TokenStreamComponents> {
        self.inner.create_components(field_name)
    }

    fn normalize_token_stream(
        &self,
        field_name: &str,
        input: Box<dyn TokenStream>,
    ) -> Box<dyn TokenStream> {
        self.inner.normalize_token_stream(field_name, input)
    }

    fn init_reader(&self, field_name: &str, reader: Box<dyn CharReader>) -> Box<dyn CharReader> {
        self.inner.init_reader(field_name, reader)
    }

    fn init_reader_for_normalization(
        &self,
        field_name: &str,
        reader: Box<dyn CharReader>,
    ) -> Box<dyn CharReader> {
        self.inner.init_reader_for_normalization(field_name, reader)
    }

    fn attribute_factory(&self, field_name: &str) -> Arc<dyn AttributeFactory> {
        self.inner.attribute_factory(field_name)
    }

    fn get_position_increment_gap(&self, field_name: &str) -> i32 {
        self.inner.get_position_increment_gap(field_name)
    }

    fn get_offset_gap(&self, field_name: &str) -> i32 {
        self.inner.get_offset_gap(field_name)
    }
}
