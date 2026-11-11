//! Port of `lucene/core/src/test/org/apache/lucene/search/TestBinarySortField.java`.
//!
//! Tests for `BinarySortField` used as an index sort (and as a search sort).

use std::cmp::Ordering;
use std::sync::{Arc, Once};

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::{
    BinaryDocValuesField, Document, NumericDocValuesField, NumericValue, SortedDocValuesField,
    Store, StoredField, StringField,
};
use rucene::error::{LuceneError, Result};
use rucene::index::directory_reader::{self, DirectoryReader};
use rucene::index::filter_directory_reader::SlowCodecReaderWrapper;
use rucene::index::index_sorter::{
    reload_sort_field_providers, sort_field_provider_for_name, write_sort_field, BinarySorter,
    IndexSorter, SortFieldProvider,
};
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::merge_policy::NoMergePolicy;
use rucene::index::{
    BinaryDocValues, CodecReader, DocValues, DocValuesIterator, IndexReader, LeafReader, Term,
};
use rucene::search::binary_sort_field::SortKeySource;
use rucene::search::field_comparator::SortValue;
use rucene::search::sort::{MissingValue, Sort, SortField, SortFieldType};
use rucene::search::{BinarySortField, DocIdSetIterator, MatchAllDocsQuery, NO_MORE_DOCS};
use rucene::store::{ByteArrayDataInput, ByteBuffersDataOutput, DataInput, DataOutput};
use rucene::util::resource_loader::{ClassLoader, ServiceProvider};
use rucene::util::spi::{NamedSPI, NamedSPIProvider};
use rucene::util::BytesRef;
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_search::support::close_dir;
use crate::core_support::{
    assert_array_equals_msg, assert_equals, class, iae_message, random, test,
};

/// `TestBinarySortField`.
pub fn class_def() -> Arc<Class> {
    register_test_sort_field_providers();
    class(
        "org.apache.lucene.search.TestBinarySortField",
        vec![
            test("testEquals", test_equals),
            test("testToString", test_to_string),
            test("testIllegalMissingValue", test_illegal_missing_value),
            test("testSerialization", test_serialization),
            test("testEmptyIndex", test_empty_index),
            test("testSingleDocument", test_single_document),
            test("testAllMissing", test_all_missing),
            test(
                "testIndexSortOnlyOnNewIndex",
                test_index_sort_only_on_new_index,
            ),
            test("testSingleSegment", test_single_segment),
            test("testReverse", test_reverse),
            test("testMultiSegmentMerge", test_multi_segment_merge),
            test("testWithMissingValues", test_with_missing_values),
            test("testEmptyByteValues", test_empty_byte_values),
            test("testNestedBlocks", test_nested_blocks),
            test("testDeletes", test_deletes),
            test("testMultipleSortFields", test_multiple_sort_fields),
            test("testMultipleBinaryFields", test_multiple_binary_fields),
            test(
                "testMixCustomAndDefaultSortFields",
                test_mix_custom_and_default_sort_fields,
            ),
            test("testAddIndexes", test_add_indexes),
            test(
                "testParityWithSortedDocValues",
                test_parity_with_sorted_doc_values,
            ),
            test("testSearchSort", test_search_sort),
            test(
                "testFirstValueSortKeyIndexSort",
                test_first_value_sort_key_index_sort,
            ),
            test("testMinValueSortKey", test_min_value_sort_key),
            test("testCustomComparator", test_custom_comparator),
            test("testRandom", test_random),
        ],
    )
}

/// `lucene/core/src/test/META-INF/services/org.apache.lucene.index.SortFieldProvider`:
/// the two providers this class declares, which Java's `ServiceLoader`
/// finds on the test class path.
fn register_test_sort_field_providers() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        const SERVICE: &str = "org.apache.lucene.index.SortFieldProvider";
        let loader = ClassLoader::with_services(
            "lucene-core-tests",
            Some(Arc::clone(rucene_test_framework::class_loader())),
            vec![
                ServiceProvider::new(
                    SERVICE,
                    "org.apache.lucene.search.TestBinarySortField$FirstValueSortField$Provider",
                    NamedSPIProvider::new(|| {
                        Arc::new(FirstValueProvider) as Arc<dyn SortFieldProvider>
                    }),
                ),
                ServiceProvider::new(
                    SERVICE,
                    "org.apache.lucene.search.TestBinarySortField$MinValueSortField$Provider",
                    NamedSPIProvider::new(|| {
                        Arc::new(MinValueProvider) as Arc<dyn SortFieldProvider>
                    }),
                ),
            ],
        );
        reload_sort_field_providers(&loader)
            .unwrap_or_else(|e| rucene_test_framework::util::java_exceptions::rethrow_java(e));
    });
}

fn br(s: &str) -> BytesRef {
    BytesRef::from_char_sequence(s)
}

fn bsf(field: &str, reverse: bool) -> Result<SortField> {
    Ok(BinarySortField::new(field, reverse)?.into_sort_field())
}

fn bsf_missing(field: &str, reverse: bool, missing: Option<MissingValue>) -> Result<SortField> {
    Ok(BinarySortField::with_missing_value(field, reverse, missing)?.into_sort_field())
}

fn sort_of(fields: Vec<SortField>) -> Result<Sort> {
    Sort::new_fields(fields)
}

/// `try (Directory dir = newDirectory()) { body }`.
fn with_dir(body: impl FnOnce(&TestDirectory) -> Result<()>) -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let result = body(&dir);
    result.and(close_dir(Some(dir)))
}

/// `try (IndexWriter w = new IndexWriter(dir, iwc)) { body }`.
fn with_writer(
    dir: &TestDirectory,
    iwc: IndexWriterConfig,
    body: impl FnOnce(&IndexWriter) -> Result<()>,
) -> Result<()> {
    let w = IndexWriter::new(dir.as_directory(), iwc)?;
    let result = body(&w);
    result.and(w.close())
}

/// `try (DirectoryReader reader = DirectoryReader.open(dir)) { body }`.
fn with_reader(
    dir: &TestDirectory,
    body: impl FnOnce(&Arc<dyn DirectoryReader>) -> Result<()>,
) -> Result<()> {
    let reader = directory_reader::open(dir.as_directory())?;
    let result = body(&reader);
    result.and(reader.close())
}

fn leaves_of(reader: &Arc<dyn DirectoryReader>) -> Result<Vec<Arc<dyn LeafReader>>> {
    Ok((Arc::clone(reader) as Arc<dyn IndexReader>)
        .leaves()?
        .iter()
        .map(|ctx| ctx.leaf_reader())
        .collect())
}

/// `new byte[TestUtil.nextInt(random(), min, max)]` filled by
/// `random().nextBytes(b)`.
fn random_bytes(min: i32, max: i32) -> Vec<u8> {
    let mut b = vec![0u8; TestUtil::next_int(&*random(), min, max) as usize];
    random().next_bytes(&mut b);
    b
}

