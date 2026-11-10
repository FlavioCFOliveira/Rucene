//! Port of
//! `lucene/core/src/test/org/apache/lucene/search/TestBooleanQueryVisitSubscorers.java`.

// TODO: refactor to a base class, that collects freqs from the scorer tree
// and test all queries with it

use std::collections::HashMap;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::analysis::Analyzer;
use rucene::document::{Document, Store, TextField};
use rucene::error::Result;
use rucene::index::directory_reader::DirectoryReader;
use rucene::index::{IndexReader, LeafReaderContext};
use rucene::search::conjunction_scorer::ConjunctionScorer;
use rucene::search::constant_score_scorer::ConstantScoreScorer;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::match_all_docs_query::MatchAllDocsQuery;
use rucene::search::phrase_query::PhraseQuery;
use rucene::search::scorable::Scorable;
use rucene::search::similarities::{ClassicSimilarity, RawTFSimilarity};
use rucene::search::top_score_doc_collector::TopScoreDocCollector;
use rucene::search::top_score_doc_collector_manager::TopScoreDocCollectorManager;
use rucene::search::wand_scorer::WANDScorer;
use rucene::search::{
    BooleanQueryBuilder, CollectionResult, Collector, CollectorManager, LeafCollector, Occur,
    Query, ScoreMode, TermScorer,
};
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::search::scorer_index_searcher::ScorerIndexSearcher;
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_search::support::{fixture_class, ftest, set_up, tear_down, tq};
use crate::core_support::{assert_equals, random};

const F1: &str = "title";
const F2: &str = "body";

/// The instance of `TestBooleanQueryVisitSubscorers`.
#[derive(Default)]
pub struct VisitSubscorers {
    analyzer: Option<Arc<dyn Analyzer>>,
    reader: Option<Arc<dyn DirectoryReader>>,
    searcher: Option<IndexSearcher>,
    scorer_searcher: Option<IndexSearcher>,
    dir: Option<TestDirectory>,
}

/// `TestBooleanQueryVisitSubscorers`.
pub fn class_def() -> Arc<Class> {
    fixture_class::<VisitSubscorers>(
        "org.apache.lucene.search.TestBooleanQueryVisitSubscorers",
        vec![
            set_up(visit_set_up),
            tear_down(visit_tear_down),
            ftest("testDisjunctions", test_disjunctions),
            ftest("testNestedDisjunctions", test_nested_disjunctions),
            ftest("testConjunctions", test_conjunctions),
            ftest("testDisjunctionMatches", test_disjunction_matches),
            ftest("testMinShouldMatchMatches", test_min_should_match_matches),
            ftest(
                "testGetChildrenMinShouldMatchSumScorer",
                test_get_children_min_should_match_sum_scorer,
            ),
            ftest(
                "testGetChildrenBoosterScorer",
                test_get_children_booster_scorer,
            ),
        ],
    )
}

fn visit_set_up(f: &mut VisitSubscorers) -> Result<()> {
    let analyzer: Arc<dyn Analyzer> = Arc::new(MockAnalyzer::new(&*random()));
    let dir = LuceneTestCase::new_directory();
    let mut config = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::clone(&analyzer)));
    config.set_merge_policy(Some(Arc::new(LuceneTestCase::new_log_merge_policy())))?; // we will use docids to validate
    let writer = RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), config)?;
    writer.add_document(&doc(
        "lucene",
        "lucene is a very popular search engine library",
    )?)?;
    writer.add_document(&doc(
        "solr",
        "solr is a very popular search server and is using lucene",
    )?)?;
    writer.add_document(&doc(
        "nutch",
        "nutch is an internet search engine with web crawler and is using lucene and hadoop",
    )?)?;
    let reader = writer.get_reader()?;
    writer.close()?;
    let mut searcher = LuceneTestCase::new_searcher_assert(Arc::clone(&reader), true, false);
    searcher.set_similarity(Arc::new(ClassicSimilarity::new()));
    let mut scorer_searcher =
        ScorerIndexSearcher::new(Arc::clone(&reader) as Arc<dyn IndexReader>)?;
    scorer_searcher.set_similarity(Arc::new(RawTFSimilarity::new()));
    f.analyzer = Some(analyzer);
    f.dir = Some(dir);
    f.reader = Some(reader);
    f.searcher = Some(searcher);
    f.scorer_searcher = Some(scorer_searcher);
    Ok(())
}

