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

//! Port of `org.apache.lucene.facet.range.TestRangeFacetCounts`.

// Each `random.next_boolean()` of an `if` / `else if` chain is a fresh
// draw, as in the Java test, not a repeated condition.
#![allow(clippy::ifs_same_cond)]

use std::any::Any;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use randomizedtesting::generators::RandomNumbers;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::{
    Document, DoubleDocValuesField, DoublePoint, LongPoint, NumericDocValuesField,
    SortedNumericDocValuesField,
};
use rucene::error::Result;
use rucene::index::index_writer_config::OpenMode;
use rucene::index::LeafReaderContext;
use rucene::search::double_values::DoubleValues;
use rucene::search::double_values_source::{self, DoubleValuesSource};
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::long_values_source;
use rucene::search::matches::Matches;
use rucene::search::scorer::Scorer;
use rucene::search::scorer_supplier::ScorerSupplier;
use rucene::search::segment_cacheable::SegmentCacheable;
use rucene::search::similarities::Explanation;
use rucene::search::weight::{FilterWeight, Weight};
use rucene::search::{query_to_string, MatchAllDocsQuery, Query, QueryVisitor, ScoreMode};
use rucene::util::NumericUtils;
use rucene_facet::drill_down_query::DrillDownQuery;
use rucene_facet::drill_sideways::{DrillSideways, DrillSidewaysOverrides};
use rucene_facet::facet_field::FacetField;
use rucene_facet::facets::Facets;
use rucene_facet::facets_collector::FacetsCollector;
use rucene_facet::facets_collector_manager::FacetsCollectorManager;
use rucene_facet::facets_config::FacetsConfig;
use rucene_facet::label_and_value::number_int_value;
use rucene_facet::multi_double_values_source::MultiDoubleValuesSource;
use rucene_facet::multi_facets::MultiFacets;
use rucene_facet::multi_long_values_source::MultiLongValuesSource;
use rucene_facet::range::double_range::DoubleRange;
use rucene_facet::range::double_range_facet_counts::DoubleRangeFacetCounts;
use rucene_facet::range::long_range::LongRange;
use rucene_facet::range::long_range_facet_counts::LongRangeFacetCounts;
use rucene_facet::taxonomy::directory::directory_taxonomy_reader::DirectoryTaxonomyReader;
use rucene_facet::taxonomy::directory::directory_taxonomy_writer::DirectoryTaxonomyWriter;
use rucene_facet::taxonomy::taxonomy_reader::TaxonomyReader;
use rucene_facet::taxonomy::taxonomy_writer::TaxonomyWriter;
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::lucene_test_case::{Concurrency, TestDirectory, VERBOSE};
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::facet_support::core_support::{assert_equals, test};
use crate::facet_support::{facet_class, get_taxonomy_facet_counts};

const IAE: &str = "java.lang.IllegalArgumentException";

/// `TestRangeFacetCounts`.
pub fn class_def() -> Arc<Class> {
    facet_class(
        "org.apache.lucene.facet.range.TestRangeFacetCounts",
        vec![
            test("testBasicLong", test_basic_long),
            test("testBasicLongMultiValued", test_basic_long_multi_valued),
            test(
                "testBasicLongMultiValuedMixedSegmentTypes",
                test_basic_long_multi_valued_mixed_segment_types,
            ),
            test("testLongGetAllDims", test_long_get_all_dims),
            test("testUselessRange", test_useless_range),
            test("testLongMinMax", test_long_min_max),
            test("testOverlappedEndStart", test_overlapped_end_start),
            test(
                "testEmptyRangesSingleValued",
                test_empty_ranges_single_valued,
            ),
            test("testEmptyRangesMultiValued", test_empty_ranges_multi_valued),
            test(
                "testMixedRangeAndNonRangeTaxonomy",
                test_mixed_range_and_non_range_taxonomy,
            ),
            test("testBasicDouble", test_basic_double),
            test("testBasicDoubleMultiValued", test_basic_double_multi_valued),
            test(
                "testBasicDoubleMultiValuedMixedSegmentTypes",
                test_basic_double_multi_valued_mixed_segment_types,
            ),
            test(
                "testRandomLongsSingleValued",
                test_random_longs_single_valued,
            ),
            test("testRandomLongsMultiValued", test_random_longs_multi_valued),
            test(
                "testRandomDoublesSingleValued",
                test_random_doubles_single_valued,
            ),
            test(
                "testRandomDoublesMultiValued",
                test_random_doubles_multi_valued,
            ),
            test("testMissingValues", test_missing_values),
            test(
                "testMissingValuesMultiValued",
                test_missing_values_multi_valued,
            ),
            test(
                "testCustomDoubleValuesSource",
                test_custom_double_values_source,
            ),
            test("testLongRangeEquals", test_long_range_equals),
            test("testDoubleRangeEquals", test_double_range_equals),
        ],
    )
}

fn writer(d: &TestDirectory) -> Result<RandomIndexWriter> {
    RandomIndexWriter::new_with_mock_analyzer(&*LuceneTestCase::random(), d.as_directory())
}

fn match_all(s: &IndexSearcher) -> Result<FacetsCollector> {
    s.search_with_manager(
        Arc::new(MatchAllDocsQuery::instance()),
        &FacetsCollectorManager::new(),
    )
}

fn lr(label: &str, min: i64, min_incl: bool, max: i64, max_incl: bool) -> Result<LongRange> {
    LongRange::new(label, min, min_incl, max, max_incl)
}

fn dr(label: &str, min: f64, min_incl: bool, max: f64, max_incl: bool) -> Result<DoubleRange> {
    DoubleRange::new(label, min, min_incl, max, max_incl)
}

/// The five long ranges most tests facet on; `over_1000_max_incl` is the
/// `maxInclusive` of `"over 1000"`.
fn basic_long_ranges(over_1000_max_incl: bool) -> Result<Vec<LongRange>> {
    Ok(vec![
        lr("less than 10", 0, true, 10, false)?,
        lr("less than or equal to 10", 0, true, 10, true)?,
        lr("over 90", 90, false, 100, false)?,
        lr("90 or above", 90, true, 100, false)?,
        lr("over 1000", 1000, false, i64::MAX, over_1000_max_incl)?,
    ])
}

/// The five double ranges of the basic double tests.
fn basic_double_ranges() -> Result<Vec<DoubleRange>> {
    Ok(vec![
        dr("less than 10", 0.0, true, 10.0, false)?,
        dr("less than or equal to 10", 0.0, true, 10.0, true)?,
        dr("over 90", 90.0, false, 100.0, false)?,
        dr("90 or above", 90.0, true, 100.0, false)?,
        dr("over 1000", 1000.0, false, f64::INFINITY, false)?,
    ])
}

/// `facets.getAllChildren(dim).toString()`.
fn all_children(facets: &dyn Facets, dim: &str) -> Result<String> {
    Ok(facets.get_all_children(dim, &[])?.unwrap().to_string())
}

/// `facets.getTopChildren(topN, dim).toString()`.
fn top_children(facets: &dyn Facets, top_n: i32, dim: &str) -> Result<String> {
    Ok(facets
        .get_top_children(top_n, dim, &[])?
        .unwrap()
        .to_string())
}

/// The documents of `testBasicLong`, `testLongGetAllDims` and
/// `testOverlappedEndStart`: the `NumericDocValuesField` the Java test
/// mutates between `addDocument` calls is rebuilt per document.
fn index_0_to_99_and_max(w: &RandomIndexWriter) -> Result<()> {
    for l in 0..100i64 {
        let mut doc = Document::new();
        doc.add(NumericDocValuesField::new("field", l));
        w.add_document(&doc)?;
    }

    // Also add Long.MAX_VALUE
    let mut doc = Document::new();
    doc.add(NumericDocValuesField::new("field", i64::MAX));
    w.add_document(&doc)?;
    Ok(())
}

