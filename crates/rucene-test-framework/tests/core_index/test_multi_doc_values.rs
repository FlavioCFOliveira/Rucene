//! Port of `lucene/core/src/test/org/apache/lucene/index/TestMultiDocValues.java`.
//!
//! Tests MultiDocValues versus ordinary segment merging
//!
//! * The field Java keeps a reference to and changes before each
//!   `addDocument` is changed through the document (`field_mut_as`).
//! * The indexing preamble every test repeats (`newIndexWriterConfig(random(),
//!   null)`, `newLogMergePolicy()`, the commits, `getReader`, `forceMerge(1)`,
//!   `getReader`, `getOnlyLeafReader`) is [`index_and_merge`].

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use rucene::document::{
    BinaryDocValuesField, Document, NumericDocValuesField, SortedDocValuesField,
    SortedNumericDocValuesField, SortedSetDocValuesField,
};
use rucene::error::Result;
use rucene::index::doc_values::DocValuesIterator;
use rucene::index::multi_doc_values::MultiDocValues;
use rucene::index::{IndexReader, LeafReader};
use rucene::search::{DocIdSetIterator, NO_MORE_DOCS};
use rucene::util::BytesRef;
use rucene_test_framework::index::RandomIndexWriter;
use rucene_test_framework::util::lucene_test_case::TEST_NIGHTLY;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::{assert_equals, assert_null, class, random, test};
use crate::support::{field_mut_as, npe};

/// `TestMultiDocValues`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestMultiDocValues",
        vec![
            test("testNumerics", test_numerics),
            test("testBinary", test_binary),
            test("testSorted", test_sorted),
            test("testSortedWithLotsOfDups", test_sorted_with_lots_of_dups),
            test("testSortedSet", test_sorted_set),
            test("testSortedSetWithDups", test_sorted_set_with_dups),
            test("testSortedNumeric", test_sorted_numeric),
        ],
    )
}

/// The readers of one test: `ir` before the merge, `ir2` after it, and the
/// only leaf of `ir2`.
struct Indexed {
    num_docs: i32,
    dir: rucene_test_framework::util::lucene_test_case::TestDirectory,
    ir: Arc<dyn IndexReader>,
    ir2: Arc<dyn IndexReader>,
    merged: Arc<dyn LeafReader>,
}

impl Indexed {
    /// `ir.close(); ir2.close(); dir.close();`
    fn close(self) -> Result<()> {
        self.ir.close()?;
        self.ir2.close()?;
        drop(self.merged);
        self.dir.close()
    }
}

/// `numDocs = TEST_NIGHTLY ? atLeast(500) : atLeast(50)`.
fn num_docs() -> i32 {
    if *TEST_NIGHTLY {
        LuceneTestCase::at_least(500)
    } else {
        LuceneTestCase::at_least(50)
    }
}

/// The preamble and epilogue of indexing every test shares: `numDocs` is
/// drawn after the writer is created, as in Java; `add(iw)` adds one
/// document; after each, a commit with probability 1/17.
fn index_and_merge(mut add: impl FnMut(&RandomIndexWriter) -> Result<()>) -> Result<Indexed> {
    let dir = LuceneTestCase::new_directory();
    let iwc = LuceneTestCase::new_index_writer_config_with(&*random(), None);
    iwc.set_merge_policy(Some(Arc::new(LuceneTestCase::new_log_merge_policy())))?;
    let iw = RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), iwc)?;
    let num_docs = num_docs();
    for _ in 0..num_docs {
        add(&iw)?;
        if random().next_int_bound(17) == 0 {
            iw.commit()?;
        }
    }
    let ir: Arc<dyn IndexReader> = iw.get_reader()?;
    iw.force_merge(1)?;
    let ir2: Arc<dyn IndexReader> = iw.get_reader()?;
    let merged = LuceneTestCase::get_only_leaf_reader(Arc::clone(&ir2))?;
    iw.close()?;
    Ok(Indexed {
        num_docs,
        dir,
        ir,
        ir2,
        merged,
    })
}

