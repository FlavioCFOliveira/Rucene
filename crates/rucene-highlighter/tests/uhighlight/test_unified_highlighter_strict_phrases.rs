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

//! Port of `TestUnifiedHighlighterStrictPhrases.java`.

use std::any::Any;
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;

use rucene::analysis::Analyzer;
use rucene::document::Document;
use rucene::error::Result;
use rucene::index::DirectoryReader;
use rucene::search::boolean_clause::Occur;
use rucene::search::boolean_query::Builder as BooleanQueryBuilder;
use rucene::search::boost_query::BoostQuery;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::match_no_docs_query::MatchNoDocsQuery;
use rucene::search::multi_phrase_query::Builder as MultiPhraseQueryBuilder;
use rucene::search::phrase_query::{Builder as PhraseQueryBuilder, PhraseQuery};
use rucene::search::prefix_query::PrefixQuery;
use rucene::search::query::Query;
use rucene::search::query_visitor::QueryVisitor;
use rucene::search::score_doc::ScoreDoc;
use rucene::search::score_mode::ScoreMode;
use rucene::search::top_docs::TopDocs;
use rucene::search::total_hits::{TotalHits, TotalHitsRelation};
use rucene::search::weight::Weight;
use rucene::search::wildcard_query::WildcardQuery;
use rucene::util::query_builder::{QueryBuilder, QueryBuilderOps};
use rucene_highlighter::uhighlight::passage::Passage;
use rucene_highlighter::uhighlight::passage_formatter::{FormattedPassages, PassageFormatter};
use rucene_highlighter::uhighlight::unified_highlighter::{
    Builder, HighlightFlag, UnifiedHighlighter, UnifiedHighlighterOverrides,
};
use rucene_queries::spans::span_multi_term_query_wrapper::SpanMultiTermQueryWrapper;
use rucene_queries::spans::span_near_query::SpanNearQuery;
use rucene_queries::spans::span_or_query::SpanOrQuery;
use rucene_queries::spans::span_query::SpanQuery;
use rucene_queries::spans::span_term_query::SpanTermQuery;
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::util::java_exceptions::io_utils_close;
use rucene_test_framework::util::lucene_test_case::VERBOSE;
use rucene_test_framework::util::LuceneTestCase;

use crate::highlighter_support::search_support::term;
use crate::highlighter_support::ti;
use crate::uhighlight::test_unified_highlighter::{bq, builder, pq};
use crate::uhighlight::unified_highlighter_test_base::{
    add, assert_snippet, assert_snippet_msg, assert_snippets, base_set_up, base_tear_down,
    parameterized_constructor, parameters_method, random_uh_mut, search_index_order, tq, uh_class,
    UHBase, UHCase,
};

type T = TestStrictPhrases;

/// `TestUnifiedHighlighterStrictPhrases`.
pub fn class_def() -> Arc<Class> {
    uh_class::<T>(
        "org.apache.lucene.search.uhighlight.TestUnifiedHighlighterStrictPhrases",
        parameterized_constructor(T::new),
        vec![
            parameters_method(),
            ti::<T>("testBasics", T::test_basics),
            ti::<T>("testWithSameTermQuery", T::test_with_same_term_query),
            ti::<T>("testPhraseNotInDoc", T::test_phrase_not_in_doc),
            ti::<T>("testSubPhrases", T::test_sub_phrases),
            ti::<T>("testSynonyms", T::test_synonyms),
            ti::<T>("testRewriteAndMtq", T::test_rewrite_and_mtq),
            ti::<T>("testRewrite", T::test_rewrite),
            ti::<T>("testMtq", T::test_mtq),
            ti::<T>("testMultiValued", T::test_multi_valued),
            ti::<T>("testMaxLen", T::test_max_len),
            ti::<T>("testFilteredOutSpan", T::test_filtered_out_span),
            ti::<T>("testMatchNoDocsQuery", T::test_match_no_docs_query),
            ti::<T>("testPreSpanQueryRewrite", T::test_pre_span_query_rewrite),
            ti::<T>(
                "testReverseOrderSpanCollection",
                T::test_reverse_order_span_collection,
            ),
            ti::<T>(
                "testNestedSpanQueryHighlight",
                T::test_nested_span_query_highlight,
            ),
        ],
    )
}

/// The `TestUnifiedHighlighterStrictPhrases` instance.
pub struct TestStrictPhrases {
    b: UHBase,
    index_writer: Option<RandomIndexWriter>,
    searcher: Option<Arc<IndexSearcher>>,
    highlighter: Option<UnifiedHighlighter>,
    uh_builder: Option<Builder>,
    index_reader: Option<Arc<dyn DirectoryReader>>,
    /// Is it okay if a match (identified by offset pair) appears multiple
    /// times in the passage?
    dup_match_allowed: Arc<AtomicBool>,
}

