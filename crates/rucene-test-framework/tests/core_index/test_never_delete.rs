//! Port of `lucene/core/src/test/org/apache/lucene/index/TestNeverDelete.java`.
//!
//! Make sure if you use NoDeletionPolicy that no file
//! referenced by a commit point is ever deleted

use std::collections::HashSet;
use std::sync::Arc;

use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::jdk::thread::Thread;
use randomizedtesting::junit::Assert;
use rucene::document::{Document, Store};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::index_deletion_policy::NoDeletionPolicy;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::index::RandomIndexWriter;
use rucene_test_framework::util::java_exceptions::java_throwable;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::{class, random, test, verbose};

/// `TestNeverDelete`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestNeverDelete",
        // `@SuppressForbidden(reason = "Thread sleep")`: a forbidden-APIs
        // marker, with no runtime effect.
        vec![test("testIndexing", test_indexing)],
    )
}

/// Port of `TestNeverDelete.testIndexing`.
fn test_indexing() -> Result<()> {
    let tmp_dir = LuceneTestCase::create_temp_dir_prefix("TestNeverDelete");
    let mut d = LuceneTestCase::new_fs_directory(&tmp_dir);
    let iwc = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(MockAnalyzer::new(
        &*random(),
    ))));
    iwc.set_index_deletion_policy(Some(NoDeletionPolicy::instance()))?;
    let w = Arc::new(RandomIndexWriter::new_with_config(
        &*random(),
        d.as_directory(),
        iwc,
    )?);
    w.w()
        .get_config()
        .set_max_buffered_docs(TestUtil::next_int(&*random(), 5, 30))?;

    w.commit()?;
    let num_threads = random().next_int_bound(4);
    let mut index_threads = Vec::with_capacity(num_threads as usize);
    let stop_iterations = LuceneTestCase::at_least(100);
    for x in 0..num_threads {
        let w = Arc::clone(&w);
        let t = Thread::unnamed(move || {
            let outcome = (|| -> Result<()> {
                let mut doc_count = 0;
                while doc_count < stop_iterations {
                    let mut doc = Document::new();
                    doc.add(Box::new(LuceneTestCase::new_string_field(
                        "dc",
                        &doc_count.to_string(),
                        Store::YES,
                    )));
                    doc.add(Box::new(LuceneTestCase::new_text_field(
                        "field",
                        "here is some text",
                        Store::YES,
                    )));
                    w.add_document(&doc)?;

                    if doc_count % 13 == 0 {
                        w.commit()?;
                    }
                    doc_count += 1;
                }
                Ok(())
            })();
            if let Err(e) = outcome {
                Throwable::from_cause("java.lang.RuntimeException", java_throwable(&e)).throw();
            }
        });
        t.set_name(format!("Thread {x}"));
        t.start();
        index_threads.push(t);
    }

    let mut all_files: HashSet<String> = HashSet::new();

    let mut r = directory_reader::open(d.as_directory())?;
    let mut iterations = 0;
    loop {
        iterations += 1;
        if iterations >= stop_iterations {
            break;
        }
        let ic = r.index_commit()?;
        if verbose() {
            println!("TEST: check files: {:?}", ic.get_file_names()?);
        }
        all_files.extend(ic.get_file_names()?);
        // Make sure no old files were removed
        for file_name in &all_files {
            Assert::assert_true_msg(
                &format!("file {file_name} does not exist"),
                LuceneTestCase::slow_file_exists(&*d.as_directory(), file_name)?,
            );
        }
        if let Some(r2) = directory_reader::open_if_changed(Arc::clone(&r))? {
            r.close()?;
            r = r2;
        }
        Thread::sleep(1);
    }
    r.close()?;

    for t in &index_threads {
        let _ = t.join();
    }
    w.close()?;
    drop(w);
    d.close()
}
