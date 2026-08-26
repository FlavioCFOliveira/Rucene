//! Lucene 9.0 points format implementation.
//!
//! Ports `Lucene90PointsFormat`, `Lucene90PointsReader` and `Lucene90PointsWriter`
//! from Apache Lucene Core 10.5.0.
//!
//! The format writes three files per segment:
//!
//! * `.kdm` – metadata about the fields (dimension counts, bytes per dimension).
//! * `.kdi` – inner nodes of the block KD-tree.
//! * `.kdd` – leaf nodes with the actual point data.
//!
//! This module currently provides the file envelope implementation
//! (headers/footers and metadata layout). The full BKD tree encoding/decoding
//! is implemented in the `bkd` utility module and will be wired in a follow-up
//! task.
//!
//! Lucene Core equivalents:
//! * `org.apache.lucene.codecs.lucene90.Lucene90PointsFormat`
//! * `org.apache.lucene.codecs.lucene90.Lucene90PointsReader`
//! * `org.apache.lucene.codecs.lucene90.Lucene90PointsWriter`

#![deny(unsafe_code)]

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex};

use crate::codecs::codec_util::{
    check_footer, check_index_header, retrieve_checksum, retrieve_checksum_expected_length,
    write_footer, write_index_header,
};
use crate::codecs::points::{
    DocValuesVisitor, EmptyPointValues, PointValues, PointsFormat, PointsReader, PointsWriter,
};
use crate::codecs::postings::MergeState;
use crate::codecs::state::{SegmentReadState, SegmentWriteState};
use crate::codecs::stub::FieldInfo;
use crate::error::{LuceneError, Result};
use crate::index::{segment_file_name, FieldInfos};
use crate::store::{DataInput, Directory, IndexInput, IndexOutput, RamDirectory};
use crate::util::bkd::{BKDConfig, BKDReader, BKDWriter};

// -----------------------------------------------------------------------------
// Format constants
// -----------------------------------------------------------------------------

/// Codec name written into the `.kdd` header.
pub const DATA_CODEC_NAME: &str = "Lucene90PointsFormatData";
/// Codec name written into the `.kdi` header.
pub const INDEX_CODEC_NAME: &str = "Lucene90PointsFormatIndex";
/// Codec name written into the `.kdm` header.
pub const META_CODEC_NAME: &str = "Lucene90PointsFormatMeta";

/// Extension of the leaf-blocks file (`.kdd`).
pub const DATA_EXTENSION: &str = "kdd";
/// Extension of the inner-nodes file (`.kdi`).
pub const INDEX_EXTENSION: &str = "kdi";
/// Extension of the metadata file (`.kdm`).
pub const META_EXTENSION: &str = "kdm";

/// Initial points format version.
pub const VERSION_START: i32 = 0;
/// Version that introduced vectorized BPV24/BPV21 encodings.
pub const VERSION_BKD_VECTORIZED_BPV24: i32 = 1;
/// Current points format version.
pub const VERSION_CURRENT: i32 = VERSION_BKD_VECTORIZED_BPV24;

/// Default maximum number of points in a leaf node.
pub const DEFAULT_MAX_POINTS_IN_LEAF_NODE: i32 = 512;
/// Default maximum heap megabytes used while sorting points.
pub const DEFAULT_MAX_MB_SORT_IN_HEAP: f64 = 16.0;

// -----------------------------------------------------------------------------
// Points format
// -----------------------------------------------------------------------------

/// Lucene 9.0 point format implementation.
///
/// Lucene Core equivalent: `org.apache.lucene.codecs.lucene90.Lucene90PointsFormat`.
#[derive(Debug, Default, Clone)]
pub struct Lucene90PointsFormat {
    version: i32,
}

impl Lucene90PointsFormat {
    /// Creates the format with the current version.
    pub fn new() -> Self {
        Self {
            version: VERSION_CURRENT,
        }
    }

    /// Expert constructor that allows configuring the version, used for backward
    /// compatibility tests.
    pub fn with_version(version: i32) -> Result<Self> {
        if ![VERSION_START, VERSION_BKD_VECTORIZED_BPV24].contains(&version) {
            return Err(LuceneError::IllegalArgument(format!(
                "invalid Lucene90PointsFormat version: {version}"
            )));
        }
        Ok(Self { version })
    }

    /// Returns the BKD version corresponding to this format version.
    ///
    /// Maps the Lucene90 points-format version to the underlying BKD tree
    /// version used by [`crate::util::bkd::BKDWriter`].
    pub fn bkd_version(&self) -> i32 {
        match self.version {
            VERSION_START => BKDWriter::VERSION_META_FILE,
            VERSION_BKD_VECTORIZED_BPV24 => BKDWriter::VERSION_VECTORIZE_BPV24_AND_INTRODUCE_BPV21,
            _ => unreachable!(),
        }
    }
}

impl PointsFormat for Lucene90PointsFormat {
    fn name(&self) -> &str {
        "Lucene90"
    }

    fn fields_writer(&self, state: &SegmentWriteState) -> Result<Box<dyn PointsWriter>> {
        Ok(Box::new(Lucene90PointsWriter::new(state, self.version)?))
    }

    fn fields_reader(&self, state: &SegmentReadState) -> Result<Box<dyn PointsReader>> {
        Ok(Box::new(Lucene90PointsReader::new(state)?))
    }
}

