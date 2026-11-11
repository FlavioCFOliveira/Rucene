//! Port of `lucene/core/src/test/org/apache/lucene/index/TestIndexingSequenceNumbers.java`.
//!
//! * The threads' `Operation` lists, `seqNos` array and `commits` list are
//!   shared through `Arc<Mutex<…>>`: Java shares the arrays and lists between
//!   the threads and the test thread, which reads them after `join()`.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::jdk::thread::Thread;
use randomizedtesting::junit::Assert;
use rucene::document::{
    Document, NumericDocValuesField, NumericValue, Store, StoredField, StringField,
};
use rucene::error::{LuceneError, Result};
use rucene::index::directory_reader;
use rucene::index::index_deletion_policy::NoDeletionPolicy;
use rucene::index::index_writer::IndexWriter;
use rucene::index::multi_doc_values::MultiDocValues;
use rucene::index::terms::Term;
use rucene::index::IndexReader;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::{Query, TermQuery};
use rucene_test_framework::index::RandomIndexWriter;
use rucene_test_framework::util::java_exceptions::rethrow_java;
use rucene_test_framework::util::lucene_test_case::Nightly;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::{assert_equals, assert_equals_msg, class, random, test, verbose};
use crate::support::{npe, CountDownLatch};

/// `TestIndexingSequenceNumbers`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestIndexingSequenceNumbers",
        vec![
            test("testBasic", test_basic),
            test("testAfterRefresh", test_after_refresh),
            test("testAfterCommit", test_after_commit),
            test("testStressUpdateSameID", test_stress_update_same_id).annotate(Nightly),
            test("testStressConcurrentCommit", test_stress_concurrent_commit).annotate(Nightly),
            test(
                "testStressConcurrentDocValuesUpdatesCommit",
                test_stress_concurrent_doc_values_updates_commit,
            )
            .annotate(Nightly),
            test(
                "testStressConcurrentAddAndDeleteAndCommit",
                test_stress_concurrent_add_and_delete_and_commit,
            )
            .annotate(Nightly),
            test("testDeleteAll", test_delete_all),
        ],
    )
}

/// `new IndexWriter(dir, newIndexWriterConfig())`.
fn writer(
    dir: &rucene_test_framework::util::lucene_test_case::TestDirectory,
) -> Result<Arc<IndexWriter>> {
    IndexWriter::new(
        dir.as_directory(),
        LuceneTestCase::new_index_writer_config(),
    )
}

/// Port of `TestIndexingSequenceNumbers.testBasic`.
fn test_basic() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let w = writer(&dir)?;
    let a = w.add_document(&Document::new())?;
    let b = w.add_document(&Document::new())?;
    Assert::assert_true(b > a);
    w.close()?;
    dir.close()
}

/// Port of `TestIndexingSequenceNumbers.testAfterRefresh`.
fn test_after_refresh() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let w = writer(&dir)?;
    let a = w.add_document(&Document::new())?;
    directory_reader::open_with_writer(&*w)?.close()?;
    let b = w.add_document(&Document::new())?;
    Assert::assert_true(b > a);
    w.close()?;
    dir.close()
}

/// Port of `TestIndexingSequenceNumbers.testAfterCommit`.
fn test_after_commit() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let w = writer(&dir)?;
    let a = w.add_document(&Document::new())?;
    w.commit()?;
    let b = w.add_document(&Document::new())?;
    Assert::assert_true(b > a);
    w.close()?;
    dir.close()
}

/// Joins every thread, rethrowing what a thread threw.
fn join_all(threads: &[Thread]) {
    for thread in threads {
        if let Err(payload) = thread.join() {
            std::panic::resume_unwind(payload);
        }
    }
}

/// `catch (Exception e) { throw new RuntimeException(e); }` around a thread
/// body.
fn run_or_rethrow(body: impl FnOnce() -> Result<()>) {
    if let Err(e) = body() {
        rethrow_java(LuceneError::runtime_exception(e));
    }
}