impl UHCase for TestStrictPhrases {
    fn base_mut(&mut self) -> &mut UHBase {
        &mut self.b
    }

    fn set_up(&mut self) -> Result<()> {
        base_set_up(&mut self.b)?;
        self.b.mock().set_position_increment_gap(3); // more than default
        self.index_writer = Some(self.b.writer()?);
        Ok(())
    }

    fn tear_down(&mut self) -> Result<()> {
        let reader = self.index_reader.take().map_or(Ok(()), |r| r.close());
        let writer = self.index_writer.take().map_or(Ok(()), |w| w.close());
        io_utils_close([reader, writer])?;
        base_tear_down(&mut self.b)
    }
}

/// The formatter of `initReaderSearcherHighlighter`, which checks
/// constraints on the passage.
#[derive(Debug)]
struct CheckingFormatter {
    dup_match_allowed: Arc<AtomicBool>,
    default_formatter: Arc<dyn PassageFormatter>,
}

impl PassageFormatter for CheckingFormatter {
    fn format(&self, passages: &[Passage], content: &str) -> Result<Box<dyn FormattedPassages>> {
        let this_dup_match_allowed = self.dup_match_allowed.swap(true, Ordering::SeqCst);
        for passage in passages {
            let mut prev_pair = String::new();
            for i in 0..passage.get_num_matches() as usize {
                // pad each to make comparable
                let pair = format!(
                    "{:03}-{:03}",
                    passage.get_match_starts()[i],
                    passage.get_match_ends()[i]
                );
                let cmp = prev_pair.cmp(&pair);
                if cmp == std::cmp::Ordering::Equal {
                    Assert::assert_true_msg(
                        &format!("dup match in passage at offset {pair}"),
                        this_dup_match_allowed,
                    );
                } else if cmp == std::cmp::Ordering::Greater {
                    Assert::fail(&format!("bad match order in passage at offset {pair}"));
                }
                prev_pair = pair;
            }
        }
        self.default_formatter.format(passages, content)
    }
}

/// The overrides of the anonymous highlighter of `testPreSpanQueryRewrite`.
#[derive(Debug)]
struct PreSpanQueryRewrite;

impl UnifiedHighlighterOverrides for PreSpanQueryRewrite {
    fn get_flags(&self, this: &UnifiedHighlighter, field: &str) -> HashSet<HighlightFlag> {
        let mut flags = this.super_get_flags(field);
        flags.remove(&HighlightFlag::WEIGHT_MATCHES); // unsupported
        flags
    }

    fn pre_span_query_rewrite(&self, query: &Arc<dyn Query>) -> Option<Vec<Arc<dyn Query>>> {
        query
            .as_any()
            .downcast_ref::<MyQuery>()
            .map(|q| vec![Arc::clone(&q.wrapped)])
    }
}

fn span_term(field: &str, text: &str) -> Arc<dyn SpanQuery> {
    Arc::new(SpanTermQuery::new(term(field, text)))
}

fn span_near(
    clauses: Vec<Arc<dyn SpanQuery>>,
    slop: i32,
    in_order: bool,
) -> Result<Arc<SpanNearQuery>> {
    Ok(Arc::new(SpanNearQuery::new(clauses, slop, in_order)?))
}

fn prefix(field: &str, text: &str) -> Result<Arc<dyn Query>> {
    Ok(Arc::new(PrefixQuery::new(term(field, text))?))
}

fn span_prefix(field: &str, text: &str) -> Result<Arc<dyn SpanQuery>> {
    Ok(Arc::new(SpanMultiTermQueryWrapper::new(Arc::new(
        PrefixQuery::new(term(field, text))?,
    ))))
}

impl TestStrictPhrases {
    fn new(field_type: Arc<rucene::document::FieldType>) -> Self {
        Self {
            b: UHBase::new(field_type),
            index_writer: None,
            searcher: None,
            highlighter: None,
            uh_builder: None,
            index_reader: None,
            dup_match_allowed: Arc::new(AtomicBool::new(true)),
        }
    }

    fn iw(&self) -> &RandomIndexWriter {
        self.index_writer.as_ref().expect("setUp ran")
    }

    fn searcher(&self) -> &Arc<IndexSearcher> {
        self.searcher
            .as_ref()
            .expect("initReaderSearcherHighlighter ran")
    }

    fn highlighter(&self) -> &UnifiedHighlighter {
        self.highlighter
            .as_ref()
            .expect("initReaderSearcherHighlighter ran")
    }

