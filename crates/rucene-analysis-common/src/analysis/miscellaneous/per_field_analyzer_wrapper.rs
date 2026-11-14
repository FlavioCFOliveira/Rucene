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
//! An analyzer that picks a different analyzer per field, ported from
//! `org.apache.lucene.analysis.miscellaneous.PerFieldAnalyzerWrapper`.

use std::sync::Arc;

use rucene::analysis::{
    Analyzer, AnalyzerState, CharReader, DelegatingAnalyzerWrapper, DelegatingAnalyzerWrapperLogic,
    PerFieldReuseStrategy, TokenStream, TokenStreamComponents,
};
use rucene::error::Result;
use rucene::util::attribute::AttributeFactory;
use rucene::util::java_collections::JavaHashMap;

/// What [`PerFieldAnalyzerWrapper`] overrides of `DelegatingAnalyzerWrapper`.
#[derive(Debug)]
struct PerFieldAnalyzerWrapperLogic {
    default_analyzer: Arc<dyn Analyzer>,
    field_analyzers: Arc<JavaHashMap<Arc<dyn Analyzer>>>,
}

impl DelegatingAnalyzerWrapperLogic for PerFieldAnalyzerWrapperLogic {
    /// Equivalent to `PerFieldAnalyzerWrapper.getWrappedAnalyzer(String)`.
    fn get_wrapped_analyzer(&self, field_name: &str) -> &dyn Analyzer {
        match self.field_analyzers.get(field_name) {
            Some(analyzer) => analyzer.as_ref(),
            None => self.default_analyzer.as_ref(),
        }
    }
}

/// This analyzer is used to facilitate scenarios where different fields
/// require different analysis techniques. Use the Map argument in
/// [`PerFieldAnalyzerWrapper::new_with_fields`] to add non-default analyzers
/// for fields.
///
/// A `PerFieldAnalyzerWrapper` can be used like any other analyzer, for both
/// indexing and query parsing.
///
/// Equivalent to `org.apache.lucene.analysis.miscellaneous.PerFieldAnalyzerWrapper`,
/// a `DelegatingAnalyzerWrapper` with the `PER_FIELD_REUSE_STRATEGY`. The
/// field map is a [`JavaHashMap`], so that `toString()` lists the fields in
/// the order a `java.util.HashMap` holding them lists them.
#[derive(Debug)]
pub struct PerFieldAnalyzerWrapper {
    inner: DelegatingAnalyzerWrapper,
    default_analyzer: Arc<dyn Analyzer>,
    field_analyzers: Arc<JavaHashMap<Arc<dyn Analyzer>>>,
}

impl PerFieldAnalyzerWrapper {
    /// Constructs with default analyzer.
    ///
    /// Equivalent to `PerFieldAnalyzerWrapper(Analyzer)`, `this(defaultAnalyzer,
    /// null)`.
    pub fn new(default_analyzer: Arc<dyn Analyzer>) -> Self {
        Self::new_with_fields(default_analyzer, None)
    }

    /// Constructs with default analyzer and a map of analyzers to use for
    /// specific fields: `default_analyzer` is used for all fields except
    /// those specifically named in `field_analyzers`, a map (String field
    /// name to the Analyzer) to be used for those fields.
    ///
    /// Equivalent to `PerFieldAnalyzerWrapper(Analyzer, Map<String,
    /// Analyzer>)`; `None` is Java's `null`, an empty map.
    pub fn new_with_fields(
        default_analyzer: Arc<dyn Analyzer>,
        field_analyzers: Option<JavaHashMap<Arc<dyn Analyzer>>>,
    ) -> Self {
        let field_analyzers = Arc::new(field_analyzers.unwrap_or_default());
        let inner = DelegatingAnalyzerWrapper::new(
            Box::new(PerFieldReuseStrategy),
            Box::new(PerFieldAnalyzerWrapperLogic {
                default_analyzer: Arc::clone(&default_analyzer),
                field_analyzers: Arc::clone(&field_analyzers),
            }),
        );
        Self {
            inner,
            default_analyzer,
            field_analyzers,
        }
    }
}

impl Analyzer for PerFieldAnalyzerWrapper {
    fn analyzer_state(&self) -> &AnalyzerState {
        self.inner.analyzer_state()
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.analysis.miscellaneous.PerFieldAnalyzerWrapper"
    }

    /// Equivalent to `PerFieldAnalyzerWrapper.toString()`: `fieldAnalyzers`
    /// rendered as `AbstractMap.toString()` renders it.
    fn analyzer_to_string(&self) -> String {
        let fields = self
            .field_analyzers
            .iter()
            .map(|(field, analyzer)| format!("{field}={}", analyzer.analyzer_to_string()))
            .collect::<Vec<_>>()
            .join(", ");
        format!(
            "PerFieldAnalyzerWrapper({{{fields}}}, default={})",
            self.default_analyzer.analyzer_to_string()
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
