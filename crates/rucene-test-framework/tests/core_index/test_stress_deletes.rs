//! Port of `lucene/core/src/test/org/apache/lucene/index/TestStressDeletes.java`.
//!
//! `exists` is a `ConcurrentHashMap<Integer, Boolean>`; with the small
//! non-negative `Integer` keys of the test, its iteration order is the key
//! order, which the `BTreeMap` here reproduces.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::jdk::thread::Thread;
use rucene::document::{Document, Store};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::terms::Term;
use rucene::search::term_query::TermQuery;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::util::java_exceptions::java_throwable;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::{assert_equals, class, random, test};
use crate::support::CountDownLatch;

/// `TestStressDeletes`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestStressDeletes",
        vec![test("test", test_)],
    )
}

/// One iteration of a thread's loop body.
fn thread_iteration(
    w: &IndexWriter,
    locks: &[Mutex<()>],
    exists: &Mutex<BTreeMap<i32, bool>>,
    num_ids: i32,
    delete_mode: i32,
) -> Result<()> {
    let id = random().next_int_bound(num_ids);
    {
        let _lock = locks[id as usize].lock().unwrap_or_else(|e| e.into_inner());
        let v = exists
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&id)
            .copied();
        if v.is_none() || !v.expect("checked") {
            let mut doc = Document::new();
            doc.add(Box::new(LuceneTestCase::new_string_field(
                "id",
                &id.to_string(),
                Store::NO,
            )));
            w.add_document(&doc)?;
            exists
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(id, true);
        } else {
            if delete_mode == 0 {
                // Always delete by term
                w.delete_documents_by_terms(vec![Term::from_text("id", &id.to_string())])?;
            } else if delete_mode == 1 {
                // Always delete by query
                w.delete_documents_by_queries(vec![Arc::new(TermQuery::new(Term::from_text(
                    "id",
                    &id.to_string(),
                )))])?;
            } else {
                // Mixed
                if random().next_boolean() {
                    w.delete_documents_by_terms(vec![Term::from_text("id", &id.to_string())])?;
                } else {
                    w.delete_documents_by_queries(vec![Arc::new(TermQuery::new(
                        Term::from_text("id", &id.to_string()),
                    ))])?;
                }
            }
            exists
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(id, false);
        }
    }
    if random().next_int_bound(500) == 2 {
        directory_reader::open_with_writer_flags(w, random().next_boolean(), false)?.close()?;
    }
    if random().next_int_bound(500) == 2 {
        w.commit()?;
    }
    Ok(())
}

/// Make sure that order of adds/deletes across threads is respected as long as each ID is only
/// changed by one thread at a time.
///
/// Port of `TestStressDeletes.test`.
fn test_() -> Result<()> {
    let num_ids = LuceneTestCase::at_least(100);
    let locks: Arc<Vec<Mutex<()>>> = Arc::new((0..num_ids).map(|_| Mutex::new(())).collect());

    let dir = LuceneTestCase::new_directory();
    let iwc = IndexWriterConfig::with_analyzer(Some(Arc::new(MockAnalyzer::new(&*random()))));
    let w = IndexWriter::new(dir.as_directory(), iwc)?;
    let iters = LuceneTestCase::at_least(2000);
    let exists: Arc<Mutex<BTreeMap<i32, bool>>> = Arc::new(Mutex::new(BTreeMap::new()));
    let num_threads = TestUtil::next_int(&*random(), 2, 6);
    let starting_gun = Arc::new(CountDownLatch::new(1));
    let delete_mode = random().next_int_bound(3);
    let mut threads = Vec::new();
    for _ in 0..num_threads {
        let (w, locks, exists, starting_gun) = (
            Arc::clone(&w),
            Arc::clone(&locks),
            Arc::clone(&exists),
            Arc::clone(&starting_gun),
        );
        let thread = Thread::unnamed(move || {
            let run = || -> Result<()> {
                starting_gun.await_();
                for _ in 0..iters {
                    thread_iteration(&w, &locks, &exists, num_ids, delete_mode)?;
                }
                Ok(())
            };
            if let Err(e) = run() {
                Throwable::from_cause("java.lang.RuntimeException", java_throwable(&e)).throw()
            }
        });
        thread.start();
        threads.push(thread);
    }

    starting_gun.count_down();
    for thread in &threads {
        let _ = thread.join();
    }

    let r = directory_reader::open_with_writer(&*w)?;
    let s = LuceneTestCase::new_searcher(Arc::clone(&r));
    let entries: Vec<(i32, bool)> = exists
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .map(|(k, v)| (*k, *v))
        .collect();
    for (id, value) in entries {
        let hits = s.search(
            Arc::new(TermQuery::new(Term::from_text("id", &id.to_string()))),
            1,
        )?;
        if value {
            assert_equals(&1, &hits.total_hits.value());
        } else {
            assert_equals(&0, &hits.total_hits.value());
        }
    }
    r.close()?;
    w.close()?;
    dir.close()
}
