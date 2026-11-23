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

//! Helpers shared by the ports of the tests of Lucene 10.5.0
//! `lucene/facet/src/test` (sprint 17, rmp #630): the suite plumbing of the
//! sprint-9 core test ports (`core_support`, `search_support`), reused by
//! path, and the class `org.apache.lucene.facet.FacetTestCase`.

#![allow(dead_code, unused_imports)]

#[path = "../../../rucene-test-framework/tests/core_support/mod.rs"]
pub mod core_support;

#[path = "../../../rucene-test-framework/tests/core_search/support.rs"]
pub mod search_support;

use std::sync::{Arc, LazyLock};

use randomizedtesting::jdk::reflect::{Class, Method};
use randomizedtesting::junit::Assert;
use rucene::document::NumericValue;
use rucene::error::Result;
use rucene::search::doc_id_set_iterator::NO_MORE_DOCS;
use rucene_facet::facet_result::FacetResult;
use rucene_facet::facets::Facets;
use rucene_facet::facets_collector::FacetsCollector;
use rucene_facet::facets_config::{FacetsConfig, DEFAULT_INDEX_FIELD_NAME};
use rucene_facet::label_and_value::{number_double_value, number_float_value, LabelAndValue};
use rucene_facet::taxonomy::facet_label::FacetLabel;
use rucene_facet::taxonomy::fast_taxonomy_facet_counts::FastTaxonomyFacetCounts;
use rucene_facet::taxonomy::taxonomy_facet_labels::{FacetLabelReader, TaxonomyFacetLabels};
use rucene_facet::taxonomy::taxonomy_reader::TaxonomyReader;
use rucene_test_framework::util::lucene_test_case::VERBOSE;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

/// The class `org.apache.lucene.facet.FacetTestCase` (`public abstract
/// class FacetTestCase extends LuceneTestCase`). Its helper methods are the
/// free functions of this module.
fn facet_test_case_class() -> Arc<Class> {
    static CLASS: LazyLock<Arc<Class>> = LazyLock::new(|| {
        Class::builder("org.apache.lucene.facet.FacetTestCase")
            .extends(&LuceneTestCase::class())
            .build()
    });
    Arc::clone(&CLASS)
}

/// A test class `name` extending `FacetTestCase`, with static `methods`.
pub fn facet_class(name: &str, methods: Vec<Method>) -> Arc<Class> {
    let mut builder = Class::builder(name)
        .extends(&facet_test_case_class())
        .constructor(randomizedtesting::jdk::reflect::Constructor::no_args(|| ()));
    for m in methods {
        builder = builder.method(m);
    }
    builder.build()
}

/// [`facet_class`] with class-level annotations (`@SuppressCodecs`, ...).
pub fn facet_class_annotated(
    name: &str,
    methods: Vec<Method>,
    annotate: impl FnOnce(
        randomizedtesting::jdk::reflect::ClassBuilder,
    ) -> randomizedtesting::jdk::reflect::ClassBuilder,
) -> Arc<Class> {
    let mut builder = annotate(
        Class::builder(name)
            .extends(&facet_test_case_class())
            .constructor(randomizedtesting::jdk::reflect::Constructor::no_args(|| ())),
    );
    for m in methods {
        builder = builder.method(m);
    }
    builder.build()
}

/// A test class `name` extending `FacetTestCase` whose instance is `F`
/// (`F::default()`, Java's field initialisers), with `methods`.
pub fn facet_fixture_class<F: std::any::Any + Send + Default>(
    name: &str,
    methods: Vec<Method>,
) -> Arc<Class> {
    fixture_class_extending::<F>(name, &facet_test_case_class(), methods)
}

/// A test class `name` extending `parent` whose instance is `F`.
pub fn fixture_class_extending<F: std::any::Any + Send + Default>(
    name: &str,
    parent: &Arc<Class>,
    methods: Vec<Method>,
) -> Arc<Class> {
    let mut builder = Class::builder(name).extends(parent).constructor(
        randomizedtesting::jdk::reflect::Constructor::no_args(F::default),
    );
    for m in methods {
        builder = builder.method(m);
    }
    builder.build()
}

