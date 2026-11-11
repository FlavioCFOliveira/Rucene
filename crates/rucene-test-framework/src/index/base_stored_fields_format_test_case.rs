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

//! Port of `org.apache.lucene.tests.index.BaseStoredFieldsFormatTestCase`:
//! base class aiming at testing stored fields formats. To test a new format,
//! all you need is to register a new codec which uses it and extend this
//! class and override `getCodec()`.
//!
//! The class is declared by [`class()`] and its members are the trait
//! [`BaseStoredFieldsFormatTestCase`]; see
//! [`base_index_file_format_test_case`](super::base_index_file_format_test_case)
//! for how a suite runs.
//!
//! **Divergences** (declared, each confined to the member named):
//! * `docs` maps (`testRandomStoredFields`, `testRandomStoredFieldsWithIndexSort`):
//!   Java's `HashMap<String, Document>` key order decides which document a
//!   random index picks; the port keeps the expected values in a
//!   `HashMap` and orders the keys as `java.util.HashMap` iterates them
//!   ([`java_hash_map_key_order`]).
//! * `testMergeFilterReader`: `StoredFieldDataInput(new
//!   ByteArrayDataInput(bytes, offset, length))` — the port's
//!   `ByteArrayDataInput` borrows its bytes and a stored field owns its
//!   input, so the input is an owning reader over a copy of the same bytes
//!   (same length, same content read). `DummyFilterDirectoryReader
//!   .getReaderCacheHelper()` returns `null` in Java; the port's single
//!   `FilterDirectoryReader` answers the wrapped reader's helper, which this
//!   test never asks for (`addIndexesSlowly` reads the leaves only).

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::{Arc, LazyLock, Mutex, PoisonError};

use randomizedtesting::generators::{RandomNumbers, RandomPicks, RandomStrings};
use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::random::Random;
use randomizedtesting::jdk::reflect::{modifier, Class};
use randomizedtesting::jdk::thread::Thread;
use randomizedtesting::junit::Assert;
use rucene::analysis::Analyzer;
use rucene::codecs::Codec;
use rucene::document::{
    Document, Field, FieldType, IntPoint, NumericDocValuesField, NumericValue, Store, StoredField,
    StringField, TextField,
};
use rucene::error::{LuceneError, Result};
use rucene::index::directory_reader::{self, DirectoryReader};
use rucene::index::filter_directory_reader::{
    DirectoryReaderWrapper, FilterDirectoryReader, SubReaderWrapper,
};
use rucene::index::index_reader::{CacheHelper, StoredFields};
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::merge_policy::NoMergePolicy;
use rucene::index::terms::Term;
use rucene::index::{IndexOptions, IndexReader, IndexableField, LeafReader, StoredFieldVisitor};
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::sort::{Sort, SortField, SortFieldType};
use rucene::search::term_query::TermQuery;
use rucene::search::Query;
use rucene::store::Directory;
use rucene::util::BytesRef;

use super::base_index_file_format_test_case::{format_test, rethrow, BaseIndexFileFormatTestCase};
use crate::analysis::MockAnalyzer;
use crate::index::random_index_writer::RandomIndexWriter;
use crate::index::MismatchedDirectoryReader;
use crate::store::{MockDirectoryWrapper, Throttling};
use crate::util::line_file_docs::LineFileDocs;
use crate::util::lucene_test_case::{LuceneTestCase, Nightly, VERBOSE};
use crate::util::TestUtil;

/// The declaration of `org.apache.lucene.tests.index.BaseStoredFieldsFormatTestCase`
/// (`public abstract class BaseStoredFieldsFormatTestCase extends
/// BaseIndexFileFormatTestCase`), with its tests.
pub fn class() -> Arc<Class> {
    static CLASS: LazyLock<Arc<Class>> = LazyLock::new(|| {
        let t = |name: &str, body: fn(&dyn BaseStoredFieldsFormatTestCase) -> Result<()>| {
            format_test(name, move |c| body(stored_fields_case(c)))
        };
        Class::builder("org.apache.lucene.tests.index.BaseStoredFieldsFormatTestCase")
            .modifiers(modifier::PUBLIC | modifier::ABSTRACT)
            .extends(&super::base_index_file_format_test_case::class())
            .method(t("testRandomStoredFields", |c| {
                c.test_random_stored_fields()
            }))
            .method(t("testStoredFieldsOrder", |c| c.test_stored_fields_order()))
            .method(t("testBinaryFieldOffsetLength", |c| {
                c.test_binary_field_offset_length()
            }))
            .method(t("testNumericField", |c| c.test_numeric_field()))
            .method(t("testIndexedBit", |c| c.test_indexed_bit()))
            .method(t("testReadSkip", |c| c.test_read_skip()))
            .method(t("testEmptyDocs", |c| c.test_empty_docs()))
            .method(t("testConcurrentReads", |c| c.test_concurrent_reads()))
            .method(t("testWriteReadMerge", |c| c.test_write_read_merge()))
            .method(t("testMergeFilterReader", |c| c.test_merge_filter_reader()))
            .method(t("testBigDocuments", |c| c.test_big_documents()).annotate(Nightly))
            .method(t("testBulkMergeWithDeletes", |c| {
                c.test_bulk_merge_with_deletes()
            }))
            .method(t("testMismatchedFields", |c| c.test_mismatched_fields()))
            .method(t("testRandomStoredFieldsWithIndexSort", |c| {
                c.test_random_stored_fields_with_index_sort()
            }))
            .method(t("testLineFileDocs", |c| c.test_line_file_docs()))
            .build()
    });
    Arc::clone(&CLASS)
}

/// The instance's subclass as a `BaseStoredFieldsFormatTestCase`.
fn stored_fields_case(c: &dyn BaseIndexFileFormatTestCase) -> &dyn BaseStoredFieldsFormatTestCase {
    c.as_stored_fields().unwrap_or_else(|| {
        Throwable::new(
            "java.lang.IllegalArgumentException",
            "object is not an instance of org.apache.lucene.tests.index.BaseStoredFieldsFormatTestCase",
        )
        .throw()
    })
}

// -----------------------------------------------------------------------------
// Helpers
// -----------------------------------------------------------------------------

/// A null result Java would dereference: `NullPointerException`.
fn npe<T>(value: Option<T>) -> T {
    value.unwrap_or_else(|| {
        randomizedtesting::jdk::exception::throw_without_message("java.lang.NullPointerException")
    })
}

/// `random()`.
fn random() -> Arc<dyn Random> {
    LuceneTestCase::random()
}

/// `new Term(field, text)`.
fn term(field: &str, text: &str) -> Term {
    Term::new(field, BytesRef::from_char_sequence(text))
}

/// `new TermQuery(new Term(field, text))`.
fn term_query(field: &str, text: &str) -> Arc<dyn Query> {
    Arc::new(TermQuery::new(term(field, text)))
}

/// `Document.get(name)`: the string value of the first field of that name
/// that has one, or `null`.
fn doc_get(doc: &Document, name: &str) -> Option<String> {
    doc.get_values(name).into_iter().next()
}

/// `doc.getField(name)`, dereferenced as Java does.
fn get_field<'a>(doc: &'a Document, name: &str) -> &'a dyn IndexableField {
    npe(doc.get_field(name))
}

/// `field.numericValue()`, dereferenced as Java does.
fn numeric_value(field: &dyn IndexableField) -> NumericValue {
    npe(field.numeric_value())
}

/// `Number.intValue()`.
fn int_value(n: NumericValue) -> i32 {
    match n {
        NumericValue::Int(v) => v,
        NumericValue::Long(v) => v as i32,
        NumericValue::Float(v) => v as i32,
        NumericValue::Double(v) => v as i32,
    }
}

/// A boxed number as JUnit prints and compares it: `Number.equals` (same
/// class and value; `Float.equals`/`Double.equals` compare raw bits) and
/// `toString()`.
#[derive(Clone, Copy)]
struct JavaNumber(Option<NumericValue>);

