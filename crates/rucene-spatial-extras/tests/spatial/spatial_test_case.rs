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

//! Port of `SpatialTestCase.java` (package `org.apache.lucene.spatial`): a
//! base test class for spatial lucene. It's mostly Lucene generic.
//!
//! * The Java class hierarchy `SpatialTestCase` → `StrategyTestCase` →
//!   `RandomSpatialOpStrategyTestCase` → concrete class is one instance
//!   type per concrete class implementing [`SpatialCase`]: its required
//!   methods reach the inherited fields ([`SpatialFixture`]), its provided
//!   methods are the methods subclasses override (`setUp`, `tearDown`,
//!   `convertShapeFromGetDocuments`, `newDoc`, `randomIndexedShape`, …), and
//!   the extension traits ([`SpatialTestCaseOps`],
//!   `StrategyTestCaseOps`, `RandomSpatialOpStrategyTestCaseOps`) are the
//!   methods no subclass overrides.
//! * `setUp`/`tearDown` are declared on each concrete class (the runner
//!   dispatches them by name, as Java's virtual call does), whose body is
//!   the instance type's [`SpatialCase::set_up`] / [`SpatialCase::tear_down`].

use std::any::Any;
use std::fmt;
use std::sync::{Arc, LazyLock};

use randomizedtesting::jdk::logging::Logger;
use randomizedtesting::jdk::reflect::{modifier, Class, ClassBuilder, Constructor, Method};
use randomizedtesting::junit::Assert;
use randomizedtesting::RandomizedTest;
use rucene::analysis::Analyzer;
use rucene::document::Document;
use rucene::error::{LuceneError, LuceneErrorKind, Result};
use rucene::index::{directory_reader, DirectoryReader};
use rucene::search::{IndexSearcher, Query};
use rucene_spatial_extras::query::spatial_args_parser::SpatialArgsParser;
use rucene_spatial_extras::spatial_strategy::SpatialStrategy;
use rucene_test_framework::analysis::mock_analyzer::MockAnalyzer;
use rucene_test_framework::index::RandomIndexWriter;
use rucene_test_framework::util::java_exceptions::rethrow_java;
use rucene_test_framework::util::lucene_test_case::{SuppressSysoutChecks, TestDirectory};
use rucene_test_framework::util::LuceneTestCase;
use spatial4j::context::spatial_context::SpatialContext;
use spatial4j::distance::distance_utils::DistanceUtils;
use spatial4j::shape::point::Point;
use spatial4j::shape::rectangle::Rectangle;
use spatial4j::shape::shape::Shape;

use crate::spatial::spatial_test_query::SpatialTestQuery;

/// The fields of `SpatialTestCase` and of `StrategyTestCase` (every test
/// class extending `StrategyTestCase` sees both; the others leave the
/// strategy fields unused).
pub struct SpatialFixture {
    /// `log` (`protected Logger log = Logger.getLogger(getClass().getName())`).
    pub log: Logger,
    /// `indexReader` (`private`).
    index_reader: Option<Arc<dyn DirectoryReader>>,
    /// `indexWriter` (`protected`).
    pub index_writer: Option<RandomIndexWriter>,
    /// `directory` (`private`).
    directory: Option<TestDirectory>,
    /// `analyzer` (`private`).
    analyzer: Option<Arc<dyn Analyzer>>,
    /// `indexSearcher` (`protected`).
    pub index_searcher: Option<IndexSearcher>,
    /// `ctx` (`protected SpatialContext ctx; // subclass must initialize`).
    pub ctx: Option<Arc<SpatialContext>>,
    /// `StrategyTestCase.argsParser` (`protected final`).
    pub args_parser: SpatialArgsParser,
    /// `StrategyTestCase.strategy` (`protected`).
    pub strategy: Option<Arc<dyn SpatialStrategy>>,
    /// `StrategyTestCase.storeShape` (`protected boolean storeShape = true`).
    pub store_shape: bool,
}

