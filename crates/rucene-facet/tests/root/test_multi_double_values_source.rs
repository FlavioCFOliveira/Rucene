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

//! Port of `org.apache.lucene.facet.TestMultiDoubleValuesSource`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::index::doc_values::DocValues;
use rucene::search::double_values_source as dvs;
use rucene_facet::multi_double_values_source::{LongToDoubleFunction, MultiDoubleValuesSource};

use crate::facet_support::core_support::assert_not_equals;
use crate::facet_support::search_support::{ftest, set_up, tear_down};
use crate::facet_support::{
    assert_object_equals, assert_object_not_equals, fixture_class_extending,
};
use crate::root::multi_values_source_test_case::{
    self as base, multi_values_source_test_case_class, validate_numeric_double_precision,
    validate_sorted_double, validate_sorted_double_precision, MultiValuesSourceTestCase,
};

type F = MultiValuesSourceTestCase;
type Mdvs = dyn MultiDoubleValuesSource;

/// `TestMultiDoubleValuesSource`.
pub fn class_def() -> Arc<Class> {
    fixture_class_extending::<F>(
        "org.apache.lucene.facet.TestMultiDoubleValuesSource",
        &multi_values_source_test_case_class(),
        vec![
            set_up(base::set_up),
            tear_down(base::tear_down),
            ftest("testRandom", test_random),
            ftest("testFromSingleValued", test_from_single_valued),
            ftest("testCacheable", test_cacheable),
            ftest("testEqualsAndHashcode", test_equals_and_hashcode),
        ],
    )
}

/// The loop over the leaves with `validateFieldBasedSource(docValues,
/// values, maxDoc)` (`precision` `None`) or with `useDoublePrecision`.
fn check(f: &F, values_source: &Arc<Mdvs>, field: &str, precision: Option<bool>) -> Result<()> {
    for ctx in Arc::clone(f.reader()).leaves()? {
        let leaf = ctx.leaf_reader();
        let mut doc_values = DocValues::get_sorted_numeric(&*leaf, field)?;
        let mut values = values_source.get_values(&ctx)?;
        match precision {
            None => validate_sorted_double(&mut *doc_values, &mut *values, leaf.max_doc())?,
            Some(p) => {
                validate_sorted_double_precision(&mut *doc_values, &mut *values, leaf.max_doc(), p)?
            }
        }
    }
    Ok(())
}

fn test_random(f: &mut F) -> Result<()> {
    check(f, &Mdvs::from_int_field("single_int"), "single_int", None)?;
    check(
        f,
        &Mdvs::from_long_field("single_long"),
        "single_long",
        None,
    )?;
    check(f, &Mdvs::from_int_field("multi_int"), "multi_int", None)?;
    check(f, &Mdvs::from_long_field("multi_long"), "multi_long", None)?;
    check(
        f,
        &Mdvs::from_float_field("single_float"),
        "single_float",
        Some(false),
    )?;
    check(
        f,
        &Mdvs::from_double_field("single_double"),
        "single_double",
        Some(true),
    )?;
    check(
        f,
        &Mdvs::from_float_field("multi_float"),
        "multi_float",
        Some(false),
    )?;
    check(
        f,
        &Mdvs::from_double_field("multi_double"),
        "multi_double",
        Some(true),
    )
}

fn check_single(
    f: &F,
    single: Arc<dyn dvs::DoubleValuesSource>,
    field: &str,
    use_double_precision: bool,
) -> Result<()> {
    let values_source = Mdvs::from_single_valued(single);
    let singleton = Mdvs::unwrap_singleton(&*values_source);
    Assert::assert_true(singleton.is_some());
    let singleton = singleton.unwrap();
    for ctx in Arc::clone(f.reader()).leaves()? {
        let leaf = ctx.leaf_reader();
        let mut doc_values = DocValues::get_sorted_numeric(&*leaf, field)?;
        let mut values = values_source.get_values(&ctx)?;
        validate_sorted_double_precision(
            &mut *doc_values,
            &mut *values,
            leaf.max_doc(),
            use_double_precision,
        )?;
        let mut singleton_dv = DocValues::get_numeric(&*leaf, field)?;
        let mut singleton_vals = Arc::clone(&singleton).get_values(&ctx, None)?;
        validate_numeric_double_precision(
            &mut *singleton_dv,
            &mut *singleton_vals,
            leaf.max_doc(),
            use_double_precision,
        )?;
    }
    Ok(())
}

fn test_from_single_valued(f: &mut F) -> Result<()> {
    check_single(
        f,
        dvs::from_float_field("single_float"),
        "single_float",
        false,
    )?;
    check_single(
        f,
        dvs::from_double_field("single_double"),
        "single_double",
        true,
    )
}

fn test_cacheable(f: &mut F) -> Result<()> {
    let values_source = Mdvs::from_double_field("multi_double");
    for ctx in Arc::clone(f.searcher().get_index_reader()).leaves()? {
        Assert::assert_equals(
            DocValues::is_cacheable(&ctx, &["multi_double"]),
            values_source.is_cacheable(&ctx),
        );
    }
    Ok(())
}

fn test_equals_and_hashcode(_f: &mut F) -> Result<()> {
    let v1 = Mdvs::from_long_field("multi_long");
    let v2 = Mdvs::from_long_field("multi_long");
    let v3 = Mdvs::from_long_field("multi_int");
    assert_object_equals(v1.source_eq(&*v2), &v1, &v2);
    assert_object_not_equals(v1.source_eq(&*v3), &v1);
    Assert::assert_equals(v1.source_hash(), v2.source_hash());
    assert_not_equals(&v1.source_hash(), &v3.source_hash());

    let v1 = Mdvs::from_single_valued(dvs::from_long_field("single_long"));
    let v2 = Mdvs::from_single_valued(dvs::from_long_field("single_long"));
    let v3 = Mdvs::from_single_valued(dvs::from_long_field("single_int"));
    assert_object_equals(v1.source_eq(&*v2), &v1, &v2);
    assert_object_not_equals(v1.source_eq(&*v3), &v1);
    Assert::assert_equals(v1.source_hash(), v2.source_hash());
    assert_not_equals(&v1.source_hash(), &v3.source_hash());

    let s1 = Mdvs::unwrap_singleton(&*v1).unwrap();
    let s2 = Mdvs::unwrap_singleton(&*v2).unwrap();
    let s3 = Mdvs::unwrap_singleton(&*v3).unwrap();
    assert_object_equals(s1.source_eq(&*s2), &s1, &s2);
    assert_object_not_equals(s1.source_eq(&*s3), &s1);
    Assert::assert_equals(s1.source_hash(), s2.source_hash());
    assert_not_equals(&s1.source_hash(), &s3.source_hash());

    // `Long::valueOf` (a method reference; one object per evaluation) and
    // `v -> -1 * v`.
    let long_value_of = || -> LongToDoubleFunction { Arc::new(|v: i64| v as f64) };
    let negate: LongToDoubleFunction = Arc::new(|v: i64| (-v) as f64);
    let v1 = Mdvs::from_field("single_long", long_value_of());
    let v2 = Mdvs::from_field("single_long", negate);
    let v3 = Mdvs::from_field("single_int", long_value_of());
    assert_object_not_equals(v1.source_eq(&*v2), &v1);
    assert_object_not_equals(v1.source_eq(&*v3), &v1);
    assert_not_equals(&v1.source_hash(), &v2.source_hash());
    assert_not_equals(&v1.source_hash(), &v3.source_hash());
    Ok(())
}
