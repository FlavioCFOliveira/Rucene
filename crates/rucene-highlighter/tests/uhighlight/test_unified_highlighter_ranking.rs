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

//! Port of `TestUnifiedHighlighterRanking.java`.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::random::Random;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::analysis::Analyzer;
use rucene::document::{Document, Field, Store, StringField};
use rucene::error::Result;
use rucene::search::boolean_clause::Occur;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::query::Query;
use rucene_highlighter::uhighlight::passage::Passage;
use rucene_highlighter::uhighlight::passage_formatter::{FormattedPassages, PassageFormatter};
use rucene_highlighter::uhighlight::passage_scorer::PassageScorer;
use rucene_highlighter::uhighlight::unified_highlighter::{
    Builder, HighlightFlag, UnifiedHighlighter, UnifiedHighlighterOverrides,
};
use rucene_test_framework::analysis::mock_tokenizer::SIMPLE;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::util::test_util::TestUtil;
use rucene_test_framework::util::LuceneTestCase;

use crate::highlighter_support::core_support::random;
use crate::highlighter_support::ti;
use crate::uhighlight::test_unified_highlighter::{bq, builder};
use crate::uhighlight::unified_highlighter_test_base::{
    add, random_type_constructor, s, search_index_order, set, tq, uh_class, UHBase, UHTest,
};

type T = UHTest;

/// `TestUnifiedHighlighterRanking`.
pub fn class_def() -> Arc<Class> {
    uh_class::<T>(
        "org.apache.lucene.search.uhighlight.TestUnifiedHighlighterRanking",
        random_type_constructor(|ft| UHTest { b: UHBase::new(ft) }),
        vec![
            ti::<T>("testRanking", test_ranking),
            ti::<T>("testCustomB", test_custom_b),
            ti::<T>("testCustomK1", test_custom_k1),
        ],
    )
}

/// Indexes a bunch of gibberish, and then highlights top(n). asserts that
/// top(n) highlights is a subset of top(n+1) up to some max N.
// TODO: this only tests single-valued fields. we should also index multiple values per field!
fn test_ranking(t: &mut T) -> Result<()> {
    // number of documents: we will check each one
    let num_docs = LuceneTestCase::at_least(20);
    // number of top-N snippets, we will check 1 .. N
    let max_top_n = 3;
    // maximum number of elements to put in a sentence.
    let max_sentence_length = 10;
    // maximum number of sentences in a document
    let max_num_sentences = 20;

    let iw = t.writer()?;
    let mut document = Document::new();
    let id = Arc::new(StringField::new("id", String::new(), Store::NO)?);
    let body = t.field("body", "")?;
    document.add_shared(Arc::clone(&id) as Arc<dyn rucene::index::IndexableField>);
    add(&mut document, &body);

    for i in 0..num_docs {
        let mut body_text = String::new();
        let num_sentences = TestUtil::next_int(&*random(), 1, max_num_sentences);
        for _ in 0..num_sentences {
            body_text.push_str(&new_sentence(&*random(), max_sentence_length));
        }
        set(&body, &body_text)?;
        id.set_string_value(i.to_string())?;
        iw.add_document(&document)?;
    }

    let ir = iw.get_reader()?;
    let searcher = Arc::new(LuceneTestCase::new_searcher(Arc::clone(&ir)));
    for i in 0..num_docs {
        check_document(t, &searcher, i, max_top_n)?;
    }
    iw.close()?;
    ir.close()?;
    t.dir().close()
}

fn check_document(t: &UHBase, is: &Arc<IndexSearcher>, doc: i32, max_top_n: i32) -> Result<()> {
    for ch in b'a'..=b'z' {
        let term = (ch as char).to_string();
        // check a simple term query
        check_query(t, is, tq("body", &term), doc, max_top_n)?;
        // check a boolean query
        let next_term = ((ch + 1) as char).to_string();
        let bq = bq(vec![
            (tq("body", &term), Occur::SHOULD),
            (tq("body", &next_term), Occur::SHOULD),
        ])?;
        check_query(t, is, bq, doc, max_top_n)?;
    }
    Ok(())
}

