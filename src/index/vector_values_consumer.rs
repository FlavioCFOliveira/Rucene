//! The KNN-vectors half of the per-segment indexing pipeline.
//!
//! | Rucene | Apache Lucene Core 10.5.0 |
//! | --- | --- |
//! | [`VectorValuesConsumer`] | `org.apache.lucene.index.VectorValuesConsumer` |
//!
//! # Macro objective
//!
//! Own the KNN-vectors stream of one segment: give every field that declares
//! vector dimensions somewhere to put its vectors, and, when the segment
//! flushes, drive the codec's vectors writer to completion so the segment gains
//! its vector files — with the default codec, the `.vec` and `.vemf` of the
//! flat format and the `.vex` and `.vem` of the HNSW graph on top of it.
//!
//! # Responsibility boundary
//!
//! This consumer owns, and only owns:
//!
//! * the **lazy creation** of the codec writer — no vector file exists until a
//!   document actually carries a vector;
//! * the **write state** the writer is created from, which must outlive the
//!   document that triggered the creation;
//! * the flush, abort and close lifecycle of that writer;
//! * the RAM footprint the writer reports to the indexing chain.
//!
//! It deliberately does **not** own the buffering. Lucene is explicit about
//! this — *"The codec's vectors writer is responsible for buffering and
//! processing vectors"* (`VectorValuesConsumer.java:31-34`) — and it is why
//! this consumer has no counterpart to
//! [`PointValuesWriter`](crate::index::point_values_writer::PointValuesWriter)
//! or [`NormValuesWriter`](crate::index::norms_writer::NormValuesWriter): the
//! per-field buffer is
//! [`DefaultFlatFieldVectorsWriter`](crate::codecs::hnsw::flat_vectors::DefaultFlatFieldVectorsWriter),
//! which lives inside the codec, and the indexing chain holds the handle to it
//! that [`VectorValuesConsumer::add_field`] returns.
//!
//! # Why the writer is created during indexing, not at flush
//!
//! Every other consumer in the chain creates its codec writer at flush time,
//! inside the call that owns the `SegmentWriteState`. This one cannot: Lucene
//! creates it on the **first vector field of the first document that has one**
//! (`IndexingChain.initializeFieldInfo`, `IndexingChain.java:1375-1382`),
//! because the per-field writer the chain must hand the vectors to comes from
//! the codec writer itself. Two consequences follow, and both are observable:
//!
//! * the vector files of the segment are created while documents are still
//!   being indexed, so they appear in the tracking directory's created-file set
//!   before the flush, and [`VectorValuesConsumer::abort`] is what removes
//!   them;
//! * the **order of the per-field entries** inside the `.vemf` and the `.vem`
//!   is the order the fields were first seen in the documents, not the
//!   field-hash order that fixes the entry order of the `.dvm` and the `.kdm`.
//!   `initializeFieldInfo` runs from the first pass of `processDocument`, in
//!   document field order (`IndexingChain.java:617-648`), while `writeDocValues`
//!   and `writePoints` walk the `fieldHash` table.
//!
//! Because the writer outlives the call that made it, the state it is built
//! from must be owned rather than borrowed; see
//! [`OwnedSegmentWriteState`](crate::codecs::state::OwnedSegmentWriteState) for
//! the one divergence this requires.

#![deny(unsafe_code)]

use std::sync::Arc;

use crate::codecs::knn_vectors::{FieldVectorWriter, KnnVectorsWriter, SorterDocMap};
use crate::codecs::state::OwnedSegmentWriteState;
use crate::codecs::stub::BufferedUpdates;
use crate::codecs::Codec;
use crate::error::Result;
use crate::index::{FieldInfo, FieldInfos, SegmentInfo};
use crate::store::{DefaultIOContext, Directory};
use crate::util::InfoStream;

/// Streams vector values for indexing to the codec's vectors writer.
///
/// Equivalent to `org.apache.lucene.index.VectorValuesConsumer`.
pub struct VectorValuesConsumer {
    codec: Arc<dyn Codec>,
    directory: Arc<dyn Directory>,
    segment_info: SegmentInfo,
    info_stream: Arc<dyn InfoStream>,
    /// The codec's vectors writer, absent until the first vector field is
    /// added. Java's `accountable` field tracks the same thing: it is
    /// `Accountable.NULL_ACCOUNTABLE` until the writer exists and the writer
    /// afterwards, which is exactly `Option::map_or(0, ..)` here.
    writer: Option<Box<dyn KnnVectorsWriter>>,
}

