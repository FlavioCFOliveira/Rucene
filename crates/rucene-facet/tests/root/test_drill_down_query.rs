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

//! Port of `org.apache.lucene.facet.TestDrillDownQuery`.

use std::sync::{Arc, Mutex, MutexGuard};

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::{AfterClass, Assert, BeforeClass};
use rucene::document::{Document, Store, TextField};
use rucene::error::Result;
use rucene::index::directory_reader::DirectoryReader;
use rucene::index::Term;
use rucene::search::query::Query;
use rucene::search::term_query::TermQuery;
use rucene::search::MatchAllDocsQuery;
use rucene_facet::drill_down_query::{self, DrillDownQuery};
use rucene_facet::facet_field::FacetField;
use rucene_facet::facets_collector_manager::FacetsCollectorManager;
use rucene_facet::facets_config::{DrillDownTermsIndexing, FacetsConfig};
use rucene_facet::taxonomy::directory::directory_taxonomy_reader::DirectoryTaxonomyReader;
use rucene_facet::taxonomy::directory::directory_taxonomy_writer::DirectoryTaxonomyWriter;
use rucene_facet::taxonomy::int_association_facet_field::IntAssociationFacetField;
use rucene_facet::taxonomy::taxonomy_reader::TaxonomyReader;
use rucene_facet::taxonomy::taxonomy_writer::TaxonomyWriter;
use rucene_test_framework::analysis::mock_tokenizer::KEYWORD;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::search::query_utils::QueryUtils;
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::LuceneTestCase;

use crate::facet_support::core_support::test;
use crate::facet_support::search_support::static_hook;
use crate::facet_support::{facet_class, get_taxonomy_facet_counts_field};

/// `TestDrillDownQuery`.
pub fn class_def() -> Arc<Class> {
    facet_class(
        "org.apache.lucene.facet.TestDrillDownQuery",
        vec![
            static_hook(
                "afterClassDrillDownQueryTest",
                AfterClass,
                after_class_drill_down_query_test,
            ),
            static_hook(
                "beforeClassDrillDownQueryTest",
                BeforeClass,
                before_class_drill_down_query_test,
            ),
            test("testAndOrs", test_and_ors),
            test("testQuery", test_query),
            test(
                "testQueryImplicitDefaultParams",
                test_query_implicit_default_params,
            ),
            test("testZeroLimit", test_zero_limit),
            test("testScoring", test_scoring),
            test("testScoringNoBaseQuery", test_scoring_no_base_query),
            test("testTermNonDefault", test_term_non_default),
            test("testClone", test_clone),
            test("testNoDrillDown", test_no_drill_down),
            test(
                "testSkipDrillDownTermsIndexing",
                test_skip_drill_down_terms_indexing,
            ),
            test(
                "testDrillDownTermsDifferentOptions",
                test_drill_down_terms_different_options,
            ),
            test(
                "testDrillDownTermsDefaultWithHierarchicalSetting",
                test_drill_down_terms_default_with_hierarchical_setting,
            ),
            test("testGetDrillDownQueries", test_get_drill_down_queries),
        ],
    )
}

/// The static fields `reader`, `taxo`, `dir`, `taxoDir`, `config`.
struct Statics {
    reader: Arc<dyn DirectoryReader>,
    taxo: Arc<dyn TaxonomyReader>,
    dir: TestDirectory,
    taxo_dir: TestDirectory,
    config: Arc<FacetsConfig>,
}

static STATICS: Mutex<Option<Statics>> = Mutex::new(None);

fn statics() -> MutexGuard<'static, Option<Statics>> {
    STATICS.lock().unwrap_or_else(|e| e.into_inner())
}

fn reader() -> Arc<dyn DirectoryReader> {
    Arc::clone(&statics().as_ref().expect("beforeClass").reader)
}

fn taxo() -> Arc<dyn TaxonomyReader> {
    Arc::clone(&statics().as_ref().expect("beforeClass").taxo)
}

