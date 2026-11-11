//! Port of `lucene/core/src/test/org/apache/lucene/search/TestMatchesIterator.java`.
//!
//! `TestMatchesIterator extends MatchesTestBase`: the fixture
//! (`setUp()`/`tearDown()`, `searcher`, `reader`) and the `check*` helpers
//! are `rucene_test_framework::search::matches_test_base`.

use std::collections::HashSet;
use std::fmt;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;

use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::reflect::{Class, Constructor};
use randomizedtesting::junit::Assert;
use rucene::document::{IntPoint, NumericDocValuesField};
use rucene::error::Result;
use rucene::index::filter_reader::{FilterLeafReader, FilterTerms, FilterTermsEnum};
use rucene::index::index_reader::CacheHelper;
use rucene::index::multi_reader::reader_util::sub_index_from_leaves;
use rucene::index::postings_enum::{ImpactsEnum, PostingsEnum};
use rucene::index::{IndexReader, LeafReader, SeekStatus, TermState, Terms, TermsEnum};
use rucene::search::disjunction_matches_iterator;
use rucene::search::index_or_doc_values_query::IndexOrDocValuesQuery;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::matches::MatchesIterator;
use rucene::search::multi_phrase_query::Builder as MultiPhraseQueryBuilder;
use rucene::search::named_matches;
use rucene::search::phrase_query::{Builder as PhraseQueryBuilder, PhraseQuery};
use rucene::search::prefix_query::PrefixQuery;
use rucene::search::regexp_query::RegexpQuery;
use rucene::search::synonym_query::Builder as SynonymQueryBuilder;
use rucene::search::{BooleanQueryBuilder, Occur, Query, ScoreMode};
use rucene::util::attribute::AttributeSource;
use rucene::util::{Bits, BytesRef, IOBooleanSupplier};
use rucene_test_framework::search::asserting_matches::AssertingMatches;
use rucene_test_framework::search::matches_test_base::{
    self, MatchesState, MatchesTestBase, MatchesTestInstance, FIELD_DOCS_ONLY, FIELD_FREQS,
    FIELD_NO_OFFSETS, FIELD_POINT, FIELD_WITH_OFFSETS,
};
use rucene_test_framework::util::LuceneTestCase;

use crate::core_search::support::{ftest, term, tq};
use crate::core_support::{arrays_to_string, assert_equals, assert_equals_msg, assert_null};

/// `TestMatchesIterator extends MatchesTestBase`.
struct TestMatchesIterator;

impl MatchesTestBase for TestMatchesIterator {
    fn get_documents(&self) -> Vec<String> {
        [
            "w1 w2 w3 w4 w5",
            "w1 w3 w2 w3 zz",
            "w1 xx w2 yy w4",
            "w1 w2 w1 w4 w2 w3",
            "a phrase sentence with many phrase sentence iterations of a phrase sentence",
            "nothing matches this document",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect()
    }
}

type Inst = MatchesTestInstance;

/// `TestMatchesIterator`.
pub fn class_def() -> Arc<Class> {
    let methods = vec![
        ftest::<Inst>("testTermQuery", test_term_query),
        ftest::<Inst>(
            "testTermQueryNoStoredOffsets",
            test_term_query_no_stored_offsets,
        ),
        ftest::<Inst>("testTermQueryNoPositions", test_term_query_no_positions),
        ftest::<Inst>("testDisjunction", test_disjunction),
        ftest::<Inst>("testDisjunctionNoPositions", test_disjunction_no_positions),
        ftest::<Inst>("testReqOpt", test_req_opt),
        ftest::<Inst>("testReqOptNoPositions", test_req_opt_no_positions),
        ftest::<Inst>("testMinShouldMatch", test_min_should_match),
        ftest::<Inst>(
            "testMinShouldMatchNoPositions",
            test_min_should_match_no_positions,
        ),
        ftest::<Inst>("testExclusion", test_exclusion),
        ftest::<Inst>("testExclusionNoPositions", test_exclusion_no_positions),
        ftest::<Inst>("testConjunction", test_conjunction),
        ftest::<Inst>("testConjunctionNoPositions", test_conjunction_no_positions),
        ftest::<Inst>("testWildcards", test_wildcards),
        ftest::<Inst>("testNoMatchWildcards", test_no_match_wildcards),
        ftest::<Inst>("testWildcardsNoPositions", test_wildcards_no_positions),
        ftest::<Inst>("testSynonymQuery", test_synonym_query),
        ftest::<Inst>(
            "testSynonymQueryNoPositions",
            test_synonym_query_no_positions,
        ),
        ftest::<Inst>("testMultipleFields", test_multiple_fields),
        ftest::<Inst>(
            "testSloppyPhraseQueryWithRepeats",
            test_sloppy_phrase_query_with_repeats,
        ),
        ftest::<Inst>("testSloppyPhraseQuery", test_sloppy_phrase_query),
        ftest::<Inst>("testExactPhraseQuery", test_exact_phrase_query),
        ftest::<Inst>("testSloppyMultiPhraseQuery", test_sloppy_multi_phrase_query),
        ftest::<Inst>("testExactMultiPhraseQuery", test_exact_multi_phrase_query),
        ftest::<Inst>("testPointQuery", test_point_query),
        ftest::<Inst>(
            "testMinimalSeekingWithWildcards",
            test_minimal_seeking_with_wildcards,
        ),
        ftest::<Inst>("testFromSubIteratorsMethod", test_from_sub_iterators_method),
    ];
    let mut builder = Class::builder("org.apache.lucene.search.TestMatchesIterator")
        .extends(&matches_test_base::class())
        .constructor(Constructor::no_args(|| {
            MatchesTestInstance::new(TestMatchesIterator)
        }));
    for m in methods {
        builder = builder.method(m);
    }
    builder.build()
}

/// The fixture `setUp()` built.
fn state(inst: &Inst) -> &MatchesState {
    inst.state.as_ref().expect("setUp() ran")
}

/// `new int[][] {...}`.
fn rows(rows: &[&[i32]]) -> Vec<Vec<i32>> {
    rows.iter().map(|r| r.to_vec()).collect()
}

/// `new String[][] {...}`.
fn names<'a>(rows: &[&[&'a str]]) -> Vec<Vec<&'a str>> {
    rows.iter().map(|r| r.to_vec()).collect()
}

