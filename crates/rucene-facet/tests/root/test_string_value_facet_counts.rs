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

//! Port of `org.apache.lucene.facet.TestStringValueFacetCounts`.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::{
    Document, NumericValue, SortedDocValuesField, SortedSetDocValuesField, Store, StringField,
};
use rucene::error::Result;
use rucene::index::{IndexReader, Term};
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::{MatchAllDocsQuery, MatchNoDocsQuery};
use rucene::util::BytesRef;
use rucene_facet::drill_down_query::DrillDownQuery;
use rucene_facet::facet_result::FacetResult;
use rucene_facet::facets::Facets;
use rucene_facet::facets_collector_manager::FacetsCollectorManager;
use rucene_facet::facets_config::FacetsConfig;
use rucene_facet::label_and_value::{number_long_value, LabelAndValue};
use rucene_facet::string_doc_values_reader_state::StringDocValuesReaderState;
use rucene_facet::string_value_facet_counts::StringValueFacetCounts;
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::lucene_test_case::TEST_NIGHTLY;
use rucene_test_framework::util::LuceneTestCase;

use crate::facet_support::core_support::test;
use crate::facet_support::{assert_facet_result, facet_class, get_random_tokens};

const IAE: &str = "java.lang.IllegalArgumentException";

/// `TestStringValueFacetCounts`.
pub fn class_def() -> Arc<Class> {
    facet_class(
        "org.apache.lucene.facet.TestStringValueFacetCounts",
        vec![
            test("testBasicSingleValued", test_basic_single_valued),
            test("testEmptyMatchset", test_empty_matchset),
            test("testCountAll", test_count_all),
            test(
                "testBasicSingleValuedUsingSortedDoc",
                test_basic_single_valued_using_sorted_doc,
            ),
            test("testBasicMultiValued", test_basic_multi_valued),
            test("testSparseMultiSegmentCase", test_sparse_multi_segment_case),
            test("testMissingSegment", test_missing_segment),
            test("testStaleState", test_stale_state),
            test("testRandom", test_random),
        ],
    )
}

fn ssdv(value: &str) -> SortedSetDocValuesField {
    SortedSetDocValuesField::new("field", BytesRef::from_char_sequence(value))
}

fn counts(pairs: &[(&str, i32)]) -> HashMap<String, i32> {
    pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect()
}

fn writer(
    dir: &rucene_test_framework::util::lucene_test_case::TestDirectory,
) -> Result<RandomIndexWriter> {
    RandomIndexWriter::new_with_mock_analyzer(&*LuceneTestCase::random(), dir.as_directory())
}

fn state(searcher: &IndexSearcher) -> Result<StringDocValuesReaderState> {
    StringDocValuesReaderState::new(Arc::clone(searcher.get_index_reader()), "field")
}

fn test_basic_single_valued() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let writer = writer(&dir)?;
    for v in ["foo", "foo", "bar", "bar", "baz"] {
        let mut doc = Document::new();
        doc.add(ssdv(v));
        writer.add_document(&doc)?;
    }
    let expected_counts = counts(&[("foo", 2), ("bar", 2), ("baz", 1)]);
    let expected_total_doc_count = 5;

    let searcher = LuceneTestCase::new_searcher(writer.get_reader()?);
    writer.close()?;

    let state = state(&searcher)?;
    check_top_n_facet_result(
        &expected_counts,
        expected_total_doc_count,
        &searcher,
        &state,
        &[10, 2, 1, 0],
    )?;

    searcher.get_index_reader().close()?;
    dir.close()
}

fn assert_empty_facet_result(result: &FacetResult) {
    Assert::assert_equals(0usize, result.path.len());
    Assert::assert_equals(NumericValue::Int(0), result.value);
    Assert::assert_equals(0, result.child_count);
    Assert::assert_equals(0usize, result.label_values.len());
}

