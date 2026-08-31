//! The stored-fields half of the per-segment indexing pipeline.
//!
//! | Rucene | Apache Lucene Core 10.5.0 |
//! | --- | --- |
//! | [`StoredFieldsConsumer`] | `org.apache.lucene.index.StoredFieldsConsumer` |
//!
//! # Macro objective
//!
//! Own the stored-fields stream of one segment. The consumer receives, in
//! document order, the values the [`IndexingChain`](crate::index::IndexingChain)
//! extracted from the stored fields of each document, and streams them through
//! the codec's [`StoredFieldsWriter`], producing the segment's `.fdt`, `.fdx`
//! and `.fdm` files.
//!
//! # Responsibility boundary
//!
//! The consumer owns, and only owns:
//!
//! * the lazy creation of the codec writer (no writer exists until a document
//!   frame is actually opened);
//! * per-document framing — exactly one `start_document`/`finish_document` pair
//!   per document of the segment;
//! * gap filling, so that a document that stored nothing still occupies its own
//!   empty frame and doc ids stay aligned with the stream;
//! * dispatching a [`StoredValue`] to the matching typed writer call;
//! * the flush and abort lifecycle of the writer.
//!
//! It deliberately does **not** own: which fields are stored (the indexing
//! chain decides, from the field type), value validation such as the maximum
//! stored string length (the indexing chain, mirroring
//! `IndexingChain.invertAndStore`), chunking, compression and the on-disk
//! layout (the codec writer), or the index-sorting variant (see
//! *Index sorting* below).
//!
//! # Lifecycle and invariants
//!
//! 1. `last_doc` starts at `-1` (`StoredFieldsConsumer.java:43`).
//! 2. [`start_document`](StoredFieldsConsumer::start_document) requires
//!    `last_doc < doc_id`: document frames are opened strictly in increasing
//!    doc-id order and a document is never revisited
//!    (`StoredFieldsConsumer.java:56`).
//! 3. [`start_document`](StoredFieldsConsumer::start_document) creates the
//!    writer on first use (`StoredFieldsConsumer.java:46-57`), then opens and
//!    immediately closes one empty frame for every doc id skipped since the
//!    last call, and finally opens the frame of `doc_id`
//!    (`StoredFieldsConsumer.java:58-62`). Afterwards `last_doc == doc_id`.
//! 4. [`write_field`](StoredFieldsConsumer::write_field) may only be called
//!    between `start_document` and `finish_document`, and dispatches on the
//!    type of the value (`StoredFieldsConsumer.java:65-91`).
//! 5. [`finish`](StoredFieldsConsumer::finish) opens and closes an empty frame
//!    for every document up to `max_doc - 1` that was never started, so the
//!    writer sees exactly `max_doc` frames (`StoredFieldsConsumer.java:97-102`).
//! 6. [`flush`](StoredFieldsConsumer::flush) calls `finish(maxDoc)` on the
//!    writer and closes it, closing it even when `finish` failed
//!    (`StoredFieldsConsumer.java:104-110`).
//! 7. [`abort`](StoredFieldsConsumer::abort) closes the writer, discarding any
//!    error, and is safe to call when no writer was ever created
//!    (`StoredFieldsConsumer.java:112-114`). An aborted consumer stays aborted:
//!    see the adaptation below.
//!
//! # Java to Rust adaptations
//!
//! * **The writer is an `Option`, not a nullable field.** Java reads
//!   `writer.finish(...)` in `flush` without a null check and relies on
//!   `IndexingChain.flush` having called `finish(maxDoc)` first, which for a
//!   segment with at least one document always creates the writer. Rust makes
//!   the absence explicit and reports it as
//!   [`LuceneError::IllegalState`] instead of dereferencing null.
//! * **Segment attributes are synchronised at flush.** Java's consumer holds a
//!   reference to the very `SegmentInfo` that ends up committed, so when
//!   `Lucene90StoredFieldsFormat.fieldsWriter` records its
//!   `Lucene90StoredFieldsFormat.mode` attribute the committed segment carries
//!   it. Rust cannot share that object — the `DocumentsWriterPerThread` clones
//!   its `SegmentInfo` for the flush — so [`StoredFieldsConsumer::flush`]
//!   copies the attributes the format recorded onto the segment being flushed.
//!   Without it the segment would be unreadable: the format refuses to open a
//!   segment whose mode attribute is missing.
//! * **An aborted consumer refuses to write again — a deliberate deviation.**
//!   Java's `abort()` is exactly `IOUtils.closeWhileHandlingException(writer)`
//!   and leaves the field pointing at the closed writer, whose `close()` has
//!   nulled all four of its own streams; a further `startDocument` would
//!   therefore appear to succeed and only fail later with a
//!   `NullPointerException` inside `flush`. Rust has to release the writer in
//!   order to close it, which would let the lazy initialisation build a
//!   *second* set of stored-fields files for a segment that was already
//!   discarded. This port records the abort instead and
//!   [`StoredFieldsConsumer::init_stored_fields_writer`] reports
//!   [`LuceneError::AlreadyClosed`]. The `DocumentsWriterPerThread` contract
//!   never reaches this state, so no behaviour Lucene relies on changes.
//! * **`Accountable` is not ported.** Lucene wraps the writer in an
//!   `Accountable` so that a null writer reports zero bytes;
//!   [`StoredFieldsConsumer::ram_bytes_used`] returns zero for the same reason,
//!   without the wrapper.
//!
//! # Index sorting
//!
//! Lucene subclasses this consumer with `SortingStoredFieldsConsumer`, which
//! writes to a temporary, uncompressed stored-fields file during indexing and,
//! at flush time, replays the documents through the real codec writer in the
//! order given by a `Sorter.DocMap`. That subclass depends on index-sorting
//! infrastructure (`Sorter.DocMap`, `IndexSorter`,
//! `TrackingTmpOutputDirectoryWrapper`) which is not part of this port yet, so
//! it belongs to the index-sorting task. The extension points it needs are in
//! place and are not expected to change: writer creation is isolated in
//! [`StoredFieldsConsumer::init_stored_fields_writer`], and
//! [`StoredFieldsConsumer::flush`] already takes the segment being flushed, so
//! a sorting variant adds the temporary directory and the doc map without
//! altering this type's contract.