/// `NamedMatches.wrapQuery(name, new TermQuery(new Term(field, text)))`.
fn named(name: &str, field: &str, text: &str) -> Arc<dyn Query> {
    named_matches::wrap_query(name, tq(field, text))
}

/// `new BooleanQuery.Builder().add(a, occur_a).add(b, occur_b).build()`.
fn bq2(
    a: Arc<dyn Query>,
    occur_a: Occur,
    b: Arc<dyn Query>,
    occur_b: Occur,
) -> Result<Arc<dyn Query>> {
    Ok(Arc::new(
        BooleanQueryBuilder::new()
            .add(a, occur_a)?
            .add(b, occur_b)?
            .build(),
    ))
}

/// Port of `TestMatchesIterator.testTermQuery`.
fn test_term_query(inst: &mut Inst) -> Result<()> {
    let s = state(inst);
    let q = named("q", FIELD_WITH_OFFSETS, "w1");
    s.check_matches(
        Arc::clone(&q),
        FIELD_WITH_OFFSETS,
        &rows(&[
            &[0, 0, 0, 0, 2],
            &[1, 0, 0, 0, 2],
            &[2, 0, 0, 0, 2],
            &[3, 0, 0, 0, 2, 2, 2, 6, 8],
            &[4],
        ]),
    )?;
    s.check_label_count(Arc::clone(&q), FIELD_WITH_OFFSETS, &[1, 1, 1, 1, 0, 0])?;
    s.assert_is_leaf_match(Arc::clone(&q), FIELD_WITH_OFFSETS)?;
    s.check_sub_matches(q, &names(&[&["q"], &["q"], &["q"], &["q"], &[], &[]]))
}

/// Port of `TestMatchesIterator.testTermQueryNoStoredOffsets`.
fn test_term_query_no_stored_offsets(inst: &mut Inst) -> Result<()> {
    let q = tq(FIELD_NO_OFFSETS, "w1");
    state(inst).check_matches(
        q,
        FIELD_NO_OFFSETS,
        &rows(&[
            &[0, 0, 0, -1, -1],
            &[1, 0, 0, -1, -1],
            &[2, 0, 0, -1, -1],
            &[3, 0, 0, -1, -1, 2, 2, -1, -1],
            &[4],
        ]),
    )
}

/// Port of `TestMatchesIterator.testTermQueryNoPositions`.
fn test_term_query_no_positions(inst: &mut Inst) -> Result<()> {
    for field in [FIELD_DOCS_ONLY, FIELD_FREQS] {
        let q = tq(field, "w1");
        state(inst).check_no_positions_matches(q, field, &[true, true, true, true, false])?;
    }
    Ok(())
}

/// Port of `TestMatchesIterator.testDisjunction`.
fn test_disjunction(inst: &mut Inst) -> Result<()> {
    let s = state(inst);
    let w1 = named("w1", FIELD_WITH_OFFSETS, "w1");
    let w3 = named("w3", FIELD_WITH_OFFSETS, "w3");
    let q = bq2(w1, Occur::SHOULD, w3, Occur::SHOULD)?;
    s.check_matches(
        Arc::clone(&q),
        FIELD_WITH_OFFSETS,
        &rows(&[
            &[0, 0, 0, 0, 2, 2, 2, 6, 8],
            &[1, 0, 0, 0, 2, 1, 1, 3, 5, 3, 3, 9, 11],
            &[2, 0, 0, 0, 2],
            &[3, 0, 0, 0, 2, 2, 2, 6, 8, 5, 5, 15, 17],
            &[4],
        ]),
    )?;
    s.check_label_count(Arc::clone(&q), FIELD_WITH_OFFSETS, &[2, 2, 1, 2, 0, 0])?;
    s.assert_is_leaf_match(Arc::clone(&q), FIELD_WITH_OFFSETS)?;
    s.check_sub_matches(
        q,
        &names(&[
            &["w1", "w3"],
            &["w1", "w3"],
            &["w1"],
            &["w1", "w3"],
            &[],
            &[],
        ]),
    )
}

/// Port of `TestMatchesIterator.testDisjunctionNoPositions`.
fn test_disjunction_no_positions(inst: &mut Inst) -> Result<()> {
    for field in [FIELD_DOCS_ONLY, FIELD_FREQS] {
        let q = bq2(
            tq(field, "w1"),
            Occur::SHOULD,
            tq(field, "w3"),
            Occur::SHOULD,
        )?;
        state(inst).check_no_positions_matches(q, field, &[true, true, true, true, false])?;
    }
    Ok(())
}

