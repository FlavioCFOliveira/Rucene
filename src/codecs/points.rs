//! Points format base traits.
//!
//! Equivalent to `org.apache.lucene.codecs.PointsFormat`, `PointsReader` and
//! `PointsWriter`.
//!
//! These traits provide the abstract read/write API for indexed point values.
//! Concrete codecs implement the format-specific encoding underneath this API.

#![deny(unsafe_code)]

use std::fmt;
use std::sync::Arc;

use crate::error::{LuceneError, Result};

// The codec layer speaks the same visitor language as the index layer; Java's
// `PointsReader.getValues` likewise returns an `org.apache.lucene.index.PointValues`
// whose `intersect` takes `PointValues.IntersectVisitor`. The codec re-exports the
// single canonical `PointValues`/`IntersectVisitor`/`Relation`/`DocValuesVisitor`
// definitions from the index layer.
pub use crate::index::point_values::{
    DocValuesVisitor, IntersectVisitor, MutablePointTree, PointTree, PointValues, Relation,
};

use super::postings::MergeState;
use super::state::{SegmentReadState, SegmentWriteState};
use super::stub::FieldInfo;

// -----------------------------------------------------------------------------
// Reader
// -----------------------------------------------------------------------------

/// Reads point values from an index.
///
/// Equivalent to `org.apache.lucene.codecs.PointsReader`.
pub trait PointsReader: Send + Sync + fmt::Debug {
    /// Checks consistency of this reader.
    fn check_integrity(&self) -> Result<()>;

    /// Returns the point values for the given field, or `None` — Java's
    /// `null` — when this segment has none for it.
    ///
    /// Equivalent to `PointsReader.getValues(String)`. A field that indexes
    /// points can still have none in a segment, for instance in one merged
    /// from documents whose points were all deleted; every caller tests for
    /// that (`PointsWriter.java:49-52, 135-138`, `CodecReader.getPointValues`).
    fn get_values(&self, field: &str) -> Result<Option<Box<dyn PointValues>>>;

    /// Returns an instance optimized for merging.
    fn get_merge_instance(&self) -> Result<Box<dyn PointsReader>>;

    /// Returns this reader as [`std::any::Any`], for the one caller that has to
    /// recognise a concrete reader type: `Lucene90PointsWriter.merge`, which
    /// can only bulk-merge from readers of its own format.
    ///
    /// **Divergence from Lucene 10.5.0.** Java writes `reader instanceof
    /// Lucene90PointsReader` (`Lucene90PointsWriter.java:204-210`). Rust has
    /// no `instanceof` over a trait object, so the concrete type is reached
    /// through `Any`, as `StoredFieldsReader::as_any` does. The default answers
    /// `None`, which reads as "not a `Lucene90PointsReader`" and sends the
    /// caller down the same default merge Java takes for a foreign reader.
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        None
    }

    /// Closes this reader, releasing all resources.
    fn close(&mut self) -> Result<()>;
}

// -----------------------------------------------------------------------------
// Writer
// -----------------------------------------------------------------------------

/// Writes point values to an index.
///
/// Equivalent to `org.apache.lucene.codecs.PointsWriter`.
pub trait PointsWriter: Send + Sync + fmt::Debug {
    /// Writes all values contained in the provided reader for one field.
    fn write_field(&mut self, field_info: &FieldInfo, values: &dyn PointsReader) -> Result<()>;