fn binary_dv(field: &str, value: BytesRef) -> Box<BinaryDocValuesField> {
    Box::new(BinaryDocValuesField::new(field, value))
}

fn stored_int(field: &str, value: i32) -> Result<Box<StoredField>> {
    Ok(Box::new(StoredField::new_number(
        field,
        NumericValue::Int(value),
    )?))
}

/// `Integer.parseInt(s)`.
fn parse_int(s: Option<String>) -> i32 {
    s.and_then(|s| s.parse().ok()).unwrap_or_else(|| {
        randomizedtesting::jdk::exception::Throwable::new(
            "java.lang.NumberFormatException",
            "For input string",
        )
        .throw()
    })
}

fn test_equals() -> Result<()> {
    let sf = bsf("a", false)?;
    // `sf.equals(null)`: a `SortField` value is never null.
    Assert::assert_false(false);
    assert_equals(&sf, &sf);

    let sf2 = bsf("a", false)?;
    assert_equals(&sf, &sf2);
    assert_equals(&hash_of(&sf), &hash_of(&sf2));

    Assert::assert_false(sf == bsf("a", true)?);
    Assert::assert_false(sf == bsf("b", false)?);
    Assert::assert_false(sf == bsf_missing("a", false, Some(MissingValue::StringLast))?);
    // `sf.equals("foo")`: an object of another class is never equal; a
    // Rust `SortField` cannot be compared with a string.
    Assert::assert_false(false);
    Ok(())
}

fn hash_of<T: std::hash::Hash>(t: &T) -> u64 {
    use std::hash::{BuildHasher, BuildHasherDefault};
    BuildHasherDefault::<std::collections::hash_map::DefaultHasher>::default().hash_one(t)
}

fn test_to_string() -> Result<()> {
    assert_equals("<binary: \"a\">", bsf("a", false)?.to_string().as_str());
    assert_equals("<binary: \"a\">!", bsf("a", true)?.to_string().as_str());
    Assert::assert_true(
        bsf_missing("a", false, Some(MissingValue::StringLast))?
            .to_string()
            .contains("missingValue"),
    );
    Ok(())
}

fn test_illegal_missing_value() -> Result<()> {
    // `new BinarySortField("a", false, SortField.FIELD_DOC)`: Java passes a
    // `SortField` as the missing value; Rust's missing value is a
    // `MissingValue`, so a value other than STRING_FIRST/STRING_LAST stands
    // for it and takes the same check.
    iae_message(|| BinarySortField::with_missing_value("a", false, Some(MissingValue::Int(0))));
    Ok(())
}

/// Round-trips through the registered `SortFieldProvider` (verifies SPI
/// registration).
fn test_serialization() -> Result<()> {
    assert_serializes(bsf("field", false)?)?;
    assert_serializes(bsf("field", true)?)?;
    assert_serializes(bsf_missing(
        "field",
        false,
        Some(MissingValue::StringFirst),
    )?)?;
    assert_serializes(bsf_missing("field", true, Some(MissingValue::StringLast))?)
}

fn assert_serializes(sf: SortField) -> Result<()> {
    let mut out = ByteBuffersDataOutput::new();
    write_sort_field(&sf, &mut out)?;
    let mut in_ = out.to_data_input()?;
    let read = sort_field_provider_for_name(BinarySortField::NAME)?.read_sort_field(&mut in_)?;
    assert_equals(&sf, &read);
    Ok(())
}

fn index_sorted_iwc(sort: Sort) -> Result<IndexWriterConfig> {
    let iwc = LuceneTestCase::new_index_writer_config();
    iwc.set_index_sort(sort)?;
    Ok(iwc)
}

fn test_empty_index() -> Result<()> {
    with_dir(|dir| {
        let iwc = index_sorted_iwc(sort_of(vec![bsf("dv", random().next_boolean())?])?)?;
        with_writer(dir, iwc, |w| {
            w.commit()?;
            Ok(())
        })?;
        with_reader(dir, |reader| {
            assert_equals(&0, &reader.max_doc());
            Ok(())
        })
    })
}

fn test_single_document() -> Result<()> {
    with_dir(|dir| {
        let iwc = index_sorted_iwc(sort_of(vec![bsf("dv", random().next_boolean())?])?)?;
        with_writer(dir, iwc, |w| {
            let mut doc = Document::new();
            doc.add(binary_dv("dv", br("only")));
            w.add_document(&doc)?;
            w.force_merge(1)
        })?;
        with_reader(dir, |reader| {
            assert_equals(&1, &reader.max_doc());
            let mut dv = leaves_of(reader)?[0]
                .get_binary_doc_values("dv")?
                .expect("dv");
            Assert::assert_true(dv.advance_exact(0)?);
            assert_equals(&br("only"), &dv.binary_value()?);
            Ok(())
        })
    })
}

/// Index sorting on a field that no document has: sort must be a no-op,
/// index stays valid.
fn test_all_missing() -> Result<()> {
    let reverse = random().next_boolean();
    with_dir(|dir| {
        let missing = if random().next_boolean() {
            Some(MissingValue::StringFirst)
        } else {
            None
        };
        let iwc = index_sorted_iwc(sort_of(vec![bsf_missing("dv", reverse, missing)?])?)?;
        iwc.set_max_buffered_docs(TestUtil::next_int(&*random(), 10, 30))?;
        let num_docs = LuceneTestCase::at_least(50);
        with_writer(dir, iwc, |w| {
            for i in 0..num_docs {
                let mut doc = Document::new();
                doc.add(stored_int("ord", i)?);
                w.add_document(&doc)?;
            }
            if random().next_boolean() {
                w.force_merge(1)?;
            }
            Ok(())
        })?;
        with_reader(dir, |reader| {
            assert_equals(&num_docs, &reader.max_doc());
            Ok(())
        })
    })
}

/// The index sort is immutable: a binary sort can only be set on a new
/// index, and cannot be added to, or changed on, an existing one.
fn test_index_sort_only_on_new_index() -> Result<()> {
    with_dir(|dir| {
        with_writer(dir, LuceneTestCase::new_index_writer_config(), |w| {
            let mut doc = Document::new();
            doc.add(binary_dv("dv", br("a")));
            w.add_document(&doc)?;
            w.commit()?;
            Ok(())
        })?;
        let sorted_config = index_sorted_iwc(sort_of(vec![bsf("dv", false)?])?)?;
        iae_message(|| IndexWriter::new(dir.as_directory(), sorted_config));
        Ok(())
    })?;

    with_dir(|dir| {
        with_writer(
            dir,
            index_sorted_iwc(sort_of(vec![bsf("dv", false)?])?)?,
            |w| {
                let mut doc = Document::new();
                doc.add(binary_dv("dv", br("a")));
                w.add_document(&doc)?;
                w.commit()?;
                Ok(())
            },
        )?;
        with_writer(
            dir,
            index_sorted_iwc(sort_of(vec![bsf("dv", false)?])?)?,
            |w| {
                assert_equals(&1, &w.get_doc_stats()?.max_doc);
                Ok(())
            },
        )?;
        let changed = index_sorted_iwc(sort_of(vec![bsf("dv", true)?])?)?;
        iae_message(|| IndexWriter::new(dir.as_directory(), changed));
        Ok(())
    })
}

