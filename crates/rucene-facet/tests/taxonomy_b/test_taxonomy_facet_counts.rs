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

//! Port of `org.apache.lucene.facet.taxonomy.TestTaxonomyFacetCounts`.

use std::collections::HashSet;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::{Document, NumericValue, Store, StringField};
use rucene::error::Result;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::OpenMode;
use rucene::index::merge_policy::NoMergePolicy;
use rucene::index::{directory_reader, FieldInvertState, Term};
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::query::Query;
use rucene::search::similarities::per_field_similarity_wrapper::{
    per_field_compute_norm, per_field_scorer, PerFieldSimilarityWrapper,
};
use rucene::search::similarities::tfidf_similarity::ClassicSimilarity;
use rucene::search::similarities::{SimScorer, Similarity};
use rucene::search::term_query::TermQuery;
use rucene::search::{CollectionStatistics, MatchAllDocsQuery, TermStatistics};
use rucene::util::java_collections::JavaHashMap;
use rucene::util::print_stream::ByteArrayPrintStream;
use rucene_facet::drill_down_query::DrillDownQuery;
use rucene_facet::facet_field::FacetField;
use rucene_facet::facet_result::FacetResult;
use rucene_facet::facets::Facets;
use rucene_facet::facets_collector_manager::FacetsCollectorManager;
use rucene_facet::facets_config::{FacetsConfig, DEFAULT_INDEX_FIELD_NAME};
use rucene_facet::label_and_value::{number_int_value, LabelAndValue};
use rucene_facet::taxonomy::directory::directory_taxonomy_reader::DirectoryTaxonomyReader;
use rucene_facet::taxonomy::directory::directory_taxonomy_writer::DirectoryTaxonomyWriter;
use rucene_facet::taxonomy::facet_label::FacetLabel;
use rucene_facet::taxonomy::fast_taxonomy_facet_counts::FastTaxonomyFacetCounts;
use rucene_facet::taxonomy::print_taxonomy_stats::PrintTaxonomyStats;
use rucene_facet::taxonomy::taxonomy_reader::TaxonomyReader;
use rucene_facet::taxonomy::taxonomy_writer::TaxonomyWriter;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::lucene_test_case::{TEST_NIGHTLY, VERBOSE};
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::facet_support::core_support::{assert_equals, assert_equals_msg, test};
use crate::facet_support::{
    assert_facet_result, facet_class, get_all_taxonomy_facet_labels, get_random_docs,
    get_random_tokens, get_taxonomy_facet_counts, get_taxonomy_facet_counts_field,
    sort_facet_results, sort_label_values, sort_ties_results,
};

const IAE: &str = "java.lang.IllegalArgumentException";

/// `TestTaxonomyFacetCounts`.
pub fn class_def() -> Arc<Class> {
    facet_class(
        "org.apache.lucene.facet.taxonomy.TestTaxonomyFacetCounts",
        vec![
            test("testBasic", test_basic),
            test("testSparseFacets", test_sparse_facets),
            test("testWrongIndexFieldName", test_wrong_index_field_name),
            test("testNonExistentDimension", test_non_existent_dimension),
            test(
                "testReallyNoNormsForDrillDown",
                test_really_no_norms_for_drill_down,
            ),
            test("testMultiValuedHierarchy", test_multi_valued_hierarchy),
            test("testLabelWithDelimiter", test_label_with_delimiter),
            test("testRequireDimCount", test_require_dim_count),
            test(
                "testManyFacetsInOneDocument",
                test_many_facets_in_one_document,
            ),
            test(
                "testDetectHierarchicalField",
                test_detect_hierarchical_field,
            ),
            test("testDetectMultiValuedField", test_detect_multi_valued_field),
            test("testSeparateIndexedFields", test_separate_indexed_fields),
            test("testCountRoot", test_count_root),
            test("testGetFacetResultsTwice", test_get_facet_results_twice),
            test("testChildCount", test_child_count),
            test(
                "testMultiValuedFieldCountAll",
                test_multi_valued_field_count_all,
            ),
            test(
                "testCountAllSingleValuedField",
                test_count_all_single_valued_field,
            ),
            test(
                "testSegmentsWithoutCategoriesOrResults",
                test_segments_without_categories_or_results,
            ),
            test("testRandom", test_random),
        ],
    )
}

fn top(facets: &dyn Facets, top_n: i32, dim: &str) -> Result<FacetResult> {
    Ok(facets
        .get_top_children(top_n, dim, &[])?
        .expect("java.lang.NullPointerException"))
}

fn all(facets: &dyn Facets, dim: &str) -> Result<FacetResult> {
    Ok(facets
        .get_all_children(dim, &[])?
        .expect("java.lang.NullPointerException"))
}

fn lv(label: &str, value: i32) -> LabelAndValue {
    LabelAndValue::new(label, NumericValue::Int(value))
}

fn match_all() -> Arc<dyn Query> {
    Arc::new(MatchAllDocsQuery::instance())
}

fn mock_iwc() -> rucene::index::index_writer_config::IndexWriterConfig {
    let random = LuceneTestCase::random();
    LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(MockAnalyzer::new(&*random))))
}

fn random_writer(
    dir: &rucene_test_framework::util::lucene_test_case::TestDirectory,
) -> Result<RandomIndexWriter> {
    RandomIndexWriter::new_with_mock_analyzer(&*LuceneTestCase::random(), dir.as_directory())
}

fn create_taxo_writer(
    dir: &rucene_test_framework::util::lucene_test_case::TestDirectory,
) -> Result<Arc<DirectoryTaxonomyWriter>> {
    Ok(Arc::new(DirectoryTaxonomyWriter::with_open_mode(
        dir.as_directory(),
        OpenMode::CREATE,
    )?))
}

fn nrt_taxo_reader(tw: &Arc<DirectoryTaxonomyWriter>) -> Result<Arc<dyn TaxonomyReader>> {
    Ok(Arc::new(DirectoryTaxonomyReader::from_writer(Arc::clone(
        tw,
    ))?))
}

