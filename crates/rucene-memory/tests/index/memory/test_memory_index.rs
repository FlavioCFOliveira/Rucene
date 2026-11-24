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

//! Port of `TestMemoryIndex.java`.

use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::jdk::util::shuffle;
use randomizedtesting::junit::Assert;
use rucene::analysis::standard::StandardAnalyzer;
use rucene::analysis::{Analyzer, TokenStream};
use rucene::document::{
    BinaryDocValuesField, BinaryPoint, Document, DoublePoint, Field, FieldType, FloatPoint,
    IntField, IntPoint, InvertableType, KnnByteVectorField, KnnFloatVectorField, LongPoint,
    NumericDocValuesField, NumericValue, SortedDocValuesField, SortedNumericDocValuesField,
    SortedSetDocValuesField, Store, StoredField, StoredValue, StringField, TextField,
};
use rucene::error::Result;
use rucene::index::postings_enum::{POSTINGS_ENUM_ALL, POSTINGS_ENUM_OFFSETS};
use rucene::index::{
    DocValuesType, FieldInvertState, IndexOptions, IndexableField, IndexableFieldType, LeafReader,
    Term, VectorSimilarityFunction,
};
use rucene::search::match_all_docs_query::MatchAllDocsQuery;
use rucene::search::phrase_query::PhraseQuery;
use rucene::search::query::Query;
use rucene::search::similarities::{BM25Similarity, ClassicSimilarity, SimScorer, Similarity};
use rucene::search::{
    CollectionStatistics, IndexSearcher, KnnByteVectorQuery, KnnFloatVectorQuery, TermQuery,
    TermStatistics, NO_MORE_DOCS,
};
use rucene::util::BytesRef;
use rucene_memory::memory_index::MemoryIndex;
use rucene_test_framework::analysis::mock_payload_analyzer::MockPayloadAnalyzer;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::test_util::TestUtil;

use crate::memory_support::core_support::hamcrest::assert_contains_string;
use crate::memory_support::core_support::random;
use crate::memory_support::search_support::{fixture_class, ftest, set_up};

/// The instance of `TestMemoryIndex`.
#[derive(Default)]
pub struct MemoryIndexTest {
    analyzer: Option<Arc<MockAnalyzer>>,
}

impl MemoryIndexTest {
    fn analyzer(&self) -> &MockAnalyzer {
        self.analyzer.as_deref().expect("setup ran")
    }
}

/// `TestMemoryIndex`.
pub fn class_def() -> Arc<Class> {
    fixture_class::<MemoryIndexTest>(
        "org.apache.lucene.index.memory.TestMemoryIndex",
        vec![
            set_up(setup),
            ftest("testFreezeAPI", test_freeze_api),
            ftest("testSeekByTermOrd", test_seek_by_term_ord),
            ftest(
                "testFieldsOnlyReturnsIndexedFields",
                test_fields_only_returns_indexed_fields,
            ),
            ftest("testReaderConsistency", test_reader_consistency),
            ftest("testSimilarities", test_similarities),
            ftest("testOmitNorms", test_omit_norms),
            ftest("testBuildFromDocument", test_build_from_document),
            ftest("testDocValues", test_doc_values),
            ftest(
                "testDocValues_resetIterator",
                test_doc_values_reset_iterator,
            ),
            ftest("testInvalidDocValuesUsage", test_invalid_doc_values_usage),
            ftest(
                "testDocValuesDoNotAffectBoostPositionsOrOffset",
                test_doc_values_do_not_affect_boost_positions_or_offset,
            ),
            ftest("testBigBinaryDocValues", test_big_binary_doc_values),
            ftest("testBigSortedDocValues", test_big_sorted_doc_values),
            ftest("testPointValues", test_point_values),
            ftest("testMissingPoints", test_missing_points),
            ftest(
                "testPointValuesDoNotAffectPositionsOrOffset",
                test_point_values_do_not_affect_positions_or_offset,
            ),
            ftest("test2DPoints", test_2d_points),
            ftest(
                "testMultiValuedPointsSortedCorrectly",
                test_multi_valued_points_sorted_correctly,
            ),
            ftest(
                "testIndexingPointsAndDocValues",
                test_indexing_points_and_doc_values,
            ),
            ftest("testToStringDebug", test_to_string_debug),
            ftest("testStoredFields", test_stored_fields),
            ftest(
                "testKnnFloatVectorOnlyOneVectorAllowed",
                test_knn_float_vector_only_one_vector_allowed,
            ),
            ftest("testKnnFloatVectors", test_knn_float_vectors),
            ftest(
                "testKnnByteVectorOnlyOneVectorAllowed",
                test_knn_byte_vector_only_one_vector_allowed,
            ),
            ftest("testKnnByteVectors", test_knn_byte_vectors),
            ftest("testIntegerNumericDocValue", test_integer_numeric_doc_value),
            ftest(
                "testKeywordWithoutTokenStream",
                test_keyword_without_token_stream,
            ),
        ],
    )
}

fn setup(f: &mut MemoryIndexTest) -> Result<()> {
    let analyzer = MockAnalyzer::new(&*random());
    analyzer.set_enable_checks(false); // MemoryIndex can close a TokenStream on init error
    f.analyzer = Some(Arc::new(analyzer));
    Ok(())
}

/// `Document` as the `Iterable<IndexableField>` `MemoryIndex.fromDocument`
/// takes.
fn fields(doc: &Document) -> impl Iterator<Item = &dyn IndexableField> {
    doc.get_fields().iter().map(|f| &**f)
}

fn tq(field: &str, text: &str) -> Arc<dyn Query> {
    Arc::new(TermQuery::new(Term::from_text(field.to_string(), text)))
}

fn phrase(field: &str, terms: &[&str]) -> Result<Arc<dyn Query>> {
    Ok(Arc::new(PhraseQuery::from_strings(field, terms)?))
}

fn bytes(s: &str) -> BytesRef {
    BytesRef::from_char_sequence(s)
}

/// `assertNotEquals(0.0f, actual)`: `Float` objects compared with
/// `equals`.
fn assert_not_zero(actual: f32) {
    if 0.0f32.to_bits() == actual.to_bits() {
        Assert::fail("Values should be different. Actual: 0.0");
    }
}

/// `searcher.getIndexReader().leaves().get(0).reader()`.
fn leaf(searcher: &IndexSearcher) -> Arc<dyn LeafReader> {
    searcher.get_leaf_contexts()[0].leaf_reader()
}

fn test_freeze_api(f: &mut MemoryIndexTest) -> Result<()> {
    let analyzer = f.analyzer();
    let mut mi = MemoryIndex::new();
    mi.add_field_text("f1", "some text", analyzer)?;

    assert_not_zero(mi.search(Arc::new(MatchAllDocsQuery::instance()))?);
    assert_not_zero(mi.search(tq("f1", "some"))?);

    // check we can add a new field after searching
    mi.add_field_text("f2", "some more text", analyzer)?;
    assert_not_zero(mi.search(tq("f2", "some"))?);

    // freeze!
    mi.freeze()?;

    let mi = Mutex::new(mi);
    let expected = expect_throws("java.lang.RuntimeException", || {
        mi.lock()
            .unwrap()
            .add_field_text("f3", "and yet more", analyzer)
    });
    assert_contains_string(expected.message(), "frozen");

    let expected = expect_throws("java.lang.RuntimeException", || {
        mi.lock()
            .unwrap()
            .set_similarity(Arc::new(BM25Similarity::with_parameters(1.0, 1.0)?))
    });
    assert_contains_string(expected.message(), "frozen");
    let mut mi = mi.into_inner().unwrap();

    assert_not_zero(mi.search(tq("f1", "some"))?);

    mi.reset();
    mi.add_field_text("f1", "wibble", analyzer)?;
    Assert::assert_equals_float_msg(None, 0.0, mi.search(tq("f1", "some"))?, 0.0);
    assert_not_zero(mi.search(tq("f1", "wibble"))?);

    // check we can set the Similarity again
    mi.set_similarity(Arc::new(ClassicSimilarity::new()))?;
    Ok(())
}