/// Port of `TestMatchesIterator.testReqOpt`.
fn test_req_opt(inst: &mut Inst) -> Result<()> {
    let s = state(inst);
    let q = bq2(
        tq(FIELD_WITH_OFFSETS, "w1"),
        Occur::SHOULD,
        tq(FIELD_WITH_OFFSETS, "w3"),
        Occur::MUST,
    )?;
    s.check_matches(
        Arc::clone(&q),
        FIELD_WITH_OFFSETS,
        &rows(&[
            &[0, 0, 0, 0, 2, 2, 2, 6, 8],
            &[1, 0, 0, 0, 2, 1, 1, 3, 5, 3, 3, 9, 11],
            &[2],
            &[3, 0, 0, 0, 2, 2, 2, 6, 8, 5, 5, 15, 17],
            &[4],
        ]),
    )?;
    s.check_label_count(q, FIELD_WITH_OFFSETS, &[2, 2, 0, 2, 0, 0])
}

/// Port of `TestMatchesIterator.testReqOptNoPositions`.
fn test_req_opt_no_positions(inst: &mut Inst) -> Result<()> {
    for field in [FIELD_DOCS_ONLY, FIELD_FREQS] {
        let q = bq2(tq(field, "w1"), Occur::SHOULD, tq(field, "w3"), Occur::MUST)?;
        state(inst).check_no_positions_matches(q, field, &[true, true, false, true, false])?;
    }
    Ok(())
}

/// The query of `testMinShouldMatch` and `testMinShouldMatchNoPositions`.
fn min_should_match_query(
    w1: Arc<dyn Query>,
    w3: Arc<dyn Query>,
    w4: Arc<dyn Query>,
    xx: Arc<dyn Query>,
) -> Result<Arc<dyn Query>> {
    Ok(Arc::new(
        BooleanQueryBuilder::new()
            .add(w3, Occur::SHOULD)?
            .add(
                Arc::new(
                    BooleanQueryBuilder::new()
                        .add(w1, Occur::SHOULD)?
                        .add(w4, Occur::SHOULD)?
                        .add(xx, Occur::SHOULD)?
                        .set_minimum_number_should_match(2)
                        .build(),
                ),
                Occur::SHOULD,
            )?
            .build(),
    ))
}

/// Port of `TestMatchesIterator.testMinShouldMatch`.
fn test_min_should_match(inst: &mut Inst) -> Result<()> {
    let s = state(inst);
    let w1 = named("w1", FIELD_WITH_OFFSETS, "w1");
    let w3 = named("w3", FIELD_WITH_OFFSETS, "w3");
    let w4 = tq(FIELD_WITH_OFFSETS, "w4");
    let xx = named("xx", FIELD_WITH_OFFSETS, "xx");
    let q = min_should_match_query(w1, w3, w4, xx)?;
    s.check_matches(
        Arc::clone(&q),
        FIELD_WITH_OFFSETS,
        &rows(&[
            &[0, 0, 0, 0, 2, 2, 2, 6, 8, 3, 3, 9, 11],
            &[1, 1, 1, 3, 5, 3, 3, 9, 11],
            &[2, 0, 0, 0, 2, 1, 1, 3, 5, 4, 4, 12, 14],
            &[3, 0, 0, 0, 2, 2, 2, 6, 8, 3, 3, 9, 11, 5, 5, 15, 17],
            &[4],
        ]),
    )?;
    s.check_label_count(Arc::clone(&q), FIELD_WITH_OFFSETS, &[3, 1, 3, 3, 0, 0])?;
    s.assert_is_leaf_match(Arc::clone(&q), FIELD_WITH_OFFSETS)?;
    s.check_sub_matches(
        q,
        &names(&[
            &["w1", "w3"],
            &["w3"],
            &["w1", "xx"],
            &["w1", "w3"],
            &[],
            &[],
        ]),
    )
}

/// Port of `TestMatchesIterator.testMinShouldMatchNoPositions`.
fn test_min_should_match_no_positions(inst: &mut Inst) -> Result<()> {
    for field in [FIELD_FREQS, FIELD_DOCS_ONLY] {
        let q = min_should_match_query(
            tq(field, "w1"),
            tq(field, "w3"),
            tq(field, "w4"),
            tq(field, "xx"),
        )?;
        state(inst).check_no_positions_matches(q, field, &[true, true, true, true, false])?;
    }
    Ok(())
}

/// Port of `TestMatchesIterator.testExclusion`.
fn test_exclusion(inst: &mut Inst) -> Result<()> {
    let q = bq2(
        tq(FIELD_WITH_OFFSETS, "w3"),
        Occur::SHOULD,
        tq(FIELD_WITH_OFFSETS, "zz"),
        Occur::MUST_NOT,
    )?;
    state(inst).check_matches(
        q,
        FIELD_WITH_OFFSETS,
        &rows(&[&[0, 2, 2, 6, 8], &[1], &[2], &[3, 5, 5, 15, 17], &[4]]),
    )
}

/// Port of `TestMatchesIterator.testExclusionNoPositions`.
fn test_exclusion_no_positions(inst: &mut Inst) -> Result<()> {
    for field in [FIELD_FREQS, FIELD_DOCS_ONLY] {
        let q = bq2(
            tq(field, "w3"),
            Occur::SHOULD,
            tq(field, "zz"),
            Occur::MUST_NOT,
        )?;
        state(inst).check_no_positions_matches(q, field, &[true, false, false, true, false])?;
    }
    Ok(())
}