fn test_basic() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let taxo_dir = LuceneTestCase::new_directory();

    // Writes facet ords to a separate directory from the
    // main index:
    let taxo_writer = create_taxo_writer(&taxo_dir)?;

    let config = Arc::new(FacetsConfig::new());
    config.set_hierarchical("Publish Date", true);

    let writer = random_writer(&dir)?;

    for (author, date) in [
        ("Bob", ["2010", "10", "15"]),
        ("Lisa", ["2010", "10", "20"]),
        ("Lisa", ["2012", "1", "1"]),
        ("Susan", ["2012", "1", "7"]),
        ("Frank", ["1999", "5", "5"]),
    ] {
        let mut doc = Document::new();
        doc.add(FacetField::new("Author", &[author])?);
        doc.add(FacetField::new("Publish Date", &date)?);
        writer.add_document(&config.build_with_taxonomy(Some(&*taxo_writer), &doc)?)?;
    }

    // NRT open
    let r = writer.get_reader()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&r));

    // NRT open
    let taxo_reader = nrt_taxo_reader(&taxo_writer)?;

    let facets = get_all_facets(DEFAULT_INDEX_FIELD_NAME, &searcher, &taxo_reader, &config)?;

    // Publish Date is hierarchical, so we should have loaded all 3 int[]:
    Assert::assert_true(facets.siblings_loaded());
    Assert::assert_true(facets.children_loaded());

    let final_facets = &facets;
    expect_throws(IAE, || final_facets.get_top_children(0, "Author", &[]));

    // Retrieve & verify results:
    Assert::assert_equals(
        "dim=Publish Date path=[] value=5 childCount=3\n  2010 (2)\n  2012 (2)\n  1999 (1)\n",
        top(&facets, 10, "Publish Date")?.to_string().as_str(),
    );
    Assert::assert_equals(
        "dim=Author path=[] value=5 childCount=4\n  Lisa (2)\n  Bob (1)\n  Susan (1)\n  Frank (1)\n",
        top(&facets, 10, "Author")?.to_string().as_str(),
    );

    assert_facet_result(
        &all(&facets, "Publish Date")?,
        "Publish Date",
        &[],
        3,
        NumericValue::Int(5),
        &[lv("1999", 1), lv("2010", 2), lv("2012", 2)],
    );

    assert_facet_result(
        &all(&facets, "Author")?,
        "Author",
        &[],
        4,
        NumericValue::Int(5),
        &[lv("Bob", 1), lv("Frank", 1), lv("Lisa", 2), lv("Susan", 1)],
    );

    // test getAllDims
    let results = facets.get_all_dims(10)?;
    // test getTopDims(10, 10) and expect same results from getAllDims(10)
    let all_top_dims_results = facets.get_top_dims(10, 10)?;
    assert_equals(&results, &all_top_dims_results);

    // Now user drills down on Publish Date/2010:
    let mut q2 = DrillDownQuery::new(Arc::clone(&config));
    q2.add("Publish Date", &["2010"])?;
    let c = searcher.search_with_manager(Arc::new(q2), &FacetsCollectorManager::new())?;
    let facets = FastTaxonomyFacetCounts::new(Arc::clone(&taxo_reader), Arc::clone(&config), &c)?;
    Assert::assert_equals(
        "dim=Author path=[] value=2 childCount=2\n  Bob (1)\n  Lisa (1)\n",
        top(&facets, 10, "Author")?.to_string().as_str(),
    );

    assert_equals(
        &NumericValue::Int(1),
        &facets.get_specific_value("Author", &["Lisa"])?,
    );

    Assert::assert_true(
        facets
            .get_top_children(10, "Non exitent dim", &[])?
            .is_none(),
    );

    // Smoke test PrintTaxonomyStats:
    let bos = ByteArrayPrintStream::new();
    PrintTaxonomyStats::print_stats(&*taxo_reader, &bos, true)?;
    let result = bos.to_string_utf8();
    Assert::assert_true(result.contains("/Author: 4 immediate children; 5 total categories"));
    Assert::assert_true(
        result.contains("/Publish Date: 3 immediate children; 12 total categories"),
    );
    // Make sure at least a few nodes of the tree came out:
    Assert::assert_true(result.contains("  /1999"));
    Assert::assert_true(result.contains("  /2012"));
    Assert::assert_true(result.contains("      /20"));

    writer.close()?;
    taxo_writer.close()?;
    r.close()?;
    taxo_reader.close()?;
    taxo_dir.close()?;
    dir.close()
}