fn test_empty_matchset() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let writer = writer(&dir)?;
    let mut doc = Document::new();
    doc.add(ssdv("foo"));
    writer.add_document(&doc)?;

    let searcher = LuceneTestCase::new_searcher(writer.get_reader()?);
    writer.close()?;

    let facets_collector = searcher.search_with_manager(
        Arc::new(MatchNoDocsQuery::instance()),
        &FacetsCollectorManager::new(),
    )?;

    let state = state(&searcher)?;
    let counts = StringValueFacetCounts::with_hits(&state, Some(&facets_collector))?;

    let top = counts.get_top_children(10, "field", &[])?.unwrap();
    assert_empty_facet_result(&top);

    let all = counts.get_all_children("field", &[])?.unwrap();
    assert_empty_facet_result(&all);

    Assert::assert_equals(
        NumericValue::Int(0),
        counts.get_specific_value("field", &["foo"])?,
    );

    searcher.get_index_reader().close()?;
    dir.close()
}

// See: LUCENE-10070
fn test_count_all() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let writer = writer(&dir)?;
    for (id, v) in [("0", "foo"), ("1", "foo"), ("2", "bar")] {
        let mut doc = Document::new();
        doc.add(StringField::new("id", id.to_string(), Store::NO)?);
        doc.add(ssdv(v));
        writer.add_document(&doc)?;
    }

    writer.delete_documents(Term::from_text("id", "0"))?;

    let searcher = LuceneTestCase::new_searcher(writer.get_reader()?);
    writer.close()?;

    let state = state(&searcher)?;
    let facets = StringValueFacetCounts::new(&state)?;

    Assert::assert_equals(
        "dim=field path=[] value=2 childCount=2\n  bar (1)",
        facets
            .get_top_children(1, "field", &[])?
            .unwrap()
            .to_string()
            .trim(),
    );
    Assert::assert_equals(
        "dim=field path=[] value=2 childCount=2\n  bar (1)\n  foo (1)",
        facets
            .get_top_children(10, "field", &[])?
            .unwrap()
            .to_string()
            .trim(),
    );
    assert_facet_result(
        &facets.get_all_children("field", &[])?.unwrap(),
        "field",
        &[],
        2,
        NumericValue::Int(2),
        &[
            LabelAndValue::new("bar", NumericValue::Int(1)),
            LabelAndValue::new("foo", NumericValue::Int(1)),
        ],
    );

    searcher.get_index_reader().close()?;
    dir.close()
}

fn test_basic_single_valued_using_sorted_doc() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let writer = writer(&dir)?;
    for v in ["foo", "foo", "bar", "bar", "baz"] {
        let mut doc = Document::new();
        doc.add(SortedDocValuesField::new(
            "field",
            BytesRef::from_char_sequence(v),
        ));
        writer.add_document(&doc)?;
    }
    let expected_counts = counts(&[("foo", 2), ("bar", 2), ("baz", 1)]);
    let expected_total_doc_count = 5;

    let searcher = LuceneTestCase::new_searcher(writer.get_reader()?);
    writer.close()?;

    let state = state(&searcher)?;
    check_top_n_facet_result(
        &expected_counts,
        expected_total_doc_count,
        &searcher,
        &state,
        &[10, 2, 1, 0],
    )?;
    check_all_children_facet_result(
        &expected_counts,
        expected_total_doc_count,
        &searcher,
        &state,
    )?;

    searcher.get_index_reader().close()?;
    dir.close()
}

fn test_basic_multi_valued() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let writer = writer(&dir)?;
    for vals in [&["foo", "bar"][..], &["foo", "baz"], &["baz"]] {
        let mut doc = Document::new();
        for v in vals {
            doc.add(ssdv(v));
        }
        writer.add_document(&doc)?;
    }
    let expected_counts = counts(&[("foo", 2), ("bar", 1), ("baz", 2)]);
    let expected_total_doc_count = 3;

    let searcher = LuceneTestCase::new_searcher(writer.get_reader()?);
    writer.close()?;

    let state = state(&searcher)?;
    check_top_n_facet_result(
        &expected_counts,
        expected_total_doc_count,
        &searcher,
        &state,
        &[10, 2, 1, 0],
    )?;
    check_all_children_facet_result(
        &expected_counts,
        expected_total_doc_count,
        &searcher,
        &state,
    )?;

    searcher.get_index_reader().close()?;
    dir.close()
}

