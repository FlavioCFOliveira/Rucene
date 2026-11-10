//! Port of `lucene/core/src/test/org/apache/lucene/index/TestIndexableField.java`.
//!
//! * `w.addDocument(Iterable<IndexableField>)` over a lazy iterator is a
//!   `Document` holding the same fields in the same order; building it before
//!   `addDocument` draws `newStringField`'s values from the test's random
//!   before the writer runs, which draws from its own random.
//! * `IndexableFieldType.getAttributes()` returns `null` in Java; Rucene's
//!   returns a map, so `MyField`'s type holds an empty one.

use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::analysis::{Analyzer, SharedTokenStream, StringCharReader, TokenStream};
use rucene::document::{
    Document, FieldType, InvertableType, NumericValue, SharedReader, Store, StoredField,
    StoredValue,
};
use rucene::error::Result;
use rucene::index::terms::Term;
use rucene::index::{
    DocValuesSkipIndexType, DocValuesType, IndexOptions, IndexReader, IndexableField,
    IndexableFieldType, VectorEncoding, VectorSimilarityFunction, POSTINGS_ENUM_ALL,
};
use rucene::search::{BooleanQueryBuilder, Occur, Query, TermQuery, NO_MORE_DOCS};
use rucene::util::java_collections::JavaHashMap;
use rucene::util::BytesRef;
use rucene_test_framework::index::RandomIndexWriter;
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::{
    assert_equals, assert_not_null, assert_null, class, random, test, verbose, IAE,
};
use crate::support::{assert_not_null_msg, npe};

/// `TestIndexableField`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestIndexableField",
        vec![
            test("testArbitraryFields", test_arbitrary_fields),
            test("testNotIndexedTermVectors", test_not_indexed_term_vectors),
        ],
    )
}

/// The anonymous `IndexableFieldType` of `MyField`.
struct MyFieldType {
    counter: i32,
    attributes: JavaHashMap<String>,
}

impl fmt::Display for MyFieldType {
    /// `Object.toString()` of the anonymous class.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&rucene::index::index_reader::java_object_to_string(
            "org.apache.lucene.index.TestIndexableField$MyField$1",
            self,
        ))
    }
}

impl IndexableFieldType for MyFieldType {
    fn stored(&self) -> bool {
        (self.counter & 1) == 0 || (self.counter % 10) == 3
    }
    fn tokenized(&self) -> bool {
        true
    }
    fn store_term_vectors(&self) -> bool {
        self.index_options() != IndexOptions::NONE
            && self.counter % 2 == 1
            && self.counter % 10 != 9
    }
    fn store_term_vector_offsets(&self) -> bool {
        self.store_term_vectors() && self.counter % 10 != 9
    }
    fn store_term_vector_positions(&self) -> bool {
        self.store_term_vectors() && self.counter % 10 != 9
    }
    fn store_term_vector_payloads(&self) -> bool {
        self.store_term_vectors() && self.counter % 10 != 9
    }
    fn omit_norms(&self) -> bool {
        false
    }
    fn index_options(&self) -> IndexOptions {
        if self.counter % 10 == 3 {
            IndexOptions::NONE
        } else {
            IndexOptions::DOCS_AND_FREQS_AND_POSITIONS
        }
    }
    fn doc_values_type(&self) -> DocValuesType {
        DocValuesType::NONE
    }
    fn doc_values_skip_index_type(&self) -> DocValuesSkipIndexType {
        DocValuesSkipIndexType::NONE
    }
    fn point_dimension_count(&self) -> i32 {
        0
    }
    fn point_index_dimension_count(&self) -> i32 {
        0
    }
    fn point_num_bytes(&self) -> i32 {
        0
    }
    fn vector_dimension(&self) -> i32 {
        0
    }
    fn vector_encoding(&self) -> VectorEncoding {
        VectorEncoding::FLOAT32
    }
    fn vector_similarity_function(&self) -> VectorSimilarityFunction {
        VectorSimilarityFunction::EUCLIDEAN
    }
    fn attributes(&self) -> JavaHashMap<String> {
        self.attributes.clone()
    }
}

/// `private static class MyField implements IndexableField`.
struct MyField {
    counter: i32,
    field_type: MyFieldType,
    name: String,
}

impl MyField {
    fn new(counter: i32) -> Self {
        MyField {
            counter,
            field_type: MyFieldType {
                counter,
                attributes: JavaHashMap::new(),
            },
            name: format!("f{counter}"),
        }
    }
}

impl fmt::Display for MyField {
    /// `Object.toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&rucene::index::index_reader::java_object_to_string(
            "org.apache.lucene.index.TestIndexableField$MyField",
            self,
        ))
    }
}

impl IndexableField for MyField {
    fn name(&self) -> &str {
        &self.name
    }

    fn binary_value(&self) -> Option<BytesRef> {
        if (self.counter % 10) == 3 {
            let mut bytes = [0u8; 10];
            for (idx, b) in bytes.iter_mut().enumerate() {
                *b = (self.counter + idx as i32) as u8;
            }
            Some(LuceneTestCase::new_bytes_ref_slice(&bytes, 0, bytes.len()))
        } else {
            None
        }
    }

