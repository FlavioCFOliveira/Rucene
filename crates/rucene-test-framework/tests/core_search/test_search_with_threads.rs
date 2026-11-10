//! Port of `lucene/core/src/test/org/apache/lucene/search/TestSearchWithThreads.java`.

use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::jdk::thread::Thread;
use randomizedtesting::junit::Assert;
use rucene::document::{Document, Store};
use rucene::error::Result;
use rucene::index::Term;
use rucene::search::term_query::TermQuery;
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::search::dummy_total_hit_count_collector::DummyTotalHitCountCollector;
use rucene_test_framework::util::java_exceptions::rethrow_java;
use rucene_test_framework::util::lucene_test_case::TEST_NIGHTLY;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_search::support::field_mut;
use crate::core_support::{class, random, test, verbose};

/// `TestSearchWithThreads`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.TestSearchWithThreads",
        vec![test("test", test_search_with_threads)],
    )
}

/// Port of `TestSearchWithThreads.test`.
fn test_search_with_threads() -> Result<()> {
    let num_threads = if *TEST_NIGHTLY { 5 } else { 2 };
    let num_searches = if *TEST_NIGHTLY {
        LuceneTestCase::at_least(2000)
    } else {
        LuceneTestCase::at_least(500)
    };
    let num_docs = if *TEST_NIGHTLY {
        LuceneTestCase::at_least(10000)
    } else {
        LuceneTestCase::at_least(200)
    };

    let mut dir = LuceneTestCase::new_directory();
    let w = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;

    let mut doc = Document::new();
    let body = LuceneTestCase::new_text_field("body", "", Store::NO);
    doc.add(Box::new(body));
    let mut sb = String::new();
    for _doc_count in 0..num_docs {
        let num_terms = random().next_int_bound(10);
        for _term_count in 0..num_terms {
            sb.push_str(if random().next_boolean() {
                "aaa"
            } else {
                "bbb"
            });
            sb.push(' ');
        }
        field_mut(&mut doc, "body").set_string_value(sb.clone())?;
        w.add_document(&doc)?;
        sb.clear();
    }
    let r = w.get_reader()?;
    w.close()?;

    let s = Arc::new(LuceneTestCase::new_searcher(Arc::clone(&r)));

    let failed = Arc::new(AtomicBool::new(false));
    let net_search = Arc::new(AtomicI64::new(0));
    let collector_manager = Arc::new(DummyTotalHitCountCollector::create_manager());
    let mut threads = Vec::with_capacity(num_threads);
    for _thread_id in 0..num_threads {
        let failed = Arc::clone(&failed);
        let net_search = Arc::clone(&net_search);
        let s = Arc::clone(&s);
        let collector_manager = Arc::clone(&collector_manager);
        let thread = Thread::unnamed(move || {
            let run = || -> Result<()> {
                let mut tot_hits: i64 = 0;
                let mut tot_search: i64 = 0;
                while tot_search < i64::from(num_searches) && !failed.load(Ordering::SeqCst) {
                    tot_hits += i64::from(s.search_with_manager(
                        Arc::new(TermQuery::new(Term::from_text("body", "aaa"))),
                        &*collector_manager,
                    )?);
                    tot_hits += i64::from(s.search_with_manager(
                        Arc::new(TermQuery::new(Term::from_text("body", "bbb"))),
                        &*collector_manager,
                    )?);
                    tot_search += 1;
                }
                Assert::assert_true(tot_search > 0 && tot_hits > 0);
                net_search.fetch_add(tot_search, Ordering::SeqCst);
                Ok(())
            };
            if let Err(e) = run() {
                failed.store(true, Ordering::SeqCst);
                rethrow_java(e)
            }
        });
        thread.set_daemon(true);
        threads.push(thread);
    }

    for t in &threads {
        t.start();
    }

    for t in &threads {
        let _ = t.join();
    }

    if verbose() {
        println!(
            "{num_threads} threads did {} searches",
            net_search.load(Ordering::SeqCst)
        );
    }

    drop(s);
    r.close()?;
    dir.close()?;
    Ok(())
}