fn test_basic_long() -> Result<()> {
    let d = LuceneTestCase::new_directory();
    let w = writer(&d)?;
    index_0_to_99_and_max(&w)?;

    let r = w.get_reader()?;
    w.close()?;

    let s = LuceneTestCase::new_searcher(Arc::clone(&r));
    let fc = match_all(&s)?;

    let facets = LongRangeFacetCounts::new("field", &fc, basic_long_ranges(true)?)?;

    Assert::assert_equals(
        "dim=field path=[] value=22 childCount=5\n  less than 10 (10)\n  less than or equal to 10 (11)\n  over 90 (9)\n  90 or above (10)\n  over 1000 (1)\n",
        all_children(&facets, "field")?.as_str(),
    );
    Assert::assert_equals(
        "dim=field path=[] value=22 childCount=5\n  less than or equal to 10 (11)\n  90 or above (10)\n  less than 10 (10)\n  over 90 (9)\n",
        top_children(&facets, 4, "field")?.as_str(),
    );

    // test getTopChildren(0, dim)
    expect_throws(IAE, || facets.get_top_children(0, "field", &[]));

    r.close()?;
    d.close()
}

fn test_basic_long_multi_valued() -> Result<()> {
    let d = LuceneTestCase::new_directory();
    let w = writer(&d)?;
    // just index the same value twice each time and make sure we don't double count
    for l in (0..100i64).chain(std::iter::once(i64::MAX)) {
        let mut doc = Document::new();
        doc.add(SortedNumericDocValuesField::new("field", l));
        doc.add(SortedNumericDocValuesField::new("field", l));
        w.add_document(&doc)?;
    }

    let r = w.get_reader()?;
    w.close()?;

    let s = LuceneTestCase::new_searcher(Arc::clone(&r));
    let fc = match_all(&s)?;

    let facets = LongRangeFacetCounts::new("field", &fc, basic_long_ranges(true)?)?;

    Assert::assert_equals(
        "dim=field path=[] value=22 childCount=5\n  less than 10 (10)\n  less than or equal to 10 (11)\n  over 90 (9)\n  90 or above (10)\n  over 1000 (1)\n",
        all_children(&facets, "field")?.as_str(),
    );
    Assert::assert_equals(
        "dim=field path=[] value=22 childCount=5\n  less than or equal to 10 (11)\n  90 or above (10)\n  less than 10 (10)\n  over 90 (9)\n",
        top_children(&facets, 4, "field")?.as_str(),
    );

    // test getTopChildren(0, dim)
    expect_throws(IAE, || facets.get_top_children(0, "field", &[]));

    r.close()?;
    d.close()
}

/// The documents of the `*MultiValuedMixedSegmentTypes` tests: two
/// segments (50 in each), the first mixing single- and multi-value cases,
/// the second all single values; `encode` maps `l` to the indexed long.
fn index_mixed_segment_types(w: &RandomIndexWriter, encode: impl Fn(i32) -> i64) -> Result<()> {
    for l in 0..100 {
        let value = encode(l);
        let mut doc = Document::new();
        doc.add(SortedNumericDocValuesField::new("field", value));
        if l == 0 || (l < 50 && LuceneTestCase::random().next_boolean()) {
            doc.add(SortedNumericDocValuesField::new("field", value));
        }
        w.add_document(&doc)?;
        if l == 50 {
            w.commit()?;
        }
    }
    Ok(())
}

fn test_basic_long_multi_valued_mixed_segment_types() -> Result<()> {
    let d = LuceneTestCase::new_directory();
    let w = writer(&d)?;
    index_mixed_segment_types(&w, i64::from)?;

    let r = w.get_reader()?;
    w.close()?;

    let s = LuceneTestCase::new_searcher(Arc::clone(&r));
    let fc = match_all(&s)?;

    let facets = LongRangeFacetCounts::new("field", &fc, basic_long_ranges(true)?)?;

    Assert::assert_equals(
        "dim=field path=[] value=21 childCount=5\n  less than 10 (10)\n  less than or equal to 10 (11)\n  over 90 (9)\n  90 or above (10)\n  over 1000 (0)\n",
        all_children(&facets, "field")?.as_str(),
    );
    Assert::assert_equals(
        "dim=field path=[] value=21 childCount=4\n  less than or equal to 10 (11)\n  90 or above (10)\n  less than 10 (10)\n  over 90 (9)\n",
        top_children(&facets, 4, "field")?.as_str(),
    );

    r.close()?;
    d.close()
}

fn test_long_get_all_dims() -> Result<()> {
    let d = LuceneTestCase::new_directory();
    let w = writer(&d)?;
    index_0_to_99_and_max(&w)?;

    let r = w.get_reader()?;
    w.close()?;

    let s = LuceneTestCase::new_searcher(Arc::clone(&r));
    let fc = match_all(&s)?;

    let facets = LongRangeFacetCounts::new("field", &fc, basic_long_ranges(true)?)?;

    let result = facets.get_all_dims(10)?;
    Assert::assert_equals(1, result.len());
    Assert::assert_equals(
        "dim=field path=[] value=22 childCount=5\n  less than or equal to 10 (11)\n  90 or above (10)\n  less than 10 (10)\n  over 90 (9)\n  over 1000 (1)\n",
        result[0].to_string().as_str(),
    );

    // test getAllDims(1)
    let result = facets.get_all_dims(1)?;
    Assert::assert_equals(1, result.len());
    Assert::assert_equals(
        "dim=field path=[] value=22 childCount=5\n  less than or equal to 10 (11)\n",
        result[0].to_string().as_str(),
    );

    // test default implementation of getTopDims
    let top_n_dims_result = facets.get_top_dims(1, 1)?;
    assert_equals(&result, &top_n_dims_result);

    // test getTopDims(0, 1)
    let top_n_dims_result = facets.get_top_dims(0, 1)?;
    Assert::assert_equals(0, top_n_dims_result.len());

    // test getAllDims(0)
    expect_throws(IAE, || facets.get_all_dims(0));

    r.close()?;
    d.close()
}

fn test_useless_range() -> Result<()> {
    expect_throws(IAE, || LongRange::new("useless", 7, true, 6, true));
    expect_throws(IAE, || LongRange::new("useless", 7, true, 7, false));
    expect_throws(IAE, || DoubleRange::new("useless", 7.0, true, 6.0, true));
    expect_throws(IAE, || DoubleRange::new("useless", 7.0, true, 7.0, false));
    Ok(())
}

fn test_long_min_max() -> Result<()> {
    let d = LuceneTestCase::new_directory();
    let w = writer(&d)?;
    for v in [i64::MIN, 0, i64::MAX] {
        let mut doc = Document::new();
        doc.add(NumericDocValuesField::new("field", v));
        w.add_document(&doc)?;
    }

    let r = w.get_reader()?;
    w.close()?;

    let s = LuceneTestCase::new_searcher(Arc::clone(&r));
    let fc = match_all(&s)?;

    let facets = LongRangeFacetCounts::new(
        "field",
        &fc,
        vec![
            lr("min", i64::MIN, true, i64::MIN, true)?,
            lr("max", i64::MAX, true, i64::MAX, true)?,
            lr("all0", i64::MIN, true, i64::MAX, true)?,
            lr("all1", i64::MIN, false, i64::MAX, true)?,
            lr("all2", i64::MIN, true, i64::MAX, false)?,
            lr("all3", i64::MIN, false, i64::MAX, false)?,
        ],
    )?;

    Assert::assert_equals(
        "dim=field path=[] value=3 childCount=6\n  min (1)\n  max (1)\n  all0 (3)\n  all1 (2)\n  all2 (2)\n  all3 (1)\n",
        all_children(&facets, "field")?.as_str(),
    );
    Assert::assert_equals(
        "dim=field path=[] value=3 childCount=6\n  all0 (3)\n  all1 (2)\n  all2 (2)\n  all3 (1)\n  max (1)\n",
        top_children(&facets, 5, "field")?.as_str(),
    );

    r.close()?;
    d.close()
}

fn test_overlapped_end_start() -> Result<()> {
    let d = LuceneTestCase::new_directory();
    let w = writer(&d)?;
    index_0_to_99_and_max(&w)?;

    let r = w.get_reader()?;
    w.close()?;

    let s = LuceneTestCase::new_searcher(Arc::clone(&r));
    let fc = match_all(&s)?;

    let facets = LongRangeFacetCounts::new(
        "field",
        &fc,
        vec![
            lr("0-10", 0, true, 10, true)?,
            lr("10-20", 10, true, 20, true)?,
            lr("20-30", 20, true, 30, true)?,
            lr("30-40", 30, true, 40, true)?,
        ],
    )?;

    Assert::assert_equals(
        "dim=field path=[] value=41 childCount=4\n  0-10 (11)\n  10-20 (11)\n  20-30 (11)\n  30-40 (11)\n",
        all_children(&facets, "field")?.as_str(),
    );
    Assert::assert_equals(
        "dim=field path=[] value=41 childCount=4\n  0-10 (11)\n  10-20 (11)\n  20-30 (11)\n",
        top_children(&facets, 3, "field")?.as_str(),
    );
    r.close()?;
    d.close()
}