fn test_single_segment() -> Result<()> {
    assert_index_sorted(false, false, false)
}

fn test_reverse() -> Result<()> {
    assert_index_sorted(false, true, false)
}

fn test_multi_segment_merge() -> Result<()> {
    assert_index_sorted(true, false, false)?;
    assert_index_sorted(true, true, false)
}

fn test_with_missing_values() -> Result<()> {
    assert_index_sorted(false, false, true)?;
    assert_index_sorted(true, false, true)?;
    assert_index_sorted(true, true, true)
}

/// Mixes empty (zero-length) values with non-empty ones and missing values.
fn test_empty_byte_values() -> Result<()> {
    let reverse = random().next_boolean();
    let missing_value = if random().next_boolean() {
        MissingValue::StringFirst
    } else {
        MissingValue::StringLast
    };
    let missing_last = missing_value == MissingValue::StringLast;
    with_dir(|dir| {
        let iwc = index_sorted_iwc(sort_of(vec![bsf_missing(
            "dv",
            reverse,
            Some(missing_value),
        )?])?)?;
        iwc.set_max_buffered_docs(TestUtil::next_int(&*random(), 10, 30))?;
        let num_docs = LuceneTestCase::at_least(100);
        with_writer(dir, iwc, |w| {
            for _ in 0..num_docs {
                let mut doc = Document::new();
                let r = random().next_int_bound(3);
                if r == 0 {
                } else if r == 1 {
                    doc.add(binary_dv("dv", BytesRef::new(Vec::new())));
                } else {
                    doc.add(binary_dv("dv", BytesRef::new(random_bytes(1, 5))));
                }
                w.add_document(&doc)?;
            }
            w.force_merge(1)
        })?;
        with_reader(dir, |reader| {
            assert_leaves_sorted(reader, reverse, missing_last)
        })
    })
}

/// Index sorting with document blocks (nested documents): the sort value
/// lives on the parent document, and blocks must stay intact and ordered by
/// their parent's value.
fn test_nested_blocks() -> Result<()> {
    let reverse = random().next_boolean();
    with_dir(|dir| {
        let iwc = index_sorted_iwc(sort_of(vec![bsf("sort", reverse)?])?)?;
        iwc.set_parent_field(Some("parent".to_string()));
        iwc.set_merge_policy(Some(Arc::new(LuceneTestCase::new_log_merge_policy())))?;
        let num_blocks = LuceneTestCase::at_least(80);
        with_writer(dir, iwc, |w| {
            for b in 0..num_blocks {
                let mut block = Vec::new();
                let num_children = random().next_int_bound(3);
                for _c in 0..num_children {
                    let mut child = Document::new();
                    child.add(stored_int("block", b)?);
                    child.add(Box::new(StoredField::new_string(
                        "role",
                        "child".to_string(),
                    )?));
                    block.push(child);
                }
                let mut parent = Document::new();
                let val = random_bytes(1, 10);
                parent.add(binary_dv("sort", BytesRef::new(val)));
                parent.add(stored_int("block", b)?);
                parent.add(Box::new(StoredField::new_string(
                    "role",
                    "parent".to_string(),
                )?));
                block.push(parent);
                w.add_documents(&block)?;
                if LuceneTestCase::rarely() {
                    w.commit()?;
                }
            }
            if random().next_boolean() {
                w.force_merge(1)?;
            }
            Ok(())
        })?;

        with_reader(dir, |reader| {
            for leaf in leaves_of(reader)? {
                let mut parents = leaf.get_numeric_doc_values("parent")?.expect("parent");
                let mut sort = leaf.get_binary_doc_values("sort")?.expect("sort");
                let stored_fields = leaf.stored_fields()?;
                let mut previous: Option<BytesRef> = None;
                let mut previous_doc = -1;
                loop {
                    let doc = parents.next_doc()?;
                    if doc == NO_MORE_DOCS {
                        break;
                    }
                    Assert::assert_true_msg(
                        "parent must have the sort value",
                        sort.advance_exact(doc)?,
                    );
                    let current = BytesRef::deep_copy_of(&sort.binary_value()?);
                    assert_equals(
                        &Some("parent".to_string()),
                        &stored_fields.document(doc)?.get("role"),
                    );
                    let block = parse_int(stored_fields.document(doc)?.get("block"));
                    for d in previous_doc + 1..doc {
                        assert_equals(
                            &Some("child".to_string()),
                            &stored_fields.document(d)?.get("role"),
                        );
                        assert_equals(&block, &parse_int(stored_fields.document(d)?.get("block")));
                    }
                    if let Some(previous) = &previous {
                        let mut cmp = compare_to(previous, &current);
                        if reverse {
                            cmp = -cmp;
                        }
                        Assert::assert_true_msg(
                            &format!("parent sort values out of order: {previous} vs {current}"),
                            cmp <= 0,
                        );
                    }
                    previous = Some(current);
                    previous_doc = doc;
                }
            }
            Ok(())
        })
    })
}

/// `BytesRef.compareTo(BytesRef)`.
fn compare_to(a: &BytesRef, b: &BytesRef) -> i32 {
    match a.cmp(b) {
        Ordering::Less => -1,
        Ordering::Equal => 0,
        Ordering::Greater => 1,
    }
}

/// Deleting documents and then merging must keep the surviving documents
/// sorted.
fn test_deletes() -> Result<()> {
    let reverse = random().next_boolean();
    with_dir(|dir| {
        let iwc = index_sorted_iwc(sort_of(vec![bsf("dv", reverse)?])?)?;
        iwc.set_max_buffered_docs(TestUtil::next_int(&*random(), 20, 50))?;
        let num_docs = LuceneTestCase::at_least(200);
        with_writer(dir, iwc, |w| {
            for i in 0..num_docs {
                let mut doc = Document::new();
                doc.add(Box::new(StringField::new("id", i.to_string(), Store::NO)?));
                doc.add(binary_dv("dv", BytesRef::new(random_bytes(1, 8))));
                w.add_document(&doc)?;
            }
            for i in 0..num_docs {
                if random().next_int_bound(3) == 0 {
                    w.delete_documents_by_terms(vec![Term::from_text("id", &i.to_string())])?;
                }
            }
            w.force_merge(1)
        })?;
        with_reader(dir, |reader| assert_leaves_sorted(reader, reverse, false))
    })
}

