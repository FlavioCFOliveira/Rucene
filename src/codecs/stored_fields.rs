//! Stored-fields format base traits.
//!
//! Equivalent to `org.apache.lucene.codecs.StoredFieldsFormat`,
//! `StoredFieldsReader` and `StoredFieldsWriter`.

use std::fmt;

use crate::error::Result;
use crate::store::{Directory, IOContext};
use crate::util::Accountable;

use std::sync::Arc;

use super::postings::MergeState;
use super::stub::{FieldInfo, FieldInfos, SegmentInfo, StoredFieldVisitor};
use crate::index::merge::{DocIDMerger, DocIDMergerSub, DocMap};
use crate::index::stored_field_visitor::StoredFieldVisitorStatus;
use crate::index::StoredFieldDataInput;
use crate::search::doc_id_set_iterator::NO_MORE_DOCS;

/// Controls the format of stored fields.
///
/// Lucene Core equivalent: `org.apache.lucene.codecs.StoredFieldsFormat`.
pub trait StoredFieldsFormat: Send + Sync + fmt::Debug {
    /// Returns this format's SPI name.
    fn name(&self) -> &str;

    /// Returns a [`StoredFieldsReader`] to load stored fields.
    fn fields_reader(
        &self,
        directory: &dyn Directory,
        segment_info: &SegmentInfo,
        field_infos: &FieldInfos,
        context: &dyn IOContext,
    ) -> Result<Box<dyn StoredFieldsReader>>;

    /// Returns a [`StoredFieldsWriter`] to write stored fields.
    ///
    /// The directory is shared, not borrowed: as Java's writer keeps its
    /// `Directory` reference, the writer may create, read back and delete
    /// files after this call returns — the fields index streams to temporary
    /// files it only turns into the `.fdx`/`.tvx` at `finish`
    /// (`FieldsIndexWriter.java:83-87, 105-181`).
    fn fields_writer(
        &self,
        directory: &std::sync::Arc<dyn Directory>,
        segment_info: &SegmentInfo,
        context: &dyn IOContext,
    ) -> Result<Box<dyn StoredFieldsWriter>>;
}

/// Codec API for reading stored fields.
///
/// Lucene Core equivalent: `org.apache.lucene.codecs.StoredFieldsReader`.
pub trait StoredFieldsReader: Send + Sync + fmt::Debug {
    /// Visits the stored fields of the given document.
    fn document(&self, doc_id: i32, visitor: &mut dyn StoredFieldVisitor) -> Result<()>;

    /// Checks consistency of this reader.
    fn check_integrity(&self) -> Result<()>;

    /// Creates a clone that one caller at a time may use to read stored fields.
    fn clone_reader(&self) -> Box<dyn StoredFieldsReader>;

    /// Returns an instance optimized for merging.
    ///
    /// The default implementation returns a clone of `self`.
    fn get_merge_instance(&self) -> Box<dyn StoredFieldsReader> {
        self.clone_reader()
    }

    /// Optional hint that the given document will be read in the near future.
    fn prefetch(&self, _doc_id: i32) -> Result<()> {
        Ok(())
    }

    /// Returns this reader as [`std::any::Any`], for the one caller that has to
    /// recognise a concrete reader type: the compressing stored-fields writer,
    /// which can only copy raw chunks out of a reader of its own format.
    ///
    /// Divergence: Lucene writes `candidate instanceof
    /// Lucene90CompressingStoredFieldsReader`
    /// (`Lucene90CompressingStoredFieldsWriter.java:681-689`). Rust has no
    /// `instanceof` over a trait object, so the concrete type is reached
    /// through `Any`. The default answers `None`, which reads as "not a type
    /// anybody can bulk-merge from" and lands the caller on the same
    /// document-by-document path Java takes for a foreign reader; only the
    /// readers that support the fast path override it.
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        None
    }

    /// Closes this reader, releasing all resources.
    fn close(&mut self) -> Result<()> {
        Ok(())
    }
}