fn visit_tear_down(f: &mut VisitSubscorers) -> Result<()> {
    drop(f.searcher.take());
    drop(f.scorer_searcher.take());
    if let Some(reader) = f.reader.take() {
        reader.close()?;
    }
    if let Some(mut dir) = f.dir.take() {
        dir.close()?;
    }
    Ok(())
}

fn scorer_searcher(f: &VisitSubscorers) -> &IndexSearcher {
    f.scorer_searcher.as_ref().expect("setUp ran")
}

fn test_disjunctions(f: &mut VisitSubscorers) -> Result<()> {
    let mut bq = BooleanQueryBuilder::new();
    bq.add(tq(F1, "lucene"), Occur::SHOULD)?;
    bq.add(tq(F2, "lucene"), Occur::SHOULD)?;
    bq.add(tq(F2, "search"), Occur::SHOULD)?;
    let tfs = get_doc_counts(scorer_searcher(f), Arc::new(bq.build()))?;
    assert_equals(&3, &tfs.len()); // 3 documents
    assert_equals(&3, &tfs[&0]); // f1:lucene + f2:lucene + f2:search
    assert_equals(&2, &tfs[&1]); // f2:search + f2:lucene
    assert_equals(&2, &tfs[&2]); // f2:search + f2:lucene
    Ok(())
}

fn test_nested_disjunctions(f: &mut VisitSubscorers) -> Result<()> {
    let mut bq = BooleanQueryBuilder::new();
    bq.add(tq(F1, "lucene"), Occur::SHOULD)?;
    let mut bq2 = BooleanQueryBuilder::new();
    bq2.add(tq(F2, "lucene"), Occur::SHOULD)?;
    bq2.add(tq(F2, "search"), Occur::SHOULD)?;
    bq.add(Arc::new(bq2.build()), Occur::SHOULD)?;
    let tfs = get_doc_counts(scorer_searcher(f), Arc::new(bq.build()))?;
    assert_equals(&3, &tfs.len()); // 3 documents
    assert_equals(&3, &tfs[&0]); // f1:lucene + f2:lucene + f2:search
    assert_equals(&2, &tfs[&1]); // f2:search + f2:lucene
    assert_equals(&2, &tfs[&2]); // f2:search + f2:lucene
    Ok(())
}

fn test_conjunctions(f: &mut VisitSubscorers) -> Result<()> {
    let mut bq = BooleanQueryBuilder::new();
    bq.add(tq(F2, "lucene"), Occur::MUST)?;
    bq.add(tq(F2, "is"), Occur::MUST)?;
    let tfs = get_doc_counts(scorer_searcher(f), Arc::new(bq.build()))?;
    assert_equals(&3, &tfs.len()); // 3 documents
    assert_equals(&2, &tfs[&0]); // f2:lucene + f2:is
    assert_equals(&3, &tfs[&1]); // f2:is + f2:is + f2:lucene
    assert_equals(&3, &tfs[&2]); // f2:is + f2:is + f2:lucene
    Ok(())
}

fn doc(v1: &str, v2: &str) -> Result<Document> {
    let mut doc = Document::new();
    doc.add(Box::new(TextField::new(F1, v1.to_string(), Store::YES)?));
    doc.add(Box::new(TextField::new(F2, v2.to_string(), Store::YES)?));
    Ok(doc)
}

/// The anonymous `CollectorManager<MyCollector, Map<Integer, Integer>>`.
struct DocCountsManager;

impl CollectorManager for DocCountsManager {
    type Collector = MyCollector;
    type Output = HashMap<i32, i32>;

    fn new_collector(&self) -> Result<MyCollector> {
        MyCollector::new()
    }

