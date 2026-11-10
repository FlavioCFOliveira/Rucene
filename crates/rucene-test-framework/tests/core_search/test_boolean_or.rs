//! Port of `lucene/core/src/test/org/apache/lucene/search/TestBooleanOr.java`.

use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::jdk::util::shuffle;
use randomizedtesting::junit::Assert;
use rucene::document::{Document, TextField};
use rucene::error::Result;
use rucene::index::directory_reader::DirectoryReader;
use rucene::search::doc_id_set::DocIdSet;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::scorable::Scorable;
use rucene::search::scorer::Scorer;
use rucene::search::{
    BooleanClause, BooleanQueryBuilder, BooleanScorer, BulkScorer, CollectionResult,
    DocIdSetIterator, LeafCollector, Occur, Query, ScoreMode, NO_MORE_DOCS,
};
use rucene::util::doc_id_set::IntArrayDocIdSet;
use rucene::util::FixedBitSet;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::search::query_utils::QueryUtils;
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_search::support::{fixture_class, ftest, set_up, tear_down, tq};
use crate::core_support::{assert_equals, random};

const FIELD_T: &str = "T";
const FIELD_C: &str = "C";

/// The instance of `TestBooleanOr`.
pub struct BooleanOr {
    t1: Arc<dyn Query>,
    t2: Arc<dyn Query>,
    c1: Arc<dyn Query>,
    c2: Arc<dyn Query>,
    searcher: Option<IndexSearcher>,
    dir: Option<TestDirectory>,
    reader: Option<Arc<dyn DirectoryReader>>,
}

impl Default for BooleanOr {
    fn default() -> Self {
        BooleanOr {
            t1: tq(FIELD_T, "files"),
            t2: tq(FIELD_T, "deleting"),
            c1: tq(FIELD_C, "production"),
            c2: tq(FIELD_C, "optimize"),
            searcher: None,
            dir: None,
            reader: None,
        }
    }
}

/// `TestBooleanOr`.
pub fn class_def() -> Arc<Class> {
    fixture_class::<BooleanOr>(
        "org.apache.lucene.search.TestBooleanOr",
        vec![
            set_up(boolean_or_set_up),
            tear_down(boolean_or_tear_down),
            ftest("testElements", test_elements),
            ftest("testFlat", test_flat),
            ftest("testParenthesisMust", test_parenthesis_must),
            ftest("testParenthesisMust2", test_parenthesis_must2),
            ftest("testParenthesisShould", test_parenthesis_should),
            ftest("testBooleanScorerMax", test_boolean_scorer_max),
            ftest(
                "testSubScorerNextIsNotMatch",
                test_sub_scorer_next_is_not_match,
            ),
        ],
    )
}

fn search(f: &BooleanOr, q: Arc<dyn Query>) -> Result<i64> {
    let searcher = f.searcher.as_ref().expect("setUp ran");
    QueryUtils::check_searcher(&*random(), &q, Some(searcher));
    Ok(searcher.search(q, 1000)?.total_hits.value())
}

fn test_elements(f: &mut BooleanOr) -> Result<()> {
    assert_equals(&1, &search(f, Arc::clone(&f.t1))?);
    assert_equals(&1, &search(f, Arc::clone(&f.t2))?);
    assert_equals(&1, &search(f, Arc::clone(&f.c1))?);
    assert_equals(&1, &search(f, Arc::clone(&f.c2))?);
    Ok(())
}

fn clause(q: &Arc<dyn Query>, occur: Occur) -> BooleanClause {
    BooleanClause::new(Arc::clone(q), occur)
}

/// `T:files T:deleting C:production C:optimize` it works.
fn test_flat(f: &mut BooleanOr) -> Result<()> {
    let mut q = BooleanQueryBuilder::new();
    q.add_clause(clause(&f.t1, Occur::SHOULD))?;
    q.add_clause(clause(&f.t2, Occur::SHOULD))?;
    q.add_clause(clause(&f.c1, Occur::SHOULD))?;
    q.add_clause(clause(&f.c2, Occur::SHOULD))?;
    assert_equals(&1, &search(f, Arc::new(q.build()))?);
    Ok(())
}

