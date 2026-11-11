//! Port of `lucene/core/src/test/org/apache/lucene/index/TestRollingUpdates.java`.

use std::sync::Arc;

use randomizedtesting::jdk::random::{JdkRandom, Random};
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::jdk::thread::Thread;
use randomizedtesting::junit::Assert;
use rucene::document::{Document, Store};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::index_file_names::is_codec_file;
use rucene::index::index_writer::{IndexWriter, MAX_TERM_LENGTH};
use rucene::index::segment_infos::SegmentInfos;
use rucene::index::terms::Term;
use rucene::index::{DirectoryReader, IndexReader};
use rucene::search::term_query::TermQuery;
use rucene::search::IndexSearcher;
use rucene::util::BytesRef;
use rucene_codecs::memory::DirectPostingsFormat;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::util::java_exceptions::rethrow_java;
use rucene_test_framework::util::lucene_test_case::{TEST_NIGHTLY, VERBOSE};
use rucene_test_framework::util::{LineFileDocs, LuceneTestCase, TestUtil};

use crate::core_support::{assert_equals, assert_equals_msg, class, random, test};
use crate::support::{assert_no_unreferenced_files, field_mut};

/// `TestRollingUpdates`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestRollingUpdates",
        vec![
            test("testRollingUpdates", test_rolling_updates),
            test("testUpdateSameDoc", test_update_same_doc),
        ],
    )
}

/// Just updates the same set of N docs over and over, to stress out deletions.
///
/// Port of `TestRollingUpdates.testRollingUpdates`.
fn test_rolling_updates() -> Result<()> {
    let random = JdkRandom::new(crate::core_support::random().next_long());
    let mut dir = LuceneTestCase::new_directory();

    let docs = LineFileDocs::with_default_file(&random)?;

    if crate::core_support::random().next_boolean() {
        rucene::codecs::set_default(TestUtil::always_postings_format(Arc::new(
            DirectPostingsFormat::new(),
        )));
    }

    let analyzer = MockAnalyzer::new(&*crate::core_support::random());
    analyzer.set_max_token_length(TestUtil::next_int(
        &*crate::core_support::random(),
        1,
        MAX_TERM_LENGTH as i32,
    ));

    let w = IndexWriter::new(
        dir.as_directory(),
        LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(analyzer))),
    )?;
    let size = LuceneTestCase::at_least(20);
    let mut id = 0;
    let mut r: Option<Arc<dyn DirectoryReader>> = None;
    let mut s: Option<IndexSearcher> = None;
    let num_updates = (size as f64
        * (2.0
            + if *TEST_NIGHTLY {
                200.0 * crate::core_support::random().next_double()
            } else {
                5.0 * crate::core_support::random().next_double()
            })) as i32;
    if *VERBOSE {
        println!("TEST: numUpdates={num_updates}");
    }
    let mut update_count = 0;
    // TODO: sometimes update ids not in order...
    for doc_iter in 0..num_updates {
        let doc = docs.next_doc()?;
        let my_id = id.to_string();
        if id == size - 1 {
            id = 0;
        } else {
            id += 1;
        }
        if *VERBOSE {
            println!("  docIter={doc_iter} id={id}");
        }
        field_mut(&mut doc.borrow_mut(), "docid").set_string_value(my_id.clone())?;

        let id_term = Term::from_text("docid", &my_id);

        let do_update;
        if s.is_some() && update_count < size {
            let searcher = s.as_ref().expect("checked");
            let hits = searcher.search(Arc::new(TermQuery::new(id_term.clone())), 1)?;
            assert_equals(&1i64, &hits.total_hits.value());
            let reader: Arc<dyn IndexReader> =
                Arc::clone(r.as_ref().expect("a searcher implies a reader")) as _;
            do_update = w.try_delete_document(&reader, hits.score_docs[0].doc)? == -1;
            if *VERBOSE {
                if do_update {
                    println!("  tryDeleteDocument failed");
                } else {
                    println!("  tryDeleteDocument succeeded");
                }
            }
        } else {
            do_update = true;
            if *VERBOSE {
                println!("  no searcher: doUpdate=true");
            }
        }

        update_count += 1;

        if do_update {
            if crate::core_support::random().next_boolean() {
                w.update_document(id_term, &*doc.borrow())?;
            } else {
                // It's OK to not be atomic for this test (no separate thread reopening readers):
                w.delete_documents_by_queries(vec![Arc::new(TermQuery::new(id_term))])?;
                w.add_document(&*doc.borrow())?;
            }
        } else {
            w.add_document(&*doc.borrow())?;
        }

        if doc_iter >= size && crate::core_support::random().next_int_bound(50) == 17 {
            if let Some(r) = r.take() {
                r.close()?;
            }

            let apply_deletions = crate::core_support::random().next_boolean();

            if *VERBOSE {
                println!("TEST: reopen applyDeletions={apply_deletions}");
            }

            let reader = directory_reader::open_with_writer_flags(&*w, apply_deletions, false)?;
            if apply_deletions {
                s = Some(LuceneTestCase::new_searcher(Arc::clone(&reader)));
            } else {
                s = None;
            }
            Assert::assert_true_msg(
                &format!(
                    "applyDeletions={apply_deletions} r.numDocs()={} vs SIZE={size}",
                    reader.num_docs()
                ),
                !apply_deletions || reader.num_docs() == size,
            );
            r = Some(reader);
            update_count = 0;
        }
    }

    if let Some(r) = r.take() {
        r.close()?;
    }

    w.commit()?;
    assert_equals(&size, &w.get_doc_stats()?.num_docs);

    w.close()?;

    assert_no_unreferenced_files(&dir.as_directory(), "leftover files after rolling updates")?;

    docs.close();

    // LUCENE-4455:
    let mut infos = SegmentInfos::read_latest_commit(dir.as_directory())?;
    let mut total_bytes = 0i64;
    for sipc in infos.iter_mut() {
        total_bytes += sipc.size_in_bytes()?;
    }
    let mut total_bytes2 = 0i64;

    for file_name in dir.as_directory().list_all()? {
        if is_codec_file(&file_name) {
            total_bytes2 += dir.as_directory().file_length(&file_name)?;
        }
    }
    assert_equals(&total_bytes2, &total_bytes);
    drop(s);
    dir.close()
}