/// Port of `TestMultiDocValues.testNumerics`.
fn test_numerics() -> Result<()> {
    let mut doc = Document::new();
    doc.add(Box::new(NumericDocValuesField::new("numbers", 0)));
    let ix = index_and_merge(|iw| {
        field_mut_as::<NumericDocValuesField>(&mut doc, "numbers").set_value(random().next_long());
        iw.add_document(&doc)?;
        Ok(())
    })?;
    let num_docs = ix.num_docs;
    let mut multi =
        MultiDocValues::get_numeric_values(&ix.ir, "numbers")?.unwrap_or_else(|| npe("multi"));
    let mut single = ix
        .merged
        .get_numeric_doc_values("numbers")?
        .unwrap_or_else(|| npe("single"));
    for i in 0..num_docs {
        assert_equals(&i, &multi.next_doc()?);
        assert_equals(&i, &single.next_doc()?);
        assert_equals(&single.long_value()?, &multi.long_value()?);
    }
    test_random_advance(
        &mut *ix
            .merged
            .get_numeric_doc_values("numbers")?
            .unwrap_or_else(|| npe("iter1")),
        &mut *MultiDocValues::get_numeric_values(&ix.ir, "numbers")?
            .unwrap_or_else(|| npe("iter2")),
    )?;
    test_random_advance_exact(
        &mut *ix
            .merged
            .get_numeric_doc_values("numbers")?
            .unwrap_or_else(|| npe("iter1")),
        &mut *MultiDocValues::get_numeric_values(&ix.ir, "numbers")?
            .unwrap_or_else(|| npe("iter2")),
        ix.merged.max_doc(),
    )?;
    ix.close()
}