fn test_seek_by_term_ord(f: &mut MemoryIndexTest) -> Result<()> {
    let mut mi = MemoryIndex::new();
    mi.add_field_text("field", "some terms be here", f.analyzer())?;
    let searcher = mi.create_searcher()?;
    let reader = leaf(&searcher);
    let mut terms = reader.terms("field")?.expect("terms").iterator()?;
    terms.seek_ord(0)?;
    Assert::assert_equals(
        "be".to_string(),
        terms.term()?.expect("term").utf8_to_string()?,
    );
    TestUtil::check_reader(searcher.get_index_reader())?;
    Ok(())
}

fn test_fields_only_returns_indexed_fields(f: &mut MemoryIndexTest) -> Result<()> {
    let mut doc = Document::new();

    doc.add(NumericDocValuesField::new("numeric", 29));
    doc.add(TextField::new("text", "some text".to_string(), Store::NO)?);

    let mi = MemoryIndex::from_document(fields(&doc), Some(f.analyzer()))?;
    let searcher = mi.create_searcher()?;
    let reader = searcher.get_index_reader();

    Assert::assert_equals(reader.term_vectors()?.get(0)?.expect("fields").size(), 1);
    Ok(())
}

fn test_reader_consistency(_f: &mut MemoryIndexTest) -> Result<()> {
    let analyzer = MockPayloadAnalyzer::new();

    // defaults
    let mut mi = MemoryIndex::new();
    mi.add_field_text("field", "some terms be here", &analyzer)?;
    TestUtil::check_reader(mi.create_searcher()?.get_index_reader())?;

    // all combinations of offsets/payloads options
    for (offsets, payloads) in [(true, true), (true, false), (false, true), (false, false)] {
        let mut mi = MemoryIndex::with_offsets_and_payloads(offsets, payloads);
        mi.add_field_text("field", "some terms be here", &analyzer)?;
        TestUtil::check_reader(mi.create_searcher()?.get_index_reader())?;
    }

    analyzer.close();
    Ok(())
}

/// The anonymous `Similarity` of `testSimilarities`.
#[derive(Debug)]
struct FixedNormSimilarity;

impl Similarity for FixedNormSimilarity {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.index.memory.TestMemoryIndex$1"
    }

    fn compute_norm(&self, _state: &FieldInvertState) -> Result<i64> {
        Ok(74)
    }

    fn scorer<'a>(
        &'a self,
        _boost: f32,
        _collection_stats: &CollectionStatistics,
        _term_stats: &[TermStatistics],
    ) -> Box<dyn SimScorer + 'a> {
        randomizedtesting::jdk::exception::throw_without_message(
            "java.lang.UnsupportedOperationException",
        )
    }
}

fn test_similarities(f: &mut MemoryIndexTest) -> Result<()> {
    let mut mi = MemoryIndex::new();
    mi.add_field_text(
        "f1",
        "a long text field that contains many many terms",
        f.analyzer(),
    )?;

    let searcher = mi.create_searcher()?;
    let reader = leaf(&searcher);
    let mut norms = reader.get_norm_values("f1")?.expect("norms");
    Assert::assert_equals(0, norms.next_doc()?);
    let n1 = norms.long_value()? as f32;

    // Norms are re-computed when we change the Similarity
    mi.set_similarity(Arc::new(FixedNormSimilarity))?;
    let mut norms = reader.get_norm_values("f1")?.expect("norms");
    Assert::assert_equals(0, norms.next_doc()?);
    let n2 = norms.long_value()? as f32;

    Assert::assert_true(n1 != n2);
    TestUtil::check_reader(searcher.get_index_reader())?;
    Ok(())
}

fn test_omit_norms(f: &mut MemoryIndexTest) -> Result<()> {
    let mut mi = MemoryIndex::new();
    let ft = FieldType::new();
    ft.set_tokenized(true)?;
    ft.set_index_options(IndexOptions::DOCS_AND_FREQS)?;
    ft.set_omit_norms(true)?;
    mi.add_field(
        &Field::new("f1", "some text in here".to_string(), ft)?,
        Some(f.analyzer()),
    )?;
    mi.freeze()?;

    let leader = leaf(&mi.create_searcher()?);
    let norms = leader.get_norm_values("f1")?;
    Assert::assert_true(norms.is_none());
    Ok(())
}

fn test_build_from_document(f: &mut MemoryIndexTest) -> Result<()> {
    let mut doc = Document::new();
    doc.add(TextField::new(
        "field1",
        "some text".to_string(),
        Store::NO,
    )?);
    doc.add(TextField::new(
        "field1",
        "some more text".to_string(),
        Store::NO,
    )?);
    doc.add(StringField::new(
        "field2",
        "untokenized text".to_string(),
        Store::NO,
    )?);

    f.analyzer().set_position_increment_gap(100);

    let mi = MemoryIndex::from_document(fields(&doc), Some(f.analyzer()))?;

    assert_not_zero(mi.search(tq("field1", "text"))?);
    Assert::assert_equals_float_msg(None, 0.0, mi.search(tq("field2", "text"))?, 0.0);
    assert_not_zero(mi.search(tq("field2", "untokenized text"))?);

    Assert::assert_equals_float_msg(None, 0.0, mi.search(tq("field1", "some more text"))?, 0.0);
    assert_not_zero(mi.search(phrase("field1", &["some", "more", "text"])?)?);
    assert_not_zero(mi.search(phrase("field1", &["some", "text"])?)?);
    Assert::assert_equals_float_msg(
        None,
        0.0,
        mi.search(phrase("field1", &["text", "some"])?)?,
        0.0,
    );
    Ok(())
}