impl SpatialFixture {
    /// The field initializers, for the concrete class `class_name`
    /// (`getClass().getName()`).
    pub fn new(class_name: &str) -> Self {
        Self {
            log: Logger::get_logger(class_name),
            index_reader: None,
            index_writer: None,
            directory: None,
            analyzer: None,
            index_searcher: None,
            ctx: None,
            args_parser: SpatialArgsParser::new(),
            strategy: None,
            store_shape: true,
        }
    }

    /// `ctx`, dereferenced (Java's `NullPointerException` when unset).
    pub fn ctx(&self) -> Arc<SpatialContext> {
        Arc::clone(
            self.ctx
                .as_ref()
                .expect("java.lang.NullPointerException: ctx"),
        )
    }

    /// `strategy`, dereferenced (Java's `NullPointerException` when unset).
    pub fn strategy(&self) -> Arc<dyn SpatialStrategy> {
        Arc::clone(
            self.strategy
                .as_ref()
                .expect("java.lang.NullPointerException: strategy"),
        )
    }

    /// `indexWriter`, dereferenced.
    pub fn index_writer(&self) -> &RandomIndexWriter {
        self.index_writer
            .as_ref()
            .expect("java.lang.NullPointerException: indexWriter")
    }

    /// `indexSearcher`, dereferenced.
    pub fn index_searcher(&self) -> &IndexSearcher {
        self.index_searcher
            .as_ref()
            .expect("java.lang.NullPointerException: indexSearcher")
    }

    /// `indexReader`, dereferenced.
    pub fn index_reader(&self) -> &Arc<dyn DirectoryReader> {
        self.index_reader
            .as_ref()
            .expect("java.lang.NullPointerException: indexReader")
    }
}

/// A concrete class extending `SpatialTestCase`: the inherited fields and
/// the methods subclasses override (see the module documentation).
pub trait SpatialCase: Any + Send + Sized {
    /// The inherited fields.
    fn fx(&self) -> &SpatialFixture;

    /// The inherited fields, mutable.
    fn fx_mut(&mut self) -> &mut SpatialFixture;

    /// `setUp()`.
    ///
    /// # Errors
    ///
    /// The errors of opening the index.
    fn set_up(&mut self) -> Result<()> {
        spatial_set_up(self.fx_mut())
    }

    /// `tearDown()`.
    ///
    /// # Errors
    ///
    /// The errors of closing the index.
    fn tear_down(&mut self) -> Result<()> {
        spatial_tear_down(self.fx_mut())
    }

    /// `StrategyTestCase.convertShapeFromGetDocuments(Shape shape)`:
    /// subclasses may override to transform or remove a shape for indexing.
    fn convert_shape_from_get_documents(&self, shape: Arc<dyn Shape>) -> Option<Arc<dyn Shape>> {
        Some(shape)
    }

    /// `StrategyTestCase.makeQuery(SpatialTestQuery q)`.
    ///
    /// # Errors
    ///
    /// The errors of `strategy.makeQuery`.
    fn make_query(&self, q: &SpatialTestQuery) -> Result<Arc<dyn Query>> {
        self.fx().strategy().make_query(&q.args)
    }

    /// `StrategyTestCase.newDoc(String id, Shape shape)`; `None` stands for
    /// a `null` shape.
    ///
    /// # Errors
    ///
    /// The errors of `strategy.createIndexableFields`.
    fn new_doc(&self, id: &str, shape: Option<&Arc<dyn Shape>>) -> Result<Document> {
        crate::spatial::strategy_test_case::strategy_new_doc(self.fx(), id, shape)
    }

    /// `RandomSpatialOpStrategyTestCase.randomIndexedShape()` (`protected
    /// abstract`).
    ///
    /// # Errors
    ///
    /// The errors of the shapes made.
    fn random_indexed_shape(&mut self) -> Result<Arc<dyn Shape>> {
        unreachable!("abstract method randomIndexedShape() of a class that does not declare it")
    }

