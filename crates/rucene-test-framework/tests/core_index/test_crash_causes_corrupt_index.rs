//! Port of `lucene/core/src/test/org/apache/lucene/index/TestCrashCausesCorruptIndex.java`.

use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::{Document, Store};
use rucene::error::{LuceneError, LuceneErrorKind, Result};
use rucene::index::index_writer::IndexWriter;
use rucene::index::terms::Term;
use rucene::search::TermQuery;
use rucene::store::java_nio_file::Path;
use rucene::store::{
    BufferedChecksumIndexInput, Directory, FSDirectory, IOContext, IndexInput, IndexOutput, Lock,
};
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_equals, assert_not_null, class, random, verbose};

/// `TestCrashCausesCorruptIndex`.
///
/// Java's instance field `path` is set by the one test method; the port
/// passes it to the helpers.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestCrashCausesCorruptIndex",
        vec![crate::core_support::test(
            "testCrashCorruptsIndexing",
            test_crash_corrupts_indexing,
        )],
    )
}

/// LUCENE-3627: This test fails.
///
/// Port of `TestCrashCausesCorruptIndex.testCrashCorruptsIndexing`.
fn test_crash_corrupts_indexing() -> Result<()> {
    let path = LuceneTestCase::create_temp_dir_prefix("testCrashCorruptsIndexing");

    index_and_crash_on_create_output_segments2(&path)?;

    search_for_fleas(&path, 2)?;

    index_after_restart(&path)?;

    search_for_fleas(&path, 3)
}

/// `newIndexWriterConfig(new MockAnalyzer(random()))`.
fn mock_iwc() -> rucene::index::index_writer_config::IndexWriterConfig {
    LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(MockAnalyzer::new(&*random()))))
}

/// index 1 document and commit. prepare for crashing. index 1 more
/// document, and upon commit, creation of segments_2 will crash.
fn index_and_crash_on_create_output_segments2(path: &Path) -> Result<()> {
    let real_directory: Box<dyn Directory> = Box::new(FSDirectory::open(path.clone())?);
    let crash_after_create_output = Arc::new(CrashAfterCreateOutput::new(real_directory));

    // NOTE: cannot use RandomIndexWriter because it
    // sometimes commits:
    let index_writer = IndexWriter::new(
        Arc::clone(&crash_after_create_output) as Arc<dyn Directory>,
        mock_iwc(),
    )?;

    index_writer.add_document(&get_document())?;
    // writes segments_1:
    index_writer.commit()?;

    crash_after_create_output.set_crash_after_create_output("pending_segments_2");
    index_writer.add_document(&get_document())?;
    // tries to write segments_2 but hits fake exc:
    expect_throws(CRASHING_EXCEPTION, || index_writer.commit());

    // writes segments_3
    index_writer.close()?;
    drop(index_writer);
    Assert::assert_false(LuceneTestCase::slow_file_exists(
        crash_after_create_output.r#in.as_ref(),
        "segments_2",
    )?);
    match Arc::try_unwrap(crash_after_create_output) {
        Ok(mut dir) => dir.close(),
        Err(_) => Err(LuceneError::illegal_state(
            "the directory is still referenced",
        )),
    }
}

/// Attempts to index another 1 document.
fn index_after_restart(path: &Path) -> Result<()> {
    let mut real_directory = LuceneTestCase::new_fs_directory(path);

    // LUCENE-3627 (before the fix): this line fails because
    // it doesn't know what to do with the created but empty
    // segments_2 file
    let index_writer = IndexWriter::new(real_directory.as_directory(), mock_iwc())?;

    // currently the test fails above.
    // however, to test the fix, the following lines should pass as well.
    index_writer.add_document(&get_document())?;
    index_writer.close()?;
    Assert::assert_false(LuceneTestCase::slow_file_exists(
        &*real_directory.as_directory(),
        "segments_2",
    )?);
    real_directory.close()
}

/// Run an example search.
fn search_for_fleas(path: &Path, expected_total_hits: i64) -> Result<()> {
    let mut real_directory = LuceneTestCase::new_fs_directory(path);
    let index_reader = crate::support::open_reader(&real_directory)?;
    let index_searcher = LuceneTestCase::new_searcher(Arc::clone(&index_reader));
    let top_docs = Some(index_searcher.search(
        Arc::new(TermQuery::new(Term::from_text(TEXT_FIELD, "fleas"))),
        10,
    )?);
    assert_not_null(&top_docs);
    let top_docs = top_docs.expect("checked by assertNotNull");
    assert_equals(&expected_total_hits, &top_docs.total_hits.value());
    index_reader.close()?;
    real_directory.close()
}

