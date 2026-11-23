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

//! Port of `org.apache.lucene.facet.sortedset.TestSortedSetDocValuesFacets`.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::Duration;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::{Document, NumericValue, Store, StringField};
use rucene::error::Result;
use rucene::index::{IndexReader, Term};
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::task_executor::Executor;
use rucene::search::{MatchAllDocsQuery, TermQuery};
use rucene::util::concurrent::NamedThreadFactory;
use rucene_facet::drill_down_query::DrillDownQuery;
use rucene_facet::facet_result::FacetResult;
use rucene_facet::facets::Facets;
use rucene_facet::facets_collector_manager::FacetsCollectorManager;
use rucene_facet::facets_config::{
    DrillDownTermsIndexing, FacetsConfig, DEFAULT_INDEX_FIELD_NAME, DELIM_CHAR,
};
use rucene_facet::label_and_value::{
    number_equals, number_int_value, number_long_value, LabelAndValue,
};
use rucene_facet::sortedset::concurrent_sorted_set_doc_values_facet_counts::ConcurrentSortedSetDocValuesFacetCounts;
use rucene_facet::sortedset::default_sorted_set_doc_values_reader_state::DefaultSortedSetDocValuesReaderState;
use rucene_facet::sortedset::sorted_set_doc_values_facet_counts::SortedSetDocValuesFacetCounts;
use rucene_facet::sortedset::sorted_set_doc_values_facet_field::SortedSetDocValuesFacetField;
use rucene_facet::sortedset::sorted_set_doc_values_reader_state::SortedSetDocValuesReaderState;
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::jdk::thread_pool_executor::ThreadPoolExecutor;
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::lucene_test_case::{TestDirectory, TEST_NIGHTLY, VERBOSE};
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::facet_support::core_support::{assert_equals, assert_null, test};
use crate::facet_support::{
    assert_facet_result, facet_class, get_random_docs, get_random_tokens, sort_facet_results,
    sort_label_values,
};

const IAE: &str = "java.lang.IllegalArgumentException";

/// `TestSortedSetDocValuesFacets`.
pub fn class_def() -> Arc<Class> {
    facet_class(
        "org.apache.lucene.facet.sortedset.TestSortedSetDocValuesFacets",
        vec![
            test("testBasic", test_basic),
            test("testCombinationsOfConfig", test_combinations_of_config),
            test("testBasicHierarchical", test_basic_hierarchical),
            test("testCountAll", test_count_all),
            test("testHierarchicalCountAll", test_hierarchical_count_all),
            test("testBasicSingleValued", test_basic_single_valued),
            test(
                "testHierarchicalBasicSingleValues",
                test_hierarchical_basic_single_values,
            ),
            test("testDrillDownOptions", test_drill_down_options),
            test(
                "testHierarchicalDrillDownOptions",
                test_hierarchical_drill_down_options,
            ),
            test("testStaleState", test_stale_state),
            test("testSparseFacets", test_sparse_facets),
            test(
                "testHierarchicalSparseFacets",
                test_hierarchical_sparse_facets,
            ),
            test("testSomeSegmentsMissing", test_some_segments_missing),
            test(
                "testHierarchicalSomeSegmentsMissing",
                test_hierarchical_some_segments_missing,
            ),
            test("testRandom", test_random),
            test(
                "testRandomHierarchicalFlatMix",
                test_random_hierarchical_flat_mix,
            ),
            test("testNonExistentDimension", test_non_existent_dimension),
            test(
                "testHierarchicalNonExistentDimension",
                test_hierarchical_non_existent_dimension,
            ),
            test(
                "testHierarchicalNonExistentPath",
                test_hierarchical_non_existent_path,
            ),
        ],
    )
}

/// `new SortedSetDocValuesFacetField(dim, path...)`.
fn ssdv(dim: &str, path: &[&str]) -> Result<SortedSetDocValuesFacetField> {
    SortedSetDocValuesFacetField::new(dim, path)
}

fn lv(label: &str, value: i32) -> LabelAndValue {
    LabelAndValue::new(label, NumericValue::Int(value))
}

/// The `try (Directory dir = newDirectory(); RandomIndexWriter writer =
/// new RandomIndexWriter(random(), dir))` resources.
struct Resources {
    dir: TestDirectory,
    writer: RandomIndexWriter,
}

impl Resources {
    fn open() -> Result<Self> {
        let dir = LuceneTestCase::new_directory();
        let writer = RandomIndexWriter::new_with_mock_analyzer(
            &*LuceneTestCase::random(),
            dir.as_directory(),
        )?;
        Ok(Self { dir, writer })
    }

    /// The end of the `try` block: the writer, then the directory.
    fn close(self) -> Result<()> {
        self.writer.close()?;
        self.dir.close()
    }
}

/// The `ExecutorService` of a test and its `finally { if (exec != null)
/// exec.shutdownNow(); }`, run on drop so that a failing assertion still
/// shuts the pool down.
struct ExecGuard(Option<Arc<ThreadPoolExecutor>>);

impl ExecGuard {
    fn exec(&self) -> Option<Arc<dyn Executor>> {
        self.0.as_ref().map(|e| Arc::clone(e) as Arc<dyn Executor>)
    }
}

impl Drop for ExecGuard {
    fn drop(&mut self) {
        if let Some(exec) = self.0.take() {
            exec.shutdown_now();
        }
    }
}

/// `new ThreadPoolExecutor(1, TestUtil.nextInt(random(), 2, 6),
/// Long.MAX_VALUE, TimeUnit.MILLISECONDS, new LinkedBlockingQueue<>(),
/// new NamedThreadFactory("TestIndexSearcher"))`.
fn new_executor() -> ExecGuard {
    ExecGuard(Some(Arc::new(ThreadPoolExecutor::new(
        1,
        TestUtil::next_int(&*LuceneTestCase::random(), 2, 6),
        Duration::from_millis(i64::MAX as u64),
        NamedThreadFactory::new("TestIndexSearcher"),
    ))))
}

/// `randomExecutorServiceOrNull()`.
fn random_executor_service_or_null() -> ExecGuard {
    if LuceneTestCase::random().next_boolean() {
        ExecGuard(None)
    } else {
        new_executor()
    }
}

/// `getAllFacets(IndexSearcher, SortedSetDocValuesReaderState,
/// ExecutorService)`.
fn get_all_facets(
    searcher: &IndexSearcher,
    state: &Arc<dyn SortedSetDocValuesReaderState>,
    exec: Option<Arc<dyn Executor>>,
) -> Result<Box<dyn Facets>> {
    if LuceneTestCase::random().next_boolean() {
        let c = searcher.search_with_manager(
            Arc::new(MatchAllDocsQuery::instance()),
            &FacetsCollectorManager::new(),
        )?;
        match exec {
            Some(exec) => Ok(Box::new(
                ConcurrentSortedSetDocValuesFacetCounts::with_hits(
                    Arc::clone(state),
                    Some(&c),
                    exec,
                )?,
            )),
            None => Ok(Box::new(SortedSetDocValuesFacetCounts::with_hits(
                Arc::clone(state),
                Some(&c),
            )?)),
        }
    } else if let Some(exec) = exec {
        Ok(Box::new(ConcurrentSortedSetDocValuesFacetCounts::new(
            Arc::clone(state),
            exec,
        )?))
    } else {
        Ok(Box::new(SortedSetDocValuesFacetCounts::new(Arc::clone(
            state,
        ))?))
    }
}

/// `new DefaultSortedSetDocValuesReaderState(searcher.getIndexReader(),
/// config)`.
fn new_state(
    searcher: &IndexSearcher,
    config: &Arc<FacetsConfig>,
) -> Result<Arc<dyn SortedSetDocValuesReaderState>> {
    Ok(Arc::new(DefaultSortedSetDocValuesReaderState::new(
        Arc::clone(searcher.get_index_reader()),
        Arc::clone(config),
    )?))
}

/// `facets.getTopChildren(topN, dim, path...).toString()`.
fn top(facets: &dyn Facets, top_n: i32, dim: &str, path: &[&str]) -> Result<String> {
    Ok(facets
        .get_top_children(top_n, dim, path)?
        .unwrap()
        .to_string())
}

/// `facets.getAllChildren(dim, path...)`, non-null.
fn all(facets: &dyn Facets, dim: &str, path: &[&str]) -> Result<FacetResult> {
    Ok(facets.get_all_children(dim, path)?.unwrap())
}

/// `searcher.search(q, n).totalHits.value()`.
fn hits(searcher: &IndexSearcher, q: &DrillDownQuery, n: i32) -> Result<i64> {
    Ok(searcher
        .search(Arc::new(q.clone_query()), n)?
        .total_hits
        .value())
}

