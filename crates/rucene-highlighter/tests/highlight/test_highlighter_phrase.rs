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

//! Port of `TestHighlighterPhrase.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::analysis::{default_token_attribute_factory, Analyzer, TokenStream};
use rucene::document::{Document, Field, FieldType, Store, TextField};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::IndexReader;
use rucene::search::phrase_query::{Builder as PhraseQueryBuilder, PhraseQuery};
use rucene::search::query::Query;
use rucene::util::atomic_ref_cell::AtomicRefCell;
use rucene::util::attribute::AttributeSource;
use rucene::util::bit_set::BitSet;
use rucene_highlighter::highlight::highlighter::Highlighter;
use rucene_highlighter::highlight::query_scorer::QueryScorer;
use rucene_highlighter::highlight::simple_html_encoder::SimpleHTMLEncoder;
use rucene_highlighter::highlight::simple_html_formatter::SimpleHTMLFormatter;
use rucene_highlighter::highlight::token_sources::TokenSources;
use rucene_queries::spans::span_near_query::SpanNearQuery;
use rucene_queries::spans::span_query::SpanQuery;
use rucene_queries::spans::span_term_query::SpanTermQuery;
use rucene_test_framework::analysis::attributes::{char_term_id, offset_id, pos_inc_id, Att};
use rucene_test_framework::analysis::mock_token_filter::ENGLISH_STOPSET;
use rucene_test_framework::analysis::mock_tokenizer::WHITESPACE;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::search::fixed_bit_set_collector::FixedBitSetCollector;
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::LuceneTestCase;

use crate::highlighter_support::attempt;
use crate::highlighter_support::core_support::{class, random, test};
use crate::highlighter_support::search_support::term;

const FIELD: &str = "text";

/// `TestHighlighterPhrase`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.highlight.TestHighlighterPhrase",
        vec![
            test("testConcurrentPhrase", test_concurrent_phrase),
            test("testConcurrentSpan", test_concurrent_span),
            test("testSparsePhrase", test_sparse_phrase),
            test(
                "testSparsePhraseWithNoPositions",
                test_sparse_phrase_with_no_positions,
            ),
            test("testSparseSpan", test_sparse_span),
            test("testStopWords", test_stop_words),
            test("testInOrderWithStopWords", test_in_order_with_stop_words),
        ],
    )
}

/// `new FieldType(TextField.TYPE_NOT_STORED)` with term vectors, positions
/// and offsets.
fn tv_type() -> Result<Arc<FieldType>> {
    let custom_type = FieldType::new_from(TextField::type_not_stored());
    custom_type.set_store_term_vector_offsets(true)?;
    custom_type.set_store_term_vector_positions(true)?;
    custom_type.set_store_term_vectors(true)?;
    Ok(Arc::new(custom_type))
}

/// Indexes one document holding `field` with an
/// `IndexWriter(newIndexWriterConfig(analyzer))`.
fn index_one(directory: &TestDirectory, analyzer: Arc<dyn Analyzer>, field: Field) -> Result<()> {
    let index_writer = IndexWriter::new(
        directory.as_directory(),
        LuceneTestCase::new_index_writer_config_analyzer(Some(analyzer)),
    )?;
    let result = attempt(|| {
        let mut document = Document::new();
        document.add(field);
        index_writer.add_document(&document)
    });
    index_writer.close()?;
    result.map(|_| ())
}

fn whitespace_analyzer() -> Arc<dyn Analyzer> {
    Arc::new(MockAnalyzer::with_automaton(
        &*random(),
        Arc::clone(&WHITESPACE),
        false,
    ))
}

fn stream_field(stream: FixedTokenStream) -> Result<Field> {
    let stream: Arc<AtomicRefCell<dyn TokenStream>> = Arc::new(AtomicRefCell::new(stream));
    Field::new_with_token_stream(FIELD, stream, tv_type()?)
}

fn highlighter(query: Arc<dyn Query>) -> Highlighter {
    Highlighter::with_formatter_and_encoder(
        Box::new(SimpleHTMLFormatter::new()),
        Box::new(SimpleHTMLEncoder::new()),
        Box::new(QueryScorer::new(query)),
    )
}

