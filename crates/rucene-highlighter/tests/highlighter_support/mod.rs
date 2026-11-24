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

//! Helpers shared by the test targets of `rucene-highlighter` that port the tests
//! of Lucene 10.5.0 `lucene/highlighter/src/test` (sprint 17, rmp #629): the suite
//! plumbing of the sprint-9 core test ports (`core_support`,
//! `search_support`), reused by path.

#![allow(dead_code, unused_imports)]

#[path = "../../../rucene-test-framework/tests/core_support/mod.rs"]
pub mod core_support;

#[path = "../../../rucene-test-framework/tests/core_search/support.rs"]
pub mod search_support;

use std::fmt;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::{Class, Constructor, Method};
use rucene::analysis::{Analyzer, AnalyzerState, GlobalReuseStrategy, TokenStreamComponents};
use rucene::error::Result;
use rucene_test_framework::analysis::base_token_stream_test_case::BaseTokenStreamTestCase;

/// The declaration of a Java test class `name` (`public class … extends
/// BaseTokenStreamTestCase`) with `methods`.
pub fn btstc_class(name: &str, methods: Vec<Method>) -> Arc<Class> {
    let mut builder = Class::builder(name)
        .extends(&BaseTokenStreamTestCase::class())
        .constructor(Constructor::no_args(|| ()));
    for m in methods {
        builder = builder.method(m);
    }
    builder.build()
}

/// The `createComponents(String)` body of an anonymous analyzer.
pub type CreateComponents = dyn Fn(Option<&str>) -> Result<TokenStreamComponents> + Send + Sync;

/// An anonymous `new Analyzer() { createComponents }` of a test
/// (`GLOBAL_REUSE_STRATEGY`): its Java binary name and body.
pub struct FnAnalyzer {
    state: AnalyzerState,
    name: &'static str,
    create: Box<CreateComponents>,
}

impl fmt::Debug for FnAnalyzer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FnAnalyzer")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

impl FnAnalyzer {
    /// `new Analyzer() { createComponents }`.
    pub fn new(
        name: &'static str,
        create: impl Fn(Option<&str>) -> Result<TokenStreamComponents> + Send + Sync + 'static,
    ) -> FnAnalyzer {
        FnAnalyzer {
            state: AnalyzerState::new(Box::new(GlobalReuseStrategy)),
            name,
            create: Box::new(create),
        }
    }

    /// The analyzer behind an `Arc`.
    pub fn arc(self) -> Arc<dyn Analyzer> {
        Arc::new(self)
    }
}

impl Analyzer for FnAnalyzer {
    fn analyzer_state(&self) -> &AnalyzerState {
        &self.state
    }

    fn java_class_name(&self) -> &'static str {
        self.name
    }

    fn create_components(&self, field_name: Option<&str>) -> Result<TokenStreamComponents> {
        (self.create)(field_name)
    }
}

/// Runs `body`, Java's `try { body }` whose `finally` the caller runs after.
pub fn attempt<T>(body: impl FnOnce() -> Result<T>) -> Result<T> {
    body()
}

/// A public instance method `name` of a Java test class whose instance is a
/// `T` (fields initialised by its constructor): an `Err` is thrown as its
/// Java exception.
pub fn ti<T: std::any::Any + Send>(name: &str, body: fn(&mut T) -> Result<()>) -> Method {
    Method::new(name)
        .throws(&["java.lang.Exception"])
        .body(move |t: &mut T| {
            if let Err(e) = body(t) {
                rucene_test_framework::util::java_exceptions::rethrow_java(e)
            }
        })
}

/// The declaration of a Java test class `name` extending `parent`, whose
/// instances `constructor` builds, with `methods`.
pub fn instance_class(
    name: &str,
    parent: &Arc<Class>,
    constructor: Constructor,
    methods: Vec<Method>,
) -> Arc<Class> {
    let mut builder = Class::builder(name)
        .extends(parent)
        .constructor(constructor);
    for m in methods {
        builder = builder.method(m);
    }
    builder.build()
}