/// `new DrillDownQuery(config)` with one `add(dim, path...)`.
fn ddq(config: &Arc<FacetsConfig>, dim: &str, path: &[&str]) -> Result<DrillDownQuery> {
    let mut q = DrillDownQuery::new(Arc::clone(config));
    q.add(dim, path)?;
    Ok(q)
}

fn test_basic() -> Result<()> {
    let config = FacetsConfig::new();
    config.set_multi_valued("a", true);
    config.set_multi_valued("b", true);
    config.set_require_dim_count("b", true);
    let config = Arc::new(config);
    let res = Resources::open()?;
    let writer = &res.writer;
    let mut doc = Document::new();
    doc.add(ssdv("a", &["foo"])?);
    doc.add(ssdv("a", &["bar"])?);
    doc.add(ssdv("a", &["zoo"])?);
    doc.add(ssdv("b", &["baz"])?);
    doc.add(ssdv("b", &["buzz"])?);
    writer.add_document(&config.build(&doc)?)?;
    if LuceneTestCase::random().next_boolean() {
        writer.commit()?;
    }

    let mut doc = Document::new();
    doc.add(ssdv("a", &["foo"])?);
    doc.add(ssdv("b", &["buzz"])?);
    writer.add_document(&config.build(&doc)?)?;

    // NRT open
    let r = writer.get_reader()?;
    {
        let searcher = LuceneTestCase::new_searcher(Arc::clone(&r));

        // Per-top-reader state:
        let state = new_state(&searcher, &config)?;

        let exec = random_executor_service_or_null();
        let facets = get_all_facets(&searcher, &state, exec.exec())?;

        // value for dim a should be -1 since it's multivalued but doesn't require dim counts:
        Assert::assert_equals(
            "dim=a path=[] value=-1 childCount=3\n  foo (2)\n  bar (1)\n  zoo (1)\n",
            top(&*facets, 10, "a", &[])?.as_str(),
        );
        // value for dim b should be 2 since it's multivalued but _does_ require dim counts:
        Assert::assert_equals(
            "dim=b path=[] value=2 childCount=2\n  buzz (2)\n  baz (1)\n",
            top(&*facets, 10, "b", &[])?.as_str(),
        );

        // test getAllChildren
        assert_facet_result(
            &all(&*facets, "a", &[])?,
            "a",
            &[],
            3,
            NumericValue::Int(-1),
            &[lv("bar", 1), lv("foo", 2), lv("zoo", 1)],
        );

        // test getAllDims
        let results = facets.get_all_dims(10)?;
        Assert::assert_equals(2, results.len());
        Assert::assert_equals(
            "dim=b path=[] value=2 childCount=2\n  buzz (2)\n  baz (1)\n",
            results[0].to_string().as_str(),
        );
        Assert::assert_equals(
            "dim=a path=[] value=-1 childCount=3\n  foo (2)\n  bar (1)\n  zoo (1)\n",
            results[1].to_string().as_str(),
        );

        // test getTopDims(10, 10) and expect same results from getAllDims(10)
        let all_dims_results = facets.get_top_dims(10, 10)?;
        assert_equals(&results, &all_dims_results);

        // test getTopDims(2, 1)
        let top_dims_results = facets.get_top_dims(2, 1)?;
        Assert::assert_equals(2, top_dims_results.len());
        Assert::assert_equals(
            "dim=b path=[] value=2 childCount=2\n  buzz (2)\n",
            top_dims_results[0].to_string().as_str(),
        );
        Assert::assert_equals(
            "dim=a path=[] value=-1 childCount=3\n  foo (2)\n",
            top_dims_results[1].to_string().as_str(),
        );

        // test getAllDims
        let results2 = facets.get_all_dims(1)?;
        Assert::assert_equals(2, results2.len());
        Assert::assert_equals(
            "dim=b path=[] value=2 childCount=2\n  buzz (2)\n",
            results2[0].to_string().as_str(),
        );

        // test getTopDims(1, 1)
        let top_dims_results1 = facets.get_top_dims(1, 1)?;
        Assert::assert_equals(1, top_dims_results1.len());
        Assert::assert_equals(
            "dim=b path=[] value=2 childCount=2\n  buzz (2)\n",
            top_dims_results1[0].to_string().as_str(),
        );

        // test getTopDims(0)
        expect_throws(IAE, || facets.get_all_dims(0));

        // test getSpecificValue
        Assert::assert_equals(
            NumericValue::Int(2),
            facets.get_specific_value("a", &["foo"])?,
        );
        expect_throws(IAE, || facets.get_specific_value("a", &["foo", "bar"]));

        // DrillDown:
        let mut q = DrillDownQuery::new(Arc::clone(&config));
        q.add("a", &["foo"])?;
        q.add("b", &["baz"])?;
        Assert::assert_equals(1, hits(&searcher, &q, 1)?);
    }
    r.close()?;
    res.close()
}

// test tricky combinations of the three config: MultiValued, Hierarchical, and RequireDimCount of
// a dim
fn test_combinations_of_config() -> Result<()> {
    let config = FacetsConfig::new();

    // case 1: dimension "a" is hierarchical and non-multiValued
    // expect returns counts[pathOrd]
    config.set_multi_valued("a", false);
    config.set_hierarchical("a", true);

    // case 2:  dimension "b" is hierarchical and multiValued and setRequireDimCount = true
    // expect returns counts[pathOrd]
    config.set_multi_valued("b", true);
    config.set_hierarchical("b", true);
    config.set_require_dim_count("b", true);

    // case 3: dimension "c" is hierarchical and multiValued and setRequireDimCount != true
    // expect always returns counts[pathOrd] for Hierarchical = true
    config.set_multi_valued("c", true);
    config.set_hierarchical("c", true);

    // case 4: dimension "d" is non-hierarchical but multiValued and setRequireDimCount = true
    // expect returns counts[pathOrd]
    config.set_multi_valued("d", true);
    config.set_hierarchical("d", false);
    config.set_require_dim_count("d", true);

    // case 4: dimension "e" that is non-hierarchical and multiValued and setRequireDimCount = false
    // expect returns -1, this is the only case that we reset dimCount to -1
    config.set_multi_valued("e", true);
    config.set_hierarchical("e", false);
    config.set_require_dim_count("e", false);

    // case 5: dimension "f" that it is non-hierarchical and non-multiValued and expect returns
    // counts[pathOrd]
    config.set_multi_valued("f", false);
    config.set_hierarchical("f", false);

    // case 6: expect returns counts[pathOrd] for dims with setHierarchical = true
    config.set_hierarchical("g", true);

    // case 7: expect returns counts[pathOrd] for dims with setHierarchical = true
    config.set_hierarchical("g-2", false);

    // case 8: expect returns counts[pathOrd] for dims with setHierarchical = true
    config.set_require_dim_count("h", true);
    config.set_multi_valued("h", true);
    let config = Arc::new(config);

    let res = Resources::open()?;
    let writer = &res.writer;
    let mut doc = Document::new();
    doc.add(ssdv("a", &["foo"])?);
    doc.add(ssdv("b", &["bar"])?);
    doc.add(ssdv("c", &["zoo"])?);
    doc.add(ssdv("d", &["baz"])?);
    doc.add(ssdv("e", &["buzz"])?);
    doc.add(ssdv("f", &["buzze"])?);
    doc.add(ssdv("g", &["buzzel"])?);
    doc.add(ssdv("g-2", &["buzzell"])?);
    doc.add(ssdv("h", &["buzzele"])?);
    writer.add_document(&config.build(&doc)?)?;

    // NRT open
    let r = writer.get_reader()?;
    {
        let searcher = LuceneTestCase::new_searcher(Arc::clone(&r));

        // Per-top-reader state:
        let state = new_state(&searcher, &config)?;

        let exec = random_executor_service_or_null();
        let facets = get_all_facets(&searcher, &state, exec.exec())?;
        let f = &*facets;
        Assert::assert_equals(
            "dim=a path=[] value=1 childCount=1\n  foo (1)\n",
            top(f, 10, "a", &[])?.as_str(),
        );
        // value for dim b should be 1 since it's multivalued but _does_ require dim counts:
        Assert::assert_equals(
            "dim=b path=[] value=1 childCount=1\n  bar (1)\n",
            top(f, 10, "b", &[])?.as_str(),
        );
        Assert::assert_equals(
            "dim=c path=[] value=1 childCount=1\n  zoo (1)\n",
            top(f, 10, "c", &[])?.as_str(),
        );
        Assert::assert_equals(
            "dim=d path=[] value=1 childCount=1\n  baz (1)\n",
            top(f, 10, "d", &[])?.as_str(),
        );
        // value for dim e should be -1 since it's multivalued but doesn't require dim counts:
        Assert::assert_equals(
            "dim=e path=[] value=-1 childCount=1\n  buzz (1)\n",
            top(f, 10, "e", &[])?.as_str(),
        );
        Assert::assert_equals(
            "dim=f path=[] value=1 childCount=1\n  buzze (1)\n",
            top(f, 10, "f", &[])?.as_str(),
        );
        Assert::assert_equals(
            "dim=g path=[] value=1 childCount=1\n  buzzel (1)\n",
            top(f, 10, "g", &[])?.as_str(),
        );
        Assert::assert_equals(
            "dim=g-2 path=[] value=1 childCount=1\n  buzzell (1)\n",
            top(f, 10, "g-2", &[])?.as_str(),
        );
        Assert::assert_equals(
            "dim=h path=[] value=1 childCount=1\n  buzzele (1)\n",
            top(f, 10, "h", &[])?.as_str(),
        );

        // test getAllDims
        let results = f.get_all_dims(10)?;
        Assert::assert_equals(9, results.len());
        let expected = [
            "dim=a path=[] value=1 childCount=1\n  foo (1)\n",
            "dim=b path=[] value=1 childCount=1\n  bar (1)\n",
            "dim=c path=[] value=1 childCount=1\n  zoo (1)\n",
            "dim=d path=[] value=1 childCount=1\n  baz (1)\n",
            "dim=f path=[] value=1 childCount=1\n  buzze (1)\n",
            "dim=g path=[] value=1 childCount=1\n  buzzel (1)\n",
            "dim=g-2 path=[] value=1 childCount=1\n  buzzell (1)\n",
            "dim=h path=[] value=1 childCount=1\n  buzzele (1)\n",
            "dim=e path=[] value=-1 childCount=1\n  buzz (1)\n",
        ];
        for (i, e) in expected.iter().enumerate() {
            Assert::assert_equals(*e, results[i].to_string().as_str());
        }

        // test getTopDims(10, 10) and expect same results from getAllDims(10)
        let all_top_dims_results = f.get_top_dims(10, 10)?;
        assert_equals(&results, &all_top_dims_results);

        // test getTopDims(n, 10)
        if !all_top_dims_results.is_empty() {
            for i in 1..results.len() {
                assert_equals(&results[..i].to_vec(), &f.get_top_dims(i as i32, 10)?);
            }
        }
    }
    r.close()?;
    res.close()
}