/// Codec API for writing stored fields.
///
/// Lucene Core equivalent: `org.apache.lucene.codecs.StoredFieldsWriter`.
///
/// A writer is owned by exactly one
/// [`StoredFieldsConsumer`](crate::index::StoredFieldsConsumer) and is never
/// shared, so it needs no interior synchronisation; `Send` is required only so
/// that the `DocumentsWriterPerThread` holding it can move between indexing
/// threads.
///
/// Java declares `StoredFieldsWriter implements Accountable`, so the flush
/// policy can count the documents buffered in RAM; [`Accountable`] is a
/// supertrait here for the same reason.
pub trait StoredFieldsWriter: Accountable + Send + fmt::Debug {
    /// Called before writing the stored fields of a document.
    fn start_document(&mut self) -> Result<()>;

    /// Called after a document and all its fields have been added.
    fn finish_document(&mut self) -> Result<()> {
        Ok(())
    }

    /// Writes a stored int value.
    fn write_field_i32(&mut self, info: &FieldInfo, value: i32) -> Result<()>;

    /// Writes a stored long value.
    fn write_field_i64(&mut self, info: &FieldInfo, value: i64) -> Result<()>;

    /// Writes a stored float value.
    fn write_field_f32(&mut self, info: &FieldInfo, value: f32) -> Result<()>;

    /// Writes a stored double value.
    fn write_field_f64(&mut self, info: &FieldInfo, value: f64) -> Result<()>;

    /// Writes a stored binary value.
    fn write_field_bytes(&mut self, info: &FieldInfo, value: &[u8]) -> Result<()>;

    /// Writes a stored binary value read from a [`StoredFieldDataInput`].
    ///
    /// Lucene Core equivalent:
    /// `StoredFieldsWriter.writeField(FieldInfo, StoredFieldDataInput)`. The
    /// default implementation reads every byte into a fresh buffer and calls
    /// [`Self::write_field_bytes`], exactly as the Java base class does; a
    /// codec that can stream the bytes straight into its own buffer overrides
    /// it, as `Lucene90CompressingStoredFieldsWriter` does.
    ///
    /// # Errors
    ///
    /// Returns [`crate::error::LuceneError::IllegalArgument`] when the declared
    /// length is negative, and propagates any I/O error.
    fn write_field_data_input(
        &mut self,
        info: &FieldInfo,
        value: &mut StoredFieldDataInput<'_>,
    ) -> Result<()> {
        let length = value.length();
        if length < 0 {
            return Err(crate::error::LuceneError::IllegalArgument(format!(
                "stored binary field \"{}\" has a negative length: {length}",
                info.name
            )));
        }
        let length = length as usize;
        let mut bytes = vec![0u8; length];
        value.data_input().read_bytes(&mut bytes, 0, length)?;
        self.write_field_bytes(info, &bytes)
    }

    /// Writes a stored string value.
    fn write_field_string(&mut self, info: &FieldInfo, value: &str) -> Result<()>;

    /// Called before close, passing the number of documents written.
    fn finish(&mut self, num_docs: i32) -> Result<()>;