fn test_doc_values(f: &mut MemoryIndexTest) -> Result<()> {
    let mut doc = Document::new();
    doc.add(NumericDocValuesField::new("numeric", 29));
    doc.add(SortedNumericDocValuesField::new("sorted_numeric", 33));
    doc.add(SortedNumericDocValuesField::new("sorted_numeric", 32));
    doc.add(SortedNumericDocValuesField::new("sorted_numeric", 32));
    doc.add(SortedNumericDocValuesField::new("sorted_numeric", 31));
    doc.add(SortedNumericDocValuesField::new("sorted_numeric", 30));
    doc.add(BinaryDocValuesField::new("binary", bytes("a")));
    doc.add(SortedDocValuesField::new("sorted", bytes("b")));
    doc.add(SortedSetDocValuesField::new("sorted_set", bytes("f")));
    doc.add(SortedSetDocValuesField::new("sorted_set", bytes("d")));
    doc.add(SortedSetDocValuesField::new("sorted_set", bytes("d")));
    doc.add(SortedSetDocValuesField::new("sorted_set", bytes("c")));

    let mi = MemoryIndex::from_document(fields(&doc), Some(f.analyzer()))?;
    let leaf_reader = leaf(&mi.create_searcher()?);
    let mut numeric_doc_values = leaf_reader.get_numeric_doc_values("numeric")?.expect("dv");
    Assert::assert_equals(0, numeric_doc_values.next_doc()?);
    Assert::assert_equals(29i64, numeric_doc_values.long_value()?);
    Assert::assert_equals(NO_MORE_DOCS, numeric_doc_values.next_doc()?);
    let mut sorted_numeric_doc_values = leaf_reader
        .get_sorted_numeric_doc_values("sorted_numeric")?
        .expect("dv");
    Assert::assert_equals(0, sorted_numeric_doc_values.next_doc()?);
    Assert::assert_equals(5, sorted_numeric_doc_values.doc_value_count()?);
    for v in [30i64, 31, 32, 32, 33] {
        Assert::assert_equals(v, sorted_numeric_doc_values.next_value()?);
    }
    Assert::assert_equals(NO_MORE_DOCS, sorted_numeric_doc_values.next_doc()?);
    let mut binary_doc_values = leaf_reader.get_binary_doc_values("binary")?.expect("dv");
    Assert::assert_equals(0, binary_doc_values.next_doc()?);
    Assert::assert_equals(
        "a".to_string(),
        binary_doc_values.binary_value()?.utf8_to_string()?,
    );
    Assert::assert_equals(NO_MORE_DOCS, binary_doc_values.next_doc()?);
    let mut sorted_doc_values = leaf_reader.get_sorted_doc_values("sorted")?.expect("dv");
    Assert::assert_equals(0, sorted_doc_values.next_doc()?);
    let ord = sorted_doc_values.ord_value()?;
    Assert::assert_equals(
        "b".to_string(),
        sorted_doc_values.lookup_ord(ord)?.utf8_to_string()?,
    );
    Assert::assert_equals(0, sorted_doc_values.ord_value()?);
    Assert::assert_equals(
        "b".to_string(),
        sorted_doc_values.lookup_ord(0)?.utf8_to_string()?,
    );
    Assert::assert_equals(NO_MORE_DOCS, sorted_doc_values.next_doc()?);
    let mut sorted_set_doc_values = leaf_reader
        .get_sorted_set_doc_values("sorted_set")?
        .expect("dv");
    Assert::assert_equals(3i64, sorted_set_doc_values.get_value_count()?);
    Assert::assert_equals(0, sorted_set_doc_values.next_doc()?);
    Assert::assert_equals(3, sorted_set_doc_values.doc_value_count()?);
    Assert::assert_equals(0i64, sorted_set_doc_values.next_ord()?);
    Assert::assert_equals(1i64, sorted_set_doc_values.next_ord()?);
    Assert::assert_equals(2i64, sorted_set_doc_values.next_ord()?);
    Assert::assert_equals(
        "c".to_string(),
        sorted_set_doc_values.lookup_ord(0)?.utf8_to_string()?,
    );
    Assert::assert_equals(
        "d".to_string(),
        sorted_set_doc_values.lookup_ord(1)?.utf8_to_string()?,
    );
    Assert::assert_equals(
        "f".to_string(),
        sorted_set_doc_values.lookup_ord(2)?.utf8_to_string()?,
    );
    Assert::assert_equals(NO_MORE_DOCS, sorted_doc_values.next_doc()?);
    Ok(())
}

fn test_doc_values_reset_iterator(f: &mut MemoryIndexTest) -> Result<()> {
    let mut doc = Document::new();

    doc.add(SortedSetDocValuesField::new("sorted_set", bytes("f")));
    doc.add(SortedSetDocValuesField::new("sorted_set", bytes("d")));
    doc.add(SortedSetDocValuesField::new("sorted_set", bytes("d")));
    doc.add(SortedSetDocValuesField::new("sorted_set", bytes("c")));

    doc.add(SortedNumericDocValuesField::new("sorted_numeric", 33));
    doc.add(SortedNumericDocValuesField::new("sorted_numeric", 32));
    doc.add(SortedNumericDocValuesField::new("sorted_numeric", 32));
    doc.add(SortedNumericDocValuesField::new("sorted_numeric", 31));
    doc.add(SortedNumericDocValuesField::new("sorted_numeric", 30));

    let mi = MemoryIndex::from_document(fields(&doc), Some(f.analyzer()))?;
    let leaf_reader = leaf(&mi.create_searcher()?);

    let mut sorted_set_doc_values = leaf_reader
        .get_sorted_set_doc_values("sorted_set")?
        .expect("dv");
    Assert::assert_equals(3i64, sorted_set_doc_values.get_value_count()?);
    for _times in 0..3 {
        Assert::assert_true(sorted_set_doc_values.advance_exact(0)?);
        Assert::assert_equals(3, sorted_set_doc_values.doc_value_count()?);
        Assert::assert_equals(0i64, sorted_set_doc_values.next_ord()?);
        Assert::assert_equals(1i64, sorted_set_doc_values.next_ord()?);
        Assert::assert_equals(2i64, sorted_set_doc_values.next_ord()?);
    }

    let mut sorted_numeric_doc_values = leaf_reader
        .get_sorted_numeric_doc_values("sorted_numeric")?
        .expect("dv");
    for _times in 0..3 {
        Assert::assert_true(sorted_numeric_doc_values.advance_exact(0)?);
        Assert::assert_equals(5, sorted_numeric_doc_values.doc_value_count()?);
        for v in [30i64, 31, 32, 32, 33] {
            Assert::assert_equals(v, sorted_numeric_doc_values.next_value()?);
        }
    }
    Ok(())
}

/// `try { MemoryIndex.fromDocument(doc, analyzer); } catch
/// (IllegalArgumentException e) { assertEquals(message, e.getMessage()); }`.
fn assert_from_document_iae(doc: &Document, analyzer: &dyn Analyzer, message: &str) {
    if let Err(e) = MemoryIndex::from_document(fields(doc), Some(analyzer)) {
        let t = rucene_test_framework::util::java_exceptions::java_throwable(&e);
        if t.is_instance("java.lang.IllegalArgumentException") {
            Assert::assert_equals_str(None, Some(message), t.message());
        } else {
            t.throw();
        }
    }
}

fn test_invalid_doc_values_usage(f: &mut MemoryIndexTest) -> Result<()> {
    let mut doc = Document::new();
    doc.add(NumericDocValuesField::new("field", 29));
    doc.add(BinaryDocValuesField::new("field", bytes("30")));
    assert_from_document_iae(
        &doc,
        f.analyzer(),
        "cannot change DocValues type from NUMERIC to BINARY for field \"field\"",
    );

    let mut doc = Document::new();
    doc.add(NumericDocValuesField::new("field", 29));
    doc.add(NumericDocValuesField::new("field", 30));
    assert_from_document_iae(
        &doc,
        f.analyzer(),
        "Only one value per field allowed for [NUMERIC] doc values field [field]",
    );

    let mut doc = Document::new();
    doc.add(TextField::new("field", "a b".to_string(), Store::NO)?);
    doc.add(BinaryDocValuesField::new("field", bytes("a")));
    doc.add(BinaryDocValuesField::new("field", bytes("b")));
    assert_from_document_iae(
        &doc,
        f.analyzer(),
        "Only one value per field allowed for [BINARY] doc values field [field]",
    );

    let mut doc = Document::new();
    doc.add(SortedDocValuesField::new("field", bytes("a")));
    doc.add(SortedDocValuesField::new("field", bytes("b")));
    doc.add(TextField::new("field", "a b".to_string(), Store::NO)?);
    assert_from_document_iae(
        &doc,
        f.analyzer(),
        "Only one value per field allowed for [SORTED] doc values field [field]",
    );
    Ok(())
}