/// `doc.getField("thread").numericValue().intValue()`: `Number.intValue()`
/// narrows as a Java cast does.
fn thread_value(doc: &Document) -> Result<i32> {
    let value = doc
        .get_field("thread")
        .unwrap_or_else(|| npe("thread"))
        .numeric_value()
        .unwrap_or_else(|| npe("numericValue"));
    Ok(match value {
        NumericValue::Int(v) => v,
        NumericValue::Long(v) => v as i32,
        NumericValue::Float(v) => v as i32,
        NumericValue::Double(v) => v as i32,
    })
}

/// Port of `TestIndexingSequenceNumbers.testStressUpdateSameID`.
fn test_stress_update_same_id() -> Result<()> {
    let iters = LuceneTestCase::at_least(100);
    for _ in 0..iters {
        let dir = LuceneTestCase::new_directory();
        let w = Arc::new(RandomIndexWriter::new_with_mock_analyzer(
            &*random(),
            dir.as_directory(),
        )?);
        let num_threads = TestUtil::next_int(&*random(), 2, 5) as usize;
        let starting_gun = Arc::new(CountDownLatch::new(1));
        let seq_nos = Arc::new(Mutex::new(vec![0i64; num_threads]));
        let id = Term::from_text("id", "id");
        // multiple threads update the same document
        let mut threads = Vec::with_capacity(num_threads);
        for i in 0..num_threads {
            let thread_id = i;
            let w = Arc::clone(&w);
            let starting_gun = Arc::clone(&starting_gun);
            let seq_nos = Arc::clone(&seq_nos);
            let id = id.clone();
            let thread = Thread::unnamed(move || {
                run_or_rethrow(|| {
                    let mut doc = Document::new();
                    doc.add(Box::new(StoredField::new_number(
                        "thread",
                        NumericValue::Int(thread_id as i32),
                    )?));
                    doc.add(Box::new(StringField::new(
                        "id",
                        "id".to_string(),
                        Store::NO,
                    )?));
                    starting_gun.await_();
                    for _ in 0..100 {
                        let seq_no = if random().next_boolean() {
                            w.update_document(id.clone(), &doc)?
                        } else {
                            w.update_documents(id.clone(), std::slice::from_ref(&doc))?
                        };
                        seq_nos.lock().unwrap_or_else(|e| e.into_inner())[thread_id] = seq_no;
                    }
                    Ok(())
                })
            });
            thread.start();
            threads.push(thread);
        }
        starting_gun.count_down();
        join_all(&threads);

        // now confirm that the reported sequence numbers agree with the index:
        let seq_nos = seq_nos.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let mut max_thread = 0;
        let mut all_seq_nos = HashSet::new();
        for i in 0..num_threads {
            all_seq_nos.insert(seq_nos[i]);
            if seq_nos[i] > seq_nos[max_thread] {
                max_thread = i;
            }
        }
        // make sure all sequence numbers were different
        assert_equals(&num_threads, &all_seq_nos.len());
        let r = w.get_reader()?;
        let s = LuceneTestCase::new_searcher(Arc::clone(&r));
        let hits = s.search(Arc::new(TermQuery::new(id.clone())), 1)?;
        assert_equals_msg(
            Some(&format!("maxDoc: {}", r.max_doc())),
            &1,
            &hits.total_hits.value(),
        );
        let doc = r.stored_fields()?.document(hits.score_docs[0].doc)?;
        assert_equals(&(max_thread as i32), &thread_value(&doc)?);
        r.close()?;
        drop(s);
        w.close()?;
        drop(w);
        dir.close()?;
    }
    Ok(())
}

/// `static class Operation`.
#[derive(Clone, Copy, Default, Debug)]
struct Operation {
    /// 0 = update, 1 = delete, 2 = commit, 3 = add
    what: u8,
    id: i32,
    #[allow(dead_code)]
    thread_id: i32,
    seq_no: i64,
}

type Ops = Arc<Mutex<Vec<Operation>>>;

/// The `synchronized (commitLock)` block: commits and records the commit
/// when its sequence number is not `-1`.
fn commit_op(w: &IndexWriter, commits: &Ops, op: &mut Operation) -> Result<()> {
    let mut commits = commits.lock().unwrap_or_else(|e| e.into_inner());
    op.seq_no = w.commit()?;
    if op.seq_no != -1 {
        commits.push(*op);
    }
    Ok(())
}

