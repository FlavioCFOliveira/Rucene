//! Port of `lucene/core/src/test/org/apache/lucene/index/TestStressNRT.java`.
//!
//! The instance fields the threads guard with `synchronized
//! (TestStressNRT.this)` (`reader`, `committedModel`, `snapshotCount`,
//! `committedModelClock`) are one [`NrtState`] behind a mutex; `model` is the
//! `ConcurrentHashMap`, `lastId` the `volatile int`, `syncArr` one mutex per
//! id.

use std::collections::HashMap;
use std::sync::atomic::{AtomicI32, AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::random::{JdkRandom, Random};
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::jdk::thread::Thread;
use randomizedtesting::junit::Assert;
use rucene::document::{Document, FieldType, Store};
use rucene::error::Result;
use rucene::index::directory_reader::{self, DirectoryReader};
use rucene::index::filter_directory_reader::{
    DirectoryReaderWrapper, SoftDeletesDirectoryReaderWrapper,
};
use rucene::index::terms::Term;
use rucene::search::term_query::TermQuery;
use rucene::search::{IndexSearcher, Query};
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::index::RandomIndexWriter;
use rucene_test_framework::util::java_exceptions::java_throwable;
use rucene_test_framework::util::lucene_test_case::TEST_NIGHTLY;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::{class, random, test, verbose};
use crate::support::npe;

/// `TestStressNRT`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestStressNRT",
        vec![test("test", test_)],
    )
}

/// `final String field = "val_l"`.
const FIELD: &str = "val_l";

/// The fields guarded by `synchronized (TestStressNRT.this)`.
struct NrtState {
    /// `volatile DirectoryReader reader`.
    reader: Arc<dyn DirectoryReader>,
    committed_model: HashMap<i32, i64>,
    snapshot_count: i64,
    committed_model_clock: i64,
}

/// The shared state of the test instance.
struct Shared {
    state: Mutex<NrtState>,
    /// `final ConcurrentHashMap<Integer, Long> model`.
    model: Mutex<HashMap<i32, i64>>,
    /// `volatile int lastId`.
    last_id: AtomicI32,
    /// `Object[] syncArr`.
    sync_arr: Vec<Mutex<()>>,
}

/// Java's `assert cond;` in the test (tests run with `-ea`).
fn java_assert(cond: bool) {
    if !cond {
        Throwable::new("java.lang.AssertionError", "").throw()
    }
}

/// The test's draw-independent parameters, shared by the threads.
#[derive(Clone, Copy)]
struct Params {
    commit_percent: i32,
    soft_commit_percent: i32,
    delete_percent: i32,
    delete_by_query_percent: i32,
    ndocs: i32,
    max_concurrent_commits: i32,
    tombstones: bool,
}

