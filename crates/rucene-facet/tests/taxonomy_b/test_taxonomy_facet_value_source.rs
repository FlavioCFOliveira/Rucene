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

//! Port of `org.apache.lucene.facet.taxonomy.TestTaxonomyFacetValueSource`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::NumericValue;
use rucene::document::{Document, FloatDocValuesField, NumericDocValuesField, Store, StringField};
use rucene::error::Result;
use rucene::index::index_writer_config::OpenMode;
use rucene::index::{directory_reader, index_writer::IndexWriter, Term};
use rucene::search::boost_query::BoostQuery;
use rucene::search::constant_score_query::ConstantScoreQuery;
use rucene::search::double_values_source::{self, DoubleValuesSource};
use rucene::search::query::Query;
use rucene::search::term_query::TermQuery;
use rucene::search::MatchAllDocsQuery;
use rucene::util::java_collections::JavaHashMap;
use rucene_facet::facet_field::FacetField;
use rucene_facet::facet_result::FacetResult;
use rucene_facet::facets::Facets;
use rucene_facet::facets_collector::FacetsCollector;
use rucene_facet::facets_collector_manager::{FacetsCollectorManager, FacetsTopDocs};
use rucene_facet::facets_config::FacetsConfig;
use rucene_facet::label_and_value::{number_double_value, number_int_value, LabelAndValue};
use rucene_facet::taxonomy::association_aggregation_function::{
    AssociationAggregationFunction, MaxAggregation, SumAggregation,
};
use rucene_facet::taxonomy::directory::directory_taxonomy_reader::DirectoryTaxonomyReader;
use rucene_facet::taxonomy::directory::directory_taxonomy_writer::DirectoryTaxonomyWriter;
use rucene_facet::taxonomy::taxonomy_facet_float_associations::TaxonomyFacetFloatAssociations;
use rucene_facet::taxonomy::taxonomy_reader::TaxonomyReader;
use rucene_facet::taxonomy::taxonomy_writer::TaxonomyWriter;
use rucene_queries::function::function_query::FunctionQuery;
use rucene_queries::function::valuesource::long_field_source::LongFieldSource;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::lucene_test_case::VERBOSE;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::facet_support::core_support::{assert_equals, test};
use crate::facet_support::{
    assert_float_values_equals_list, facet_class, get_random_docs, get_random_tokens,
    get_taxonomy_facet_counts, sort_facet_results, sort_label_values, sort_ties_results, TestDoc,
};

const IAE: &str = "java.lang.IllegalArgumentException";

/// `TestTaxonomyFacetValueSource`.
pub fn class_def() -> Arc<Class> {
    facet_class(
        "org.apache.lucene.facet.taxonomy.TestTaxonomyFacetValueSource",
        vec![
            test("testBasic", test_basic),
            test("testSparseFacets", test_sparse_facets),
            test("testWrongIndexFieldName", test_wrong_index_field_name),
            test("testScoreAggregator", test_score_aggregator),
            test("testNoScore", test_no_score),
            test("testWithScore", test_with_score),
            test("testRollupValues", test_rollup_values),
            test(
                "testChildrenAndSiblingsLoaded",
                test_children_and_siblings_loaded,
            ),
            test("testCountAndSumScore", test_count_and_sum_score),
            test("testRandom", test_random),
        ],
    )
}

fn sum() -> Arc<dyn AssociationAggregationFunction> {
    Arc::new(SumAggregation)
}

fn max() -> Arc<dyn AssociationAggregationFunction> {
    Arc::new(MaxAggregation)
}

/// `facets.getTopChildren(topN, dim).toString()`.
fn top(facets: &dyn Facets, top_n: i32, dim: &str) -> Result<String> {
    Ok(facets
        .get_top_children(top_n, dim, &[])?
        .expect("java.lang.NullPointerException")
        .to_string())
}

fn match_all() -> Arc<dyn Query> {
    Arc::new(MatchAllDocsQuery::instance())
}