/// The recorded commit operations and the directory's commits.
type FinalCommits = (
    Vec<Operation>,
    Vec<Arc<dyn rucene::index::index_commit::IndexCommit>>,
);

/// The final commit and the listing of the commits, checked against the
/// recorded ones.
fn final_commits(
    w: &IndexWriter,
    dir: &rucene_test_framework::util::lucene_test_case::TestDirectory,
    commits: &Ops,
) -> Result<FinalCommits> {
    let commit_op = Operation {
        seq_no: w.commit()?,
        ..Operation::default()
    };
    let mut commits = commits.lock().unwrap_or_else(|e| e.into_inner()).clone();
    if commit_op.seq_no != -1 {
        commits.push(commit_op);
    }

    let index_commits = directory_reader::list_commits(dir.as_directory())?;
    assert_equals(&commits.len(), &index_commits.len());
    Ok((commits, index_commits))
}

/// `newIndexWriterConfig()` with `NoDeletionPolicy.INSTANCE`, as the
/// `IndexWriter` the stress tests open (they cannot use RIW since it randomly
/// commits).
fn no_deletion_writer(
    dir: &rucene_test_framework::util::lucene_test_case::TestDirectory,
) -> Result<Arc<IndexWriter>> {
    let iwc = LuceneTestCase::new_index_writer_config();
    iwc.set_index_deletion_policy(Some(NoDeletionPolicy::instance()))?;
    IndexWriter::new(dir.as_directory(), iwc)
}

/// `new TermQuery(new Term("id", "" + id))`.
fn id_query(id: i32) -> Arc<dyn Query> {
    Arc::new(TermQuery::new(Term::from_text("id", &id.to_string())))
}

