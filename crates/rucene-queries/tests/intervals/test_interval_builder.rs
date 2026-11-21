/*
 * Code adopted from ASL-licensed Elasticsearch.
 * https://github.com/elastic/elasticsearch/blob/7.10/server/src/test/java/org/elasticsearch/index/query/IntervalBuilderTests.java
 *
 * Licensed to Elasticsearch under one or more contributor
 * license agreements. See the NOTICE file distributed with
 * this work for additional information regarding copyright
 * ownership. Elasticsearch licenses this file to you under
 * the Apache License, Version 2.0 (the "License"); you may
 * not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *    http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing,
 * software distributed under the License is distributed on an
 * "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
 * KIND, either express or implied.  See the License for the
 * specific language governing permissions and limitations
 * under the License.
 */

//! Port of `TestIntervalBuilder.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::analysis::CachingTokenFilter;
use rucene::document::{Document, Store};
use rucene::error::Result;
use rucene::index::{directory_reader, IndexReader};
use rucene::store::Directory;
use rucene_queries::intervals::interval_builder::IntervalBuilder;
use rucene_queries::intervals::intervals::Intervals;
use rucene_queries::intervals::intervals_source::IntervalsSource;
use rucene_test_framework::analysis::canned_token_stream::CannedTokenStream;
use rucene_test_framework::analysis::token::Token;
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::util::LuceneTestCase;

use crate::intervals::support::{
    assert_source_equals, extend, or, ordered, phrase, phrase_s, term, unordered,
};
use crate::queries_support::core_support::{class, random, test};

/// `TestIntervalBuilder`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.queries.intervals.TestIntervalBuilder",
        vec![
            test("testSimpleTerm", test_simple_term),
            test("testOrdered", test_ordered),
            test("testUnordered", test_unordered),
            test("testPhrase", test_phrase),
            test("testPhraseWithStopword", test_phrase_with_stopword),
            test("testEmptyTokenStream", test_empty_token_stream),
            test("testSimpleSynonyms", test_simple_synonyms),
            test("testSimpleSynonymsWithGap", test_simple_synonyms_with_gap),
            test("testGraphSynonyms", test_graph_synonyms),
            test("testGraphSynonymsWithGaps", test_graph_synonyms_with_gaps),
            test("testGraphTerminatesOnGap", test_graph_terminates_on_gap),
            test("testEmptyIntervals", test_empty_intervals),
        ],
    )
}

/// `IntervalBuilder.analyzeText(new CachingTokenFilter(new CannedTokenStream(tokens)), maxGaps,
/// ordered)`.
fn analyze(tokens: Vec<Token>, max_gaps: i32, ordered: bool) -> Result<Arc<dyn IntervalsSource>> {
    let ts = CannedTokenStream::new(tokens);
    IntervalBuilder::analyze_text(
        &mut CachingTokenFilter::new(Box::new(ts)),
        max_gaps,
        ordered,
    )
}

/// `new Token(text, start, end)`.
fn tok(text: &str, start: i32, end: i32) -> Token {
    Token::with_offsets(text, start, end)
}

/// `new Token(text, posInc, start, end)`.
fn tok_inc(text: &str, pos_inc: i32, start: i32, end: i32) -> Token {
    Token::with_pos_inc(text, pos_inc, start, end)
}

/// `new Token(text, posInc, start, end, posLength)`.
fn tok_len(text: &str, pos_inc: i32, start: i32, end: i32, pos_length: i32) -> Token {
    Token::with_pos_len(text, pos_inc, start, end, pos_length)
}

fn test_simple_term() -> Result<()> {
    let source = analyze(vec![tok("term1", 1, 2)], -1, true)?;
    let expected = term("term1");
    assert_source_equals(&expected, &source);
    Ok(())
}

fn test_ordered() -> Result<()> {
    let source = analyze(
        vec![tok("term1", 1, 2), tok("term2", 3, 4), tok("term3", 5, 6)],
        -1,
        true,
    )?;
    let expected = ordered(vec![term("term1"), term("term2"), term("term3")]);
    assert_source_equals(&expected, &source);
    Ok(())
}

fn test_unordered() -> Result<()> {
    let source = analyze(
        vec![tok("term1", 1, 2), tok("term2", 3, 4), tok("term3", 5, 6)],
        -1,
        false,
    )?;
    let expected = unordered(vec![term("term1"), term("term2"), term("term3")]);
    assert_source_equals(&expected, &source);
    Ok(())
}

fn test_phrase() -> Result<()> {
    let source = analyze(
        vec![tok("term1", 1, 2), tok("term2", 3, 4), tok("term3", 5, 6)],
        0,
        true,
    )?;
    let expected = phrase(vec![term("term1"), term("term2"), term("term3")]);
    assert_source_equals(&expected, &source);
    Ok(())
}