    /// Merges one field by re-indexing every value of every incoming segment.
    ///
    /// Equivalent to the default `PointsWriter.mergeOneField(MergeState,
    /// FieldInfo)` (`PointsWriter.java:42-215`): it counts the values every
    /// reader holds for the field, deleted documents included, and hands
    /// [`write_field`](Self::write_field) a reader whose single-node tree
    /// visits each segment's values in turn, mapping every document through
    /// that segment's doc map and skipping the deleted ones. The default codec
    /// overrides the merge of one-dimensional fields with a faster one.
    ///
    /// # Errors
    ///
    /// Propagates whatever reading the sources or writing the field raises.
    fn merge_one_field(&mut self, merge_state: &MergeState, field_info: &FieldInfo) -> Result<()> {
        let mut max_point_count = 0i64;
        for (i, points_reader) in merge_state.points_readers.iter().enumerate() {
            let Some(points_reader) = points_reader else {
                continue;
            };
            let has_points = merge_state.field_infos[i]
                .field_info(&field_info.name)
                .is_some_and(|reader_field_info| reader_field_info.point_dimension_count > 0);
            if has_points {
                if let Some(values) = points_reader.get_values(&field_info.name)? {
                    max_point_count += values.size()?;
                }
            }
        }
        let merged = MergedPointsReader {
            field_name: field_info.name.clone(),
            max_point_count,
            merge_state: Arc::new(merge_state.clone()),
        };
        self.write_field(field_info, &merged)
    }

    /// Merges the point values of the readers in `merge_state`.
    ///
    /// Equivalent to the default `PointsWriter.merge(MergeState)`
    /// (`PointsWriter.java:221-236`): every incoming reader is
    /// integrity-checked, each field that indexes points is merged through
    /// [`merge_one_field`](Self::merge_one_field), and the writer is
    /// [`finish`](Self::finish)ed.
    ///
    /// # Errors
    ///
    /// Propagates whatever the integrity checks, the merges or `finish` raise.
    fn merge(&mut self, merge_state: &MergeState) -> Result<()> {
        // Check each incoming reader.
        for reader in merge_state.points_readers.iter().flatten() {
            merge_state.check_aborted()?;
            reader.check_integrity()?;
        }
        // Merge field at a time.
        for field_info in merge_state.merge_field_infos.iter() {
            if field_info.point_dimension_count != 0 {
                self.merge_one_field(merge_state, field_info)?;
            }
        }
        self.finish()
    }

    /// Called once at the end before close.
    fn finish(&mut self) -> Result<()>;

    /// Closes this writer, releasing all resources.
    fn close(&mut self) -> Result<()>;
}

// -----------------------------------------------------------------------------
// Default merge view
// -----------------------------------------------------------------------------

/// The reader the default [`PointsWriter::merge_one_field`] hands to
/// [`PointsWriter::write_field`]: one field, whose values are every incoming
/// segment's.
///
/// Equivalent to the anonymous `PointsReader` of `PointsWriter.mergeOneField`
/// (`PointsWriter.java:59-214`). It holds the merge state the anonymous class
/// captures — a [`Clone`] of it, which shares every reader and doc map.
struct MergedPointsReader {
    field_name: String,
    max_point_count: i64,
    merge_state: Arc<MergeState>,
}

impl fmt::Debug for MergedPointsReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MergedPointsReader")
            .field("field", &self.field_name)
            .field("max_point_count", &self.max_point_count)
            .finish_non_exhaustive()
    }
}

/// What Java throws `UnsupportedOperationException` for in the merged view.
fn unsupported(method: &str) -> LuceneError {
    LuceneError::UnsupportedOperation(format!(
        "{method} is not supported by the merged points view"
    ))
}

impl PointsReader for MergedPointsReader {
    /// Java throws `UnsupportedOperationException` (`PointsWriter.java:210-213`).
    fn check_integrity(&self) -> Result<()> {
        Err(unsupported("checkIntegrity"))
    }

    fn get_values(&self, field: &str) -> Result<Option<Box<dyn PointValues>>> {
        if field != self.field_name {
            return Err(LuceneError::IllegalArgument(
                "field name must match the field being merged".to_string(),
            ));
        }
        Ok(Some(Box::new(MergedPointValues {
            field_name: self.field_name.clone(),
            max_point_count: self.max_point_count,
            merge_state: Arc::clone(&self.merge_state),
        })))
    }

    /// The anonymous reader does not override `getMergeInstance()`, so Java
    /// inherits `PointsReader.getMergeInstance()`, which returns `this`; the
    /// owned equivalent is a view over the same field and merge state.
    fn get_merge_instance(&self) -> Result<Box<dyn PointsReader>> {
        Ok(Box::new(MergedPointsReader {
            field_name: self.field_name.clone(),
            max_point_count: self.max_point_count,
            merge_state: Arc::clone(&self.merge_state),
        }))
    }