/// The terms `brown`, `fox` and `quick` of "quick brown fox" with their
/// positions and offsets, as `testDocValuesDoNotAffectBoostPositionsOrOffset`
/// and `testPointValuesDoNotAffectPositionsOrOffset` check them.
fn assert_quick_brown_fox(leaf_reader: &Arc<dyn LeafReader>) -> Result<()> {
    let mut tenum = leaf_reader.terms("text")?.expect("terms").iterator()?;

    let mut penum = None;
    for (term, position, start, end) in
        [("brown", 1, 6, 11), ("fox", 2, 12, 15), ("quick", 0, 0, 5)]
    {
        Assert::assert_equals(
            term.to_string(),
            tenum.next()?.expect("term").utf8_to_string()?,
        );
        let mut p = tenum
            .postings(penum.take(), POSTINGS_ENUM_OFFSETS)?
            .expect("postings");
        Assert::assert_equals(0, p.next_doc()?);
        Assert::assert_equals(1, p.freq()?);
        Assert::assert_equals(position, p.next_position()?);
        Assert::assert_equals(start, p.start_offset());
        Assert::assert_equals(end, p.end_offset());
        penum = Some(p);
    }
    Ok(())
}

fn test_doc_values_do_not_affect_boost_positions_or_offset(f: &mut MemoryIndexTest) -> Result<()> {
    let mut doc = Document::new();
    doc.add(BinaryDocValuesField::new("text", bytes("quick brown fox")));
    doc.add(TextField::new(
        "text",
        "quick brown fox".to_string(),
        Store::NO,
    )?);
    let mi = MemoryIndex::from_document_with_options(fields(&doc), Some(f.analyzer()), true, true)?;
    let leaf_reader = leaf(&mi.create_searcher()?);
    assert_quick_brown_fox(&leaf_reader)?;

    let mut binary_doc_values = leaf_reader.get_binary_doc_values("text")?.expect("dv");
    Assert::assert_equals(0, binary_doc_values.next_doc()?);
    Assert::assert_equals(
        "quick brown fox".to_string(),
        binary_doc_values.binary_value()?.utf8_to_string()?,
    );
    Ok(())
}

/// `random().nextBytes(new byte[n])`.
fn random_bytes(n: usize) -> Vec<u8> {
    let mut bytes = vec![0u8; n];
    random().next_bytes(&mut bytes);
    bytes
}

/// The valid bytes of a `BytesRef`; Java compares `.bytes`, whose offset is 0
/// and whose length is the array's here.
fn valid(b: &BytesRef) -> &[u8] {
    &b.bytes[b.offset..b.offset + b.length]
}

fn test_big_binary_doc_values(f: &mut MemoryIndexTest) -> Result<()> {
    let mut doc = Document::new();
    let bytes = random_bytes(33 * 1024);
    doc.add(BinaryDocValuesField::new(
        "binary",
        BytesRef::new(bytes.clone()),
    ));
    let mi = MemoryIndex::from_document_with_options(fields(&doc), Some(f.analyzer()), true, true)?;
    let leaf_reader = leaf(&mi.create_searcher()?);
    let mut binary_doc_values = leaf_reader.get_binary_doc_values("binary")?.expect("dv");
    Assert::assert_equals(0, binary_doc_values.next_doc()?);
    let value = binary_doc_values.binary_value()?;
    Assert::assert_true(bytes.as_slice() == &value.bytes[..]);
    Ok(())
}

fn test_big_sorted_doc_values(f: &mut MemoryIndexTest) -> Result<()> {
    let mut doc = Document::new();
    let bytes = random_bytes(33 * 1024);
    doc.add(SortedDocValuesField::new(
        "binary",
        BytesRef::new(bytes.clone()),
    ));
    let mi = MemoryIndex::from_document_with_options(fields(&doc), Some(f.analyzer()), true, true)?;
    let leaf_reader = leaf(&mi.create_searcher()?);
    let mut sorted_doc_values = leaf_reader.get_sorted_doc_values("binary")?.expect("dv");
    Assert::assert_equals(0, sorted_doc_values.next_doc()?);
    let value = sorted_doc_values.lookup_ord(0)?;
    Assert::assert_true(bytes.as_slice() == &value.bytes[..]);
    Ok(())
}

type FieldFn = fn(i64) -> Result<Box<dyn IndexableField>>;
type ExactFn = fn(i64) -> Result<Arc<dyn Query>>;
type SetFn = fn(&[i64]) -> Result<Arc<dyn Query>>;
type RangeFn = fn(i64, i64) -> Result<Arc<dyn Query>>;

fn test_point_values(f: &mut MemoryIndexTest) -> Result<()> {
    let field_functions: [FieldFn; 4] = [
        |t| Ok(Box::new(IntPoint::new("number", &[t as i32])?)),
        |t| Ok(Box::new(LongPoint::new("number", &[t])?)),
        |t| Ok(Box::new(FloatPoint::new("number", &[t as f32])?)),
        |t| Ok(Box::new(DoublePoint::new("number", &[t as f64])?)),
    ];
    let exact_query_functions: [ExactFn; 4] = [
        |t| IntPoint::new_exact_query("number", t as i32),
        |t| LongPoint::new_exact_query("number", t),
        |t| FloatPoint::new_exact_query("number", t as f32),
        |t| DoublePoint::new_exact_query("number", t as f64),
    ];
    let set_query_functions: [SetFn; 4] = [
        |t| IntPoint::new_set_query("number", &t.iter().map(|&v| v as i32).collect::<Vec<_>>()),
        |t| LongPoint::new_set_query("number", t),
        |t| FloatPoint::new_set_query("number", &t.iter().map(|&v| v as f32).collect::<Vec<_>>()),
        |t| DoublePoint::new_set_query("number", &t.iter().map(|&v| v as f64).collect::<Vec<_>>()),
    ];
    let range_query_functions: [RangeFn; 4] = [
        |t, u| IntPoint::new_range_query("number", &[t as i32], &[u as i32]),
        |t, u| LongPoint::new_range_query("number", &[t], &[u]),
        |t, u| FloatPoint::new_range_query("number", &[t as f32], &[u as f32]),
        |t, u| DoublePoint::new_range_query("number", &[t as f64], &[u as f64]),
    ];

    for i in 0..field_functions.len() {
        let field_function = field_functions[i];
        let exact_query_function = exact_query_functions[i];
        let set_query_function = set_query_functions[i];
        let range_query_function = range_query_functions[i];

        let mut doc = Document::new();
        let mut number = 1;
        while number < 32 {
            doc.add_shared(Arc::from(field_function(number)?));
            number += 2;
        }
        let mi = MemoryIndex::from_document(fields(&doc), Some(f.analyzer()))?;
        let index_searcher = mi.create_searcher()?;
        Assert::assert_equals(1, index_searcher.count(exact_query_function(5)?)?);
        Assert::assert_equals(0, index_searcher.count(exact_query_function(4)?)?);

        Assert::assert_equals(1, index_searcher.count(set_query_function(&[3, 9, 19])?)?);
        Assert::assert_equals(1, index_searcher.count(set_query_function(&[2, 8, 13])?)?);
        Assert::assert_equals(0, index_searcher.count(set_query_function(&[2, 8, 16])?)?);

        Assert::assert_equals(1, index_searcher.count(range_query_function(2, 16)?)?);
        Assert::assert_equals(1, index_searcher.count(range_query_function(24, 48)?)?);
        Assert::assert_equals(0, index_searcher.count(range_query_function(48, 68)?)?);
    }
    Ok(())
}

