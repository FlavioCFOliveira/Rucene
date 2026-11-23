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

//! Port of `org.apache.lucene.facet.taxonomy.TestTaxonomyFacetAssociations`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::{AfterClass, Assert, BeforeClass};
use rucene::document::{Document, NumericValue, Store, TextField};
use rucene::error::Result;
use rucene::index::{DirectoryReader, DocValues, Term};
use rucene::search::query::Query;
use rucene::search::term_query::TermQuery;
use rucene::search::{MatchAllDocsQuery, NO_MORE_DOCS};
use rucene::store::Directory;
use rucene::util::java_collections::JavaHashMap;
use rucene_facet::drill_down_query::DrillDownQuery;
use rucene_facet::facet_field::FacetField;
use rucene_facet::facet_result::FacetResult;
use rucene_facet::facets::Facets;
use rucene_facet::facets_collector::FacetsCollector;
use rucene_facet::facets_collector_manager::FacetsCollectorManager;
use rucene_facet::facets_config::FacetsConfig;
use rucene_facet::label_and_value::{
    number_float_value, number_int_value, number_long_value, LabelAndValue,
};
use rucene_facet::taxonomy::association_aggregation_function::{
    AssociationAggregationFunction, MaxAggregation, SumAggregation,
};
use rucene_facet::taxonomy::directory::directory_taxonomy_reader::DirectoryTaxonomyReader;
use rucene_facet::taxonomy::directory::directory_taxonomy_writer::DirectoryTaxonomyWriter;
use rucene_facet::taxonomy::float_association_facet_field::FloatAssociationFacetField;
use rucene_facet::taxonomy::int_association_facet_field::IntAssociationFacetField;
use rucene_facet::taxonomy::taxonomy_facet_float_associations::TaxonomyFacetFloatAssociations;
use rucene_facet::taxonomy::taxonomy_facet_int_associations::TaxonomyFacetIntAssociations;
use rucene_facet::taxonomy::taxonomy_reader::TaxonomyReader;
use rucene_facet::taxonomy::taxonomy_writer::TaxonomyWriter;
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::LuceneTestCase;

use crate::facet_support::core_support::{assert_equals, test};
use crate::facet_support::search_support::static_hook;
use crate::facet_support::{assert_facet_result, assert_facet_result_counts, facet_class};

const IAE: &str = "java.lang.IllegalArgumentException";

/// `TestTaxonomyFacetAssociations`.
pub fn class_def() -> Arc<Class> {
    facet_class(
        "org.apache.lucene.facet.taxonomy.TestTaxonomyFacetAssociations",
        vec![
            static_hook("afterClass", AfterClass, after_class),
            static_hook("beforeClass", BeforeClass, before_class),
            test("testIntSumAssociation", test_int_sum_association),
            test("testIntAssociationRandom", test_int_association_random),
            test("testFloatSumAssociation", test_float_sum_association),
            test("testFloatAssociationRandom", test_float_association_random),
            test("testIntAndFloatAssocation", test_int_and_float_assocation),
            test("testWrongIndexFieldName", test_wrong_index_field_name),
            test(
                "testMixedTypesInSameIndexField",
                test_mixed_types_in_same_index_field,
            ),
            test("testNoHierarchy", test_no_hierarchy),
            test("testRequireDimCount", test_require_dim_count),
            test(
                "testIntSumAssociationDrillDown",
                test_int_sum_association_drill_down,
            ),
            test(
                "testNonPositiveAggregations",
                test_non_positive_aggregations,
            ),
            test("testAggregationCounts", test_aggregation_counts),
        ],
    )
}

/// The static fields.
struct Statics {
    dir: TestDirectory,
    reader: Arc<dyn DirectoryReader>,
    taxo_dir: TestDirectory,
    taxo_reader: Arc<dyn TaxonomyReader>,
    config: Arc<FacetsConfig>,
    random_int_values: JavaHashMap<Vec<i32>>,
    random_float_values: JavaHashMap<Vec<f32>>,
    random_int_single_valued: JavaHashMap<Vec<i32>>,
    random_float_single_valued: JavaHashMap<Vec<f32>>,
}

static STATICS: Mutex<Option<Arc<Statics>>> = Mutex::new(None);

fn statics_lock() -> MutexGuard<'static, Option<Arc<Statics>>> {
    STATICS.lock().unwrap_or_else(|e| e.into_inner())
}

fn statics() -> Arc<Statics> {
    Arc::clone(statics_lock().as_ref().expect("beforeClass"))
}

fn sum() -> Arc<dyn AssociationAggregationFunction> {
    Arc::new(SumAggregation)
}

fn max() -> Arc<dyn AssociationAggregationFunction> {
    Arc::new(MaxAggregation)
}

