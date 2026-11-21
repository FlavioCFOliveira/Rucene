//! Port of `lucene/core/src/test/org/apache/lucene/index/TestAllFilesDetectTruncation.java`:
//! test that a plain default detects index file truncation early (on opening
//! a reader).

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use rucene::codecs::codec_util;
use rucene::document::LongPoint;
use rucene::error::Result;
use rucene::index::check_index::Level;
use rucene::index::directory_reader;
use rucene::index::WRITE_LOCK_NAME;
use rucene::store::{Directory, IndexInput, DEFAULT_IO_CONTEXT, READONCE_IO_CONTEXT};
use rucene_test_framework::index::RandomIndexWriter;
use rucene_test_framework::util::java_exceptions::{expect_throws, is_instance};
use rucene_test_framework::util::lucene_test_case::SuppressFileSystems;
use rucene_test_framework::util::lucene_test_case::TEST_NIGHTLY;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::{class_annotated, random, test, verbose};
use crate::support::{add_mutated_docs, disable_cfs, set_check_index_on_close, sync_one};

/// `TestAllFilesDetectTruncation`.
pub fn class_def() -> Arc<Class> {
    class_annotated(
        "org.apache.lucene.index.TestAllFilesDetectTruncation",
        vec![test("test", test_), test("testCFS", test_cfs)],
        |b| {
            b.annotate(SuppressFileSystems {
                value: vec!["ExtrasFS".to_string()],
            })
        },
    )
}

/// Port of `TestAllFilesDetectTruncation.test`.
fn test_() -> Result<()> {
    do_test(false)
}

/// Port of `TestAllFilesDetectTruncation.testCFS`.
fn test_cfs() -> Result<()> {
    do_test(true)
}

/// `private void doTest(boolean cfs)`.
fn do_test(cfs: bool) -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let mut conf = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(
        rucene_test_framework::analysis::MockAnalyzer::new(&*random()),
    )));
    conf.set_codec(TestUtil::get_default_codec())?;

    if !cfs {
        disable_cfs(&mut conf)?;
    }

    let riw = RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), conf)?;
    add_mutated_docs(&riw)?;
    if !*TEST_NIGHTLY {
        riw.force_merge(1)?;
    }
    riw.delete_documents_by_query(LongPoint::new_range_query("long", &[0], &[2])?)?;
    riw.close()?;
    check_truncation(&dir.as_directory())?;
    dir.close()
}

/// `private void checkTruncation(Directory dir)`.
fn check_truncation(dir: &Arc<dyn Directory>) -> Result<()> {
    for name in dir.list_all()? {
        if name != WRITE_LOCK_NAME {
            truncate_one_file(dir, &name)?;
        }
    }
    Ok(())
}

/// `private void truncateOneFile(Directory dir, String victim)`.
fn truncate_one_file(dir: &Arc<dyn Directory>, victim: &str) -> Result<()> {
    let dir_copy = LuceneTestCase::new_directory();
    set_check_index_on_close(&dir_copy, false);
    let copy = dir_copy.as_directory();
    let victim_length = dir.file_length(victim)?;
    let lost_bytes = TestUtil::next_int(&*random(), 1, victim_length.min(100) as i32);
    debug_assert!(victim_length > 0);

    if verbose() {
        println!(
            "TEST: now truncate file {victim} by removing {lost_bytes} of {victim_length} bytes"
        );
    }

    for name in dir.list_all()? {
        if name != victim {
            copy.copy_from(&**dir, &name, &name, &*DEFAULT_IO_CONTEXT)?;
        } else {
            {
                let mut input = dir.open_checksum_input(&name)?;
                match codec_util::check_footer(&mut *input) {
                    Ok(_) => {
                        // In some rare cases, the codec footer would still appear as correct even
                        // though the file has been truncated. We just skip the test is this rare
                        // case.
                        input.close()?;
                        return dir_copy.close();
                    }
                    Err(e) if is_instance(&e, "org.apache.lucene.index.CorruptIndexException") => {}
                    Err(e) => {
                        let _ = input.close();
                        return Err(e);
                    }
                }
                input.close()?;
            }

            let mut out = copy.create_output(&name, &*DEFAULT_IO_CONTEXT)?;
            let mut input = dir.open_input(&name, &*READONCE_IO_CONTEXT)?;
            out.copy_bytes(&mut *input, victim_length - lost_bytes as i64)?;
            input.close()?;
            out.close()?;
        }
        sync_one(&copy, &name)?;
    }

    expect_throws("java.lang.Exception", || {
        directory_reader::open(Arc::clone(&copy))?.close()
    });
    expect_throws("java.lang.Exception", || {
        TestUtil::check_index_with(
            Arc::clone(&copy),
            Level::MIN_LEVEL_FOR_SLOW_CHECKS,
            true,
            true,
            None,
        )
    });
    dir_copy.close()
}