    fn new_doc(&self, body_vals: &[&str]) -> Result<Document> {
        let mut doc = Document::new();
        for body_val in body_vals {
            add(&mut doc, &self.b.field("body", body_val)?);
        }
        Ok(doc)
    }

    fn add_doc(&self, body_vals: &[&str]) -> Result<()> {
        let doc = self.new_doc(body_vals)?;
        self.iw().add_document(&doc)?;
        Ok(())
    }

    fn init_reader_searcher_highlighter(&mut self) -> Result<()> {
        let index_reader = self.iw().get_reader()?;
        self.index_reader = Some(Arc::clone(&index_reader));
        let searcher = Arc::new(LuceneTestCase::new_searcher(index_reader));
        self.searcher = Some(Arc::clone(&searcher));
        let mut uh_builder = builder(&searcher, self.b.analyzer());
        let highlighter = random_uh_mut(
            &mut uh_builder,
            &[HighlightFlag::PHRASES, HighlightFlag::MULTI_TERM_QUERY],
            Some(true),
        );
        // intercept the formatter in order to check constraints on the passage.
        let default_formatter = highlighter.get_formatter("");
        uh_builder = uh_builder.with_formatter(Arc::new(CheckingFormatter {
            dup_match_allowed: Arc::clone(&self.dup_match_allowed),
            default_formatter,
        }));
        self.highlighter = Some(uh_builder.clone().build());
        self.uh_builder = Some(uh_builder);
        Ok(())
    }

    fn new_phrase_query(&self, field: &str, phrase: &str) -> Result<Arc<PhraseQuery>> {
        let query = QueryBuilder::new(Some(self.b.analyzer()))
            .create_phrase_query(Some(field), phrase)?
            .expect("NullPointerException: no query");
        Ok(Arc::new(
            query
                .as_any()
                .downcast_ref::<PhraseQuery>()
                .unwrap_or_else(|| panic!("java.lang.ClassCastException: not a PhraseQuery"))
                .clone(),
        ))
    }

    fn set_slop(query: &PhraseQuery, slop: i32) -> Result<Arc<PhraseQuery>> {
        let mut builder = PhraseQueryBuilder::new();
        let terms = query.get_terms();
        let positions = query.get_positions();
        for i in 0..terms.len() {
            builder.add_at(terms[i].clone(), positions[i])?;
        }
        builder.set_slop(slop);
        Ok(Arc::new(builder.build()?))
    }

    fn weight_matches(&self) -> bool {
        self.highlighter()
            .get_flags("body")
            .contains(&HighlightFlag::WEIGHT_MATCHES)
    }

    fn highlight(&self, query: &Arc<dyn Query>) -> Result<Vec<Option<String>>> {
        let top_docs = search_index_order(self.searcher(), query, 10)?;
        self.highlighter().highlight("body", query, &top_docs)
    }

    fn test_basics(&mut self) -> Result<()> {
        self.add_doc(&["Yin yang, filter"])?; // filter out. test getTermToSpanLists reader 1-doc filter
        self.add_doc(&["yin alone, Yin yang, yin gap yang"])?;
        self.init_reader_searcher_highlighter()?;

        // query:  -filter +"yin yang"
        let query = bq(vec![
            (tq("body", "filter"), Occur::MUST_NOT),
            (self.new_phrase_query("body", "yin yang")?, Occur::MUST),
        ])?;

        let snippets = self.highlight(&query)?;
        if self.weight_matches() {
            assert_snippets(&["yin alone, <b>Yin yang</b>, yin gap yang"], &snippets);
        } else {
            assert_snippets(
                &["yin alone, <b>Yin</b> <b>yang</b>, yin gap yang"],
                &snippets,
            );
        }
        Ok(())
    }

    fn test_with_same_term_query(&mut self) -> Result<()> {
        self.add_doc(&["Yin yang loooooooooong, yin gap yang yong"])?;
        self.init_reader_searcher_highlighter()?;

        let query = bq(vec![
            (tq("body", "yin"), Occur::MUST),
            (tq("body", "yang"), Occur::MUST),
            (tq("body", "loooooooooong"), Occur::MUST),
            (
                self.new_phrase_query("body", "yin\\ yang\\ loooooooooong")?,
                Occur::MUST,
            ),
            // add queries for other fields; we shouldn't highlight these because of that.
            (tq("title", "yang"), Occur::SHOULD),
        ])?;

        let top_docs = search_index_order(self.searcher(), &query, 10)?;
        self.dup_match_allowed.store(false, Ordering::SeqCst); // We don't want duplicates from "Yin" being in TermQuery & PhraseQuery.
        let snippets = self.highlighter().highlight("body", &query, &top_docs)?;
        if self.weight_matches() {
            assert_snippets(
                &["<b>Yin yang loooooooooong</b>, <b>yin</b> gap <b>yang</b> yong"],
                &snippets,
            );
        } else {
            assert_snippets(
                &["<b>Yin</b> <b>yang</b> <b>loooooooooong</b>, <b>yin</b> gap <b>yang</b> yong"],
                &snippets,
            );
        }
        Ok(())
    }