fn test_basic_hierarchical() -> Result<()> {
    let config = FacetsConfig::new();
    config.set_multi_valued("a", true);
    config.set_require_dim_count("a", true);
    config.set_multi_valued("c", true);
    config.set_hierarchical("c", true);
    let config = Arc::new(config);
    let res = Resources::open()?;
    let writer = &res.writer;
    let mut doc = Document::new();
    doc.add(ssdv("a", &["foo"])?);
    doc.add(ssdv("a", &["bar"])?);
    doc.add(ssdv("a", &["zoo"])?);
    doc.add(ssdv("b", &["baz"])?);
    doc.add(ssdv("c", &["buzz"])?);
    doc.add(ssdv("c", &["buzz", "bee"])?);
    doc.add(ssdv("c", &["buzz", "bif"])?);
    doc.add(ssdv("c", &["buzz", "bif", "baf"])?);
    doc.add(ssdv("c", &["buzz", "biz"])?);
    doc.add(ssdv("c", &["buzz", "biz", "bar"])?);
    writer.add_document(&config.build(&doc)?)?;
    if LuceneTestCase::random().next_boolean() {
        writer.commit()?;
    }

    let mut doc = Document::new();
    doc.add(ssdv("a", &["foo"])?);
    doc.add(ssdv("c", &["buzz", "bif", "baf"])?);
    writer.add_document(&config.build(&doc)?)?;

    // NRT open
    let r = writer.get_reader()?;
    {
        let searcher = LuceneTestCase::new_searcher(Arc::clone(&r));

        // Per-top-reader state:
        let state = new_state(&searcher, &config)?;

        let exec = random_executor_service_or_null();
        let facets = get_all_facets(&searcher, &state, exec.exec())?;
        let f = &*facets;

        // since a is not set to be hierarchical but _is_ multivalued, we expect a value of 2
        // (since two unique docs contain at least one value for this dim):
        Assert::assert_equals(
            "dim=a path=[] value=2 childCount=3\n  foo (2)\n  bar (1)\n  zoo (1)\n",
            top(f, 10, "a", &[])?.as_str(),
        );
        Assert::assert_equals(
            "dim=b path=[] value=1 childCount=1\n  baz (1)\n",
            top(f, 10, "b", &[])?.as_str(),
        );
        Assert::assert_equals(
            "dim=c path=[buzz] value=2 childCount=3\n  bif (2)\n  bee (1)\n  biz (1)\n",
            top(f, 10, "c", &["buzz"])?.as_str(),
        );
        Assert::assert_equals(
            "dim=c path=[buzz, bif] value=2 childCount=1\n  baf (2)\n",
            top(f, 10, "c", &["buzz", "bif"])?.as_str(),
        );

        assert_facet_result(
            &all(f, "a", &[])?,
            "a",
            &[],
            3,
            NumericValue::Int(2),
            &[lv("bar", 1), lv("foo", 2), lv("zoo", 1)],
        );

        assert_facet_result(
            &all(f, "c", &["buzz"])?,
            "c",
            &["buzz"],
            3,
            NumericValue::Int(2),
            &[lv("bee", 1), lv("bif", 2), lv("biz", 1)],
        );

        assert_facet_result(
            &all(f, "c", &["buzz", "bif"])?,
            "c",
            &["buzz", "bif"],
            1,
            NumericValue::Int(2),
            &[lv("baf", 2)],
        );

        // test getSpecificValue (and make sure hierarchical dims are supported: LUCENE-10584):
        Assert::assert_equals(NumericValue::Int(2), f.get_specific_value("c", &["buzz"])?);
        // should be able to request deeper paths on hierarchical dims:
        Assert::assert_equals(
            NumericValue::Int(1),
            f.get_specific_value("c", &["buzz", "bee"])?,
        );
        // ... but not on non-hierarchical dims:
        expect_throws(IAE, || f.get_specific_value("a", &["foo", "bar)"]));
        // DrillDown:
        let mut q = DrillDownQuery::new(Arc::clone(&config));
        q.add("a", &["foo"])?;
        q.add("b", &["baz"])?;
        Assert::assert_equals(1, hits(&searcher, &q, 1)?);

        let q = ddq(&config, "c", &["buzz", "bif"])?;
        Assert::assert_equals(2, hits(&searcher, &q, 2)?);

        let q = ddq(&config, "c", &["buzz", "biz", "bar"])?;
        Assert::assert_equals(1, hits(&searcher, &q, 2)?);
    }
    r.close()?;
    res.close()
}

/// The `ConcurrentSortedSetDocValuesFacetCounts` part shared by
/// `testCountAll` and `testHierarchicalCountAll`: `checks` asserts the top
/// children, then `getTopDims(n, 10)` is compared with `getAllDims(10)`.
fn check_concurrent_count_all(
    state: &Arc<dyn SortedSetDocValuesReaderState>,
    checks: impl Fn(&dyn Facets) -> Result<()>,
) -> Result<()> {
    let exec = new_executor();
    let facets =
        ConcurrentSortedSetDocValuesFacetCounts::new(Arc::clone(state), exec.exec().unwrap())?;

    checks(&facets)?;

    // test getTopDims in ConcurrentSortedSetDocValuesFacetCounts
    let results = facets.get_all_dims(10)?;
    // test getTopDims(10, 10) and expect same results from getAllDims(10)
    let all_top_dims_results = facets.get_top_dims(10, 10)?;

    // test getTopDims(n, 10)
    if !all_top_dims_results.is_empty() {
        for i in 1..results.len() {
            assert_equals(&results[..i].to_vec(), &facets.get_top_dims(i as i32, 10)?);
        }
    }
    Ok(())
}

