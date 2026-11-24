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

//! Port of `org.apache.lucene.facet.TestLongValueFacetCounts`.

use std::collections::HashMap;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::{
    Document, IntPoint, NumericDocValuesField, NumericValue, SortedNumericDocValuesField, Store,
    StringField,
};
use rucene::error::Result;
use rucene::index::{IndexReader, Term};
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::long_values_source as lvs;
use rucene::search::query::Query;
use rucene::search::MatchAllDocsQuery;
use rucene_facet::facet_result::FacetResult;
use rucene_facet::facets::Facets;
use rucene_facet::facets_collector::FacetsCollector;
use rucene_facet::facets_collector_manager::FacetsCollectorManager;
use rucene_facet::label_and_value::{number_int_value, number_long_value, LabelAndValue};
use rucene_facet::long_value_facet_counts::LongValueFacetCounts;
use rucene_facet::multi_long_values_source::MultiLongValuesSource;
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::lucene_test_case::VERBOSE;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::facet_support::core_support::test;
use crate::facet_support::{assert_facet_result, facet_class};

/// `TestLongValueFacetCounts`.
pub fn class_def() -> Arc<Class> {
    facet_class(
        "org.apache.lucene.facet.TestLongValueFacetCounts",
        vec![
            test("testBasic", test_basic),
            test("testCountAll", test_count_all),
            test("testOnlyBigLongs", test_only_big_longs),
            test("testGetAllDims", test_get_all_dims),
            test("testRandomSingleValued", test_random_single_valued),
            test("testRandomMultiValued", test_random_multi_valued),
            test("testDuplicateLongValues", test_duplicate_long_values),
        ],
    )
}

fn lv(label: &str, value: i32) -> LabelAndValue {
    LabelAndValue::new(label, NumericValue::Int(value))
}

fn match_all(s: &IndexSearcher) -> Result<FacetsCollector> {
    s.search_with_manager(
        Arc::new(MatchAllDocsQuery::instance()),
        &FacetsCollectorManager::new(),
    )
}

fn as_index_reader(
    r: &Arc<dyn rucene::index::directory_reader::DirectoryReader>,
) -> Arc<dyn IndexReader> {
    Arc::clone(r) as Arc<dyn IndexReader>
}

fn test_basic() -> Result<()> {
    let d = LuceneTestCase::new_directory();
    let w =
        RandomIndexWriter::new_with_mock_analyzer(&*LuceneTestCase::random(), d.as_directory())?;
    for l in 0..100i64 {
        let mut doc = Document::new();
        doc.add(NumericDocValuesField::new("field", l % 5));
        w.add_document(&doc)?;
    }

    // Also add Long.MAX_VALUE
    let mut doc = Document::new();
    doc.add(NumericDocValuesField::new("field", i64::MAX));
    w.add_document(&doc)?;

    let r = w.get_reader()?;
    w.close()?;

    let s = LuceneTestCase::new_searcher(Arc::clone(&r));
    let fc = match_all(&s)?;

    let facets = LongValueFacetCounts::new("field", &fc)?;

    let result = facets.get_all_children_sort_by_value();
    Assert::assert_equals(
        "dim=field path=[] value=101 childCount=6\n  0 (20)\n  1 (20)\n  2 (20)\n  3 (20)\n  4 (20)\n  9223372036854775807 (1)\n",
        result.to_string().as_str(),
    );

    let top_children_result = facets.get_top_children(2, "field", &[])?.unwrap();
    Assert::assert_equals(
        "dim=field path=[] value=101 childCount=6\n  0 (20)\n  1 (20)\n",
        top_children_result.to_string().as_str(),
    );

    assert_facet_result(
        &facets.get_all_children("field", &[])?.unwrap(),
        "field",
        &[],
        6,
        NumericValue::Int(101),
        &[
            lv("0", 20),
            lv("1", 20),
            lv("2", 20),
            lv("3", 20),
            lv("4", 20),
            lv("9223372036854775807", 1),
        ],
    );

    r.close()?;
    d.close()
}