// LUCENE-5333
fn test_sparse_facets() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let taxo_dir = LuceneTestCase::new_directory();

    // Writes facet ords to a separate directory from the
    // main index:
    let taxo_writer = create_taxo_writer(&taxo_dir)?;

    let writer = random_writer(&dir)?;
    let config = Arc::new(FacetsConfig::new());

    let mut doc = Document::new();
    doc.add(FacetField::new("a", &["foo1"])?);
    doc.add(FacetField::new("b", &["aar1"])?);
    writer.add_document(&config.build_with_taxonomy(Some(&*taxo_writer), &doc)?)?;

    if LuceneTestCase::random().next_boolean() {
        writer.commit()?;
    }

    let mut doc = Document::new();
    doc.add(FacetField::new("a", &["foo2"])?);
    doc.add(FacetField::new("b", &["bar1"])?);
    writer.add_document(&config.build_with_taxonomy(Some(&*taxo_writer), &doc)?)?;

    if LuceneTestCase::random().next_boolean() {
        writer.commit()?;
    }

    let mut doc = Document::new();
    doc.add(FacetField::new("a", &["foo3"])?);
    doc.add(FacetField::new("b", &["bar2"])?);
    doc.add(FacetField::new("c", &["baz1"])?);
    writer.add_document(&config.build_with_taxonomy(Some(&*taxo_writer), &doc)?)?;

    // NRT open
    let r = writer.get_reader()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&r));

    // NRT open
    let taxo_reader = nrt_taxo_reader(&taxo_writer)?;

    let facets = get_all_facets(DEFAULT_INDEX_FIELD_NAME, &searcher, &taxo_reader, &config)?;

    // test getAllDims(0)
    expect_throws(IAE, || facets.get_all_dims(0));

    // Ask for top 10 labels for any dims that have counts:
    let results = facets.get_all_dims(10)?;

    Assert::assert_equals(3, results.len());
    Assert::assert_equals(
        "dim=a path=[] value=3 childCount=3\n  foo1 (1)\n  foo2 (1)\n  foo3 (1)\n",
        results[0].to_string().as_str(),
    );
    Assert::assert_equals(
        "dim=b path=[] value=3 childCount=3\n  aar1 (1)\n  bar1 (1)\n  bar2 (1)\n",
        results[1].to_string().as_str(),
    );
    Assert::assert_equals(
        "dim=c path=[] value=1 childCount=1\n  baz1 (1)\n",
        results[2].to_string().as_str(),
    );

    // test getAllDims with topN = 1, sort by dim names when values are equal
    let top1results = facets.get_all_dims(1)?;

    Assert::assert_equals(3, results.len());
    Assert::assert_equals(
        "dim=a path=[] value=3 childCount=3\n  foo1 (1)\n",
        top1results[0].to_string().as_str(),
    );
    Assert::assert_equals(
        "dim=b path=[] value=3 childCount=3\n  aar1 (1)\n",
        top1results[1].to_string().as_str(),
    );
    Assert::assert_equals(
        "dim=c path=[] value=1 childCount=1\n  baz1 (1)\n",
        top1results[2].to_string().as_str(),
    );

    // test default implementation of getTopDims
    let top_n_dims_result = facets.get_top_dims(2, 1)?;
    Assert::assert_equals(2, top_n_dims_result.len());
    Assert::assert_equals(
        "dim=a path=[] value=3 childCount=3\n  foo1 (1)\n",
        top_n_dims_result[0].to_string().as_str(),
    );
    Assert::assert_equals(
        "dim=b path=[] value=3 childCount=3\n  aar1 (1)\n",
        top_n_dims_result[1].to_string().as_str(),
    );

    // test getTopDims(10, 10) and expect same results from getAllDims(10)
    let all_dims_results = facets.get_top_dims(10, 10)?;
    assert_equals(&results, &all_dims_results);

    // test getTopDims(0, 1)
    expect_throws(IAE, || facets.get_top_dims(0, 1));

    // test getTopDims(1, 0) with topNChildren = 0
    expect_throws(IAE, || facets.get_top_dims(1, 0));

    writer.close()?;
    taxo_writer.close()?;
    r.close()?;
    taxo_reader.close()?;
    taxo_dir.close()?;
    dir.close()
}

fn test_wrong_index_field_name() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let taxo_dir = LuceneTestCase::new_directory();

    // Writes facet ords to a separate directory from the
    // main index:
    let taxo_writer = create_taxo_writer(&taxo_dir)?;

    let config = Arc::new(FacetsConfig::new());
    config.set_index_field_name("a", "$facets2");
    let writer = random_writer(&dir)?;

    let mut doc = Document::new();
    doc.add(FacetField::new("a", &["foo1"])?);
    writer.add_document(&config.build_with_taxonomy(Some(&*taxo_writer), &doc)?)?;

    // NRT open
    let r = writer.get_reader()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&r));

    // NRT open
    let taxo_reader = nrt_taxo_reader(&taxo_writer)?;

    let c = searcher.search_with_manager(match_all(), &FacetsCollectorManager::new())?;

    // Uses default $facets field:
    let facets = FastTaxonomyFacetCounts::new(Arc::clone(&taxo_reader), Arc::clone(&config), &c)?;

    // Ask for top 10 labels for any dims that have counts:
    let results = facets.get_all_dims(10)?;
    Assert::assert_true(results.is_empty());

    // test getTopDims(10, 10) and expect same results from getAllDims(10)
    let all_top_dims_results = facets.get_top_dims(10, 10)?;
    assert_equals(&results, &all_top_dims_results);

    expect_throws(IAE, || facets.get_specific_value("a", &[]));

    expect_throws(IAE, || facets.get_top_children(10, "a", &[]));
    expect_throws(IAE, || facets.get_all_children("a", &[]));

    writer.close()?;
    taxo_writer.close()?;
    r.close()?;
    taxo_reader.close()?;
    taxo_dir.close()?;
    dir.close()
}

fn test_non_existent_dimension() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let taxo_dir = LuceneTestCase::new_directory();

    let taxo_writer = create_taxo_writer(&taxo_dir)?;

    let config = Arc::new(FacetsConfig::new());
    config.set_index_field_name("foo", "$custom");

    let mut doc = Document::new();
    doc.add(FacetField::new("foo", &["bar"])?);

    let writer = random_writer(&dir)?;
    writer.add_document(&config.build_with_taxonomy(Some(&*taxo_writer), &doc)?)?;

    let r = writer.get_reader()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&r));
    let taxo_reader = nrt_taxo_reader(&taxo_writer)?;

    let facets = get_all_facets("$custom", &searcher, &taxo_reader, &config)?;

    // get facets for the dimension, which was never configured or indexed before
    let result = facets.get_top_children(5, "non-existent dimension", &[])?;

    // make sure the result is null (and no exception was thrown)
    Assert::assert_true(result.is_none());

    // get facets for the dimension, which was never configured or indexed before
    let all_children_result = facets.get_all_children("non-existent dimension", &[])?;

    // make sure the result is null (and no exception was thrown)
    Assert::assert_true(all_children_result.is_none());

    writer.close()?;
    taxo_writer.close()?;
    r.close()?;
    taxo_reader.close()?;
    taxo_dir.close()?;
    dir.close()
}

/// The anonymous `PerFieldSimilarityWrapper` of
/// `testReallyNoNormsForDrillDown`.
#[derive(Debug)]
struct OnlyFieldSimilarity {
    sim: ClassicSimilarity,
}

impl PerFieldSimilarityWrapper for OnlyFieldSimilarity {
    fn get(&self, name: &str) -> &dyn Similarity {
        Assert::assert_equals("field", name);
        &self.sim
    }
}

impl std::fmt::Display for OnlyFieldSimilarity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PerFieldSimilarityWrapper")
    }
}