    fn reduce(&self, collectors: Vec<MyCollector>) -> Result<HashMap<i32, i32>> {
        let mut doc_counts = HashMap::new();
        for my_collector in collectors {
            doc_counts.extend(my_collector.doc_counts);
        }
        Ok(doc_counts)
    }
}

fn get_doc_counts(searcher: &IndexSearcher, query: Arc<dyn Query>) -> Result<HashMap<i32, i32>> {
    searcher.search_with_manager(query, &DocCountsManager)
}

/// `MyCollector extends FilterCollector`.
///
/// **Divergence from Lucene 10.5.0.** Java's leaf collector keeps the
/// `TermScorer`s it found under the scorer in `setScorer` (`tqsSet`) and
/// reads them in `collect`. Rucene lends the scorer to each call only, so
/// `collect` walks the scorer it is given with the same `fillLeaves`; the
/// set and the sum it yields are the same.
pub struct MyCollector {
    /// `in`, the wrapped collector.
    inner: TopScoreDocCollector,
    pub doc_counts: HashMap<i32, i32>,
}

impl MyCollector {
    fn new() -> Result<MyCollector> {
        Ok(MyCollector {
            inner: TopScoreDocCollectorManager::new(10, None, i32::MAX)?.new_collector()?,
            doc_counts: HashMap::new(),
        })
    }

    /// `topDocs()`.
    #[allow(dead_code)]
    pub fn top_docs(&mut self) -> Result<rucene::search::TopDocs> {
        use rucene::search::top_docs_collector::TopDocsCollector;
        self.inner.top_docs()
    }

    /// `freq(doc)`.
    #[allow(dead_code)]
    pub fn freq(&self, doc: i32) -> i32 {
        self.doc_counts[&doc]
    }
}

impl Collector for MyCollector {
    fn get_leaf_collector<'a>(
        &'a mut self,
        context: &LeafReaderContext,
    ) -> CollectionResult<Box<dyn LeafCollector + 'a>> {
        let doc_base = context.doc_base();
        let inner = self.inner.get_leaf_collector(context)?;
        Ok(Box::new(MyLeafCollector {
            inner,
            doc_counts: &mut self.doc_counts,
            doc_base,
        }))
    }

    fn score_mode(&self) -> ScoreMode {
        self.inner.score_mode()
    }
}

/// The anonymous `FilterLeafCollector` of `MyCollector.getLeafCollector`.
struct MyLeafCollector<'a> {
    inner: Box<dyn LeafCollector + 'a>,
    doc_counts: &'a mut HashMap<i32, i32>,
    doc_base: i32,
}

impl LeafCollector for MyLeafCollector<'_> {
    fn set_scorer(&mut self, scorer: &mut dyn Scorable) -> Result<()> {
        self.inner.set_scorer(scorer)
        // tqsSet.clear(); fillLeaves(scorer, tqsSet): done in `collect`.
    }

    fn collect(&mut self, doc: i32, scorer: &mut dyn Scorable) -> CollectionResult<()> {
        let mut freq = 0i32;
        fill_leaves_and_sum(scorer, doc, &mut freq)?;
        self.doc_counts.insert(doc + self.doc_base, freq);
        self.inner.collect(doc, scorer)
    }
}

/// `fillLeaves(scorer, set)` followed by the `collect` loop over the set:
/// adds the score of every `TermScorer` leaf positioned on `doc`.
fn fill_leaves_and_sum(scorer: &mut dyn Scorable, doc: i32, freq: &mut i32) -> Result<()> {
    let is_term_scorer = scorer
        .as_scorer()
        .is_some_and(|s| s.as_any().is_some_and(|a| a.is::<TermScorer>()));
    if is_term_scorer {
        let s = scorer.as_scorer().expect("checked above");
        if doc == s.doc_id() {
            // Java: `int freq; freq += scorer.score()` (compound assignment
            // narrows the float sum to int on each step).
            *freq = (*freq as f32 + s.score()?) as i32;
        }
    } else {
        for child in scorer.children()? {
            fill_leaves_and_sum(child.child, doc, freq)?;
        }
    }
    Ok(())
}

