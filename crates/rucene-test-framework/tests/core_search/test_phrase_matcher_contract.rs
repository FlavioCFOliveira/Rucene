//! Port of `lucene/core/src/test/org/apache/lucene/search/TestPhraseMatcherContract.java`.
//!
//! Tests the `PhraseMatcher` contract, particularly the ability to call
//! `PhraseMatcher#maxFreq()` before `PhraseMatcher#resetPositions()`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::{Document, Store, TextField};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::postings_enum::POSTINGS_ENUM_ALL;
use rucene::index::{IndexReader, Term, TermStates};
use rucene::search::doc_id_set_iterator::NO_MORE_DOCS;
use rucene::search::exact_phrase_matcher::ExactPhraseMatcher;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::phrase_matcher::{PhraseMatcher, SharedPostings};
use rucene::search::phrase_query::PostingsAndFreq;
use rucene::search::sim_scorer_source::SharedSimScorer;
use rucene::search::similarities::{BM25Similarity, Similarity};
use rucene::search::sloppy_phrase_matcher::SloppyPhraseMatcher;
use rucene::search::ScoreMode;
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_search::support::term;
use crate::core_support::{assert_not_equals, assert_not_null, class, random, test};

/// `TestPhraseMatcherContract`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.TestPhraseMatcherContract",
        vec![
            test(
                "testExactPhraseMatcherContract",
                test_exact_phrase_matcher_contract,
            ),
            test(
                "testSloppyPhraseMatcherContract",
                test_sloppy_phrase_matcher_contract,
            ),
        ],
    )
}

/// The postings of `t1` and `t2` (positions 0 and 1) on the only leaf, and
/// the BM25 scorer over their statistics, as both tests build them.
fn postings_and_scorer(
    reader: &Arc<dyn IndexReader>,
    t1: &Term,
    t2: &Term,
) -> Result<(Vec<PostingsAndFreq>, SharedSimScorer)> {
    let leaves = Arc::clone(reader).leaves()?;
    let context = &leaves[0];
    let body_terms = context
        .leaf_reader()
        .terms("body")?
        .expect("body is indexed");
    let mut te1 = body_terms.iterator()?;
    te1.seek_exact(t1.bytes())?;
    let pe1 = te1.impacts(POSTINGS_ENUM_ALL)?;
    let mut te2 = body_terms.iterator()?;
    te2.seek_exact(t2.bytes())?;
    let pe2 = te2.impacts(POSTINGS_ENUM_ALL)?;
    // `new PhraseQuery.PostingsAndFreq(pe, (ImpactsEnum) pe, position, t)`:
    // the shared postings answer both roles.
    let postings = vec![
        PostingsAndFreq::new(SharedPostings::new(pe1), 0, vec![t1.clone()]),
        PostingsAndFreq::new(SharedPostings::new(pe2), 1, vec![t2.clone()]),
    ];

    let searcher = IndexSearcher::new(Arc::clone(reader))?;
    let field_stats = searcher.collection_statistics("body")?;
    assert_not_null(&field_stats);
    let field_stats = field_stats.expect("assertNotNull(fieldStats)");
    // Same stats path as PhraseWeight#getStats (PhraseQuery#createWeight)
    let ts1 = TermStates::build(&searcher, t1, true)?;
    let ts2 = TermStates::build(&searcher, t2, true)?;
    Assert::assert_true(ts1.doc_freq()? > 0);
    Assert::assert_true(ts2.doc_freq()? > 0);
    let scorer: SharedSimScorer = Arc::from(Arc::new(BM25Similarity::new()).scorer_owned(
        1.0,
        &field_stats,
        &[
            searcher.term_statistics(t1, ts1.doc_freq()?, ts1.total_term_freq()?)?,
            searcher.term_statistics(t2, ts2.doc_freq()?, ts2.total_term_freq()?)?,
        ],
    ));
    Ok((postings, scorer))
}

/// The contract checks both tests make on their matcher.
fn check_contract(matcher: &mut dyn PhraseMatcher) -> Result<()> {
    let doc_id = matcher.approximation().next_doc()?;
    assert_not_equals(&NO_MORE_DOCS, &doc_id);

    // CALL maxFreq BEFORE resetPositions
    let max_freq = matcher.max_freq()?;
    Assert::assert_true(max_freq >= 1.0);

    // Then resetPositions
    matcher.reset_positions()?;
    Assert::assert_true_msg(
        &format!("Should have a match in doc {doc_id}"),
        matcher.next_match()?,
    );
    Assert::assert_false(matcher.next_match()?);
    Ok(())
}

/// One document with `text` in `body`.
fn one_doc_index(
    text: &str,
) -> Result<rucene_test_framework::util::lucene_test_case::TestDirectory> {
    let dir = LuceneTestCase::new_directory();
    let fw = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    let mut doc = Document::new();
    doc.add(Box::new(TextField::new(
        "body",
        text.to_string(),
        Store::NO,
    )?));
    fw.add_document(&doc)?;
    fw.close()?;
    Ok(dir)
}

/// Port of `TestPhraseMatcherContract.testExactPhraseMatcherContract`.
fn test_exact_phrase_matcher_contract() -> Result<()> {
    let mut dir = one_doc_index("the quick brown fox")?;
    {
        let reader = directory_reader::open(dir.as_directory())?;
        let ir = Arc::clone(&reader) as Arc<dyn IndexReader>;
        let t1 = term("body", "quick");
        let t2 = term("body", "brown");
        let (postings, scorer) = postings_and_scorer(&ir, &t1, &t2)?;
        let mut matcher = ExactPhraseMatcher::new(postings, ScoreMode::TOP_SCORES, scorer, 1.0)?;
        check_contract(&mut matcher)?;
        drop(matcher);
        reader.close()?;
    }
    dir.close()
}

/// Port of `TestPhraseMatcherContract.testSloppyPhraseMatcherContract`.
fn test_sloppy_phrase_matcher_contract() -> Result<()> {
    let mut dir = one_doc_index("the quick silver fox")?;
    {
        let reader = directory_reader::open(dir.as_directory())?;
        let ir = Arc::clone(&reader) as Arc<dyn IndexReader>;
        let t1 = term("body", "quick");
        let t2 = term("body", "fox");
        let (postings, scorer) = postings_and_scorer(&ir, &t1, &t2)?;
        // Slop 2 allows "quick silver fox"
        let mut matcher =
            SloppyPhraseMatcher::new(postings, 2, ScoreMode::TOP_SCORES, scorer, 1.0, false)?;
        check_contract(&mut matcher)?;
        drop(matcher);
        reader.close()?;
    }
    dir.close()
}