/// Port of `TestRollingUpdates.testUpdateSameDoc`.
fn test_update_same_doc() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();

    let docs = Arc::new(LineFileDocs::with_default_file(&*random())?);
    for _r in 0..3 {
        let conf = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(
            MockAnalyzer::new(&*random()),
        )));
        conf.set_max_buffered_docs(2)?;
        let w = IndexWriter::new(dir.as_directory(), conf)?;
        let num_updates = LuceneTestCase::at_least(20);
        let num_threads = TestUtil::next_int(&*random(), 2, 6);
        let mut threads = Vec::with_capacity(num_threads as usize);
        for _i in 0..num_threads {
            let t = indexing_thread(Arc::clone(&docs), Arc::clone(&w), num_updates);
            t.start();
            threads.push(t);
        }

        for t in &threads {
            if let Err(p) = t.join() {
                std::panic::resume_unwind(p);
            }
        }

        w.close()?;
    }

    let open = directory_reader::open(dir.as_directory())?;
    assert_equals(&1, &open.num_docs());
    open.close()?;
    docs.close();
    dir.close()
}

/// `static class IndexingThread extends Thread`: `new IndexingThread(docs,
/// writer, num)`.
fn indexing_thread(_docs: Arc<LineFileDocs>, writer: Arc<IndexWriter>, num: i32) -> Thread {
    Thread::unnamed(move || {
        let run = || -> Result<()> {
            let mut open: Option<Arc<dyn DirectoryReader>> = None;
            for i in 0..num {
                let mut doc = Document::new(); // docs.nextDoc();
                let br = BytesRef::new(b"test".to_vec());
                doc.add(Box::new(LuceneTestCase::new_string_field_bytes(
                    "id",
                    br.clone(),
                    Store::NO,
                )));
                writer.update_document(Term::new("id", br), &doc)?;
                if random().next_int_bound(3) == 0 {
                    if open.is_none() {
                        open = Some(directory_reader::open_with_writer(&*writer)?);
                    }
                    let current = open.take().expect("opened above");
                    let reader = directory_reader::open_if_changed(Arc::clone(&current))?;
                    let current = match reader {
                        Some(reader) => {
                            current.close()?;
                            reader
                        }
                        None => current,
                    };
                    assert_equals_msg(
                        Some(&format!(
                            "iter: {i} numDocs: {} del: {} max: {}",
                            current.num_docs(),
                            current.num_deleted_docs(),
                            current.max_doc()
                        )),
                        &1,
                        &current.num_docs(),
                    );
                    open = Some(current);
                }
            }
            if let Some(open) = open {
                open.close()?;
            }
            Ok(())
        };
        if let Err(e) = run() {
            // `throw new RuntimeException(e)`
            rethrow_java(e);
        }
    })
}
