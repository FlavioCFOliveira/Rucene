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

//! Port of `TestMemoryIndexAgainstDirectory.java`: verifies that Lucene
//! MemoryIndex and RAM-resident Directory have the same behaviour, returning
//! the same results for queries on some randomish indexes.

use std::collections::HashSet;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::analysis::{
    Analyzer, AnalyzerState, GlobalReuseStrategy, TokenFilter, TokenFilterLogic, TokenStream,
    TokenStreamComponents,
};
use rucene::document::{
    BinaryDocValuesField, Document, DoublePoint, Field, FieldType, FloatPoint, IntPoint, LongPoint,
    NumericDocValuesField, SortedDocValuesField, SortedNumericDocValuesField,
    SortedSetDocValuesField, Store, StringField, TextField,
};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::field_infos::FieldInfos;
use rucene::index::index_writer::{IndexWriter, MAX_TERM_LENGTH};
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::multi_doc_values::MultiDocValues;
use rucene::index::postings_enum::{
    POSTINGS_ENUM_ALL, POSTINGS_ENUM_FREQS, POSTINGS_ENUM_NONE, POSTINGS_ENUM_POSITIONS,
};
use rucene::index::{IndexOptions, IndexReader, IndexableField, LeafReader, Term, Terms};
use rucene::search::phrase_query::PhraseQuery;
use rucene::search::query::Query;
use rucene::search::regexp_query::RegexpQuery;
use rucene::search::{IndexSearcher, TermQuery, NO_MORE_DOCS};
use rucene::store::{ByteBuffersDirectory, Directory};
use rucene::util::atomic_ref_cell::AtomicRefCell;
use rucene::util::attribute::AttributeSource;
use rucene::util::BytesRef;
use rucene_memory::memory_index::MemoryIndex;
use rucene_queries::spans::span_multi_term_query_wrapper::SpanMultiTermQueryWrapper;
use rucene_queries::spans::span_or_query::SpanOrQuery;
use rucene_queries::spans::span_query::SpanQuery;
use rucene_queryparser::classic::query_parser::QueryParser;
use rucene_queryparser::classic::query_parser_base::QueryParserBaseOps;
use rucene_test_framework::analysis::attributes::{char_term_id, Att};
use rucene_test_framework::analysis::canned_token_stream::CannedTokenStream;
use rucene_test_framework::analysis::mock_token_filter::ENGLISH_STOPSET;
use rucene_test_framework::analysis::mock_tokenizer::{MockTokenizer, SIMPLE, WHITESPACE};
use rucene_test_framework::analysis::token::Token;
use rucene_test_framework::analysis::{components, sink, BaseTokenStreamTestCase, MockAnalyzer};
use rucene_test_framework::util::line_file_docs::LineFileDocs;
use rucene_test_framework::util::lucene_test_case::{RANDOM_MULTIPLIER, TEST_NIGHTLY};
use rucene_test_framework::util::test_util::TestUtil;
use rucene_test_framework::util::LuceneTestCase;

use randomizedtesting::jdk::reflect::Constructor;

use crate::memory_support::core_support::random;
use crate::memory_support::search_support::{ftest, set_up};

/// The instance of `TestMemoryIndexAgainstDirectory`.
#[derive(Default)]
pub struct AgainstDirectoryTest {
    queries: HashSet<String>,
}

/// `TestMemoryIndexAgainstDirectory extends BaseTokenStreamTestCase`.
pub fn class_def() -> Arc<Class> {
    let methods = vec![
        set_up(do_set_up),
        ftest("testRandomQueries", test_random_queries),
        ftest("testDocsEnumStart", test_docs_enum_start),
        ftest(
            "testDocsAndPositionsEnumStart",
            test_docs_and_positions_enum_start,
        ),
        ftest("testNullPointerException", test_null_pointer_exception),
        ftest("testPassesIfWrapped", test_passes_if_wrapped),
        ftest(
            "testSameFieldAddedMultipleTimes",
            test_same_field_added_multiple_times,
        ),
        ftest("testNonExistentField", test_non_existent_field),
        ftest(
            "testDocValuesMemoryIndexVsNormalIndex",
            test_doc_values_memory_index_vs_normal_index,
        ),
        ftest("testNormsWithDocValues", test_norms_with_doc_values),
        ftest(
            "testPointValuesMemoryIndexVsNormalIndex",
            test_point_values_memory_index_vs_normal_index,
        ),
        ftest("testDuellMemIndex", test_duell_mem_index),
        ftest("testEmptyString", test_empty_string),
        ftest(
            "testDuelMemoryIndexCoreDirectoryWithArrayField",
            test_duel_memory_index_core_directory_with_array_field,
        ),
    ];
    let mut builder =
        Class::builder("org.apache.lucene.index.memory.TestMemoryIndexAgainstDirectory")
            .extends(&BaseTokenStreamTestCase::class())
            .constructor(Constructor::no_args(AgainstDirectoryTest::default));
    for m in methods {
        builder = builder.method(m);
    }
    builder.build()
}

fn do_set_up(f: &mut AgainstDirectoryTest) -> Result<()> {
    f.queries
        .extend(read_queries(include_str!("../../fixtures/testqueries.txt")));
    f.queries.extend(read_queries(include_str!(
        "../../fixtures/testqueries2.txt"
    )));
    Ok(())
}

/// read a set of queries from a resource file
fn read_queries(resource: &str) -> HashSet<String> {
    let mut queries = HashSet::new();
    for line in resource.lines() {
        // `String.trim()`: the leading and trailing chars <= U+0020
        let line = line.trim_matches(|c: char| c <= '\u{20}');
        if !line.is_empty() && !line.starts_with('#') && !line.starts_with("//") {
            queries.insert(line.to_string());
        }
    }
    queries
}