impl PartialEq for JavaNumber {
    fn eq(&self, other: &Self) -> bool {
        match (self.0, other.0) {
            (None, None) => true,
            (Some(NumericValue::Int(a)), Some(NumericValue::Int(b))) => a == b,
            (Some(NumericValue::Long(a)), Some(NumericValue::Long(b))) => a == b,
            (Some(NumericValue::Float(a)), Some(NumericValue::Float(b))) => {
                java_float_bits(a) == java_float_bits(b)
            }
            (Some(NumericValue::Double(a)), Some(NumericValue::Double(b))) => {
                java_double_bits(a) == java_double_bits(b)
            }
            _ => false,
        }
    }
}

/// `Float.floatToIntBits` (NaNs collapse to the canonical NaN).
fn java_float_bits(f: f32) -> u32 {
    if f.is_nan() {
        0x7fc0_0000
    } else {
        f.to_bits()
    }
}

/// `Double.doubleToLongBits` (NaNs collapse to the canonical NaN).
fn java_double_bits(d: f64) -> u64 {
    if d.is_nan() {
        0x7ff8_0000_0000_0000
    } else {
        d.to_bits()
    }
}

impl fmt::Display for JavaNumber {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            None => f.write_str("null"),
            Some(n) => write!(f, "{n}"),
        }
    }
}

/// The class of a boxed number, as `Class.toString()` prints it.
#[derive(PartialEq, Clone, Copy)]
struct NumberClass(&'static str);

impl NumberClass {
    fn of(n: NumericValue) -> Self {
        NumberClass(match n {
            NumericValue::Int(_) => "java.lang.Integer",
            NumericValue::Long(_) => "java.lang.Long",
            NumericValue::Float(_) => "java.lang.Float",
            NumericValue::Double(_) => "java.lang.Double",
        })
    }
}

impl fmt::Display for NumberClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "class {}", self.0)
    }
}

/// A `BytesRef` as JUnit compares (`BytesRef.equals`) and prints it, or
/// `null`.
#[derive(PartialEq)]
struct JavaBytes(Option<Vec<u8>>);

impl JavaBytes {
    fn of(b: Option<BytesRef>) -> Self {
        JavaBytes(b.map(|b| b.bytes[b.offset..b.offset + b.length].to_vec()))
    }
}

impl fmt::Display for JavaBytes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.0 {
            None => f.write_str("null"),
            Some(bytes) => write!(f, "{}", BytesRef::new(bytes.clone())),
        }
    }
}

/// JUnit's `assertArrayEquals(byte[] expecteds, byte[] actuals)`.
fn assert_byte_array_equals(expecteds: &[u8], actuals: &[u8]) {
    if expecteds == actuals {
        return;
    }
    let mut header = String::new();
    if expecteds.len() != actuals.len() {
        header = format!(
            "array lengths differed, expected.length={} actual.length={}; ",
            expecteds.len(),
            actuals.len()
        );
    }
    let n = expecteds.len().max(actuals.len());
    for i in 0..n {
        let e = expecteds.get(i);
        let a = actuals.get(i);
        if e != a {
            let render =
                |v: Option<&u8>| v.map_or("end of array".to_string(), |v| (*v as i8).to_string());
            Assert::fail(&format!(
                "{header}arrays first differed at element [{i}]; expected:<{}> but was:<{}>",
                render(e),
                render(a)
            ));
        }
    }
}

/// The order in which a `java.util.HashMap<String, ?>` built with `new
/// HashMap<>()` iterates its keys: `keys` are the keys it holds, in the
/// order they were inserted, and `max_size` the largest size it reached
/// (its table never shrinks). Buckets are visited in index order
/// (`(h ^ h >>> 16) & (n - 1)`) and the keys of a bucket in insertion order, which a
/// resize keeps; a treeified bucket (eight or more colliding keys in a table
/// of at least 64) is not reproduced.
pub fn java_hash_map_key_order(keys: &[String], max_size: usize) -> Vec<String> {
    let mut table = 16usize;
    while max_size > table * 3 / 4 {
        table *= 2;
    }
    let mut keyed: Vec<(usize, usize, &String)> = keys
        .iter()
        .enumerate()
        .map(|(rank, k)| {
            let h = randomizedtesting::jdk::lang::string_hash_code(k);
            let spread = h ^ ((h as u32) >> 16) as i32;
            ((spread as u32 as usize) & (table - 1), rank, k)
        })
        .collect();
    keyed.sort();
    keyed.into_iter().map(|(_, _, k)| k.clone()).collect()
}

/// A `HashMap<String, V>` that remembers what `java.util.HashMap` needs to
/// iterate its keys in Java's order: the insertion order and the largest
/// size reached.
struct JavaKeyedMap<V> {
    map: HashMap<String, V>,
    inserted: Vec<String>,
    max_size: usize,
}

impl<V> JavaKeyedMap<V> {
    fn new() -> Self {
        JavaKeyedMap {
            map: HashMap::new(),
            inserted: Vec::new(),
            max_size: 0,
        }
    }

    /// `put(key, value)`: a new key goes to the end of its bucket; an
    /// existing one keeps its place.
    fn put(&mut self, key: String, value: V) {
        if self.map.insert(key.clone(), value).is_none() {
            self.inserted.push(key);
            self.max_size = self.max_size.max(self.map.len());
        }
    }

    /// `remove(key)`.
    fn remove(&mut self, key: &str) {
        if self.map.remove(key).is_some() {
            self.inserted.retain(|k| k != key);
        }
    }

    fn get(&self, key: &str) -> Option<&V> {
        self.map.get(key)
    }

    fn len(&self) -> usize {
        self.map.len()
    }

    /// `keySet()` in Java's iteration order.
    fn key_set(&self) -> Vec<String> {
        java_hash_map_key_order(&self.inserted, self.max_size)
    }
}

/// `new MockAnalyzer(random())`.
fn mock_analyzer() -> Arc<dyn Analyzer> {
    Arc::new(MockAnalyzer::new(&*random()))
}

/// The stored field values a document holds, by name, in document order:
/// what the tests compare a read document with.
type StoredValues = Vec<(String, Option<String>)>;

/// The stored fields of `doc`: name and string value, in order.
fn stored_values(doc: &Document) -> StoredValues {
    doc.get_fields()
        .iter()
        .filter(|f| f.field_type().stored())
        .map(|f| (f.name().to_string(), f.string_value()))
        .collect()
}

// -----------------------------------------------------------------------------
// The members
// -----------------------------------------------------------------------------

/// Port of the members of `BaseStoredFieldsFormatTestCase`.
pub trait BaseStoredFieldsFormatTestCase: BaseIndexFileFormatTestCase {
    /// `addRandomFields(Document)`, the override every stored-fields suite
    /// inherits; a subclass's [`BaseIndexFileFormatTestCase::add_random_fields`]
    /// delegates here.
    fn stored_fields_add_random_fields(&self, d: &mut Document) {
        let num_values = random().next_int_bound(3);
        for _ in 0..num_values {
            let value = TestUtil::random_simple_string_max(&*random(), 100);
            d.add(Box::new(
                StoredField::new_string("f", value).unwrap_or_else(|e| rethrow(e)),
            ));
        }
    }