/// Binary sort combined with a numeric sort, as primary and as secondary.
fn test_multiple_sort_fields() -> Result<()> {
    for binary_first in [true, false] {
        with_dir(|dir| {
            let binary = bsf("bin", false)?;
            let numeric = SortField::new_reverse(Some("num".into()), SortFieldType::Long, false)?;
            let sort = if binary_first {
                sort_of(vec![binary, numeric])?
            } else {
                sort_of(vec![numeric, binary])?
            };
            let iwc = index_sorted_iwc(sort)?;
            iwc.set_max_buffered_docs(TestUtil::next_int(&*random(), 20, 50))?;
            let num_docs = LuceneTestCase::at_least(200);
            with_writer(dir, iwc, |w| {
                for _ in 0..num_docs {
                    let mut doc = Document::new();
                    let b = vec![random().next_int_bound(4) as u8];
                    doc.add(binary_dv("bin", BytesRef::new(b)));
                    doc.add(Box::new(NumericDocValuesField::new(
                        "num",
                        i64::from(random().next_int_bound(4)),
                    )));
                    w.add_document(&doc)?;
                }
                w.force_merge(1)
            })?;
            with_reader(dir, |reader| {
                let leaf = LuceneTestCase::get_only_leaf_reader(
                    Arc::clone(reader) as Arc<dyn IndexReader>
                )?;
                let mut bin = leaf.get_binary_doc_values("bin")?.expect("bin");
                let mut num = leaf.get_numeric_doc_values("num")?.expect("num");
                let mut prev_bin: Option<BytesRef> = None;
                let mut prev_num = 0i64;
                for doc in 0..leaf.max_doc() {
                    Assert::assert_true(bin.advance_exact(doc)?);
                    Assert::assert_true(num.advance_exact(doc)?);
                    let cur_bin = BytesRef::deep_copy_of(&bin.binary_value()?);
                    let cur_num = num.long_value()?;
                    if doc > 0 {
                        let prev = prev_bin.as_ref().expect("doc > 0");
                        if binary_first {
                            let c = compare_to(prev, &cur_bin);
                            Assert::assert_true(c <= 0);
                            if c == 0 {
                                Assert::assert_true(prev_num <= cur_num);
                            }
                        } else {
                            Assert::assert_true(prev_num <= cur_num);
                            if prev_num == cur_num {
                                Assert::assert_true(compare_to(prev, &cur_bin) <= 0);
                            }
                        }
                    }
                    prev_bin = Some(cur_bin);
                    prev_num = cur_num;
                }
                Ok(())
            })
        })?;
    }
    Ok(())
}

/// Two binary doc-values fields in the same segment, used together as a
/// two-level index sort.
fn test_multiple_binary_fields() -> Result<()> {
    with_dir(|dir| {
        let iwc = index_sorted_iwc(sort_of(vec![bsf("a", false)?, bsf("b", false)?])?)?;
        iwc.set_max_buffered_docs(25)?;
        iwc.set_merge_policy(Some(NoMergePolicy::instance()))?; // keep the flush-sorted segments separate
        let num_docs = 300;
        with_writer(dir, iwc, |w| {
            for _ in 0..num_docs {
                let mut doc = Document::new();
                let a = vec![random().next_int_bound(3) as u8]; // low cardinality so ties on 'a' exercise 'b'
                let b = random_bytes(1, 6);
                doc.add(binary_dv("a", BytesRef::new(a)));
                doc.add(binary_dv("b", BytesRef::new(b)));
                w.add_document(&doc)?;
            }
            Ok(())
        })?;
        with_reader(dir, |reader| {
            let leaves = leaves_of(reader)?;
            Assert::assert_true_msg("expected multiple flush-sorted segments", leaves.len() > 1);
            for leaf in leaves {
                let mut da = leaf.get_binary_doc_values("a")?.expect("a");
                let mut db = leaf.get_binary_doc_values("b")?.expect("b");
                let mut prev_a: Option<BytesRef> = None;
                let mut prev_b: Option<BytesRef> = None;
                for doc in 0..leaf.max_doc() {
                    Assert::assert_true(da.advance_exact(doc)?);
                    Assert::assert_true(db.advance_exact(doc)?);
                    let a = BytesRef::deep_copy_of(&da.binary_value()?);
                    let b = BytesRef::deep_copy_of(&db.binary_value()?);
                    if let Some(pa) = &prev_a {
                        let c = compare_to(pa, &a);
                        Assert::assert_true_msg("primary binary field out of order", c <= 0);
                        if c == 0 {
                            Assert::assert_true_msg(
                                "secondary binary field out of order",
                                compare_to(prev_b.as_ref().expect("set with a"), &b) <= 0,
                            );
                        }
                    }
                    prev_a = Some(a);
                    prev_b = Some(b);
                }
            }
            Ok(())
        })
    })
}

/// Mixes a custom `BinarySortField` subclass (`FirstValueSortField`, which
/// decodes its value) with a default binary sort field and a numeric sort
/// field in a single index sort. Also exercises two binary doc-values fields
/// in the same segment, through flush, merge and reopen.
fn test_mix_custom_and_default_sort_fields() -> Result<()> {
    with_dir(|dir| {
        let sort = sort_of(vec![
            first_value_sort_field("a", false)?, // custom binary (decoding comparator)
            bsf("b", false)?,                    // default binary
            SortField::new_reverse(Some("n".into()), SortFieldType::Long, false)?, // numeric
        ])?;
        let iwc = index_sorted_iwc(sort.clone())?;
        iwc.set_max_buffered_docs(25)?;
        if random().next_boolean() {
            iwc.set_merge_policy(Some(NoMergePolicy::instance()))?;
        } else {
            iwc.set_merge_policy(Some(Arc::new(LuceneTestCase::new_log_merge_policy())))?;
        }
        let num_docs = LuceneTestCase::at_least(300);
        with_writer(dir, iwc, |w| {
            for _ in 0..num_docs {
                let mut doc = Document::new();
                let mut a_values = vec![vec![random().next_int_bound(3) as u8]]; // low-card first value -> ties
                if random().next_boolean() {
                    a_values.push(random_bytes(1, 4));
                }
                doc.add(binary_dv("a", encode_values(&a_values)?));
                doc.add(binary_dv(
                    "b",
                    BytesRef::new(vec![random().next_int_bound(3) as u8]),
                ));
                doc.add(Box::new(NumericDocValuesField::new(
                    "n",
                    i64::from(random().next_int_bound(5)),
                )));
                w.add_document(&doc)?;
            }
            if random().next_boolean() {
                w.force_merge(1)?;
            }
            Ok(())
        })?;
        with_reader(dir, |reader| {
            for leaf in leaves_of(reader)? {
                assert_equals(&Some(&sort), &leaf.get_meta_data().sort());
                let mut da = leaf.get_binary_doc_values("a")?.expect("a");
                let mut db = leaf.get_binary_doc_values("b")?.expect("b");
                let mut dn = leaf.get_numeric_doc_values("n")?.expect("n");
                let mut prev: Option<(BytesRef, BytesRef, i64)> = None;
                for doc in 0..leaf.max_doc() {
                    Assert::assert_true(da.advance_exact(doc)?);
                    Assert::assert_true(db.advance_exact(doc)?);
                    Assert::assert_true(dn.advance_exact(doc)?);
                    let a = BytesRef::deep_copy_of(&da.binary_value()?);
                    let b = BytesRef::deep_copy_of(&db.binary_value()?);
                    let n = dn.long_value()?;
                    if let Some((prev_a, prev_b, prev_n)) = &prev {
                        let ca = compare_to(&first_value(prev_a)?, &first_value(&a)?); // decodes the first value
                        Assert::assert_true_msg("custom primary out of order", ca <= 0);
                        if ca == 0 {
                            let cb = compare_to(prev_b, &b);
                            Assert::assert_true_msg(
                                "default binary secondary out of order",
                                cb <= 0,
                            );
                            if cb == 0 {
                                Assert::assert_true_msg(
                                    "numeric tertiary out of order",
                                    *prev_n <= n,
                                );
                            }
                        }
                    }
                    prev = Some((a, b, n));
                }
            }
            Ok(())
        })
    })
}

