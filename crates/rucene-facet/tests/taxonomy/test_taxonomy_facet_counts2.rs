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

//! Port of `org.apache.lucene.facet.taxonomy.TestTaxonomyFacetCounts2`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::jdk::util::shuffle;
use randomizedtesting::junit::{AfterClass, Assert, BeforeClass};
use rucene::document::{Document, NumericValue, Store, StringField};
use rucene::error::Result;
use rucene::index::index_writer::IndexWriter;
use rucene::index::merge_policy::NoMergePolicy;
use rucene::index::{directory_reader, Term};
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::query::Query;
use rucene::search::term_query::TermQuery;
use rucene::search::MatchAllDocsQuery;
use rucene_facet::facet_field::FacetField;
use rucene_facet::facets::Facets;
use rucene_facet::facets_collector_manager::FacetsCollectorManager;
use rucene_facet::facets_config::FacetsConfig;
use rucene_facet::label_and_value::number_int_value;
use rucene_facet::taxonomy::directory::directory_taxonomy_reader::DirectoryTaxonomyReader;
use rucene_facet::taxonomy::directory::directory_taxonomy_writer::DirectoryTaxonomyWriter;
use rucene_facet::taxonomy::taxonomy_reader::TaxonomyReader;
use rucene_facet::taxonomy::taxonomy_writer::TaxonomyWriter;
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::LuceneTestCase;

use crate::facet_support::core_support::test;
use crate::facet_support::search_support::static_hook;
use crate::facet_support::{facet_class, get_taxonomy_facet_counts};

/// `TestTaxonomyFacetCounts2`.
pub fn class_def() -> Arc<Class> {
    facet_class(
        "org.apache.lucene.facet.taxonomy.TestTaxonomyFacetCounts2",
        vec![
            static_hook(
                "afterClassCountingFacetsAggregatorTest",
                AfterClass,
                after_class_counting_facets_aggregator_test,
            ),
            static_hook(
                "beforeClassCountingFacetsAggregatorTest",
                BeforeClass,
                before_class_counting_facets_aggregator_test,
            ),
            test("testDifferentNumResults", test_different_num_results),
            test("testAllCounts", test_all_counts),
            test("testBigNumResults", test_big_num_results),
            test("testNoParents", test_no_parents),
        ],
    )
}

const A_FIELD: &str = "f";
const A_TEXT: &str = "a";
const CP_A: &str = "A";
const CP_B: &str = "B";
const CP_C: &str = "C"; // indexed w/ NO_PARENTS
const CP_D: &str = "D"; // indexed w/ NO_PARENTS
const NUM_CHILDREN_CP_A: i32 = 5;
const NUM_CHILDREN_CP_B: i32 = 3;
const NUM_CHILDREN_CP_C: i32 = 5;
const NUM_CHILDREN_CP_D: i32 = 5;

/// A `FacetField` of the static `CATEGORIES_*` arrays, as its `dim` and
/// `path` (each document gets a fresh field with the same label).
#[derive(Clone)]
struct Category {
    dim: &'static str,
    path: Vec<String>,
}

impl Category {
    fn field(&self) -> Result<FacetField> {
        let path: Vec<&str> = self.path.iter().map(String::as_str).collect();
        FacetField::new(self.dim, &path)
    }
}

fn categories(dim: &'static str, n: i32, multi_level: bool) -> Vec<Category> {
    (0..n)
        .map(|i| {
            let val = i.to_string();
            let path = if multi_level {
                vec![val.clone(), format!("{val}{val}")] // e.g. D/1/11, D/2/22...
            } else {
                vec![val]
            };
            Category { dim, path }
        })
        .collect()
}

fn categories_a() -> Vec<Category> {
    categories(CP_A, NUM_CHILDREN_CP_A, false)
}
fn categories_b() -> Vec<Category> {
    categories(CP_B, NUM_CHILDREN_CP_B, false)
}
// NO_PARENTS categories
fn categories_c() -> Vec<Category> {
    categories(CP_C, NUM_CHILDREN_CP_C, false)
}
// Multi-level categories
fn categories_d() -> Vec<Category> {
    categories(CP_D, NUM_CHILDREN_CP_D, true)
}