/// runs random tests, up to ITERATIONS times.
fn test_random_queries(f: &mut AgainstDirectoryTest) -> Result<()> {
    let mut index = random_memory_index();
    let iterations = if *TEST_NIGHTLY {
        100 * *RANDOM_MULTIPLIER
    } else {
        10 * *RANDOM_MULTIPLIER
    };
    for _i in 0..iterations {
        assert_against_directory(f, &mut index)?;
    }
    Ok(())
}

/// Build a randomish document for both Directory and MemoryIndex, and run
/// all the queries against it.
fn assert_against_directory(f: &AgainstDirectoryTest, memory: &mut MemoryIndex) -> Result<()> {
    memory.reset();
    let mut foo_field = String::new();
    let mut term_field = String::new();

    // add up to 250 terms to field "foo"
    let num_foo_terms = random().next_int_bound(250 * *RANDOM_MULTIPLIER);
    for _i in 0..num_foo_terms {
        foo_field.push(' ');
        foo_field.push_str(&random_term());
    }

    // add up to 250 terms to field "term"
    let num_term_terms = random().next_int_bound(250 * *RANDOM_MULTIPLIER);
    for _i in 0..num_term_terms {
        term_field.push(' ');
        term_field.push_str(&random_term());
    }

    let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
    let analyzer = random_analyzer();
    let config = IndexWriterConfig::with_analyzer(Some(Arc::clone(&analyzer)));
    config.set_codec(TestUtil::always_postings_format(
        TestUtil::get_default_postings_format(),
    ))?;
    let writer = IndexWriter::new(Arc::clone(&dir), config)?;
    let mut doc = Document::new();
    let field1 = LuceneTestCase::new_text_field("foo", &foo_field, Store::NO);
    let field2 = LuceneTestCase::new_text_field("term", &term_field, Store::NO);
    doc.add(field1);
    doc.add(field2);
    writer.add_document(&doc)?;
    writer.close()?;

    memory.add_field_text("foo", &foo_field, &*analyzer)?;
    memory.add_field_text("term", &term_field, &*analyzer)?;

    let searcher = memory.create_searcher()?;
    let reader = Arc::clone(searcher.get_index_reader());
    let leaf = searcher.get_leaf_contexts()[0].leaf_reader();
    TestUtil::check_reader(&reader)?;
    let competitor: Arc<dyn IndexReader> = directory_reader::open(Arc::clone(&dir))?;
    duell_readers(&competitor, &leaf)?;
    reader.close()?;
    competitor.close()?;
    assert_all_queries(f, memory, &dir, &analyzer)?;
    dir.close()?;
    Ok(())
}