impl std::fmt::Debug for VectorValuesConsumer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VectorValuesConsumer")
            .field("segment", &self.segment_info.name)
            .field("writer", &self.writer.is_some())
            .finish_non_exhaustive()
    }
}

impl VectorValuesConsumer {
    /// Creates a consumer for the segment `segment_info` describes.
    ///
    /// Nothing is written and no file is created here; the codec writer is
    /// built by the first [`VectorValuesConsumer::add_field`] call.
    pub fn new(
        codec: Arc<dyn Codec>,
        directory: Arc<dyn Directory>,
        segment_info: SegmentInfo,
        info_stream: Arc<dyn InfoStream>,
    ) -> Self {
        Self {
            codec,
            directory,
            segment_info,
            info_stream,
            writer: None,
        }
    }

    /// Creates the codec's vectors writer, once.
    ///
    /// Equivalent to `VectorValuesConsumer.initKnnVectorsWriter`
    /// (`VectorValuesConsumer.java:52-66`).
    ///
    /// Java also rejects a codec whose `knnVectorsFormat()` is `null` with
    /// `IllegalStateException("field=\"..\" was indexed as vectors but codec
    /// does not support vectors")`. [`Codec::knn_vectors_format`] returns
    /// `&dyn KnnVectorsFormat` rather than an `Option`, so every codec this
    /// crate can hold has one and that state is unrepresentable; the check is
    /// therefore not ported, exactly as
    /// [`DefaultIndexingChain`](crate::index::indexing_chain::DefaultIndexingChain)'s
    /// `write_points` does not port the same check for the points format.
    ///
    /// # Errors
    ///
    /// Propagates whatever the KNN-vectors format raises while creating the
    /// writer — with the default codec, the failure to create the `.vec`,
    /// `.vemf`, `.vex` or `.vem` output.
    fn init_knn_vectors_writer(&mut self) -> Result<()> {
        if self.writer.is_some() {
            return Ok(());
        }
        // Java's `initialWriteState` passes `null` for the field infos and for
        // the buffered updates (`VectorValuesConsumer.java:61-62`): neither is
        // reachable from a vectors writer, which reads only the directory, the
        // segment name, the segment suffix and the I/O context. Rust has no
        // null, so both are the empty value, and no writer can tell the
        // difference.
        let state = OwnedSegmentWriteState::new(
            Arc::clone(&self.info_stream),
            Arc::clone(&self.directory),
            self.segment_info.clone(),
            FieldInfos::default(),
            BufferedUpdates,
            // `IOContext.DEFAULT`, as Java passes.
            Arc::new(DefaultIOContext::default()),
        );
        self.writer = Some(self.codec.knn_vectors_format().fields_writer(&state)?);
        Ok(())
    }

    /// Registers a vector field and returns the writer its values go to.
    ///
    /// Equivalent to `VectorValuesConsumer.addField`
    /// (`VectorValuesConsumer.java:68-71`). The returned variant matches the
    /// field's [`VectorEncoding`](crate::index::VectorEncoding).
    ///
    /// # Errors
    ///
    /// Propagates the writer-creation failure of
    /// [`VectorValuesConsumer::init_knn_vectors_writer`] and whatever the codec
    /// writer raises for the field itself.
    pub fn add_field(&mut self, field_info: &FieldInfo) -> Result<FieldVectorWriter> {
        self.init_knn_vectors_writer()?;
        self.writer
            .as_mut()
            .expect("INVARIANT: init_knn_vectors_writer left a writer or returned an error")
            .add_field(field_info)
    }

    /// Flushes every buffered vector and closes the writer.
    ///
    /// Equivalent to `VectorValuesConsumer.flush`
    /// (`VectorValuesConsumer.java:73-81`). A segment whose documents carried
    /// no vector never created a writer and writes no file, which is why the
    /// absent writer returns without error.
    ///
    /// # Errors
    ///
    /// Propagates the failure of the codec writer's flush, finish or close.
    /// Java runs the close in a `finally` block rather than a
    /// try-with-resources, so a failure to close **replaces** a failure to
    /// flush instead of being suppressed by it; this reproduces that, which is
    /// the opposite of what `write_points` does for the points writer — and it
    /// is the opposite in Lucene too.
    pub fn flush(&mut self, max_doc: i32, sort_map: Option<&SorterDocMap>) -> Result<()> {
        let Some(writer) = self.writer.as_mut() else {
            return Ok(());
        };
        let outcome = writer
            .flush(max_doc, sort_map)
            .and_then(|()| writer.finish());
        let close_outcome = writer.close();
        close_outcome?;
        outcome
    }