fn config() -> Arc<FacetsConfig> {
    Arc::clone(&statics().as_ref().expect("beforeClass").config)
}

fn after_class_drill_down_query_test() -> Result<()> {
    if let Some(s) = statics().take() {
        s.reader.close()?;
        s.taxo.close()?;
        s.dir.close()?;
        s.taxo_dir.close()?;
    }
    Ok(())
}

fn keyword_writer(dir: &TestDirectory) -> Result<RandomIndexWriter> {
    let r = LuceneTestCase::random();
    RandomIndexWriter::new_with_config(
        &*r,
        dir.as_directory(),
        LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(
            MockAnalyzer::with_automaton(&*r, Arc::clone(&KEYWORD), false),
        ))),
    )
}

fn before_class_drill_down_query_test() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let r = LuceneTestCase::random();
    let writer = keyword_writer(&dir)?;

    let taxo_dir = LuceneTestCase::new_directory();
    let taxo_writer = DirectoryTaxonomyWriter::new(taxo_dir.as_directory())?;
    let config = FacetsConfig::new();

    // Randomize the per-dim config:
    config.set_hierarchical("a", r.next_boolean());
    config.set_multi_valued("a", r.next_boolean());
    if r.next_boolean() {
        config.set_index_field_name("a", "$a");
    }
    config.set_require_dim_count("a", true);
    config.set_hierarchical("b", r.next_boolean());
    config.set_multi_valued("b", r.next_boolean());
    if r.next_boolean() {
        config.set_index_field_name("b", "$b");
    }
    config.set_require_dim_count("b", true);

    for i in 0..100 {
        let mut doc = Document::new();
        if i % 2 == 0 {
            // 50
            doc.add(TextField::new("content", "foo".to_string(), Store::NO)?);
        }
        if i % 3 == 0 {
            // 33
            doc.add(TextField::new("content", "bar".to_string(), Store::NO)?);
        }
        if i % 4 == 0 {
            // 25
            if r.next_boolean() {
                doc.add(FacetField::new("a", &["1"])?);
            } else {
                doc.add(FacetField::new("a", &["2"])?);
            }
        }
        if i % 5 == 0 {
            // 20
            doc.add(FacetField::new("b", &["1"])?);
        }
        writer.add_document(&config.build_with_taxonomy(Some(&taxo_writer), &doc)?)?;
    }

    taxo_writer.close()?;
    let reader = writer.get_reader()?;
    writer.close()?;

    let taxo: Arc<dyn TaxonomyReader> =
        Arc::new(DirectoryTaxonomyReader::open(taxo_dir.as_directory())?);
    *statics() = Some(Statics {
        reader,
        taxo,
        dir,
        taxo_dir,
        config: Arc::new(config),
    });
    Ok(())
}

fn ddq(config: &Arc<FacetsConfig>) -> DrillDownQuery {
    DrillDownQuery::new(Arc::clone(config))
}

fn ddq_base(config: &Arc<FacetsConfig>, base: Arc<dyn Query>) -> DrillDownQuery {
    DrillDownQuery::with_base_query(Arc::clone(config), Some(base))
}

fn test_and_ors() -> Result<()> {
    let config = config();
    let searcher = LuceneTestCase::new_searcher(reader());
    // test (a/1 OR a/2) AND b/1
    let mut q = ddq(&config);
    q.add("a", &["1"])?;
    q.add("a", &["2"])?;
    q.add("b", &["1"])?;
    let docs = searcher.search(Arc::new(q), 100)?;
    Assert::assert_equals(5i64, docs.total_hits.value());
    Ok(())
}