fn test_missing_points(f: &mut MemoryIndexTest) -> Result<()> {
    let mut doc = Document::new();
    doc.add(StoredField::new_number("field", NumericValue::Int(42))?);
    let mi = MemoryIndex::from_document(fields(&doc), Some(f.analyzer()))?;
    let index_searcher = mi.create_searcher()?;
    // field that exists but does not have points
    Assert::assert_true(leaf(&index_searcher).get_point_values("field")?.is_none());
    // field that does not exist
    Assert::assert_true(
        leaf(&index_searcher)
            .get_point_values("some_missing_field")?
            .is_none(),
    );
    Ok(())
}

fn binary_point(name: &str, value: &str) -> Result<BinaryPoint> {
    BinaryPoint::new(
        name.to_string(),
        &[BytesRef::new(value.as_bytes().to_vec())],
    )
}

fn test_point_values_do_not_affect_positions_or_offset(f: &mut MemoryIndexTest) -> Result<()> {
    let mut mi = MemoryIndex::with_offsets_and_payloads(true, true);
    mi.add_field(
        &TextField::new("text", "quick brown fox".to_string(), Store::NO)?,
        Some(f.analyzer()),
    )?;
    mi.add_field(&binary_point("text", "quick")?, Some(f.analyzer()))?;
    mi.add_field(&binary_point("text", "brown")?, Some(f.analyzer()))?;
    let leaf_reader = leaf(&mi.create_searcher()?);
    assert_quick_brown_fox(&leaf_reader)?;

    let index_searcher = mi.create_searcher()?;
    Assert::assert_equals(
        1,
        index_searcher.count(BinaryPoint::new_exact_query("text", b"quick")?)?,
    );
    Assert::assert_equals(
        1,
        index_searcher.count(BinaryPoint::new_exact_query("text", b"brown")?)?,
    );
    Assert::assert_equals(
        0,
        index_searcher.count(BinaryPoint::new_exact_query("text", b"jumps")?)?,
    );
    Ok(())
}

fn test_2d_points(f: &mut MemoryIndexTest) -> Result<()> {
    let mut doc = Document::new();
    doc.add(IntPoint::new("ints", &[0, -100])?);
    doc.add(IntPoint::new("ints", &[20, 20])?);
    doc.add(IntPoint::new("ints", &[100, -100])?);
    doc.add(LongPoint::new("longs", &[0, -100])?);
    doc.add(LongPoint::new("longs", &[20, 20])?);
    doc.add(LongPoint::new("longs", &[100, -100])?);
    doc.add(FloatPoint::new("floats", &[0.0, -100.0])?);
    doc.add(FloatPoint::new("floats", &[20.0, 20.0])?);
    doc.add(FloatPoint::new("floats", &[100.0, -100.0])?);
    doc.add(DoublePoint::new("doubles", &[0.0, -100.0])?);
    doc.add(DoublePoint::new("doubles", &[20.0, 20.0])?);
    doc.add(DoublePoint::new("doubles", &[100.0, -100.0])?);

    let mi = MemoryIndex::from_document(fields(&doc), Some(f.analyzer()))?;
    let s = mi.create_searcher()?;

    Assert::assert_equals(
        1,
        s.count(IntPoint::new_range_query("ints", &[10, 10], &[30, 30])?)?,
    );
    Assert::assert_equals(
        1,
        s.count(LongPoint::new_range_query("longs", &[10, 10], &[30, 30])?)?,
    );
    Assert::assert_equals(
        1,
        s.count(FloatPoint::new_range_query(
            "floats",
            &[10.0, 10.0],
            &[30.0, 30.0],
        )?)?,
    );
    Assert::assert_equals(
        1,
        s.count(DoublePoint::new_range_query(
            "doubles",
            &[10.0, 10.0],
            &[30.0, 30.0],
        )?)?,
    );
    Ok(())
}

fn test_multi_valued_points_sorted_correctly(f: &mut MemoryIndexTest) -> Result<()> {
    let mut doc = Document::new();
    for v in [3, 2, 1] {
        doc.add(IntPoint::new("ints", &[v])?);
    }
    for v in [3i64, 2, 1] {
        doc.add(LongPoint::new("longs", &[v])?);
    }
    for v in [3.0f32, 2.0, 1.0] {
        doc.add(FloatPoint::new("floats", &[v])?);
    }
    for v in [3.0f64, 2.0, 1.0] {
        doc.add(DoublePoint::new("doubles", &[v])?);
    }

    let mi = MemoryIndex::from_document(fields(&doc), Some(f.analyzer()))?;
    let s = mi.create_searcher()?;

    Assert::assert_equals(1, s.count(IntPoint::new_set_query("ints", &[2])?)?);
    Assert::assert_equals(1, s.count(LongPoint::new_set_query("longs", &[2])?)?);
    Assert::assert_equals(1, s.count(FloatPoint::new_set_query("floats", &[2.0])?)?);
    Assert::assert_equals(1, s.count(DoublePoint::new_set_query("doubles", &[2.0])?)?);
    Ok(())
}

/// `new FieldType()` with `setDimensions(1, 4)` and `setDocValuesType(BINARY)`,
/// frozen.
fn point_and_dv_type() -> Result<FieldType> {
    let ty = FieldType::new();
    ty.set_dimensions(1, 4)?;
    ty.set_doc_values_type(DocValuesType::BINARY)?;
    ty.freeze();
    Ok(ty)
}

fn test_indexing_points_and_doc_values(f: &mut MemoryIndexTest) -> Result<()> {
    let ty = point_and_dv_type()?;
    let mut doc = Document::new();
    let packed_point = b"term".to_vec();
    doc.add(BinaryPoint::new_with_packed(
        "field",
        BytesRef::new(packed_point.clone()),
        ty,
    )?);
    let mi = MemoryIndex::from_document(fields(&doc), Some(f.analyzer()))?;
    let leaf_reader = leaf(&mi.create_searcher()?);

    let points = leaf_reader.get_point_values("field")?.expect("points");
    Assert::assert_equals(1i64, points.size()?);
    Assert::assert_true(
        packed_point.as_slice() == points.min_packed_value()?.expect("min").as_slice(),
    );
    Assert::assert_true(
        packed_point.as_slice() == points.max_packed_value()?.expect("max").as_slice(),
    );

    let mut dvs = leaf_reader.get_binary_doc_values("field")?.expect("dv");
    Assert::assert_equals(0, dvs.next_doc()?);
    Assert::assert_equals("term".to_string(), dvs.binary_value()?.utf8_to_string()?);
    Ok(())
}