/// Forwards the named `LeafReader` members of a `FilterLeafReader` subclass
/// to its base, the core `FilterLeafReader` held in the named field: a copy
/// of `rucene_test_framework::index`'s crate-private `forward_leaf_reader!`
/// (mode `filter`), for the highlighter tests. own subclasses.
#[macro_export]
macro_rules! forward_leaf_reader {
    ($mode:ident $inner:ident; $($m:ident),* $(,)?) => {
        $( $crate::forward_leaf_reader!(@ $mode $inner $m); )*
    };
    (@ $mode:ident $inner:ident core) => {
        fn core(&self) -> &rucene::index::index_reader::IndexReaderCore {
            rucene::index::LeafReader::core(&self.$inner)
        }
    };
    (@ $mode:ident $inner:ident filter_leaf_reader_delegate) => {
        /// `this instanceof FilterLeafReader`: the delegate `getDelegate()`
        /// returns.
        fn filter_leaf_reader_delegate(
            &self,
        ) -> Option<&std::sync::Arc<dyn rucene::index::LeafReader>> {
            rucene::index::LeafReader::filter_leaf_reader_delegate(&self.$inner)
        }
    };
    (@ $mode:ident $inner:ident do_close) => {
        /// The base class's `doClose()`: `in.close()` for a
        /// `FilterLeafReader`, `in.doClose()` for a `FilterCodecReader`.
        fn do_close(&self) -> rucene::error::Result<()> {
            rucene::index::LeafReader::do_close(&self.$inner)
        }
    };
    (@ $mode:ident $inner:ident term_vectors) => {
        fn term_vectors(
            &self,
        ) -> rucene::error::Result<Box<dyn rucene::index::leaf_reader::TermVectors>> {
            (&self.$inner as &dyn rucene::index::LeafReader).term_vectors()
        }
    };
    (@ $mode:ident $inner:ident num_docs) => {
        fn num_docs(&self) -> i32 {
            (&self.$inner as &dyn rucene::index::LeafReader).num_docs()
        }
    };
    (@ $mode:ident $inner:ident max_doc) => {
        fn max_doc(&self) -> i32 {
            (&self.$inner as &dyn rucene::index::LeafReader).max_doc()
        }
    };
    (@ $mode:ident $inner:ident stored_fields) => {
        fn stored_fields(
            &self,
        ) -> rucene::error::Result<Box<dyn rucene::index::index_reader::StoredFields>> {
            (&self.$inner as &dyn rucene::index::LeafReader).stored_fields()
        }
    };
    (@ $mode:ident $inner:ident get_reader_cache_helper) => {
        fn get_reader_cache_helper(
            &self,
        ) -> Option<Box<dyn rucene::index::index_reader::CacheHelper>> {
            (&self.$inner as &dyn rucene::index::LeafReader).get_reader_cache_helper()
        }
    };
    (@ $mode:ident $inner:ident get_core_cache_helper) => {
        fn get_core_cache_helper(
            &self,
        ) -> Option<Box<dyn rucene::index::index_reader::CacheHelper>> {
            (&self.$inner as &dyn rucene::index::LeafReader).get_core_cache_helper()
        }
    };
    (@ $mode:ident $inner:ident terms) => {
        fn terms(
            &self,
            field: &str,
        ) -> rucene::error::Result<Option<std::sync::Arc<dyn rucene::index::Terms>>> {
            (&self.$inner as &dyn rucene::index::LeafReader).terms(field)
        }
    };
    (@ $mode:ident $inner:ident get_numeric_doc_values) => {
        fn get_numeric_doc_values(
            &self,
            field: &str,
        ) -> rucene::error::Result<Option<Box<dyn rucene::index::NumericDocValues>>> {
            (&self.$inner as &dyn rucene::index::LeafReader).get_numeric_doc_values(field)
        }
    };
    (@ $mode:ident $inner:ident get_binary_doc_values) => {
        fn get_binary_doc_values(
            &self,
            field: &str,
        ) -> rucene::error::Result<Option<Box<dyn rucene::index::BinaryDocValues>>> {
            (&self.$inner as &dyn rucene::index::LeafReader).get_binary_doc_values(field)
        }
    };
    (@ $mode:ident $inner:ident get_sorted_doc_values) => {
        fn get_sorted_doc_values(
            &self,
            field: &str,
        ) -> rucene::error::Result<Option<Box<dyn rucene::index::SortedDocValues>>> {
            (&self.$inner as &dyn rucene::index::LeafReader).get_sorted_doc_values(field)
        }
    };
    (@ $mode:ident $inner:ident get_sorted_numeric_doc_values) => {
        fn get_sorted_numeric_doc_values(
            &self,
            field: &str,
        ) -> rucene::error::Result<Option<Box<dyn rucene::index::SortedNumericDocValues>>> {
            (&self.$inner as &dyn rucene::index::LeafReader).get_sorted_numeric_doc_values(field)
        }
    };
    (@ $mode:ident $inner:ident get_sorted_set_doc_values) => {
        fn get_sorted_set_doc_values(
            &self,
            field: &str,
        ) -> rucene::error::Result<Option<Box<dyn rucene::index::SortedSetDocValues>>> {
            (&self.$inner as &dyn rucene::index::LeafReader).get_sorted_set_doc_values(field)
        }
    };
    (@ $mode:ident $inner:ident get_norm_values) => {
        fn get_norm_values(
            &self,
            field: &str,
        ) -> rucene::error::Result<Option<Box<dyn rucene::index::NumericDocValues>>> {
            (&self.$inner as &dyn rucene::index::LeafReader).get_norm_values(field)
        }
    };
    (@ $mode:ident $inner:ident get_doc_values_skipper) => {
        fn get_doc_values_skipper(
            &self,
            field: &str,
        ) -> rucene::error::Result<Option<Box<dyn rucene::index::DocValuesSkipper>>> {
            (&self.$inner as &dyn rucene::index::LeafReader).get_doc_values_skipper(field)
        }
    };
    (@ $mode:ident $inner:ident get_float_vector_values) => {
        fn get_float_vector_values(
            &self,
            field: &str,
        ) -> rucene::error::Result<Option<Box<dyn rucene::index::FloatVectorValues>>> {
            (&self.$inner as &dyn rucene::index::LeafReader).get_float_vector_values(field)
        }
    };
    (@ $mode:ident $inner:ident get_byte_vector_values) => {
        fn get_byte_vector_values(
            &self,
            field: &str,
        ) -> rucene::error::Result<Option<Box<dyn rucene::index::ByteVectorValues>>> {
            (&self.$inner as &dyn rucene::index::LeafReader).get_byte_vector_values(field)
        }
    };
    (@ $mode:ident $inner:ident search_nearest_vectors) => {
        fn search_nearest_vectors(
            &self,
            field: &str,
            target: &[f32],
            collector: &mut dyn rucene::search::knn::KnnCollector,
            accept_docs: &mut dyn rucene::search::AcceptDocs,
        ) -> rucene::error::Result<()> {
            self.$inner
                .search_nearest_vectors(field, target, collector, accept_docs)
        }
    };
    (@ $mode:ident $inner:ident search_nearest_vectors_byte) => {
        fn search_nearest_vectors_byte(
            &self,
            field: &str,
            target: &[u8],
            collector: &mut dyn rucene::search::knn::KnnCollector,
            accept_docs: &mut dyn rucene::search::AcceptDocs,
        ) -> rucene::error::Result<()> {
            self.$inner
                .search_nearest_vectors_byte(field, target, collector, accept_docs)
        }
    };
    (@ $mode:ident $inner:ident get_field_infos) => {
        fn get_field_infos(&self) -> rucene::index::FieldInfos {
            (&self.$inner as &dyn rucene::index::LeafReader).get_field_infos()
        }
    };
    (@ $mode:ident $inner:ident get_live_docs) => {
        fn get_live_docs(&self) -> Option<Box<dyn rucene::util::Bits>> {
            (&self.$inner as &dyn rucene::index::LeafReader).get_live_docs()
        }
    };
    (@ $mode:ident $inner:ident get_point_values) => {
        fn get_point_values(
            &self,
            field: &str,
        ) -> rucene::error::Result<Option<Box<dyn rucene::index::PointValues>>> {
            (&self.$inner as &dyn rucene::index::LeafReader).get_point_values(field)
        }
    };
    (@ $mode:ident $inner:ident check_integrity) => {
        fn check_integrity(&self) -> rucene::error::Result<()> {
            (&self.$inner as &dyn rucene::index::LeafReader).check_integrity()
        }
    };
    (@ $mode:ident $inner:ident get_meta_data) => {
        fn get_meta_data(&self) -> rucene::index::LeafMetaData {
            (&self.$inner as &dyn rucene::index::LeafReader).get_meta_data()
        }
    };
}