// See: LUCENE-10070
fn test_count_all() -> Result<()> {
    let d = LuceneTestCase::new_directory();
    let w =
        RandomIndexWriter::new_with_mock_analyzer(&*LuceneTestCase::random(), d.as_directory())?;
    for i in 0..10i64 {
        let mut doc = Document::new();
        doc.add(StringField::new("id", i.to_string(), Store::NO)?);
        doc.add(NumericDocValuesField::new("field", i % 2));
        w.add_document(&doc)?;
    }

    w.delete_documents(Term::from_text("id", "0"))?;

    let r = w.get_reader()?;
    w.close()?;

    let facets = LongValueFacetCounts::count_all_docs("field", &as_index_reader(&r))?;

    let result = facets.get_all_children_sort_by_value();
    Assert::assert_equals(
        "dim=field path=[] value=9 childCount=2\n  0 (4)\n  1 (5)\n",
        result.to_string().as_str(),
    );
    let result = facets.get_top_children(10, "field", &[])?.unwrap();
    Assert::assert_equals(
        "dim=field path=[] value=9 childCount=2\n  1 (5)\n  0 (4)\n",
        result.to_string().as_str(),
    );

    assert_facet_result(
        &facets.get_all_children("field", &[])?.unwrap(),
        "field",
        &[],
        2,
        NumericValue::Int(9),
        &[lv("0", 4), lv("1", 5)],
    );

    r.close()?;
    d.close()
}

/// `String.compareTo`: UTF-16 code unit order.
fn java_compare(a: &str, b: &str) -> std::cmp::Ordering {
    a.encode_utf16().cmp(b.encode_utf16())
}

/// `Comparator.comparing(a -> a.label).thenComparingLong(a ->
/// a.value.longValue())`.
fn sort_by_label_then_value(label_values: &mut [LabelAndValue]) {
    label_values.sort_by(|a, b| {
        java_compare(&a.label, &b.label)
            .then_with(|| number_long_value(&a.value).cmp(&number_long_value(&b.value)))
    });
}

fn test_only_big_longs() -> Result<()> {
    let d = LuceneTestCase::new_directory();
    let w =
        RandomIndexWriter::new_with_mock_analyzer(&*LuceneTestCase::random(), d.as_directory())?;
    for l in 0..3i64 {
        let mut doc = Document::new();
        doc.add(NumericDocValuesField::new("field", i64::MAX - l));
        w.add_document(&doc)?;
    }

    let r = w.get_reader()?;
    w.close()?;

    let s = LuceneTestCase::new_searcher(Arc::clone(&r));
    let fc = match_all(&s)?;

    let facets = LongValueFacetCounts::new("field", &fc)?;

    let mut result = facets.get_all_children_sort_by_value();

    assert_facet_result(
        &facets.get_all_children("field", &[])?.unwrap(),
        "field",
        &[],
        3,
        NumericValue::Int(3),
        &[
            lv("9223372036854775805", 1),
            lv("9223372036854775806", 1),
            lv("9223372036854775807", 1),
        ],
    );

    // since we have no insight into the value order in the hashMap, we sort labels by value and
    // count in
    // ascending order in order to compare with expected results
    sort_by_label_then_value(&mut result.label_values);

    Assert::assert_equals(
        "dim=field path=[] value=3 childCount=3\n  9223372036854775805 (1)\n  9223372036854775806 (1)\n  9223372036854775807 (1)\n",
        result.to_string().as_str(),
    );
    r.close()?;
    d.close()
}