    fn close(&mut self) -> Result<()> {
        Ok(())
    }
}

/// The values of the merged view: Java's anonymous `PointValues` of
/// `PointsWriter.mergeOneField` (`PointsWriter.java:70-207`), which answers
/// only `size()` and `getPointTree()`.
struct MergedPointValues {
    field_name: String,
    max_point_count: i64,
    merge_state: Arc<MergeState>,
}

impl PointValues for MergedPointValues {
    fn point_tree(&self) -> Result<Box<dyn PointTree>> {
        Ok(Box::new(MergedPointTree {
            field_name: self.field_name.clone(),
            max_point_count: self.max_point_count,
            merge_state: Arc::clone(&self.merge_state),
        }))
    }

    fn size(&self) -> Result<i64> {
        Ok(self.max_point_count)
    }

    fn doc_count(&self) -> Result<i32> {
        Err(unsupported("getDocCount"))
    }

    fn min_packed_value(&self) -> Result<Option<Vec<u8>>> {
        Err(unsupported("getMinPackedValue"))
    }

    fn max_packed_value(&self) -> Result<Option<Vec<u8>>> {
        Err(unsupported("getMaxPackedValue"))
    }

    fn num_dimensions(&self) -> Result<i32> {
        Err(unsupported("getNumDimensions"))
    }

    fn num_index_dimensions(&self) -> Result<i32> {
        Err(unsupported("getNumIndexDimensions"))
    }

    fn bytes_per_dimension(&self) -> Result<i32> {
        Err(unsupported("getBytesPerDimension"))
    }
}

/// The single-node tree of the merged view: Java's anonymous `PointTree` of
/// `PointsWriter.mergeOneField` (`PointsWriter.java:74-170`). It is its own
/// root and only leaf; visiting its values visits every incoming segment's.
struct MergedPointTree {
    field_name: String,
    max_point_count: i64,
    merge_state: Arc<MergeState>,
}

impl PointTree for MergedPointTree {
    fn clone_tree(&self) -> Result<Box<dyn PointTree>> {
        Err(unsupported("PointTree.clone"))
    }

    fn move_to_child(&mut self) -> Result<bool> {
        Ok(false)
    }

    fn move_to_sibling(&mut self) -> Result<bool> {
        Ok(false)
    }

    fn move_to_parent(&mut self) -> Result<bool> {
        Ok(false)
    }

    fn min_packed_value(&self) -> Result<&[u8]> {
        Err(unsupported("PointTree.getMinPackedValue"))
    }

    fn max_packed_value(&self) -> Result<&[u8]> {
        Err(unsupported("PointTree.getMaxPackedValue"))
    }

    fn size(&self) -> Result<i64> {
        Ok(self.max_point_count)
    }

    fn visit_doc_ids(&mut self, _visitor: &mut dyn IntersectVisitor) -> Result<()> {
        Err(unsupported("PointTree.visitDocIDs"))
    }

    /// Equivalent to the anonymous tree's `visitDocValues(IntersectVisitor)`
    /// (`PointsWriter.java:117-169`): every segment that has values for the
    /// field is visited in reader order, and each value reaches
    /// `merged_visitor` under the document's merged id — or not at all when
    /// the document is deleted.
    fn visit_doc_values(&mut self, merged_visitor: &mut dyn IntersectVisitor) -> Result<()> {
        let merge_state = &self.merge_state;
        for (i, points_reader) in merge_state.points_readers.iter().enumerate() {
            let Some(points_reader) = points_reader else {
                // This segment has no points.
                continue;
            };
            let Some(reader_field_info) = merge_state.field_infos[i].field_info(&self.field_name)
            else {
                // This segment never saw this field.
                continue;
            };
            if reader_field_info.point_dimension_count == 0 {
                // This segment saw this field, but the field did not index
                // points in it.
                continue;
            }
            let Some(values) = points_reader.get_values(&self.field_name)? else {
                continue;
            };
            let mut visitor = DocMapVisitor {
                doc_map: &merge_state.doc_maps[i],
                merged_visitor: &mut *merged_visitor,
            };
            values.point_tree()?.visit_doc_values(&mut visitor)?;
        }
        Ok(())
    }
}

