//! Port of `lucene/core/src/test/org/apache/lucene/index/TestSortingStoredFieldsConsumer.java`.
//!
//! The identity `Sorter.DocMap` is a `SorterDocMap` built from its values
//! (`SorterDocMap::from_old_to_new`), as the port declares for every
//! anonymous `Sorter.DocMap`; the `SegmentWriteState` the Java flush takes is
//! passed as its parts, as `SortingStoredFieldsConsumer::flush` declares.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::codecs::lucene90::stored_fields::FIELDS_EXTENSION;
use rucene::document::DocumentStoredFieldVisitor;
use rucene::error::Result;
use rucene::index::index_sorter::SorterDocMap;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::sorting_consumers::SortingStoredFieldsConsumer;
use rucene::index::{FieldInfos, SegmentInfo};
use rucene::store::{
    flush_io_context, BufferedChecksumIndexInput, DataInput, Directory, FilterIndexInput,
    FlushInfo, IOContext, IndexInput, IndexOutput, Lock, RandomAccessInput, READONCE_IO_CONTEXT,
};
use rucene::util::string_helper::StringHelper;
use rucene::util::Version;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::{assert_equals_msg, assert_not_null, class, random, test};

/// `TestSortingStoredFieldsConsumer`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestSortingStoredFieldsConsumer",
        vec![test(
            "testFlushWithNoStoredFieldsSkipsPerDocumentSeeks",
            test_flush_with_no_stored_fields_skips_per_document_seeks,
        )],
    )
}

/// The anonymous `FilterDirectory` of the test: its `openInput` wraps the
/// stored fields file in a seek-counting `FilterIndexInput`.
struct TrackingDirectory {
    r#in: Arc<dyn Directory>,
    num_seeks: Arc<AtomicI32>,
}