impl Similarity for OnlyFieldSimilarity {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.facet.taxonomy.TestTaxonomyFacetCounts$1"
    }

    fn compute_norm(&self, state: &FieldInvertState) -> Result<i64> {
        per_field_compute_norm(self, state)
    }

    fn scorer<'a>(
        &'a self,
        boost: f32,
        collection_stats: &CollectionStatistics,
        term_stats: &[TermStatistics],
    ) -> Box<dyn SimScorer + 'a> {
        per_field_scorer(self, boost, collection_stats, term_stats)
    }
}

fn test_really_no_norms_for_drill_down() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let taxo_dir = LuceneTestCase::new_directory();
    let iwc = mock_iwc();
    iwc.set_similarity(Some(Arc::new(OnlyFieldSimilarity {
        sim: ClassicSimilarity::new(),
    })))?;
    let taxo_writer = create_taxo_writer(&taxo_dir)?;
    let writer =
        RandomIndexWriter::new_with_config(&*LuceneTestCase::random(), dir.as_directory(), iwc)?;
    let config = FacetsConfig::new();

    let mut doc = Document::new();
    doc.add(LuceneTestCase::new_text_field("field", "text", Store::NO));
    doc.add(FacetField::new("a", &["path"])?);
    writer.add_document(&config.build_with_taxonomy(Some(&*taxo_writer), &doc)?)?;
    writer.close()?;
    taxo_writer.close()?;
    dir.close()?;
    taxo_dir.close()
}

fn test_multi_valued_hierarchy() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let taxo_dir = LuceneTestCase::new_directory();
    let taxo_writer = create_taxo_writer(&taxo_dir)?;
    let config = Arc::new(FacetsConfig::new());
    config.set_hierarchical("a", true);
    config.set_multi_valued("a", true);
    let writer = random_writer(&dir)?;

    let mut doc = Document::new();
    doc.add(LuceneTestCase::new_text_field("field", "text", Store::NO));
    doc.add(FacetField::new("a", &["path", "x"])?);
    doc.add(FacetField::new("a", &["path", "y"])?);
    writer.add_document(&config.build_with_taxonomy(Some(&*taxo_writer), &doc)?)?;

    // NRT open
    let r = writer.get_reader()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&r));

    // NRT open
    let taxo_reader = nrt_taxo_reader(&taxo_writer)?;

    let facets = get_all_facets(DEFAULT_INDEX_FIELD_NAME, &searcher, &taxo_reader, &config)?;

    expect_throws(IAE, || facets.get_specific_value("a", &[]));

    let result = top(&facets, 10, "a")?;
    Assert::assert_equals(1, result.label_values.len());
    Assert::assert_equals(1, number_int_value(&result.label_values[0].value));

    let all_children = all(&facets, "a")?;
    Assert::assert_equals(1, all_children.label_values.len());
    Assert::assert_equals(1, number_int_value(&all_children.label_values[0].value));

    writer.close()?;
    taxo_writer.close()?;
    r.close()?;
    taxo_reader.close()?;
    dir.close()?;
    taxo_dir.close()
}

fn test_label_with_delimiter() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let taxo_dir = LuceneTestCase::new_directory();
    let writer = random_writer(&dir)?;
    let taxo_writer = create_taxo_writer(&taxo_dir)?;

    let config = Arc::new(FacetsConfig::new());
    config.set_multi_valued("dim", true);

    let mut doc = Document::new();
    doc.add(LuceneTestCase::new_text_field("field", "text", Store::NO));
    doc.add(FacetField::new("dim", &["test\u{001F}one"])?);
    doc.add(FacetField::new("dim", &["test\u{001E}two"])?);
    writer.add_document(&config.build_with_taxonomy(Some(&*taxo_writer), &doc)?)?;

    // NRT open
    let r = writer.get_reader()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&r));

    // NRT open
    let taxo_reader = nrt_taxo_reader(&taxo_writer)?;

    let facets = get_all_facets(DEFAULT_INDEX_FIELD_NAME, &searcher, &taxo_reader, &config)?;

    assert_equals(
        &NumericValue::Int(1),
        &facets.get_specific_value("dim", &["test\u{001F}one"])?,
    );
    assert_equals(
        &NumericValue::Int(1),
        &facets.get_specific_value("dim", &["test\u{001E}two"])?,
    );

    // no hierarchy
    Assert::assert_false(facets.siblings_loaded());
    Assert::assert_false(facets.children_loaded());

    let result = top(&facets, 10, "dim")?;
    Assert::assert_equals(
        "dim=dim path=[] value=-1 childCount=2\n  test\u{001F}one (1)\n  test\u{001E}two (1)\n",
        result.to_string().as_str(),
    );

    assert_facet_result(
        &all(&facets, "dim")?,
        "dim",
        &[],
        2,
        NumericValue::Int(-1),
        &[lv("test\u{001E}two", 1), lv("test\u{001F}one", 1)],
    );
    writer.close()?;
    taxo_writer.close()?;
    r.close()?;
    taxo_reader.close()?;
    dir.close()?;
    taxo_dir.close()
}

fn test_require_dim_count() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let taxo_dir = LuceneTestCase::new_directory();
    let writer = random_writer(&dir)?;
    let taxo_writer = create_taxo_writer(&taxo_dir)?;

    let config = Arc::new(FacetsConfig::new());
    config.set_require_dim_count("dim", true);

    config.set_multi_valued("dim2", true);
    config.set_require_dim_count("dim2", true);

    config.set_multi_valued("dim3", true);
    config.set_hierarchical("dim3", true);
    config.set_require_dim_count("dim3", true);

    let mut doc = Document::new();
    doc.add(LuceneTestCase::new_text_field("field", "text", Store::NO));
    doc.add(FacetField::new("dim", &["a"])?);
    doc.add(FacetField::new("dim2", &["a"])?);
    doc.add(FacetField::new("dim2", &["b"])?);
    doc.add(FacetField::new("dim3", &["a", "b"])?);
    doc.add(FacetField::new("dim3", &["a", "c"])?);
    writer.add_document(&config.build_with_taxonomy(Some(&*taxo_writer), &doc)?)?;

    // NRT open
    let r = writer.get_reader()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&r));

    // NRT open
    let taxo_reader = nrt_taxo_reader(&taxo_writer)?;
    let facets = get_all_facets(DEFAULT_INDEX_FIELD_NAME, &searcher, &taxo_reader, &config)?;

    let one = NumericValue::Int(1);
    assert_equals(&one, &top(&facets, 10, "dim")?.value);
    assert_equals(&one, &all(&facets, "dim")?.value);
    assert_equals(&one, &top(&facets, 10, "dim2")?.value);
    assert_equals(&one, &all(&facets, "dim2")?.value);
    assert_equals(&one, &top(&facets, 10, "dim3")?.value);
    assert_equals(&one, &all(&facets, "dim3")?.value);
    expect_throws(IAE, || facets.get_specific_value("dim", &[]));

    assert_equals(&one, &facets.get_specific_value("dim2", &[])?);
    assert_equals(&one, &facets.get_specific_value("dim3", &[])?);
    writer.close()?;
    taxo_writer.close()?;
    r.close()?;
    taxo_reader.close()?;
    dir.close()?;
    taxo_dir.close()
}