    /// `RandomSpatialOpStrategyTestCase.randomQueryShape()` (`protected
    /// abstract`).
    ///
    /// # Errors
    ///
    /// The errors of the shapes made.
    fn random_query_shape(&mut self) -> Result<Arc<dyn Shape>> {
        unreachable!("abstract method randomQueryShape() of a class that does not declare it")
    }
}

/// `SpatialTestCase.setUp()`: `super.setUp()`, then a new directory,
/// analyzer, writer, reader and searcher.
///
/// # Errors
///
/// The errors of opening the index.
pub fn spatial_set_up(fx: &mut SpatialFixture) -> Result<()> {
    LuceneTestCase::super_set_up();
    let directory = LuceneTestCase::new_directory();
    let analyzer: Arc<dyn Analyzer> = Arc::new(MockAnalyzer::new(&*LuceneTestCase::random()));
    let index_writer = RandomIndexWriter::new_with_config(
        &*LuceneTestCase::random(),
        directory.as_directory(),
        LuceneTestCase::new_index_writer_config_with(
            &*LuceneTestCase::random(),
            Some(Arc::clone(&analyzer)),
        ),
    )?;
    let index_reader = index_writer.get_reader()?;
    fx.index_searcher = Some(LuceneTestCase::new_searcher(Arc::clone(&index_reader)));
    fx.directory = Some(directory);
    fx.analyzer = Some(analyzer);
    fx.index_writer = Some(index_writer);
    fx.index_reader = Some(index_reader);
    Ok(())
}

/// `SpatialTestCase.tearDown()`: `IOUtils.close(indexWriter, indexReader,
/// analyzer, directory)`, then `super.tearDown()`.
///
/// # Errors
///
/// The first error of closing them (all are closed, as `IOUtils.close`
/// does).
pub fn spatial_tear_down(fx: &mut SpatialFixture) -> Result<()> {
    let mut first: Option<LuceneError> = None;
    let mut keep = |r: Result<()>| {
        if let Err(e) = r {
            first.get_or_insert(e);
        }
    };
    if let Some(w) = fx.index_writer.take() {
        keep(w.close());
    }
    if let Some(r) = fx.index_reader.take() {
        keep(r.close());
    }
    if let Some(a) = fx.analyzer.take() {
        a.close();
    }
    if let Some(d) = fx.directory.take() {
        keep(d.close());
    }
    fx.index_searcher = None;
    if let Some(e) = first {
        return Err(e);
    }
    LuceneTestCase::super_tear_down();
    Ok(())
}

/// `SpatialTestCase.SearchResults` (`protected static class`).
pub struct SearchResults {
    /// `numFound`.
    pub num_found: i64,
    /// `results`.
    pub results: Vec<SearchResult>,
}

impl SearchResults {
    /// `SearchResults(long numFound, List<SearchResult> results)`.
    pub fn new(num_found: i64, results: Vec<SearchResult>) -> Self {
        Self { num_found, results }
    }

    /// `toDebugString()`.
    pub fn to_debug_string(&self) -> String {
        let mut str = String::new();
        str.push_str(&format!("found: {}[", self.num_found));
        for r in &self.results {
            let id = r.get_id();
            str.push_str(&format!("{}, ", id.as_deref().unwrap_or("null")));
        }
        str.push(']');
        str
    }
}

impl fmt::Display for SearchResults {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let results: Vec<String> = self.results.iter().map(|r| r.to_string()).collect();
        write!(f, "[found:{} [{}]]", self.num_found, results.join(", "))
    }
}

/// `SpatialTestCase.SearchResult` (`protected static class`).
pub struct SearchResult {
    /// `score`.
    pub score: f32,
    /// `document`.
    pub document: Document,
}

impl SearchResult {
    /// `SearchResult(float score, Document document)`.
    pub fn new(score: f32, document: Document) -> Self {
        Self { score, document }
    }

    /// `getId()`: `document.get("id")`.
    pub fn get_id(&self) -> Option<String> {
        self.document.get("id")
    }
}

