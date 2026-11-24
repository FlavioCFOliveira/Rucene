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

//! Port of `TestTokenSources.java`.

use std::sync::Arc;

use randomizedtesting::annotations::Repeat;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::analysis::{Analyzer, TokenStream};
use rucene::document::{Document, Field, FieldType, TextField};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::{IndexReader, IndexableFieldType};
use rucene::search::disjunction_max_query::DisjunctionMaxQuery;
use rucene::search::query::Query;
use rucene::util::atomic_ref_cell::AtomicRefCell;
use rucene::util::BytesRef;
use rucene_highlighter::highlight::highlighter::Highlighter;
use rucene_highlighter::highlight::query_scorer::QueryScorer;
use rucene_highlighter::highlight::simple_html_encoder::SimpleHTMLEncoder;
use rucene_highlighter::highlight::simple_html_formatter::SimpleHTMLFormatter;
use rucene_highlighter::highlight::token_sources::TokenSources;
use rucene_queries::spans::span_near_query::SpanNearQuery;
use rucene_queries::spans::span_query::SpanQuery;
use rucene_queries::spans::span_term_query::SpanTermQuery;
use rucene_test_framework::analysis::attributes::{
    char_term_id, offset_id, payload_id, pos_inc_id, Att,
};
use rucene_test_framework::analysis::token::Token;
use rucene_test_framework::analysis::{BaseTokenStreamTestCase, CannedTokenStream, MockAnalyzer};
use rucene_test_framework::index::base_term_vectors_format_test_case::RandomTokenStream;
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::test_util::TestUtil;
use rucene_test_framework::util::LuceneTestCase;

use super::test_highlighter_phrase::FixedTokenStream;
use crate::highlighter_support::attempt;
use crate::highlighter_support::btstc_class;
use crate::highlighter_support::core_support::{random, test, verbose};
use crate::highlighter_support::search_support::term;

const FIELD: &str = "text";

/// `TestTokenSources`.
pub fn class_def() -> Arc<Class> {
    btstc_class(
        "org.apache.lucene.search.highlight.TestTokenSources",
        vec![
            test("testOverlapWithOffset", test_overlap_with_offset),
            test(
                "testOverlapWithPositionsAndOffset",
                test_overlap_with_positions_and_offset,
            ),
            test(
                "testOverlapWithOffsetExactPhrase",
                test_overlap_with_offset_exact_phrase,
            ),
            test(
                "testOverlapWithPositionsAndOffsetExactPhrase",
                test_overlap_with_positions_and_offset_exact_phrase,
            ),
            test(
                "testTermVectorWithoutOffsetsDoesntWork",
                test_term_vector_without_offsets_doesnt_work,
            ),
            test("testPayloads", test_payloads),
            test("testRandomizedRoundTrip", test_randomized_round_trip).annotate(Repeat {
                iterations: 10,
                use_constant_seed: false,
            }),
            test(
                "testMaxStartOffsetConsistency",
                test_max_start_offset_consistency,
            ),
        ],
    )
}

/// `new OverlappingTokenStream()`.
fn overlapping_token_stream() -> FixedTokenStream {
    FixedTokenStream::new(&[
        ("the", 0, 3, 1),
        ("{fox}", 0, 7, 0),
        ("fox", 4, 7, 1),
        ("did", 8, 11, 1),
        ("not", 12, 15, 1),
        ("jump", 16, 20, 1),
    ])
}

/// Indexes one document holding an `OverlappingTokenStream` field of
/// `TYPE_NOT_STORED` with term vectors, offsets (`offsets`) and positions
/// (`positions`), through `new IndexWriter(directory,
/// newIndexWriterConfig(null))`.
fn index_overlapping(directory: &TestDirectory, offsets: bool, positions: bool) -> Result<()> {
    let index_writer = IndexWriter::new(
        directory.as_directory(),
        LuceneTestCase::new_index_writer_config_analyzer(None),
    )?;
    let result = attempt(|| {
        let mut document = Document::new();
        let custom_type = FieldType::new_from(TextField::type_not_stored());
        custom_type.set_store_term_vectors(true)?;
        custom_type.set_store_term_vector_offsets(offsets)?;
        if positions {
            custom_type.set_store_term_vector_positions(true)?;
        }
        let stream: Arc<AtomicRefCell<dyn TokenStream>> =
            Arc::new(AtomicRefCell::new(overlapping_token_stream()));
        document.add(Field::new_with_token_stream(
            FIELD,
            stream,
            Arc::new(custom_type),
        )?);
        index_writer.add_document(&document)
    });
    index_writer.close()?;
    result.map(|_| ())
}

