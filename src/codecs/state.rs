//! Common read/write state bundles passed to codec formats.
//!
//! Equivalent to `org.apache.lucene.index.SegmentReadState` and
//! `org.apache.lucene.index.SegmentWriteState`.
//!
//! The base format traits in this crate keep the raw parameter lists from
//! Lucene's Java base classes, but these structs aggregate the same
//! information for formats that prefer a single state object.

use std::sync::Arc;

use crate::store::{Directory, IOContext};
use crate::util::{FixedBitSet, InfoStream};

use super::stub::BufferedUpdates;
use crate::index::{FieldInfos, SegmentInfo};

/// Parameters used when reading a segment.
///
/// Lucene Core equivalent: `org.apache.lucene.index.SegmentReadState`.
#[derive(Clone)]
pub struct SegmentReadState<'a> {
    /// Directory that contains the segment files.
    pub directory: &'a dyn Directory,

    /// Metadata describing the segment.
    pub segment_info: &'a SegmentInfo,

    /// Field metadata for the segment.
    pub field_infos: &'a FieldInfos,

    /// I/O context for all reads.
    pub context: &'a dyn IOContext,

    /// Suffix used for files read by this format instance.
    pub segment_suffix: String,
}

impl std::fmt::Debug for SegmentReadState<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SegmentReadState")
            .field("segment_suffix", &self.segment_suffix)
            .finish_non_exhaustive()
    }
}

impl<'a> SegmentReadState<'a> {
    /// Creates a read state with an empty segment suffix.
    pub fn new(
        directory: &'a dyn Directory,
        segment_info: &'a SegmentInfo,
        field_infos: &'a FieldInfos,
        context: &'a dyn IOContext,
    ) -> Self {
        Self {
            directory,
            segment_info,
            field_infos,
            context,
            segment_suffix: String::new(),
        }
    }

    /// Creates a read state with the given segment suffix.
    pub fn with_suffix(
        directory: &'a dyn Directory,
        segment_info: &'a SegmentInfo,
        field_infos: &'a FieldInfos,
        context: &'a dyn IOContext,
        segment_suffix: String,
    ) -> Self {
        Self {
            directory,
            segment_info,
            field_infos,
            context,
            segment_suffix,
        }
    }

    /// Returns a copy of this state with a new segment suffix.
    pub fn with_new_suffix(&self, segment_suffix: String) -> SegmentReadState<'a> {
        SegmentReadState {
            directory: self.directory,
            segment_info: self.segment_info,
            field_infos: self.field_infos,
            context: self.context,
            segment_suffix,
        }
    }
}

/// Parameters used when writing a segment.
///
/// Lucene Core equivalent: `org.apache.lucene.index.SegmentWriteState`.
#[derive(Clone)]
pub struct SegmentWriteState<'a> {
    /// Info stream for diagnostic messages.
    pub info_stream: &'a dyn InfoStream,

    /// Directory that will receive the segment files.
    pub directory: &'a dyn Directory,

    /// Metadata describing the segment.
    pub segment_info: &'a SegmentInfo,

    /// Field metadata for the segment.
    pub field_infos: &'a FieldInfos,

    /// Buffered deletes/updates for this segment while flushing.
    pub seg_updates: &'a BufferedUpdates,

    /// I/O context for all writes.
    pub context: &'a dyn IOContext,

    /// Suffix used for files written by this format instance.
    pub segment_suffix: String,

    /// Number of deleted documents set while flushing the segment.
    pub del_count_on_flush: i32,

    /// Number of soft-deleted documents set while flushing the segment.
    pub soft_del_count_on_flush: i32,

    /// Live documents; only set when there are deletions.
    pub live_docs: Option<FixedBitSet>,
}

impl std::fmt::Debug for SegmentWriteState<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SegmentWriteState")
            .field("segment_suffix", &self.segment_suffix)
            .field("del_count_on_flush", &self.del_count_on_flush)
            .field("soft_del_count_on_flush", &self.soft_del_count_on_flush)
            .field("live_docs", &self.live_docs.is_some())
            .finish_non_exhaustive()
    }
}

impl<'a> SegmentWriteState<'a> {
    /// Creates a write state with an empty segment suffix.
    pub fn new(
        info_stream: &'a dyn InfoStream,
        directory: &'a dyn Directory,
        segment_info: &'a SegmentInfo,
        field_infos: &'a FieldInfos,
        seg_updates: &'a BufferedUpdates,
        context: &'a dyn IOContext,
    ) -> Self {
        Self {
            info_stream,
            directory,
            segment_info,
            field_infos,
            seg_updates,
            context,
            segment_suffix: String::new(),
            del_count_on_flush: 0,
            soft_del_count_on_flush: 0,
            live_docs: None,
        }
    }

    /// Creates a write state with the given segment suffix.
    pub fn with_suffix(
        info_stream: &'a dyn InfoStream,
        directory: &'a dyn Directory,
        segment_info: &'a SegmentInfo,
        field_infos: &'a FieldInfos,
        seg_updates: &'a BufferedUpdates,
        context: &'a dyn IOContext,
        segment_suffix: String,
    ) -> Self {
        Self {
            info_stream,
            directory,
            segment_info,
            field_infos,
            seg_updates,
            context,
            segment_suffix,
            del_count_on_flush: 0,
            soft_del_count_on_flush: 0,
            live_docs: None,
        }
    }

    /// Returns a copy of this state with a new segment suffix.
    pub fn with_new_suffix(&self, segment_suffix: String) -> SegmentWriteState<'a> {
        SegmentWriteState {
            info_stream: self.info_stream,
            directory: self.directory,
            segment_info: self.segment_info,
            field_infos: self.field_infos,
            seg_updates: self.seg_updates,
            context: self.context,
            segment_suffix,
            del_count_on_flush: self.del_count_on_flush,
            soft_del_count_on_flush: self.soft_del_count_on_flush,
            live_docs: self.live_docs.clone(),
        }
    }
}