    /// Copies every document's stored fields from the segments being merged.
    ///
    /// Equivalent to `StoredFieldsWriter.merge(MergeState)`
    /// (`StoredFieldsWriter.java:122-160`). The documents come out in merged
    /// order through a [`DocIDMerger`], each one read back through the merge
    /// state's own reader and written straight out again; the return value is
    /// the number of documents written, which the caller checks against the
    /// merged segment's `max_doc`.
    ///
    /// Every source segment must have a stored-fields reader: Java calls
    /// `checkIntegrity()` on each slot unconditionally, so a missing reader
    /// stops the merge there rather than leaving that segment's documents out.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`](crate::error::LuceneError::IllegalState)
    /// when a source segment has no stored-fields reader — where Java's
    /// `storedFieldsReader.checkIntegrity()` throws `NullPointerException` —
    /// and propagates abort, integrity, read and write errors.
    fn merge(&mut self, merge_state: &MergeState) -> Result<i32> {
        // The readers stay outside the merger: the sub carries only the index
        // of the one it walks, and the document is read through the merge
        // state's reader itself, as Java's `sub.reader` is that same object.
        let mut readers: Vec<&dyn StoredFieldsReader> = Vec::new();
        let mut remappers: Vec<Option<FieldInfos>> = Vec::new();
        let mut subs: Vec<StoredFieldsMergeSub> = Vec::new();
        for (i, reader) in merge_state.stored_fields_readers.iter().enumerate() {
            merge_state.check_aborted()?;
            let Some(reader) = reader.as_deref() else {
                return Err(crate::error::LuceneError::illegal_state(format!(
                    "source segment {i} of the merge has no stored-fields reader"
                )));
            };
            reader.check_integrity()?;
            subs.push(StoredFieldsMergeSub {
                doc_map: Arc::clone(&merge_state.doc_maps[i]),
                reader_index: readers.len(),
                max_doc: merge_state.max_docs[i],
                doc_id: -1,
                mapped_doc_id: -1,
            });
            readers.push(reader);
            remappers.push(merge_visitor_remapper(merge_state, i));
        }

        let mut merger = DocIDMerger::new(subs, merge_state.needs_index_sort)?;
        let mut doc_count = 0i32;
        while let Some((reader_index, doc_id, mapped_doc_id)) = merger
            .next_sub()?
            .map(|sub| (sub.reader_index, sub.doc_id, sub.mapped_doc_id))
        {
            debug_assert_eq!(mapped_doc_id, doc_count);
            self.start_document()?;
            let mut visitor = MergeVisitor {
                writer: self,
                remapper: remappers[reader_index].as_ref(),
            };
            readers[reader_index].document(doc_id, &mut visitor)?;
            self.finish_document()?;
            doc_count += 1;
        }
        self.finish(doc_count)?;
        Ok(doc_count)
    }

    /// Closes the writer and releases any resources.
    fn close(&mut self) -> Result<()>;
}

/// A minimal no-op stored-fields format.
#[derive(Debug, Copy, Clone, Default)]
pub struct EmptyStoredFieldsFormat;

impl StoredFieldsFormat for EmptyStoredFieldsFormat {
    fn name(&self) -> &str {
        "EmptyStoredFields"
    }

    fn fields_reader(
        &self,
        _directory: &dyn Directory,
        _segment_info: &SegmentInfo,
        _field_infos: &FieldInfos,
        _context: &dyn IOContext,
    ) -> Result<Box<dyn StoredFieldsReader>> {
        Ok(Box::new(EmptyStoredFieldsReader))
    }

    fn fields_writer(
        &self,
        _directory: &std::sync::Arc<dyn Directory>,
        _segment_info: &SegmentInfo,
        _context: &dyn IOContext,
    ) -> Result<Box<dyn StoredFieldsWriter>> {
        Ok(Box::new(EmptyStoredFieldsWriter))
    }
}

/// A minimal no-op stored-fields reader.
#[derive(Debug, Copy, Clone, Default)]
pub struct EmptyStoredFieldsReader;

impl StoredFieldsReader for EmptyStoredFieldsReader {
    fn document(&self, _doc_id: i32, _visitor: &mut dyn StoredFieldVisitor) -> Result<()> {
        Ok(())
    }

    fn check_integrity(&self) -> Result<()> {
        Ok(())
    }

    fn clone_reader(&self) -> Box<dyn StoredFieldsReader> {
        Box::new(*self)
    }
}

/// A minimal no-op stored-fields writer.
#[derive(Debug, Copy, Clone, Default)]
pub struct EmptyStoredFieldsWriter;

impl Accountable for EmptyStoredFieldsWriter {
    fn ram_bytes_used(&self) -> i64 {
        0
    }
}