/// Port of `TestIndexingSequenceNumbers.testStressConcurrentCommit`.
fn test_stress_concurrent_commit() -> Result<()> {
    let op_count = LuceneTestCase::at_least(10000);
    let id_count = TestUtil::next_int(&*random(), 10, 1000);

    let dir = LuceneTestCase::new_directory();
    let w = no_deletion_writer(&dir)?;

    let num_threads = TestUtil::next_int(&*random(), 2, 10);
    let starting_gun = Arc::new(CountDownLatch::new(1));
    let mut thread_ops: Vec<Ops> = Vec::new();
    let commits: Ops = Arc::new(Mutex::new(Vec::new()));

    // multiple threads update the same set of documents, and we randomly commit, recording the
    // commit seqNo and then opening each commit in
    // the end to verify it reflects the correct updates
    let mut threads = Vec::new();
    for i in 0..num_threads {
        let ops: Ops = Arc::new(Mutex::new(Vec::new()));
        thread_ops.push(Arc::clone(&ops));
        let thread_id = i;
        let w = Arc::clone(&w);
        let starting_gun = Arc::clone(&starting_gun);
        let commits = Arc::clone(&commits);
        let thread = Thread::unnamed(move || {
            run_or_rethrow(|| {
                starting_gun.await_();
                for _ in 0..op_count {
                    let mut op = Operation {
                        thread_id,
                        ..Operation::default()
                    };
                    if random().next_int_bound(500) == 17 {
                        op.what = 2;
                        commit_op(&w, &commits, &mut op)?;
                    } else {
                        op.id = random().next_int_bound(id_count);
                        let id_term = Term::from_text("id", &op.id.to_string());
                        if random().next_int_bound(10) == 1 {
                            op.what = 1;
                            if random().next_boolean() {
                                op.seq_no = w.delete_documents_by_terms(vec![id_term])?;
                            } else {
                                op.seq_no = w.delete_documents_by_queries(vec![Arc::new(
                                    TermQuery::new(id_term),
                                )])?;
                            }
                        } else {
                            let mut doc = Document::new();
                            doc.add(Box::new(StoredField::new_number(
                                "thread",
                                NumericValue::Int(thread_id),
                            )?));
                            doc.add(Box::new(StringField::new(
                                "id",
                                op.id.to_string(),
                                Store::NO,
                            )?));
                            if random().next_boolean() {
                                let docs = vec![doc];
                                op.seq_no = w.update_documents(id_term, &docs)?;
                            } else {
                                op.seq_no = w.update_document(id_term, &doc)?;
                            }
                            op.what = 0;
                        }
                        ops.lock().unwrap_or_else(|e| e.into_inner()).push(op);
                    }
                }
                Ok(())
            })
        });
        thread.start();
        threads.push(thread);
    }
    starting_gun.count_down();
    join_all(&threads);

    let (commits, index_commits) = final_commits(&w, &dir, &commits)?;
    let thread_ops: Vec<Vec<Operation>> = thread_ops
        .iter()
        .map(|o| o.lock().unwrap_or_else(|e| e.into_inner()).clone())
        .collect();

    let mut expected_thread_ids = vec![0i32; id_count as usize];
    let mut seq_nos = vec![0i64; id_count as usize];

    for i in 0..commits.len() {
        // this commit point should reflect all operations <= this seqNo
        let commit_seq_no = commits[i].seq_no;

        expected_thread_ids.fill(-1);
        seq_nos.fill(0);

        for (thread_id, ops) in thread_ops.iter().enumerate() {
            let mut last_seq_no = 0;
            for op in ops {
                if op.seq_no <= commit_seq_no && op.seq_no > seq_nos[op.id as usize] {
                    seq_nos[op.id as usize] = op.seq_no;
                    if op.what == 0 {
                        expected_thread_ids[op.id as usize] = thread_id as i32;
                    } else {
                        expected_thread_ids[op.id as usize] = -1;
                    }
                }

                Assert::assert_true(op.seq_no > last_seq_no);
                last_seq_no = op.seq_no;
            }
        }

        let r = directory_reader::open_with_commit(&*index_commits[i])?;
        let s = IndexSearcher::new(Arc::clone(&r) as Arc<dyn IndexReader>)?;

        for id in 0..id_count {
            let hits = s.search(id_query(id), 1)?;

            if expected_thread_ids[id as usize] != -1 {
                assert_equals(&1, &hits.total_hits.value());
                let doc = r.stored_fields()?.document(hits.score_docs[0].doc)?;
                let actual_thread_id = thread_value(&doc)?;
                if expected_thread_ids[id as usize] != actual_thread_id {
                    println!(
                        "FAIL: id={id} expectedThreadID={} vs actualThreadID={actual_thread_id} commitSeqNo={commit_seq_no} numThreads={num_threads}",
                        expected_thread_ids[id as usize]
                    );
                    for (thread_id, ops) in thread_ops.iter().enumerate() {
                        for op in ops {
                            if id == op.id {
                                println!(
                                    "  threadID={thread_id} seqNo={} {}",
                                    op.seq_no,
                                    if op.what == 2 { "updated" } else { "deleted" }
                                );
                            }
                        }
                    }
                    assert_equals_msg(
                        Some(&format!("id={id}")),
                        &expected_thread_ids[id as usize],
                        &actual_thread_id,
                    );
                }
            } else if hits.total_hits.value() != 0 {
                println!(
                    "FAIL: id={id} expectedThreadID={} vs totalHits={} commitSeqNo={commit_seq_no} numThreads={num_threads}",
                    expected_thread_ids[id as usize],
                    hits.total_hits.value()
                );
                for (thread_id, ops) in thread_ops.iter().enumerate() {
                    for op in ops {
                        if id == op.id {
                            println!(
                                "  threadID={thread_id} seqNo={} {}",
                                op.seq_no,
                                if op.what == 2 { "updated" } else { "del" }
                            );
                        }
                    }
                }
                assert_equals(&0, &hits.total_hits.value());
            }
        }
        w.close()?;
        r.close()?;
    }

    dir.close()
}

