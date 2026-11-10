//! Port of `lucene/core/src/test/org/apache/lucene/search/TestMinShouldMatch2.java`.
//!
//! tests BooleanScorer2's minShouldMatch

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::{AfterClass, BeforeClass};
use rucene::document::{Document, SortedSetDocValuesField, Store, StringField};
use rucene::error::Result;
use rucene::index::directory_reader::{self, DirectoryReader};
use rucene::index::doc_values::{NumericDocValues, SortedSetDocValues};
use rucene::index::{LeafReader, Term, TermStates};
use rucene::search::boolean_weight::BooleanWeight;
use rucene::search::doc_id_set_iterator::{DocIdSetIterator, NO_MORE_DOCS};
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::scorable::Scorable;
use rucene::search::scorer::Scorer;
use rucene::search::similarities::{ClassicSimilarity, SimScorer};
use rucene::search::term_query::TermQuery;
use rucene::search::{BooleanQuery, BooleanQueryBuilder, Occur, ScoreMode, Weight};
use rucene::util::BytesRef;
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::search::BulkScorerWrapperScorer;
use rucene_test_framework::util::lucene_test_case::{Nightly, TestDirectory};
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_search::support::{static_hook, term};
use crate::core_support::{assert_equals, assert_equals_delta, class, random, shuffle, test};

/// The static fields `dir`, `r`, `reader` and `searcher`.
struct Statics {
    dir: TestDirectory,
    r: Arc<dyn DirectoryReader>,
    reader: Arc<dyn LeafReader>,
    searcher: Arc<IndexSearcher>,
}

static STATICS: Mutex<Option<Statics>> = Mutex::new(None);

/// `reader` and `searcher`.
fn statics() -> (Arc<dyn LeafReader>, Arc<IndexSearcher>) {
    let guard = STATICS.lock().unwrap_or_else(|e| e.into_inner());
    let s = guard.as_ref().expect("beforeClass ran");
    (Arc::clone(&s.reader), Arc::clone(&s.searcher))
}

const ALWAYS_TERMS: [&str; 1] = ["a"];
const COMMON_TERMS: [&str; 3] = ["b", "c", "d"];
const MEDIUM_TERMS: [&str; 3] = ["e", "f", "g"];
const RARE_TERMS: [&str; 19] = [
    "h", "i", "j", "k", "l", "m", "n", "o", "p", "q", "r", "s", "t", "u", "v", "w", "x", "y", "z",
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Scorer,
    BulkScorer,
    DocValues,
}

/// `TestMinShouldMatch2`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.TestMinShouldMatch2",
        vec![
            static_hook("beforeClass", BeforeClass, before_class),
            static_hook("afterClass", AfterClass, after_class),
            test("testNextCMR2", test_next_cmr2),
            test("testAdvanceCMR2", test_advance_cmr2),
            test("testNextAllTerms", test_next_all_terms),
            test("testAdvanceAllTerms", test_advance_all_terms),
            test(
                "testNextVaryingNumberOfTerms",
                test_next_varying_number_of_terms,
            ),
            test(
                "testAdvanceVaryingNumberOfTerms",
                test_advance_varying_number_of_terms,
            )
            .annotate(Nightly),
        ],
    )
}

fn before_class() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let iw = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    let num_docs = LuceneTestCase::at_least(300);
    for _ in 0..num_docs {
        let mut doc = Document::new();

        add_some(&mut doc, &ALWAYS_TERMS)?;

        if random().next_int_bound(100) < 90 {
            add_some(&mut doc, &COMMON_TERMS)?;
        }
        if random().next_int_bound(100) < 50 {
            add_some(&mut doc, &MEDIUM_TERMS)?;
        }
        if random().next_int_bound(100) < 10 {
            add_some(&mut doc, &RARE_TERMS)?;
        }
        iw.add_document(&doc)?;
    }
    iw.force_merge(1)?;
    iw.close()?;
    let r = directory_reader::open(dir.as_directory())?;
    let reader = LuceneTestCase::get_only_leaf_reader(
        Arc::clone(&r) as Arc<dyn rucene::index::IndexReader>
    )?;
    let mut searcher = IndexSearcher::new(Arc::clone(&reader).as_index_reader())?;
    searcher.set_similarity(Arc::new(ClassicSimilarity::new()));
    *STATICS.lock().unwrap_or_else(|e| e.into_inner()) = Some(Statics {
        dir,
        r,
        reader,
        searcher: Arc::new(searcher),
    });
    Ok(())
}