    fn test_phrase_not_in_doc(&mut self) -> Result<()> {
        self.add_doc(&["Whatever yin"])?; // query matches this; highlight it
        self.add_doc(&["nextdoc yin"])?; // query does NOT match this, only the SHOULD clause does
        self.init_reader_searcher_highlighter()?;

        let query = bq(vec![
            // MUST:
            (tq("body", "whatever"), Occur::MUST),
            // SHOULD: (yet won't)
            (self.new_phrase_query("body", "nextdoc yin")?, Occur::SHOULD),
            (
                self.new_phrase_query("body", "nonexistent yin")?,
                Occur::SHOULD,
            ),
        ])?;
        let snippets = self.highlight(&query)?;

        assert_snippets(&["<b>Whatever</b> yin"], &snippets);
        Ok(())
    }

    fn test_sub_phrases(&mut self) -> Result<()> {
        self.add_doc(&["alpha bravo charlie - charlie bravo alpha"])?;
        self.init_reader_searcher_highlighter()?;

        let query = bq(vec![
            (
                self.new_phrase_query("body", "alpha bravo charlie")?,
                Occur::MUST,
            ),
            (self.new_phrase_query("body", "alpha bravo")?, Occur::MUST),
        ])?;
        let top_docs = search_index_order(self.searcher(), &query, 10)?;
        self.dup_match_allowed.store(false, Ordering::SeqCst); // We don't want duplicates from both PhraseQuery
        let snippets = self.highlighter().highlight("body", &query, &top_docs)?;

        if self.weight_matches() {
            assert_snippets(
                &["<b>alpha bravo charlie</b> - charlie bravo alpha"],
                &snippets,
            );
        } else {
            assert_snippets(
                &["<b>alpha</b> <b>bravo</b> <b>charlie</b> - charlie bravo alpha"],
                &snippets,
            );
        }
        Ok(())
    }

    fn test_synonyms(&mut self) -> Result<()> {
        self.add_doc(&["mother father w mom father w dad"])?;
        self.init_reader_searcher_highlighter()?;

        let mut builder = MultiPhraseQueryBuilder::new();
        builder.add_terms(vec![term("body", "mom"), term("body", "mother")])?;
        builder.add_terms(vec![term("body", "dad"), term("body", "father")])?;
        let query: Arc<dyn Query> = Arc::new(builder.build());

        let snippets = self.highlight(&query)?;

        if self.weight_matches() {
            assert_snippets(
                &["<b>mother father</b> w <b>mom father</b> w dad"],
                &snippets,
            );
        } else {
            assert_snippets(
                &["<b>mother</b> <b>father</b> w <b>mom</b> <b>father</b> w dad"],
                &snippets,
            );
        }
        Ok(())
    }

    /// The highlighter without MTQ that the tests of rewrites use last.
    fn no_mtq_snippets(&mut self, query: &Arc<dyn Query>) -> Result<Vec<Option<String>>> {
        // do again, this time with MTQ disabled.
        let highlighter =
            UnifiedHighlighter::builder(Arc::clone(self.searcher()), self.b.analyzer())
                .with_handle_multi_term_query(false) // disable but leave phrase processing enabled
                .build();
        self.highlighter = Some(highlighter);
        self.highlight(query)
    }

    /// Test it does *not* highlight the same term's not next to the
    /// span-near. "charlie" in this case. This particular example exercises
    /// "Rewrite" plus "MTQ" in the same query.
    fn test_rewrite_and_mtq(&mut self) -> Result<()> {
        self.add_doc(&["alpha bravo charlie - charlie bravo alpha"])?;
        self.init_reader_searcher_highlighter()?;
        let snq = span_near(
            vec![span_term("body", "bravo"), span_prefix("body", "ch")?], // REWRITES
            0,
            true,
        )?;
        let query = bq(vec![
            (snq, Occur::MUST),
            (prefix("body", "al")?, Occur::MUST), // MTQ
            (self.new_phrase_query("body", "alpha bravo")?, Occur::MUST),
            // add queries for other fields; we shouldn't highlight these because of that.
            (
                self.new_phrase_query("title", "bravo alpha")?,
                Occur::SHOULD,
            ),
        ])?;

        let mut snippets = self.highlight(&query)?;

        if self.weight_matches() {
            assert_snippets(
                &["<b>alpha bravo</b> <b>charlie</b> - charlie bravo <b>alpha</b>"],
                &snippets,
            );
        } else {
            assert_snippets(
                &["<b>alpha</b> <b>bravo</b> <b>charlie</b> - charlie bravo <b>alpha</b>"],
                &snippets,
            );
        }

        // do again, this time with MTQ disabled.  We should only find "alpha bravo".
        snippets = self.no_mtq_snippets(&query)?;

        assert_snippets(
            &["<b>alpha</b> <b>bravo</b> charlie - charlie bravo alpha"],
            &snippets,
        );
        Ok(())
    }