/// An abstract class `name` extending `parent` (an intermediate base test
/// class such as `MultiValuesSourceTestCase`).
pub fn abstract_class(name: &str, parent: &Arc<Class>) -> Arc<Class> {
    Class::builder(name).extends(parent).build()
}

/// `assertEquals(Object, Object)` of objects compared by `equals`, `eq`
/// being `expected.equals(actual)`.
pub fn assert_object_equals(
    eq: bool,
    expected: &dyn std::fmt::Debug,
    actual: &dyn std::fmt::Debug,
) {
    if !eq {
        Assert::fail(&format!("expected:<{expected:?}> but was:<{actual:?}>"));
    }
}

/// `assertNotEquals(Object, Object)` of objects compared by `equals`.
pub fn assert_object_not_equals(eq: bool, unexpected: &dyn std::fmt::Debug) {
    if eq {
        Assert::fail(&format!(
            "Values should be different. Actual: {unexpected:?}"
        ));
    }
}

/// `getTaxonomyFacetCounts(TaxonomyReader, FacetsConfig, FacetsCollector)`.
pub fn get_taxonomy_facet_counts(
    taxo_reader: &Arc<dyn TaxonomyReader>,
    config: &Arc<FacetsConfig>,
    c: &FacetsCollector,
) -> Result<Arc<dyn Facets>> {
    get_taxonomy_facet_counts_field(taxo_reader, config, c, DEFAULT_INDEX_FIELD_NAME)
}

/// `getTaxonomyFacetCounts(TaxonomyReader, FacetsConfig, FacetsCollector,
/// String indexFieldName)`.
pub fn get_taxonomy_facet_counts_field(
    taxo_reader: &Arc<dyn TaxonomyReader>,
    config: &Arc<FacetsConfig>,
    c: &FacetsCollector,
    index_field_name: &str,
) -> Result<Arc<dyn Facets>> {
    Ok(Arc::new(FastTaxonomyFacetCounts::with_index_field_name(
        index_field_name,
        Arc::clone(taxo_reader),
        Arc::clone(config),
        c,
    )?))
}

/// `getAllTaxonomyFacetLabels(String dimension, TaxonomyReader,
/// FacetsCollector)`.
pub fn get_all_taxonomy_facet_labels(
    dimension: Option<&str>,
    taxo_reader: &Arc<dyn TaxonomyReader>,
    fc: &FacetsCollector,
) -> Result<Vec<Vec<FacetLabel>>> {
    let mut actual_labels = Vec::new();
    let taxo_labels = TaxonomyFacetLabels::new(Arc::clone(taxo_reader), DEFAULT_INDEX_FIELD_NAME);
    for m in fc.get_matching_docs()? {
        let mut facet_label_reader = taxo_labels.get_facet_label_reader(m.context())?;
        let mut disi = m.bits().iterator()?;
        loop {
            let doc = disi.next_doc()?;
            if doc == NO_MORE_DOCS {
                break;
            }
            actual_labels.push(all_facet_labels(doc, dimension, &mut facet_label_reader)?);
        }
    }
    Ok(actual_labels)
}

/// `allFacetLabels(int docId, String dimension, FacetLabelReader)`.
pub fn all_facet_labels(
    doc_id: i32,
    dimension: Option<&str>,
    facet_label_reader: &mut FacetLabelReader,
) -> Result<Vec<FacetLabel>> {
    let mut facet_labels = Vec::new();
    match dimension {
        Some(dimension) => {
            while let Some(facet_label) =
                facet_label_reader.next_facet_label_for_dimension(doc_id, dimension)?
            {
                facet_labels.push(facet_label);
            }
        }
        None => {
            while let Some(facet_label) = facet_label_reader.next_facet_label(doc_id)? {
                facet_labels.push(facet_label);
            }
        }
    }
    Ok(facet_labels)
}