fn after_class() -> Result<()> {
    let statics = STATICS.lock().unwrap_or_else(|e| e.into_inner()).take();
    if let Some(Statics {
        mut dir,
        r,
        reader,
        searcher,
    }) = statics
    {
        // reader.close(): the leaf of `r`, whose closing releases `r`'s
        // reference to it.
        drop(searcher);
        reader.as_index_reader().close()?;
        drop(r);
        dir.close()?;
    }
    Ok(())
}

fn add_some(doc: &mut Document, values: &[&str]) -> Result<()> {
    let mut list: Vec<&str> = values.to_vec();
    shuffle(&mut list, &*random());
    let how_many = TestUtil::next_int(&*random(), 1, list.len() as i32);
    for i in 0..how_many as usize {
        doc.add(Box::new(StringField::new(
            "field",
            list[i].to_string(),
            Store::NO,
        )?));
        doc.add(Box::new(SortedSetDocValuesField::new(
            "dv",
            BytesRef::from_char_sequence(list[i]),
        )));
    }
    Ok(())
}

fn scorer(values: &[&str], min_should_match: i32, mode: Mode) -> Result<Option<Box<dyn Scorer>>> {
    let (reader, searcher) = statics();
    let mut bq = BooleanQueryBuilder::new();
    for value in values {
        bq.add(
            Arc::new(TermQuery::new(term("field", value))),
            Occur::SHOULD,
        )?;
    }
    bq.set_minimum_number_should_match(min_should_match);

    let weight = searcher.create_weight(
        searcher.rewrite(Arc::new(bq.build()))?,
        ScoreMode::COMPLETE,
        1.0,
    )?;
    // (BooleanWeight) weight
    let boolean_weight = weight
        .as_any()
        .and_then(|a| a.downcast_ref::<BooleanWeight>())
        .unwrap_or_else(|| {
            randomizedtesting::jdk::exception::Throwable::new(
                "java.lang.ClassCastException",
                "not a BooleanWeight",
            )
            .throw()
        });
    let context = &searcher.get_leaf_contexts()[0];

    match mode {
        Mode::DocValues => Ok(Some(Box::new(SlowMinShouldMatchScorer::new(
            boolean_weight,
            &*reader,
            &searcher,
        )?))),
        Mode::Scorer => weight.scorer(context),
        Mode::BulkScorer => {
            let mut ss = weight
                .scorer_supplier(context)?
                .expect("the weight has a scorer supplier");
            // `if (bulkScorer == null)`: the port's `bulkScorer()` never
            // answers null, so Java's `AssertionError` branch is unreachable.
            let bulk_scorer = ss.bulk_scorer()?;
            Ok(Some(Box::new(BulkScorerWrapperScorer::new(
                bulk_scorer,
                TestUtil::next_int(&*random(), 1, 100) as usize,
            ))))
        }
    }
}

fn assert_next(expected: &mut dyn Scorer, actual: Option<Box<dyn Scorer>>) -> Result<()> {
    let Some(mut actual) = actual else {
        assert_equals(&NO_MORE_DOCS, &expected.iterator().next_doc()?);
        return Ok(());
    };
    loop {
        let doc = expected.iterator().next_doc()?;
        if doc == NO_MORE_DOCS {
            break;
        }
        assert_equals(&doc, &actual.iterator().next_doc()?);
        let expected_score = expected.score()?;
        let actual_score = actual.score()?;
        assert_equals_delta(expected_score as f64, actual_score as f64, 0.0);
    }
    assert_equals(&NO_MORE_DOCS, &actual.iterator().next_doc()?);
    Ok(())
}

fn assert_advance(
    expected: &mut dyn Scorer,
    actual: Option<Box<dyn Scorer>>,
    amount: i32,
) -> Result<()> {
    let Some(mut actual) = actual else {
        assert_equals(&NO_MORE_DOCS, &expected.iterator().next_doc()?);
        return Ok(());
    };
    let mut prev_doc = 0;
    loop {
        let doc = expected.iterator().advance(prev_doc + amount)?;
        if doc == NO_MORE_DOCS {
            break;
        }
        assert_equals(&doc, &actual.iterator().advance(prev_doc + amount)?);
        let expected_score = expected.score()?;
        let actual_score = actual.score()?;
        assert_equals_delta(expected_score as f64, actual_score as f64, 0.0);
        prev_doc = doc;
    }
    assert_equals(
        &NO_MORE_DOCS,
        &actual.iterator().advance(prev_doc + amount)?,
    );
    Ok(())
}

/// `scorer(values, msm, DOC_VALUES)`, which is never null.
fn expected_scorer(values: &[&str], min_should_match: i32) -> Result<Box<dyn Scorer>> {
    Ok(scorer(values, min_should_match, Mode::DocValues)?.expect("the slow scorer is built"))
}