fn mock_iwc() -> rucene::index::index_writer_config::IndexWriterConfig {
    let random = LuceneTestCase::random();
    LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(MockAnalyzer::new(&*random))))
}

fn test_basic() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let taxo_dir = LuceneTestCase::new_directory();

    // Writes facet ords to a separate directory from the
    // main index:
    let taxo_writer = Arc::new(DirectoryTaxonomyWriter::with_open_mode(
        taxo_dir.as_directory(),
        OpenMode::CREATE,
    )?);

    let writer =
        RandomIndexWriter::new_with_mock_analyzer(&*LuceneTestCase::random(), dir.as_directory())?;
    let config = FacetsConfig::new();

    // Reused across documents, to add the necessary facet
    // fields:
    for (num, author) in [
        (10, "Bob"),
        (20, "Lisa"),
        (30, "Lisa"),
        (40, "Susan"),
        (45, "Frank"),
    ] {
        let mut doc = Document::new();
        doc.add(NumericDocValuesField::new("num", num));
        doc.add(FacetField::new("Author", &[author])?);
        writer.add_document(&config.build_with_taxonomy(Some(&*taxo_writer), &doc)?)?;
    }

    // NRT open
    let r = writer.get_reader()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&r));
    writer.close()?;

    // NRT open
    let taxo_reader: Arc<dyn TaxonomyReader> = Arc::new(DirectoryTaxonomyReader::from_writer(
        Arc::clone(&taxo_writer),
    )?);
    taxo_writer.close()?;

    // MatchAllDocsQuery is for "browsing" (counts facets
    // for all non-deleted docs in the index); normally
    // you'd use a "normal" query and one of the
    // Facets.search utility methods:
    let c = searcher.search_with_manager(match_all(), &FacetsCollectorManager::new())?;

    let facets_config = Arc::new(FacetsConfig::new());
    let values_source = double_values_source::from_int_field("num");

    // Test SUM:
    let facets = TaxonomyFacetFloatAssociations::with_values_source(
        Arc::clone(&taxo_reader),
        Arc::clone(&facets_config),
        &c,
        sum(),
        Arc::clone(&values_source),
    )?;
    // Retrieve & verify results:
    Assert::assert_equals(
        "dim=Author path=[] value=145.0 childCount=4\n  Lisa (50.0)\n  Frank (45.0)\n  Susan (40.0)\n  Bob (10.0)\n",
        top(&facets, 10, "Author")?.as_str(),
    );

    // Test MAX:
    let facets = TaxonomyFacetFloatAssociations::with_values_source(
        Arc::clone(&taxo_reader),
        Arc::clone(&facets_config),
        &c,
        max(),
        values_source,
    )?;
    Assert::assert_equals(
        "dim=Author path=[] value=45.0 childCount=4\n  Frank (45.0)\n  Susan (40.0)\n  Lisa (30.0)\n  Bob (10.0)\n",
        top(&facets, 10, "Author")?.as_str(),
    );

    // test getTopChildren(0, dim)
    let f = &facets;
    expect_throws(IAE, || f.get_top_children(0, "Author", &[]));

    taxo_reader.close()?;
    r.close()?;
    dir.close()?;
    taxo_dir.close()
}

