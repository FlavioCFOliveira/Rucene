//! Port of `lucene/core/src/test/org/apache/lucene/document/column/TestColumnBatchLongColumn.java`:
//! tests for `LongColumn` batch indexing.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use rucene::document::column::NumericKind;
use rucene::document::{
    DoublePoint, FieldType, FloatPoint, IntPoint, LongPoint, NumericDocValuesField,
    SortedNumericDocValuesField,
};
use rucene::error::Result;
use rucene::index::DocValuesType;
use rucene::search::NO_MORE_DOCS;
use rucene::util::NumericUtils;
use rucene_test_framework::util::java_exceptions::expect_throws;

use super::column_batch_test_util::{
    field, ft, iota, nn, only_leaf, searcher, simple_batch, widen, ArrayDenseLongColumn,
    ArrayLongColumn, Ctx,
};
use crate::core_support::java_number::{double_value, float_value, int_value, long_value};
use crate::core_support::{assert_equals, assert_equals_delta, class, test, IAE};

/// `TestColumnBatchLongColumn`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.document.column.TestColumnBatchLongColumn",
        vec![
            test("testNumericDocValues", test_numeric_doc_values),
            test("testSortedNumericDocValues", test_sorted_numeric_doc_values),
            test("testSparseDocValues", test_sparse_doc_values),
            test("testPointsColumn", test_points_column),
            test("testPointsWithDocValues", test_points_with_doc_values),
            test("testSparsePointsColumn", test_sparse_points_column),
            test("testStoredLongColumn", test_stored_long_column),
            test("testStoredOnlyColumn", test_stored_only_column),
            test("testStoredPointsColumn", test_stored_points_column),
            test("testDenseNumericDocValues", test_dense_numeric_doc_values),
            test(
                "testDenseSortedNumericDocValues",
                test_dense_sorted_numeric_doc_values,
            ),
            test(
                "testIntSparseNumericDocValues",
                test_int_sparse_numeric_doc_values,
            ),
            test(
                "testLongColumnPointWidthMismatchThrows",
                test_long_column_point_width_mismatch_throws,
            ),
            test(
                "testLongColumnMultiDimPointsThrows",
                test_long_column_multi_dim_points_throws,
            ),
            test(
                "testDenseLongColumnWithStoredFields",
                test_dense_long_column_with_stored_fields,
            ),
            test(
                "testStoredTypeIntegerFromLongColumn",
                test_stored_type_integer_from_long_column,
            ),
            test(
                "testStoredTypeLongFromLongColumn",
                test_stored_type_long_from_long_column,
            ),
            test(
                "testStoredTypeFloatFromLongColumn",
                test_stored_type_float_from_long_column,
            ),
            test(
                "testStoredTypeDoubleFromLongColumn",
                test_stored_type_double_from_long_column,
            ),
            test(
                "testStoredTypeFloatWithNumericDV",
                test_stored_type_float_with_numeric_dv,
            ),
            test(
                "testStoredTypeIntegerWithNumericDV",
                test_stored_type_integer_with_numeric_dv,
            ),
            test(
                "testStoredTypeDoubleWithNumericDV",
                test_stored_type_double_with_numeric_dv,
            ),
            test(
                "testNumericKindIntPointsAndDV",
                test_numeric_kind_int_points_and_dv,
            ),
            test(
                "testNumericKindLongPointsAndDV",
                test_numeric_kind_long_points_and_dv,
            ),
            test(
                "testNumericKindFloatPointsAndDV",
                test_numeric_kind_float_points_and_dv,
            ),
            test(
                "testNumericKindDoublePointsAndDV",
                test_numeric_kind_double_points_and_dv,
            ),
            test(
                "testNumericKindPointsAndDVMultiDimRejected",
                test_numeric_kind_points_and_dv_multi_dim_rejected,
            ),
            test(
                "testNumericKindPointsAndDVWidthMismatch",
                test_numeric_kind_points_and_dv_width_mismatch,
            ),
            test(
                "testNumericKindFloatDVOnly",
                test_numeric_kind_float_dv_only,
            ),
            test("testDensePointsIntOnly", test_dense_points_int_only),
            test("testDensePointsLongOnly", test_dense_points_long_only),
            test("testDensePointsFloatOnly", test_dense_points_float_only),
            test("testDensePointsDoubleOnly", test_dense_points_double_only),
            test(
                "testDensePointsIntWithSortedNumericDV",
                test_dense_points_int_with_sorted_numeric_dv,
            ),
            test(
                "testDensePointsLongWithNumericDV",
                test_dense_points_long_with_numeric_dv,
            ),
            test(
                "testDensePointsLongLargeBatch",
                test_dense_points_long_large_batch,
            ),
        ],
    )
}

const INT_BYTES: i32 = 4;
const LONG_BYTES: i32 = 8;
const FLOAT_BYTES: i32 = 4;
const DOUBLE_BYTES: i32 = 8;

/// `new FieldType()` with `setDimensions(dims, bytes)` (when `dims > 0`),
/// `setStored(stored)` and `setDocValuesType(dv)` (when given), frozen.
fn field_type(dims: i32, bytes: i32, stored: bool, dv: Option<DocValuesType>) -> Result<FieldType> {
    let mut t = FieldType::new();
    if stored {
        t.set_stored(true)?;
    }
    if dims > 0 {
        t.set_dimensions(dims, bytes)?;
    }
    if let Some(dv) = dv {
        t.set_doc_values_type(dv)?;
    }
    t.freeze();
    Ok(t)
}

