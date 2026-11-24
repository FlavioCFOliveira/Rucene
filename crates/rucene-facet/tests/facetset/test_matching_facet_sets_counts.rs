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

//! Port of `org.apache.lucene.facet.facetset.TestMatchingFacetSetsCounts`.

use std::sync::Arc;

use randomizedtesting::generators::RandomNumbers;
use randomizedtesting::jdk::reflect::Class;
use rucene::document::{Document, NumericValue};
use rucene::error::Result;
use rucene::search::MatchAllDocsQuery;
use rucene_facet::facets::Facets;
use rucene_facet::facets_collector_manager::FacetsCollectorManager;
use rucene_facet::facetset::exact_facet_set_matcher::ExactFacetSetMatcher;
use rucene_facet::facetset::facet_set_decoder::decode_longs;
use rucene_facet::facetset::facet_set_matcher::FacetSetMatcher;
use rucene_facet::facetset::facet_sets_field::FacetSetsField;
use rucene_facet::facetset::long_facet_set::LongFacetSet;
use rucene_facet::facetset::matching_facet_sets_counts::MatchingFacetSetsCounts;
use rucene_facet::label_and_value::LabelAndValue;
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::LuceneTestCase;

use crate::facet_support::core_support::{expect_throws_panic, test};
use crate::facet_support::{assert_facet_result, facet_class};

const FORD_ORD: i32 = 100;
const TOYOTA_ORD: i32 = 101;
const CHEVY_ORD: i32 = 102;
const NISSAN_ORD: i32 = 103;
const MANUFACTURER_ORDS: [i32; 4] = [FORD_ORD, TOYOTA_ORD, CHEVY_ORD, NISSAN_ORD];

/// `TestMatchingFacetSetsCounts`.
pub fn class_def() -> Arc<Class> {
    facet_class(
        "org.apache.lucene.facet.facetset.TestMatchingFacetSetsCounts",
        vec![
            test("testInvalidTopN", test_invalid_top_n),
            test(
                "testInconsistentNumOfIndexedDimensions",
                test_inconsistent_num_of_indexed_dimensions,
            ),
            test("testTopChildren", test_top_children),
        ],
    )
}

fn long_set(values: &[i64]) -> Result<LongFacetSet> {
    LongFacetSet::new(values.to_vec())
}

fn exact(label: &str, values: &[i64]) -> Result<Arc<dyn FacetSetMatcher>> {
    Ok(Arc::new(ExactFacetSetMatcher::new(
        label,
        &long_set(values)?,
    )?))
}

fn test_invalid_top_n() -> Result<()> {
    let d = LuceneTestCase::new_directory();
    let w =
        RandomIndexWriter::new_with_mock_analyzer(&*LuceneTestCase::random(), d.as_directory())?;

    let mut doc = Document::new();
    doc.add(FacetSetsField::create("field", &[&long_set(&[123, 456])?])?);
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
        vec![exact("Test", &[123, 456])?],
    )?;

    expect_throws("java.lang.IllegalArgumentException", || {
        facets.get_top_children(0, "field", &[])
    });

    r.close()?;
    d.close()
}

fn test_inconsistent_num_of_indexed_dimensions() -> Result<()> {
    let d = LuceneTestCase::new_directory();
    let w =
        RandomIndexWriter::new_with_mock_analyzer(&*LuceneTestCase::random(), d.as_directory())?;

    let mut doc = Document::new();
    doc.add(FacetSetsField::create("field", &[&long_set(&[123, 456])?])?);
    w.add_document(&doc)?;

    let mut doc = Document::new();
    doc.add(FacetSetsField::create("field", &[&long_set(&[123])?])?);
    w.add_document(&doc)?;

    let r = w.get_reader()?;
    w.close()?;

    let s = LuceneTestCase::new_searcher(Arc::clone(&r));
    let fc = s.search_with_manager(
        Arc::new(MatchAllDocsQuery::instance()),
        &FacetsCollectorManager::new(),
    )?;

    expect_throws_panic("java.lang.AssertionError", || {
        let _ = MatchingFacetSetsCounts::new(
            "field",
            &fc,
            Arc::new(decode_longs),
            vec![exact("Test", &[1]).unwrap()],
        );
    });

    r.close()?;
    d.close()
}