// See: LUCENE-10070
fn test_count_all() -> Result<()> {
    let res = Resources::open()?;
    let writer = &res.writer;
    let config = Arc::new(FacetsConfig::new());

    let mut doc = Document::new();
    doc.add(StringField::new("id", "0".to_string(), Store::NO)?);
    doc.add(ssdv("a", &["foo"])?);
    writer.add_document(&config.build(&doc)?)?;

    let mut doc = Document::new();
    doc.add(StringField::new("id", "1".to_string(), Store::NO)?);
    doc.add(ssdv("a", &["bar"])?);
    writer.add_document(&config.build(&doc)?)?;

    writer.delete_documents(Term::from_text("id", "0"))?;

    // NRT open
    let r = writer.get_reader()?;
    {
        let searcher = LuceneTestCase::new_searcher(Arc::clone(&r));

        // Per-top-reader state:
        let state = new_state(&searcher, &config)?;

        let facets = SortedSetDocValuesFacetCounts::new(Arc::clone(&state))?;

        Assert::assert_equals(
            "dim=a path=[] value=1 childCount=1\n  bar (1)\n",
            top(&facets, 10, "a", &[])?.as_str(),
        );

        // test getAllChildren
        assert_facet_result(
            &all(&facets, "a", &[])?,
            "a",
            &[],
            1,
            NumericValue::Int(1),
            &[lv("bar", 1)],
        );

        // test topNChildren = 0
        expect_throws(IAE, || facets.get_top_children(0, "a", &[]));

        check_concurrent_count_all(&state, |facets| {
            Assert::assert_equals(
                "dim=a path=[] value=1 childCount=1\n  bar (1)\n",
                top(facets, 10, "a", &[])?.as_str(),
            );
            Ok(())
        })?;
    }
    r.close()?;
    res.close()
}

fn test_hierarchical_count_all() -> Result<()> {
    let res = Resources::open()?;
    let writer = &res.writer;
    let config = FacetsConfig::new();
    config.set_hierarchical("b", true);
    let config = Arc::new(config);

    for (id, a, b) in [
        ("0", "foo", &["buzz", "bee"][..]),
        ("1", "bar", &["buzz", "baz"]),
        ("2", "buz", &["bar", "foo"]),
        ("2", "baz", &["bar"]),
    ] {
        let mut doc = Document::new();
        doc.add(StringField::new("id", id.to_string(), Store::NO)?);
        doc.add(ssdv("a", &[a])?);
        doc.add(ssdv("b", b)?);
        writer.add_document(&config.build(&doc)?)?;
    }

    writer.delete_documents(Term::from_text("id", "0"))?;

    // NRT open
    let r = writer.get_reader()?;
    {
        let searcher = LuceneTestCase::new_searcher(Arc::clone(&r));

        // Per-top-reader state:
        let state = new_state(&searcher, &config)?;

        let facets = SortedSetDocValuesFacetCounts::new(Arc::clone(&state))?;

        Assert::assert_equals(
            "dim=a path=[] value=3 childCount=3\n  bar (1)\n  baz (1)\n  buz (1)\n",
            top(&facets, 10, "a", &[])?.as_str(),
        );
        Assert::assert_equals(
            "dim=b path=[buzz] value=1 childCount=1\n  baz (1)\n",
            top(&facets, 10, "b", &["buzz"])?.as_str(),
        );

        assert_facet_result(
            &all(&facets, "a", &[])?,
            "a",
            &[],
            3,
            NumericValue::Int(3),
            &[lv("bar", 1), lv("baz", 1), lv("buz", 1)],
        );

        assert_facet_result(
            &all(&facets, "b", &[])?,
            "b",
            &[],
            2,
            NumericValue::Int(3),
            &[lv("bar", 2), lv("buzz", 1)],
        );

        check_concurrent_count_all(&state, |facets| {
            Assert::assert_equals(
                "dim=a path=[] value=3 childCount=3\n  bar (1)\n  baz (1)\n  buz (1)\n",
                top(facets, 10, "a", &[])?.as_str(),
            );
            Assert::assert_equals(
                "dim=b path=[buzz] value=1 childCount=1\n  baz (1)\n",
                top(facets, 10, "b", &["buzz"])?.as_str(),
            );
            Ok(())
        })?;
    }
    r.close()?;
    res.close()
}

fn test_basic_single_valued() -> Result<()> {
    let config = FacetsConfig::new();
    config.set_multi_valued("a", false);
    let config = Arc::new(config);
    let res = Resources::open()?;
    let writer = &res.writer;
    let mut doc = Document::new();
    doc.add(ssdv("a", &["foo"])?);
    doc.add(ssdv("b", &["bar"])?);
    writer.add_document(&config.build(&doc)?)?;
    let mut doc = Document::new();
    doc.add(ssdv("a", &["foo"])?);
    writer.add_document(&config.build(&doc)?)?;
    if LuceneTestCase::random().next_boolean() {
        writer.commit()?;
    }

    let mut doc = Document::new();
    doc.add(ssdv("a", &["baz"])?);
    writer.add_document(&config.build(&doc)?)?;

    // NRT open
    let r = writer.get_reader()?;
    {
        let searcher = LuceneTestCase::new_searcher(Arc::clone(&r));

        // Per-top-reader state:
        let state = new_state(&searcher, &config)?;

        let exec = random_executor_service_or_null();
        let facets = get_all_facets(&searcher, &state, exec.exec())?;
        let f = &*facets;

        Assert::assert_equals(
            "dim=a path=[] value=3 childCount=2\n  foo (2)\n  baz (1)\n",
            top(f, 10, "a", &[])?.as_str(),
        );
        Assert::assert_equals(
            "dim=b path=[] value=1 childCount=1\n  bar (1)\n",
            top(f, 10, "b", &[])?.as_str(),
        );
        assert_facet_result(
            &all(f, "a", &[])?,
            "a",
            &[],
            2,
            NumericValue::Int(3),
            &[lv("baz", 1), lv("foo", 2)],
        );

        // DrillDown:
        let mut q = DrillDownQuery::new(Arc::clone(&config));
        q.add("a", &["foo"])?;
        q.add("b", &["bar"])?;
        Assert::assert_equals(1, hits(&searcher, &q, 1)?);
    }
    r.close()?;
    res.close()
}

fn test_hierarchical_basic_single_values() -> Result<()> {
    let config = FacetsConfig::new();
    config.set_hierarchical("c", true);
    let config = Arc::new(config);
    let res = Resources::open()?;
    let writer = &res.writer;
    for path in [&["buzz", "bar"][..], &["buzz", "buz"], &["buz", "baz"]] {
        let mut doc = Document::new();
        doc.add(ssdv("c", path)?);
        writer.add_document(&config.build(&doc)?)?;
    }
    if LuceneTestCase::random().next_boolean() {
        writer.commit()?;
    }

    let mut doc = Document::new();
    doc.add(ssdv("c", &["baz"])?);
    writer.add_document(&config.build(&doc)?)?;

    // NRT open
    let r = writer.get_reader()?;
    {
        let searcher = LuceneTestCase::new_searcher(Arc::clone(&r));

        // Per-top-reader state:
        let state = new_state(&searcher, &config)?;

        let exec = random_executor_service_or_null();
        let facets = get_all_facets(&searcher, &state, exec.exec())?;
        let f = &*facets;

        Assert::assert_equals(
            "dim=c path=[buzz] value=2 childCount=2\n  bar (1)\n  buz (1)\n",
            top(f, 10, "c", &["buzz"])?.as_str(),
        );
        Assert::assert_equals(
            "dim=c path=[buzz] value=2 childCount=2\n  bar (1)\n  buz (1)\n",
            top(f, 10, "c", &["buzz"])?.as_str(),
        );

        let q = ddq(&config, "c", &["buzz"])?;
        Assert::assert_equals(2, hits(&searcher, &q, 1)?);

        let q = ddq(&config, "c", &["buzz", "bar"])?;
        Assert::assert_equals(1, hits(&searcher, &q, 1)?);
    }
    r.close()?;
    res.close()
}

/// `config.setDrillDownTermsIndexing` for dims `c` to `g`, as both
/// drill-down option tests configure them.
fn drill_down_options_config() -> FacetsConfig {
    let config = FacetsConfig::new();
    config.set_drill_down_terms_indexing("c", DrillDownTermsIndexing::NONE);
    config.set_drill_down_terms_indexing("d", DrillDownTermsIndexing::DIMENSION_AND_FULL_PATH);
    config.set_drill_down_terms_indexing("e", DrillDownTermsIndexing::ALL_PATHS_NO_DIM);
    config.set_drill_down_terms_indexing("f", DrillDownTermsIndexing::FULL_PATH_ONLY);
    config.set_drill_down_terms_indexing("g", DrillDownTermsIndexing::ALL);
    config
}