fn test_empty_ranges_single_valued() -> Result<()> {
    let d = LuceneTestCase::new_directory();
    let w = writer(&d)?;
    for l in 0..100i64 {
        let mut doc = Document::new();
        doc.add(NumericDocValuesField::new("field", l));
        w.add_document(&doc)?;
    }

    let r = w.get_reader()?;
    w.close()?;

    let s = LuceneTestCase::new_searcher(Arc::clone(&r));
    let fc = match_all(&s)?;

    let facets = LongRangeFacetCounts::new("field", &fc, Vec::new())?;

    Assert::assert_equals(
        "dim=field path=[] value=0 childCount=0\n",
        all_children(&facets, "field")?.as_str(),
    );
    Assert::assert_equals(
        "dim=field path=[] value=0 childCount=0\n",
        top_children(&facets, 1, "field")?.as_str(),
    );

    r.close()?;
    d.close()
}

fn test_empty_ranges_multi_valued() -> Result<()> {
    let d = LuceneTestCase::new_directory();
    let w = writer(&d)?;
    for l in 0..100i64 {
        let mut doc = Document::new();
        doc.add(SortedNumericDocValuesField::new("field", l));
        doc.add(SortedNumericDocValuesField::new("field", l));
        w.add_document(&doc)?;
    }

    let r = w.get_reader()?;
    w.close()?;

    let s = LuceneTestCase::new_searcher(Arc::clone(&r));
    let fc = match_all(&s)?;

    let facets = LongRangeFacetCounts::new("field", &fc, Vec::new())?;

    Assert::assert_equals(
        "dim=field path=[] value=0 childCount=0\n",
        all_children(&facets, "field")?.as_str(),
    );
    Assert::assert_equals(
        "dim=field path=[] value=0 childCount=0\n",
        top_children(&facets, 1, "field")?.as_str(),
    );

    r.close()?;
    d.close()
}

/// The anonymous `DrillSideways` subclass of
/// `testMixedRangeAndNonRangeTaxonomy`.
struct MixedRangeOverrides {
    config: Arc<FacetsConfig>,
}

impl DrillSidewaysOverrides for MixedRangeOverrides {
    fn build_facets_result(
        &self,
        drill_sideways: &DrillSideways,
        drill_downs: Option<&FacetsCollector>,
        drill_sideways_collectors: Option<&[FacetsCollector]>,
        drill_sideways_dims: Option<&[String]>,
    ) -> Result<Option<Arc<dyn Facets>>> {
        let mut dim_fc = drill_downs;
        let mut field_fc = drill_downs;
        if let Some(collectors) = drill_sideways_collectors {
            let dims = drill_sideways_dims.expect("dims accompany the collectors");
            for (i, collector) in collectors.iter().enumerate() {
                let dim = &dims[i];
                if dim == "field" {
                    field_fc = Some(collector);
                } else {
                    dim_fc = Some(collector);
                }
            }
        }

        let by_dim: Vec<(String, Arc<dyn Facets>)> = vec![
            (
                "field".to_string(),
                Arc::new(LongRangeFacetCounts::new(
                    "field",
                    field_fc.expect("java.lang.NullPointerException"),
                    basic_long_ranges(false)?,
                )?),
            ),
            (
                "dim".to_string(),
                get_taxonomy_facet_counts(
                    drill_sideways.taxo_reader.as_ref().expect("taxoReader"),
                    &self.config,
                    dim_fc.expect("java.lang.NullPointerException"),
                )?,
            ),
        ];
        // The facet counts hold non-`Send` readers, shared as Java shares them.
        #[allow(clippy::arc_with_non_send_sync)]
        Ok(Some(Arc::new(MultiFacets::with_default(by_dim, None))))
    }

    fn score_sub_docs_at_once(&self) -> bool {
        LuceneTestCase::random().next_boolean()
    }
}

/// Tests single request that mixes Range and non-Range faceting, with
/// DrillSideways and taxonomy.
fn test_mixed_range_and_non_range_taxonomy() -> Result<()> {
    let d = LuceneTestCase::new_directory();
    let w = writer(&d)?;
    let td = LuceneTestCase::new_directory();
    let tw = Arc::new(DirectoryTaxonomyWriter::with_open_mode(
        td.as_directory(),
        OpenMode::CREATE,
    )?);

    let config = Arc::new(FacetsConfig::new());

    for l in 0..100i64 {
        let mut doc = Document::new();
        // For computing range facet counts:
        doc.add(NumericDocValuesField::new("field", l));
        // For drill down by numeric range:
        doc.add(LongPoint::new("field", &[l])?);

        if (l & 3) == 0 {
            doc.add(FacetField::new("dim", &["a"])?);
        } else {
            doc.add(FacetField::new("dim", &["b"])?);
        }
        w.add_document(&config.build_with_taxonomy(Some(&*tw), &doc)?)?;
    }

    let r = w.get_reader()?;

    let tr: Arc<dyn TaxonomyReader> =
        Arc::new(DirectoryTaxonomyReader::from_writer(Arc::clone(&tw))?);

    let mut s = LuceneTestCase::new_searcher_concurrency(
        Arc::clone(&r),
        false,
        false,
        Concurrency::InterSegment,
    );
    // DrillSideways requires the entire range of docs to be scored at once, so it doesn't support
    // timeouts whose implementation scores one window of doc IDs at a time.
    s.set_timeout(None);

    if *VERBOSE {
        println!("TEST: searcher={s:?}");
    }

    let ds = DrillSideways::with_overrides(
        Arc::new(s),
        Arc::clone(&config),
        Some(Arc::clone(&tr)),
        None,
        None,
        Arc::new(MixedRangeOverrides {
            config: Arc::clone(&config),
        }),
    );

    // First search, no drill downs:
    let ddq = DrillDownQuery::new(Arc::clone(&config));
    let dsr = ds.search_after(None, ddq, 10)?;

    Assert::assert_equals(100, dsr.hits.as_ref().unwrap().total_hits().value());
    let facets = dsr.facets.unwrap();
    Assert::assert_equals(
        "dim=dim path=[] value=100 childCount=2\n  b (75)\n  a (25)\n",
        top_children(&*facets, 10, "dim")?.as_str(),
    );
    Assert::assert_equals(
        "dim=field path=[] value=21 childCount=4\n  less than or equal to 10 (11)\n  90 or above (10)\n  less than 10 (10)\n  over 90 (9)\n",
        top_children(&*facets, 10, "field")?.as_str(),
    );

    // Second search, drill down on dim=b:
    let mut ddq = DrillDownQuery::new(Arc::clone(&config));
    ddq.add("dim", &["b"])?;
    let dsr = ds.search_after(None, ddq, 10)?;

    Assert::assert_equals(75, dsr.hits.as_ref().unwrap().total_hits().value());
    let facets = dsr.facets.unwrap();
    Assert::assert_equals(
        "dim=dim path=[] value=100 childCount=2\n  b (75)\n  a (25)\n",
        top_children(&*facets, 10, "dim")?.as_str(),
    );
    Assert::assert_equals(
        "dim=field path=[] value=16 childCount=4\n  90 or above (8)\n  less than or equal to 10 (8)\n  less than 10 (7)\n  over 90 (7)\n",
        top_children(&*facets, 10, "field")?.as_str(),
    );

    // Third search, drill down on "less than or equal to 10":
    let mut ddq = DrillDownQuery::new(Arc::clone(&config));
    ddq.add_query("field", LongPoint::new_range_query("field", &[0], &[10])?)?;
    let dsr = ds.search_after(None, ddq, 10)?;

    Assert::assert_equals(11, dsr.hits.as_ref().unwrap().total_hits().value());
    let facets = dsr.facets.unwrap();
    Assert::assert_equals(
        "dim=dim path=[] value=11 childCount=2\n  b (8)\n  a (3)\n",
        top_children(&*facets, 10, "dim")?.as_str(),
    );
    Assert::assert_equals(
        "dim=field path=[] value=21 childCount=4\n  less than or equal to 10 (11)\n  90 or above (10)\n  less than 10 (10)\n  over 90 (9)\n",
        top_children(&*facets, 10, "field")?.as_str(),
    );
    w.close()?;
    tw.close()?;
    tr.close()?;
    td.close()?;
    r.close()?;
    d.close()
}