impl fmt::Display for SearchResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "[{}={}]",
            rucene::util::java_fmt::float_to_string(self.score),
            self.document
        )
    }
}

/// The methods of `SpatialTestCase` no subclass overrides.
pub trait SpatialTestCaseOps: SpatialCase {
    /// `addDocument(Document doc)`.
    ///
    /// # Errors
    ///
    /// The errors of `indexWriter.addDocument`.
    fn add_document(&mut self, doc: &Document) -> Result<()> {
        self.fx().index_writer().add_document(doc)?;
        Ok(())
    }

    /// `addDocumentsAndCommit(List<Document> documents)`.
    ///
    /// # Errors
    ///
    /// The errors of adding and committing.
    fn add_documents_and_commit(&mut self, documents: &[Document]) -> Result<()> {
        for document in documents {
            self.fx().index_writer().add_document(document)?;
        }
        self.commit()
    }

    /// `deleteAll()`.
    ///
    /// # Errors
    ///
    /// The errors of `indexWriter.deleteAll`.
    fn delete_all(&mut self) -> Result<()> {
        self.fx().index_writer().delete_all()?;
        Ok(())
    }

    /// `commit()`.
    ///
    /// # Errors
    ///
    /// The errors of committing and reopening.
    fn commit(&mut self) -> Result<()> {
        let fx = self.fx_mut();
        fx.index_writer().commit()?;
        let new_reader = directory_reader::open_if_changed(Arc::clone(fx.index_reader()))?;
        if let Some(new_reader) = new_reader {
            if let Some(old) = fx.index_reader.replace(new_reader) {
                old.close()?;
            }
        }
        fx.index_searcher = Some(LuceneTestCase::new_searcher(Arc::clone(fx.index_reader())));
        Ok(())
    }

    /// `verifyDocumentsIndexed(int numDocs)`.
    fn verify_documents_indexed(&self, num_docs: usize) {
        Assert::assert_equals(num_docs as i32, self.fx().index_reader().num_docs());
    }

    /// `executeQuery(Query query, int numDocs)`.
    ///
    /// # Errors
    ///
    /// Java's `RuntimeException("IOException thrown while executing
    /// query", ioe)` for an I/O error; the other errors of searching.
    fn execute_query(&self, query: Arc<dyn Query>, num_docs: i32) -> Result<SearchResults> {
        let searcher = self.fx().index_searcher();
        let run = || -> Result<SearchResults> {
            let top_docs = searcher.search(query, num_docs)?;
            let stored_fields = searcher.stored_fields()?;
            let mut results = Vec::new();
            for score_doc in &top_docs.score_docs {
                results.push(SearchResult::new(
                    score_doc.score,
                    stored_fields.document(score_doc.doc)?,
                ));
            }
            Ok(SearchResults::new(top_docs.total_hits.value(), results))
        };
        run().map_err(|e| {
            if e.is_io_exception() {
                LuceneError::from(LuceneErrorKind::Unchecked {
                    class: "java.lang.RuntimeException",
                    message: Some("IOException thrown while executing query".to_string()),
                    cause: Some(std::sync::Arc::new(e)),
                })
            } else {
                e
            }
        })
    }

    /// `randomPoint()`.
    ///
    /// # Errors
    ///
    /// The `InvalidShapeException` of the shape factory.
    fn random_point(&self) -> Result<Arc<dyn Point>> {
        let ctx = self.fx().ctx();
        let wb = ctx.get_world_bounds();
        ctx.get_shape_factory().point_xy(
            f64::from(RandomizedTest::random_int_between(
                wb.get_min_x() as i32,
                wb.get_max_x() as i32,
            )),
            f64::from(RandomizedTest::random_int_between(
                wb.get_min_y() as i32,
                wb.get_max_y() as i32,
            )),
        )
    }