fn test_get_all_dims() -> Result<()> {
    let d = LuceneTestCase::new_directory();
    let w =
        RandomIndexWriter::new_with_mock_analyzer(&*LuceneTestCase::random(), d.as_directory())?;
    for l in 0..100i64 {
        let mut doc = Document::new();
        doc.add(NumericDocValuesField::new("field", l % 5));
        w.add_document(&doc)?;
    }

    // Also add Long.MAX_VALUE
    let mut doc = Document::new();
    doc.add(NumericDocValuesField::new("field", i64::MAX));
    w.add_document(&doc)?;

    let r = w.get_reader()?;
    w.close()?;

    let s = LuceneTestCase::new_searcher(Arc::clone(&r));
    let fc = match_all(&s)?;

    let facets: Box<dyn Facets> = Box::new(LongValueFacetCounts::new("field", &fc)?);

    let result = facets.get_all_dims(10)?;
    Assert::assert_equals(1usize, result.len());
    Assert::assert_equals(
        "dim=field path=[] value=101 childCount=6\n  0 (20)\n  1 (20)\n  2 (20)\n  3 (20)\n  4 (20)\n  9223372036854775807 (1)\n",
        result[0].to_string().as_str(),
    );

    // test default implementation of getTopDims
    let get_top_dim_result = facets.get_top_dims(1, 1)?;
    Assert::assert_equals(1usize, get_top_dim_result.len());
    Assert::assert_equals(
        "dim=field path=[] value=101 childCount=6\n  0 (20)\n",
        get_top_dim_result[0].to_string().as_str(),
    );

    // test getTopDims(10, 10) and expect same results from getAllDims(10)
    let all_dims_results = facets.get_top_dims(10, 10)?;
    Assert::assert_true_msg(
        &format!("expected:<{result:?}> but was:<{all_dims_results:?}>"),
        result == all_dims_results,
    );

    // test getTopDims(0, 1)
    let top_dims_results2 = facets.get_top_dims(0, 1)?;
    Assert::assert_equals(0usize, top_dims_results2.len());

    // test getAllDims(0)
    expect_throws("java.lang.IllegalArgumentException", || {
        facets.get_all_dims(0)
    });

    r.close()?;
    d.close()
}

/// `random().nextInt(1, docCount)` or `docCount`.
fn random_top_n(doc_count: i32) -> i32 {
    let random = LuceneTestCase::random();
    if random.next_boolean() {
        doc_count
    } else {
        random.next_int_range(1, doc_count)
    }
}

/// Sort by count high-to-low, tie break by value.
fn sort_by_count(expected_counts: &mut [(i64, i32)]) {
    expected_counts.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
}

/// One of the `new LongValueFacetCounts("field", <single-valued source
/// choice>, fc)` constructions of `testRandomSingleValued`.
// Each `random.next_boolean()` draws: the repeated conditions are Java's
// `if (random().nextBoolean()) ... else if (random().nextBoolean())`.
#[allow(clippy::ifs_same_cond)]
fn single_valued_source_counts(fc: &FacetsCollector) -> Result<LongValueFacetCounts> {
    let random = LuceneTestCase::random();
    if random.next_boolean() {
        LongValueFacetCounts::with_values_source("field", Some(lvs::from_long_field("field")), fc)
    } else if random.next_boolean() {
        LongValueFacetCounts::with_multi_values_source(
            "field",
            Some(<dyn MultiLongValuesSource>::from_long_field("field")),
            fc,
        )
    } else {
        LongValueFacetCounts::with_multi_values_source(
            "field",
            Some(<dyn MultiLongValuesSource>::from_single_valued(
                lvs::from_long_field("field"),
            )),
            fc,
        )
    }
}

fn id_range_query(min_id: i32, max_id: i32) -> Result<Arc<dyn Query>> {
    IntPoint::new_range_query("id", &[min_id], &[max_id])
}