fn test_to_string_debug(_f: &mut MemoryIndexTest) -> Result<()> {
    let mut mi = MemoryIndex::with_offsets_and_payloads(true, true);
    let analyzer = MockPayloadAnalyzer::new();

    mi.add_field_text("analyzedField", "aa bb aa", &analyzer)?;

    let ty = point_and_dv_type()?;
    mi.add_field(
        &BinaryPoint::new_with_packed("pointAndDvField", BytesRef::new(b"term".to_vec()), ty)?,
        Some(&analyzer),
    )?;

    Assert::assert_equals(
        concat!(
            "analyzedField:\n",
            "\t'[61 61]':2: [(0, 0, 2, [70 6f 73 3a 20 30]), (1, 6, 8, [70 6f 73 3a 20 32])]\n",
            "\t'[62 62]':1: [(1, 3, 5, [70 6f 73 3a 20 31])]\n",
            "\tterms=2, positions=3\n",
            "pointAndDvField:\n",
            "\tterms=0, positions=0\n",
            "\n",
            "fields=2, terms=2, positions=3"
        )
        .to_string(),
        mi.to_string_debug()?,
    );
    Ok(())
}

/// A value `assertContains` and `assertMultiContains` compare with
/// `Objects.equals`: a boxed `Number` (its class and value) or a `String`.
#[derive(Debug, Clone, PartialEq)]
enum Value {
    Number(NumericValue),
    Str(String),
}

fn numeric_value(f: &dyn IndexableField) -> Option<Value> {
    f.numeric_value().map(Value::Number)
}

fn string_value(f: &dyn IndexableField) -> Option<Value> {
    f.string_value().map(Value::Str)
}

fn test_stored_fields(_f: &mut MemoryIndexTest) -> Result<()> {
    let mut stored: Vec<StoredField> = vec![
        StoredField::new_number("float", NumericValue::Float(1.5))?,
        StoredField::new_number("multifloat", NumericValue::Float(2.5))?,
        StoredField::new_number("multifloat", NumericValue::Float(3.5))?,
        StoredField::new_number("long", NumericValue::Long(10))?,
        StoredField::new_number("multilong", NumericValue::Long(15))?,
        StoredField::new_number("multilong", NumericValue::Long(20))?,
        StoredField::new_number("int", NumericValue::Double(1.7))?,
        StoredField::new_number("multiint", NumericValue::Double(2.7))?,
        StoredField::new_number("multiint", NumericValue::Double(2.8))?,
        StoredField::new_number("multiint", NumericValue::Double(2.9))?,
        StoredField::new_number("double", NumericValue::Double(3.7))?,
        StoredField::new_number("multidouble", NumericValue::Double(4.5))?,
        StoredField::new_number("multidouble", NumericValue::Double(4.6))?,
        StoredField::new_number("multidouble", NumericValue::Double(4.7))?,
        StoredField::new_string("string", "foo".to_string())?,
        StoredField::new_string("multistring", "bar".to_string())?,
        StoredField::new_string("multistring", "baz".to_string())?,
        StoredField::new_bytes("binary", BytesRef::new(b"bfoo".to_vec()))?,
        StoredField::new_bytes("multibinary", BytesRef::new(b"bbar".to_vec()))?,
        StoredField::new_bytes("multibinary", BytesRef::new(b"bbaz".to_vec()))?,
    ];

    shuffle(&mut stored, &*random());
    let mut doc = Document::new();
    for f in stored {
        doc.add(f);
    }

    let mi = MemoryIndex::from_document(fields(&doc), Some(&StandardAnalyzer::new()))?;
    let d = mi.create_searcher()?.stored_fields()?.document(0)?;

    assert_contains(
        &d,
        "long",
        Value::Number(NumericValue::Long(10)),
        numeric_value,
    );
    assert_contains(
        &d,
        "int",
        Value::Number(NumericValue::Double(1.7)),
        numeric_value,
    );
    assert_contains(
        &d,
        "double",
        Value::Number(NumericValue::Double(3.7)),
        numeric_value,
    );
    assert_contains(
        &d,
        "float",
        Value::Number(NumericValue::Float(1.5)),
        numeric_value,
    );
    assert_contains(&d, "string", Value::Str("foo".to_string()), string_value);
    assert_binary_contains(&d, "binary", &BytesRef::new(b"bfoo".to_vec()));

    assert_multi_contains(
        &d,
        "multilong",
        &[
            Value::Number(NumericValue::Long(15)),
            Value::Number(NumericValue::Long(20)),
        ],
        numeric_value,
    );
    assert_multi_contains(
        &d,
        "multiint",
        &[
            Value::Number(NumericValue::Double(2.7)),
            Value::Number(NumericValue::Double(2.8)),
            Value::Number(NumericValue::Double(2.9)),
        ],
        numeric_value,
    );
    assert_multi_contains(
        &d,
        "multidouble",
        &[
            Value::Number(NumericValue::Double(4.5)),
            Value::Number(NumericValue::Double(4.6)),
            Value::Number(NumericValue::Double(4.7)),
        ],
        numeric_value,
    );
    assert_multi_contains(
        &d,
        "multifloat",
        &[
            Value::Number(NumericValue::Float(2.5)),
            Value::Number(NumericValue::Float(3.5)),
        ],
        numeric_value,
    );
    assert_multi_contains(
        &d,
        "multistring",
        &[Value::Str("bar".to_string()), Value::Str("baz".to_string())],
        string_value,
    );
    assert_binary_multi_contains(
        &d,
        "multibinary",
        &[
            BytesRef::new(b"bbar".to_vec()),
            BytesRef::new(b"bbaz".to_vec()),
        ],
    );
    Ok(())
}

fn test_knn_float_vector_only_one_vector_allowed(_f: &mut MemoryIndexTest) -> Result<()> {
    let mut doc = Document::new();
    doc.add(KnnFloatVectorField::with_euclidean(
        "knnFloatA",
        &[1.0, 2.0],
    )?);
    doc.add(KnnFloatVectorField::with_euclidean(
        "knnFloatA",
        &[3.0, 4.0],
    )?);
    expect_throws("java.lang.IllegalArgumentException", || {
        MemoryIndex::from_document(fields(&doc), Some(&StandardAnalyzer::new()))
    });
    Ok(())
}

fn test_knn_float_vectors(_f: &mut MemoryIndexTest) -> Result<()> {
    let mut vectors: Vec<KnnFloatVectorField> = vec![
        KnnFloatVectorField::with_euclidean("knnFloatA", &[1.0, 2.0])?,
        KnnFloatVectorField::with_euclidean("knnFloatB", &[3.0, 4.0, 5.0, 6.0])?,
        KnnFloatVectorField::new(
            "knnFloatC",
            &[7.0, 8.0, 9.0],
            VectorSimilarityFunction::DOT_PRODUCT,
        )?,
    ];
    shuffle(&mut vectors, &*random());
    let mut doc = Document::new();
    for f in vectors {
        doc.add(f);
    }

    let mi = MemoryIndex::from_document(fields(&doc), Some(&StandardAnalyzer::new()))?;
    assert_float_vector_value(&mi, "knnFloatA", &[1.0, 2.0])?;
    assert_float_vector_value(&mi, "knnFloatB", &[3.0, 4.0, 5.0, 6.0])?;
    assert_float_vector_value(&mi, "knnFloatC", &[7.0, 8.0, 9.0])?;

    assert_float_vector_score(&mi, "knnFloatA", &[1.0, 1.0], 0.5)?;
    assert_float_vector_score(&mi, "knnFloatB", &[3.0, 3.0, 3.0, 3.0], 0.06666667)?;
    assert_float_vector_score(&mi, "knnFloatC", &[7.0, 7.0, 7.0], 84.5)?;

    Assert::assert_true(
        leaf(&mi.create_searcher()?)
            .get_float_vector_values("knnFloatMissing")?
            .is_none(),
    );
    Assert::assert_true(
        leaf(&mi.create_searcher()?)
            .get_byte_vector_values("knnByteVectorValue")?
            .is_none(),
    );
    let docs = mi.create_searcher()?.search(
        Arc::new(KnnFloatVectorQuery::new("knnFloatA", vec![1.0, 1.0], 1)?),
        10,
    )?;
    // we don't really do knn search right now for MemoryIndex
    Assert::assert_equals(0i64, docs.total_hits.value());
    Ok(())
}