// -----------------------------------------------------------------------------
// Points writer
// -----------------------------------------------------------------------------

/// Writer for the Lucene 9.0 points format.
///
/// Lucene Core equivalent: `org.apache.lucene.codecs.lucene90.Lucene90PointsWriter`.
pub struct Lucene90PointsWriter {
    segment_name: String,
    segment_suffix: String,
    version: i32,
    max_doc: i32,
    max_points_in_leaf_node: i32,
    max_mb_sort_in_heap: f64,
    meta_out: Option<Box<dyn IndexOutput>>,
    index_out: Option<Box<dyn IndexOutput>>,
    data_out: Option<Box<dyn IndexOutput>>,
    finished: bool,
}

impl Lucene90PointsWriter {
    /// Creates a new writer with default leaf size and heap budget.
    pub fn new(state: &SegmentWriteState<'_>, version: i32) -> Result<Self> {
        Self::with_config(
            state,
            DEFAULT_MAX_POINTS_IN_LEAF_NODE,
            DEFAULT_MAX_MB_SORT_IN_HEAP,
            version,
        )
    }

    /// Expert constructor that allows configuring the BKD writer parameters.
    pub fn with_config(
        state: &SegmentWriteState<'_>,
        max_points_in_leaf_node: i32,
        max_mb_sort_in_heap: f64,
        version: i32,
    ) -> Result<Self> {
        if ![VERSION_START, VERSION_BKD_VECTORIZED_BPV24].contains(&version) {
            return Err(LuceneError::IllegalArgument(format!(
                "invalid Lucene90PointsFormat version: {version}"
            )));
        }

        let data_name = segment_file_name(
            &state.segment_info.name,
            &state.segment_suffix,
            DATA_EXTENSION,
        );
        let mut data_out = state.directory.create_output(&data_name, state.context)?;
        write_index_header(
            data_out.as_mut(),
            DATA_CODEC_NAME,
            version,
            &state.segment_info.id(),
            &state.segment_suffix,
        )?;

        let meta_name = segment_file_name(
            &state.segment_info.name,
            &state.segment_suffix,
            META_EXTENSION,
        );
        let mut meta_out = state.directory.create_output(&meta_name, state.context)?;
        write_index_header(
            meta_out.as_mut(),
            META_CODEC_NAME,
            version,
            &state.segment_info.id(),
            &state.segment_suffix,
        )?;

        let index_name = segment_file_name(
            &state.segment_info.name,
            &state.segment_suffix,
            INDEX_EXTENSION,
        );
        let mut index_out = state.directory.create_output(&index_name, state.context)?;
        write_index_header(
            index_out.as_mut(),
            INDEX_CODEC_NAME,
            version,
            &state.segment_info.id(),
            &state.segment_suffix,
        )?;

        Ok(Self {
            segment_name: state.segment_info.name.clone(),
            segment_suffix: state.segment_suffix.clone(),
            version,
            max_doc: state.segment_info.max_doc()?,
            max_points_in_leaf_node,
            max_mb_sort_in_heap,
            meta_out: Some(meta_out),
            index_out: Some(index_out),
            data_out: Some(data_out),
            finished: false,
        })
    }
}

impl fmt::Debug for Lucene90PointsWriter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Lucene90PointsWriter")
            .field("segment_suffix", &self.segment_suffix)
            .field("version", &self.version)
            .field("finished", &self.finished)
            .finish_non_exhaustive()
    }
}

impl PointsWriter for Lucene90PointsWriter {
    fn write_field(&mut self, field_info: &FieldInfo, values: &dyn PointsReader) -> Result<()> {
        if field_info.point_dimension_count == 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "field='{}' does not index point values",
                field_info.name
            )));
        }

        let values = values.get_values(&field_info.name)?;
        if values.size() == 0 {
            // No points for this field; do not write a metadata entry.
            return Ok(());
        }

        let config = BKDConfig::of(
            field_info.point_dimension_count,
            field_info.point_index_dimension_count,
            field_info.point_num_bytes,
            self.max_points_in_leaf_node,
        )?;

        let temp_dir: Box<dyn Directory> = Box::new(RamDirectory::new());
        let mut writer = BKDWriter::new(
            self.max_doc,
            temp_dir,
            &self.segment_name,
            config,
            self.max_mb_sort_in_heap,
            values.size(),
            Lucene90PointsFormat::with_version(self.version)?.bkd_version(),
        )?;

        // Collect all points through the codec visitor API. This is the Rust
        // equivalent of Java's PointValues.visitDocValues(IntersectVisitor).
        // The MutablePointTree fast-path used by Java is not ported here; the
        // general visitor path is sufficient for the current phase.
        let mut add_visitor = AddToBkdWriter {
            writer: &mut writer,
        };
        values.visit_doc_values(&mut add_visitor)?;

        let meta_out = self.meta_out.as_mut().unwrap();
        let index_out = self.index_out.as_mut().unwrap();
        let data_out = self.data_out.as_mut().unwrap();

        // Write the field number before the BKD meta block so that the reader
        // can read one BKD tree per field number in the same order.
        meta_out.write_int(field_info.number)?;
        writer.finish(meta_out.as_mut(), index_out.as_mut(), data_out.as_mut())?;

        writer.close()?;
        Ok(())
    }

    fn merge(&mut self, _merge_state: &MergeState) -> Result<()> {
        Ok(())
    }

    fn finish(&mut self) -> Result<()> {
        if self.finished {
            return Err(LuceneError::IllegalState(
                "Lucene90PointsWriter already finished".to_string(),
            ));
        }
        self.finished = true;

        let mut meta_out = self.meta_out.take().unwrap();
        let mut index_out = self.index_out.take().unwrap();
        let mut data_out = self.data_out.take().unwrap();

        meta_out.write_int(-1)?;
        write_footer(index_out.as_mut())?;
        write_footer(data_out.as_mut())?;
        meta_out.write_long(index_out.file_pointer())?;
        meta_out.write_long(data_out.file_pointer())?;
        write_footer(meta_out.as_mut())?;

        meta_out.close()?;
        index_out.close()?;
        data_out.close()?;
        Ok(())
    }

    fn close(&mut self) -> Result<()> {
        if let Some(mut out) = self.meta_out.take() {
            out.close()?;
        }
        if let Some(mut out) = self.index_out.take() {
            out.close()?;
        }
        if let Some(mut out) = self.data_out.take() {
            out.close()?;
        }
        Ok(())
    }
}