fn float_sortable(raw: &[f32]) -> Vec<i64> {
    raw.iter()
        .map(|&f| NumericUtils::float_to_sortable_int(f) as i64)
        .collect()
}

fn double_sortable(raw: &[f64]) -> Vec<i64> {
    raw.iter()
        .map(|&d| NumericUtils::double_to_sortable_long(d))
        .collect()
}

/// Port of `testNumericDocValues`.
fn test_numeric_doc_values() -> Result<()> {
    let c = Ctx::open()?;
    let values = vec![10i64, 20, 30];
    let doc_ids = vec![0, 1, 2];
    c.add_batch(&simple_batch(
        3,
        vec![ArrayLongColumn::new(
            "numeric",
            ft(NumericDocValuesField::type_()),
            doc_ids,
            values.clone(),
        )?
        .into()],
    ))?;
    let r = c.reader()?;
    let leaf = only_leaf(&r)?;
    let mut dv = nn(leaf.get_numeric_doc_values("numeric")?);
    for i in 0..values.len() {
        assert_equals(&(i as i32), &dv.next_doc()?);
        assert_equals(&values[i], &dv.long_value()?);
    }
    assert_equals(&NO_MORE_DOCS, &dv.next_doc()?);
    c.close(Some(r))
}

/// Port of `testSortedNumericDocValues`.
fn test_sorted_numeric_doc_values() -> Result<()> {
    let c = Ctx::open()?;
    // Doc 0 has two values, doc 1 has one value
    let doc_ids = vec![0, 0, 1];
    let values = vec![5i64, 15, 25];
    c.add_batch(&simple_batch(
        2,
        vec![ArrayLongColumn::new(
            "sortedNumeric",
            ft(SortedNumericDocValuesField::type_()),
            doc_ids,
            values,
        )?
        .into()],
    ))?;
    let r = c.reader()?;
    let leaf = only_leaf(&r)?;
    let mut dv = nn(leaf.get_sorted_numeric_doc_values("sortedNumeric")?);

    assert_equals(&0, &dv.next_doc()?);
    assert_equals(&2, &dv.doc_value_count()?);
    assert_equals(&5i64, &dv.next_value()?);
    assert_equals(&15i64, &dv.next_value()?);

    assert_equals(&1, &dv.next_doc()?);
    assert_equals(&1, &dv.doc_value_count()?);
    assert_equals(&25i64, &dv.next_value()?);

    assert_equals(&NO_MORE_DOCS, &dv.next_doc()?);
    c.close(Some(r))
}

/// Port of `testSparseDocValues`.
fn test_sparse_doc_values() -> Result<()> {
    let c = Ctx::open()?;
    // Only doc 1 has a value (docs 0 and 2 are missing)
    c.add_batch(&simple_batch(
        3,
        vec![ArrayLongColumn::new(
            "sparse",
            ft(NumericDocValuesField::type_()),
            vec![1],
            vec![42],
        )?
        .into()],
    ))?;
    let r = c.reader()?;
    let leaf = only_leaf(&r)?;
    let mut dv = nn(leaf.get_numeric_doc_values("sparse")?);
    assert_equals(&1, &dv.next_doc()?);
    assert_equals(&42i64, &dv.long_value()?);
    assert_equals(&NO_MORE_DOCS, &dv.next_doc()?);
    c.close(Some(r))
}

/// Port of `testPointsColumn`.
fn test_points_column() -> Result<()> {
    let c = Ctx::open()?;
    // Create a points-only FieldType (1 dimension, Integer.BYTES)
    let point_type = field_type(1, INT_BYTES, false, None)?;
    let raw = [10, 20, 30];
    c.add_batch(&simple_batch(
        3,
        vec![ArrayLongColumn::with_kind(
            "point",
            ft(&point_type),
            NumericKind::Int,
            vec![0, 1, 2],
            widen(&raw),
        )?
        .into()],
    ))?;
    let r = c.reader()?;
    let s = searcher(&r)?;
    assert_equals(&1, &s.count(IntPoint::new_exact_query("point", 10)?)?);
    assert_equals(&1, &s.count(IntPoint::new_exact_query("point", 20)?)?);
    assert_equals(&1, &s.count(IntPoint::new_exact_query("point", 30)?)?);
    assert_equals(&0, &s.count(IntPoint::new_exact_query("point", 99)?)?);
    assert_equals(
        &3,
        &s.count(IntPoint::new_range_query("point", &[10], &[30])?)?,
    );
    c.close(Some(r))
}

/// Port of `testPointsWithDocValues`.
fn test_points_with_doc_values() -> Result<()> {
    let c = Ctx::open()?;
    // 1D int points + SORTED_NUMERIC DV via the compat layer.
    let point_and_dv_type = field_type(1, INT_BYTES, false, Some(DocValuesType::SORTED_NUMERIC))?;
    let raw = [10, 20, 30];
    c.add_batch(&simple_batch(
        3,
        vec![ArrayLongColumn::with_kind(
            "field",
            ft(&point_and_dv_type),
            NumericKind::Int,
            vec![0, 1, 2],
            widen(&raw),
        )?
        .into()],
    ))?;
    let r = c.reader()?;
    let s = searcher(&r)?;
    assert_equals(&1, &s.count(IntPoint::new_exact_query("field", 10)?)?);
    assert_equals(
        &3,
        &s.count(IntPoint::new_range_query("field", &[10], &[30])?)?,
    );
    let leaf = only_leaf(&r)?;
    let mut dv = nn(leaf.get_sorted_numeric_doc_values("field")?);
    for i in 0..3 {
        assert_equals(&(i as i32), &dv.next_doc()?);
        assert_equals(&(raw[i] as i64), &dv.next_value()?);
    }
    c.close(Some(r))
}