/// Port of `TestMatchesIterator.testConjunction`.
fn test_conjunction(inst: &mut Inst) -> Result<()> {
    let q = bq2(
        tq(FIELD_WITH_OFFSETS, "w3"),
        Occur::MUST,
        tq(FIELD_WITH_OFFSETS, "w4"),
        Occur::MUST,
    )?;
    state(inst).check_matches(
        q,
        FIELD_WITH_OFFSETS,
        &rows(&[
            &[0, 2, 2, 6, 8, 3, 3, 9, 11],
            &[1],
            &[2],
            &[3, 3, 3, 9, 11, 5, 5, 15, 17],
            &[4],
        ]),
    )
}

/// Port of `TestMatchesIterator.testConjunctionNoPositions`.
fn test_conjunction_no_positions(inst: &mut Inst) -> Result<()> {
    for field in [FIELD_FREQS, FIELD_DOCS_ONLY] {
        let q = bq2(tq(field, "w3"), Occur::MUST, tq(field, "w4"), Occur::MUST)?;
        state(inst).check_no_positions_matches(q, field, &[true, false, false, true, false])?;
    }
    Ok(())
}

/// Port of `TestMatchesIterator.testWildcards`.
fn test_wildcards(inst: &mut Inst) -> Result<()> {
    let s = state(inst);
    let q: Arc<dyn Query> = Arc::new(PrefixQuery::new(term(FIELD_WITH_OFFSETS, "x"))?);
    s.check_matches(
        q,
        FIELD_WITH_OFFSETS,
        &rows(&[&[0], &[1], &[2, 1, 1, 3, 5], &[3], &[4]]),
    )?;

    let rq: Arc<dyn Query> = Arc::new(RegexpQuery::new(term(FIELD_WITH_OFFSETS, "w[1-2]"))?);
    s.check_matches(
        Arc::clone(&rq),
        FIELD_WITH_OFFSETS,
        &rows(&[
            &[0, 0, 0, 0, 2, 1, 1, 3, 5],
            &[1, 0, 0, 0, 2, 2, 2, 6, 8],
            &[2, 0, 0, 0, 2, 2, 2, 6, 8],
            &[3, 0, 0, 0, 2, 1, 1, 3, 5, 2, 2, 6, 8, 4, 4, 12, 14],
            &[4],
        ]),
    )?;
    s.check_label_count(Arc::clone(&rq), FIELD_WITH_OFFSETS, &[1, 1, 1, 1, 0])?;
    s.assert_is_leaf_match(rq, FIELD_WITH_OFFSETS)
}

/// Port of `TestMatchesIterator.testNoMatchWildcards`.
fn test_no_match_wildcards(inst: &mut Inst) -> Result<()> {
    let s = state(inst);
    let nomatch: Arc<dyn Query> = Arc::new(PrefixQuery::new(term(FIELD_WITH_OFFSETS, "wibble"))?);
    let matches = s
        .searcher
        .create_weight(
            s.searcher.rewrite(nomatch)?,
            ScoreMode::COMPLETE_NO_SCORES,
            1.0,
        )?
        .matches(&s.searcher.get_leaf_contexts()[0], 0)?;
    assert_null(&matches.map(|_| "Matches"));
    Ok(())
}

/// Port of `TestMatchesIterator.testWildcardsNoPositions`.
fn test_wildcards_no_positions(inst: &mut Inst) -> Result<()> {
    for field in [FIELD_FREQS, FIELD_DOCS_ONLY] {
        let q: Arc<dyn Query> = Arc::new(PrefixQuery::new(term(field, "x"))?);
        state(inst).check_no_positions_matches(q, field, &[false, false, true, false, false])?;
    }
    Ok(())
}

/// `new SynonymQuery.Builder(field).addTerm(w1).addTerm(w2).build()`.
fn synonym_w1_w2(field: &str) -> Result<Arc<dyn Query>> {
    Ok(Arc::new(
        SynonymQueryBuilder::new(Some(field))
            .add_term(&term(field, "w1"))?
            .add_term(&term(field, "w2"))?
            .build(),
    ))
}

/// Port of `TestMatchesIterator.testSynonymQuery`.
fn test_synonym_query(inst: &mut Inst) -> Result<()> {
    let s = state(inst);
    let q = synonym_w1_w2(FIELD_WITH_OFFSETS)?;
    s.check_matches(
        Arc::clone(&q),
        FIELD_WITH_OFFSETS,
        &rows(&[
            &[0, 0, 0, 0, 2, 1, 1, 3, 5],
            &[1, 0, 0, 0, 2, 2, 2, 6, 8],
            &[2, 0, 0, 0, 2, 2, 2, 6, 8],
            &[3, 0, 0, 0, 2, 1, 1, 3, 5, 2, 2, 6, 8, 4, 4, 12, 14],
            &[4],
        ]),
    )?;
    s.assert_is_leaf_match(q, FIELD_WITH_OFFSETS)
}

/// Port of `TestMatchesIterator.testSynonymQueryNoPositions`.
fn test_synonym_query_no_positions(inst: &mut Inst) -> Result<()> {
    for field in [FIELD_FREQS, FIELD_DOCS_ONLY] {
        let q = synonym_w1_w2(field)?;
        state(inst).check_no_positions_matches(q, field, &[true, true, true, true, false])?;
    }
    Ok(())
}

