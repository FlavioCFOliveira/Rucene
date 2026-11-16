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

//! Port of `TestCommonTermsQuery.java`.

use std::collections::HashSet;
use std::sync::Arc;

use randomizedtesting::jdk::random::{JdkRandom, Random};
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::{Assert, Assume};
use rucene::document::{Document, Store};
use rucene::error::Result;
use rucene::index::index_writer::MAX_TERM_LENGTH;
use rucene::index::{IndexReader, Term, TermStates};
use rucene::search::boolean_clause::Occur;
use rucene::search::boolean_query::Builder as BooleanQueryBuilder;
use rucene::search::boost_query::BoostQuery;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::query::Query;
use rucene::search::similarities::BM25Similarity;
use rucene::search::term_query::TermQuery;
use rucene::util::extra::{PriorityQueue, PriorityQueueComparator};
use rucene::util::BytesRef;
use rucene_queries::common_terms_query::{CommonTermsQuery, CommonTermsQueryOverrides};
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::search::query_utils::QueryUtils;
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::line_file_docs::LineFileDocs;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::queries_support::core_support::{class, random, test, IAE};

/// `TestCommonTermsQuery`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.queries.TestCommonTermsQuery",
        vec![
            test("testBasics", test_basics),
            test("testEqualsHashCode", test_equals_hash_code),
            test("testNullTerm", test_null_term),
            test("testMinShouldMatch", test_min_should_match),
            test("testIllegalOccur", test_illegal_occur),
            test("testExtend", test_extend),
            test("testRandomIndex", test_random_index),
        ],
    )
}

const DOCS: [&str; 4] = [
    "this is the end of the world right",
    "is this it or maybe not",
    "this is the end of the universe as we know it",
    "there is the famous restaurant at the end of the universe",
];

fn term(text: &str) -> Term {
    Term::from_text("field", text)
}

/// `r.storedFields().document(doc).get("id")`.
fn id_of(r: &Arc<dyn IndexReader>, doc: i32) -> Result<Option<String>> {
    Ok(r.stored_fields()?.document(doc)?.get("id"))
}

fn assert_id(expected: &str, r: &Arc<dyn IndexReader>, doc: i32) -> Result<()> {
    Assert::assert_equals_str(None, Some(expected), id_of(r, doc)?.as_deref());
    Ok(())
}

/// `random().nextBoolean() ? 2.0f : 0.5f`.
fn random_max_freq() -> f32 {
    if random().next_boolean() {
        2.0
    } else {
        0.5
    }
}

/// Adds the four documents of the class.
fn add_docs(w: &RandomIndexWriter) -> Result<()> {
    for (i, text) in DOCS.iter().enumerate() {
        let mut doc = Document::new();
        doc.add(LuceneTestCase::new_string_field(
            "id",
            &i.to_string(),
            Store::YES,
        ));
        doc.add(LuceneTestCase::new_text_field("field", text, Store::NO));
        w.add_document(&doc)?;
    }
    Ok(())
}

fn search(s: &IndexSearcher, query: CommonTermsQuery, n: i32) -> Result<rucene::search::TopDocs> {
    s.search(Arc::new(query), n)
}

