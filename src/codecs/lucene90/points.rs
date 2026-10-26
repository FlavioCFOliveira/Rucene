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
//! The writer encodes every field as a block KD-tree through
//! [`BKDWriter`](crate::util::bkd::BKDWriter) — straight from the indexing
//! buffer's mutable tree at flush, through `BKDWriter.merge` for a
//! one-dimensional merge, and through `add`/`finish` otherwise, spilling to the
//! segment's directory past its heap budget — and the reader serves each field
//! through a [`BKDReader`](crate::util::bkd::BKDReader).
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
    check_footer, check_index_header, checksum_entire_file, retrieve_checksum,
    retrieve_checksum_expected_length, write_footer, write_index_header,
};
use crate::codecs::points::{
    IntersectVisitor, PointValues, PointsFormat, PointsReader, PointsWriter, Relation,
};
use crate::codecs::postings::MergeState;
use crate::codecs::state::{SegmentReadState, SegmentWriteState};
use crate::codecs::stub::FieldInfo;
use crate::error::{LuceneError, Result};
use crate::index::{segment_file_name, FieldInfos};
use crate::store::{DataInput, Directory, IndexInput, IndexOutput};
use crate::util::bkd::{BKDConfig, BKDFinalizer, BKDReader, BKDWriter};

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
    /// The segment's directory, which every `BKDWriter` this writer creates
    /// receives for its temporary files (`Lucene90PointsWriter.java:152, 252`).
    directory: Arc<dyn Directory>,
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
            // Java writes `VERSION_CURRENT` whatever `version` the writer was
            // built with (`Lucene90PointsWriter.java:72, 85, 98`); `version`
            // only selects the BKD encoding.
            VERSION_CURRENT,
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
            // Java writes `VERSION_CURRENT` whatever `version` the writer was
            // built with (`Lucene90PointsWriter.java:72, 85, 98`); `version`
            // only selects the BKD encoding.
            VERSION_CURRENT,
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
            // Java writes `VERSION_CURRENT` whatever `version` the writer was
            // built with (`Lucene90PointsWriter.java:72, 85, 98`); `version`
            // only selects the BKD encoding.
            VERSION_CURRENT,
            &state.segment_info.id(),
            &state.segment_suffix,
        )?;

        Ok(Self {
            directory: Arc::clone(&state.directory),
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

impl Lucene90PointsWriter {
    /// Creates the BKD writer for one field of the segment being written.
    ///
    /// Java creates it inline, in `writeField` and in `merge`
    /// (`Lucene90PointsWriter.java:149-157, 249-257`).
    fn bkd_writer(&self, field_info: &FieldInfo, total_point_count: i64) -> Result<BKDWriter> {
        let config = BKDConfig::of(
            field_info.point_dimension_count,
            field_info.point_index_dimension_count,
            field_info.point_num_bytes,
            self.max_points_in_leaf_node,
        )?;
        // `BKDWriter` gets the segment's own directory for its temporary
        // files, which it spills to once a field's points exceed
        // `maxMBSortInHeap` (`Lucene90PointsWriter.java:149-157, 249-257`).
        BKDWriter::new(
            self.max_doc,
            Arc::clone(&self.directory),
            &self.segment_name,
            config,
            self.max_mb_sort_in_heap,
            total_point_count,
            Lucene90PointsFormat::with_version(self.version)?.bkd_version(),
        )
    }

    /// Writes the field number and then runs `finalizer`, or writes nothing
    /// when there is no finalizer because the field ended up with no point.
    ///
    /// Equivalent to the three `if (finalizer != null) { metaOut.writeInt(
    /// fieldInfo.number); finalizer.run(); }` blocks of
    /// `Lucene90PointsWriter` (`:163-166, 190-193, 287-290`).
    fn write_finalizer(
        &mut self,
        field_number: i32,
        finalizer: Option<BKDFinalizer>,
    ) -> Result<()> {
        if let Some(finalizer) = finalizer {
            let meta_out = self
                .meta_out
                .as_mut()
                .expect("INVARIANT: open until finish");
            meta_out.write_int(field_number)?;
            let index_out = self
                .index_out
                .as_mut()
                .expect("INVARIANT: open until finish");
            finalizer.run(meta_out.as_mut(), index_out.as_mut())?;
        }
        Ok(())
    }
}

impl PointsWriter for Lucene90PointsWriter {
    /// Equivalent to `Lucene90PointsWriter.writeField(FieldInfo, PointsReader)`
    /// (`Lucene90PointsWriter.java:137-195`).
    fn write_field(&mut self, field_info: &FieldInfo, reader: &dyn PointsReader) -> Result<()> {
        // Java dereferences `reader.getValues(fieldInfo.name)` unguarded; every
        // caller hands it a reader that has values for the field.
        let values = reader.get_values(&field_info.name)?.ok_or_else(|| {
            LuceneError::IllegalState(format!(
                "field=\"{}\" has no point values to write",
                field_info.name
            ))
        })?;
        let mut tree = values.point_tree()?;

        let mut writer = self.bkd_writer(field_info, tree.size()?)?;

        // Java tests `values instanceof MutablePointTree` and, when it holds,
        // hands the tree straight to `BKDWriter.writeField`, which sorts and
        // partitions the points in place rather than buffering them through
        // `BKDWriter.add` and re-sorting them offline
        // (`Lucene90PointsWriter.java:159-168`). `PointTree::as_mutable` is
        // this port's `instanceof`: it answers `Some` for the indexing
        // buffer's tree and `None` for every other tree.
        if let Some(mutable) = tree.as_mutable() {
            let data_out = self
                .data_out
                .as_mut()
                .expect("INVARIANT: open until finish");
            let finalizer = writer.write_field(data_out.as_mut(), mutable)?;
            self.write_finalizer(field_info.number, finalizer)?;
            return writer.close();
        }

        tree.visit_doc_values(&mut AddToBkdWriter {
            writer: &mut writer,
        })?;

        // We could have 0 points on merge since all docs with dimensional
        // fields may be deleted.
        let data_out = self
            .data_out
            .as_mut()
            .expect("INVARIANT: open until finish");
        let finalizer = writer.finish(data_out.as_mut())?;
        self.write_finalizer(field_info.number, finalizer)?;
        writer.close()
    }

    /// Equivalent to `Lucene90PointsWriter.merge(MergeState)`
    /// (`Lucene90PointsWriter.java:197-299`).
    ///
    /// When every incoming reader is a `Lucene90PointsReader` — a segment with
    /// no points has none, and so does not qualify — each one-dimensional
    /// field is merged by [`BKDWriter::merge`], a single merge sort of the
    /// already sorted segments; every other field takes the default
    /// [`merge_one_field`](PointsWriter::merge_one_field). Otherwise the whole
    /// merge is the default [`PointsWriter`] one.
    fn merge(&mut self, merge_state: &MergeState) -> Result<()> {
        // If index sorting is active and some of the leaves are not sorted,
        // the test below catches it and the non-optimized merge runs. If the
        // readers are all sorted then it is safe to bulk-merge the points.
        let all_lucene90 = merge_state.points_readers.iter().all(|reader| {
            reader
                .as_ref()
                .and_then(|reader| reader.as_any())
                .is_some_and(|any| any.is::<Lucene90PointsReader>())
        });
        if !all_lucene90 {
            // We can only bulk merge when all to-be-merged segments use our
            // format.
            #[cfg(test)]
            for field_info in merge_state.merge_field_infos.iter() {
                if field_info.point_dimension_count != 0 {
                    merge_path_probe::record(&field_info.name, merge_path_probe::DEFAULT_MERGE);
                }
            }
            return merge_default(self, merge_state);
        }
        for reader in merge_state.points_readers.iter().flatten() {
            merge_state.check_aborted()?;
            reader.check_integrity()?;
        }

        for field_info in merge_state.merge_field_infos.iter() {
            if field_info.point_dimension_count == 0 {
                continue;
            }
            if field_info.point_dimension_count != 1 {
                #[cfg(test)]
                merge_path_probe::record(&field_info.name, merge_path_probe::MERGE_ONE_FIELD);
                self.merge_one_field(merge_state, field_info)?;
                continue;
            }

            // Worst case total maximum size (if none of the points are
            // deleted).
            let mut tot_max_size = 0i64;
            for (i, reader) in merge_state.points_readers.iter().enumerate() {
                let Some(reader) = reader else {
                    continue;
                };
                let has_points = merge_state.field_infos[i]
                    .field_info(&field_info.name)
                    .is_some_and(|reader_field_info| reader_field_info.point_dimension_count > 0);
                if has_points {
                    if let Some(values) = reader.get_values(&field_info.name)? {
                        tot_max_size += values.size()?;
                    }
                }
            }

            // Optimize the 1D case to use BKDWriter.merge, which does a single
            // merge sort of the already sorted incoming segments, instead of
            // trying to sort all points again as if we were simply reindexing
            // them.
            let mut writer = self.bkd_writer(field_info, tot_max_size)?;
            let mut point_values: Vec<Box<dyn PointValues>> = Vec::new();
            let mut doc_maps: Vec<crate::index::merge::DocMap> = Vec::new();
            for (i, reader) in merge_state.points_readers.iter().enumerate() {
                let Some(reader) = reader else {
                    continue;
                };
                // NOTE: we cannot just use the merged field number (instead of
                // resolving to this reader's field info as we do below)
                // because field numbers can easily be different when
                // `addIndexes(Directory...)` copies over segments from
                // another index.
                let Some(reader_field_info) =
                    merge_state.field_infos[i].field_info(&field_info.name)
                else {
                    continue;
                };
                if reader_field_info.point_dimension_count > 0 {
                    if let Some(values) = reader.get_values(&reader_field_info.name)? {
                        point_values.push(values);
                        doc_maps.push(Arc::clone(&merge_state.doc_maps[i]));
                    }
                }
            }
            let readers: Vec<&dyn PointValues> = point_values.iter().map(|v| v.as_ref()).collect();
            let data_out = self
                .data_out
                .as_mut()
                .expect("INVARIANT: open until finish");
            #[cfg(test)]
            merge_path_probe::record(&field_info.name, merge_path_probe::BKD_MERGE);
            let finalizer = writer.merge(data_out.as_mut(), Some(&doc_maps), &readers)?;
            self.write_finalizer(field_info.number, finalizer)?;
            writer.close()?;
        }

        self.finish()
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

/// Runs the default [`PointsWriter::merge`] on `writer`: Java's
/// `super.merge(mergeState)` from `Lucene90PointsWriter.merge`
/// (`Lucene90PointsWriter.java:207`).
///
/// A Rust override cannot call the trait's default body it replaces, so the
/// default is reached through a wrapper that forwards every method except
/// `merge`, which it leaves to the trait.
fn merge_default(writer: &mut Lucene90PointsWriter, merge_state: &MergeState) -> Result<()> {
    struct Default<'w>(&'w mut Lucene90PointsWriter);

    impl fmt::Debug for Default<'_> {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            self.0.fmt(f)
        }
    }

    impl PointsWriter for Default<'_> {
        fn write_field(&mut self, field_info: &FieldInfo, values: &dyn PointsReader) -> Result<()> {
            self.0.write_field(field_info, values)
        }

        fn finish(&mut self) -> Result<()> {
            self.0.finish()
        }

        fn close(&mut self) -> Result<()> {
            self.0.close()
        }
    }

    Default(writer).merge(merge_state)
}