/// `new TokenStreamComponents(tokenizer, filters(tokenizer))`: the
/// components of an analyzer whose source is `tokenizer` and whose sink is
/// the filter chain `filters` builds over it.
pub fn chain<L: rucene::analysis::TokenizerLogic + 'static>(
    tokenizer: rucene::analysis::Tokenizer<L>,
    filters: impl FnOnce(
        Box<dyn rucene::analysis::TokenStream>,
    ) -> Result<
        Arc<rucene::util::atomic_ref_cell::AtomicRefCell<dyn rucene::analysis::TokenStream>>,
    >,
) -> Result<TokenStreamComponents> {
    use rucene::analysis::{SharedTokenStream, TokenStream};
    use rucene::util::atomic_ref_cell::AtomicRefCell;
    let tokenizer = Arc::new(AtomicRefCell::new(tokenizer));
    let shared: Arc<AtomicRefCell<dyn TokenStream>> = tokenizer.clone();
    let sink = filters(Box::new(SharedTokenStream::new(shared)))?;
    Ok(TokenStreamComponents::new(
        Box::new(move |reader| tokenizer.borrow_mut().set_reader(reader)),
        sink,
    ))
}

/// `Arc<AtomicRefCell<dyn TokenStream>>` of a concrete stream.
pub fn sink<S: rucene::analysis::TokenStream + 'static>(
    stream: S,
) -> Arc<rucene::util::atomic_ref_cell::AtomicRefCell<dyn rucene::analysis::TokenStream>> {
    Arc::new(rucene::util::atomic_ref_cell::AtomicRefCell::new(stream))
}