/// The body shared by the drill-down option tests: index `path` under dims
/// `c` to `g` (then `a/foo`), and check each `(dim, path, expected hits)`.
fn check_drill_down_options(
    config: FacetsConfig,
    path: &[&str],
    expected: &[(&str, &[&str], i64)],
) -> Result<()> {
    let config = Arc::new(config);
    let res = Resources::open()?;
    let writer = &res.writer;

    let mut doc = Document::new();
    for dim in ["c", "d", "e", "f", "g"] {
        doc.add(ssdv(dim, path)?);
    }
    writer.add_document(&config.build(&doc)?)?;
    if LuceneTestCase::random().next_boolean() {
        writer.commit()?;
    }

    let mut doc = Document::new();
    doc.add(ssdv("a", &["foo"])?);
    writer.add_document(&config.build(&doc)?)?;

    // NRT open
    let r = writer.get_reader()?;
    {
        let searcher = LuceneTestCase::new_searcher(Arc::clone(&r));
        // Drill down with different indexing configuration options
        for (dim, path, expected_hits) in expected {
            let q = ddq(&config, dim, path)?;
            Assert::assert_equals(*expected_hits, hits(&searcher, &q, 1)?);
        }
    }
    r.close()?;
    res.close()
}

fn test_drill_down_options() -> Result<()> {
    check_drill_down_options(
        drill_down_options_config(),
        &["foo"],
        &[
            ("c", &[], 0),
            ("c", &["foo"], 0),
            ("d", &[], 1),
            ("d", &["foo"], 1),
            ("e", &[], 0),
            ("e", &["foo"], 1),
            ("f", &[], 0),
            ("f", &["foo"], 1),
            ("g", &[], 1),
            ("g", &["foo"], 1),
        ],
    )
}

fn test_hierarchical_drill_down_options() -> Result<()> {
    let config = drill_down_options_config();
    config.set_hierarchical("c", true);
    config.set_hierarchical("d", true);
    config.set_hierarchical("e", true);
    config.set_hierarchical("f", true);
    config.set_hierarchical("g", true);
    check_drill_down_options(
        config,
        &["biz", "baz"],
        &[
            ("c", &[], 0),
            ("c", &["biz"], 0),
            ("c", &["biz", "baz"], 0),
            ("c", &["foo"], 0),
            ("d", &[], 1),
            ("d", &["foo"], 0),
            ("d", &["biz"], 0),
            ("d", &["biz", "baz"], 1),
            ("e", &[], 0),
            ("e", &["foo"], 0),
            ("e", &["biz"], 1),
            ("e", &["biz", "baz"], 1),
            ("f", &[], 0),
            ("f", &["foo"], 0),
            ("f", &["biz"], 0),
            ("f", &["biz", "baz"], 1),
            ("g", &[], 1),
            ("g", &["foo"], 0),
            ("g", &["biz"], 1),
            ("g", &["biz", "baz"], 1),
        ],
    )
}

// LUCENE-5090
fn test_stale_state() -> Result<()> {
    let res = Resources::open()?;
    let writer = &res.writer;

    let config = Arc::new(FacetsConfig::new());

    let mut doc = Document::new();
    doc.add(ssdv("a", &["foo"])?);
    writer.add_document(&config.build(&doc)?)?;

    let r = writer.get_reader()?;
    {
        let state: Arc<dyn SortedSetDocValuesReaderState> =
            Arc::new(DefaultSortedSetDocValuesReaderState::new(
                Arc::clone(&r) as Arc<dyn IndexReader>,
                Arc::clone(&config),
            )?);

        let mut doc = Document::new();
        doc.add(ssdv("a", &["bar"])?);
        writer.add_document(&config.build(&doc)?)?;

        let mut doc = Document::new();
        doc.add(ssdv("a", &["baz"])?);
        writer.add_document(&config.build(&doc)?)?;

        let r2 = writer.get_reader()?;
        {
            let searcher = LuceneTestCase::new_searcher(Arc::clone(&r2));

            let c = searcher.search_with_manager(
                Arc::new(MatchAllDocsQuery::instance()),
                &FacetsCollectorManager::new(),
            )?;

            expect_throws("java.lang.IllegalStateException", || {
                SortedSetDocValuesFacetCounts::with_hits(Arc::clone(&state), Some(&c))
            });
        }
        r2.close()?;
    }
    r.close()?;
    res.close()
}

/// `state.getChildResources().toString()`: `Collection.toString()` of the
/// resources' `toString()`s.
fn child_resources_to_string(state: &dyn SortedSetDocValuesReaderState) -> (bool, String) {
    let resources = state.child_resources();
    let rendered: Vec<String> = resources
        .iter()
        .map(|r| r.accountable_to_string())
        .collect();
    (resources.is_empty(), format!("[{}]", rendered.join(", ")))
}

/// The accounting assertions closing the two sparse-facets tests.
fn check_state_accounting(
    searcher: &IndexSearcher,
    state: &dyn SortedSetDocValuesReaderState,
) -> Result<()> {
    let (resources_empty, resources) = child_resources_to_string(state);
    Assert::assert_true(state.to_string().contains(DEFAULT_INDEX_FIELD_NAME));
    if Arc::clone(searcher.get_index_reader()).leaves()?.len() > 1 {
        Assert::assert_true(state.ram_bytes_used() > 0);
        Assert::assert_false(resources_empty);
        Assert::assert_true(resources.contains(DEFAULT_INDEX_FIELD_NAME));
    } else {
        Assert::assert_equals(0, state.ram_bytes_used());
        Assert::assert_true(resources_empty);
    }
    Ok(())
}

// LUCENE-5333
fn test_sparse_facets() -> Result<()> {
    let res = Resources::open()?;
    let writer = &res.writer;

    let config = Arc::new(FacetsConfig::new());

    let mut doc = Document::new();
    doc.add(ssdv("a", &["foo1"])?);
    writer.add_document(&config.build(&doc)?)?;

    if LuceneTestCase::random().next_boolean() {
        writer.commit()?;
    }

    let mut doc = Document::new();
    doc.add(ssdv("a", &["foo2"])?);
    doc.add(ssdv("b", &["bar1"])?);
    writer.add_document(&config.build(&doc)?)?;

    if LuceneTestCase::random().next_boolean() {
        writer.commit()?;
    }

    let mut doc = Document::new();
    doc.add(ssdv("a", &["foo3"])?);
    doc.add(ssdv("b", &["bar2"])?);
    doc.add(ssdv("c", &["baz1"])?);
    doc.add(ssdv("d", &["biz1"])?);
    writer.add_document(&config.build(&doc)?)?;

    let mut doc = Document::new();
    doc.add(ssdv("d", &["biz2"])?);
    writer.add_document(&config.build(&doc)?)?;

    // NRT open
    let r = writer.get_reader()?;
    {
        let searcher = LuceneTestCase::new_searcher(Arc::clone(&r));

        // Per-top-reader state:
        let state = new_state(&searcher, &config)?;

        let exec = random_executor_service_or_null();
        let facets = get_all_facets(&searcher, &state, exec.exec())?;
        let f = &*facets;

        // Ask for top 10 labels for any dims that have counts:
        let results = f.get_all_dims(10)?;

        Assert::assert_equals(4, results.len());
        Assert::assert_equals(
            "dim=a path=[] value=3 childCount=3\n  foo1 (1)\n  foo2 (1)\n  foo3 (1)\n",
            results[0].to_string().as_str(),
        );
        Assert::assert_equals(
            "dim=b path=[] value=2 childCount=2\n  bar1 (1)\n  bar2 (1)\n",
            results[1].to_string().as_str(),
        );
        Assert::assert_equals(
            "dim=d path=[] value=2 childCount=2\n  biz1 (1)\n  biz2 (1)\n",
            results[2].to_string().as_str(),
        );
        Assert::assert_equals(
            "dim=c path=[] value=1 childCount=1\n  baz1 (1)\n",
            results[3].to_string().as_str(),
        );

        // test getAllDims with topN = 1, sort by dim names when values are equal
        let top1results = f.get_all_dims(1)?;
        Assert::assert_equals(4, results.len());
        Assert::assert_equals(
            "dim=a path=[] value=3 childCount=3\n  foo1 (1)\n",
            top1results[0].to_string().as_str(),
        );
        Assert::assert_equals(
            "dim=b path=[] value=2 childCount=2\n  bar1 (1)\n",
            top1results[1].to_string().as_str(),
        );
        Assert::assert_equals(
            "dim=d path=[] value=2 childCount=2\n  biz1 (1)\n",
            top1results[2].to_string().as_str(),
        );
        Assert::assert_equals(
            "dim=c path=[] value=1 childCount=1\n  baz1 (1)\n",
            top1results[3].to_string().as_str(),
        );

        // test getTopDims(1, 1)
        let top_dims_results1 = f.get_top_dims(1, 1)?;
        Assert::assert_equals(1, top_dims_results1.len());
        Assert::assert_equals(
            "dim=a path=[] value=3 childCount=3\n  foo1 (1)\n",
            top_dims_results1[0].to_string().as_str(),
        );

        // test top 2 dims that have the same counts, expect to sort by dim names
        let top_dims_results2 = f.get_top_dims(3, 2)?;
        Assert::assert_equals(3, top_dims_results2.len());
        Assert::assert_equals(
            "dim=a path=[] value=3 childCount=3\n  foo1 (1)\n  foo2 (1)\n",
            top_dims_results2[0].to_string().as_str(),
        );
        Assert::assert_equals(
            "dim=b path=[] value=2 childCount=2\n  bar1 (1)\n  bar2 (1)\n",
            top_dims_results2[1].to_string().as_str(),
        );
        Assert::assert_equals(
            "dim=d path=[] value=2 childCount=2\n  biz1 (1)\n  biz2 (1)\n",
            top_dims_results2[2].to_string().as_str(),
        );

        assert_facet_result(
            &all(f, "a", &[])?,
            "a",
            &[],
            3,
            NumericValue::Int(3),
            &[lv("foo1", 1), lv("foo2", 1), lv("foo3", 1)],
        );

        assert_facet_result(
            &all(f, "b", &[])?,
            "b",
            &[],
            2,
            NumericValue::Int(2),
            &[lv("bar1", 1), lv("bar2", 1)],
        );

        assert_facet_result(
            &all(f, "c", &[])?,
            "c",
            &[],
            1,
            NumericValue::Int(1),
            &[lv("baz1", 1)],
        );

        assert_facet_result(
            &all(f, "d", &[])?,
            "d",
            &[],
            2,
            NumericValue::Int(2),
            &[lv("biz1", 1), lv("biz2", 1)],
        );

        check_state_accounting(&searcher, &*state)?;
    }
    r.close()?;
    res.close()
}