fn check_query(
    t: &UHBase,
    is: &Arc<IndexSearcher>,
    query: Arc<dyn Query>,
    doc: i32,
    max_top_n: i32,
) -> Result<()> {
    for n in 1..max_top_n {
        let f1 = Arc::new(FakePassageFormatter::default());
        let p1 = creat_uh_object_for_current_test_suite(
            builder(is, t.analyzer())
                .with_formatter(Arc::clone(&f1) as Arc<dyn PassageFormatter>)
                .with_max_length(i32::MAX - 1)?,
        );
        let f2 = Arc::new(FakePassageFormatter::default());
        let p2 = creat_uh_object_for_current_test_suite(
            builder(is, t.analyzer())
                .with_formatter(Arc::clone(&f2) as Arc<dyn PassageFormatter>)
                .with_max_length(i32::MAX - 1)?,
        );
        let bq = bq(vec![
            (Arc::clone(&query), Occur::MUST),
            (tq("id", &doc.to_string()), Occur::MUST),
        ])?;
        let td = is.search(Arc::clone(&bq), 1)?;
        p1.highlight_max_passages("body", &bq, &td, n)?;
        p2.highlight_max_passages("body", &bq, &td, n + 1)?;
        let seen1 = f1.seen.lock().unwrap_or_else(|e| e.into_inner());
        let seen2 = f2.seen.lock().unwrap_or_else(|e| e.into_inner());
        Assert::assert_true(seen1.is_subset(&seen2));
    }
    Ok(())
}

/// Returns a new random sentence, up to `max_sentence_length` "words" in
/// length. each word is a single character (a-z). The first one is
/// capitalized.
fn new_sentence(r: &dyn Random, max_sentence_length: i32) -> String {
    let mut sb = String::new();
    let num_elements = TestUtil::next_int(r, 1, max_sentence_length);
    for _ in 0..num_elements {
        if !sb.is_empty() {
            sb.push(' ');
            sb.push(TestUtil::next_int(r, 'a' as i32, 'z' as i32) as u8 as char);
        } else {
            // capitalize the first word to help breakiterator
            sb.push(TestUtil::next_int(r, 'A' as i32, 'Z' as i32) as u8 as char);
        }
    }
    sb.push_str(". "); // finalize sentence
    sb
}

/// A fake formatter that doesn't actually format passages. instead it just
/// collects them for asserts!
#[derive(Debug, Default)]
struct FakePassageFormatter {
    /// `seen`: the `Pair(start, end)` records.
    seen: Mutex<HashSet<(i32, i32)>>,
}

impl PassageFormatter for FakePassageFormatter {
    fn format(&self, passages: &[Passage], content: &str) -> Result<Box<dyn FormattedPassages>> {
        let content: Vec<u16> = content.encode_utf16().collect();
        let content_length = content.len() as i32;
        for p in passages {
            // verify some basics about the passage
            Assert::assert_true(p.get_score() >= 0.0);
            Assert::assert_true(p.get_num_matches() > 0);
            Assert::assert_true(p.get_start_offset() >= 0);
            Assert::assert_true(p.get_start_offset() <= content_length);
            Assert::assert_true(p.get_end_offset() >= p.get_start_offset());
            Assert::assert_true(p.get_end_offset() <= content_length);

            // we use a very simple analyzer. so we can assert the matches are correct
            let mut last_match_start = -1;
            for i in 0..p.get_num_matches() as usize {
                let term = &p.get_match_terms()[i];
                let match_start = p.get_match_starts()[i];
                Assert::assert_true(match_start >= 0);
                // must at least start within the passage
                Assert::assert_true(match_start < p.get_end_offset());
                let match_end = p.get_match_ends()[i];
                Assert::assert_true(match_end >= 0);
                // always moving forward
                Assert::assert_true(match_start >= last_match_start);
                last_match_start = match_start;
                // single character terms
                Assert::assert_equals(match_start + 1, match_end);
                // and the offsets must be correct...
                Assert::assert_equals(1, term.length);
                let c = content[match_start as usize];
                let lower =
                    char::from_u32(c as u32).map_or(c, |ch| ch.to_ascii_lowercase() as u32 as u16);
                Assert::assert_equals(term.bytes[term.offset] as u16, lower);
            }
            // record just the start/end offset for simplicity
            self.seen
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert((p.get_start_offset(), p.get_end_offset()));
        }
        Ok(Box::new("bogus!!!!!!".to_string()))
    }
}

/// The overrides of the anonymous highlighters of `testCustomB` and
/// `testCustomK1`.
#[derive(Debug)]
struct CustomScorer {
    k1: f32,
    b: f32,
}

impl UnifiedHighlighterOverrides for CustomScorer {
    fn get_flags(&self, this: &UnifiedHighlighter, field: &str) -> HashSet<HighlightFlag> {
        if random().next_boolean() {
            [
                HighlightFlag::MULTI_TERM_QUERY,
                HighlightFlag::PHRASES,
                HighlightFlag::WEIGHT_MATCHES,
            ]
            .into_iter()
            .collect()
        } else {
            this.super_get_flags(field)
        }
    }

    fn get_scorer(&self, _this: &UnifiedHighlighter, _field: &str) -> PassageScorer {
        PassageScorer::with_params(self.k1, self.b, 87.0)
    }
}