/// `TokenSources.getTermVectorTokenStreamOrNull(FIELD,
/// indexReader.termVectors().get(doc), -1)`.
fn tv_stream(index_reader: &dyn IndexReader, doc: i32) -> Result<Box<dyn TokenStream>> {
    let fields = index_reader.term_vectors()?.get(doc)?;
    Ok(
        TokenSources::get_term_vector_token_stream_or_null(FIELD, fields.as_deref(), -1)?
            .expect("NullPointerException: no term vector token stream"),
    )
}

fn assert_same_fragment(
    highlighter: &mut Highlighter,
    expected_stream: FixedTokenStream,
    token_stream: Box<dyn TokenStream>,
    text: &str,
) -> Result<()> {
    let expected = highlighter.get_best_fragment_from_stream(Box::new(expected_stream), text)?;
    let actual = highlighter.get_best_fragment_from_stream(token_stream, text)?;
    Assert::assert_equals_str(None, expected.as_deref(), actual.as_deref());
    Ok(())
}

fn span_near(first: &str, second: &str) -> Result<Arc<dyn Query>> {
    let clauses: Vec<Arc<dyn SpanQuery>> = vec![
        Arc::new(SpanTermQuery::new(term(FIELD, first))),
        Arc::new(SpanTermQuery::new(term(FIELD, second))),
    ];
    Ok(Arc::new(SpanNearQuery::new(clauses, 0, true)?))
}

fn test_concurrent_phrase() -> Result<()> {
    let text = "the fox jumped";
    let directory = LuceneTestCase::new_directory();
    index_one(
        &directory,
        whitespace_analyzer(),
        stream_field(FixedTokenStream::concurrent())?,
    )?;
    let index_reader = directory_reader::open(directory.as_directory())?;
    let result = attempt(|| {
        Assert::assert_equals(1, index_reader.num_docs());
        let index_searcher = LuceneTestCase::new_searcher(Arc::clone(&index_reader));
        let phrase_query: Arc<dyn Query> =
            Arc::new(PhraseQuery::with_slop(0, FIELD, &["fox", "jumped"])?);
        let hits = index_searcher.search(Arc::clone(&phrase_query), 1)?;
        Assert::assert_equals(1, hits.total_hits.value());
        let mut highlighter = highlighter(phrase_query);

        let token_stream = tv_stream(&*index_reader, 0)?;
        assert_same_fragment(
            &mut highlighter,
            FixedTokenStream::concurrent(),
            token_stream,
            text,
        )
    });
    index_reader.close()?;
    directory.close()?;
    result
}

fn test_concurrent_span() -> Result<()> {
    let text = "the fox jumped";
    let directory = LuceneTestCase::new_directory();
    index_one(
        &directory,
        whitespace_analyzer(),
        stream_field(FixedTokenStream::concurrent())?,
    )?;
    let index_reader = directory_reader::open(directory.as_directory())?;
    let result = attempt(|| {
        Assert::assert_equals(1, index_reader.num_docs());
        let index_searcher = LuceneTestCase::new_searcher(Arc::clone(&index_reader));
        let phrase_query = span_near("fox", "jumped")?;
        let bitset = index_searcher.search_with_manager(
            Arc::clone(&phrase_query),
            &FixedBitSetCollector::create_manager(index_reader.max_doc()),
        )?;
        Assert::assert_equals(1, bitset.cardinality() as i64);
        let max_doc = index_reader.max_doc();
        let mut highlighter = highlighter(phrase_query);
        let next_set_bit = |i: i32| bitset.next_set_bit(i);
        let mut position = next_set_bit(0);
        while position < max_doc - 1 {
            Assert::assert_equals(0, position);
            let token_stream = tv_stream(&*index_reader, position)?;
            assert_same_fragment(
                &mut highlighter,
                FixedTokenStream::concurrent(),
                token_stream,
                text,
            )?;
            position = next_set_bit(position + 1);
        }
        Ok(())
    });
    index_reader.close()?;
    directory.close()?;
    result
}

fn test_sparse_phrase() -> Result<()> {
    let text = "the fox did not jump";
    let directory = LuceneTestCase::new_directory();
    index_one(
        &directory,
        whitespace_analyzer(),
        stream_field(FixedTokenStream::sparse())?,
    )?;
    let index_reader = directory_reader::open(directory.as_directory())?;
    let result = attempt(|| {
        Assert::assert_equals(1, index_reader.num_docs());
        let index_searcher = LuceneTestCase::new_searcher(Arc::clone(&index_reader));
        let phrase_query: Arc<dyn Query> =
            Arc::new(PhraseQuery::with_slop(0, FIELD, &["did", "jump"])?);
        let hits = index_searcher.search(Arc::clone(&phrase_query), 1)?;
        Assert::assert_equals(0, hits.total_hits.value());
        let mut highlighter = highlighter(phrase_query);
        let token_stream = tv_stream(&*index_reader, 0)?;
        assert_same_fragment(
            &mut highlighter,
            FixedTokenStream::sparse(),
            token_stream,
            text,
        )
    });
    index_reader.close()?;
    directory.close()?;
    result
}