fn test_basic_double() -> Result<()> {
    let d = LuceneTestCase::new_directory();
    let w = writer(&d)?;
    for i in 0..100 {
        let mut doc = Document::new();
        doc.add(DoubleDocValuesField::new("field", f64::from(i)));
        w.add_document(&doc)?;
    }

    let r = w.get_reader()?;

    let s = LuceneTestCase::new_searcher(Arc::clone(&r));
    let fc = match_all(&s)?;
    let facets = DoubleRangeFacetCounts::new("field", &fc, basic_double_ranges()?)?;

    Assert::assert_equals(
        "dim=field path=[] value=21 childCount=5\n  less than 10 (10)\n  less than or equal to 10 (11)\n  over 90 (9)\n  90 or above (10)\n  over 1000 (0)\n",
        all_children(&facets, "field")?.as_str(),
    );
    Assert::assert_equals(
        "dim=field path=[] value=21 childCount=4\n  less than or equal to 10 (11)\n  90 or above (10)\n  less than 10 (10)\n  over 90 (9)\n",
        top_children(&facets, 4, "field")?.as_str(),
    );
    w.close()?;
    r.close()?;
    d.close()
}

fn test_basic_double_multi_valued() -> Result<()> {
    let d = LuceneTestCase::new_directory();
    let w = writer(&d)?;
    // index the same value twice and make sure we don't double count
    for i in 0..100 {
        let mut doc = Document::new();
        let v = NumericUtils::double_to_sortable_long(f64::from(i));
        doc.add(SortedNumericDocValuesField::new("field", v));
        doc.add(SortedNumericDocValuesField::new("field", v));
        w.add_document(&doc)?;
    }

    let r = w.get_reader()?;

    let s = LuceneTestCase::new_searcher(Arc::clone(&r));
    let fc = match_all(&s)?;
    let facets = DoubleRangeFacetCounts::new("field", &fc, basic_double_ranges()?)?;

    facets.get_top_children(4, "field", &[])?;
    Assert::assert_equals(
        "dim=field path=[] value=21 childCount=5\n  less than 10 (10)\n  less than or equal to 10 (11)\n  over 90 (9)\n  90 or above (10)\n  over 1000 (0)\n",
        all_children(&facets, "field")?.as_str(),
    );
    Assert::assert_equals(
        "dim=field path=[] value=21 childCount=4\n  less than or equal to 10 (11)\n  90 or above (10)\n  less than 10 (10)\n  over 90 (9)\n",
        top_children(&facets, 4, "field")?.as_str(),
    );
    facets.get_top_children(4, "field", &[])?;
    w.close()?;
    r.close()?;
    d.close()
}

fn test_basic_double_multi_valued_mixed_segment_types() -> Result<()> {
    let d = LuceneTestCase::new_directory();
    let w = writer(&d)?;
    index_mixed_segment_types(&w, |l| NumericUtils::double_to_sortable_long(f64::from(l)))?;

    let r = w.get_reader()?;
    w.close()?;

    let s = LuceneTestCase::new_searcher(Arc::clone(&r));
    let fc = match_all(&s)?;

    let facets = DoubleRangeFacetCounts::new("field", &fc, basic_double_ranges()?)?;

    Assert::assert_equals(
        "dim=field path=[] value=21 childCount=5\n  less than 10 (10)\n  less than or equal to 10 (11)\n  over 90 (9)\n  90 or above (10)\n  over 1000 (0)\n",
        all_children(&facets, "field")?.as_str(),
    );
    Assert::assert_equals(
        "dim=field path=[] value=21 childCount=4\n  less than or equal to 10 (11)\n  90 or above (10)\n  less than 10 (10)\n  over 90 (9)\n",
        top_children(&facets, 4, "field")?.as_str(),
    );

    r.close()?;
    d.close()
}

/// The random long ranges of `testRandomLongs*`, with their expected counts
/// computed by `accept(range bounds, inclusive flags) -> per-doc matches`.
struct RandomLongRanges {
    ranges: Vec<LongRange>,
    expected_counts: Vec<i32>,
    expected_top_n_children_counts: Vec<i32>,
    min_accepted_value: i64,
    max_accepted_value: i64,
}

/// The range-building loop shared by `testRandomLongsSingleValued` and
/// `testRandomLongsMultiValued`; `values[i]` holds the values of doc `i`
/// and `first_match_only` stops at the first matching value of a doc.
fn random_long_ranges(values: &[Vec<i64>], first_match_only: bool) -> Result<RandomLongRanges> {
    let random = LuceneTestCase::random();
    let num_range = TestUtil::next_int(&*random, 1, 100) as usize;
    let mut ranges: Vec<LongRange> = Vec::with_capacity(num_range);
    let mut expected_counts = vec![0i32; num_range];
    let mut expected_top_n_children_counts = vec![0i32; num_range];
    let mut min_accepted_value = i64::MAX;
    let mut max_accepted_value = i64::MIN;
    for range_id in 0..num_range {
        let mut min = if range_id > 0 && random.next_int_bound(10) == 7 {
            // Use an existing boundary:
            let prev_range = &ranges[random.next_int_bound(range_id as i32) as usize];
            if random.next_boolean() {
                prev_range.min
            } else {
                prev_range.max
            }
        } else {
            random.next_long()
        };
        let mut max = if range_id > 0 && random.next_int_bound(10) == 7 {
            // Use an existing boundary:
            let prev_range = &ranges[random.next_int_bound(range_id as i32) as usize];
            if random.next_boolean() {
                prev_range.min
            } else {
                prev_range.max
            }
        } else {
            random.next_long()
        };

        if min > max {
            std::mem::swap(&mut min, &mut max);
        }
        // NOTE: max - min >= 0 is here to handle the common overflow case!
        let diff = max.wrapping_sub(min);
        let (min_incl, max_incl) = if (0..2).contains(&diff) {
            // If max == min or max == min+1, we always do inclusive, else we might pass an empty
            // range and hit exc from LongRange's ctor:
            (true, true)
        } else {
            (random.next_boolean(), random.next_boolean())
        };
        ranges.push(LongRange::new(
            &format!("r{range_id}"),
            min,
            min_incl,
            max,
            max_incl,
        )?);
        if *VERBOSE {
            println!("  range {range_id}: {}", ranges[range_id]);
        }

        // Do "slow but hopefully correct" computation of
        // expected count:
        for doc_values in values {
            for &v in doc_values {
                let mut accept = true;
                if min_incl {
                    accept &= v >= min;
                } else {
                    accept &= v > min;
                }
                if max_incl {
                    accept &= v <= max;
                } else {
                    accept &= v < max;
                }
                if accept {
                    expected_counts[range_id] += 1;
                    expected_top_n_children_counts[range_id] += 1;
                    min_accepted_value = min_accepted_value.min(v);
                    max_accepted_value = max_accepted_value.max(v);
                    if first_match_only {
                        break; // ensure each doc can contribute at most 1 count to each range
                    }
                }
            }
        }
    }
    Ok(RandomLongRanges {
        ranges,
        expected_counts,
        expected_top_n_children_counts,
        min_accepted_value,
        max_accepted_value,
    })
}

/// The assertions shared by the random tests: `result` / `top_n_result`
/// against the expected counts, then a drill-down per range whose query is
/// `drill_down(rangeID)`.
fn check_random_results(
    s: &IndexSearcher,
    config: &Arc<FacetsConfig>,
    facets: &dyn Facets,
    expected_counts: &[i32],
    mut expected_top_n_children_counts: Vec<i32>,
    mut drill_down: impl FnMut(usize) -> Result<Arc<dyn Query>>,
) -> Result<()> {
    let num_range = expected_counts.len();
    let result = facets.get_all_children("field", &[])?.unwrap();
    Assert::assert_equals(num_range, result.label_values.len());
    let top_n_result = facets
        .get_top_children(num_range as i32, "field", &[])?
        .unwrap();
    expected_top_n_children_counts.sort_unstable();

    for range_id in 0..num_range {
        if *VERBOSE {
            println!(
                "  range {range_id} expectedCount={}",
                expected_counts[range_id]
            );
        }
        let sub_node = &result.label_values[range_id];
        Assert::assert_equals(format!("r{range_id}").as_str(), sub_node.label.as_str());
        Assert::assert_equals(expected_counts[range_id], number_int_value(&sub_node.value));
        // test topNChildren and assert topNResults are sorted by count
        if range_id < top_n_result.label_values.len() {
            let top_n_sub_node = &top_n_result.label_values[range_id];
            Assert::assert_equals(
                expected_top_n_children_counts[num_range - range_id - 1],
                number_int_value(&top_n_sub_node.value),
            );
        }

        // Test drill-down:
        let mut ddq = DrillDownQuery::new(Arc::clone(config));
        ddq.add_query("field", drill_down(range_id)?)?;
        Assert::assert_equals(expected_counts[range_id], s.count(Arc::new(ddq))?);
    }
    Ok(())
}

