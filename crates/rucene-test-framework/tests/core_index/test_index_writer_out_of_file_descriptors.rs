//! Port of `lucene/core/src/test/org/apache/lucene/index/TestIndexWriterOutOfFileDescriptors.java`.
//!
//! * `catch (AssertionError | IOException | IllegalStateException ioe)`
//!   catches the error a call returns and the `AssertionError` an
//!   assertion panics with.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::Arc;

use randomizedtesting::jdk::exception::{Throwable, ASSERTION_ERROR};
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::index::directory_reader::{self, DirectoryReader};
use rucene::index::index_file_names::{self, is_codec_file};
use rucene::index::index_writer::{IndexWriter, MAX_TERM_LENGTH};
use rucene::index::merge_scheduler::ConcurrentMergeScheduler;
use rucene::store::{Directory, DEFAULT_IO_CONTEXT};
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::store::MockDirectoryWrapper;
use rucene_test_framework::util::java_exceptions::java_throwable;
use rucene_test_framework::util::{LineFileDocs, LuceneTestCase, TestUtil};

use crate::core_support::{class, random, test, verbose};

const IOE: &str = "java.io.IOException";
const ISE: &str = "java.lang.IllegalStateException";

/// `TestIndexWriterOutOfFileDescriptors`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestIndexWriterOutOfFileDescriptors",
        vec![test("test", test_)],
    )
}

/// `ms instanceof ConcurrentMergeScheduler ? (ConcurrentMergeScheduler) ms : null`.
fn as_cms_opt(
    ms: &Arc<dyn rucene::index::merge_scheduler::MergeScheduler>,
) -> Option<&ConcurrentMergeScheduler> {
    ms.as_any()
        .and_then(|a| a.downcast_ref::<ConcurrentMergeScheduler>())
}

