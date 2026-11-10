//! Port of `lucene/core/src/test/org/apache/lucene/search/TestLiveFieldValues.java`.
//!
//! The anonymous `LiveFieldValues` subclass overriding `lookupFromSearcher`
//! is a [`LiveFieldValuesLookup`]; the anonymous `SearcherFactory` is
//! [`PlainSearcherFactory`].

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::random::{JdkRandom, Random};
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::jdk::thread::Thread;
use randomizedtesting::junit::Assert;
use rucene::document::{NumericValue, Store, StoredField};
use rucene::error::Result;
use rucene::index::index_writer::IndexWriter;
use rucene::index::terms::Term;
use rucene::index::IndexReader;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::live_field_values::{LiveFieldValues, LiveFieldValuesLookup};
use rucene::search::searcher_factory::SearcherFactory;
use rucene::search::searcher_manager::{ManagedSearcher, SearcherManager};
use rucene::search::term_query::TermQuery;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::util::java_exceptions::rethrow_java;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_search::support::{term, CountDownLatch};
use crate::core_support::{assert_equals, assert_equals_msg, class, random, test, verbose};

/// `TestLiveFieldValues`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.TestLiveFieldValues",
        vec![test("test", test_live_field_values)],
    )
}

/// The anonymous `SearcherFactory`: `new IndexSearcher(r)`.
#[derive(Debug)]
struct PlainSearcherFactory;

impl SearcherFactory for PlainSearcherFactory {
    fn new_searcher(
        &self,
        r: Arc<dyn IndexReader>,
        _previous: Option<Arc<dyn IndexReader>>,
    ) -> Result<IndexSearcher> {
        IndexSearcher::new(r)
    }
}

/// The anonymous `LiveFieldValues<IndexSearcher, Integer>.lookupFromSearcher`.
struct Lookup;

impl LiveFieldValuesLookup<ManagedSearcher, i32> for Lookup {
    fn lookup_from_searcher(&self, s: &Arc<ManagedSearcher>, id: &str) -> Result<Option<i32>> {
        let s = s.searcher();
        let tq = Arc::new(TermQuery::new(term("id", id)));
        let hits = s.search(tq, 1)?;
        Assert::assert_true(hits.total_hits.value() <= 1);
        if hits.total_hits.value() == 0 {
            Ok(None)
        } else {
            let doc = s.stored_fields()?.document(hits.score_docs[0].doc)?;
            match doc.get_field("field").and_then(|f| f.numeric_value()) {
                Some(NumericValue::Int(v)) => Ok(Some(v)),
                other => panic!("java.lang.ClassCastException: {other:?}"),
            }
        }
    }
}

fn test_live_field_values() -> Result<()> {
    let mut dir = LuceneTestCase::new_fs_directory(&LuceneTestCase::create_temp_dir_prefix(
        "livefieldupdates",
    ));
    let iwc = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(MockAnalyzer::new(
        &*random(),
    ))));

    let w = IndexWriter::new(dir.as_directory(), iwc)?;

    let mgr = Arc::new(SearcherManager::from_writer(
        &*w,
        true,
        false,
        Some(Arc::new(PlainSearcherFactory)),
        None,
    )?);

    let missing: i32 = -1;

    let rt = LiveFieldValues::new(Arc::clone(&mgr), missing, Lookup);

    let num_threads = TestUtil::next_int(&*random(), 2, 5);
    if verbose() {
        println!("{num_threads} threads");
    }

    let starting_gun = Arc::new(CountDownLatch::new(1));
    let mut threads = Vec::new();

    let iters = LuceneTestCase::at_least(1000);
    let id_count = TestUtil::next_int(&*random(), 100, 10000);

    let reopen_chance = random().next_double() * 0.01;
    let delete_chance = random().next_double() * 0.25;
    let add_chance = random().next_double() * 0.5;

    for t in 0..num_threads {
        let thread_id = t;
        let thread_random = JdkRandom::new(random().next_long());
        let starting_gun = Arc::clone(&starting_gun);
        let w = Arc::clone(&w);
        let rt = Arc::clone(&rt);
        let mgr = Arc::clone(&mgr);
        let thread = Thread::unnamed(move || {
            let run = || -> Result<()> {
                let mut values: HashMap<String, i32> = HashMap::new();
                let all_ids: Mutex<Vec<String>> = Mutex::new(Vec::new());

                starting_gun.await_();
                for _iter in 0..iters {
                    // Add/update a document
                    let mut doc = rucene::document::Document::new();
                    // Threads must not update the same id at the
                    // same time:
                    if thread_random.next_double() <= add_chance {
                        let id = format!(
                            "{}_{:04x}",
                            thread_id,
                            thread_random.next_int_bound(id_count)
                        );
                        let field = thread_random.next_int_bound(i32::MAX);
                        doc.add(Box::new(LuceneTestCase::new_string_field_bytes(
                            "id",
                            LuceneTestCase::new_bytes_ref_str(&id),
                            Store::YES,
                        )));
                        doc.add(Box::new(StoredField::new_number(
                            "field",
                            NumericValue::Int(field),
                        )?));
                        w.update_document(term("id", &id), &doc)?;
                        rt.add(id.clone(), field);
                        if values.insert(id.clone(), field).is_none() {
                            all_ids.lock().unwrap_or_else(|e| e.into_inner()).push(id);
                        }
                    }

                    let len = all_ids.lock().unwrap_or_else(|e| e.into_inner()).len();
                    if len > 0 && thread_random.next_double() <= delete_chance {
                        let random_id = all_ids.lock().unwrap_or_else(|e| e.into_inner())
                            [thread_random.next_int_bound(len as i32) as usize]
                            .clone();
                        w.delete_documents_by_terms(vec![Term::new(
                            "id",
                            LuceneTestCase::new_bytes_ref_str(&random_id),
                        )])?;
                        rt.delete(random_id.clone());
                        values.insert(random_id, missing);
                    }

                    if thread_random.next_double() <= reopen_chance || rt.size() > 10000 {
                        // System.out.println("refresh @ " + rt.size());
                        mgr.maybe_refresh()?;
                        if verbose() {
                            let s = mgr.acquire()?;
                            println!("TEST: reopen {:?}", s.searcher());
                            mgr.release(s)?;
                            println!("TEST: {} values", values.len());
                        }
                    }

                    if thread_random.next_int_bound(10) == 7 {
                        assert_equals(&None, &rt.get("foo")?);
                    }

                    let len = all_ids.lock().unwrap_or_else(|e| e.into_inner()).len();
                    if len > 0 {
                        let random_id = all_ids.lock().unwrap_or_else(|e| e.into_inner())
                            [thread_random.next_int_bound(len as i32) as usize]
                            .clone();
                        let mut expected = values.get(&random_id).copied();
                        if expected == Some(missing) {
                            expected = None;
                        }
                        assert_equals_msg(
                            Some(&format!("id={random_id}")),
                            &expected,
                            &rt.get(&random_id)?,
                        );
                    }
                }
                Ok(())
            };
            // `catch (Throwable t) { throw new RuntimeException(t); }`
            if let Err(e) = run() {
                rethrow_java(e)
            }
        });
        thread.start();
        threads.push(thread);
    }

    starting_gun.count_down();

    for thread in &threads {
        let _ = thread.join();
    }
    mgr.maybe_refresh()?;
    assert_equals(&0, &rt.size());

    rt.close();
    mgr.close()?;
    w.close()?;
    dir.close()?;
    Ok(())
}