fn test_basics() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let analyzer = Arc::new(MockAnalyzer::new(&*random()));
    let w = RandomIndexWriter::new(
        &*random(),
        dir.as_directory(),
        Arc::clone(&analyzer) as Arc<dyn rucene::analysis::Analyzer>,
    )?;
    add_docs(&w)?;

    let r = w.get_reader()?;
    let r_reader: Arc<dyn IndexReader> = Arc::clone(&r) as Arc<dyn IndexReader>;
    let s = LuceneTestCase::new_searcher(Arc::clone(&r));
    {
        let mut query = CommonTermsQuery::new(Occur::SHOULD, Occur::SHOULD, random_max_freq())?;
        for t in ["is", "this", "end", "world", "universe", "right"] {
            query.add(term(t));
        }
        let search = search(&s, query, 10)?;
        Assert::assert_equals(search.total_hits.value(), 3);
        assert_id("0", &r_reader, search.score_docs[0].doc)?;
        assert_id("2", &r_reader, search.score_docs[1].doc)?;
        assert_id("3", &r_reader, search.score_docs[2].doc)?;
    }

    {
        // only high freq
        let mut query = CommonTermsQuery::new(Occur::SHOULD, Occur::SHOULD, random_max_freq())?;
        for t in ["is", "this", "end"] {
            query.add(term(t));
        }
        let search = search(&s, query, 10)?;
        Assert::assert_equals(search.total_hits.value(), 2);
        assert_id("0", &r_reader, search.score_docs[0].doc)?;
        assert_id("2", &r_reader, search.score_docs[1].doc)?;
    }

    {
        // low freq is mandatory
        let mut query = CommonTermsQuery::new(Occur::SHOULD, Occur::MUST, random_max_freq())?;
        for t in ["is", "this", "end", "world"] {
            query.add(term(t));
        }
        let search = search(&s, query, 10)?;
        Assert::assert_equals(search.total_hits.value(), 1);
        assert_id("0", &r_reader, search.score_docs[0].doc)?;
    }

    {
        // low freq is mandatory
        let mut query = CommonTermsQuery::new(Occur::SHOULD, Occur::MUST, random_max_freq())?;
        for t in ["restaurant", "universe"] {
            query.add(term(t));
        }
        let search = search(&s, query, 10)?;
        Assert::assert_equals(search.total_hits.value(), 1);
        assert_id("3", &r_reader, search.score_docs[0].doc)?;
    }
    // IOUtils.close(r, w, dir, analyzer)
    drop(s);
    r.close()?;
    w.close()?;
    dir.close()?;
    (analyzer as Arc<dyn rucene::analysis::Analyzer>).close();
    Ok(())
}

/// `randomOccur(Random)`.
fn random_occur(random: &dyn Random) -> Occur {
    if random.next_boolean() {
        Occur::MUST
    } else {
        Occur::SHOULD
    }
}

/// `new Term(randomRealisticUnicodeString(r), randomRealisticUnicodeString(r))`.
fn random_term(r: &dyn Random) -> Term {
    let field = TestUtil::random_realistic_unicode_string(r);
    let text = TestUtil::random_realistic_unicode_string(r);
    Term::new(&field, BytesRef::from_char_sequence(&text))
}

fn test_equals_hash_code() -> Result<()> {
    let mut query = CommonTermsQuery::new(
        random_occur(&*random()),
        random_occur(&*random()),
        random().next_float(),
    )?;
    let terms = LuceneTestCase::at_least(2);
    for _ in 0..terms {
        query.add(random_term(&*random()));
    }
    QueryUtils::check_hash_equals(&query);
    QueryUtils::check_unequal(
        &CommonTermsQuery::new(
            random_occur(&*random()),
            random_occur(&*random()),
            random().next_float(),
        )?,
        &query,
    );

    {
        let seed = random().next_long();
        let r = JdkRandom::new(seed);
        let mut left = CommonTermsQuery::new(random_occur(&r), random_occur(&r), r.next_float())?;
        let left_terms = LuceneTestCase::at_least_with(&r, 2);
        for _ in 0..left_terms {
            left.add(random_term(&r));
        }
        left.set_high_freq_minimum_number_should_match(r.next_int_bound(4) as f32);
        left.set_low_freq_minimum_number_should_match(r.next_int_bound(4) as f32);

        let r = JdkRandom::new(seed);
        let mut right = CommonTermsQuery::new(random_occur(&r), random_occur(&r), r.next_float())?;
        let right_terms = LuceneTestCase::at_least_with(&r, 2);
        for _ in 0..right_terms {
            right.add(random_term(&r));
        }
        right.set_high_freq_minimum_number_should_match(r.next_int_bound(4) as f32);
        right.set_low_freq_minimum_number_should_match(r.next_int_bound(4) as f32);
        QueryUtils::check_equal(&left, &right);
    }
    Ok(())
}