fn test_query() -> Result<()> {
    let config = config();
    let searcher = LuceneTestCase::new_searcher(reader());

    // Making sure the query yields 25 documents with the facet "a"
    let mut q = ddq(&config);
    q.add("a", &[])?;
    QueryUtils::check(&q);
    let q: Arc<dyn Query> = Arc::new(q);
    let docs = searcher.search(Arc::clone(&q), 100)?;
    Assert::assert_equals(25i64, docs.total_hits.value());

    // Making sure the query yields 5 documents with the facet "b" and the
    // previous (facet "a") query as a base query
    let mut q2 = ddq_base(&config, q);
    q2.add("b", &[])?;
    let docs = searcher.search(Arc::new(q2), 100)?;
    Assert::assert_equals(5i64, docs.total_hits.value());

    // Making sure that a query of both facet "a" and facet "b" yields 5 results
    let mut q3 = ddq(&config);
    q3.add("a", &[])?;
    q3.add("b", &[])?;
    let docs = searcher.search(Arc::new(q3), 100)?;
    Assert::assert_equals(5i64, docs.total_hits.value());

    // Check that content:foo (which yields 50% results) and facet/b (which yields 20%)
    // would gather together 10 results (10%..)
    let foo_query: Arc<dyn Query> = Arc::new(TermQuery::new(Term::from_text("content", "foo")));
    let mut q4 = ddq_base(&config, foo_query);
    q4.add("b", &[])?;
    let docs = searcher.search(Arc::new(q4), 100)?;
    Assert::assert_equals(10i64, docs.total_hits.value());
    Ok(())
}

fn test_query_implicit_default_params() -> Result<()> {
    let config = config();
    let searcher = LuceneTestCase::new_searcher(reader());

    // Create the base query to start with
    let mut q = ddq(&config);
    q.add("a", &[])?;

    // Making sure the query yields 5 documents with the facet "b" and the
    // previous (facet "a") query as a base query
    let mut q2 = ddq_base(&config, Arc::new(q));
    q2.add("b", &[])?;
    let docs = searcher.search(Arc::new(q2), 100)?;
    Assert::assert_equals(5i64, docs.total_hits.value());

    // Check that content:foo (which yields 50% results) and facet/b (which yields 20%)
    // would gather together 10 results (10%..)
    let foo_query: Arc<dyn Query> = Arc::new(TermQuery::new(Term::from_text("content", "foo")));
    let mut q4 = ddq_base(&config, foo_query);
    q4.add("b", &[])?;
    let docs = searcher.search(Arc::new(q4), 100)?;
    Assert::assert_equals(10i64, docs.total_hits.value());
    Ok(())
}

fn test_zero_limit() -> Result<()> {
    let config = config();
    let searcher = LuceneTestCase::new_searcher(reader());
    let mut q = ddq(&config);
    q.add("b", &["1"])?;
    let limit = 0;
    let result = FacetsCollectorManager::search(
        &searcher,
        Arc::new(q),
        limit,
        &FacetsCollectorManager::new(),
    )?;
    let facet_collector = result.facets_collector();
    let facets = get_taxonomy_facet_counts_field(
        &taxo(),
        &config,
        facet_collector,
        &config.get_dim_config("b").index_field_name(),
    )?;
    Assert::assert_true(facets.get_top_children(10, "b", &[])?.is_some());
    Ok(())
}

fn test_scoring() -> Result<()> {
    // verify that drill-down queries do not modify scores
    let config = config();
    let reader = reader();
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));

    let mut scores = vec![0f32; reader.max_doc() as usize];

    let q: Arc<dyn Query> = Arc::new(TermQuery::new(Term::from_text("content", "foo")));
    let docs = searcher.search(Arc::clone(&q), reader.max_doc())?; // fetch all available docs to this query
    for sd in &docs.score_docs {
        scores[sd.doc as usize] = sd.score;
    }

    // create a drill-down query with category "a", scores should not change
    let mut q2 = ddq_base(&config, q);
    q2.add("a", &[])?;
    let docs = searcher.search(Arc::new(q2), reader.max_doc())?; // fetch all available docs to this query
    for sd in &docs.score_docs {
        Assert::assert_equals_float_msg(
            Some(&format!("score of doc={} modified", sd.doc)),
            scores[sd.doc as usize],
            sd.score,
            0.0,
        );
    }
    Ok(())
}