    /// Like `testRewriteAndMtq` but no freestanding MTQ.
    fn test_rewrite(&mut self) -> Result<()> {
        self.add_doc(&["alpha bravo charlie - charlie bravo alpha"])?;
        self.init_reader_searcher_highlighter()?;

        let snq = span_near(
            vec![span_term("body", "bravo"), span_prefix("body", "ch")?], // REWRITES
            0,
            true,
        )?;
        let query = bq(vec![
            (snq, Occur::MUST),
            //          .add(new PrefixQuery(new Term("body", "al")), BooleanClause.Occur.MUST) //
            // MTQ
            (self.new_phrase_query("body", "alpha bravo")?, Occur::MUST),
            // add queries for other fields; we shouldn't highlight these because of that.
            (
                self.new_phrase_query("title", "bravo alpha")?,
                Occur::SHOULD,
            ),
        ])?;
        let mut snippets = self.highlight(&query)?;

        if self.weight_matches() {
            assert_snippets(
                &["<b>alpha bravo</b> <b>charlie</b> - charlie bravo alpha"],
                &snippets,
            );
        } else {
            assert_snippets(
                &["<b>alpha</b> <b>bravo</b> <b>charlie</b> - charlie bravo alpha"],
                &snippets,
            );
        }

        // do again, this time with MTQ disabled.  We should only find "alpha bravo".
        snippets = self.no_mtq_snippets(&query)?;

        assert_snippets(
            &["<b>alpha</b> <b>bravo</b> charlie - charlie bravo alpha"],
            &snippets,
        );
        Ok(())
    }

    /// Like `testRewriteAndMtq` but no rewrite.
    fn test_mtq(&mut self) -> Result<()> {
        self.add_doc(&["alpha bravo charlie - charlie bravo alpha"])?;
        self.init_reader_searcher_highlighter()?;

        let snq = span_near(
            vec![span_term("body", "bravo"), span_term("body", "charlie")], // does NOT rewrite
            0,
            true,
        )?;

        let query = bq(vec![
            (snq, Occur::MUST),
            (prefix("body", "al")?, Occur::MUST), // MTQ
            (self.new_phrase_query("body", "alpha bravo")?, Occur::MUST),
            // add queries for other fields; we shouldn't highlight these because of that.
            (
                self.new_phrase_query("title", "bravo alpha")?,
                Occur::SHOULD,
            ),
        ])?;
        let mut snippets = self.highlight(&query)?;

        if self.weight_matches() {
            assert_snippets(
                &["<b>alpha bravo</b> <b>charlie</b> - charlie bravo <b>alpha</b>"],
                &snippets,
            );
        } else {
            assert_snippets(
                &["<b>alpha</b> <b>bravo</b> <b>charlie</b> - charlie bravo <b>alpha</b>"],
                &snippets,
            );
        }

        // do again, this time with MTQ disabled.
        snippets = self.no_mtq_snippets(&query)?;

        // note: without MTQ, the WEIGHT_MATCHES is disabled which affects the snippet boundaries
        assert_snippets(
            &["<b>alpha</b> <b>bravo</b> <b>charlie</b> - charlie bravo alpha"],
            &snippets,
        );
        Ok(())
    }

