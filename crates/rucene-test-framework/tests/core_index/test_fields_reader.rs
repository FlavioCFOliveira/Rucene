//! Port of `lucene/core/src/test/org/apache/lucene/index/TestFieldsReader.java`.
//!
//! * The static `testDoc` is rebuilt with `DocHelper.setupDoc` where it is
//!   used: Rucene's `Document` owns its fields and is not `Send`, and
//!   `setupDoc` draws nothing, so each build holds the same fields.
//! * `FaultyIndexInput extends BufferedIndexInput` is the unbuffered source a
//!   Rucene `BufferedIndexInput` buffers: its `read_bytes` is
//!   `readInternal`, its `seek` is `seekInternal` (as
//!   `core_store_tests.rs`'s `MyBufferedIndexInput`).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::{AfterClass, Assert, BeforeClass};
use rucene::document::{Document, DocumentStoredFieldVisitor};
use rucene::error::{LuceneError, Result};
use rucene::index::directory_reader;
use rucene::index::field_infos::{FieldInfo, FieldInfosBuilder, FieldNumbers};
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::OpenMode;
use rucene::index::{IndexOptions, IndexReader, VectorEncoding, VectorSimilarityFunction};
use rucene::store::{
    BufferedChecksumIndexInput, BufferedIndexInput, DataInput, Directory, IOContext, IndexInput,
    IndexOutput, Lock, BUFFER_SIZE,
};
use rucene::util::java_collections::{JavaHashMap, JavaStringMap};
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::index::doc_helper::{self, DocHelper};
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_equals, class, random, test};
use crate::support::{npe, static_hook};

/// `TestFieldsReader`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestFieldsReader",
        vec![
            static_hook("beforeClass", BeforeClass, before_class),
            static_hook("afterClass", AfterClass, after_class),
            test("test", test_),
            test("testExceptions", test_exceptions),
        ],
    )
}

/// The static fields `dir` and `fieldInfos`.
struct Statics {
    dir: Option<TestDirectory>,
    field_infos: Option<FieldInfosBuilder>,
}

static STATICS: Mutex<Statics> = Mutex::new(Statics {
    dir: None,
    field_infos: None,
});

fn statics() -> std::sync::MutexGuard<'static, Statics> {
    STATICS.lock().unwrap_or_else(|e| e.into_inner())
}

/// `testDoc`: `new Document()` filled by `DocHelper.setupDoc`.
fn test_doc() -> Document {
    let mut test_doc = Document::new();
    DocHelper::setup_doc(&mut test_doc);
    test_doc
}

/// `@BeforeClass public static void beforeClass()`.
fn before_class() -> Result<()> {
    let test_doc = test_doc();
    let mut field_infos = FieldInfosBuilder::new(Arc::new(FieldNumbers::new(None, None)?));
    for field in test_doc.get_fields() {
        let ift = field.field_type();
        field_infos.add(&FieldInfo::new_full(
            field.name(),
            -1,
            false,
            ift.omit_norms(),
            false,
            ift.index_options(),
            ift.doc_values_type(),
            ift.doc_values_skip_index_type(),
            -1,
            JavaStringMap::Hash(JavaHashMap::new()),
            0,
            0,
            0,
            0,
            VectorEncoding::FLOAT32,
            VectorSimilarityFunction::EUCLIDEAN,
            false,
            false,
        )?)?;
    }
    let dir = LuceneTestCase::new_directory();
    let conf = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(MockAnalyzer::new(
        &*random(),
    ))));
    conf.set_merge_policy(Some(Arc::new(LuceneTestCase::new_log_merge_policy())))?;
    conf.merge_policy().set_no_cfs_ratio(0.0)?;
    let writer = IndexWriter::new(dir.as_directory(), conf)?;
    writer.add_document(&test_doc)?;
    writer.close()?;
    let mut s = statics();
    s.field_infos = Some(field_infos);
    s.dir = Some(dir);
    Ok(())
}

/// `@AfterClass public static void afterClass()`.
fn after_class() -> Result<()> {
    let mut s = statics();
    let mut dir = s.dir.take().unwrap_or_else(|| npe("dir"));
    dir.close()?;
    s.field_infos = None;
    Ok(())
}

