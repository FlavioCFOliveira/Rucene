//! Port of `lucene/core/src/test/org/apache/lucene/document/TestLateInteractionField.java`.
//!
//! Java's `float[][]` may be `null` and hold `null` rows; the port's
//! `LateInteractionField` takes it as `Option<&[Option<Vec<f32>>]>`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use rucene::document::shape_field::LateInteractionField;
use rucene::error::Result;
use rucene_test_framework::util::java_exceptions::expect_throws;

use crate::core_support::{assert_equals, assert_equals_delta, class, random, test, IAE};

/// `TestLateInteractionField`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.document.TestLateInteractionField",
        vec![
            test("testEncodeDecode", test_encode_decode),
            test("testSetterGetter", test_setter_getter),
            test("testInputValidation", test_input_validation),
        ],
    )
}

/// `TestVectorUtil.randomVector(dim)`.
fn random_vector(dim: i32) -> Vec<f32> {
    let random = random();
    (0..dim).map(|_| random.next_float()).collect()
}

/// `float[][]` of non-null rows, as the field takes it.
fn rows(v: &[Vec<f32>]) -> Vec<Option<Vec<f32>>> {
    v.iter().cloned().map(Some).collect()
}

/// Port of `testEncodeDecode`.
fn test_encode_decode() -> Result<()> {
    let n = random().next_int_range(3, 12);
    let dim = 128;
    let mut value = Vec::with_capacity(n as usize);
    for _i in 0..n {
        value.push(random_vector(dim));
    }
    let field = LateInteractionField::new("test", Some(&rows(&value)))?;
    let encoded = LateInteractionField::encode(Some(&rows(&value)))?;
    let decoded = LateInteractionField::decode(&encoded)?;
    assert_equal_arrays(&value, &decoded);
    assert_equal_arrays(&value, &field.get_value()?);
    Ok(())
}

/// Port of `testSetterGetter`.
fn test_setter_getter() -> Result<()> {
    let dim = 128;
    let n = random().next_int_range(3, 12);
    let mut value = Vec::with_capacity(n as usize);
    for _i in 0..n {
        value.push(random_vector(dim));
    }
    let field = LateInteractionField::new("test", Some(&rows(&value)))?;
    let n2 = random().next_int_range(3, 12);
    let mut value2 = Vec::with_capacity(n2 as usize);
    for _i in 0..n2 {
        value2.push(random_vector(dim));
    }
    assert_equal_arrays(&field.get_value()?, &value);
    field.set_value(Some(&rows(&value2)))?;
    assert_equal_arrays(&field.get_value()?, &value2);
    Ok(())
}

/// Port of `testInputValidation`.
fn test_input_validation() -> Result<()> {
    expect_throws(IAE, || LateInteractionField::encode(None));
    expect_throws(IAE, || LateInteractionField::new("test", None));
    expect_throws(IAE, || LateInteractionField::encode(Some(&[])));
    // `new float[3][]`: three null rows.
    expect_throws(IAE, || {
        LateInteractionField::encode(Some(&[None, None, None]))
    });
    let empty_tokens = vec![Some(Vec::<f32>::new())];
    expect_throws(IAE, || LateInteractionField::encode(Some(&empty_tokens)));

    let dim = 128;
    let num_vectors = random().next_int_range(3, 12);
    let mut value: Vec<Option<Vec<f32>>> = vec![None; num_vectors as usize];
    for i in 0..(num_vectors - 1) as usize {
        if random().next_boolean() {
            value[i] = Some(random_vector(dim));
        } else {
            value[i] = Some(random_vector(dim + 1));
        }
    }
    value[(num_vectors - 1) as usize] = Some(random_vector(dim + 2));
    expect_throws(IAE, || LateInteractionField::encode(Some(&value)));
    Ok(())
}

/// `assertEqualArrays(a, b)`.
fn assert_equal_arrays(a: &[Vec<f32>], b: &[Vec<f32>]) {
    assert_equals(&a.len(), &b.len());
    for i in 0..a.len() {
        assert_equals(&a[i].len(), &b[i].len());
        for j in 0..a[i].len() {
            assert_equals_delta(a[i][j] as f64, b[i][j] as f64, 1e-5f32 as f64);
        }
    }
}
