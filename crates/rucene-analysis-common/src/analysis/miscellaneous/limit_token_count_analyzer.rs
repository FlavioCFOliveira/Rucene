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
//! An analyzer that limits the number of tokens of its delegate, ported from
//! `org.apache.lucene.analysis.miscellaneous.LimitTokenCountAnalyzer`.

use std::sync::Arc;

use rucene::analysis::{
    Analyzer, AnalyzerState, AnalyzerWrapper, AnalyzerWrapperLogic, CharReader, ReuseStrategy,
    SharedTokenStream, TokenStream, TokenStreamComponents,
};
use rucene::error::Result;
use rucene::util::atomic_ref_cell::AtomicRefCell;
use rucene::util::attribute::AttributeFactory;

use super::limit_token_count_filter::new_limit_token_count_filter_consuming;

/// The reuse strategy Java's `super(delegate.getReuseStrategy())` shares
/// with the delegate.
///
/// **Divergence from Lucene 10.5.0.** Java hands the delegate's very
/// strategy object to the wrapper; a Rust strategy is owned by one
/// [`AnalyzerState`], so this strategy forwards every call to the
/// delegate's, with the wrapper's state, which is what the shared object
/// receives in Java.
#[derive(Debug)]
struct DelegateReuseStrategy(Arc<dyn Analyzer>);

impl ReuseStrategy for DelegateReuseStrategy {
    fn get_reusable_components(
        &self,
        state: &AnalyzerState,
        field_name: &str,
    ) -> Option<Arc<AtomicRefCell<TokenStreamComponents>>> {
        self.0
            .analyzer_state()
            .reuse_strategy()
            .get_reusable_components(state, field_name)
    }

    fn set_reusable_components(
        &self,
        state: &AnalyzerState,
        field_name: &str,
        components: Arc<AtomicRefCell<TokenStreamComponents>>,
    ) {
        self.0
            .analyzer_state()
            .reuse_strategy()
            .set_reusable_components(state, field_name, components)
    }
}

/// What [`LimitTokenCountAnalyzer`] overrides of `AnalyzerWrapper`.
#[derive(Debug)]
struct LimitTokenCountAnalyzerLogic {
    delegate: Arc<dyn Analyzer>,
    max_token_count: i32,
    consume_all_tokens: bool,
}

impl AnalyzerWrapperLogic for LimitTokenCountAnalyzerLogic {
    /// Equivalent to `LimitTokenCountAnalyzer.getWrappedAnalyzer(String)`.
    fn get_wrapped_analyzer(&self, _field_name: &str) -> &dyn Analyzer {
        self.delegate.as_ref()
    }

    /// Equivalent to `LimitTokenCountAnalyzer.wrapComponents(String,
    /// TokenStreamComponents)`: the delegate's source, and a
    /// `LimitTokenCountFilter` over its token stream.
    ///
    /// # Panics
    ///
    /// With the `IllegalArgumentException` `LimitTokenCountFilter` throws for
    /// a `maxTokenCount` below one (the hook returns no error, and Java's
    /// exception is unchecked).
    fn wrap_components(
        &self,
        _field_name: &str,
        components: TokenStreamComponents,
    ) -> TokenStreamComponents {
        let input: Box<dyn TokenStream> =
            Box::new(SharedTokenStream::new(components.token_stream()));
        match new_limit_token_count_filter_consuming(
            input,
            self.max_token_count,
            self.consume_all_tokens,
        ) {
            Ok(filter) => components.with_sink(Arc::new(AtomicRefCell::new(filter))),
            Err(error) => rucene::store::java_nio_file::throw_unchecked(
                "java.lang.IllegalArgumentException",
                Some(&error.to_string()),
            ),
        }
    }
}

/// This Analyzer limits the number of tokens while indexing. It is a
/// replacement for the maximum field length setting inside
/// `org.apache.lucene.index.IndexWriter`.
///
/// Equivalent to `org.apache.lucene.analysis.miscellaneous.LimitTokenCountAnalyzer`,
/// an `AnalyzerWrapper`.
#[derive(Debug)]
pub struct LimitTokenCountAnalyzer {
    inner: AnalyzerWrapper,
    delegate: Arc<dyn Analyzer>,
    max_token_count: i32,
    consume_all_tokens: bool,
}

impl LimitTokenCountAnalyzer {
    /// Build an analyzer that limits the maximum number of tokens per field.
    /// This analyzer will not consume any tokens beyond the
    /// `max_token_count` limit.
    ///
    /// Equivalent to `LimitTokenCountAnalyzer(Analyzer, int)`, `this(delegate,
    /// maxTokenCount, false)`.
    pub fn new(delegate: Arc<dyn Analyzer>, max_token_count: i32) -> Self {
        Self::new_consuming(delegate, max_token_count, false)
    }

    /// Build an analyzer that limits the maximum number of tokens per field:
    /// `delegate` is the analyzer to wrap, `max_token_count` the maximum
    /// number of tokens to return, and `consume_all_tokens` whether all
    /// tokens from the delegate should be consumed even if `max_token_count`
    /// is reached.
    ///
    /// Equivalent to `LimitTokenCountAnalyzer(Analyzer, int, boolean)`, whose
    /// `super(delegate.getReuseStrategy())` shares the delegate's strategy.
    pub fn new_consuming(
        delegate: Arc<dyn Analyzer>,
        max_token_count: i32,
        consume_all_tokens: bool,
    ) -> Self {
        let inner = AnalyzerWrapper::new(
            Box::new(LimitTokenCountAnalyzerLogic {
                delegate: Arc::clone(&delegate),
                max_token_count,
                consume_all_tokens,
            }),
            Box::new(DelegateReuseStrategy(Arc::clone(&delegate))),
        );
        Self {
            inner,
            delegate,
            max_token_count,
            consume_all_tokens,
        }
    }
}

impl Analyzer for LimitTokenCountAnalyzer {
    fn analyzer_state(&self) -> &AnalyzerState {
        self.inner.analyzer_state()
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.analysis.miscellaneous.LimitTokenCountAnalyzer"
    }

    /// Equivalent to `LimitTokenCountAnalyzer.toString()`.
    fn analyzer_to_string(&self) -> String {
        format!(
            "LimitTokenCountAnalyzer({}, maxTokenCount={}, consumeAllTokens={})",
            self.delegate.analyzer_to_string(),
            self.max_token_count,
            self.consume_all_tokens
        )
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