fn test_scoring_no_base_query() -> Result<()> {
    // verify that drill-down queries (with no base query) returns 0.0 score
    let config = config();
    let reader = reader();
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));

    let mut q = ddq(&config);
    q.add("a", &[])?;
    let docs = searcher.search(Arc::new(q), reader.max_doc())?; // fetch all available docs to this query
    for sd in &docs.score_docs {
        Assert::assert_equals_float_msg(None, 0.0, sd.score, 0.0);
    }
    Ok(())
}

fn test_term_non_default() -> Result<()> {
    let config = config();
    let a_field = config.get_dim_config("a").index_field_name();
    let term_a = drill_down_query::term(&a_field, "a", &[])?;
    Assert::assert_equals(Term::from_text(&a_field, "a"), term_a);
    let b_field = config.get_dim_config("b").index_field_name();
    let term_b = drill_down_query::term(&b_field, "b", &[])?;
    Assert::assert_equals(Term::from_text(&b_field, "b"), term_b);
    Ok(())
}

fn test_clone() -> Result<()> {
    let config = config();
    let mut q = ddq_base(&config, Arc::new(MatchAllDocsQuery::instance()));
    q.add("a", &[])?;

    let mut clone = q.clone_query();
    clone.add("b", &[])?;

    let (qs, cs) = (q.to_query_string(""), clone.to_query_string(""));
    Assert::assert_false_msg(
        &format!("query wasn't cloned: source={qs} clone={cs}"),
        qs == cs,
    );
    Ok(())
}

/// `query.rewrite(searcher)`: the query itself when unchanged.
fn rewrite(
    q: Arc<dyn Query>,
    searcher: &rucene::search::index_searcher::IndexSearcher,
) -> Result<Arc<dyn Query>> {
    Ok(q.rewrite(searcher)?.unwrap_or(q))
}

fn test_no_drill_down() -> Result<()> {
    let config = config();
    let base: Arc<dyn Query> = Arc::new(MatchAllDocsQuery::instance());
    let q: Arc<dyn Query> = Arc::new(ddq_base(&config, Arc::clone(&base)));
    let searcher = LuceneTestCase::new_searcher(reader());
    let rewrite = rewrite(rewrite(q, &searcher)?, &searcher)?;
    Assert::assert_true_msg(
        &format!(
            "expected:<{}> but was:<{}>",
            base.to_query_string(""),
            rewrite.to_query_string("")
        ),
        base.query_eq(&*rewrite),
    );
    Ok(())
}

/// `searcher.count(q)` for a drill-down `q` built by `add`.
fn count(
    searcher: &rucene::search::index_searcher::IndexSearcher,
    config: &Arc<FacetsConfig>,
    dim: &str,
    path: &[&str],
) -> Result<i32> {
    let mut q = ddq(config);
    q.add(dim, path)?;
    searcher.count(Arc::new(q))
}

fn test_skip_drill_down_terms_indexing() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let writer = keyword_writer(&dir)?;
    let taxo_dir = LuceneTestCase::new_directory();
    let taxo_writer = DirectoryTaxonomyWriter::new(taxo_dir.as_directory())?;
    let config = Arc::new(FacetsConfig::new());
    config.set_drill_down_terms_indexing("a", DrillDownTermsIndexing::FULL_PATH_ONLY);
    config.set_drill_down_terms_indexing("b", DrillDownTermsIndexing::FULL_PATH_ONLY);

    let mut doc = Document::new();
    doc.add(FacetField::new("a", &["1"])?);
    doc.add(FacetField::new("b", &["2"])?);
    writer.add_document(&config.build_with_taxonomy(Some(&taxo_writer), &doc)?)?;
    taxo_writer.close()?;

    let reader = writer.get_reader()?;
    let taxo_reader = DirectoryTaxonomyReader::open(taxo_dir.as_directory())?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));

    // no hits because we disabled dimension drill down completely
    Assert::assert_equals(0, count(&searcher, &config, "a", &[])?);
    Assert::assert_equals(1, count(&searcher, &config, "a", &["1"])?);
    // no hits because we disabled dimension drill down completely
    Assert::assert_equals(0, count(&searcher, &config, "b", &[])?);
    Assert::assert_equals(1, count(&searcher, &config, "b", &["2"])?);

    taxo_reader.close()?;
    reader.close()?;
    writer.close()?;
    dir.close()?;
    taxo_dir.close()
}