/// `testNullTerm`: Java's `query.add(null)` throws
/// `IllegalArgumentException`; the port's `CommonTermsQuery::add` takes a
/// `Term`, which cannot be `null`, so the call is rejected at compile time
/// (the declared divergence of `CommonTermsQuery::add`). The query is still
/// built from the same random draws, and the method stays registered so the
/// suite keeps Java's method list.
fn test_null_term() -> Result<()> {
    let random = random();
    let _query = CommonTermsQuery::new(
        random_occur(&*random),
        random_occur(&*random),
        crate::queries_support::core_support::random().next_float(),
    )?;
    // null values are not supported
    Ok(())
}

fn test_min_should_match() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let analyzer = Arc::new(MockAnalyzer::new(&*random()));
    let config = LuceneTestCase::new_index_writer_config_analyzer(Some(
        Arc::clone(&analyzer) as Arc<dyn rucene::analysis::Analyzer>
    ));
    config.set_merge_policy(Some(LuceneTestCase::new_merge_policy_include_mock(
        &*random(),
        false,
    )))?;
    let w = RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), config)?;
    add_docs(&w)?;

    let r = w.get_reader()?;
    let r_reader: Arc<dyn IndexReader> = Arc::clone(&r) as Arc<dyn IndexReader>;
    let s = LuceneTestCase::new_searcher(Arc::clone(&r));
    let six = || -> Result<CommonTermsQuery> {
        let mut query = CommonTermsQuery::new(Occur::SHOULD, Occur::SHOULD, random_max_freq())?;
        for t in ["is", "this", "end", "world", "universe", "right"] {
            query.add(term(t));
        }
        Ok(query)
    };
    {
        let mut query = six()?;
        query.set_low_freq_minimum_number_should_match(0.5);
        let search = search(&s, query, 10)?;
        Assert::assert_equals(search.total_hits.value(), 1);
        assert_id("0", &r_reader, search.score_docs[0].doc)?;
    }
    {
        let mut query = six()?;
        query.set_low_freq_minimum_number_should_match(2.0);
        let search = search(&s, query, 10)?;
        Assert::assert_equals(search.total_hits.value(), 1);
        assert_id("0", &r_reader, search.score_docs[0].doc)?;
    }

    {
        let mut query = six()?;
        query.set_low_freq_minimum_number_should_match(0.49);
        let search = search(&s, query, 10)?;
        Assert::assert_equals(search.total_hits.value(), 3);
        assert_id("0", &r_reader, search.score_docs[0].doc)?;
        assert_id("2", &r_reader, search.score_docs[1].doc)?;
        assert_id("3", &r_reader, search.score_docs[2].doc)?;
    }

    {
        let mut query = six()?;
        query.set_low_freq_minimum_number_should_match(1.0);
        let search = search(&s, query, 10)?;
        Assert::assert_equals(search.total_hits.value(), 3);
        assert_id("0", &r_reader, search.score_docs[0].doc)?;
        assert_id("2", &r_reader, search.score_docs[1].doc)?;
        assert_id("3", &r_reader, search.score_docs[2].doc)?;
        Assert::assert_true(search.score_docs[1].score >= search.score_docs[2].score);
    }

    {
        let mut query = six()?;
        query.set_low_freq_minimum_number_should_match(1.0);
        query.set_high_freq_minimum_number_should_match(4.0);
        let search = search(&s, query, 10)?;
        Assert::assert_equals(search.total_hits.value(), 3);
        Assert::assert_equals_float_msg(
            None,
            search.score_docs[1].score,
            search.score_docs[2].score,
            0.0,
        );
        assert_id("0", &r_reader, search.score_docs[0].doc)?;
        // doc 2 and 3 only get a score from low freq terms
        let expected: HashSet<Option<String>> = [Some("2".to_string()), Some("3".to_string())]
            .into_iter()
            .collect();
        let actual: HashSet<Option<String>> = [
            id_of(&r_reader, search.score_docs[1].doc)?,
            id_of(&r_reader, search.score_docs[2].doc)?,
        ]
        .into_iter()
        .collect();
        Assert::assert_true_msg(
            &format!("expected:<{expected:?}> but was:<{actual:?}>"),
            expected == actual,
        );
    }

    {
        // only high freq terms around - check that min should match is applied
        let mut query = CommonTermsQuery::new(Occur::SHOULD, Occur::SHOULD, random_max_freq())?;
        for t in ["is", "this", "the"] {
            query.add(term(t));
        }
        query.set_low_freq_minimum_number_should_match(1.0);
        query.set_high_freq_minimum_number_should_match(2.0);
        let search = search(&s, query, 10)?;
        Assert::assert_equals(search.total_hits.value(), 4);
    }

    {
        // only high freq terms around - check that min should match is applied
        let mut query = CommonTermsQuery::new(Occur::MUST, Occur::SHOULD, random_max_freq())?;
        for t in ["is", "this", "the"] {
            query.add(term(t));
        }
        query.set_low_freq_minimum_number_should_match(1.0);
        query.set_high_freq_minimum_number_should_match(2.0);
        let search = search(&s, query, 10)?;
        Assert::assert_equals(search.total_hits.value(), 2);
        let expected: HashSet<Option<String>> = [Some("0".to_string()), Some("2".to_string())]
            .into_iter()
            .collect();
        let actual: HashSet<Option<String>> = [
            id_of(&r_reader, search.score_docs[0].doc)?,
            id_of(&r_reader, search.score_docs[1].doc)?,
        ]
        .into_iter()
        .collect();
        Assert::assert_true_msg(
            &format!("expected:<{expected:?}> but was:<{actual:?}>"),
            expected == actual,
        );
    }
    // IOUtils.close(r, w, dir, analyzer)
    drop(s);
    r.close()?;
    w.close()?;
    dir.close()?;
    (analyzer as Arc<dyn rucene::analysis::Analyzer>).close();
    Ok(())
}