    fn string_value(&self) -> Option<String> {
        let field_id = self.counter % 10;
        if field_id != 3 && field_id != 7 {
            Some(format!("text {}", self.counter))
        } else {
            None
        }
    }

    fn reader_value(&self) -> Option<SharedReader> {
        if self.counter % 10 == 7 {
            Some(Rc::new(RefCell::new(Box::new(StringCharReader::new(
                format!("text {}", self.counter),
            )))))
        } else {
            None
        }
    }

    fn numeric_value(&self) -> Option<NumericValue> {
        None
    }

    fn field_type(&self) -> &dyn IndexableFieldType {
        &self.field_type
    }

    fn token_stream(
        &self,
        analyzer: Option<&dyn Analyzer>,
        _previous: Option<Box<dyn TokenStream>>,
    ) -> Result<Box<dyn TokenStream>> {
        // `analyzer.tokenStream(...)` on a `null` analyzer.
        let analyzer = analyzer.ok_or_else(rucene::error::LuceneError::null_pointer)?;
        let stream = if self.reader_value().is_some() {
            analyzer.token_stream(
                self.name(),
                Box::new(StringCharReader::new(format!("text {}", self.counter))),
            )
        } else {
            analyzer.token_stream(
                self.name(),
                Box::new(StringCharReader::new(
                    self.string_value().unwrap_or_else(|| npe("stringValue")),
                )),
            )
        }?;
        Ok(Box::new(SharedTokenStream::new(stream)))
    }

    fn stored_value(&self) -> Result<Option<StoredValue>> {
        if let Some(s) = self.string_value() {
            Ok(Some(StoredValue::String(s)))
        } else if let Some(b) = self.binary_value() {
            Ok(Some(StoredValue::Binary(b)))
        } else {
            Ok(None)
        }
    }

    fn invertable_type(&self) -> Option<InvertableType> {
        Some(InvertableType::TOKEN_STREAM)
    }
}

/// `new TermQuery(new Term(field, text))`.
fn tq(field: &str, text: &str) -> Arc<dyn Query> {
    Arc::new(TermQuery::new(Term::from_text(field, text)))
}