fn test_drill_down_terms_different_options() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let writer = keyword_writer(&dir)?;
    let taxo_dir = LuceneTestCase::new_directory();
    let taxo_writer = DirectoryTaxonomyWriter::new(taxo_dir.as_directory())?;
    let config = Arc::new(FacetsConfig::new());
    for d in ["a", "b", "c", "d", "e"] {
        config.set_hierarchical(d, true);
    }
    config.set_drill_down_terms_indexing("a", DrillDownTermsIndexing::NONE);
    config.set_drill_down_terms_indexing("b", DrillDownTermsIndexing::FULL_PATH_ONLY);
    config.set_drill_down_terms_indexing("c", DrillDownTermsIndexing::ALL_PATHS_NO_DIM);
    config.set_drill_down_terms_indexing("d", DrillDownTermsIndexing::DIMENSION_AND_FULL_PATH);
    config.set_drill_down_terms_indexing("e", DrillDownTermsIndexing::ALL);
    config.set_drill_down_terms_indexing("f", DrillDownTermsIndexing::NONE);
    config.set_index_field_name("f", "facet-for-f");

    let mut doc = Document::new();
    doc.add(FacetField::new("a", &["a1", "a2", "a3"])?);
    doc.add(FacetField::new("b", &["b1", "b2", "b3"])?);
    doc.add(FacetField::new("c", &["c1", "c2", "c3"])?);
    doc.add(FacetField::new("d", &["d1", "d2", "d3"])?);
    doc.add(FacetField::new("e", &["e1", "e2", "e3"])?);
    doc.add(IntAssociationFacetField::new(5, "f", &["f1"])?);
    writer.add_document(&config.build_with_taxonomy(Some(&taxo_writer), &doc)?)?;
    taxo_writer.close()?;

    let reader = writer.get_reader()?;
    let taxo_reader = DirectoryTaxonomyReader::open(taxo_dir.as_directory())?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    let s = &searcher;
    let c = &config;

    // Verifies for FacetsConfig.DrillDownTermsIndexing.NONE option
    Assert::assert_equals(0, count(s, c, "a", &[])?);
    Assert::assert_equals(0, count(s, c, "a", &["a1"])?);
    Assert::assert_equals(0, count(s, c, "a", &["a1", "a2"])?);
    Assert::assert_equals(0, count(s, c, "a", &["a1", "a2", "a3"])?);

    // Verifies for FacetsConfig.DrillDownTermsIndexing.FULL_PATH_ONLY option
    Assert::assert_equals(0, count(s, c, "b", &[])?);
    Assert::assert_equals(0, count(s, c, "b", &["b1"])?);
    Assert::assert_equals(0, count(s, c, "b", &["b1", "b2"])?);
    Assert::assert_equals(1, count(s, c, "b", &["b1", "b2", "b3"])?);

    // Verifies for FacetsConfig.DrillDownTermsIndexing.ALL_PATHS_NO_DIM option
    Assert::assert_equals(0, count(s, c, "c", &[])?);
    Assert::assert_equals(1, count(s, c, "c", &["c1"])?);
    Assert::assert_equals(1, count(s, c, "c", &["c1", "c2"])?);
    Assert::assert_equals(1, count(s, c, "c", &["c1", "c2", "c3"])?);

    // Verifies for FacetsConfig.DrillDownTermsIndexing.DIMENSION_AND_FULL_PATH option
    Assert::assert_equals(1, count(s, c, "d", &[])?);
    Assert::assert_equals(0, count(s, c, "d", &["d1"])?);
    Assert::assert_equals(0, count(s, c, "d", &["d1", "d2"])?);
    Assert::assert_equals(1, count(s, c, "d", &["d1", "d2", "d3"])?);

    // Verifies for FacetsConfig.DrillDownTermsIndexing.ALL option
    Assert::assert_equals(1, count(s, c, "e", &[])?);
    Assert::assert_equals(1, count(s, c, "e", &["e1"])?);
    Assert::assert_equals(1, count(s, c, "e", &["e1", "e2"])?);
    Assert::assert_equals(1, count(s, c, "e", &["e1", "e2", "e3"])?);

    // Verifies for FacetsConfig.DrillDownTermsIndexing.DIMENSION_AND_FULL_PATH option with
    // IntAssociationFacetField
    Assert::assert_equals(0, count(s, c, "f", &[])?);
    Assert::assert_equals(0, count(s, c, "f", &["f1"])?);

    taxo_reader.close()?;
    reader.close()?;
    writer.close()?;
    dir.close()?;
    taxo_dir.close()
}