// LUCENE-5333
fn test_sparse_facets() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let taxo_dir = LuceneTestCase::new_directory();

    // Writes facet ords to a separate directory from the
    // main index:
    let taxo_writer = Arc::new(DirectoryTaxonomyWriter::with_open_mode(
        taxo_dir.as_directory(),
        OpenMode::CREATE,
    )?);

    let writer =
        RandomIndexWriter::new_with_mock_analyzer(&*LuceneTestCase::random(), dir.as_directory())?;
    let config = FacetsConfig::new();

    let mut doc = Document::new();
    doc.add(NumericDocValuesField::new("num", 10));
    doc.add(FacetField::new("a", &["foo1"])?);
    writer.add_document(&config.build_with_taxonomy(Some(&*taxo_writer), &doc)?)?;

    if LuceneTestCase::random().next_boolean() {
        writer.commit()?;
    }

    let mut doc = Document::new();
    doc.add(NumericDocValuesField::new("num", 20));
    doc.add(FacetField::new("a", &["foo2"])?);
    doc.add(FacetField::new("b", &["bar1"])?);
    writer.add_document(&config.build_with_taxonomy(Some(&*taxo_writer), &doc)?)?;

    if LuceneTestCase::random().next_boolean() {
        writer.commit()?;
    }

    let mut doc = Document::new();
    doc.add(NumericDocValuesField::new("num", 30));
    doc.add(FacetField::new("a", &["foo3"])?);
    doc.add(FacetField::new("b", &["bar2"])?);
    doc.add(FacetField::new("c", &["baz1"])?);
    writer.add_document(&config.build_with_taxonomy(Some(&*taxo_writer), &doc)?)?;

    // NRT open
    let r = writer.get_reader()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&r));
    writer.close()?;

    // NRT open
    let taxo_reader: Arc<dyn TaxonomyReader> = Arc::new(DirectoryTaxonomyReader::from_writer(
        Arc::clone(&taxo_writer),
    )?);
    taxo_writer.close()?;

    let c = searcher.search_with_manager(match_all(), &FacetsCollectorManager::new())?;

    let facets = TaxonomyFacetFloatAssociations::with_values_source(
        Arc::clone(&taxo_reader),
        Arc::new(FacetsConfig::new()),
        &c,
        sum(),
        double_values_source::from_int_field("num"),
    )?;

    // Ask for top 10 labels for any dims that have counts:
    let results = facets.get_all_dims(10)?;

    // test getAllDims(0)
    expect_throws(IAE, || facets.get_all_dims(0));

    Assert::assert_equals(3, results.len());
    Assert::assert_equals(
        "dim=a path=[] value=60.0 childCount=3\n  foo3 (30.0)\n  foo2 (20.0)\n  foo1 (10.0)\n",
        results[0].to_string().as_str(),
    );
    Assert::assert_equals(
        "dim=b path=[] value=50.0 childCount=2\n  bar2 (30.0)\n  bar1 (20.0)\n",
        results[1].to_string().as_str(),
    );
    Assert::assert_equals(
        "dim=c path=[] value=30.0 childCount=1\n  baz1 (30.0)\n",
        results[2].to_string().as_str(),
    );

    // test default implementation of getTopDims
    let top_n_dims_result = facets.get_top_dims(2, 1)?;
    Assert::assert_equals(2, top_n_dims_result.len());
    Assert::assert_equals(
        "dim=a path=[] value=60.0 childCount=3\n  foo3 (30.0)\n",
        top_n_dims_result[0].to_string().as_str(),
    );
    Assert::assert_equals(
        "dim=b path=[] value=50.0 childCount=2\n  bar2 (30.0)\n",
        top_n_dims_result[1].to_string().as_str(),
    );

    // test getTopDims(10, 10) and expect same results from getAllDims(10)
    let all_dims_results = facets.get_top_dims(10, 10)?;
    assert_equals(&results, &all_dims_results);

    // test getTopDims(0, 1)
    expect_throws(IAE, || facets.get_top_dims(0, 1));

    // test getTopDims(1, 0) with topNChildren = 0
    expect_throws(IAE, || facets.get_top_dims(1, 0));

    r.close()?;
    taxo_reader.close()?;
    dir.close()?;
    taxo_dir.close()
}

