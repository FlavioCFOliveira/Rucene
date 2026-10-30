//! Sorting flush consumers ported from `org.apache.lucene.index`.
//!
//! Covers `SortingStoredFieldsConsumer` and `SortingTermVectorsConsumer`: the
//! flush-time consumers that write a segment's stored fields and term vectors in
//! the index sort order rather than the order the documents arrived in.
//!
//! Both follow Lucene's design. While documents are indexed, the wrapped
//! consumer writes them, in arrival order, through a temporary format with no
//! compression (`TempStoredFields`, `TempTermVectors`), into temporary files of
//! the segment's directory created through a
//! [`TrackingTmpOutputDirectoryWrapper`]. At flush, the temporary files are
//! read back document by document in the sorted order —
//! `sortMap.newToOld(docID)` — and copied into the segment's real writer; then
//! they are deleted, as they are on abort.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use crate::codecs::lucene90::stored_fields::Lucene90CompressingStoredFieldsFormat;
use crate::codecs::lucene90::term_vectors::Lucene90CompressingTermVectorsFormat;
use crate::codecs::stored_fields::{MergeVisitor, StoredFieldsFormat};
use crate::codecs::term_vectors::{TermVectorsFormat, TermVectorsWriter};
use crate::codecs::{Codec, CompressionMode};
use crate::document::StoredValue;
use crate::error::{LuceneError, Result};
use crate::index::index_sorter::SorterDocMap;
use crate::index::postings_enum::{POSTINGS_ENUM_OFFSETS, POSTINGS_ENUM_PAYLOADS};
use crate::index::segment_info::SegmentInfo;
use crate::index::stored_fields_consumer::StoredFieldsConsumer;
use crate::index::term_vectors_consumer::TermVectorsConsumer;
use crate::index::{FieldInfo, FieldInfos, Fields};
use crate::search::{DocIdSetIterator, NO_MORE_DOCS};
use crate::store::{Directory, IOContext, IndexInput, IndexOutput, Lock, DEFAULT_IO_CONTEXT};
use crate::util::BytesRef;

/// A directory that turns every output it is asked to create into a temporary
/// output of the wrapped directory, and remembers the temporary name each
/// requested name was given.
///
/// Equivalent to `org.apache.lucene.index.TrackingTmpOutputDirectoryWrapper`
/// (`TrackingTmpOutputDirectoryWrapper.java:29-53`), a `FilterDirectory`:
/// `createOutput(name)` is `createTempOutput(name, "", context)`, and
/// `openInput(name)` opens the temporary file `name` was mapped to, or `name`
/// itself when it was not. Every other call is passed through. Java wraps the
/// `Directory` the consumer holds; this port's consumers share theirs through
/// an `Arc`, so the wrapper delegates to it directly rather than through
/// [`FilterDirectory`](crate::store::FilterDirectory), which owns a `Box`.
pub(crate) struct TrackingTmpOutputDirectoryWrapper {
    inner: Arc<dyn Directory>,
    /// `fileNames`: requested name to temporary name.
    file_names: Mutex<HashMap<String, String>>,
}

impl TrackingTmpOutputDirectoryWrapper {
    /// Equivalent to `TrackingTmpOutputDirectoryWrapper(Directory)`.
    pub(crate) fn new(inner: Arc<dyn Directory>) -> Self {
        Self {
            inner,
            file_names: Mutex::new(HashMap::new()),
        }
    }

    /// The temporary files created so far.
    ///
    /// Equivalent to `getTemporaryFiles().values()`.
    pub(crate) fn temporary_files(&self) -> Vec<String> {
        self.file_names
            .lock()
            .expect("temporary file names mutex poisoned")
            .values()
            .cloned()
            .collect()
    }

    /// Deletes every temporary file, stopping at the first failure.
    ///
    /// Equivalent to `IOUtils.deleteFiles(tmpDirectory,
    /// tmpDirectory.getTemporaryFiles().values())`.
    fn delete_temporary_files(&self) -> Result<()> {
        for name in self.temporary_files() {
            self.delete_file(&name)?;
        }
        Ok(())
    }

    /// Deletes every temporary file, ignoring failures.
    ///
    /// Equivalent to `IOUtils.deleteFilesIgnoringExceptions(tmpDirectory,
    /// tmpDirectory.getTemporaryFiles().values())`.
    fn delete_temporary_files_ignoring_errors(&self) {
        for name in self.temporary_files() {
            let _ = self.delete_file(&name);
        }
    }
}