/// Port of `TestFieldsReader.test`.
fn test_() -> Result<()> {
    let dir = {
        let s = statics();
        Assert::assert_true(s.dir.is_some());
        Assert::assert_true(s.field_infos.is_some());
        s.dir.as_ref().unwrap_or_else(|| npe("dir")).as_directory()
    };
    let reader = directory_reader::open(dir)?;
    let doc = reader.stored_fields()?.document(0)?;
    Assert::assert_true(doc.get_field(doc_helper::TEXT_FIELD_1_KEY).is_some());

    let field = doc.get_field(doc_helper::TEXT_FIELD_2_KEY);
    Assert::assert_true(field.is_some());
    let field = field.unwrap_or_else(|| npe("field"));
    Assert::assert_true(field.field_type().store_term_vectors());

    Assert::assert_false(field.field_type().omit_norms());
    Assert::assert_true(
        field.field_type().index_options() == IndexOptions::DOCS_AND_FREQS_AND_POSITIONS,
    );

    let field = doc.get_field(doc_helper::TEXT_FIELD_3_KEY);
    Assert::assert_true(field.is_some());
    let field = field.unwrap_or_else(|| npe("field"));
    Assert::assert_false(field.field_type().store_term_vectors());
    Assert::assert_true(field.field_type().omit_norms());
    Assert::assert_true(
        field.field_type().index_options() == IndexOptions::DOCS_AND_FREQS_AND_POSITIONS,
    );

    let field = doc.get_field(doc_helper::NO_TF_KEY);
    Assert::assert_true(field.is_some());
    let field = field.unwrap_or_else(|| npe("field"));
    Assert::assert_false(field.field_type().store_term_vectors());
    Assert::assert_false(field.field_type().omit_norms());
    Assert::assert_true(field.field_type().index_options() == IndexOptions::DOCS);

    let mut visitor = DocumentStoredFieldVisitor::with_fields([doc_helper::TEXT_FIELD_3_KEY]);
    reader
        .stored_fields()?
        .document_with_visitor(0, &mut visitor)?;
    let fields = visitor.document().get_fields();
    assert_equals(&1, &fields.len());
    assert_equals(doc_helper::TEXT_FIELD_3_KEY, fields[0].name());
    reader.close()
}

/// `public class FaultyFSDirectory extends FilterDirectory`.
struct FaultyFSDirectory {
    /// The `fsDir` the test opened, which `close()` closes.
    fs_dir: Mutex<Option<TestDirectory>>,
    r#in: Arc<dyn Directory>,
    do_fail: Arc<AtomicBool>,
}

impl FaultyFSDirectory {
    fn new(fs_dir: TestDirectory) -> Self {
        let r#in = fs_dir.as_directory();
        FaultyFSDirectory {
            fs_dir: Mutex::new(Some(fs_dir)),
            r#in,
            do_fail: Arc::new(AtomicBool::new(false)),
        }
    }

    fn start_failing(&self) {
        self.do_fail.store(true, Ordering::SeqCst);
    }

    /// `close()`: `in.close()`, closing the `fsDir` the test opened.
    fn close_in(&self) -> Result<()> {
        match self.fs_dir.lock().unwrap_or_else(|e| e.into_inner()).take() {
            Some(mut fs_dir) => fs_dir.close(),
            None => Ok(()),
        }
    }
}

impl Directory for FaultyFSDirectory {
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
        self.r#in.create_output(name, cxt)
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
        faulty_index_input(
            Arc::clone(&self.do_fail),
            self.r#in.open_input(name, context)?,
        )
    }
    fn open_checksum_input(&self, name: &str) -> Result<Box<BufferedChecksumIndexInput>> {
        self.r#in.open_checksum_input(name)
    }
    fn obtain_lock(&self, name: &str) -> Result<Box<dyn Lock>> {
        self.r#in.obtain_lock(name)
    }
    fn close(&mut self) -> Result<()> {
        self.close_in()
    }
    fn get_pending_deletions(&self) -> Result<std::collections::HashSet<String>> {
        self.r#in.get_pending_deletions()
    }
    fn as_filter_directory(&self) -> Option<&dyn Directory> {
        Some(self.r#in.as_ref())
    }
}

/// `new FaultyIndexInput(doFail, delegate)`: the `BufferedIndexInput`
/// (`BufferedIndexInput.BUFFER_SIZE`) over the faulty source.
fn faulty_index_input(
    do_fail: Arc<AtomicBool>,
    delegate: Box<dyn IndexInput>,
) -> Result<Box<dyn IndexInput>> {
    let description = format!("FaultyIndexInput({})", delegate.resource_description());
    Ok(Box::new(BufferedIndexInput::new(
        Box::new(FaultyIndexInput {
            do_fail,
            delegate,
            count: 0,
            pos: 0,
            description,
        }),
        BUFFER_SIZE,
    )?))
}