/// The static fields.
struct Statics {
    index_dir: TestDirectory,
    taxo_dir: TestDirectory,
    all_expected_counts: HashMap<String, i32>,
    term_expected_counts: HashMap<String, i32>,
}

static STATICS: Mutex<Option<Statics>> = Mutex::new(None);

fn statics() -> MutexGuard<'static, Option<Statics>> {
    STATICS.lock().unwrap_or_else(|e| e.into_inner())
}

fn after_class_counting_facets_aggregator_test() -> Result<()> {
    if let Some(s) = statics().take() {
        s.index_dir.close()?;
        s.taxo_dir.close()?;
    }
    Ok(())
}

fn random_categories(random: &dyn randomizedtesting::jdk::random::Random) -> Vec<Category> {
    // add random categories from the two dimensions, ensuring that the same
    // category is not added twice.
    let num_facets_a = (random.next_int_bound(3) + 1) as usize; // 1-3
    let num_facets_b = (random.next_int_bound(2) + 1) as usize; // 1-2
    let mut categories_a = categories_a();
    let mut categories_b = categories_b();
    shuffle(&mut categories_a, random);
    shuffle(&mut categories_b, random);

    let mut categories = Vec::new();
    categories.extend_from_slice(&categories_a[..num_facets_a]);
    categories.extend_from_slice(&categories_b[..num_facets_b]);

    // add the NO_PARENT categories
    let r = LuceneTestCase::random();
    categories.push(categories_c()[r.next_int_bound(NUM_CHILDREN_CP_C) as usize].clone());
    categories.push(categories_d()[r.next_int_bound(NUM_CHILDREN_CP_D) as usize].clone());
    categories
}

fn add_field(doc: &mut Document) -> Result<()> {
    doc.add(StringField::new(A_FIELD, A_TEXT.to_string(), Store::NO)?);
    Ok(())
}

fn incr(counts: &mut HashMap<String, i32>, key: &str) {
    let v = *counts
        .get(key)
        .unwrap_or_else(|| panic!("java.lang.NullPointerException: no count for {key}"));
    counts.insert(key.to_string(), v + 1);
}

fn add_facets(
    doc: &mut Document,
    update_term_expected_counts: bool,
    all: &mut HashMap<String, i32>,
    term: &mut HashMap<String, i32>,
) -> Result<()> {
    let doc_categories = random_categories(&*LuceneTestCase::random());
    for ff in doc_categories {
        doc.add(ff.field()?);
        let cp = format!("{}/{}", ff.dim, ff.path[0]);
        incr(all, &cp);
        if update_term_expected_counts {
            incr(term, &cp);
        }
    }
    // add 1 to each NO_PARENTS dimension
    incr(all, CP_B);
    incr(all, CP_C);
    incr(all, CP_D);
    if update_term_expected_counts {
        incr(term, CP_B);
        incr(term, CP_C);
        incr(term, CP_D);
    }
    Ok(())
}

fn get_config() -> Arc<FacetsConfig> {
    let config = FacetsConfig::new();
    config.set_multi_valued("A", true);
    config.set_multi_valued("B", true);
    config.set_require_dim_count("B", true);
    config.set_hierarchical("D", true);
    Arc::new(config)
}

fn index_docs_no_facets(index_writer: &IndexWriter) -> Result<()> {
    let num_docs = LuceneTestCase::at_least(2);
    for _ in 0..num_docs {
        let mut doc = Document::new();
        add_field(&mut doc)?;
        index_writer.add_document(&doc)?;
    }
    index_writer.commit()?; // flush a segment
    Ok(())
}