impl std::fmt::Debug for TrackingTmpOutputDirectoryWrapper {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TrackingTmpOutputDirectoryWrapper")
            .field("file_names", &self.file_names)
            .finish_non_exhaustive()
    }
}

impl Directory for TrackingTmpOutputDirectoryWrapper {
    fn list_all(&self) -> Result<Vec<String>> {
        self.inner.list_all()
    }

    fn delete_file(&self, name: &str) -> Result<()> {
        self.inner.delete_file(name)
    }

    fn file_length(&self, name: &str) -> Result<i64> {
        self.inner.file_length(name)
    }

    /// `super.createTempOutput(name, "", context)`, recording the temporary
    /// file's name under `name`.
    fn create_output(&self, name: &str, context: &dyn IOContext) -> Result<Box<dyn IndexOutput>> {
        let output = self.inner.create_temp_output(name, "", context)?;
        self.file_names
            .lock()
            .expect("temporary file names mutex poisoned")
            .insert(name.to_string(), output.name().to_string());
        Ok(output)
    }

    fn create_temp_output(
        &self,
        prefix: &str,
        suffix: &str,
        context: &dyn IOContext,
    ) -> Result<Box<dyn IndexOutput>> {
        self.inner.create_temp_output(prefix, suffix, context)
    }

    fn sync(&self, names: &[String]) -> Result<()> {
        self.inner.sync(names)
    }

    fn sync_metadata(&self) -> Result<()> {
        self.inner.sync_metadata()
    }

    fn rename(&self, source: &str, dest: &str) -> Result<()> {
        self.inner.rename(source, dest)
    }

    /// Opens the temporary file `name` was mapped to, keeping `name` when there
    /// is no mapping, since it might be a temporary file already.
    fn open_input(&self, name: &str, context: &dyn IOContext) -> Result<Box<dyn IndexInput>> {
        let temporary = self
            .file_names
            .lock()
            .expect("temporary file names mutex poisoned")
            .get(name)
            .cloned();
        self.inner
            .open_input(temporary.as_deref().unwrap_or(name), context)
    }

    fn obtain_lock(&self, name: &str) -> Result<Box<dyn Lock>> {
        self.inner.obtain_lock(name)
    }

    /// The wrapped directory belongs to the segment, which closes it; the
    /// wrapper holds it through a shared handle and has nothing of its own to
    /// close.
    fn close(&mut self) -> Result<()> {
        Ok(())
    }

    fn get_pending_deletions(&self) -> Result<HashSet<String>> {
        self.inner.get_pending_deletions()
    }

    fn ensure_open(&self) -> Result<()> {
        self.inner.ensure_open()
    }
}

/// A compression mode that does no compression at all.
///
/// Equivalent to `SortingStoredFieldsConsumer.NO_COMPRESSION`
/// (`SortingStoredFieldsConsumer.java:42-79`), which
/// `SortingTermVectorsConsumer` uses too.
static NO_COMPRESSION: std::sync::LazyLock<Arc<dyn CompressionMode>> =
    std::sync::LazyLock::new(|| Arc::new(crate::codecs::compressing::NoCompressionMode));

/// The format the stored fields of an index-sorted segment are buffered in.
///
/// Equivalent to `SortingStoredFieldsConsumer.TEMP_STORED_FIELDS_FORMAT`
/// (`SortingStoredFieldsConsumer.java:79-81`): a compressing format named
/// `TempStoredFields`, with no compression, 128 KB chunks, one document per
/// chunk and a block shift of 10.
fn temp_stored_fields_format() -> Result<Lucene90CompressingStoredFieldsFormat> {
    Lucene90CompressingStoredFieldsFormat::new(
        "TempStoredFields",
        Arc::clone(&NO_COMPRESSION),
        128 * 1024,
        1,
        10,
    )
}

/// The format the term vectors of an index-sorted segment are buffered in.
///
/// Equivalent to `SortingTermVectorsConsumer.TEMP_TERM_VECTORS_FORMAT`
/// (`SortingTermVectorsConsumer.java:39-41`).
fn temp_term_vectors_format() -> Result<Lucene90CompressingTermVectorsFormat> {
    Lucene90CompressingTermVectorsFormat::new(
        "TempTermVectors",
        "",
        Arc::clone(&NO_COMPRESSION),
        8 * 1024,
        128,
        10,
    )
}

