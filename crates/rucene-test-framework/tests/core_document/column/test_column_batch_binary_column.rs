//! Port of `lucene/core/src/test/org/apache/lucene/document/column/TestColumnBatchBinaryColumn.java`:
//! tests for `BinaryColumn` batch indexing.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use rucene::document::column::NumericKind;
use rucene::document::{
    BinaryDocValuesField, BinaryPoint, IntPoint, NumericDocValuesField, SortedDocValuesField,
    SortedSetDocValuesField, StoredValueType, StringField, TextField,
};
use rucene::error::Result;
use rucene::index::{DocValuesType, IndexReader};
use rucene::search::{IndexSearcher, NO_MORE_DOCS};
use rucene::util::{BytesRef, NumericUtils};
use rucene_test_framework::util::java_exceptions::expect_throws;

use super::column_batch_test_util::{
    count_term, field, frozen, ft, iota, nbr, nn, only_leaf, searcher, simple_batch, string_like,
    widen, ArrayBinaryColumn, ArrayDenseBinaryColumn, ArrayLongColumn, ContiguousDenseBinaryColumn,
    Ctx,
};
use crate::core_support::java_number::{int_value, long_value};
use crate::core_support::{assert_equals, assert_equals_msg, assert_not_null, class, test, IAE};

/// `TestColumnBatchBinaryColumn`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.document.column.TestColumnBatchBinaryColumn",
        vec![
            test("testBinaryDocValues", test_binary_doc_values),
            test("testSortedDocValues", test_sorted_doc_values),
            test("testSortedSetDocValues", test_sorted_set_doc_values),
            test("testStoredBinaryColumn", test_stored_binary_column),
            test("testInvertedColumn", test_inverted_column),
            test("testInvertedWithDocValues", test_inverted_with_doc_values),
            test("testInvertedWithStored", test_inverted_with_stored),
            test(
                "testInvertedWithStoredAndDocValues",
                test_inverted_with_stored_and_doc_values,
            ),
            test("testInvertedSparse", test_inverted_sparse),
            test("testTokenizedColumn", test_tokenized_column),
            test("testTokenizedWithStored", test_tokenized_with_stored),
            test(
                "testStoredWithDocValuesAndPoints",
                test_stored_with_doc_values_and_points,
            ),
            test(
                "testMultiValuedStoredWithDocValues",
                test_multi_valued_stored_with_doc_values,
            ),
            test(
                "testBinaryColumnNumericDVBadFixedSizeThrows",
                test_binary_column_numeric_dv_bad_fixed_size_throws,
            ),
            test(
                "testStoredTypeStringFromBinaryColumn",
                test_stored_type_string_from_binary_column,
            ),
            test(
                "testStoredTypeDataInputRejected",
                test_stored_type_data_input_rejected,
            ),
            test(
                "testBinaryColumnMultiDimPointsOnly",
                test_binary_column_multi_dim_points_only,
            ),
            test(
                "testBinaryColumnPointsOnlyArbitraryWidth",
                test_binary_column_points_only_arbitrary_width,
            ),
            test(
                "testBinaryColumnSortedDVAndPoints",
                test_binary_column_sorted_dv_and_points,
            ),
            test(
                "testMultiValuedBinaryAcrossDocs",
                test_multi_valued_binary_across_docs,
            ),
            test("testDenseBinaryPoints", test_dense_binary_points),
            test(
                "testDenseBinaryPointsWithSortedDocValues",
                test_dense_binary_points_with_sorted_doc_values,
            ),
            test(
                "testDenseBinaryPointsLargeBatch",
                test_dense_binary_points_large_batch,
            ),
            test(
                "testDenseBinaryPointsContiguousFill",
                test_dense_binary_points_contiguous_fill,
            ),
        ],
    )
}

const INT_BYTES: usize = 4;
const LONG_BYTES: usize = 8;

/// `binaryValue()` of a stored field, which Java compares with `equals`.
fn bin(f: &dyn rucene::index::IndexableField) -> Option<BytesRef> {
    f.binary_value()
}

/// Port of `testBinaryDocValues`.
fn test_binary_doc_values() -> Result<()> {
    let c = Ctx::open()?;
    let values = vec![nbr("aaa"), nbr("bbb"), nbr("ccc")];
    c.add_batch(&simple_batch(
        3,
        vec![ArrayBinaryColumn::new(
            "binary",
            ft(BinaryDocValuesField::type_()),
            vec![0, 1, 2],
            values.clone(),
        )?
        .into()],
    ))?;
    let r = c.reader()?;
    let leaf = only_leaf(&r)?;
    let mut dv = nn(leaf.get_binary_doc_values("binary")?);
    for i in 0..values.len() {
        assert_equals(&(i as i32), &dv.next_doc()?);
        assert_equals(&values[i], &dv.binary_value()?);
    }
    assert_equals(&NO_MORE_DOCS, &dv.next_doc()?);
    c.close(Some(r))
}