/// Port of `TestIndexingSequenceNumbers.testStressConcurrentDocValuesUpdatesCommit`.
fn test_stress_concurrent_doc_values_updates_commit() -> Result<()> {
    let op_count = LuceneTestCase::at_least(10000);
    let id_count = TestUtil::next_int(&*random(), 10, 1000);

    let dir = LuceneTestCase::new_directory();
    let w = no_deletion_writer(&dir)?;

    let num_threads = TestUtil::next_int(&*random(), 2, 10);
    if verbose() {
        println!("TEST: numThreads={num_threads}");
    }
    let starting_gun = Arc::new(CountDownLatch::new(1));
    let mut thread_ops: Vec<Ops> = Vec::new();
    let commits: Ops = Arc::new(Mutex::new(Vec::new()));

    let ops1: Ops = Arc::new(Mutex::new(Vec::new()));
    thread_ops.push(Arc::clone(&ops1));

    // pre-index every ID so none are missing:
    for id in 0..id_count {
        let thread_id = 0;
        let mut op = Operation {
            thread_id,
            id,
            ..Operation::default()
        };

        let mut doc = Document::new();
        doc.add(Box::new(StoredField::new_number(
            "thread",
            NumericValue::Int(thread_id),
        )?));
        doc.add(Box::new(NumericDocValuesField::new(
            "thread",
            thread_id as i64,
        )));
        doc.add(Box::new(StringField::new("id", id.to_string(), Store::NO)?));
        op.seq_no = w.add_document(&doc)?;
        ops1.lock().unwrap_or_else(|e| e.into_inner()).push(op);
    }

    // multiple threads update the same set of documents, and we randomly commit, recording the
    // commit seqNo and then opening each commit in
    // the end to verify it reflects the correct updates
    let mut threads = Vec::new();
    for i in 0..num_threads {
        let ops: Ops = if i == 0 {
            Arc::clone(&thread_ops[0])
        } else {
            let ops = Arc::new(Mutex::new(Vec::new()));
            thread_ops.push(Arc::clone(&ops));
            ops
        };

        let thread_id = i;
        let w = Arc::clone(&w);
        let starting_gun = Arc::clone(&starting_gun);
        let commits = Arc::clone(&commits);
        let thread = Thread::unnamed(move || {
            run_or_rethrow(|| {
                starting_gun.await_();
                for _ in 0..op_count {
                    let mut op = Operation {
                        thread_id,
                        ..Operation::default()
                    };
                    if random().next_int_bound(500) == 17 {
                        op.what = 2;
                        commit_op(&w, &commits, &mut op)?;
                    } else {
                        op.id = random().next_int_bound(id_count);
                        let id_term = Term::from_text("id", &op.id.to_string());
                        op.seq_no =
                            w.update_numeric_doc_value(id_term, "thread", thread_id as i64)?;
                        op.what = 0;
                        ops.lock().unwrap_or_else(|e| e.into_inner()).push(op);
                    }
                }
                Ok(())
            })
        });
        thread.set_name(format!("thread{i}"));
        thread.start();
        threads.push(thread);
    }
    starting_gun.count_down();
    join_all(&threads);

    let (commits, index_commits) = final_commits(&w, &dir, &commits)?;
    let thread_ops: Vec<Vec<Operation>> = thread_ops
        .iter()
        .map(|o| o.lock().unwrap_or_else(|e| e.into_inner()).clone())
        .collect();

    let mut expected_thread_ids = vec![0i32; id_count as usize];
    let mut seq_nos = vec![0i64; id_count as usize];

    for i in 0..commits.len() {
        // this commit point should reflect all operations <= this seqNo
        let commit_seq_no = commits[i].seq_no;

        expected_thread_ids.fill(-1);
        seq_nos.fill(0);

        for (thread_id, ops) in thread_ops.iter().enumerate() {
            let mut last_seq_no = 0;
            for op in ops {
                if op.seq_no <= commit_seq_no && op.seq_no > seq_nos[op.id as usize] {
                    seq_nos[op.id as usize] = op.seq_no;
                    debug_assert!(op.what == 0);
                    expected_thread_ids[op.id as usize] = thread_id as i32;
                }

                Assert::assert_true(op.seq_no > last_seq_no);
                last_seq_no = op.seq_no;
            }
        }

        let r = directory_reader::open_with_commit(&*index_commits[i])?;
        let ir: Arc<dyn IndexReader> = Arc::clone(&r) as Arc<dyn IndexReader>;
        let s = IndexSearcher::new(Arc::clone(&ir))?;

        for id in 0..id_count {
            let hits = s.search(id_query(id), 1)?;
            let mut doc_values = MultiDocValues::get_numeric_values(&ir, "thread")?
                .unwrap_or_else(|| npe("docValues"));

            // We pre-add all ids up front:
            debug_assert!(expected_thread_ids[id as usize] != -1);
            assert_equals(&1, &hits.total_hits.value());
            let hit_doc = hits.score_docs[0].doc;
            assert_equals(&hit_doc, &doc_values.advance(hit_doc)?);
            let actual_thread_id = doc_values.long_value()? as i32;
            if expected_thread_ids[id as usize] != actual_thread_id {
                println!(
                    "FAIL: commit={i} (of {}) id={id} expectedThreadID={} vs actualThreadID={actual_thread_id} commitSeqNo={commit_seq_no} numThreads={num_threads} reader={r} commit={}",
                    commits.len(),
                    expected_thread_ids[id as usize],
                    index_commits[i]
                );
                for (thread_id, ops) in thread_ops.iter().enumerate() {
                    for op in ops {
                        if id == op.id {
                            println!("  threadID={thread_id} seqNo={}", op.seq_no);
                        }
                    }
                }
                assert_equals_msg(
                    Some(&format!("id={id} docID={}", hits.score_docs[0].doc)),
                    &expected_thread_ids[id as usize],
                    &actual_thread_id,
                );
            }
        }
        w.close()?;
        r.close()?;
    }

    dir.close()
}