/// Writes stored fields in the index sort order.
///
/// Equivalent to `org.apache.lucene.index.SortingStoredFieldsConsumer`
/// (`SortingStoredFieldsConsumer.java:40-203`), which
/// `IndexingChain` picks over `StoredFieldsConsumer` when the segment has an
/// index sort (`IndexingChain.java:136-156`). Java's subclass overrides
/// `initStoredFieldsWriter` to write through the temporary format into a
/// [`TrackingTmpOutputDirectoryWrapper`] over the segment's directory
/// (`SortingStoredFieldsConsumer.java:95-100`); this port gives the wrapped
/// [`StoredFieldsConsumer`] that format and that directory, and every
/// per-document call goes to it unchanged.
pub struct SortingStoredFieldsConsumer {
    inner: StoredFieldsConsumer,
    codec: Arc<dyn Codec>,
    /// The segment's directory, which the sorted copy is written to.
    directory: Arc<dyn Directory>,
    /// Java's `tmpDirectory`. Java creates it with the temporary writer; it
    /// creates no file until then, so the port creates it with the consumer.
    tmp_directory: Arc<TrackingTmpOutputDirectoryWrapper>,
    /// Whether any stored field value was written; when none was, the flush
    /// skips reading the temporary files back.
    has_stored_fields: bool,
}

impl std::fmt::Debug for SortingStoredFieldsConsumer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SortingStoredFieldsConsumer")
            .field("inner", &self.inner)
            .field("has_stored_fields", &self.has_stored_fields)
            .finish_non_exhaustive()
    }
}

impl SortingStoredFieldsConsumer {
    /// Creates the consumer of the segment `info`, written into `directory`.
    ///
    /// Equivalent to `SortingStoredFieldsConsumer(Codec, Directory,
    /// SegmentInfo)`.
    ///
    /// # Errors
    ///
    /// Propagates the construction error of the temporary format, which its
    /// constant parameters never trigger.
    pub fn new(
        codec: Arc<dyn Codec>,
        directory: Arc<dyn Directory>,
        info: SegmentInfo,
    ) -> Result<Self> {
        let tmp_directory = Arc::new(TrackingTmpOutputDirectoryWrapper::new(Arc::clone(
            &directory,
        )));
        let inner = StoredFieldsConsumer::with_format(
            Arc::clone(&codec),
            Arc::new(temp_stored_fields_format()?),
            Arc::clone(&tmp_directory) as Arc<dyn Directory>,
            info,
        );
        Ok(Self {
            inner,
            codec,
            directory,
            tmp_directory,
            has_stored_fields: false,
        })
    }

    /// Returns the wrapped consumer.
    pub fn get_delegate(&self) -> &StoredFieldsConsumer {
        &self.inner
    }

    /// Returns the wrapped consumer, mutably.
    pub fn get_delegate_mut(&mut self) -> &mut StoredFieldsConsumer {
        &mut self.inner
    }

    /// Writes one stored field of the current document, and records that the
    /// segment has stored fields.
    ///
    /// Equivalent to `SortingStoredFieldsConsumer.writeField(FieldInfo,
    /// StoredValue)`.
    ///
    /// # Errors
    ///
    /// Propagates whatever the wrapped consumer raises.
    pub fn write_field(&mut self, info: &FieldInfo, value: StoredValue) -> Result<()> {
        self.inner.write_field(info, value)?;
        self.has_stored_fields = true;
        Ok(())
    }