/// The two `assertNext` (or `assertAdvance`, with `Some(amount)`) of each
/// iteration: against `SCORER`, then against `second`.
fn compare(values: &[&str], msm: i32, amount: Option<i32>, second: Mode) -> Result<()> {
    for mode in [Mode::Scorer, second] {
        let mut expected = expected_scorer(values, msm)?;
        let actual = scorer(values, msm, mode)?;
        match amount {
            None => assert_next(&mut *expected, actual)?,
            Some(amount) => assert_advance(&mut *expected, actual, amount)?,
        }
    }
    Ok(())
}

/// simple test for next(): minShouldMatch=2 on 3 terms (one common, one
/// medium, one rare)
///
/// Port of `TestMinShouldMatch2.testNextCMR2`.
fn test_next_cmr2() -> Result<()> {
    for common in COMMON_TERMS {
        for medium in MEDIUM_TERMS {
            for rare in RARE_TERMS {
                compare(&[common, medium, rare], 2, None, Mode::BulkScorer)?;
            }
        }
    }
    Ok(())
}

/// simple test for advance(): minShouldMatch=2 on 3 terms (one common, one
/// medium, one rare)
///
/// Port of `TestMinShouldMatch2.testAdvanceCMR2`.
fn test_advance_cmr2() -> Result<()> {
    let mut amount = 25;
    while amount < 200 {
        for common in COMMON_TERMS {
            for medium in MEDIUM_TERMS {
                for rare in RARE_TERMS {
                    compare(&[common, medium, rare], 2, Some(amount), Mode::BulkScorer)?;
                }
            }
        }
        amount += 25;
    }
    Ok(())
}

/// `commonTerms + mediumTerms + rareTerms`.
fn all_terms() -> Vec<&'static str> {
    let mut terms_list = Vec::new();
    terms_list.extend_from_slice(&COMMON_TERMS);
    terms_list.extend_from_slice(&MEDIUM_TERMS);
    terms_list.extend_from_slice(&RARE_TERMS);
    terms_list
}

/// test next with giant bq of all terms with varying minShouldMatch
///
/// Port of `TestMinShouldMatch2.testNextAllTerms`.
fn test_next_all_terms() -> Result<()> {
    let terms = all_terms();
    for min_nr_should_match in 1..terms.len() as i32 {
        compare(&terms, min_nr_should_match, None, Mode::BulkScorer)?;
    }
    Ok(())
}

/// test advance with giant bq of all terms with varying minShouldMatch
///
/// Port of `TestMinShouldMatch2.testAdvanceAllTerms`.
fn test_advance_all_terms() -> Result<()> {
    let terms = all_terms();
    let mut amount = 25;
    while amount < 200 {
        for min_nr_should_match in 1..terms.len() as i32 {
            compare(&terms, min_nr_should_match, Some(amount), Mode::BulkScorer)?;
        }
        amount += 25;
    }
    Ok(())
}

/// test next with varying numbers of terms with varying minShouldMatch
///
/// Port of `TestMinShouldMatch2.testNextVaryingNumberOfTerms`.
fn test_next_varying_number_of_terms() -> Result<()> {
    let mut terms_list = all_terms();
    shuffle(&mut terms_list, &*random());
    for num_terms in 2..=terms_list.len() {
        let terms = &terms_list[..num_terms];
        for min_nr_should_match in 1..terms.len() as i32 {
            compare(terms, min_nr_should_match, None, Mode::BulkScorer)?;
        }
    }
    Ok(())
}

/// test advance with varying numbers of terms with varying minShouldMatch
///
/// Port of `TestMinShouldMatch2.testAdvanceVaryingNumberOfTerms`; Java
/// compares against `SCORER` twice.
fn test_advance_varying_number_of_terms() -> Result<()> {
    let mut terms_list = all_terms();
    shuffle(&mut terms_list, &*random());

    let mut amount = 25;
    while amount < 200 {
        for num_terms in 2..=terms_list.len() {
            let terms = &terms_list[..num_terms];
            for min_nr_should_match in 1..terms.len() as i32 {
                compare(terms, min_nr_should_match, Some(amount), Mode::Scorer)?;
            }
        }
        amount += 25;
    }
    Ok(())
}

// TODO: more tests

/// The iteration state of a [`SlowMinShouldMatchScorer`]: its `iterator()`.
struct SlowMinShouldMatchIterator {
    /// current docid
    current_doc: i32,
    /// current number of terms matched
    current_matched: i32,

    dv: Box<dyn SortedSetDocValues>,
    max_doc: i32,

    ords: HashSet<i64>,
    sims: Vec<Option<Box<dyn SimScorer + Send + Sync>>>,
    norms: Option<Box<dyn NumericDocValues>>,
    min_nr_should_match: i32,