fn random_writer(dir: Arc<dyn Directory>) -> Result<RandomIndexWriter> {
    RandomIndexWriter::new_with_mock_analyzer(&*LuceneTestCase::random(), dir)
}

fn before_class() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let taxo_dir = LuceneTestCase::new_directory();
    // preparations - index, taxonomy, content

    let taxo_writer = DirectoryTaxonomyWriter::new(taxo_dir.as_directory())?;

    // Cannot mix ints & floats in the same indexed field:
    let config = Arc::new(FacetsConfig::new());
    config.set_index_field_name("int", "$facets.int");
    config.set_multi_valued("int", true);
    config.set_index_field_name("int_random", "$facets.int");
    config.set_multi_valued("int_random", true);
    config.set_index_field_name("int_single_valued", "$facets.int");
    config.set_index_field_name("float", "$facets.float");
    config.set_multi_valued("float", true);
    config.set_index_field_name("float_random", "$facets.float");
    config.set_multi_valued("float_random", true);
    config.set_index_field_name("float_single_valued", "$facets.float");

    let writer = random_writer(dir.as_directory())?;

    // index documents, 50% have only 'b' and all have 'a'
    for i in 0..110 {
        let mut doc = Document::new();
        // every 11th document is added empty, this used to cause the association
        // aggregators to go into an infinite loop
        if i % 11 != 0 {
            doc.add(IntAssociationFacetField::new(2, "int", &["a"])?);
            doc.add(FloatAssociationFacetField::new(0.5f32, "float", &["a"])?);
            if i % 2 == 0 {
                // 50
                doc.add(IntAssociationFacetField::new(3, "int", &["b"])?);
                doc.add(FloatAssociationFacetField::new(0.2f32, "float", &["b"])?);
            }
        }
        doc.add(TextField::new("match", "yes".to_string(), Store::NO)?);
        writer.add_document(&config.build_with_taxonomy(Some(&taxo_writer), &doc)?)?;
    }

    // Also index random content for more random testing:
    let paths = ["a", "b", "c"];
    let random = LuceneTestCase::random();
    let count = random.next_int_bound(1000);
    let mut random_int_values: JavaHashMap<Vec<i32>> = JavaHashMap::new();
    let mut random_float_values: JavaHashMap<Vec<f32>> = JavaHashMap::new();
    let mut random_int_single_valued: JavaHashMap<Vec<i32>> = JavaHashMap::new();
    let mut random_float_single_valued: JavaHashMap<Vec<f32>> = JavaHashMap::new();
    for _ in 0..count {
        let mut doc = Document::new();

        if random.next_int_bound(10) >= 2 {
            // occasionally don't add any fields
            // Add up to five ordinals + values for each doc. Note that duplicates are totally fine:
            for _ in 0..5 {
                let path = paths[random.next_int_bound(3) as usize];
                if random.next_boolean() {
                    // maybe index an int association with the dim
                    let next_int = LuceneTestCase::at_least(1);
                    random_int_values
                        .compute_if_absent(path, Vec::new)
                        .push(next_int);
                    doc.add(IntAssociationFacetField::new(
                        next_int,
                        "int_random",
                        &[path],
                    )?);
                }
                if random.next_boolean() {
                    // maybe index a float association with the dim
                    let next_float = random.next_float() * 10000f32;
                    doc.add(FloatAssociationFacetField::new(
                        next_float,
                        "float_random",
                        &[path],
                    )?);
                }
            }

            // Also, (maybe) add to the single-valued association fields:
            let path = paths[random.next_int_bound(3) as usize];
            if random.next_boolean() {
                let next_int = LuceneTestCase::at_least(1);
                random_int_single_valued
                    .compute_if_absent(path, Vec::new)
                    .push(next_int);
                doc.add(IntAssociationFacetField::new(
                    next_int,
                    "int_single_valued",
                    &[path],
                )?);
            }
            if random.next_boolean() {
                let next_float = random.next_float() * 10000f32;
                doc.add(FloatAssociationFacetField::new(
                    next_float,
                    "float_single_valued",
                    &[path],
                )?);
            }
        }

        doc.add(TextField::new("match", "yes".to_string(), Store::NO)?);
        writer.add_document(&config.build_with_taxonomy(Some(&taxo_writer), &doc)?)?;
    }

    // Add more random labels and documents to randomly make the test run on sparse/dense
    // aggregation values.
    let count = random.next_int_bound(10_000);
    for i in 0..count {
        let mut doc = Document::new();
        doc.add(FacetField::new(&format!("random_dim_{i}"), &["path"])?);
        doc.add(TextField::new("match", "no".to_string(), Store::NO)?);
        writer.add_document(&config.build_with_taxonomy(Some(&taxo_writer), &doc)?)?;
    }

    taxo_writer.close()?;
    let reader = writer.get_reader()?;
    writer.close()?;
    let taxo_reader: Arc<dyn TaxonomyReader> =
        Arc::new(DirectoryTaxonomyReader::open(taxo_dir.as_directory())?);

    // To avoid floating point precision issues, it's useful to keep track of the values in the
    // exact same order they appear when iterating the doc values in the index. This ensures we
    // sum them in the same order when determining expected values for tests cases and when the
    // actual facets implementation sums them. See LUCENE-10530:
    for ctx in Arc::clone(&reader).leaves()? {
        let mut dv = DocValues::get_binary(&*ctx.leaf_reader(), "$facets.float")?;
        let mut doc = dv.next_doc()?;
        while doc != NO_MORE_DOCS {
            let bytes_ref = dv.binary_value()?;
            let bytes = &bytes_ref.bytes;
            let end = bytes_ref.offset + bytes_ref.length;
            let mut offset = bytes_ref.offset;
            while offset < end {
                let ord =
                    i32::from_be_bytes(bytes[offset..offset + 4].try_into().expect("4 bytes"));
                offset += 4;
                let value =
                    f32::from_be_bytes(bytes[offset..offset + 4].try_into().expect("4 bytes"));
                offset += 4;
                let label = taxo_reader
                    .get_path(ord)?
                    .expect("java.lang.NullPointerException");
                let dim = label.components[0].as_str();
                let child = label.components[1].as_str();
                if "float_random" == dim {
                    random_float_values
                        .compute_if_absent(child, Vec::new)
                        .push(value);
                } else if "float_single_valued" == dim {
                    random_float_single_valued
                        .compute_if_absent(child, Vec::new)
                        .push(value);
                }
            }
            doc = dv.next_doc()?;
        }
    }

    *statics_lock() = Some(Arc::new(Statics {
        dir,
        reader,
        taxo_dir,
        taxo_reader,
        config,
        random_int_values,
        random_float_values,
        random_int_single_valued,
        random_float_single_valued,
    }));
    Ok(())
}

