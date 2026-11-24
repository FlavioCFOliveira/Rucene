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

//! Port of `org.apache.lucene.facet.facetset.TestExactFacetSetMatcher`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::jdk::util::shuffle;
use randomizedtesting::junit::Assert;
use rucene::document::{Document, NumericValue, Store, StringField};
use rucene::error::Result;
use rucene::search::term_in_set_query::TermInSetQuery;
use rucene::search::{BooleanQueryBuilder, MatchAllDocsQuery, Occur, Query};
use rucene::util::BytesRef;
use rucene_facet::facets::Facets;
use rucene_facet::facets_collector_manager::FacetsCollectorManager;
use rucene_facet::facetset::double_facet_set::DoubleFacetSet;
use rucene_facet::facetset::exact_facet_set_matcher::ExactFacetSetMatcher;
use rucene_facet::facetset::facet_set::FacetSet;
use rucene_facet::facetset::facet_set_decoder::{decode_ints, decode_longs, FacetSetDecoder};
use rucene_facet::facetset::facet_set_matcher::FacetSetMatcher;
use rucene_facet::facetset::facet_sets_field::FacetSetsField;
use rucene_facet::facetset::float_facet_set::FloatFacetSet;
use rucene_facet::facetset::int_facet_set::IntFacetSet;
use rucene_facet::facetset::long_facet_set::LongFacetSet;
use rucene_facet::facetset::matching_facet_sets_counts::MatchingFacetSetsCounts;
use rucene_facet::label_and_value::LabelAndValue;
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::facet_support::core_support::test;
use crate::facet_support::facet_class;

const FORD_ORD: i32 = 100;
const TOYOTA_ORD: i32 = 101;
const CHEVY_ORD: i32 = 102;
const NISSAN_ORD: i32 = 103;
const MANUFACTURER_ORDS: [i32; 4] = [FORD_ORD, TOYOTA_ORD, CHEVY_ORD, NISSAN_ORD];
const YEARS: [i32; 3] = [2010, 2011, 2012];

/// `TestExactFacetSetMatcher`.
pub fn class_def() -> Arc<Class> {
    facet_class(
        "org.apache.lucene.facet.facetset.TestExactFacetSetMatcher",
        vec![
            test("testSimpleFacetSetMatching", test_simple_facet_set_matching),
            test("testLongFacetSetMatching", test_long_facet_set_matching),
            test("testIntFacetSetMatching", test_int_facet_set_matching),
            test("testDoubleFacetSetMatching", test_double_facet_set_matching),
            test("testFloatFacetSetMatching", test_float_facet_set_matching),
            test(
                "testLongFacetSetMatchingWithFastMatchQuery",
                test_long_facet_set_matching_with_fast_match_query,
            ),
        ],
    )
}

fn long_set(values: &[i64]) -> Result<LongFacetSet> {
    LongFacetSet::new(values.to_vec())
}

fn exact(label: &str, facet_set: &dyn FacetSet) -> Result<Arc<dyn FacetSetMatcher>> {
    Ok(Arc::new(ExactFacetSetMatcher::new(label, facet_set)?))
}

fn test_simple_facet_set_matching() -> Result<()> {
    let d = LuceneTestCase::new_directory();
    let w =
        RandomIndexWriter::new_with_mock_analyzer(&*LuceneTestCase::random(), d.as_directory())?;

    // Ford-2010, Chevy-2011
    let mut doc = Document::new();
    doc.add(FacetSetsField::create(
        "field",
        &[
            &long_set(&[FORD_ORD as i64, 2010])?,
            &long_set(&[CHEVY_ORD as i64, 2011])?,
        ],
    )?);
    w.add_document(&doc)?;

    // Ford-2011, Chevy-2010
    let mut doc = Document::new();
    doc.add(FacetSetsField::create(
        "field",
        &[
            &long_set(&[FORD_ORD as i64, 2011])?,
            &long_set(&[CHEVY_ORD as i64, 2010])?,
        ],
    )?);
    w.add_document(&doc)?;

    let r = w.get_reader()?;
    w.close()?;

    let s = LuceneTestCase::new_searcher(Arc::clone(&r));
    let fc = s.search_with_manager(
        Arc::new(MatchAllDocsQuery::instance()),
        &FacetsCollectorManager::new(),
    )?;

    let facets = MatchingFacetSetsCounts::new(
        "field",
        &fc,
        Arc::new(decode_longs),
        vec![
            exact("Ford 2010", &long_set(&[FORD_ORD as i64, 2010])?)?,
            exact("Chevy 2011", &long_set(&[CHEVY_ORD as i64, 2011])?)?,
        ],
    )?;

    let result = facets.get_all_children("field", &[])?.unwrap();

    Assert::assert_equals("field", result.dim.as_str());
    Assert::assert_equals(0, result.path.len());
    Assert::assert_equals(NumericValue::Int(1), result.value);
    Assert::assert_equals(2, result.child_count);

    Assert::assert_equals(
        LabelAndValue::new("Ford 2010", NumericValue::Int(1)),
        result.label_values[0].clone(),
    );
    Assert::assert_equals(
        LabelAndValue::new("Chevy 2011", NumericValue::Int(1)),
        result.label_values[1].clone(),
    );

    r.close()?;
    d.close()
}

