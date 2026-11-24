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

//! Port of `org.apache.lucene.facet.TestMultiLongValuesSource`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::index::doc_values::DocValues;
use rucene::search::long_values_source as lvs;
use rucene_facet::multi_long_values_source::{
    to_multi_double_values_source, MultiLongValuesSource,
};

use crate::facet_support::core_support::assert_not_equals;
use crate::facet_support::search_support::{ftest, set_up, tear_down};
use crate::facet_support::{
    assert_object_equals, assert_object_not_equals, fixture_class_extending,
};
use crate::root::multi_values_source_test_case::{
    self as base, multi_values_source_test_case_class, validate_numeric_long,
    validate_sorted_double, validate_sorted_long, MultiValuesSourceTestCase,
};

type F = MultiValuesSourceTestCase;

/// `TestMultiLongValuesSource`.
pub fn class_def() -> Arc<Class> {
    fixture_class_extending::<F>(
        "org.apache.lucene.facet.TestMultiLongValuesSource",
        &multi_values_source_test_case_class(),
        vec![
            set_up(base::set_up),
            tear_down(base::tear_down),
            ftest("testRandom", test_random),
            ftest("testFromSingleValued", test_from_single_valued),
            ftest("testToDouble", test_to_double),
            ftest("testCacheable", test_cacheable),
            ftest("testEqualsAndHashcode", test_equals_and_hashcode),
        ],
    )
}

fn check_multi(f: &F, values_source: &Arc<dyn MultiLongValuesSource>, field: &str) -> Result<()> {
    for ctx in Arc::clone(f.reader()).leaves()? {
        let leaf = ctx.leaf_reader();
        let mut doc_values = DocValues::get_sorted_numeric(&*leaf, field)?;
        let mut values = values_source.get_values(&ctx)?;
        validate_sorted_long(&mut *doc_values, &mut *values, leaf.max_doc())?;
    }
    Ok(())
}

fn test_random(f: &mut F) -> Result<()> {
    let values_source = <dyn MultiLongValuesSource>::from_int_field("single_int");
    check_multi(f, &values_source, "single_int")?;
    let values_source = <dyn MultiLongValuesSource>::from_long_field("single_long");
    check_multi(f, &values_source, "single_long")?;
    let values_source = <dyn MultiLongValuesSource>::from_int_field("multi_int");
    check_multi(f, &values_source, "multi_int")?;
    let values_source = <dyn MultiLongValuesSource>::from_long_field("multi_long");
    check_multi(f, &values_source, "multi_long")
}

fn check_single(f: &F, single: Arc<dyn lvs::LongValuesSource>, field: &str) -> Result<()> {
    let values_source = <dyn MultiLongValuesSource>::from_single_valued(single);
    let singleton = <dyn MultiLongValuesSource>::unwrap_singleton(&*values_source);
    Assert::assert_true(singleton.is_some());
    let singleton = singleton.unwrap();
    for ctx in Arc::clone(f.reader()).leaves()? {
        let leaf = ctx.leaf_reader();
        let mut doc_values = DocValues::get_sorted_numeric(&*leaf, field)?;
        let mut values = values_source.get_values(&ctx)?;
        validate_sorted_long(&mut *doc_values, &mut *values, leaf.max_doc())?;
        let mut singleton_dv = DocValues::get_numeric(&*leaf, field)?;
        let mut singleton_vals = Arc::clone(&singleton).get_values(&ctx, None)?;
        validate_numeric_long(&mut *singleton_dv, &mut *singleton_vals, leaf.max_doc())?;
    }
    Ok(())
}

fn test_from_single_valued(f: &mut F) -> Result<()> {
    check_single(f, lvs::from_int_field("single_int"), "single_int")?;
    check_single(f, lvs::from_long_field("single_long"), "single_long")
}

fn test_to_double(f: &mut F) -> Result<()> {
    let values_source =
        to_multi_double_values_source(<dyn MultiLongValuesSource>::from_long_field("multi_long"));
    for ctx in Arc::clone(f.reader()).leaves()? {
        let leaf = ctx.leaf_reader();
        let mut doc_values = DocValues::get_sorted_numeric(&*leaf, "multi_long")?;
        let mut values = values_source.get_values(&ctx)?;
        validate_sorted_double(&mut *doc_values, &mut *values, leaf.max_doc())?;
    }
    Ok(())
}

fn test_cacheable(f: &mut F) -> Result<()> {
    let values_source = <dyn MultiLongValuesSource>::from_long_field("multi_long");
    for ctx in Arc::clone(f.searcher().get_index_reader()).leaves()? {
        Assert::assert_equals(
            DocValues::is_cacheable(&ctx, &["multi_long"]),
            values_source.is_cacheable(&ctx),
        );
    }
    Ok(())
}

fn test_equals_and_hashcode(_f: &mut F) -> Result<()> {
    let v1 = <dyn MultiLongValuesSource>::from_long_field("multi_long");
    let v2 = <dyn MultiLongValuesSource>::from_long_field("multi_long");
    let v3 = <dyn MultiLongValuesSource>::from_long_field("multi_int");
    assert_object_equals(v1.source_eq(&*v2), &v1, &v2);
    assert_object_not_equals(v1.source_eq(&*v3), &v1);
    Assert::assert_equals(v1.source_hash(), v2.source_hash());
    assert_not_equals(&v1.source_hash(), &v3.source_hash());

    let v1 = <dyn MultiLongValuesSource>::from_single_valued(lvs::from_long_field("single_long"));
    let v2 = <dyn MultiLongValuesSource>::from_single_valued(lvs::from_long_field("single_long"));
    let v3 = <dyn MultiLongValuesSource>::from_single_valued(lvs::from_long_field("single_int"));
    assert_object_equals(v1.source_eq(&*v2), &v1, &v2);
    assert_object_not_equals(v1.source_eq(&*v3), &v1);
    Assert::assert_equals(v1.source_hash(), v2.source_hash());
    assert_not_equals(&v1.source_hash(), &v3.source_hash());

    let s1 = <dyn MultiLongValuesSource>::unwrap_singleton(&*v1).unwrap();
    let s2 = <dyn MultiLongValuesSource>::unwrap_singleton(&*v2).unwrap();
    let s3 = <dyn MultiLongValuesSource>::unwrap_singleton(&*v3).unwrap();
    assert_object_equals(s1.source_eq(&*s2), &s1, &s2);
    assert_object_not_equals(s1.source_eq(&*s3), &s1);
    Assert::assert_equals(s1.source_hash(), s2.source_hash());
    assert_not_equals(&s1.source_hash(), &s3.source_hash());

    let d1 = to_multi_double_values_source(v1);
    let d2 = to_multi_double_values_source(v2);
    let d3 = to_multi_double_values_source(v3);
    assert_object_equals(d1.source_eq(&*d2), &d1, &d2);
    assert_object_not_equals(d1.source_eq(&*d3), &d1);
    Assert::assert_equals(d1.source_hash(), d2.source_hash());
    assert_not_equals(&d1.source_hash(), &d3.source_hash());
    Ok(())
}