/// Port of `testSparsePointsColumn`.
fn test_sparse_points_column() -> Result<()> {
    let c = Ctx::open()?;
    let point_type = field_type(1, INT_BYTES, false, None)?;
    // Only doc 1 out of 3 has a point value
    c.add_batch(&simple_batch(
        3,
        vec![ArrayLongColumn::with_kind(
            "point",
            ft(&point_type),
            NumericKind::Int,
            vec![1],
            vec![42],
        )?
        .into()],
    ))?;
    let r = c.reader()?;
    let s = searcher(&r)?;
    assert_equals(&1, &s.count(IntPoint::new_exact_query("point", 42)?)?);
    assert_equals(&0, &s.count(IntPoint::new_exact_query("point", 0)?)?);
    c.close(Some(r))
}

/// Port of `testStoredLongColumn`.
fn test_stored_long_column() -> Result<()> {
    let c = Ctx::open()?;
    // stored + NUMERIC doc values
    let stored_numeric_type = field_type(0, 0, true, Some(DocValuesType::NUMERIC))?;
    let values = vec![100i64, 200, 300];
    c.add_batch(&simple_batch(
        3,
        vec![ArrayLongColumn::new(
            "val",
            ft(&stored_numeric_type),
            vec![0, 1, 2],
            values.clone(),
        )?
        .into()],
    ))?;
    let r = c.reader()?;
    let leaf = only_leaf(&r)?;
    // Verify stored fields
    let stored_fields = leaf.stored_fields()?;
    for i in 0..3 {
        let doc = stored_fields.document(i as i32)?;
        assert_equals(&values[i], &long_value(field(&doc, "val").numeric_value()));
    }
    // Verify doc values
    let mut dv = nn(leaf.get_numeric_doc_values("val")?);
    for i in 0..3 {
        assert_equals(&(i as i32), &dv.next_doc()?);
        assert_equals(&values[i], &dv.long_value()?);
    }
    c.close(Some(r))
}

/// Port of `testStoredOnlyColumn`.
fn test_stored_only_column() -> Result<()> {
    let c = Ctx::open()?;
    // stored only — no doc values, no points
    let stored_only_type = field_type(0, 0, true, None)?;
    let values = vec![10i64, 20, 30];
    c.add_batch(&simple_batch(
        3,
        vec![ArrayLongColumn::new(
            "stored",
            ft(&stored_only_type),
            vec![0, 1, 2],
            values.clone(),
        )?
        .into()],
    ))?;
    let r = c.reader()?;
    let leaf = only_leaf(&r)?;
    let stored_fields = leaf.stored_fields()?;
    for i in 0..3 {
        let doc = stored_fields.document(i as i32)?;
        assert_equals(
            &values[i],
            &long_value(field(&doc, "stored").numeric_value()),
        );
    }
    c.close(Some(r))
}

/// Port of `testStoredPointsColumn`.
fn test_stored_points_column() -> Result<()> {
    let c = Ctx::open()?;
    // stored + points
    let stored_point_type = field_type(1, INT_BYTES, true, None)?;
    let raw = [10, 20, 30];
    c.add_batch(&simple_batch(
        3,
        vec![ArrayLongColumn::with_kind(
            "pt",
            ft(&stored_point_type),
            NumericKind::Int,
            vec![0, 1, 2],
            widen(&raw),
        )?
        .into()],
    ))?;
    let r = c.reader()?;
    let leaf = only_leaf(&r)?;
    // Verify stored fields — decoded as ints.
    let stored_fields = leaf.stored_fields()?;
    for i in 0..raw.len() {
        let doc = stored_fields.document(i as i32)?;
        assert_equals(&raw[i], &int_value(field(&doc, "pt").numeric_value()));
    }
    // Verify points
    let s = searcher(&r)?;
    assert_equals(&1, &s.count(IntPoint::new_exact_query("pt", 10)?)?);
    assert_equals(
        &3,
        &s.count(IntPoint::new_range_query("pt", &[10], &[30])?)?,
    );
    c.close(Some(r))
}

/// Port of `testDenseNumericDocValues`.
fn test_dense_numeric_doc_values() -> Result<()> {
    let c = Ctx::open()?;
    let values = vec![100i64, 200, 300];
    c.add_batch(&simple_batch(
        3,
        vec![
            ArrayDenseLongColumn::new("val", ft(NumericDocValuesField::type_()), values.clone())?
                .into(),
        ],
    ))?;
    let r = c.reader()?;
    let leaf = only_leaf(&r)?;
    let mut dv = nn(leaf.get_numeric_doc_values("val")?);
    for i in 0..3 {
        assert_equals(&(i as i32), &dv.next_doc()?);
        assert_equals(&values[i], &dv.long_value()?);
    }
    c.close(Some(r))
}