/// The visitor `PointsWriter.mergeOneField` runs over one segment's values
/// (`PointsWriter.java:143-167`): it forwards every value of a live document to
/// the merged visitor under its merged id, and asks to see every cell.
struct DocMapVisitor<'a> {
    doc_map: &'a crate::index::merge::DocMap,
    merged_visitor: &'a mut dyn IntersectVisitor,
}

impl IntersectVisitor for DocMapVisitor<'_> {
    fn visit(&mut self, _doc_id: i32) -> Result<()> {
        // Should never be called during `visit_doc_values`.
        Err(LuceneError::IllegalState(String::new()))
    }

    fn visit_with_value(&mut self, doc_id: i32, packed_value: &[u8]) -> Result<()> {
        let new_doc_id = (self.doc_map)(doc_id);
        if new_doc_id != -1 {
            // Not deleted.
            self.merged_visitor
                .visit_with_value(new_doc_id, packed_value)?;
        }
        Ok(())
    }

    fn compare(&self, _min_packed_value: &[u8], _max_packed_value: &[u8]) -> Result<Relation> {
        // Forces this segment's points reader to always visit all docs and
        // values.
        Ok(Relation::CellCrossesQuery)
    }
}

// -----------------------------------------------------------------------------
// Format
// -----------------------------------------------------------------------------

/// Encodes and decodes point values.
///
/// Equivalent to `org.apache.lucene.codecs.PointsFormat`.
pub trait PointsFormat: Send + Sync + fmt::Debug {
    /// Returns this format's SPI name.
    fn name(&self) -> &str;

    /// Returns a writer to write points to the index.
    fn fields_writer(&self, state: &SegmentWriteState) -> Result<Box<dyn PointsWriter>>;

    /// Returns a reader to read points from the index.
    fn fields_reader(&self, state: &SegmentReadState) -> Result<Box<dyn PointsReader>>;
}

// -----------------------------------------------------------------------------
// No-op implementations
// -----------------------------------------------------------------------------

/// A points reader over a segment that has nothing indexed with points.
///
/// Equivalent to the anonymous `PointsReader` that `PointsFormat.EMPTY`'s
/// `fieldsReader(SegmentReadState)` returns (`codecs/PointsFormat.java:55-68`):
/// `close` and `checkIntegrity` do nothing, and `getValues` rejects every field
/// with an `IllegalArgumentException`, because no field of such a segment was
/// indexed with points.
#[derive(Debug, Default, Clone)]
pub struct EmptyPointsReader;

impl PointsReader for EmptyPointsReader {
    fn check_integrity(&self) -> Result<()> {
        Ok(())
    }

    /// Equivalent to `PointsFormat.EMPTY.fieldsReader(...).getValues(String)`
    /// (`codecs/PointsFormat.java:63-66`).
    ///
    /// # Errors
    ///
    /// Always returns [`LuceneError::IllegalArgument`].
    fn get_values(&self, field: &str) -> Result<Option<Box<dyn PointValues>>> {
        Err(LuceneError::IllegalArgument(format!(
            "field=\"{field}\" was not indexed with points"
        )))
    }

    fn get_merge_instance(&self) -> Result<Box<dyn PointsReader>> {
        Ok(Box::new(self.clone()))
    }

    fn close(&mut self) -> Result<()> {
        Ok(())
    }
}

/// The writer of a codec that indexes no points.
///
/// `PointsFormat.EMPTY` throws `UnsupportedOperationException` instead of
/// handing out a writer (`codecs/PointsFormat.java:49-52`); this writer refuses
/// every field with the same error.
#[derive(Debug, Default, Clone)]
pub struct EmptyPointsWriter;