fn test_drill_down_terms_default_with_hierarchical_setting() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let writer = keyword_writer(&dir)?;
    let taxo_dir = LuceneTestCase::new_directory();
    let taxo_writer = DirectoryTaxonomyWriter::new(taxo_dir.as_directory())?;
    let config = Arc::new(FacetsConfig::new());
    config.set_hierarchical("a", true);

    let mut doc = Document::new();
    doc.add(FacetField::new("a", &["1", "2", "3"])?);
    doc.add(FacetField::new("b", &["4"])?);
    writer.add_document(&config.build_with_taxonomy(Some(&taxo_writer), &doc)?)?;
    taxo_writer.close()?;

    let reader = writer.get_reader()?;
    let taxo_reader = DirectoryTaxonomyReader::open(taxo_dir.as_directory())?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    let s = &searcher;
    let c = &config;

    Assert::assert_equals(1, count(s, c, "a", &[])?);
    Assert::assert_equals(1, count(s, c, "a", &["1"])?);
    Assert::assert_equals(1, count(s, c, "a", &["1", "2"])?);
    Assert::assert_equals(1, count(s, c, "a", &["1", "2", "3"])?);
    Assert::assert_equals(1, count(s, c, "b", &[])?);
    Assert::assert_equals(1, count(s, c, "b", &["4"])?);

    taxo_reader.close()?;
    reader.close()?;
    writer.close()?;
    dir.close()?;
    taxo_dir.close()
}

/// `Arrays.equals(Query[], Query[])`.
fn queries_equal(a: &[Arc<dyn Query>], b: &[Arc<dyn Query>]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.query_eq(&**y))
}

fn test_get_drill_down_queries() -> Result<()> {
    let config = config();
    let mut q = ddq(&config);
    q.add("a", &["1"])?;
    q.add("b", &["1"])?;

    let drill_down_queries = q.get_drill_down_queries();
    let drill_down_queries_copy = q.get_drill_down_queries();

    assert!(queries_equal(&drill_down_queries, &drill_down_queries_copy));

    q.add("c", &["1"])?;
    q.add("a", &["2"])?;
    q.add("a", &["3"])?;
    let drill_down_queries_modified = q.get_drill_down_queries();
    let drill_down_queries_modified_copy = q.get_drill_down_queries();

    // the cached builtDimQueries object is now stale
    assert!(!queries_equal(
        &drill_down_queries_modified,
        &drill_down_queries_copy
    ));
    assert!(queries_equal(
        &drill_down_queries_modified,
        &drill_down_queries_modified_copy
    ));
    Ok(())
}