impl StoredFieldsWriter for EmptyStoredFieldsWriter {
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
        Ok(())
    }

    fn close(&mut self) -> Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_stored_fields_format_name() {
        let format = EmptyStoredFieldsFormat;
        assert_eq!(format.name(), "EmptyStoredFields");
    }

    #[test]
    fn empty_stored_fields_reader_methods() {
        let reader = EmptyStoredFieldsReader;
        reader.check_integrity().unwrap();
        let mut visitor = NullStoredFieldVisitor;
        reader.document(0, &mut visitor).unwrap();
        let _clone = reader.clone_reader();
    }

    #[test]
    fn empty_stored_fields_writer_methods() {
        let mut writer = EmptyStoredFieldsWriter;
        writer.start_document().unwrap();
        let info = FieldInfo::default();
        writer.write_field_i32(&info, 1).unwrap();
        writer.write_field_i64(&info, 2).unwrap();
        writer.write_field_f32(&info, 1.0).unwrap();
        writer.write_field_f64(&info, 2.0).unwrap();
        writer.write_field_bytes(&info, b"x").unwrap();
        writer.write_field_string(&info, "s").unwrap();
        let input_src = vec![1, 2, 3];
        let mut input = crate::store::ByteArrayDataInput::new(&input_src);
        let mut stored_input = StoredFieldDataInput::new(&mut input, 3);
        writer
            .write_field_data_input(&info, &mut stored_input)
            .unwrap();
        writer.finish_document().unwrap();
        writer.finish(1).unwrap();
        writer.close().unwrap();
    }

    struct NullStoredFieldVisitor;

    impl StoredFieldVisitor for NullStoredFieldVisitor {
        fn needs_field(
            &mut self,
            _info: &FieldInfo,
        ) -> Result<super::super::stub::StoredFieldVisitorStatus> {
            Ok(super::super::stub::StoredFieldVisitorStatus::Yes)
        }
    }

    /// A segment whose every document stores its own doc id in field `value`,
    /// and which counts the documents read through this very instance.
    #[derive(Debug)]
    struct CountingStoredFieldsReader {
        field: FieldInfo,
        reads: Arc<std::sync::atomic::AtomicUsize>,
    }

    impl StoredFieldsReader for CountingStoredFieldsReader {
        fn document(&self, doc_id: i32, visitor: &mut dyn StoredFieldVisitor) -> Result<()> {
            self.reads.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if visitor.needs_field(&self.field)? == StoredFieldVisitorStatus::Yes {
                visitor.int_field(&self.field, doc_id)?;
            }
            Ok(())
        }

        fn check_integrity(&self) -> Result<()> {
            Ok(())
        }

        /// A clone counts its reads apart, so a merge that read through a
        /// clone instead of the merge state's own reader is visible.
        fn clone_reader(&self) -> Box<dyn StoredFieldsReader> {
            Box::new(Self {
                field: self.field.clone(),
                reads: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            })
        }
    }

    /// A writer that keeps the default `merge` and records what it was given.
    #[derive(Debug, Default)]
    struct RecordingStoredFieldsWriter {
        values: Vec<i32>,
        finished_with: Option<i32>,
    }

    impl Accountable for RecordingStoredFieldsWriter {
        fn ram_bytes_used(&self) -> i64 {
            0
        }
    }

    impl StoredFieldsWriter for RecordingStoredFieldsWriter {
        fn start_document(&mut self) -> Result<()> {
            Ok(())
        }

        fn write_field_i32(&mut self, _info: &FieldInfo, value: i32) -> Result<()> {
            self.values.push(value);
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

        fn finish(&mut self, num_docs: i32) -> Result<()> {
            self.finished_with = Some(num_docs);
            Ok(())
        }

        fn close(&mut self) -> Result<()> {
            Ok(())
        }
    }

    /// A merge state over two segments of `max_docs` documents, laid end to
    /// end, holding the given stored-fields readers.
    fn two_segment_merge_state(
        readers: Vec<Option<Box<dyn StoredFieldsReader>>>,
        max_docs: Vec<i32>,
    ) -> MergeState {
        let field = FieldInfo::new("value", 0);
        let field_infos = FieldInfos::new(vec![field]).unwrap();
        let mut doc_maps = Vec::new();
        let mut doc_base = 0;
        for &max_doc in &max_docs {
            doc_maps.push(crate::index::merge::identity_doc_map(max_doc, doc_base));
            doc_base += max_doc;
        }
        let n = readers.len();
        MergeState::new(
            doc_maps,
            crate::codecs::tests::test_segment_info("_m", doc_base),
            field_infos.clone(),
            readers,
            (0..n).map(|_| None).collect(),
            (0..n).map(|_| None).collect(),
            (0..n).map(|_| None).collect(),
            (0..n).map(|_| field_infos.clone()).collect(),
            (0..n).map(|_| None).collect(),
            (0..n).map(|_| None).collect(),
            (0..n).map(|_| None).collect(),
            (0..n).map(|_| None).collect(),
            max_docs,
            None,
            false,
            None,
        )
    }

    /// A segment without a stored-fields reader stops the default merge with
    /// an error; it does not merge the other segments and leave its own
    /// documents out.
    ///
    /// `StoredFieldsWriter.merge` calls `storedFieldsReader.checkIntegrity()`
    /// on every slot (`StoredFieldsWriter.java:131-135`), so a missing reader
    /// raises `NullPointerException` there.
    #[test]
    fn the_default_merge_refuses_a_segment_without_a_reader() {
        let reader = CountingStoredFieldsReader {
            field: FieldInfo::new("value", 0),
            reads: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        };
        let merge_state = two_segment_merge_state(vec![Some(Box::new(reader)), None], vec![3, 2]);
        let mut writer = RecordingStoredFieldsWriter::default();
        match writer.merge(&merge_state) {
            Err(crate::error::LuceneError::IllegalState { message, .. }) => assert_eq!(
                message,
                "source segment 1 of the merge has no stored-fields reader"
            ),
            other => panic!("a missing reader must fail the merge, got {other:?}"),
        }
        assert_eq!(
            writer.finished_with, None,
            "the merge stops before any document is written"
        );
        assert!(writer.values.is_empty());
    }

    /// The default merge reads every document through the merge state's own
    /// reader, as Java's `sub.reader` is that object, and writes them in
    /// merged order.
    #[test]
    fn the_default_merge_reads_through_the_merge_states_readers() {
        let reads: Vec<Arc<std::sync::atomic::AtomicUsize>> = (0..2)
            .map(|_| Arc::new(std::sync::atomic::AtomicUsize::new(0)))
            .collect();
        let readers = reads
            .iter()
            .map(|reads| {
                Some(Box::new(CountingStoredFieldsReader {
                    field: FieldInfo::new("value", 0),
                    reads: Arc::clone(reads),
                }) as Box<dyn StoredFieldsReader>)
            })
            .collect();
        let merge_state = two_segment_merge_state(readers, vec![3, 2]);
        let mut writer = RecordingStoredFieldsWriter::default();
        assert_eq!(writer.merge(&merge_state).unwrap(), 5);
        assert_eq!(writer.finished_with, Some(5));
        assert_eq!(writer.values, vec![0, 1, 2, 0, 1]);
        assert_eq!(
            reads
                .iter()
                .map(|reads| reads.load(std::sync::atomic::Ordering::SeqCst))
                .collect::<Vec<_>>(),
            vec![3, 2],
            "every document is read through the reader the merge state holds"
        );
    }
}