/// `TokenSources.getTermVectorTokenStreamOrNull(field,
/// indexReader.termVectors().get(doc), -1)`.
fn tv_stream(
    index_reader: &dyn IndexReader,
    field: &str,
    doc: i32,
) -> Result<Option<Box<dyn TokenStream>>> {
    let fields = index_reader.term_vectors()?.get(doc)?;
    TokenSources::get_term_vector_token_stream_or_null(field, fields.as_deref(), -1)
}

/// The common body of the four overlap tests: one hit for `query`, and the
/// best fragment of the term-vector stream.
fn check_overlap(positions: bool, query: Arc<dyn Query>) -> Result<()> {
    let text = "the fox did not jump";
    let directory = LuceneTestCase::new_directory();
    index_overlapping(&directory, true, positions)?;
    let index_reader = directory_reader::open(directory.as_directory())?;
    let result = attempt(|| {
        Assert::assert_equals(1, index_reader.num_docs());
        let index_searcher = LuceneTestCase::new_searcher(Arc::clone(&index_reader));
        let hits = index_searcher.search(Arc::clone(&query), 1)?;
        Assert::assert_equals(1, hits.total_hits.value());
        let mut highlighter = Highlighter::with_formatter_and_encoder(
            Box::new(SimpleHTMLFormatter::new()),
            Box::new(SimpleHTMLEncoder::new()),
            Box::new(QueryScorer::new(query)),
        );
        let token_stream = tv_stream(&*index_reader, FIELD, 0)?
            .expect("NullPointerException: no term vector token stream");
        Assert::assert_equals_str(
            None,
            Some("<B>the fox</B> did not jump"),
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

fn dismax_fox() -> Arc<dyn Query> {
    Arc::new(
        DisjunctionMaxQuery::new(
            vec![
                Arc::new(SpanTermQuery::new(term(FIELD, "{fox}"))) as Arc<dyn Query>,
                Arc::new(SpanTermQuery::new(term(FIELD, "fox"))),
            ],
            1.0,
        )
        .expect("a valid tie breaker"),
    )
}

fn span_the_fox() -> Result<Arc<dyn Query>> {
    let clauses: Vec<Arc<dyn SpanQuery>> = vec![
        Arc::new(SpanTermQuery::new(term(FIELD, "the"))),
        Arc::new(SpanTermQuery::new(term(FIELD, "fox"))),
    ];
    Ok(Arc::new(SpanNearQuery::new(clauses, 0, true)?))
}

fn test_overlap_with_offset() -> Result<()> {
    // no positions!
    check_overlap(false, dismax_fox())
}

fn test_overlap_with_positions_and_offset() -> Result<()> {
    check_overlap(true, dismax_fox())
}

fn test_overlap_with_offset_exact_phrase() -> Result<()> {
    // no positions!
    check_overlap(false, span_the_fox()?)
}

fn test_overlap_with_positions_and_offset_exact_phrase() -> Result<()> {
    check_overlap(true, span_the_fox()?)
}

fn test_term_vector_without_offsets_doesnt_work() -> Result<()> {
    let directory = LuceneTestCase::new_directory();
    index_overlapping(&directory, false, true)?;
    let index_reader = directory_reader::open(directory.as_directory())?;
    let result = attempt(|| {
        Assert::assert_equals(1, index_reader.num_docs());
        let token_stream = tv_stream(&*index_reader, FIELD, 0)?;
        Assert::assert_true(token_stream.is_none());
        Ok(())
    });
    index_reader.close()?;
    directory.close()?;
    result
}

/// Just make a token with the text, and set the payload to the text as well.
/// Offsets increment "naturally".
fn get_token(cur_offset: &mut i32, text: &str) -> Token {
    use rucene::analysis::tokenattributes::PayloadAttribute;
    let mut t = Token::with_offsets(text, *cur_offset, *cur_offset + text.len() as i32);
    t.set_payload(Some(BytesRef::from_char_sequence(text)));
    *cur_offset += 1;
    t
}

// LUCENE-5294
fn test_payloads() -> Result<()> {
    use rucene::analysis::tokenattributes::{
        OffsetAttribute, PayloadAttribute, PositionIncrementAttribute,
    };
    let dir = LuceneTestCase::new_directory();
    let writer = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    let my_field_type = FieldType::new_from(TextField::type_not_stored());
    my_field_type.set_store_term_vectors(true)?;
    my_field_type.set_store_term_vector_offsets(true)?;
    my_field_type.set_store_term_vector_positions(true)?;
    my_field_type.set_store_term_vector_payloads(true)?;

    let mut cur_offset = 0;
    let tokens = vec![
        get_token(&mut cur_offset, "foxes"),
        get_token(&mut cur_offset, "can"),
        get_token(&mut cur_offset, "jump"),
        get_token(&mut cur_offset, "high"),
    ];

    let mut doc = Document::new();
    let canned: Arc<AtomicRefCell<dyn TokenStream>> =
        Arc::new(AtomicRefCell::new(CannedTokenStream::new(tokens.clone())));
    doc.add(Field::new_with_token_stream(
        "field",
        canned,
        Arc::new(my_field_type),
    )?);
    writer.add_document(&doc)?;

    let reader = writer.get_reader()?;
    writer.close()?;
    Assert::assert_equals(1, reader.num_docs());

    let mut ts = tv_stream(&*reader, "field", 0)?
        .expect("NullPointerException: no term vector token stream");

    let term_att = Att::get(ts.attribute_source(), char_term_id()).expect("a CharTermAttribute");
    let pos_inc_att =
        Att::get(ts.attribute_source(), pos_inc_id()).expect("a PositionIncrementAttribute");
    let offset_att = Att::get(ts.attribute_source(), offset_id()).expect("an OffsetAttribute");
    let payload_att = Att::add(ts.attribute_source_mut(), payload_id())?;

    ts.reset()?;
    for token in &tokens {
        Assert::assert_true(ts.increment_token()?);
        Assert::assert_equals(token.term(), term_att.term_string());
        Assert::assert_equals(
            token.get_position_increment(),
            pos_inc_att.pos_inc().get_position_increment(),
        );
        Assert::assert_true(token.get_payload() == payload_att.payload().get_payload());
        Assert::assert_equals(token.start_offset(), offset_att.offset().start_offset());
        Assert::assert_equals(token.end_offset(), offset_att.offset().end_offset());
    }

    Assert::assert_false(ts.increment_token()?);

    reader.close()?;
    dir.close()?;
    Ok(())
}

fn test_randomized_round_trip() -> Result<()> {
    let distinct = TestUtil::next_int(&*random(), 1, 10);

    let mut terms = Vec::with_capacity(distinct as usize);
    let mut term_bytes = Vec::with_capacity(distinct as usize);
    for _ in 0..distinct {
        let t = TestUtil::random_realistic_unicode_string(&*random());
        term_bytes.push(BytesRef::from_char_sequence(&t));
        terms.push(t);
    }

    let r_token_stream = Arc::new(AtomicRefCell::new(RandomTokenStream::new(
        TestUtil::next_int(&*random(), 1, 10),
        &terms,
        &term_bytes,
    )));
    let data = Arc::clone(r_token_stream.borrow().data());
    // check to see if the token streams might have non-deterministic testable result
    let store_term_vector_positions = random().next_boolean();
    let start_offsets = data.get_start_offsets();
    let positions_increments = data.get_positions_increments();
    for i in 1..positions_increments.len() {
        if store_term_vector_positions && positions_increments[i] != 0 {
            continue;
        }
        // TODO should RandomTokenStream ensure endOffsets for tokens at same position and same
        // startOffset are greater
        // than previous token's endOffset?  That would increase the testable possibilities.
        if start_offsets[i] == start_offsets[i - 1] {
            if verbose() {
                println!(
                    "Skipping test because can't easily validate random token-stream is correct."
                );
            }
            r_token_stream.borrow_mut().close()?;
            return Ok(());
        }
    }

    let r_terms: Vec<&str> = data.get_terms().iter().map(String::as_str).collect();
    // sanity check itself
    BaseTokenStreamTestCase::assert_token_stream_contents_offsets_pos_increments(
        &mut *r_token_stream.borrow_mut(),
        &r_terms,
        data.get_start_offsets(),
        data.get_end_offsets(),
        data.get_positions_increments(),
    )?;

    let dir = LuceneTestCase::new_directory();
    let writer = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    let my_field_type = FieldType::new_from(TextField::type_not_stored());
    my_field_type.set_store_term_vectors(true)?;
    my_field_type.set_store_term_vector_offsets(true)?;
    my_field_type.set_store_term_vector_positions(store_term_vector_positions)?;
    // payloads require positions; it will throw an error otherwise
    my_field_type
        .set_store_term_vector_payloads(store_term_vector_positions && random().next_boolean())?;
    let store_payloads = my_field_type.store_term_vector_payloads();
    let my_field_type = Arc::new(my_field_type);
    let mut doc = Document::new();
    let stream: Arc<AtomicRefCell<dyn TokenStream>> = Arc::clone(&r_token_stream) as _;
    doc.add(Field::new_with_token_stream(
        "field",
        stream,
        Arc::clone(&my_field_type),
    )?);
    writer.add_document(&doc)?;

    let reader = writer.get_reader()?;
    writer.close()?;
    Assert::assert_equals(1, reader.num_docs());

    let mut vector_token_stream = tv_stream(&*reader, "field", 0)?
        .expect("NullPointerException: no term vector token stream");

    // sometimes check payloads
    let mut payload_attribute = None;
    if store_payloads && LuceneTestCase::usually() {
        payload_attribute = Some(Att::add(
            vector_token_stream.attribute_source_mut(),
            payload_id(),
        )?);
    }
    BaseTokenStreamTestCase::assert_token_stream_contents_final_offset(
        &mut *vector_token_stream,
        &r_terms,
        Some(data.get_start_offsets()),
        Some(data.get_end_offsets()),
        None,
        if my_field_type.store_term_vector_positions() {
            Some(data.get_positions_increments())
        } else {
            None
        },
        None,
        None,
    )?;

    // test payloads
    if let Some(payload_attribute) = payload_attribute {
        vector_token_stream.reset()?;
        let mut i = 0;
        while vector_token_stream.increment_token()? {
            Assert::assert_true(
                data.get_payloads()[i].as_ref() == payload_attribute.payload().get_payload(),
            );
            i += 1;
        }
    }

    reader.close()?;
    dir.close()?;
    r_token_stream.borrow_mut().close()?;
    Ok(())
}

fn test_max_start_offset_consistency() -> Result<()> {
    let tv_field_type = FieldType::new_from(TextField::type_not_stored());
    tv_field_type.set_store_term_vectors(true)?;
    tv_field_type.set_store_term_vector_offsets(true)?;
    tv_field_type.set_store_term_vector_positions(true)?;
    let dir = LuceneTestCase::new_directory();

    let analyzer = Arc::new(MockAnalyzer::new(&*random()));
    // we don't necessarily consume the whole stream because of limiting by startOffset
    analyzer.set_enable_checks(false);
    let mut doc = Document::new();
    let text = " f gg h";
    doc.add(Field::new_with_token_stream(
        "fld_tv",
        analyzer.token_stream_from_str("fooFld", text)?,
        Arc::new(tv_field_type),
    )?);
    doc.add(TextField::new_with_token_stream(
        "fld_notv",
        analyzer.token_stream_from_str("barFld", text)?,
    )?);

    let reader = {
        let writer = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
        let result = attempt(|| {
            writer.add_document(&doc)?;
            writer.get_reader()
        });
        writer.close()?;
        result?
    };
    let result = attempt(|| {
        let tv_fields = reader.term_vectors()?.get(0)?;
        for max_start_offset in -1..=text.len() as i32 {
            let mut tv_stream = TokenSources::get_token_stream(
                "fld_tv",
                tv_fields.as_deref(),
                text,
                &*analyzer,
                max_start_offset,
            )?;
            let mut ana_stream = TokenSources::get_token_stream(
                "fld_notv",
                tv_fields.as_deref(),
                text,
                &*analyzer,
                max_start_offset,
            )?;

            // assert have same tokens, none of which has a start offset > maxStartOffset
            let tv_off_att = Att::add(tv_stream.attribute_source_mut(), offset_id())?;
            let ana_off_att = Att::add(ana_stream.attribute_source_mut(), offset_id())?;
            tv_stream.reset()?;
            ana_stream.reset()?;
            while tv_stream.increment_token()? {
                Assert::assert_true(ana_stream.increment_token()?);
                let tv_start = tv_off_att.offset().start_offset();
                Assert::assert_equals(tv_start, ana_off_att.offset().start_offset());
                if max_start_offset >= 0 {
                    Assert::assert_true(tv_start <= max_start_offset);
                }
            }
            Assert::assert_true(!ana_stream.increment_token()?);
            tv_stream.end()?;
            ana_stream.end()?;
            tv_stream.close()?;
            ana_stream.close()?;
        }
        Ok(())
    });
    reader.close()?;
    dir.close()?;
    result
}