fn test_random_longs_single_valued() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let w = writer(&dir)?;

    let num_docs = LuceneTestCase::at_least(1000) as usize;
    if *VERBOSE {
        println!("TEST: numDocs={num_docs}");
    }
    let mut values: Vec<Vec<i64>> = Vec::with_capacity(num_docs);
    let mut min_value = i64::MAX;
    let mut max_value = i64::MIN;
    for _ in 0..num_docs {
        let mut doc = Document::new();
        let v = LuceneTestCase::random().next_long();
        values.push(vec![v]);
        doc.add(NumericDocValuesField::new("field", v));
        doc.add(LongPoint::new("field", &[v])?);
        w.add_document(&doc)?;
        min_value = min_value.min(v);
        max_value = max_value.max(v);
    }
    let r = w.get_reader()?;

    let s = LuceneTestCase::new_searcher_wrap(Arc::clone(&r), false);
    let config = Arc::new(FacetsConfig::new());

    let num_iters = LuceneTestCase::at_least(10);
    for iter in 0..num_iters {
        if *VERBOSE {
            println!("TEST: iter={iter}");
        }
        let rr = random_long_ranges(&values, false)?;
        let random = LuceneTestCase::random();

        let sfc = match_all(&s)?;
        let fast_match_query: Option<Arc<dyn Query>> = if random.next_boolean() {
            if random.next_boolean() {
                Some(LongPoint::new_range_query(
                    "field",
                    &[min_value],
                    &[max_value],
                )?)
            } else {
                Some(LongPoint::new_range_query(
                    "field",
                    &[rr.min_accepted_value],
                    &[rr.max_accepted_value],
                )?)
            }
        } else {
            None
        };

        if random.next_boolean() {
            let vs = long_values_source::from_long_field("field");
            let mvs = <dyn MultiLongValuesSource>::from_long_field("field");
            let facets = if random.next_boolean() {
                LongRangeFacetCounts::with_value_source_and_fast_match(
                    "field",
                    Some(Arc::clone(&vs)),
                    &sfc,
                    fast_match_query.clone(),
                    rr.ranges.clone(),
                )?
            } else if random.next_boolean() {
                LongRangeFacetCounts::with_multi_values_source_and_fast_match(
                    "field",
                    Some(Arc::clone(&mvs)),
                    &sfc,
                    fast_match_query.clone(),
                    rr.ranges.clone(),
                )?
            } else {
                LongRangeFacetCounts::with_multi_values_source_and_fast_match(
                    "field",
                    Some(<dyn MultiLongValuesSource>::from_single_valued(Arc::clone(
                        &vs,
                    ))),
                    &sfc,
                    fast_match_query.clone(),
                    rr.ranges.clone(),
                )?
            };
            check_random_results(
                &s,
                &config,
                &facets,
                &rr.expected_counts,
                rr.expected_top_n_children_counts.clone(),
                |range_id| {
                    let range = &rr.ranges[range_id];
                    if random.next_boolean() {
                        LongPoint::new_range_query("field", &[range.min], &[range.max])
                    } else if random.next_boolean() {
                        Ok(range.get_query_multi(fast_match_query.clone(), Arc::clone(&mvs)))
                    } else {
                        Ok(range.get_query(fast_match_query.clone(), Arc::clone(&vs)))
                    }
                },
            )?;
        } else {
            let vs = <dyn MultiLongValuesSource>::from_long_field("field");
            let facets = LongRangeFacetCounts::with_multi_values_source_and_fast_match(
                "field",
                Some(Arc::clone(&vs)),
                &sfc,
                fast_match_query.clone(),
                rr.ranges.clone(),
            )?;
            check_random_results(
                &s,
                &config,
                &facets,
                &rr.expected_counts,
                rr.expected_top_n_children_counts.clone(),
                |range_id| {
                    let range = &rr.ranges[range_id];
                    if random.next_boolean() {
                        LongPoint::new_range_query("field", &[range.min], &[range.max])
                    } else {
                        Ok(range.get_query_multi(fast_match_query.clone(), Arc::clone(&vs)))
                    }
                },
            )?;
        }
    }

    w.close()?;
    r.close()?;
    dir.close()
}

fn test_random_longs_multi_valued() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let w = writer(&dir)?;

    let num_docs = LuceneTestCase::at_least(1000) as usize;
    if *VERBOSE {
        println!("TEST: numDocs={num_docs}");
    }
    let mut values: Vec<Vec<i64>> = Vec::with_capacity(num_docs);
    let mut min_value = i64::MAX;
    let mut max_value = i64::MIN;
    for _ in 0..num_docs {
        let random = LuceneTestCase::random();
        let mut doc = Document::new();
        let mut num_vals = RandomNumbers::random_int_between(&*random, 1, 50);
        if random.next_int_bound(10) == 0 {
            num_vals = 1; // make sure we have ample testing of single-value cases
        }
        let mut doc_values = Vec::with_capacity(num_vals as usize);
        for _ in 0..num_vals {
            let v = random.next_long();
            doc_values.push(v);
            doc.add(SortedNumericDocValuesField::new("field", v));
            doc.add(LongPoint::new("field", &[v])?);
            min_value = min_value.min(v);
            max_value = max_value.max(v);
        }
        values.push(doc_values);
        w.add_document(&doc)?;
    }
    let r = w.get_reader()?;

    let s = LuceneTestCase::new_searcher_wrap(Arc::clone(&r), false);
    let config = Arc::new(FacetsConfig::new());

    let num_iters = LuceneTestCase::at_least(10);
    for iter in 0..num_iters {
        if *VERBOSE {
            println!("TEST: iter={iter}");
        }
        let rr = random_long_ranges(&values, true)?;
        let random = LuceneTestCase::random();

        let sfc = match_all(&s)?;
        let fast_match_query: Option<Arc<dyn Query>> = if random.next_boolean() {
            if random.next_boolean() {
                Some(LongPoint::new_range_query(
                    "field",
                    &[min_value],
                    &[max_value],
                )?)
            } else {
                Some(LongPoint::new_range_query(
                    "field",
                    &[rr.min_accepted_value],
                    &[rr.max_accepted_value],
                )?)
            }
        } else {
            None
        };
        let facets = if random.next_boolean() {
            LongRangeFacetCounts::with_multi_values_source_and_fast_match(
                "field",
                Some(<dyn MultiLongValuesSource>::from_long_field("field")),
                &sfc,
                fast_match_query.clone(),
                rr.ranges.clone(),
            )?
        } else {
            LongRangeFacetCounts::with_multi_values_source_and_fast_match(
                "field",
                None,
                &sfc,
                fast_match_query.clone(),
                rr.ranges.clone(),
            )?
        };
        check_random_results(
            &s,
            &config,
            &facets,
            &rr.expected_counts,
            rr.expected_top_n_children_counts.clone(),
            |range_id| {
                let range = &rr.ranges[range_id];
                if random.next_boolean() {
                    LongPoint::new_range_query("field", &[range.min], &[range.max])
                } else {
                    Ok(SortedNumericDocValuesField::new_slow_range_query(
                        "field", range.min, range.max,
                    ))
                }
            },
        )?;
    }

    w.close()?;
    r.close()?;
    dir.close()
}

/// The random double ranges of `testRandomDoubles*` and their expected
/// counts.
struct RandomDoubleRanges {
    ranges: Vec<DoubleRange>,
    expected_counts: Vec<i32>,
    expected_top_n_children_counts: Vec<i32>,
    min_accepted_value: f64,
    max_accepted_value: f64,
}