fn test_disjunction_matches(f: &mut VisitSubscorers) -> Result<()> {
    let s = scorer_searcher(f);
    let leaves = Arc::clone(s.get_index_reader()).leaves()?;
    let mut bq1 = BooleanQueryBuilder::new();
    bq1.add(tq(F1, "lucene"), Occur::SHOULD)?;
    bq1.add(
        Arc::new(PhraseQuery::from_strings(F2, &["search", "engine"])?),
        Occur::SHOULD,
    )?;

    let w1 = s.create_weight(s.rewrite(Arc::new(bq1.build()))?, ScoreMode::COMPLETE, 1.0)?;
    let mut s1 = w1.scorer(&leaves[0])?.expect("scorer");
    assert_equals(&0, &s1.iterator().next_doc()?);
    assert_equals(&2, &s1.children()?.len());

    let mut bq2 = BooleanQueryBuilder::new();
    bq2.add(tq(F1, "lucene"), Occur::SHOULD)?;
    bq2.add(
        Arc::new(PhraseQuery::from_strings(F2, &["search", "library"])?),
        Occur::SHOULD,
    )?;

    let w2 = s.create_weight(s.rewrite(Arc::new(bq2.build()))?, ScoreMode::COMPLETE, 1.0)?;
    let mut s2 = w2.scorer(&leaves[0])?.expect("scorer");
    assert_equals(&0, &s2.iterator().next_doc()?);
    assert_equals(&1, &s2.children()?.len());
    Ok(())
}

fn test_min_should_match_matches(f: &mut VisitSubscorers) -> Result<()> {
    let s = scorer_searcher(f);
    let leaves = Arc::clone(s.get_index_reader()).leaves()?;
    let mut bq = BooleanQueryBuilder::new();
    bq.add(tq(F1, "lucene"), Occur::SHOULD)?;
    bq.add(tq(F2, "lucene"), Occur::SHOULD)?;
    bq.add(
        Arc::new(PhraseQuery::from_strings(F2, &["search", "library"])?),
        Occur::SHOULD,
    )?;
    bq.set_minimum_number_should_match(2);

    let w = s.create_weight(s.rewrite(Arc::new(bq.build()))?, ScoreMode::COMPLETE, 1.0)?;
    let mut sc = w.scorer(&leaves[0])?.expect("scorer");
    assert_equals(&0, &sc.iterator().next_doc()?);
    assert_equals(&2, &sc.children()?.len());
    Ok(())
}

fn test_get_children_min_should_match_sum_scorer(f: &mut VisitSubscorers) -> Result<()> {
    let mut query = BooleanQueryBuilder::new();
    query.add(tq(F2, "nutch"), Occur::SHOULD)?;
    query.add(tq(F2, "web"), Occur::SHOULD)?;
    query.add(tq(F2, "crawler"), Occur::SHOULD)?;
    query.set_minimum_number_should_match(2);
    query.add(Arc::new(MatchAllDocsQuery::instance()), Occur::MUST)?;
    let score_summary = f
        .searcher
        .as_ref()
        .expect("setUp ran")
        .search_with_manager(Arc::new(query.build()), &ScorerSummarizingCollectorManager)?;
    assert_equals(&1, &score_summary.num_hits.load(Ordering::SeqCst));
    Assert::assert_false(score_summary.summaries.is_empty());
    for summary in &score_summary.summaries {
        assert_equals(
            "ConjunctionScorer\n\
             \x20   MUST ConstantScoreScorer\n\
             \x20   MUST WANDScorer\n\
             \x20           SHOULD TermScorer\n\
             \x20           SHOULD TermScorer\n\
             \x20           SHOULD TermScorer",
            summary.as_str(),
        );
    }
    Ok(())
}