// Each `random.next_boolean()` draws: the repeated conditions are Java's
// `if (random().nextBoolean()) ... else if (random().nextBoolean())`.
#[allow(clippy::ifs_same_cond)]
fn test_random_single_valued() -> Result<()> {
    let random = LuceneTestCase::random();
    let dir = LuceneTestCase::new_directory();
    let w = RandomIndexWriter::new_with_mock_analyzer(&*random, dir.as_directory())?;

    let doc_count = LuceneTestCase::at_least(1000);
    let missing_chance = random.next_double();
    let max_value = if random.next_boolean() {
        random.next_long() & i64::MAX
    } else {
        i64::from(random.next_int_bound(1000))
    };
    if *VERBOSE {
        println!(
            "TEST: valueCount={doc_count} valueRange=-{max_value}-{max_value} missingChance={missing_chance}"
        );
    }
    let mut values: Vec<Option<i64>> = vec![None; doc_count as usize];
    let mut missing_count = 0;
    for i in 0..doc_count {
        let mut doc = Document::new();
        doc.add(IntPoint::new("id", &[i])?);
        if random.next_double() > missing_chance {
            let value = TestUtil::next_long(&*random, -max_value, max_value);
            doc.add(NumericDocValuesField::new("field", value));
            values[i as usize] = Some(value);
        } else {
            missing_count += 1;
        }
        w.add_document(&doc)?;
    }

    let r = w.get_reader()?;
    w.close()?;
    let ir = as_index_reader(&r);

    let s = LuceneTestCase::new_searcher(Arc::clone(&r));

    let iters = LuceneTestCase::at_least(100);
    for iter in 0..iters {
        if *VERBOSE {
            println!("\nTEST: iter={iter}");
            println!("  test all docs");
        }

        // all docs
        let mut expected: HashMap<i64, i32> = HashMap::new();
        let mut expected_child_count = 0;
        for v in values.iter().flatten() {
            let cur = expected.entry(*v).or_insert_with(|| {
                expected_child_count += 1;
                0
            });
            *cur += 1;
        }

        let mut expected_counts: Vec<(i64, i32)> = expected.into_iter().collect();

        // sort by value
        expected_counts.sort_by_key(|e| e.0);

        let facet_counts = if random.next_boolean() {
            let fc = match_all(&s)?;
            if random.next_boolean() {
                if *VERBOSE {
                    println!("  use value source");
                }
                single_valued_source_counts(&fc)?
            } else {
                if *VERBOSE {
                    println!("  use doc values");
                }
                LongValueFacetCounts::new("field", &fc)?
            }
        } else {
            // optimized count all:
            if random.next_boolean() {
                if *VERBOSE {
                    println!("  count all value source");
                }
                if random.next_boolean() {
                    LongValueFacetCounts::count_all_docs_with_source(
                        "field",
                        Some(lvs::from_long_field("field")),
                        &ir,
                    )?
                } else if random.next_boolean() {
                    LongValueFacetCounts::count_all_docs_with_multi_source(
                        "field",
                        Some(<dyn MultiLongValuesSource>::from_long_field("field")),
                        &ir,
                    )?
                } else {
                    LongValueFacetCounts::count_all_docs_with_multi_source(
                        "field",
                        Some(<dyn MultiLongValuesSource>::from_single_valued(
                            lvs::from_long_field("field"),
                        )),
                        &ir,
                    )?
                }
            } else {
                if *VERBOSE {
                    println!("  count all doc values");
                }
                LongValueFacetCounts::count_all_docs("field", &ir)?
            }
        };

        let actual = facet_counts.get_all_children_sort_by_value();
        assert_same(
            "all docs, sort facets by value",
            &expected_counts,
            expected_child_count,
            doc_count - missing_count,
            &actual,
            i32::MAX,
        );

        // test getAllChildren
        expected_counts.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
        let mut all_children = facet_counts.get_all_children("field", &[])?.unwrap();
        // sort labels by value, count in ascending order
        sort_by_label_then_value(&mut all_children.label_values);
        assert_same(
            "test getAllChildren",
            &expected_counts,
            expected_child_count,
            doc_count - missing_count,
            &actual,
            i32::MAX,
        );

        // sort by count
        sort_by_count(&mut expected_counts);
        let top_n = random_top_n(doc_count);
        if *VERBOSE {
            println!("  topN={top_n}");
        }
        let actual = facet_counts.get_top_children(top_n, "field", &[])?.unwrap();
        assert_same(
            "all docs, sort facets by count",
            &expected_counts,
            expected_child_count,
            doc_count - missing_count,
            &actual,
            top_n,
        );

        // subset of docs
        let mut min_id = random.next_int_bound(doc_count);
        let mut max_id = random.next_int_bound(doc_count);
        if min_id > max_id {
            std::mem::swap(&mut min_id, &mut max_id);
        }
        if *VERBOSE {
            println!("  test id range {min_id}-{max_id}");
        }

        let fc = s.search_with_manager(
            id_range_query(min_id, max_id)?,
            &FacetsCollectorManager::new(),
        )?;
        let facet_counts = if random.next_boolean() {
            if *VERBOSE {
                println!("  use doc values");
            }
            LongValueFacetCounts::new("field", &fc)?
        } else {
            if *VERBOSE {
                println!("  use value source");
            }
            single_valued_source_counts(&fc)?
        };

        let mut expected: HashMap<i64, i32> = HashMap::new();
        let mut expected_child_count = 0;
        let mut tot_count = 0;
        for v in values[min_id as usize..=max_id as usize].iter().flatten() {
            tot_count += 1;
            let cur = expected.entry(*v).or_insert_with(|| {
                expected_child_count += 1;
                0
            });
            *cur += 1;
        }
        let mut expected_counts: Vec<(i64, i32)> = expected.into_iter().collect();

        // sort by value
        expected_counts.sort_by_key(|e| e.0);

        let actual = facet_counts.get_all_children_sort_by_value();
        assert_same(
            &format!("id {min_id}-{max_id}, sort facets by value"),
            &expected_counts,
            expected_child_count,
            tot_count,
            &actual,
            i32::MAX,
        );

        // sort by count
        sort_by_count(&mut expected_counts);
        let top_n = random_top_n(doc_count);
        let actual = facet_counts.get_top_children(top_n, "field", &[])?.unwrap();
        assert_same(
            &format!("id {min_id}-{max_id}, sort facets by count"),
            &expected_counts,
            expected_child_count,
            tot_count,
            &actual,
            top_n,
        );
    }
    r.close()?;
    dir.close()
}