fn test_hierarchical_sparse_facets() -> Result<()> {
    let res = Resources::open()?;
    let writer = &res.writer;

    let config = FacetsConfig::new();
    config.set_hierarchical("d", true);
    config.set_hierarchical("e", true);
    let config = Arc::new(config);

    let mut doc = Document::new();
    doc.add(ssdv("d", &["foo", "bar"])?);
    writer.add_document(&config.build(&doc)?)?;

    if LuceneTestCase::random().next_boolean() {
        writer.commit()?;
    }

    let mut doc = Document::new();
    doc.add(ssdv("d", &["foo", "baz"])?);
    writer.add_document(&config.build(&doc)?)?;

    if LuceneTestCase::random().next_boolean() {
        writer.commit()?;
    }

    let mut doc = Document::new();
    doc.add(ssdv("e", &["biz", "baz"])?);
    writer.add_document(&config.build(&doc)?)?;

    // NRT open
    let r = writer.get_reader()?;
    {
        let searcher = LuceneTestCase::new_searcher(Arc::clone(&r));

        // Per-top-reader state:
        let state = new_state(&searcher, &config)?;

        let exec = random_executor_service_or_null();
        let facets = get_all_facets(&searcher, &state, exec.exec())?;
        let f = &*facets;

        // Ask for top 10 labels for any dims that have counts:
        let results = f.get_all_dims(10)?;

        Assert::assert_equals(2, results.len());
        Assert::assert_equals(
            "dim=d path=[] value=2 childCount=1\n  foo (2)\n",
            results[0].to_string().as_str(),
        );
        Assert::assert_equals(
            "dim=e path=[] value=1 childCount=1\n  biz (1)\n",
            results[1].to_string().as_str(),
        );

        // test getTopDims(1, 1)
        let top_dims_results1 = f.get_top_dims(1, 1)?;
        Assert::assert_equals(1, top_dims_results1.len());
        Assert::assert_equals(
            "dim=d path=[] value=2 childCount=1\n  foo (2)\n",
            results[0].to_string().as_str(),
        );
        assert_facet_result(
            &all(f, "d", &["foo"])?,
            "d",
            &["foo"],
            2,
            NumericValue::Int(2),
            &[lv("bar", 1), lv("baz", 1)],
        );
        assert_facet_result(
            &all(f, "d", &[])?,
            "d",
            &[],
            1,
            NumericValue::Int(2),
            &[lv("foo", 2)],
        );

        check_state_accounting(&searcher, &*state)?;
    }
    r.close()?;
    res.close()
}

fn test_some_segments_missing() -> Result<()> {
    let res = Resources::open()?;
    let writer = &res.writer;

    let config = Arc::new(FacetsConfig::new());

    let mut doc = Document::new();
    doc.add(ssdv("a", &["foo1"])?);
    writer.add_document(&config.build(&doc)?)?;
    writer.commit()?;

    let doc = Document::new();
    writer.add_document(&config.build(&doc)?)?;
    writer.commit()?;

    let mut doc = Document::new();
    doc.add(ssdv("a", &["foo2"])?);
    writer.add_document(&config.build(&doc)?)?;
    writer.commit()?;

    // NRT open
    let r = writer.get_reader()?;
    {
        let searcher = LuceneTestCase::new_searcher(Arc::clone(&r));

        // Per-top-reader state:
        let state = new_state(&searcher, &config)?;

        let exec = random_executor_service_or_null();
        let facets = get_all_facets(&searcher, &state, exec.exec())?;
        let f = &*facets;

        // Ask for top 10 labels for any dims that have counts:
        Assert::assert_equals(
            "dim=a path=[] value=2 childCount=2\n  foo1 (1)\n  foo2 (1)\n",
            top(f, 10, "a", &[])?.as_str(),
        );
        assert_facet_result(
            &all(f, "a", &[])?,
            "a",
            &[],
            2,
            NumericValue::Int(2),
            &[lv("foo1", 1), lv("foo2", 1)],
        );
    }
    r.close()?;
    res.close()
}

fn test_hierarchical_some_segments_missing() -> Result<()> {
    let res = Resources::open()?;
    let writer = &res.writer;

    let config = FacetsConfig::new();
    config.set_hierarchical("b", true);
    config.set_multi_valued("b", true);
    let config = Arc::new(config);

    let mut doc = Document::new();
    doc.add(ssdv("a", &["foo1"])?);
    doc.add(ssdv("b", &["foo", "bar"])?);
    doc.add(ssdv("b", &["boo", "buzzz"])?);
    writer.add_document(&config.build(&doc)?)?;
    writer.commit()?;

    let doc = Document::new();
    writer.add_document(&config.build(&doc)?)?;
    writer.commit()?;

    let mut doc = Document::new();
    doc.add(ssdv("a", &["foo2"])?);
    doc.add(ssdv("b", &["foo", "buzz"])?);
    writer.add_document(&config.build(&doc)?)?;
    writer.commit()?;

    // NRT open
    let r = writer.get_reader()?;
    {
        let searcher = LuceneTestCase::new_searcher(Arc::clone(&r));

        // Per-top-reader state:
        let state = new_state(&searcher, &config)?;

        let exec = random_executor_service_or_null();
        let facets = get_all_facets(&searcher, &state, exec.exec())?;
        let f = &*facets;

        // Ask for top 10 labels for any dims that have counts:
        Assert::assert_equals(
            "dim=a path=[] value=2 childCount=2\n  foo1 (1)\n  foo2 (1)\n",
            top(f, 10, "a", &[])?.as_str(),
        );
        Assert::assert_equals(
            "dim=b path=[] value=2 childCount=2\n  foo (2)\n  boo (1)\n",
            top(f, 10, "b", &[])?.as_str(),
        );
        Assert::assert_equals(
            "dim=b path=[foo] value=2 childCount=2\n  bar (1)\n  buzz (1)\n",
            top(f, 10, "b", &["foo"])?.as_str(),
        );
        assert_facet_result(
            &all(f, "b", &[])?,
            "b",
            &[],
            2,
            NumericValue::Int(2),
            &[lv("boo", 1), lv("foo", 2)],
        );
    }
    r.close()?;
    res.close()
}

/// `FacetsCollectorManager.search(searcher, new TermQuery(new
/// Term("content", searchToken)), 10, new FacetsCollectorManager())
/// .facetsCollector()`, then the `SortedSetDocValuesFacetCounts` (or the
/// concurrent one when `exec` is set) over it.
fn search_facets(
    searcher: &IndexSearcher,
    state: &Arc<dyn SortedSetDocValuesReaderState>,
    exec: Option<Arc<dyn Executor>>,
    search_token: &str,
) -> Result<Box<dyn Facets>> {
    let fc = FacetsCollectorManager::search(
        searcher,
        Arc::new(TermQuery::new(Term::from_text("content", search_token))),
        10,
        &FacetsCollectorManager::new(),
    )?
    .facets_collector;
    Ok(match exec {
        Some(exec) => Box::new(ConcurrentSortedSetDocValuesFacetCounts::with_hits(
            Arc::clone(state),
            Some(&fc),
            exec,
        )?),
        None => Box::new(SortedSetDocValuesFacetCounts::with_hits(
            Arc::clone(state),
            Some(&fc),
        )?),
    })
}