/// addIndexes(CodecReader...) merges already-sorted segments into a sorted
/// index.
fn test_add_indexes() -> Result<()> {
    let reverse = random().next_boolean();
    let sort = sort_of(vec![bsf("dv", reverse)?])?;
    with_dir(|src1| {
        with_dir(|src2| {
            with_dir(|dest| {
                index_random(src1, sort.clone(), LuceneTestCase::at_least(100))?;
                index_random(src2, sort.clone(), LuceneTestCase::at_least(100))?;

                let iwc = index_sorted_iwc(sort.clone())?;
                with_writer(dest, iwc, |w| {
                    with_reader(src1, |r1| {
                        with_reader(src2, |r2| {
                            let mut readers: Vec<Arc<dyn CodecReader>> = Vec::new();
                            for leaf in leaves_of(r1)? {
                                readers.push(SlowCodecReaderWrapper::wrap(leaf)?);
                            }
                            for leaf in leaves_of(r2)? {
                                readers.push(SlowCodecReaderWrapper::wrap(leaf)?);
                            }
                            w.add_indexes_codec_readers(&readers)?;
                            w.force_merge(1)
                        })
                    })
                })?;
                with_reader(dest, |reader| assert_leaves_sorted(reader, reverse, false))
            })
        })
    })
}

/// Verifies parity of the produced order with sorting the same values as
/// SortedDocValues.
fn test_parity_with_sorted_doc_values() -> Result<()> {
    let num_docs = LuceneTestCase::at_least(300);
    let reverse = random().next_boolean();
    let mut values = Vec::new();
    for _ in 0..num_docs {
        values.push(random_bytes(1, 6));
    }

    let binary_order = index_and_read_stored_order(&values, bsf("dv", reverse)?, true)?;
    let sorted_order = index_and_read_stored_order(
        &values,
        SortField::new_reverse(Some("dv".into()), SortFieldType::String, reverse)?,
        false,
    )?;

    for i in 0..num_docs as usize {
        assert_array_equals_msg(
            Some(&format!("mismatch at position {i}")),
            &values[binary_order[i] as usize],
            &values[sorted_order[i] as usize],
        );
    }
    Ok(())
}

/// Uses `BinarySortField` to sort search results (the `getComparator` path).
fn test_search_sort() -> Result<()> {
    let reverse = random().next_boolean();
    let missing_value = if random().next_boolean() {
        MissingValue::StringFirst
    } else {
        MissingValue::StringLast
    };
    let missing_last = missing_value == MissingValue::StringLast;
    with_dir(|dir| {
        let num_docs = LuceneTestCase::at_least(100);
        with_writer(dir, LuceneTestCase::new_index_writer_config(), |w| {
            for _ in 0..num_docs {
                let mut doc = Document::new();
                if random().next_int_bound(5) != 0 {
                    doc.add(binary_dv("dv", BytesRef::new(random_bytes(0, 8))));
                }
                w.add_document(&doc)?;
            }
            Ok(())
        })?;
        with_reader(dir, |reader| {
            let searcher = LuceneTestCase::new_searcher(Arc::clone(reader));
            let td = searcher.search_with_sort(
                Arc::new(MatchAllDocsQuery::instance()),
                reader.max_doc(),
                &sort_of(vec![bsf_missing("dv", reverse, Some(missing_value))?])?,
            )?;
            let mut previous: Option<BytesRef> = None;
            let mut previous_missing = false;
            let mut first = true;
            for sd in &td.score_docs {
                let current = match sd.fields.as_ref().and_then(|f| f.first()) {
                    Some(SortValue::Bytes(b)) => Some(b.clone()),
                    _ => None,
                };
                let missing = current.is_none();
                if !first {
                    let mut cmp = compare_for_sort(
                        previous.as_ref(),
                        previous_missing,
                        current.as_ref(),
                        missing,
                        missing_last,
                    );
                    if reverse {
                        cmp = -cmp;
                    }
                    Assert::assert_true_msg("search results out of order", cmp <= 0);
                }
                previous = current;
                previous_missing = missing;
                first = false;
            }
            Ok(())
        })
    })
}

/// End-to-end test of the value-source extension point with a key that must
/// be *decoded* from the binary value (the raw bytes are not sortable
/// as-is). `FirstValueSortField` sorts on the first of several
/// length-prefixed values, mirroring how a multi-valued field is encoded.
/// The sort is persisted, reopened (which deserializes it through the custom
/// `SortFieldProvider` and runs `CheckIndex`) and merged, then the stored
/// order is verified against the decoded key — proving the derived key
/// survives serialization and is used on merge.
fn test_first_value_sort_key_index_sort() -> Result<()> {
    let reverse = random().next_boolean();
    with_dir(|dir| {
        let iwc = index_sorted_iwc(sort_of(vec![first_value_sort_field("dv", reverse)?])?)?;
        iwc.set_max_buffered_docs(TestUtil::next_int(&*random(), 20, 50))?;
        iwc.set_merge_policy(Some(Arc::new(LuceneTestCase::new_log_merge_policy())))?;
        let num_docs = LuceneTestCase::at_least(200);
        with_writer(dir, iwc, |w| {
            for _ in 0..num_docs {
                let mut doc = Document::new();
                doc.add(binary_dv("dv", encode_values(&random_values())?));
                w.add_document(&doc)?;
                if random().next_int_bound(30) == 0 {
                    w.commit()?;
                }
            }
            if random().next_boolean() {
                w.force_merge(1)?;
            }
            Ok(())
        })?;
        with_reader(dir, |reader| {
            for leaf in leaves_of(reader)? {
                assert_equals(
                    &Some(&sort_of(vec![first_value_sort_field("dv", reverse)?])?),
                    &leaf.get_meta_data().sort(),
                );
                let mut dv = leaf.get_binary_doc_values("dv")?.expect("dv");
                let mut previous: Option<BytesRef> = None;
                for doc in 0..leaf.max_doc() {
                    Assert::assert_true(dv.advance_exact(doc)?);
                    let current = BytesRef::deep_copy_of(&dv.binary_value()?);
                    if let Some(previous) = &previous {
                        let mut cmp = compare_to(&first_value(previous)?, &first_value(&current)?);
                        if reverse {
                            cmp = -cmp;
                        }
                        Assert::assert_true_msg("custom order violated", cmp <= 0);
                    }
                    previous = Some(current);
                }
            }
            Ok(())
        })
    })
}