    score: f64,
}

impl DocIdSetIterator for SlowMinShouldMatchIterator {
    fn next_doc(&mut self) -> Result<i32> {
        debug_assert!(self.current_doc != NO_MORE_DOCS);
        self.current_doc += 1;
        while self.current_doc < self.max_doc {
            self.current_matched = 0;
            self.score = 0.0;
            if self.current_doc > self.dv.doc_id() {
                self.dv.advance(self.current_doc)?;
            }
            if self.current_doc != self.dv.doc_id() {
                self.current_doc += 1;
                continue;
            }
            let mut norm = 1i64;
            if let Some(norms) = self.norms.as_mut() {
                if norms.advance_exact(self.current_doc)? {
                    norm = norms.long_value()?;
                }
            }
            for _ in 0..self.dv.doc_value_count()? {
                let ord = self.dv.next_ord()?;
                if self.ords.contains(&ord) {
                    self.current_matched += 1;
                    self.score += self.sims[ord as usize]
                        .as_ref()
                        .expect("a sim for every query ord")
                        .score(1.0, norm) as f64;
                }
            }
            if self.current_matched >= self.min_nr_should_match {
                return Ok(self.current_doc);
            }
            self.current_doc += 1;
        }
        self.current_doc = NO_MORE_DOCS;
        Ok(self.current_doc)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        loop {
            let doc = self.next_doc()?;
            if doc >= target {
                return Ok(doc);
            }
        }
    }

    fn cost(&self) -> i64 {
        self.max_doc as i64
    }

    fn doc_id(&self) -> i32 {
        self.current_doc
    }
}

/// a slow min-should match scorer that uses a docvalues field. later, we can
/// make debugging easier as it can record the set of ords it currently matched
/// and e.g. print out their values and so on for the document
///
/// `static class SlowMinShouldMatchScorer extends Scorer`.
struct SlowMinShouldMatchScorer {
    it: SlowMinShouldMatchIterator,
}

impl SlowMinShouldMatchScorer {
    fn new(
        weight: &BooleanWeight,
        reader: &dyn LeafReader,
        searcher: &IndexSearcher,
    ) -> Result<Self> {
        let dv = reader
            .get_sorted_set_doc_values("dv")?
            .expect("the dv field exists");
        let max_doc = reader.max_doc();
        let bq_query = weight.get_query();
        let bq = bq_query
            .as_any()
            .downcast_ref::<BooleanQuery>()
            .expect("(BooleanQuery) weight.getQuery()");
        let min_nr_should_match = bq.get_minimum_number_should_match();
        let mut sims: Vec<Option<Box<dyn SimScorer + Send + Sync>>> =
            (0..dv.get_value_count()?).map(|_| None).collect();
        let mut ords = HashSet::new();
        for clause in bq.clauses() {
            debug_assert!(!clause.is_prohibited());
            debug_assert!(!clause.is_required());
            let t: Term = clause
                .query()
                .as_any()
                .downcast_ref::<TermQuery>()
                .expect("((TermQuery) clause.query())")
                .get_term()
                .clone();
            let ord = dv.lookup_term(t.bytes())?;
            if ord >= 0 {
                let success = ords.insert(ord);
                debug_assert!(success); // no dups
                let ts = TermStates::build(searcher, &t, true)?;
                let collection_stats = searcher
                    .collection_statistics("field")?
                    .expect("the field has statistics");
                let term_stats =
                    searcher.term_statistics(&t, ts.doc_freq()?, ts.total_term_freq()?)?;
                sims[ord as usize] = Some(Arc::clone(weight.similarity()).scorer_owned(
                    1.0,
                    &collection_stats,
                    &[term_stats],
                ));
            }
        }
        let norms = reader.get_norm_values("field")?;
        Ok(SlowMinShouldMatchScorer {
            it: SlowMinShouldMatchIterator {
                current_doc: -1,
                current_matched: -1,
                dv,
                max_doc,
                ords,
                sims,
                norms,
                min_nr_should_match,
                score: f32::NAN as f64,
            },
        })
    }
}

impl Scorable for SlowMinShouldMatchScorer {
    fn score(&mut self) -> Result<f32> {
        debug_assert!(self.it.score != 0.0, "{}", self.it.current_matched);
        Ok(self.it.score as f32)
    }
}

impl Scorer for SlowMinShouldMatchScorer {
    fn get_max_score(&mut self, _up_to: i32) -> Result<f32> {
        Ok(f32::INFINITY)
    }

    fn doc_id(&self) -> i32 {
        self.it.current_doc
    }

    fn iterator(&mut self) -> &mut dyn DocIdSetIterator {
        &mut self.it
    }
}