// LUCENE-4583: make sure if we require > 32 KB for one
// document, we don't hit exc when using Facet42DocValuesFormat
fn test_many_facets_in_one_document() -> Result<()> {
    LuceneTestCase::assume_true(
        "default Codec doesn't support huge BinaryDocValues",
        TestUtil::field_supports_huge_binary_doc_values(DEFAULT_INDEX_FIELD_NAME),
    );
    let dir = LuceneTestCase::new_directory();
    let taxo_dir = LuceneTestCase::new_directory();
    let iwc = mock_iwc();
    let writer =
        RandomIndexWriter::new_with_config(&*LuceneTestCase::random(), dir.as_directory(), iwc)?;
    let taxo_writer = create_taxo_writer(&taxo_dir)?;

    let config = Arc::new(FacetsConfig::new());
    config.set_multi_valued("dim", true);

    let num_labels = if *TEST_NIGHTLY {
        TestUtil::next_int(&*LuceneTestCase::random(), 40000, 100000)
    } else {
        TestUtil::next_int(&*LuceneTestCase::random(), 4000, 10000)
    };

    let mut doc = Document::new();
    doc.add(LuceneTestCase::new_text_field("field", "text", Store::NO));
    for i in 0..num_labels {
        doc.add(FacetField::new("dim", &[i.to_string().as_str()])?);
    }
    writer.add_document(&config.build_with_taxonomy(Some(&*taxo_writer), &doc)?)?;

    // NRT open
    let r = writer.get_reader()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&r));

    // NRT open
    let taxo_reader = nrt_taxo_reader(&taxo_writer)?;

    let facets = get_all_facets(DEFAULT_INDEX_FIELD_NAME, &searcher, &taxo_reader, &config)?;

    let result = top(&facets, i32::MAX, "dim")?;
    Assert::assert_equals(num_labels as usize, result.label_values.len());
    let mut all_labels = HashSet::new();
    for label_value in &result.label_values {
        all_labels.insert(label_value.label.clone());
        Assert::assert_equals(1, number_int_value(&label_value.value));
    }
    Assert::assert_equals(num_labels as usize, all_labels.len());

    let all_children_result = all(&facets, "dim")?;
    Assert::assert_equals(num_labels as usize, result.label_values.len());
    let mut all_children_labels = HashSet::new();
    for label_value in &all_children_result.label_values {
        all_children_labels.insert(label_value.label.clone());
        Assert::assert_equals(1, number_int_value(&label_value.value));
    }
    Assert::assert_equals(num_labels as usize, all_children_labels.len());

    writer.close()?;
    r.close()?;
    taxo_writer.close()?;
    taxo_reader.close()?;
    dir.close()?;
    taxo_dir.close()
}

// Make sure we catch when app didn't declare field as
// hierarchical but it was:
fn test_detect_hierarchical_field() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let taxo_dir = LuceneTestCase::new_directory();
    let taxo_writer = create_taxo_writer(&taxo_dir)?;
    let writer = random_writer(&dir)?;
    let config = FacetsConfig::new();

    let mut doc = Document::new();
    doc.add(LuceneTestCase::new_text_field("field", "text", Store::NO));
    doc.add(FacetField::new("a", &["path", "other"])?);
    expect_throws(IAE, || {
        config.build_with_taxonomy(Some(&*taxo_writer), &doc)
    });

    writer.close()?;
    taxo_writer.close()?;
    dir.close()?;
    taxo_dir.close()
}

// Make sure we catch when app didn't declare field as
// multi-valued but it was:
fn test_detect_multi_valued_field() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let taxo_dir = LuceneTestCase::new_directory();
    let taxo_writer = create_taxo_writer(&taxo_dir)?;
    let writer = random_writer(&dir)?;
    let config = FacetsConfig::new();

    let mut doc = Document::new();
    doc.add(LuceneTestCase::new_text_field("field", "text", Store::NO));
    doc.add(FacetField::new("a", &["path"])?);
    doc.add(FacetField::new("a", &["path2"])?);
    expect_throws(IAE, || {
        config.build_with_taxonomy(Some(&*taxo_writer), &doc)
    });

    writer.close()?;
    taxo_writer.close()?;
    dir.close()?;
    taxo_dir.close()
}

fn test_separate_indexed_fields() -> Result<()> {
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
    let taxo_reader = nrt_taxo_reader(&taxo_writer)?;

    let sfc = LuceneTestCase::new_searcher(Arc::clone(&r))
        .search_with_manager(match_all(), &FacetsCollectorManager::new())?;
    let facets1 = get_taxonomy_facet_counts(&taxo_reader, &config, &sfc)?;
    let facets2 = get_taxonomy_facet_counts_field(&taxo_reader, &config, &sfc, "$b")?;
    Assert::assert_equals(
        r.max_doc(),
        number_int_value(&top(&*facets1, 10, "a")?.value),
    );
    Assert::assert_equals(r.max_doc(), number_int_value(&all(&*facets1, "a")?.value));
    Assert::assert_equals(
        r.max_doc(),
        number_int_value(&top(&*facets2, 10, "b")?.value),
    );
    Assert::assert_equals(r.max_doc(), number_int_value(&all(&*facets2, "b")?.value));
    iw.close()?;
    taxo_writer.close()?;
    taxo_reader.close()?;
    taxo_dir.close()?;
    r.close()?;
    index_dir.close()
}