/// Which branch of [`Lucene90PointsWriter::merge`] each field took, recorded
/// for the tests only: the bulk `BKDWriter.merge` path and the default
/// re-indexing path write the same bytes for most inputs, so the files alone
/// cannot tell them apart.
#[cfg(test)]
pub(crate) mod merge_path_probe {
    use std::sync::Mutex;

    /// `BKDWriter.merge` (`Lucene90PointsWriter.java:246-291`).
    pub(crate) const BKD_MERGE: &str = "BKDWriter.merge";
    /// `mergeOneField` from the bulk loop (`Lucene90PointsWriter.java:292-294`).
    pub(crate) const MERGE_ONE_FIELD: &str = "mergeOneField";
    /// `super.merge` (`Lucene90PointsWriter.java:204-209`).
    pub(crate) const DEFAULT_MERGE: &str = "PointsWriter.merge";

    static PATHS: Mutex<Vec<(String, &'static str)>> = Mutex::new(Vec::new());

    pub(crate) fn record(field: &str, path: &'static str) {
        PATHS.lock().unwrap().push((field.to_string(), path));
    }

    /// The paths recorded for `field`, in order.
    pub(crate) fn paths_of(field: &str) -> Vec<&'static str> {
        PATHS
            .lock()
            .unwrap()
            .iter()
            .filter(|(name, _)| name == field)
            .map(|&(_, path)| path)
            .collect()
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
    /// Equivalent to `Lucene90PointsReader.checkIntegrity()`
    /// (`Lucene90PointsReader.java:148-152`): checksums the whole index and
    /// data files.
    fn check_integrity(&self) -> Result<()> {
        checksum_entire_file(self.index_in.clone_input()?.as_mut())?;
        checksum_entire_file(self.data_in.clone_input()?.as_mut())?;
        Ok(())
    }

    /// Equivalent to `Lucene90PointsReader.getValues(String)`
    /// (`Lucene90PointsReader.java:135-146`): `None`, Java's `null`, for a
    /// field that indexes points but has no tree in this segment.
    fn get_values(&self, field: &str) -> Result<Option<Box<dyn PointValues>>> {
        let field_info = self.field_infos.field_info(field).ok_or_else(|| {
            LuceneError::IllegalArgument(format!("field=\"{field}\" is unrecognized"))
        })?;
        if field_info.point_dimension_count == 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "field=\"{field}\" did not index point values"
            )));
        }
        Ok(self
            .readers
            .get(&field_info.number)
            .map(|values| Box::new(values.clone()) as Box<dyn PointValues>))
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    /// Returns a reader over the same trees.
    ///
    /// `Lucene90PointsReader` does not override `getMergeInstance()`, so Java
    /// inherits `PointsReader.getMergeInstance()`, which returns `this`
    /// (`PointsReader.java:56-58`). Rust hands out an owned box, so this shares the
    /// per-field trees — each [`BkdPointValues`] clone reads the same bytes —
    /// and clones the two inputs, which is the closest owned equivalent of
    /// returning the reader itself.
    fn get_merge_instance(&self) -> Result<Box<dyn PointsReader>> {
        Ok(Box::new(Self {
            readers: self.readers.clone(),
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

    fn size(&self) -> Result<i64> {
        Ok(self.reader.lock().unwrap().point_count())
    }

    fn doc_count(&self) -> Result<i32> {
        Ok(self.reader.lock().unwrap().doc_count())
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
/// The visitor `Lucene90PointsWriter.writeField` runs over a tree that is not
/// a `MutablePointTree` (`Lucene90PointsWriter.java:170-186`): every value is
/// added to the BKD writer, and every cell is visited.
struct AddToBkdWriter<'a> {
    writer: &'a mut BKDWriter,
}

impl IntersectVisitor for AddToBkdWriter<'_> {
    fn visit(&mut self, _doc_id: i32) -> Result<()> {
        Err(LuceneError::IllegalState(String::new()))
    }

    fn visit_with_value(&mut self, doc_id: i32, packed_value: &[u8]) -> Result<()> {
        self.writer.add(packed_value, doc_id)
    }

    fn compare(&self, _min_packed_value: &[u8], _max_packed_value: &[u8]) -> Result<Relation> {
        Ok(Relation::CellCrossesQuery)
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
    use crate::index::point_values::DocValuesVisitor;
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

    /// A single-node tree with no points, for a fixture holding none.
    #[derive(Debug, Clone, Copy)]
    struct NoPointsTree;

    impl crate::index::point_values::PointTree for NoPointsTree {
        fn clone_tree(&self) -> Result<Box<dyn crate::index::point_values::PointTree>> {
            Ok(Box::new(*self))
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
            Ok(&[])
        }

        fn max_packed_value(&self) -> Result<&[u8]> {
            Ok(&[])
        }

        fn size(&self) -> Result<i64> {
            Ok(0)
        }

        fn visit_doc_ids(
            &mut self,
            _visitor: &mut dyn crate::index::point_values::IntersectVisitor,
        ) -> Result<()> {
            Ok(())
        }

        fn visit_doc_values(
            &mut self,
            _visitor: &mut dyn crate::index::point_values::IntersectVisitor,
        ) -> Result<()> {
            Ok(())
        }
    }

    impl PointValues for VecPointValues {
        fn point_tree(&self) -> Result<Box<dyn crate::index::point_values::PointTree>> {
            if self.points.is_empty() {
                // An in-memory leaf must hold a point; a source with none is a
                // tree with nothing to visit.
                return Ok(Box::new(NoPointsTree));
            }
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

        fn size(&self) -> Result<i64> {
            Ok(self.points.len() as i64)
        }

        fn doc_count(&self) -> Result<i32> {
            let mut docs: std::collections::HashSet<i32> = std::collections::HashSet::new();
            for (doc, _) in &self.points {
                docs.insert(*doc);
            }
            Ok(docs.len() as i32)
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

        fn get_values(&self, field: &str) -> Result<Option<Box<dyn PointValues>>> {
            Ok(self
                .values
                .get(field)
                .map(|v| Box::new(v.clone()) as Box<dyn PointValues>))
        }

        fn get_merge_instance(&self) -> Result<Box<dyn PointsReader>> {
            Ok(Box::new(self.clone()))
        }

        fn close(&mut self) -> Result<()> {
            Ok(())
        }
    }

    struct ProbeFailVisitor<'v> {
        inner: &'v mut dyn crate::index::point_values::IntersectVisitor,
        n: usize,
        dir: Arc<dyn Directory>,
        seen: Arc<Mutex<Vec<String>>>,
    }
    impl crate::index::point_values::IntersectVisitor for ProbeFailVisitor<'_> {
        fn visit(&mut self, doc_id: i32) -> Result<()> {
            self.inner.visit(doc_id)
        }
        fn visit_with_value(&mut self, doc_id: i32, packed_value: &[u8]) -> Result<()> {
            self.n += 1;
            if self.n > 600 {
                // What the directory holds when the source fails: the spill file.
                *self.seen.lock().unwrap() = self.dir.list_all()?;
                return Err(LuceneError::IllegalState("probe: source failed".into()));
            }
            self.inner.visit_with_value(doc_id, packed_value)
        }
        fn compare(&self, a: &[u8], b: &[u8]) -> Result<crate::index::point_values::Relation> {
            self.inner.compare(a, b)
        }
    }
    struct ProbeFailTree {
        inner: Box<dyn crate::index::point_values::PointTree>,
        dir: Arc<dyn Directory>,
        seen: Arc<Mutex<Vec<String>>>,
    }
    impl crate::index::point_values::PointTree for ProbeFailTree {
        fn clone_tree(&self) -> Result<Box<dyn crate::index::point_values::PointTree>> {
            Ok(Box::new(ProbeFailTree {
                inner: self.inner.clone_tree()?,
                dir: Arc::clone(&self.dir),
                seen: Arc::clone(&self.seen),
            }))
        }
        fn move_to_child(&mut self) -> Result<bool> {
            self.inner.move_to_child()
        }
        fn move_to_sibling(&mut self) -> Result<bool> {
            self.inner.move_to_sibling()
        }
        fn move_to_parent(&mut self) -> Result<bool> {
            self.inner.move_to_parent()
        }
        fn min_packed_value(&self) -> Result<&[u8]> {
            self.inner.min_packed_value()
        }
        fn max_packed_value(&self) -> Result<&[u8]> {
            self.inner.max_packed_value()
        }
        fn size(&self) -> Result<i64> {
            self.inner.size()
        }
        fn visit_doc_ids(
            &mut self,
            v: &mut dyn crate::index::point_values::IntersectVisitor,
        ) -> Result<()> {
            self.inner.visit_doc_ids(v)
        }
        fn visit_doc_values(
            &mut self,
            v: &mut dyn crate::index::point_values::IntersectVisitor,
        ) -> Result<()> {
            let mut f = ProbeFailVisitor {
                inner: v,
                n: 0,
                dir: Arc::clone(&self.dir),
                seen: Arc::clone(&self.seen),
            };
            self.inner.visit_doc_values(&mut f)
        }
    }
    #[derive(Clone)]
    struct ProbeFailValues(VecPointValues, Arc<dyn Directory>, Arc<Mutex<Vec<String>>>);
    impl std::fmt::Debug for ProbeFailValues {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("ProbeFailValues")
        }
    }
    impl PointValues for ProbeFailValues {
        fn point_tree(&self) -> Result<Box<dyn crate::index::point_values::PointTree>> {
            Ok(Box::new(ProbeFailTree {
                inner: self.0.point_tree()?,
                dir: Arc::clone(&self.1),
                seen: Arc::clone(&self.2),
            }))
        }
        fn size(&self) -> Result<i64> {
            self.0.size()
        }
        fn doc_count(&self) -> Result<i32> {
            self.0.doc_count()
        }
        fn min_packed_value(&self) -> Result<Option<Vec<u8>>> {
            self.0.min_packed_value()
        }
        fn max_packed_value(&self) -> Result<Option<Vec<u8>>> {
            self.0.max_packed_value()
        }
        fn num_dimensions(&self) -> Result<i32> {
            self.0.num_dimensions()
        }
        fn num_index_dimensions(&self) -> Result<i32> {
            self.0.num_index_dimensions()
        }
        fn bytes_per_dimension(&self) -> Result<i32> {
            self.0.bytes_per_dimension()
        }
    }
    #[derive(Debug)]
    struct ProbeFailReader(ProbeFailValues);
    impl PointsReader for ProbeFailReader {
        fn check_integrity(&self) -> Result<()> {
            Ok(())
        }
        fn get_values(&self, _f: &str) -> Result<Option<Box<dyn PointValues>>> {
            Ok(Some(Box::new(self.0.clone())))
        }
        fn get_merge_instance(&self) -> Result<Box<dyn PointsReader>> {
            unreachable!()
        }
        fn close(&mut self) -> Result<()> {
            Ok(())
        }
    }

    /// Round 3: a source that fails while `BKDWriter.add` is spilling leaves
    /// no temporary file. Java's `try`-with-resources closes the `BKDWriter`
    /// on that exit (`Lucene90PointsWriter.java:149-157`), and `close` deletes
    /// the spill file (`BKDWriter.java:1546-1556`); here dropping the writer
    /// does.
    #[test]
    fn a_failing_source_leaves_no_spill_file() {
        let tmp = tempfile::tempdir().unwrap();
        let dir: Arc<dyn Directory> =
            Arc::new(crate::store::FSDirectory::open(tmp.path()).unwrap());
        let segment_info = test_segment_info("_p", 2000);
        let fi = point_field("p", 0, 2, 2, 4);
        let field_infos = FieldInfos::new(vec![fi.clone()]).unwrap();
        let state = SegmentWriteState::new(
            crate::util::default_info_stream(),
            Arc::clone(&dir),
            &segment_info,
            &field_infos,
            &crate::codecs::stub::BufferedUpdates,
            &*crate::store::DEFAULT_IO_CONTEXT,
        );
        // 2000 points of 12 bytes against 0.001 MB, which holds 87: `add` spills.
        let mut writer =
            Lucene90PointsWriter::with_config(&state, 64, 0.001, VERSION_CURRENT).unwrap();
        let points: Vec<(i32, Vec<u8>)> = (0..2000)
            .map(|i: i32| {
                let mut v = vec![0u8; 8];
                v[..4].copy_from_slice(&((i * 7919) % 1000).to_be_bytes());
                v[4..].copy_from_slice(&((i * 31) % 997).to_be_bytes());
                (i, v)
            })
            .collect();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let reader = ProbeFailReader(ProbeFailValues(
            VecPointValues::new("p", 2, 2, 4, points),
            Arc::clone(&dir),
            Arc::clone(&seen),
        ));
        let error = writer
            .write_field(&fi, &reader)
            .expect_err("the source fails");
        assert!(
            error.to_string().contains("probe: source failed"),
            "{error}"
        );
        assert!(
            seen.lock()
                .unwrap()
                .iter()
                .any(|n| n.contains("_bkd_spill_") && n.ends_with(".tmp")),
            "the writer was spilling when the source failed: {:?}",
            seen.lock().unwrap()
        );
        let _ = writer.close();
        let left: Vec<String> = dir
            .list_all()
            .unwrap()
            .into_iter()
            .filter(|n| n.ends_with(".tmp"))
            .collect();
        assert!(left.is_empty(), "temporary files left behind: {left:?}");
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

    /// rmp #244: Java writes `VERSION_CURRENT` into all three headers whatever
    /// version the writer was built with (`Lucene90PointsWriter.java:72, 85,
    /// 98`); the version only picks the BKD encoding.
    #[test]
    fn headers_carry_version_current_whatever_the_writer_version() {
        let dir: Arc<dyn Directory> = Arc::new(RamDirectory::default());
        let segment_info = test_segment_info("points_version", 10);
        let field_infos = FieldInfos::new(vec![int_point_field("p", 0, 1)]).unwrap();
        let write_state = SegmentWriteState::new(
            crate::util::default_info_stream(),
            Arc::clone(&dir),
            &segment_info,
            &field_infos,
            &crate::codecs::stub::BufferedUpdates,
            &*crate::store::DEFAULT_IO_CONTEXT,
        );
        let mut writer = Lucene90PointsWriter::new(&write_state, VERSION_START).unwrap();
        writer.finish().unwrap();
        for (extension, codec) in [
            (DATA_EXTENSION, DATA_CODEC_NAME),
            (META_EXTENSION, META_CODEC_NAME),
            (INDEX_EXTENSION, INDEX_CODEC_NAME),
        ] {
            let name = segment_file_name("points_version", "", extension);
            let mut input = dir
                .open_input(&name, &*crate::store::DEFAULT_IO_CONTEXT)
                .unwrap();
            let read_be_int = crate::codecs::codec_util::read_be_int;
            assert_eq!(
                read_be_int(input.as_mut()).unwrap(),
                crate::codecs::codec_util::CODEC_MAGIC
            );
            assert_eq!(input.read_string().unwrap(), codec);
            assert_eq!(
                read_be_int(input.as_mut()).unwrap(),
                VERSION_CURRENT,
                "{name}"
            );
        }
    }

    #[test]
    fn writer_reader_round_trip_single_1d_field() {
        let dir: Arc<dyn Directory> = Arc::new(RamDirectory::default());
        let dir_ref: &dyn Directory = &*dir;
        let segment_info = test_segment_info("points_test", 10);
        let field = int_point_field("int_point", 0, 1);
        let field_infos = FieldInfos::new(vec![field.clone()]).unwrap();

        let write_state = SegmentWriteState::new(
            crate::util::default_info_stream(),
            Arc::clone(&dir),
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
        let read_values = points_reader.get_values("int_point").unwrap().unwrap();

        assert_eq!(read_values.size().unwrap(), 6);
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
        let dir: Arc<dyn Directory> = Arc::new(RamDirectory::default());
        let dir_ref: &dyn Directory = &*dir;
        let segment_info = test_segment_info("points_test", 10);

        let f1 = point_field("f1", 0, 2, 2, 4);
        let f2 = point_field("f2", 1, 1, 1, 4);
        let field_infos = FieldInfos::new(vec![f1.clone(), f2.clone()]).unwrap();

        let write_state = SegmentWriteState::new(
            crate::util::default_info_stream(),
            Arc::clone(&dir),
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

        let read_f1 = points_reader.get_values("f1").unwrap().unwrap();
        assert_eq!(read_f1.size().unwrap(), 5);

        let read_f2 = points_reader.get_values("f2").unwrap().unwrap();
        assert_eq!(read_f2.size().unwrap(), 3);

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

    /// Force-merges two segments with a one-dimensional field and a
    /// two-dimensional one, each name unique to the caller.
    /// `third_without_points` adds a third segment that indexes no point,
    /// whose points reader is therefore null.
    fn force_merge_points(one_dim: &str, two_dims: &str, third_without_points: bool) {
        use crate::document::{Document, IntPoint, Store, StringField};
        use crate::index::index_writer::IndexWriter;
        use crate::index::index_writer_config::IndexWriterConfig;

        let _ = crate::codecs::register_codec("Lucene104", crate::codecs::Lucene104Codec::new());
        let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
        let writer = IndexWriter::new(Arc::clone(&dir), IndexWriterConfig::new()).unwrap();
        for segment in 0..2 {
            for i in 0..600 {
                let i = segment * 600 + i;
                let mut doc = Document::new();
                doc.add(Box::new(IntPoint::new(one_dim, &[i * 7 % 1013]).unwrap()));
                doc.add(Box::new(
                    IntPoint::new(two_dims, &[i % 97, i % 89]).unwrap(),
                ));
                writer.add_document(&doc).unwrap();
            }
            writer.commit().unwrap();
        }
        if third_without_points {
            let mut doc = Document::new();
            doc.add(Box::new(
                StringField::new("id", "no-points".to_string(), Store::NO).unwrap(),
            ));
            writer.add_document(&doc).unwrap();
            writer.commit().unwrap();
        }
        writer.force_merge(1).unwrap();
        writer.close().unwrap();
    }

    /// When every source segment has a `Lucene90PointsReader`, a
    /// one-dimensional field merges through `BKDWriter.merge` and a
    /// multi-dimensional one through `mergeOneField`
    /// (`Lucene90PointsWriter.java:204-299`); a source without points sends
    /// every field down the default `PointsWriter.merge`. The two paths write
    /// the same bytes here, so only the probe can tell them apart.
    #[test]
    fn a_merge_takes_the_bkd_merge_path_only_when_every_reader_is_lucene90() {
        use merge_path_probe::{paths_of, BKD_MERGE, DEFAULT_MERGE, MERGE_ONE_FIELD};

        force_merge_points("probe_bulk_1d", "probe_bulk_2d", false);
        assert_eq!(paths_of("probe_bulk_1d"), vec![BKD_MERGE]);
        assert_eq!(paths_of("probe_bulk_2d"), vec![MERGE_ONE_FIELD]);

        force_merge_points("probe_default_1d", "probe_default_2d", true);
        assert_eq!(paths_of("probe_default_1d"), vec![DEFAULT_MERGE]);
        assert_eq!(paths_of("probe_default_2d"), vec![DEFAULT_MERGE]);
    }

    #[test]
    fn empty_point_field_skips_metadata_entry() {
        let dir: Arc<dyn Directory> = Arc::new(RamDirectory::default());
        let dir_ref: &dyn Directory = &*dir;
        let segment_info = test_segment_info("points_test", 10);
        let field = int_point_field("int_point", 0, 1);
        let field_infos = FieldInfos::new(vec![field.clone()]).unwrap();

        let write_state = SegmentWriteState::new(
            crate::util::default_info_stream(),
            Arc::clone(&dir),
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
        // Java's `getValues` answers `null` for a field with no tree.
        assert!(points_reader.get_values("int_point").unwrap().is_none());
    }

    #[test]
    fn single_point_round_trip() {
        let dir: Arc<dyn Directory> = Arc::new(RamDirectory::default());
        let dir_ref: &dyn Directory = &*dir;
        let segment_info = test_segment_info("points_test", 5);
        let field = int_point_field("int_point", 0, 1);
        let field_infos = FieldInfos::new(vec![field.clone()]).unwrap();

        let write_state = SegmentWriteState::new(
            crate::util::default_info_stream(),
            Arc::clone(&dir),
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
        let read_values = points_reader.get_values("int_point").unwrap().unwrap();
        assert_eq!(read_values.size().unwrap(), 1);

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
        let dir: Arc<dyn Directory> = Arc::new(RamDirectory::default());
        let dir_ref: &dyn Directory = &*dir;
        let segment_info = test_segment_info("points_test", 10);
        let field = int_point_field("int_point", 0, 1);
        let field_infos = FieldInfos::new(vec![field.clone()]).unwrap();

        let write_state = SegmentWriteState::new(
            crate::util::default_info_stream(),
            Arc::clone(&dir),
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
        let read_values = points_reader.get_values("int_point").unwrap().unwrap();

        assert_eq!(read_values.size().unwrap(), 4);
        assert_eq!(read_values.doc_count().unwrap(), 3);
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
        let dir: Arc<dyn Directory> = Arc::new(RamDirectory::default());
        let dir_ref: &dyn Directory = &*dir;
        let segment_info = test_segment_info("points_test", 5);
        let field = FieldInfo::new("text", 0);
        let field_infos = FieldInfos::new(vec![field]).unwrap();

        // Create and finish an empty points writer so the segment files exist.
        let write_state = SegmentWriteState::new(
            crate::util::default_info_stream(),
            Arc::clone(&dir),
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

        let dir: Arc<dyn Directory> = Arc::new(RamDirectory::default());
        let dir_ref: &dyn Directory = &*dir;
        let segment_info = test_segment_info("points_test", 20);
        let field = int_point_field("int_point", 0, 1);
        let field_infos = FieldInfos::new(vec![field.clone()]).unwrap();

        let write_state = SegmentWriteState::new(
            crate::util::default_info_stream(),
            Arc::clone(&dir),
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
        let read_values = points_reader.get_values("int_point").unwrap().unwrap();

        struct RangeVisitor {
            min: i32,
            max: i32,
            found: Vec<i32>,
        }

        impl IntersectVisitor for RangeVisitor {
            fn compare(&self, min_packed: &[u8], max_packed: &[u8]) -> Result<Relation> {
                let min_v = BitUtil::read_le_int(min_packed, 0);
                let max_v = BitUtil::read_le_int(max_packed, 0);
                Ok(if max_v < self.min || min_v > self.max {
                    Relation::CellOutsideQuery
                } else if min_v >= self.min && max_v <= self.max {
                    Relation::CellInsideQuery
                } else {
                    Relation::CellCrossesQuery
                })
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

        let dir: Arc<dyn Directory> = Arc::new(RamDirectory::default());
        let dir_ref: &dyn Directory = &*dir;
        let segment_info = test_segment_info("points_test", 20);
        let field = point_field("box_point", 0, 2, 2, 4);
        let field_infos = FieldInfos::new(vec![field.clone()]).unwrap();

        let write_state = SegmentWriteState::new(
            crate::util::default_info_stream(),
            Arc::clone(&dir),
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
        let read_values = points_reader.get_values("box_point").unwrap().unwrap();

        struct BoxVisitor {
            min_x: i32,
            max_x: i32,
            min_y: i32,
            max_y: i32,
            found: Vec<i32>,
        }

        impl IntersectVisitor for BoxVisitor {
            fn compare(&self, min_packed: &[u8], max_packed: &[u8]) -> Result<Relation> {
                let min_x = BitUtil::read_le_int(min_packed, 0);
                let max_x = BitUtil::read_le_int(max_packed, 0);
                let min_y = BitUtil::read_le_int(min_packed, 4);
                let max_y = BitUtil::read_le_int(max_packed, 4);
                Ok(
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
                    },
                )
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