/// The shared body of the `test*FacetSetMatching` methods: `all_sets` are
/// the candidate sets, `ford_2010` / `chevy_2011` classify one set,
/// `add_fields` adds the drill-down fields of a document (the fast-match
/// variant) and `fast_match_query` is passed to `MatchingFacetSetsCounts`.
#[allow(clippy::too_many_arguments)]
fn check_facet_set_matching<S: FacetSet + Clone>(
    mut all_sets: Vec<S>,
    ford_2010: impl Fn(&S) -> bool,
    chevy_2011: impl Fn(&S) -> bool,
    add_fields: impl Fn(&mut Document, &[S]) -> Result<()>,
    decoder: Arc<dyn FacetSetDecoder>,
    fast_match_query: Option<Arc<dyn Query>>,
    ford_matcher: S,
    chevy_matcher: S,
) -> Result<()> {
    let d = LuceneTestCase::new_directory();
    let w =
        RandomIndexWriter::new_with_mock_analyzer(&*LuceneTestCase::random(), d.as_directory())?;

    let mut num_ford_2010 = 0;
    let mut num_chevy_2011 = 0;
    let mut num_matching_docs = 0;
    for _ in 0..100 {
        let mut doc = Document::new();
        let num_sets = TestUtil::next_int(&*LuceneTestCase::random(), 1, 4) as usize;
        shuffle(&mut all_sets, &*LuceneTestCase::random());
        let facet_sets: Vec<S> = all_sets[..num_sets].to_vec();
        let mut matching_doc = false;
        for facet_set in &facet_sets {
            if ford_2010(facet_set) {
                num_ford_2010 += 1;
                matching_doc = true;
            } else if chevy_2011(facet_set) {
                num_chevy_2011 += 1;
                matching_doc = true;
            }
        }
        num_matching_docs += if matching_doc { 1 } else { 0 };
        let refs: Vec<&dyn FacetSet> = facet_sets.iter().map(|s| s as &dyn FacetSet).collect();
        doc.add(FacetSetsField::create("field", &refs)?); // field for aggregation
        add_fields(&mut doc, &facet_sets)?;
        w.add_document(&doc)?;
    }

    let r = w.get_reader()?;
    w.close()?;

    let s = LuceneTestCase::new_searcher(Arc::clone(&r));
    let fc = s.search_with_manager(
        Arc::new(MatchAllDocsQuery::instance()),
        &FacetsCollectorManager::new(),
    )?;

    let facets = MatchingFacetSetsCounts::with_fast_match(
        "field",
        &fc,
        decoder,
        fast_match_query,
        vec![
            exact("Ford 2010", &ford_matcher)?,
            exact("Chevy 2011", &chevy_matcher)?,
        ],
    )?;

    let result = facets.get_all_children("field", &[])?.unwrap();

    Assert::assert_equals("field", result.dim.as_str());
    Assert::assert_equals(0, result.path.len());
    Assert::assert_equals(NumericValue::Int(num_matching_docs), result.value);
    Assert::assert_equals(2, result.child_count);

    Assert::assert_equals(
        LabelAndValue::new("Ford 2010", NumericValue::Int(num_ford_2010)),
        result.label_values[0].clone(),
    );
    Assert::assert_equals(
        LabelAndValue::new("Chevy 2011", NumericValue::Int(num_chevy_2011)),
        result.label_values[1].clone(),
    );

    r.close()?;
    d.close()
}

fn no_fields<S>(_doc: &mut Document, _sets: &[S]) -> Result<()> {
    Ok(())
}

fn test_long_facet_set_matching() -> Result<()> {
    let mut all_sets = Vec::new();
    for manufacturer_ord in MANUFACTURER_ORDS {
        for year in YEARS {
            all_sets.push(long_set(&[manufacturer_ord as i64, year as i64])?);
        }
    }
    check_facet_set_matching(
        all_sets,
        |s: &LongFacetSet| FORD_ORD as i64 == s.values[0] && s.values[1] == 2010,
        |s: &LongFacetSet| CHEVY_ORD as i64 == s.values[0] && s.values[1] == 2011,
        no_fields,
        Arc::new(decode_longs),
        None,
        long_set(&[FORD_ORD as i64, 2010])?,
        long_set(&[CHEVY_ORD as i64, 2011])?,
    )
}