fn after_class() -> Result<()> {
    if let Some(s) = statics_lock().take() {
        s.reader.close()?;
        s.dir.close()?;
        s.taxo_reader.close()?;
        s.taxo_dir.close()?;
    }
    Ok(())
}

/// `searcher.search(new TermQuery(new Term("match", "yes")), new
/// FacetsCollectorManager())` over the static reader.
fn match_yes(s: &Statics) -> Result<FacetsCollector> {
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&s.reader));
    searcher.search_with_manager(
        Arc::new(TermQuery::new(Term::from_text("match", "yes"))),
        &FacetsCollectorManager::new(),
    )
}

fn top(facets: &dyn Facets, top_n: i32, dim: &str) -> Result<Option<FacetResult>> {
    facets.get_top_children(top_n, dim, &[])
}

fn top_string(facets: &dyn Facets, top_n: i32, dim: &str) -> Result<String> {
    Ok(top(facets, top_n, dim)?
        .expect("java.lang.NullPointerException")
        .to_string())
}

fn all_children(facets: &dyn Facets, dim: &str) -> Result<FacetResult> {
    Ok(facets
        .get_all_children(dim, &[])?
        .expect("java.lang.NullPointerException"))
}

fn counts_a100_b50() -> HashMap<String, i32> {
    HashMap::from([("a".to_string(), 100), ("b".to_string(), 50)])
}

fn test_int_sum_association() -> Result<()> {
    let s = statics();
    let fc = match_yes(&s)?;

    let facets = TaxonomyFacetIntAssociations::with_index_field_name(
        "$facets.int",
        Arc::clone(&s.taxo_reader),
        Arc::clone(&s.config),
        &fc,
        sum(),
    )?;
    Assert::assert_equals(
        "dim=int path=[] value=-1 childCount=2\n  a (200)\n  b (150)\n",
        top_string(&facets, 10, "int")?.as_str(),
    );
    assert_facet_result_counts(
        &all_children(&facets, "int")?,
        "int",
        &[],
        2,
        NumericValue::Int(-1),
        &counts_a100_b50(),
        &[
            LabelAndValue::new("a", NumericValue::Int(200)),
            LabelAndValue::new("b", NumericValue::Int(150)),
        ],
    );
    Assert::assert_equals_msg(
        Some("Wrong count for category 'a'!"),
        200,
        number_int_value(&facets.get_specific_value("int", &["a"])?),
    );
    Assert::assert_equals_msg(
        Some("Wrong count for category 'b'!"),
        150,
        number_int_value(&facets.get_specific_value("int", &["b"])?),
    );

    // test getAllDims and getTopDims
    let top_dims = facets.get_top_dims(10, 10)?;
    let all_dims = facets.get_all_dims(10)?;
    assert_equals(&top_dims, &all_dims);
    Ok(())
}