#![deny(unsafe_code)]

use std::sync::Arc;

use crate::codecs::stored_fields::StoredFieldsWriter;
use crate::codecs::Codec;
use crate::document::StoredValue;
use crate::error::{LuceneError, Result};
use crate::index::{FieldInfo, SegmentInfo, StoredFieldDataInput};
use crate::store::{ByteArrayDataInput, Directory, DEFAULT_IO_CONTEXT};

/// Writes the stored fields of one segment.
///
/// Equivalent to `org.apache.lucene.index.StoredFieldsConsumer`. See the module
/// documentation for the specification this type implements.
pub struct StoredFieldsConsumer {
    codec: Arc<dyn Codec>,
    directory: Arc<dyn Directory>,
    info: SegmentInfo,
    writer: Option<Box<dyn StoredFieldsWriter>>,
    last_doc: i32,
    aborted: bool,
}

impl std::fmt::Debug for StoredFieldsConsumer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StoredFieldsConsumer")
            .field("segment", &self.info.name)
            .field("has_writer", &self.writer.is_some())
            .field("last_doc", &self.last_doc)
            .finish_non_exhaustive()
    }
}

impl StoredFieldsConsumer {
    /// Creates a consumer that will write the stored fields of `info`.
    ///
    /// Equivalent to
    /// `StoredFieldsConsumer(Codec, Directory, SegmentInfo)`. No file is
    /// created until the first document frame is opened.
    ///
    /// `directory` must be the tracking directory of the segment, so that the
    /// files the codec creates are recorded in the segment's file list.
    pub fn new(codec: Arc<dyn Codec>, directory: Arc<dyn Directory>, info: SegmentInfo) -> Self {
        Self {
            codec,
            directory,
            info,
            writer: None,
            last_doc: -1,
            aborted: false,
        }
    }

    /// Returns `true` once the codec writer has been created.
    ///
    /// The stored-fields files of a segment exist exactly when this is `true`.
    pub fn has_writer(&self) -> bool {
        self.writer.is_some()
    }

    /// Returns the highest doc id whose frame has been opened, or `-1`.
    ///
    /// Equivalent to reading the private `lastDoc` field.
    pub fn last_doc(&self) -> i32 {
        self.last_doc
    }

    /// Returns the approximate heap usage of the buffered stored fields.
    ///
    /// Equivalent to `StoredFieldsConsumer.accountable.ramBytesUsed()`, which
    /// reports zero until the writer exists.
    pub fn ram_bytes_used(&self) -> i64 {
        self.writer
            .as_ref()
            .map_or(0, |writer| writer.ram_bytes_used())
    }

    /// Creates the codec writer if it does not exist yet.
    ///
    /// Equivalent to `StoredFieldsConsumer.initStoredFieldsWriter()`. This is
    /// the single point a subclass overrides to redirect the stream, which is
    /// how `SortingStoredFieldsConsumer` writes to a temporary file instead.
    ///
    /// # Errors
    ///
    /// Propagates any error raised while creating the segment's stored-fields
    /// files.
    pub fn init_stored_fields_writer(&mut self) -> Result<()> {
        if self.aborted {
            // A deliberate deviation, for a state the DWPT contract never
            // reaches. Java's `abort()` keeps the writer
            // (`StoredFieldsConsumer.java:112-114`), but
            // `Lucene90CompressingStoredFieldsWriter.close()` nulls
            // `metaStream`, `fieldsStream`, `indexWriter` and `compressor` in
            // its `finally` (`:166-176`) and its `startDocument()` is empty
            // (`:181`), so a further document would appear to succeed and only
            // fail much later with a `NullPointerException` inside `flush`.
            // Rust has to release the writer in order to close it, which would
            // instead let the lazy initialisation build a *second* set of
            // `.fdt`/`.fdx`/`.fdm` files for a segment that was already
            // discarded. Recording the abort and refusing immediately is the
            // only one of the three that is neither silent nor destructive.
            return Err(LuceneError::AlreadyClosed(format!(
                "the stored fields of segment {} were aborted",
                self.info.name
            )));
        }
        if self.writer.is_none() {
            self.writer = Some(self.codec.stored_fields_format().fields_writer(
                self.directory.as_ref(),
                &self.info,
                &*DEFAULT_IO_CONTEXT,
            )?);
        }
        Ok(())
    }

    /// Opens the stored-fields frame of `doc_id`.
    ///
    /// Equivalent to `StoredFieldsConsumer.startDocument(int)`. Every doc id
    /// between the previous call and `doc_id` gets an empty frame, so that a
    /// document which stored nothing still occupies its slot and the stream
    /// stays aligned with the doc ids.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] when `doc_id` does not advance —
    /// Lucene asserts `lastDoc < docID` — and propagates any I/O error.
    pub fn start_document(&mut self, doc_id: i32) -> Result<()> {
        if self.last_doc >= doc_id {
            return Err(LuceneError::IllegalState(format!(
                "stored fields must be written in increasing doc order: lastDoc={} docID={doc_id}",
                self.last_doc
            )));
        }
        self.init_stored_fields_writer()?;
        // Borrow the writer and the cursor separately so that `last_doc`
        // advances with every frame actually written, exactly as Java's
        // `while (++lastDoc < docID)` does: an I/O failure half-way through
        // must leave the cursor on the last frame that made it to the writer.
        let Self {
            writer, last_doc, ..
        } = self;
        let writer = writer.as_mut().ok_or_else(|| {
            LuceneError::IllegalState(
                "the stored-fields writer was not created by initStoredFieldsWriter".to_string(),
            )
        })?;
        loop {
            *last_doc += 1;
            if *last_doc >= doc_id {
                break;
            }
            writer.start_document()?;
            writer.finish_document()?;
        }
        writer.start_document()
    }