// Java's indexed loop over `values[i]` alongside the doc id `i`.
#[allow(clippy::needless_range_loop)]
fn test_random_multi_valued() -> Result<()> {
    let random = LuceneTestCase::random();
    let dir = LuceneTestCase::new_directory();
    let w = RandomIndexWriter::new_with_mock_analyzer(&*random, dir.as_directory())?;

    let doc_count = LuceneTestCase::at_least(1000);
    let missing_chance = random.next_double();

    // sometimes exercise codec optimizations when a claimed multi valued field is in fact single
    // valued:
    let all_single_valued = LuceneTestCase::rarely();
    let max_value = if random.next_boolean() {
        random.next_long() & i64::MAX
    } else {
        i64::from(random.next_int_bound(1000))
    };
    if *VERBOSE {
        println!(
            "TEST: valueCount={doc_count} valueRange=-{max_value}-{max_value} missingChance={missing_chance} allSingleValued={all_single_valued}"
        );
    }

    let mut values: Vec<Option<Vec<i64>>> = vec![None; doc_count as usize];
    for i in 0..doc_count as usize {
        let mut doc = Document::new();
        doc.add(IntPoint::new("id", &[i as i32])?);
        if random.next_double() > missing_chance {
            let len = if all_single_valued {
                1
            } else {
                TestUtil::next_int(&*random, 1, 5) as usize
            };
            let mut vals = vec![0i64; len];
            for v in vals.iter_mut() {
                let value = TestUtil::next_long(&*random, -max_value, max_value);
                *v = value;
                doc.add(SortedNumericDocValuesField::new("field", value));
            }
            if *VERBOSE {
                println!("  doc={i} values={vals:?}");
            }
            // sort values to enable duplicate detection by comparing with the previous value
            vals.sort_unstable();
            values[i] = Some(vals);
        } else if *VERBOSE {
            println!("  doc={i} missing values");
        }
        w.add_document(&doc)?;
    }

    let r = w.get_reader()?;
    w.close()?;
    let ir = as_index_reader(&r);

    let s = LuceneTestCase::new_searcher(Arc::clone(&r));

    let iters = LuceneTestCase::at_least(100);
    for iter in 0..iters {
        if *VERBOSE {
            println!("\nTEST: iter={iter}");
            println!("  test all docs");
        }

        // all docs
        let mut expected: HashMap<i64, i32> = HashMap::new();
        let mut expected_total_count = 0;
        for v in values.iter().flatten() {
            if !v.is_empty() {
                expected_total_count += 1;
                set_expected_frequencies(v, &mut expected);
            }
        }

        let expected_child_count = expected.len() as i32;
        let mut expected_counts: Vec<(i64, i32)> = expected.into_iter().collect();

        // sort by value
        expected_counts.sort_by_key(|e| e.0);

        let facet_counts = if random.next_boolean() {
            let fc = match_all(&s)?;
            if *VERBOSE {
                println!("  use doc values");
            }
            if random.next_boolean() {
                LongValueFacetCounts::new("field", &fc)?
            } else {
                LongValueFacetCounts::with_multi_values_source(
                    "field",
                    Some(<dyn MultiLongValuesSource>::from_long_field("field")),
                    &fc,
                )?
            }
        } else {
            // optimized count all:
            if *VERBOSE {
                println!("  count all doc values");
            }
            if random.next_boolean() {
                LongValueFacetCounts::count_all_docs("field", &ir)?
            } else {
                LongValueFacetCounts::count_all_docs_with_multi_source(
                    "field",
                    Some(<dyn MultiLongValuesSource>::from_long_field("field")),
                    &ir,
                )?
            }
        };

        let actual = facet_counts.get_all_children_sort_by_value();
        assert_same(
            "all docs, sort facets by value",
            &expected_counts,
            expected_child_count,
            expected_total_count,
            &actual,
            i32::MAX,
        );

        // test getAllChildren
        expected_counts.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
        let mut all_children = facet_counts.get_all_children("field", &[])?.unwrap();
        // sort labels by value, count in ascending order
        sort_by_label_then_value(&mut all_children.label_values);
        assert_same(
            "test getAllChildren",
            &expected_counts,
            expected_child_count,
            expected_total_count,
            &actual,
            i32::MAX,
        );

        // sort by count
        sort_by_count(&mut expected_counts);
        let top_n = random_top_n(doc_count);
        if *VERBOSE {
            println!("  topN={top_n}");
        }
        let actual = facet_counts.get_top_children(top_n, "field", &[])?.unwrap();
        assert_same(
            "all docs, sort facets by count",
            &expected_counts,
            expected_child_count,
            expected_total_count,
            &actual,
            top_n,
        );

        // subset of docs
        let mut min_id = random.next_int_bound(doc_count);
        let mut max_id = random.next_int_bound(doc_count);
        if min_id > max_id {
            std::mem::swap(&mut min_id, &mut max_id);
        }
        if *VERBOSE {
            println!("  test id range {min_id}-{max_id}");
        }

        let fc = s.search_with_manager(
            id_range_query(min_id, max_id)?,
            &FacetsCollectorManager::new(),
        )?;
        let facet_counts = if random.next_boolean() {
            LongValueFacetCounts::new("field", &fc)?
        } else {
            LongValueFacetCounts::with_multi_values_source(
                "field",
                Some(<dyn MultiLongValuesSource>::from_long_field("field")),
                &fc,
            )?
        };

        let mut expected: HashMap<i64, i32> = HashMap::new();
        let mut expected_total_count = 0;
        for v in values[min_id as usize..=max_id as usize].iter().flatten() {
            if !v.is_empty() {
                expected_total_count += 1;
                set_expected_frequencies(v, &mut expected);
            }
        }
        let expected_child_count = expected.len() as i32;
        let mut expected_counts: Vec<(i64, i32)> = expected.into_iter().collect();

        // sort by value
        expected_counts.sort_by_key(|e| e.0);

        let actual = facet_counts.get_all_children_sort_by_value();
        assert_same(
            &format!("id {min_id}-{max_id}, sort facets by value"),
            &expected_counts,
            expected_child_count,
            expected_total_count,
            &actual,
            i32::MAX,
        );

        // sort by count
        sort_by_count(&mut expected_counts);
        let top_n = random_top_n(doc_count);
        let actual = facet_counts.get_top_children(top_n, "field", &[])?.unwrap();
        assert_same(
            &format!("id {min_id}-{max_id}, sort facets by count"),
            &expected_counts,
            expected_child_count,
            expected_total_count,
            &actual,
            top_n,
        );
    }
    r.close()?;
    dir.close()
}