    /// Writes the buffered stored fields into the segment in the sorted order.
    ///
    /// Equivalent to `SortingStoredFieldsConsumer.flush(SegmentWriteState,
    /// Sorter.DocMap)` (`SortingStoredFieldsConsumer.java:109-139`): the
    /// temporary writer is finished and closed, the temporary files are read
    /// back, and every document — `sortMap.newToOld(docID)`, or `docID` when
    /// the segment needed no reordering — is copied into the codec's writer,
    /// opened with the flush `context`. An empty document is still started
    /// and finished, so the numbering is kept. The reader and the writer are
    /// closed and the temporary files deleted whatever happened.
    ///
    /// # Errors
    ///
    /// Propagates whatever finishing the temporary writer, reading it back or
    /// writing the sorted copy raises.
    pub fn flush(
        &mut self,
        segment_info: &SegmentInfo,
        field_infos: &FieldInfos,
        context: &dyn IOContext,
        sort_map: Option<&SorterDocMap>,
    ) -> Result<()> {
        self.inner.flush(segment_info)?;
        let max_doc = segment_info.max_doc()?;
        let opened = (|| -> Result<_> {
            let reader = temp_stored_fields_format()?.fields_reader(
                self.tmp_directory.as_ref(),
                segment_info,
                field_infos,
                &*DEFAULT_IO_CONTEXT,
            )?;
            // Don't pull a merge instance, since merge instances optimize for
            // sequential access while stored fields are consumed in random
            // order here.
            let sort_writer = self.codec.stored_fields_format().fields_writer(
                &self.directory,
                segment_info,
                context,
            )?;
            Ok((reader, sort_writer))
        })();
        let (reader, mut sort_writer) = match opened {
            Ok(opened) => opened,
            Err(error) => {
                let _ = self.tmp_directory.delete_temporary_files();
                return Err(error);
            }
        };
        // Java does not run `reader.checkIntegrity()` here, so as not to read
        // the temporary files twice.
        let copied = (|| -> Result<()> {
            for doc_id in 0..max_doc {
                sort_writer.start_document()?;
                // Skip per-document stored-field reads when no stored field
                // was written.
                if self.has_stored_fields {
                    let old_doc_id = sort_map.map_or(doc_id, |map| map.new_to_old(doc_id));
                    let mut visitor = MergeVisitor {
                        writer: sort_writer.as_mut(),
                        remapper: None,
                    };
                    reader.document(old_doc_id, &mut visitor)?;
                }
                sort_writer.finish_document()?;
            }
            sort_writer.finish(max_doc)
        })();
        // `IOUtils.close(reader, sortWriter)`, then
        // `IOUtils.deleteFiles(tmpDirectory, ...)`.
        drop(reader);
        let closed = sort_writer.close();
        let deleted = self.tmp_directory.delete_temporary_files();
        copied.and(closed).and(deleted)
    }

    /// Discards everything buffered.
    ///
    /// Equivalent to `SortingStoredFieldsConsumer.abort()`
    /// (`SortingStoredFieldsConsumer.java:142-151`): the plain consumer
    /// aborts, then the temporary files are deleted, ignoring failures.
    pub fn abort(&mut self) {
        self.inner.abort();
        self.tmp_directory.delete_temporary_files_ignoring_errors();
    }
}

/// Writes term vectors in the index sort order.
///
/// Equivalent to `org.apache.lucene.index.SortingTermVectorsConsumer`
/// (`SortingTermVectorsConsumer.java:37-207`), which `IndexingChain` picks over
/// `TermVectorsConsumer` when the segment has an index sort. Java's subclass
/// overrides `initTermVectorsWriter` to write through the temporary format
/// into a [`TrackingTmpOutputDirectoryWrapper`] over the segment's directory
/// (`SortingTermVectorsConsumer.java:87-94`); this port gives the wrapped
/// [`TermVectorsConsumer`] that format and that directory, and every
/// per-document call goes to it unchanged.
pub struct SortingTermVectorsConsumer {
    inner: TermVectorsConsumer,
    codec: Arc<dyn Codec>,
    /// The segment's directory, which the sorted copy is written to.
    directory: Arc<dyn Directory>,
    /// Java's `tmpDirectory`, created here rather than with the temporary
    /// writer; whether that writer exists is what Java's
    /// `tmpDirectory != null` tests.
    tmp_directory: Arc<TrackingTmpOutputDirectoryWrapper>,
}

impl std::fmt::Debug for SortingTermVectorsConsumer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SortingTermVectorsConsumer")
            .field("inner", &self.inner)
            .finish_non_exhaustive()
    }
}

impl SortingTermVectorsConsumer {
    /// Creates the consumer of the segment `info`, written into `directory`.
    ///
    /// Equivalent to `SortingTermVectorsConsumer(IntBlockPool.Allocator,
    /// ByteBlockPool.Allocator, Directory, SegmentInfo, Codec)`.
    ///
    /// # Errors
    ///
    /// Propagates the construction error of the temporary format, which its
    /// constant parameters never trigger.
    pub fn new(
        codec: Arc<dyn Codec>,
        directory: Arc<dyn Directory>,
        info: SegmentInfo,
        bytes_used: Arc<std::sync::atomic::AtomicI64>,
    ) -> Result<Self> {
        let tmp_directory = Arc::new(TrackingTmpOutputDirectoryWrapper::new(Arc::clone(
            &directory,
        )));
        let inner = TermVectorsConsumer::with_format(
            Arc::clone(&codec),
            Arc::new(temp_term_vectors_format()?),
            Arc::clone(&tmp_directory) as Arc<dyn Directory>,
            info,
            bytes_used,
        );
        Ok(Self {
            inner,
            codec,
            directory,
            tmp_directory,
        })
    }