fn test_wrong_index_field_name() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let taxo_dir = LuceneTestCase::new_directory();

    // Writes facet ords to a separate directory from the
    // main index:
    let taxo_writer = Arc::new(DirectoryTaxonomyWriter::with_open_mode(
        taxo_dir.as_directory(),
        OpenMode::CREATE,
    )?);

    let config = Arc::new(FacetsConfig::new());
    config.set_index_field_name("a", "$facets2");

    let writer =
        RandomIndexWriter::new_with_mock_analyzer(&*LuceneTestCase::random(), dir.as_directory())?;

    let mut doc = Document::new();
    doc.add(NumericDocValuesField::new("num", 10));
    doc.add(FacetField::new("a", &["foo1"])?);
    writer.add_document(&config.build_with_taxonomy(Some(&*taxo_writer), &doc)?)?;

    // NRT open
    let r = writer.get_reader()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&r));
    writer.close()?;

    // NRT open
    let taxo_reader: Arc<dyn TaxonomyReader> = Arc::new(DirectoryTaxonomyReader::from_writer(
        Arc::clone(&taxo_writer),
    )?);
    taxo_writer.close()?;

    let c = searcher.search_with_manager(match_all(), &FacetsCollectorManager::new())?;

    let facets = TaxonomyFacetFloatAssociations::with_values_source(
        Arc::clone(&taxo_reader),
        Arc::clone(&config),
        &c,
        sum(),
        double_values_source::from_int_field("num"),
    )?;

    // Ask for top 10 labels for any dims that have counts:
    let results = facets.get_all_dims(10)?;
    Assert::assert_true(results.is_empty());

    // test default implementation of getTopDims
    let top_dims_results = facets.get_top_dims(10, 10)?;
    Assert::assert_true(top_dims_results.is_empty());
    expect_throws(IAE, || facets.get_specific_value("a", &[]));

    expect_throws(IAE, || facets.get_top_children(10, "a", &[]));

    r.close()?;
    taxo_reader.close()?;
    dir.close()?;
    taxo_dir.close()
}

fn test_score_aggregator() -> Result<()> {
    let index_dir = LuceneTestCase::new_directory();
    let taxo_dir = LuceneTestCase::new_directory();

    let taxo_writer = Arc::new(DirectoryTaxonomyWriter::new(taxo_dir.as_directory())?);
    let iw = IndexWriter::new(index_dir.as_directory(), mock_iwc())?;

    let config = Arc::new(FacetsConfig::new());

    let mut i = LuceneTestCase::at_least(30);
    while i > 0 {
        let mut doc = Document::new();
        if LuceneTestCase::random().next_boolean() {
            // don't match all documents
            doc.add(StringField::new("f", "v".to_string(), Store::NO)?);
        }
        doc.add(FacetField::new("dim", &["a"])?);
        iw.add_document(&config.build_with_taxonomy(Some(&*taxo_writer), &doc)?)?;
        i -= 1;
    }

    let r = directory_reader::open_with_writer(&*iw)?;
    let taxo_reader: Arc<dyn TaxonomyReader> = Arc::new(DirectoryTaxonomyReader::from_writer(
        Arc::clone(&taxo_writer),
    )?);

    let csq = Arc::new(BoostQuery::new(
        Arc::new(ConstantScoreQuery::new(match_all())),
        2f32,
    )?);

    let facets_result = FacetsCollectorManager::search(
        &LuceneTestCase::new_searcher(Arc::clone(&r)),
        Arc::clone(&csq) as Arc<dyn Query>,
        10,
        &FacetsCollectorManager::with_keep_scores(true),
    )?;
    let td_total_hits = match facets_result.top_docs() {
        FacetsTopDocs::TopDocs(td) => td.total_hits.value(),
        FacetsTopDocs::TopFieldDocs(td) => td.total_hits.value(),
    };
    let fc = facets_result.facets_collector();
    Assert::assert_true(fc.get_keep_scores());

    // Test SUM:
    let facets = TaxonomyFacetFloatAssociations::with_values_source(
        Arc::clone(&taxo_reader),
        Arc::clone(&config),
        fc,
        sum(),
        double_values_source::scores(),
    )?;
    let expected = (csq.get_boost() * td_total_hits as f32) as i32;
    Assert::assert_equals(
        expected,
        number_int_value(&facets.get_specific_value("dim", &["a"])?),
    );

    // Test MAX:
    let facets = TaxonomyFacetFloatAssociations::with_values_source(
        Arc::clone(&taxo_reader),
        Arc::clone(&config),
        fc,
        max(),
        double_values_source::scores(),
    )?;
    let expected = csq.get_boost() as i32;
    Assert::assert_equals(
        expected,
        number_int_value(&facets.get_specific_value("dim", &["a"])?),
    );

    iw.close()?;
    taxo_writer.close()?;
    taxo_reader.close()?;
    taxo_dir.close()?;
    r.close()?;
    index_dir.close()
}