/// `private class FaultyIndexInput extends BufferedIndexInput`: the source
/// its `BufferedIndexInput` reads.
struct FaultyIndexInput {
    do_fail: Arc<AtomicBool>,
    delegate: Box<dyn IndexInput>,
    count: i32,
    /// The `getFilePointer()` `readInternal` seeks the delegate to.
    pos: i64,
    description: String,
}

impl FaultyIndexInput {
    fn sim_outage(&mut self) -> Result<()> {
        let count = self.count;
        if self.do_fail.load(Ordering::SeqCst) && {
            self.count += 1;
            count % 2 == 1
        } {
            return Err(LuceneError::Io(std::io::Error::other(
                "Simulated network outage",
            )));
        }
        Ok(())
    }
}

impl DataInput for FaultyIndexInput {
    fn to_java_string(&self) -> String {
        self.description.clone()
    }

    fn read_byte(&mut self) -> Result<u8> {
        let mut b = [0u8; 1];
        self.read_bytes(Some(&mut b), 0, 1)?;
        Ok(b[0])
    }

    /// `readInternal(ByteBuffer b)`.
    fn read_bytes(&mut self, b: Option<&mut [u8]>, offset: usize, len: usize) -> Result<()> {
        self.sim_outage()?;
        self.delegate.seek(self.pos)?;
        self.delegate.read_bytes(b, offset, len)?;
        self.pos += len as i64;
        Ok(())
    }

    fn skip_bytes(&mut self, num_bytes: i64) -> Result<()> {
        self.pos += num_bytes;
        Ok(())
    }
}

impl IndexInput for FaultyIndexInput {
    fn close(&mut self) -> Result<()> {
        self.delegate.close()
    }

    fn file_pointer(&self) -> i64 {
        self.pos
    }

    fn length(&self) -> i64 {
        self.delegate.length()
    }

    /// `seekInternal(long pos) {}`: the position `readInternal` reads from.
    fn seek(&mut self, pos: i64) -> Result<()> {
        self.pos = pos;
        Ok(())
    }

    fn slice(
        &self,
        slice_description: &str,
        offset: i64,
        length: i64,
    ) -> Result<Box<dyn IndexInput>> {
        let slice = self.delegate.slice(slice_description, offset, length)?;
        faulty_index_input(Arc::clone(&self.do_fail), slice)
    }

    fn clone_input(&self) -> Result<Box<dyn IndexInput>> {
        Ok(Box::new(FaultyIndexInput {
            do_fail: Arc::clone(&self.do_fail),
            delegate: self.delegate.clone_input()?,
            count: 0,
            // seek the clone to our current position
            pos: self.pos,
            description: format!("FaultyIndexInput({})", self.delegate.resource_description()),
        }))
    }

    fn resource_description(&self) -> &str {
        &self.description
    }
}

/// LUCENE-1262
/// Port of `TestFieldsReader.testExceptions`.
fn test_exceptions() -> Result<()> {
    let index_dir = LuceneTestCase::create_temp_dir_prefix("testfieldswriterexceptions");

    let fs_dir = LuceneTestCase::new_fs_directory(&index_dir);
    let dir = Arc::new(FaultyFSDirectory::new(fs_dir));
    let iwc = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(MockAnalyzer::new(
        &*random(),
    ))));
    iwc.set_open_mode(OpenMode::CREATE)?;
    let writer = IndexWriter::new(Arc::clone(&dir) as Arc<dyn Directory>, iwc)?;
    let test_doc = test_doc();
    for _ in 0..2 {
        writer.add_document(&test_doc)?;
    }
    writer.force_merge(1)?;
    writer.close()?;
    drop(writer);

    let reader = directory_reader::open(Arc::clone(&dir) as Arc<dyn Directory>)?;
    dir.start_failing();

    let mut exc = false;

    let stored_fields = reader.stored_fields()?;
    for i in 0..2 {
        match stored_fields.document(i) {
            Ok(_) => {}
            // expected
            Err(e)
                if rucene_test_framework::util::java_exceptions::is_instance(
                    &e,
                    "java.io.IOException",
                ) =>
            {
                exc = true
            }
            Err(e) => return Err(e),
        }
        match stored_fields.document(i) {
            Ok(_) => {}
            // expected
            Err(e)
                if rucene_test_framework::util::java_exceptions::is_instance(
                    &e,
                    "java.io.IOException",
                ) =>
            {
                exc = true
            }
            Err(e) => return Err(e),
        }
    }
    Assert::assert_true(exc);
    drop(stored_fields);
    (reader as Arc<dyn IndexReader>).close()?;
    dir.close_in()
}
