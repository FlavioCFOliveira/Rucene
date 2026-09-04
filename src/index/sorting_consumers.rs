//! Sorting flush consumers ported from `org.apache.lucene.index`.
//!
//! Covers `SortingStoredFieldsConsumer` and `SortingTermVectorsConsumer`: the
//! flush-time consumers that write a segment's stored fields and term vectors in
//! the index sort order rather than the order the documents arrived in.

use std::sync::Arc;

use crate::codecs::term_vectors::TermVectorsWriter;
use crate::error::{LuceneError, Result};
use crate::index::index_sorter::SorterDocMap;
use crate::index::postings_enum::{POSTINGS_ENUM_OFFSETS, POSTINGS_ENUM_PAYLOADS};
use crate::index::segment_info::SegmentInfo;
use crate::index::stored_fields_consumer::StoredFieldsConsumer;
use crate::index::term_vectors_consumer::TermVectorsConsumer;
use crate::index::{FieldInfos, Fields};
use crate::search::{DocIdSetIterator, NO_MORE_DOCS};
use crate::util::BytesRef;

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
                LuceneError::IllegalState(format!(
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

            let mut num_terms = terms.size() as i32;
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