/// Port of `testDenseSortedNumericDocValues`.
fn test_dense_sorted_numeric_doc_values() -> Result<()> {
    let c = Ctx::open()?;
    let values = vec![10i64, 20, 30, 40, 50];
    c.add_batch(&simple_batch(
        5,
        vec![ArrayDenseLongColumn::new(
            "val",
            ft(SortedNumericDocValuesField::type_()),
            values.clone(),
        )?
        .into()],
    ))?;
    let r = c.reader()?;
    let leaf = only_leaf(&r)?;
    let mut dv = nn(leaf.get_sorted_numeric_doc_values("val")?);
    for i in 0..5 {
        assert_equals(&(i as i32), &dv.next_doc()?);
        assert_equals(&1, &dv.doc_value_count()?);
        assert_equals(&values[i], &dv.next_value()?);
    }
    c.close(Some(r))
}

/// Port of `testIntSparseNumericDocValues`.
fn test_int_sparse_numeric_doc_values() -> Result<()> {
    let c = Ctx::open()?;
    // Sparse: only docs 0 and 2 have values.
    let raw = [-7, 9];
    c.add_batch(&simple_batch(
        3,
        vec![ArrayLongColumn::with_kind(
            "val",
            ft(NumericDocValuesField::type_()),
            NumericKind::Int,
            vec![0, 2],
            widen(&raw),
        )?
        .into()],
    ))?;
    let r = c.reader()?;
    let leaf = only_leaf(&r)?;
    let mut dv = nn(leaf.get_numeric_doc_values("val")?);
    assert_equals(&0, &dv.next_doc()?);
    assert_equals(&-7i64, &dv.long_value()?);
    assert_equals(&2, &dv.next_doc()?);
    assert_equals(&9i64, &dv.long_value()?);
    assert_equals(&NO_MORE_DOCS, &dv.next_doc()?);
    c.close(Some(r))
}

/// `expectThrows(IllegalArgumentException.class, () -> w.addBatch(simpleBatch(1, column)))`
/// over a one-document `ArrayLongColumn` `val`/`pt` of `kind`, then the
/// test's `w.close(); dir.close();`.
fn expect_iae_one_doc(name: &str, t: &FieldType, kind: NumericKind, value: i64) -> Result<()> {
    let c = Ctx::open()?;
    expect_throws(IAE, || {
        c.add_batch(&simple_batch(
            1,
            vec![ArrayLongColumn::with_kind(name, ft(t), kind, vec![0], vec![value])?.into()],
        ))
    });
    c.close(None)
}

/// Port of `testLongColumnPointWidthMismatchThrows`.
fn test_long_column_point_width_mismatch_throws() -> Result<()> {
    let point_type = field_type(1, INT_BYTES, false, None)?; // expects 4 bytes
                                                             // LONG kind implies 8-byte point bytes; should fail validation against a 4-byte point type.
    expect_iae_one_doc("pt", &point_type, NumericKind::Long, 1)
}

/// Port of `testLongColumnMultiDimPointsThrows`.
fn test_long_column_multi_dim_points_throws() -> Result<()> {
    let point_type = field_type(2, LONG_BYTES, false, None)?;
    // `new ArrayLongColumn("pt", pointType, new int[] {0}, new long[] {1})`: kind LONG.
    expect_iae_one_doc("pt", &point_type, NumericKind::Long, 1)
}

/// Port of `testDenseLongColumnWithStoredFields`.
fn test_dense_long_column_with_stored_fields() -> Result<()> {
    // Covers the "single column consumed by both passes via fresh cursors" case: a dense
    // LongColumn with stored+numeric DV. Row pass uses tuples(), column pass uses values().
    let c = Ctx::open()?;
    let stored_numeric_type = field_type(0, 0, true, Some(DocValuesType::NUMERIC))?;
    let values = vec![100i64, 200, 300, 400];
    c.add_batch(&simple_batch(
        4,
        vec![ArrayDenseLongColumn::new("val", ft(&stored_numeric_type), values.clone())?.into()],
    ))?;
    let r = c.reader()?;
    let leaf = only_leaf(&r)?;
    let stored_fields = leaf.stored_fields()?;
    for i in 0..values.len() {
        let doc = stored_fields.document(i as i32)?;
        assert_equals(&values[i], &long_value(field(&doc, "val").numeric_value()));
    }
    let mut dv = nn(leaf.get_numeric_doc_values("val")?);
    for i in 0..values.len() {
        assert_equals(&(i as i32), &dv.next_doc()?);
        assert_equals(&values[i], &dv.long_value()?);
    }
    c.close(Some(r))
}

/// Indexes `values` (kind `kind`) as a stored column `val` over docs
/// `0..n`, with `dv` doc values when given; returns the open reader.
fn index_stored(
    c: &Ctx,
    kind: NumericKind,
    dv: Option<DocValuesType>,
    values: Vec<i64>,
) -> Result<Arc<dyn rucene::index::IndexReader>> {
    let t = field_type(0, 0, true, dv)?;
    let n = values.len();
    c.add_batch(&simple_batch(
        n as i32,
        vec![ArrayLongColumn::with_kind("val", ft(&t), kind, iota(n), values)?.into()],
    ))?;
    c.reader()
}

/// Port of `testStoredTypeIntegerFromLongColumn`.
fn test_stored_type_integer_from_long_column() -> Result<()> {
    let c = Ctx::open()?;
    let raw = [1, -2, 3];
    let r = index_stored(&c, NumericKind::Int, None, widen(&raw))?;
    let leaf = only_leaf(&r)?;
    let stored_fields = leaf.stored_fields()?;
    for i in 0..raw.len() {
        let doc = stored_fields.document(i as i32)?;
        assert_equals(&raw[i], &int_value(field(&doc, "val").numeric_value()));
    }
    c.close(Some(r))
}