/// Port of `TestIndexingSequenceNumbers.testStressConcurrentAddAndDeleteAndCommit`.
fn test_stress_concurrent_add_and_delete_and_commit() -> Result<()> {
    let op_count = LuceneTestCase::at_least(10000);
    let id_count = TestUtil::next_int(&*random(), 10, 1000);

    let dir = LuceneTestCase::new_directory();
    let w = no_deletion_writer(&dir)?;

    let num_threads = TestUtil::next_int(&*random(), 2, 5);
    let starting_gun = Arc::new(CountDownLatch::new(1));
    let mut thread_ops: Vec<Ops> = Vec::new();
    let commits: Ops = Arc::new(Mutex::new(Vec::new()));

    // multiple threads update the same set of documents, and we randomly commit
    let mut threads = Vec::new();
    for i in 0..num_threads {
        let ops: Ops = Arc::new(Mutex::new(Vec::new()));
        thread_ops.push(Arc::clone(&ops));
        let thread_id = i;
        let w = Arc::clone(&w);
        let starting_gun = Arc::clone(&starting_gun);
        let commits = Arc::clone(&commits);
        let thread = Thread::unnamed(move || {
            run_or_rethrow(|| {
                starting_gun.await_();
                for _ in 0..op_count {
                    let mut op = Operation {
                        thread_id,
                        ..Operation::default()
                    };
                    if random().next_int_bound(500) == 17 {
                        op.what = 2;
                        commit_op(&w, &commits, &mut op)?;
                    } else {
                        op.id = random().next_int_bound(id_count);
                        let id_term = Term::from_text("id", &op.id.to_string());
                        if random().next_int_bound(10) == 1 {
                            op.what = 1;
                            if random().next_boolean() {
                                op.seq_no = w.delete_documents_by_terms(vec![id_term])?;
                            } else {
                                op.seq_no = w.delete_documents_by_queries(vec![Arc::new(
                                    TermQuery::new(id_term),
                                )])?;
                            }
                        } else {
                            let ops_size = ops.lock().unwrap_or_else(|e| e.into_inner()).len();
                            let mut doc = Document::new();
                            doc.add(Box::new(StoredField::new_string(
                                "threadop",
                                format!("{thread_id}-{ops_size}"),
                            )?));
                            doc.add(Box::new(StringField::new(
                                "id",
                                op.id.to_string(),
                                Store::NO,
                            )?));
                            if random().next_boolean() {
                                let docs = vec![doc];
                                op.seq_no = w.add_documents(&docs)?;
                            } else {
                                op.seq_no = w.add_document(&doc)?;
                            }
                            op.what = 3;
                        }
                        ops.lock().unwrap_or_else(|e| e.into_inner()).push(op);
                    }
                }
                Ok(())
            })
        });
        thread.set_name(format!("thread{thread_id}"));
        thread.start();
        threads.push(thread);
    }
    starting_gun.count_down();
    join_all(&threads);

    let (commits, index_commits) = final_commits(&w, &dir, &commits)?;
    let thread_ops: Vec<Vec<Operation>> = thread_ops
        .iter()
        .map(|o| o.lock().unwrap_or_else(|e| e.into_inner()).clone())
        .collect();

    // how many docs with this id are expected:
    let mut expected_counts = vec![0i32; id_count as usize];
    let mut last_del_seq_nos = vec![0i64; id_count as usize];

    for i in 0..commits.len() {
        // this commit point should reflect all operations <= this seqNo
        let commit_seq_no = commits[i].seq_no;

        // first find the highest seqNo of the last delete op, for each id, prior to this commit:
        last_del_seq_nos.fill(-1);
        for ops in &thread_ops {
            let mut last_seq_no = 0;
            for op in ops {
                if op.what == 1
                    && op.seq_no <= commit_seq_no
                    && op.seq_no > last_del_seq_nos[op.id as usize]
                {
                    last_del_seq_nos[op.id as usize] = op.seq_no;
                }

                // within one thread the seqNos must only increase:
                Assert::assert_true(op.seq_no > last_seq_no);
                last_seq_no = op.seq_no;
            }
        }

        // then count how many adds happened since the last delete and before this commit:
        expected_counts.fill(0);
        for ops in &thread_ops {
            for op in ops {
                if op.what == 3
                    && op.seq_no <= commit_seq_no
                    && op.seq_no > last_del_seq_nos[op.id as usize]
                {
                    expected_counts[op.id as usize] += 1;
                }
            }
        }

        let r = directory_reader::open_with_commit(&*index_commits[i])?;
        let ir: Arc<dyn IndexReader> = Arc::clone(&r) as Arc<dyn IndexReader>;
        let s = IndexSearcher::new(Arc::clone(&ir))?;

        for id in 0..id_count {
            let actual_count = s.count(id_query(id))?;
            if expected_counts[id as usize] != actual_count {
                println!("TEST: FAIL r={r} id={id} commitSeqNo={commit_seq_no}");
                for (thread_id, ops) in thread_ops.iter().enumerate() {
                    let mut op_count2 = 0;
                    for op in ops {
                        if op.id == id {
                            let should_count = op.seq_no <= commit_seq_no
                                && op.seq_no > last_del_seq_nos[op.id as usize];
                            println!(
                                "  id={id} what={} threadop={thread_id}-{op_count2} seqNo={} vs lastDelSeqNo={} shouldCount={should_count}",
                                op.what,
                                op.seq_no,
                                last_del_seq_nos[op.id as usize]
                            );
                        }
                        op_count2 += 1;
                    }
                }
                let hits = s.search(id_query(id), 1 + actual_count)?;
                let stored_fields = s.stored_fields()?;
                for hit in &hits.score_docs {
                    println!(
                        "  hit: {:?}",
                        stored_fields.document(hit.doc)?.get("threadop")
                    );
                }

                for ctx in Arc::clone(&ir).leaves()? {
                    let reader = ctx.leaf_reader();
                    println!("  sub={reader}");
                    let live_docs = reader.get_live_docs();
                    let stored_fields = reader.stored_fields()?;
                    for doc_id in 0..reader.max_doc() {
                        println!(
                            "    docID={doc_id} threadop={:?}{}",
                            stored_fields.document(doc_id)?.get("threadop"),
                            if live_docs.as_ref().is_some_and(|l| !l.get(doc_id as usize)) {
                                " (deleted)"
                            } else {
                                ""
                            }
                        );
                    }
                }

                assert_equals_msg(
                    Some(&format!(
                        "commit {i} of {} id={id} reader={r}",
                        commits.len()
                    )),
                    &expected_counts[id as usize],
                    &actual_count,
                );
            }
        }
        w.close()?;
        r.close()?;
    }

    dir.close()
}

/// Port of `TestIndexingSequenceNumbers.testDeleteAll`.
fn test_delete_all() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let w = writer(&dir)?;
    let a = w.add_document(&Document::new())?;
    let b = w.delete_all()?;
    Assert::assert_true(a < b);
    let c = w.commit()?;
    Assert::assert_true(b < c);
    w.close()?;
    dir.close()
}