/// Port of `TestStressNRT.test`.
fn test_() -> Result<()> {
    // update variables
    let commit_percent = random().next_int_bound(20);
    let soft_commit_percent = random().next_int_bound(100); // what percent of the commits are soft
    let delete_percent = random().next_int_bound(50);
    let delete_by_query_percent = random().next_int_bound(25);
    let ndocs = LuceneTestCase::at_least(50);
    let n_write_threads = TestUtil::next_int(&*random(), 1, if *TEST_NIGHTLY { 10 } else { 5 });
    // number of committers at a time... needed if we want to avoid commit errors
    // due to exceeding the max
    let max_concurrent_commits =
        TestUtil::next_int(&*random(), 1, if *TEST_NIGHTLY { 10 } else { 5 });
    let use_soft_deletes = random().next_int_bound(10) < 3;

    let tombstones = random().next_boolean();

    // query variables
    // number of query operations to perform in total
    let operations = Arc::new(AtomicI64::new(i64::from(LuceneTestCase::at_least(10000))));

    let n_read_threads = TestUtil::next_int(&*random(), 1, if *TEST_NIGHTLY { 10 } else { 5 });

    // initModel(ndocs)
    let mut model = HashMap::new();
    let mut sync_arr = Vec::new();
    for i in 0..ndocs {
        model.insert(i, -1i64);
        sync_arr.push(Mutex::new(()));
    }
    let committed_model = model.clone();

    let stored_only_type = Arc::new(FieldType::new());
    stored_only_type.set_stored(true)?;

    if verbose() {
        println!("\n");
        println!("TEST: commitPercent={commit_percent}");
        println!("TEST: softCommitPercent={soft_commit_percent}");
        println!("TEST: deletePercent={delete_percent}");
        println!("TEST: deleteByQueryPercent={delete_by_query_percent}");
        println!("TEST: ndocs={ndocs}");
        println!("TEST: nWriteThreads={n_write_threads}");
        println!("TEST: nReadThreads={n_read_threads}");
        println!("TEST: maxConcurrentCommits={max_concurrent_commits}");
        println!("TEST: tombstones={tombstones}");
        println!("TEST: operations={}", operations.load(Ordering::SeqCst));
        println!("\n");
    }

    let num_committing = Arc::new(AtomicI32::new(0));

    let mut threads: Vec<Thread> = Vec::new();

    let dir = LuceneTestCase::new_maybe_virus_checking_directory();

    let writer = Arc::new(RandomIndexWriter::new_with_soft_deletes(
        &*random(),
        dir.as_directory(),
        LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(MockAnalyzer::new(
            &*random(),
        )))),
        use_soft_deletes,
    )?);
    writer.set_do_random_force_merge_assert(false);
    writer.commit()?;
    let reader: Arc<dyn DirectoryReader> = if use_soft_deletes {
        SoftDeletesDirectoryReaderWrapper::new(
            writer
                .w()
                .get_config()
                .soft_deletes_field()
                .unwrap_or_else(|| npe("softDeletesField")),
        )
        .wrap(directory_reader::open(dir.as_directory())?)?
    } else {
        directory_reader::open(dir.as_directory())?
    };

    let shared = Arc::new(Shared {
        state: Mutex::new(NrtState {
            reader,
            committed_model,
            snapshot_count: 0,
            committed_model_clock: 0,
        }),
        model: Mutex::new(model),
        last_id: AtomicI32::new(0),
        sync_arr,
    });

    let params = Params {
        commit_percent,
        soft_commit_percent,
        delete_percent,
        delete_by_query_percent,
        ndocs,
        max_concurrent_commits,
        tombstones,
    };

    for i in 0..n_write_threads {
        let rand = JdkRandom::new(i64::from(random().next_int()));
        let (shared, writer, operations, num_committing, stored_only_type) = (
            Arc::clone(&shared),
            Arc::clone(&writer),
            Arc::clone(&operations),
            Arc::clone(&num_committing),
            Arc::clone(&stored_only_type),
        );
        let thread = Thread::new(format!("WRITER{i}"), move || {
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                writer_run(
                    &rand,
                    &shared,
                    &writer,
                    &operations,
                    &num_committing,
                    &stored_only_type,
                    params,
                )
            }));
            let e = match outcome {
                Ok(Ok(())) => return,
                Ok(Err(e)) => java_throwable(&e),
                Err(payload) => Throwable::from_panic(payload),
            };
            println!("{}: FAILED: unexpected exception", Thread::current().name());
            println!("{e}");
            Throwable::from_cause("java.lang.RuntimeException", e).throw()
        });

        threads.push(thread);
    }

    for i in 0..n_read_threads {
        let rand = JdkRandom::new(i64::from(random().next_int()));
        let (shared, operations) = (Arc::clone(&shared), Arc::clone(&operations));
        let thread = Thread::new(format!("READER{i}"), move || {
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                reader_run(&rand, &shared, &operations, params)
            }));
            let e = match outcome {
                Ok(Ok(())) => return,
                Ok(Err(e)) => java_throwable(&e),
                Err(payload) => Throwable::from_panic(payload),
            };
            operations.store(-1, Ordering::SeqCst);
            println!("{}: FAILED: unexpected exception", Thread::current().name());
            println!("{e}");
            Throwable::from_cause("java.lang.RuntimeException", e).throw()
        });

        threads.push(thread);
    }

    for thread in &threads {
        thread.start();
    }

    for thread in &threads {
        let _ = thread.join();
    }

    writer.close()?;
    let reader = Arc::clone(
        &shared
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .reader,
    );
    if verbose() {
        println!("TEST: close reader={reader}");
    }
    reader.close()?;
    drop(reader);
    drop(shared);
    dir.close()
}