/// `(T:files T:deleting) (o4 C:production o4 C:optimize)` combined with
/// `o_q4`.
fn parenthesis(f: &BooleanOr, o4: Occur, o_q4: Occur) -> Result<()> {
    let mut q3 = BooleanQueryBuilder::new();
    q3.add_clause(clause(&f.t1, Occur::SHOULD))?;
    q3.add_clause(clause(&f.t2, Occur::SHOULD))?;
    let mut q4 = BooleanQueryBuilder::new();
    q4.add_clause(clause(&f.c1, o4))?;
    q4.add_clause(clause(&f.c2, o4))?;
    let mut q2 = BooleanQueryBuilder::new();
    q2.add(Arc::new(q3.build()), Occur::SHOULD)?;
    q2.add(Arc::new(q4.build()), o_q4)?;
    assert_equals(&1, &search(f, Arc::new(q2.build()))?);
    Ok(())
}

/// `(T:files T:deleting) (+C:production +C:optimize)` it works.
fn test_parenthesis_must(f: &mut BooleanOr) -> Result<()> {
    parenthesis(f, Occur::MUST, Occur::SHOULD)
}

/// `(T:files T:deleting) +(C:production C:optimize)` not working. results
/// NO HIT.
fn test_parenthesis_must2(f: &mut BooleanOr) -> Result<()> {
    parenthesis(f, Occur::SHOULD, Occur::MUST)
}

/// `(T:files T:deleting) (C:production C:optimize)` not working. results NO
/// HIT.
fn test_parenthesis_should(f: &mut BooleanOr) -> Result<()> {
    parenthesis(f, Occur::SHOULD, Occur::SHOULD)
}

fn boolean_or_set_up(f: &mut BooleanOr) -> Result<()> {
    //
    let dir = LuceneTestCase::new_directory();
    //
    let writer = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    //
    let mut d = Document::new();
    d.add(Box::new(LuceneTestCase::new_field(
        FIELD_T,
        "Optimize not deleting all files",
        TextField::type_stored(),
    )));
    d.add(Box::new(LuceneTestCase::new_field(
        FIELD_C,
        "Deleted When I run an optimize in our production environment.",
        TextField::type_stored(),
    )));
    //
    writer.add_document(&d)?;
    let reader = writer.get_reader()?;
    //
    f.searcher = Some(LuceneTestCase::new_searcher(Arc::clone(&reader)));
    f.reader = Some(reader);
    f.dir = Some(dir);
    writer.close()?;
    Ok(())
}

fn boolean_or_tear_down(f: &mut BooleanOr) -> Result<()> {
    drop(f.searcher.take());
    if let Some(reader) = f.reader.take() {
        reader.close()?;
    }
    if let Some(mut dir) = f.dir.take() {
        dir.close()?;
    }
    Ok(())
}

/// The anonymous `SimpleCollector` of `testBooleanScorerMax`.
///
/// Java's collector also answers `scoreMode()` with `COMPLETE_NO_SCORES`;
/// it is only used as a `LeafCollector`, which has no `scoreMode` here.
struct MaxCollector<'a> {
    hits: &'a Mutex<FixedBitSet>,
    end: &'a AtomicI32,
}

impl LeafCollector for MaxCollector<'_> {
    fn set_scorer(&mut self, _scorer: &mut dyn Scorable) -> Result<()> {
        Ok(())
    }

    fn collect(&mut self, doc: i32, _scorer: &mut dyn Scorable) -> CollectionResult<()> {
        let end = self.end.load(Ordering::SeqCst);
        Assert::assert_true_msg(&format!("collected doc={doc} beyond max={end}"), doc < end);
        self.hits
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .set(doc as usize);
        Ok(())
    }
}