/// `getRandomTokens(int count)`.
pub fn get_random_tokens(count: usize) -> Vec<String> {
    let random = LuceneTestCase::random();
    (0..count)
        .map(|_| TestUtil::random_realistic_unicode_string_between(&*random, 1, 10))
        .collect()
}

/// `pickToken(String[] tokens)`.
pub fn pick_token(tokens: &[String]) -> String {
    let random = LuceneTestCase::random();
    for token in tokens {
        if random.next_boolean() {
            return token.clone();
        }
    }
    // Move long tail onto first token:
    tokens[0].clone()
}

/// `protected static class TestDoc`.
#[derive(Clone, Debug, Default)]
pub struct TestDoc {
    /// `content`.
    pub content: String,
    /// `dims`: `null` entries are `None`.
    pub dims: Vec<Option<String>>,
    /// `value`.
    pub value: f32,
}

/// `getRandomDocs(String[] tokens, int count, int numDims)`.
pub fn get_random_docs(tokens: &[String], count: usize, num_dims: usize) -> Vec<TestDoc> {
    let random = LuceneTestCase::random();
    let mut docs = Vec::new();
    for i in 0..count {
        let mut doc = TestDoc {
            content: pick_token(tokens),
            dims: vec![None; num_dims],
            value: 0.0,
        };
        for dim in doc.dims.iter_mut() {
            *dim = Some(pick_token(tokens));
            if random.next_int_bound(10) < 3 {
                break;
            }
        }
        if *VERBOSE {
            println!("  doc {i}: content={}", doc.content);
            for (j, dim) in doc.dims.iter().enumerate() {
                if let Some(dim) = dim {
                    println!("    dim[{j}]={dim}");
                }
            }
        }
        docs.push(doc);
    }
    docs
}

/// `sortTies(List<FacetResult> results)`.
pub fn sort_ties_results(results: &mut [FacetResult]) {
    for result in results {
        sort_ties(&mut result.label_values);
    }
}

/// `sortTies(LabelAndValue[] labelValues)`.
pub fn sort_ties(label_values: &mut [LabelAndValue]) {
    let mut last_value = -1.0f64;
    let mut num_in_row = 0usize;
    let mut i = 0usize;
    while i <= label_values.len() {
        if i < label_values.len() && number_double_value(&label_values[i].value) == last_value {
            num_in_row += 1;
        } else {
            if num_in_row > 1 {
                label_values[i - num_in_row..i].sort_by(|a, b| {
                    assert!(number_double_value(&a.value) == number_double_value(&b.value));
                    a.label.as_bytes().cmp(b.label.as_bytes())
                });
            }
            num_in_row = 1;
            if i < label_values.len() {
                last_value = number_double_value(&label_values[i].value);
            }
        }
        i += 1;
    }
}

/// `sortLabelValues(List<LabelAndValue> labelValues)`.
pub fn sort_label_values(label_values: &mut [LabelAndValue]) {
    label_values.sort_by(|a, b| {
        let (av, bv) = (number_double_value(&a.value), number_double_value(&b.value));
        if av > bv {
            std::cmp::Ordering::Less
        } else if av < bv {
            std::cmp::Ordering::Greater
        } else {
            a.label.as_bytes().cmp(b.label.as_bytes())
        }
    });
}

/// `sortFacetResults(List<FacetResult> results)`.
pub fn sort_facet_results(results: &mut [FacetResult]) {
    results.sort_by(|a, b| {
        let (av, bv) = (number_double_value(&a.value), number_double_value(&b.value));
        if av > bv {
            std::cmp::Ordering::Less
        } else if bv > av {
            std::cmp::Ordering::Greater
        } else {
            a.dim.encode_utf16().cmp(b.dim.encode_utf16())
        }
    });
}