/// Port of `testStoredTypeLongFromLongColumn`.
fn test_stored_type_long_from_long_column() -> Result<()> {
    let c = Ctx::open()?;
    let raw = [i64::MIN, 0, i64::MAX];
    // `new ArrayLongColumn("val", type, new int[] {0, 1, 2}, raw.clone())`: kind LONG.
    let r = index_stored(&c, NumericKind::Long, None, raw.to_vec())?;
    let leaf = only_leaf(&r)?;
    let stored_fields = leaf.stored_fields()?;
    for i in 0..raw.len() {
        let doc = stored_fields.document(i as i32)?;
        assert_equals(&raw[i], &long_value(field(&doc, "val").numeric_value()));
    }
    c.close(Some(r))
}

/// Port of `testStoredTypeFloatFromLongColumn`.
fn test_stored_type_float_from_long_column() -> Result<()> {
    let c = Ctx::open()?;
    let raw = [1.5f32, -2.25, f32::MAX];
    let r = index_stored(&c, NumericKind::Float, None, float_sortable(&raw))?;
    let leaf = only_leaf(&r)?;
    let stored_fields = leaf.stored_fields()?;
    for i in 0..raw.len() {
        let doc = stored_fields.document(i as i32)?;
        // `assertEquals(float, float, 0f)`.
        assert_equals_delta(
            raw[i] as f64,
            float_value(field(&doc, "val").numeric_value()) as f64,
            0.0,
        );
    }
    c.close(Some(r))
}

/// Port of `testStoredTypeDoubleFromLongColumn`.
fn test_stored_type_double_from_long_column() -> Result<()> {
    let c = Ctx::open()?;
    let raw = [1.5f64, -2.25, f64::MAX];
    let r = index_stored(&c, NumericKind::Double, None, double_sortable(&raw))?;
    let leaf = only_leaf(&r)?;
    let stored_fields = leaf.stored_fields()?;
    for i in 0..raw.len() {
        let doc = stored_fields.document(i as i32)?;
        assert_equals_delta(
            raw[i],
            double_value(field(&doc, "val").numeric_value()),
            0.0,
        );
    }
    c.close(Some(r))
}

/// Port of `testStoredTypeFloatWithNumericDV`.
fn test_stored_type_float_with_numeric_dv() -> Result<()> {
    // FLOAT kind on a LongColumn that also feeds NumericDV.
    let c = Ctx::open()?;
    let raw = [1.5f32, -2.25, 42.0];
    let values = float_sortable(&raw);
    let r = index_stored(
        &c,
        NumericKind::Float,
        Some(DocValuesType::NUMERIC),
        values.clone(),
    )?;
    let leaf = only_leaf(&r)?;
    // Stored values decoded as floats.
    let stored_fields = leaf.stored_fields()?;
    for i in 0..raw.len() {
        let doc = stored_fields.document(i as i32)?;
        assert_equals_delta(
            raw[i] as f64,
            float_value(field(&doc, "val").numeric_value()) as f64,
            0.0,
        );
    }
    // NumericDV stores the sortable-int encoding sign-extended to long.
    let mut dv = nn(leaf.get_numeric_doc_values("val")?);
    for i in 0..raw.len() {
        assert_equals(&(i as i32), &dv.next_doc()?);
        assert_equals(&values[i], &dv.long_value()?);
    }
    c.close(Some(r))
}

/// Port of `testStoredTypeIntegerWithNumericDV`.
fn test_stored_type_integer_with_numeric_dv() -> Result<()> {
    // INT kind on a LongColumn that also feeds NumericDV.
    let c = Ctx::open()?;
    let raw = [i32::MIN, -1, 0, 42, i32::MAX];
    let r = index_stored(
        &c,
        NumericKind::Int,
        Some(DocValuesType::NUMERIC),
        widen(&raw),
    )?;
    let leaf = only_leaf(&r)?;
    let stored_fields = leaf.stored_fields()?;
    for i in 0..raw.len() {
        let doc = stored_fields.document(i as i32)?;
        assert_equals(&raw[i], &int_value(field(&doc, "val").numeric_value()));
    }
    let mut dv = nn(leaf.get_numeric_doc_values("val")?);
    for i in 0..raw.len() {
        assert_equals(&(i as i32), &dv.next_doc()?);
        assert_equals(&(raw[i] as i64), &dv.long_value()?);
    }
    c.close(Some(r))
}

/// Port of `testStoredTypeDoubleWithNumericDV`.
fn test_stored_type_double_with_numeric_dv() -> Result<()> {
    // DOUBLE kind on a LongColumn that also feeds NumericDV.
    let c = Ctx::open()?;
    let raw = [f64::NEG_INFINITY, -1.5, 0.0, 2.25, f64::INFINITY];
    let values = double_sortable(&raw);
    let r = index_stored(
        &c,
        NumericKind::Double,
        Some(DocValuesType::NUMERIC),
        values.clone(),
    )?;
    let leaf = only_leaf(&r)?;
    let stored_fields = leaf.stored_fields()?;
    for i in 0..raw.len() {
        let doc = stored_fields.document(i as i32)?;
        assert_equals_delta(
            raw[i],
            double_value(field(&doc, "val").numeric_value()),
            0.0,
        );
    }
    let mut dv = nn(leaf.get_numeric_doc_values("val")?);
    for i in 0..raw.len() {
        assert_equals(&(i as i32), &dv.next_doc()?);
        assert_equals(&values[i], &dv.long_value()?);
    }
    c.close(Some(r))
}

