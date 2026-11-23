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

//! Port of `org.apache.lucene.facet.rangeonrange.TestRangeOnRangeFacetCounts`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::range_fields::{
    DoubleRange as DoubleRangeField, LongRange as LongRangeField, RangeQueryType,
};
use rucene::document::{Document, DoubleRangeDocValuesField, LongRangeDocValuesField};
use rucene::error::Result;
use rucene::index::index_writer_config::OpenMode;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::{MatchAllDocsQuery, Query};
use rucene_facet::drill_down_query::DrillDownQuery;
use rucene_facet::drill_sideways::{DrillSideways, DrillSidewaysOverrides};
use rucene_facet::facet_field::FacetField;
use rucene_facet::facets::Facets;
use rucene_facet::facets_collector::FacetsCollector;
use rucene_facet::facets_collector_manager::FacetsCollectorManager;
use rucene_facet::facets_config::FacetsConfig;
use rucene_facet::label_and_value::number_int_value;
use rucene_facet::multi_facets::MultiFacets;
use rucene_facet::rangeonrange::double_range::DoubleRange;
use rucene_facet::rangeonrange::double_range_on_range_facet_counts::DoubleRangeOnRangeFacetCounts;
use rucene_facet::rangeonrange::long_range::LongRange;
use rucene_facet::rangeonrange::long_range_on_range_facet_counts::LongRangeOnRangeFacetCounts;
use rucene_facet::taxonomy::directory::directory_taxonomy_reader::DirectoryTaxonomyReader;
use rucene_facet::taxonomy::directory::directory_taxonomy_writer::DirectoryTaxonomyWriter;
use rucene_facet::taxonomy::taxonomy_reader::TaxonomyReader;
use rucene_facet::taxonomy::taxonomy_writer::TaxonomyWriter;
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::lucene_test_case::{Concurrency, TestDirectory, VERBOSE};
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::facet_support::core_support::{assert_equals, assert_not_equals, test};
use crate::facet_support::{facet_class, get_taxonomy_facet_counts};

const IAE: &str = "java.lang.IllegalArgumentException";
const INTERSECTS: RangeQueryType = RangeQueryType::Intersects;

