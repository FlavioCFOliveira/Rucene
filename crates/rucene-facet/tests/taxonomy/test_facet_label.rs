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

//! Port of `org.apache.lucene.facet.taxonomy.TestFacetLabel`.
//!
//! `testEmptyNullComponents`: the rows with a `null` component (and the
//! `null` dimension cases) cannot be built, because the port's constructors
//! take `&str` (Java's `null` check is carried by the type); the rows with an
//! empty component or dimension are ported as is.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::util::BytesRef;
use rucene_facet::facet_field::FacetField;
use rucene_facet::sortedset::sorted_set_doc_values_facet_field::SortedSetDocValuesFacetField;
use rucene_facet::taxonomy::association_facet_field::AssociationFacetField;
use rucene_facet::taxonomy::facet_label::FacetLabel;
use rucene_facet::taxonomy::float_association_facet_field::FloatAssociationFacetField;
use rucene_facet::taxonomy::int_association_facet_field::IntAssociationFacetField;
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::facet_support::core_support::test;
use crate::facet_support::facet_class;

const IAE: &str = "java.lang.IllegalArgumentException";

/// `TestFacetLabel`.
pub fn class_def() -> Arc<Class> {
    facet_class(
        "org.apache.lucene.facet.taxonomy.TestFacetLabel",
        vec![
            test("testBasic", test_basic),
            test("testToString", test_to_string),
            test("testGetComponent", test_get_component),
            test("testDefaultConstructor", test_default_constructor),
            test("testSubPath", test_sub_path),
            test("testEquals", test_equals),
            test("testHashCode", test_hash_code),
            test("testLongHashCode", test_long_hash_code),
            test("testArrayConstructor", test_array_constructor),
            test("testCompareTo", test_compare_to),
            test("testEmptyNullComponents", test_empty_null_components),
            test("testLongPath", test_long_path),
        ],
    )
}

fn fl(components: &[&str]) -> Result<FacetLabel> {
    FacetLabel::new(components.iter().copied())
}

fn test_basic() -> Result<()> {
    Assert::assert_equals(0usize, FacetLabel::empty().length);
    Assert::assert_equals(1usize, fl(&["hello"])?.length);
    Assert::assert_equals(2usize, fl(&["hello", "world"])?.length);
    Ok(())
}

fn test_to_string() -> Result<()> {
    // When the category is empty, we expect an empty string
    Assert::assert_equals("FacetLabel: []", FacetLabel::empty().to_string().as_str());
    // one category
    Assert::assert_equals("FacetLabel: [hello]", fl(&["hello"])?.to_string().as_str());
    // more than one category
    Assert::assert_equals(
        "FacetLabel: [hello, world]",
        fl(&["hello", "world"])?.to_string().as_str(),
    );
    Ok(())
}

fn test_get_component() -> Result<()> {
    let components: Vec<String> = (0..LuceneTestCase::at_least(10))
        .map(|i| i.to_string())
        .collect();
    let cp = FacetLabel::new(components.iter().cloned())?;
    for i in 0..components.len() {
        Assert::assert_equals(i as i32, cp.components[i].parse::<i32>().unwrap());
    }
    Ok(())
}

fn test_default_constructor() -> Result<()> {
    // test that the default constructor (no parameters) currently
    // defaults to creating an object with a 0 initial capacity.
    // If we change this default later, we also need to change this
    // test.
    let p = FacetLabel::empty();
    Assert::assert_equals(0usize, p.length);
    Assert::assert_equals("FacetLabel: []", p.to_string().as_str());
    Ok(())
}

fn test_sub_path() -> Result<()> {
    let p = fl(&["hi", "there", "man"])?;
    Assert::assert_equals(p.length, 3);

    let p1 = p.subpath_i32(2);
    Assert::assert_equals(2usize, p1.length);
    Assert::assert_equals("FacetLabel: [hi, there]", p1.to_string().as_str());

    let p1 = p.subpath_i32(1);
    Assert::assert_equals(1usize, p1.length);
    Assert::assert_equals("FacetLabel: [hi]", p1.to_string().as_str());

    let p1 = p.subpath_i32(0);
    Assert::assert_equals(0usize, p1.length);
    Assert::assert_equals("FacetLabel: []", p1.to_string().as_str());

    // with all the following lengths, the prefix should be the whole path
    let lengths = [3, -1, 4];
    for length in lengths {
        let p1 = p.subpath_i32(length);
        Assert::assert_equals(3usize, p1.length);
        Assert::assert_equals("FacetLabel: [hi, there, man]", p1.to_string().as_str());
        Assert::assert_equals(&p, &p1);
    }
    Ok(())
}