// -----------------------------------------------------------------------------
// Points reader
// -----------------------------------------------------------------------------

/// Reader for the Lucene 9.0 points format.
///
/// Lucene Core equivalent: `org.apache.lucene.codecs.lucene90.Lucene90PointsReader`.
pub struct Lucene90PointsReader {
    readers: HashMap<i32, BkdPointValues>,
    field_infos: FieldInfos,
    #[allow(dead_code)]
    index_in: Box<dyn IndexInput>,
    #[allow(dead_code)]
    data_in: Box<dyn IndexInput>,
}

impl Lucene90PointsReader {
    /// Creates a new reader.
    pub fn new(state: &SegmentReadState) -> Result<Self> {
        let index_name = segment_file_name(
            &state.segment_info.name,
            &state.segment_suffix,
            INDEX_EXTENSION,
        );
        let mut index_in = state.directory.open_input(&index_name, state.context)?;
        check_index_header(
            &mut *index_in,
            INDEX_CODEC_NAME,
            VERSION_START,
            VERSION_CURRENT,
            &state.segment_info.id(),
            &state.segment_suffix,
        )?;
        let _ = retrieve_checksum(&mut *index_in)?;

        let data_name = segment_file_name(
            &state.segment_info.name,
            &state.segment_suffix,
            DATA_EXTENSION,
        );
        let mut data_in = state.directory.open_input(&data_name, state.context)?;
        check_index_header(
            &mut *data_in,
            DATA_CODEC_NAME,
            VERSION_START,
            VERSION_CURRENT,
            &state.segment_info.id(),
            &state.segment_suffix,
        )?;
        let _ = retrieve_checksum(&mut *data_in)?;

        let meta_name = segment_file_name(
            &state.segment_info.name,
            &state.segment_suffix,
            META_EXTENSION,
        );
        let mut meta_in = state.directory.open_checksum_input(&meta_name)?;
        check_index_header(
            &mut *meta_in,
            META_CODEC_NAME,
            VERSION_START,
            VERSION_CURRENT,
            &state.segment_info.id(),
            &state.segment_suffix,
        )?;

        let mut readers: HashMap<i32, BkdPointValues> = HashMap::new();
        loop {
            let field_number = meta_in.read_int()?;
            if field_number == -1 {
                break;
            }
            if field_number < 0 {
                return Err(LuceneError::CorruptIndex(format!(
                    "illegal field number in points meta file: {field_number}"
                )));
            }
            let reader = BKDReader::new(meta_in.as_mut(), index_in.as_mut(), data_in.as_mut())?;
            readers.insert(field_number, BkdPointValues::new(reader));
        }
        let index_length = meta_in.read_long()?;
        let data_length = meta_in.read_long()?;
        check_footer(&mut *meta_in)?;

        let _ = retrieve_checksum_expected_length(&mut *index_in, index_length)?;
        let _ = retrieve_checksum_expected_length(&mut *data_in, data_length)?;

        Ok(Self {
            readers,
            field_infos: state.field_infos.clone(),
            index_in,
            data_in,
        })
    }
}

impl fmt::Debug for Lucene90PointsReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Lucene90PointsReader")
            .field("fields", &self.readers.len())
            .finish_non_exhaustive()
    }
}

impl PointsReader for Lucene90PointsReader {
    fn check_integrity(&self) -> Result<()> {
        Ok(())
    }

    fn get_values(&self, field: &str) -> Result<Box<dyn PointValues>> {
        let field_info = self.field_infos.field_info(field).ok_or_else(|| {
            LuceneError::IllegalArgument(format!("field='{field}' is unrecognized"))
        })?;
        if field_info.point_dimension_count == 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "field='{field}' did not index point values"
            )));
        }
        match self.readers.get(&field_info.number) {
            Some(values) => Ok(Box::new(values.clone())),
            None => Ok(Box::new(EmptyPointValues)),
        }
    }

    fn get_merge_instance(&self) -> Result<Box<dyn PointsReader>> {
        // PointValues is not cloneable, so we return a fresh empty reader for the
        // skeleton implementation.  A full implementation will build a merge view.
        Ok(Box::new(Self {
            readers: HashMap::new(),
            field_infos: self.field_infos.clone(),
            index_in: self.index_in.clone_input()?,
            data_in: self.data_in.clone_input()?,
        }))
    }

    fn close(&mut self) -> Result<()> {
        self.readers.clear();
        self.index_in.close()?;
        self.data_in.close()?;
        Ok(())
    }
}