fn test_sparse_multi_segment_case() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let writer = writer(&dir)?;

    let mut expected_value_counts = HashMap::new();

    // Create two segments, each with only one doc that has a large number of SSDV field values.
    // This ensures "sparse" counting will occur in StringValueFacetCounts (i.e., small number
    // of hits relative to the field cardinality):
    let mut doc = Document::new();
    for i in 0..100 {
        doc.add(ssdv(&format!("foo_{i}")));
        expected_value_counts.insert(format!("foo_{i}"), 1);
    }
    writer.add_document(&doc)?;
    writer.commit()?;

    let mut doc = Document::new();
    for i in 0..100 {
        doc.add(ssdv(&format!("bar_{i}")));
        expected_value_counts.insert(format!("bar_{i}"), 1);
    }
    writer.add_document(&doc)?;

    let expected_total_doc_count = 2;

    let searcher = LuceneTestCase::new_searcher(writer.get_reader()?);
    writer.close()?;

    let state = state(&searcher)?;
    check_top_n_facet_result(
        &expected_value_counts,
        expected_total_doc_count,
        &searcher,
        &state,
        &[10, 2, 1, 0],
    )?;
    check_all_children_facet_result(
        &expected_value_counts,
        expected_total_doc_count,
        &searcher,
        &state,
    )?;

    searcher.get_index_reader().close()?;
    dir.close()
}

fn test_missing_segment() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let writer = writer(&dir)?;

    let mut doc = Document::new();
    doc.add(ssdv("foo"));
    doc.add(ssdv("bar"));
    writer.add_document(&doc)?;
    writer.commit()?;

    // segment with no values
    writer.add_document(&Document::new())?;
    writer.commit()?;

    let mut doc = Document::new();
    doc.add(ssdv("baz"));
    writer.add_document(&doc)?;
    writer.commit()?;

    let expected_counts = counts(&[("foo", 1), ("bar", 1), ("baz", 1)]);
    let expected_total_doc_count = 2;

    let searcher = LuceneTestCase::new_searcher(writer.get_reader()?);
    writer.close()?;

    let state = state(&searcher)?;
    check_top_n_facet_result(
        &expected_counts,
        expected_total_doc_count,
        &searcher,
        &state,
        &[10, 2, 1, 0],
    )?;
    check_all_children_facet_result(
        &expected_counts,
        expected_total_doc_count,
        &searcher,
        &state,
    )?;

    searcher.get_index_reader().close()?;
    dir.close()
}

fn test_stale_state() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let writer = writer(&dir)?;

    let mut doc = Document::new();
    doc.add(ssdv("foo"));
    writer.add_document(&doc)?;

    let reader = writer.get_reader()?;
    let state =
        StringDocValuesReaderState::new(Arc::clone(&reader) as Arc<dyn IndexReader>, "field")?;

    let mut doc = Document::new();
    doc.add(ssdv("bar"));
    writer.add_document(&doc)?;

    let searcher = LuceneTestCase::new_searcher(writer.get_reader()?);
    writer.close()?;

    let c = searcher.search_with_manager(
        Arc::new(MatchAllDocsQuery::instance()),
        &FacetsCollectorManager::new(),
    )?;

    // using a stale state
    expect_throws("java.lang.IllegalStateException", || {
        StringValueFacetCounts::with_hits(&state, Some(&c))
    });

    reader.close()?;
    searcher.get_index_reader().close()?;
    dir.close()
}