/// Port of `TestIndexWriterOutOfFileDescriptors.test`.
fn test_() -> Result<()> {
    let dir = LuceneTestCase::new_mock_fs_directory(&LuceneTestCase::create_temp_dir_prefix(
        "TestIndexWriterOutOfFileDescriptors",
    ));
    let rate = random().next_double() * 0.01;
    // System.out.println("rate=" + rate);
    dir.set_random_io_exception_rate_on_open(rate);
    let iters = LuceneTestCase::at_least(20);
    let docs = LineFileDocs::with_default_file(&*random())?;
    let mut r: Option<Arc<dyn DirectoryReader>> = None;
    let mut r2: Option<Arc<dyn DirectoryReader>> = None;
    let mut any = false;
    let mut dir_copy: Option<MockDirectoryWrapper> = None;
    let mut last_num_docs = 0;
    for iter in 0..iters {
        let mut w: Option<Arc<IndexWriter>> = None;
        if verbose() {
            println!("TEST: iter={iter}");
        }
        let attempt = catch_unwind(AssertUnwindSafe(|| -> Result<()> {
            let analyzer = MockAnalyzer::new(&*random());
            analyzer.set_max_token_length(TestUtil::next_int(
                &*random(),
                1,
                MAX_TERM_LENGTH as i32,
            ));
            let iwc = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(analyzer)));

            if verbose() {
                // Do this ourselves instead of relying on LTC so
                // we see incrementing messageID:
                iwc.set_info_stream_print_stream(Box::new(std::io::stdout()))?;
            }
            let ms = iwc.merge_scheduler();
            if let Some(cms) = as_cms_opt(&ms) {
                cms.set_suppress_exceptions();
            }
            let writer = w.insert(IndexWriter::new(dir.as_directory(), iwc)?);
            if r.is_some() && random().next_int_bound(5) == 3 {
                if random().next_boolean() {
                    if verbose() {
                        println!("TEST: addIndexes LR[]");
                    }
                    #[allow(clippy::unnecessary_unwrap)]
                    // Java's `if (r != null)` then the use of `r`
                    TestUtil::add_indexes_slowly(
                        writer,
                        &[Arc::clone(r.as_ref().expect("checked above"))],
                    )?;
                } else {
                    if verbose() {
                        println!("TEST: addIndexes Directory[]");
                    }
                    let copy = dir_copy
                        .as_ref()
                        .unwrap_or_else(|| crate::support::npe("dirCopy"));
                    writer.add_indexes_dirs(&[copy.as_directory()])?;
                }
            } else {
                if verbose() {
                    println!("TEST: addDocument");
                }
                writer.add_document(&*docs.next_doc()?.borrow())?;
            }
            dir.set_random_io_exception_rate_on_open(0.0);
            if let Some(cms) = as_cms_opt(&ms) {
                cms.sync();
            }
            // If exc hit CMS then writer will be tragically closed:
            if writer.get_tragic_exception().is_none() {
                writer.close()?;
            }
            w = None;

            // NOTE: This is O(N^2)!  Only enable for temporary debugging:
            // dir.setRandomIOExceptionRateOnOpen(0.0);
            // _TestUtil.checkIndex(dir);
            // dir.setRandomIOExceptionRateOnOpen(rate);

            // Verify numDocs only increases, to catch IndexWriter
            // accidentally deleting the index:
            dir.set_random_io_exception_rate_on_open(0.0);
            Assert::assert_true(directory_reader::index_exists(&*dir.as_directory())?);
            match r2.take() {
                None => r2 = Some(directory_reader::open(dir.as_directory())?),
                Some(old) => match directory_reader::open_if_changed(Arc::clone(&old))? {
                    Some(r3) => {
                        r2 = Some(Arc::clone(&old));
                        old.close()?;
                        r2 = Some(r3);
                    }
                    None => r2 = Some(old),
                },
            }
            let current = r2.as_ref().expect("just set");
            Assert::assert_true_msg(
                &format!("before={last_num_docs} after={}", current.num_docs()),
                current.num_docs() >= last_num_docs,
            );
            last_num_docs = current.num_docs();
            // System.out.println("numDocs=" + lastNumDocs);
            dir.set_random_io_exception_rate_on_open(rate);

            any = true;
            if verbose() {
                println!("TEST: iter={iter}: success");
            }
            Ok(())
        }));
        let caught: Option<Throwable> = match attempt {
            Ok(Ok(())) => None,
            Ok(Err(e)) => {
                let t = java_throwable(&e);
                if t.is_instance(ASSERTION_ERROR) || t.is_instance(IOE) || t.is_instance(ISE) {
                    Some(t)
                } else {
                    return Err(e);
                }
            }
            Err(payload) => {
                let t = Throwable::from_panic(payload);
                if t.is_instance(ASSERTION_ERROR) {
                    Some(t)
                } else {
                    t.throw()
                }
            }
        };
        if let Some(ioe) = caught {
            if verbose() {
                println!("TEST: iter={iter}: exception");
                println!("{ioe}");
            }
            if let Some(w) = &w {
                // NOTE: leave random IO exceptions enabled here,
                // to verify that rollback does not try to write
                // anything:
                w.rollback()?;
            }
        }

        if any && r.is_none() && random().next_boolean() {
            // Make a copy of a non-empty index so we can use
            // it to addIndexes later:
            dir.set_random_io_exception_rate_on_open(0.0);
            r = Some(directory_reader::open(dir.as_directory())?);
            let copy = dir_copy.insert(LuceneTestCase::new_mock_fs_directory(
                &LuceneTestCase::create_temp_dir_prefix("TestIndexWriterOutOfFileDescriptors.copy"),
            ));
            let mut files: Vec<String> = Vec::new();
            for file in dir.list_all()? {
                if file.starts_with(index_file_names::SEGMENTS) || is_codec_file(&file) {
                    copy.copy_from(&dir, &file, &file, &*DEFAULT_IO_CONTEXT)?;
                    if !files.contains(&file) {
                        files.push(file);
                    }
                }
            }
            copy.sync(&files)?;
            // Have IW kiss the dir so we remove any leftover
            // files ... we can easily have leftover files at
            // the time we take a copy because we are holding
            // open a reader:
            IndexWriter::new(
                copy.as_directory(),
                LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(
                    MockAnalyzer::new(&*random()),
                ))),
            )?
            .close()?;
            copy.set_random_io_exception_rate(rate);
            dir.set_random_io_exception_rate_on_open(rate);
        }
    }

    if let Some(r2) = r2 {
        r2.close()?;
    }
    if let Some(r) = r {
        r.close()?;
        if let Some(copy) = dir_copy {
            copy.close()?;
        }
    }
    dir.close()
}