fn duell_readers(
    other: &Arc<dyn IndexReader>,
    mem_index_reader: &Arc<dyn LeafReader>,
) -> Result<()> {
    let mem_fields = mem_index_reader
        .term_vectors()?
        .get(0)?
        .expect("the term vectors of the document");
    for field in FieldInfos::get_indexed_fields(Arc::clone(other))?.iter() {
        let field: &str = field;
        let mem_terms = mem_fields.terms(field)?;
        let iw_terms = mem_index_reader.terms(field)?;
        match iw_terms {
            None => Assert::assert_true(mem_terms.is_none()),
            Some(iw_terms) => {
                let norm_values = MultiDocValues::get_norm_values(other, field)?;
                let mem_norm_values = mem_index_reader.get_norm_values(field)?;
                if let Some(mut norm_values) = norm_values {
                    // mem idx always computes norms on the fly
                    Assert::assert_true(mem_norm_values.is_some());
                    let mut mem_norm_values = mem_norm_values.unwrap();
                    Assert::assert_equals(0, norm_values.next_doc()?);
                    Assert::assert_equals(0, mem_norm_values.next_doc()?);
                    Assert::assert_equals(norm_values.long_value()?, mem_norm_values.long_value()?);
                }

                Assert::assert_true(mem_terms.is_some());
                let mem_terms = mem_terms.unwrap();
                Assert::assert_equals(iw_terms.doc_count()?, mem_terms.doc_count()?);
                Assert::assert_equals(iw_terms.sum_doc_freq()?, mem_terms.sum_doc_freq()?);
                Assert::assert_equals(
                    iw_terms.sum_total_term_freq()?,
                    mem_terms.sum_total_term_freq()?,
                );
                let mut iw_terms_iter = iw_terms.iterator()?;
                let mut mem_terms_iter = mem_terms.iterator()?;
                if iw_terms.has_positions() {
                    let offsets = iw_terms.has_offsets() && mem_terms.has_offsets();

                    while iw_terms_iter.next()?.is_some() {
                        Assert::assert_true(mem_terms_iter.next()?.is_some());
                        let iw_term = iw_terms_iter.term()?.cloned();
                        Assert::assert_true(iw_term.as_ref() == mem_terms_iter.term()?);
                        let iw_term = iw_term.expect("term");
                        let mut iw_docs_and_pos = iw_terms_iter
                            .postings(None, POSTINGS_ENUM_ALL)?
                            .expect("postings");
                        let mut mem_docs_and_pos = mem_terms_iter
                            .postings(None, POSTINGS_ENUM_ALL)?
                            .expect("postings");
                        while iw_docs_and_pos.next_doc()? != NO_MORE_DOCS {
                            Assert::assert_equals(
                                iw_docs_and_pos.doc_id(),
                                mem_docs_and_pos.next_doc()?,
                            );
                            Assert::assert_equals(
                                iw_docs_and_pos.freq()?,
                                mem_docs_and_pos.freq()?,
                            );
                            for _i in 0..iw_docs_and_pos.freq()? {
                                Assert::assert_equals_msg(
                                    Some(&format!("term: {}", iw_term.utf8_to_string()?)),
                                    iw_docs_and_pos.next_position()?,
                                    mem_docs_and_pos.next_position()?,
                                );
                                if offsets {
                                    Assert::assert_equals(
                                        iw_docs_and_pos.start_offset(),
                                        mem_docs_and_pos.start_offset(),
                                    );
                                    Assert::assert_equals(
                                        iw_docs_and_pos.end_offset(),
                                        mem_docs_and_pos.end_offset(),
                                    );
                                }

                                if iw_terms.has_payloads() {
                                    Assert::assert_true(
                                        iw_docs_and_pos.get_payload()?
                                            == mem_docs_and_pos.get_payload()?,
                                    );
                                }
                            }
                        }
                    }
                } else {
                    while iw_terms_iter.next()?.is_some() {
                        Assert::assert_true(iw_terms_iter.term()? == mem_terms_iter.term()?);
                        let mut iw_docs_and_pos = iw_terms_iter
                            .postings(None, POSTINGS_ENUM_FREQS)?
                            .expect("postings");
                        let mut mem_docs_and_pos = mem_terms_iter
                            .postings(None, POSTINGS_ENUM_FREQS)?
                            .expect("postings");
                        while iw_docs_and_pos.next_doc()? != NO_MORE_DOCS {
                            Assert::assert_equals(
                                iw_docs_and_pos.doc_id(),
                                mem_docs_and_pos.next_doc()?,
                            );
                            Assert::assert_equals(
                                iw_docs_and_pos.freq()?,
                                mem_docs_and_pos.freq()?,
                            );
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

/// Run all queries against both the Directory and MemoryIndex, ensuring
/// they are the same.
fn assert_all_queries(
    f: &AgainstDirectoryTest,
    memory: &MemoryIndex,
    directory: &Arc<dyn Directory>,
    analyzer: &Arc<dyn Analyzer>,
) -> Result<()> {
    let reader = directory_reader::open(Arc::clone(directory))?;
    let ram = LuceneTestCase::new_searcher(Arc::clone(&reader));
    let mem = memory.create_searcher()?;
    let mut qp = QueryParser::new(Some("foo"), Some(Arc::clone(analyzer)))?;
    for query in &f.queries {
        let ram_docs = ram.search(qp.parse(query)?, 1)?;
        let mem_docs = mem.search(qp.parse(query)?, 1)?;
        Assert::assert_equals_msg(
            Some(query),
            ram_docs.total_hits.value(),
            mem_docs.total_hits.value(),
        );
    }
    reader.close()?;
    Ok(())
}

/// The anonymous `Analyzer` of `randomAnalyzer` (case 2): a `MockTokenizer`
/// and a [`CrazyTokenFilter`].
#[derive(Debug)]
struct CrazyAnalyzer {
    state: AnalyzerState,
}

impl Analyzer for CrazyAnalyzer {
    fn analyzer_state(&self) -> &AnalyzerState {
        &self.state
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.index.memory.TestMemoryIndexAgainstDirectory$1"
    }

    fn create_components(&self, _field_name: Option<&str>) -> Result<TokenStreamComponents> {
        let tokenizer = Arc::new(AtomicRefCell::new(MockTokenizer::whitespace_lower_case()?));
        let filter = crazy_token_filter(rucene_test_framework::analysis::shared(&tokenizer))?;
        Ok(components(tokenizer, sink(filter)))
    }
}

/// Return a random analyzer (Simple, Stop, Standard) to analyze the terms.
fn random_analyzer() -> Arc<dyn Analyzer> {
    match random().next_int_bound(4) {
        0 => Arc::new(MockAnalyzer::with_automaton(
            &*random(),
            Arc::clone(&SIMPLE),
            true,
        )),
        1 => Arc::new(MockAnalyzer::with_filter(
            &*random(),
            Arc::clone(&SIMPLE),
            true,
            Arc::clone(&ENGLISH_STOPSET),
        )),
        2 => Arc::new(CrazyAnalyzer {
            state: AnalyzerState::new(Box::new(GlobalReuseStrategy)),
        }),
        _ => Arc::new(MockAnalyzer::with_automaton(
            &*random(),
            Arc::clone(&WHITESPACE),
            false,
        )),
    }
}

/// a tokenfilter that makes all terms starting with 't' empty strings
#[derive(Debug)]
struct CrazyTokenFilterLogic {
    term_att: Att,
}

impl TokenFilterLogic for CrazyTokenFilterLogic {
    fn increment_token(
        &mut self,
        _source: &mut AttributeSource,
        input: &mut dyn TokenStream,
    ) -> Result<bool> {
        if input.increment_token()? {
            let mut term = self.term_att.char_term_mut();
            if term.length() > 0 && term.buffer()[0] == 't' {
                term.set_length(0);
            }
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

/// `new CrazyTokenFilter(input)`.
fn crazy_token_filter(
    mut input: Box<dyn TokenStream>,
) -> Result<TokenFilter<CrazyTokenFilterLogic>> {
    let term_att = Att::add(input.attribute_source_mut(), char_term_id())?;
    Ok(TokenFilter::new(input, CrazyTokenFilterLogic { term_att }))
}

/// Some terms to be indexed, in addition to random words. These terms are
/// commonly used in the queries.
const TEST_TERMS: [&str; 23] = [
    "term",
    "Term",
    "tErm",
    "TERM",
    "telm",
    "stop",
    "drop",
    "roll",
    "phrase",
    "a",
    "c",
    "bar",
    "blar",
    "gack",
    "weltbank",
    "worlbank",
    "hello",
    "on",
    "the",
    "apache",
    "Apache",
    "copyright",
    "Copyright",
];

/// half of the time, returns a random term from TEST_TERMS. the other half
/// of the time, returns a random unicode string.
fn random_term() -> String {
    if random().next_boolean() {
        // return a random TEST_TERM
        TEST_TERMS[random().next_int_bound(TEST_TERMS.len() as i32) as usize].to_string()
    } else {
        // return a random unicode term
        TestUtil::random_unicode_string(&*random())
    }
}

/// `(LeafReader) memory.createSearcher().getIndexReader()`, with the reader
/// it is.
fn mem_reader(memory: &MemoryIndex) -> Result<(Arc<dyn IndexReader>, Arc<dyn LeafReader>)> {
    let searcher = memory.create_searcher()?;
    Ok((
        Arc::clone(searcher.get_index_reader()),
        searcher.get_leaf_contexts()[0].leaf_reader(),
    ))
}

fn test_docs_enum_start(_f: &mut AgainstDirectoryTest) -> Result<()> {
    let analyzer = MockAnalyzer::new(&*random());
    let mut memory = MemoryIndex::with_max_reused_bytes(
        random().next_boolean(),
        false,
        random().next_int_bound(50) as i64 * 1024 * 1024,
    );
    memory.add_field_text("foo", "bar", &analyzer)?;
    let (reader, leaf) = mem_reader(&memory)?;
    TestUtil::check_reader(&reader)?;
    let mut disi = TestUtil::docs(
        &*random(),
        &reader,
        "foo",
        &BytesRef::from_char_sequence("bar"),
        None,
        POSTINGS_ENUM_NONE,
    )?
    .expect("postings");
    let docid = disi.doc_id();
    Assert::assert_equals(-1, docid);
    Assert::assert_true(disi.next_doc()? != NO_MORE_DOCS);

    // now reuse and check again
    let mut te = leaf.terms("foo")?.expect("terms").iterator()?;
    Assert::assert_true(te.seek_exact(&BytesRef::from_char_sequence("bar"))?);
    let mut disi = te
        .postings(Some(disi), POSTINGS_ENUM_NONE)?
        .expect("postings");
    let docid = disi.doc_id();
    Assert::assert_equals(-1, docid);
    Assert::assert_true(disi.next_doc()? != NO_MORE_DOCS);
    reader.close()?;
    Ok(())
}

fn random_memory_index() -> MemoryIndex {
    MemoryIndex::with_max_reused_bytes(
        random().next_boolean(),
        random().next_boolean(),
        random().next_int_bound(50) as i64 * 1024 * 1024,
    )
}

fn test_docs_and_positions_enum_start(_f: &mut AgainstDirectoryTest) -> Result<()> {
    let analyzer = MockAnalyzer::new(&*random());
    let num_iters = LuceneTestCase::at_least(3);
    let mut memory = MemoryIndex::with_max_reused_bytes(
        true,
        false,
        random().next_int_bound(50) as i64 * 1024 * 1024,
    );
    for _i in 0..num_iters {
        // check reuse
        memory.add_field_text("foo", "bar", &analyzer)?;
        let (reader, leaf) = mem_reader(&memory)?;
        TestUtil::check_reader(&reader)?;
        Assert::assert_equals(
            1i64,
            leaf.terms("foo")?.expect("terms").sum_total_term_freq()?,
        );
        let mut disi = leaf
            .postings(&Term::from_text("foo", "bar"), POSTINGS_ENUM_ALL)?
            .expect("postings");
        let docid = disi.doc_id();
        Assert::assert_equals(-1, docid);
        Assert::assert_true(disi.next_doc()? != NO_MORE_DOCS);
        Assert::assert_equals(0, disi.next_position()?);
        Assert::assert_equals(0, disi.start_offset());
        Assert::assert_equals(3, disi.end_offset());

        // now reuse and check again
        let mut te = leaf.terms("foo")?.expect("terms").iterator()?;
        Assert::assert_true(te.seek_exact(&BytesRef::from_char_sequence("bar"))?);
        let mut disi = te
            .postings(Some(disi), POSTINGS_ENUM_FREQS)?
            .expect("postings");
        let docid = disi.doc_id();
        Assert::assert_equals(-1, docid);
        Assert::assert_true(disi.next_doc()? != NO_MORE_DOCS);
        reader.close()?;
        memory.reset();
    }
    Ok(())
}

/// `mindex.addField("field", new MockAnalyzer(random()).tokenStream("field",
/// "hello there"))`.
fn add_hello_there(mindex: &mut MemoryIndex) -> Result<()> {
    let analyzer = MockAnalyzer::new(&*random());
    let stream = analyzer.token_stream_from_str("field", "hello there")?;
    let mut stream = stream.borrow_mut();
    mindex.add_field_token_stream("field", &mut *stream)
}

// LUCENE-3831
fn test_null_pointer_exception(_f: &mut AgainstDirectoryTest) -> Result<()> {
    let regex = RegexpQuery::new(Term::from_text("field", "worl."))?;
    let wrappedquery: Arc<dyn SpanQuery> =
        Arc::new(SpanMultiTermQueryWrapper::new(Arc::new(regex)));

    let mut mindex = random_memory_index();
    add_hello_there(&mut mindex)?;

    // This throws an NPE
    Assert::assert_equals_float_msg(
        None,
        0.0,
        mindex.search(wrappedquery as Arc<dyn Query>)?,
        0.00001,
    );
    TestUtil::check_reader(mindex.create_searcher()?.get_index_reader())?;
    Ok(())
}

// LUCENE-3831
fn test_passes_if_wrapped(_f: &mut AgainstDirectoryTest) -> Result<()> {
    let regex = RegexpQuery::new(Term::from_text("field", "worl."))?;
    let inner: Arc<dyn SpanQuery> = Arc::new(SpanMultiTermQueryWrapper::new(Arc::new(regex)));
    let wrappedquery: Arc<dyn SpanQuery> = Arc::new(SpanOrQuery::new(vec![inner])?);

    let mut mindex = random_memory_index();
    add_hello_there(&mut mindex)?;

    // This passes though
    Assert::assert_equals_float_msg(
        None,
        0.0,
        mindex.search(wrappedquery as Arc<dyn Query>)?,
        0.00001,
    );
    TestUtil::check_reader(mindex.create_searcher()?.get_index_reader())?;
    Ok(())
}

fn test_same_field_added_multiple_times(_f: &mut AgainstDirectoryTest) -> Result<()> {
    let mut mindex = random_memory_index();
    let mock_analyzer = MockAnalyzer::new(&*random());
    mindex.add_field_text("field", "the quick brown fox", &mock_analyzer)?;
    mindex.add_field_text("field", "jumps over the", &mock_analyzer)?;
    let (reader, leaf) = mem_reader(&mindex)?;
    TestUtil::check_reader(&reader)?;
    Assert::assert_equals(
        7i64,
        leaf.terms("field")?.expect("terms").sum_total_term_freq()?,
    );
    let query: Arc<dyn Query> = Arc::new(PhraseQuery::from_strings("field", &["fox", "jumps"])?);
    Assert::assert_true(mindex.search(Arc::clone(&query))? as f64 > 0.1);
    mindex.reset();
    mock_analyzer.set_position_increment_gap(1 + random().next_int_bound(10));
    mindex.add_field_text("field", "the quick brown fox", &mock_analyzer)?;
    mindex.add_field_text("field", "jumps over the", &mock_analyzer)?;
    Assert::assert_equals_float_msg(None, 0.0, mindex.search(Arc::clone(&query))?, 0.00001);
    let query: Arc<dyn Query> = Arc::new(PhraseQuery::with_slop(10, "field", &["fox", "jumps"])?);
    Assert::assert_true_msg(
        &format!(
            "posGap{}",
            mock_analyzer.get_position_increment_gap(Some("field"))
        ),
        mindex.search(query)? as f64 > 0.0001,
    );
    TestUtil::check_reader(mindex.create_searcher()?.get_index_reader())?;
    Ok(())
}

fn test_non_existent_field(_f: &mut AgainstDirectoryTest) -> Result<()> {
    let mut mindex = random_memory_index();
    let mock_analyzer = MockAnalyzer::new(&*random());
    mindex.add_field_text("field", "the quick brown fox", &mock_analyzer)?;
    let (reader, leaf) = mem_reader(&mindex)?;
    TestUtil::check_reader(&reader)?;
    Assert::assert_true(leaf.get_numeric_doc_values("not-in-index")?.is_none());
    Assert::assert_true(leaf.get_norm_values("not-in-index")?.is_none());
    Assert::assert_true(
        leaf.postings(&Term::from_text("not-in-index", "foo"), POSTINGS_ENUM_FREQS)?
            .is_none(),
    );
    Assert::assert_true(
        leaf.postings(&Term::from_text("not-in-index", "foo"), POSTINGS_ENUM_ALL)?
            .is_none(),
    );
    Assert::assert_true(leaf.terms("not-in-index")?.is_none());
    Ok(())
}

/// `new BytesRef(randomTerm())`.
fn random_term_bytes() -> BytesRef {
    BytesRef::from_char_sequence(&random_term())
}

fn test_doc_values_memory_index_vs_normal_index(_f: &mut AgainstDirectoryTest) -> Result<()> {
    let mut doc = Document::new();
    let mut random_long = random().next_long();
    doc.add(NumericDocValuesField::new("numeric", random_long));
    let mut num_values = LuceneTestCase::at_least(5);
    for _i in 0..num_values {
        random_long = random().next_long();
        doc.add(SortedNumericDocValuesField::new(
            "sorted_numeric",
            random_long,
        ));
        if random().next_boolean() {
            // randomly duplicate field/value
            doc.add(SortedNumericDocValuesField::new(
                "sorted_numeric",
                random_long,
            ));
        }
    }
    let mut random_term = random_term_bytes();
    doc.add(BinaryDocValuesField::new("binary", random_term.clone()));
    if random().next_boolean() {
        doc.add(StringField::new_with_bytes(
            "binary",
            random_term.clone(),
            Store::NO,
        )?);
    }
    random_term = random_term_bytes();
    doc.add(SortedDocValuesField::new("sorted", random_term.clone()));
    if random().next_boolean() {
        doc.add(StringField::new_with_bytes(
            "sorted",
            random_term.clone(),
            Store::NO,
        )?);
    }
    num_values = LuceneTestCase::at_least(5);
    for _i in 0..num_values {
        random_term = random_term_bytes();
        doc.add(SortedSetDocValuesField::new(
            "sorted_set",
            random_term.clone(),
        ));
        if random().next_boolean() {
            // randomly duplicate field/value
            doc.add(SortedSetDocValuesField::new(
                "sorted_set",
                random_term.clone(),
            ));
        }
        if random().next_boolean() {
            // randomly just add a normal string field
            doc.add(StringField::new_with_bytes(
                "sorted_set",
                random_term.clone(),
                Store::NO,
            )?);
        }
    }

    let mock_analyzer = Arc::new(MockAnalyzer::new(&*random()));
    let memory_index = MemoryIndex::from_document(
        doc.get_fields().iter().map(|f| &**f as &dyn IndexableField),
        Some(&*mock_analyzer),
    )?;
    let searcher = memory_index.create_searcher()?;
    let index_reader = Arc::clone(searcher.get_index_reader());
    let leaf_reader = searcher.get_leaf_contexts()[0].leaf_reader();

    let dir = LuceneTestCase::new_directory();
    let writer = IndexWriter::new(
        dir.as_directory(),
        LuceneTestCase::new_index_writer_config_with(&*random(), Some(mock_analyzer.clone())),
    )?;
    writer.add_document(&doc)?;
    writer.close()?;
    let control_index_reader: Arc<dyn IndexReader> = directory_reader::open(dir.as_directory())?;
    let control_leaf_reader = Arc::clone(&control_index_reader).leaves()?[0].leaf_reader();

    let mut numeric_doc_values = leaf_reader.get_numeric_doc_values("numeric")?.expect("dv");
    let mut control_numeric_doc_values = control_leaf_reader
        .get_numeric_doc_values("numeric")?
        .expect("dv");
    Assert::assert_equals(0, numeric_doc_values.next_doc()?);
    Assert::assert_equals(0, control_numeric_doc_values.next_doc()?);
    Assert::assert_equals(
        control_numeric_doc_values.long_value()?,
        numeric_doc_values.long_value()?,
    );

    let mut sorted_numeric_doc_values = leaf_reader
        .get_sorted_numeric_doc_values("sorted_numeric")?
        .expect("dv");
    Assert::assert_equals(0, sorted_numeric_doc_values.next_doc()?);
    let mut control_sorted_numeric_doc_values = control_leaf_reader
        .get_sorted_numeric_doc_values("sorted_numeric")?
        .expect("dv");
    Assert::assert_equals(0, control_sorted_numeric_doc_values.next_doc()?);
    Assert::assert_equals(
        control_sorted_numeric_doc_values.doc_value_count()?,
        sorted_numeric_doc_values.doc_value_count()?,
    );
    for _i in 0..control_sorted_numeric_doc_values.doc_value_count()? {
        Assert::assert_equals(
            control_sorted_numeric_doc_values.next_value()?,
            sorted_numeric_doc_values.next_value()?,
        );
    }

    let mut binary_doc_values = leaf_reader.get_binary_doc_values("binary")?.expect("dv");
    let mut control_binary_doc_values = control_leaf_reader
        .get_binary_doc_values("binary")?
        .expect("dv");
    Assert::assert_equals(0, binary_doc_values.next_doc()?);
    Assert::assert_equals(0, control_binary_doc_values.next_doc()?);
    Assert::assert_true(
        control_binary_doc_values.binary_value()? == binary_doc_values.binary_value()?,
    );

    let mut sorted_doc_values = leaf_reader.get_sorted_doc_values("sorted")?.expect("dv");
    let mut control_sorted_doc_values = control_leaf_reader
        .get_sorted_doc_values("sorted")?
        .expect("dv");
    Assert::assert_equals(
        control_sorted_doc_values.get_value_count()?,
        sorted_doc_values.get_value_count()?,
    );
    Assert::assert_equals(0, sorted_doc_values.next_doc()?);
    Assert::assert_equals(0, control_sorted_doc_values.next_doc()?);
    Assert::assert_equals(
        control_sorted_doc_values.ord_value()?,
        sorted_doc_values.ord_value()?,
    );
    Assert::assert_true(
        control_sorted_doc_values.lookup_ord(0)? == sorted_doc_values.lookup_ord(0)?,
    );

    let mut sorted_set_doc_values = leaf_reader
        .get_sorted_set_doc_values("sorted_set")?
        .expect("dv");
    Assert::assert_equals(0, sorted_set_doc_values.next_doc()?);
    let mut control_sorted_set_doc_values = control_leaf_reader
        .get_sorted_set_doc_values("sorted_set")?
        .expect("dv");
    Assert::assert_equals(0, control_sorted_set_doc_values.next_doc()?);
    Assert::assert_equals(
        control_sorted_set_doc_values.get_value_count()?,
        sorted_set_doc_values.get_value_count()?,
    );
    for _i in 0..control_sorted_set_doc_values.doc_value_count()? {
        let control_ord = control_sorted_set_doc_values.next_ord()?;
        Assert::assert_equals(control_ord, sorted_set_doc_values.next_ord()?);
        Assert::assert_true(
            control_sorted_set_doc_values.lookup_ord(control_ord)?
                == sorted_set_doc_values.lookup_ord(control_ord)?,
        );
    }

    index_reader.close()?;
    control_index_reader.close()?;
    dir.close()?;
    Ok(())
}

fn test_norms_with_doc_values(_f: &mut AgainstDirectoryTest) -> Result<()> {
    let mut mi = MemoryIndex::with_offsets_and_payloads(true, true);
    let mock_analyzer = Arc::new(MockAnalyzer::new(&*random()));

    mi.add_field(
        &BinaryDocValuesField::new("text", BytesRef::from_char_sequence("quick brown fox")),
        Some(&*mock_analyzer),
    )?;
    mi.add_field(
        &TextField::new("text", "quick brown fox".to_string(), Store::NO)?,
        Some(&*mock_analyzer),
    )?;
    let leaf_reader = mi.create_searcher()?.get_leaf_contexts()[0].leaf_reader();

    let mut doc = Document::new();
    doc.add(BinaryDocValuesField::new(
        "text",
        BytesRef::from_char_sequence("quick brown fox"),
    ));
    let field = TextField::new("text", "quick brown fox".to_string(), Store::NO)?;
    doc.add(field);
    let dir = LuceneTestCase::new_directory();
    let writer = IndexWriter::new(
        dir.as_directory(),
        LuceneTestCase::new_index_writer_config_with(&*random(), Some(mock_analyzer.clone())),
    )?;
    writer.add_document(&doc)?;
    writer.close()?;

    let control_index_reader: Arc<dyn IndexReader> = directory_reader::open(dir.as_directory())?;
    let control_leaf_reader = Arc::clone(&control_index_reader).leaves()?[0].leaf_reader();

    let mut norms = control_leaf_reader.get_norm_values("text")?.expect("norms");
    Assert::assert_equals(0, norms.next_doc()?);
    let mut norms2 = leaf_reader.get_norm_values("text")?.expect("norms");
    Assert::assert_equals(0, norms2.next_doc()?);
    Assert::assert_equals(norms.long_value()?, norms2.long_value()?);

    control_index_reader.close()?;
    dir.close()?;
    Ok(())
}

fn test_point_values_memory_index_vs_normal_index(_f: &mut AgainstDirectoryTest) -> Result<()> {
    let size = LuceneTestCase::at_least(12);

    let mut random_values: Vec<i32> = Vec::new();

    let mut doc = Document::new();
    // `random().ints(size).toArray()`
    let ints: Vec<i32> = (0..size).map(|_| random().next_int()).collect();
    for random_integer in ints {
        doc.add(IntPoint::new("int", &[random_integer])?);
        random_values.push(random_integer);
        doc.add(LongPoint::new("long", &[random_integer as i64])?);
        doc.add(FloatPoint::new("float", &[random_integer as f32])?);
        doc.add(DoublePoint::new("double", &[random_integer as f64])?);
    }

    let mock_analyzer = Arc::new(MockAnalyzer::new(&*random()));
    let memory_index = MemoryIndex::from_document(
        doc.get_fields().iter().map(|f| &**f as &dyn IndexableField),
        Some(&*mock_analyzer),
    )?;
    let memory_index_searcher = memory_index.create_searcher()?;

    let dir = LuceneTestCase::new_directory();
    let writer = IndexWriter::new(
        dir.as_directory(),
        LuceneTestCase::new_index_writer_config_with(&*random(), Some(mock_analyzer.clone())),
    )?;
    writer.add_document(&doc)?;
    writer.close()?;
    let control_index_reader: Arc<dyn IndexReader> = directory_reader::open(dir.as_directory())?;
    let control_index_searcher = IndexSearcher::new(Arc::clone(&control_index_reader))?;

    let value_supplier =
        || random_values[random().next_int_bound(random_values.len() as i32) as usize];
    let queries: Vec<Arc<dyn Query>> = vec![
        IntPoint::new_exact_query("int", value_supplier())?,
        LongPoint::new_exact_query("long", value_supplier() as i64)?,
        FloatPoint::new_exact_query("float", value_supplier() as f32)?,
        DoublePoint::new_exact_query("double", value_supplier() as f64)?,
        IntPoint::new_set_query("int", &[value_supplier(), value_supplier()])?,
        LongPoint::new_set_query("long", &[value_supplier() as i64, value_supplier() as i64])?,
        FloatPoint::new_set_query("float", &[value_supplier() as f32, value_supplier() as f32])?,
        DoublePoint::new_set_query(
            "double",
            &[value_supplier() as f64, value_supplier() as f64],
        )?,
        IntPoint::new_range_query("int", &[value_supplier()], &[value_supplier()])?,
        LongPoint::new_range_query(
            "long",
            &[value_supplier() as i64],
            &[value_supplier() as i64],
        )?,
        FloatPoint::new_range_query(
            "float",
            &[value_supplier() as f32],
            &[value_supplier() as f32],
        )?,
        DoublePoint::new_range_query(
            "double",
            &[value_supplier() as f64],
            &[value_supplier() as f64],
        )?,
    ];
    for query in queries {
        Assert::assert_equals(
            control_index_searcher.count(Arc::clone(&query))?,
            control_index_searcher.count(query)?,
        );
    }

    memory_index_searcher.get_index_reader().close()?;
    control_index_reader.close()?;
    dir.close()?;
    Ok(())
}

fn test_duell_mem_index(_f: &mut AgainstDirectoryTest) -> Result<()> {
    let line_file_docs = LineFileDocs::with_default_file(&*random())?;
    let num_docs = LuceneTestCase::at_least(10);
    let mut memory = random_memory_index();
    for _i in 0..num_docs {
        let dir = LuceneTestCase::new_directory();
        let mock_analyzer = Arc::new(MockAnalyzer::new(&*random()));
        mock_analyzer.set_max_token_length(TestUtil::next_int(
            &*random(),
            1,
            MAX_TERM_LENGTH as i32,
        ));
        let writer = IndexWriter::new(
            dir.as_directory(),
            LuceneTestCase::new_index_writer_config_with(&*random(), Some(mock_analyzer.clone())),
        )?;
        let next_doc = line_file_docs.next_doc()?;
        let mut doc = Document::new();
        for field in next_doc.borrow().get_fields() {
            if field.field_type().index_options() != IndexOptions::NONE {
                doc.add_shared(Arc::clone(field));
                if random().next_int_bound(3) == 0 {
                    doc.add_shared(Arc::clone(field)); // randomly add the same field twice
                }
            }
        }

        writer.add_document(&doc)?;
        writer.close()?;
        for field in doc.get_fields() {
            memory.add_field_text(
                field.name(),
                &field.string_value().expect("a string value"),
                &*mock_analyzer,
            )?;
        }
        let competitor: Arc<dyn IndexReader> = directory_reader::open(dir.as_directory())?;
        let (mem_reader, mem_index_reader) = mem_reader(&memory)?;
        TestUtil::check_reader(&mem_reader)?;
        duell_readers(&competitor, &mem_index_reader)?;
        competitor.close()?;
        mem_reader.close()?;
        memory.reset();
        dir.close()?;
    }
    line_file_docs.close();
    Ok(())
}

// LUCENE-4880
fn test_empty_string(_f: &mut AgainstDirectoryTest) -> Result<()> {
    let mut memory = MemoryIndex::new();
    let mut stream = CannedTokenStream::new(vec![Token::with_offsets("", 0, 5)]);
    memory.add_field_token_stream("foo", &mut stream)?;
    let searcher = memory.create_searcher()?;
    let docs = searcher.search(Arc::new(TermQuery::new(Term::from_text("foo", ""))), 10)?;
    Assert::assert_equals(1i64, docs.total_hits.value());
    TestUtil::check_reader(searcher.get_index_reader())?;
    Ok(())
}

fn test_duel_memory_index_core_directory_with_array_field(
    _f: &mut AgainstDirectoryTest,
) -> Result<()> {
    let field_name = "text";
    let mock_analyzer = Arc::new(MockAnalyzer::new(&*random()));
    if random().next_boolean() {
        mock_analyzer.set_offset_gap(random().next_int_bound(100));
    }
    // index into a random directory
    let ty = FieldType::new_from(TextField::type_stored());
    ty.set_store_term_vector_offsets(true)?;
    ty.set_store_term_vector_payloads(false)?;
    ty.set_store_term_vector_positions(true)?;
    ty.set_store_term_vectors(true)?;
    ty.freeze();
    let ty = Arc::new(ty);

    let mut doc = Document::new();
    doc.add(Field::new(
        field_name,
        "la la".to_string(),
        Arc::clone(&ty),
    )?);
    doc.add(Field::new(
        field_name,
        "foo bar foo bar foo".to_string(),
        Arc::clone(&ty),
    )?);

    let dir = LuceneTestCase::new_directory();
    let writer = IndexWriter::new(
        dir.as_directory(),
        LuceneTestCase::new_index_writer_config_with(&*random(), Some(mock_analyzer.clone())),
    )?;
    writer.update_document(Term::from_text("id", "1"), &doc)?;
    writer.commit()?;
    writer.close()?;
    let reader: Arc<dyn IndexReader> = directory_reader::open(dir.as_directory())?;

    // Index document in Memory index
    let mut mem_index = MemoryIndex::with_offsets(true);
    mem_index.add_field_text(field_name, "la la", &*mock_analyzer)?;
    mem_index.add_field_text(field_name, "foo bar foo bar foo", &*mock_analyzer)?;

    // compare term vectors
    let ram_tv = reader
        .term_vectors()?
        .get_field(0, field_name)?
        .expect("tv");
    let mem_index_reader = Arc::clone(mem_index.create_searcher()?.get_index_reader());
    TestUtil::check_reader(&mem_index_reader)?;
    let mem_tv = mem_index_reader
        .term_vectors()?
        .get_field(0, field_name)?
        .expect("tv");

    compare_term_vectors(&*ram_tv, &*mem_tv, field_name)?;
    mem_index_reader.close()?;
    reader.close()?;
    dir.close()?;
    Ok(())
}

fn compare_term_vectors(terms: &dyn Terms, mem_terms: &dyn Terms, field_name: &str) -> Result<()> {
    let mut term_enum = terms.iterator()?;
    let mut mem_term_enum = mem_terms.iterator()?;

    while term_enum.next()?.is_some() {
        Assert::assert_true(mem_term_enum.next()?.is_some());
        Assert::assert_equals(
            mem_term_enum.total_term_freq()?,
            term_enum.total_term_freq()?,
        );

        let mut docs_pos_enum = term_enum
            .postings(None, POSTINGS_ENUM_POSITIONS)?
            .expect("postings");
        let mut mem_docs_pos_enum = mem_term_enum
            .postings(None, POSTINGS_ENUM_POSITIONS)?
            .expect("postings");
        let current_term = term_enum.term()?.expect("term").utf8_to_string()?;

        Assert::assert_equals_msg(
            Some(&format!("Token mismatch for field: {field_name}")),
            mem_term_enum.term()?.expect("term").utf8_to_string()?,
            current_term.clone(),
        );

        docs_pos_enum.next_doc()?;
        mem_docs_pos_enum.next_doc()?;

        let freq = docs_pos_enum.freq()?;
        Assert::assert_equals(mem_docs_pos_enum.freq()?, freq);
        for _i in 0..freq {
            let fail_desc = format!(" (field:{field_name} term:{current_term})");
            let mem_pos = mem_docs_pos_enum.next_position()?;
            let pos = docs_pos_enum.next_position()?;
            Assert::assert_equals_msg(
                Some(&format!("Position test failed{fail_desc}")),
                pos,
                mem_pos,
            );
            Assert::assert_equals_msg(
                Some(&format!("Start offset test failed{fail_desc}")),
                docs_pos_enum.start_offset(),
                mem_docs_pos_enum.start_offset(),
            );
            Assert::assert_equals_msg(
                Some(&format!("End offset test failed{fail_desc}")),
                docs_pos_enum.end_offset(),
                mem_docs_pos_enum.end_offset(),
            );
            Assert::assert_true_msg(
                &format!("Missing payload test failed{fail_desc}"),
                docs_pos_enum.get_payload()? == mem_docs_pos_enum.get_payload()?,
            );
        }
    }
    Assert::assert_true_msg(
        "Still some tokens not processed",
        mem_term_enum.next()?.is_none(),
    );
    Ok(())
}