    /// Returns the wrapped consumer.
    pub fn get_delegate(&self) -> &TermVectorsConsumer {
        &self.inner
    }

    /// Returns the wrapped consumer, mutably.
    pub fn get_delegate_mut(&mut self) -> &mut TermVectorsConsumer {
        &mut self.inner
    }

    /// Writes the buffered term vectors into the segment in the sorted order.
    ///
    /// Equivalent to `SortingTermVectorsConsumer.flush(Map, SegmentWriteState,
    /// Sorter.DocMap, NormsProducer)` (`SortingTermVectorsConsumer.java:54-84`):
    /// the temporary writer is filled, finished and closed; when any document
    /// had vectors — Java's `tmpDirectory != null` — the temporary files are
    /// read back, checked, and every document's vectors —
    /// `sortMap.newToOld(docID)` — are copied into the codec's writer, opened
    /// with the flush `context`, through
    /// [`write_term_vectors`](Self::write_term_vectors). The reader and the
    /// writer are closed and the temporary files deleted whatever happened.
    ///
    /// # Errors
    ///
    /// Propagates whatever finishing the temporary writer, reading it back or
    /// writing the sorted copy raises.
    pub fn flush(
        &mut self,
        segment_info: &SegmentInfo,
        field_infos: &FieldInfos,
        context: &dyn IOContext,
        sort_map: Option<&SorterDocMap>,
    ) -> Result<()> {
        let max_doc = segment_info.max_doc()?;
        let had_vectors = self.inner.has_writer();
        self.inner.flush(max_doc)?;
        if !had_vectors {
            return Ok(());
        }
        let opened = (|| -> Result<_> {
            let reader = temp_term_vectors_format()?.vectors_reader(
                self.tmp_directory.as_ref(),
                segment_info,
                field_infos,
                &*DEFAULT_IO_CONTEXT,
            )?;
            // Don't pull a merge instance, since merge instances optimize for
            // sequential access while term vectors will likely be accessed in
            // random order here.
            let writer = self.codec.term_vectors_format().vectors_writer(
                &self.directory,
                segment_info,
                context,
            )?;
            Ok((reader, writer))
        })();
        let (reader, mut writer) = match opened {
            Ok(opened) => opened,
            Err(error) => {
                let _ = self.tmp_directory.delete_temporary_files();
                return Err(error);
            }
        };
        let copied = (|| -> Result<()> {
            reader.check_integrity()?;
            for doc_id in 0..max_doc {
                let old_doc_id = sort_map.map_or(doc_id, |map| map.new_to_old(doc_id));
                let vectors = reader.get(old_doc_id)?;
                Self::write_term_vectors(writer.as_mut(), vectors.as_deref(), field_infos)?;
            }
            writer.finish(max_doc)
        })();
        // `IOUtils.close(reader, writer)`, then
        // `IOUtils.deleteFiles(tmpDirectory, ...)`.
        drop(reader);
        let closed = writer.close();
        let deleted = self.tmp_directory.delete_temporary_files();
        copied.and(closed).and(deleted)
    }

    /// Discards everything buffered.
    ///
    /// Equivalent to `SortingTermVectorsConsumer.abort()`
    /// (`SortingTermVectorsConsumer.java:97-106`): the plain consumer aborts,
    /// then the temporary files are deleted, ignoring failures.
    pub fn abort(&mut self) {
        self.inner.abort();
        self.tmp_directory.delete_temporary_files_ignoring_errors();
    }