/// Port of `TestMatchesIterator.testMultipleFields`.
fn test_multiple_fields(inst: &mut Inst) -> Result<()> {
    let s = state(inst);
    let q = bq2(
        tq("id", "1"),
        Occur::SHOULD,
        tq(FIELD_WITH_OFFSETS, "w3"),
        Occur::MUST,
    )?;
    let w = s
        .searcher
        .create_weight(s.searcher.rewrite(q)?, ScoreMode::COMPLETE, 1.0)?;

    let leaf_contexts = s.searcher.get_leaf_contexts();
    let ctx = &leaf_contexts[sub_index_from_leaves(1, leaf_contexts) as usize];
    let m = w.matches(ctx, 1 - ctx.doc_base())?;
    let m = m.unwrap_or_else(|| Assert::fail_without_message());
    let mut id = m.get_matches("id")?.expect("matches of id");
    s.check_field_matches(&mut *id, &[-1, 0, 0, -1, -1])?;
    let mut offsets = m
        .get_matches(FIELD_WITH_OFFSETS)?
        .expect("matches of the offsets field");
    s.check_field_matches(&mut *offsets, &[-1, 1, 1, 3, 5, 3, 3, 9, 11])?;
    assert_null(&m.get_matches("bogus")?.map(|_| "MatchesIterator"));

    let mut fields = HashSet::new();
    for field in m.fields() {
        fields.insert(field);
    }
    assert_equals(&2, &fields.len());
    Assert::assert_true(fields.contains(FIELD_WITH_OFFSETS));
    Assert::assert_true(fields.contains("id"));

    assert_equals(&2, &AssertingMatches::un_wrap(m).get_sub_matches().len());
    Ok(())
}

//  0         1         2         3         4         5         6         7
// "a phrase sentence with many phrase sentence iterations of a phrase sentence",

/// Port of `TestMatchesIterator.testSloppyPhraseQueryWithRepeats`.
fn test_sloppy_phrase_query_with_repeats(inst: &mut Inst) -> Result<()> {
    let s = state(inst);
    let pq: Arc<dyn Query> = Arc::new(PhraseQuery::with_slop(
        10,
        FIELD_WITH_OFFSETS,
        &["phrase", "sentence", "sentence"],
    )?);
    s.check_matches(
        Arc::clone(&pq),
        FIELD_WITH_OFFSETS,
        &rows(&[
            &[0],
            &[1],
            &[2],
            &[3],
            &[4, 1, 6, 2, 43, 2, 11, 9, 75, 5, 11, 28, 75, 6, 11, 35, 75],
        ]),
    )?;
    s.check_label_count(Arc::clone(&pq), FIELD_WITH_OFFSETS, &[0, 0, 0, 0, 1])?;
    s.assert_is_leaf_match(pq, FIELD_WITH_OFFSETS)
}

/// Port of `TestMatchesIterator.testSloppyPhraseQuery`.
fn test_sloppy_phrase_query(inst: &mut Inst) -> Result<()> {
    let s = state(inst);
    let pq: Arc<dyn Query> = Arc::new(PhraseQuery::with_slop(
        4,
        FIELD_WITH_OFFSETS,
        &["a", "sentence"],
    )?);
    s.check_matches(
        Arc::clone(&pq),
        FIELD_WITH_OFFSETS,
        &rows(&[
            &[0],
            &[1],
            &[2],
            &[3],
            &[4, 0, 2, 0, 17, 6, 9, 35, 59, 9, 11, 58, 75],
        ]),
    )?;
    s.assert_is_leaf_match(pq, FIELD_WITH_OFFSETS)
}

/// Port of `TestMatchesIterator.testExactPhraseQuery`.
fn test_exact_phrase_query(inst: &mut Inst) -> Result<()> {
    let s = state(inst);
    let pq: Arc<dyn Query> = Arc::new(PhraseQuery::from_strings(
        FIELD_WITH_OFFSETS,
        &["phrase", "sentence"],
    )?);
    s.check_matches(
        pq,
        FIELD_WITH_OFFSETS,
        &rows(&[
            &[0],
            &[1],
            &[2],
            &[3],
            &[4, 1, 2, 2, 17, 5, 6, 28, 43, 10, 11, 60, 75],
        ]),
    )?;

    let a = term(FIELD_WITH_OFFSETS, "a");
    let st = term(FIELD_WITH_OFFSETS, "sentence");
    let pq2: Arc<dyn Query> = Arc::new(PhraseQueryBuilder::new().add(a)?.add_at(st, 2)?.build()?);
    s.check_matches(
        Arc::clone(&pq2),
        FIELD_WITH_OFFSETS,
        &rows(&[&[0], &[1], &[2], &[3], &[4, 0, 2, 0, 17, 9, 11, 58, 75]]),
    )?;
    s.assert_is_leaf_match(pq2, FIELD_WITH_OFFSETS)
}

//  0         1         2         3         4         5         6         7
// "a phrase sentence with many phrase sentence iterations of a phrase sentence",

/// Port of `TestMatchesIterator.testSloppyMultiPhraseQuery`.
fn test_sloppy_multi_phrase_query(inst: &mut Inst) -> Result<()> {
    let s = state(inst);
    let p = term(FIELD_WITH_OFFSETS, "phrase");
    let st = term(FIELD_WITH_OFFSETS, "sentence");
    let i = term(FIELD_WITH_OFFSETS, "iterations");
    let mpq: Arc<dyn Query> = Arc::new(
        MultiPhraseQueryBuilder::new()
            .add(p)?
            .add_terms(vec![st, i])?
            .set_slop(4)?
            .build(),
    );
    s.check_matches(
        Arc::clone(&mpq),
        FIELD_WITH_OFFSETS,
        &rows(&[
            &[0],
            &[1],
            &[2],
            &[3],
            &[4, 1, 2, 2, 17, 5, 6, 28, 43, 5, 7, 28, 54, 10, 11, 60, 75],
        ]),
    )?;
    s.assert_is_leaf_match(mpq, FIELD_WITH_OFFSETS)
}