// -----------------------------------------------------------------------------
// Merging
// -----------------------------------------------------------------------------

/// Decides whether a sub-reader's field numbers need remapping.
///
/// Equivalent to the loop in `StoredFieldsWriter.MergeVisitor`'s constructor
/// (`StoredFieldsWriter.java:182-193`): when every field of the sub-reader
/// keeps its number and name in the merged `FieldInfos`, the numbers are
/// aligned and each field access can skip the lookup; one mismatch and every
/// field must be looked up by name instead. Returns the merged `FieldInfos` to
/// remap through, or `None` when the numbers are aligned.
///
/// Shared with the codecs whose own `merge` keeps the visitor route for some
/// segments, as `Lucene90CompressingStoredFieldsWriter#merge` does
/// (`Lucene90CompressingStoredFieldsWriter.java:613-615`).
pub(crate) fn merge_visitor_remapper(
    merge_state: &MergeState,
    reader_index: usize,
) -> Option<FieldInfos> {
    let Some(reader_field_infos) = merge_state.field_infos.get(reader_index) else {
        return None;
    };
    for fi in reader_field_infos.iter() {
        match merge_state
            .merge_field_infos
            .field_info_by_number(fi.number)
        {
            Some(other) if other.name == fi.name => {}
            _ => return Some(merge_state.merge_field_infos.clone()),
        }
    }
    None
}

/// One sub-reader's stored fields, as the merger walks them.
///
/// Equivalent to `StoredFieldsWriter.StoredFieldsMergeSub`.
struct StoredFieldsMergeSub {
    doc_map: DocMap,
    /// Index into the merge's reader list, so the reader itself stays outside
    /// the merger and need not be cloned per document.
    reader_index: usize,
    max_doc: i32,
    doc_id: i32,
    mapped_doc_id: i32,
}