/// The index and taxonomy directories, writers and config of
/// [`price_index`].
type PriceIndex = (
    rucene_test_framework::util::lucene_test_case::TestDirectory,
    rucene_test_framework::util::lucene_test_case::TestDirectory,
    Arc<DirectoryTaxonomyWriter>,
    Arc<IndexWriter>,
    Arc<FacetsConfig>,
);

/// The body shared by `testNoScore`, `testWithScore` and `testRollupValues`: four documents
/// with `price` 1..4 and `a/<i % 2>` (`hierarchical`: `a/<i % 2>/1`).
fn price_index(hierarchical: bool, with_empty_doc: bool) -> Result<PriceIndex> {
    let index_dir = LuceneTestCase::new_directory();
    let taxo_dir = LuceneTestCase::new_directory();

    let taxo_writer = Arc::new(DirectoryTaxonomyWriter::new(taxo_dir.as_directory())?);
    let iw = IndexWriter::new(index_dir.as_directory(), mock_iwc())?;
    let config = Arc::new(FacetsConfig::new());
    if hierarchical {
        config.set_hierarchical("a", true);
        // config.setRequireDimCount("a", true);
    }

    if with_empty_doc {
        // Add a doc without the price field to exercise the bug found in LUCENE-10491:
        let doc = Document::new();
        iw.add_document(&config.build_with_taxonomy(Some(&*taxo_writer), &doc)?)?;
    }

    for i in 0..4i64 {
        let mut doc = Document::new();
        doc.add(NumericDocValuesField::new("price", i + 1));
        let label = (i % 2).to_string();
        if hierarchical {
            doc.add(FacetField::new("a", &[label.as_str(), "1"])?);
        } else {
            doc.add(FacetField::new("a", &[label.as_str()])?);
        }
        iw.add_document(&config.build_with_taxonomy(Some(&*taxo_writer), &doc)?)?;
    }
    Ok((index_dir, taxo_dir, taxo_writer, iw, config))
}

/// The SUM / MAX assertions of `testNoScore`, `testWithScore` and
/// `testRollupValues`.
fn check_price_sums(
    taxo_reader: &Arc<dyn TaxonomyReader>,
    config: &Arc<FacetsConfig>,
    fc: &FacetsCollector,
    values_source: Arc<dyn DoubleValuesSource>,
) -> Result<()> {
    // Test SUM:
    let facets = TaxonomyFacetFloatAssociations::with_values_source(
        Arc::clone(taxo_reader),
        Arc::clone(config),
        fc,
        sum(),
        Arc::clone(&values_source),
    )?;
    Assert::assert_equals(
        "dim=a path=[] value=10.0 childCount=2\n  1 (6.0)\n  0 (4.0)\n",
        top(&facets, 10, "a")?.as_str(),
    );

    // Test MAX:
    let facets = TaxonomyFacetFloatAssociations::with_values_source(
        Arc::clone(taxo_reader),
        Arc::clone(config),
        fc,
        max(),
        values_source,
    )?;
    Assert::assert_equals(
        "dim=a path=[] value=4.0 childCount=2\n  1 (4.0)\n  0 (3.0)\n",
        top(&facets, 10, "a")?.as_str(),
    );
    Ok(())
}

fn test_no_score() -> Result<()> {
    let (index_dir, taxo_dir, taxo_writer, iw, config) = price_index(false, false)?;

    let r = directory_reader::open_with_writer(&*iw)?;
    let taxo_reader: Arc<dyn TaxonomyReader> = Arc::new(DirectoryTaxonomyReader::from_writer(
        Arc::clone(&taxo_writer),
    )?);

    let sfc = LuceneTestCase::new_searcher(Arc::clone(&r))
        .search_with_manager(match_all(), &FacetsCollectorManager::new())?;

    check_price_sums(
        &taxo_reader,
        &config,
        &sfc,
        double_values_source::from_long_field("price"),
    )?;

    iw.close()?;
    taxo_writer.close()?;
    taxo_reader.close()?;
    taxo_dir.close()?;
    r.close()?;
    index_dir.close()
}