/// The range-building loop shared by `testRandomDoublesSingleValued` and
/// `testRandomDoublesMultiValued`.
fn random_double_ranges(values: &[Vec<f64>], first_match_only: bool) -> Result<RandomDoubleRanges> {
    let random = LuceneTestCase::random();
    let num_range = TestUtil::next_int(&*random, 1, 5) as usize;
    let mut ranges: Vec<DoubleRange> = Vec::with_capacity(num_range);
    let mut expected_counts = vec![0i32; num_range];
    let mut expected_top_n_children_counts = vec![0i32; num_range];
    let mut min_accepted_value = f64::INFINITY;
    let mut max_accepted_value = f64::NEG_INFINITY;
    for range_id in 0..num_range {
        let mut min = if range_id > 0 && random.next_int_bound(10) == 7 {
            // Use an existing boundary:
            let prev_range = &ranges[random.next_int_bound(range_id as i32) as usize];
            if random.next_boolean() {
                prev_range.min
            } else {
                prev_range.max
            }
        } else {
            random.next_double()
        };
        let mut max = if range_id > 0 && random.next_int_bound(10) == 7 {
            // Use an existing boundary:
            let prev_range = &ranges[random.next_int_bound(range_id as i32) as usize];
            if random.next_boolean() {
                prev_range.min
            } else {
                prev_range.max
            }
        } else {
            random.next_double()
        };

        if min > max {
            std::mem::swap(&mut min, &mut max);
        }

        let min_as_long = NumericUtils::double_to_sortable_long(min);
        let max_as_long = NumericUtils::double_to_sortable_long(max);
        // NOTE: maxAsLong - minAsLong >= 0 is here to handle the common overflow case!
        let diff = max_as_long.wrapping_sub(min_as_long);
        let (min_incl, max_incl) = if (0..2).contains(&diff) {
            (true, true)
        } else {
            (random.next_boolean(), random.next_boolean())
        };
        ranges.push(DoubleRange::new(
            &format!("r{range_id}"),
            min,
            min_incl,
            max,
            max_incl,
        )?);

        // Do "slow but hopefully correct" computation of
        // expected count:
        for doc_values in values {
            for &v in doc_values {
                let mut accept = true;
                if min_incl {
                    accept &= v >= min;
                } else {
                    accept &= v > min;
                }
                if max_incl {
                    accept &= v <= max;
                } else {
                    accept &= v < max;
                }
                if accept {
                    expected_counts[range_id] += 1;
                    expected_top_n_children_counts[range_id] += 1;
                    min_accepted_value = min_accepted_value.min(v);
                    max_accepted_value = max_accepted_value.max(v);
                    if first_match_only {
                        break; // ensure each doc can contribute at most 1 count to each range
                    }
                }
            }
        }
    }
    Ok(RandomDoubleRanges {
        ranges,
        expected_counts,
        expected_top_n_children_counts,
        min_accepted_value,
        max_accepted_value,
    })
}

fn test_random_doubles_single_valued() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let w = writer(&dir)?;

    let num_docs = LuceneTestCase::at_least(1000) as usize;
    let mut values: Vec<Vec<f64>> = Vec::with_capacity(num_docs);
    let mut min_value = f64::INFINITY;
    let mut max_value = f64::NEG_INFINITY;
    for _ in 0..num_docs {
        let mut doc = Document::new();
        let v = LuceneTestCase::random().next_double();
        values.push(vec![v]);
        doc.add(DoubleDocValuesField::new("field", v));
        doc.add(DoublePoint::new("field", &[v])?);
        w.add_document(&doc)?;
        min_value = min_value.min(v);
        max_value = max_value.max(v);
    }
    let r = w.get_reader()?;

    let s = LuceneTestCase::new_searcher_wrap(Arc::clone(&r), false);
    let config = Arc::new(FacetsConfig::new());

    let num_iters = LuceneTestCase::at_least(10);
    for iter in 0..num_iters {
        if *VERBOSE {
            println!("TEST: iter={iter}");
        }
        let rr = random_double_ranges(&values, false)?;
        let random = LuceneTestCase::random();

        let sfc = match_all(&s)?;
        let fast_match_filter: Option<Arc<dyn Query>> = if random.next_boolean() {
            if random.next_boolean() {
                Some(DoublePoint::new_range_query(
                    "field",
                    &[min_value],
                    &[max_value],
                )?)
            } else {
                Some(DoublePoint::new_range_query(
                    "field",
                    &[rr.min_accepted_value],
                    &[rr.max_accepted_value],
                )?)
            }
        } else {
            None
        };
        let vs = double_values_source::from_double_field("field");
        let mvs = <dyn MultiDoubleValuesSource>::from_double_field("field");
        let facets = if random.next_boolean() {
            DoubleRangeFacetCounts::with_value_source_and_fast_match(
                "field",
                Some(Arc::clone(&vs)),
                &sfc,
                fast_match_filter.clone(),
                rr.ranges.clone(),
            )?
        } else if random.next_boolean() {
            DoubleRangeFacetCounts::with_multi_values_source_and_fast_match(
                "field",
                Some(<dyn MultiDoubleValuesSource>::from_single_valued(
                    Arc::clone(&vs),
                )),
                &sfc,
                fast_match_filter.clone(),
                rr.ranges.clone(),
            )?
        } else {
            DoubleRangeFacetCounts::with_multi_values_source_and_fast_match(
                "field",
                Some(Arc::clone(&mvs)),
                &sfc,
                fast_match_filter.clone(),
                rr.ranges.clone(),
            )?
        };
        check_random_results(
            &s,
            &config,
            &facets,
            &rr.expected_counts,
            rr.expected_top_n_children_counts.clone(),
            |range_id| {
                let range = &rr.ranges[range_id];
                if random.next_boolean() {
                    DoublePoint::new_range_query("field", &[range.min], &[range.max])
                } else if random.next_boolean() {
                    Ok(range.get_query(fast_match_filter.clone(), Arc::clone(&vs)))
                } else {
                    Ok(range.get_query_multi(fast_match_filter.clone(), Arc::clone(&mvs)))
                }
            },
        )?;
    }

    w.close()?;
    r.close()?;
    dir.close()
}

fn test_random_doubles_multi_valued() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let w = writer(&dir)?;

    let num_docs = LuceneTestCase::at_least(1000) as usize;
    let mut values: Vec<Vec<f64>> = Vec::with_capacity(num_docs);
    let mut min_value = f64::INFINITY;
    let mut max_value = f64::NEG_INFINITY;
    for _ in 0..num_docs {
        let random = LuceneTestCase::random();
        let mut doc = Document::new();
        let mut num_vals = RandomNumbers::random_int_between(&*random, 1, 50);
        if random.next_int_bound(10) == 0 {
            num_vals = 1; // make sure we have ample testing of single-value cases
        }
        let mut doc_values = Vec::with_capacity(num_vals as usize);
        for _ in 0..num_vals {
            let v = random.next_double();
            doc_values.push(v);
            doc.add(SortedNumericDocValuesField::new(
                "field",
                v.to_bits() as i64,
            ));
            doc.add(DoublePoint::new("field", &[v])?);
            min_value = min_value.min(v);
            max_value = max_value.max(v);
        }
        values.push(doc_values);
        w.add_document(&doc)?;
    }
    let r = w.get_reader()?;

    let s = LuceneTestCase::new_searcher_wrap(Arc::clone(&r), false);
    let config = Arc::new(FacetsConfig::new());

    let num_iters = LuceneTestCase::at_least(10);
    for iter in 0..num_iters {
        if *VERBOSE {
            println!("TEST: iter={iter}");
        }
        let rr = random_double_ranges(&values, true)?;
        let random = LuceneTestCase::random();

        let sfc = match_all(&s)?;
        let fast_match_filter: Option<Arc<dyn Query>> = if random.next_boolean() {
            if random.next_boolean() {
                Some(DoublePoint::new_range_query(
                    "field",
                    &[min_value],
                    &[max_value],
                )?)
            } else {
                Some(DoublePoint::new_range_query(
                    "field",
                    &[rr.min_accepted_value],
                    &[rr.max_accepted_value],
                )?)
            }
        } else {
            None
        };
        let facets = if random.next_boolean() {
            DoubleRangeFacetCounts::with_multi_values_source_and_fast_match(
                "field",
                Some(<dyn MultiDoubleValuesSource>::from_double_field("field")),
                &sfc,
                fast_match_filter.clone(),
                rr.ranges.clone(),
            )?
        } else {
            DoubleRangeFacetCounts::with_multi_values_source_and_fast_match(
                "field",
                None,
                &sfc,
                fast_match_filter.clone(),
                rr.ranges.clone(),
            )?
        };
        check_random_results(
            &s,
            &config,
            &facets,
            &rr.expected_counts,
            rr.expected_top_n_children_counts.clone(),
            |range_id| {
                let range = &rr.ranges[range_id];
                if random.next_boolean() {
                    DoublePoint::new_range_query("field", &[range.min], &[range.max])
                } else {
                    Ok(SortedNumericDocValuesField::new_slow_range_query(
                        "field",
                        range.min.to_bits() as i64,
                        range.max.to_bits() as i64,
                    ))
                }
            },
        )?;
    }

    w.close()?;
    r.close()?;
    dir.close()
}