/// A tombstone or a document: `id` and `val_l`.
fn id_doc(id: &str, val: i64, stored_only_type: &FieldType) -> Document {
    let mut d = Document::new();
    d.add(Box::new(LuceneTestCase::new_string_field(
        "id",
        id,
        Store::YES,
    )));
    d.add(Box::new(LuceneTestCase::new_field(
        FIELD,
        &val.to_string(),
        stored_only_type,
    )));
    d
}

/// The `run()` of a `WRITER` thread.
fn writer_run(
    rand: &JdkRandom,
    shared: &Shared,
    writer: &RandomIndexWriter,
    operations: &AtomicI64,
    num_committing: &AtomicI32,
    stored_only_type: &FieldType,
    p: Params,
) -> Result<()> {
    let name = Thread::current().name();
    while operations.load(Ordering::SeqCst) > 0 {
        let oper = rand.next_int_bound(100);

        if oper < p.commit_percent {
            if num_committing.fetch_add(1, Ordering::SeqCst) + 1 <= p.max_concurrent_commits {
                let new_committed_model;
                let version;
                let old_reader;

                {
                    let mut state = shared.state.lock().unwrap_or_else(|e| e.into_inner());
                    // take a snapshot
                    new_committed_model = shared
                        .model
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .clone();
                    version = state.snapshot_count;
                    state.snapshot_count += 1;
                    old_reader = Arc::clone(&state.reader);
                    // increment the reference since we will use this for
                    // reopening
                    old_reader.inc_ref()?;
                }

                let mut new_reader: Option<Arc<dyn DirectoryReader>>;
                if rand.next_int_bound(100) < p.soft_commit_percent {
                    // assertU(h.commit("softCommit","true"));
                    if random().next_boolean() {
                        if verbose() {
                            println!("TEST: {name}: call writer.getReader");
                        }
                        new_reader = Some(writer.get_reader()?);
                    } else {
                        if verbose() {
                            println!("TEST: {name}: reopen reader={old_reader} version={version}");
                        }
                        new_reader = directory_reader::open_if_changed_with_writer(
                            Arc::clone(&old_reader),
                            &**writer.w(),
                            true,
                        )?;
                    }
                } else {
                    // assertU(commit());
                    if verbose() {
                        println!(
                            "TEST: {name}: commit+reopen reader={old_reader} version={version}"
                        );
                    }
                    writer.commit()?;
                    if verbose() {
                        println!("TEST: {name}: now reopen after commit");
                    }
                    new_reader = directory_reader::open_if_changed(Arc::clone(&old_reader))?;
                }

                // Code below assumes newReader comes w/
                // extra ref:
                if new_reader.is_none() {
                    old_reader.inc_ref()?;
                    new_reader = Some(Arc::clone(&old_reader));
                }
                let new_reader = new_reader.expect("set");

                old_reader.dec_ref()?;

                {
                    let mut state = shared.state.lock().unwrap_or_else(|e| e.into_inner());
                    // install the new reader if it's newest (and check the current version
                    // since another reader may have already been installed)
                    // System.out.println(Thread.currentThread().getName() + ": newVersion=" +
                    // newReader.getVersion());
                    java_assert(new_reader.get_ref_count() > 0);
                    java_assert(state.reader.get_ref_count() > 0);
                    if new_reader.version()? > state.reader.version()? {
                        if verbose() {
                            println!("TEST: {name}: install new reader={new_reader}");
                        }
                        state.reader.dec_ref()?;
                        state.reader = Arc::clone(&new_reader);

                        // Silly: forces fieldInfos to be
                        // loaded so we don't hit IOE on later
                        // reader.toString
                        let _ = new_reader.to_string();

                        // install this snapshot only if it's newer than the current one
                        if version >= state.committed_model_clock {
                            if verbose() {
                                println!("TEST: {name}: install new model version={version}");
                            }
                            state.committed_model = new_committed_model;
                            state.committed_model_clock = version;
                        } else if verbose() {
                            println!("TEST: {name}: skip install new model version={version}");
                        }
                    } else {
                        // if the same reader, don't decRef.
                        if verbose() {
                            println!("TEST: {name}: skip install new reader={new_reader}");
                        }
                        new_reader.dec_ref()?;
                    }
                }
            }
            num_committing.fetch_sub(1, Ordering::SeqCst);
        } else {
            let id = rand.next_int_bound(p.ndocs);
            let sync = &shared.sync_arr[id as usize];

            // set the lastId before we actually change it sometimes to try and
            // uncover more race conditions between writing and reading
            let before = random().next_boolean();
            if before {
                shared.last_id.store(id, Ordering::SeqCst);
            }

            // We can't concurrently update the same document and retain our invariants of
            // increasing values
            // since we can't guarantee what order the updates will be executed.
            {
                let _sync = sync.lock().unwrap_or_else(|e| e.into_inner());
                let val = *shared
                    .model
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .get(&id)
                    .unwrap_or_else(|| npe("model.get(id)"));
                let next_val = val.abs() + 1;

                if oper < p.commit_percent + p.delete_percent {
                    // assertU("<delete><id>" + id + "</id></delete>");

                    // add tombstone first
                    if p.tombstones {
                        let d = id_doc(&format!("-{id}"), next_val, stored_only_type);
                        writer.update_document(Term::from_text("id", &format!("-{id}")), &d)?;
                    }

                    if verbose() {
                        println!("TEST: {name}: term delDocs id:{id} nextVal={next_val}");
                    }
                    writer.delete_documents(Term::from_text("id", &id.to_string()))?;
                    shared
                        .model
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .insert(id, -next_val);
                } else if oper < p.commit_percent + p.delete_percent + p.delete_by_query_percent {
                    // assertU("<delete><query>id:" + id + "</query></delete>");

                    // add tombstone first
                    if p.tombstones {
                        let d = id_doc(&format!("-{id}"), next_val, stored_only_type);
                        writer.update_document(Term::from_text("id", &format!("-{id}")), &d)?;
                    }

                    if verbose() {
                        println!("TEST: {name}: query delDocs id:{id} nextVal={next_val}");
                    }
                    writer.delete_documents_by_query(Arc::new(TermQuery::new(Term::from_text(
                        "id",
                        &id.to_string(),
                    ))))?;
                    shared
                        .model
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .insert(id, -next_val);
                } else {
                    // assertU(adoc("id",Integer.toString(id), field, Long.toString(nextVal)));
                    let d = id_doc(&id.to_string(), next_val, stored_only_type);
                    if verbose() {
                        println!("TEST: {name}: u id:{id} val={next_val}");
                    }
                    writer.update_document(Term::from_text("id", &id.to_string()), &d)?;
                    if p.tombstones {
                        // remove tombstone after new addition (this should be optional?)
                        writer.delete_documents(Term::from_text("id", &format!("-{id}")))?;
                    }
                    shared
                        .model
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .insert(id, next_val);
                }
            }

            if !before {
                shared.last_id.store(id, Ordering::SeqCst);
            }
        }
    }
    Ok(())
}

