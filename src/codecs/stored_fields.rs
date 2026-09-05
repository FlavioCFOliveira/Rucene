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
    fn fields_writer(
        &self,
        directory: &dyn Directory,
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
    /// Equivalent to `StoredFieldsWriter.merge(MergeState)`. The documents come
    /// out in merged order through a [`DocIDMerger`], each one read back through
    /// its own reader and written straight out again; the return value is the
    /// number of documents written, which the caller checks against the merged
    /// segment's `max_doc`.
    fn merge(&mut self, merge_state: &MergeState) -> Result<i32> {
        // The readers stay outside the merger: the sub carries only the index
        // of the one it walks, so the reader can be used while the merger is
        // borrowed, and no reader is cloned per document.
        let mut readers: Vec<Box<dyn StoredFieldsReader>> = Vec::new();
        let mut remappers: Vec<Option<FieldInfos>> = Vec::new();
        let mut subs: Vec<StoredFieldsMergeSub> = Vec::new();
        for (i, reader) in merge_state.stored_fields_readers.iter().enumerate() {
            let Some(reader) = reader.as_ref() else {
                continue;
            };
            reader.check_integrity()?;
            subs.push(StoredFieldsMergeSub {
                doc_map: Arc::clone(&merge_state.doc_maps[i]),
                reader_index: readers.len(),
                max_doc: merge_state.max_docs[i],
                doc_id: -1,
                mapped_doc_id: -1,
            });
            readers.push(reader.clone_reader());
            remappers.push(merge_visitor_remapper(merge_state, i));
        }

        let mut merger = DocIDMerger::new(subs, merge_state.needs_index_sort)?;
        let mut doc_count = 0i32;
        while let Some((reader_index, doc_id)) =
            merger.next_sub()?.map(|sub| (sub.reader_index, sub.doc_id))
        {
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
        _directory: &dyn Directory,
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
}

// -----------------------------------------------------------------------------
// Merging
// -----------------------------------------------------------------------------

/// Decides whether a sub-reader's field numbers need remapping.
///
/// Equivalent to the loop in `StoredFieldsWriter.MergeVisitor`'s constructor:
/// when every field of the sub-reader keeps its number and name in the merged
/// `FieldInfos`, the numbers are aligned and each field access can skip the
/// lookup; one mismatch and every field must be looked up by name instead.
fn merge_visitor_remapper(merge_state: &MergeState, reader_index: usize) -> Option<FieldInfos> {
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
/// Equivalent to `StoredFieldsWriter.MergeVisitor`, which Java also makes an
/// `IndexableField` so it can hand itself to `writeField`. This port's writer
/// takes the value directly, one method per type, so the visitor forwards the
/// callback instead of impersonating a field.
struct MergeVisitor<'a, W: StoredFieldsWriter + ?Sized> {
    writer: &'a mut W,
    remapper: Option<&'a FieldInfos>,
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
                crate::error::LuceneError::IllegalState(format!(
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