fn test_random() -> Result<()> {
    let random = LuceneTestCase::random();
    let full_iterations = if *TEST_NIGHTLY { 20 } else { 3 };
    for _ in 0..full_iterations {
        let dir = LuceneTestCase::new_directory();
        let writer = writer(&dir)?;

        // Build up test data
        let tokens = get_random_tokens(50); // 50 random values to pick from
        let num_docs = LuceneTestCase::at_least(1000);
        let mut expected_total_doc_count = 0;
        let mut expected: HashMap<String, i32> = HashMap::new();
        for _ in 0..num_docs {
            let mut doc = Document::new();
            // Sometimes we restrict docs to be single-valued, but most of the time they can have up to
            // 5:
            let max_values_per_doc = if random.next_int_bound(10) < 8 { 5 } else { 1 };
            let val_count = random.next_int_bound(max_values_per_doc);
            let mut doc_vals = HashSet::new();
            for _ in 0..val_count {
                let token_idx = random.next_int_bound(tokens.len() as i32);
                let val = &tokens[token_idx as usize];
                // values should only be counted once per document
                if !doc_vals.contains(val) {
                    *expected.entry(val.clone()).or_insert(0) += 1;
                }
                doc_vals.insert(val.clone());
                doc.add(ssdv(val));
            }
            // only docs with at least one value in the field should be counted in the total
            if !doc_vals.is_empty() {
                expected_total_doc_count += 1;
            }
            writer.add_document(&doc)?;
            if random.next_int_bound(10) == 0 {
                writer.commit()?; // sometimes commit
            }
        }

        let searcher = LuceneTestCase::new_searcher(writer.get_reader()?);
        writer.close()?;

        // run iterations with random values of topN
        let iterations = if *TEST_NIGHTLY { 10_000 } else { 50 };
        let top_ns: Vec<i32> = (0..iterations)
            .map(|_| LuceneTestCase::at_least(1))
            .collect();

        let state = state(&searcher)?;
        check_top_n_facet_result(
            &expected,
            expected_total_doc_count,
            &searcher,
            &state,
            &top_ns,
        )?;
        check_all_children_facet_result(&expected, expected_total_doc_count, &searcher, &state)?;

        searcher.get_index_reader().close()?;
        dir.close()?;
    }
    Ok(())
}

/// `String.compareTo`: UTF-16 code unit order.
fn java_compare(a: &str, b: &str) -> std::cmp::Ordering {
    a.encode_utf16().cmp(b.encode_utf16())
}

/// `facets = random().nextBoolean() ? new StringValueFacetCounts(state, c) :
/// new StringValueFacetCounts(state)`.
fn new_facets(
    state: &StringDocValuesReaderState,
    c: &rucene_facet::facets_collector::FacetsCollector,
) -> Result<StringValueFacetCounts> {
    if LuceneTestCase::random().next_boolean() {
        StringValueFacetCounts::with_hits(state, Some(c))
    } else {
        StringValueFacetCounts::new(state)
    }
}