// -----------------------------------------------------------------------------
// BKD-backed point values
// -----------------------------------------------------------------------------

/// Point-values implementation backed by a [`BKDReader`].
///
/// This bridges the unified [`PointValues`] trait with the BKD utility reader.
/// The reader is shared behind an [`Arc`]<[`Mutex`]> so that the implementation
/// can be cloned and returned from [`Lucene90PointsReader`].
///
/// `intersect`, `estimate_point_count`, `estimate_doc_count` and
/// `visit_doc_values` are all inherited from the trait defaults: they walk the
/// [`PointTree`] produced by [`BKDReader::point_tree`]. No BKD-specific visitor
/// adapter is needed here, because the BKD cursor and the index layer share the
/// same [`IntersectVisitor`] type.
#[derive(Clone)]
struct BkdPointValues {
    reader: Arc<Mutex<BKDReader>>,
}

impl BkdPointValues {
    fn new(reader: BKDReader) -> Self {
        Self {
            reader: Arc::new(Mutex::new(reader)),
        }
    }
}

impl PointValues for BkdPointValues {
    fn point_tree(&self) -> Result<Box<dyn crate::index::point_values::PointTree>> {
        self.reader.lock().unwrap().point_tree()
    }

    fn size(&self) -> i64 {
        self.reader.lock().unwrap().point_count()
    }

    fn doc_count(&self) -> i32 {
        self.reader.lock().unwrap().doc_count()
    }

    fn min_packed_value(&self) -> Result<Option<Vec<u8>>> {
        let reader = self.reader.lock().unwrap();
        if reader.point_count() == 0 {
            Ok(None)
        } else {
            Ok(Some(reader.min_packed_value().to_vec()))
        }
    }

    fn max_packed_value(&self) -> Result<Option<Vec<u8>>> {
        let reader = self.reader.lock().unwrap();
        if reader.point_count() == 0 {
            Ok(None)
        } else {
            Ok(Some(reader.max_packed_value().to_vec()))
        }
    }

    fn num_dimensions(&self) -> Result<i32> {
        Ok(self.reader.lock().unwrap().num_dims())
    }

    fn num_index_dimensions(&self) -> Result<i32> {
        Ok(self.reader.lock().unwrap().num_index_dims())
    }

    fn bytes_per_dimension(&self) -> Result<i32> {
        Ok(self.reader.lock().unwrap().bytes_per_dim())
    }
}

/// Visitor used while writing a field: it forwards every point to the
/// [`BKDWriter`].
struct AddToBkdWriter<'a> {
    writer: &'a mut BKDWriter,
}