fn random_values() -> Vec<Vec<u8>> {
    let count = TestUtil::next_int(&*random(), 1, 3);
    let mut values = Vec::new();
    for _ in 0..count {
        values.push(random_bytes(1, 8));
    }
    values
}

/// Encodes values as `[vInt length][bytes]` pairs, like a multi-valued
/// binary field.
fn encode_values(values: &[Vec<u8>]) -> Result<BytesRef> {
    let mut out = ByteBuffersDataOutput::new();
    for v in values {
        out.write_v_int(v.len() as i32)?;
        out.write_bytes(v, 0, v.len())?;
    }
    Ok(BytesRef::new(out.to_array_copy()?))
}

/// Selecting the MIN value of a multi-valued, self-describing binary field
/// (`MinValueSortField`). Forces many small flushed segments (no
/// force-merge) so each is sorted at flush using the value source, and also
/// covers the merge case; the order is verified against the per-document
/// minimum value.
fn test_min_value_sort_key() -> Result<()> {
    let reverse = random().next_boolean();
    let merge = random().next_boolean();
    with_dir(|dir| {
        let iwc = index_sorted_iwc(sort_of(vec![min_value_sort_field("dv", reverse)?])?)?;
        iwc.set_max_buffered_docs(25)?; // force frequent flushes
        if merge {
            iwc.set_merge_policy(Some(Arc::new(LuceneTestCase::new_log_merge_policy())))?;
        } else {
            iwc.set_merge_policy(Some(NoMergePolicy::instance()))?;
        }
        let num_docs = 400;
        with_writer(dir, iwc, |w| {
            for _ in 0..num_docs {
                let mut doc = Document::new();
                let count = TestUtil::next_int(&*random(), 1, 4);
                let mut values = Vec::new();
                for _v in 0..count {
                    values.push(random_bytes(1, 6));
                }
                doc.add(binary_dv("dv", encode_values(&values)?));
                w.add_document(&doc)?;
            }
            if merge && random().next_boolean() {
                w.force_merge(1)?;
            }
            Ok(())
        })?;
        with_reader(dir, |reader| {
            let leaves = leaves_of(reader)?;
            if !merge {
                Assert::assert_true_msg(
                    "expected multiple flush-sorted segments",
                    leaves.len() > 1,
                );
            }
            for leaf in leaves {
                assert_equals(
                    &Some(&sort_of(vec![min_value_sort_field("dv", reverse)?])?),
                    &leaf.get_meta_data().sort(),
                );
                let mut dv = leaf.get_binary_doc_values("dv")?.expect("dv");
                let mut previous: Option<BytesRef> = None;
                for doc in 0..leaf.max_doc() {
                    Assert::assert_true(dv.advance_exact(doc)?);
                    let key =
                        BytesRef::deep_copy_of(&min(&dv.binary_value()?)?.unwrap_or_default());
                    if let Some(previous) = &previous {
                        let mut cmp = compare_to(previous, &key);
                        if reverse {
                            cmp = -cmp;
                        }
                        Assert::assert_true_msg("min-value order violated", cmp <= 0);
                    }
                    previous = Some(key);
                }
            }
            Ok(())
        })
    })
}

/// `Comparator.comparingInt((BytesRef b) -> b.length).thenComparing(naturalOrder())`.
fn length_then_natural(a: &[u8], b: &[u8]) -> Ordering {
    a.len().cmp(&b.len()).then_with(|| a.cmp(b))
}

/// `Integer.signum`.
fn signum(o: Ordering) -> i32 {
    match o {
        Ordering::Less => -1,
        Ordering::Equal => 0,
        Ordering::Greater => 1,
    }
}

/// Exercises a non-natural `Comparator` through both the flush
/// (`IndexSorter#getDocComparator`) and merge
/// (`IndexSorter#getComparableValues`) comparison paths.
fn test_custom_comparator() -> Result<()> {
    // `IndexSorter.BinarySorter` compares `BytesRef`s that are never null
    // here (every document has a value), so the comparator ignores missing
    // values as Java's lambda would never see one.
    let sorter = BinarySorter::new(
        "test",
        false,
        Arc::new(|a: Option<&[u8]>, b: Option<&[u8]>| {
            length_then_natural(a.unwrap_or_default(), b.unwrap_or_default())
        }),
        Arc::new(|r: &dyn LeafReader| DocValues::get_binary(r, "dv")),
    );

    with_dir(|dir| {
        let num_docs = LuceneTestCase::at_least(100);
        let iwc = LuceneTestCase::new_index_writer_config();
        iwc.set_merge_policy(Some(NoMergePolicy::instance()))?;
        with_writer(dir, iwc, |w| {
            for i in 0..num_docs {
                let mut doc = Document::new();
                doc.add(binary_dv("dv", BytesRef::new(random_bytes(1, 8))));
                w.add_document(&doc)?;
                if i == num_docs / 2 {
                    w.commit()?;
                }
            }
            Ok(())
        })?;
        with_reader(dir, |reader| {
            let leaves = leaves_of(reader)?;
            for leaf in &leaves {
                let dc = sorter.get_doc_comparator(&**leaf, leaf.max_doc())?;
                let doc_values = read_all(&**leaf, "dv")?;
                for _iter in 0..200 {
                    let a = random().next_int_bound(leaf.max_doc());
                    let b = random().next_int_bound(leaf.max_doc());
                    assert_equals(
                        &signum(length_then_natural(
                            doc_values[a as usize].slice(),
                            doc_values[b as usize].slice(),
                        )),
                        &signum(dc(a, b)),
                    );
                }
            }

            if leaves.len() >= 2 {
                let mut cv = sorter.get_comparable_values(&leaves)?;
                let mut values = Vec::new();
                for leaf in &leaves {
                    values.push(read_all(&**leaf, "dv")?);
                }
                let n = leaves.len();
                let mut max_common = leaves[0].max_doc();
                for leaf in &leaves {
                    max_common = max_common.min(leaf.max_doc());
                }
                for doc in 0..max_common {
                    for r in 0..n {
                        cv.set_top_value(r, doc)?;
                    }
                    for a in 0..n {
                        for b in 0..n {
                            assert_equals(
                                &signum(length_then_natural(
                                    values[a][doc as usize].slice(),
                                    values[b][doc as usize].slice(),
                                )),
                                &signum(cv.compare(a, b)),
                            );
                        }
                    }
                }
            }
            Ok(())
        })
    })
}

