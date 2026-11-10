//! The document limit `IndexWriter.setMaxDocs` lowers, compared with Apache
//! Lucene Core 10.5.0 (`TestIndexWriterMaxDocs`, `TestAddIndexes`).
//!
//! * Reading a commit checks the documents of its segments against
//!   `IndexWriter.getActualMaxDocs()` and fails with `CorruptIndexException`
//!   (`SegmentInfos.java:513-520`), when a reader or a writer opens the
//!   index (`TestIndexWriterMaxDocs.testCorruptIndexExceptionTooLarge` and
//!   `testCorruptIndexExceptionTooLargeWriter`).
//! * A near-real-time reader, which reads no commit, meets the same limit in
//!   the `BaseCompositeReader` constructor its `StandardDirectoryReader`
//!   runs (`BaseCompositeReader.java:90-109`).
//! * `addIndexes(CodecReader...)` refuses readers the index has no room for
//!   before it merges anything
//!   (`TestAddIndexes.testAddIndexesHittingMaxDocsLimit`).
//!
//! The limit is process-wide and these tests lower it far below what other
//! tests index, so they run in their own binary, one at a time, and put the
//! previous limit back through a drop guard even when an assertion fails.

use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use rucene::codecs::{register_codec, Lucene104Codec};
use rucene::document::{Document, Store, StringField};
use rucene::error::LuceneError;
use rucene::index::directory_reader::{self, DirectoryReader};
use rucene::index::filter_directory_reader::SlowCodecReaderWrapper;
use rucene::index::index_reader::IndexReader;
use rucene::index::index_writer::{get_actual_max_docs, set_max_docs, IndexWriter};
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::merge_policy::{MergeTrigger, NoMergePolicy};
use rucene::index::merge_scheduler::{MergeScheduler, MergeSource, SerialMergeScheduler};
use rucene::store::{ByteBuffersDirectory, Directory};
use rucene::util::SameThreadExecutorService;
use rucene::util::Version;

/// Serialises the tests of this binary, which all change the same limit.
static LIMIT: Mutex<()> = Mutex::new(());

/// Holds this binary's test lock and restores the document limit it found
/// when dropped, so a failing assertion cannot leave the limit lowered.
struct MaxDocsGuard {
    previous: i32,
    _serial: MutexGuard<'static, ()>,
}

impl MaxDocsGuard {
    fn acquire() -> Self {
        // A test that panicked while holding the lock has already restored
        // the limit through its own guard.
        let serial = LIMIT
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        Self {
            previous: get_actual_max_docs(),
            _serial: serial,
        }
    }

    /// `IndexWriter.setMaxDocs(limit)`, as `setIndexWriterMaxDocs` does.
    fn lower(&self, limit: i32) {
        set_max_docs(limit).unwrap();
    }

    /// `restoreIndexWriterMaxDocs()`.
    fn restore(&self) {
        set_max_docs(self.previous).unwrap();
    }
}

impl Drop for MaxDocsGuard {
    fn drop(&mut self) {
        let _ = set_max_docs(self.previous);
    }
}

fn ensure_codec() {
    let _ = register_codec("Lucene104", Lucene104Codec::new());
}

fn id_doc(id: &str) -> Document {
    let mut doc = Document::new();
    doc.add(Box::new(
        StringField::new("id", id.to_string(), Store::NO).unwrap(),
    ));
    doc
}

/// A directory with `count` committed documents.
fn index_with(count: usize) -> Arc<dyn Directory> {
    ensure_codec();
    let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
    let writer = IndexWriter::new(Arc::clone(&dir), IndexWriterConfig::new()).unwrap();
    for i in 0..count {
        writer.add_document(&id_doc(&i.to_string())).unwrap();
    }
    writer.close().unwrap();
    dir
}

fn corrupt_message(error: &LuceneError) -> &str {
    match error {
        LuceneError::CorruptIndex(message) => message,
        other => panic!("a CorruptIndexException, got {other:?}"),
    }
}

/// `TestIndexWriterMaxDocs.testCorruptIndexExceptionTooLarge`
/// (`TestIndexWriterMaxDocs.java:697-716`). Measured against
/// `lucene-core-10.5.0.jar` (`Probe2`): `CorruptIndexException: Too many
/// documents: an index cannot exceed 1 but readers have total maxDoc=2`.
#[test]
fn opening_a_reader_over_too_many_documents_is_corrupt() {
    let guard = MaxDocsGuard::acquire();
    let dir = index_with(2);

    guard.lower(1);
    let error = directory_reader::open(Arc::clone(&dir)).unwrap_err();
    assert!(
        corrupt_message(&error).starts_with(
            "Too many documents: an index cannot exceed 1 but readers have total maxDoc=2"
        ),
        "{error:?}"
    );
}

/// `TestIndexWriterMaxDocs.testCorruptIndexExceptionTooLargeWriter`
/// (`TestIndexWriterMaxDocs.java:719-738`): the writer reads the commit the
/// same way.
#[test]
fn opening_a_writer_over_too_many_documents_is_corrupt() {
    let guard = MaxDocsGuard::acquire();
    let dir = index_with(2);

    guard.lower(1);
    let error = IndexWriter::new(Arc::clone(&dir), IndexWriterConfig::new()).unwrap_err();
    assert!(
        corrupt_message(&error).starts_with(
            "Too many documents: an index cannot exceed 1 but readers have total maxDoc=2"
        ),
        "{error:?}"
    );
}