/// Indexes `values` as an `ArrayLongColumn` `val` of `kind` over docs
/// `0..n` with a 1-D `bytes`-wide point and SORTED_NUMERIC doc values.
fn index_points_and_dv(
    c: &Ctx,
    bytes: i32,
    kind: NumericKind,
    values: Vec<i64>,
) -> Result<Arc<dyn rucene::index::IndexReader>> {
    let t = field_type(1, bytes, false, Some(DocValuesType::SORTED_NUMERIC))?;
    let n = values.len();
    c.add_batch(&simple_batch(
        n as i32,
        vec![ArrayLongColumn::with_kind("val", ft(&t), kind, iota(n), values)?.into()],
    ))?;
    c.reader()
}

/// Port of `testNumericKindIntPointsAndDV`.
fn test_numeric_kind_int_points_and_dv() -> Result<()> {
    let c = Ctx::open()?;
    let raw = [-5, -1, 0, 7, i32::MAX];
    let r = index_points_and_dv(&c, INT_BYTES, NumericKind::Int, widen(&raw))?;
    let leaf = only_leaf(&r)?;
    let mut dv = nn(leaf.get_sorted_numeric_doc_values("val")?);
    for i in 0..raw.len() {
        assert_equals(&(i as i32), &dv.next_doc()?);
        assert_equals(&1, &dv.doc_value_count()?);
        assert_equals(&(raw[i] as i64), &dv.next_value()?);
    }
    let s = searcher(&r)?;
    assert_equals(
        &(raw.len() as i32),
        &s.count(IntPoint::new_range_query("val", &[i32::MIN], &[i32::MAX])?)?,
    );
    assert_equals(&1, &s.count(IntPoint::new_exact_query("val", -5)?)?);
    assert_equals(
        &3,
        &s.count(IntPoint::new_range_query("val", &[-1], &[7])?)?,
    );
    c.close(Some(r))
}

/// Port of `testNumericKindLongPointsAndDV`.
fn test_numeric_kind_long_points_and_dv() -> Result<()> {
    let c = Ctx::open()?;
    let raw = [i64::MIN, -100, 0, 42, i64::MAX];
    // `new ArrayLongColumn("val", type, docIds, raw.clone())`: kind LONG.
    let r = index_points_and_dv(&c, LONG_BYTES, NumericKind::Long, raw.to_vec())?;
    let leaf = only_leaf(&r)?;
    let mut dv = nn(leaf.get_sorted_numeric_doc_values("val")?);
    for i in 0..raw.len() {
        assert_equals(&(i as i32), &dv.next_doc()?);
        assert_equals(&raw[i], &dv.next_value()?);
    }
    let s = searcher(&r)?;
    assert_equals(
        &(raw.len() as i32),
        &s.count(LongPoint::new_range_query("val", &[i64::MIN], &[i64::MAX])?)?,
    );
    assert_equals(&1, &s.count(LongPoint::new_exact_query("val", i64::MIN)?)?);
    assert_equals(
        &3,
        &s.count(LongPoint::new_range_query("val", &[-100], &[42])?)?,
    );
    c.close(Some(r))
}

/// Port of `testNumericKindFloatPointsAndDV`.
fn test_numeric_kind_float_points_and_dv() -> Result<()> {
    let c = Ctx::open()?;
    let raw = [f32::NEG_INFINITY, -1.5, 0.0, 2.25, f32::INFINITY];
    let r = index_points_and_dv(&c, FLOAT_BYTES, NumericKind::Float, float_sortable(&raw))?;
    let leaf = only_leaf(&r)?;
    // DV stores the sortable-int encoding; decode via sortableIntToFloat.
    let mut dv = nn(leaf.get_sorted_numeric_doc_values("val")?);
    for i in 0..raw.len() {
        assert_equals(&(i as i32), &dv.next_doc()?);
        assert_equals_delta(
            raw[i] as f64,
            NumericUtils::sortable_int_to_float(dv.next_value()? as i32) as f64,
            0.0,
        );
    }
    // Points sort numerically.
    let s = searcher(&r)?;
    assert_equals(
        &(raw.len() as i32),
        &s.count(FloatPoint::new_range_query(
            "val",
            &[f32::NEG_INFINITY],
            &[f32::INFINITY],
        )?)?,
    );
    assert_equals(&1, &s.count(FloatPoint::new_exact_query("val", -1.5)?)?);
    assert_equals(
        &3,
        &s.count(FloatPoint::new_range_query("val", &[-1.5], &[2.25])?)?,
    );
    c.close(Some(r))
}

/// Port of `testNumericKindDoublePointsAndDV`.
fn test_numeric_kind_double_points_and_dv() -> Result<()> {
    let c = Ctx::open()?;
    let raw = [f64::NEG_INFINITY, -1.5, 0.0, 2.25, f64::INFINITY];
    let r = index_points_and_dv(&c, DOUBLE_BYTES, NumericKind::Double, double_sortable(&raw))?;
    let leaf = only_leaf(&r)?;
    let mut dv = nn(leaf.get_sorted_numeric_doc_values("val")?);
    for i in 0..raw.len() {
        assert_equals(&(i as i32), &dv.next_doc()?);
        assert_equals_delta(
            raw[i],
            NumericUtils::sortable_long_to_double(dv.next_value()?),
            0.0,
        );
    }
    let s = searcher(&r)?;
    assert_equals(
        &(raw.len() as i32),
        &s.count(DoublePoint::new_range_query(
            "val",
            &[f64::NEG_INFINITY],
            &[f64::INFINITY],
        )?)?,
    );
    assert_equals(&1, &s.count(DoublePoint::new_exact_query("val", -1.5)?)?);
    assert_equals(
        &3,
        &s.count(DoublePoint::new_range_query("val", &[-1.5], &[2.25])?)?,
    );
    c.close(Some(r))
}