/// Silly test showing how to index documents w/o using Lucene's core
/// Document nor Field class
/// Port of `TestIndexableField.testArbitraryFields`.
fn test_arbitrary_fields() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let w = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;

    let num_docs = LuceneTestCase::at_least(27);
    if verbose() {
        println!("TEST: {num_docs} docs");
    }
    let mut fields_per_doc = vec![0i32; num_docs as usize];
    let mut base_count = 0;

    for doc_count in 0..num_docs {
        let field_count = TestUtil::next_int(&*random(), 1, 17);
        fields_per_doc[doc_count as usize] = field_count - 1;

        if verbose() {
            println!("TEST: {field_count} fields in doc {doc_count}");
        }

        let final_base_count = base_count;
        base_count += field_count - 1;

        let mut d = Document::new();
        for field_upto in 0..field_count {
            if field_upto == 0 {
                d.add(Box::new(LuceneTestCase::new_string_field(
                    "id",
                    &doc_count.to_string(),
                    Store::YES,
                )));
            } else {
                d.add(Box::new(MyField::new(final_base_count + (field_upto - 1))));
            }
        }
        w.add_document(&d)?;
    }

    let dr = w.get_reader()?;
    let r: Arc<dyn IndexReader> = Arc::clone(&dr) as Arc<dyn IndexReader>;
    w.close()?;

    let s = LuceneTestCase::new_searcher(dr);
    let stored_fields = s.stored_fields()?;
    let term_vectors = r.term_vectors()?;
    let mut counter = 0;
    for id in 0..num_docs {
        if verbose() {
            println!(
                "TEST: verify doc id={id} ({} fields) counter={counter}",
                fields_per_doc[id as usize]
            );
        }

        let hits = s.search(tq("id", &id.to_string()), 1)?;
        assert_equals(&1, &hits.total_hits.value());
        let doc_id = hits.score_docs[0].doc;
        let doc = stored_fields.document(doc_id)?;
        let end_counter = counter + fields_per_doc[id as usize];
        while counter < end_counter {
            let name = format!("f{counter}");
            let field_id = counter % 10;

            let stored = (counter & 1) == 0 || field_id == 3;
            let binary = field_id == 3;
            let indexed = field_id != 3;

            let string_value = if field_id != 3 && field_id != 9 {
                Some(format!("text {counter}"))
            } else {
                None
            };

            // stored:
            if stored {
                let f = doc.get_field(&name);
                assert_not_null_msg(
                    &format!("doc {id} doesn't have field f{counter}"),
                    &f.map(|_| ()),
                );
                let f = f.unwrap_or_else(|| npe("f"));
                if binary {
                    let b = f.binary_value();
                    assert_not_null(&b);
                    let b = b.unwrap_or_else(|| npe("b"));
                    assert_equals(&10, &b.length);
                    for idx in 0..10 {
                        assert_equals(
                            &((idx + counter) as i8),
                            &(b.bytes[b.offset + idx as usize] as i8),
                        );
                    }
                } else {
                    debug_assert!(string_value.is_some());
                    assert_equals(&string_value, &f.string_value());
                }
            }

            if indexed {
                let tv = counter % 2 == 1 && field_id != 9;
                if tv {
                    let tfv = term_vectors
                        .get(doc_id)?
                        .unwrap_or_else(|| npe("vectors"))
                        .terms(&name)?;
                    assert_not_null(&tfv.as_ref().map(|_| ()));
                    let mut terms_enum = tfv.unwrap_or_else(|| npe("tfv")).iterator()?;
                    assert_equals(
                        &Some(LuceneTestCase::new_bytes_ref_str(&counter.to_string())),
                        &terms_enum.next()?,
                    );
                    assert_equals(&1, &terms_enum.total_term_freq()?);
                    let mut dp_enum = terms_enum.postings(None, POSTINGS_ENUM_ALL)?;
                    Assert::assert_true(dp_enum.next_doc()? != NO_MORE_DOCS);
                    assert_equals(&1, &dp_enum.freq()?);
                    assert_equals(&1, &dp_enum.next_position()?);

                    assert_equals(
                        &Some(LuceneTestCase::new_bytes_ref_str("text")),
                        &terms_enum.next()?,
                    );
                    assert_equals(&1, &terms_enum.total_term_freq()?);
                    let mut dp_enum = terms_enum.postings(Some(dp_enum), POSTINGS_ENUM_ALL)?;
                    Assert::assert_true(dp_enum.next_doc()? != NO_MORE_DOCS);
                    assert_equals(&1, &dp_enum.freq()?);
                    assert_equals(&0, &dp_enum.next_position()?);

                    assert_null(&terms_enum.next()?);

                    // TODO: offsets
                } else {
                    let vectors = term_vectors.get(doc_id)?;
                    Assert::assert_true(match vectors {
                        None => true,
                        Some(vectors) => vectors.terms(&name)?.is_none(),
                    });
                }

                let mut bq = BooleanQueryBuilder::new();
                bq.add(tq("id", &id.to_string()), Occur::MUST)?;
                bq.add(tq(&name, "text"), Occur::MUST)?;
                let hits2 = s.search(Arc::new(bq.build()), 1)?;
                assert_equals(&1, &hits2.total_hits.value());
                assert_equals(&doc_id, &hits2.score_docs[0].doc);

                let mut bq = BooleanQueryBuilder::new();
                bq.add(tq("id", &id.to_string()), Occur::MUST)?;
                bq.add(tq(&name, &counter.to_string()), Occur::MUST)?;
                let hits3 = s.search(Arc::new(bq.build()), 1)?;
                assert_equals(&1, &hits3.total_hits.value());
                assert_equals(&doc_id, &hits3.score_docs[0].doc);
            }

            counter += 1;
        }
    }

    r.close()?;
    drop(s);
    dir.close()
}

/// `private static class CustomField implements IndexableField`.
struct CustomField {
    field_type: FieldType,
}

impl CustomField {
    fn new() -> Result<Self> {
        // `fieldType()`: `new FieldType(StoredField.TYPE)` with
        // `setStoreTermVectors(true)`, frozen.
        let ft = FieldType::new_from(StoredField::type_());
        ft.set_store_term_vectors(true)?;
        ft.freeze();
        Ok(CustomField { field_type: ft })
    }
}

impl fmt::Display for CustomField {
    /// `Object.toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&rucene::index::index_reader::java_object_to_string(
            "org.apache.lucene.index.TestIndexableField$CustomField",
            self,
        ))
    }
}

impl IndexableField for CustomField {
    fn binary_value(&self) -> Option<BytesRef> {
        None
    }

    fn string_value(&self) -> Option<String> {
        Some("foobar".to_string())
    }

    fn reader_value(&self) -> Option<SharedReader> {
        None
    }

    fn numeric_value(&self) -> Option<NumericValue> {
        None
    }

    fn name(&self) -> &str {
        "field"
    }

    /// Java returns `null`.
    fn token_stream(
        &self,
        _a: Option<&dyn Analyzer>,
        _reuse: Option<Box<dyn TokenStream>>,
    ) -> Result<Box<dyn TokenStream>> {
        npe("CustomField.tokenStream returned null")
    }

    fn field_type(&self) -> &dyn IndexableFieldType {
        &self.field_type
    }

    fn stored_value(&self) -> Result<Option<StoredValue>> {
        Ok(None)
    }

    fn invertable_type(&self) -> Option<InvertableType> {
        Some(InvertableType::TOKEN_STREAM)
    }
}

/// LUCENE-5611
/// Port of `TestIndexableField.testNotIndexedTermVectors`.
fn test_not_indexed_term_vectors() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let w = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    expect_throws(IAE, || {
        let mut doc = Document::new();
        doc.add(Box::new(CustomField::new()?));
        w.add_document(&doc)
    });
    w.close()?;
    dir.close()
}