// LUCENE-5178
fn test_missing_values() -> Result<()> {
    let d = LuceneTestCase::new_directory();
    let w = writer(&d)?;
    for l in 0..100i64 {
        if l % 5 == 0 {
            // Every 5th doc is missing the value:
            w.add_document(&Document::new())?;
            continue;
        }
        let mut doc = Document::new();
        doc.add(NumericDocValuesField::new("field", l));
        w.add_document(&doc)?;
    }

    let r = w.get_reader()?;

    let s = LuceneTestCase::new_searcher(Arc::clone(&r));
    let fc = match_all(&s)?;
    let facets = LongRangeFacetCounts::new("field", &fc, basic_long_ranges(false)?)?;

    Assert::assert_equals(
        "dim=field path=[] value=16 childCount=5\n  less than 10 (8)\n  less than or equal to 10 (8)\n  over 90 (8)\n  90 or above (8)\n  over 1000 (0)\n",
        all_children(&facets, "field")?.as_str(),
    );
    Assert::assert_equals(
        "dim=field path=[] value=16 childCount=4\n  90 or above (8)\n  less than 10 (8)\n  less than or equal to 10 (8)\n  over 90 (8)\n",
        top_children(&facets, 4, "field")?.as_str(),
    );

    w.close()?;
    r.close()?;
    d.close()
}

fn test_missing_values_multi_valued() -> Result<()> {
    let d = LuceneTestCase::new_directory();
    let w = writer(&d)?;
    // index the same field twice to test multi-valued logic
    for l in 0..100i64 {
        if l % 5 == 0 {
            // Every 5th doc is missing the value:
            w.add_document(&Document::new())?;
            continue;
        }
        let mut doc = Document::new();
        doc.add(SortedNumericDocValuesField::new("field", l));
        doc.add(SortedNumericDocValuesField::new("field", l));
        w.add_document(&doc)?;
    }

    let r = w.get_reader()?;

    let s = LuceneTestCase::new_searcher(Arc::clone(&r));
    let fc = match_all(&s)?;
    let facets = LongRangeFacetCounts::new("field", &fc, basic_long_ranges(false)?)?;

    Assert::assert_equals(
        "dim=field path=[] value=16 childCount=5\n  less than 10 (8)\n  less than or equal to 10 (8)\n  over 90 (8)\n  90 or above (8)\n  over 1000 (0)\n",
        all_children(&facets, "field")?.as_str(),
    );
    Assert::assert_equals(
        "dim=field path=[] value=16 childCount=4\n  90 or above (8)\n  less than 10 (8)\n  less than or equal to 10 (8)\n  over 90 (8)\n",
        top_children(&facets, 4, "field")?.as_str(),
    );

    w.close()?;
    r.close()?;
    d.close()
}

/// `private static class UsedQuery extends Query`.
#[derive(Debug, Clone)]
struct UsedQuery {
    used: Arc<AtomicBool>,
    r#in: Arc<dyn Query>,
}

impl UsedQuery {
    fn new(r#in: Arc<dyn Query>, used: Arc<AtomicBool>) -> Self {
        Self { used, r#in }
    }
}

impl Query for UsedQuery {
    fn create_weight(
        &self,
        searcher: &IndexSearcher,
        score_mode: ScoreMode,
        boost: f32,
    ) -> Result<Arc<dyn Weight>> {
        let r#in = self.r#in.create_weight(searcher, score_mode, boost)?;
        Ok(Arc::new(UsedWeight {
            base: FilterWeight::new(r#in),
            used: Arc::clone(&self.used),
        }))
    }

    fn rewrite(&self, index_searcher: &IndexSearcher) -> Result<Option<Arc<dyn Query>>> {
        match self.r#in.rewrite(index_searcher)? {
            Some(in_rewritten) => Ok(Some(Arc::new(UsedQuery::new(
                in_rewritten,
                Arc::clone(&self.used),
            )))),
            // `super.rewrite(indexSearcher)`, which is `this`.
            None => Ok(None),
        }
    }