/// Port of `TestMatchesIterator.testExactMultiPhraseQuery`.
fn test_exact_multi_phrase_query(inst: &mut Inst) -> Result<()> {
    let s = state(inst);
    let mpq: Arc<dyn Query> = Arc::new(
        MultiPhraseQueryBuilder::new()
            .add(term(FIELD_WITH_OFFSETS, "sentence"))?
            .add_terms(vec![
                term(FIELD_WITH_OFFSETS, "with"),
                term(FIELD_WITH_OFFSETS, "iterations"),
            ])?
            .build(),
    );
    s.check_matches(
        mpq,
        FIELD_WITH_OFFSETS,
        &rows(&[&[0], &[1], &[2], &[3], &[4, 2, 3, 9, 22, 6, 7, 35, 54]]),
    )?;

    let mpq2: Arc<dyn Query> = Arc::new(
        MultiPhraseQueryBuilder::new()
            .add_terms(vec![
                term(FIELD_WITH_OFFSETS, "a"),
                term(FIELD_WITH_OFFSETS, "many"),
            ])?
            .add(term(FIELD_WITH_OFFSETS, "phrase"))?
            .build(),
    );
    s.check_matches(
        Arc::clone(&mpq2),
        FIELD_WITH_OFFSETS,
        &rows(&[
            &[0],
            &[1],
            &[2],
            &[3],
            &[4, 0, 1, 0, 8, 4, 5, 23, 34, 9, 10, 58, 66],
        ]),
    )?;
    s.assert_is_leaf_match(mpq2, FIELD_WITH_OFFSETS)
}

/// `new IndexOrDocValuesQuery(IntPoint.newExactQuery(FIELD_POINT, value),
/// NumericDocValuesField.newSlowExactQuery(FIELD_POINT, value))`.
fn index_or_dv_point_query(value: i32) -> Result<Arc<dyn Query>> {
    Ok(Arc::new(IndexOrDocValuesQuery::new(
        IntPoint::new_exact_query(FIELD_POINT, value)?,
        NumericDocValuesField::new_slow_exact_query(FIELD_POINT, value as i64),
    )))
}

/// Port of `TestMatchesIterator.testPointQuery`.
fn test_point_query(inst: &mut Inst) -> Result<()> {
    let s = state(inst);
    let mut point_query = index_or_dv_point_query(10)?;
    let w1_rows = rows(&[
        &[0, 0, 0, 0, 2],
        &[1, 0, 0, 0, 2],
        &[2, 0, 0, 0, 2],
        &[3, 0, 0, 0, 2, 2, 2, 6, 8],
        &[4],
    ]);
    let mut query = bq2(
        tq(FIELD_WITH_OFFSETS, "w1"),
        Occur::MUST,
        Arc::clone(&point_query),
        Occur::MUST,
    )?;

    s.check_matches(Arc::clone(&point_query), FIELD_WITH_OFFSETS, &[])?;

    s.check_matches(query, FIELD_WITH_OFFSETS, &w1_rows)?;

    point_query = index_or_dv_point_query(11)?;

    query = bq2(
        tq(FIELD_WITH_OFFSETS, "w1"),
        Occur::MUST,
        Arc::clone(&point_query),
        Occur::MUST,
    )?;
    s.check_matches(query, FIELD_WITH_OFFSETS, &[])?;

    query = bq2(
        tq(FIELD_WITH_OFFSETS, "w1"),
        Occur::MUST,
        point_query,
        Occur::SHOULD,
    )?;
    s.check_matches(query, FIELD_WITH_OFFSETS, &w1_rows)
}

/// Port of `TestMatchesIterator.testMinimalSeekingWithWildcards`.
fn test_minimal_seeking_with_wildcards(inst: &mut Inst) -> Result<()> {
    let reader = {
        let s = state(inst);
        Arc::new(SeekCountingLeafReader::new(
            LuceneTestCase::get_only_leaf_reader(Arc::clone(&s.reader))?,
        )?)
    };
    inst.state.as_mut().expect("setUp() ran").searcher =
        IndexSearcher::new(Arc::clone(&reader) as Arc<dyn IndexReader>)?;
    let searcher = &state(inst).searcher;
    let query = PrefixQuery::new(term(FIELD_WITH_OFFSETS, "w"))?;
    let rewritten = query
        .rewrite(searcher)?
        .unwrap_or_else(|| Arc::new(query.clone()));
    let w = searcher.create_weight(rewritten, ScoreMode::COMPLETE, 1.0)?;

    // docs 0-3 match several different terms here, but we only seek to the
    // first term and then short-cut return; other terms are ignored until we
    // try and iterate over matches
    let expected_seeks = [1, 1, 1, 1, 6, 6];
    let mut i = 0;
    for ctx in (Arc::clone(&reader) as Arc<dyn IndexReader>).leaves()? {
        for doc in 0..ctx.leaf_reader().max_doc() {
            reader.seeks.store(0, Ordering::SeqCst);
            w.matches(&ctx, doc)?;
            assert_equals_msg(
                Some(&format!("Unexpected seek count on doc {doc}")),
                &expected_seeks[i],
                &reader.seeks.load(Ordering::SeqCst),
            );
            i += 1;
        }
    }
    Ok(())
}