fn test_phrase_with_stopword() -> Result<()> {
    let source = analyze(
        vec![tok_inc("term1", 1, 1, 2), tok_inc("term3", 2, 5, 6)],
        0,
        true,
    )?;
    let expected = phrase(vec![term("term1"), extend(term("term3"), 1, 0)]);
    assert_source_equals(&expected, &source);
    Ok(())
}

fn test_empty_token_stream() -> Result<()> {
    let source = analyze(vec![], 0, true)?;
    assert_source_equals(
        &Intervals::no_intervals("No terms in analyzed text"),
        &source,
    );
    Ok(())
}

fn test_simple_synonyms() -> Result<()> {
    let source = analyze(
        vec![
            tok("term1", 1, 2),
            tok("term2", 3, 4),
            tok_inc("term4", 0, 3, 4),
            tok("term3", 5, 6),
        ],
        -1,
        true,
    )?;
    let expected = ordered(vec![
        term("term1"),
        or(vec![term("term2"), term("term4")]),
        term("term3"),
    ]);
    assert_source_equals(&expected, &source);
    Ok(())
}

fn test_simple_synonyms_with_gap() -> Result<()> {
    // term1 [] term2/term3/term4 term5
    let source = analyze(
        vec![
            tok("term1", 1, 2),
            tok_inc("term2", 2, 3, 4),
            tok_inc("term3", 0, 3, 4),
            tok_inc("term4", 0, 3, 4),
            tok("term5", 5, 6),
        ],
        -1,
        true,
    )?;
    let expected = ordered(vec![
        term("term1"),
        extend(or(vec![term("term2"), term("term3"), term("term4")]), 1, 0),
        term("term5"),
    ]);
    assert_source_equals(&expected, &source);
    Ok(())
}

fn test_graph_synonyms() -> Result<()> {
    // term1 term2:2/term3 term4 term5
    let source = analyze(
        vec![
            tok("term1", 1, 2),
            tok_len("term2", 1, 3, 4, 2),
            tok_inc("term3", 0, 3, 4),
            tok("term4", 5, 6),
            tok("term5", 6, 7),
        ],
        -1,
        true,
    )?;
    let expected = ordered(vec![
        term("term1"),
        or(vec![term("term2"), phrase_s(&["term3", "term4"])]),
        term("term5"),
    ]);
    assert_source_equals(&expected, &source);
    Ok(())
}

fn test_graph_synonyms_with_gaps() -> Result<()> {
    // term1 [] term2:4/term3 [] [] term4 term5
    let source = analyze(
        vec![
            tok("term1", 1, 2),
            tok_len("term2", 2, 3, 4, 4),
            tok_inc("term3", 0, 3, 4),
            tok_inc("term4", 3, 5, 6),
            tok("term5", 6, 7),
        ],
        -1,
        true,
    )?;
    let expected = ordered(vec![
        term("term1"),
        or(vec![
            extend(term("term2"), 1, 0),
            phrase(vec![
                extend(term("term3"), 1, 0),
                extend(term("term4"), 2, 0),
            ]),
        ]),
        term("term5"),
    ]);
    assert_source_equals(&expected, &source);
    Ok(())
}

fn test_graph_terminates_on_gap() -> Result<()> {
    // term1 term2:2/term3 term4 [] term5
    let source = analyze(
        vec![
            tok("term1", 1, 2),
            tok_len("term2", 1, 2, 3, 2),
            tok_inc("term3", 0, 2, 3),
            tok("term4", 2, 3),
            tok_inc("term5", 2, 6, 7),
        ],
        -1,
        true,
    )?;
    let expected = ordered(vec![
        term("term1"),
        or(vec![term("term2"), phrase_s(&["term3", "term4"])]),
        extend(term("term5"), 1, 0),
    ]);
    assert_source_equals(&expected, &source);
    Ok(())
}

fn test_empty_intervals() -> Result<()> {
    let source = analyze(vec![], -1, true)?;

    let directory = LuceneTestCase::new_directory();
    let w = RandomIndexWriter::new_with_mock_analyzer(&*random(), directory.as_directory())?;
    let mut doc = Document::new();
    doc.add(LuceneTestCase::new_string_field(
        "field",
        "value",
        Store::NO,
    ));
    w.add_document(&doc)?;
    w.close()?;

    let dir: Arc<dyn Directory> = directory.as_directory();
    let reader: Arc<dyn IndexReader> = directory_reader::open(dir)? as Arc<dyn IndexReader>;
    let ctx = Arc::clone(&Arc::clone(&reader).leaves()?[0]);
    let it = source.intervals("field", &ctx)?;
    Assert::assert_true_msg("expected null, but was:<IntervalIterator>", it.is_none());
    reader.close()?;
    directory.close()
}