/// `e.getValue().stream().reduce(Integer::sum).orElse(0)` per key.
fn int_sums(values: &JavaHashMap<Vec<i32>>) -> JavaHashMap<i32> {
    let mut expected = JavaHashMap::new();
    for (k, v) in values.iter() {
        expected.insert(
            k.clone(),
            v.iter().copied().reduce(i32::wrapping_add).unwrap_or(0),
        );
    }
    expected
}

/// `e.getValue().stream().max(Integer::compareTo).orElse(0)` per key.
fn int_maxes(values: &JavaHashMap<Vec<i32>>) -> JavaHashMap<i32> {
    let mut expected = JavaHashMap::new();
    for (k, v) in values.iter() {
        expected.insert(k.clone(), v.iter().copied().max().unwrap_or(0));
    }
    expected
}

/// `e.getValue().stream().reduce(Float::sum).orElse(0f)` per key.
fn float_sums(values: &JavaHashMap<Vec<f32>>) -> JavaHashMap<f32> {
    let mut expected = JavaHashMap::new();
    for (k, v) in values.iter() {
        expected.insert(
            k.clone(),
            v.iter().copied().reduce(|a, b| a + b).unwrap_or(0f32),
        );
    }
    expected
}

/// `e.getValue().stream().max(Float::compareTo).orElse(0f)` per key.
fn float_maxes(values: &JavaHashMap<Vec<f32>>) -> JavaHashMap<f32> {
    let mut expected = JavaHashMap::new();
    for (k, v) in values.iter() {
        // `Stream.max` keeps the later element only when it compares greater.
        let m = v
            .iter()
            .copied()
            .reduce(|a, b| if java_float_compare(a, b) >= 0 { a } else { b })
            .unwrap_or(0f32);
        expected.insert(k.clone(), m);
    }
    expected
}

/// `Float.compare(float, float)`.
fn java_float_compare(a: f32, b: f32) -> i32 {
    if a < b {
        return -1;
    }
    if a > b {
        return 1;
    }
    let (ab, bb) = (java_float_to_int_bits(a), java_float_to_int_bits(b));
    match ab.cmp(&bb) {
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater => 1,
    }
}

/// `Float.floatToIntBits(float)` (canonical NaN).
fn java_float_to_int_bits(f: f32) -> i32 {
    if f.is_nan() {
        0x7fc0_0000
    } else {
        f.to_bits() as i32
    }
}

fn test_int_association_random() -> Result<()> {
    let s = statics();
    let fc = match_yes(&s)?;

    // SUM:
    let facets = TaxonomyFacetIntAssociations::with_index_field_name(
        "$facets.int",
        Arc::clone(&s.taxo_reader),
        Arc::clone(&s.config),
        &fc,
        sum(),
    )?;
    let expected = int_sums(&s.random_int_values);
    validate_ints("int_random", &expected, &SumAggregation, true, &facets)?;
    let expected = int_sums(&s.random_int_single_valued);
    validate_ints(
        "int_single_valued",
        &expected,
        &SumAggregation,
        false,
        &facets,
    )?;

    // test getAllDims and getTopDims
    let all_dims = facets.get_all_dims(10)?;
    let top_dims = facets.get_top_dims(10, 10)?;
    assert_equals(&top_dims, &all_dims);

    // MAX:
    let facets = TaxonomyFacetIntAssociations::with_index_field_name(
        "$facets.int",
        Arc::clone(&s.taxo_reader),
        Arc::clone(&s.config),
        &fc,
        max(),
    )?;
    let expected = int_maxes(&s.random_int_values);
    validate_ints("int_random", &expected, &MaxAggregation, true, &facets)?;
    let expected = int_maxes(&s.random_int_single_valued);
    validate_ints(
        "int_single_valued",
        &expected,
        &MaxAggregation,
        false,
        &facets,
    )?;

    // test getAllDims and getTopDims
    let top_dims = facets.get_top_dims(10, 10)?;
    let all_dims = facets.get_all_dims(10)?;
    assert_equals(&top_dims, &all_dims);
    Ok(())
}