/// The `run()` of a `READER` thread.
fn reader_run(rand: &JdkRandom, shared: &Shared, operations: &AtomicI64, p: Params) -> Result<()> {
    let name = Thread::current().name();
    let mut last_reader: Option<Arc<dyn DirectoryReader>> = None;
    let mut last_searcher: Option<Arc<IndexSearcher>> = None;

    while operations.fetch_sub(1, Ordering::SeqCst) - 1 >= 0 {
        // bias toward a recently changed doc
        let id = if rand.next_int_bound(100) < 25 {
            shared.last_id.load(Ordering::SeqCst)
        } else {
            rand.next_int_bound(p.ndocs)
        };

        // when indexing, we update the index, then the model
        // so when querying, we should first check the model, and then the index

        let val;
        let r;
        {
            let state = shared.state.lock().unwrap_or_else(|e| e.into_inner());
            val = *state
                .committed_model
                .get(&id)
                .unwrap_or_else(|| npe("committedModel.get(id)"));
            r = Arc::clone(&state.reader);
            r.inc_ref()?;
        }

        if verbose() {
            println!("TEST: {name}: s id={id} val={val} r={}", r.version()?);
        }

        //  sreq = req("wt","json", "q","id:"+Integer.toString(id), "omitHeader","true");
        let searcher: Arc<IndexSearcher> = match (&last_reader, &last_searcher) {
            (Some(lr), Some(ls)) if std::ptr::addr_eq(Arc::as_ptr(lr), Arc::as_ptr(&r)) => {
                // Just re-use lastSearcher, else
                // newSearcher may create too many thread
                // pools (ExecutorService):
                Arc::clone(ls)
            }
            _ => {
                let searcher = Arc::new(LuceneTestCase::new_searcher(Arc::clone(&r)));
                last_reader = Some(Arc::clone(&r));
                last_searcher = Some(Arc::clone(&searcher));
                searcher
            }
        };
        let mut q: Arc<dyn Query> =
            Arc::new(TermQuery::new(Term::from_text("id", &id.to_string())));
        let mut results = searcher.search(q, 10)?;

        if results.total_hits.value() == 0 && p.tombstones {
            // if we couldn't find the doc, look for its tombstone
            q = Arc::new(TermQuery::new(Term::from_text("id", &format!("-{id}"))));
            results = searcher.search(q, 1)?;
            if results.total_hits.value() == 0 {
                if val == -1 {
                    // expected... no doc was added yet
                    r.dec_ref()?;
                    continue;
                }
                Assert::fail(&format!(
                    "No documents or tombstones found for id {id}, expected at least {val} reader={r}"
                ));
            }
        }

        if results.total_hits.value() == 0 && !p.tombstones {
            // nothing to do - we can't tell anything from a deleted doc without tombstones
        } else {
            // we should have found the document, or its tombstone
            if results.total_hits.value() != 1 {
                println!("FAIL: hits id:{id} val={val}");
                for sd in &results.score_docs {
                    let doc = r.stored_fields()?.document(sd.doc)?;
                    println!(
                        "  docID={} id:{} foundVal={}",
                        sd.doc,
                        doc.get("id").unwrap_or_else(|| "null".to_string()),
                        doc.get(FIELD).unwrap_or_else(|| "null".to_string())
                    );
                }
                Assert::fail(&format!(
                    "id={id} reader={r} totalHits={}",
                    results.total_hits.value()
                ));
            }
            let doc = searcher
                .stored_fields()?
                .document(results.score_docs[0].doc)?;
            let found_val: i64 = doc
                .get(FIELD)
                .unwrap_or_else(|| npe("doc.get(field)"))
                .parse()
                .unwrap_or_else(|_| {
                    Throwable::new("java.lang.NumberFormatException", "Long.parseLong").throw()
                });
            if found_val < val.abs() {
                Assert::fail(&format!(
                    "foundVal={found_val} val={val} id={id} reader={r}"
                ));
            }
        }

        r.dec_ref()?;
    }
    Ok(())
}