/// The local `class CountIterator implements MatchesIterator` of
/// `testFromSubIteratorsMethod`.
struct CountIterator {
    count: i32,
    max: i32,
}

impl CountIterator {
    fn new(count: i32) -> Self {
        CountIterator { count, max: count }
    }
}

impl MatchesIterator for CountIterator {
    fn next(&mut self) -> Result<bool> {
        if self.count == 0 {
            Ok(false)
        } else {
            self.count -= 1;
            Ok(true)
        }
    }

    fn start_position(&self) -> i32 {
        self.max - self.count
    }

    fn end_position(&self) -> i32 {
        self.max - self.count
    }

    fn start_offset(&self) -> Result<i32> {
        Throwable::new("java.lang.AssertionError", "").throw()
    }

    fn end_offset(&self) -> Result<i32> {
        Throwable::new("java.lang.AssertionError", "").throw()
    }

    fn get_sub_matches(&self) -> Result<Option<Box<dyn MatchesIterator>>> {
        Throwable::new("java.lang.AssertionError", "").throw()
    }

    fn get_query(&self) -> Arc<dyn Query> {
        Throwable::new("java.lang.AssertionError", "").throw()
    }
}

/// Port of `TestMatchesIterator.testFromSubIteratorsMethod`.
fn test_from_sub_iterators_method(_inst: &mut Inst) -> Result<()> {
    let checks: [&[i32]; 13] = [
        &[0],
        &[1],
        &[0, 0],
        &[0, 1],
        &[1, 0],
        &[1, 1],
        &[0, 0, 0],
        &[0, 0, 1],
        &[0, 1, 0],
        &[1, 0, 0],
        &[1, 0, 1],
        &[1, 1, 0],
        &[1, 1, 1],
    ];

    for counts in checks {
        let its: Vec<Box<dyn MatchesIterator>> = counts
            .iter()
            .map(|&c| Box::new(CountIterator::new(c)) as Box<dyn MatchesIterator>)
            .collect();

        let expected_count: i32 = counts.iter().sum();

        let mut merged = disjunction_matches_iterator::from_sub_iterators(its)?
            .expect("a non-empty list of iterators");
        let mut actual_count = 0;
        while merged.next()? {
            actual_count += 1;
        }

        assert_equals_msg(
            Some(&format!(
                "Sub-iterator count is not right for: {}",
                arrays_to_string(counts)
            )),
            &expected_count,
            &actual_count,
        );
    }
    Ok(())
}

// -----------------------------------------------------------------------------
// SeekCountingLeafReader
// -----------------------------------------------------------------------------

/// `private static class SeekCountingLeafReader extends FilterLeafReader`.
struct SeekCountingLeafReader {
    inner: FilterLeafReader,
    seeks: Arc<AtomicI32>,
}

impl SeekCountingLeafReader {
    fn new(r#in: Arc<dyn LeafReader>) -> Result<Self> {
        Ok(SeekCountingLeafReader {
            inner: FilterLeafReader::new(r#in)?,
            seeks: Arc::new(AtomicI32::new(0)),
        })
    }
}

/// The anonymous `FilterTerms` of `SeekCountingLeafReader.terms`.
struct SeekCountingTerms {
    inner: FilterTerms,
    seeks: Arc<AtomicI32>,
}

impl Terms for SeekCountingTerms {
    fn iterator(&self) -> Result<Box<dyn TermsEnum>> {
        Ok(Box::new(SeekCountingTermsEnum {
            inner: FilterTermsEnum::new(self.inner.iterator()?),
            seeks: Arc::clone(&self.seeks),
        }))
    }
    fn size(&self) -> Result<i64> {
        self.inner.size()
    }
    fn sum_total_term_freq(&self) -> Result<i64> {
        self.inner.sum_total_term_freq()
    }
    fn sum_doc_freq(&self) -> Result<i64> {
        self.inner.sum_doc_freq()
    }
    fn doc_count(&self) -> Result<i32> {
        self.inner.doc_count()
    }
    fn has_freqs(&self) -> bool {
        self.inner.has_freqs()
    }
    fn has_offsets(&self) -> bool {
        self.inner.has_offsets()
    }
    fn has_positions(&self) -> bool {
        self.inner.has_positions()
    }
    fn has_payloads(&self) -> bool {
        self.inner.has_payloads()
    }
    fn stats(&self) -> Result<String> {
        self.inner.stats()
    }
}

/// The anonymous `FilterTermsEnum` of `SeekCountingLeafReader.terms`, which
/// counts `seekExact` calls.
struct SeekCountingTermsEnum {
    inner: FilterTermsEnum,
    seeks: Arc<AtomicI32>,
}

impl fmt::Display for SeekCountingTermsEnum {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.inner, f)
    }
}