    /// `testRandomStoredFields()`.
    fn test_random_stored_fields(&self) -> Result<()> {
        let mut dir = LuceneTestCase::new_directory();
        let rand = random();
        let iwc = LuceneTestCase::new_index_writer_config_analyzer(Some(mock_analyzer()));
        iwc.set_max_buffered_docs(TestUtil::next_int(&*rand, 5, 20))?;
        let w = RandomIndexWriter::new_with_config(&*rand, dir.as_directory(), iwc)?;
        // w.w.setNoCFSRatio(0.0);
        let doc_count = LuceneTestCase::at_least(200);
        let field_count = TestUtil::next_int(&*rand, 1, 5);

        let mut field_ids: Vec<i32> = Vec::new();

        let custom_type = FieldType::new_from(TextField::type_stored());
        custom_type.set_tokenized(false)?;
        let id_field = Arc::new(LuceneTestCase::new_field("id", "", &custom_type));

        for i in 0..field_count {
            field_ids.push(i);
        }

        // The expected value of each `f<i>` (`docExp.get("f" + i)`), by id.
        let mut docs: JavaKeyedMap<Vec<Option<String>>> = JavaKeyedMap::new();

        if *VERBOSE {
            println!("TEST: build index docCount={doc_count}");
        }

        let custom_type2 = FieldType::new();
        custom_type2.set_stored(true)?;
        for i in 0..doc_count {
            let mut doc = Document::new();
            let id = i.to_string();
            doc.add_shared(Arc::clone(&id_field) as Arc<dyn IndexableField>);
            id_field.set_string_value(id.clone())?;
            if *VERBOSE {
                println!("TEST: add doc id={id}");
            }

            let mut expected: Vec<Option<String>> = vec![None; field_count as usize];
            for &field in &field_ids {
                if rand.next_int_bound(4) != 3 {
                    let s = TestUtil::random_unicode_string_max(&*rand, 1000);
                    doc.add(Box::new(LuceneTestCase::new_field(
                        &format!("f{field}"),
                        &s,
                        &custom_type2,
                    )));
                    expected[field as usize] = Some(s);
                }
            }
            docs.put(id, expected);
            w.add_document(&doc)?;
            if rand.next_int_bound(50) == 17 {
                // mixup binding of field name -> Number every so often
                randomizedtesting::jdk::util::shuffle(&mut field_ids, &*random());
            }
            if rand.next_int_bound(5) == 3 && i > 0 {
                let del_id = rand.next_int_bound(i).to_string();
                if *VERBOSE {
                    println!("TEST: delete doc id={del_id}");
                }
                w.delete_documents(term("id", &del_id))?;
                docs.remove(&del_id);
            }
        }

        if *VERBOSE {
            println!("TEST: {} docs in index; now load fields", docs.len());
        }
        if docs.len() > 0 {
            let ids_list = docs.key_set();

            for x in 0..2 {
                let r = self.maybe_wrap_with_merging_reader(w.get_reader()?)?;
                let s = LuceneTestCase::new_searcher(Arc::clone(&r));
                let stored_fields = r.stored_fields()?;

                if *VERBOSE {
                    println!("TEST: cycle x={x} r={r}");
                }

                let num = LuceneTestCase::at_least(100);
                for _ in 0..num {
                    let test_id = &ids_list[rand.next_int_bound(ids_list.len() as i32) as usize];
                    if *VERBOSE {
                        println!("TEST: test id={test_id}");
                    }
                    let hits = s.search(term_query("id", test_id), 1)?;
                    Assert::assert_equals(1i64, hits.total_hits.value());
                    let doc = stored_fields.document(hits.score_docs[0].doc)?;
                    let doc_exp = npe(docs.get(test_id));
                    for i in 0..field_count {
                        Assert::assert_equals_str(
                            Some(&format!("doc {test_id}, field f{field_count} is wrong")),
                            doc_exp[i as usize].as_deref(),
                            doc_get(&doc, &format!("f{i}")).as_deref(),
                        );
                    }
                }
                drop((s, stored_fields));
                r.close()?;
                drop(r);
                w.force_merge(1)?;
            }
        }
        w.close()?;
        drop(w);
        dir.close()?;
        Ok(())
    }

    /// `testStoredFieldsOrder()`: LUCENE-1727, make sure doc fields are
    /// stored in order.
    fn test_stored_fields_order(&self) -> Result<()> {
        let mut d = LuceneTestCase::new_directory();
        let w = IndexWriter::new(
            d.as_directory(),
            LuceneTestCase::new_index_writer_config_analyzer(Some(mock_analyzer())),
        )?;
        let mut doc = Document::new();

        let custom_type = FieldType::new();
        custom_type.set_stored(true)?;
        doc.add(Box::new(LuceneTestCase::new_field(
            "zzz",
            "a b c",
            &custom_type,
        )));
        doc.add(Box::new(LuceneTestCase::new_field(
            "aaa",
            "a b c",
            &custom_type,
        )));
        doc.add(Box::new(LuceneTestCase::new_field(
            "zzz",
            "1 2 3",
            &custom_type,
        )));
        w.add_document(&doc)?;
        let r = self.maybe_wrap_with_merging_reader(directory_reader::open_with_writer(&*w)?)?;
        let stored_fields = r.stored_fields()?;
        let doc2 = stored_fields.document(0)?;
        let mut it = doc2.get_fields().iter();
        for (name, value) in [("zzz", "a b c"), ("aaa", "a b c"), ("zzz", "1 2 3")] {
            let f = it.next();
            Assert::assert_true(f.is_some());
            let f = npe(f);
            Assert::assert_equals_str(None, Some(f.name()), Some(name));
            Assert::assert_equals_str(None, f.string_value().as_deref(), Some(value));
        }
        Assert::assert_false(it.next().is_some());
        drop(stored_fields);
        r.close()?;
        drop(r);
        w.close()?;
        drop(w);
        d.close()?;
        Ok(())
    }

    /// `testBinaryFieldOffsetLength()`: LUCENE-1219.
    fn test_binary_field_offset_length(&self) -> Result<()> {
        let mut dir = LuceneTestCase::new_directory();
        let w = IndexWriter::new(
            dir.as_directory(),
            LuceneTestCase::new_index_writer_config_analyzer(Some(mock_analyzer())),
        )?;
        let mut b = vec![0u8; 50];
        for (i, byte) in b.iter_mut().enumerate() {
            *byte = (i + 77) as u8;
        }

        let mut doc = Document::new();
        let f = StoredField::new_bytes(
            "binary",
            BytesRef {
                bytes: b,
                offset: 10,
                length: 17,
            },
        )?;
        let bx = npe(f.binary_value());
        Assert::assert_true(!bx.bytes.is_empty());
        Assert::assert_equals(50, bx.bytes.len());
        Assert::assert_equals(10, bx.offset);
        Assert::assert_equals(17, bx.length);
        doc.add(Box::new(f));
        w.add_document(&doc)?;
        w.close()?;
        drop(w);

        let ir = directory_reader::open(dir.as_directory())?;
        let stored_fields = ir.stored_fields()?;
        let doc2 = stored_fields.document(0)?;
        let f2 = get_field(&doc2, "binary");
        let b = npe(f2.binary_value());
        let b = &b.bytes[b.offset..b.offset + b.length];
        Assert::assert_true(!b.is_empty());
        // assertEquals(17, b.length, 17): the delta overload, as doubles.
        Assert::assert_equals_double_msg(None, 17.0, b.len() as f64, 17.0);
        Assert::assert_equals(87i8, b[0] as i8);
        drop(stored_fields);
        ir.close()?;
        drop(ir);
        dir.close()?;
        Ok(())
    }

    /// `testNumericField()`.
    fn test_numeric_field(&self) -> Result<()> {
        let mut dir = LuceneTestCase::new_directory();
        let w = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
        let num_docs = LuceneTestCase::at_least(500);
        let mut answers: Vec<NumericValue> = Vec::with_capacity(num_docs as usize);
        for id in 0..num_docs {
            let mut doc = Document::new();
            let answer = if random().next_boolean() {
                // float/double
                if random().next_boolean() {
                    NumericValue::Float(random().next_float())
                } else {
                    NumericValue::Double(random().next_double())
                }
            } else {
                // int/long
                if random().next_boolean() {
                    NumericValue::Int(random().next_int())
                } else {
                    NumericValue::Long(random().next_long())
                }
            };
            doc.add(Box::new(StoredField::new_number("nf", answer)?));
            answers.push(answer);
            doc.add(Box::new(StoredField::new_number(
                "id",
                NumericValue::Int(id),
            )?));
            doc.add(Box::new(IntPoint::new("id", &[id])?));
            doc.add(Box::new(NumericDocValuesField::new("id", i64::from(id))));
            w.add_document(&doc)?;
        }
        let r = self.maybe_wrap_with_merging_reader(w.get_reader()?)?;
        w.close()?;
        drop(w);

        Assert::assert_equals(num_docs, r.num_docs());

        for ctx in Arc::clone(&r).leaves()? {
            let sub = ctx.leaf_reader();
            let mut ids = rucene::index::doc_values::DocValues::get_numeric(&*sub, "id")?;
            let stored_fields = sub.stored_fields()?;
            for doc_id in 0..sub.num_docs() {
                let doc = stored_fields.document(doc_id)?;
                let f = get_field(&doc, "nf");
                Assert::assert_true_msg(
                    &format!("got f={}", JavaNumber(f.numeric_value())),
                    f.as_any().is_some_and(|a| a.is::<StoredField>()),
                );
                Assert::assert_equals(doc_id, ids.next_doc()?);
                let answer = answers[ids.long_value()? as usize];
                Assert::assert_equals(JavaNumber(Some(answer)), JavaNumber(f.numeric_value()));
                Assert::assert_equals(NumberClass::of(answer), NumberClass::of(numeric_value(f)));
            }
        }
        r.close()?;
        drop(r);
        dir.close()?;
        Ok(())
    }

