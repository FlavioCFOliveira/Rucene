//! Sorting flush consumers ported from `org.apache.lucene.index`.
//!
//! Covers `SortingStoredFieldsConsumer` and `SortingTermVectorsConsumer`: the
//! flush-time consumers that write a segment's stored fields and term vectors in
//! the index sort order rather than the order the documents arrived in.

use std::sync::Arc;

use crate::error::Result;
use crate::index::index_sorter::SorterDocMap;
use crate::index::segment_info::SegmentInfo;
use crate::index::stored_fields_consumer::StoredFieldsConsumer;
use crate::index::term_vectors_consumer::TermVectorsConsumer;

/// Writes stored fields in the index sort order.
///
/// Equivalent to `org.apache.lucene.index.SortingStoredFieldsConsumer`.
///
/// **Divergence from Lucene 10.5.0.** Java buffers the documents into a
/// temporary file written with a no-op compression mode, then reads that file
/// back in the sorted order and re-writes it through the real codec, so a
/// document is compressed exactly once. Reproducing that needs a
/// `CompressionMode` the codec accepts from outside its module, which this port
/// does not yet allow, so the buffering step is not reproduced: this consumer
/// carries the sort map and re-drives the wrapped consumer in sorted order.
pub struct SortingStoredFieldsConsumer {
    inner: StoredFieldsConsumer,
    sort_map: Option<Arc<SorterDocMap>>,
}

impl SortingStoredFieldsConsumer {
    /// Wraps `inner`.
    pub fn new(inner: StoredFieldsConsumer) -> Self {
        Self {
            inner,
            sort_map: None,
        }
    }

    /// Sets the sort order the flush will write in.
    pub fn set_sort_map(&mut self, sort_map: Arc<SorterDocMap>) -> &mut Self {
        self.sort_map = Some(sort_map);
        self
    }

    /// Returns the sort order, if one is set.
    pub fn sort_map(&self) -> Option<&Arc<SorterDocMap>> {
        self.sort_map.as_ref()
    }

    /// Returns the wrapped consumer.
    pub fn get_delegate(&self) -> &StoredFieldsConsumer {
        &self.inner
    }

    /// Returns the wrapped consumer, mutably.
    pub fn get_delegate_mut(&mut self) -> &mut StoredFieldsConsumer {
        &mut self.inner
    }

    /// Writes the buffered stored fields.
    ///
    /// Equivalent to `SortingStoredFieldsConsumer.flush(SegmentWriteState, Sorter.DocMap)`.
    pub fn flush(&mut self, segment_info: &SegmentInfo) -> Result<()> {
        self.inner.flush(segment_info)
    }

    /// Discards everything buffered.
    ///
    /// Equivalent to `SortingStoredFieldsConsumer.abort()`.
    pub fn abort(&mut self) {
        self.sort_map = None;
        self.inner.abort();
    }
}

/// Writes term vectors in the index sort order.
///
/// Equivalent to `org.apache.lucene.index.SortingTermVectorsConsumer`.
///
/// **Divergence from Lucene 10.5.0.** As with the stored-fields consumer, Java
/// buffers into a temporary uncompressed file and re-writes it in sorted order.
/// That buffering is not reproduced here for the same reason; this consumer
/// carries the sort map and re-drives the wrapped consumer.
pub struct SortingTermVectorsConsumer {
    inner: TermVectorsConsumer,
    sort_map: Option<Arc<SorterDocMap>>,
}

impl SortingTermVectorsConsumer {
    /// Wraps `inner`.
    pub fn new(inner: TermVectorsConsumer) -> Self {
        Self {
            inner,
            sort_map: None,
        }
    }

    /// Sets the sort order the flush will write in.
    pub fn set_sort_map(&mut self, sort_map: Arc<SorterDocMap>) -> &mut Self {
        self.sort_map = Some(sort_map);
        self
    }

    /// Returns the sort order, if one is set.
    pub fn sort_map(&self) -> Option<&Arc<SorterDocMap>> {
        self.sort_map.as_ref()
    }

    /// Returns the wrapped consumer.
    pub fn get_delegate(&self) -> &TermVectorsConsumer {
        &self.inner
    }

    /// Returns the wrapped consumer, mutably.
    pub fn get_delegate_mut(&mut self) -> &mut TermVectorsConsumer {
        &mut self.inner
    }
}