fn test_with_score() -> Result<()> {
    let (index_dir, taxo_dir, taxo_writer, iw, config) = price_index(false, true)?;

    let r = directory_reader::open_with_writer(&*iw)?;
    let taxo_reader: Arc<dyn TaxonomyReader> = Arc::new(DirectoryTaxonomyReader::from_writer(
        Arc::clone(&taxo_writer),
    )?);

    // score documents by their 'price' field - makes asserting the correct counts for the
    // categories easier
    let q: Arc<dyn Query> = Arc::new(FunctionQuery::new(Arc::new(LongFieldSource::new("price"))));
    let fc = FacetsCollectorManager::search(
        &LuceneTestCase::new_searcher(Arc::clone(&r)),
        q,
        10,
        &FacetsCollectorManager::with_keep_scores(true),
    )?
    .facets_collector;
    Assert::assert_true(fc.get_keep_scores());

    check_price_sums(&taxo_reader, &config, &fc, double_values_source::scores())?;

    iw.close()?;
    taxo_writer.close()?;
    taxo_reader.close()?;
    taxo_dir.close()?;
    r.close()?;
    index_dir.close()
}

fn test_rollup_values() -> Result<()> {
    let (index_dir, taxo_dir, taxo_writer, iw, config) = price_index(true, false)?;

    let r = directory_reader::open_with_writer(&*iw)?;
    let taxo_reader: Arc<dyn TaxonomyReader> = Arc::new(DirectoryTaxonomyReader::from_writer(
        Arc::clone(&taxo_writer),
    )?);

    let sfc = LuceneTestCase::new_searcher(Arc::clone(&r))
        .search_with_manager(match_all(), &FacetsCollectorManager::new())?;
    check_price_sums(
        &taxo_reader,
        &config,
        &sfc,
        double_values_source::from_long_field("price"),
    )?;

    iw.close()?;
    taxo_writer.close()?;
    taxo_reader.close()?;
    taxo_dir.close()?;
    r.close()?;
    index_dir.close()
}

// LUCENE-10495
fn test_children_and_siblings_loaded() -> Result<()> {
    let should_load = [false, true];
    for load in should_load {
        let index_dir = LuceneTestCase::new_directory();
        let taxo_dir = LuceneTestCase::new_directory();

        let taxo_writer = Arc::new(DirectoryTaxonomyWriter::new(taxo_dir.as_directory())?);
        let iw = IndexWriter::new(index_dir.as_directory(), mock_iwc())?;
        let config = Arc::new(FacetsConfig::new());

        config.set_hierarchical("a", true);
        config.set_multi_valued("a", !load);
        config.set_require_dim_count("a", true);

        let mut doc = Document::new();
        doc.add(FacetField::new("a", &["1", "2"])?);
        iw.add_document(&config.build_with_taxonomy(Some(&*taxo_writer), &doc)?)?;

        let r = directory_reader::open_with_writer(&*iw)?;
        let taxo_reader: Arc<dyn TaxonomyReader> = Arc::new(DirectoryTaxonomyReader::from_writer(
            Arc::clone(&taxo_writer),
        )?);

        let sfc = LuceneTestCase::new_searcher(Arc::clone(&r))
            .search_with_manager(match_all(), &FacetsCollectorManager::new())?;

        let facets = TaxonomyFacetFloatAssociations::with_values_source(
            Arc::clone(&taxo_reader),
            Arc::clone(&config),
            &sfc,
            max(),
            double_values_source::from_long_field("price"),
        )?;

        Assert::assert_equals(load, facets.children_loaded());
        Assert::assert_equals(load, facets.siblings_loaded());

        iw.close()?;
        taxo_writer.close()?;
        taxo_reader.close()?;
        taxo_dir.close()?;
        r.close()?;
        index_dir.close()?;
    }
    Ok(())
}