    fn test_multi_valued(&mut self) -> Result<()> {
        self.add_doc(&["one bravo three", "four bravo six"])?;
        self.init_reader_searcher_highlighter()?;

        let mut query = bq(vec![
            (self.new_phrase_query("body", "one bravo")?, Occur::MUST),
            (self.new_phrase_query("body", "four bravo")?, Occur::MUST),
            (prefix("body", "br")?, Occur::MUST),
        ])?;
        let mut top_docs = search_index_order(self.searcher(), &query, 10)?;

        let mut snippets = self
            .highlighter()
            .highlight_max_passages("body", &query, &top_docs, 2)?;

        if self.weight_matches() {
            assert_snippets(
                &["<b>one bravo</b> three... <b>four bravo</b> six"],
                &snippets,
            );
        } else {
            assert_snippets(
                &["<b>one</b> <b>bravo</b> three... <b>four</b> <b>bravo</b> six"],
                &snippets,
            );
        }

        // now test phraseQuery won't span across values
        let gap = self.b.mock().get_position_increment_gap(Some("body"));
        assert!(gap > 0);

        let mut phrase_query = self.new_phrase_query("body", "three four")?;
        // 1 too little; won't span
        phrase_query = Self::set_slop(&phrase_query, gap - 1)?;

        query = bq(vec![
            (tq("body", "bravo"), Occur::MUST),
            (phrase_query, Occur::SHOULD),
        ])?;

        top_docs = self.searcher().search(Arc::clone(&query), 10)?;
        snippets = self
            .highlighter()
            .highlight_max_passages("body", &query, &top_docs, 2)?;
        assert_snippet(
            "one <b>bravo</b> three... four <b>bravo</b> six",
            &snippets[0],
        );

        // and add just enough slop to cross the values:
        phrase_query = self.new_phrase_query("body", "three four")?;
        phrase_query = Self::set_slop(&phrase_query, gap)?; // just enough to span
        query = bq(vec![
            (tq("body", "bravo"), Occur::MUST),
            (phrase_query, Occur::MUST), // must match and it will
        ])?;
        top_docs = self.searcher().search(Arc::clone(&query), 10)?;
        Assert::assert_equals(1, top_docs.total_hits.value());
        snippets = self
            .highlighter()
            .highlight_max_passages("body", &query, &top_docs, 2)?;
        if self.weight_matches() {
            assert_snippet(
                "one <b>bravo</b> <b>three</b>... four <b>bravo</b> six",
                &snippets[0],
            );
        } else {
            assert_snippet(
                "one <b>bravo</b> <b>three</b>... <b>four</b> <b>bravo</b> six",
                &snippets[0],
            );
        }
        Ok(())
    }

    fn test_max_len(&mut self) -> Result<()> {
        self.add_doc(&["alpha bravo charlie - gap alpha bravo"])?; // hyphen is at char 21
        self.init_reader_searcher_highlighter()?;
        let uh_builder = self
            .uh_builder
            .take()
            .expect("initReaderSearcherHighlighter ran")
            .with_max_length(21)?;
        self.highlighter = Some(uh_builder.clone().build());
        self.uh_builder = Some(uh_builder);

        let query = bq(vec![
            (self.new_phrase_query("body", "alpha bravo")?, Occur::SHOULD),
            (self.new_phrase_query("body", "gap alpha")?, Occur::SHOULD),
            (self.new_phrase_query("body", "charlie gap")?, Occur::SHOULD),
        ])?;

        let snippets = self.highlight(&query)?;
        let weight_matches = self.weight_matches();
        if self.b.is_reanalysis() || weight_matches {
            if weight_matches {
                assert_snippets(&["<b>alpha bravo</b> charlie -"], &snippets);
            } else {
                assert_snippets(&["<b>alpha</b> <b>bravo</b> charlie -"], &snippets);
            }
        } else {
            assert_snippets(&["<b>alpha</b> <b>bravo</b> <b>charlie</b> -"], &snippets);
        }
        Ok(())
    }

    fn test_filtered_out_span(&mut self) -> Result<()> {
        self.add_doc(&["freezing cold stuff like stuff freedom of speech"])?;
        self.init_reader_searcher_highlighter()?;

        let wildcard_span_query: Arc<dyn SpanQuery> = Arc::new(SpanMultiTermQueryWrapper::new(
            Arc::new(WildcardQuery::new(term("body", "free*"))?),
        ));
        let term_query = span_term("body", "speech");
        let span_query = span_near(vec![wildcard_span_query, term_query], 3, false)?;

        let query = bq(vec![(span_query, Occur::MUST)])?;
        let snippets = self.highlight(&query)?;
        // spans' MatchesIterator exposes each underlying term; thus doesn't enclose intermediate "of"
        assert_snippets(
            &["freezing cold stuff like stuff <b>freedom</b> of <b>speech</b>"],
            &snippets,
        );
        Ok(())
    }

    fn test_match_no_docs_query(&mut self) -> Result<()> {
        let highlighter = UnifiedHighlighter::builder_without_searcher(self.b.analyzer())
            .with_highlight_phrases_strictly(true)
            .build();
        let content = "whatever";
        let query: Arc<dyn Query> = Arc::new(MatchNoDocsQuery::instance());
        let o = highlighter.highlight_without_searcher("body", &query, content, 1)?;
        self.highlighter = Some(highlighter);
        // assertEquals(Object, Object): the snippet object equals the String content
        Assert::assert_equals_str(None, Some(content), o.map(|o| o.to_string()).as_deref());
        Ok(())
    }