fn test_sparse_phrase_with_no_positions() -> Result<()> {
    let text = "the fox did not jump";
    let directory = LuceneTestCase::new_directory();
    let custom_type = FieldType::new_from(TextField::type_stored());
    custom_type.set_store_term_vector_offsets(true)?;
    custom_type.set_store_term_vectors(true)?;
    index_one(
        &directory,
        whitespace_analyzer(),
        Field::new(FIELD, text.to_string(), Arc::new(custom_type))?,
    )?;
    let index_reader = directory_reader::open(directory.as_directory())?;
    let result = attempt(|| {
        Assert::assert_equals(1, index_reader.num_docs());
        let index_searcher = LuceneTestCase::new_searcher(Arc::clone(&index_reader));
        let phrase_query: Arc<dyn Query> =
            Arc::new(PhraseQuery::with_slop(1, FIELD, &["did", "jump"])?);
        let hits = index_searcher.search(Arc::clone(&phrase_query), 1)?;
        Assert::assert_equals(1, hits.total_hits.value());
        let mut highlighter = highlighter(phrase_query);
        let token_stream = tv_stream(&*index_reader, 0)?;
        Assert::assert_equals_str(
            None,
            Some("the fox <B>did</B> not <B>jump</B>"),
            highlighter
                .get_best_fragment_from_stream(token_stream, text)?
                .as_deref(),
        );
        Ok(())
    });
    index_reader.close()?;
    directory.close()?;
    result
}

fn test_sparse_span() -> Result<()> {
    let text = "the fox did not jump";
    let directory = LuceneTestCase::new_directory();
    index_one(
        &directory,
        whitespace_analyzer(),
        stream_field(FixedTokenStream::sparse())?,
    )?;
    let index_reader = directory_reader::open(directory.as_directory())?;
    let result = attempt(|| {
        Assert::assert_equals(1, index_reader.num_docs());
        let index_searcher = LuceneTestCase::new_searcher(Arc::clone(&index_reader));
        let phrase_query = span_near("did", "jump")?;
        let hits = index_searcher.search(Arc::clone(&phrase_query), 1)?;
        Assert::assert_equals(0, hits.total_hits.value());
        let mut highlighter = highlighter(phrase_query);
        let token_stream = tv_stream(&*index_reader, 0)?;
        assert_same_fragment(
            &mut highlighter,
            FixedTokenStream::sparse(),
            token_stream,
            text,
        )
    });
    index_reader.close()?;
    directory.close()?;
    result
}

fn stop_analyzer() -> Arc<MockAnalyzer> {
    Arc::new(MockAnalyzer::with_filter(
        &*random(),
        Arc::clone(&WHITESPACE),
        true,
        Arc::clone(&ENGLISH_STOPSET),
    ))
}

/// The common start of `testStopWords` and `testInOrderWithStopWords`:
/// one document holding `text`, then the hits of `phrase_query` (expected
/// 1) and its best fragments.
fn stop_words_fragments(text: &str, phrase_query: Arc<dyn Query>) -> Result<Vec<String>> {
    let stop_analyzer = stop_analyzer();
    let directory = LuceneTestCase::new_directory();
    index_one(
        &directory,
        Arc::clone(&stop_analyzer) as Arc<dyn Analyzer>,
        LuceneTestCase::new_text_field(FIELD, text, Store::YES),
    )?;
    let result = attempt(|| {
        let index_reader = directory_reader::open(directory.as_directory())?;
        let inner = attempt(|| {
            Assert::assert_equals(1, index_reader.num_docs());
            let index_searcher = LuceneTestCase::new_searcher(Arc::clone(&index_reader));
            let hits = index_searcher.search(Arc::clone(&phrase_query), 100)?;
            Assert::assert_equals(1, hits.total_hits.value());
            let mut highlighter = highlighter(Arc::clone(&phrase_query));
            highlighter.get_best_fragments(&*stop_analyzer, FIELD, text, 10)
        });
        index_reader.close()?;
        inner
    });
    directory.close()?;
    result
}