/// Port of `testNumericKindPointsAndDVMultiDimRejected`.
fn test_numeric_kind_points_and_dv_multi_dim_rejected() -> Result<()> {
    // 2D int: scenario 3 requires 1D.
    let t = field_type(2, INT_BYTES, false, Some(DocValuesType::SORTED_NUMERIC))?;
    expect_iae_one_doc("val", &t, NumericKind::Long, 1)
}

/// Port of `testNumericKindPointsAndDVWidthMismatch`.
fn test_numeric_kind_points_and_dv_width_mismatch() -> Result<()> {
    // FLOAT kind requires a 4-byte point field; declaring Long.BYTES should throw.
    let t = field_type(1, LONG_BYTES, false, Some(DocValuesType::SORTED_NUMERIC))?;
    expect_iae_one_doc("val", &t, NumericKind::Float, 1)
}

/// Port of `testNumericKindFloatDVOnly`.
fn test_numeric_kind_float_dv_only() -> Result<()> {
    // DV only (no points): LongColumn stores the long value unchanged. For FLOAT, callers feed
    // sortable-int bits in the low 32 bits, and DV reads them back sign-extended to long.
    let c = Ctx::open()?;
    let t = field_type(0, 0, false, Some(DocValuesType::NUMERIC))?;
    let raw = [1.5f32, -2.25, f32::MAX];
    let values = float_sortable(&raw);
    c.add_batch(&simple_batch(
        raw.len() as i32,
        vec![ArrayLongColumn::with_kind(
            "val",
            ft(&t),
            NumericKind::Float,
            iota(raw.len()),
            values.clone(),
        )?
        .into()],
    ))?;
    let r = c.reader()?;
    let leaf = only_leaf(&r)?;
    let mut dv = nn(leaf.get_numeric_doc_values("val")?);
    for i in 0..raw.len() {
        assert_equals(&(i as i32), &dv.next_doc()?);
        assert_equals(&values[i], &dv.long_value()?);
    }
    c.close(Some(r))
}

// -----------------------------------------------------------------------------
// Dense 1-D points fast path (PointValuesWriter#addDense1DIntValues /
// addDense1DLongValues, LongValuesCursor#fillIntPoints / fillLongPoints).
// -----------------------------------------------------------------------------

/// Indexes `values` as an `ArrayDenseLongColumn` `v` of `kind` with a 1-D
/// `bytes`-wide point and `dv` doc values when given.
fn index_dense_points(
    c: &Ctx,
    bytes: i32,
    dv: Option<DocValuesType>,
    kind: NumericKind,
    values: Vec<i64>,
) -> Result<Arc<dyn rucene::index::IndexReader>> {
    let t = field_type(1, bytes, false, dv)?;
    let n = values.len();
    c.add_batch(&simple_batch(
        n as i32,
        vec![ArrayDenseLongColumn::with_kind("v", ft(&t), kind, values)?.into()],
    ))?;
    c.reader()
}

/// Port of `testDensePointsIntOnly`.
fn test_dense_points_int_only() -> Result<()> {
    let c = Ctx::open()?;
    let raw = [i32::MIN, -7, 0, 13, i32::MAX];
    let r = index_dense_points(&c, INT_BYTES, None, NumericKind::Int, widen(&raw))?;
    let s = searcher(&r)?;
    assert_equals(
        &(raw.len() as i32),
        &s.count(IntPoint::new_range_query("v", &[i32::MIN], &[i32::MAX])?)?,
    );
    assert_equals(&1, &s.count(IntPoint::new_exact_query("v", -7)?)?);
    assert_equals(&1, &s.count(IntPoint::new_exact_query("v", 13)?)?);
    assert_equals(&0, &s.count(IntPoint::new_exact_query("v", 99)?)?);
    assert_equals(&3, &s.count(IntPoint::new_range_query("v", &[-7], &[13])?)?);
    c.close(Some(r))
}

/// Port of `testDensePointsLongOnly`.
fn test_dense_points_long_only() -> Result<()> {
    let c = Ctx::open()?;
    let values = [i64::MIN, -100, 0, 42, i64::MAX];
    let r = index_dense_points(&c, LONG_BYTES, None, NumericKind::Long, values.to_vec())?;
    let s = searcher(&r)?;
    assert_equals(
        &(values.len() as i32),
        &s.count(LongPoint::new_range_query("v", &[i64::MIN], &[i64::MAX])?)?,
    );
    assert_equals(&1, &s.count(LongPoint::new_exact_query("v", -100)?)?);
    assert_equals(
        &3,
        &s.count(LongPoint::new_range_query("v", &[-100], &[42])?)?,
    );
    c.close(Some(r))
}