fn test_float_sum_association() -> Result<()> {
    let s = statics();
    let fc = match_yes(&s)?;

    let facets = TaxonomyFacetFloatAssociations::with_index_field_name(
        "$facets.float",
        Arc::clone(&s.taxo_reader),
        Arc::clone(&s.config),
        &fc,
        sum(),
    )?;
    Assert::assert_equals(
        "dim=float path=[] value=-1.0 childCount=2\n  a (50.0)\n  b (9.999995)\n",
        top_string(&facets, 10, "float")?.as_str(),
    );

    assert_facet_result_counts(
        &all_children(&facets, "float")?,
        "float",
        &[],
        2,
        NumericValue::Float(-1f32),
        &counts_a100_b50(),
        &[
            LabelAndValue::new("a", NumericValue::Float(50.0f32)),
            LabelAndValue::new("b", NumericValue::Float(9.999995f32)),
        ],
    );

    Assert::assert_equals_double_msg(
        Some("Wrong count for category 'a'!"),
        50f64,
        f64::from(number_float_value(
            &facets.get_specific_value("float", &["a"])?,
        )),
        0.00001,
    );
    Assert::assert_equals_double_msg(
        Some("Wrong count for category 'b'!"),
        10f64,
        f64::from(number_float_value(
            &facets.get_specific_value("float", &["b"])?,
        )),
        0.00001,
    );

    // test getAllDims and getTopDims
    let top_dims = facets.get_top_dims(10, 10)?;
    let all_dims = facets.get_all_dims(10)?;
    assert_float_facet_results_equal(&top_dims, &all_dims);
    Ok(())
}

fn test_float_association_random() -> Result<()> {
    let s = statics();
    // disabling search concurrency because validateFloats relies on ordering which requires
    // sequential execution
    let searcher = LuceneTestCase::new_searcher_threads(Arc::clone(&s.reader), true, true, false);
    let fc = searcher.search_with_manager(
        Arc::new(TermQuery::new(Term::from_text("match", "yes"))),
        &FacetsCollectorManager::new(),
    )?;

    // SUM:
    let facets = TaxonomyFacetFloatAssociations::with_index_field_name(
        "$facets.float",
        Arc::clone(&s.taxo_reader),
        Arc::clone(&s.config),
        &fc,
        sum(),
    )?;
    let expected = float_sums(&s.random_float_values);
    validate_floats("float_random", &expected, &SumAggregation, true, &facets)?;
    let expected = float_sums(&s.random_float_single_valued);
    validate_floats(
        "float_single_valued",
        &expected,
        &SumAggregation,
        false,
        &facets,
    )?;

    // test getAllDims and getTopDims
    let top_dims = facets.get_top_dims(10, 10)?;
    let all_dims = facets.get_all_dims(10)?;
    assert_float_facet_results_equal(&top_dims, &all_dims);

    // MAX:
    let facets = TaxonomyFacetFloatAssociations::with_index_field_name(
        "$facets.float",
        Arc::clone(&s.taxo_reader),
        Arc::clone(&s.config),
        &fc,
        max(),
    )?;
    let expected = float_maxes(&s.random_float_values);
    validate_floats("float_random", &expected, &MaxAggregation, true, &facets)?;
    let expected = float_maxes(&s.random_float_single_valued);
    validate_floats(
        "float_single_valued",
        &expected,
        &MaxAggregation,
        false,
        &facets,
    )?;

    // test getAllDims and getTopDims
    let top_dims = facets.get_top_dims(10, 10)?;
    let all_dims = facets.get_all_dims(10)?;
    assert_float_facet_results_equal(&top_dims, &all_dims);
    Ok(())
}

/// Make sure we can test both int and float assocs in one index, as long as
/// we send each to a different field.
fn test_int_and_float_assocation() -> Result<()> {
    let s = statics();
    let fc = match_yes(&s)?;

    let facets = TaxonomyFacetFloatAssociations::with_index_field_name(
        "$facets.float",
        Arc::clone(&s.taxo_reader),
        Arc::clone(&s.config),
        &fc,
        sum(),
    )?;
    Assert::assert_equals_double_msg(
        Some("Wrong count for category 'a'!"),
        50f64,
        f64::from(number_float_value(
            &facets.get_specific_value("float", &["a"])?,
        )),
        0.00001,
    );
    Assert::assert_equals_double_msg(
        Some("Wrong count for category 'b'!"),
        10f64,
        f64::from(number_float_value(
            &facets.get_specific_value("float", &["b"])?,
        )),
        0.00001,
    );

    let facets = TaxonomyFacetIntAssociations::with_index_field_name(
        "$facets.int",
        Arc::clone(&s.taxo_reader),
        Arc::clone(&s.config),
        &fc,
        sum(),
    )?;
    Assert::assert_equals_msg(
        Some("Wrong count for category 'a'!"),
        200,
        number_int_value(&facets.get_specific_value("int", &["a"])?),
    );
    Assert::assert_equals_msg(
        Some("Wrong count for category 'b'!"),
        150,
        number_int_value(&facets.get_specific_value("int", &["b"])?),
    );
    Ok(())
}