    /// Copies every vector field of one document from `vectors` into `writer`.
    ///
    /// This is the safe — but slow — default way to move a document's term
    /// vectors from one writer to another: the fields are walked in order, and
    /// for each one every term, its frequency, and, when the field has them, its
    /// positions, offsets and payloads are handed to the writer one at a time.
    /// A `None` `vectors` writes an empty document, which is how a document that
    /// carried no term vectors keeps its place in the numbering.
    ///
    /// Equivalent to the private static
    /// `SortingTermVectorsConsumer.writeTermVectors(TermVectorsWriter, Fields, FieldInfos)`.
    ///
    /// **Divergence from Lucene 10.5.0.** Java asserts that the field names
    /// arrive in increasing order, that a field with payloads also has
    /// positions, that the postings enum is positioned on a document whose
    /// frequency matches, that a position is non-negative, and that the term and
    /// field counts match what was announced. Those are `assert`s, so they are
    /// `debug_assert!`s here. Java also looks the field up in `field_infos`
    /// before every `startField`; a `Fields` view that names a field the field
    /// infos do not know cannot be described to the writer at all, so that case
    /// is an error here rather than a `NullPointerException`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] when `vectors` names a field that
    /// `field_infos` does not know, and propagates any error raised while
    /// reading the vectors or writing them.
    pub fn write_term_vectors(
        writer: &mut dyn TermVectorsWriter,
        vectors: Option<&dyn Fields>,
        field_infos: &FieldInfos,
    ) -> Result<()> {
        let Some(vectors) = vectors else {
            writer.start_document(0)?;
            writer.finish_document()?;
            return Ok(());
        };

        let mut num_fields = vectors.size();
        if num_fields == -1 {
            // Count manually: `Fields.size()` is not a mandatory statistic.
            num_fields = 0;
            for _ in vectors.iterator() {
                num_fields += 1;
            }
        }
        writer.start_document(num_fields)?;

        let mut last_field_name: Option<String> = None;
        let mut field_count = 0i32;

        for field_name in vectors.iterator() {
            field_count += 1;
            let field_info = field_infos.field_info(&field_name).ok_or_else(|| {
                LuceneError::illegal_state(format!(
                    "term vectors name field \"{field_name}\", which the field infos do not know"
                ))
            })?;

            debug_assert!(
                last_field_name
                    .as_ref()
                    .is_none_or(|last| field_name.as_str() > last.as_str()),
                "lastFieldName={last_field_name:?} fieldName={field_name}"
            );
            last_field_name = Some(field_name.clone());

            let Some(terms) = vectors.terms(&field_name)? else {
                // FieldsEnum shouldn't lie...
                continue;
            };

            let has_positions = terms.has_positions();
            let has_offsets = terms.has_offsets();
            let has_payloads = terms.has_payloads();
            debug_assert!(!has_payloads || has_positions);

            let mut num_terms = terms.size()? as i32;
            if num_terms == -1 {
                // Count manually. It is stupid, but needed, as `Terms.size()` is
                // not a mandatory statistics function.
                num_terms = 0;
                let mut terms_enum = terms.iterator()?;
                while terms_enum.next()?.is_some() {
                    num_terms += 1;
                }
            }

            writer.start_field(
                field_info,
                num_terms,
                has_positions,
                has_offsets,
                has_payloads,
            )?;
            let mut terms_enum = terms.iterator()?;

            let mut term_count = 0i32;
            while let Some(term) = terms_enum.next()? {
                term_count += 1;

                let freq = terms_enum.total_term_freq()? as i32;
                writer.start_term(&term, freq)?;

                if has_positions || has_offsets {
                    let mut docs_and_positions_enum = terms_enum
                        .postings(None, POSTINGS_ENUM_OFFSETS | POSTINGS_ENUM_PAYLOADS)?;

                    let doc_id = docs_and_positions_enum.next_doc()?;
                    debug_assert_ne!(doc_id, NO_MORE_DOCS);
                    debug_assert_eq!(docs_and_positions_enum.freq()?, freq);

                    for _ in 0..freq {
                        let pos = docs_and_positions_enum.next_position()?;
                        let start_offset = docs_and_positions_enum.start_offset();
                        let end_offset = docs_and_positions_enum.end_offset();
                        let payload = docs_and_positions_enum
                            .get_payload()?
                            .map(|bytes| BytesRef::new(bytes.to_vec()));

                        debug_assert!(!has_positions || pos >= 0);
                        writer.add_position(pos, start_offset, end_offset, payload.as_ref())?;
                    }
                }
                writer.finish_term()?;
            }
            debug_assert_eq!(term_count, num_terms);
            writer.finish_field()?;
        }
        debug_assert_eq!(field_count, num_fields);
        writer.finish_document()
    }
}
