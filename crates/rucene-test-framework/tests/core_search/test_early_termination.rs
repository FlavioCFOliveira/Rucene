//! Port of `lucene/core/src/test/org/apache/lucene/search/TestEarlyTermination.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::Document;
use rucene::error::Result;
use rucene::index::LeafReaderContext;
use rucene::search::collection_terminated_exception::CollectionTerminatedException;
use rucene::search::match_all_docs_query::MatchAllDocsQuery;
use rucene::search::scorable::Scorable;
use rucene::search::{CollectionResult, Collector, CollectorManager, LeafCollector, ScoreMode};
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_search::support::{fixture_class, ftest, set_up};
use crate::core_support::random;

/// The instance of `TestEarlyTermination`.
#[derive(Default)]
pub struct EarlyTermination {
    dir: Option<TestDirectory>,
    writer: Option<RandomIndexWriter>,
}

/// `TestEarlyTermination`.
pub fn class_def() -> Arc<Class> {
    fixture_class::<EarlyTermination>(
        "org.apache.lucene.search.TestEarlyTermination",
        vec![
            set_up(early_termination_set_up),
            // `tearDown` calls `super.tearDown()` first, then closes.
            rucene_tear_down(),
            ftest("testEarlyTermination", test_early_termination),
        ],
    )
}

/// `public void tearDown() { super.tearDown(); writer.close(); dir.close(); }`:
/// the reverse order of `support::tear_down`.
fn rucene_tear_down() -> randomizedtesting::jdk::reflect::Method {
    randomizedtesting::jdk::reflect::Method::new("tearDown")
        .throws(&["java.lang.Exception"])
        .body(|f: &mut EarlyTermination| {
            LuceneTestCase::super_tear_down();
            let mut run = || -> Result<()> {
                if let Some(w) = f.writer.take() {
                    w.close()?;
                }
                if let Some(d) = f.dir.take() {
                    d.close()?;
                }
                Ok(())
            };
            if let Err(e) = run() {
                rucene_test_framework::util::java_exceptions::rethrow_java(e)
            }
        })
}

fn early_termination_set_up(f: &mut EarlyTermination) -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let writer = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    let num_docs = LuceneTestCase::at_least(100);
    for _i in 0..num_docs {
        writer.add_document(&Document::new())?;
        if LuceneTestCase::rarely() {
            writer.commit()?;
        }
    }
    f.dir = Some(dir);
    f.writer = Some(writer);
    Ok(())
}

/// The anonymous `SimpleCollector` of `testEarlyTermination`.
///
/// **Divergence from Lucene 10.5.0.** Java throws
/// `CollectionTerminatedException` from `doSetNextReader`; Rucene's
/// `SimpleCollectorImpl::do_set_next_reader` returns a `LuceneError`, which
/// cannot carry that signal, so the collector implements `Collector`
/// itself and returns it from `get_leaf_collector`, where `SimpleCollector`
/// calls `doSetNextReader`.
struct TerminatingCollector {
    collection_terminated: bool,
}

impl Collector for TerminatingCollector {
    fn get_leaf_collector<'a>(
        &'a mut self,
        _context: &LeafReaderContext,
    ) -> CollectionResult<Box<dyn LeafCollector + 'a>> {
        // doSetNextReader(context)
        if random().next_boolean() {
            self.collection_terminated = true;
            return Err(CollectionTerminatedException::new().into());
        } else {
            self.collection_terminated = false;
        }
        Ok(Box::new(TerminatingLeafCollector {
            collection_terminated: &mut self.collection_terminated,
        }))
    }

    fn score_mode(&self) -> ScoreMode {
        ScoreMode::COMPLETE_NO_SCORES
    }
}

struct TerminatingLeafCollector<'a> {
    collection_terminated: &'a mut bool,
}

impl LeafCollector for TerminatingLeafCollector<'_> {
    fn set_scorer(&mut self, _scorer: &mut dyn Scorable) -> Result<()> {
        Ok(())
    }

    fn collect(&mut self, _doc: i32, _scorer: &mut dyn Scorable) -> CollectionResult<()> {
        Assert::assert_false(*self.collection_terminated);
        if LuceneTestCase::rarely() {
            *self.collection_terminated = true;
            return Err(CollectionTerminatedException::new().into());
        }
        Ok(())
    }
}

/// The anonymous `CollectorManager<SimpleCollector, Void>`.
struct TerminatingManager;

impl CollectorManager for TerminatingManager {
    type Collector = TerminatingCollector;
    type Output = ();

    fn new_collector(&self) -> Result<TerminatingCollector> {
        // `boolean collectionTerminated = true;`
        Ok(TerminatingCollector {
            collection_terminated: true,
        })
    }

    fn reduce(&self, _collectors: Vec<TerminatingCollector>) -> Result<()> {
        Ok(())
    }
}

fn test_early_termination(f: &mut EarlyTermination) -> Result<()> {
    let iters = LuceneTestCase::at_least(5);
    let reader = f.writer.as_ref().expect("setUp ran").get_reader()?;

    for _i in 0..iters {
        let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
        searcher
            .search_with_manager(Arc::new(MatchAllDocsQuery::instance()), &TerminatingManager)?;
    }
    reader.close()?;
    Ok(())
}
