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

//! Port of `org.apache.lucene.facet.facetset.TestRangeFacetSetMatcher`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::jdk::util::shuffle;
use randomizedtesting::junit::Assert;
use rucene::document::range_fields::LongRange;
use rucene::document::{Document, LongPoint, NumericValue};
use rucene::error::Result;
use rucene::search::{BooleanQueryBuilder, MatchAllDocsQuery, Occur, Query};
use rucene_facet::facets::Facets;
use rucene_facet::facets_collector_manager::FacetsCollectorManager;
use rucene_facet::facetset::dim_range::DimRange;
use rucene_facet::facetset::double_facet_set::DoubleFacetSet;
use rucene_facet::facetset::facet_set::FacetSet;
use rucene_facet::facetset::facet_set_decoder::{decode_ints, decode_longs, FacetSetDecoder};
use rucene_facet::facetset::facet_set_matcher::FacetSetMatcher;
use rucene_facet::facetset::facet_sets_field::FacetSetsField;
use rucene_facet::facetset::float_facet_set::FloatFacetSet;
use rucene_facet::facetset::int_facet_set::IntFacetSet;
use rucene_facet::facetset::long_facet_set::LongFacetSet;
use rucene_facet::facetset::matching_facet_sets_counts::MatchingFacetSetsCounts;
use rucene_facet::facetset::range_facet_set_matcher::RangeFacetSetMatcher;
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
const YEARS: [i32; 5] = [2010, 2011, 2012, 2013, 2014];

/// `TestRangeFacetSetMatcher`.
pub fn class_def() -> Arc<Class> {
    facet_class(
        "org.apache.lucene.facet.facetset.TestRangeFacetSetMatcher",
        vec![
            test(
                "testLongRangeFacetSetMatching",
                test_long_range_facet_set_matching,
            ),
            test(
                "testIntRangeFacetSetMatching",
                test_int_range_facet_set_matching,
            ),
            test(
                "testDoubleRangeFacetSetMatching",
                test_double_range_facet_set_matching,
            ),
            test(
                "testFloatRangeFacetSetMatching",
                test_float_range_facet_set_matching,
            ),
            test(
                "testLongRangeFacetSetMatchingWithFastMatchQuery",
                test_long_range_facet_set_matching_with_fast_match_query,
            ),
        ],
    )
}

/// The four counters of one test, in the order of its four matchers.
type Counts = [i32; 4];