/// Port of `testSortedDocValues`.
fn test_sorted_doc_values() -> Result<()> {
    let c = Ctx::open()?;
    let values = vec![nbr("x"), nbr("y"), nbr("x")];
    c.add_batch(&simple_batch(
        3,
        vec![ArrayBinaryColumn::new(
            "sorted",
            ft(SortedDocValuesField::type_()),
            vec![0, 1, 2],
            values,
        )?
        .into()],
    ))?;
    let r = c.reader()?;
    let leaf = only_leaf(&r)?;
    let mut dv = nn(leaf.get_sorted_doc_values("sorted")?);
    assert_equals(&0, &dv.next_doc()?);
    assert_equals(&nbr("x"), &dv.lookup_ord(dv.ord_value()?)?);
    assert_equals(&1, &dv.next_doc()?);
    assert_equals(&nbr("y"), &dv.lookup_ord(dv.ord_value()?)?);
    assert_equals(&2, &dv.next_doc()?);
    assert_equals(&nbr("x"), &dv.lookup_ord(dv.ord_value()?)?);
    // "x" and "y" should share ord space
    assert_equals(&2, &dv.get_value_count()?);
    c.close(Some(r))
}

/// Port of `testSortedSetDocValues`.
fn test_sorted_set_doc_values() -> Result<()> {
    let c = Ctx::open()?;
    // Doc 0 has two values, doc 1 has one value
    let values = vec![nbr("a"), nbr("b"), nbr("a")];
    c.add_batch(&simple_batch(
        2,
        vec![ArrayBinaryColumn::new(
            "sortedSet",
            ft(SortedSetDocValuesField::type_()),
            vec![0, 0, 1],
            values,
        )?
        .into()],
    ))?;
    let r = c.reader()?;
    let leaf = only_leaf(&r)?;
    let mut dv = nn(leaf.get_sorted_set_doc_values("sortedSet")?);
    assert_equals(&0, &dv.next_doc()?);
    assert_equals(&2, &dv.doc_value_count()?);
    let o = dv.next_ord()?;
    assert_equals(&nbr("a"), &dv.lookup_ord(o)?);
    let o = dv.next_ord()?;
    assert_equals(&nbr("b"), &dv.lookup_ord(o)?);
    assert_equals(&1, &dv.next_doc()?);
    assert_equals(&1, &dv.doc_value_count()?);
    let o = dv.next_ord()?;
    assert_equals(&nbr("a"), &dv.lookup_ord(o)?);
    assert_equals(&NO_MORE_DOCS, &dv.next_doc()?);
    c.close(Some(r))
}