    /// `randomRectangle()`.
    ///
    /// # Errors
    ///
    /// The `InvalidShapeException` of the shape factory.
    fn random_rectangle(&self) -> Result<Arc<dyn Rectangle>> {
        self.random_rectangle_in(&*self.fx().ctx().get_world_bounds())
    }

    /// `randomRectangle(Rectangle bounds)`.
    ///
    /// # Errors
    ///
    /// The `InvalidShapeException` of the shape factory.
    fn random_rectangle_in(&self, bounds: &dyn Rectangle) -> Result<Arc<dyn Rectangle>> {
        let x_new_start_and_width = random_sub_range(bounds.get_min_x(), bounds.get_width());
        let mut x_min = x_new_start_and_width[0];
        let mut x_max = x_min + x_new_start_and_width[1];
        if bounds.get_crosses_date_line() {
            x_min = DistanceUtils::norm_lon_deg(x_min);
            x_max = DistanceUtils::norm_lon_deg(x_max);
        }

        let y_new_start_and_height = random_sub_range(bounds.get_min_y(), bounds.get_height());
        let y_min = y_new_start_and_height[0];
        let y_max = y_min + y_new_start_and_height[1];

        self.fx()
            .ctx()
            .get_shape_factory()
            .rect(x_min, x_max, y_min, y_max)
    }
}

impl<T: SpatialCase> SpatialTestCaseOps for T {}

/// `randomSubRange(double boundStart, double boundLen)`: returns new
/// minStart and new length that is inside the range specified by the
/// arguments.
pub fn random_sub_range(bound_start: f64, bound_len: f64) -> [f64; 2] {
    if bound_len >= 3.0 && LuceneTestCase::usually() {
        // typical
        // prefer integers for ease of debugability ... and prefer 1/16th of bound
        let int_bound_start = bound_start.ceil() as i32;
        let int_bound_end = (bound_start + bound_len) as i32;
        let int_bound_len = int_bound_end - int_bound_start;
        let new_len =
            random_gaussian_mean_max(f64::from(int_bound_len) / 16.0, f64::from(int_bound_len))
                as i32;
        let new_start =
            int_bound_start + RandomizedTest::random_int_between(0, int_bound_len - new_len);
        [f64::from(new_start), f64::from(new_len)]
    } else {
        // (no int rounding)
        let new_len = random_gaussian_mean_max(bound_len / 16.0, bound_len);
        let new_start = bound_start
            + if bound_len - new_len == 0.0 {
                0.0
            } else {
                RandomizedTest::random_double() % (bound_len - new_len)
            };
        [new_start, new_len]
    }
}

/// `randomGaussianMeanMax(double mean, double max)` (`private`): within one
/// standard deviation (68% of the time) the result is "close" to mean. By
/// "close": when greater than mean, it's the lesser of 2*mean or half way to
/// max, when lesser than mean, it's the greater of max-2*mean or half way to
/// 0. The other 32% of the time it's in the rest of the range, touching
/// either 0 or max but never exceeding.
fn random_gaussian_mean_max(mean: f64, max: f64) -> f64 {
    // DWS: I verified the results empirically
    java_assert(mean <= max && mean >= 0.0);
    let mut g = RandomizedTest::random_gaussian();
    let mut mean2 = mean;
    let mut flip = 1.0;
    if g < 0.0 {
        mean2 = max - mean;
        flip = -1.0;
        g *= -1.0;
    }
    // pivot is the distance from mean2 towards max where the boundary of
    // 1 standard deviation alters the calculation
    let pivot_max = max - mean2;
    let pivot = mean2.min(pivot_max / 2.0); // from 0 to max-mean2
    java_assert(pivot >= 0.0 && pivot_max >= pivot && g >= 0.0);
    let pivot_result = if g <= 1.0 {
        pivot * g
    } else {
        pivot_max.min((g - 1.0) * (pivot_max - pivot) + pivot)
    };

    let result = mean + flip * pivot_result;
    if result < 0.0 || result > max {
        mean
    } else {
        result
    } // due this due to computational numerical precision
}