fn test_count_and_sum_score() -> Result<()> {
    let index_dir = LuceneTestCase::new_directory();
    let taxo_dir = LuceneTestCase::new_directory();

    let taxo_writer = Arc::new(DirectoryTaxonomyWriter::new(taxo_dir.as_directory())?);
    let iw = IndexWriter::new(index_dir.as_directory(), mock_iwc())?;
    let config = Arc::new(FacetsConfig::new());
    config.set_index_field_name("b", "$b");

    let mut i = LuceneTestCase::at_least(30);
    while i > 0 {
        let mut doc = Document::new();
        doc.add(StringField::new("f", "v".to_string(), Store::NO)?);
        doc.add(FacetField::new("a", &["1"])?);
        doc.add(FacetField::new("b", &["1"])?);
        iw.add_document(&config.build_with_taxonomy(Some(&*taxo_writer), &doc)?)?;
        i -= 1;
    }

    let r = directory_reader::open_with_writer(&*iw)?;
    let taxo_reader: Arc<dyn TaxonomyReader> = Arc::new(DirectoryTaxonomyReader::from_writer(
        Arc::clone(&taxo_writer),
    )?);

    let fc = FacetsCollectorManager::search(
        &LuceneTestCase::new_searcher(Arc::clone(&r)),
        match_all(),
        10,
        &FacetsCollectorManager::with_keep_scores(true),
    )?
    .facets_collector;
    Assert::assert_true(fc.get_keep_scores());

    let facets1 = get_taxonomy_facet_counts(&taxo_reader, &config, &fc)?;
    let facets2 = TaxonomyFacetFloatAssociations::with_index_field_name_and_values_source(
        "$b",
        Arc::clone(&taxo_reader),
        Arc::clone(&config),
        &fc,
        sum(),
        double_values_source::scores(),
    )?;

    let top_a = facets1.get_top_children(10, "a", &[])?.unwrap();
    Assert::assert_equals(r.max_doc(), number_int_value(&top_a.value));
    let top_b = facets2.get_top_children(10, "b", &[])?.unwrap();
    Assert::assert_equals_double_msg(
        None,
        f64::from(r.max_doc()),
        number_double_value(&top_b.value),
        1E-10,
    );
    iw.close()?;
    taxo_writer.close()?;
    taxo_reader.close()?;
    taxo_dir.close()?;
    r.close()?;
    index_dir.close()
}

fn test_random() -> Result<()> {
    let tokens = get_random_tokens(10);
    let index_dir = LuceneTestCase::new_directory();
    let taxo_dir = LuceneTestCase::new_directory();

    let w = RandomIndexWriter::new_with_mock_analyzer(
        &*LuceneTestCase::random(),
        index_dir.as_directory(),
    )?;
    let tw = Arc::new(DirectoryTaxonomyWriter::new(taxo_dir.as_directory())?);
    let config = Arc::new(FacetsConfig::new());
    let num_docs = LuceneTestCase::at_least(1000) as usize;
    let num_dims = TestUtil::next_int(&*LuceneTestCase::random(), 1, 7) as usize;
    let mut test_docs = get_random_docs(&tokens, num_docs, num_dims);
    for test_doc in test_docs.iter_mut() {
        let mut doc = Document::new();
        doc.add(LuceneTestCase::new_string_field(
            "content",
            &test_doc.content,
            Store::NO,
        ));
        test_doc.value = LuceneTestCase::random().next_float();
        doc.add(FloatDocValuesField::new("value", test_doc.value));
        for (j, dim) in test_doc.dims.iter().enumerate().take(num_dims) {
            if let Some(dim) = dim {
                doc.add(FacetField::new(&format!("dim{j}"), &[dim.as_str()])?);
            }
        }
        w.add_document(&config.build_with_taxonomy(Some(&*tw), &doc)?)?;
    }

    // NRT open
    let r = w.get_reader()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&r));

    // NRT open
    let tr: Arc<dyn TaxonomyReader> =
        Arc::new(DirectoryTaxonomyReader::from_writer(Arc::clone(&tw))?);

    let iters = LuceneTestCase::at_least(100);
    for _iter in 0..iters {
        let search_token =
            &tokens[LuceneTestCase::random().next_int_bound(tokens.len() as i32) as usize];
        if *VERBOSE {
            println!("\nTEST: iter content={search_token}");
        }
        let fc = FacetsCollectorManager::search(
            &searcher,
            Arc::new(TermQuery::new(Term::from_text("content", search_token))),
            10,
            &FacetsCollectorManager::new(),
        )?
        .facets_collector;

        check_results(
            num_dims,
            &test_docs,
            search_token,
            &tr,
            &config,
            &fc,
            double_values_source::from_float_field("value"),
            sum(),
        )?;
        check_results(
            num_dims,
            &test_docs,
            search_token,
            &tr,
            &config,
            &fc,
            double_values_source::from_float_field("value"),
            max(),
        )?;
    }

    w.close()?;
    tw.close()?;
    r.close()?;
    tr.close()?;
    index_dir.close()?;
    taxo_dir.close()
}