impl DocIDMergerSub for StoredFieldsMergeSub {
    fn next_doc(&mut self) -> Result<i32> {
        self.doc_id += 1;
        if self.doc_id == self.max_doc {
            Ok(NO_MORE_DOCS)
        } else {
            Ok(self.doc_id)
        }
    }

    fn doc_map(&self) -> &DocMap {
        &self.doc_map
    }

    fn mapped_doc_id(&self) -> i32 {
        self.mapped_doc_id
    }

    fn set_mapped_doc_id(&mut self, doc_id: i32) {
        self.mapped_doc_id = doc_id;
    }
}

/// Forwards every stored field read from a sub-reader straight to the writer.
///
/// Equivalent to `StoredFieldsWriter.MergeVisitor`
/// (`StoredFieldsWriter.java:176-245`): each typed callback calls the writer's
/// matching `writeField` overload with the field remapped to the merged
/// numbering, and `needsField` always answers `YES`.
///
/// Java's visitor is an inner class that reaches its writer through the
/// enclosing instance and is built once per sub-reader; this one borrows the
/// writer for one document at a time, so callers keep the per-sub-reader state
/// — the remapper from [`merge_visitor_remapper`] — and rebuild the visitor
/// around it for each document.
pub(crate) struct MergeVisitor<'a, W: StoredFieldsWriter + ?Sized> {
    /// The writer every field is forwarded to; Java's enclosing
    /// `StoredFieldsWriter` instance.
    pub(crate) writer: &'a mut W,
    /// The merged `FieldInfos` to look each field up in by name, or `None`
    /// when the sub-reader's numbering is aligned; Java's `remapper` field.
    pub(crate) remapper: Option<&'a FieldInfos>,
}

impl<W: StoredFieldsWriter + ?Sized> MergeVisitor<'_, W> {
    /// Returns the field info to write under, remapping by name when the
    /// numbers are not aligned.
    ///
    /// Equivalent to `MergeVisitor.remap(FieldInfo)`.
    fn remap<'f>(&'f self, field: &'f FieldInfo) -> Result<&'f FieldInfo> {
        match self.remapper {
            None => Ok(field),
            Some(remapper) => remapper.field_info(&field.name).ok_or_else(|| {
                crate::error::LuceneError::illegal_state(format!(
                    "field \"{}\" is absent from the merged field infos",
                    field.name
                ))
            }),
        }
    }
}

impl<W: StoredFieldsWriter + ?Sized> StoredFieldVisitor for MergeVisitor<'_, W> {
    fn binary_field(&mut self, field_info: &FieldInfo, value: &[u8]) -> Result<()> {
        let remapped = self.remap(field_info)?.clone();
        self.writer.write_field_bytes(&remapped, value)
    }

    fn string_field(&mut self, field_info: &FieldInfo, value: &str) -> Result<()> {
        let remapped = self.remap(field_info)?.clone();
        self.writer.write_field_string(&remapped, value)
    }

    fn int_field(&mut self, field_info: &FieldInfo, value: i32) -> Result<()> {
        let remapped = self.remap(field_info)?.clone();
        self.writer.write_field_i32(&remapped, value)
    }

    fn long_field(&mut self, field_info: &FieldInfo, value: i64) -> Result<()> {
        let remapped = self.remap(field_info)?.clone();
        self.writer.write_field_i64(&remapped, value)
    }

    fn float_field(&mut self, field_info: &FieldInfo, value: f32) -> Result<()> {
        let remapped = self.remap(field_info)?.clone();
        self.writer.write_field_f32(&remapped, value)
    }

    fn double_field(&mut self, field_info: &FieldInfo, value: f64) -> Result<()> {
        let remapped = self.remap(field_info)?.clone();
        self.writer.write_field_f64(&remapped, value)
    }

    fn needs_field(&mut self, _field_info: &FieldInfo) -> Result<StoredFieldVisitorStatus> {
        Ok(StoredFieldVisitorStatus::Yes)
    }
}