impl PointsWriter for EmptyPointsWriter {
    /// # Errors
    ///
    /// Always returns [`LuceneError::UnsupportedOperation`].
    fn write_field(&mut self, _field_info: &FieldInfo, _values: &dyn PointsReader) -> Result<()> {
        Err(LuceneError::UnsupportedOperation(
            "EmptyPointsWriter indexes no points".to_string(),
        ))
    }

    fn finish(&mut self) -> Result<()> {
        Ok(())
    }

    fn close(&mut self) -> Result<()> {
        Ok(())
    }
}

/// A points format that indexes nothing.
///
/// Equivalent to `PointsFormat.EMPTY` (`codecs/PointsFormat.java:46-70`).
#[derive(Debug, Default, Clone)]
pub struct EmptyPointsFormat {
    name: String,
}

impl EmptyPointsFormat {
    /// Creates a new no-op points format with the given SPI name.
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }
}

impl PointsFormat for EmptyPointsFormat {
    fn name(&self) -> &str {
        &self.name
    }

    /// Equivalent to `PointsFormat.EMPTY.fieldsWriter(SegmentWriteState)`
    /// (`codecs/PointsFormat.java:49-52`).
    ///
    /// # Errors
    ///
    /// Always returns [`LuceneError::UnsupportedOperation`].
    fn fields_writer(&self, _state: &SegmentWriteState) -> Result<Box<dyn PointsWriter>> {
        Err(LuceneError::UnsupportedOperation(
            "PointsFormat.EMPTY has no writer".to_string(),
        ))
    }

    fn fields_reader(&self, _state: &SegmentReadState) -> Result<Box<dyn PointsReader>> {
        Ok(Box::new(EmptyPointsReader))
    }
}

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codecs::stub::BufferedUpdates;
    use crate::index::FieldInfos;
    use std::sync::Arc;

    /// `PointsFormat.EMPTY`'s reader rejects every field
    /// (`codecs/PointsFormat.java:63-66`); it never hands out values.
    #[test]
    fn empty_points_reader_rejects_every_field() {
        let mut reader = EmptyPointsReader;
        match reader.get_values("field") {
            Err(LuceneError::IllegalArgument(message)) => {
                assert_eq!(message, "field=\"field\" was not indexed with points");
            }
            Err(other) => panic!("expected IllegalArgument, got {other:?}"),
            Ok(_) => panic!("expected IllegalArgument, got values"),
        }
        reader.check_integrity().unwrap();
        let _merge = reader.get_merge_instance().unwrap();
        reader.close().unwrap();
    }

    #[test]
    fn empty_points_writer_refuses_every_field() {
        let mut writer = EmptyPointsWriter;
        let reader = EmptyPointsReader;
        let field = FieldInfo::default();
        assert!(matches!(
            writer.write_field(&field, &reader),
            Err(LuceneError::UnsupportedOperation(_))
        ));
        writer.finish().unwrap();
        writer.close().unwrap();
    }

    #[test]
    fn empty_points_format_name_and_factories() {
        let format = EmptyPointsFormat::new("EmptyPoints");
        assert_eq!(format.name(), "EmptyPoints");

        let dir: Arc<dyn crate::store::Directory> = Arc::new(crate::store::RamDirectory::default());
        let dir_ref: &dyn crate::store::Directory = &*dir;
        let context = &*crate::store::DEFAULT_IO_CONTEXT;
        let field_infos = FieldInfos::default();
        let segment_info = crate::codecs::tests::test_segment_info("test", 10);
        let write_state = SegmentWriteState::new(
            crate::util::default_info_stream(),
            Arc::clone(&dir),
            &segment_info,
            &field_infos,
            &BufferedUpdates,
            context,
        );
        // `PointsFormat.EMPTY` has no writer (`codecs/PointsFormat.java:49-52`).
        assert!(matches!(
            format.fields_writer(&write_state),
            Err(LuceneError::UnsupportedOperation(_))
        ));

        let read_state = SegmentReadState::new(dir_ref, &segment_info, &field_infos, context);
        let _reader = format.fields_reader(&read_state).unwrap();
    }
}