fn test_get_children_booster_scorer(f: &mut VisitSubscorers) -> Result<()> {
    let mut query = BooleanQueryBuilder::new();
    query.add(tq(F2, "nutch"), Occur::SHOULD)?;
    query.add(tq(F2, "miss"), Occur::SHOULD)?;
    let score_summary = scorer_searcher(f)
        .search_with_manager(Arc::new(query.build()), &ScorerSummarizingCollectorManager)?;
    assert_equals(&1, &score_summary.num_hits.load(Ordering::SeqCst));
    Assert::assert_false(score_summary.summaries.is_empty());
    for summary in &score_summary.summaries {
        assert_equals("TermScorer", summary.as_str());
    }
    Ok(())
}

#[derive(Default)]
struct ScoreSummary {
    summaries: Vec<String>,
    num_hits: AtomicI32,
}

struct ScorerSummarizingCollectorManager;

impl CollectorManager for ScorerSummarizingCollectorManager {
    type Collector = ScorerSummarizingCollector;
    type Output = ScoreSummary;

    fn new_collector(&self) -> Result<ScorerSummarizingCollector> {
        Ok(ScorerSummarizingCollector {
            score_summary: ScoreSummary::default(),
        })
    }

    fn reduce(&self, collectors: Vec<ScorerSummarizingCollector>) -> Result<ScoreSummary> {
        let mut score_summary = ScoreSummary::default();
        for collector in collectors {
            score_summary
                .summaries
                .extend(collector.score_summary.summaries);
            score_summary.num_hits.fetch_add(
                collector.score_summary.num_hits.load(Ordering::SeqCst),
                Ordering::SeqCst,
            );
        }
        Ok(score_summary)
    }
}

struct ScorerSummarizingCollector {
    score_summary: ScoreSummary,
}

impl Collector for ScorerSummarizingCollector {
    fn score_mode(&self) -> ScoreMode {
        ScoreMode::COMPLETE
    }

    fn get_leaf_collector<'a>(
        &'a mut self,
        _context: &LeafReaderContext,
    ) -> CollectionResult<Box<dyn LeafCollector + 'a>> {
        Ok(Box::new(SummarizingLeafCollector {
            score_summary: &mut self.score_summary,
        }))
    }
}

/// The anonymous `LeafCollector` of `ScorerSummarizingCollector`.
struct SummarizingLeafCollector<'a> {
    score_summary: &'a mut ScoreSummary,
}

impl LeafCollector for SummarizingLeafCollector<'_> {
    fn set_scorer(&mut self, scorer: &mut dyn Scorable) -> Result<()> {
        let mut builder = String::new();
        summarize_scorer(&mut builder, scorer, 0)?;
        self.score_summary.summaries.push(builder);
        Ok(())
    }

    fn collect(&mut self, _doc: i32, _scorer: &mut dyn Scorable) -> CollectionResult<()> {
        self.score_summary.num_hits.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

/// `scorer.getClass().getSimpleName()`.
///
/// **Divergence from Lucene 10.5.0.** Java reflects on the run-time class;
/// Rucene has no class names at run time, so the scorer classes this test
/// meets are a declared list, tested through the opt-in `as_any` pattern.
fn simple_name(scorer: &mut dyn Scorable) -> &'static str {
    let Some(any) = scorer.as_scorer().and_then(|s| s.as_any()) else {
        return "<unlisted Scorable>";
    };
    if any.is::<ConjunctionScorer>() {
        "ConjunctionScorer"
    } else if any.is::<ConstantScoreScorer>() {
        "ConstantScoreScorer"
    } else if any.is::<WANDScorer>() {
        "WANDScorer"
    } else if any.is::<TermScorer>() {
        "TermScorer"
    } else {
        "<unlisted Scorer>"
    }
}

fn summarize_scorer(builder: &mut String, scorer: &mut dyn Scorable, indent: i32) -> Result<()> {
    builder.push_str(simple_name(scorer));
    for child_scorer in scorer.children()? {
        indent_to(builder, indent + 1);
        builder.push_str(&child_scorer.relationship);
        builder.push(' ');
        summarize_scorer(builder, child_scorer.child, indent + 2)?;
    }
    Ok(())
}

fn indent_to(builder: &mut String, indent: i32) {
    if !builder.is_empty() {
        builder.push('\n');
    }
    for _i in 0..indent {
        builder.push_str("    ");
    }
}