/// Randomized stress test over values, missing, duplicates, commits,
/// deletes and merges.
fn test_random() -> Result<()> {
    let reverse = random().next_boolean();
    let missing_value = if random().next_boolean() {
        None
    } else if random().next_boolean() {
        Some(MissingValue::StringFirst)
    } else {
        Some(MissingValue::StringLast)
    };
    let missing_last = missing_value == Some(MissingValue::StringLast);
    let cardinality = TestUtil::next_int(&*random(), 1, 1000);
    let mut pool = Vec::with_capacity(cardinality as usize);
    for _ in 0..cardinality {
        pool.push(random_bytes(0, 12));
    }
    with_dir(|dir| {
        let iwc = index_sorted_iwc(sort_of(vec![bsf_missing("dv", reverse, missing_value)?])?)?;
        if random().next_boolean() {
            iwc.set_max_buffered_docs(TestUtil::next_int(&*random(), 20, 80))?;
        }
        iwc.set_merge_policy(Some(Arc::new(LuceneTestCase::new_log_merge_policy())))?;
        let num_docs = LuceneTestCase::at_least(500);
        with_writer(dir, iwc, |w| {
            for i in 0..num_docs {
                let mut doc = Document::new();
                doc.add(Box::new(StringField::new("id", i.to_string(), Store::NO)?));
                if random().next_int_bound(8) != 0 {
                    doc.add(binary_dv(
                        "dv",
                        BytesRef::new(pool[random().next_int_bound(cardinality) as usize].clone()),
                    ));
                }
                w.add_document(&doc)?;
                if random().next_int_bound(100) == 0 {
                    w.commit()?;
                }
                if random().next_int_bound(100) == 0 {
                    w.delete_documents_by_terms(vec![Term::from_text(
                        "id",
                        &random().next_int_bound(i + 1).to_string(),
                    )])?;
                }
            }
            if random().next_boolean() {
                w.force_merge(TestUtil::next_int(&*random(), 1, 3))?;
            }
            Ok(())
        })?;
        with_reader(dir, |reader| {
            assert_leaves_sorted(reader, reverse, missing_last)
        })
    })
}

fn index_random(dir: &TestDirectory, sort: Sort, num_docs: i32) -> Result<()> {
    let iwc = index_sorted_iwc(sort)?;
    iwc.set_max_buffered_docs(TestUtil::next_int(&*random(), 20, 50))?;
    with_writer(dir, iwc, |w| {
        for _ in 0..num_docs {
            let mut doc = Document::new();
            doc.add(binary_dv("dv", BytesRef::new(random_bytes(1, 8))));
            w.add_document(&doc)?;
        }
        Ok(())
    })
}

fn read_all(leaf: &dyn LeafReader, field: &str) -> Result<Vec<BytesRef>> {
    let mut values = Vec::new();
    let mut dv = leaf.get_binary_doc_values(field)?;
    for doc in 0..leaf.max_doc() {
        let has = match dv.as_mut() {
            Some(dv) => dv.advance_exact(doc)?,
            None => randomizedtesting::jdk::exception::Throwable::new(
                "java.lang.NullPointerException",
                "",
            )
            .throw(),
        };
        Assert::assert_true_msg("test assumes a value on every document", has);
        values.push(BytesRef::deep_copy_of(
            &dv.as_ref().expect("checked").binary_value()?,
        ));
    }
    Ok(values)
}

/// Asserts every leaf is sorted by "dv" according to the given reverse /
/// missing policy.
fn assert_leaves_sorted(
    reader: &Arc<dyn DirectoryReader>,
    reverse: bool,
    missing_last: bool,
) -> Result<()> {
    for leaf in leaves_of(reader)? {
        let mut dv = leaf.get_binary_doc_values("dv")?;
        let mut previous: Option<BytesRef> = None;
        let mut previous_missing = false;
        let mut first = true;
        for doc in 0..leaf.max_doc() {
            let has_value = match dv.as_mut() {
                Some(dv) => dv.advance_exact(doc)?,
                None => false,
            };
            let current = if has_value {
                Some(BytesRef::deep_copy_of(
                    &dv.as_ref().expect("has value").binary_value()?,
                ))
            } else {
                None
            };
            if !first {
                let mut cmp = compare_for_sort(
                    previous.as_ref(),
                    previous_missing,
                    current.as_ref(),
                    !has_value,
                    missing_last,
                );
                if reverse {
                    cmp = -cmp;
                }
                Assert::assert_true_msg(
                    &format!(
                        "values out of order at doc {doc}: prev={} cur={}",
                        opt_string(&previous),
                        opt_string(&current)
                    ),
                    cmp <= 0,
                );
            }
            previous = current;
            previous_missing = !has_value;
            first = false;
        }
    }
    Ok(())
}

/// `String.valueOf(BytesRef)`.
fn opt_string(b: &Option<BytesRef>) -> String {
    b.as_ref()
        .map(|b| b.to_string())
        .unwrap_or_else(|| "null".to_string())
}

/// Indexes random byte[] values, sorts the index, and verifies the order
/// within each leaf.
fn assert_index_sorted(multi_segment: bool, reverse: bool, missing: bool) -> Result<()> {
    let num_docs = LuceneTestCase::at_least(200);
    let missing_value = if missing {
        if random().next_boolean() {
            Some(MissingValue::StringFirst)
        } else {
            Some(MissingValue::StringLast)
        }
    } else {
        None
    };
    let missing_last = missing_value == Some(MissingValue::StringLast);

    with_dir(|dir| {
        let iwc = index_sorted_iwc(sort_of(vec![bsf_missing("dv", reverse, missing_value)?])?)?;
        if multi_segment {
            iwc.set_max_buffered_docs(TestUtil::next_int(&*random(), 20, 50))?;
            iwc.set_merge_policy(Some(Arc::new(LuceneTestCase::new_log_merge_policy())))?;
        }
        with_writer(dir, iwc, |w| {
            for i in 0..num_docs {
                let mut doc = Document::new();
                if !missing || random().next_int_bound(5) != 0 {
                    doc.add(binary_dv("dv", BytesRef::new(random_bytes(0, 12))));
                }
                doc.add(stored_int("ord", i)?);
                w.add_document(&doc)?;
                if multi_segment && random().next_int_bound(50) == 0 {
                    w.commit()?;
                }
            }
            if multi_segment && random().next_boolean() {
                w.force_merge(1)?;
            }
            Ok(())
        })?;

        with_reader(dir, |reader| {
            assert_leaves_sorted(reader, reverse, missing_last)?;
            for leaf in leaves_of(reader)? {
                Assert::assert_true_msg(
                    "index sort not persisted",
                    leaf.get_meta_data().sort().is_some(),
                );
            }
            Ok(())
        })
    })
}

/// Compares two (possibly missing) values the way the index sort would,
/// before applying reverse.
fn compare_for_sort(
    a: Option<&BytesRef>,
    a_missing: bool,
    b: Option<&BytesRef>,
    b_missing: bool,
    missing_last: bool,
) -> i32 {
    if a_missing || b_missing {
        if a_missing == b_missing {
            return 0;
        }
        let c = if a_missing { -1 } else { 1 };
        return if missing_last { -c } else { c };
    }
    compare_to(a.expect("not missing"), b.expect("not missing"))
}