/// Java's `assert condition;` under `-ea` (Lucene's test runs): an
/// `AssertionError` without a message.
///
/// **Divergence.** Java evaluates `assert` only with `-ea` (as Lucene's
/// builds run tests); the port evaluates it when `debug_assertions` are on.
pub fn java_assert(condition: bool) {
    if cfg!(debug_assertions) && !condition {
        Assert::fail_without_message();
    }
}

/// Java's `assert condition : message;` under `-ea`.
pub fn java_assert_msg(condition: bool, message: &str) {
    if cfg!(debug_assertions) && !condition {
        Assert::fail(message);
    }
}

/// The abstract class `SpatialTestCase` (`@SuppressSysoutChecks(bugUrl =
/// "These tests use JUL extensively.")`).
pub fn spatial_test_case_class() -> Arc<Class> {
    static CLASS: LazyLock<Arc<Class>> = LazyLock::new(|| {
        Class::builder("org.apache.lucene.spatial.SpatialTestCase")
            .modifiers(modifier::PUBLIC | modifier::ABSTRACT)
            .extends(&LuceneTestCase::class())
            .annotate(SuppressSysoutChecks {
                bug_url: "These tests use JUL extensively.".to_string(),
            })
            .build()
    });
    Arc::clone(&CLASS)
}

/// The abstract class `StrategyTestCase extends SpatialTestCase`.
pub fn strategy_test_case_class() -> Arc<Class> {
    static CLASS: LazyLock<Arc<Class>> = LazyLock::new(|| {
        Class::builder("org.apache.lucene.spatial.StrategyTestCase")
            .modifiers(modifier::PUBLIC | modifier::ABSTRACT)
            .extends(&spatial_test_case_class())
            .build()
    });
    Arc::clone(&CLASS)
}

/// The abstract class `RandomSpatialOpStrategyTestCase extends
/// StrategyTestCase`.
pub fn random_spatial_op_strategy_test_case_class() -> Arc<Class> {
    static CLASS: LazyLock<Arc<Class>> = LazyLock::new(|| {
        Class::builder("org.apache.lucene.spatial.prefix.RandomSpatialOpStrategyTestCase")
            .modifiers(modifier::PUBLIC | modifier::ABSTRACT)
            .extends(&strategy_test_case_class())
            .build()
    });
    Arc::clone(&CLASS)
}

/// A public method of the concrete class `T` whose body runs on the test
/// instance; an `Err` is thrown as its Java exception.
pub fn ti<T: SpatialCase>(name: &str, body: fn(&mut T) -> Result<()>) -> Method {
    Method::new(name)
        .throws(&["java.lang.Exception"])
        .body(move |t: &mut T| {
            if let Err(e) = body(t) {
                rethrow_java(e)
            }
        })
}

/// The concrete class `name` extending `parent`, built by `constructor`,
/// with its `setUp`/`tearDown` (the instance type's
/// [`SpatialCase::set_up`] and [`SpatialCase::tear_down`]) and `methods`.
pub fn spatial_class<T: SpatialCase>(
    name: &str,
    parent: &Arc<Class>,
    constructor: Constructor,
    methods: Vec<Method>,
) -> Arc<Class> {
    spatial_class_annotated::<T>(name, parent, constructor, methods, |b| b)
}

/// [`spatial_class`] with class-level annotations: `annotate` adds them to
/// the builder (`@SuppressCodecs`, …).
pub fn spatial_class_annotated<T: SpatialCase>(
    name: &str,
    parent: &Arc<Class>,
    constructor: Constructor,
    methods: Vec<Method>,
    annotate: impl FnOnce(ClassBuilder) -> ClassBuilder,
) -> Arc<Class> {
    let mut builder = annotate(Class::builder(name))
        .extends(parent)
        .constructor(constructor)
        .method(ti::<T>("setUp", T::set_up))
        .method(ti::<T>("tearDown", T::tear_down));
    for m in methods {
        builder = builder.method(m);
    }
    builder.build()
}