fn test_wrong_index_field_name() -> Result<()> {
    let s = statics();
    let fc = match_yes(&s)?;
    let facets = TaxonomyFacetFloatAssociations::with_index_field_name(
        "wrong_field",
        Arc::clone(&s.taxo_reader),
        Arc::clone(&s.config),
        &fc,
        sum(),
    )?;
    expect_throws(IAE, || facets.get_specific_value("float", &[]));

    expect_throws(IAE, || facets.get_top_children(10, "float", &[]));

    expect_throws(IAE, || facets.get_all_children("float", &[]));
    Ok(())
}

/// The body shared by `testMixedTypesInSameIndexField`, `testNoHierarchy`
/// and `testRequireDimCount`: `configure` sets up the config, `fill` the
/// document whose indexing must throw `IllegalArgumentException`.
fn expect_build_failure(
    configure: impl FnOnce(&FacetsConfig),
    fill: impl FnOnce(&mut Document) -> Result<()>,
) -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let taxo_dir = LuceneTestCase::new_directory();

    let taxo_writer = DirectoryTaxonomyWriter::new(taxo_dir.as_directory())?;
    let config = FacetsConfig::new();
    configure(&config);
    let writer = random_writer(dir.as_directory())?;

    let mut doc = Document::new();
    fill(&mut doc)?;
    expect_throws(IAE, || {
        writer.add_document(&config.build_with_taxonomy(Some(&taxo_writer), &doc)?)
    });

    writer.close()?;
    taxo_writer.close()?;
    dir.close()?;
    taxo_dir.close()
}

fn test_mixed_types_in_same_index_field() -> Result<()> {
    expect_build_failure(
        |_| {},
        |doc| {
            doc.add(IntAssociationFacetField::new(14, "a", &["x"])?);
            doc.add(FloatAssociationFacetField::new(55.0f32, "b", &["y"])?);
            Ok(())
        },
    )
}

fn test_no_hierarchy() -> Result<()> {
    expect_build_failure(
        |config| config.set_hierarchical("a", true),
        |doc| {
            doc.add(IntAssociationFacetField::new(14, "a", &["x"])?);
            Ok(())
        },
    )
}

fn test_require_dim_count() -> Result<()> {
    expect_build_failure(
        |config| config.set_require_dim_count("a", true),
        |doc| {
            doc.add(IntAssociationFacetField::new(14, "a", &["x"])?);
            Ok(())
        },
    )
}

fn test_int_sum_association_drill_down() -> Result<()> {
    let s = statics();
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&s.reader));
    let mut q = DrillDownQuery::new(Arc::clone(&s.config));
    q.add("int", &["b"])?;
    let fc = searcher.search_with_manager(Arc::new(q), &FacetsCollectorManager::new())?;

    let facets = TaxonomyFacetIntAssociations::with_index_field_name(
        "$facets.int",
        Arc::clone(&s.taxo_reader),
        Arc::clone(&s.config),
        &fc,
        sum(),
    )?;
    Assert::assert_equals(
        "dim=int path=[] value=-1 childCount=2\n  b (150)\n  a (100)\n",
        top_string(&facets, 10, "int")?.as_str(),
    );

    Assert::assert_equals(
        "dim=int path=[] value=-1 childCount=2\n  a (100)\n  b (150)\n",
        sort_all_children(all_children(&facets, "int")?)
            .to_string()
            .as_str(),
    );

    assert_facet_result(
        &all_children(&facets, "int")?,
        "int",
        &[],
        2,
        NumericValue::Int(-1),
        &[
            LabelAndValue::new("a", NumericValue::Int(100)),
            LabelAndValue::new("b", NumericValue::Int(150)),
        ],
    );

    Assert::assert_equals_msg(
        Some("Wrong count for category 'a'!"),
        100,
        number_int_value(&facets.get_specific_value("int", &["a"])?),
    );
    Assert::assert_equals_msg(
        Some("Wrong count for category 'b'!"),
        150,
        number_int_value(&facets.get_specific_value("int", &["b"])?),
    );
    Ok(())
}