    fn test_pre_span_query_rewrite(&mut self) -> Result<()> {
        self.add_doc(&["There is no accord and satisfaction with this - Consideration of the accord is arbitrary."])?;
        self.init_reader_searcher_highlighter()?;
        let uh_builder =
            builder(self.searcher(), self.b.analyzer()).with_highlight_phrases_strictly(true);
        let mut highlighter = UnifiedHighlighter::from_builder(uh_builder);
        highlighter.set_overrides(Arc::new(PreSpanQueryRewrite));
        self.highlighter = Some(highlighter);

        let phrase_query: Arc<dyn Query> = Arc::new(BoostQuery::new(
            pq(0, "body", &["accord", "and", "satisfaction"])?,
            2.0,
        )?);
        let mut ored = BooleanQueryBuilder::new();
        ored.set_minimum_number_should_match(2);
        ored.add(tq("body", "accord"), Occur::SHOULD)?;
        ored.add(tq("body", "satisfaction"), Occur::SHOULD)?;
        ored.add(tq("body", "consideration"), Occur::SHOULD)?;
        let ored_terms: Arc<dyn Query> = Arc::new(ored.build());
        let proximity_boosting_query: Arc<dyn Query> = Arc::new(MyQuery {
            wrapped: ored_terms,
        });
        let total_query = bq(vec![
            (phrase_query, Occur::SHOULD),
            (proximity_boosting_query, Occur::SHOULD),
        ])?;
        let top_docs = search_index_order(self.searcher(), &total_query, 10)?;
        Assert::assert_equals(1, top_docs.total_hits.value());
        let snippets = self
            .highlighter()
            .highlight("body", &total_query, &top_docs)?;
        assert_snippets(
            &["There is no <b>accord</b> <b>and</b> <b>satisfaction</b> with this - <b>Consideration</b> of the <b>accord</b> is arbitrary."],
            &snippets,
        );
        Ok(())
    }

    // Tests that terms collected out of order due to being present in multiple Spans are handled
    // correctly
    // See LUCENE-8365
    fn test_reverse_order_span_collection(&mut self) -> Result<()> {
        // Processing order may depend on various optimizations or other weird factor.
        self.add_doc(&["alpha bravo - alpha charlie"])?;
        self.add_doc(&["alpha charlie - alpha bravo"])?;
        self.init_reader_searcher_highlighter()?;

        let query: Arc<dyn Query> = span_near(
            vec![
                span_near(
                    vec![span_term("body", "alpha"), span_term("body", "bravo")],
                    0,
                    true,
                )?,
                span_near(
                    vec![span_term("body", "alpha"), span_term("body", "charlie")],
                    0,
                    true,
                )?,
            ],
            10,
            false,
        )?;

        let snippets = self.highlight(&query)?;
        assert_snippets(
            &[
                "<b>alpha</b> <b>bravo</b> - <b>alpha</b> <b>charlie</b>",
                "<b>alpha</b> <b>charlie</b> - <b>alpha</b> <b>bravo</b>",
            ],
            &snippets,
        );
        Ok(())
    }