    /// `testIndexedBit()`.
    fn test_indexed_bit(&self) -> Result<()> {
        let mut dir = LuceneTestCase::new_directory();
        let w = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
        let mut doc = Document::new();
        let only_stored = FieldType::new();
        only_stored.set_stored(true)?;
        doc.add(Box::new(Field::new(
            "field",
            "value".to_string(),
            only_stored,
        )?));
        doc.add(Box::new(StringField::new(
            "field2",
            "value".to_string(),
            Store::YES,
        )?));
        w.add_document(&doc)?;
        let r = self.maybe_wrap_with_merging_reader(w.get_reader()?)?;
        let stored_fields = r.stored_fields()?;
        w.close()?;
        drop(w);
        let options = get_field(&stored_fields.document(0)?, "field")
            .field_type()
            .index_options();
        Assert::assert_true(options == IndexOptions::NONE);
        // assertNotNull(...indexOptions()): an enum value is never null.
        let _ = get_field(&stored_fields.document(0)?, "field2")
            .field_type()
            .index_options();
        drop(stored_fields);
        r.close()?;
        drop(r);
        dir.close()?;
        Ok(())
    }

    /// `testReadSkip()`.
    fn test_read_skip(&self) -> Result<()> {
        let mut dir = LuceneTestCase::new_directory();
        let iw_conf = LuceneTestCase::new_index_writer_config_analyzer(Some(mock_analyzer()));
        iw_conf.set_max_buffered_docs(RandomNumbers::random_int_between(&*random(), 2, 30))?;
        let iw = RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), iw_conf)?;

        let ft = FieldType::new();
        ft.set_stored(true)?;
        ft.freeze();

        let string = TestUtil::random_simple_string_max(&*random(), 50);
        let bytes = string.as_bytes().to_vec();
        let l = if random().next_boolean() {
            i64::from(random().next_int_bound(42))
        } else {
            random().next_long()
        };
        let i = if random().next_boolean() {
            random().next_int_bound(42)
        } else {
            random().next_int()
        };
        let f = random().next_float();
        let d = random().next_double();

        let fields: Vec<Arc<dyn IndexableField>> = vec![
            Arc::new(Field::new_with_bytes(
                "bytes",
                BytesRef::new(bytes),
                ft.clone(),
            )?),
            Arc::new(Field::new("string", string, ft.clone())?),
            Arc::new(StoredField::new_number("long", NumericValue::Long(l))?),
            Arc::new(StoredField::new_number("int", NumericValue::Int(i))?),
            Arc::new(StoredField::new_number("float", NumericValue::Float(f))?),
            Arc::new(StoredField::new_number("double", NumericValue::Double(d))?),
        ];

        for _ in 0..100 {
            let mut doc = Document::new();
            for fld in &fields {
                doc.add_shared(Arc::clone(fld));
            }
            iw.w().add_document(&doc)?;
        }
        iw.commit()?;

        let reader =
            self.maybe_wrap_with_merging_reader(directory_reader::open(dir.as_directory())?)?;
        let stored_fields = reader.stored_fields()?;
        let doc_id = random().next_int_bound(100);
        for fld in &fields {
            let fld_name = fld.name();
            let s_doc =
                stored_fields.document_fields(doc_id, &HashSet::from([fld_name.to_string()]))?;
            let s_field = get_field(&s_doc, fld_name);
            // `Field.class.equals(fld.getClass())`
            if fld.as_any().is_some_and(|any| any.is::<Field>()) {
                Assert::assert_equals(
                    JavaBytes::of(fld.binary_value()),
                    JavaBytes::of(s_field.binary_value()),
                );
                Assert::assert_equals_str(
                    None,
                    fld.string_value().as_deref(),
                    s_field.string_value().as_deref(),
                );
            } else {
                Assert::assert_equals(
                    JavaNumber(fld.numeric_value()),
                    JavaNumber(s_field.numeric_value()),
                );
            }
        }
        drop(stored_fields);
        reader.close()?;
        drop(reader);
        iw.close()?;
        drop(iw);
        dir.close()?;
        Ok(())
    }

    /// `testEmptyDocs()`.
    fn test_empty_docs(&self) -> Result<()> {
        let mut dir = LuceneTestCase::new_directory();
        let iw_conf = LuceneTestCase::new_index_writer_config_analyzer(Some(mock_analyzer()));
        iw_conf.set_max_buffered_docs(RandomNumbers::random_int_between(&*random(), 2, 30))?;
        let iw = RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), iw_conf)?;

        // make sure that the fact that documents might be empty is not a problem
        let empty_doc = Document::new();
        let num_docs = if random().next_boolean() {
            1
        } else {
            LuceneTestCase::at_least(1000)
        };
        for _ in 0..num_docs {
            iw.add_document(&empty_doc)?;
        }
        iw.commit()?;
        let rd =
            self.maybe_wrap_with_merging_reader(directory_reader::open(dir.as_directory())?)?;
        let stored_fields = rd.stored_fields()?;
        for i in 0..num_docs {
            let doc = stored_fields.document(i)?;
            Assert::assert_true(doc.get_fields().is_empty());
        }
        drop(stored_fields);
        rd.close()?;
        drop(rd);

        iw.close()?;
        drop(iw);
        dir.close()?;
        Ok(())
    }

    /// `testConcurrentReads()`.
    fn test_concurrent_reads(&self) -> Result<()> {
        let mut dir = LuceneTestCase::new_directory();
        let iw_conf = LuceneTestCase::new_index_writer_config_analyzer(Some(mock_analyzer()));
        iw_conf.set_max_buffered_docs(RandomNumbers::random_int_between(&*random(), 2, 30))?;
        let iw = RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), iw_conf)?;

        // make sure the readers are properly cloned
        let mut doc = Document::new();
        let field = StringField::new("fld", String::new(), Store::YES)?;
        doc.add(Box::new(field));
        let num_docs = LuceneTestCase::at_least(1000);
        for i in 0..num_docs {
            doc.get_field_mut_as::<StringField>("fld")
                .expect("field")
                .set_string_value(i.to_string())?;
            iw.add_document(&doc)?;
        }
        iw.commit()?;

        let rd =
            self.maybe_wrap_with_merging_reader(directory_reader::open(dir.as_directory())?)?;
        let searcher = Arc::new(IndexSearcher::new(Arc::clone(&rd) as Arc<dyn IndexReader>)?);
        let concurrent_reads = LuceneTestCase::at_least(5);
        let reads_per_thread = LuceneTestCase::at_least(50);
        let mut read_threads: Vec<Thread> = Vec::new();
        // `AtomicReference<Exception> ex`: the first exception a thread
        // caught.
        let ex: Arc<Mutex<Option<LuceneError>>> = Arc::new(Mutex::new(None));
        for _ in 0..concurrent_reads {
            // The instance initializer, on this thread.
            let queries: Vec<i32> = (0..reads_per_thread)
                .map(|_| random().next_int_bound(num_docs))
                .collect();
            let (rd, searcher, ex) = (Arc::clone(&rd), Arc::clone(&searcher), Arc::clone(&ex));
            read_threads.push(Thread::unnamed(move || {
                for q in queries {
                    let query = term_query("fld", &q.to_string());
                    let outcome = (|| -> Result<()> {
                        let stored_fields = rd.stored_fields()?;
                        let top_docs = searcher.search(query, 1)?;
                        if top_docs.total_hits.value() != 1 {
                            return Err(LuceneError::illegal_state(format!(
                                "Expected 1 hit, got {}",
                                top_docs.total_hits.value()
                            )));
                        }
                        let sdoc = stored_fields.document(top_docs.score_docs[0].doc)?;
                        let Some(fld) = doc_get(&sdoc, "fld") else {
                            return Err(LuceneError::illegal_state(format!(
                                "Could not find document {q}"
                            )));
                        };
                        if q.to_string() != fld {
                            return Err(LuceneError::illegal_state(format!(
                                "Expected {q}, but got {fld}"
                            )));
                        }
                        Ok(())
                    })();
                    if let Err(e) = outcome {
                        let mut slot = ex.lock().unwrap_or_else(PoisonError::into_inner);
                        if slot.is_none() {
                            *slot = Some(e);
                        }
                    }
                }
            }));
        }
        for thread in &read_threads {
            thread.start();
        }
        for thread in &read_threads {
            if let Err(payload) = thread.join() {
                std::panic::resume_unwind(payload);
            }
        }
        drop(searcher);
        rd.close()?;
        drop(rd);
        if let Some(e) = ex.lock().unwrap_or_else(PoisonError::into_inner).take() {
            return Err(e);
        }

        iw.close()?;
        drop(iw);
        dir.close()?;
        Ok(())
    }

    /// `randomByteArray(length, max)` (`private`).
    fn random_byte_array(&self, length: i32, max: i32) -> Vec<u8> {
        (0..length)
            .map(|_| random().next_int_bound(max) as u8)
            .collect()
    }

    /// `testWriteReadMerge()`.
    fn test_write_read_merge(&self) -> Result<()> {
        // get another codec, other than the default: so we are merging segments across different codecs
        let default_codec = npe(rucene::codecs::get_default());
        let other_codec: Arc<dyn Codec> = if default_codec.name() == "SimpleText" {
            TestUtil::get_default_codec()
        } else {
            Arc::new(rucene_codecs::simpletext::SimpleTextCodec::new())
        };
        let mut dir = LuceneTestCase::new_directory();
        let iw_conf = LuceneTestCase::new_index_writer_config_analyzer(Some(mock_analyzer()));
        iw_conf.set_max_buffered_docs(RandomNumbers::random_int_between(&*random(), 2, 30))?;
        let mut iw_codec = iw_conf.codec();
        let mut iw = RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), iw_conf)?;

        let doc_count = LuceneTestCase::at_least(200);
        let mut data: Vec<Vec<Vec<u8>>> = Vec::with_capacity(doc_count as usize);
        for _ in 0..doc_count {
            let field_count = if LuceneTestCase::rarely() {
                RandomNumbers::random_int_between(&*random(), 1, 500)
            } else {
                RandomNumbers::random_int_between(&*random(), 1, 5)
            };
            let mut fields = Vec::with_capacity(field_count as usize);
            for _ in 0..field_count {
                let length = if LuceneTestCase::rarely() {
                    random().next_int_bound(1000)
                } else {
                    random().next_int_bound(10)
                };
                let max = if LuceneTestCase::rarely() { 256 } else { 2 };
                fields.push(self.random_byte_array(length, max));
            }
            data.push(fields);
        }

        let ty = FieldType::new_from(StringField::type_stored());
        ty.set_index_options(IndexOptions::NONE)?;
        ty.freeze();
        let id = Arc::new(IntPoint::new("id", &[0])?);
        let id_stored = Arc::new(StoredField::new_number("id", NumericValue::Int(0))?);
        for (i, fields) in data.iter().enumerate() {
            let mut doc = Document::new();
            doc.add_shared(Arc::clone(&id) as Arc<dyn IndexableField>);
            doc.add_shared(Arc::clone(&id_stored) as Arc<dyn IndexableField>);
            id.set_int_value(i as i32)?;
            id_stored.set_int_value(i as i32)?;
            for (j, bytes) in fields.iter().enumerate() {
                doc.add(Box::new(Field::new_with_bytes(
                    &format!("bytes{j}"),
                    BytesRef::new(bytes.clone()),
                    ty.clone(),
                )?));
            }
            iw.w().add_document(&doc)?;
            if random().next_boolean() && (i % (data.len() / 10) == 0) {
                iw.w().close()?;
                let iw_conf_new =
                    LuceneTestCase::new_index_writer_config_analyzer(Some(mock_analyzer()));
                // test merging against a non-compressing codec
                if std::ptr::addr_eq(Arc::as_ptr(&iw_codec), Arc::as_ptr(&other_codec)) {
                    iw_conf_new.set_codec(npe(rucene::codecs::get_default()))?;
                } else {
                    iw_conf_new.set_codec(Arc::clone(&other_codec))?;
                }
                iw_codec = iw_conf_new.codec();
                iw = RandomIndexWriter::new_with_config(
                    &*random(),
                    dir.as_directory(),
                    iw_conf_new,
                )?;
            }
        }

        for _ in 0..10 {
            let min = random().next_int_bound(data.len() as i32);
            let max = min + random().next_int_bound(20);
            iw.delete_documents_by_query(IntPoint::new_range_query("id", &[min], &[max - 1])?)?;
        }

        iw.force_merge(2)?; // force merges with deletions

        iw.commit()?;

        let ir =
            self.maybe_wrap_with_merging_reader(directory_reader::open(dir.as_directory())?)?;
        let stored_fields = ir.stored_fields()?;
        Assert::assert_true(ir.num_docs() > 0);
        let mut num_docs = 0;
        for i in 0..ir.max_doc() {
            let doc = stored_fields.document(i)?;
            num_docs += 1;
            let doc_id = int_value(numeric_value(get_field(&doc, "id"))) as usize;
            Assert::assert_equals(data[doc_id].len() + 1, doc.get_fields().len());
            for (j, arr) in data[doc_id].iter().enumerate() {
                let arr2_ref = npe(doc.get_binary_value(&format!("bytes{j}")));
                let arr2 = &arr2_ref.bytes[arr2_ref.offset..arr2_ref.offset + arr2_ref.length];
                assert_byte_array_equals(arr, arr2);
            }
        }
        Assert::assert_true(ir.num_docs() <= num_docs);
        drop(stored_fields);
        ir.close()?;
        drop(ir);

        iw.delete_all()?;
        iw.commit()?;
        iw.force_merge(1)?;

        iw.close()?;
        drop(iw);
        dir.close()?;
        Ok(())
    }

    /// `testMergeFilterReader()`.
    fn test_merge_filter_reader(&self) -> Result<()> {
        let mut dir = LuceneTestCase::new_directory();
        let w = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
        let num_docs = LuceneTestCase::at_least(200);
        let string_values: Vec<String> = (0..10)
            .map(|_| RandomStrings::random_realistic_unicode_of_length(&*random(), 10))
            .collect();
        let mut docs: Vec<MergeFilterExpected> = Vec::with_capacity(num_docs as usize);
        for i in 0..num_docs {
            let mut doc = Document::new();
            let to_delete = if random().next_boolean() { "yes" } else { "no" };
            doc.add(Box::new(StringField::new(
                "to_delete",
                to_delete.to_string(),
                Store::NO,
            )?));
            doc.add(Box::new(StoredField::new_number(
                "id",
                NumericValue::Int(i),
            )?));
            let iv = NumericValue::Int(random().next_int_bound(50));
            doc.add(Box::new(StoredField::new_number("i", iv)?));
            let lv = NumericValue::Long(random().next_long());
            doc.add(Box::new(StoredField::new_number("l", lv)?));
            let dv = NumericValue::Double(random().next_double());
            doc.add(Box::new(StoredField::new_number("d", dv)?));
            let fv = NumericValue::Float(random().next_float());
            doc.add(Box::new(StoredField::new_number("f", fv)?));
            let s = RandomPicks::random_from(&*random(), &string_values).clone();
            doc.add(Box::new(StoredField::new_string("s", s.clone())?));
            let value =
                BytesRef::from_char_sequence(RandomPicks::random_from(&*random(), &string_values));
            doc.add(Box::new(StoredField::new_bytes("b", value.clone())?));
            let bytes = value.bytes[value.offset..value.offset + value.length].to_vec();
            let length = bytes.len() as i32;
            doc.add(Box::new(StoredField::new_stored_input(
                "b2",
                Some(Box::new(rucene::store::InputStreamDataInput::new(
                    std::io::Cursor::new(bytes.clone()),
                ))),
                length,
            )?));
            docs.push(MergeFilterExpected {
                s,
                i: iv,
                l: lv,
                d: dv,
                f: fv,
                b: bytes,
            });
            w.add_document(&doc)?;
        }
        if random().next_boolean() {
            w.delete_documents(term("to_delete", "yes"))?;
        }
        w.commit()?;
        w.close()?;
        drop(w);

        let reader: Arc<dyn DirectoryReader> = Arc::new(DummyFilterDirectoryReader::new(
            self.maybe_wrap_with_merging_reader(directory_reader::open(dir.as_directory())?)?,
        )?);

        let mut dir2 = LuceneTestCase::new_directory();
        let w = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir2.as_directory())?;
        TestUtil::add_indexes_slowly(w.w(), std::slice::from_ref(&reader))?;
        reader.close()?;
        drop(reader);
        dir.close()?;

        let reader = self.maybe_wrap_with_merging_reader(w.get_reader()?)?;
        let stored_fields = reader.stored_fields()?;
        for i in 0..reader.max_doc() {
            let doc = stored_fields.document(i)?;
            let id = int_value(numeric_value(get_field(&doc, "id")));
            let expected = &docs[id as usize];
            Assert::assert_equals_str(None, Some(&expected.s), doc_get(&doc, "s").as_deref());
            for (name, value) in [
                ("i", expected.i),
                ("l", expected.l),
                ("d", expected.d),
                ("f", expected.f),
            ] {
                Assert::assert_equals(
                    JavaNumber(Some(value)),
                    JavaNumber(get_field(&doc, name).numeric_value()),
                );
            }
            Assert::assert_equals(
                JavaBytes(Some(expected.b.clone())),
                JavaBytes::of(get_field(&doc, "b").binary_value()),
            );
            // The value is the same for fields "b" and "b2". Read the expected value from "b" as "b2" was
            // consumed during indexing
            Assert::assert_equals(
                JavaBytes(Some(expected.b.clone())),
                JavaBytes::of(get_field(&doc, "b2").binary_value()),
            );
        }

        drop(stored_fields);
        reader.close()?;
        drop(reader);
        w.close()?;
        drop(w);
        TestUtil::check_index(dir2.as_directory())?;
        dir2.close()?;
        Ok(())
    }

    /// `testBigDocuments()` (`@Nightly`): "big" as "much bigger than the
    /// chunk size".
    fn test_big_documents(&self) -> Result<()> {
        // for this test we force an FS dir
        // we can't just use newFSDirectory, because this test doesn't really index anything.
        // so if we get NRTCachingDir+SimpleText, we make massive stored fields and OOM (LUCENE-4484)
        let mmap = rucene::store::mmap::MMapDirectory::open(
            LuceneTestCase::create_temp_dir_prefix("testBigDocuments"),
        )?;
        let mut dir = MockDirectoryWrapper::new(&*random(), Arc::new(mmap));
        let iw_conf = LuceneTestCase::new_index_writer_config_analyzer(Some(mock_analyzer()));
        iw_conf.set_max_buffered_docs(RandomNumbers::random_int_between(&*random(), 2, 30))?;
        let iw = RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), iw_conf)?;

        dir.set_throttling(Throttling::Never);

        let mut empty_doc = Document::new(); // emptyDoc
        let mut big_doc1 = Document::new(); // lot of small fields
        let mut big_doc2 = Document::new(); // 1 very big field

        let id_field = Arc::new(StringField::new("id", String::new(), Store::NO)?);
        empty_doc.add_shared(Arc::clone(&id_field) as Arc<dyn IndexableField>);
        big_doc1.add_shared(Arc::clone(&id_field) as Arc<dyn IndexableField>);
        big_doc2.add_shared(Arc::clone(&id_field) as Arc<dyn IndexableField>);

        let only_stored = FieldType::new_from(StringField::type_stored());
        only_stored.set_index_options(IndexOptions::NONE)?;

        let small_field: Arc<dyn IndexableField> = Arc::new(Field::new_with_bytes(
            "fld",
            BytesRef::new(self.random_byte_array(random().next_int_bound(10), 256)),
            only_stored.clone(),
        )?);
        let num_fields = RandomNumbers::random_int_between(&*random(), 500_000, 1_000_000);
        for _ in 0..num_fields {
            big_doc1.add_shared(Arc::clone(&small_field));
        }

        let big_field = Field::new_with_bytes(
            "fld",
            BytesRef::new(self.random_byte_array(
                RandomNumbers::random_int_between(&*random(), 1_000_000, 5_000_000),
                2,
            )),
            only_stored,
        )?;
        big_doc2.add(Box::new(big_field));

        let num_docs = LuceneTestCase::at_least(5);
        let choices = [&empty_doc, &big_doc1, &big_doc2];
        let docs: Vec<&Document> = (0..num_docs)
            .map(|_| *RandomPicks::random_from_list(&*random(), &choices))
            .collect();
        for (i, doc) in docs.iter().enumerate() {
            id_field.set_string_value(i.to_string())?;
            iw.add_document(*doc)?;
            if random().next_int_bound(num_docs) == 0 {
                iw.commit()?;
            }
        }
        iw.commit()?;
        iw.force_merge(1)?; // look at what happens when big docs are merged
        let rd =
            self.maybe_wrap_with_merging_reader(directory_reader::open(dir.as_directory())?)?;
        let searcher = IndexSearcher::new(Arc::clone(&rd) as Arc<dyn IndexReader>)?;
        let stored_fields = rd.stored_fields()?;
        for (i, expected_doc) in docs.iter().enumerate() {
            let query = term_query("id", &i.to_string());
            let top_docs = searcher.search(query, 1)?;
            Assert::assert_equals_msg(Some(&i.to_string()), 1i64, top_docs.total_hits.value());
            let doc = stored_fields.document(top_docs.score_docs[0].doc)?;
            let field_values = doc.get_fields_by_name("fld");
            let expected_values = expected_doc.get_fields_by_name("fld");
            Assert::assert_equals(expected_values.len(), field_values.len());
            if !field_values.is_empty() {
                Assert::assert_equals(
                    JavaBytes::of(expected_values[0].binary_value()),
                    JavaBytes::of(field_values[0].binary_value()),
                );
            }
        }
        drop((searcher, stored_fields));
        rd.close()?;
        drop(rd);
        iw.close()?;
        drop(iw);
        dir.close()?;
        Ok(())
    }

    /// `testBulkMergeWithDeletes()`.
    fn test_bulk_merge_with_deletes(&self) -> Result<()> {
        let num_docs = LuceneTestCase::at_least(200);
        let mut dir = LuceneTestCase::new_directory();
        let iwc = LuceneTestCase::new_index_writer_config_analyzer(Some(mock_analyzer()));
        iwc.set_merge_policy(Some(NoMergePolicy::instance()))?;
        let w = RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), iwc)?;
        for i in 0..num_docs {
            let mut doc = Document::new();
            doc.add(Box::new(StringField::new("id", i.to_string(), Store::YES)?));
            doc.add(Box::new(StoredField::new_string(
                "f",
                TestUtil::random_simple_string(&*random()),
            )?));
            w.add_document(&doc)?;
        }
        let delete_count = TestUtil::next_int(&*random(), 5, num_docs);
        for _ in 0..delete_count {
            let id = random().next_int_bound(num_docs);
            w.delete_documents(term("id", &id.to_string()))?;
        }
        w.commit()?;
        w.close()?;
        drop(w);
        let w = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
        w.force_merge(TestUtil::next_int(&*random(), 1, 3))?;
        w.commit()?;
        w.close()?;
        drop(w);
        TestUtil::check_index(dir.as_directory())?;
        dir.close()?;
        Ok(())
    }

    /// `testMismatchedFields()`: mix up field numbers, merge, and check that
    /// data is correct.
    fn test_mismatched_fields(&self) -> Result<()> {
        let mut dirs = Vec::with_capacity(10);
        for _ in 0..10 {
            let mut dir = LuceneTestCase::new_directory();
            let iwc = IndexWriterConfig::with_analyzer(None);
            let iw = IndexWriter::new(dir.as_directory(), iwc)?;
            let mut doc = Document::new();
            for j in 0..10 {
                // add fields where name=value (e.g. 3=3) so we can detect if stuff gets screwed up.
                doc.add(Box::new(StringField::new(
                    &j.to_string(),
                    j.to_string(),
                    Store::YES,
                )?));
            }
            for _ in 0..10 {
                iw.add_document(&doc)?;
            }

            let mut reader =
                self.maybe_wrap_with_merging_reader(directory_reader::open_with_writer(&*iw)?)?;
            // mix up fields explicitly
            if random().next_boolean() {
                reader = Arc::new(MismatchedDirectoryReader::new(reader, random())?);
            }
            let added = LuceneTestCase::new_directory();
            let adder =
                IndexWriter::new(added.as_directory(), IndexWriterConfig::with_analyzer(None))?;
            TestUtil::add_indexes_slowly(&adder, std::slice::from_ref(&reader))?;
            adder.commit()?;
            adder.close()?;
            drop(adder);
            dirs.push(added);

            // IOUtils.close(reader, iw, dir)
            reader.close()?;
            drop(reader);
            iw.close()?;
            drop(iw);
            dir.close()?;
        }

        let mut everything = LuceneTestCase::new_directory();
        let iw = IndexWriter::new(
            everything.as_directory(),
            IndexWriterConfig::with_analyzer(None),
        )?;
        let dir_handles: Vec<Arc<dyn Directory>> = dirs.iter().map(|d| d.as_directory()).collect();
        iw.add_indexes_dirs(&dir_handles)?;
        drop(dir_handles);
        iw.force_merge(1)?;

        let ir =
            LuceneTestCase::get_only_leaf_reader(directory_reader::open_with_writer(&*iw)? as _)?;
        let stored_fields = ir.stored_fields()?;
        for i in 0..ir.max_doc() {
            let doc = stored_fields.document(i)?;
            Assert::assert_equals(10, doc.get_fields().len());
            for j in 0..10 {
                Assert::assert_equals_str(
                    None,
                    Some(&j.to_string()),
                    doc_get(&doc, &j.to_string()).as_deref(),
                );
            }
        }

        drop(stored_fields);
        // IOUtils.close(iw, ir, everything)
        iw.close()?;
        drop(iw);
        Arc::clone(&ir).as_index_reader().close()?;
        drop(ir);
        everything.close()?;
        // IOUtils.close(dirs)
        for dir in &mut dirs {
            dir.close()?;
        }
        Ok(())
    }

    /// `testRandomStoredFieldsWithIndexSort()`.
    fn test_random_stored_fields_with_index_sort(&self) -> Result<()> {
        let sort_fields: Vec<(&str, SortFieldType)> = if random().next_boolean() {
            vec![
                ("sort-1", SortFieldType::Long),
                ("sort-2", SortFieldType::Int),
            ]
        } else {
            vec![("sort-1", SortFieldType::Long)]
        };
        let sort = || -> Result<Sort> {
            Sort::new_fields(
                sort_fields
                    .iter()
                    .map(|(name, ty)| SortField::new(Some(name.to_string()), *ty))
                    .collect::<Result<Vec<_>>>()?,
            )
        };
        let num_fields = TestUtil::next_int(&*random(), 1, 10);
        let mut stored_fields: Vec<String> = (0..num_fields).map(|i| format!("f-{i}")).collect();
        let store_type = FieldType::new_from(TextField::type_stored());
        store_type.set_stored(true)?;
        // `documentFactory`: the document, and the stored fields it holds.
        let mut document_factory = |id: &str| -> Result<(Document, StoredValues)> {
            let mut doc = Document::new();
            let store = if random().next_boolean() {
                Store::YES
            } else {
                Store::NO
            };
            doc.add(Box::new(StringField::new("id", id.to_string(), store)?));
            if random().next_int_bound(100) <= 5 {
                randomizedtesting::jdk::util::shuffle(&mut stored_fields, &*random());
            }
            for field_name in &stored_fields {
                if random().next_boolean() {
                    let s = TestUtil::random_unicode_string_max(&*random(), 100);
                    doc.add(Box::new(LuceneTestCase::new_field(
                        field_name,
                        &s,
                        &store_type,
                    )));
                }
            }
            for (name, _) in &sort_fields {
                doc.add(Box::new(NumericDocValuesField::new(
                    name,
                    i64::from(TestUtil::next_int(&*random(), 0, 10000)),
                )));
            }
            let stored = stored_values(&doc);
            Ok((doc, stored))
        };

        let mut docs: JavaKeyedMap<(Document, StoredValues)> = JavaKeyedMap::new();
        let num_docs = LuceneTestCase::at_least(100);
        for i in 0..num_docs {
            let id = i.to_string();
            let built = document_factory(&id)?;
            docs.put(id, built);
        }

        let mut dir = LuceneTestCase::new_directory();
        let iwc = LuceneTestCase::new_index_writer_config();
        iwc.set_max_buffered_docs(TestUtil::next_int(&*random(), 5, 20))?;
        iwc.set_index_sort(sort()?)?;
        let iw = RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), iwc)?;
        let mut added_ids: Vec<String> = Vec::new();
        let verify_store_fields = |iw: &RandomIndexWriter,
                                   added_ids: &[String],
                                   docs: &JavaKeyedMap<(Document, StoredValues)>|
         -> Result<()> {
            if added_ids.is_empty() {
                return Ok(());
            }
            let reader = self.maybe_wrap_with_merging_reader(iw.get_reader()?)?;
            let outcome = (|| -> Result<()> {
                let actual_stored_fields = reader.stored_fields()?;
                let searcher = IndexSearcher::new(Arc::clone(&reader) as Arc<dyn IndexReader>)?;
                let iters = TestUtil::next_int(&*random(), 1, 10);
                for _ in 0..iters {
                    let test_id =
                        &added_ids[random().next_int_bound(added_ids.len() as i32) as usize];
                    if *VERBOSE {
                        println!("TEST: test id={test_id}");
                    }
                    let hits = searcher.search(term_query("id", test_id), 1)?;
                    Assert::assert_equals(1i64, hits.total_hits.value());
                    let expected_fields = &npe(docs.get(test_id)).1;
                    let actual_doc = actual_stored_fields.document(hits.score_docs[0].doc)?;
                    Assert::assert_equals(expected_fields.len(), actual_doc.get_fields().len());
                    for (name, value) in expected_fields {
                        let actual_fields = actual_doc.get_fields_by_name(name);
                        Assert::assert_equals(1, actual_fields.len());
                        Assert::assert_equals_str(
                            None,
                            value.as_deref(),
                            actual_fields[0].string_value().as_deref(),
                        );
                    }
                }
                Ok(())
            })();
            // try-with-resources: the reader is closed after the body, a
            // failure to close suppressed by the body's own.
            let closed = reader.close();
            outcome?;
            closed
        };
        let mut ids = docs.key_set();
        randomizedtesting::jdk::util::shuffle(&mut ids, &*random());
        for id in &ids {
            if random().next_int_bound(100) < 5 {
                // add via foreign reader
                let other_iwc = LuceneTestCase::new_index_writer_config();
                other_iwc.set_index_sort(sort()?)?;
                let mut other_dir = LuceneTestCase::new_directory();
                let other_iw = RandomIndexWriter::new_with_config(
                    &*random(),
                    other_dir.as_directory(),
                    other_iwc,
                )?;
                let outcome = (|| -> Result<()> {
                    other_iw.add_document(&npe(docs.get(id)).0)?;
                    let other_reader = other_iw.get_reader()?;
                    let added =
                        TestUtil::add_indexes_slowly(iw.w(), std::slice::from_ref(&other_reader));
                    let closed = other_reader.close();
                    added?;
                    closed
                })();
                // try-with-resources closes otherIw, then otherDir.
                let closed_iw = other_iw.close();
                drop(other_iw);
                let closed_dir = other_dir.close();
                outcome?;
                closed_iw?;
                closed_dir?;
            } else {
                // add normally
                iw.add_document(&npe(docs.get(id)).0)?;
            }
            added_ids.push(id.clone());
            if random().next_int_bound(100) < 5 {
                let deleting_id =
                    added_ids.remove(random().next_int_bound(added_ids.len() as i32) as usize);
                if random().next_boolean() {
                    iw.delete_documents_by_query(term_query("id", &deleting_id))?;
                    if let Some(pos) = added_ids.iter().position(|a| *a == deleting_id) {
                        added_ids.remove(pos);
                    }
                } else {
                    let new_doc = document_factory(&deleting_id)?;
                    iw.update_document(term("id", &deleting_id), &new_doc.0)?;
                    docs.put(deleting_id, new_doc);
                }
            }
            if random().next_int_bound(100) < 5 {
                verify_store_fields(&iw, &added_ids, &docs)?;
            }
            if random().next_int_bound(100) < 2 {
                iw.force_merge(TestUtil::next_int(&*random(), 1, 3))?;
            }
        }
        verify_store_fields(&iw, &added_ids, &docs)?;
        iw.force_merge(TestUtil::next_int(&*random(), 1, 3))?;
        verify_store_fields(&iw, &added_ids, &docs)?;
        // IOUtils.close(iw, dir)
        iw.close()?;
        drop(iw);
        dir.close()?;
        Ok(())
    }

    /// `testLineFileDocs()`: test realistic data, which typically compresses
    /// better than random data.
    fn test_line_file_docs(&self) -> Result<()> {
        // Use an FS dir and a non-randomized IWC to not slow down indexing
        let mut dir = LuceneTestCase::new_fs_directory(&LuceneTestCase::create_temp_dir());
        {
            let docs = LineFileDocs::with_default_file(&*random()).map_err(LuceneError::Io)?;
            let w = IndexWriter::new(dir.as_directory(), IndexWriterConfig::new())?;
            let outcome = (|| -> Result<()> {
                let num_docs = LuceneTestCase::at_least(10_000);
                for _ in 0..num_docs {
                    // Only keep stored fields
                    let doc = docs.next_doc()?;
                    let mut stored_doc = Document::new();
                    for field in doc.borrow().get_fields().iter().cloned() {
                        if field.field_type().stored() {
                            let stored_field: Arc<dyn IndexableField> = match field.string_value() {
                                // Disable indexing
                                Some(value) => {
                                    Arc::new(StoredField::new_string(field.name(), value)?)
                                }
                                None => field,
                            };
                            stored_doc.add_shared(stored_field);
                        }
                    }

                    w.add_document(&stored_doc)?;
                }
                w.force_merge(1)
            })();
            // try-with-resources closes w, then docs.
            let closed = w.close();
            docs.close();
            outcome?;
            closed?;
        }
        TestUtil::check_index(dir.as_directory())?;
        dir.close()?;
        Ok(())
    }
}

