//! Port of `lucene/core/src/test/org/apache/lucene/index/TestAllFilesDetectMismatchedChecksum.java`:
//! test that the default codec detects mismatched checksums at open or
//! checkIntegrity time.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use rucene::codecs::codec_util;
use rucene::document::LongPoint;
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::WRITE_LOCK_NAME;
use rucene::store::{Directory, DEFAULT_IO_CONTEXT, READONCE_IO_CONTEXT};
use rucene_test_framework::index::RandomIndexWriter;
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::lucene_test_case::SuppressFileSystems;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::{class_annotated, random, test, verbose};
use crate::support::{add_mutated_docs, disable_cfs, set_check_index_on_close, sync_one};

/// `TestAllFilesDetectMismatchedChecksum`.
pub fn class_def() -> Arc<Class> {
    class_annotated(
        "org.apache.lucene.index.TestAllFilesDetectMismatchedChecksum",
        vec![test("test", test_)],
        |b| {
            b.annotate(SuppressFileSystems {
                value: vec!["ExtrasFS".to_string()],
            })
        },
    )
}

/// Port of `TestAllFilesDetectMismatchedChecksum.test`.
fn test_() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let mut conf = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(
        rucene_test_framework::analysis::MockAnalyzer::new(&*random()),
    )));
    conf.set_codec(TestUtil::get_default_codec())?;
    // Disable CFS, which makes it harder to test due to its double checksumming
    disable_cfs(&mut conf)?;

    let riw = RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), conf)?;
    add_mutated_docs(&riw)?;
    riw.delete_documents_by_query(LongPoint::new_range_query("long", &[0], &[2])?)?;
    riw.close()?;
    check_mismatched_checksum(&dir.as_directory())?;
    dir.close()
}

/// `private void checkMismatchedChecksum(Directory dir)`.
fn check_mismatched_checksum(dir: &Arc<dyn Directory>) -> Result<()> {
    for name in dir.list_all()? {
        if name != WRITE_LOCK_NAME {
            corrupt_file(dir, &name)?;
        }
    }
    Ok(())
}

/// `private void corruptFile(Directory dir, String victim)`.
fn corrupt_file(dir: &Arc<dyn Directory>, victim: &str) -> Result<()> {
    let dir_copy = LuceneTestCase::new_directory();
    set_check_index_on_close(&dir_copy, false);
    let copy = dir_copy.as_directory();
    let victim_length = dir.file_length(victim)?;
    let flip_offset = TestUtil::next_long(
        &*random(),
        0.max(victim_length - codec_util::footer_length() as i64),
        victim_length - 1,
    );

    if verbose() {
        println!(
            "TEST: now corrupt file {victim} by changing byte at offset {flip_offset} (length= {victim_length})"
        );
    }

    for name in dir.list_all()? {
        if name != victim {
            copy.copy_from(&**dir, &name, &name, &*DEFAULT_IO_CONTEXT)?;
        } else {
            let mut out = copy.create_output(&name, &*DEFAULT_IO_CONTEXT)?;
            let mut input = dir.open_input(&name, &*READONCE_IO_CONTEXT)?;
            out.copy_bytes(&mut *input, flip_offset)?;
            let b = input.read_byte()?;
            out.write_byte((b as i8 as i32 + TestUtil::next_int(&*random(), 0x01, 0xFF)) as u8)?;
            out.copy_bytes(&mut *input, victim_length - flip_offset - 1)?;
            input.close()?;
            out.close()?;
        }
        sync_one(&copy, &name)?;
    }

    // corruption must be detected
    expect_throws("org.apache.lucene.index.CorruptIndexException", || {
        let reader = directory_reader::open(Arc::clone(&copy))?;
        let outcome = (|| -> Result<()> {
            for context in Arc::clone(&reader).leaves()? {
                context.leaf_reader().check_integrity()?;
            }
            Ok(())
        })();
        let closed = reader.close();
        outcome?;
        closed
    });
    dir_copy.close()
}