/// `indexDocsWithFacetsNoTerms` (`mode` 0), `indexDocsWithFacetsAndTerms`
/// (1) and `indexDocsWithFacetsAndSomeTerms` (2).
fn index_docs_with_facets(
    index_writer: &IndexWriter,
    taxo_writer: &dyn TaxonomyWriter,
    mode: u8,
    all: &mut HashMap<String, i32>,
    term: &mut HashMap<String, i32>,
) -> Result<()> {
    let random = LuceneTestCase::random();
    let num_docs = LuceneTestCase::at_least_with(&*random, 2);
    let config = get_config();
    for _ in 0..num_docs {
        let mut doc = Document::new();
        match mode {
            0 => add_facets(&mut doc, false, all, term)?,
            1 => {
                add_facets(&mut doc, true, all, term)?;
                add_field(&mut doc)?;
            }
            _ => {
                let has_content = random.next_boolean();
                if has_content {
                    add_field(&mut doc)?;
                }
                add_facets(&mut doc, has_content, all, term)?;
            }
        }
        index_writer.add_document(&config.build_with_taxonomy(Some(taxo_writer), &doc)?)?;
    }
    index_writer.commit()?; // flush a segment
    Ok(())
}

// initialize expectedCounts w/ 0 for all categories
fn new_counts() -> HashMap<String, i32> {
    let mut counts = HashMap::new();
    for cp in [CP_A, CP_B, CP_C, CP_D] {
        counts.insert(cp.to_string(), 0);
    }
    for ff in categories_a()
        .into_iter()
        .chain(categories_b())
        .chain(categories_c())
        .chain(categories_d())
    {
        counts.insert(format!("{}/{}", ff.dim, ff.path[0]), 0);
    }
    counts
}

fn before_class_counting_facets_aggregator_test() -> Result<()> {
    let index_dir = LuceneTestCase::new_directory();
    let taxo_dir = LuceneTestCase::new_directory();

    // create an index which has:
    // 1. Segment with no categories, but matching results
    // 2. Segment w/ categories, but no results
    // 3. Segment w/ categories and results
    // 4. Segment w/ categories, but only some results

    let conf = LuceneTestCase::new_index_writer_config();
    conf.set_merge_policy(Some(NoMergePolicy::instance()))?; // prevent merges, so we can control the index segments
    let index_writer = IndexWriter::new(index_dir.as_directory(), conf)?;
    let taxo_writer = DirectoryTaxonomyWriter::new(taxo_dir.as_directory())?;

    let mut all = new_counts();
    let mut term = new_counts();

    // segment w/ no categories
    index_docs_no_facets(&index_writer)?;

    // segment w/ categories, no content
    index_docs_with_facets(&index_writer, &taxo_writer, 0, &mut all, &mut term)?;

    // segment w/ categories and content
    index_docs_with_facets(&index_writer, &taxo_writer, 1, &mut all, &mut term)?;

    // segment w/ categories and some content
    index_docs_with_facets(&index_writer, &taxo_writer, 2, &mut all, &mut term)?;

    index_writer.close()?;
    taxo_writer.close()?;
    *statics() = Some(Statics {
        index_dir,
        taxo_dir,
        all_expected_counts: all,
        term_expected_counts: term,
    });
    Ok(())
}

/// Opens the index and taxonomy, runs `q` with a `FacetsCollectorManager`
/// and builds the taxonomy facet counts; `check` gets the facets and the
/// expected count maps.
fn with_facets(
    q: Arc<dyn Query>,
    check: impl FnOnce(&dyn Facets, &HashMap<String, i32>, &HashMap<String, i32>) -> Result<()>,
) -> Result<()> {
    let (index_dir, taxo_dir, all, term) = {
        let s = statics();
        let s = s.as_ref().expect("beforeClass");
        (
            s.index_dir.as_directory(),
            s.taxo_dir.as_directory(),
            s.all_expected_counts.clone(),
            s.term_expected_counts.clone(),
        )
    };
    let index_reader = directory_reader::open(index_dir)?;
    let taxo_reader: Arc<dyn TaxonomyReader> = Arc::new(DirectoryTaxonomyReader::open(taxo_dir)?);
    let searcher: IndexSearcher = LuceneTestCase::new_searcher(Arc::clone(&index_reader));
    let sfc = searcher.search_with_manager(q, &FacetsCollectorManager::new())?;
    let facets = get_taxonomy_facet_counts(&taxo_reader, &get_config(), &sfc)?;
    check(&*facets, &all, &term)?;
    index_reader.close()?;
    taxo_reader.close()
}

fn int(map: &HashMap<String, i32>, key: &str) -> NumericValue {
    NumericValue::Int(
        *map.get(key)
            .unwrap_or_else(|| panic!("java.lang.NullPointerException: no count for {key}")),
    )
}