/// What `testMergeFilterReader` reads back for one document (`docs[id]`).
struct MergeFilterExpected {
    s: String,
    i: NumericValue,
    l: NumericValue,
    d: NumericValue,
    f: NumericValue,
    b: Vec<u8>,
}

// -----------------------------------------------------------------------------
// DummyFilterLeafReader / DummyFilterDirectoryReader
// -----------------------------------------------------------------------------

/// `DummyFilterLeafReader` (`private static class ... extends
/// FilterLeafReader`): a dummy filter reader that reverses the order of
/// documents in stored fields.
struct DummyFilterLeafReader {
    base: rucene::index::filter_reader::FilterLeafReader,
    r#in: Arc<dyn LeafReader>,
}

impl DummyFilterLeafReader {
    /// `new DummyFilterLeafReader(in)`.
    fn new(r#in: Arc<dyn LeafReader>) -> Result<Self> {
        let base = rucene::index::filter_reader::FilterLeafReader::new(Arc::clone(&r#in))?;
        Ok(DummyFilterLeafReader { base, r#in })
    }
}

impl fmt::Debug for DummyFilterLeafReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DummyFilterLeafReader")
            .finish_non_exhaustive()
    }
}

impl fmt::Display for DummyFilterLeafReader {
    /// The inherited `FilterLeafReader.toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "FilterLeafReader({})", self.r#in)
    }
}

impl LeafReader for DummyFilterLeafReader {
    forward_leaf_reader!(filter base;
        core,
        filter_leaf_reader_delegate,
        term_vectors,
        num_docs,
        max_doc,
        do_close,
        terms,
        get_numeric_doc_values,
        get_binary_doc_values,
        get_sorted_doc_values,
        get_sorted_numeric_doc_values,
        get_sorted_set_doc_values,
        get_norm_values,
        get_doc_values_skipper,
        get_float_vector_values,
        get_byte_vector_values,
        search_nearest_vectors,
        search_nearest_vectors_byte,
        get_field_infos,
        get_live_docs,
        get_point_values,
        check_integrity,
        get_meta_data,
    );