fn test_count_root() -> Result<()> {
    // LUCENE-4882: FacetsAccumulator threw NPE if a FacetRequest was defined on CP.EMPTY
    let index_dir = LuceneTestCase::new_directory();
    let taxo_dir = LuceneTestCase::new_directory();

    let taxo_writer = Arc::new(DirectoryTaxonomyWriter::new(taxo_dir.as_directory())?);
    let iw = IndexWriter::new(index_dir.as_directory(), mock_iwc())?;
    let config = Arc::new(FacetsConfig::new());
    let mut i = LuceneTestCase::at_least(30);
    while i > 0 {
        let mut doc = Document::new();
        doc.add(FacetField::new("a", &["1"])?);
        doc.add(FacetField::new("b", &["1"])?);
        iw.add_document(&config.build_with_taxonomy(Some(&*taxo_writer), &doc)?)?;
        i -= 1;
    }

    let r = directory_reader::open_with_writer(&*iw)?;
    let taxo_reader = nrt_taxo_reader(&taxo_writer)?;

    let facets = get_all_facets(
        DEFAULT_INDEX_FIELD_NAME,
        &LuceneTestCase::new_searcher(Arc::clone(&r)),
        &taxo_reader,
        &config,
    )?;

    let all_dims_result = facets.get_all_dims(10)?;
    for result in &all_dims_result {
        Assert::assert_equals(r.num_docs(), number_int_value(&result.value));
    }

    // test override implementation of getTopDims
    if !all_dims_result.is_empty() {
        let top_n_dims_result = facets.get_top_dims(1, 10)?;
        assert_equals(&all_dims_result[0], &top_n_dims_result[0]);
    }

    // test getTopDims(0, 1)
    expect_throws(IAE, || facets.get_top_dims(0, 1));

    // test getTopDims(1, 0) with topNChildren = 0
    expect_throws(IAE, || facets.get_top_dims(1, 0));

    // test getAllDims(0)
    expect_throws(IAE, || facets.get_all_dims(0));

    iw.close()?;
    taxo_writer.close()?;
    taxo_reader.close()?;
    taxo_dir.close()?;
    r.close()?;
    index_dir.close()
}

fn test_get_facet_results_twice() -> Result<()> {
    // LUCENE-4893: counts were multiplied as many times as getFacetResults was called.
    let index_dir = LuceneTestCase::new_directory();
    let taxo_dir = LuceneTestCase::new_directory();

    let taxo_writer = Arc::new(DirectoryTaxonomyWriter::new(taxo_dir.as_directory())?);
    let iw = IndexWriter::new(index_dir.as_directory(), mock_iwc())?;
    let config = Arc::new(FacetsConfig::new());

    let mut doc = Document::new();
    doc.add(FacetField::new("a", &["1"])?);
    doc.add(FacetField::new("b", &["1"])?);
    iw.add_document(&config.build_with_taxonomy(Some(&*taxo_writer), &doc)?)?;

    let r = directory_reader::open_with_writer(&*iw)?;
    let taxo_reader = nrt_taxo_reader(&taxo_writer)?;

    let facets = get_all_facets(
        DEFAULT_INDEX_FIELD_NAME,
        &LuceneTestCase::new_searcher(Arc::clone(&r)),
        &taxo_reader,
        &config,
    )?;

    let res1 = facets.get_all_dims(10)?;
    let res2 = facets.get_all_dims(10)?;
    assert_equals_msg(
        Some("calling getFacetResults twice should return the .equals()=true result"),
        &res1,
        &res2,
    );

    // test getTopDims(n, 10)
    if !res1.is_empty() {
        for i in 1..res1.len() {
            assert_equals(&res1[0..i].to_vec(), &facets.get_top_dims(i as i32, 10)?);
        }
    }

    iw.close()?;
    taxo_writer.close()?;
    taxo_reader.close()?;
    taxo_dir.close()?;
    r.close()?;
    index_dir.close()
}

fn test_child_count() -> Result<()> {
    // LUCENE-4885: FacetResult.numValidDescendants was not set properly by FacetsAccumulator
    let index_dir = LuceneTestCase::new_directory();
    let taxo_dir = LuceneTestCase::new_directory();

    let taxo_writer = Arc::new(DirectoryTaxonomyWriter::new(taxo_dir.as_directory())?);
    let iw = IndexWriter::new(index_dir.as_directory(), mock_iwc())?;
    let config = Arc::new(FacetsConfig::new());
    for i in 0..10 {
        let mut doc = Document::new();
        doc.add(FacetField::new("a", &[i.to_string().as_str()])?);
        iw.add_document(&config.build_with_taxonomy(Some(&*taxo_writer), &doc)?)?;
    }

    let r = directory_reader::open_with_writer(&*iw)?;
    let taxo_reader = nrt_taxo_reader(&taxo_writer)?;

    let facets = get_all_facets(
        DEFAULT_INDEX_FIELD_NAME,
        &LuceneTestCase::new_searcher(Arc::clone(&r)),
        &taxo_reader,
        &config,
    )?;

    Assert::assert_equals(10, top(&facets, 2, "a")?.child_count);
    Assert::assert_equals(10, all(&facets, "a")?.child_count);

    iw.close()?;
    taxo_writer.close()?;
    taxo_reader.close()?;
    taxo_dir.close()?;
    r.close()?;
    index_dir.close()
}