const TEXT_FIELD: &str = "text";

/// Gets a document with content "my dog has fleas".
fn get_document() -> Document {
    let mut document = Document::new();
    document.add(Box::new(LuceneTestCase::new_text_field(
        TEXT_FIELD,
        "my dog has fleas",
        Store::NO,
    )));
    document
}

/// The Java class of the marker `RuntimeException` (`private static class
/// CrashingException extends RuntimeException`) that we use in lieu of an
/// actual machine crash.
const CRASHING_EXCEPTION: &str =
    "org.apache.lucene.index.TestCrashCausesCorruptIndex$CrashingException";

/// `new CrashingException(msg)`.
fn crashing_exception(msg: String) -> LuceneError {
    LuceneError::from(LuceneErrorKind::Unchecked {
        class: CRASHING_EXCEPTION,
        message: Some(msg),
        cause: None,
    })
}

/// This test class provides direct access to "simulating" a crash right
/// after realDirectory.createOutput(..) has been called on a certain
/// specified name.
///
/// `private static class CrashAfterCreateOutput extends FilterDirectory`:
/// every method delegates to `in`, except `createOutput`.
struct CrashAfterCreateOutput {
    r#in: Box<dyn Directory>,
    crash_after_create_output: Mutex<Option<String>>,
}

impl CrashAfterCreateOutput {
    fn new(real_directory: Box<dyn Directory>) -> Self {
        CrashAfterCreateOutput {
            r#in: real_directory,
            crash_after_create_output: Mutex::new(None),
        }
    }

    fn set_crash_after_create_output(&self, name: &str) {
        *self
            .crash_after_create_output
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(name.to_string());
    }
}

impl Directory for CrashAfterCreateOutput {
    fn list_all(&self) -> Result<Vec<String>> {
        self.r#in.list_all()
    }
    fn delete_file(&self, name: &str) -> Result<()> {
        self.r#in.delete_file(name)
    }
    fn file_length(&self, name: &str) -> Result<i64> {
        self.r#in.file_length(name)
    }
    fn create_output(&self, name: &str, cxt: &dyn IOContext) -> Result<Box<dyn IndexOutput>> {
        let mut index_output = self.r#in.create_output(name, cxt)?;
        let crash_after = self
            .crash_after_create_output
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        if let Some(crash_after) = crash_after {
            if name == crash_after {
                // CRASH!
                index_output.close()?;
                if verbose() {
                    println!("TEST: now crash");
                    println!("java.lang.Throwable");
                }
                return Err(crashing_exception(format!(
                    "crashAfterCreateOutput {crash_after}"
                )));
            }
        }
        Ok(index_output)
    }
    fn create_temp_output(
        &self,
        prefix: &str,
        suffix: &str,
        context: &dyn IOContext,
    ) -> Result<Box<dyn IndexOutput>> {
        self.r#in.create_temp_output(prefix, suffix, context)
    }
    fn sync(&self, names: &[String]) -> Result<()> {
        self.r#in.sync(names)
    }
    fn sync_metadata(&self) -> Result<()> {
        self.r#in.sync_metadata()
    }
    fn rename(&self, source: &str, dest: &str) -> Result<()> {
        self.r#in.rename(source, dest)
    }
    fn open_input(&self, name: &str, context: &dyn IOContext) -> Result<Box<dyn IndexInput>> {
        self.r#in.open_input(name, context)
    }
    fn open_checksum_input(&self, name: &str) -> Result<Box<BufferedChecksumIndexInput>> {
        self.r#in.open_checksum_input(name)
    }
    fn obtain_lock(&self, name: &str) -> Result<Box<dyn Lock>> {
        self.r#in.obtain_lock(name)
    }
    fn close(&mut self) -> Result<()> {
        self.r#in.close()
    }
    fn get_pending_deletions(&self) -> Result<std::collections::HashSet<String>> {
        self.r#in.get_pending_deletions()
    }
    fn as_filter_directory(&self) -> Option<&dyn Directory> {
        Some(self.r#in.as_ref())
    }
}