fn test_boolean_scorer_max(_f: &mut BooleanOr) -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let riw = RandomIndexWriter::new_with_config(
        &*random(),
        dir.as_directory(),
        LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(MockAnalyzer::new(
            &*random(),
        )))),
    )?;
    let doc_count = LuceneTestCase::at_least(10000);

    for _ in 0..doc_count {
        let mut doc = Document::new();
        doc.add(Box::new(LuceneTestCase::new_field(
            "field",
            "a",
            TextField::type_not_stored(),
        )));
        riw.add_document(&doc)?;
    }

    riw.force_merge(1)?;
    let r = riw.get_reader()?;
    riw.close()?;

    let s = LuceneTestCase::new_searcher(Arc::clone(&r));
    let mut bq = BooleanQueryBuilder::new();
    bq.add(tq("field", "a"), Occur::SHOULD)?;
    bq.add(tq("field", "a"), Occur::SHOULD)?;

    let w = s.create_weight(s.rewrite(Arc::new(bq.build()))?, ScoreMode::COMPLETE, 1.0)?;

    let leaves = Arc::clone(s.get_index_reader()).leaves()?;
    assert_equals(&1, &leaves.len());
    let mut scorer: Box<dyn BulkScorer> = w.bulk_scorer(&leaves[0])?.expect("matches");

    let hits = Mutex::new(FixedBitSet::new(doc_count as usize));
    let end = AtomicI32::new(0);
    let mut c = MaxCollector {
        hits: &hits,
        end: &end,
    };

    while end.load(Ordering::SeqCst) < doc_count {
        let min = end.load(Ordering::SeqCst);
        let inc = TestUtil::next_int(&*random(), 1, 1000);
        let max = end.fetch_add(inc, Ordering::SeqCst) + inc;
        scorer
            .score(&mut c, None, min, max)
            .map_err(rucene::search::CollectionError::into_lucene_error)?;
    }

    assert_equals(
        &(doc_count as usize),
        &hits.lock().unwrap_or_else(|e| e.into_inner()).cardinality(),
    );
    drop((scorer, w, s));
    r.close()?;
    dir.close()?;
    Ok(())
}

/// The anonymous `Scorer` of `scorer(int...)`.
struct MatchesScorer {
    it: Box<dyn DocIdSetIterator>,
}

impl Scorable for MatchesScorer {
    fn score(&mut self) -> Result<f32> {
        Ok(0.0)
    }
}

impl Scorer for MatchesScorer {
    fn iterator(&mut self) -> &mut dyn DocIdSetIterator {
        &mut *self.it
    }

    fn doc_id(&self) -> i32 {
        self.it.doc_id()
    }

    fn get_max_score(&mut self, _up_to: i32) -> Result<f32> {
        Ok(f32::MAX)
    }
}

fn scorer(matches: &[i32]) -> Result<Box<dyn Scorer>> {
    let mut matches = matches.to_vec();
    matches.push(NO_MORE_DOCS);
    let len = matches.len() - 1;
    let it = IntArrayDocIdSet::new(matches, len)?.iterator()?;
    Ok(Box::new(MatchesScorer { it }))
}

/// The anonymous `LeafCollector` of `testSubScorerNextIsNotMatch`.
struct ListCollector(Vec<i32>);

impl LeafCollector for ListCollector {
    fn set_scorer(&mut self, _scorer: &mut dyn Scorable) -> Result<()> {
        Ok(())
    }

    fn collect(&mut self, doc: i32, _scorer: &mut dyn Scorable) -> CollectionResult<()> {
        self.0.push(doc);
        Ok(())
    }
}

// Make sure that BooleanScorer keeps working even if the sub clauses return
// next matching docs which are less than the actual next match
fn test_sub_scorer_next_is_not_match(_f: &mut BooleanOr) -> Result<()> {
    let mut optional_scorers: Vec<Box<dyn Scorer>> = vec![
        scorer(&[100000, 1000001, 9999999])?,
        scorer(&[4000, 1000051])?,
        scorer(&[5000, 100000, 9999998, 9999999])?,
    ];
    shuffle(&mut optional_scorers, &*random());
    let mut scorer = BooleanScorer::new(optional_scorers, 1, random().next_boolean())?;
    let mut matches = ListCollector(Vec::new());
    scorer
        .score(&mut matches, None, 0, NO_MORE_DOCS)
        .map_err(rucene::search::CollectionError::into_lucene_error)?;
    assert_equals(
        &vec![4000, 5000, 100000, 1000001, 1000051, 9999998, 9999999],
        &matches.0,
    );
    Ok(())
}