/// `TestRangeOnRangeFacetCounts`.
pub fn class_def() -> Arc<Class> {
    facet_class(
        "org.apache.lucene.facet.rangeonrange.TestRangeOnRangeFacetCounts",
        vec![
            test("testBasicSingleDimLong", test_basic_single_dim_long),
            test("testBasicMultiDimLong", test_basic_multi_dim_long),
            test(
                "testLongGetAllDimsSingleDim",
                test_long_get_all_dims_single_dim,
            ),
            test("testGetAllDimsMultiDim", test_get_all_dims_multi_dim),
            test("testUselessRangeSingleDim", test_useless_range_single_dim),
            test("testUselessMultiDimRange", test_useless_multi_dim_range),
            test("testSingleDimLongMinMax", test_single_dim_long_min_max),
            test("testMultiDimLongMinMax", test_multi_dim_long_min_max),
            test(
                "testSingleDimOverlappedEndStart",
                test_single_dim_overlapped_end_start,
            ),
            test(
                "testSingleDimMixedRangeAndNonRangeTaxonomy",
                test_single_dim_mixed_range_and_non_range_taxonomy,
            ),
            test(
                "testMultiDimMixedRangeAndNonRangeTaxonomy",
                test_multi_dim_mixed_range_and_non_range_taxonomy,
            ),
            test("testBasicSingleDimDouble", test_basic_single_dim_double),
            test("testBasicMultiDimDouble", test_basic_multi_dim_double),
            test("testRandomSingleDimLongs", test_random_single_dim_longs),
            test("testRandomMultiDimLongs", test_random_multi_dim_longs),
            test("testRandomSingleDimDoubles", test_random_single_dim_doubles),
            test("testRandomMultiDimDoubles", test_random_multi_dim_doubles),
            test("testMissingValues", test_missing_values),
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

fn lrm(label: &str, min: &[i64], max: &[i64]) -> Result<LongRange> {
    LongRange::new_multi(label, min.to_vec(), max.to_vec())
}

fn dr(label: &str, min: f64, min_incl: bool, max: f64, max_incl: bool) -> Result<DoubleRange> {
    DoubleRange::new(label, min, min_incl, max, max_incl)
}

fn drm(label: &str, min: &[f64], max: &[f64]) -> Result<DoubleRange> {
    DoubleRange::new_multi(label, min.to_vec(), max.to_vec())
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

/// A document holding one `LongRangeDocValuesField` with the given bounds:
/// Java's `setLongRangeValue` re-encodes the bounds into the shared field
/// with `LongRange.verifyAndEncode`, which the field's constructor does.
fn long_range_doc(min: &[i64], max: &[i64]) -> Result<Document> {
    let mut doc = Document::new();
    doc.add(
        LongRangeDocValuesField::new("field", min, max)?
            .as_binary_range_doc_values_field()
            .clone(),
    );
    Ok(doc)
}

/// A document holding one `DoubleRangeDocValuesField` (`setDoubleRangeValue`).
fn double_range_doc(min: &[f64], max: &[f64]) -> Result<Document> {
    let mut doc = Document::new();
    doc.add(
        DoubleRangeDocValuesField::new("field", min, max)?
            .as_binary_range_doc_values_field()
            .clone(),
    );
    Ok(doc)
}

/// The five single-dimension long ranges; `over_1000_max_incl` is the
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

/// The five three-dimension long ranges.
fn multi_dim_long_ranges() -> Result<Vec<LongRange>> {
    Ok(vec![
        lrm("less than 10", &[0, 1, 2], &[8, 9, 10])?,
        lrm("a bit over 10", &[0, 1, 2], &[10, 11, 12])?,
        lrm("a bit under 90", &[88, 89, 90], &[100, 101, 102])?,
        lrm("over 90", &[90, 91, 92], &[100, 101, 102])?,
        lrm(
            "over 1000",
            &[1000, 1001, 1002],
            &[i64::MAX - 2, i64::MAX - 1, i64::MAX],
        )?,
    ])
}

/// The documents of the single-dimension long tests: `[l, 10 + l]` for
/// `l` in `0..100`, then `[Long.MAX_VALUE - 10, Long.MAX_VALUE]`.
fn index_single_dim_longs(w: &RandomIndexWriter) -> Result<()> {
    for l in 0..100i64 {
        w.add_document(&long_range_doc(&[l], &[10 + l])?)?;
    }

    // Also add Long.MAX_VALUE
    w.add_document(&long_range_doc(&[i64::MAX - 10], &[i64::MAX])?)?;
    Ok(())
}

/// The documents of the three-dimension long tests.
fn index_multi_dim_longs(w: &RandomIndexWriter) -> Result<()> {
    for l in 0..100i64 {
        w.add_document(&long_range_doc(
            &[l, l + 1, l + 2],
            &[l + 10, l + 11, l + 12],
        )?)?;
    }

    // Also add Long.MAX_VALUE
    let inf_min = [i64::MAX - 12, i64::MAX - 11, i64::MAX - 10];
    let inf_max = [i64::MAX - 2, i64::MAX - 1, i64::MAX];
    w.add_document(&long_range_doc(&inf_min, &inf_max)?)?;
    Ok(())
}

fn test_basic_single_dim_long() -> Result<()> {
    let d = LuceneTestCase::new_directory();
    let w = writer(&d)?;
    index_single_dim_longs(&w)?;

    let r = w.get_reader()?;
    w.close()?;

    let s = LuceneTestCase::new_searcher(Arc::clone(&r));
    let fc = match_all(&s)?;

    let facets =
        LongRangeOnRangeFacetCounts::new("field", &fc, INTERSECTS, &basic_long_ranges(true)?)?;

    Assert::assert_equals(
        "dim=field path=[] value=32 childCount=5\n  less than 10 (10)\n  less than or equal to 10 (11)\n  over 90 (19)\n  90 or above (20)\n  over 1000 (1)\n",
        all_children(&facets, "field")?.as_str(),
    );
    Assert::assert_equals(
        "dim=field path=[] value=32 childCount=5\n  90 or above (20)\n  over 90 (19)\n  less than or equal to 10 (11)\n  less than 10 (10)\n",
        top_children(&facets, 4, "field")?.as_str(),
    );

    // test getTopChildren(0, dim)
    expect_throws(IAE, || facets.get_top_children(0, "field", &[]));

    r.close()?;
    d.close()
}

fn test_basic_multi_dim_long() -> Result<()> {
    let d = LuceneTestCase::new_directory();
    let w = writer(&d)?;
    index_multi_dim_longs(&w)?;

    let r = w.get_reader()?;
    w.close()?;

    let s = LuceneTestCase::new_searcher(Arc::clone(&r));
    let fc = match_all(&s)?;

    let facets =
        LongRangeOnRangeFacetCounts::new("field", &fc, INTERSECTS, &multi_dim_long_ranges()?)?;

    Assert::assert_equals(
        "dim=field path=[] value=34 childCount=5\n  less than 10 (9)\n  a bit over 10 (11)\n  a bit under 90 (22)\n  over 90 (20)\n  over 1000 (1)\n",
        all_children(&facets, "field")?.as_str(),
    );
    Assert::assert_equals(
        "dim=field path=[] value=34 childCount=5\n  a bit under 90 (22)\n  over 90 (20)\n  a bit over 10 (11)\n  less than 10 (9)\n",
        top_children(&facets, 4, "field")?.as_str(),
    );

    // test getTopChildren(0, dim)
    expect_throws(IAE, || facets.get_top_children(0, "field", &[]));

    r.close()?;
    d.close()
}

fn test_long_get_all_dims_single_dim() -> Result<()> {
    let d = LuceneTestCase::new_directory();
    let w = writer(&d)?;
    index_single_dim_longs(&w)?;

    let r = w.get_reader()?;
    w.close()?;

    let s = LuceneTestCase::new_searcher(Arc::clone(&r));
    let fc = match_all(&s)?;

    let facets =
        LongRangeOnRangeFacetCounts::new("field", &fc, INTERSECTS, &basic_long_ranges(true)?)?;

    let result = facets.get_all_dims(10)?;
    Assert::assert_equals(1, result.len());
    Assert::assert_equals(
        "dim=field path=[] value=32 childCount=5\n  90 or above (20)\n  over 90 (19)\n  less than or equal to 10 (11)\n  less than 10 (10)\n  over 1000 (1)\n",
        result[0].to_string().as_str(),
    );

    // test getAllDims(1)
    let result = facets.get_all_dims(1)?;
    Assert::assert_equals(1, result.len());
    Assert::assert_equals(
        "dim=field path=[] value=32 childCount=5\n  90 or above (20)\n",
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

fn test_get_all_dims_multi_dim() -> Result<()> {
    let d = LuceneTestCase::new_directory();
    let w = writer(&d)?;
    index_multi_dim_longs(&w)?;

    let r = w.get_reader()?;
    w.close()?;

    let s = LuceneTestCase::new_searcher(Arc::clone(&r));
    let fc = match_all(&s)?;

    let facets =
        LongRangeOnRangeFacetCounts::new("field", &fc, INTERSECTS, &multi_dim_long_ranges()?)?;

    let result = facets.get_all_dims(10)?;
    Assert::assert_equals(1, result.len());
    Assert::assert_equals(
        "dim=field path=[] value=34 childCount=5\n  a bit under 90 (22)\n  over 90 (20)\n  a bit over 10 (11)\n  less than 10 (9)\n  over 1000 (1)\n",
        result[0].to_string().as_str(),
    );

    // test getAllDims(1)
    let result = facets.get_all_dims(1)?;
    Assert::assert_equals(1, result.len());
    Assert::assert_equals(
        "dim=field path=[] value=34 childCount=5\n  a bit under 90 (22)\n",
        result[0].to_string().as_str(),
    );

    // test getTopChildren(0, dim)
    expect_throws(IAE, || facets.get_top_children(0, "field", &[]));

    r.close()?;
    d.close()
}

fn test_useless_range_single_dim() -> Result<()> {
    expect_throws(IAE, || LongRange::new("useless", 7, true, 6, true));
    expect_throws(IAE, || LongRange::new("useless", 7, true, 7, false));
    expect_throws(IAE, || DoubleRange::new("useless", 7.0, true, 6.0, true));
    expect_throws(IAE, || DoubleRange::new("useless", 7.0, true, 7.0, false));
    Ok(())
}

fn test_useless_multi_dim_range() -> Result<()> {
    expect_throws(IAE, || lrm("useless", &[7, 7], &[6, 6]));
    expect_throws(IAE, || lrm("useless", &[7, 7], &[7, 6]));
    expect_throws(IAE, || drm("useless", &[7.0, 7.0], &[6.0, 6.0]));
    expect_throws(IAE, || drm("useless", &[7.0, 7.0], &[7.0, 6.0]));
    Ok(())
}

fn test_single_dim_long_min_max() -> Result<()> {
    let d = LuceneTestCase::new_directory();
    let w = writer(&d)?;
    w.add_document(&long_range_doc(&[i64::MIN], &[i64::MIN + 10])?)?;
    w.add_document(&long_range_doc(&[0], &[10])?)?;
    w.add_document(&long_range_doc(&[i64::MAX - 10], &[i64::MAX])?)?;

    let r = w.get_reader()?;
    w.close()?;

    let s = LuceneTestCase::new_searcher(Arc::clone(&r));
    let fc = match_all(&s)?;

    let facets = LongRangeOnRangeFacetCounts::new(
        "field",
        &fc,
        INTERSECTS,
        &[
            lr("min", i64::MIN, true, i64::MIN, true)?,
            lr("max", i64::MAX, true, i64::MAX, true)?,
            lr("all0", i64::MIN, true, i64::MAX, true)?,
            lr("all1", i64::MIN, false, i64::MAX, true)?,
            lr("all2", i64::MIN, true, i64::MAX, false)?,
            lr("all3", i64::MIN, false, i64::MAX, false)?,
        ],
    )?;

    Assert::assert_equals(
        "dim=field path=[] value=3 childCount=6\n  min (1)\n  max (1)\n  all0 (3)\n  all1 (3)\n  all2 (3)\n  all3 (3)\n",
        all_children(&facets, "field")?.as_str(),
    );
    Assert::assert_equals(
        "dim=field path=[] value=3 childCount=6\n  all0 (3)\n  all1 (3)\n  all2 (3)\n  all3 (3)\n  max (1)\n",
        top_children(&facets, 5, "field")?.as_str(),
    );

    r.close()?;
    d.close()
}

fn test_multi_dim_long_min_max() -> Result<()> {
    let d = LuceneTestCase::new_directory();
    let w = writer(&d)?;
    w.add_document(&long_range_doc(
        &[i64::MIN, i64::MIN],
        &[i64::MIN + 10, i64::MIN + 11],
    )?)?;
    w.add_document(&long_range_doc(&[0, 1], &[10, 11])?)?;
    w.add_document(&long_range_doc(
        &[i64::MAX - 11, i64::MAX - 10],
        &[i64::MAX, i64::MAX],
    )?)?;

    let r = w.get_reader()?;
    w.close()?;

    let s = LuceneTestCase::new_searcher(Arc::clone(&r));
    let fc = match_all(&s)?;

    let facets = LongRangeOnRangeFacetCounts::new(
        "field",
        &fc,
        INTERSECTS,
        &[
            lrm("min", &[i64::MIN, i64::MIN], &[i64::MIN, i64::MIN])?,
            lrm("max", &[i64::MAX, i64::MAX], &[i64::MAX, i64::MAX])?,
            lrm("all0", &[i64::MIN, i64::MIN], &[i64::MIN, i64::MAX])?,
            lrm("all1", &[i64::MIN, i64::MIN], &[i64::MAX, i64::MIN])?,
            lrm("all2", &[i64::MIN, i64::MIN], &[i64::MAX, i64::MAX])?,
        ],
    )?;

    Assert::assert_equals(
        "dim=field path=[] value=3 childCount=5\n  min (1)\n  max (1)\n  all0 (1)\n  all1 (1)\n  all2 (3)\n",
        all_children(&facets, "field")?.as_str(),
    );
    Assert::assert_equals(
        "dim=field path=[] value=3 childCount=5\n  all2 (3)\n  all0 (1)\n  all1 (1)\n  max (1)\n",
        top_children(&facets, 4, "field")?.as_str(),
    );

    r.close()?;
    d.close()
}

fn test_single_dim_overlapped_end_start() -> Result<()> {
    let d = LuceneTestCase::new_directory();
    let w = writer(&d)?;
    index_single_dim_longs(&w)?;

    let r = w.get_reader()?;
    w.close()?;

    let s = LuceneTestCase::new_searcher(Arc::clone(&r));
    let fc = match_all(&s)?;

    let facets = LongRangeOnRangeFacetCounts::new(
        "field",
        &fc,
        INTERSECTS,
        &[
            lr("0-10", 0, true, 10, true)?,
            lr("10-20", 10, true, 20, true)?,
            lr("20-30", 20, true, 30, true)?,
            lr("30-40", 30, true, 40, true)?,
        ],
    )?;

    Assert::assert_equals(
        "dim=field path=[] value=41 childCount=4\n  0-10 (11)\n  10-20 (21)\n  20-30 (21)\n  30-40 (21)\n",
        all_children(&facets, "field")?.as_str(),
    );
    Assert::assert_equals(
        "dim=field path=[] value=41 childCount=4\n  10-20 (21)\n  20-30 (21)\n  30-40 (21)\n",
        top_children(&facets, 3, "field")?.as_str(),
    );
    r.close()?;
    d.close()
}

/// The anonymous `DrillSideways` subclasses of the
/// `test*MixedRangeAndNonRangeTaxonomy` methods; `ranges` builds the
/// ranges of the `"field"` dimension.
struct MixedRangeOverrides {
    config: Arc<FacetsConfig>,
    ranges: fn() -> Result<Vec<LongRange>>,
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
                Arc::new(LongRangeOnRangeFacetCounts::new(
                    "field",
                    field_fc.expect("java.lang.NullPointerException"),
                    INTERSECTS,
                    &(self.ranges)()?,
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

/// The body shared by the two `test*MixedRangeAndNonRangeTaxonomy`
/// methods: `bounds(l)` gives the indexed range of doc `l`, `ranges` the
/// faceted ranges and `expected_field` the two `"field"` results.
fn check_mixed_range_and_non_range_taxonomy(
    bounds: impl Fn(i64) -> (Vec<i64>, Vec<i64>),
    ranges: fn() -> Result<Vec<LongRange>>,
    expected_field: [&str; 2],
) -> Result<()> {
    let d = LuceneTestCase::new_directory();
    let w = writer(&d)?;
    let td = LuceneTestCase::new_directory();
    let tw = Arc::new(DirectoryTaxonomyWriter::with_open_mode(
        td.as_directory(),
        OpenMode::CREATE,
    )?);

    let config = Arc::new(FacetsConfig::new());

    for l in 0..100i64 {
        let (min, max) = bounds(l);
        let mut doc = Document::new();
        // For computing range facet counts:
        doc.add(
            LongRangeDocValuesField::new("field", &min, &max)?
                .as_binary_range_doc_values_field()
                .clone(),
        );
        // For drill down by numeric range:
        doc.add(LongRangeField::new("field", &min, &max)?);

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
            ranges,
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
        expected_field[0],
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
        expected_field[1],
        top_children(&*facets, 10, "field")?.as_str(),
    );

    w.close()?;
    tw.close()?;
    tr.close()?;
    td.close()?;
    r.close()?;
    d.close()
}

fn test_single_dim_mixed_range_and_non_range_taxonomy() -> Result<()> {
    check_mixed_range_and_non_range_taxonomy(
        |l| (vec![l], vec![l + 10]),
        || basic_long_ranges(false),
        [
            "dim=field path=[] value=31 childCount=4\n  90 or above (20)\n  over 90 (19)\n  less than or equal to 10 (11)\n  less than 10 (10)\n",
            "dim=field path=[] value=23 childCount=4\n  90 or above (15)\n  over 90 (15)\n  less than or equal to 10 (8)\n  less than 10 (7)\n",
        ],
    )
}

fn test_multi_dim_mixed_range_and_non_range_taxonomy() -> Result<()> {
    check_mixed_range_and_non_range_taxonomy(
        |l| (vec![l, l + 1, l + 2], vec![l + 10, l + 11, l + 12]),
        multi_dim_long_ranges,
        [
            "dim=field path=[] value=33 childCount=4\n  a bit under 90 (22)\n  over 90 (20)\n  a bit over 10 (11)\n  less than 10 (9)\n",
            "dim=field path=[] value=25 childCount=4\n  a bit under 90 (17)\n  over 90 (15)\n  a bit over 10 (8)\n  less than 10 (6)\n",
        ],
    )
}

fn test_basic_single_dim_double() -> Result<()> {
    let d = LuceneTestCase::new_directory();
    let w = writer(&d)?;
    for l in 0..100 {
        let l = f64::from(l);
        w.add_document(&double_range_doc(&[l], &[10.0 + l])?)?;
    }

    let r = w.get_reader()?;
    w.close()?;

    let s = LuceneTestCase::new_searcher(Arc::clone(&r));
    let fc = match_all(&s)?;

    let facets = DoubleRangeOnRangeFacetCounts::new(
        "field",
        &fc,
        INTERSECTS,
        &[
            dr("less than 10", 0.0, true, 10.0, false)?,
            dr("less than or equal to 10", 0.0, true, 10.0, true)?,
            dr("over 90", 90.0, false, 100.0, false)?,
            dr("90 or above", 90.0, true, 100.0, false)?,
            dr("over 1000", 1000.0, false, f64::INFINITY, false)?,
        ],
    )?;

    Assert::assert_equals(
        "dim=field path=[] value=31 childCount=5\n  less than 10 (10)\n  less than or equal to 10 (11)\n  over 90 (19)\n  90 or above (20)\n  over 1000 (0)\n",
        all_children(&facets, "field")?.as_str(),
    );
    Assert::assert_equals(
        "dim=field path=[] value=31 childCount=4\n  90 or above (20)\n  over 90 (19)\n  less than or equal to 10 (11)\n  less than 10 (10)\n",
        top_children(&facets, 4, "field")?.as_str(),
    );

    // test getTopChildren(0, dim)
    expect_throws(IAE, || facets.get_top_children(0, "field", &[]));

    r.close()?;
    d.close()
}

fn test_basic_multi_dim_double() -> Result<()> {
    let d = LuceneTestCase::new_directory();
    let w = writer(&d)?;
    for l in 0..100 {
        let l = f64::from(l);
        w.add_document(&double_range_doc(
            &[l, l + 1.0, l + 2.0],
            &[l + 10.0, l + 11.0, l + 12.0],
        )?)?;
    }

    let r = w.get_reader()?;
    w.close()?;

    let s = LuceneTestCase::new_searcher(Arc::clone(&r));
    let fc = match_all(&s)?;

    let facets = DoubleRangeOnRangeFacetCounts::new(
        "field",
        &fc,
        INTERSECTS,
        &[
            drm("less than 10", &[0.0, 1.0, 2.0], &[8.0, 9.0, 10.0])?,
            drm("a bit over 10", &[0.0, 1.0, 2.0], &[10.0, 11.0, 12.0])?,
            drm(
                "a bit under 90",
                &[88.0, 89.0, 90.0],
                &[100.0, 101.0, 102.0],
            )?,
            drm("over 90", &[90.0, 91.0, 92.0], &[100.0, 101.0, 102.0])?,
            drm(
                "over 1000",
                &[1000.0, 1001.0, 1002.0],
                &[f64::INFINITY - 2.0, f64::INFINITY - 1.0, f64::INFINITY],
            )?,
        ],
    )?;

    Assert::assert_equals(
        "dim=field path=[] value=33 childCount=5\n  less than 10 (9)\n  a bit over 10 (11)\n  a bit under 90 (22)\n  over 90 (20)\n  over 1000 (0)\n",
        all_children(&facets, "field")?.as_str(),
    );
    Assert::assert_equals(
        "dim=field path=[] value=33 childCount=4\n  a bit under 90 (22)\n  over 90 (20)\n  a bit over 10 (11)\n  less than 10 (9)\n",
        top_children(&facets, 4, "field")?.as_str(),
    );

    // test getTopChildren(0, dim)
    expect_throws(IAE, || facets.get_top_children(0, "field", &[]));

    r.close()?;
    d.close()
}

/// The assertions shared by the random tests: the results against the
/// expected counts, then a drill-down per range with
/// `drill_down(rangeID)`.
fn check_random_results(
    s: &IndexSearcher,
    config: &Arc<FacetsConfig>,
    facets: &dyn Facets,
    expected_counts: &[i32],
    mut expected_top_n_children_counts: Vec<i32>,
    drill_down: impl Fn(usize) -> Result<Arc<dyn Query>>,
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

/// `LongRange.verifyAndEncode(min, max, new byte[2 * Long.BYTES * dims])`.
fn encode_longs(min: &[i64], max: &[i64]) -> Result<Vec<u8>> {
    let mut packed = vec![0u8; 2 * 8 * min.len()];
    LongRangeField::verify_and_encode(min, max, &mut packed)?;
    Ok(packed)
}

/// `DoubleRange.verifyAndEncode(min, max, new byte[2 * Double.BYTES * dims])`.
fn encode_doubles(min: &[f64], max: &[f64]) -> Result<Vec<u8>> {
    let mut packed = vec![0u8; 2 * 8 * min.len()];
    DoubleRangeField::verify_and_encode(min, max, &mut packed)?;
    Ok(packed)
}

fn test_random_single_dim_longs() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let w = writer(&dir)?;

    let num_docs = LuceneTestCase::at_least(1000) as usize;
    if *VERBOSE {
        println!("TEST: numDocs={num_docs}");
    }
    let mut mins = vec![0i64; num_docs];
    let mut maxes = vec![0i64; num_docs];
    let mut min_value = i64::MAX;
    let mut max_value = i64::MIN;
    for i in 0..num_docs {
        let random = LuceneTestCase::random();
        let mut doc = Document::new();
        let mut min = random.next_long();
        let mut max = random.next_long();
        if min > max {
            std::mem::swap(&mut min, &mut max);
        }
        mins[i] = min;
        maxes[i] = max;
        doc.add(
            LongRangeDocValuesField::new("field", &[min], &[max])?
                .as_binary_range_doc_values_field()
                .clone(),
        );
        doc.add(LongRangeField::new("field", &[min], &[max])?);
        w.add_document(&doc)?;
        min_value = min_value.min(min);
        max_value = max_value.max(min);
    }
    // `minValue` / `maxValue` are computed and never read, as in Java.
    let _ = (min_value, max_value);
    let r = w.get_reader()?;

    let s = LuceneTestCase::new_searcher_wrap(Arc::clone(&r), false);
    let config = Arc::new(FacetsConfig::new());

    let num_iters = LuceneTestCase::at_least(10);
    for iter in 0..num_iters {
        if *VERBOSE {
            println!("TEST: iter={iter}");
        }
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
                    prev_range.min[0]
                } else {
                    prev_range.max[0]
                }
            } else {
                random.next_long()
            };
            let mut max = if range_id > 0 && random.next_int_bound(10) == 7 {
                // Use an existing boundary:
                let prev_range = &ranges[random.next_int_bound(range_id as i32) as usize];
                if random.next_boolean() {
                    prev_range.min[0]
                } else {
                    prev_range.max[0]
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
                // If max == min or max == min+1, we always do inclusive, else we might pass an
                // empty range and hit exc from LongRange's ctor:
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

            let query_packed_value = encode_longs(&[min], &[max])?;
            for i in 0..num_docs {
                let packed_value = encode_longs(&[mins[i]], &[maxes[i]])?;
                if INTERSECTS.matches(&query_packed_value, &packed_value, 1, 8) {
                    expected_counts[range_id] += 1;
                    expected_top_n_children_counts[range_id] += 1;
                    min_accepted_value = min_accepted_value.min(mins[i]);
                    max_accepted_value = max_accepted_value.max(maxes[i]);
                }
            }
        }

        let sfc = match_all(&s)?;
        let fast_match_query: Option<Arc<dyn Query>> = if random.next_boolean() {
            Some(Arc::new(LongRangeField::new_intersects_query(
                "field",
                &[min_accepted_value],
                &[max_accepted_value],
            )?))
        } else {
            None
        };

        let facets = LongRangeOnRangeFacetCounts::with_fast_match(
            "field",
            &sfc,
            INTERSECTS,
            fast_match_query,
            &ranges,
        )?;
        check_random_results(
            &s,
            &config,
            &facets,
            &expected_counts,
            expected_top_n_children_counts,
            |range_id| {
                let range = &ranges[range_id];
                Ok(Arc::new(LongRangeField::new_intersects_query(
                    "field", &range.min, &range.max,
                )?))
            },
        )?;
    }

    w.close()?;
    r.close()?;
    dir.close()
}

fn test_random_multi_dim_longs() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let w = writer(&dir)?;

    let num_docs = LuceneTestCase::at_least(1000) as usize;
    let dims = LuceneTestCase::random().next_int_range(2, 5) as usize;
    if *VERBOSE {
        println!("TEST: numDocs={num_docs}");
    }
    let mut mins = vec![vec![0i64; dims]; num_docs];
    let mut maxes = vec![vec![0i64; dims]; num_docs];
    for i in 0..num_docs {
        let random = LuceneTestCase::random();
        let mut doc = Document::new();
        for dim in 0..dims {
            let mut min = random.next_long();
            let mut max = random.next_long();
            if min > max {
                std::mem::swap(&mut min, &mut max);
            }
            mins[i][dim] = min;
            maxes[i][dim] = max;
        }
        doc.add(
            LongRangeDocValuesField::new("field", &mins[i], &maxes[i])?
                .as_binary_range_doc_values_field()
                .clone(),
        );
        doc.add(LongRangeField::new("field", &mins[i], &maxes[i])?);
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
        let random = LuceneTestCase::random();
        let num_range = TestUtil::next_int(&*random, 1, 100) as usize;
        let mut ranges: Vec<LongRange> = Vec::with_capacity(num_range);
        let mut expected_counts = vec![0i32; num_range];
        let mut expected_top_n_children_counts = vec![0i32; num_range];
        let mut min_accepted_value = vec![i64::MAX; dims];
        let mut max_accepted_value = vec![i64::MIN; dims];
        for range_id in 0..num_range {
            let mut min = vec![0i64; dims];
            if range_id > 0 && random.next_int_bound(10) == 7 {
                // Use an existing boundary:
                let prev_range = &ranges[random.next_int_bound(range_id as i32) as usize];
                for dim in 0..dims {
                    if random.next_boolean() {
                        min[dim] = prev_range.min[dim];
                    } else {
                        min[dim] = prev_range.max[dim];
                    }
                }
            } else {
                for v in min.iter_mut() {
                    *v = random.next_long();
                }
            }
            let mut max = vec![0i64; dims];
            if range_id > 0 && random.next_int_bound(10) == 7 {
                // Use an existing boundary:
                let prev_range = &ranges[random.next_int_bound(range_id as i32) as usize];
                for dim in 0..dims {
                    if random.next_boolean() {
                        max[dim] = prev_range.min[dim];
                    } else {
                        max[dim] = prev_range.max[dim];
                    }
                }
            } else {
                for v in max.iter_mut() {
                    *v = random.next_long();
                }
            }

            for dim in 0..dims {
                if min[dim] > max[dim] {
                    std::mem::swap(&mut min[dim], &mut max[dim]);
                }
            }

            ranges.push(LongRange::new_multi(
                &format!("r{range_id}"),
                min.clone(),
                max.clone(),
            )?);
            if *VERBOSE {
                println!("  range {range_id}: {}", ranges[range_id]);
            }

            let query_packed_value = encode_longs(&min, &max)?;
            for i in 0..num_docs {
                let packed_value = encode_longs(&mins[i], &maxes[i])?;
                if INTERSECTS.matches(&query_packed_value, &packed_value, dims, 8) {
                    expected_counts[range_id] += 1;
                    expected_top_n_children_counts[range_id] += 1;
                    for dim in 0..dims {
                        min_accepted_value[dim] = min_accepted_value[dim].min(mins[i][dim]);
                        max_accepted_value[dim] = max_accepted_value[dim].max(maxes[i][dim]);
                    }
                }
            }
        }

        let sfc = match_all(&s)?;
        let fast_match_query: Option<Arc<dyn Query>> = if random.next_boolean() {
            Some(Arc::new(LongRangeField::new_intersects_query(
                "field",
                &min_accepted_value,
                &max_accepted_value,
            )?))
        } else {
            None
        };

        let facets = LongRangeOnRangeFacetCounts::with_fast_match(
            "field",
            &sfc,
            INTERSECTS,
            fast_match_query,
            &ranges,
        )?;
        check_random_results(
            &s,
            &config,
            &facets,
            &expected_counts,
            expected_top_n_children_counts,
            |range_id| {
                let range = &ranges[range_id];
                Ok(Arc::new(LongRangeField::new_intersects_query(
                    "field", &range.min, &range.max,
                )?))
            },
        )?;
    }

    w.close()?;
    r.close()?;
    dir.close()
}

fn test_random_single_dim_doubles() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let w = writer(&dir)?;

    let num_docs = LuceneTestCase::at_least(1000) as usize;
    if *VERBOSE {
        println!("TEST: numDocs={num_docs}");
    }
    let mut mins = vec![0f64; num_docs];
    let mut maxes = vec![0f64; num_docs];
    let mut min_value = f64::INFINITY;
    let mut max_value = f64::NEG_INFINITY;
    for i in 0..num_docs {
        let random = LuceneTestCase::random();
        let mut doc = Document::new();
        let mut min = random.next_double();
        let mut max = random.next_double();
        if min > max {
            std::mem::swap(&mut min, &mut max);
        }
        mins[i] = min;
        maxes[i] = max;
        doc.add(
            DoubleRangeDocValuesField::new("field", &[min], &[max])?
                .as_binary_range_doc_values_field()
                .clone(),
        );
        doc.add(DoubleRangeField::new("field", &[min], &[max])?);
        w.add_document(&doc)?;
        min_value = min_value.min(min);
        max_value = max_value.max(min);
    }
    // `minValue` / `maxValue` are computed and never read, as in Java.
    let _ = (min_value, max_value);
    let r = w.get_reader()?;

    let s = LuceneTestCase::new_searcher_wrap(Arc::clone(&r), false);
    let config = Arc::new(FacetsConfig::new());

    let num_iters = LuceneTestCase::at_least(10);
    for iter in 0..num_iters {
        if *VERBOSE {
            println!("TEST: iter={iter}");
        }
        let random = LuceneTestCase::random();
        let num_range = TestUtil::next_int(&*random, 1, 100) as usize;
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
                    prev_range.min[0]
                } else {
                    prev_range.max[0]
                }
            } else {
                random.next_double()
            };
            let mut max = if range_id > 0 && random.next_int_bound(10) == 7 {
                // Use an existing boundary:
                let prev_range = &ranges[random.next_int_bound(range_id as i32) as usize];
                if random.next_boolean() {
                    prev_range.min[0]
                } else {
                    prev_range.max[0]
                }
            } else {
                random.next_double()
            };

            if min > max {
                std::mem::swap(&mut min, &mut max);
            }
            // NOTE: max - min >= 0 is here to handle the common overflow case!
            let (min_incl, max_incl) = if max - min >= 0.0 && max - min < 2.0 {
                // If max == min or max == min+1, we always do inclusive, else we might pass an
                // empty range and hit exc from LongRange's ctor:
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
            if *VERBOSE {
                println!("  range {range_id}: {}", ranges[range_id]);
            }

            let query_packed_value = encode_doubles(&[min], &[max])?;
            for i in 0..num_docs {
                let packed_value = encode_doubles(&[mins[i]], &[maxes[i]])?;
                if INTERSECTS.matches(&query_packed_value, &packed_value, 1, 8) {
                    expected_counts[range_id] += 1;
                    expected_top_n_children_counts[range_id] += 1;
                    min_accepted_value = min_accepted_value.min(mins[i]);
                    max_accepted_value = max_accepted_value.max(maxes[i]);
                }
            }
        }

        let sfc = match_all(&s)?;
        let fast_match_query: Option<Arc<dyn Query>> = if random.next_boolean() {
            Some(Arc::new(DoubleRangeField::new_intersects_query(
                "field",
                &[min_accepted_value],
                &[max_accepted_value],
            )?))
        } else {
            None
        };

        let facets = DoubleRangeOnRangeFacetCounts::with_fast_match(
            "field",
            &sfc,
            INTERSECTS,
            fast_match_query,
            &ranges,
        )?;
        check_random_results(
            &s,
            &config,
            &facets,
            &expected_counts,
            expected_top_n_children_counts,
            |range_id| {
                let range = &ranges[range_id];
                Ok(Arc::new(DoubleRangeField::new_intersects_query(
                    "field", &range.min, &range.max,
                )?))
            },
        )?;
    }

    w.close()?;
    r.close()?;
    dir.close()
}

fn test_random_multi_dim_doubles() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let w = writer(&dir)?;

    let num_docs = LuceneTestCase::at_least(1000) as usize;
    let dims = LuceneTestCase::random().next_int_range(2, 5) as usize;
    if *VERBOSE {
        println!("TEST: numDocs={num_docs}");
    }
    let mut mins = vec![vec![0f64; dims]; num_docs];
    let mut maxes = vec![vec![0f64; dims]; num_docs];
    for i in 0..num_docs {
        let random = LuceneTestCase::random();
        let mut doc = Document::new();
        for dim in 0..dims {
            let mut min = random.next_double();
            let mut max = random.next_double();
            if min > max {
                std::mem::swap(&mut min, &mut max);
            }
            mins[i][dim] = min;
            maxes[i][dim] = max;
        }
        doc.add(
            DoubleRangeDocValuesField::new("field", &mins[i], &maxes[i])?
                .as_binary_range_doc_values_field()
                .clone(),
        );
        doc.add(DoubleRangeField::new("field", &mins[i], &maxes[i])?);
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
        let random = LuceneTestCase::random();
        let num_range = TestUtil::next_int(&*random, 1, 100) as usize;
        let mut ranges: Vec<DoubleRange> = Vec::with_capacity(num_range);
        let mut expected_counts = vec![0i32; num_range];
        let mut expected_top_n_children_counts = vec![0i32; num_range];
        let mut min_accepted_value = vec![f64::INFINITY; dims];
        let mut max_accepted_value = vec![f64::NEG_INFINITY; dims];
        for range_id in 0..num_range {
            let mut min = vec![0f64; dims];
            if range_id > 0 && random.next_int_bound(10) == 7 {
                // Use an existing boundary:
                let prev_range = &ranges[random.next_int_bound(range_id as i32) as usize];
                for dim in 0..dims {
                    if random.next_boolean() {
                        min[dim] = prev_range.min[dim];
                    } else {
                        min[dim] = prev_range.max[dim];
                    }
                }
            } else {
                for v in min.iter_mut() {
                    *v = random.next_double();
                }
            }
            let mut max = vec![0f64; dims];
            if range_id > 0 && random.next_int_bound(10) == 7 {
                // Use an existing boundary:
                let prev_range = &ranges[random.next_int_bound(range_id as i32) as usize];
                for dim in 0..dims {
                    if random.next_boolean() {
                        max[dim] = prev_range.min[dim];
                    } else {
                        max[dim] = prev_range.max[dim];
                    }
                }
            } else {
                for v in max.iter_mut() {
                    *v = random.next_double();
                }
            }

            for dim in 0..dims {
                if min[dim] > max[dim] {
                    std::mem::swap(&mut min[dim], &mut max[dim]);
                }
            }

            ranges.push(DoubleRange::new_multi(
                &format!("r{range_id}"),
                min.clone(),
                max.clone(),
            )?);
            if *VERBOSE {
                println!("  range {range_id}: {}", ranges[range_id]);
            }

            let query_packed_value = encode_doubles(&min, &max)?;
            for i in 0..num_docs {
                let packed_value = encode_doubles(&mins[i], &maxes[i])?;
                if INTERSECTS.matches(&query_packed_value, &packed_value, dims, 8) {
                    expected_counts[range_id] += 1;
                    expected_top_n_children_counts[range_id] += 1;
                    for dim in 0..dims {
                        min_accepted_value[dim] = min_accepted_value[dim].min(mins[i][dim]);
                        max_accepted_value[dim] = max_accepted_value[dim].max(maxes[i][dim]);
                    }
                }
            }
        }

        let sfc = match_all(&s)?;
        let fast_match_query: Option<Arc<dyn Query>> = if random.next_boolean() {
            Some(Arc::new(DoubleRangeField::new_intersects_query(
                "field",
                &min_accepted_value,
                &max_accepted_value,
            )?))
        } else {
            None
        };

        let facets = DoubleRangeOnRangeFacetCounts::with_fast_match(
            "field",
            &sfc,
            INTERSECTS,
            fast_match_query,
            &ranges,
        )?;
        check_random_results(
            &s,
            &config,
            &facets,
            &expected_counts,
            expected_top_n_children_counts,
            |range_id| {
                let range = &ranges[range_id];
                Ok(Arc::new(DoubleRangeField::new_intersects_query(
                    "field", &range.min, &range.max,
                )?))
            },
        )?;
    }

    w.close()?;
    r.close()?;
    dir.close()
}

fn test_missing_values() -> Result<()> {
    let d = LuceneTestCase::new_directory();
    let w = writer(&d)?;
    for l in 0..100i64 {
        if l % 5 == 0 {
            // Every 5th doc is missing the value:
            w.add_document(&Document::new())?;
            continue;
        }
        w.add_document(&long_range_doc(&[l], &[l + 10])?)?;
    }

    let r = w.get_reader()?;

    let s = LuceneTestCase::new_searcher(Arc::clone(&r));
    let fc = match_all(&s)?;
    let facets =
        LongRangeOnRangeFacetCounts::new("field", &fc, INTERSECTS, &basic_long_ranges(false)?)?;

    Assert::assert_equals(
        "dim=field path=[] value=24 childCount=5\n  less than 10 (8)\n  less than or equal to 10 (8)\n  over 90 (16)\n  90 or above (16)\n  over 1000 (0)\n",
        all_children(&facets, "field")?.as_str(),
    );
    Assert::assert_equals(
        "dim=field path=[] value=24 childCount=4\n  90 or above (16)\n  over 90 (16)\n  less than 10 (8)\n  less than or equal to 10 (8)\n",
        top_children(&facets, 4, "field")?.as_str(),
    );

    w.close()?;
    r.close()?;
    d.close()
}

fn test_long_range_equals() -> Result<()> {
    let r = |max: i64, max_incl: bool| LongRange::new("field", -7, true, max, max_incl).unwrap();
    let m = |min: &[i64], max: &[i64]| lrm("field", min, max).unwrap();
    assert_equals(&r(17, false), &r(17, false));
    Assert::assert_equals(r(17, false).hash_code(), r(17, false).hash_code());
    assert_not_equals(&r(17, false), &r(17, true));
    assert_not_equals(&r(17, false).hash_code(), &r(17, true).hash_code());
    assert_not_equals(&r(17, false), &r(18, false));
    assert_not_equals(&r(17, false).hash_code(), &r(18, false).hash_code());
    // mutlidim tests
    assert_equals(&m(&[-7, -8], &[17, 18]), &m(&[-7, -8], &[17, 18]));
    Assert::assert_equals(
        m(&[-7, -8], &[17, 18]).hash_code(),
        m(&[-7, -8], &[17, 18]).hash_code(),
    );
    assert_not_equals(
        &m(&[-7, -8, -9], &[17, 18, 19]).hash_code(),
        &m(&[-7, -8], &[17, 18]).hash_code(),
    );
    assert_not_equals(
        &m(&[-7, -9], &[17, 18]).hash_code(),
        &m(&[-7, -8], &[17, 18]).hash_code(),
    );
    assert_not_equals(
        &m(&[-7, -8], &[17, 19]).hash_code(),
        &m(&[-7, -8], &[17, 18]).hash_code(),
    );
    Ok(())
}

fn test_double_range_equals() -> Result<()> {
    let r =
        |max: f64, max_incl: bool| DoubleRange::new("field", -7.0, true, max, max_incl).unwrap();
    let m = |min: &[f64], max: &[f64]| drm("field", min, max).unwrap();
    assert_equals(&r(17.0, false), &r(17.0, false));
    Assert::assert_equals(r(17.0, false).hash_code(), r(17.0, false).hash_code());
    assert_not_equals(&r(17.0, false), &r(17.0, true));
    assert_not_equals(&r(17.0, false).hash_code(), &r(17.0, true).hash_code());
    assert_not_equals(&r(17.0, false), &r(18.0, false));
    assert_not_equals(&r(17.0, false).hash_code(), &r(18.0, false).hash_code());
    // multidim tests
    assert_equals(
        &m(&[-7.0, -8.0], &[17.0, 18.0]),
        &m(&[-7.0, -8.0], &[17.0, 18.0]),
    );
    Assert::assert_equals(
        m(&[-7.0, -8.0], &[17.0, 18.0]).hash_code(),
        m(&[-7.0, -8.0], &[17.0, 18.0]).hash_code(),
    );
    assert_not_equals(
        &m(&[-7.0, -8.0, -9.0], &[17.0, 18.0, 19.0]),
        &m(&[-7.0, -8.0], &[17.0, 18.0]),
    );
    assert_not_equals(
        &m(&[-7.0, -9.0], &[17.0, 18.0]),
        &m(&[-7.0, -8.0], &[17.0, 18.0]),
    );
    assert_not_equals(
        &m(&[-7.0, -8.0], &[17.0, 19.0]),
        &m(&[-7.0, -8.0], &[17.0, 18.0]),
    );
    Ok(())
}