fn test_non_positive_aggregations() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let taxo_dir = LuceneTestCase::new_directory();

    let taxo_writer = DirectoryTaxonomyWriter::new(taxo_dir.as_directory())?;

    let config = Arc::new(FacetsConfig::new());
    config.set_index_field_name("a", "$float_facets");
    config.set_index_field_name("b", "$int_facets");

    let writer = random_writer(dir.as_directory())?;

    let mut d = Document::new();
    // Positive association
    d.add(FloatAssociationFacetField::new(1f32, "a", &["1"])?);
    d.add(IntAssociationFacetField::new(1, "b", &["1"])?);
    writer.add_document(&config.build_with_taxonomy(Some(&taxo_writer), &d)?)?;

    let mut d = Document::new();
    // Zero association
    d.add(FloatAssociationFacetField::new(0f32, "a", &["2"])?);
    d.add(IntAssociationFacetField::new(0, "b", &["2"])?);
    writer.add_document(&config.build_with_taxonomy(Some(&taxo_writer), &d)?)?;

    let mut d = Document::new();
    // Negative association
    d.add(FloatAssociationFacetField::new(-1f32, "a", &["3"])?);
    d.add(IntAssociationFacetField::new(-1, "b", &["3"])?);
    writer.add_document(&config.build_with_taxonomy(Some(&taxo_writer), &d)?)?;

    let reader = writer.get_reader()?;
    taxo_writer.close()?;
    writer.close()?;

    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    let q: Arc<dyn Query> = Arc::new(MatchAllDocsQuery::instance());
    let fc = searcher.search_with_manager(q, &FacetsCollectorManager::new())?;

    let taxo_reader: Arc<dyn TaxonomyReader> =
        Arc::new(DirectoryTaxonomyReader::open(taxo_dir.as_directory())?);
    let float_facets = TaxonomyFacetFloatAssociations::with_index_field_name(
        "$float_facets",
        Arc::clone(&taxo_reader),
        Arc::clone(&config),
        &fc,
        sum(),
    )?;
    let int_facets = TaxonomyFacetIntAssociations::with_index_field_name(
        "$int_facets",
        Arc::clone(&taxo_reader),
        Arc::clone(&config),
        &fc,
        sum(),
    )?;

    // "2" and "3" are included in the result despite having non-positive values associated to them.
    Assert::assert_equals(
        "dim=a path=[] value=0.0 childCount=3\n  1 (1.0)\n  2 (0.0)\n  3 (-1.0)\n",
        top_string(&float_facets, 10, "a")?.as_str(),
    );
    Assert::assert_equals(
        "dim=b path=[] value=0 childCount=3\n  1 (1)\n  2 (0)\n  3 (-1)\n",
        top_string(&int_facets, 10, "b")?.as_str(),
    );

    taxo_reader.close()?;
    reader.close()?;
    taxo_dir.close()?;
    dir.close()
}

fn test_aggregation_counts() -> Result<()> {
    let taxo_dir = LuceneTestCase::new_directory();

    let taxo_writer = DirectoryTaxonomyWriter::new(taxo_dir.as_directory())?;

    let config = Arc::new(FacetsConfig::new());
    config.set_index_field_name("a", "$int_facets");

    // Java writes into the static `dir` (the class index), as here.
    let writer = random_writer(statics().dir.as_directory())?;

    let mut d = Document::new();
    d.add(IntAssociationFacetField::new(1, "a", &["1"])?);
    writer.add_document(&config.build_with_taxonomy(Some(&taxo_writer), &d)?)?;

    let mut d = Document::new();
    d.add(IntAssociationFacetField::new(5, "a", &["2"])?);
    writer.add_document(&config.build_with_taxonomy(Some(&taxo_writer), &d)?)?;

    let mut d = Document::new();
    d.add(IntAssociationFacetField::new(1, "a", &["1"])?);
    writer.add_document(&config.build_with_taxonomy(Some(&taxo_writer), &d)?)?;

    let reader = writer.get_reader()?;
    taxo_writer.close()?;
    writer.close()?;

    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    let q: Arc<dyn Query> = Arc::new(MatchAllDocsQuery::instance());
    let fc = searcher.search_with_manager(q, &FacetsCollectorManager::new())?;

    let taxo_reader: Arc<dyn TaxonomyReader> =
        Arc::new(DirectoryTaxonomyReader::open(taxo_dir.as_directory())?);
    let int_facets = TaxonomyFacetIntAssociations::with_index_field_name(
        "$int_facets",
        Arc::clone(&taxo_reader),
        Arc::clone(&config),
        &fc,
        sum(),
    )?;

    let result = top(&int_facets, 10, "a")?.expect("java.lang.NullPointerException");
    Assert::assert_equals(
        "dim=a path=[] value=7 childCount=2\n  2 (5)\n  1 (2)\n",
        result.to_string().as_str(),
    );
    Assert::assert_equals(1, result.label_values[0].count);
    Assert::assert_equals(2, result.label_values[1].count);

    taxo_reader.close()?;
    reader.close()?;
    taxo_dir.close()
}