/// Port of `testStoredBinaryColumn`.
fn test_stored_binary_column() -> Result<()> {
    let c = Ctx::open()?;
    // stored + SORTED doc values
    let stored_sorted_type = frozen(|t| {
        t.set_stored(true)?;
        t.set_doc_values_type(DocValuesType::SORTED)
    })?;
    let values = vec![nbr("aaa"), nbr("bbb"), nbr("ccc")];
    c.add_batch(&simple_batch(
        3,
        vec![ArrayBinaryColumn::new(
            "val",
            ft(&stored_sorted_type),
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
        assert_equals(&Some(values[i].clone()), &bin(field(&doc, "val")));
    }
    // Verify doc values
    let mut dv = nn(leaf.get_sorted_doc_values("val")?);
    for i in 0..3 {
        assert_equals(&(i as i32), &dv.next_doc()?);
        assert_equals(&values[i], &dv.lookup_ord(dv.ord_value()?)?);
    }
    c.close(Some(r))
}

/// Port of `testInvertedColumn`.
fn test_inverted_column() -> Result<()> {
    let c = Ctx::open()?;
    // StringField-like: DOCS, omitNorms, non-tokenized
    let string_type = string_like(|_| Ok(()))?;
    let values = vec![nbr("alpha"), nbr("beta"), nbr("alpha")];
    c.add_batch(&simple_batch(
        3,
        vec![ArrayBinaryColumn::new("tag", ft(&string_type), vec![0, 1, 2], values)?.into()],
    ))?;
    let r = c.reader()?;
    let s = searcher(&r)?;
    assert_equals(&2, &count_term(&s, "tag", "alpha")?);
    assert_equals(&1, &count_term(&s, "tag", "beta")?);
    assert_equals(&0, &count_term(&s, "tag", "gamma")?);
    c.close(Some(r))
}

/// Port of `testInvertedWithDocValues`.
fn test_inverted_with_doc_values() -> Result<()> {
    let c = Ctx::open()?;
    // Inverted + SORTED doc values (like a StringField with doc values)
    let inverted_dv_type = string_like(|t| t.set_doc_values_type(DocValuesType::SORTED))?;
    let values = vec![nbr("x"), nbr("y"), nbr("x")];
    c.add_batch(&simple_batch(
        3,
        vec![ArrayBinaryColumn::new(
            "field",
            ft(&inverted_dv_type),
            vec![0, 1, 2],
            values.clone(),
        )?
        .into()],
    ))?;
    let r = c.reader()?;
    let s = searcher(&r)?;
    // Verify inverted index
    assert_equals(&2, &count_term(&s, "field", "x")?);
    assert_equals(&1, &count_term(&s, "field", "y")?);
    // Verify doc values
    let leaf = only_leaf(&r)?;
    let mut dv = nn(leaf.get_sorted_doc_values("field")?);
    for i in 0..3 {
        assert_equals(&(i as i32), &dv.next_doc()?);
        assert_equals(&values[i], &dv.lookup_ord(dv.ord_value()?)?);
    }
    c.close(Some(r))
}

/// Port of `testInvertedWithStored`.
fn test_inverted_with_stored() -> Result<()> {
    let c = Ctx::open()?;
    // Inverted + stored (like StringField with Store.YES)
    let inverted_stored_type = rucene::document::FieldType::new_from(StringField::type_stored());
    inverted_stored_type.freeze();
    let values = vec![nbr("aaa"), nbr("bbb"), nbr("ccc")];
    c.add_batch(&simple_batch(
        3,
        vec![ArrayBinaryColumn::new(
            "field",
            ft(&inverted_stored_type),
            vec![0, 1, 2],
            values.clone(),
        )?
        .into()],
    ))?;
    let r = c.reader()?;
    let s = searcher(&r)?;
    // Verify inverted index
    assert_equals(&1, &count_term(&s, "field", "aaa")?);
    assert_equals(&1, &count_term(&s, "field", "bbb")?);
    // Verify stored fields
    let leaf = only_leaf(&r)?;
    let stored_fields = leaf.stored_fields()?;
    for i in 0..3 {
        let doc = stored_fields.document(i as i32)?;
        assert_equals(&Some(values[i].clone()), &bin(field(&doc, "field")));
    }
    c.close(Some(r))
}

/// Port of `testInvertedWithStoredAndDocValues`.
fn test_inverted_with_stored_and_doc_values() -> Result<()> {
    let c = Ctx::open()?;
    // Inverted + stored + SORTED doc values
    let all_type = string_like(|t| {
        t.set_stored(true)?;
        t.set_doc_values_type(DocValuesType::SORTED)
    })?;
    let values = vec![nbr("x"), nbr("y"), nbr("z")];
    c.add_batch(&simple_batch(
        3,
        vec![ArrayBinaryColumn::new("field", ft(&all_type), vec![0, 1, 2], values.clone())?.into()],
    ))?;
    let r = c.reader()?;
    let leaf = only_leaf(&r)?;
    let s = searcher(&r)?;
    // Verify inverted index
    assert_equals(&1, &count_term(&s, "field", "x")?);
    // Verify stored fields
    let stored_fields = leaf.stored_fields()?;
    for i in 0..3 {
        let doc = stored_fields.document(i as i32)?;
        assert_equals(&Some(values[i].clone()), &bin(field(&doc, "field")));
    }
    // Verify doc values
    let mut dv = nn(leaf.get_sorted_doc_values("field")?);
    for i in 0..3 {
        assert_equals(&(i as i32), &dv.next_doc()?);
        assert_equals(&values[i], &dv.lookup_ord(dv.ord_value()?)?);
    }
    c.close(Some(r))
}

/// Port of `testInvertedSparse`.
fn test_inverted_sparse() -> Result<()> {
    let c = Ctx::open()?;
    let string_type = string_like(|_| Ok(()))?;
    // Only doc 1 out of 3 has a term
    c.add_batch(&simple_batch(
        3,
        vec![ArrayBinaryColumn::new("tag", ft(&string_type), vec![1], vec![nbr("found")])?.into()],
    ))?;
    let r = c.reader()?;
    let s = searcher(&r)?;
    assert_equals(&1, &count_term(&s, "tag", "found")?);
    c.close(Some(r))
}

/// Port of `testTokenizedColumn`.
fn test_tokenized_column() -> Result<()> {
    let c = Ctx::open_mock_analyzer()?;
    // TextField-like: tokenized, DOCS_AND_FREQS_AND_POSITIONS
    let values = vec![
        nbr("quick brown fox"),
        nbr("lazy brown dog"),
        nbr("quick fox jumps"),
    ];
    c.add_batch(&simple_batch(
        3,
        vec![ArrayBinaryColumn::new(
            "text",
            ft(TextField::type_not_stored()),
            vec![0, 1, 2],
            values,
        )?
        .into()],
    ))?;
    let r = c.reader()?;
    let s = searcher(&r)?;
    // Each word was tokenized — verify individual terms
    assert_equals(&2, &count_term(&s, "text", "quick")?);
    assert_equals(&2, &count_term(&s, "text", "brown")?);
    assert_equals(&2, &count_term(&s, "text", "fox")?);
    assert_equals(&1, &count_term(&s, "text", "lazy")?);
    assert_equals(&1, &count_term(&s, "text", "dog")?);
    assert_equals(&1, &count_term(&s, "text", "jumps")?);
    assert_equals(&0, &count_term(&s, "text", "missing")?);
    c.close(Some(r))
}

/// Port of `testTokenizedWithStored`.
fn test_tokenized_with_stored() -> Result<()> {
    let c = Ctx::open_mock_analyzer()?;
    let values = vec![nbr("hello world"), nbr("goodbye world")];
    c.add_batch(&simple_batch(
        2,
        vec![ArrayBinaryColumn::new(
            "text",
            ft(TextField::type_stored()),
            vec![0, 1],
            values.clone(),
        )?
        .into()],
    ))?;
    let r = c.reader()?;
    let leaf = only_leaf(&r)?;
    let s = searcher(&r)?;
    // Verify tokenized search
    assert_equals(&2, &count_term(&s, "text", "world")?);
    assert_equals(&1, &count_term(&s, "text", "hello")?);
    // Verify stored fields
    let stored_fields = leaf.stored_fields()?;
    assert_equals(
        &Some(values[0].clone()),
        &bin(field(&stored_fields.document(0)?, "text")),
    );
    assert_equals(
        &Some(values[1].clone()),
        &bin(field(&stored_fields.document(1)?, "text")),
    );
    c.close(Some(r))
}

/// Port of `testStoredWithDocValuesAndPoints`.
fn test_stored_with_doc_values_and_points() -> Result<()> {
    let c = Ctx::open()?;
    // stored + SORTED_NUMERIC DV + 4-byte points
    let all_type = frozen(|t| {
        t.set_stored(true)?;
        t.set_doc_values_type(DocValuesType::SORTED_NUMERIC)?;
        t.set_dimensions(1, INT_BYTES as i32)
    })?;
    let raw = [10, 20, 30];
    c.add_batch(&simple_batch(
        3,
        vec![ArrayLongColumn::with_kind(
            "field",
            ft(&all_type),
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
        assert_equals(&raw[i], &int_value(field(&doc, "field").numeric_value()));
    }
    // Verify doc values (raw int widened to long).
    let mut dv = nn(leaf.get_sorted_numeric_doc_values("field")?);
    for i in 0..raw.len() {
        assert_equals(&(i as i32), &dv.next_doc()?);
        assert_equals(&(raw[i] as i64), &dv.next_value()?);
    }
    // Verify points
    assert_equals(&3i64, &nn(leaf.get_point_values("field")?).size()?);
    c.close(Some(r))
}

/// Port of `testMultiValuedStoredWithDocValues`.
fn test_multi_valued_stored_with_doc_values() -> Result<()> {
    let c = Ctx::open()?;
    // stored + SORTED_NUMERIC doc values (multi-valued)
    let t = frozen(|t| {
        t.set_stored(true)?;
        t.set_doc_values_type(DocValuesType::SORTED_NUMERIC)
    })?;
    // Doc 0 has two values (10, 20), doc 1 has one value (30)
    c.add_batch(&simple_batch(
        2,
        vec![ArrayLongColumn::new("val", ft(&t), vec![0, 0, 1], vec![10, 20, 30])?.into()],
    ))?;
    let r = c.reader()?;
    let leaf = only_leaf(&r)?;
    // Verify stored fields — each value occurrence is stored separately
    let stored_fields = leaf.stored_fields()?;
    let doc0 = stored_fields.document(0)?;
    assert_equals(&2, &doc0.get_fields_by_name("val").len());
    assert_equals(
        &10i64,
        &long_value(doc0.get_fields_by_name("val")[0].numeric_value()),
    );
    assert_equals(
        &20i64,
        &long_value(doc0.get_fields_by_name("val")[1].numeric_value()),
    );
    let doc1 = stored_fields.document(1)?;
    assert_equals(&1, &doc1.get_fields_by_name("val").len());
    assert_equals(
        &30i64,
        &long_value(doc1.get_fields_by_name("val")[0].numeric_value()),
    );
    // Verify doc values
    let mut dv = nn(leaf.get_sorted_numeric_doc_values("val")?);
    assert_equals(&0, &dv.next_doc()?);
    assert_equals(&2, &dv.doc_value_count()?);
    assert_equals(&10i64, &dv.next_value()?);
    assert_equals(&20i64, &dv.next_value()?);
    assert_equals(&1, &dv.next_doc()?);
    assert_equals(&1, &dv.doc_value_count()?);
    assert_equals(&30i64, &dv.next_value()?);
    c.close(Some(r))
}

/// Port of `testBinaryColumnNumericDVBadFixedSizeThrows`.
fn test_binary_column_numeric_dv_bad_fixed_size_throws() -> Result<()> {
    let c = Ctx::open()?;
    // Variable-size binary into NUMERIC DV should fail validation (fixedSize=-1).
    expect_throws(IAE, || {
        c.add_batch(&simple_batch(
            1,
            vec![ArrayBinaryColumn::new(
                "val",
                ft(NumericDocValuesField::type_()),
                vec![0],
                vec![nbr("x")],
            )?
            .into()],
        ))
    });
    c.close(None)
}

/// Port of `testStoredTypeStringFromBinaryColumn`.
fn test_stored_type_string_from_binary_column() -> Result<()> {
    let c = Ctx::open()?;
    let t = frozen(|t| t.set_stored(true))?;
    let raw = ["hello", "wörld", "🦜"];
    let values: Vec<BytesRef> = raw.iter().map(|s| nbr(s)).collect();
    c.add_batch(&simple_batch(
        3,
        vec![ArrayBinaryColumn::with_stored_type(
            "val",
            ft(&t),
            vec![0, 1, 2],
            values,
            StoredValueType::STRING,
        )?
        .into()],
    ))?;
    let r = c.reader()?;
    let leaf = only_leaf(&r)?;
    let stored_fields = leaf.stored_fields()?;
    for i in 0..raw.len() {
        let doc = stored_fields.document(i as i32)?;
        assert_equals(
            &Some(raw[i].to_string()),
            &field(&doc, "val").string_value(),
        );
    }
    c.close(Some(r))
}

/// Port of `testStoredTypeDataInputRejected`.
fn test_stored_type_data_input_rejected() -> Result<()> {
    let c = Ctx::open()?;
    let t = frozen(|t| t.set_stored(true))?;
    expect_throws(IAE, || {
        c.add_batch(&simple_batch(
            1,
            vec![ArrayBinaryColumn::with_stored_type(
                "val",
                ft(&t),
                vec![0],
                vec![nbr("x")],
                StoredValueType::DATA_INPUT,
            )?
            .into()],
        ))
    });
    c.close(None)
}

/// Port of `testBinaryColumnMultiDimPointsOnly`.
fn test_binary_column_multi_dim_points_only() -> Result<()> {
    // Plain BinaryColumn with 2-D int points (fixedSize = 2 * 4 = 8). Caller pre-packs bytes via
    // IntPoint.pack; the chain writes them to points unchanged.
    let c = Ctx::open()?;
    let point_type = frozen(|t| t.set_dimensions(2, INT_BYTES as i32))?;
    let values = vec![
        IntPoint::pack(&[1, 10])?,
        IntPoint::pack(&[2, 20])?,
        IntPoint::pack(&[3, 30])?,
    ];
    c.add_batch(&simple_batch(
        3,
        vec![ArrayBinaryColumn::new("pt", ft(&point_type), vec![0, 1, 2], values)?.into()],
    ))?;
    let r = c.reader()?;
    let s = searcher(&r)?;
    assert_equals(
        &1,
        &s.count(IntPoint::new_range_query("pt", &[1, 10], &[1, 10])?)?,
    );
    assert_equals(
        &3,
        &s.count(IntPoint::new_range_query("pt", &[0, 0], &[10, 100])?)?,
    );
    c.close(Some(r))
}

/// Port of `testBinaryColumnPointsOnlyArbitraryWidth`.
fn test_binary_column_points_only_arbitrary_width() -> Result<()> {
    // 3-D int points (12 bytes) via plain BinaryColumn — arbitrary widths are fine for the
    // opaque-bytes path since no numeric transform is applied.
    let c = Ctx::open()?;
    let t = frozen(|t| t.set_dimensions(3, INT_BYTES as i32))?;
    let raw = [[1, 2, 3], [4, 5, 6], [10, 20, 30]];
    let mut values = Vec::with_capacity(raw.len());
    for p in &raw {
        values.push(IntPoint::pack(p)?);
    }
    c.add_batch(&simple_batch(
        raw.len() as i32,
        vec![ArrayBinaryColumn::new("pt", ft(&t), iota(raw.len()), values)?.into()],
    ))?;
    let r = c.reader()?;
    let s = searcher(&r)?;
    assert_equals(
        &1,
        &s.count(IntPoint::new_range_query("pt", &[1, 2, 3], &[1, 2, 3])?)?,
    );
    assert_equals(
        &3,
        &s.count(IntPoint::new_range_query(
            "pt",
            &[0, 0, 0],
            &[100, 100, 100],
        )?)?,
    );
    c.close(Some(r))
}

/// Port of `testBinaryColumnSortedDVAndPoints`.
fn test_binary_column_sorted_dv_and_points() -> Result<()> {
    // Plain BinaryColumn with SORTED DV + 1-D int point. Same BytesRef goes to both writers.
    let c = Ctx::open()?;
    let t = frozen(|t| {
        t.set_dimensions(1, INT_BYTES as i32)?;
        t.set_doc_values_type(DocValuesType::SORTED)
    })?;
    let values = vec![
        IntPoint::pack(&[10])?,
        IntPoint::pack(&[20])?,
        IntPoint::pack(&[30])?,
    ];
    c.add_batch(&simple_batch(
        3,
        vec![ArrayBinaryColumn::new("field", ft(&t), vec![0, 1, 2], values.clone())?.into()],
    ))?;
    let r = c.reader()?;
    let leaf = only_leaf(&r)?;
    let mut dv = nn(leaf.get_sorted_doc_values("field")?);
    for i in 0..3 {
        assert_equals(&(i as i32), &dv.next_doc()?);
        assert_equals(&values[i], &dv.lookup_ord(dv.ord_value()?)?);
    }
    let s = searcher(&r)?;
    assert_equals(&1, &s.count(IntPoint::new_exact_query("field", 10)?)?);
    assert_equals(
        &3,
        &s.count(IntPoint::new_range_query("field", &[10], &[30])?)?,
    );
    c.close(Some(r))
}

/// Port of `testMultiValuedBinaryAcrossDocs`: a single SORTED_SET column whose
/// tuple cursor emits multiple values per doc. Doc-ids repeat in
/// non-decreasing order; values within a doc are deduplicated and ord-sorted
/// by the writer.
fn test_multi_valued_binary_across_docs() -> Result<()> {
    let c = Ctx::open()?;
    // doc 0: {a, b}; doc 1: {c}; doc 2: {b, d, e}; doc 3: {a, e}
    let doc_ids = vec![0, 0, 1, 2, 2, 2, 3, 3];
    let values = vec![
        nbr("a"),
        nbr("b"),
        nbr("c"),
        nbr("b"),
        nbr("d"),
        nbr("e"),
        nbr("a"),
        nbr("e"),
    ];
    c.add_batch(&simple_batch(
        4,
        vec![
            ArrayBinaryColumn::new("set", ft(SortedSetDocValuesField::type_()), doc_ids, values)?
                .into(),
        ],
    ))?;
    let r = c.reader()?;
    let leaf = only_leaf(&r)?;
    let mut dv = nn(leaf.get_sorted_set_doc_values("set")?);
    let expected_per_doc: [&[&str]; 4] = [&["a", "b"], &["c"], &["b", "d", "e"], &["a", "e"]];
    for d in 0..expected_per_doc.len() {
        assert_equals(&(d as i32), &dv.next_doc()?);
        assert_equals(&(expected_per_doc[d].len() as i32), &dv.doc_value_count()?);
        for i in 0..expected_per_doc[d].len() {
            let ord = dv.next_ord()?;
            assert_equals(&nbr(expected_per_doc[d][i]), &dv.lookup_ord(ord)?);
        }
    }
    assert_equals(&NO_MORE_DOCS, &dv.next_doc()?);
    c.close(Some(r))
}

fn int_sortable(v: i32) -> Vec<u8> {
    let mut b = vec![0u8; INT_BYTES];
    NumericUtils::int_to_sortable_bytes(v, &mut b, 0);
    b
}

fn long_sortable(v: i64) -> Vec<u8> {
    let mut b = vec![0u8; LONG_BYTES];
    NumericUtils::long_to_sortable_bytes(v, &mut b, 0);
    b
}

/// `BinaryPoint.newRangeQuery(field, lower, upper)` (one dimension).
fn binary_range(s: &IndexSearcher, f: &str, lower: Vec<u8>, upper: Vec<u8>) -> Result<i32> {
    s.count(BinaryPoint::new_range_query(
        f,
        &[BytesRef::new(lower)],
        &[BytesRef::new(upper)],
    )?)
}

/// Port of `testDenseBinaryPoints`.
fn test_dense_binary_points() -> Result<()> {
    let c = Ctx::open()?;
    // 1-D, 4-byte binary points (sort-encoded ints).
    let t = frozen(|t| t.set_dimensions(1, INT_BYTES as i32))?;
    let raw_values = [-100, 0, 1, 42, 999];
    let encoded: Vec<BytesRef> = raw_values
        .iter()
        .map(|&v| BytesRef::new(int_sortable(v)))
        .collect();
    c.add_batch(&simple_batch(
        raw_values.len() as i32,
        vec![ArrayDenseBinaryColumn::new("p", ft(&t), encoded)?.into()],
    ))?;
    let r = c.reader()?;
    let s = searcher(&r)?;
    // All docs match a full-range query.
    assert_equals(
        &(raw_values.len() as i32),
        &binary_range(&s, "p", int_sortable(i32::MIN), int_sortable(i32::MAX))?,
    );
    // Exact queries for first, last, and a middle value.
    for raw in [
        raw_values[0],
        raw_values[raw_values.len() - 1],
        raw_values[2],
    ] {
        let exact = int_sortable(raw);
        assert_equals(&1, &s.count(BinaryPoint::new_exact_query("p", &exact)?)?);
    }
    // A value that was not indexed should match nothing.
    let absent = int_sortable(-1);
    assert_equals(&0, &s.count(BinaryPoint::new_exact_query("p", &absent)?)?);
    c.close(Some(r))
}

/// Port of `testDenseBinaryPointsWithSortedDocValues`.
fn test_dense_binary_points_with_sorted_doc_values() -> Result<()> {
    let c = Ctx::open()?;
    // SORTED DV + 1-D 4-byte points. Same BytesRef serves both.
    let t = frozen(|t| {
        t.set_doc_values_type(DocValuesType::SORTED)?;
        t.set_dimensions(1, INT_BYTES as i32)
    })?;
    // 4 docs; sort-encoded ints are used for both DV and points.
    let n = 4usize;
    let encoded: Vec<BytesRef> = (0..n as i32)
        .map(|i| BytesRef::new(int_sortable(i * 10)))
        .collect();
    c.add_batch(&simple_batch(
        n as i32,
        vec![ArrayDenseBinaryColumn::new("sdv", ft(&t), encoded.clone())?.into()],
    ))?;
    let r = c.reader()?;
    let leaf = only_leaf(&r)?;
    let s = IndexSearcher::new(Arc::clone(&leaf).as_index_reader())?;
    // Verify sorted DV: all 4 docs should have DV (4 distinct values).
    let dv = leaf.get_sorted_doc_values("sdv")?;
    assert_not_null(&dv);
    let mut dv = nn(dv);
    assert_equals(&(n as i32), &dv.get_value_count()?);
    for i in 0..n {
        assert_equals(&(i as i32), &dv.next_doc()?);
        assert_equals(&encoded[i], &dv.lookup_ord(dv.ord_value()?)?);
    }
    assert_equals(&NO_MORE_DOCS, &dv.next_doc()?);
    // Verify points: exact query for each encoded value returns exactly 1 doc.
    for e in &encoded {
        assert_equals(
            &1,
            &s.count(BinaryPoint::new_exact_query("sdv", &e.bytes.clone())?)?,
        );
    }
    // Range query spanning all values.
    assert_equals(
        &(n as i32),
        &binary_range(&s, "sdv", int_sortable(i32::MIN), int_sortable(i32::MAX))?,
    );
    c.close(Some(r))
}

/// Port of `testDenseBinaryPointsLargeBatch`.
fn test_dense_binary_points_large_batch() -> Result<()> {
    let c = Ctx::open()?;
    // 1-D, 8-bytes-per-dim binary points: packedLen = 8, chunk = 4096/8 = 512.
    let bytes_per_dim = LONG_BYTES;
    let t = frozen(|t| t.set_dimensions(1, bytes_per_dim as i32))?;
    // n = 1100 > 2 * 512 → loop runs at least 3 times.
    let n = 1100usize;
    let encoded: Vec<BytesRef> = (0..n as i64)
        .map(|i| BytesRef::new(long_sortable(i * 1_000_000 - 100)))
        .collect();
    c.add_batch(&simple_batch(
        n as i32,
        vec![ArrayDenseBinaryColumn::new("p1d8", ft(&t), encoded.clone())?.into()],
    ))?;
    let r = c.reader()?;
    let s = searcher(&r)?;
    // All docs indexed.
    assert_equals(
        &(n as i32),
        &binary_range(&s, "p1d8", long_sortable(i64::MIN), long_sortable(i64::MAX))?,
    );
    // Values at and around the chunk boundary (index 511 / 512) round-trip correctly.
    assert_equals(
        &1,
        &s.count(BinaryPoint::new_exact_query(
            "p1d8",
            &encoded[511].bytes.clone(),
        )?)?,
    );
    assert_equals(
        &1,
        &s.count(BinaryPoint::new_exact_query(
            "p1d8",
            &encoded[512].bytes.clone(),
        )?)?,
    );
    // A value that was not indexed.
    let mut absent = vec![0u8; bytes_per_dim];
    absent[0] = 0xFF; // sorts above everything we indexed
    assert_equals(
        &0,
        &s.count(BinaryPoint::new_exact_query("p1d8", &absent)?)?,
    );
    c.close(Some(r))
}

/// Port of `testDenseBinaryPointsContiguousFill`.
fn test_dense_binary_points_contiguous_fill() -> Result<()> {
    let bytes_per_dim = LONG_BYTES; // packedLen = 8, perChunk = 512
    let n = 600usize; // spans two chunk iterations (512 + 88)
    let t = frozen(|t| t.set_dimensions(1, bytes_per_dim as i32))?;

    // Build both representations from the same encoded values.
    let mut per_value_array = Vec::with_capacity(n);
    let mut packed_flat = vec![0u8; n * bytes_per_dim];
    for i in 0..n {
        let b = long_sortable(i as i64 * 500);
        packed_flat[i * bytes_per_dim..(i + 1) * bytes_per_dim].copy_from_slice(&b);
        per_value_array.push(BytesRef::new(b));
    }

    // Index via the default per-value path (ArrayDenseBinaryColumn).
    let c_default = Ctx::open()?;
    c_default.add_batch(&simple_batch(
        n as i32,
        vec![ArrayDenseBinaryColumn::new("p", ft(&t), per_value_array.clone())?.into()],
    ))?;
    let r_default: Arc<dyn IndexReader> = c_default.reader()?;
    let searcher_default = searcher(&r_default)?;

    // Index via the contiguous bulk-copy override path (ContiguousDenseBinaryColumn).
    let c_bulk = Ctx::open()?;
    c_bulk.add_batch(&simple_batch(
        n as i32,
        vec![ContiguousDenseBinaryColumn::new("p", ft(&t), packed_flat, bytes_per_dim)?.into()],
    ))?;
    let r_bulk = c_bulk.reader()?;
    let searcher_bulk = searcher(&r_bulk)?;

    // Both indexes must return the same results for a full range query.
    let min_b = long_sortable(i64::MIN);
    let max_b = long_sortable(i64::MAX);
    assert_equals(
        &(n as i32),
        &binary_range(&searcher_default, "p", min_b.clone(), max_b.clone())?,
    );
    assert_equals(
        &(n as i32),
        &binary_range(&searcher_bulk, "p", min_b, max_b)?,
    );

    // Exact queries for a sample of values, including the chunk boundary (index 511/512).
    for idx in [0, 1, 255, 511, 512, 513, n - 1] {
        let exact = per_value_array[idx].bytes.clone();
        assert_equals_msg(
            Some(&format!("default path mismatch at idx={idx}")),
            &1,
            &searcher_default.count(BinaryPoint::new_exact_query("p", &exact)?)?,
        );
        assert_equals_msg(
            Some(&format!("bulk override path mismatch at idx={idx}")),
            &1,
            &searcher_bulk.count(BinaryPoint::new_exact_query("p", &exact)?)?,
        );
    }
    c_default.close(Some(r_default))?;
    c_bulk.close(Some(r_bulk))
}