fn test_different_num_results() -> Result<()> {
    // test the collector w/ FacetRequests and different numResults
    let q: Arc<dyn Query> = Arc::new(TermQuery::new(Term::from_text(A_FIELD, A_TEXT)));
    with_facets(q, |facets, _all, term| {
        let result = facets
            .get_top_children(NUM_CHILDREN_CP_A, CP_A, &[])?
            .unwrap();
        Assert::assert_equals(-1, number_int_value(&result.value));
        for label_value in &result.label_values {
            Assert::assert_equals(
                int(term, &format!("{CP_A}/{}", label_value.label)),
                label_value.value,
            );
        }
        let result = facets
            .get_top_children(NUM_CHILDREN_CP_B, CP_B, &[])?
            .unwrap();
        Assert::assert_equals(int(term, CP_B), result.value);
        for label_value in &result.label_values {
            Assert::assert_equals(
                int(term, &format!("{CP_B}/{}", label_value.label)),
                label_value.value,
            );
        }
        Ok(())
    })
}

fn check_sorted(
    facets: &dyn Facets,
    all: &HashMap<String, i32>,
    dim: &str,
    top_n: i32,
) -> Result<()> {
    let result = facets.get_top_children(top_n, dim, &[])?.unwrap();
    if dim == CP_A {
        Assert::assert_equals(-1, number_int_value(&result.value));
    } else {
        Assert::assert_equals(int(all, dim), result.value);
    }
    let mut prev_value = i32::MAX;
    for label_value in &result.label_values {
        Assert::assert_equals(
            int(all, &format!("{dim}/{}", label_value.label)),
            label_value.value,
        );
        Assert::assert_true_msg(
            &format!(
                "wrong sort order of sub results: labelValue.value={} prevValue={prev_value}",
                label_value.value
            ),
            number_int_value(&label_value.value) <= prev_value,
        );
        prev_value = number_int_value(&label_value.value);
    }
    Ok(())
}

fn test_all_counts() -> Result<()> {
    with_facets(
        Arc::new(MatchAllDocsQuery::instance()),
        |facets, all, _term| {
            check_sorted(facets, all, CP_A, NUM_CHILDREN_CP_A)?;
            check_sorted(facets, all, CP_B, NUM_CHILDREN_CP_B)
        },
    )
}

fn test_big_num_results() -> Result<()> {
    with_facets(
        Arc::new(MatchAllDocsQuery::instance()),
        |facets, all, _term| {
            let result = facets.get_top_children(i32::MAX, CP_A, &[])?.unwrap();
            Assert::assert_equals(-1, number_int_value(&result.value));
            for label_value in &result.label_values {
                Assert::assert_equals(
                    int(all, &format!("{CP_A}/{}", label_value.label)),
                    label_value.value,
                );
            }
            let result = facets.get_top_children(i32::MAX, CP_B, &[])?.unwrap();
            Assert::assert_equals(int(all, CP_B), result.value);
            for label_value in &result.label_values {
                Assert::assert_equals(
                    int(all, &format!("{CP_B}/{}", label_value.label)),
                    label_value.value,
                );
            }
            Ok(())
        },
    )
}

fn test_no_parents() -> Result<()> {
    with_facets(
        Arc::new(MatchAllDocsQuery::instance()),
        |facets, all, _term| {
            let result = facets
                .get_top_children(NUM_CHILDREN_CP_C, CP_C, &[])?
                .unwrap();
            Assert::assert_equals(int(all, CP_C), result.value);
            for label_value in &result.label_values {
                Assert::assert_equals(
                    int(all, &format!("{CP_C}/{}", label_value.label)),
                    label_value.value,
                );
            }
            let result = facets
                .get_top_children(NUM_CHILDREN_CP_D, CP_D, &[])?
                .unwrap();
            Assert::assert_equals(int(all, CP_C), result.value);
            for label_value in &result.label_values {
                Assert::assert_equals(
                    int(all, &format!("{CP_D}/{}", label_value.label)),
                    label_value.value,
                );
            }
            Ok(())
        },
    )
}