/// The body of `testMultiValuedFieldCountAll` (`hierarchical_dim` "Publish
/// Date", with that dimension indexed) and `testCountAllSingleValuedField`
/// ("Author").
fn count_all_two_docs(hierarchical_dim: &str, with_date: bool) -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let taxo_dir = LuceneTestCase::new_directory();

    let taxo_writer = create_taxo_writer(&taxo_dir)?;

    let config = Arc::new(FacetsConfig::new());
    config.set_hierarchical(hierarchical_dim, true);

    let writer = random_writer(&dir)?;

    for (id, author, date) in [
        ("0", "Bob", ["2010", "10", "15"]),
        ("1", "Lisa", ["2010", "10", "20"]),
    ] {
        let mut doc = Document::new();
        doc.add(StringField::new("id", id.to_string(), Store::NO)?);
        doc.add(FacetField::new("Author", &[author])?);
        if with_date {
            doc.add(FacetField::new("Publish Date", &date)?);
        }
        writer.add_document(&config.build_with_taxonomy(Some(&*taxo_writer), &doc)?)?;
    }

    let r1 = writer.get_reader()?;
    let searcher1 = LuceneTestCase::new_searcher(Arc::clone(&r1));
    let taxo_reader = nrt_taxo_reader(&taxo_writer)?;
    let facets = FastTaxonomyFacetCounts::count_all_docs(
        DEFAULT_INDEX_FIELD_NAME,
        searcher1.get_index_reader(),
        Arc::clone(&taxo_reader),
        Arc::clone(&config),
    )?;

    Assert::assert_equals(
        "dim=Author path=[] value=2 childCount=2\n  Bob (1)\n  Lisa (1)",
        top(&facets, 10, "Author")?.to_string().trim(),
    );

    assert_facet_result(
        &all(&facets, "Author")?,
        "Author",
        &[],
        2,
        NumericValue::Int(2),
        &[lv("Bob", 1), lv("Lisa", 1)],
    );

    // -- delete to trigger liveDocs != null
    writer.delete_documents(Term::from_text("id", "0"))?;
    let r2 = writer.get_reader()?;
    let searcher2 = LuceneTestCase::new_searcher(Arc::clone(&r2));
    let facets = FastTaxonomyFacetCounts::count_all_docs(
        DEFAULT_INDEX_FIELD_NAME,
        searcher2.get_index_reader(),
        Arc::clone(&taxo_reader),
        Arc::clone(&config),
    )?;

    Assert::assert_equals(
        "dim=Author path=[] value=1 childCount=1\n  Lisa (1)",
        top(&facets, 10, "Author")?.to_string().trim(),
    );

    assert_facet_result(
        &all(&facets, "Author")?,
        "Author",
        &[],
        1,
        NumericValue::Int(1),
        &[lv("Lisa", 1)],
    );

    writer.close()?;
    taxo_writer.close()?;
    r1.close()?;
    r2.close()?;
    taxo_reader.close()?;
    taxo_dir.close()?;
    dir.close()
}

fn test_multi_valued_field_count_all() -> Result<()> {
    count_all_two_docs("Publish Date", true)
}

fn test_count_all_single_valued_field() -> Result<()> {
    count_all_two_docs("Author", false)
}

fn index_two_docs(
    taxo_writer: &dyn TaxonomyWriter,
    index_writer: &IndexWriter,
    config: Option<&FacetsConfig>,
    with_content: bool,
) -> Result<()> {
    for i in 0..2 {
        let mut doc = Document::new();
        if with_content {
            doc.add(StringField::new("f", "a".to_string(), Store::NO)?);
        }
        match config {
            Some(config) => {
                doc.add(FacetField::new("A", &[i.to_string().as_str()])?);
                index_writer.add_document(&config.build_with_taxonomy(Some(taxo_writer), &doc)?)?;
            }
            None => {
                index_writer.add_document(&doc)?;
            }
        }
    }

    index_writer.commit()?;
    Ok(())
}

fn test_segments_without_categories_or_results() -> Result<()> {
    // tests the accumulator when there are segments with no results
    let index_dir = LuceneTestCase::new_directory();
    let taxo_dir = LuceneTestCase::new_directory();

    let iwc = mock_iwc();
    iwc.set_merge_policy(Some(NoMergePolicy::instance()))?; // prevent merges
    let index_writer = IndexWriter::new(index_dir.as_directory(), iwc)?;

    let taxo_writer = DirectoryTaxonomyWriter::new(taxo_dir.as_directory())?;
    let config = Arc::new(FacetsConfig::new());
    index_two_docs(&taxo_writer, &index_writer, Some(&config), false)?; // 1st segment, no content, with categories
    index_two_docs(&taxo_writer, &index_writer, None, true)?; // 2nd segment, with content, no categories
    index_two_docs(&taxo_writer, &index_writer, Some(&config), true)?; // 3rd segment ok
    index_two_docs(&taxo_writer, &index_writer, None, false)?; // 4th segment, no content, or categories
    index_two_docs(&taxo_writer, &index_writer, None, true)?; // 5th segment, with content, no categories
    index_two_docs(&taxo_writer, &index_writer, Some(&config), true)?; // 6th segment, with content, with categories
    index_two_docs(&taxo_writer, &index_writer, None, true)?; // 7th segment, with content, no categories
    index_writer.close()?;
    taxo_writer.close()?;

    let index_reader = directory_reader::open(index_dir.as_directory())?;
    let taxo_reader: Arc<dyn TaxonomyReader> =
        Arc::new(DirectoryTaxonomyReader::open(taxo_dir.as_directory())?);
    let index_searcher = LuceneTestCase::new_searcher(Arc::clone(&index_reader));

    // search for "f:a", only segments 1 and 3 should match results
    let q: Arc<dyn Query> = Arc::new(TermQuery::new(Term::from_text("f", "a")));
    let sfc = index_searcher.search_with_manager(q, &FacetsCollectorManager::new())?;
    let facets = get_taxonomy_facet_counts(&taxo_reader, &config, &sfc)?;
    let result = top(&*facets, 10, "A")?;
    assert_equals_msg(
        Some("wrong number of children"),
        &2,
        &result.label_values.len(),
    );
    for label_value in &result.label_values {
        assert_equals_msg(
            Some(&format!("wrong weight for child {}", label_value.label)),
            &2,
            &number_int_value(&label_value.value),
        );
    }
    let all_children_result = all(&*facets, "A")?;
    assert_equals_msg(
        Some("wrong number of children"),
        &2,
        &all_children_result.label_values.len(),
    );
    for label_value in &all_children_result.label_values {
        assert_equals_msg(
            Some(&format!("wrong weight for child {}", label_value.label)),
            &2,
            &number_int_value(&label_value.value),
        );
    }

    index_reader.close()?;
    taxo_reader.close()?;
    index_dir.close()?;
    taxo_dir.close()
}