fn label(day_bin: usize, ord_index: usize) -> String {
    format!("{}:{}", day_bin, MANUFACTURER_ORDS[ord_index])
}

fn test_top_children() -> Result<()> {
    let d = LuceneTestCase::new_directory();
    let w =
        RandomIndexWriter::new_with_mock_analyzer(&*LuceneTestCase::random(), d.as_directory())?;

    // As a test scenario, we're faceting on the number of vehicles produced per day/make
    // combination over the past 30 days:
    let num_bins = 30usize;

    let mut expected_counts = vec![0i32; num_bins * MANUFACTURER_ORDS.len()];
    let mut facet_set_matchers: Vec<Arc<dyn FacetSetMatcher>> = Vec::new();

    let mut total_docs = 0;
    let mut total_non_zero_bins = 0;
    let mut index = 0;
    for i in 0..num_bins {
        for ord in MANUFACTURER_ORDS {
            facet_set_matchers.push(exact(&format!("{i}:{ord}"), &[i as i64, ord as i64])?);

            let cars_manufactured =
                RandomNumbers::random_int_between(&*LuceneTestCase::random(), 0, 100);
            for _ in 0..cars_manufactured {
                // Create a document for every vehicle produced:
                let mut doc = Document::new();
                doc.add(FacetSetsField::create(
                    "field",
                    &[&long_set(&[i as i64, ord as i64])?],
                )?);
                w.add_document(&doc)?;
            }

            if cars_manufactured > 0 {
                total_non_zero_bins += 1;
            }
            total_docs += cars_manufactured;
            expected_counts[index] = cars_manufactured;
            index += 1;
        }
    }

    let r = w.get_reader()?;
    w.close()?;

    let s = LuceneTestCase::new_searcher(Arc::clone(&r));
    let fc = s.search_with_manager(
        Arc::new(MatchAllDocsQuery::instance()),
        &FacetsCollectorManager::new(),
    )?;

    let facets =
        MatchingFacetSetsCounts::new("field", &fc, Arc::new(decode_longs), facet_set_matchers)?;

    // Sort by count (high-to-low) and tie-break on label, same as in
    // MatchingFacetCounts#getTopChildren (`InPlaceMergeSorter`, a stable
    // sort, over the parallel `expectedCounts` / `originalIndexes`):
    let n = MANUFACTURER_ORDS.len();
    let mut pairs: Vec<(i32, usize)> = expected_counts
        .iter()
        .copied()
        .zip(0..expected_counts.len())
        .collect();
    pairs.sort_by(|&(count_i, orig_i), &(count_j, orig_j)| {
        count_j.cmp(&count_i).then_with(|| {
            let label_i = label(orig_i / n, orig_i % n);
            let label_j = label(orig_j / n, orig_j % n);
            label_i.as_bytes().cmp(label_j.as_bytes())
        })
    });

    let top_n = 10usize;
    let mut expected = Vec::with_capacity(top_n);
    for &(count, original_index) in pairs.iter().take(top_n) {
        let day_bin = original_index / n;
        let ord_index = original_index % n;
        expected.push(LabelAndValue::new(
            label(day_bin, ord_index),
            NumericValue::Int(count),
        ));
    }

    let result = facets
        .get_top_children(top_n as i32, "field", &[])?
        .unwrap();
    assert_facet_result(
        &result,
        "field",
        &[],
        total_non_zero_bins,
        NumericValue::Int(total_docs),
        &expected,
    );

    r.close()?;
    d.close()
}