fn test_int_facet_set_matching() -> Result<()> {
    let mut all_sets = Vec::new();
    for manufacturer_ord in MANUFACTURER_ORDS {
        for year in YEARS {
            all_sets.push(IntFacetSet::new(vec![manufacturer_ord, year])?);
        }
    }
    check_facet_set_matching(
        all_sets,
        |s: &IntFacetSet| FORD_ORD == s.values[0] && s.values[1] == 2010,
        |s: &IntFacetSet| CHEVY_ORD == s.values[0] && s.values[1] == 2011,
        no_fields,
        Arc::new(decode_ints),
        None,
        IntFacetSet::new(vec![FORD_ORD, 2010])?,
        IntFacetSet::new(vec![CHEVY_ORD, 2011])?,
    )
}

fn test_double_facet_set_matching() -> Result<()> {
    let mut all_sets = Vec::new();
    for manufacturer_ord in MANUFACTURER_ORDS {
        for year in YEARS {
            all_sets.push(DoubleFacetSet::new(vec![
                f64::from(manufacturer_ord),
                f64::from(year) + 0.5,
            ])?);
        }
    }
    check_facet_set_matching(
        all_sets,
        |s: &DoubleFacetSet| f64::from(FORD_ORD) == s.values[0] && s.values[1] == 2010.5,
        |s: &DoubleFacetSet| f64::from(CHEVY_ORD) == s.values[0] && s.values[1] == 2011.5,
        no_fields,
        Arc::new(decode_longs),
        None,
        DoubleFacetSet::new(vec![f64::from(FORD_ORD), 2010.5])?,
        DoubleFacetSet::new(vec![f64::from(CHEVY_ORD), 2011.5])?,
    )
}

fn test_float_facet_set_matching() -> Result<()> {
    let mut all_sets = Vec::new();
    for manufacturer_ord in MANUFACTURER_ORDS {
        for year in YEARS {
            all_sets.push(FloatFacetSet::new(vec![
                manufacturer_ord as f32,
                year as f32 + 0.5f32,
            ])?);
        }
    }
    check_facet_set_matching(
        all_sets,
        |s: &FloatFacetSet| FORD_ORD as f32 == s.values[0] && s.values[1] == 2010.5f32,
        |s: &FloatFacetSet| CHEVY_ORD as f32 == s.values[0] && s.values[1] == 2011.5f32,
        no_fields,
        Arc::new(decode_ints),
        None,
        FloatFacetSet::new(vec![FORD_ORD as f32, 2010.5f32])?,
        FloatFacetSet::new(vec![CHEVY_ORD as f32, 2011.5f32])?,
    )
}

fn test_long_facet_set_matching_with_fast_match_query() -> Result<()> {
    let mut all_sets = Vec::new();
    for manufacturer_ord in MANUFACTURER_ORDS {
        for year in YEARS {
            all_sets.push(long_set(&[manufacturer_ord as i64, year as i64])?);
        }
    }
    let mut fast_match_query = BooleanQueryBuilder::new();
    fast_match_query.add(
        create_fast_match_query("manufacturer", &[FORD_ORD, CHEVY_ORD])?,
        Occur::MUST,
    )?;
    fast_match_query.add(create_fast_match_query("year", &[2010, 2011])?, Occur::MUST)?;
    let fast_match_query: Arc<dyn Query> = Arc::new(fast_match_query.build());
    check_facet_set_matching(
        all_sets,
        |s: &LongFacetSet| FORD_ORD as i64 == s.values[0] && s.values[1] == 2010,
        |s: &LongFacetSet| CHEVY_ORD as i64 == s.values[0] && s.values[1] == 2011,
        |doc: &mut Document, facet_sets: &[LongFacetSet]| {
            // add fields for drill-down + fast matching
            add_fast_match_field("manufacturer", doc, facet_sets, 0)?;
            add_fast_match_field("year", doc, facet_sets, 1)
        },
        Arc::new(decode_longs),
        Some(fast_match_query),
        long_set(&[FORD_ORD as i64, 2010])?,
        long_set(&[CHEVY_ORD as i64, 2011])?,
    )
}

/// `createFastMatchQuery(String, int...)`.
fn create_fast_match_query(field: &str, values: &[i32]) -> Result<Arc<dyn Query>> {
    Ok(Arc::new(TermInSetQuery::new(
        field,
        values
            .iter()
            .map(|v| BytesRef::from_char_sequence(&v.to_string()))
            .collect(),
    )?))
}

/// `addFastMatchField(String, Document, LongFacetSet[], int)`.
fn add_fast_match_field(
    field: &str,
    doc: &mut Document,
    facet_sets: &[LongFacetSet],
    index: usize,
) -> Result<()> {
    // `Stream.distinct()`: the first occurrence of each value, in order.
    let mut seen = Vec::new();
    for value in facet_sets.iter().map(|facet_set| facet_set.values[index]) {
        if !seen.contains(&value) {
            seen.push(value);
            doc.add(StringField::new(
                field.to_string(),
                value.to_string(),
                Store::NO,
            )?);
        }
    }
    Ok(())
}