/// `random().nextInt(10) < 8 ? TestUtil.nextInt(random(), 1, 7) : 1`.
fn random_num_dims() -> usize {
    let random = LuceneTestCase::random();
    if random.next_int_bound(10) < 8 {
        TestUtil::next_int(&*random, 1, 7) as usize
    } else {
        1
    }
}

fn test_random() -> Result<()> {
    let full_iterations = if *TEST_NIGHTLY { 20 } else { 3 };
    for _ in 0..full_iterations {
        let tokens = get_random_tokens(10);

        let index_dir = LuceneTestCase::new_directory();
        let w = RandomIndexWriter::new_with_mock_analyzer(
            &*LuceneTestCase::random(),
            index_dir.as_directory(),
        )?;
        let config = Arc::new(FacetsConfig::new());
        let num_docs = LuceneTestCase::at_least(1000) as usize;
        // Most of the time allow up to 7 dims per doc, but occasionally limit all docs to a single
        // dim:
        let num_dims = random_num_dims();
        let test_docs = get_random_docs(&tokens, num_docs, num_dims);
        for test_doc in &test_docs {
            let mut doc = Document::new();
            doc.add(LuceneTestCase::new_string_field(
                "content",
                &test_doc.content,
                Store::NO,
            ));
            for (j, dim) in test_doc.dims.iter().enumerate().take(num_dims) {
                if let Some(dim) = dim {
                    doc.add(ssdv(&format!("dim{j}"), &[dim.as_str()])?);
                }
            }
            w.add_document(&config.build(&doc)?)?;
        }

        // NRT open
        let r = w.get_reader()?;
        {
            let searcher = LuceneTestCase::new_searcher(Arc::clone(&r));

            // Per-top-reader state:
            let state = new_state(&searcher, &config)?;
            let exec = random_executor_service_or_null();
            let iters = LuceneTestCase::at_least(100);
            for _ in 0..iters {
                let search_token =
                    &tokens[LuceneTestCase::random().next_int_bound(tokens.len() as i32) as usize];
                if *VERBOSE {
                    println!("\nTEST: iter content={search_token}");
                }
                let facets = search_facets(&searcher, &state, exec.exec(), search_token)?;

                // Slow, yet hopefully bug-free, faceting:
                let mut expected_counts: Vec<HashMap<String, i32>> =
                    (0..num_dims).map(|_| HashMap::new()).collect();

                for doc in &test_docs {
                    if doc.content == *search_token {
                        for j in 0..num_dims {
                            if let Some(dim) = &doc.dims[j] {
                                *expected_counts[j].entry(dim.clone()).or_insert(0) += 1;
                            }
                        }
                    }
                }

                let mut expected: Vec<FacetResult> = Vec::new();
                for (i, counts) in expected_counts.iter().enumerate() {
                    let mut label_values: Vec<LabelAndValue> = Vec::new();
                    let mut tot_count = 0;
                    for (key, value) in counts {
                        label_values.push(lv(key, *value));
                        tot_count += value;
                    }
                    sort_label_values(&mut label_values);
                    if tot_count > 0 {
                        let size = label_values.len() as i32;
                        expected.push(FacetResult::new(
                            format!("dim{i}"),
                            Vec::new(),
                            NumericValue::Int(tot_count),
                            label_values,
                            size,
                        ));
                    }
                }

                // Sort by highest value, tie-break by value:
                sort_facet_results(&mut expected);

                let actual = facets.get_all_dims(10)?;
                // Messy: fixup ties
                // sortTies(actual);

                assert_equals(&expected, &actual);

                // test getTopDims(1, 10)
                if !actual.is_empty() {
                    let top_dims_results1 = facets.get_top_dims(1, 10)?;
                    assert_equals(&actual[0], &top_dims_results1[0]);
                }
            }
        }
        r.close()?;
        w.close()?;
        index_dir.close()?;
    }
    Ok(())
}

/// `Arrays.sort(labelValues, ...)` of `getAllChildren`'s check: by label
/// (`String.compareTo`, UTF-16 order), then by value high-to-low.
fn sort_by_label_then_value(label_values: &mut [LabelAndValue]) {
    label_values.sort_by(|a, b| {
        a.label
            .encode_utf16()
            .cmp(b.label.encode_utf16()) // low-to-high
            .then_with(|| number_long_value(&b.value).cmp(&number_long_value(&a.value)))
    });
}