fn test_random() -> Result<()> {
    let tokens = get_random_tokens(10);
    let index_dir = LuceneTestCase::new_directory();
    let taxo_dir = LuceneTestCase::new_directory();

    let w = random_writer(&index_dir)?;
    let tw = Arc::new(DirectoryTaxonomyWriter::new(taxo_dir.as_directory())?);
    let config = Arc::new(FacetsConfig::new());
    let num_docs = LuceneTestCase::at_least(1000) as usize;
    let num_dims = TestUtil::next_int(&*LuceneTestCase::random(), 1, 7) as usize;
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
                doc.add(FacetField::new(&format!("dim{j}"), &[dim.as_str()])?);
            }
        }
        w.add_document(&config.build_with_taxonomy(Some(&*tw), &doc)?)?;
    }

    // NRT open
    let r = w.get_reader()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&r));

    // NRT open
    let tr = nrt_taxo_reader(&tw)?;

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
        let facets = get_taxonomy_facet_counts(&tr, &config, &fc)?;

        // Slow, yet hopefully bug-free, faceting:
        let mut expected_counts: Vec<JavaHashMap<i32>> =
            (0..num_dims).map(|_| JavaHashMap::new()).collect();
        let mut expected_labels: Vec<Vec<FacetLabel>> = Vec::new();

        for doc in &test_docs {
            if doc.content == *search_token {
                let mut facet_labels = Vec::new();
                for j in 0..num_dims {
                    if let Some(dim) = &doc.dims[j] {
                        let v = expected_counts[j].get(dim).copied().unwrap_or(0);
                        expected_counts[j].insert(dim.clone(), v + 1);
                        // Add document facet labels
                        facet_labels.push(FacetLabel::new([format!("dim{j}"), dim.clone()])?);
                    }
                }
                expected_labels.push(facet_labels);
            }
        }

        let mut expected = Vec::new();
        for (i, counts) in expected_counts.iter().enumerate() {
            let mut label_values = Vec::new();
            let mut tot_count = 0;
            for (key, value) in counts.iter() {
                label_values.push(LabelAndValue::new(key.clone(), NumericValue::Int(*value)));
                tot_count += *value;
            }
            sort_label_values(&mut label_values);
            if tot_count > 0 {
                let child_count = label_values.len() as i32;
                expected.push(FacetResult::new(
                    format!("dim{i}"),
                    Vec::new(),
                    NumericValue::Int(tot_count),
                    label_values,
                    child_count,
                ));
            }
        }

        // Sort by highest value, tie-break by value:
        sort_facet_results(&mut expected);

        let mut actual = facets.get_all_dims(10)?;
        // Messy: fixup ties
        sort_ties_results(&mut actual);

        assert_equals(&expected, &actual);

        // test getTopDims
        if !actual.is_empty() {
            let mut top_n_dims_result = facets.get_top_dims(actual.len() as i32, 10)?;
            sort_ties_results(&mut top_n_dims_result);
            assert_equals(&actual, &top_n_dims_result);
        }

        // Test facet labels for each matching test doc
        let mut actual_labels = get_all_taxonomy_facet_labels(None, &tr, &fc)?;
        Assert::assert_equals(expected_labels.len(), actual_labels.len());
        assert_equals(
            sorted_facet_labels(&mut expected_labels),
            sorted_facet_labels(&mut actual_labels),
        );

        // Test facet labels for each matching test doc, given a specific dimension chosen randomly
        let dimension = format!(
            "dim{}",
            LuceneTestCase::random().next_int_bound(num_dims as i32)
        );
        for list in expected_labels.iter_mut() {
            list.retain(|f| f.components[0] == dimension);
        }

        let mut actual_labels = get_all_taxonomy_facet_labels(Some(&dimension), &tr, &fc)?;
        assert_equals(
            sorted_facet_labels(&mut expected_labels),
            sorted_facet_labels(&mut actual_labels),
        );
    }

    w.close()?;
    tw.close()?;
    r.close()?;
    tr.close()?;
    index_dir.close()?;
    taxo_dir.close()
}

fn sorted_facet_labels(all_facet_labels: &mut Vec<Vec<FacetLabel>>) -> &Vec<Vec<FacetLabel>> {
    // Sort each inner list since there is no guaranteed order in which
    // FacetLabels are expected to be retrieved for each document.
    for facet_labels in all_facet_labels.iter_mut() {
        facet_labels.sort_by(|a, b| a.compare_to(b).cmp(&0));
    }

    all_facet_labels.sort_by(|o1, o2| {
        let diff = o1.len() as i64 - o2.len() as i64;
        if diff != 0 {
            return diff.cmp(&0);
        }

        // the lists are equal in size and sorted
        for i in 0..o1.len() {
            let comp = o1[i].compare_to(&o2[i]);
            if comp != 0 {
                return comp.cmp(&0);
            }
        }
        // all elements are equal
        std::cmp::Ordering::Equal
    });

    all_facet_labels
}

fn get_all_facets(
    index_field_name: &str,
    searcher: &IndexSearcher,
    taxo_reader: &Arc<dyn TaxonomyReader>,
    config: &Arc<FacetsConfig>,
) -> Result<FastTaxonomyFacetCounts> {
    if LuceneTestCase::random().next_boolean() {
        // MatchAllDocsQuery is for "browsing" (counts facets
        // for all non-deleted docs in the index); normally
        // you'd use a "normal" query, and use MultiCollector to
        // wrap collecting the "normal" hits and also facets:
        let c = searcher.search_with_manager(match_all(), &FacetsCollectorManager::new())?;

        FastTaxonomyFacetCounts::new(Arc::clone(taxo_reader), Arc::clone(config), &c)
    } else {
        FastTaxonomyFacetCounts::count_all_docs(
            index_field_name,
            searcher.get_index_reader(),
            Arc::clone(taxo_reader),
            Arc::clone(config),
        )
    }
}