fn check_top_n_facet_result(
    expected_counts: &HashMap<String, i32>,
    expected_total_docs_with_value: i32,
    searcher: &IndexSearcher,
    state: &StringDocValuesReaderState,
    top_ns: &[i32],
) -> Result<()> {
    let random = LuceneTestCase::random();
    let c = searcher.search_with_manager(
        Arc::new(MatchAllDocsQuery::instance()),
        &FacetsCollectorManager::new(),
    )?;

    for &top_n in top_ns {
        // should get the same result whether-or-not we provide a FacetsCollector since it's doing
        // a MatchAllDocsQuery:
        let facets = new_facets(state, &c)?;

        // sort expected counts by count, value
        let mut expected_counts_sorted: Vec<(&String, &i32)> = expected_counts.iter().collect();
        expected_counts_sorted.sort_by(|a, b| {
            b.1.cmp(a.1) // high-to-low
                .then_with(|| java_compare(a.0, b.0)) // low-to-high
        });

        // number of labels we expect is the number with a non-zero count
        let expected_label_count =
            expected_counts_sorted.iter().filter(|e| *e.1 > 0).count() as i32;

        // topN == 0 is intentionally unsupported
        if top_n == 0 {
            expect_throws(IAE, || facets.get_top_children(top_n, "field", &[]));
            return Ok(());
        }

        let facet_result = facets.get_top_children(top_n, "field", &[])?.unwrap();

        Assert::assert_equals("field", facet_result.dim.as_str());
        Assert::assert_equals(0usize, facet_result.path.len());
        Assert::assert_equals(
            NumericValue::Int(expected_total_docs_with_value),
            facet_result.value,
        );
        Assert::assert_equals(expected_label_count, facet_result.child_count);

        // getAllDims should return a singleton list with the same results as getTopChildren
        let all_dims = facets.get_all_dims(top_n)?;
        Assert::assert_equals(1usize, all_dims.len());
        Assert::assert_equals(&facet_result, &all_dims[0]);

        // test default implementation of getTopDims
        let top_n_dims_result = facets.get_top_dims(2, top_n)?;
        Assert::assert_equals(1usize, top_n_dims_result.len());
        Assert::assert_equals(&facet_result, &top_n_dims_result[0]);

        // test getAllDims(0)
        expect_throws(IAE, || facets.get_all_dims(0));

        // This is a little strange, but we request all labels at this point so that when we
        // secondarily sort by label value in order to compare to the expected results, we have
        // all the values. See LUCENE-9991:
        let max_top_n = expected_counts_sorted.len();
        let mut facet_result = facets
            .get_top_children(max_top_n as i32, "field", &[])?
            .unwrap();

        // also sort expected labels by count, value (these will be sorted by count, ord -- but since
        // we have no insight into the ordinals assigned to the values, we resort)
        facet_result.label_values.sort_by(|a, b| {
            number_long_value(&b.value)
                .cmp(&number_long_value(&a.value)) // high-to-low
                .then_with(|| java_compare(&a.label, &b.label)) // low-to-high
        });

        for i in 0..(top_n as usize).min(max_top_n) {
            let expected_key = expected_counts_sorted[i].0;
            let expected_value = *expected_counts_sorted[i].1;
            Assert::assert_equals(
                expected_key.as_str(),
                facet_result.label_values[i].label.as_str(),
            );
            Assert::assert_equals(
                NumericValue::Int(expected_value),
                facet_result.label_values[i].value,
            );
            // make sure getSpecificValue reports the same count
            Assert::assert_equals(
                NumericValue::Int(expected_value),
                facets.get_specific_value("field", &[expected_key])?,
            );
        }

        // execute a "drill down" query on one of the values at random and make sure the total hits
        // match the expected count provided by faceting
        if !expected_counts_sorted.is_empty() {
            let mut q = DrillDownQuery::new(Arc::new(FacetsConfig::new()));
            let random_test_val_idx =
                random.next_int_bound((expected_counts_sorted.len() as i32).min(top_n)) as usize;
            q.add("field", &[expected_counts_sorted[random_test_val_idx].0])?;
            searcher.search(Arc::new(q), 1)?;
            Assert::assert_equals(
                NumericValue::Int(*expected_counts_sorted[random_test_val_idx].1),
                facet_result.label_values[random_test_val_idx].value,
            );
        }
    }
    Ok(())
}

fn check_all_children_facet_result(
    expected_value_counts: &HashMap<String, i32>,
    expected_total_docs_with_value: i32,
    searcher: &IndexSearcher,
    state: &StringDocValuesReaderState,
) -> Result<()> {
    let c = searcher.search_with_manager(
        Arc::new(MatchAllDocsQuery::instance()),
        &FacetsCollectorManager::new(),
    )?;
    // should get the same result whether-or-not we provide a FacetsCollector since it's doing
    // a MatchAllDocsQuery:
    let facets = new_facets(state, &c)?;

    let mut expected_counts_sorted_by_value: Vec<(&String, &i32)> =
        expected_value_counts.iter().collect();
    // sort expected counts by value, count
    expected_counts_sorted_by_value
        .sort_by(|a, b| java_compare(a.0, b.0).then_with(|| a.1.cmp(b.1)));

    let mut facet_result = facets.get_all_children("field", &[])?.unwrap();
    Assert::assert_equals(
        NumericValue::Int(expected_total_docs_with_value),
        facet_result.value,
    );

    // since we have no insight into the ordinals assigned to the values, we sort labels by value
    // and count in ascending order in order to compare with expected results
    facet_result.label_values.sort_by(|a, b| {
        java_compare(&a.label, &b.label)
            .then_with(|| number_long_value(&a.value).cmp(&number_long_value(&b.value)))
    });

    for (i, (expected_key, expected_value)) in expected_counts_sorted_by_value.iter().enumerate() {
        Assert::assert_equals(
            expected_key.as_str(),
            facet_result.label_values[i].label.as_str(),
        );
        Assert::assert_equals(
            NumericValue::Int(**expected_value),
            facet_result.label_values[i].value,
        );
        // make sure getSpecificValue reports the same count
        Assert::assert_equals(
            NumericValue::Int(**expected_value),
            facets.get_specific_value("field", &[expected_key])?,
        );
    }
    Ok(())
}