fn test_random_hierarchical_flat_mix() -> Result<()> {
    let full_iterations = if *TEST_NIGHTLY { 20 } else { 3 };
    for _ in 0..full_iterations {
        let tokens = get_random_tokens(10);

        let index_dir = LuceneTestCase::new_directory();
        let w = RandomIndexWriter::new_with_mock_analyzer(
            &*LuceneTestCase::random(),
            index_dir.as_directory(),
        )?;
        let config = FacetsConfig::new();
        let num_docs = LuceneTestCase::at_least(1000) as usize;
        // Most of the time allow up to 7 dims per doc, but occasionally limit all docs to a single
        // dim:
        let num_dims = random_num_dims();
        let mut hierarchical_dims = vec![false; num_dims];
        for (i, hierarchical) in hierarchical_dims.iter_mut().enumerate() {
            let is_hierarchical_dim = LuceneTestCase::random().next_boolean();
            config.set_hierarchical(&format!("dim{i}"), is_hierarchical_dim);
            *hierarchical = is_hierarchical_dim;
        }
        let config = Arc::new(config);
        let test_docs = get_random_docs(&tokens, num_docs, num_dims);
        // `Set<SortedSetDocValuesFacetField>`: the field has no `equals`, so
        // the set holds every added instance; its iteration order does not
        // affect the expected results built below.
        let mut test_doc_facets: Vec<Vec<SortedSetDocValuesFacetField>> = Vec::new();
        for test_doc in &test_docs {
            let mut doc = Document::new();
            let mut doc_facets: Vec<SortedSetDocValuesFacetField> = Vec::new();
            doc.add(LuceneTestCase::new_string_field(
                "content",
                &test_doc.content,
                Store::NO,
            ));
            for i in 0..num_dims {
                let dim = format!("dim{i}");
                if hierarchical_dims[i] {
                    let path_length = if num_dims == 1 {
                        1
                    } else {
                        LuceneTestCase::random().next_int_bound(num_dims as i32 - 1) as usize + 1
                    };
                    let mut path: Vec<&str> = Vec::new();
                    for label in test_doc.dims.iter().take(path_length).flatten() {
                        path.push(label.as_str());
                    }
                    doc.add(ssdv(&dim, &path)?);
                    for j in 0..path.len() {
                        doc_facets.push(ssdv(&dim, &path[..j + 1])?);
                    }
                } else if let Some(label) = &test_doc.dims[i] {
                    doc.add(ssdv(&dim, &[label.as_str()])?);
                    doc_facets.push(ssdv(&dim, &[label.as_str()])?);
                }
            }
            test_doc_facets.push(doc_facets);
            w.add_document(&config.build(&doc)?)?;
        }

        // NRT open
        let r = w.get_reader()?;
        {
            let searcher = LuceneTestCase::new_searcher(Arc::clone(&r));

            // Per-top-reader state:
            let state = new_state(&searcher, &config)?;
            let exec = random_executor_service_or_null();
            let iters = LuceneTestCase::at_least(100);
            for iter in 0..iters {
                let search_token =
                    &tokens[LuceneTestCase::random().next_int_bound(tokens.len() as i32) as usize];
                if *VERBOSE {
                    println!("\nTEST: iter content={search_token}");
                }
                let facets = search_facets(&searcher, &state, exec.exec(), search_token)?;
                // Slow, yet hopefully bug-free, faceting:
                let mut expected_results: HashMap<String, FacetResult> = HashMap::new();

                for (i, doc) in test_docs.iter().enumerate() {
                    if doc.content == *search_token {
                        // goes through all facets paths in the doc
                        for facet_field in &test_doc_facets[i] {
                            let path = &facet_field.path;
                            let parent_dim_path_string = if path.len() == 1 {
                                facet_field.dim.clone()
                            } else {
                                format!(
                                    "{}{}{}",
                                    facet_field.dim,
                                    DELIM_CHAR,
                                    FacetsConfig::path_to_string_len(path, path.len() - 1)?
                                )
                            };
                            let result = match expected_results.get(&parent_dim_path_string) {
                                Some(result) => result.clone(),
                                None => FacetResult::new(
                                    facet_field.dim.clone(),
                                    path[..path.len() - 1].to_vec(),
                                    NumericValue::Int(0),
                                    Vec::new(),
                                    0,
                                ),
                            };
                            let child = &path[path.len() - 1];
                            let mut label_and_values = result.label_values.clone();
                            let mut contains_child = false;
                            for lav in label_and_values.iter_mut() {
                                if lav.label == *child {
                                    contains_child = true;
                                    *lav = lv(&lav.label, number_int_value(&lav.value) + 1);
                                    break;
                                }
                            }
                            let mut child_count = result.child_count;
                            if !contains_child {
                                label_and_values.push(lv(child, 1));
                                child_count += 1;
                            }
                            label_and_values.sort_by(|o1, o2| {
                                if number_equals(&o1.value, &o2.value) {
                                    o1.label.as_bytes().cmp(o2.label.as_bytes())
                                } else {
                                    (number_int_value(&o2.value) - number_int_value(&o1.value))
                                        .cmp(&0)
                                }
                            });
                            let new_result = FacetResult::new(
                                result.dim.clone(),
                                result.path.clone(),
                                NumericValue::Int(0),
                                label_and_values,
                                child_count,
                            );
                            expected_results.insert(parent_dim_path_string, new_result);
                        }
                    }
                }

                // second pass to update values
                for (i, doc) in test_docs.iter().enumerate() {
                    if doc.content == *search_token {
                        let mut dims_counted: HashSet<String> = HashSet::new();
                        for facet_field in &test_doc_facets[i] {
                            let dim_path_string = FacetsConfig::path_to_string_dim(
                                &facet_field.dim,
                                &facet_field.path,
                            )?;
                            let result = expected_results.get(&dim_path_string).cloned();
                            let dim_result = expected_results.get(&facet_field.dim).cloned();
                            if let Some(result) = result {
                                expected_results.insert(
                                    dim_path_string,
                                    FacetResult::new(
                                        result.dim,
                                        result.path,
                                        NumericValue::Int(number_int_value(&result.value) + 1),
                                        result.label_values,
                                        result.child_count,
                                    ),
                                );
                            }
                            if let Some(dim_result) = dim_result {
                                if dims_counted.insert(facet_field.dim.clone()) {
                                    expected_results.insert(
                                        facet_field.dim.clone(),
                                        FacetResult::new(
                                            dim_result.dim,
                                            dim_result.path,
                                            NumericValue::Int(
                                                number_int_value(&dim_result.value) + 1,
                                            ),
                                            dim_result.label_values,
                                            dim_result.child_count,
                                        ),
                                    );
                                }
                            }
                        }
                    }
                }

                let expected: Vec<FacetResult> = expected_results.values().cloned().collect();

                let mut expected_all_dims: Vec<FacetResult> = Vec::new();
                for result in &expected {
                    if result.path.is_empty() {
                        expected_all_dims.push(result.clone());
                        if expected_all_dims.len() >= 10 {
                            break;
                        }
                    }
                }
                sort_facet_results(&mut expected_all_dims);

                let actual_all_dims = facets.get_all_dims(10)?;

                assert_equals(&expected_all_dims, &actual_all_dims);

                // test getTopDims(n, 10)
                if !actual_all_dims.is_empty() {
                    for i in 1..actual_all_dims.len() {
                        assert_equals(
                            &actual_all_dims[..i].to_vec(),
                            &facets.get_top_dims(i as i32, 10)?,
                        );
                    }
                }

                // Dfs through top children
                for dim_result in &actual_all_dims {
                    if config.get_dim_config(&dim_result.dim).hierarchical() {
                        let mut stack: VecDeque<Vec<String>> = VecDeque::new();
                        for label_and_value in &dim_result.label_values {
                            stack.push_back(vec![label_and_value.label.clone()]);
                        }
                        while let Some(curr_path) = stack.pop_front() {
                            let curr_path_refs: Vec<&str> =
                                curr_path.iter().map(String::as_str).collect();
                            let expected_result =
                                get_facet_result_for_path(&expected, &dim_result.dim, &curr_path);
                            let actual_result =
                                facets.get_top_children(10, &dim_result.dim, &curr_path_refs)?;
                            let check = || -> Result<()> {
                                assert_equals(&expected_result, &actual_result);
                                // test getAllChildren
                                let all_children_result =
                                    facets.get_all_children(&dim_result.dim, &curr_path_refs)?;
                                if let (Some(expected_result), Some(mut all_children_result)) =
                                    (expected_result.clone(), all_children_result)
                                {
                                    let mut expected_result = expected_result;
                                    // sort actual allChildrenResult labels by value, count (since
                                    // we have no insight into the ordinals assigned to the values,
                                    // we resort)
                                    sort_by_label_then_value(&mut all_children_result.label_values);
                                    // also sort expected labels by value to match the sorting
                                    // behavior of getAllChildren
                                    sort_by_label_then_value(&mut expected_result.label_values);
                                    assert_equals(&expected_result, &all_children_result);
                                }
                                Ok(())
                            };
                            // `catch (AssertionError e)`: print, then rethrow.
                            match std::panic::catch_unwind(std::panic::AssertUnwindSafe(check)) {
                                Ok(r) => r?,
                                Err(payload) => {
                                    println!("{iter}");
                                    println!(
                                        "{}",
                                        config.get_dim_config(&dim_result.dim).hierarchical()
                                    );
                                    std::panic::resume_unwind(payload);
                                }
                            }
                            if let Some(actual_result) = &actual_result {
                                for label_and_value in &actual_result.label_values {
                                    let mut path = curr_path.clone();
                                    path.push(label_and_value.label.clone());
                                    stack.push_back(path);
                                }
                            }
                        }
                    }
                }
            }
        }
        r.close()?;
        w.close()?;
        index_dir.close()?;
    }
    Ok(())
}

/// `getFacetResultForPath(List<FacetResult>, String, String[])`.
fn get_facet_result_for_path(
    all_paths: &[FacetResult],
    dim: &str,
    path: &[String],
) -> Option<FacetResult> {
    for result in all_paths {
        if path.is_empty() {
            if result.path.is_empty() && result.dim == dim {
                return Some(result.clone());
            }
        } else {
            let is_equal_path = path.len() == result.path.len()
                && path.iter().zip(&result.path).all(|(a, b)| a == b);
            if is_equal_path && result.dim == dim {
                return Some(result.clone());
            }
        }
    }
    None
}

/// The body shared by the non-existent dimension / path tests: index
/// `fields` into one committed doc, then run `checks` on all facets.
fn check_non_existent(
    config: FacetsConfig,
    fields: &[(&str, &[&str])],
    checks: impl Fn(&dyn Facets) -> Result<()>,
) -> Result<()> {
    let config = Arc::new(config);
    let res = Resources::open()?;
    let writer = &res.writer;

    let mut doc = Document::new();
    for (dim, path) in fields {
        doc.add(ssdv(dim, path)?);
    }
    writer.add_document(&config.build(&doc)?)?;
    writer.commit()?;

    let r = writer.get_reader()?;
    {
        let searcher = LuceneTestCase::new_searcher(Arc::clone(&r));

        let state = new_state(&searcher, &config)?;

        let exec = random_executor_service_or_null();
        let facets = get_all_facets(&searcher, &state, exec.exec())?;
        checks(&*facets)?;
    }
    r.close()?;
    res.close()
}

fn test_non_existent_dimension() -> Result<()> {
    check_non_existent(FacetsConfig::new(), &[("foo", &["bar"])], |facets| {
        let result = facets.get_top_children(5, "non-existent dimension", &[])?;

        // make sure the result is null (and no exception was thrown)
        assert_null(&result);
        let result = facets.get_all_children("non-existent dimension", &[])?;
        assert_null(&result);
        Ok(())
    })
}

fn test_hierarchical_non_existent_dimension() -> Result<()> {
    let config = FacetsConfig::new();
    config.set_hierarchical("fizz", true);
    check_non_existent(
        config,
        &[("foo", &["bar"]), ("fizz", &["buzz", "baz"])],
        |facets| {
            let result = facets.get_top_children(5, "non-existent dimension", &[])?;

            // make sure the result is null (and no exception was thrown)
            assert_null(&result);
            let result = facets.get_all_children("non-existent dimension", &[])?;
            assert_null(&result);

            expect_throws(IAE, || {
                facets.get_top_children(5, "non-existent dimension", &["with a path"])
            });
            Ok(())
        },
    )
}

fn test_hierarchical_non_existent_path() -> Result<()> {
    let config = FacetsConfig::new();
    config.set_hierarchical("fizz", true);
    check_non_existent(config, &[("fizz", &["buzz", "baz"])], |facets| {
        let result = facets.get_top_children(5, "fizz", &["fake", "path"])?;

        // make sure the result is null (and no exception was thrown)
        assert_null(&result);
        let result = facets.get_all_children("fizz", &["fake", "path"])?;
        assert_null(&result);
        Ok(())
    })
}