/// The shared body of the `test*RangeFacetSetMatching` methods.
/// `classify(set, counts)` updates the four counters (in matcher order)
/// for one set and answers whether it matched; `add_fields` adds the
/// drill-down fields of a document; `matchers` are the four labelled
/// matchers in order.
fn check_range_matching<S: FacetSet + Clone>(
    mut all_sets: Vec<S>,
    classify: impl Fn(&S, &mut Counts) -> bool,
    add_fields: impl Fn(&mut Document, &[S]) -> Result<()>,
    decoder: Arc<dyn FacetSetDecoder>,
    fast_match_query: Option<Arc<dyn Query>>,
    matchers: Vec<(&str, DimRange, DimRange)>,
) -> Result<()> {
    let d = LuceneTestCase::new_directory();
    let w =
        RandomIndexWriter::new_with_mock_analyzer(&*LuceneTestCase::random(), d.as_directory())?;

    let mut counts: Counts = [0; 4];
    let mut num_matching_docs = 0;
    for _ in 0..100 {
        let mut doc = Document::new();
        let num_sets = TestUtil::next_int(&*LuceneTestCase::random(), 1, 4) as usize;
        shuffle(&mut all_sets, &*LuceneTestCase::random());
        let facet_sets: Vec<S> = all_sets[..num_sets].to_vec();
        let mut matching_doc = false;
        for facet_set in &facet_sets {
            if classify(facet_set, &mut counts) {
                matching_doc = true;
            }
        }
        num_matching_docs += if matching_doc { 1 } else { 0 };
        let refs: Vec<&dyn FacetSet> = facet_sets.iter().map(|s| s as &dyn FacetSet).collect();
        doc.add(FacetSetsField::create("field", &refs)?);
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

    let labels: Vec<String> = matchers.iter().map(|(l, _, _)| l.to_string()).collect();
    let mut facet_set_matchers: Vec<Arc<dyn FacetSetMatcher>> = Vec::new();
    for (label, manufacturer, year) in matchers {
        facet_set_matchers.push(Arc::new(RangeFacetSetMatcher::new(
            label,
            &[manufacturer, year],
        )?));
    }
    let facets = MatchingFacetSetsCounts::with_fast_match(
        "field",
        &fc,
        decoder,
        fast_match_query,
        facet_set_matchers,
    )?;

    let result = facets.get_all_children("field", &[])?.unwrap();

    Assert::assert_equals("field", result.dim.as_str());
    Assert::assert_equals(0, result.path.len());
    Assert::assert_equals(NumericValue::Int(num_matching_docs), result.value);
    Assert::assert_equals(4, result.child_count);

    for (i, label) in labels.iter().enumerate() {
        Assert::assert_equals(
            LabelAndValue::new(label.as_str(), NumericValue::Int(counts[i])),
            result.label_values[i].clone(),
        );
    }

    r.close()?;
    d.close()
}

fn no_fields<S>(_doc: &mut Document, _sets: &[S]) -> Result<()> {
    Ok(())
}

/// The counters of the long and int tests, indexed as the matchers
/// `[2010-2014]`, `(2010-2014]`, `[2010-2014)`, `(2010-2014)`.
fn classify_long_years(year: i64, counts: &mut Counts) -> bool {
    // numFord2010_2014, numFord2011_2014, numFord2010_2013, numFord2011_2013
    if year > 2010 && year < 2014 {
        counts[2] += 1;
        counts[0] += 1;
        counts[3] += 1;
        counts[1] += 1;
        true
    } else if year == 2014 {
        counts[0] += 1;
        counts[1] += 1;
        true
    } else if year == 2010 {
        counts[0] += 1;
        counts[2] += 1;
        true
    } else {
        false
    }
}

fn long_matchers() -> Result<Vec<(&'static str, DimRange, DimRange)>> {
    Ok(vec![
        (
            "Ford [2010-2014]",
            single_long(FORD_ORD as i64)?,
            long_range(2010, true, 2014, true)?,
        ),
        (
            "Ford (2010-2014]",
            single_long(FORD_ORD as i64)?,
            long_range(2010, false, 2014, true)?,
        ),
        (
            "Ford [2010-2014)",
            single_long(FORD_ORD as i64)?,
            long_range(2010, true, 2014, false)?,
        ),
        (
            "Ford (2010-2014)",
            single_long(FORD_ORD as i64)?,
            long_range(2010, false, 2014, false)?,
        ),
    ])
}

fn long_sets() -> Result<Vec<LongFacetSet>> {
    let mut all_sets = Vec::new();
    for manufacturer_ord in MANUFACTURER_ORDS {
        for year in YEARS {
            all_sets.push(LongFacetSet::new(vec![
                manufacturer_ord as i64,
                year as i64,
            ])?);
        }
    }
    Ok(all_sets)
}

fn test_long_range_facet_set_matching() -> Result<()> {
    check_range_matching(
        long_sets()?,
        |s: &LongFacetSet, counts: &mut Counts| {
            if FORD_ORD as i64 != s.values[0] {
                return false;
            }
            classify_long_years(s.values[1], counts)
        },
        no_fields,
        Arc::new(decode_longs),
        None,
        long_matchers()?,
    )
}

fn test_int_range_facet_set_matching() -> Result<()> {
    let mut all_sets = Vec::new();
    for manufacturer_ord in MANUFACTURER_ORDS {
        for year in YEARS {
            all_sets.push(IntFacetSet::new(vec![manufacturer_ord, year])?);
        }
    }
    check_range_matching(
        all_sets,
        |s: &IntFacetSet, counts: &mut Counts| {
            if FORD_ORD != s.values[0] {
                return false;
            }
            classify_long_years(i64::from(s.values[1]), counts)
        },
        no_fields,
        Arc::new(decode_ints),
        None,
        long_matchers()?,
    )
}

/// The counters of the double and float tests, indexed as the matchers
/// `[2010-2015]`, `(2011-2015]`, `[2010-2014)`, `(2011-2014)`, years being
/// `year + 0.5`.
fn classify_half_years(year: f64, lo: f64, hi: f64, counts: &mut Counts) -> bool {
    // numFord2010_2015, numFord2011_2015, numFord2010_2014, numFord2011_2014
    if year > lo && year < hi {
        counts[2] += 1;
        counts[0] += 1;
        counts[3] += 1;
        counts[1] += 1;
        true
    } else if year == hi {
        counts[0] += 1;
        counts[1] += 1;
        true
    } else if year == lo {
        counts[0] += 1;
        counts[2] += 1;
        true
    } else {
        false
    }
}

fn test_double_range_facet_set_matching() -> Result<()> {
    let mut all_sets = Vec::new();
    for manufacturer_ord in MANUFACTURER_ORDS {
        for year in YEARS {
            all_sets.push(DoubleFacetSet::new(vec![
                f64::from(manufacturer_ord),
                f64::from(year) + 0.5,
            ])?);
        }
    }
    check_range_matching(
        all_sets,
        |s: &DoubleFacetSet, counts: &mut Counts| {
            // `Double.compare(FORD_ORD, facetSet.values[0]) != 0`
            if f64::from(FORD_ORD).total_cmp(&s.values[0]) != std::cmp::Ordering::Equal {
                return false;
            }
            classify_half_years(s.values[1], 2010.5, 2014.5, counts)
        },
        no_fields,
        Arc::new(decode_longs),
        None,
        vec![
            (
                "Ford [2010-2015]",
                single_double(f64::from(FORD_ORD))?,
                double_range(2010.0, true, 2015.0, true)?,
            ),
            (
                "Ford (2011-2015]",
                single_double(f64::from(FORD_ORD))?,
                double_range(2011.0, false, 2015.0, true)?,
            ),
            (
                "Ford [2010-2014)",
                single_double(f64::from(FORD_ORD))?,
                double_range(2010.0, true, 2014.0, false)?,
            ),
            (
                "Ford (2011-2014)",
                single_double(f64::from(FORD_ORD))?,
                double_range(2011.0, false, 2014.0, false)?,
            ),
        ],
    )
}

fn test_float_range_facet_set_matching() -> Result<()> {
    let mut all_sets = Vec::new();
    for manufacturer_ord in MANUFACTURER_ORDS {
        for year in YEARS {
            all_sets.push(FloatFacetSet::new(vec![
                manufacturer_ord as f32,
                year as f32 + 0.5f32,
            ])?);
        }
    }
    check_range_matching(
        all_sets,
        |s: &FloatFacetSet, counts: &mut Counts| {
            // `Double.compare(FORD_ORD, facetSet.values[0]) != 0`
            if f64::from(FORD_ORD).total_cmp(&f64::from(s.values[0])) != std::cmp::Ordering::Equal {
                return false;
            }
            classify_half_years(
                f64::from(s.values[1]),
                f64::from(2010.5f32),
                f64::from(2014.5f32),
                counts,
            )
        },
        no_fields,
        Arc::new(decode_ints),
        None,
        vec![
            (
                "Ford [2010-2015]",
                single_float(FORD_ORD as f32)?,
                float_range(2010.0f32, true, 2015.0f32, true)?,
            ),
            (
                "Ford (2010-2015]",
                single_float(FORD_ORD as f32)?,
                float_range(2010.5f32, false, 2015.0f32, true)?,
            ),
            (
                "Ford [2010-2014)",
                single_float(FORD_ORD as f32)?,
                float_range(2010.0f32, true, 2014.0f32, false)?,
            ),
            (
                "Ford (2011-2014)",
                single_float(FORD_ORD as f32)?,
                float_range(2011.0f32, false, 2014.0f32, false)?,
            ),
        ],
    )
}

fn test_long_range_facet_set_matching_with_fast_match_query() -> Result<()> {
    let mut fast_match_query = BooleanQueryBuilder::new();
    fast_match_query.add(
        create_fast_match_query("manufacturer", FORD_ORD as i64, FORD_ORD as i64)?,
        Occur::MUST,
    )?;
    fast_match_query.add(create_fast_match_query("year", 2010, 2014)?, Occur::MUST)?;
    let fast_match_query: Arc<dyn Query> = Arc::new(fast_match_query.build());
    check_range_matching(
        long_sets()?,
        |s: &LongFacetSet, counts: &mut Counts| {
            if FORD_ORD as i64 != s.values[0] {
                return false;
            }
            classify_long_years(s.values[1], counts)
        },
        |doc: &mut Document, facet_sets: &[LongFacetSet]| {
            // add fields for drill-down + fast matching
            add_fast_match_field("manufacturer", doc, facet_sets, 0)?;
            add_fast_match_field("year", doc, facet_sets, 1)
        },
        Arc::new(decode_longs),
        Some(fast_match_query),
        long_matchers()?,
    )
}

/// `singleLong(long)`.
fn single_long(value: i64) -> Result<DimRange> {
    DimRange::from_longs(value, true, value, true)
}

/// `longRange(long, boolean, long, boolean)`.
fn long_range(min: i64, min_exclusive: bool, max: i64, max_exclusive: bool) -> Result<DimRange> {
    DimRange::from_longs(min, min_exclusive, max, max_exclusive)
}

/// `singleDouble(double)`.
fn single_double(value: f64) -> Result<DimRange> {
    DimRange::from_doubles(value, true, value, true)
}

/// `doubleRange(double, boolean, double, boolean)`.
fn double_range(min: f64, min_exclusive: bool, max: f64, max_exclusive: bool) -> Result<DimRange> {
    DimRange::from_doubles(min, min_exclusive, max, max_exclusive)
}

/// `singleFloat(float)`.
fn single_float(value: f32) -> Result<DimRange> {
    DimRange::from_floats(value, true, value, true)
}

/// `floatRange(float, boolean, float, boolean)`.
fn float_range(min: f32, min_exclusive: bool, max: f32, max_exclusive: bool) -> Result<DimRange> {
    DimRange::from_floats(min, min_exclusive, max, max_exclusive)
}

/// `createFastMatchQuery(String, long, long)`.
fn create_fast_match_query(field: &str, min: i64, max: i64) -> Result<Arc<dyn Query>> {
    Ok(Arc::new(LongRange::new_intersects_query(
        field,
        &[min],
        &[max],
    )?))
}

/// `addFastMatchField(String, Document, LongFacetSet[], int)`.
fn add_fast_match_field(
    field: &str,
    doc: &mut Document,
    facet_sets: &[LongFacetSet],
    index: usize,
) -> Result<()> {
    let min = facet_sets
        .iter()
        .map(|facet_set| facet_set.values[index])
        .min()
        .expect("java.util.NoSuchElementException");
    let max = facet_sets
        .iter()
        .map(|facet_set| facet_set.values[index])
        .max()
        .expect("java.util.NoSuchElementException");
    doc.add(LongPoint::new(field.to_string(), &[min, max])?);
    Ok(())
}