impl TermsEnum for SeekCountingTermsEnum {
    fn attributes(&mut self) -> &mut AttributeSource {
        self.inner.attributes()
    }
    fn seek_ceil(&mut self, text: &BytesRef) -> Result<SeekStatus> {
        self.inner.seek_ceil(text)
    }
    fn seek_exact(&mut self, text: &BytesRef) -> Result<bool> {
        self.seeks.fetch_add(1, Ordering::SeqCst);
        self.inner.seek_exact(text)
    }
    fn seek_ord(&mut self, ord: i64) -> Result<()> {
        self.inner.seek_ord(ord)
    }
    fn next(&mut self) -> Result<Option<BytesRef>> {
        self.inner.next()
    }
    fn term(&self) -> Result<BytesRef> {
        self.inner.term()
    }
    fn ord(&self) -> Result<i64> {
        self.inner.ord()
    }
    fn doc_freq(&mut self) -> Result<i32> {
        self.inner.doc_freq()
    }
    fn total_term_freq(&mut self) -> Result<i64> {
        self.inner.total_term_freq()
    }
    fn postings(
        &mut self,
        reuse: Option<Box<dyn PostingsEnum>>,
        flags: i32,
    ) -> Result<Box<dyn PostingsEnum>> {
        self.inner.postings(reuse, flags)
    }
    fn impacts(&mut self, flags: i32) -> Result<Box<dyn ImpactsEnum>> {
        self.inner.impacts(flags)
    }
    fn seek_term_state(&mut self, text: &BytesRef, state: &dyn TermState) -> Result<()> {
        self.inner.seek_term_state(text, state)
    }
    /// `FilterTermsEnum.prepareSeekExact` delegates to `in`
    /// (`FilterLeafReader.java:243`), so it is not counted.
    fn prepare_seek_exact<'a>(
        &'a mut self,
        text: &BytesRef,
    ) -> Result<Option<Box<dyn IOBooleanSupplier + 'a>>> {
        self.inner.prepare_seek_exact(text)
    }
    fn term_state(&mut self) -> Result<Box<dyn TermState>> {
        self.inner.term_state()
    }
}

impl fmt::Debug for SeekCountingLeafReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SeekCountingLeafReader")
            .finish_non_exhaustive()
    }
}

impl fmt::Display for SeekCountingLeafReader {
    /// The inherited `FilterLeafReader.toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.inner, f)
    }
}

impl LeafReader for SeekCountingLeafReader {
    fn core(&self) -> &rucene::index::IndexReaderCore {
        LeafReader::core(&self.inner)
    }
    fn term_vectors(&self) -> Result<Box<dyn rucene::index::TermVectors>> {
        LeafReader::term_vectors(&self.inner)
    }
    fn num_docs(&self) -> i32 {
        LeafReader::num_docs(&self.inner)
    }
    fn max_doc(&self) -> i32 {
        LeafReader::max_doc(&self.inner)
    }
    fn stored_fields(&self) -> Result<Box<dyn rucene::index::StoredFields>> {
        LeafReader::stored_fields(&self.inner)
    }
    fn do_close(&self) -> Result<()> {
        LeafReader::do_close(&self.inner)
    }
    fn get_reader_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        None
    }
    fn get_core_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        None
    }
    fn terms(&self, field: &str) -> Result<Option<Box<dyn Terms>>> {
        let Some(terms) = self.inner.terms(field)? else {
            return Ok(None);
        };
        Ok(Some(Box::new(SeekCountingTerms {
            inner: FilterTerms::new(terms),
            seeks: Arc::clone(&self.seeks),
        })))
    }
    fn get_numeric_doc_values(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn rucene::index::NumericDocValues>>> {
        self.inner.get_numeric_doc_values(field)
    }
    fn get_binary_doc_values(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn rucene::index::BinaryDocValues>>> {
        self.inner.get_binary_doc_values(field)
    }
    fn get_sorted_doc_values(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn rucene::index::SortedDocValues>>> {
        self.inner.get_sorted_doc_values(field)
    }
    fn get_sorted_numeric_doc_values(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn rucene::index::SortedNumericDocValues>>> {
        self.inner.get_sorted_numeric_doc_values(field)
    }
    fn get_sorted_set_doc_values(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn rucene::index::SortedSetDocValues>>> {
        self.inner.get_sorted_set_doc_values(field)
    }
    fn get_norm_values(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn rucene::index::NumericDocValues>>> {
        self.inner.get_norm_values(field)
    }
    fn get_doc_values_skipper(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn rucene::index::DocValuesSkipper>>> {
        self.inner.get_doc_values_skipper(field)
    }
    fn get_float_vector_values(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn rucene::index::FloatVectorValues>>> {
        self.inner.get_float_vector_values(field)
    }
    fn get_byte_vector_values(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn rucene::index::ByteVectorValues>>> {
        self.inner.get_byte_vector_values(field)
    }
    fn search_nearest_vectors(
        &self,
        field: &str,
        target: &[f32],
        collector: &mut dyn rucene::search::knn::KnnCollector,
        accept_docs: &mut dyn rucene::search::AcceptDocs,
    ) -> Result<()> {
        self.inner
            .search_nearest_vectors(field, target, collector, accept_docs)
    }
    fn search_nearest_vectors_byte(
        &self,
        field: &str,
        target: &[u8],
        collector: &mut dyn rucene::search::knn::KnnCollector,
        accept_docs: &mut dyn rucene::search::AcceptDocs,
    ) -> Result<()> {
        self.inner
            .search_nearest_vectors_byte(field, target, collector, accept_docs)
    }
    fn get_field_infos(&self) -> rucene::index::FieldInfos {
        self.inner.get_field_infos()
    }
    fn get_live_docs(&self) -> Option<Box<dyn Bits>> {
        LeafReader::get_live_docs(&self.inner)
    }
    fn get_point_values(&self, field: &str) -> Result<Option<Box<dyn rucene::index::PointValues>>> {
        self.inner.get_point_values(field)
    }
    fn check_integrity(&self) -> Result<()> {
        self.inner.check_integrity()
    }
    fn get_meta_data(&self) -> rucene::index::LeafMetaData {
        self.inner.get_meta_data()
    }
}