/// MUST_NOT is not supported.
fn test_illegal_occur() -> Result<()> {
    let random = random();
    expect_throws(IAE, || {
        CommonTermsQuery::new(
            Occur::MUST_NOT,
            random_occur(&*random),
            crate::queries_support::core_support::random().next_float(),
        )
    });
    expect_throws(IAE, || {
        CommonTermsQuery::new(
            random_occur(&*random),
            Occur::MUST_NOT,
            crate::queries_support::core_support::random().next_float(),
        )
    });
    Ok(())
}

/// `ExtendedCommonTermsQuery`: boosts the term query of `universe` by 100.
#[derive(Debug)]
struct ExtendedCommonTermsQuery;

impl CommonTermsQueryOverrides for ExtendedCommonTermsQuery {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.queries.TestCommonTermsQuery$ExtendedCommonTermsQuery"
    }

    fn new_term_query(
        &self,
        query: &CommonTermsQuery,
        term: Term,
        term_states: Option<Arc<TermStates>>,
    ) -> Arc<dyn Query> {
        let is_universe = term.text() == "universe";
        let mut q = query.super_new_term_query(term, term_states);
        if is_universe {
            q =
                Arc::new(BoostQuery::new(q, 100.0).unwrap_or_else(|e| {
                    rucene_test_framework::util::java_exceptions::rethrow_java(e)
                }));
        }
        q
    }
}