/// `assertFloatValuesEquals(List<FacetResult> a, List<FacetResult> b)`.
pub fn assert_float_values_equals_list(a: &[FacetResult], b: &[FacetResult]) {
    Assert::assert_equals(a.len(), b.len());
    let mut last_value = f32::INFINITY;
    let mut a_by_dim = std::collections::HashMap::new();
    for r in a {
        Assert::assert_true(number_float_value(&r.value) <= last_value);
        last_value = number_float_value(&r.value);
        a_by_dim.insert(r.dim.clone(), r);
    }
    last_value = f32::INFINITY;
    let mut b_by_dim = std::collections::HashMap::new();
    for r in b {
        b_by_dim.insert(r.dim.clone(), r);
        Assert::assert_true(number_float_value(&r.value) <= last_value);
        last_value = number_float_value(&r.value);
    }
    for (dim, ra) in &a_by_dim {
        let rb = b_by_dim
            .get(dim)
            .unwrap_or_else(|| panic!("java.lang.NullPointerException: no result for dim {dim}"));
        assert_float_values_equals(ra, rb);
    }
}

/// `assertFloatValuesEquals(FacetResult a, FacetResult b)`.
pub fn assert_float_values_equals(a: &FacetResult, b: &FacetResult) {
    Assert::assert_equals(a.dim.as_str(), b.dim.as_str());
    Assert::assert_true(a.path == b.path);
    Assert::assert_equals(a.child_count, b.child_count);
    assert_numeric_values_equals(&a.value, &b.value);
    Assert::assert_equals(a.label_values.len(), b.label_values.len());
    for (la, lb) in a.label_values.iter().zip(&b.label_values) {
        Assert::assert_equals(la.label.as_str(), lb.label.as_str());
        assert_numeric_values_equals(&la.value, &lb.value);
    }
}

/// `assertNumericValuesEquals(Number a, Number b)`.
pub fn assert_numeric_values_equals(a: &NumericValue, b: &NumericValue) {
    Assert::assert_true(std::mem::discriminant(a) == std::mem::discriminant(b));
    match (a, b) {
        (NumericValue::Float(x), NumericValue::Float(y)) => {
            Assert::assert_equals_float_msg(None, *x, *y, *x / 1e5)
        }
        (NumericValue::Double(x), NumericValue::Double(y)) => {
            Assert::assert_equals_double_msg(None, *x, *y, *x / 1e5)
        }
        _ => Assert::assert_equals(*a, *b),
    }
}

/// `assertFacetResult(FacetResult, String expectedDim, String[] expectedPath,
/// int expectedChildCount, Number expectedValue, LabelAndValue...
/// expectedChildren)`.
pub fn assert_facet_result(
    result: &FacetResult,
    expected_dim: &str,
    expected_path: &[&str],
    expected_child_count: i32,
    expected_value: NumericValue,
    expected_children: &[LabelAndValue],
) {
    Assert::assert_equals(expected_dim, result.dim.as_str());
    Assert::assert_true_msg(
        &format!("arrays differ: {expected_path:?} vs {:?}", result.path),
        expected_path
            .iter()
            .copied()
            .eq(result.path.iter().map(String::as_str)),
    );
    Assert::assert_equals(expected_child_count, result.child_count);
    assert_numeric_values_equals(&expected_value, &result.value);
    Assert::assert_equals(expected_children.len(), result.label_values.len());
    // assert children equal with no assumption of the children ordering
    Assert::assert_true(
        expected_children
            .iter()
            .all(|c| result.label_values.contains(c)),
    );
}

/// `assertFacetResult(..., Map<String, Integer> countPerLabel,
/// LabelAndValue... expectedChildren)`.
pub fn assert_facet_result_counts(
    result: &FacetResult,
    expected_dim: &str,
    expected_path: &[&str],
    expected_child_count: i32,
    expected_value: NumericValue,
    count_per_label: &std::collections::HashMap<String, i32>,
    expected_children: &[LabelAndValue],
) {
    assert_facet_result(
        result,
        expected_dim,
        expected_path,
        expected_child_count,
        expected_value,
        expected_children,
    );
    Assert::assert_equals(result.label_values.len(), count_per_label.len());
    for lv in &result.label_values {
        let expected = *count_per_label
            .get(&lv.label)
            .unwrap_or_else(|| panic!("java.lang.NullPointerException: no count for {}", lv.label));
        Assert::assert_equals(lv.count, expected);
    }
}