    /// Closes the writer, discarding any failure.
    ///
    /// Equivalent to `VectorValuesConsumer.abort`
    /// (`VectorValuesConsumer.java:83-85`), which calls
    /// `IOUtils.closeWhileHandlingException`: the segment is being thrown away,
    /// so a failure to close one of its files changes nothing. Like Java's, it
    /// keeps the writer afterwards rather than clearing it.
    pub fn abort(&mut self) {
        if let Some(writer) = self.writer.as_mut() {
            let _ = writer.close();
        }
    }

    /// Returns the RAM the buffered vectors hold.
    ///
    /// Equivalent to `VectorValuesConsumer.getAccountable().ramBytesUsed()`,
    /// where Java's `accountable` is `Accountable.NULL_ACCOUNTABLE` — zero —
    /// until the writer exists.
    pub fn ram_bytes_used(&self) -> i64 {
        self.writer
            .as_ref()
            .map_or(0, |writer| writer.ram_bytes_used())
    }

    /// Returns whether a codec writer was created, and therefore whether this
    /// segment has vector files.
    pub fn has_writer(&self) -> bool {
        self.writer.is_some()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::codecs::knn_vectors::FieldVectorWriter;
    use crate::codecs::Lucene104Codec;
    use crate::index::{
        DocValuesSkipIndexType, DocValuesType, IndexOptions, VectorEncoding,
        VectorSimilarityFunction,
    };
    use crate::store::{Directory, RamDirectory, TrackingDirectoryWrapper};
    use crate::util::{NoOutputInfoStream, Version};

    fn vector_field(name: &str, number: i32, dim: i32, encoding: VectorEncoding) -> FieldInfo {
        FieldInfo::new_full(
            name,
            number,
            false,
            false,
            false,
            IndexOptions::NONE,
            DocValuesType::NONE,
            DocValuesSkipIndexType::NONE,
            -1,
            HashMap::new(),
            0,
            0,
            0,
            dim,
            encoding,
            VectorSimilarityFunction::EUCLIDEAN,
            false,
            false,
        )
        .expect("field info")
    }

    /// A consumer over a fresh in-memory directory, plus the tracking wrapper
    /// around it so a test can see which files it created.
    ///
    /// The wrapper, not the directory's own listing, is what answers "which
    /// files does this segment own": `RamDirectory` publishes a file only once
    /// its output is closed, and the whole point of these tests is that the
    /// consumer opens its files while documents are still being indexed.
    fn consumer() -> (VectorValuesConsumer, Arc<TrackingDirectoryWrapper>) {
        let directory = Arc::new(TrackingDirectoryWrapper::new(Box::new(
            RamDirectory::default(),
        )));
        let codec: Arc<dyn crate::codecs::Codec> = Arc::new(Lucene104Codec::new());
        // `maxDoc` is deliberately unset, exactly as it is while a
        // `DocumentsWriterPerThread` is still indexing: the consumer builds its
        // writer at that point, so the writer must not need it.
        let segment_info = SegmentInfo::new(
            Arc::clone(&directory) as Arc<dyn Directory>,
            Version::LUCENE_10_5_0,
            Some(Version::LUCENE_10_5_0),
            "_0".to_string(),
            -1,
            false,
            false,
            Arc::clone(&codec),
            HashMap::new(),
            [7u8; crate::util::string_helper::ID_LENGTH],
            HashMap::new(),
            crate::search::Sort::default(),
        )
        .expect("segment info");
        let consumer = VectorValuesConsumer::new(
            codec,
            Arc::clone(&directory) as Arc<dyn Directory>,
            segment_info,
            Arc::new(NoOutputInfoStream),
        );
        (consumer, directory)
    }

    /// The consumer must create nothing until a field actually asks for a
    /// writer, because a segment whose documents carry no vector must produce
    /// no vector file at all.
    #[test]
    fn no_field_means_no_writer_and_no_file() {
        let (mut consumer, directory) = consumer();
        assert!(!consumer.has_writer());
        assert_eq!(consumer.ram_bytes_used(), 0);
        assert!(
            directory.get_created_files().is_empty(),
            "the consumer created a file before any field asked for one"
        );
        // Flushing a consumer that never saw a field is a no-op, not an error:
        // Java returns early on a null writer.
        consumer.flush(10, None).expect("flush without a writer");
        assert!(directory.get_created_files().is_empty());
    }

    /// The first field creates the writer, and with it the segment's vector
    /// files — during indexing, not at flush. That timing is what
    /// `IndexingChain.abort` has to clean up.
    #[test]
    fn the_first_field_creates_the_writer_and_its_files() {
        let (mut consumer, directory) = consumer();
        let writer = consumer
            .add_field(&vector_field("v", 0, 3, VectorEncoding::FLOAT32))
            .expect("add field");
        assert!(matches!(writer, FieldVectorWriter::Float(_)));
        assert!(consumer.has_writer());

        let mut files: Vec<String> = directory.get_created_files().into_iter().collect();
        files.sort();
        assert_eq!(
            files,
            vec![
                "_0_Lucene99HnswVectorsFormat_0.vec",
                "_0_Lucene99HnswVectorsFormat_0.vem",
                "_0_Lucene99HnswVectorsFormat_0.vemf",
                "_0_Lucene99HnswVectorsFormat_0.vex",
            ],
            "the four vector files must exist as soon as the first field is added"
        );
    }

    /// A second field must reuse the writer the first one created, rather than
    /// opening a second set of files.
    #[test]
    fn a_second_field_reuses_the_same_writer() {
        let (mut consumer, directory) = consumer();
        consumer
            .add_field(&vector_field("a", 0, 3, VectorEncoding::FLOAT32))
            .expect("add a");
        let after_first = directory.get_created_files().len();
        consumer
            .add_field(&vector_field("b", 1, 4, VectorEncoding::BYTE))
            .expect("add b");
        assert_eq!(
            directory.get_created_files().len(),
            after_first,
            "the second field must not open a second set of vector files"
        );
    }

    /// The encoding of the returned writer follows the field, because the
    /// indexing chain dispatches on the variant rather than on the field info.
    #[test]
    fn the_writer_variant_follows_the_field_encoding() {
        let (mut consumer, _directory) = consumer();
        let float = consumer
            .add_field(&vector_field("f", 0, 2, VectorEncoding::FLOAT32))
            .expect("add float");
        assert!(matches!(float, FieldVectorWriter::Float(_)));
        let byte = consumer
            .add_field(&vector_field("b", 1, 2, VectorEncoding::BYTE))
            .expect("add byte");
        assert!(matches!(byte, FieldVectorWriter::Byte(_)));
    }

    /// The footprint must follow the buffered vectors, because it is what the
    /// indexing chain adds to the segment's RAM total and therefore what
    /// decides when the segment flushes. A consumer that answered a constant
    /// would make a segment full of vectors look free.
    #[test]
    fn the_footprint_grows_with_the_buffered_vectors() {
        let (mut consumer, _directory) = consumer();
        let writer = consumer
            .add_field(&vector_field("v", 0, 64, VectorEncoding::FLOAT32))
            .expect("add field");
        let FieldVectorWriter::Float(mut writer) = writer else {
            panic!("expected a float writer");
        };
        let empty = consumer.ram_bytes_used();
        for doc in 0..64 {
            writer.add_value(doc, vec![1.0f32; 64]).expect("add value");
        }
        let full = consumer.ram_bytes_used();
        assert!(
            full > empty,
            "the footprint did not grow: {empty} then {full}"
        );
        // 64 vectors of 64 float32 components are 16384 bytes of payload alone,
        // and Java counts that payload exactly the same way.
        assert!(
            full - empty >= 64 * 64 * 4,
            "the footprint grew by {} bytes, less than the {} bytes of vector payload",
            full - empty,
            64 * 64 * 4
        );
    }

    /// Aborting must close the writer without raising, because the segment is
    /// being discarded and a failure to close one of its files changes nothing.
    #[test]
    fn abort_closes_the_writer_quietly() {
        let (mut consumer, _directory) = consumer();
        consumer
            .add_field(&vector_field("v", 0, 3, VectorEncoding::FLOAT32))
            .expect("add field");
        consumer.abort();
        // Java keeps the writer reference after aborting, and so does this.
        assert!(consumer.has_writer());
        // A second abort must also be quiet.
        consumer.abort();
    }
}