// ---------------------------------------------------------------------------
// OwnedSegmentWriteState
// ---------------------------------------------------------------------------

/// A [`SegmentWriteState`] whose handles are owned rather than borrowed.
///
/// # Why this type exists
///
/// Apache Lucene Core 10.5.0 has exactly one `SegmentWriteState`
/// (`org.apache.lucene.index.SegmentWriteState`), whose fields are ordinary
/// Java references: an object that stores one keeps the directory, the segment
/// info and the I/O context alive for as long as it needs them. Almost every
/// codec writer in this crate is created inside the flush call and dies there,
/// so the borrowed [`SegmentWriteState`] serves them exactly.
///
/// The KNN-vectors writer is the one that cannot work that way.
/// `VectorValuesConsumer` creates it on the **first vector field of the first
/// document** (`VectorValuesConsumer.java:52-71`) and keeps it until the
/// segment flushes, and `PerFieldKnnVectorsFormat.FieldsWriter` stores the
/// state itself so it can build a suffixed sub-state for every field it later
/// meets (`PerFieldKnnVectorsFormat.java:96-136`). A Rust writer that retained
/// a `SegmentWriteState<'a>` would be bound to `'a` and could not be stored
/// beside the data it borrows.
///
/// This type is therefore the owned twin of [`SegmentWriteState`], holding the
/// three trait objects behind [`Arc`] and cloning the rest.
/// [`OwnedSegmentWriteState::borrow`] produces the borrowed form the codec
/// writers already take, so no format had to change how it reads its state.
///
/// **Divergence:** Lucene has one state type where this port has two. The
/// alternative was to make every field of [`SegmentWriteState`] owned, which
/// would have changed all 32 of its construction sites across the codec layer
/// for the benefit of one writer. The blast radius is confined to the
/// KNN-vectors seam: [`crate::codecs::knn_vectors::KnnVectorsFormat::fields_writer`]
/// is the only format entry point that takes this type.
pub struct OwnedSegmentWriteState {
    /// Info stream for diagnostic messages.
    pub info_stream: Arc<dyn InfoStream>,

    /// Directory that will receive the segment files.
    pub directory: Arc<dyn Directory>,

    /// Metadata describing the segment.
    pub segment_info: SegmentInfo,

    /// Field metadata for the segment.
    pub field_infos: FieldInfos,

    /// Buffered deletes/updates for this segment while flushing.
    pub seg_updates: BufferedUpdates,

    /// I/O context for all writes.
    pub context: Arc<dyn IOContext>,

    /// Suffix used for files written by this format instance.
    pub segment_suffix: String,

    /// Number of deleted documents set while flushing the segment.
    pub del_count_on_flush: i32,

    /// Number of soft-deleted documents set while flushing the segment.
    pub soft_del_count_on_flush: i32,

    /// Live documents; only set when there are deletions.
    pub live_docs: Option<FixedBitSet>,
}

impl std::fmt::Debug for OwnedSegmentWriteState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OwnedSegmentWriteState")
            .field("segment_suffix", &self.segment_suffix)
            .field("del_count_on_flush", &self.del_count_on_flush)
            .field("soft_del_count_on_flush", &self.soft_del_count_on_flush)
            .field("live_docs", &self.live_docs.is_some())
            .finish_non_exhaustive()
    }
}

impl OwnedSegmentWriteState {
    /// Creates an owned write state with an empty segment suffix.
    pub fn new(
        info_stream: Arc<dyn InfoStream>,
        directory: Arc<dyn Directory>,
        segment_info: SegmentInfo,
        field_infos: FieldInfos,
        seg_updates: BufferedUpdates,
        context: Arc<dyn IOContext>,
    ) -> Self {
        Self {
            info_stream,
            directory,
            segment_info,
            field_infos,
            seg_updates,
            context,
            segment_suffix: String::new(),
            del_count_on_flush: 0,
            soft_del_count_on_flush: 0,
            live_docs: None,
        }
    }

    /// Borrows this state as the [`SegmentWriteState`] every codec writer takes.
    pub fn borrow(&self) -> SegmentWriteState<'_> {
        SegmentWriteState {
            info_stream: &*self.info_stream,
            directory: &*self.directory,
            segment_info: &self.segment_info,
            field_infos: &self.field_infos,
            seg_updates: &self.seg_updates,
            context: &*self.context,
            segment_suffix: self.segment_suffix.clone(),
            del_count_on_flush: self.del_count_on_flush,
            soft_del_count_on_flush: self.soft_del_count_on_flush,
            live_docs: self.live_docs.clone(),
        }
    }

    /// Returns a copy of this state with a new segment suffix.
    ///
    /// Equivalent to Java's `new SegmentWriteState(state, segmentSuffix)` copy
    /// constructor, which `PerFieldKnnVectorsFormat.FieldsWriter.getInstance`
    /// uses for every field it dispatches.
    pub fn with_new_suffix(&self, segment_suffix: String) -> Self {
        Self {
            info_stream: Arc::clone(&self.info_stream),
            directory: Arc::clone(&self.directory),
            segment_info: self.segment_info.clone(),
            field_infos: self.field_infos.clone(),
            seg_updates: self.seg_updates.clone(),
            context: Arc::clone(&self.context),
            segment_suffix,
            del_count_on_flush: self.del_count_on_flush,
            soft_del_count_on_flush: self.soft_del_count_on_flush,
            live_docs: self.live_docs.clone(),
        }
    }
}