// shows the need to sum the increments in WeightedSpanTermExtractor
fn test_stop_words() -> Result<()> {
    let text = "the ab the the cd the the the ef the";
    // equivalent of "ab the the cd the the the ef"
    let mut builder = PhraseQueryBuilder::new();
    builder.add_at(term(FIELD, "ab"), 0)?;
    builder.add_at(term(FIELD, "cd"), 3)?;
    builder.add_at(term(FIELD, "ef"), 7)?;
    let phrase_query: Arc<dyn Query> = Arc::new(builder.build()?);
    let frags = stop_words_fragments(text, phrase_query)?;
    Assert::assert_equals(1, frags.len());
    Ok(())
}

// shows the need to require inOrder if getSlop() == 0, not if final slop == 0
// in WeightedSpanTermExtractor
fn test_in_order_with_stop_words() -> Result<()> {
    let text = "the cd the ab the the the the the the the ab the cd the";
    // equivalent of "ab the cd"
    let mut builder = PhraseQueryBuilder::new();
    builder.add_at(term(FIELD, "ab"), 0)?;
    builder.add_at(term(FIELD, "cd"), 2)?;
    let phrase_query: Arc<dyn Query> = Arc::new(builder.build()?);
    let frags = stop_words_fragments(text, phrase_query)?;
    Assert::assert_equals(1, frags.len());
    Assert::assert_true_msg(
        "contains <B>ab</B> the <B>cd</B>",
        frags[0].contains("<B>ab</B> the <B>cd</B>"),
    );
    Assert::assert_true_msg(
        "does not contain <B>cd</B> the <B>ab</B>",
        !frags[0].contains("<B>cd</B> the <B>ab</B>"),
    );
    Ok(())
}

/// `TokenStreamSparse` and `TokenStreamConcurrent`: a stream over fixed
/// tokens (term, start offset, end offset, position increment), restarted
/// by `reset()`.
#[derive(Debug)]
pub(crate) struct FixedTokenStream {
    source: AttributeSource,
    tokens: &'static [(&'static str, i32, i32, i32)],
    i: isize,
    term_attribute: Att,
    offset_attribute: Att,
    position_increment_attribute: Att,
}

impl FixedTokenStream {
    pub(crate) fn new(tokens: &'static [(&'static str, i32, i32, i32)]) -> Self {
        let mut source = AttributeSource::new_with_factory(default_token_attribute_factory());
        let term_attribute =
            Att::add(&mut source, char_term_id()).expect("INVARIANT: a token attribute");
        let offset_attribute =
            Att::add(&mut source, offset_id()).expect("INVARIANT: a token attribute");
        let position_increment_attribute =
            Att::add(&mut source, pos_inc_id()).expect("INVARIANT: a token attribute");
        let mut stream = Self {
            source,
            tokens,
            i: -1,
            term_attribute,
            offset_attribute,
            position_increment_attribute,
        };
        stream.reset().expect("INVARIANT: reset does not fail");
        stream
    }

    /// `new TokenStreamSparse()`.
    fn sparse() -> Self {
        Self::new(&[
            ("the", 0, 3, 1),
            ("fox", 4, 7, 1),
            ("did", 8, 11, 1),
            ("jump", 16, 20, 2),
        ])
    }

    /// `new TokenStreamConcurrent()`.
    fn concurrent() -> Self {
        Self::new(&[
            ("the", 0, 3, 1),
            ("fox", 4, 7, 1),
            ("jump", 8, 14, 1),
            ("jumped", 8, 14, 0),
        ])
    }
}

impl TokenStream for FixedTokenStream {
    fn increment_token(&mut self) -> Result<bool> {
        self.i += 1;
        if self.i >= self.tokens.len() as isize {
            return Ok(false);
        }
        self.source.clear_attributes();
        let (text, start, end, pos_inc) = self.tokens[self.i as usize];
        self.term_attribute.set_term(text);
        self.offset_attribute.offset_mut().set_offset(start, end);
        self.position_increment_attribute
            .pos_inc_mut()
            .set_position_increment(pos_inc);
        Ok(true)
    }

    fn reset(&mut self) -> Result<()> {
        self.i = -1;
        Ok(())
    }

    fn attribute_source(&self) -> &AttributeSource {
        &self.source
    }

    fn attribute_source_mut(&mut self) -> &mut AttributeSource {
        &mut self.source
    }
}