/// A near-real-time reader reads no commit; its `StandardDirectoryReader`
/// runs the `BaseCompositeReader` check, which names a directory reader's
/// segment readers as the corrupt resource (`BaseCompositeReader.java:90-99`).
/// Measured against `lucene-core-10.5.0.jar` (`Probe5`):
/// `CorruptIndexException: Too many documents: an index cannot exceed 1 but
/// readers have total maxDoc=2 (resource=[_0(10.5.0):c2])`.
#[test]
fn an_nrt_reader_over_too_many_documents_is_corrupt() {
    ensure_codec();
    let guard = MaxDocsGuard::acquire();
    let config = IndexWriterConfig::new();
    config
        .set_merge_policy(Some(Arc::new(NoMergePolicy::new())))
        .unwrap();
    let writer = IndexWriter::new(Arc::new(ByteBuffersDirectory::new()), config).unwrap();
    writer.add_document(&id_doc("a")).unwrap();
    writer.add_document(&id_doc("b")).unwrap();

    guard.lower(1);
    let error = directory_reader::open_with_writer(writer.as_ref()).unwrap_err();
    assert_eq!(
        corrupt_message(&error),
        format!(
            "Too many documents: an index cannot exceed 1 but readers have total maxDoc=2 \
             (resource=[_0({}):c2])",
            Version::LATEST
        )
    );
    guard.restore();
    writer.rollback().unwrap();
}

/// After `TestAddIndexes.CountingSerialMergeScheduler`
/// (`TestAddIndexes.java:855-879`): a serial scheduler that counts the
/// `addIndexes` merges it takes. Java's counts them once they ran; this one
/// counts them before they run, so a merge that fails is counted too.
#[derive(Default)]
struct CountingSerialMergeScheduler {
    serial: SerialMergeScheduler,
    add_indexes_merges_taken: AtomicI32,
}

impl std::fmt::Display for CountingSerialMergeScheduler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CountingSerialMergeScheduler")
    }
}

impl MergeScheduler for CountingSerialMergeScheduler {
    fn merge(
        &self,
        merge_source: Arc<dyn MergeSource>,
        trigger: MergeTrigger,
    ) -> rucene::error::Result<()> {
        while let Some(merge) = merge_source.get_next_merge()? {
            if trigger == MergeTrigger::AddIndexes {
                self.add_indexes_merges_taken.fetch_add(1, Ordering::SeqCst);
            }
            merge_source.merge(merge)?;
        }
        Ok(())
    }

    fn same_thread_executor(&self) -> &Arc<SameThreadExecutorService> {
        self.serial.same_thread_executor()
    }
}

/// `TestAddIndexes.testAddIndexesHittingMaxDocsLimit`
/// (`TestAddIndexes.java:925-969`): with room for 15 documents, all taken,
/// `addIndexes` of twenty 10-document readers fails up front
/// (`testReserveDocs`, `IndexWriter.java:3285`) and adds nothing.
/// Measured against `lucene-core-10.5.0.jar` (`Probe5`, one reader):
/// `IllegalArgumentException: number of documents in the index cannot exceed
/// 15 (current document count is 15; added numDocs is 10)`. The scheduler
/// takes no `addIndexes` merge: the check comes before the merges are
/// registered (`IndexWriter.java:3285`).
#[test]
fn add_indexes_codec_readers_refuses_readers_over_the_limit() {
    ensure_codec();
    let guard = MaxDocsGuard::acquire();
    let writer_max_docs = 15;
    guard.lower(writer_max_docs);

    // Create the destination writer.
    let dest_dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
    let scheduler = Arc::new(CountingSerialMergeScheduler::default());
    let config = IndexWriterConfig::new();
    config
        .set_merge_scheduler(Some(Arc::clone(&scheduler) as Arc<dyn MergeScheduler>))
        .unwrap();
    let dest_writer = IndexWriter::new(Arc::clone(&dest_dir), config).unwrap();
    for i in 0..writer_max_docs {
        dest_writer.add_document(&id_doc(&format!("d{i}"))).unwrap();
    }
    dest_writer.commit().unwrap();

    // Create the readers to add.
    let dir = index_with(10);
    let readers: Vec<Arc<dyn DirectoryReader>> = (0..20)
        .map(|_| directory_reader::open(Arc::clone(&dir)).unwrap())
        .collect();
    let leaves = readers
        .iter()
        .flat_map(|reader| Arc::clone(reader).leaves().unwrap())
        .map(|leaf| SlowCodecReaderWrapper::wrap(leaf.leaf_reader()).unwrap())
        .collect::<Vec<_>>();

    let error = dest_writer.add_indexes_codec_readers(&leaves).unwrap_err();
    match &error {
        LuceneError::IllegalArgument(message) => assert!(
            message.contains(&format!(
                "number of documents in the index cannot exceed {writer_max_docs}"
            )),
            "{message}"
        ),
        other => panic!("an IllegalArgumentException, got {other:?}"),
    }
    assert_eq!(scheduler.add_indexes_merges_taken.load(Ordering::SeqCst), 0);

    // Verify no docs were added.
    dest_writer.commit().unwrap();
    let reader = directory_reader::open(Arc::clone(&dest_dir)).unwrap();
    assert_eq!(reader.num_docs(), writer_max_docs);
    IndexReader::close(reader.as_ref()).unwrap();

    dest_writer.close().unwrap();
    drop(leaves);
    for reader in readers {
        IndexReader::close(reader.as_ref()).unwrap();
    }
}