fn test_knn_byte_vector_only_one_vector_allowed(_f: &mut MemoryIndexTest) -> Result<()> {
    let mut doc = Document::new();
    doc.add(KnnByteVectorField::with_euclidean("knnByteA", &[1, 2])?);
    doc.add(KnnByteVectorField::with_euclidean("knnByteA", &[3, 4])?);
    expect_throws("java.lang.IllegalArgumentException", || {
        MemoryIndex::from_document(fields(&doc), Some(&StandardAnalyzer::new()))
    });
    Ok(())
}

fn test_knn_byte_vectors(_f: &mut MemoryIndexTest) -> Result<()> {
    let mut vectors: Vec<KnnByteVectorField> = vec![
        KnnByteVectorField::with_euclidean("knnByteA", &[1, 2])?,
        KnnByteVectorField::with_euclidean("knnByteB", &[3, 4, 5, 6])?,
        KnnByteVectorField::new(
            "knnByteC",
            &[7, 8, 9],
            VectorSimilarityFunction::DOT_PRODUCT,
        )?,
    ];
    shuffle(&mut vectors, &*random());
    let mut doc = Document::new();
    for f in vectors {
        doc.add(f);
    }

    let mi = MemoryIndex::from_document(fields(&doc), Some(&StandardAnalyzer::new()))?;
    assert_byte_vector_value(&mi, "knnByteA", &[1, 2])?;
    assert_byte_vector_value(&mi, "knnByteB", &[3, 4, 5, 6])?;
    assert_byte_vector_value(&mi, "knnByteC", &[7, 8, 9])?;

    assert_byte_vector_score(&mi, "knnByteA", &[1, 1], 0.5)?;
    assert_byte_vector_score(&mi, "knnByteB", &[3, 3, 3, 3], 0.06666667)?;
    assert_byte_vector_score(&mi, "knnByteC", &[7, 7, 7], 0.501709)?;

    Assert::assert_true(
        leaf(&mi.create_searcher()?)
            .get_byte_vector_values("knnByteMissing")?
            .is_none(),
    );
    Assert::assert_true(
        leaf(&mi.create_searcher()?)
            .get_float_vector_values("knnFloatVectorValue")?
            .is_none(),
    );
    let docs = mi.create_searcher()?.search(
        Arc::new(KnnByteVectorQuery::new("knnByteA", vec![1, 1], 1)?),
        10,
    )?;
    // we don't really do knn search right now for MemoryIndex
    Assert::assert_equals(0i64, docs.total_hits.value());
    Ok(())
}

fn assert_float_vector_value(mi: &MemoryIndex, field_name: &str, expected: &[f32]) -> Result<()> {
    let fvv = leaf(&mi.create_searcher()?).get_float_vector_values(field_name)?;
    Assert::assert_true(fvv.is_some());
    let fvv = fvv.unwrap();
    let mut iterator = fvv.iterator()?;
    Assert::assert_equals(0, iterator.next_doc()?);
    let actual = fvv.vector_value(0)?;
    Assert::assert_equals(expected.len(), actual.len());
    for (e, a) in expected.iter().zip(actual.iter()) {
        Assert::assert_equals_float_msg(None, *e, *a, 1e-6);
    }
    Assert::assert_equals(NO_MORE_DOCS, iterator.next_doc()?);
    Ok(())
}

fn assert_float_vector_score(
    mi: &MemoryIndex,
    field_name: &str,
    query_vector: &[f32],
    expected_score: f32,
) -> Result<()> {
    let fvv = leaf(&mi.create_searcher()?).get_float_vector_values(field_name)?;
    Assert::assert_true(fvv.is_some());
    let fvv = fvv.unwrap();
    if random().next_boolean() {
        fvv.iterator()?.next_doc()?;
    }
    let mut scorer = fvv.scorer(query_vector)?.expect("scorer");
    Assert::assert_equals(0, scorer.iterator().next_doc()?);
    Assert::assert_equals_float_msg(None, expected_score, scorer.score()?, 0.0);
    Assert::assert_equals(NO_MORE_DOCS, scorer.iterator().next_doc()?);
    Ok(())
}

fn assert_byte_vector_value(mi: &MemoryIndex, field_name: &str, expected: &[u8]) -> Result<()> {
    let bvv = leaf(&mi.create_searcher()?).get_byte_vector_values(field_name)?;
    Assert::assert_true(bvv.is_some());
    let bvv = bvv.unwrap();
    let mut iterator = bvv.iterator()?;
    Assert::assert_equals(0, iterator.next_doc()?);
    Assert::assert_true(expected == &bvv.vector_value(0)?[..]);
    Assert::assert_equals(NO_MORE_DOCS, iterator.next_doc()?);
    Ok(())
}

fn assert_byte_vector_score(
    mi: &MemoryIndex,
    field_name: &str,
    query_vector: &[u8],
    expected_score: f32,
) -> Result<()> {
    let bvv = leaf(&mi.create_searcher()?).get_byte_vector_values(field_name)?;
    Assert::assert_true(bvv.is_some());
    let bvv = bvv.unwrap();
    if random().next_boolean() {
        bvv.iterator()?.next_doc()?;
    }
    let mut scorer = bvv.scorer(query_vector)?.expect("scorer");
    Assert::assert_equals(0, scorer.iterator().next_doc()?);
    Assert::assert_equals_float_msg(None, expected_score, scorer.score()?, 0.0);
    Assert::assert_equals(NO_MORE_DOCS, scorer.iterator().next_doc()?);
    Ok(())
}

fn assert_contains(
    d: &Document,
    field: &str,
    expected: Value,
    value: fn(&dyn IndexableField) -> Option<Value>,
) {
    Assert::assert_true(d.get_field(field).is_some());
    let fields = d.get_fields_by_name(field);
    Assert::assert_equals(1, fields.len());
    Assert::assert_true(Some(expected) == value(fields[0]));
}

fn assert_binary_contains(d: &Document, field: &str, expected: &BytesRef) {
    Assert::assert_true(d.get_field(field).is_some());
    let fields = d.get_fields_by_name(field);
    Assert::assert_equals(1, fields.len());
    let actual = fields[0].binary_value().expect("binary value");
    Assert::assert_equals(0, valid(expected).cmp(valid(&actual)) as i32);
}

fn assert_multi_contains(
    d: &Document,
    field: &str,
    expected: &[Value],
    value: fn(&dyn IndexableField) -> Option<Value>,
) {
    Assert::assert_true(d.get(field).is_some());
    let fields = d.get_fields_by_name(field);
    Assert::assert_equals(expected.len(), fields.len());
    for f in fields {
        let actual = value(f);
        Assert::assert_true(array_contains(expected, &actual));
    }
}

