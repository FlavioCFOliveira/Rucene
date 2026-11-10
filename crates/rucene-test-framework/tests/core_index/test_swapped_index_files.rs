//! Port of `lucene/core/src/test/org/apache/lucene/index/TestSwappedIndexFiles.java`.
//!
//! Test that the same file name, but from a different index, is detected as foreign.

use std::sync::Arc;

use randomizedtesting::jdk::random::JdkRandom;
use randomizedtesting::jdk::reflect::Class;
use rucene::document::Document;
use rucene::error::Result;
use rucene::index::check_index::Level;
use rucene::index::directory_reader;
use rucene::index::WRITE_LOCK_NAME;
use rucene::store::{Directory, DEFAULT_IO_CONTEXT};
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::index::RandomIndexWriter;
use rucene_test_framework::util::java_exceptions::expect_throws_any_of;
use rucene_test_framework::util::lucene_test_case::SuppressFileSystems;
use rucene_test_framework::util::{LineFileDocs, LuceneTestCase, TestUtil};

use crate::core_support::{class_annotated, random, test, verbose};
use crate::support::{set_check_index_on_close, sync_one};

/// `TestSwappedIndexFiles`.
pub fn class_def() -> Arc<Class> {
    class_annotated(
        "org.apache.lucene.index.TestSwappedIndexFiles",
        vec![test("test", test_)],
        |b| {
            b.annotate(SuppressFileSystems {
                value: vec!["ExtrasFS".to_string()],
            })
        },
    )
}

/// Port of `TestSwappedIndexFiles.test`.
fn test_() -> Result<()> {
    let mut dir1 = LuceneTestCase::new_directory();
    let mut dir2 = LuceneTestCase::new_directory();

    // Disable CFS 80% of the time so we can truncate individual files, but the other 20% of the
    // time we test truncation of .cfs/.cfe too:
    let use_cfs = random().next_int_bound(5) == 1;

    // Use LineFileDocs so we (hopefully) get most Lucene features
    // tested, e.g. IntPoint was recently added to it:
    let docs = LineFileDocs::with_default_file(&*random())?;
    let doc = docs.next_doc()?;
    let seed = random().next_long();

    index_one_doc(seed, &dir1.as_directory(), &doc.borrow(), use_cfs)?;
    index_one_doc(seed, &dir2.as_directory(), &doc.borrow(), use_cfs)?;

    swap_files(&dir1.as_directory(), &dir2.as_directory())?;
    dir1.close()?;
    dir2.close()
}

/// `private void indexOneDoc(long seed, Directory dir, Document doc, boolean useCFS)`.
fn index_one_doc(seed: i64, dir: &Arc<dyn Directory>, doc: &Document, use_cfs: bool) -> Result<()> {
    let random = JdkRandom::new(seed);
    let conf = LuceneTestCase::new_index_writer_config_with(
        &random,
        Some(Arc::new(MockAnalyzer::new(&random))),
    );
    conf.set_codec(TestUtil::get_default_codec())?;

    if !use_cfs {
        conf.set_use_compound_file(false)?;
        conf.merge_policy().set_no_cfs_ratio(0.0)?;
    } else {
        conf.set_use_compound_file(true)?;
        conf.merge_policy().set_no_cfs_ratio(1.0)?;
    }

    let w = RandomIndexWriter::new_with_config(&random, Arc::clone(dir), conf)?;
    w.add_document(doc)?;
    w.close()
}

/// `private void swapFiles(Directory dir1, Directory dir2)`.
fn swap_files(dir1: &Arc<dyn Directory>, dir2: &Arc<dyn Directory>) -> Result<()> {
    if verbose() {
        println!("TEST: dir1 files: [{}]", dir1.list_all()?.join(", "));
        println!("TEST: dir2 files: [{}]", dir2.list_all()?.join(", "));
    }
    for name in dir1.list_all()? {
        if name == WRITE_LOCK_NAME {
            continue;
        }
        swap_one_file(dir1, dir2, &name)?;
    }
    Ok(())
}

/// `private void swapOneFile(Directory dir1, Directory dir2, String victim)`.
fn swap_one_file(dir1: &Arc<dyn Directory>, dir2: &Arc<dyn Directory>, victim: &str) -> Result<()> {
    if verbose() {
        println!("TEST: swap file {victim}");
    }
    let mut dir_copy = LuceneTestCase::new_directory();
    set_check_index_on_close(&dir_copy, false);
    let copy = dir_copy.as_directory();

    // Copy all files from dir1 to dirCopy, except victim which we copy from dir2:
    for name in dir1.list_all()? {
        if name != victim {
            copy.copy_from(&**dir1, &name, &name, &*DEFAULT_IO_CONTEXT)?;
        } else {
            copy.copy_from(&**dir2, &name, &name, &*DEFAULT_IO_CONTEXT)?;
        }
        sync_one(&copy, &name)?;
    }

    // NOTE: we .close so that if the test fails (truncation not detected) we don't also get all
    // these confusing errors about open files:
    expect_throws_any_of(
        &[
            "org.apache.lucene.index.CorruptIndexException",
            "java.io.EOFException",
            "org.apache.lucene.index.IndexFormatTooOldException",
        ],
        || directory_reader::open(Arc::clone(&copy))?.close(),
    );

    // CheckIndex should also fail:
    expect_throws_any_of(
        &[
            "org.apache.lucene.index.CorruptIndexException",
            "java.io.EOFException",
            "org.apache.lucene.index.IndexFormatTooOldException",
            "org.apache.lucene.index.CheckIndex$CheckIndexException",
        ],
        || {
            TestUtil::check_index_with(
                Arc::clone(&copy),
                Level::MIN_LEVEL_FOR_SLOW_CHECKS,
                true,
                true,
                None,
            )
        },
    );
    drop(copy);
    dir_copy.close()
}