/// Port of `TestMultiDocValues.testBinary`.
fn test_binary() -> Result<()> {
    let mut doc = Document::new();
    doc.add(Box::new(BinaryDocValuesField::new(
        "bytes",
        BytesRef::default(),
    )));
    let ix = index_and_merge(|iw| {
        let r#ref = BytesRef::from_char_sequence(&TestUtil::random_unicode_string(&*random()));
        field_mut_as::<BinaryDocValuesField>(&mut doc, "bytes").set_bytes_value(r#ref)?;
        iw.add_document(&doc)?;
        Ok(())
    })?;
    let num_docs = ix.num_docs;
    let mut multi =
        MultiDocValues::get_binary_values(&ix.ir, "bytes")?.unwrap_or_else(|| npe("multi"));
    let mut single = ix
        .merged
        .get_binary_doc_values("bytes")?
        .unwrap_or_else(|| npe("single"));
    for i in 0..num_docs {
        assert_equals(&i, &multi.next_doc()?);
        assert_equals(&i, &single.next_doc()?);
        let expected = BytesRef::deep_copy_of(&single.binary_value()?);
        let actual = multi.binary_value()?;
        assert_equals(&expected, &actual);
    }
    test_random_advance(
        &mut *ix
            .merged
            .get_binary_doc_values("bytes")?
            .unwrap_or_else(|| npe("iter1")),
        &mut *MultiDocValues::get_binary_values(&ix.ir, "bytes")?.unwrap_or_else(|| npe("iter2")),
    )?;
    test_random_advance_exact(
        &mut *ix
            .merged
            .get_binary_doc_values("bytes")?
            .unwrap_or_else(|| npe("iter1")),
        &mut *MultiDocValues::get_binary_values(&ix.ir, "bytes")?.unwrap_or_else(|| npe("iter2")),
        ix.merged.max_doc(),
    )?;
    ix.close()
}

/// The checks `testSorted` and `testSortedWithLotsOfDups` share after their
/// own value loop: the random advances.
fn sorted_advances(ix: &Indexed) -> Result<()> {
    test_random_advance(
        &mut *ix
            .merged
            .get_sorted_doc_values("bytes")?
            .unwrap_or_else(|| npe("iter1")),
        &mut *MultiDocValues::get_sorted_values(&ix.ir, "bytes")?.unwrap_or_else(|| npe("iter2")),
    )?;
    test_random_advance_exact(
        &mut *ix
            .merged
            .get_sorted_doc_values("bytes")?
            .unwrap_or_else(|| npe("iter1")),
        &mut *MultiDocValues::get_sorted_values(&ix.ir, "bytes")?.unwrap_or_else(|| npe("iter2")),
        ix.merged.max_doc(),
    )
}

/// Port of `TestMultiDocValues.testSorted`.
fn test_sorted() -> Result<()> {
    let mut doc = Document::new();
    doc.add(Box::new(SortedDocValuesField::new(
        "bytes",
        BytesRef::default(),
    )));
    let ix = index_and_merge(|iw| {
        let r#ref = BytesRef::from_char_sequence(&TestUtil::random_unicode_string(&*random()));
        field_mut_as::<SortedDocValuesField>(&mut doc, "bytes").set_bytes_value(r#ref)?;
        if random().next_int_bound(7) == 0 {
            iw.add_document(&Document::new())?;
        }
        iw.add_document(&doc)?;
        Ok(())
    })?;
    let mut multi =
        MultiDocValues::get_sorted_values(&ix.ir, "bytes")?.unwrap_or_else(|| npe("multi"));
    let mut single = ix
        .merged
        .get_sorted_doc_values("bytes")?
        .unwrap_or_else(|| npe("single"));
    assert_equals(&single.get_value_count()?, &multi.get_value_count()?);
    loop {
        assert_equals(&single.next_doc()?, &multi.next_doc()?);
        if single.doc_id() == NO_MORE_DOCS {
            break;
        }
        let expected = BytesRef::deep_copy_of(&single.lookup_ord(single.ord_value()?)?);
        let actual = multi.lookup_ord(multi.ord_value()?)?;
        assert_equals(&expected, &actual);
        assert_equals(&single.ord_value()?, &multi.ord_value()?);
    }
    sorted_advances(&ix)?;
    ix.close()
}

/// Port of `TestMultiDocValues.testSortedWithLotsOfDups`.
fn test_sorted_with_lots_of_dups() -> Result<()> {
    let mut doc = Document::new();
    doc.add(Box::new(SortedDocValuesField::new(
        "bytes",
        BytesRef::default(),
    )));
    let ix = index_and_merge(|iw| {
        let r#ref =
            BytesRef::from_char_sequence(&TestUtil::random_simple_string_max(&*random(), 2));
        field_mut_as::<SortedDocValuesField>(&mut doc, "bytes").set_bytes_value(r#ref)?;
        iw.add_document(&doc)?;
        Ok(())
    })?;
    let num_docs = ix.num_docs;
    let mut multi =
        MultiDocValues::get_sorted_values(&ix.ir, "bytes")?.unwrap_or_else(|| npe("multi"));
    let mut single = ix
        .merged
        .get_sorted_doc_values("bytes")?
        .unwrap_or_else(|| npe("single"));
    assert_equals(&single.get_value_count()?, &multi.get_value_count()?);
    for i in 0..num_docs {
        assert_equals(&i, &multi.next_doc()?);
        assert_equals(&i, &single.next_doc()?);
        assert_equals(&single.ord_value()?, &multi.ord_value()?);
        let expected = BytesRef::deep_copy_of(&single.lookup_ord(single.ord_value()?)?);
        let actual = multi.lookup_ord(multi.ord_value()?)?;
        assert_equals(&expected, &actual);
    }
    sorted_advances(&ix)?;
    ix.close()
}

/// The body `testSortedSet` and `testSortedSetWithDups` share; `value()`
/// draws one value.
fn sorted_set_case(value: impl Fn() -> String) -> Result<()> {
    let ix = index_and_merge(|iw| {
        let mut doc = Document::new();
        let num_values = random().next_int_bound(5);
        for _ in 0..num_values {
            doc.add(Box::new(SortedSetDocValuesField::new(
                "bytes",
                BytesRef::from_char_sequence(&value()),
            )));
        }
        iw.add_document(&doc)?;
        Ok(())
    })?;
    let multi = MultiDocValues::get_sorted_set_values(&ix.ir, "bytes")?;
    let single = ix.merged.get_sorted_set_doc_values("bytes")?;
    match multi {
        None => assert_null(&single.as_ref().map(|_| ())),
        Some(mut multi) => {
            let mut single = single.unwrap_or_else(|| npe("single"));
            assert_equals(&single.get_value_count()?, &multi.get_value_count()?);
            for i in 0..single.get_value_count()? {
                let expected = BytesRef::deep_copy_of(&single.lookup_ord(i)?);
                let actual = multi.lookup_ord(i)?;
                assert_equals(&expected, &actual);
            }
            loop {
                let doc_id = single.next_doc()?;
                assert_equals(&doc_id, &multi.next_doc()?);
                if doc_id == NO_MORE_DOCS {
                    break;
                }
                assert_equals(&single.doc_value_count()?, &multi.doc_value_count()?);
                for _ in 0..single.doc_value_count()? {
                    assert_equals(&single.next_ord()?, &multi.next_ord()?);
                }
            }
        }
    }
    test_random_advance(
        &mut *ix
            .merged
            .get_sorted_set_doc_values("bytes")?
            .unwrap_or_else(|| npe("iter1")),
        &mut *MultiDocValues::get_sorted_set_values(&ix.ir, "bytes")?
            .unwrap_or_else(|| npe("iter2")),
    )?;
    test_random_advance_exact(
        &mut *ix
            .merged
            .get_sorted_set_doc_values("bytes")?
            .unwrap_or_else(|| npe("iter1")),
        &mut *MultiDocValues::get_sorted_set_values(&ix.ir, "bytes")?
            .unwrap_or_else(|| npe("iter2")),
        ix.merged.max_doc(),
    )?;
    ix.close()
}

/// Port of `TestMultiDocValues.testSortedSet`.
fn test_sorted_set() -> Result<()> {
    sorted_set_case(|| TestUtil::random_unicode_string(&*random()))
}

/// Port of `TestMultiDocValues.testSortedSetWithDups`.
fn test_sorted_set_with_dups() -> Result<()> {
    sorted_set_case(|| TestUtil::random_simple_string_max(&*random(), 2))
}

/// Port of `TestMultiDocValues.testSortedNumeric`.
fn test_sorted_numeric() -> Result<()> {
    let ix = index_and_merge(|iw| {
        let mut doc = Document::new();
        let num_values = random().next_int_bound(5);
        for _ in 0..num_values {
            doc.add(Box::new(SortedNumericDocValuesField::new(
                "nums",
                TestUtil::next_long(&*random(), i64::MIN, i64::MAX),
            )));
        }
        iw.add_document(&doc)?;
        Ok(())
    })?;
    let num_docs = ix.num_docs;
    let multi = MultiDocValues::get_sorted_numeric_values(&ix.ir, "nums")?;
    let single = ix.merged.get_sorted_numeric_doc_values("nums")?;
    match multi {
        None => assert_null(&single.as_ref().map(|_| ())),
        Some(mut multi) => {
            let mut single = single.unwrap_or_else(|| npe("single"));
            for i in 0..num_docs {
                if i > single.doc_id() {
                    assert_equals(&single.next_doc()?, &multi.next_doc()?);
                }
                if i == single.doc_id() {
                    assert_equals(&single.doc_value_count()?, &multi.doc_value_count()?);
                    for _ in 0..single.doc_value_count()? {
                        assert_equals(&single.next_value()?, &multi.next_value()?);
                    }
                }
            }
        }
    }
    test_random_advance(
        &mut *ix
            .merged
            .get_sorted_numeric_doc_values("nums")?
            .unwrap_or_else(|| npe("iter1")),
        &mut *MultiDocValues::get_sorted_numeric_values(&ix.ir, "nums")?
            .unwrap_or_else(|| npe("iter2")),
    )?;
    test_random_advance_exact(
        &mut *ix
            .merged
            .get_sorted_numeric_doc_values("nums")?
            .unwrap_or_else(|| npe("iter1")),
        &mut *MultiDocValues::get_sorted_numeric_values(&ix.ir, "nums")?
            .unwrap_or_else(|| npe("iter2")),
        ix.merged.max_doc(),
    )?;
    ix.close()
}

/// Port of `TestMultiDocValues.testRandomAdvance` (`private`).
fn test_random_advance<A, B>(iter1: &mut A, iter2: &mut B) -> Result<()>
where
    A: DocIdSetIterator + ?Sized,
    B: DocIdSetIterator + ?Sized,
{
    assert_equals(&-1, &iter1.doc_id());
    assert_equals(&-1, &iter2.doc_id());

    while iter1.doc_id() != NO_MORE_DOCS {
        if random().next_boolean() {
            assert_equals(&iter1.next_doc()?, &iter2.next_doc()?);
        } else {
            let target = iter1.doc_id() + TestUtil::next_int(&*random(), 1, 100);
            assert_equals(&iter1.advance(target)?, &iter2.advance(target)?);
        }
    }
    Ok(())
}

/// Port of `TestMultiDocValues.testRandomAdvanceExact` (`private`).
fn test_random_advance_exact<A, B>(iter1: &mut A, iter2: &mut B, max_doc: i32) -> Result<()>
where
    A: DocValuesIterator + ?Sized,
    B: DocValuesIterator + ?Sized,
{
    let mut target = random().next_int_bound(max_doc.min(10));
    while target < max_doc {
        let exists1 = iter1.advance_exact(target)?;
        let exists2 = iter2.advance_exact(target)?;
        assert_equals(&exists1, &exists2);
        target += random().next_int_bound(10);
    }
    Ok(())
}