#[allow(clippy::too_many_arguments)]
fn check_results(
    num_dims: usize,
    test_docs: &[TestDoc],
    search_token: &str,
    taxo_reader: &Arc<dyn TaxonomyReader>,
    facets_config: &Arc<FacetsConfig>,
    facets_collector: &FacetsCollector,
    values_source: Arc<dyn DoubleValuesSource>,
    aggregation_function: Arc<dyn AssociationAggregationFunction>,
) -> Result<()> {
    // Slow, yet hopefully bug-free, faceting:
    let mut expected_values: Vec<JavaHashMap<f32>> =
        (0..num_dims).map(|_| JavaHashMap::new()).collect();

    for doc in test_docs {
        if doc.content == search_token {
            for j in 0..num_dims {
                if let Some(dim) = &doc.dims[j] {
                    match expected_values[j].get(dim).copied() {
                        None => {
                            expected_values[j].insert(dim.clone(), doc.value);
                        }
                        Some(v) => {
                            let new_value = aggregation_function.aggregate_float(v, doc.value);
                            expected_values[j].insert(dim.clone(), new_value);
                        }
                    }
                }
            }
        }
    }

    let mut expected = Vec::new();
    for (i, values) in expected_values.iter().enumerate() {
        let mut label_values = Vec::new();
        let mut aggregated_value = 0f32;
        for (key, value) in values.iter() {
            label_values.push(LabelAndValue::new(key.clone(), NumericValue::Float(*value)));
            aggregated_value = aggregation_function.aggregate_float(aggregated_value, *value);
        }
        sort_label_values(&mut label_values);
        if aggregated_value > 0.0 {
            let child_count = label_values.len() as i32;
            expected.push(FacetResult::new(
                format!("dim{i}"),
                Vec::new(),
                NumericValue::Float(aggregated_value),
                label_values,
                child_count,
            ));
        }
    }

    // Sort by highest value, tie break by value:
    sort_facet_results(&mut expected);

    let facets = TaxonomyFacetFloatAssociations::with_values_source(
        Arc::clone(taxo_reader),
        Arc::clone(facets_config),
        facets_collector,
        aggregation_function,
        values_source,
    )?;

    let mut actual = facets.get_all_dims(10)?;

    // test default implementation of getTopDims
    if !actual.is_empty() {
        let top_dims_results1 = facets.get_top_dims(1, 10)?;
        assert_equals(&actual[0], &top_dims_results1[0]);
    }

    // Messy: fixup ties
    sort_ties_results(&mut actual);

    if *VERBOSE {
        println!("expected=\n{}", java_list(&expected));
        println!("actual=\n{}", java_list(&actual));
    }

    assert_float_values_equals_list(&expected, &actual);
    Ok(())
}

/// `List.toString()` of facet results.
fn java_list(results: &[FacetResult]) -> String {
    let parts: Vec<String> = results.iter().map(ToString::to_string).collect();
    format!("[{}]", parts.join(", "))
}