fn assert_binary_multi_contains(d: &Document, field: &str, expected: &[BytesRef]) {
    let fields = d.get_fields_by_name(field);
    Assert::assert_equals(expected.len(), fields.len());
    for f in fields {
        let actual = f.binary_value().expect("binary value");
        Assert::assert_true(array_binary_contains(expected, &actual));
    }
}

fn array_contains(array: &[Value], value: &Option<Value>) -> bool {
    array.iter().any(|o| Some(o) == value.as_ref())
}

fn array_binary_contains(array: &[BytesRef], value: &BytesRef) -> bool {
    array.iter().any(|b| valid(b) == valid(value))
}

fn test_integer_numeric_doc_value(_f: &mut MemoryIndexTest) -> Result<()> {
    // MemoryIndex used to fail when doc values are enabled and numericValue() returns an Integer
    // such as with IntField.
    let ft = FieldType::new();
    ft.set_doc_values_type(DocValuesType::NUMERIC)?;
    ft.freeze();
    // `new Field("field", ft) {{ fieldsData = 35; }}`
    let field = Field::new_with_number("field", NumericValue::Int(35), ft)?;

    let multi_ft = FieldType::new();
    multi_ft.set_doc_values_type(DocValuesType::SORTED_NUMERIC)?;
    multi_ft.freeze();
    let multi_field = Field::new_with_number("multi_field", NumericValue::Int(42), multi_ft)?;

    let int_field = IntField::new("int_field", 50, Store::NO);

    let doc: [&dyn IndexableField; 3] = [&field, &multi_field, &int_field];
    let index = MemoryIndex::from_document(doc, None)?;
    let searcher = index.create_searcher()?;

    let mut ndv = leaf(&searcher)
        .get_numeric_doc_values("field")?
        .expect("dv");
    Assert::assert_true(ndv.advance_exact(0)?);
    Assert::assert_equals(35i64, ndv.long_value()?);

    let mut sndv = leaf(&searcher)
        .get_sorted_numeric_doc_values("multi_field")?
        .expect("dv");
    Assert::assert_true(sndv.advance_exact(0)?);
    Assert::assert_equals(1, sndv.doc_value_count()?);
    Assert::assert_equals(42i64, sndv.next_value()?);

    let mut sndv = leaf(&searcher)
        .get_sorted_numeric_doc_values("int_field")?
        .expect("dv");
    Assert::assert_true(sndv.advance_exact(0)?);
    Assert::assert_equals(1, sndv.doc_value_count()?);
    Assert::assert_equals(50i64, sndv.next_value()?);
    Ok(())
}

/// `private record MockIndexableField(String field, BytesRef value,
/// IndexableFieldType fieldType) implements IndexableField`.
struct MockIndexableField {
    field: String,
    value: BytesRef,
    field_type: FieldType,
}

impl std::fmt::Display for MockIndexableField {
    /// The record's `toString()`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "MockIndexableField[field={}, value={}, fieldType={}]",
            self.field, self.value, self.field_type
        )
    }
}

impl IndexableField for MockIndexableField {
    fn name(&self) -> &str {
        &self.field
    }

    fn field_type(&self) -> &dyn IndexableFieldType {
        &self.field_type
    }

    fn token_stream(
        &self,
        _analyzer: Option<&dyn Analyzer>,
        _reuse: &mut Option<Box<dyn TokenStream>>,
    ) -> Result<Box<dyn TokenStream>> {
        // `return null;`: never asked, see `token_stream_or_null`.
        randomizedtesting::jdk::exception::throw_without_message("java.lang.NullPointerException")
    }

    /// `return null;`
    fn token_stream_or_null(
        &self,
        _analyzer: Option<&dyn Analyzer>,
        _reuse: &mut Option<Box<dyn TokenStream>>,
    ) -> Result<Option<Box<dyn TokenStream>>> {
        Ok(None)
    }

    fn binary_value(&self) -> Option<BytesRef> {
        Some(self.value.clone())
    }

    fn string_value(&self) -> Option<String> {
        None
    }

    fn reader_value(&self) -> Option<rucene::document::SharedReader> {
        None
    }

    fn numeric_value(&self) -> Option<NumericValue> {
        None
    }

    fn stored_value(&self) -> Result<Option<StoredValue>> {
        Ok(None)
    }

    fn invertable_type(&self) -> Option<InvertableType> {
        Some(InvertableType::BINARY)
    }
}

fn test_keyword_without_token_stream(_f: &mut MemoryIndexTest) -> Result<()> {
    let mut legal_field_types: Vec<FieldType> = Vec::new();
    for (index_options, omit_norms, store_term_vectors) in [
        (IndexOptions::DOCS, Some(false), false),
        (IndexOptions::DOCS_AND_FREQS, Some(false), false),
        (IndexOptions::DOCS, Some(true), false),
        (IndexOptions::DOCS_AND_FREQS, Some(true), false),
        (IndexOptions::DOCS, None, true),
        (IndexOptions::DOCS_AND_FREQS, None, true),
    ] {
        let ft = FieldType::new();
        ft.set_tokenized(false)?;
        ft.set_index_options(index_options)?;
        if let Some(omit_norms) = omit_norms {
            ft.set_omit_norms(omit_norms)?;
        }
        if store_term_vectors {
            ft.set_store_term_vectors(true)?;
        }
        ft.freeze();
        legal_field_types.push(ft);
    }

    for ft in legal_field_types {
        let store_term_vectors = ft.store_term_vectors();
        let field = MockIndexableField {
            field: "field".to_string(),
            value: bytes("a"),
            field_type: ft,
        };
        let doc: [&dyn IndexableField; 2] = [&field, &field];
        let index = MemoryIndex::from_document(doc, None)?;
        let leaf_reader = leaf(&index.create_searcher()?);
        {
            let terms = leaf_reader.terms("field")?.expect("terms");
            Assert::assert_equals(1i64, terms.sum_doc_freq()?);
            Assert::assert_equals(2i64, terms.sum_total_term_freq()?);
            let mut terms_enum = terms.iterator()?;
            Assert::assert_true(terms_enum.seek_exact(&bytes("a"))?);
            let mut pe = terms_enum
                .postings(None, POSTINGS_ENUM_ALL)?
                .expect("postings");
            Assert::assert_equals(0, pe.next_doc()?);
            Assert::assert_equals(2, pe.freq()?);
            Assert::assert_equals(0, pe.next_position()?);
            Assert::assert_equals(1, pe.next_position()?);
            Assert::assert_equals(NO_MORE_DOCS, pe.next_doc()?);
        }

        if store_term_vectors {
            let tv_terms = leaf_reader
                .term_vectors()?
                .get(0)?
                .expect("fields")
                .terms("field")?
                .expect("terms");
            Assert::assert_equals(1i64, tv_terms.sum_doc_freq()?);
            Assert::assert_equals(2i64, tv_terms.sum_total_term_freq()?);
            let mut tv_terms_enum = tv_terms.iterator()?;
            Assert::assert_true(tv_terms_enum.seek_exact(&bytes("a"))?);
            let mut pe = tv_terms_enum
                .postings(None, POSTINGS_ENUM_ALL)?
                .expect("postings");
            Assert::assert_equals(0, pe.next_doc()?);
            Assert::assert_equals(2, pe.freq()?);
            Assert::assert_equals(0, pe.next_position()?);
            Assert::assert_equals(1, pe.next_position()?);
            Assert::assert_equals(NO_MORE_DOCS, pe.next_doc()?);
        }
    }
    Ok(())
}