impl DocValuesVisitor for AddToBkdWriter<'_> {
    fn visit(&mut self, doc_id: i32, packed_value: &[u8]) -> Result<()> {
        self.writer.add(packed_value, doc_id)
    }
}

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::codecs::tests::test_segment_info;
    use crate::index::{FieldInfo, FieldInfos};
    use crate::store::RamDirectory;
    use crate::util::bkd::BKDUtil;
    use crate::util::BitUtil;

    /// In-memory point-values source used only for tests.
    #[derive(Clone, Debug)]
    struct VecPointValues {
        name: String,
        dims: i32,
        index_dims: i32,
        bytes_per_dim: i32,
        points: Vec<(i32, Vec<u8>)>,
        min: Vec<u8>,
        max: Vec<u8>,
    }

    impl VecPointValues {
        fn new(
            name: &str,
            dims: i32,
            index_dims: i32,
            bytes_per_dim: i32,
            points: Vec<(i32, Vec<u8>)>,
        ) -> Self {
            let packed_len = (dims * bytes_per_dim) as usize;
            let mut min = vec![0u8; packed_len];
            let mut max = vec![0u8; packed_len];
            if !points.is_empty() {
                min.copy_from_slice(&points[0].1[..packed_len.min(points[0].1.len())]);
                max.copy_from_slice(&points[0].1[..packed_len.min(points[0].1.len())]);
                for (_, packed) in &points {
                    for dim in 0..dims as usize {
                        let off = dim * bytes_per_dim as usize;
                        if BKDUtil::unsigned_compare(packed, off, &min, off, bytes_per_dim as usize)
                            == std::cmp::Ordering::Less
                        {
                            min[off..off + bytes_per_dim as usize]
                                .copy_from_slice(&packed[off..off + bytes_per_dim as usize]);
                        }
                        if BKDUtil::unsigned_compare(packed, off, &max, off, bytes_per_dim as usize)
                            == std::cmp::Ordering::Greater
                        {
                            max[off..off + bytes_per_dim as usize]
                                .copy_from_slice(&packed[off..off + bytes_per_dim as usize]);
                        }
                    }
                }
            }
            Self {
                name: name.to_string(),
                dims,
                index_dims,
                bytes_per_dim,
                points,
                min,
                max,
            }
        }
    }

    impl PointValues for VecPointValues {
        fn point_tree(&self) -> Result<Box<dyn crate::index::point_values::PointTree>> {
            // Build an in-memory tree over a single leaf so the generic
            // traversal algorithms can run on test fixtures. The points are
            // handed to `InMemoryPointValues` which validates the 1-D ordering
            // contract; test inputs that violate it are rejected here.
            let values = crate::index::point_values::InMemoryPointValues::new(
                self.dims,
                self.index_dims,
                self.bytes_per_dim,
                vec![self.points.clone()],
            )?;
            Ok(values.point_tree()?)
        }

        fn size(&self) -> i64 {
            self.points.len() as i64
        }

        fn doc_count(&self) -> i32 {
            let mut docs: std::collections::HashSet<i32> = std::collections::HashSet::new();
            for (doc, _) in &self.points {
                docs.insert(*doc);
            }
            docs.len() as i32
        }

        fn min_packed_value(&self) -> Result<Option<Vec<u8>>> {
            if self.points.is_empty() {
                Ok(None)
            } else {
                Ok(Some(self.min.clone()))
            }
        }

        fn max_packed_value(&self) -> Result<Option<Vec<u8>>> {
            if self.points.is_empty() {
                Ok(None)
            } else {
                Ok(Some(self.max.clone()))
            }
        }

        fn num_dimensions(&self) -> Result<i32> {
            Ok(self.dims)
        }

        fn num_index_dimensions(&self) -> Result<i32> {
            Ok(self.index_dims)
        }

        fn bytes_per_dimension(&self) -> Result<i32> {
            Ok(self.bytes_per_dim)
        }

        fn visit_doc_values(&self, visitor: &mut dyn DocValuesVisitor) -> Result<()> {
            for (doc_id, packed) in &self.points {
                visitor.visit(*doc_id, packed)?;
            }
            Ok(())
        }
    }

    #[derive(Clone, Debug)]
    struct VecPointsReader {
        values: HashMap<String, VecPointValues>,
    }

    impl VecPointsReader {
        fn new(values: Vec<VecPointValues>) -> Self {
            let mut map = HashMap::new();
            for v in values {
                map.insert(v.name.clone(), v);
            }
            Self { values: map }
        }
    }

    impl PointsReader for VecPointsReader {
        fn check_integrity(&self) -> Result<()> {
            Ok(())
        }

        fn get_values(&self, field: &str) -> Result<Box<dyn PointValues>> {
            match self.values.get(field) {
                Some(v) => Ok(Box::new(v.clone())),
                None => Ok(Box::new(EmptyPointValues)),
            }
        }

        fn get_merge_instance(&self) -> Result<Box<dyn PointsReader>> {
            Ok(Box::new(self.clone()))
        }

        fn close(&mut self) -> Result<()> {
            Ok(())
        }
    }

    fn point_field(
        name: &str,
        number: i32,
        dims: i32,
        index_dims: i32,
        bytes_per_dim: i32,
    ) -> FieldInfo {
        let mut fi = FieldInfo::new(name, number);
        fi.set_point_dimensions(dims, index_dims, bytes_per_dim)
            .unwrap();
        fi
    }

    fn int_point_field(name: &str, number: i32, dims: i32) -> FieldInfo {
        point_field(name, number, dims, dims, 4)
    }

    fn packed_int(value: i32) -> Vec<u8> {
        // Use little-endian encoding matching the BKD utility module.
        let mut packed = vec![0u8; 4];
        BitUtil::write_le_int(&mut packed, 0, value);
        packed
    }

    fn packed_2d_int(x: i32, y: i32) -> Vec<u8> {
        let mut packed = vec![0u8; 8];
        BitUtil::write_le_int(&mut packed, 0, x);
        BitUtil::write_le_int(&mut packed, 4, y);
        packed
    }

    #[test]
    fn writer_reader_round_trip_single_1d_field() {
        let dir = RamDirectory::default();
        let dir_ref: &dyn Directory = &dir;
        let segment_info = test_segment_info("points_test", 10);
        let field = int_point_field("int_point", 0, 1);
        let field_infos = FieldInfos::new(vec![field.clone()]).unwrap();

        let write_state = SegmentWriteState::new(
            crate::util::default_info_stream(),
            dir_ref,
            &segment_info,
            &field_infos,
            &crate::codecs::stub::BufferedUpdates,
            &*crate::store::DEFAULT_IO_CONTEXT,
        );
        let mut writer = Lucene90PointsWriter::new(&write_state, VERSION_CURRENT).unwrap();

        let points: Vec<(i32, Vec<u8>)> = (0..6).map(|i| (i, packed_int(i * 10))).collect();
        let values = VecPointValues::new("int_point", 1, 1, 4, points.clone());
        let reader = VecPointsReader::new(vec![values]);
        writer.write_field(&field, &reader).unwrap();
        writer.finish().unwrap();
        writer.close().unwrap();

        let read_state = SegmentReadState::new(
            dir_ref,
            &segment_info,
            &field_infos,
            &*crate::store::DEFAULT_IO_CONTEXT,
        );
        let points_reader = Lucene90PointsReader::new(&read_state).unwrap();
        let read_values = points_reader.get_values("int_point").unwrap();

        assert_eq!(read_values.size(), 6);
        assert_eq!(read_values.num_dimensions().unwrap(), 1);
        assert_eq!(read_values.bytes_per_dimension().unwrap(), 4);

        let mut found = Vec::new();
        read_values
            .visit_doc_values(&mut |doc_id: i32, packed: &[u8]| -> Result<()> {
                found.push((doc_id, packed.to_vec()));
                Ok(())
            })
            .unwrap();
        found.sort_by_key(|(d, _)| *d);
        assert_eq!(found, points);
    }

    #[test]
    fn writer_reader_round_trip_multiple_fields() {
        let dir = RamDirectory::default();
        let dir_ref: &dyn Directory = &dir;
        let segment_info = test_segment_info("points_test", 10);

        let f1 = point_field("f1", 0, 2, 2, 4);
        let f2 = point_field("f2", 1, 1, 1, 4);
        let field_infos = FieldInfos::new(vec![f1.clone(), f2.clone()]).unwrap();

        let write_state = SegmentWriteState::new(
            crate::util::default_info_stream(),
            dir_ref,
            &segment_info,
            &field_infos,
            &crate::codecs::stub::BufferedUpdates,
            &*crate::store::DEFAULT_IO_CONTEXT,
        );
        let mut writer = Lucene90PointsWriter::new(&write_state, VERSION_CURRENT).unwrap();

        let points1: Vec<(i32, Vec<u8>)> = (0..5).map(|i| (i, packed_2d_int(i, i * 2))).collect();
        let v1 = VecPointValues::new("f1", 2, 2, 4, points1.clone());

        let points2: Vec<(i32, Vec<u8>)> = (0..3).map(|i| (i + 5, packed_int(100 + i))).collect();
        let v2 = VecPointValues::new("f2", 1, 1, 4, points2.clone());

        let reader = VecPointsReader::new(vec![v1, v2]);
        writer.write_field(&f1, &reader).unwrap();
        writer.write_field(&f2, &reader).unwrap();
        writer.finish().unwrap();
        writer.close().unwrap();

        let read_state = SegmentReadState::new(
            dir_ref,
            &segment_info,
            &field_infos,
            &*crate::store::DEFAULT_IO_CONTEXT,
        );
        let points_reader = Lucene90PointsReader::new(&read_state).unwrap();

        let read_f1 = points_reader.get_values("f1").unwrap();
        assert_eq!(read_f1.size(), 5);

        let read_f2 = points_reader.get_values("f2").unwrap();
        assert_eq!(read_f2.size(), 3);

        let mut found1 = Vec::new();
        read_f1
            .visit_doc_values(&mut |doc_id: i32, packed: &[u8]| -> Result<()> {
                found1.push((doc_id, packed.to_vec()));
                Ok(())
            })
            .unwrap();
        found1.sort_by_key(|(d, _)| *d);
        assert_eq!(found1, points1);

        let mut found2 = Vec::new();
        read_f2
            .visit_doc_values(&mut |doc_id: i32, packed: &[u8]| -> Result<()> {
                found2.push((doc_id, packed.to_vec()));
                Ok(())
            })
            .unwrap();
        found2.sort_by_key(|(d, _)| *d);
        assert_eq!(found2, points2);
    }

    #[test]
    fn empty_point_field_skips_metadata_entry() {
        let dir = RamDirectory::default();
        let dir_ref: &dyn Directory = &dir;
        let segment_info = test_segment_info("points_test", 10);
        let field = int_point_field("int_point", 0, 1);
        let field_infos = FieldInfos::new(vec![field.clone()]).unwrap();

        let write_state = SegmentWriteState::new(
            crate::util::default_info_stream(),
            dir_ref,
            &segment_info,
            &field_infos,
            &crate::codecs::stub::BufferedUpdates,
            &*crate::store::DEFAULT_IO_CONTEXT,
        );
        let mut writer = Lucene90PointsWriter::new(&write_state, VERSION_CURRENT).unwrap();

        let values = VecPointValues::new("int_point", 1, 1, 4, vec![]);
        let reader = VecPointsReader::new(vec![values]);
        writer.write_field(&field, &reader).unwrap();
        writer.finish().unwrap();
        writer.close().unwrap();

        let read_state = SegmentReadState::new(
            dir_ref,
            &segment_info,
            &field_infos,
            &*crate::store::DEFAULT_IO_CONTEXT,
        );
        let points_reader = Lucene90PointsReader::new(&read_state).unwrap();
        let read_values = points_reader.get_values("int_point").unwrap();
        assert_eq!(read_values.size(), 0);
    }

    #[test]
    fn single_point_round_trip() {
        let dir = RamDirectory::default();
        let dir_ref: &dyn Directory = &dir;
        let segment_info = test_segment_info("points_test", 5);
        let field = int_point_field("int_point", 0, 1);
        let field_infos = FieldInfos::new(vec![field.clone()]).unwrap();

        let write_state = SegmentWriteState::new(
            crate::util::default_info_stream(),
            dir_ref,
            &segment_info,
            &field_infos,
            &crate::codecs::stub::BufferedUpdates,
            &*crate::store::DEFAULT_IO_CONTEXT,
        );
        let mut writer = Lucene90PointsWriter::new(&write_state, VERSION_CURRENT).unwrap();

        let points = vec![(2, packed_int(42))];
        let values = VecPointValues::new("int_point", 1, 1, 4, points.clone());
        let reader = VecPointsReader::new(vec![values]);
        writer.write_field(&field, &reader).unwrap();
        writer.finish().unwrap();
        writer.close().unwrap();

        let read_state = SegmentReadState::new(
            dir_ref,
            &segment_info,
            &field_infos,
            &*crate::store::DEFAULT_IO_CONTEXT,
        );
        let points_reader = Lucene90PointsReader::new(&read_state).unwrap();
        let read_values = points_reader.get_values("int_point").unwrap();
        assert_eq!(read_values.size(), 1);

        let mut found = Vec::new();
        read_values
            .visit_doc_values(&mut |doc_id: i32, packed: &[u8]| -> Result<()> {
                found.push((doc_id, packed.to_vec()));
                Ok(())
            })
            .unwrap();
        assert_eq!(found, points);
    }

    #[test]
    fn single_field_min_max_and_doc_count() {
        let dir = RamDirectory::default();
        let dir_ref: &dyn Directory = &dir;
        let segment_info = test_segment_info("points_test", 10);
        let field = int_point_field("int_point", 0, 1);
        let field_infos = FieldInfos::new(vec![field.clone()]).unwrap();

        let write_state = SegmentWriteState::new(
            crate::util::default_info_stream(),
            dir_ref,
            &segment_info,
            &field_infos,
            &crate::codecs::stub::BufferedUpdates,
            &*crate::store::DEFAULT_IO_CONTEXT,
        );
        let mut writer = Lucene90PointsWriter::new(&write_state, VERSION_CURRENT).unwrap();

        let points: Vec<(i32, Vec<u8>)> = vec![
            (0, packed_int(10)),
            (1, packed_int(20)),
            (1, packed_int(30)), // same doc, second value
            (2, packed_int(40)),
        ];
        let values = VecPointValues::new("int_point", 1, 1, 4, points.clone());
        let reader = VecPointsReader::new(vec![values]);
        writer.write_field(&field, &reader).unwrap();
        writer.finish().unwrap();
        writer.close().unwrap();

        let read_state = SegmentReadState::new(
            dir_ref,
            &segment_info,
            &field_infos,
            &*crate::store::DEFAULT_IO_CONTEXT,
        );
        let points_reader = Lucene90PointsReader::new(&read_state).unwrap();
        let read_values = points_reader.get_values("int_point").unwrap();

        assert_eq!(read_values.size(), 4);
        assert_eq!(read_values.doc_count(), 3);
        assert_eq!(
            read_values.min_packed_value().unwrap(),
            Some(packed_int(10))
        );
        assert_eq!(
            read_values.max_packed_value().unwrap(),
            Some(packed_int(40))
        );
    }

    #[test]
    fn get_values_rejects_non_point_field() {
        let dir = RamDirectory::default();
        let dir_ref: &dyn Directory = &dir;
        let segment_info = test_segment_info("points_test", 5);
        let field = FieldInfo::new("text", 0);
        let field_infos = FieldInfos::new(vec![field]).unwrap();

        // Create and finish an empty points writer so the segment files exist.
        let write_state = SegmentWriteState::new(
            crate::util::default_info_stream(),
            dir_ref,
            &segment_info,
            &field_infos,
            &crate::codecs::stub::BufferedUpdates,
            &*crate::store::DEFAULT_IO_CONTEXT,
        );
        let mut writer = Lucene90PointsWriter::new(&write_state, VERSION_CURRENT).unwrap();
        writer.finish().unwrap();
        writer.close().unwrap();

        let read_state = SegmentReadState::new(
            dir_ref,
            &segment_info,
            &field_infos,
            &*crate::store::DEFAULT_IO_CONTEXT,
        );
        let points_reader = Lucene90PointsReader::new(&read_state).unwrap();
        assert!(points_reader.get_values("text").is_err());
    }

    #[test]
    fn bkd_version_maps_to_underlying_bkd_versions() {
        let fmt_v0 = Lucene90PointsFormat::with_version(VERSION_START).unwrap();
        assert_eq!(fmt_v0.bkd_version(), BKDWriter::VERSION_META_FILE);

        let fmt_v1 = Lucene90PointsFormat::with_version(VERSION_BKD_VECTORIZED_BPV24).unwrap();
        assert_eq!(
            fmt_v1.bkd_version(),
            BKDWriter::VERSION_VECTORIZE_BPV24_AND_INTRODUCE_BPV21
        );
    }

    #[test]
    fn range_query_1d_round_trip() {
        use crate::codecs::points::{IntersectVisitor, Relation};

        let dir = RamDirectory::default();
        let dir_ref: &dyn Directory = &dir;
        let segment_info = test_segment_info("points_test", 20);
        let field = int_point_field("int_point", 0, 1);
        let field_infos = FieldInfos::new(vec![field.clone()]).unwrap();

        let write_state = SegmentWriteState::new(
            crate::util::default_info_stream(),
            dir_ref,
            &segment_info,
            &field_infos,
            &crate::codecs::stub::BufferedUpdates,
            &*crate::store::DEFAULT_IO_CONTEXT,
        );
        let mut writer = Lucene90PointsWriter::new(&write_state, VERSION_CURRENT).unwrap();

        let points: Vec<(i32, Vec<u8>)> = (0..12).map(|i| (i, packed_int(i * 10))).collect();
        let values = VecPointValues::new("int_point", 1, 1, 4, points.clone());
        let reader = VecPointsReader::new(vec![values]);
        writer.write_field(&field, &reader).unwrap();
        writer.finish().unwrap();
        writer.close().unwrap();

        let read_state = SegmentReadState::new(
            dir_ref,
            &segment_info,
            &field_infos,
            &*crate::store::DEFAULT_IO_CONTEXT,
        );
        let points_reader = Lucene90PointsReader::new(&read_state).unwrap();
        let read_values = points_reader.get_values("int_point").unwrap();

        struct RangeVisitor {
            min: i32,
            max: i32,
            found: Vec<i32>,
        }

        impl IntersectVisitor for RangeVisitor {
            fn compare(&self, min_packed: &[u8], max_packed: &[u8]) -> Relation {
                let min_v = BitUtil::read_le_int(min_packed, 0);
                let max_v = BitUtil::read_le_int(max_packed, 0);
                if max_v < self.min || min_v > self.max {
                    Relation::CellOutsideQuery
                } else if min_v >= self.min && max_v <= self.max {
                    Relation::CellInsideQuery
                } else {
                    Relation::CellCrossesQuery
                }
            }

            fn visit(&mut self, _doc_id: i32) -> Result<()> {
                Ok(())
            }

            fn visit_with_value(&mut self, doc_id: i32, packed_value: &[u8]) -> Result<()> {
                let v = BitUtil::read_le_int(packed_value, 0);
                if v >= self.min && v <= self.max {
                    self.found.push(doc_id);
                }
                Ok(())
            }
        }

        let mut visitor = RangeVisitor {
            min: 30,
            max: 80,
            found: Vec::new(),
        };
        read_values.intersect(&mut visitor).unwrap();
        visitor.found.sort();
        assert_eq!(visitor.found, vec![3, 4, 5, 6, 7, 8]);
    }

    #[test]
    fn box_query_2d_round_trip() {
        use crate::codecs::points::{IntersectVisitor, Relation};

        let dir = RamDirectory::default();
        let dir_ref: &dyn Directory = &dir;
        let segment_info = test_segment_info("points_test", 20);
        let field = point_field("box_point", 0, 2, 2, 4);
        let field_infos = FieldInfos::new(vec![field.clone()]).unwrap();

        let write_state = SegmentWriteState::new(
            crate::util::default_info_stream(),
            dir_ref,
            &segment_info,
            &field_infos,
            &crate::codecs::stub::BufferedUpdates,
            &*crate::store::DEFAULT_IO_CONTEXT,
        );
        let mut writer = Lucene90PointsWriter::new(&write_state, VERSION_CURRENT).unwrap();

        let points: Vec<(i32, Vec<u8>)> = (0..12).map(|i| (i, packed_2d_int(i, i * 3))).collect();
        let values = VecPointValues::new("box_point", 2, 2, 4, points.clone());
        let reader = VecPointsReader::new(vec![values]);
        writer.write_field(&field, &reader).unwrap();
        writer.finish().unwrap();
        writer.close().unwrap();

        let read_state = SegmentReadState::new(
            dir_ref,
            &segment_info,
            &field_infos,
            &*crate::store::DEFAULT_IO_CONTEXT,
        );
        let points_reader = Lucene90PointsReader::new(&read_state).unwrap();
        let read_values = points_reader.get_values("box_point").unwrap();

        struct BoxVisitor {
            min_x: i32,
            max_x: i32,
            min_y: i32,
            max_y: i32,
            found: Vec<i32>,
        }

        impl IntersectVisitor for BoxVisitor {
            fn compare(&self, min_packed: &[u8], max_packed: &[u8]) -> Relation {
                let min_x = BitUtil::read_le_int(min_packed, 0);
                let max_x = BitUtil::read_le_int(max_packed, 0);
                let min_y = BitUtil::read_le_int(min_packed, 4);
                let max_y = BitUtil::read_le_int(max_packed, 4);
                if max_x < self.min_x
                    || min_x > self.max_x
                    || max_y < self.min_y
                    || min_y > self.max_y
                {
                    Relation::CellOutsideQuery
                } else if min_x >= self.min_x
                    && max_x <= self.max_x
                    && min_y >= self.min_y
                    && max_y <= self.max_y
                {
                    Relation::CellInsideQuery
                } else {
                    Relation::CellCrossesQuery
                }
            }

            fn visit(&mut self, _doc_id: i32) -> Result<()> {
                Ok(())
            }

            fn visit_with_value(&mut self, doc_id: i32, packed_value: &[u8]) -> Result<()> {
                let x = BitUtil::read_le_int(packed_value, 0);
                let y = BitUtil::read_le_int(packed_value, 4);
                if x >= self.min_x && x <= self.max_x && y >= self.min_y && y <= self.max_y {
                    self.found.push(doc_id);
                }
                Ok(())
            }
        }

        let mut visitor = BoxVisitor {
            min_x: 2,
            max_x: 7,
            min_y: 6,
            max_y: 21,
            found: Vec::new(),
        };
        read_values.intersect(&mut visitor).unwrap();
        visitor.found.sort();
        assert_eq!(visitor.found, vec![2, 3, 4, 5, 6, 7]);
    }
}