fn test_extend() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let analyzer = Arc::new(MockAnalyzer::new(&*random()));
    let w = RandomIndexWriter::new(
        &*random(),
        dir.as_directory(),
        Arc::clone(&analyzer) as Arc<dyn rucene::analysis::Analyzer>,
    )?;
    add_docs(&w)?;
    let r = w.get_reader()?;
    let r_reader: Arc<dyn IndexReader> = Arc::clone(&r) as Arc<dyn IndexReader>;
    let mut s = LuceneTestCase::new_searcher(Arc::clone(&r));
    // don't use a randomized similarity, e.g. stopwords for DFI can get scored as 0,
    // so boosting them is kind of crazy
    s.set_similarity(Arc::new(BM25Similarity::new()));
    {
        let mut query = CommonTermsQuery::new(Occur::SHOULD, Occur::SHOULD, random_max_freq())?;
        for t in ["is", "this", "end", "world", "universe", "right"] {
            query.add(term(t));
        }
        let search = search(&s, query, 10)?;
        Assert::assert_equals(search.total_hits.value(), 3);
        assert_id("0", &r_reader, search.score_docs[0].doc)?;
        assert_id("2", &r_reader, search.score_docs[1].doc)?;
        assert_id("3", &r_reader, search.score_docs[2].doc)?;
    }

    {
        // this one boosts the termQuery("field" "universe") by 10x
        let mut query = CommonTermsQuery::with_overrides(
            Occur::SHOULD,
            Occur::SHOULD,
            random_max_freq(),
            Arc::new(ExtendedCommonTermsQuery),
        )?;
        for t in ["is", "this", "end", "world", "universe", "right"] {
            query.add(term(t));
        }
        let search = search(&s, query, 10)?;
        Assert::assert_equals(search.total_hits.value(), 3);
        assert_id("2", &r_reader, search.score_docs[0].doc)?;
        assert_id("3", &r_reader, search.score_docs[1].doc)?;
        assert_id("0", &r_reader, search.score_docs[2].doc)?;
    }
    // IOUtils.close(r, w, dir, analyzer)
    drop(s);
    r.close()?;
    w.close()?;
    dir.close()?;
    (analyzer as Arc<dyn rucene::analysis::Analyzer>).close();
    Ok(())
}

/// `TermAndFreq`.
#[derive(Debug)]
struct TermAndFreq {
    term: BytesRef,
    freq: i32,
}

/// The anonymous `PriorityQueue` of the low frequencies: `a.freq > b.freq`.
struct LowFreqLessThan;

impl PriorityQueueComparator<TermAndFreq> for LowFreqLessThan {
    fn less_than(&self, a: &TermAndFreq, b: &TermAndFreq) -> bool {
        a.freq > b.freq
    }
}

/// The anonymous `PriorityQueue` of the high frequencies: `a.freq < b.freq`.
struct HighFreqLessThan;

impl PriorityQueueComparator<TermAndFreq> for HighFreqLessThan {
    fn less_than(&self, a: &TermAndFreq, b: &TermAndFreq) -> bool {
        a.freq < b.freq
    }
}

/// `queueToList(PriorityQueue)`.
fn queue_to_list<C: PriorityQueueComparator<TermAndFreq>>(
    queue: &mut PriorityQueue<TermAndFreq, C>,
) -> Vec<TermAndFreq> {
    let mut terms = Vec::new();
    while queue.size() > 0 {
        terms.push(queue.pop().expect("size > 0"));
    }
    terms
}

/// Populates a writer with random stuff. This must be fully reproducable
/// with the seed!
fn create_random_index(numdocs: i32, writer: &RandomIndexWriter, seed: i64) -> Result<()> {
    let random = JdkRandom::new(seed);
    // primary source for our data is from linefiledocs, it's realistic.
    let line_file_docs = LineFileDocs::with_default_file(&random)?;

    // TODO: we should add other fields that use things like docs&freqs but omit
    // positions,
    // because linefiledocs doesn't cover all the possibilities.
    for _ in 0..numdocs {
        let doc = line_file_docs.next_doc()?;
        writer.add_document(&*doc.borrow())?;
    }

    line_file_docs.close();
    Ok(())
}