    /// `storedFields()`: `in.storedFields()`, visited at `maxDoc() - 1 -
    /// docID`.
    fn stored_fields(&self) -> Result<Box<dyn StoredFields>> {
        Ok(Box::new(ReversedStoredFields {
            orig: self.r#in.stored_fields()?,
            max_doc: LeafReader::max_doc(self),
        }))
    }

    /// `getCoreCacheHelper()`: `null`.
    fn get_core_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        None
    }

    /// `getReaderCacheHelper()`: `null`.
    fn get_reader_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        None
    }
}

/// The anonymous `StoredFields` of `DummyFilterLeafReader.storedFields()`.
struct ReversedStoredFields {
    orig: Box<dyn StoredFields>,
    max_doc: i32,
}

impl fmt::Debug for ReversedStoredFields {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ReversedStoredFields")
            .finish_non_exhaustive()
    }
}

impl StoredFields for ReversedStoredFields {
    fn document_with_visitor(
        &self,
        doc_id: i32,
        visitor: &mut dyn StoredFieldVisitor,
    ) -> Result<()> {
        self.orig
            .document_with_visitor(self.max_doc - 1 - doc_id, visitor)
    }
}

/// The `SubReaderWrapper` of `DummyFilterDirectoryReader`: `new
/// DummyFilterLeafReader(reader)`.
struct DummySubReaderWrapper;