fn set_expected_frequencies(values: &[i64], expected: &mut HashMap<i64, i32>) {
    let mut previous_value = 0i64;
    for (j, &v) in values.iter().enumerate() {
        if j == 0 || previous_value != v {
            *expected.entry(v).or_insert(0) += 1;
        }
        previous_value = v;
    }
}

fn assert_same(
    desc: &str,
    expected_counts: &[(i64, i32)],
    expected_child_count: i32,
    expected_total_count: i32,
    actual: &FacetResult,
    top_n: i32,
) {
    let expected_top_n = (top_n as usize).min(expected_counts.len());
    if *VERBOSE {
        println!("  expected topN={expected_top_n}");
        for (i, (k, v)) in expected_counts.iter().take(expected_top_n).enumerate() {
            println!("    {i}: value={k} count={v}");
        }
        println!("  actual topN={}", actual.label_values.len());
        for (i, l) in actual.label_values.iter().enumerate() {
            println!("    {i}: value={} count={}", l.label, l.value);
        }
    }
    Assert::assert_equals_msg(
        Some(&format!("{desc}: topN")),
        expected_top_n,
        actual.label_values.len(),
    );
    Assert::assert_equals_msg(
        Some(&format!("{desc}: childCount")),
        expected_child_count,
        actual.child_count,
    );
    Assert::assert_equals_msg(
        Some(&format!("{desc}: totCount")),
        expected_total_count,
        number_int_value(&actual.value),
    );
    Assert::assert_true(actual.label_values.len() <= top_n as usize);

    for i in 0..expected_top_n {
        Assert::assert_equals_msg(
            Some(&format!("{desc}: label[{i}]")),
            expected_counts[i].0.to_string().as_str(),
            actual.label_values[i].label.as_str(),
        );
        Assert::assert_equals_msg(
            Some(&format!("{desc}: counts[{i}]")),
            expected_counts[i].1,
            number_int_value(&actual.label_values[i].value),
        );
    }
}

/// LUCENE-9964: Duplicate long values in a document field should only be counted once when using
/// SortedNumericDocValuesFields
fn test_duplicate_long_values() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let w =
        RandomIndexWriter::new_with_mock_analyzer(&*LuceneTestCase::random(), dir.as_directory())?;

    // these two values are not unique in a document
    for v in [42i64, 43] {
        let mut doc = Document::new();
        doc.add(SortedNumericDocValuesField::new("field", v));
        doc.add(SortedNumericDocValuesField::new("field", v));
        w.add_document(&doc)?;
    }
    let r = w.get_reader()?;
    w.close()?;

    let facet_counts = LongValueFacetCounts::count_all_docs("field", &as_index_reader(&r))?;

    let fr = facet_counts.get_all_children_sort_by_value();
    for label_and_value in &fr.label_values {
        assert!(label_and_value.value == NumericValue::Int(1));
    }

    assert_facet_result(
        &facet_counts.get_all_children("field", &[])?.unwrap(),
        "field",
        &[],
        2,
        NumericValue::Int(2),
        &[lv("42", 1), lv("43", 1)],
    );

    r.close()?;
    dir.close()
}