    // Ported from LUCENE-5455 (fixed in LUCENE-8121).  Also see LUCENE-2287.
    fn test_nested_span_query_highlight(&mut self) -> Result<()> {
        // For a long time, the highlighters used to assume all query terms within the SpanQuery were
        // valid at the Spans'
        //   position range.  This would highlight occurrences of terms that were actually not matched
        // by the query.
        //   But now using the SpanCollector API we don't make this kind of mistake.
        const FIELD_NAME: &str = "body";
        let indexed_text = "x y z x z x a";
        self.add_doc(&[indexed_text])?;
        self.init_reader_searcher_highlighter()?;
        let top_docs = TopDocs::new(
            TotalHits::new(1, TotalHitsRelation::EQUAL_TO)?,
            vec![ScoreDoc::new(0, 1.0)],
        );

        let mut expected = "<b>x</b> <b>y</b> <b>z</b> x z x <b>a</b>";
        let mut q: Arc<dyn Query> = span_near(
            vec![
                span_near(
                    vec![
                        span_term(FIELD_NAME, "x"),
                        span_term(FIELD_NAME, "y"),
                        span_term(FIELD_NAME, "z"),
                    ],
                    0,
                    true,
                )?,
                span_term(FIELD_NAME, "a"),
            ],
            10,
            false,
        )?;
        let mut observed = self
            .highlighter()
            .highlight(FIELD_NAME, &q, &top_docs)?
            .remove(0);
        if *VERBOSE {
            println!("Expected: \"{expected}\nObserved: \"{observed:?}");
        }
        assert_snippet_msg(
            "Nested SpanNear query not properly highlighted.",
            expected,
            &observed,
        );

        expected = "x <b>y</b> <b>z</b> <b>x</b> <b>z</b> x <b>a</b>";
        q = span_near(
            vec![
                Arc::new(SpanOrQuery::new(vec![
                    span_near(
                        vec![span_term(FIELD_NAME, "x"), span_term(FIELD_NAME, "z")],
                        0,
                        true,
                    )?,
                    span_near(
                        vec![span_term(FIELD_NAME, "y"), span_term(FIELD_NAME, "z")],
                        0,
                        true,
                    )?,
                ])?),
                Arc::new(SpanOrQuery::new(vec![
                    span_term(FIELD_NAME, "a"),
                    span_term(FIELD_NAME, "b"),
                ])?),
            ],
            10,
            false,
        )?;
        observed = self
            .highlighter()
            .highlight(FIELD_NAME, &q, &top_docs)?
            .remove(0);
        if *VERBOSE {
            println!("Expected: \"{expected}\nObserved: \"{observed:?}");
        }
        assert_snippet_msg(
            "Nested SpanNear query within SpanOr not properly highlighted.",
            expected,
            &observed,
        );

        expected = "x <b>y</b> <b>z</b> <b>x</b> <b>z</b> x <b>a</b>";
        q = span_near(
            vec![
                span_near(
                    vec![
                        Arc::new(SpanMultiTermQueryWrapper::new(Arc::new(
                            WildcardQuery::new(term(FIELD_NAME, "*"))?,
                        ))),
                        span_term(FIELD_NAME, "z"),
                    ],
                    0,
                    true,
                )?,
                span_term(FIELD_NAME, "a"),
            ],
            10,
            false,
        )?;
        observed = self
            .highlighter()
            .highlight(FIELD_NAME, &q, &top_docs)?
            .remove(0);
        if *VERBOSE {
            println!("Expected: \"{expected}\nObserved: \"{observed:?}");
        }
        assert_snippet_msg(
            "Nested SpanNear query with wildcard not properly highlighted.",
            expected,
            &observed,
        );

        expected = "<b>x</b> <b>y</b> z x z x <b>a</b>";
        q = span_near(
            vec![
                Arc::new(SpanOrQuery::new(vec![
                    span_near(
                        vec![span_term(FIELD_NAME, "x"), span_term(FIELD_NAME, "y")],
                        0,
                        true,
                    )?,
                    span_near(
                        vec![
                            // No hit span query
                            span_term(FIELD_NAME, "z"),
                            span_term(FIELD_NAME, "a"),
                        ],
                        0,
                        true,
                    )?,
                ])?),
                span_term(FIELD_NAME, "a"),
            ],
            10,
            false,
        )?;
        observed = self
            .highlighter()
            .highlight(FIELD_NAME, &q, &top_docs)?
            .remove(0);
        if *VERBOSE {
            println!("Expected: \"{expected}\nObserved: \"{observed:?}");
        }
        assert_snippet_msg(
            "Nested SpanNear query within SpanOr not properly highlighted.",
            expected,
            &observed,
        );
        Ok(())
    }
}

/// `private static class MyQuery extends Query`.
#[derive(Debug)]
struct MyQuery {
    wrapped: Arc<dyn Query>,
}

impl Query for MyQuery {
    fn create_weight(
        &self,
        searcher: &IndexSearcher,
        score_mode: ScoreMode,
        boost: f32,
    ) -> Result<Arc<dyn Weight>> {
        self.wrapped.create_weight(searcher, score_mode, boost)
    }

    fn rewrite(&self, index_searcher: &IndexSearcher) -> Result<Option<Arc<dyn Query>>> {
        match self.wrapped.rewrite(index_searcher)? {
            Some(new_wrapped) => Ok(Some(Arc::new(MyQuery {
                wrapped: new_wrapped,
            }))),
            None => Ok(None),
        }
    }

    fn to_query_string(&self, field: &str) -> String {
        format!("[[[{}]]]", self.wrapped.to_query_string(field))
    }

    fn query_eq(&self, other: &dyn Query) -> bool {
        // obj.getClass() == getClass() && wrapped.equals(((MyQuery) wrapped).wrapped): the cast
        // is of this query's own `wrapped`
        other.as_any().downcast_ref::<MyQuery>().is_some() && {
            let cast = self
                .wrapped
                .as_any()
                .downcast_ref::<MyQuery>()
                .unwrap_or_else(|| panic!("java.lang.ClassCastException: not a MyQuery"));
            self.wrapped.query_eq(&*cast.wrapped)
        }
    }

    fn query_hash(&self) -> i32 {
        self.wrapped.query_hash()
    }

    fn visit(&self, visitor: &mut dyn QueryVisitor) {
        self.wrapped.visit(visitor);
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.search.uhighlight.TestUnifiedHighlighterStrictPhrases$MyQuery"
    }
}