fn validate_ints(
    dim: &str,
    expected: &JavaHashMap<i32>,
    aggregation_function: &dyn AssociationAggregationFunction,
    is_multi_valued: bool,
    facets: &dyn Facets,
) -> Result<()> {
    let mut aggregated_value = 0i32;
    for (key, value) in expected.iter() {
        let value = *value;
        Assert::assert_equals(
            value,
            number_int_value(&facets.get_specific_value(dim, &[key.as_str()])?),
        );
        aggregated_value = aggregation_function.aggregate_int(aggregated_value, value);
    }

    if is_multi_valued {
        aggregated_value = -1;
    }

    let facet_result = facets.get_top_children(10, dim, &[])?;
    let all_children_result = facets.get_all_children(dim, &[])?;

    if expected.is_empty() {
        // If we hit the rare random case where nothing is indexed for the dim, we expect a null
        // facetResult (see: LUCENE-10529)
        Assert::assert_true(facet_result.is_none());
        Assert::assert_true(all_children_result.is_none());
    } else {
        let facet_result = facet_result.expect("java.lang.NullPointerException");
        let all_children_result = all_children_result.expect("java.lang.NullPointerException");
        Assert::assert_equals(dim, facet_result.dim.as_str());
        Assert::assert_equals(dim, all_children_result.dim.as_str());
        Assert::assert_equals(aggregated_value, number_int_value(&facet_result.value));
        Assert::assert_equals(
            aggregated_value,
            number_int_value(&all_children_result.value),
        );
        Assert::assert_equals(expected.len() as i32, facet_result.child_count);
        Assert::assert_equals(expected.len() as i32, all_children_result.child_count);
    }
    Ok(())
}

fn validate_floats(
    dim: &str,
    expected: &JavaHashMap<f32>,
    aggregation_function: &dyn AssociationAggregationFunction,
    is_multi_valued: bool,
    facets: &dyn Facets,
) -> Result<()> {
    let mut aggregated_value = 0f32;
    for (key, value) in expected.iter() {
        let value = *value;
        // We can expect the floats to be exactly equal here since we're ensuring that we sum them
        // in the same order when determining expected values and when computing facets. See
        // LUCENE-10530. This though requires sequential execution.
        Assert::assert_equals_float_msg(
            None,
            value,
            number_float_value(&facets.get_specific_value(dim, &[key.as_str()])?),
            0f32,
        );
        aggregated_value = aggregation_function.aggregate_float(aggregated_value, value);
    }

    if is_multi_valued {
        aggregated_value = -1f32;
    }

    let facet_result = facets.get_top_children(10, dim, &[])?;
    let all_children_result = facets.get_all_children(dim, &[])?;

    if expected.is_empty() {
        // If we hit the rare random case where nothing is indexed for the dim, we expect a null
        // facetResult (see: LUCENE-10529)
        Assert::assert_true(facet_result.is_none());
        Assert::assert_true(all_children_result.is_none());
    } else {
        let facet_result = facet_result.expect("java.lang.NullPointerException");
        let all_children_result = all_children_result.expect("java.lang.NullPointerException");
        Assert::assert_equals(dim, facet_result.dim.as_str());
        Assert::assert_equals(dim, all_children_result.dim.as_str());
        Assert::assert_equals_float_msg(
            None,
            aggregated_value,
            number_float_value(&facet_result.value),
            1f32,
        );
        Assert::assert_equals_float_msg(
            None,
            aggregated_value,
            number_float_value(&all_children_result.value),
            1f32,
        );
        Assert::assert_equals(expected.len() as i32, facet_result.child_count);
        Assert::assert_equals(expected.len() as i32, all_children_result.child_count);
    }
    Ok(())
}

fn assert_float_facet_results_equal(expected: &[FacetResult], actual: &[FacetResult]) {
    Assert::assert_equals(expected.len(), actual.len());
    for (expected_result, actual_result) in expected.iter().zip(actual) {
        Assert::assert_equals(expected_result.dim.as_str(), actual_result.dim.as_str());
        Assert::assert_true(expected_result.path == actual_result.path);
        LuceneTestCase::assert_float_ulp_equals(
            number_float_value(&expected_result.value),
            number_float_value(&actual_result.value),
            2,
        );
        Assert::assert_equals(expected_result.child_count, actual_result.child_count);
    }
}

// since we have no insight into the ordinals assigned to the values, we sort labels by value and
// count in
// ascending order in order to compare with expected results
fn sort_all_children(mut all_children_result: FacetResult) -> FacetResult {
    all_children_result.label_values.sort_by(|a, b| {
        a.label
            .encode_utf16()
            .cmp(b.label.encode_utf16())
            .then_with(|| number_long_value(&a.value).cmp(&number_long_value(&b.value)))
    });
    all_children_result
}
