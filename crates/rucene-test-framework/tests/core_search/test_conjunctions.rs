//! Port of `lucene/core/src/test/org/apache/lucene/search/TestConjunctions.java`.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::analysis::Analyzer;
use rucene::document::{Document, Store, StringField, TextField};
use rucene::error::Result;
use rucene::index::directory_reader::{self, DirectoryReader};
use rucene::index::index_writer::IndexWriter;
use rucene::index::IndexReader;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::scorable::Scorable;
use rucene::search::similarities::RawTFSimilarity;
use rucene::search::term_query::TermQuery;
use rucene::search::{
    BooleanQuery, BooleanQueryBuilder, CollectionResult, CollectorManager, LeafCollector, Occur,
    ScoreMode, SimpleCollector, SimpleCollectorImpl, Weight,
};
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_search::support::{
    close_dir, close_reader, fixture_class, ftest, set_up, tear_down, tq,
};
use crate::core_support::{assert_equals, random};

const F1: &str = "title";
const F2: &str = "body";

/// The instance of `TestConjunctions`.
#[derive(Default)]
pub struct Conjunctions {
    analyzer: Option<Arc<dyn Analyzer>>,
    dir: Option<TestDirectory>,
    reader: Option<Arc<dyn DirectoryReader>>,
    searcher: Option<IndexSearcher>,
}

/// `TestConjunctions`.
pub fn class_def() -> Arc<Class> {
    fixture_class::<Conjunctions>(
        "org.apache.lucene.search.TestConjunctions",
        vec![
            set_up(conjunctions_set_up),
            tear_down(|f: &mut Conjunctions| {
                drop(f.searcher.take());
                close_reader(f.reader.take())?;
                close_dir(f.dir.take())
            }),
            ftest(
                "testTermConjunctionsWithOmitTF",
                test_term_conjunctions_with_omit_tf,
            ),
            ftest("testScorerGetChildren", test_scorer_get_children),
        ],
    )
}

fn conjunctions_set_up(f: &mut Conjunctions) -> Result<()> {
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
    let mut searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    searcher.set_similarity(Arc::new(RawTFSimilarity::new()));
    f.analyzer = Some(analyzer);
    f.dir = Some(dir);
    f.reader = Some(reader);
    f.searcher = Some(searcher);
    Ok(())
}

fn doc(v1: &str, v2: &str) -> Result<Document> {
    let mut doc = Document::new();
    doc.add(Box::new(StringField::new(F1, v1.to_string(), Store::YES)?));
    doc.add(Box::new(TextField::new(F2, v2.to_string(), Store::YES)?));
    Ok(doc)
}

fn test_term_conjunctions_with_omit_tf(f: &mut Conjunctions) -> Result<()> {
    let mut bq = BooleanQueryBuilder::new();
    bq.add(tq(F1, "nutch"), Occur::MUST)?;
    bq.add(tq(F2, "is"), Occur::MUST)?;
    let td = f
        .searcher
        .as_ref()
        .expect("setUp ran")
        .search(Arc::new(bq.build()), 3)?;
    assert_equals(&1, &td.total_hits.value());
    Assert::assert_equals_float_msg(None, 3.0, td.score_docs[0].score, 0.001); // f1:nutch + f2:is + f2:is
    Ok(())
}

/// The test's private `TestCollector extends SimpleCollector`.
struct TestCollector {
    set_scorer_called: Arc<AtomicBool>,
}

impl LeafCollector for TestCollector {
    fn set_scorer(&mut self, s: &mut dyn Scorable) -> Result<()> {
        let child_scorers = s.children()?;
        self.set_scorer_called.store(true, Ordering::SeqCst);
        assert_equals(&2, &child_scorers.len());
        Ok(())
    }

    fn collect(&mut self, _doc: i32, _scorer: &mut dyn Scorable) -> CollectionResult<()> {
        Ok(())
    }
}

impl SimpleCollectorImpl for TestCollector {
    fn score_mode(&self) -> ScoreMode {
        ScoreMode::COMPLETE
    }

    fn set_weight(&mut self, weight: Arc<dyn Weight>) {
        let query = weight.get_query();
        let query = query
            .as_any()
            .downcast_ref::<BooleanQuery>()
            .expect("ClassCastException");
        let clause_list = query.clauses();
        assert_equals(&2, &clause_list.len());
        let mut terms = HashSet::new();
        for clause in clause_list {
            let term_query = clause.query().as_any().downcast_ref::<TermQuery>();
            assert!(term_query.is_some());
            let term = term_query.expect("asserted").get_term();
            assert_equals("field", term.field());
            terms.insert(term.text());
        }
        assert_equals(&2, &terms.len());
        Assert::assert_true(terms.contains("a"));
        Assert::assert_true(terms.contains("b"));
    }
}

/// The anonymous `CollectorManager<TestCollector, Void>`.
struct TestCollectorManager {
    set_scorer_called: std::sync::Mutex<Vec<Arc<AtomicBool>>>,
}

impl CollectorManager for TestCollectorManager {
    type Collector = SimpleCollector<TestCollector>;
    type Output = ();

    fn new_collector(&self) -> Result<Self::Collector> {
        let called = Arc::new(AtomicBool::new(false));
        self.set_scorer_called
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(Arc::clone(&called));
        Ok(SimpleCollector::new(TestCollector {
            set_scorer_called: called,
        }))
    }

    fn reduce(&self, _collectors: Vec<Self::Collector>) -> Result<()> {
        for called in self
            .set_scorer_called
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
        {
            Assert::assert_true(called.load(Ordering::SeqCst));
        }
        Ok(())
    }
}

fn test_scorer_get_children(_f: &mut Conjunctions) -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let w = IndexWriter::new(
        dir.as_directory(),
        LuceneTestCase::new_index_writer_config(),
    )?;
    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_text_field(
        "field",
        "a b",
        Store::NO,
    )));
    w.add_document(&doc)?;
    let r = directory_reader::open_with_writer(&*w)?;
    let mut b = BooleanQueryBuilder::new();
    b.add(tq("field", "a"), Occur::MUST)?;
    b.add(tq("field", "b"), Occur::FILTER)?;
    let q = Arc::new(b.build());
    let s = IndexSearcher::new(Arc::clone(&r) as Arc<dyn IndexReader>)?;
    s.search_with_manager(
        q,
        &TestCollectorManager {
            set_scorer_called: std::sync::Mutex::new(Vec::new()),
        },
    )?;
    drop(s);
    // `IOUtils.close(r, w, dir)`
    let closed = r.close();
    let closed = closed.and(w.close());
    closed.and(dir.close())
}