fn test_random_index() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let analyzer = Arc::new(MockAnalyzer::new(&*random()));
    analyzer.set_max_token_length(TestUtil::next_int(&*random(), 1, MAX_TERM_LENGTH as i32));
    let w = RandomIndexWriter::new(
        &*random(),
        dir.as_directory(),
        Arc::clone(&analyzer) as Arc<dyn rucene::analysis::Analyzer>,
    )?;
    create_random_index(LuceneTestCase::at_least(50), &w, random().next_long())?;
    w.force_merge(1)?;
    let reader = w.get_reader()?;

    // try { ... } finally { IOUtils.close(reader, w, dir, analyzer); }
    let body = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<()> {
        let wrapper =
            LuceneTestCase::get_only_leaf_reader(Arc::clone(&reader) as Arc<dyn IndexReader>)?;
        let field = "body";
        let terms = wrapper.terms(field)?.unwrap_or_else(|| {
            randomizedtesting::jdk::exception::throw_without_message(
                "java.lang.NullPointerException",
            )
        });
        let mut low_freq_queue = PriorityQueue::new(5, LowFreqLessThan)?;
        let mut high_freq_queue = PriorityQueue::new(5, HighFreqLessThan)?;

        let mut iterator = terms.iterator()?;
        while let Some(t) = iterator.next()? {
            let doc_freq = iterator.doc_freq()?;
            if high_freq_queue.size() < 5 {
                high_freq_queue.add(TermAndFreq {
                    term: BytesRef::deep_copy_of(&t),
                    freq: doc_freq,
                });
                low_freq_queue.add(TermAndFreq {
                    term: BytesRef::deep_copy_of(&t),
                    freq: doc_freq,
                });
            } else {
                if high_freq_queue.top().expect("full").freq < doc_freq {
                    let top = high_freq_queue.top_mut().expect("full");
                    top.freq = doc_freq;
                    top.term = BytesRef::deep_copy_of(&t);
                    high_freq_queue.update_top();
                }

                if low_freq_queue.top().expect("full").freq > doc_freq {
                    let top = low_freq_queue.top_mut().expect("full");
                    top.freq = doc_freq;
                    top.term = BytesRef::deep_copy_of(&t);
                    low_freq_queue.update_top();
                }
            }
        }
        let low_freq = low_freq_queue.top().expect("non-empty").freq;
        let high_freq = high_freq_queue.top().expect("non-empty").freq;
        Assume::assume_true_msg("unlucky index", high_freq - 1 > low_freq);
        let high_terms = queue_to_list(&mut high_freq_queue);
        let low_terms = queue_to_list(&mut low_freq_queue);

        let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
        let low_freq_occur = random_occur(&*random());
        let mut verify_query = BooleanQueryBuilder::new();
        let mut cq = CommonTermsQuery::new(
            random_occur(&*random()),
            low_freq_occur,
            (high_freq - 1) as f32,
        )?;
        for term_and_freq in &low_terms {
            cq.add(Term::new(field, term_and_freq.term.clone()));
            verify_query.add(
                Arc::new(TermQuery::new(Term::new(field, term_and_freq.term.clone()))),
                low_freq_occur,
            )?;
        }
        for term_and_freq in &high_terms {
            cq.add(Term::new(field, term_and_freq.term.clone()));
        }

        let cq: Arc<dyn Query> = Arc::new(cq);
        let max_doc = (Arc::clone(&reader) as Arc<dyn IndexReader>).max_doc();
        let cq_search = searcher.search(Arc::clone(&cq), max_doc)?;

        let verify_search = searcher.search(Arc::new(verify_query.build()), max_doc)?;
        Assert::assert_equals(
            verify_search.total_hits.value(),
            cq_search.total_hits.value(),
        );
        let mut hits: HashSet<i32> = HashSet::new();
        for doc in &verify_search.score_docs {
            hits.insert(doc.doc);
        }

        for doc in &cq_search.score_docs {
            Assert::assert_true(hits.remove(&doc.doc));
        }

        Assert::assert_true(hits.is_empty());

        /*
         *  need to force merge here since QueryUtils adds checks based
         *  on leave readers which have different statistics than the top
         *  level reader if we have more than one segment. This could
         *  result in a different query / results.
         */
        w.force_merge(1)?;
        let reader2 = w.get_reader()?;
        QueryUtils::check_searcher(
            &*random(),
            &cq,
            Some(&LuceneTestCase::new_searcher(Arc::clone(&reader2))),
        );
        reader2.close()?;
        Ok(())
    }));
    let closed = (|| -> Result<()> {
        reader.close()?;
        w.close()?;
        dir.close()?;
        (analyzer as Arc<dyn rucene::analysis::Analyzer>).close();
        Ok(())
    })();
    match body {
        Ok(result) => result.and(closed),
        Err(payload) => std::panic::resume_unwind(payload),
    }
}