/// Sets b=0 to disable passage length normalization.
fn test_custom_b(t: &mut T) -> Result<()> {
    let iwc = LuceneTestCase::new_index_writer_config_analyzer(Some(t.analyzer()));
    iwc.set_merge_policy(Some(Arc::new(LuceneTestCase::new_log_merge_policy())))?;
    let iw = RandomIndexWriter::new_with_config(&*random(), t.dir().as_directory(), iwc)?;

    let body = t.field("body", "")?;
    let mut doc = Document::new();
    add(&mut doc, &body);

    set(
        &body,
        concat!(
            "This is a test.  This test is a better test but the sentence is excruiatingly long, ",
            "you have no idea how painful it was for me to type this long sentence into my IDE."
        ),
    )?;
    iw.add_document(&doc)?;

    let ir = iw.get_reader()?;
    iw.close()?;

    let searcher = Arc::new(LuceneTestCase::new_searcher(Arc::clone(&ir)));
    let uh_builder = builder(&searcher, t.analyzer());
    let mut highlighter = UnifiedHighlighter::from_builder(uh_builder);
    highlighter.set_overrides(Arc::new(CustomScorer { k1: 1.2, b: 0.0 }));
    let query = tq("body", "test");
    let top_docs = search_index_order(&searcher, &query, 10)?;
    Assert::assert_equals(1, top_docs.total_hits.value());
    let snippets = highlighter.highlight_max_passages("body", &query, &top_docs, 1)?;
    Assert::assert_equals(1, snippets.len());
    Assert::assert_true(s(&snippets[0]).starts_with("This <b>test</b> is a better <b>test</b>"));

    ir.close()?;
    t.dir().close()
}

/// Sets k1=0 for simple coordinate-level match (# of query terms present).
fn test_custom_k1(t: &mut T) -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let index_analyzer: Arc<dyn Analyzer> = Arc::new(MockAnalyzer::with_automaton(
        &*random(),
        Arc::clone(&SIMPLE),
        true,
    ));
    let iwc = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::clone(&index_analyzer)));
    iwc.set_merge_policy(Some(Arc::new(LuceneTestCase::new_log_merge_policy())))?;
    let iw = RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), iwc)?;

    let body = Arc::new(Field::new(
        "body",
        String::new(),
        Arc::clone(&t.field_type),
    )?);
    let mut doc = Document::new();
    add(&mut doc, &body);

    set(
        &body,
        concat!(
            "This has only foo foo. ",
            "On the other hand this sentence contains both foo and bar. ",
            "This has only bar bar bar bar bar bar bar bar bar bar bar bar."
        ),
    )?;
    iw.add_document(&doc)?;

    let ir = iw.get_reader()?;
    iw.close()?;

    let searcher = Arc::new(LuceneTestCase::new_searcher(Arc::clone(&ir)));
    let uh_builder = builder(&searcher, Arc::clone(&index_analyzer));
    let mut highlighter = UnifiedHighlighter::from_builder(uh_builder);
    highlighter.set_overrides(Arc::new(CustomScorer { k1: 0.0, b: 0.75 }));
    let query = bq(vec![
        (tq("body", "foo"), Occur::SHOULD),
        (tq("body", "bar"), Occur::SHOULD),
    ])?;
    let top_docs = search_index_order(&searcher, &query, 10)?;
    Assert::assert_equals(1, top_docs.total_hits.value());
    let snippets = highlighter.highlight_max_passages("body", &query, &top_docs, 1)?;
    Assert::assert_equals(1, snippets.len());
    Assert::assert_true(s(&snippets[0]).starts_with("On the other hand"));

    ir.close()?;
    dir.close()
}

/// The overrides of the anonymous highlighter of
/// `creatUHObjectForCurrentTestSuite`.
#[derive(Debug)]
struct AssertBodyFormatter;

impl UnifiedHighlighterOverrides for AssertBodyFormatter {
    fn get_formatter(&self, this: &UnifiedHighlighter, field: &str) -> Arc<dyn PassageFormatter> {
        Assert::assert_equals("body", field);
        this.super_get_formatter(field)
    }
}

/// `creatUHObjectForCurrentTestSuite(searcher, indexAnalyzer, uhBuilder)`:
/// the anonymous builder's `build()` returns `new
/// UnifiedHighlighter(uhBuilder) { getFormatter }`.
fn creat_uh_object_for_current_test_suite(uh_builder: Builder) -> UnifiedHighlighter {
    let mut highlighter = UnifiedHighlighter::from_builder(uh_builder);
    highlighter.set_overrides(Arc::new(AssertBodyFormatter));
    highlighter
}