fn test_equals() -> Result<()> {
    Assert::assert_equals(&FacetLabel::empty(), &FacetLabel::empty());
    Assert::assert_false(FacetLabel::empty() == fl(&["hi"])?);
    // `new FacetLabel().equals(Integer.valueOf(3))`: an object of another
    // class is never equal.
    Assert::assert_false((&FacetLabel::empty() as &dyn std::any::Any).is::<i32>());
    Assert::assert_equals(&fl(&["hello", "world"])?, &fl(&["hello", "world"])?);
    Ok(())
}

fn test_hash_code() -> Result<()> {
    Assert::assert_equals(
        FacetLabel::empty().hash_code(),
        FacetLabel::empty().hash_code(),
    );
    Assert::assert_false(FacetLabel::empty().hash_code() == fl(&["hi"])?.hash_code());
    Assert::assert_equals(
        fl(&["hello", "world"])?.hash_code(),
        fl(&["hello", "world"])?.hash_code(),
    );
    Ok(())
}

fn test_long_hash_code() -> Result<()> {
    Assert::assert_equals(
        FacetLabel::empty().long_hash_code(),
        FacetLabel::empty().long_hash_code(),
    );
    Assert::assert_false(FacetLabel::empty().long_hash_code() == fl(&["hi"])?.long_hash_code());
    Assert::assert_equals(
        fl(&["hello", "world"])?.long_hash_code(),
        fl(&["hello", "world"])?.long_hash_code(),
    );
    Ok(())
}

fn test_array_constructor() -> Result<()> {
    let p = fl(&["hello", "world", "yo"])?;
    Assert::assert_equals(3usize, p.length);
    Assert::assert_equals("FacetLabel: [hello, world, yo]", p.to_string().as_str());
    Ok(())
}

fn test_compare_to() -> Result<()> {
    let p = fl(&["a", "b", "c", "d"])?;
    let pother = fl(&["a", "b", "c", "d"])?;
    Assert::assert_equals(0, pother.compare_to(&p));
    Assert::assert_equals(0, p.compare_to(&pother));
    let pother = FacetLabel::empty();
    Assert::assert_true(pother.compare_to(&p) < 0);
    Assert::assert_true(p.compare_to(&pother) > 0);
    let pother = fl(&["a", "b_", "c", "d"])?;
    Assert::assert_true(pother.compare_to(&p) > 0);
    Assert::assert_true(p.compare_to(&pother) < 0);
    let pother = fl(&["a", "b", "c"])?;
    Assert::assert_true(pother.compare_to(&p) < 0);
    Assert::assert_true(p.compare_to(&pother) > 0);
    let pother = fl(&["a", "b", "c", "e"])?;
    Assert::assert_true(pother.compare_to(&p) > 0);
    Assert::assert_true(p.compare_to(&pother) < 0);
    Ok(())
}

fn test_empty_null_components() -> Result<()> {
    // LUCENE-4724: CategoryPath should not allow empty or null components
    let components_tests: [&[&str]; 3] = [
        &["", "test"], // empty in the beginning
        &["test", ""], // empty in the end
        &["test", "", "foo"], // empty in the middle
                       // `{null, "test"}`, `{"test", null}`, `{"test", null, "foo"}`:
                       // not representable (`&str`), see the module docs.
    ];

    // empty or null components should not be allowed.
    for components in components_tests {
        expect_throws(IAE, || fl(components));
        expect_throws(IAE, || FacetField::new("dim", components));
        expect_throws(IAE, || {
            AssociationFacetField::new(BytesRef::new(Vec::new()), "dim", components)
        });
        expect_throws(IAE, || IntAssociationFacetField::new(17, "dim", components));
        expect_throws(IAE, || {
            FloatAssociationFacetField::new(17.0, "dim", components)
        });
    }
    // `new FacetField(null, ...)` and the other `null` dimensions: not
    // representable, see the module docs.
    expect_throws(IAE, || FacetField::new("", &["abc"]));
    expect_throws(IAE, || IntAssociationFacetField::new(17, "", &["abc"]));
    expect_throws(IAE, || FloatAssociationFacetField::new(17.0, "", &["abc"]));
    expect_throws(IAE, || {
        AssociationFacetField::new(BytesRef::new(Vec::new()), "", &["abc"])
    });
    expect_throws(IAE, || SortedSetDocValuesFacetField::new("", &["abc"]));
    expect_throws(IAE, || SortedSetDocValuesFacetField::new("dim", &[""]));
    Ok(())
}

fn test_long_path() -> Result<()> {
    let random = LuceneTestCase::random();
    let big_comp = loop {
        let len = FacetLabel::MAX_CATEGORY_PATH_LENGTH as i32;
        let big_comp = TestUtil::random_simple_string_between(&*random, len, len);
        if big_comp.contains('\u{001f}') {
            continue;
        }
        break big_comp;
    };

    // long paths should not be allowed
    expect_throws(IAE, || fl(&["dim", &big_comp]));
    Ok(())
}