impl Directory for TrackingDirectory {
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
        if name.contains(FIELDS_EXTENSION) {
            return Ok(Box::new(SeekCountingInput {
                inner: FilterIndexInput::new(name, self.r#in.open_input(name, context)?),
                num_seeks: Arc::clone(&self.num_seeks),
            }));
        }
        self.r#in.open_input(name, context)
    }
    fn open_checksum_input(&self, name: &str) -> Result<Box<BufferedChecksumIndexInput>> {
        Ok(Box::new(BufferedChecksumIndexInput::new(
            self.open_input(name, &*READONCE_IO_CONTEXT)?,
        )))
    }
    fn obtain_lock(&self, name: &str) -> Result<Box<dyn Lock>> {
        self.r#in.obtain_lock(name)
    }
    fn close(&self) -> Result<()> {
        Ok(())
    }
    fn get_pending_deletions(&self) -> Result<HashSet<String>> {
        self.r#in.get_pending_deletions()
    }
    fn as_filter_directory(&self) -> Option<&dyn Directory> {
        Some(self.r#in.as_ref())
    }
}

/// The anonymous `FilterIndexInput` whose `seek` counts the calls.
struct SeekCountingInput {
    inner: FilterIndexInput,
    num_seeks: Arc<AtomicI32>,
}

impl DataInput for SeekCountingInput {
    fn to_java_string(&self) -> String {
        self.inner.to_java_string()
    }
    fn read_byte(&mut self) -> Result<u8> {
        self.inner.read_byte()
    }
    fn read_bytes(&mut self, b: Option<&mut [u8]>, offset: usize, len: usize) -> Result<()> {
        self.inner.read_bytes(b, offset, len)
    }
    fn read_bytes_buffered(
        &mut self,
        b: Option<&mut [u8]>,
        offset: usize,
        len: usize,
        use_buffer: bool,
    ) -> Result<()> {
        self.inner.read_bytes_buffered(b, offset, len, use_buffer)
    }
    fn skip_bytes(&mut self, num_bytes: i64) -> Result<()> {
        self.inner.skip_bytes(num_bytes)
    }
}

impl IndexInput for SeekCountingInput {
    fn close(&mut self) -> Result<()> {
        self.inner.close()
    }
    fn file_pointer(&self) -> Result<i64> {
        self.inner.file_pointer()
    }
    fn length(&self) -> i64 {
        self.inner.length()
    }
    fn seek(&mut self, pos: i64) -> Result<()> {
        self.num_seeks.fetch_add(1, Ordering::SeqCst);
        self.inner.seek(pos)
    }
    fn slice(
        &self,
        slice_description: &str,
        offset: i64,
        length: i64,
    ) -> Result<Box<dyn IndexInput>> {
        self.inner.slice(slice_description, offset, length)
    }
    fn clone_input(&self) -> Result<Box<dyn IndexInput>> {
        self.inner.clone_input()
    }
    fn resource_description(&self) -> &str {
        self.inner.resource_description()
    }
    fn prefetch(&self, offset: i64, length: i64) -> Result<()> {
        self.inner.prefetch(offset, length)
    }
    fn is_loaded(&self) -> Option<bool> {
        self.inner.is_loaded()
    }
    fn random_access_slice(&self, offset: i64, length: i64) -> Result<Box<dyn RandomAccessInput>> {
        self.inner.random_access_slice(offset, length)
    }
    fn as_filter_index_input(&self) -> Option<(std::any::TypeId, &dyn IndexInput)> {
        Some((
            std::any::TypeId::of::<SeekCountingInput>(),
            self.inner.get_delegate(),
        ))
    }
}

/// Verifies that when no stored fields are written, flushing the [`SortingStoredFieldsConsumer`]
/// skips per-document seeks and reads on the stored fields file. Only
/// the initial seek for checksum retrieval should occur.
///
/// Port of `TestSortingStoredFieldsConsumer.testFlushWithNoStoredFieldsSkipsPerDocumentSeeks`.
fn test_flush_with_no_stored_fields_skips_per_document_seeks() -> Result<()> {
    let dir_holder = LuceneTestCase::new_directory();
    let dir = dir_holder.as_directory();
    let iwc = IndexWriterConfig::new();
    let codec = iwc.codec();
    let mut si = SegmentInfo::new(
        Arc::clone(&dir),
        Version::LATEST,
        None,
        "_0".to_string(),
        -1,
        false,
        false,
        Arc::clone(&codec),
        HashMap::new(),
        StringHelper::random_id(),
        HashMap::new(),
        None,
    )?;

    let num_seeks = Arc::new(AtomicI32::new(0));
    let tracking_dir: Arc<dyn Directory> = Arc::new(TrackingDirectory {
        r#in: Arc::clone(&dir),
        num_seeks: Arc::clone(&num_seeks),
    });

    let mut consumer =
        SortingStoredFieldsConsumer::new(Arc::clone(&codec), tracking_dir, si.clone())?;

    let num_docs = TestUtil::next_int(&*random(), 1, 100);
    for i in 0..num_docs {
        consumer.get_delegate_mut().start_document(i)?;
        consumer.get_delegate_mut().finish_document()?;
    }
    consumer.get_delegate_mut().finish(num_docs)?;

    si.set_max_doc(num_docs)?;
    let field_infos = FieldInfos::new(Vec::new())?;
    let context = flush_io_context(FlushInfo::new(num_docs, 10));

    // Identity mapping: doc order is unchanged
    let identity_map = SorterDocMap::from_old_to_new((0..num_docs).collect())?;
    consumer.flush(&si, &field_infos, &*context, Some(&identity_map))?;

    {
        let mut reader = codec.stored_fields_format().fields_reader(
            dir.as_ref(),
            &si,
            &field_infos,
            &*READONCE_IO_CONTEXT,
        )?;
        for i in 0..num_docs {
            let mut visitor = DocumentStoredFieldVisitor::new();
            reader.document(i, &mut visitor)?;
            let document = Some(visitor.into_document());
            assert_not_null(&document);
            #[allow(clippy::unnecessary_literal_unwrap)]
            // Java's `assertNotNull` on the value, then its use
            Assert::assert_true(document.expect("checked").get_fields().is_empty());
        }
        reader.close()?;
    }

    assert_equals_msg(
        Some("Expected only 1 seek (for checksum retrieval), not one per document"),
        &1,
        &num_seeks.load(Ordering::SeqCst),
    );
    drop(dir);
    dir_holder.close()
}