    /// Writes one stored value of the document whose frame is open.
    ///
    /// Equivalent to `StoredFieldsConsumer.writeField(FieldInfo, StoredValue)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] when no document frame is open,
    /// and propagates any I/O error.
    pub fn write_field(&mut self, info: &FieldInfo, value: StoredValue) -> Result<()> {
        let writer = self.writer_mut()?;
        match value {
            StoredValue::Integer(value) => writer.write_field_i32(info, value),
            StoredValue::Long(value) => writer.write_field_i64(info, value),
            StoredValue::Float(value) => writer.write_field_f32(info, value),
            StoredValue::Double(value) => writer.write_field_f64(info, value),
            StoredValue::Binary(value) => writer.write_field_bytes(info, value.slice()),
            StoredValue::DataInput(value) => {
                let length = i32::try_from(value.length).map_err(|_| {
                    LuceneError::IllegalArgument(format!(
                        "stored field \"{}\" is too large to store: {} bytes",
                        info.name, value.length
                    ))
                })?;
                let mut input = ByteArrayDataInput::new(value.slice().to_vec());
                let mut data_input = StoredFieldDataInput::new(&mut input, length);
                writer.write_field_data_input(info, &mut data_input)
            }
            StoredValue::String(value) => writer.write_field_string(info, &value),
        }
    }

    /// Closes the stored-fields frame of the current document.
    ///
    /// Equivalent to `StoredFieldsConsumer.finishDocument()`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] when no document frame was ever
    /// opened, and propagates any I/O error.
    pub fn finish_document(&mut self) -> Result<()> {
        self.writer_mut()?.finish_document()
    }

    /// Fills in the frames of every document that never opened one.
    ///
    /// Equivalent to `StoredFieldsConsumer.finish(int)`. After this call the
    /// writer has seen exactly `max_doc` frames, which is what
    /// [`Self::flush`] asserts against.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while writing the empty frames.
    pub fn finish(&mut self, max_doc: i32) -> Result<()> {
        while self.last_doc < max_doc - 1 {
            let next = self.last_doc + 1;
            self.start_document(next)?;
            self.finish_document()?;
        }
        Ok(())
    }

    /// Finishes and closes the codec writer.
    ///
    /// Equivalent to `StoredFieldsConsumer.flush(SegmentWriteState, Sorter.DocMap)`
    /// for an unsorted segment. `segment_info` is the segment being committed:
    /// its `maxDoc` bounds the stream, and it receives the codec attributes the
    /// stored-fields format recorded (see the module documentation).
    ///
    /// The writer is closed even when finishing it failed, mirroring Java's
    /// `try`/`finally`; the error from `finish` wins over the one from `close`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] when no document was ever written
    /// — Lucene would raise a `NullPointerException` here — or when the segment
    /// already carries a conflicting value for a codec attribute. Propagates
    /// any I/O error.
    pub fn flush(&mut self, segment_info: &SegmentInfo) -> Result<()> {
        let max_doc = segment_info.max_doc()?;
        let Some(mut writer) = self.writer.take() else {
            return Err(LuceneError::IllegalState(format!(
                "no stored-fields writer was created for segment {}: \
                 finish(maxDoc) must run before flush",
                self.info.name
            )));
        };
        let finished = writer.finish(max_doc);
        let closed = writer.close();
        let copied = self.copy_attributes_to(segment_info);
        // Java propagates the exception from `finish` unconditionally, so no
        // later step may mask it: `and` keeps the first error of the three.
        finished.and(closed).and(copied)
    }

    /// Discards the segment's stored fields.
    ///
    /// Equivalent to `StoredFieldsConsumer.abort()`, which uses
    /// `IOUtils.closeWhileHandlingException`: the writer is released, any error
    /// raised while closing it is swallowed, and calling this without a writer
    /// is a no-op.
    pub fn abort(&mut self) {
        self.aborted = true;
        if let Some(mut writer) = self.writer.take() {
            let _ = writer.close();
        }
    }

    /// Returns `true` once [`Self::abort`] has run.
    pub fn is_aborted(&self) -> bool {
        self.aborted
    }

    /// Copies the codec attributes recorded on this consumer's `SegmentInfo`
    /// onto the segment being committed.
    fn copy_attributes_to(&self, segment_info: &SegmentInfo) -> Result<()> {
        for (key, value) in self.info.get_attributes() {
            match segment_info.get_attribute(&key) {
                None => {
                    segment_info.put_attribute(key, value);
                }
                Some(existing) if existing == value => {}
                Some(existing) => {
                    return Err(LuceneError::IllegalState(format!(
                        "found existing value for {key} for segment: {} old={existing}, new={value}",
                        segment_info.name
                    )));
                }
            }
        }
        Ok(())
    }