fn index_and_read_stored_order(
    values: &[Vec<u8>],
    sort_field: SortField,
    binary: bool,
) -> Result<Vec<i32>> {
    let mut order = Vec::new();
    with_dir(|dir| {
        let iwc = index_sorted_iwc(sort_of(vec![sort_field])?)?;
        iwc.set_max_buffered_docs(TestUtil::next_int(&*random(), 20, 60))?;
        iwc.set_merge_policy(Some(Arc::new(LuceneTestCase::new_log_merge_policy())))?;
        with_writer(dir, iwc, |w| {
            for (i, value) in values.iter().enumerate() {
                let mut doc = Document::new();
                let v = BytesRef::new(value.clone());
                if binary {
                    doc.add(binary_dv("dv", v));
                } else {
                    doc.add(Box::new(SortedDocValuesField::new("dv", v)));
                }
                doc.add(stored_int("ord", i as i32)?);
                w.add_document(&doc)?;
            }
            w.force_merge(1)
        })?;
        with_reader(dir, |reader| {
            let leaf =
                LuceneTestCase::get_only_leaf_reader(Arc::clone(reader) as Arc<dyn IndexReader>)?;
            let stored_fields = leaf.stored_fields()?;
            for doc in 0..leaf.max_doc() {
                order.push(parse_int(stored_fields.document(doc)?.get("ord")));
            }
            Ok(())
        })
    })?;
    Ok(order)
}

// -----------------------------------------------------------------------------
// FirstValueSortField / MinValueSortField
// -----------------------------------------------------------------------------

/// The anonymous `BinaryDocValues` both subclasses return: the delegate's
/// iteration, with `binaryValue()` mapped by `key`.
struct KeyDocValues {
    binary: Box<dyn BinaryDocValues>,
    key: fn(&BytesRef) -> Result<BytesRef>,
}

impl DocIdSetIterator for KeyDocValues {
    fn doc_id(&self) -> i32 {
        self.binary.doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.binary.next_doc()
    }

    fn advance(&mut self, t: i32) -> Result<i32> {
        self.binary.advance(t)
    }

    fn cost(&self) -> i64 {
        self.binary.cost()
    }
}

impl DocValuesIterator for KeyDocValues {
    fn advance_exact(&mut self, t: i32) -> Result<bool> {
        self.binary.advance_exact(t)
    }
}

impl BinaryDocValues for KeyDocValues {
    fn binary_value(&self) -> Result<BytesRef> {
        (self.key)(&self.binary.binary_value()?)
    }
}

const FIRST_VALUE_NAME: &str = "FirstValueSortField";
const MIN_VALUE_NAME: &str = "MinValueSortField";

/// `new FirstValueSortField(field, reverse)`: a `BinarySortField`
/// subclass whose sort key is the first `[vInt length][bytes]` value.
fn first_value_sort_field(field: &str, reverse: bool) -> Result<SortField> {
    Ok(BinarySortField::with_sort_key(
        field,
        reverse,
        None,
        FIRST_VALUE_NAME,
        Some(SortKeySource(Arc::new(
            |this: &BinarySortField, reader: &dyn LeafReader| {
                let binary = DocValues::get_binary(reader, this.get_field().unwrap_or(""))?;
                Ok(Box::new(KeyDocValues {
                    binary,
                    key: |raw| first_value(raw),
                }) as Box<dyn BinaryDocValues>)
            },
        ))),
    )?
    .into_sort_field())
}

/// `FirstValueSortField.firstValue(BytesRef)`.
fn first_value(raw: &BytesRef) -> Result<BytesRef> {
    let mut in_ = ByteArrayDataInput::with_range(&raw.bytes, raw.offset, raw.length);
    let length = in_.read_v_int()?;
    Ok(BytesRef {
        bytes: raw.bytes.clone(),
        offset: in_.position(),
        length: length as usize,
    })
}

/// `new MinValueSortField(field, reverse)`: a `BinarySortField` subclass
/// whose sort key is the smallest `[vInt length][bytes]` value.
fn min_value_sort_field(field: &str, reverse: bool) -> Result<SortField> {
    Ok(BinarySortField::with_sort_key(
        field,
        reverse,
        None,
        MIN_VALUE_NAME,
        Some(SortKeySource(Arc::new(
            |this: &BinarySortField, reader: &dyn LeafReader| {
                let binary = DocValues::get_binary(reader, this.get_field().unwrap_or(""))?;
                Ok(Box::new(KeyDocValues {
                    binary,
                    key: |raw| {
                        // Java returns `null` for an empty value; a key that is
                        // `None` has no `BytesRef` to return here.
                        min(raw)?.ok_or_else(|| {
                            LuceneError::unchecked(
                                "java.lang.NullPointerException",
                                "min of no values",
                            )
                        })
                    },
                }) as Box<dyn BinaryDocValues>)
            },
        ))),
    )?
    .into_sort_field())
}

/// `MinValueSortField.min(BytesRef)`: the minimum of the
/// `[vInt length][bytes]` values packed in `raw`, as a view into `raw`.
fn min(raw: &BytesRef) -> Result<Option<BytesRef>> {
    let mut in_ = ByteArrayDataInput::with_range(&raw.bytes, raw.offset, raw.length);
    let mut best: Option<BytesRef> = None;
    while !in_.eof() {
        let length = in_.read_v_int()? as usize;
        let value = BytesRef {
            bytes: raw.bytes.clone(),
            offset: in_.position(),
            length,
        };
        in_.set_position(in_.position() + length);
        if best.as_ref().is_none_or(|b| value < *b) {
            best = Some(value);
        }
    }
    Ok(best)
}

/// `FirstValueSortField.Provider`.
struct FirstValueProvider;

impl NamedSPI for FirstValueProvider {
    fn get_name(&self) -> &str {
        FIRST_VALUE_NAME
    }
}

impl SortFieldProvider for FirstValueProvider {
    fn read_sort_field(&self, in_: &mut dyn DataInput) -> Result<SortField> {
        let field = in_.read_string()?;
        first_value_sort_field(&field, in_.read_int()? == 1)
    }

    fn write_sort_field(&self, sf: &SortField, out: &mut dyn DataOutput) -> Result<()> {
        out.write_string(sf.field().unwrap_or(""))?;
        out.write_int(if sf.reverse() { 1 } else { 0 })
    }
}

/// `MinValueSortField.Provider`.
struct MinValueProvider;

impl NamedSPI for MinValueProvider {
    fn get_name(&self) -> &str {
        MIN_VALUE_NAME
    }
}

impl SortFieldProvider for MinValueProvider {
    fn read_sort_field(&self, in_: &mut dyn DataInput) -> Result<SortField> {
        let field = in_.read_string()?;
        min_value_sort_field(&field, in_.read_int()? == 1)
    }

    fn write_sort_field(&self, sf: &SortField, out: &mut dyn DataOutput) -> Result<()> {
        out.write_string(sf.field().unwrap_or(""))?;
        out.write_int(if sf.reverse() { 1 } else { 0 })
    }
}