/// Port of `testDensePointsFloatOnly`.
fn test_dense_points_float_only() -> Result<()> {
    let c = Ctx::open()?;
    let raw = [f32::NEG_INFINITY, -1.5, 0.0, 2.25, f32::INFINITY];
    let r = index_dense_points(
        &c,
        FLOAT_BYTES,
        None,
        NumericKind::Float,
        float_sortable(&raw),
    )?;
    let s = searcher(&r)?;
    assert_equals(
        &(raw.len() as i32),
        &s.count(FloatPoint::new_range_query(
            "v",
            &[f32::NEG_INFINITY],
            &[f32::INFINITY],
        )?)?,
    );
    assert_equals(&1, &s.count(FloatPoint::new_exact_query("v", -1.5)?)?);
    assert_equals(
        &3,
        &s.count(FloatPoint::new_range_query("v", &[-1.5], &[2.25])?)?,
    );
    c.close(Some(r))
}

/// Port of `testDensePointsDoubleOnly`.
fn test_dense_points_double_only() -> Result<()> {
    let c = Ctx::open()?;
    let raw = [f64::NEG_INFINITY, -1.5, 0.0, 2.25, f64::INFINITY];
    let r = index_dense_points(
        &c,
        DOUBLE_BYTES,
        None,
        NumericKind::Double,
        double_sortable(&raw),
    )?;
    let s = searcher(&r)?;
    assert_equals(
        &(raw.len() as i32),
        &s.count(DoublePoint::new_range_query(
            "v",
            &[f64::NEG_INFINITY],
            &[f64::INFINITY],
        )?)?,
    );
    assert_equals(&1, &s.count(DoublePoint::new_exact_query("v", -1.5)?)?);
    assert_equals(
        &3,
        &s.count(DoublePoint::new_range_query("v", &[-1.5], &[2.25])?)?,
    );
    c.close(Some(r))
}

/// Port of `testDensePointsIntWithSortedNumericDV`.
fn test_dense_points_int_with_sorted_numeric_dv() -> Result<()> {
    let c = Ctx::open()?;
    let raw = [-5, -1, 0, 7, i32::MAX];
    let r = index_dense_points(
        &c,
        INT_BYTES,
        Some(DocValuesType::SORTED_NUMERIC),
        NumericKind::Int,
        widen(&raw),
    )?;
    let leaf = only_leaf(&r)?;
    let mut dv = nn(leaf.get_sorted_numeric_doc_values("v")?);
    for i in 0..raw.len() {
        assert_equals(&(i as i32), &dv.next_doc()?);
        assert_equals(&1, &dv.doc_value_count()?);
        assert_equals(&(raw[i] as i64), &dv.next_value()?);
    }
    assert_equals(&NO_MORE_DOCS, &dv.next_doc()?);
    let s = searcher(&r)?;
    assert_equals(
        &(raw.len() as i32),
        &s.count(IntPoint::new_range_query("v", &[i32::MIN], &[i32::MAX])?)?,
    );
    assert_equals(&3, &s.count(IntPoint::new_range_query("v", &[-1], &[7])?)?);
    c.close(Some(r))
}

/// Port of `testDensePointsLongWithNumericDV`.
fn test_dense_points_long_with_numeric_dv() -> Result<()> {
    let c = Ctx::open()?;
    let values = [i64::MIN, -100, 0, 42, i64::MAX];
    let r = index_dense_points(
        &c,
        LONG_BYTES,
        Some(DocValuesType::NUMERIC),
        NumericKind::Long,
        values.to_vec(),
    )?;
    let leaf = only_leaf(&r)?;
    let mut dv = nn(leaf.get_numeric_doc_values("v")?);
    for i in 0..values.len() {
        assert_equals(&(i as i32), &dv.next_doc()?);
        assert_equals(&values[i], &dv.long_value()?);
    }
    assert_equals(&NO_MORE_DOCS, &dv.next_doc()?);
    let s = searcher(&r)?;
    assert_equals(
        &(values.len() as i32),
        &s.count(LongPoint::new_range_query("v", &[i64::MIN], &[i64::MAX])?)?,
    );
    assert_equals(
        &3,
        &s.count(LongPoint::new_range_query("v", &[-100], &[42])?)?,
    );
    c.close(Some(r))
}

/// Port of `testDensePointsLongLargeBatch`: a dense long batch larger than
/// `POINTS_BUFFER_LONG_VALUES` (512) so the chunked encoding loop in
/// `PointValuesWriter#addDense1DLongValues` runs more than once.
fn test_dense_points_long_large_batch() -> Result<()> {
    let c = Ctx::open()?;
    let n = 1100usize; // > 2 * 512 chunks
    let values: Vec<i64> = (0..n as i64).map(|i| i * 1_000_000 - 100).collect(); // mix of negative and positive
    let r = index_dense_points(&c, LONG_BYTES, None, NumericKind::Long, values.clone())?;
    let s = searcher(&r)?;
    assert_equals(
        &(n as i32),
        &s.count(LongPoint::new_range_query("v", &[i64::MIN], &[i64::MAX])?)?,
    );
    assert_equals(&1, &s.count(LongPoint::new_exact_query("v", values[0])?)?);
    assert_equals(
        &1,
        &s.count(LongPoint::new_exact_query("v", values[n - 1])?)?,
    );
    assert_equals(&1, &s.count(LongPoint::new_exact_query("v", values[511])?)?); // chunk boundary
    assert_equals(&1, &s.count(LongPoint::new_exact_query("v", values[512])?)?);
    assert_equals(&0, &s.count(LongPoint::new_exact_query("v", -1)?)?);
    assert_equals(
        &(n as i32),
        &s.count(LongPoint::new_range_query(
            "v",
            &[values[0]],
            &[values[n - 1]],
        )?)?,
    );
    c.close(Some(r))
}