    fn visit(&self, visitor: &mut dyn QueryVisitor) {
        self.r#in.visit(visitor);
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn query_eq(&self, other: &dyn Query) -> bool {
        self.same_class_as(other)
            && other
                .as_any()
                .downcast_ref::<UsedQuery>()
                .is_some_and(|o| self.r#in.query_eq(&*o.r#in))
    }

    fn query_hash(&self) -> i32 {
        self.class_hash().wrapping_add(self.r#in.query_hash())
    }

    fn to_query_string(&self, _field: &str) -> String {
        format!("UsedQuery({})", query_to_string(&*self.r#in))
    }
}

/// The anonymous `FilterWeight` of `UsedQuery.createWeight`.
#[derive(Debug)]
struct UsedWeight {
    base: FilterWeight,
    used: Arc<AtomicBool>,
}

impl SegmentCacheable for UsedWeight {
    fn is_cacheable(&self, ctx: &LeafReaderContext) -> bool {
        self.base.is_cacheable(ctx)
    }
}

impl Weight for UsedWeight {
    fn get_query(&self) -> Arc<dyn Query> {
        self.base.get_query()
    }

    fn explain(&self, context: &LeafReaderContext, doc: i32) -> Result<Explanation> {
        self.base.explain(context, doc)
    }

    fn matches(&self, context: &LeafReaderContext, doc: i32) -> Result<Option<Arc<dyn Matches>>> {
        self.base.matches(context, doc)
    }

    fn count(&self, context: &LeafReaderContext) -> Result<i32> {
        self.base.count(context)
    }

    fn scorer_supplier(
        &self,
        context: &LeafReaderContext,
    ) -> Result<Option<Box<dyn ScorerSupplier>>> {
        let Some(mut scorer) = self.base.inner.scorer(context)? else {
            return Ok(None);
        };
        let cost = scorer.iterator().cost();
        Ok(Some(Box::new(UsedScorerSupplier {
            scorer: Some(scorer),
            cost,
            used: Arc::clone(&self.used),
        })))
    }
}

/// The anonymous `ScorerSupplier` of `UsedQuery`'s weight.
struct UsedScorerSupplier {
    scorer: Option<Box<dyn Scorer>>,
    /// `scorer.iterator().cost()`, read when the scorer is built: it is
    /// stable.
    cost: i64,
    used: Arc<AtomicBool>,
}

impl ScorerSupplier for UsedScorerSupplier {
    fn get(&mut self, _lead_cost: i64) -> Result<Box<dyn Scorer>> {
        self.used.store(true, Ordering::SeqCst);
        Ok(self.scorer.take().expect("get is called once"))
    }

    fn cost(&self) -> i64 {
        self.cost
    }
}

/// `private static class PlusOneValuesSource extends DoubleValuesSource`.
#[derive(Debug)]
struct PlusOneValuesSource;

/// The anonymous `DoubleValues` of `PlusOneValuesSource.getValues`.
struct PlusOneValues {
    doc: i32,
}

impl DoubleValues for PlusOneValues {
    fn double_value(&mut self) -> Result<f64> {
        Ok(f64::from(self.doc + 1))
    }

    fn advance_exact(&mut self, doc: i32) -> Result<bool> {
        self.doc = doc;
        Ok(true)
    }
}

impl SegmentCacheable for PlusOneValuesSource {
    fn is_cacheable(&self, _ctx: &LeafReaderContext) -> bool {
        false
    }
}

impl DoubleValuesSource for PlusOneValuesSource {
    fn get_values<'a>(
        self: Arc<Self>,
        _ctx: &LeafReaderContext,
        _scores: Option<Box<dyn DoubleValues + 'a>>,
    ) -> Result<Box<dyn DoubleValues + 'a>> {
        Ok(Box::new(PlusOneValues { doc: -1 }))
    }

    fn needs_scores(&self) -> bool {
        false
    }

    fn rewrite(self: Arc<Self>, _searcher: &IndexSearcher) -> Result<Arc<dyn DoubleValuesSource>> {
        Ok(self)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn source_eq(&self, other: &dyn DoubleValuesSource) -> bool {
        other.as_any().is::<PlusOneValuesSource>()
    }

    fn source_hash(&self) -> i32 {
        0
    }

    /// Java's `toString()` returns `null`, which string concatenation
    /// renders as `"null"`.
    fn to_source_string(&self) -> String {
        "null".to_string()
    }

    fn explain(
        self: Arc<Self>,
        _ctx: &LeafReaderContext,
        doc_id: i32,
        _score_explanation: &Explanation,
    ) -> Result<Explanation> {
        Ok(Explanation::matched(doc_id + 1, "", Vec::new()))
    }
}

/// The anonymous `DrillSideways` subclass of `testCustomDoubleValuesSource`.
struct CustomSourceOverrides {
    vs: Arc<dyn DoubleValuesSource>,
    fast_match_filter: Option<Arc<dyn Query>>,
    ranges: Vec<DoubleRange>,
}

impl DrillSidewaysOverrides for CustomSourceOverrides {
    fn build_facets_result(
        &self,
        _drill_sideways: &DrillSideways,
        _drill_downs: Option<&FacetsCollector>,
        drill_sideways_collectors: Option<&[FacetsCollector]>,
        _drill_sideways_dims: Option<&[String]>,
    ) -> Result<Option<Arc<dyn Facets>>> {
        let collectors = drill_sideways_collectors.expect("java.lang.NullPointerException");
        debug_assert!(collectors.len() == 1);
        Ok(Some(Arc::new(
            DoubleRangeFacetCounts::with_multi_values_source_and_fast_match(
                "field",
                Some(<dyn MultiDoubleValuesSource>::from_single_valued(
                    Arc::clone(&self.vs),
                )),
                &collectors[0],
                self.fast_match_filter.clone(),
                self.ranges.clone(),
            )?,
        )))
    }

    fn score_sub_docs_at_once(&self) -> bool {
        LuceneTestCase::random().next_boolean()
    }
}

fn test_custom_double_values_source() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let writer = writer(&dir)?;

    let doc = Document::new();
    writer.add_document(&doc)?;
    writer.add_document(&doc)?;
    writer.add_document(&doc)?;

    // Test wants 3 docs in one segment:
    writer.force_merge(1)?;

    let vs: Arc<dyn DoubleValuesSource> = Arc::new(PlusOneValuesSource);

    let config = Arc::new(FacetsConfig::new());

    let r = writer.get_reader()?;

    let mut s = LuceneTestCase::new_searcher_concurrency(
        Arc::clone(&r),
        false,
        false,
        Concurrency::InterSegment,
    );
    // DrillSideways requires the entire range of docs to be scored at once, so it doesn't support
    // timeouts whose implementation scores one window of doc IDs at a time.
    s.set_timeout(None);
    let s = Arc::new(s);
    let fc = match_all(&s)?;

    let ranges = vec![
        dr("< 1", 0.0, true, 1.0, false)?,
        dr("< 2", 0.0, true, 2.0, false)?,
        dr("< 5", 0.0, true, 5.0, false)?,
        dr("< 10", 0.0, true, 10.0, false)?,
        dr("< 20", 0.0, true, 20.0, false)?,
        dr("< 50", 0.0, true, 50.0, false)?,
    ];

    let filter_was_used = Arc::new(AtomicBool::new(false));
    let fast_match_filter: Option<Arc<dyn Query>> = if LuceneTestCase::random().next_boolean() {
        // Sort of silly:
        let r#in: Arc<dyn Query> = Arc::new(MatchAllDocsQuery::instance());
        Some(Arc::new(UsedQuery::new(r#in, Arc::clone(&filter_was_used))))
    } else {
        None
    };

    if *VERBOSE {
        println!(
            "TEST: fastMatchFilter={}",
            fast_match_filter
                .as_ref()
                .map_or_else(|| "null".to_string(), |q| query_to_string(&**q))
        );
    }

    let facets = if LuceneTestCase::random().next_boolean() {
        DoubleRangeFacetCounts::with_value_source_and_fast_match(
            "field",
            Some(Arc::clone(&vs)),
            &fc,
            fast_match_filter.clone(),
            ranges.clone(),
        )?
    } else {
        DoubleRangeFacetCounts::with_multi_values_source_and_fast_match(
            "field",
            Some(<dyn MultiDoubleValuesSource>::from_single_valued(
                Arc::clone(&vs),
            )),
            &fc,
            fast_match_filter.clone(),
            ranges.clone(),
        )?
    };

    Assert::assert_equals(
        "dim=field path=[] value=3 childCount=6\n  < 1 (0)\n  < 2 (1)\n  < 5 (3)\n  < 10 (3)\n  < 20 (3)\n  < 50 (3)\n",
        all_children(&facets, "field")?.as_str(),
    );
    Assert::assert_equals(
        "dim=field path=[] value=3 childCount=5\n  < 10 (3)\n  < 20 (3)\n  < 5 (3)\n  < 50 (3)\n  < 2 (1)\n",
        top_children(&facets, 5, "field")?.as_str(),
    );
    Assert::assert_true(fast_match_filter.is_none() || filter_was_used.load(Ordering::SeqCst));

    let mut ddq = DrillDownQuery::new(Arc::clone(&config));
    if LuceneTestCase::random().next_boolean() {
        ddq.add_query(
            "field",
            ranges[1].get_query(fast_match_filter.clone(), Arc::clone(&vs)),
        )?;
    } else {
        ddq.add_query(
            "field",
            ranges[1].get_query_multi(
                fast_match_filter.clone(),
                <dyn MultiDoubleValuesSource>::from_single_valued(Arc::clone(&vs)),
            ),
        )?;
    }

    // Test simple drill-down:
    Assert::assert_equals(
        1,
        s.search(Arc::new(ddq.clone_query()), 10)?
            .total_hits
            .value(),
    );

    // Test drill-sideways after drill-down
    let ds = DrillSideways::with_overrides(
        Arc::clone(&s),
        Arc::clone(&config),
        None,
        None,
        None,
        Arc::new(CustomSourceOverrides {
            vs: Arc::clone(&vs),
            fast_match_filter: fast_match_filter.clone(),
            ranges: ranges.clone(),
        }),
    );

    let dsr = ds.search(ddq, 10)?;
    Assert::assert_equals(1, dsr.hits.as_ref().unwrap().total_hits().value());
    let facets = dsr.facets.unwrap();
    Assert::assert_equals(
        "dim=field path=[] value=3 childCount=6\n  < 1 (0)\n  < 2 (1)\n  < 5 (3)\n  < 10 (3)\n  < 20 (3)\n  < 50 (3)\n",
        all_children(&*facets, "field")?.as_str(),
    );
    Assert::assert_equals(
        "dim=field path=[] value=3 childCount=5\n  < 10 (3)\n  < 20 (3)\n  < 5 (3)\n  < 50 (3)\n  < 2 (1)\n",
        top_children(&*facets, 5, "field")?.as_str(),
    );

    writer.close()?;
    r.close()?;
    dir.close()
}

fn test_long_range_equals() -> Result<()> {
    let r = |max: i64, max_incl: bool| LongRange::new("field", -7, true, max, max_incl).unwrap();
    assert_equals(&r(17, false), &r(17, false));
    Assert::assert_equals(r(17, false).hash_code(), r(17, false).hash_code());
    Assert::assert_false(r(17, false) == r(17, true));
    Assert::assert_false(r(17, false).hash_code() == r(17, true).hash_code());
    Assert::assert_false(r(17, false) == r(18, false));
    Assert::assert_false(r(17, false).hash_code() == r(18, false).hash_code());
    Ok(())
}

fn test_double_range_equals() -> Result<()> {
    let r =
        |max: f64, max_incl: bool| DoubleRange::new("field", -7.0, true, max, max_incl).unwrap();
    assert_equals(&r(17.0, false), &r(17.0, false));
    Assert::assert_equals(r(17.0, false).hash_code(), r(17.0, false).hash_code());
    Assert::assert_false(r(17.0, false) == r(17.0, true));
    Assert::assert_false(r(17.0, false).hash_code() == r(17.0, true).hash_code());
    Assert::assert_false(r(17.0, false) == r(18.0, false));
    Assert::assert_false(r(17.0, false).hash_code() == r(18.0, false).hash_code());
    Ok(())
}