impl SubReaderWrapper for DummySubReaderWrapper {
    fn wrap(&self, reader: Arc<dyn LeafReader>) -> Result<Arc<dyn LeafReader>> {
        Ok(Arc::new(DummyFilterLeafReader::new(reader)?))
    }
}

/// `DummyFilterDirectoryReader` (`private static class ... extends
/// FilterDirectoryReader`): the wrapper that stands for the subclass;
/// `doWrapDirectoryReader(in)` is `new DummyFilterDirectoryReader(in)`.
struct DummyFilterDirectoryReader;

impl DummyFilterDirectoryReader {
    /// `new DummyFilterDirectoryReader(in)`.
    #[allow(clippy::new_ret_no_self)]
    fn new(r#in: Arc<dyn DirectoryReader>) -> Result<FilterDirectoryReader> {
        FilterDirectoryReader::new(
            r#in,
            Arc::new(DummySubReaderWrapper),
            Arc::new(DummyFilterDirectoryReader),
        )
    }
}

impl DirectoryReaderWrapper for DummyFilterDirectoryReader {
    fn wrap(&self, inner: Arc<dyn DirectoryReader>) -> Result<Arc<dyn DirectoryReader>> {
        Ok(Arc::new(DummyFilterDirectoryReader::new(inner)?))
    }

    fn subclass_simple_name(&self) -> &'static str {
        "DummyFilterDirectoryReader"
    }
}