    fn writer_mut(&mut self) -> Result<&mut Box<dyn StoredFieldsWriter>> {
        self.writer.as_mut().ok_or_else(|| {
            LuceneError::IllegalState(format!(
                "no stored-fields document is open for segment {}",
                self.info.name
            ))
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::codecs::lucene90::stored_fields::Lucene90StoredFieldsFormat;
    use crate::codecs::stored_fields::{StoredFieldsFormat, StoredFieldsReader};
    use crate::codecs::{register_codec, FilterCodec, Lucene104Codec};
    use crate::index::{FieldInfos, StoredFieldVisitor, StoredFieldVisitorStatus};
    use crate::store::{ByteBuffersDirectory, IOContext};
    use crate::util::string_helper::StringHelper;
    use crate::util::{BytesRef, Version};

    // -- fixtures ---------------------------------------------------------

    fn lucene_codec() -> Arc<dyn Codec> {
        let _ = register_codec("Lucene104", Lucene104Codec::new());
        crate::codecs::default_codec().expect("Lucene104 codec is registered")
    }

    fn make_segment_info(directory: &Arc<dyn Directory>, max_doc: i32) -> SegmentInfo {
        SegmentInfo::new(
            Arc::clone(directory),
            Version::LATEST,
            Some(Version::LATEST),
            "_0".to_string(),
            max_doc,
            false,
            false,
            lucene_codec(),
            HashMap::new(),
            StringHelper::random_id(),
            HashMap::new(),
            Default::default(),
        )
        .expect("segment info")
    }

    /// The same segment as `make_segment_info`, with `maxDoc` still unset —
    /// the state a `DocumentsWriterPerThread` hands the consumer while it is
    /// still buffering documents.
    fn indexing_segment_info(directory: &Arc<dyn Directory>) -> SegmentInfo {
        make_segment_info(directory, -1)
    }

    fn field(name: &str, number: i32) -> FieldInfo {
        FieldInfo::new(name, number)
    }

    // -- a writer that records the calls it receives -----------------------

    #[derive(Debug, Default, Clone)]
    struct Log {
        calls: Arc<Mutex<Vec<String>>>,
        closes: Arc<Mutex<u32>>,
    }

    impl Log {
        fn push(&self, call: String) {
            self.calls.lock().expect("log mutex").push(call);
        }

        fn calls(&self) -> Vec<String> {
            self.calls.lock().expect("log mutex").clone()
        }

        fn closes(&self) -> u32 {
            *self.closes.lock().expect("log mutex")
        }
    }

    #[derive(Debug)]
    struct RecordingWriter {
        log: Log,
    }

    impl crate::util::Accountable for RecordingWriter {
        fn ram_bytes_used(&self) -> i64 {
            0
        }
    }

    impl StoredFieldsWriter for RecordingWriter {
        fn start_document(&mut self) -> Result<()> {
            self.log.push("start".to_string());
            Ok(())
        }
        fn finish_document(&mut self) -> Result<()> {
            self.log.push("finish".to_string());
            Ok(())
        }
        fn write_field_i32(&mut self, info: &FieldInfo, value: i32) -> Result<()> {
            self.log.push(format!("i32 {} {value}", info.name));
            Ok(())
        }
        fn write_field_i64(&mut self, info: &FieldInfo, value: i64) -> Result<()> {
            self.log.push(format!("i64 {} {value}", info.name));
            Ok(())
        }
        fn write_field_f32(&mut self, info: &FieldInfo, value: f32) -> Result<()> {
            self.log.push(format!("f32 {} {value}", info.name));
            Ok(())
        }
        fn write_field_f64(&mut self, info: &FieldInfo, value: f64) -> Result<()> {
            self.log.push(format!("f64 {} {value}", info.name));
            Ok(())
        }
        fn write_field_bytes(&mut self, info: &FieldInfo, value: &[u8]) -> Result<()> {
            self.log.push(format!("bytes {} {value:?}", info.name));
            Ok(())
        }
        fn write_field_string(&mut self, info: &FieldInfo, value: &str) -> Result<()> {
            self.log.push(format!("string {} {value}", info.name));
            Ok(())
        }
        fn finish(&mut self, num_docs: i32) -> Result<()> {
            self.log.push(format!("finish_writer {num_docs}"));
            Ok(())
        }
        fn close(&mut self) -> Result<()> {
            *self.closes.lock().expect("log mutex") += 1;
            Ok(())
        }
    }

    impl std::ops::Deref for RecordingWriter {
        type Target = Log;
        fn deref(&self) -> &Log {
            &self.log
        }
    }

    /// A writer whose `finish` always fails, to prove `flush` still closes it.
    #[derive(Debug)]
    struct FailingFinishWriter {
        log: Log,
    }

    impl crate::util::Accountable for FailingFinishWriter {
        fn ram_bytes_used(&self) -> i64 {
            0
        }
    }

    impl StoredFieldsWriter for FailingFinishWriter {
        fn start_document(&mut self) -> Result<()> {
            Ok(())
        }
        fn write_field_i32(&mut self, _info: &FieldInfo, _value: i32) -> Result<()> {
            Ok(())
        }
        fn write_field_i64(&mut self, _info: &FieldInfo, _value: i64) -> Result<()> {
            Ok(())
        }
        fn write_field_f32(&mut self, _info: &FieldInfo, _value: f32) -> Result<()> {
            Ok(())
        }
        fn write_field_f64(&mut self, _info: &FieldInfo, _value: f64) -> Result<()> {
            Ok(())
        }
        fn write_field_bytes(&mut self, _info: &FieldInfo, _value: &[u8]) -> Result<()> {
            Ok(())
        }
        fn write_field_string(&mut self, _info: &FieldInfo, _value: &str) -> Result<()> {
            Ok(())
        }
        fn finish(&mut self, _num_docs: i32) -> Result<()> {
            Err(LuceneError::Other("finish blew up".to_string()))
        }
        fn close(&mut self) -> Result<()> {
            *self.log.closes.lock().expect("log mutex") += 1;
            Ok(())
        }
    }

    /// Hands out one of the recording writers above.
    #[derive(Debug)]
    struct LoggingStoredFieldsFormat {
        log: Log,
        fail_finish: bool,
    }

    impl StoredFieldsFormat for LoggingStoredFieldsFormat {
        fn name(&self) -> &str {
            "LoggingStoredFields"
        }

        fn fields_reader(
            &self,
            _directory: &dyn Directory,
            _segment_info: &SegmentInfo,
            _field_infos: &FieldInfos,
            _context: &dyn IOContext,
        ) -> Result<Box<dyn StoredFieldsReader>> {
            Err(LuceneError::UnsupportedOperation(
                "the logging format does not read".to_string(),
            ))
        }

        fn fields_writer(
            &self,
            _directory: &dyn Directory,
            _segment_info: &SegmentInfo,
            _context: &dyn IOContext,
        ) -> Result<Box<dyn StoredFieldsWriter>> {
            self.log.push("open_writer".to_string());
            if self.fail_finish {
                Ok(Box::new(FailingFinishWriter {
                    log: self.log.clone(),
                }))
            } else {
                Ok(Box::new(RecordingWriter {
                    log: self.log.clone(),
                }))
            }
        }
    }

    fn logging_consumer(fail_finish: bool) -> (StoredFieldsConsumer, Log, Arc<dyn Directory>) {
        let log = Log::default();
        let codec: Arc<dyn Codec> = Arc::new(
            FilterCodec::new("LoggingCodec", lucene_codec()).with_stored_fields_format(
                LoggingStoredFieldsFormat {
                    log: log.clone(),
                    fail_finish,
                },
            ),
        );
        let directory: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
        let info = indexing_segment_info(&directory);
        let consumer = StoredFieldsConsumer::new(codec, Arc::clone(&directory), info);
        (consumer, log, directory)
    }

    // -- a visitor that records what it is handed --------------------------

    #[derive(Debug, Default)]
    struct Recorder {
        wanted: Option<Vec<String>>,
        stop_at: Option<String>,
        seen: Vec<String>,
    }

    impl StoredFieldVisitor for Recorder {
        fn binary_field(&mut self, info: &FieldInfo, value: &[u8]) -> Result<()> {
            self.seen.push(format!("{}=bin{value:?}", info.name));
            Ok(())
        }
        fn string_field(&mut self, info: &FieldInfo, value: &str) -> Result<()> {
            self.seen.push(format!("{}=str:{value}", info.name));
            Ok(())
        }
        fn int_field(&mut self, info: &FieldInfo, value: i32) -> Result<()> {
            self.seen.push(format!("{}=i32:{value}", info.name));
            Ok(())
        }
        fn long_field(&mut self, info: &FieldInfo, value: i64) -> Result<()> {
            self.seen.push(format!("{}=i64:{value}", info.name));
            Ok(())
        }
        fn float_field(&mut self, info: &FieldInfo, value: f32) -> Result<()> {
            self.seen
                .push(format!("{}=f32:{:08x}", info.name, value.to_bits()));
            Ok(())
        }
        fn double_field(&mut self, info: &FieldInfo, value: f64) -> Result<()> {
            self.seen
                .push(format!("{}=f64:{:016x}", info.name, value.to_bits()));
            Ok(())
        }
        fn needs_field(&mut self, info: &FieldInfo) -> Result<StoredFieldVisitorStatus> {
            if self.stop_at.as_deref() == Some(info.name.as_str()) {
                return Ok(StoredFieldVisitorStatus::Stop);
            }
            match &self.wanted {
                None => Ok(StoredFieldVisitorStatus::Yes),
                Some(names) if names.iter().any(|name| name == &info.name) => {
                    Ok(StoredFieldVisitorStatus::Yes)
                }
                Some(_) => Ok(StoredFieldVisitorStatus::No),
            }
        }
    }

    /// Writes `documents` through a real Lucene 10.5.0 codec consumer and reads
    /// every document back with `visitor_for`.
    fn round_trip(
        documents: &[Vec<(FieldInfo, StoredValue)>],
        mut visitor_for: impl FnMut() -> Recorder,
    ) -> Vec<Vec<String>> {
        let directory: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
        let indexing_info = indexing_segment_info(&directory);
        let segment_id = indexing_info.id();
        let mut consumer =
            StoredFieldsConsumer::new(lucene_codec(), Arc::clone(&directory), indexing_info);

        let mut infos = Vec::new();
        for (doc_id, fields) in documents.iter().enumerate() {
            if fields.is_empty() {
                continue;
            }
            consumer
                .start_document(doc_id as i32)
                .expect("start document");
            for (info, value) in fields {
                if !infos
                    .iter()
                    .any(|kept: &FieldInfo| kept.number == info.number)
                {
                    infos.push(info.clone());
                }
                consumer
                    .write_field(info, value.clone())
                    .expect("write field");
            }
            consumer.finish_document().expect("finish document");
        }
        let max_doc = documents.len() as i32;
        consumer.finish(max_doc).expect("finish");

        // The segment that gets committed is a *different* object, exactly as
        // in the DocumentsWriterPerThread flush path.
        let flushed = SegmentInfo::new(
            Arc::clone(&directory),
            Version::LATEST,
            Some(Version::LATEST),
            "_0".to_string(),
            max_doc,
            false,
            false,
            lucene_codec(),
            HashMap::new(),
            segment_id,
            HashMap::new(),
            Default::default(),
        )
        .expect("flushed segment info");
        consumer.flush(&flushed).expect("flush");

        infos.sort_by_key(|info| info.number);
        let field_infos = FieldInfos::new(infos).expect("field infos");
        let reader = lucene_codec()
            .stored_fields_format()
            .fields_reader(
                directory.as_ref(),
                &flushed,
                &field_infos,
                &*DEFAULT_IO_CONTEXT,
            )
            .expect("fields reader");

        (0..max_doc)
            .map(|doc_id| {
                let mut visitor = visitor_for();
                reader.document(doc_id, &mut visitor).expect("document");
                visitor.seen
            })
            .collect()
    }

    // -- lifecycle --------------------------------------------------------

    #[test]
    fn a_fresh_consumer_creates_no_file_and_starts_before_the_first_doc() {
        let (consumer, log, directory) = logging_consumer(false);
        assert!(!consumer.has_writer());
        assert_eq!(consumer.last_doc(), -1, "Java initialises lastDoc to -1");
        assert_eq!(consumer.ram_bytes_used(), 0);
        assert!(log.calls().is_empty());
        assert!(
            directory.list_all().expect("list").is_empty(),
            "no stored-fields file may exist before a document frame is opened"
        );
    }

    #[test]
    fn the_writer_is_created_on_the_first_document() {
        let (mut consumer, log, _dir) = logging_consumer(false);
        consumer.start_document(0).expect("start");
        assert!(consumer.has_writer());
        assert_eq!(log.calls(), vec!["open_writer", "start"]);
        // A second document must not create a second writer.
        consumer.finish_document().expect("finish");
        consumer.start_document(1).expect("start");
        assert_eq!(
            log.calls()
                .iter()
                .filter(|call| *call == "open_writer")
                .count(),
            1
        );
    }

    #[test]
    fn skipped_documents_get_an_empty_frame_each() {
        let (mut consumer, log, _dir) = logging_consumer(false);
        // Documents 0, 1 and 2 stored nothing; document 3 does.
        consumer.start_document(3).expect("start");
        consumer.finish_document().expect("finish");
        assert_eq!(
            log.calls(),
            vec![
                "open_writer",
                "start",
                "finish", // doc 0
                "start",
                "finish", // doc 1
                "start",
                "finish", // doc 2
                "start",
                "finish", // doc 3
            ]
        );
        assert_eq!(consumer.last_doc(), 3);
    }

    #[test]
    fn finish_fills_every_trailing_document() {
        let (mut consumer, log, _dir) = logging_consumer(false);
        consumer.start_document(0).expect("start");
        consumer.finish_document().expect("finish");
        consumer.finish(4).expect("finish gaps");
        assert_eq!(consumer.last_doc(), 3, "maxDoc - 1 frames must exist");
        let frames = log.calls().iter().filter(|call| *call == "start").count();
        assert_eq!(frames, 4, "the writer must see exactly maxDoc frames");
    }

    #[test]
    fn finish_on_a_segment_whose_documents_all_stored_nothing_still_writes_the_frames() {
        let (mut consumer, log, _dir) = logging_consumer(false);
        consumer.finish(3).expect("finish");
        assert!(
            consumer.has_writer(),
            "Lucene creates the stored-fields files even for a segment with no stored field"
        );
        assert_eq!(log.calls().iter().filter(|c| *c == "start").count(), 3);
    }

    #[test]
    fn finish_is_a_no_op_when_every_document_was_written() {
        let (mut consumer, log, _dir) = logging_consumer(false);
        for doc in 0..3 {
            consumer.start_document(doc).expect("start");
            consumer.finish_document().expect("finish");
        }
        let before = log.calls().len();
        consumer.finish(3).expect("finish");
        assert_eq!(log.calls().len(), before);
    }

    #[test]
    fn documents_must_be_written_in_increasing_order() {
        let (mut consumer, _log, _dir) = logging_consumer(false);
        consumer.start_document(2).expect("start");
        consumer.finish_document().expect("finish");
        let error = consumer
            .start_document(2)
            .expect_err("a document may not be revisited");
        assert!(matches!(error, LuceneError::IllegalState(_)), "{error:?}");
        let error = consumer
            .start_document(1)
            .expect_err("doc ids may not go backwards");
        assert!(matches!(error, LuceneError::IllegalState(_)), "{error:?}");
    }

    #[test]
    fn flush_finishes_the_writer_with_max_doc_and_closes_it() {
        let (mut consumer, log, directory) = logging_consumer(false);
        consumer.start_document(0).expect("start");
        consumer.finish_document().expect("finish");
        consumer.finish(2).expect("fill");
        let flushed = make_segment_info(&directory, 2);
        consumer.flush(&flushed).expect("flush");
        assert!(log.calls().contains(&"finish_writer 2".to_string()));
        assert_eq!(log.closes(), 1);
        assert!(!consumer.has_writer(), "flush releases the writer");
    }

    #[test]
    fn flush_closes_the_writer_even_when_finishing_it_fails() {
        let (mut consumer, log, directory) = logging_consumer(true);
        consumer.start_document(0).expect("start");
        consumer.finish_document().expect("finish");
        let flushed = make_segment_info(&directory, 1);
        let error = consumer.flush(&flushed).expect_err("finish blows up");
        assert!(
            error.to_string().contains("finish blew up"),
            "the error from finish must win over the one from close: {error}"
        );
        assert_eq!(log.closes(), 1, "Java closes the writer in a finally block");
    }

    #[test]
    fn flush_without_a_single_document_is_reported_rather_than_panicking() {
        let (mut consumer, _log, directory) = logging_consumer(false);
        let flushed = make_segment_info(&directory, 0);
        let error = consumer
            .flush(&flushed)
            .expect_err("Java would raise a NullPointerException here");
        assert!(matches!(error, LuceneError::IllegalState(_)), "{error:?}");
    }

    #[test]
    fn abort_closes_the_writer_and_is_idempotent() {
        let (mut consumer, log, _dir) = logging_consumer(false);
        consumer.start_document(0).expect("start");
        consumer.abort();
        assert_eq!(log.closes(), 1);
        assert!(!consumer.has_writer());
        consumer.abort();
        assert_eq!(log.closes(), 1, "aborting twice must not close twice");
    }

    #[test]
    fn an_aborted_consumer_refuses_to_write_again() {
        // Java leaves the closed writer in place, so the next `startDocument`
        // hits a closed `IndexOutput` and raises `AlreadyClosedException`.
        // Releasing the writer without recording the abort would instead create
        // a second set of stored-fields files for a discarded segment.
        let (mut consumer, log, directory) = logging_consumer(false);
        consumer.start_document(0).expect("start");
        consumer.abort();
        assert!(consumer.is_aborted());

        let error = consumer
            .start_document(1)
            .expect_err("an aborted consumer must not write again");
        assert!(matches!(error, LuceneError::AlreadyClosed(_)), "{error:?}");
        assert_eq!(
            log.calls().iter().filter(|c| *c == "open_writer").count(),
            1,
            "no second writer may be created after an abort"
        );

        let flushed = make_segment_info(&directory, 2);
        let error = consumer
            .flush(&flushed)
            .expect_err("an aborted segment has nothing to flush");
        assert!(matches!(error, LuceneError::IllegalState(_)), "{error:?}");
    }

    #[test]
    fn a_data_input_value_reaches_the_index_and_reads_back_as_binary() {
        // `StoredValue::DataInput` is the one variant with no direct Java
        // counterpart in this port's ownership model, so its end-to-end path is
        // pinned here: it must land in the `.fdt` as a BYTE_ARR field and come
        // back through `binary_field`, indistinguishable from `Binary`.
        let documents = vec![vec![
            (
                field("streamed", 0),
                StoredValue::DataInput(BytesRef::new((0..=255u8).collect())),
            ),
            (
                field("copied", 1),
                StoredValue::Binary(BytesRef::new((0..=255u8).collect())),
            ),
        ]];
        let seen = round_trip(&documents, Recorder::default);
        let expected: String = (0..=255u8).map(|b| format!("{b}, ")).collect();
        let expected = format!("[{}]", expected.trim_end_matches(", "));
        assert_eq!(
            seen,
            vec![vec![
                format!("streamed=bin{expected}"),
                format!("copied=bin{expected}"),
            ]]
        );
    }

    #[test]
    fn abort_without_a_writer_is_a_no_op() {
        let (mut consumer, log, _dir) = logging_consumer(false);
        consumer.abort();
        assert_eq!(log.closes(), 0);
    }

    #[test]
    fn writing_a_field_outside_a_document_is_rejected() {
        let (mut consumer, _log, _dir) = logging_consumer(false);
        let error = consumer
            .write_field(&field("f", 0), StoredValue::Integer(1))
            .expect_err("no document frame is open");
        assert!(matches!(error, LuceneError::IllegalState(_)), "{error:?}");
    }

    #[test]
    fn every_stored_value_variant_reaches_its_own_writer_call() {
        let (mut consumer, log, _dir) = logging_consumer(false);
        consumer.start_document(0).expect("start");
        consumer
            .write_field(&field("i", 0), StoredValue::Integer(-7))
            .unwrap();
        consumer
            .write_field(&field("l", 1), StoredValue::Long(1 << 40))
            .unwrap();
        consumer
            .write_field(&field("f", 2), StoredValue::Float(0.5))
            .unwrap();
        consumer
            .write_field(&field("d", 3), StoredValue::Double(-0.25))
            .unwrap();
        consumer
            .write_field(
                &field("b", 4),
                StoredValue::Binary(BytesRef::new(vec![1, 2])),
            )
            .unwrap();
        consumer
            .write_field(
                &field("p", 5),
                StoredValue::DataInput(BytesRef::new(vec![3, 4, 5])),
            )
            .unwrap();
        consumer
            .write_field(&field("s", 6), StoredValue::String("hi".to_string()))
            .unwrap();
        consumer.finish_document().unwrap();

        assert_eq!(
            log.calls(),
            vec![
                "open_writer",
                "start",
                "i32 i -7",
                "i64 l 1099511627776",
                "f32 f 0.5",
                "f64 d -0.25",
                "bytes b [1, 2]",
                // The DATA_INPUT variant reaches the writer through the
                // streaming call, whose default implementation forwards the
                // bytes; a codec that overrides it never sees this line.
                "bytes p [3, 4, 5]",
                "string s hi",
                "finish",
            ]
        );
    }

    // -- round trips through the real Lucene 10.5.0 codec -------------------

    #[test]
    fn every_stored_value_variant_round_trips_through_the_codec() {
        let documents = vec![vec![
            (field("i", 0), StoredValue::Integer(i32::MIN)),
            (field("l", 1), StoredValue::Long(i64::MAX)),
            (field("f", 2), StoredValue::Float(-0.0)),
            (field("d", 3), StoredValue::Double(f64::consts_pi())),
            (
                field("b", 4),
                StoredValue::Binary(BytesRef::new(vec![0, 255, 128])),
            ),
            (
                field("p", 5),
                StoredValue::DataInput(BytesRef::new(vec![7, 8, 9, 10])),
            ),
            (
                field("s", 6),
                StoredValue::String("olá \u{1F600} world".to_string()),
            ),
        ]];
        let seen = round_trip(&documents, Recorder::default);
        assert_eq!(
            seen,
            vec![vec![
                format!("i=i32:{}", i32::MIN),
                format!("l=i64:{}", i64::MAX),
                format!("f=f32:{:08x}", (-0.0f32).to_bits()),
                format!("d=f64:{:016x}", f64::consts_pi().to_bits()),
                "b=bin[0, 255, 128]".to_string(),
                // BINARY and DATA_INPUT share the same on-disk type, so both
                // come back through `binary_field`.
                "p=bin[7, 8, 9, 10]".to_string(),
                "s=str:olá \u{1F600} world".to_string(),
            ]]
        );
    }

    #[test]
    fn documents_without_stored_fields_keep_their_slot() {
        let documents = vec![
            Vec::new(),
            vec![(field("s", 0), StoredValue::String("one".to_string()))],
            Vec::new(),
            vec![(field("s", 0), StoredValue::String("three".to_string()))],
            Vec::new(),
        ];
        let seen = round_trip(&documents, Recorder::default);
        assert_eq!(
            seen,
            vec![
                Vec::<String>::new(),
                vec!["s=str:one".to_string()],
                Vec::new(),
                vec!["s=str:three".to_string()],
                Vec::new(),
            ],
            "a document that stored nothing must not shift the doc ids of the others"
        );
    }

    #[test]
    fn a_visitor_can_load_a_subset_of_the_fields() {
        let documents = vec![vec![
            (field("a", 0), StoredValue::String("first".to_string())),
            (field("b", 1), StoredValue::String("second".to_string())),
            (field("c", 2), StoredValue::String("third".to_string())),
        ]];
        let seen = round_trip(&documents, || Recorder {
            wanted: Some(vec!["b".to_string()]),
            ..Default::default()
        });
        assert_eq!(seen, vec![vec!["b=str:second".to_string()]]);
    }

    #[test]
    fn a_visitor_can_stop_in_the_middle_of_a_document() {
        let documents = vec![vec![
            (field("a", 0), StoredValue::String("first".to_string())),
            (field("b", 1), StoredValue::String("second".to_string())),
            (field("c", 2), StoredValue::String("third".to_string())),
        ]];
        let seen = round_trip(&documents, || Recorder {
            stop_at: Some("b".to_string()),
            ..Default::default()
        });
        assert_eq!(
            seen,
            vec![vec!["a=str:first".to_string()]],
            "STOP must skip the field it was returned for and everything after it"
        );
    }

    #[test]
    fn a_multi_valued_stored_field_keeps_every_value_in_order() {
        let documents = vec![vec![
            (field("s", 0), StoredValue::String("one".to_string())),
            (field("s", 0), StoredValue::String("two".to_string())),
            (field("s", 0), StoredValue::String("three".to_string())),
        ]];
        let seen = round_trip(&documents, Recorder::default);
        assert_eq!(
            seen,
            vec![vec![
                "s=str:one".to_string(),
                "s=str:two".to_string(),
                "s=str:three".to_string(),
            ]]
        );
    }

    #[test]
    fn flushing_hands_the_codec_attributes_to_the_committed_segment() {
        // `Lucene90StoredFieldsFormat.fieldsWriter` records the compression
        // mode on the SegmentInfo it is given, and `fieldsReader` refuses to
        // open a segment without it. Since the consumer holds its own copy of
        // the SegmentInfo, flush must hand the attribute over — the round-trip
        // tests above would fail outright without this.
        let directory: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
        let indexing_info = indexing_segment_info(&directory);
        let segment_id = indexing_info.id();
        let mut consumer =
            StoredFieldsConsumer::new(lucene_codec(), Arc::clone(&directory), indexing_info);
        consumer.start_document(0).expect("start");
        consumer
            .write_field(&field("s", 0), StoredValue::String("x".to_string()))
            .expect("write");
        consumer.finish_document().expect("finish");
        consumer.finish(1).expect("fill");

        let flushed = SegmentInfo::new(
            Arc::clone(&directory),
            Version::LATEST,
            Some(Version::LATEST),
            "_0".to_string(),
            1,
            false,
            false,
            lucene_codec(),
            HashMap::new(),
            segment_id,
            HashMap::new(),
            Default::default(),
        )
        .expect("segment info");
        assert!(
            flushed
                .get_attribute(Lucene90StoredFieldsFormat::MODE_KEY)
                .is_none(),
            "the committed segment starts without the attribute"
        );
        consumer.flush(&flushed).expect("flush");
        assert_eq!(
            flushed.get_attribute(Lucene90StoredFieldsFormat::MODE_KEY),
            Some("BEST_SPEED".to_string())
        );
    }

    #[test]
    fn flushing_reports_a_conflicting_codec_attribute() {
        let directory: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
        let indexing_info = indexing_segment_info(&directory);
        let mut consumer =
            StoredFieldsConsumer::new(lucene_codec(), Arc::clone(&directory), indexing_info);
        consumer.start_document(0).expect("start");
        consumer.finish_document().expect("finish");

        let flushed = make_segment_info(&directory, 1);
        flushed.put_attribute(
            Lucene90StoredFieldsFormat::MODE_KEY.to_string(),
            "BEST_COMPRESSION".to_string(),
        );
        let error = consumer.flush(&flushed).expect_err("conflicting attribute");
        assert!(matches!(error, LuceneError::IllegalState(_)), "{error:?}");
    }

    #[test]
    fn the_segment_files_are_tracked_by_the_directory_wrapper() {
        use crate::store::TrackingDirectoryWrapper;

        let inner = ByteBuffersDirectory::new();
        let tracking = Arc::new(TrackingDirectoryWrapper::new(Box::new(inner)));
        let plain: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
        let info = indexing_segment_info(&plain);
        let mut consumer = StoredFieldsConsumer::new(
            lucene_codec(),
            Arc::clone(&tracking) as Arc<dyn Directory>,
            info,
        );
        consumer.start_document(0).expect("start");
        consumer.finish_document().expect("finish");
        let created = tracking.get_created_files();
        let mut extensions: Vec<String> = created
            .iter()
            .filter_map(|name| name.rsplit_once('.').map(|(_, ext)| ext.to_string()))
            .collect();
        extensions.sort();
        assert_eq!(
            extensions,
            vec!["fdm".to_string(), "fdt".to_string(), "fdx".to_string()],
            "the consumer must write through the tracking wrapper so the segment lists its files"
        );
    }

    trait Pi {
        fn consts_pi() -> f64;
    }

    impl Pi for f64 {
        fn consts_pi() -> f64 {
            std::f64::consts::PI
        }
    }
}
