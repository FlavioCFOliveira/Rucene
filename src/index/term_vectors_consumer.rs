//! The term-vectors half of the per-segment indexing pipeline.
//!
//! | Rucene | Apache Lucene Core 10.5.0 |
//! | --- | --- |
//! | [`TermVectorsConsumer`] | `org.apache.lucene.index.TermVectorsConsumer` |
//! | [`TermVectorsConsumerPerField`] | `org.apache.lucene.index.TermVectorsConsumerPerField` |
//! | [`TermVectors`](crate::index::TermVectors) | `org.apache.lucene.index.TermVectors` |
//!
//! # Macro objective
//!
//! Own the term-vectors stream of one segment. For every field that asked for
//! term vectors, the consumer buffers the terms of the *current document*
//! together with their frequency and, when requested, their positions, offsets
//! and payloads; at the end of each document it streams them through the
//! codec's [`TermVectorsWriter`], producing the segment's `.tvd`, `.tvx` and
//! `.tvm` files.
//!
//! A term vector is a single-document inverted index. It is therefore built and
//! discarded one document at a time, which is what separates this consumer from
//! [`FreqProxTermsWriter`](crate::index::FreqProxTermsWriter): the latter
//! accumulates a doc list per term for the whole segment, this one keeps only
//! what the document in flight needs.
//!
//! # Responsibility boundary
//!
//! The consumer owns, and only owns:
//!
//! * the lazy creation of the codec writer — no `.tvd` exists until a document
//!   actually carries vectors;
//! * the per-field flags `doVectors` / `doVectorPositions` / `doVectorOffsets`
//!   / `doVectorPayloads`, and their consistency across every instance of a
//!   multi-valued field;
//! * the RAM buffers holding the position and offset streams of the current
//!   document (its own [`TermsHash`], see *Two term hashes* below);
//! * per-document framing, including empty frames for every document that had
//!   no vectors, so that frames stay aligned with doc ids;
//! * ordering: fields by name, terms by their bytes;
//! * the flush and abort lifecycle of the writer.
//!
//! It deliberately does **not** own: which fields request vectors (the field
//! type decides, and the indexing chain routes), the term text (interned once
//! by the postings hash), the chunking, compression and on-disk layout (the
//! codec writer), or the index-sorting variant (see *Index sorting* below).
//!
//! # Two term hashes, one term pool
//!
//! Lucene chains two `TermsHash` instances. The first,
//! `FreqProxTermsWriter`, interns every token's text into its byte pool and
//! writes the segment-wide doc/freq/prox streams there. The second is this
//! consumer: it has *its own* byte pool for the per-document position and
//! offset streams, but its `BytesRefHash` is keyed on offsets into the *first*
//! pool (`TermsHash.java:52-56`). The token text is therefore stored once, and
//! the second hash never hashes or compares a byte
//! (`TermsHashPerField.addByPoolOffset`).
//!
//! This port keeps that arrangement: [`TermVectorsConsumer`] owns a
//! [`TermsHash`] for the streams, every per-field table is built with
//! [`TermsHashPerField::new_chained`], and the term pool is passed in from
//! [`FreqProxTermsWriter::pool`](crate::index::FreqProxTermsWriter::pool)
//! wherever term bytes are actually needed — which is only when sorting the
//! terms and when writing them out.
//!
//! # Lifecycle and invariants
//!
//! Per segment:
//!
//! 1. `has_vectors` starts `false`. The indexing chain calls
//!    [`set_has_vectors`](TermVectorsConsumer::set_has_vectors) the first time
//!    it registers a field whose `FieldInfo` stores term vectors
//!    (`IndexingChain.java:1843-1845`). It is never cleared: from that document
//!    on, *every* document of the segment gets a frame, empty or not.
//! 2. `last_doc_id` starts at `0` and is reset to `0` when the writer is
//!    created (`TermVectorsConsumer.java:107`).
//!
//! Per document:
//!
//! 3. [`start_document`](TermVectorsConsumer::start_document) clears the list of
//!    fields pending for this document (`TermVectorsConsumer.java:177-180`).
//! 4. [`start_field`](TermVectorsConsumer::start_field) runs once per field
//!    *instance*. On the first instance it re-reads the four flags from the
//!    field type and validates them; on later instances it requires them to be
//!    identical (`TermVectorsConsumerPerField.java:131-233`). It returns whether
//!    tokens should be routed to this field at all.
//! 5. [`add`](TermVectorsConsumer::add) runs once per token of a field whose
//!    `start_field` returned `true`, taking the pool offset the postings hash
//!    interned the token at.
//! 6. [`finish_field`](TermVectorsConsumer::finish_field) runs once per field at
//!    the end of the document and marks the field as pending when it has
//!    vectors *and* at least one term (`TermVectorsConsumerPerField.java:69-74`).
//! 7. [`finish_document`](TermVectorsConsumer::finish_document) does nothing
//!    while `has_vectors` is `false`. Otherwise it sorts the pending fields by
//!    name, creates the writer if needed, writes an empty frame for every doc id
//!    skipped since the last one, writes this document's frame, and discards
//!    every buffer the document used (`TermVectorsConsumer.java:117-143`).
//!
//! Per segment, at the end:
//!
//! 8. [`flush`](TermVectorsConsumer::flush) pads the tail with empty frames up
//!    to `max_doc`, finishes the writer and closes it — closing it even when
//!    finishing failed (`TermVectorsConsumer.java:71-90`). With no writer, i.e.
//!    with no document that ever had vectors, it writes nothing at all.
//! 9. [`abort`](TermVectorsConsumer::abort) drops every buffer and closes the
//!    writer, discarding any error (`TermVectorsConsumer.java:145-153`).
//!
//! # Java to Rust adaptations
//!
//! * **The inheritance chain becomes explicit calls.** Java reaches this
//!   consumer through `TermsHash.startDocument()`,
//!   `TermsHashPerField.start`/`add`/`finish` and
//!   `TermsHash.finishDocument(int)`, each of which forwards to the next hash of
//!   the chain. `freq_prox_terms_writer` already flattened that template method
//!   into a value the caller dispatches on, so this port continues the same
//!   way: the indexing chain calls both consumers, in Lucene's order.
//! * **The writer is an `Option`, and an aborted consumer stays aborted.**
//!   Exactly as in
//!   [`StoredFieldsConsumer`](crate::index::StoredFieldsConsumer): Java's
//!   `abort()` leaves the field pointing at a closed writer whose streams are
//!   null, so a further document would fail late with a
//!   `NullPointerException`; Rust has to release the writer in order to close
//!   it, which would let the lazy initialisation build a *second* set of
//!   `.tvd`/`.tvx`/`.tvm` files for a segment that was already discarded.
//!   Recording the abort and refusing immediately is neither silent nor
//!   destructive.
//! * **Two RAM counters, as in Java.** The stream pool charges the chain's
//!   counter, because Java's pools are built on the
//!   `DocumentsWriterPerThread`'s shared allocators; the per-field term tables
//!   and posting slots charge a private counter, because Java hands the
//!   term-vectors `TermsHash` a `Counter.newCounter()`
//!   (`TermVectorsConsumer.java:65`) and therefore leaves them out of
//!   `IndexingChain.ramBytesUsed()`. That private counter is what sizes the
//!   `FlushInfo` when the writer is created.
//! * **The codec writer's own buffer is not accounted, and the gap is
//!   unbounded.** Java keeps the writer in an `Accountable` field so that
//!   `IndexingChain.ramBytesUsed()` adds `termVectorsWriter.accountable
//!   .ramBytesUsed()` (`IndexingChain.java:1719-1724`), which the compressing
//!   writer reports as `positionsBuf + startOffsetsBuf + lengthsBuf +
//!   payloadLengthsBuf + termSuffixes + payloadBytes + lastTerm + scratchBuffer`
//!   (`Lucene90CompressingTermVectorsWriter.java:999-1008`). Rucene's
//!   [`TermVectorsWriter`] has
//!   no `ram_bytes_used`, so [`TermVectorsConsumer::ram_bytes_used`] reports
//!   zero always. The four `int[]` in that sum grow with the position count of
//!   the largest document seen and are never shrunk, so the undercount is
//!   **not** bounded by the chunk size: one document with a million positions
//!   leaves 16 MB uncounted for the rest of the segment. It shifts only when a
//!   flush is triggered, never what is written, but a segment of very large
//!   documents can overshoot its RAM budget. Giving the trait a
//!   `ram_bytes_used` is the fix, and is filed separately.
//! * **Field names are ordered with a UTF-16 comparator.** Java sorts the
//!   pending fields with `TermsHashPerField.compareTo`, which is
//!   `String.compareTo` — UTF-16 code-unit order. The order reaches the `.tvd`
//!   bytes, so [`compare_utf16`] is used rather than Rust's UTF-8 `str`
//!   ordering; the two disagree above `U+E000`.
//!
//! # Index sorting
//!
//! Lucene subclasses this consumer with `SortingTermVectorsConsumer`, which
//! buffers the vectors into a temporary, uncompressed term-vectors file during
//! indexing and, at flush time, replays every document through the real codec
//! writer in the order a `Sorter.DocMap` gives. It overrides exactly two
//! members: `initTermVectorsWriter()`, to redirect the writer at a
//! `TrackingTmpOutputDirectoryWrapper`, and `flush(...)`, to do the replay. Both
//! of its dependencies — `Sorter.DocMap` and
//! `TrackingTmpOutputDirectoryWrapper` — belong to the index-sorting port, so
//! this type stops where `SortingStoredFieldsConsumer` stopped, and for the same
//! reason. The two extension points are in place and are not expected to change:
//! writer creation is isolated in the public
//! [`TermVectorsConsumer::init_term_vectors_writer`], which is the only place
//! the writer is built, and [`TermVectorsConsumer::flush`] already takes the
//! document count of the segment being flushed. Neither is an override point in
//! the Java sense: this is a concrete struct behind no trait, so a sorting
//! variant has to change its shape — by giving it a writer-factory field, by
//! wrapping it, or by introducing a trait — rather than subclass it. What is
//! promised here is only that the two seams are isolated and that no caller
//! reaches around them.

#![deny(unsafe_code)]

use std::collections::HashMap;
use std::sync::atomic::AtomicI64;
use std::sync::Arc;

use crate::codecs::term_vectors::TermVectorsWriter;
use crate::codecs::Codec;
use crate::error::{LuceneError, Result};
use crate::index::freq_prox_terms_writer::{
    ByteSliceReader, InvertedToken, PooledSliceReader, TermsHash, TermsHashPerField,
};
use crate::index::indexing_chain::FieldInvertState;
use crate::index::{FieldInfo, IndexableField, SegmentInfo};
use crate::store::{flush_io_context, DataInput, Directory, FlushInfo};
use crate::util::byte_block_pool::ByteBlockPool;
use crate::util::{compare_utf16, BytesRef};

/// Stream carrying the positions and payloads of a term.
///
/// Equivalent to the literal `0` Lucene passes to `writeVInt`/`writeBytes` in
/// `TermVectorsConsumerPerField.writeProx`. Note that the postings writer uses
/// the opposite convention — there, stream `0` is the doc list.
const POSITIONS_STREAM: usize = 0;

/// Stream carrying the offsets of a term.
///
/// Equivalent to the literal `1` in `TermVectorsConsumerPerField.writeProx`.
const OFFSETS_STREAM: usize = 1;

// ---------------------------------------------------------------------------
// TermVectorsConsumerPerField
// ---------------------------------------------------------------------------

/// Buffers the term vector of one field of the document in flight.
///
/// Equivalent to `org.apache.lucene.index.TermVectorsConsumerPerField`.
#[derive(Debug)]
pub struct TermVectorsConsumerPerField {
    base: TermsHashPerField,
    field_info: FieldInfo,
    do_vectors: bool,
    do_vector_positions: bool,
    do_vector_offsets: bool,
    do_vector_payloads: bool,
    /// `true` once a token of this field carried a non-empty payload *and*
    /// payloads were requested. Equivalent to
    /// `TermVectorsConsumerPerField.hasPayloads`.
    has_payloads: bool,
    /// `true` once the field's vector reached the codec writer for the current
    /// document, which is where Java calls `FieldInfo.setStoreTermVectors()`.
    wrote_vectors: bool,
}

impl TermVectorsConsumerPerField {
    /// Creates the per-field table of `field_info`.
    ///
    /// Equivalent to
    /// `TermVectorsConsumerPerField(FieldInvertState, TermVectorsConsumer, FieldInfo)`,
    /// which asks its base class for two streams and for a hash keyed on the
    /// primary hash's pool.
    ///
    /// # Panics
    ///
    /// Panics if the field's index options are
    /// [`IndexOptions::NONE`](crate::index::IndexOptions::NONE); Lucene asserts
    /// the same in `TermsHashPerField`'s constructor.
    pub fn new(field_info: FieldInfo, bytes_used: Arc<AtomicI64>) -> Self {
        Self {
            base: TermsHashPerField::new_chained(
                2,
                field_info.name.clone(),
                field_info.index_options,
                bytes_used,
            ),
            field_info,
            do_vectors: false,
            do_vector_positions: false,
            do_vector_offsets: false,
            do_vector_payloads: false,
            has_payloads: false,
            wrote_vectors: false,
        }
    }

    /// Returns the field's metadata.
    pub fn field_info(&self) -> &FieldInfo {
        &self.field_info
    }

    /// Returns the field name.
    pub fn field_name(&self) -> &str {
        &self.field_info.name
    }

    /// Returns the number of distinct terms buffered for the current document.
    pub fn num_terms(&self) -> i32 {
        self.base.num_terms()
    }

    /// Returns `true` when the current document's tokens are being collected.
    ///
    /// Equivalent to reading the private `doVectors` field.
    pub fn collects_vectors(&self) -> bool {
        self.do_vectors
    }

    /// Returns `true` when the field's vector reached the writer for the
    /// document that was just finished.
    pub fn wrote_vectors(&self) -> bool {
        self.wrote_vectors
    }

    /// Hands every accounted byte back to the shared counter.
    pub fn release_accounting(&mut self) {
        self.base.release_accounting();
        self.do_vectors = false;
        self.has_payloads = false;
    }

    /// Starts one instance of the field in the current document and returns
    /// whether its tokens should be collected.
    ///
    /// Equivalent to `TermVectorsConsumerPerField.start(IndexableField, boolean)`.
    /// `first` is `true` for the first instance of the field name in the
    /// document; every later instance must agree on all four settings.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when the field type asks for
    /// term-vector positions, offsets or payloads without the prerequisite —
    /// payloads need positions, and all three need term vectors — or when a
    /// later instance of the field disagrees with the first.
    pub fn start(&mut self, field: &dyn IndexableField, first: bool) -> Result<bool> {
        let field_type = field.field_type();
        let name = field.name();
        if first {
            if self.base.num_terms() != 0 {
                // Only reachable when the previous document hit a
                // non-aborting error while writing this field's vectors
                // (`TermVectorsConsumerPerField.java:138-143`).
                self.base.reset();
            }
            self.has_payloads = false;
            self.do_vectors = field_type.store_term_vectors();

            if self.do_vectors {
                self.do_vector_positions = field_type.store_term_vector_positions();
                // Somewhat confusingly, and unlike postings, term-vector
                // offsets may be indexed without term-vector positions
                // (`TermVectorsConsumerPerField.java:154-156`).
                self.do_vector_offsets = field_type.store_term_vector_offsets();
                if self.do_vector_positions {
                    self.do_vector_payloads = field_type.store_term_vector_payloads();
                } else {
                    self.do_vector_payloads = false;
                    if field_type.store_term_vector_payloads() {
                        return Err(LuceneError::IllegalArgument(format!(
                            "cannot index term vector payloads without term vector positions (field=\"{name}\")"
                        )));
                    }
                }
            } else {
                if field_type.store_term_vector_offsets() {
                    return Err(LuceneError::IllegalArgument(format!(
                        "cannot index term vector offsets when term vectors are not indexed (field=\"{name}\")"
                    )));
                }
                if field_type.store_term_vector_positions() {
                    return Err(LuceneError::IllegalArgument(format!(
                        "cannot index term vector positions when term vectors are not indexed (field=\"{name}\")"
                    )));
                }
                if field_type.store_term_vector_payloads() {
                    return Err(LuceneError::IllegalArgument(format!(
                        "cannot index term vector payloads when term vectors are not indexed (field=\"{name}\")"
                    )));
                }
            }
        } else {
            Self::require_same(
                self.do_vectors,
                field_type.store_term_vectors(),
                "storeTermVectors",
                name,
            )?;
            Self::require_same(
                self.do_vector_positions,
                field_type.store_term_vector_positions(),
                "storeTermVectorPositions",
                name,
            )?;
            Self::require_same(
                self.do_vector_offsets,
                field_type.store_term_vector_offsets(),
                "storeTermVectorOffsets",
                name,
            )?;
            Self::require_same(
                self.do_vector_payloads,
                field_type.store_term_vector_payloads(),
                "storeTermVectorPayloads",
                name,
            )?;
        }
        Ok(self.do_vectors)
    }

    /// Reports the message Lucene raises when two instances of one field name
    /// disagree on a term-vector setting.
    fn require_same(expected: bool, actual: bool, setting: &str, name: &str) -> Result<()> {
        if expected == actual {
            return Ok(());
        }
        Err(LuceneError::IllegalArgument(format!(
            "all instances of a given field name must have the same term vectors settings \
             ({setting} changed for field=\"{name}\")"
        )))
    }

    /// Records one occurrence of the term interned at `text_start`.
    ///
    /// Equivalent to `TermsHashPerField.add(int, int)` followed by
    /// `TermVectorsConsumerPerField.newTerm` or `.addTerm`. `streams` is this
    /// consumer's own byte pool; `text_start` addresses the *postings* pool.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when a custom term frequency is
    /// combined with term-vector positions or offsets, and propagates
    /// pool-overflow errors.
    pub fn add(
        &mut self,
        streams: &mut ByteBlockPool,
        field_state: &FieldInvertState,
        text_start: i32,
        token: &InvertedToken<'_>,
    ) -> Result<()> {
        let slot = self.base.intern_by_text_start(streams, text_start)?;
        let freq = self.term_freq(token)?;
        if slot.is_new {
            let posting = self.base.posting_mut(slot.term_id);
            posting.term_freq = freq;
            posting.last_offset = 0;
            posting.last_position = 0;
        } else {
            let posting = self.base.posting_mut(slot.term_id);
            posting.term_freq = posting.term_freq.checked_add(freq).ok_or_else(|| {
                // Java lets this `int` overflow silently; the document would
                // need more than `Integer.MAX_VALUE` occurrences of one term,
                // which `FieldInvertState.length` rejects first.
                LuceneError::IllegalArgument(format!(
                    "too many tokens for field \"{}\"",
                    self.field_info.name
                ))
            })?;
        }
        self.write_prox(streams, field_state, slot.term_id, token)
    }

    /// Returns the frequency to record for the current token.
    ///
    /// Equivalent to `TermVectorsConsumerPerField.getTermFreq()`.
    fn term_freq(&self, token: &InvertedToken<'_>) -> Result<i32> {
        if !token.has_term_freq_attribute {
            return Ok(1);
        }
        let freq = token.term_freq;
        if freq != 1 {
            if self.do_vector_positions {
                return Err(LuceneError::IllegalArgument(format!(
                    "field \"{}\": cannot index term vector positions while using custom TermFrequencyAttribute",
                    self.field_info.name
                )));
            }
            if self.do_vector_offsets {
                return Err(LuceneError::IllegalArgument(format!(
                    "field \"{}\": cannot index term vector offsets while using custom TermFrequencyAttribute",
                    self.field_info.name
                )));
            }
        }
        Ok(freq)
    }

    /// Appends the offsets and the position of the current token to the term's
    /// two streams.
    ///
    /// Equivalent to
    /// `TermVectorsConsumerPerField.writeProx(TermVectorsPostingsArray, int)`.
    fn write_prox(
        &mut self,
        streams: &mut ByteBlockPool,
        field_state: &FieldInvertState,
        term_id: i32,
        token: &InvertedToken<'_>,
    ) -> Result<()> {
        if self.do_vector_offsets {
            let start_offset = field_state.offset() + token.start_offset;
            let end_offset = field_state.offset() + token.end_offset;
            // Unlike the postings writer, the term-vectors writer stores the
            // previous *end* offset, so the gap written is the distance between
            // the end of the last occurrence and the start of this one
            // (`TermVectorsConsumerPerField.java:240-242`, decoded back by
            // `TermVectorsWriter.addProx`).
            let last_offset = self.base.posting(term_id).last_offset;
            self.base
                .write_v_int(streams, term_id, OFFSETS_STREAM, start_offset - last_offset)?;
            self.base
                .write_v_int(streams, term_id, OFFSETS_STREAM, end_offset - start_offset)?;
            self.base.posting_mut(term_id).last_offset = end_offset;
        }

        if self.do_vector_positions {
            // Java nulls out `payloadAttribute` when payloads were not asked
            // for (`TermVectorsConsumerPerField.java:224-229`), so a payload on
            // the token is ignored unless the field type requested one.
            let payload = if self.do_vector_payloads {
                token.payload
            } else {
                None
            };
            let position = field_state.position();
            let delta = position - self.base.posting(term_id).last_position;
            // `delta << 1` overflows for a delta above `Integer.MAX_VALUE / 2`,
            // which `IndexWriter.MAX_POSITION` still permits. Java wraps
            // silently and `TermVectorsWriter.addProx` recovers the value with
            // `>>>`; going through `u32` reproduces both halves instead of
            // aborting a debug build.
            let code = (delta as u32) << 1;
            match payload {
                Some(payload) if !payload.is_empty() => {
                    self.base
                        .write_v_int(streams, term_id, POSITIONS_STREAM, (code | 1) as i32)?;
                    self.base.write_v_int(
                        streams,
                        term_id,
                        POSITIONS_STREAM,
                        payload.len() as i32,
                    )?;
                    self.base
                        .write_bytes(streams, term_id, POSITIONS_STREAM, payload)?;
                    self.has_payloads = true;
                }
                _ => {
                    self.base
                        .write_v_int(streams, term_id, POSITIONS_STREAM, code as i32)?;
                }
            }
            self.base.posting_mut(term_id).last_position = position;
        }
        Ok(())
    }

    /// Returns whether the field has a vector to write for this document.
    ///
    /// Equivalent to `TermVectorsConsumerPerField.finish()`, which calls
    /// `TermVectorsConsumer.addFieldToFlush(this)` exactly when this is `true`.
    pub fn finish(&self) -> bool {
        self.do_vectors && self.base.num_terms() != 0
    }

    /// Writes this field's vector for the current document and clears the
    /// buffers it used.
    ///
    /// Equivalent to `TermVectorsConsumerPerField.finishDocument()`.
    /// `streams` is this consumer's byte pool, `term_pool` the postings pool
    /// holding the term text, and `scratch` the reusable buffer Lucene keeps as
    /// `TermVectorsConsumer.flushTerm`.
    ///
    /// # Errors
    ///
    /// Propagates any error raised by the codec writer.
    fn finish_document(
        &mut self,
        writer: &mut dyn TermVectorsWriter,
        streams: &ByteBlockPool,
        term_pool: &ByteBlockPool,
        scratch: &mut BytesRef,
    ) -> Result<()> {
        if !self.do_vectors {
            return Ok(());
        }
        self.do_vectors = false;

        let num_postings = self.base.num_terms();
        let term_ids = self.base.sorted_term_ids(term_pool);

        writer.start_field(
            &self.field_info,
            num_postings,
            self.do_vector_positions,
            self.do_vector_offsets,
            self.has_payloads,
        )?;

        for term_id in term_ids {
            let freq = self.base.posting(term_id).term_freq;
            let bytes = term_pool.term_bytes(self.base.text_start(term_id));
            scratch.bytes.clear();
            scratch.bytes.extend_from_slice(bytes);
            scratch.offset = 0;
            scratch.length = bytes.len();
            writer.start_term(scratch, freq)?;

            if self.do_vector_positions || self.do_vector_offsets {
                let mut positions = self.do_vector_positions.then(|| {
                    let mut reader = ByteSliceReader::new();
                    self.base
                        .init_reader(&mut reader, term_id, POSITIONS_STREAM);
                    PooledSliceReader::new(reader, streams)
                });
                let mut offsets = self.do_vector_offsets.then(|| {
                    let mut reader = ByteSliceReader::new();
                    self.base.init_reader(&mut reader, term_id, OFFSETS_STREAM);
                    PooledSliceReader::new(reader, streams)
                });
                writer.add_prox(
                    freq,
                    positions
                        .as_mut()
                        .map(|reader| reader as &mut dyn DataInput),
                    offsets.as_mut().map(|reader| reader as &mut dyn DataInput),
                )?;
            }
            writer.finish_term()?;
        }
        writer.finish_field()?;

        self.base.reset();
        self.wrote_vectors = true;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// TermVectorsConsumer
// ---------------------------------------------------------------------------

/// Writes the term vectors of one segment.
///
/// Equivalent to `org.apache.lucene.index.TermVectorsConsumer`. See the module
/// documentation for the specification this type implements.
pub struct TermVectorsConsumer {
    codec: Arc<dyn Codec>,
    directory: Arc<dyn Directory>,
    info: SegmentInfo,
    writer: Option<Box<dyn TermVectorsWriter>>,
    /// The second term hash of the chain: its byte pool holds the position and
    /// offset streams of the document in flight.
    terms_hash: TermsHash,
    /// The private RAM counter of the per-field term tables.
    ///
    /// Java passes `Counter.newCounter()` to the `TermsHash` constructor
    /// (`TermVectorsConsumer.java:65`) while the two block pools charge the
    /// `DocumentsWriterPerThread`'s counter through the shared allocators, so
    /// the hash tables and posting slots are deliberately invisible to
    /// `IndexingChain.ramBytesUsed()`. This is that throwaway counter; the
    /// stream pool still charges the chain's.
    field_bytes_used: Arc<AtomicI64>,
    fields: Vec<TermVectorsConsumerPerField>,
    field_index: HashMap<String, usize>,
    /// Indices into `fields` of the fields that have a vector to write for the
    /// current document. Equivalent to `perFields[0..numVectorFields]`.
    pending_fields: Vec<usize>,
    /// Reusable term buffer. Equivalent to `TermVectorsConsumer.flushTerm`.
    flush_term: BytesRef,
    has_vectors: bool,
    last_doc_id: i32,
    aborted: bool,
}

impl std::fmt::Debug for TermVectorsConsumer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TermVectorsConsumer")
            .field("segment", &self.info.name)
            .field("has_writer", &self.writer.is_some())
            .field("has_vectors", &self.has_vectors)
            .field("last_doc_id", &self.last_doc_id)
            .finish_non_exhaustive()
    }
}

impl TermVectorsConsumer {
    /// Creates a consumer that will write the term vectors of `info`.
    ///
    /// Equivalent to
    /// `TermVectorsConsumer(IntBlockPool.Allocator, ByteBlockPool.Allocator, Directory, SegmentInfo, Codec)`.
    /// No file is created until a document actually carries vectors.
    ///
    /// `directory` must be the tracking directory of the segment, so that the
    /// files the codec creates are recorded in the segment's file list.
    /// `bytes_used` is the chain's RAM counter, which the *stream pool* charges,
    /// exactly as Java's shared `ByteBlockPool.DirectTrackingAllocator` does;
    /// the per-field term tables charge a private counter instead, matching the
    /// `Counter.newCounter()` Java hands to the `TermsHash` constructor.
    pub fn new(
        codec: Arc<dyn Codec>,
        directory: Arc<dyn Directory>,
        info: SegmentInfo,
        bytes_used: Arc<AtomicI64>,
    ) -> Self {
        Self {
            codec,
            directory,
            info,
            writer: None,
            terms_hash: TermsHash::new(bytes_used),
            field_bytes_used: Arc::new(AtomicI64::new(0)),
            fields: Vec::new(),
            field_index: HashMap::new(),
            pending_fields: Vec::new(),
            flush_term: BytesRef::default(),
            has_vectors: false,
            last_doc_id: 0,
            aborted: false,
        }
    }

    /// Returns `true` once the codec writer has been created.
    ///
    /// The term-vectors files of a segment exist exactly when this is `true`.
    pub fn has_writer(&self) -> bool {
        self.writer.is_some()
    }

    /// Returns `true` once a field storing term vectors has been registered.
    ///
    /// Equivalent to reading the private `hasVectors` field.
    pub fn has_vectors(&self) -> bool {
        self.has_vectors
    }

    /// Returns the number of documents whose frame has been written.
    ///
    /// Equivalent to reading `TermVectorsConsumer.lastDocID`.
    pub fn last_doc_id(&self) -> i32 {
        self.last_doc_id
    }

    /// Returns `true` once [`Self::abort`] has run.
    pub fn is_aborted(&self) -> bool {
        self.aborted
    }

    /// Returns the approximate heap usage this consumer adds on top of the
    /// chain's RAM counter, which is zero.
    ///
    /// Equivalent to `TermVectorsConsumer.accountable.ramBytesUsed()`. The
    /// stream pool is already charged to the chain's counter, so adding it here
    /// would count it twice, and the per-field tables are deliberately
    /// uncounted in Java too. What is missing is the codec writer's own buffer,
    /// which the [`TermVectorsWriter`] trait does not expose and which grows
    /// without bound with the largest document of the segment — see the module
    /// documentation.
    pub fn ram_bytes_used(&self) -> i64 {
        0
    }

    /// Records that the segment has at least one field storing term vectors.
    ///
    /// Equivalent to `TermVectorsConsumer.setHasVectors()`, which
    /// `IndexingChain.PerField.setInvertState` calls the first time a field
    /// whose `FieldInfo` stores term vectors is registered. It is never
    /// cleared: from the document that registered the field onwards, every
    /// document of the segment occupies a frame in the stream.
    pub fn set_has_vectors(&mut self) {
        self.has_vectors = true;
    }

    /// Returns the per-field table for `field_info`, creating it on first use.
    ///
    /// Equivalent to `TermVectorsConsumer.addField(FieldInvertState, FieldInfo)`
    /// combined with the `PerField` cache of `IndexingChain`. Callers keep the
    /// returned index and pass it back to [`Self::start_field`],
    /// [`Self::add`] and [`Self::finish_field`].
    pub fn add_field(&mut self, field_info: &FieldInfo) -> usize {
        if let Some(index) = self.field_index.get(&field_info.name) {
            return *index;
        }
        let index = self.fields.len();
        self.fields.push(TermVectorsConsumerPerField::new(
            field_info.clone(),
            Arc::clone(&self.field_bytes_used),
        ));
        self.field_index.insert(field_info.name.clone(), index);
        index
    }

    /// Returns the per-field table of `index`.
    ///
    /// # Panics
    ///
    /// Panics if `index` was not returned by [`Self::add_field`].
    pub fn field(&self, index: usize) -> &TermVectorsConsumerPerField {
        &self.fields[index]
    }

    /// Returns the per-field table for `field_name`, if the field was seen.
    pub fn field_by_name(&self, field_name: &str) -> Option<&TermVectorsConsumerPerField> {
        self.field_index
            .get(field_name)
            .map(|index| &self.fields[*index])
    }

    /// Opens a new document.
    ///
    /// Equivalent to `TermVectorsConsumer.startDocument()`, which is reached
    /// through `TermsHash.startDocument()` before any field of the document is
    /// processed.
    pub fn start_document(&mut self) {
        self.reset_fields();
    }

    /// Starts one instance of the field at `index` and returns whether its
    /// tokens should be routed to this consumer.
    ///
    /// Equivalent to `TermVectorsConsumerPerField.start(IndexableField, boolean)`,
    /// whose return value Lucene stores as `TermsHashPerField.doNextCall`.
    ///
    /// # Errors
    ///
    /// Propagates the field-type validation errors described on
    /// [`TermVectorsConsumerPerField::start`].
    pub fn start_field(
        &mut self,
        index: usize,
        field: &dyn IndexableField,
        first: bool,
    ) -> Result<bool> {
        self.fields[index].start(field, first)
    }

    /// Records one occurrence of the term interned at `text_start` in the field
    /// at `index`.
    ///
    /// Equivalent to the `nextPerField.add(postingsArray.textStarts[termID],
    /// docID)` call at the end of `TermsHashPerField.add(BytesRef, int)`.
    ///
    /// # Errors
    ///
    /// Propagates the errors described on [`TermVectorsConsumerPerField::add`].
    pub fn add(
        &mut self,
        index: usize,
        field_state: &FieldInvertState,
        text_start: i32,
        token: &InvertedToken<'_>,
    ) -> Result<()> {
        let Self {
            terms_hash, fields, ..
        } = self;
        fields[index].add(terms_hash.pool_mut(), field_state, text_start, token)
    }

    /// Closes the field at `index` for the current document, queueing its
    /// vector when it has one.
    ///
    /// Equivalent to `TermVectorsConsumerPerField.finish()` plus
    /// `TermVectorsConsumer.addFieldToFlush(TermVectorsConsumerPerField)`.
    pub fn finish_field(&mut self, index: usize) {
        self.fields[index].wrote_vectors = false;
        if self.fields[index].finish() {
            self.pending_fields.push(index);
        }
    }

    /// Creates the codec writer if it does not exist yet.
    ///
    /// Equivalent to `TermVectorsConsumer.initTermVectorsWriter()`. It is the
    /// only place the writer is built, which is the seam an index-sorting
    /// variant needs in order to redirect the stream at a temporary directory —
    /// though, this being a concrete struct rather than a base class, such a
    /// variant will have to change this type's shape rather than override the
    /// method.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::AlreadyClosed`] once the consumer was aborted, and
    /// propagates any error raised while creating the segment's term-vectors
    /// files.
    pub fn init_term_vectors_writer(&mut self) -> Result<()> {
        if self.aborted {
            // The same deliberate deviation `StoredFieldsConsumer` makes, for a
            // state the `DocumentsWriterPerThread` contract never reaches: Rust
            // has to release the writer in order to close it, so without this
            // guard the lazy initialisation would build a second set of
            // `.tvd`/`.tvx`/`.tvm` files for a discarded segment.
            return Err(LuceneError::AlreadyClosed(format!(
                "the term vectors of segment {} were aborted",
                self.info.name
            )));
        }
        if self.writer.is_none() {
            // `IOContext.flush(new FlushInfo(lastDocID, bytesUsed.get()))`
            // (`TermVectorsConsumer.java:105`), where `bytesUsed` is the
            // `TermsHash`'s own counter — the private one, not the chain's.
            let context = flush_io_context(FlushInfo::new(
                self.last_doc_id,
                self.field_bytes_used
                    .load(std::sync::atomic::Ordering::Acquire),
            ));
            self.writer = Some(self.codec.term_vectors_format().vectors_writer(
                self.directory.as_ref(),
                &self.info,
                context.as_ref(),
            )?);
            self.last_doc_id = 0;
        }
        Ok(())
    }

    /// Writes an empty frame for every document below `doc_id` that never had
    /// vectors.
    ///
    /// Equivalent to `TermVectorsConsumer.fill(int)`.
    ///
    /// # Errors
    ///
    /// Propagates any error raised by the codec writer. The cursor is left on
    /// the last frame that actually reached it.
    fn fill(writer: &mut dyn TermVectorsWriter, last_doc_id: &mut i32, doc_id: i32) -> Result<()> {
        while *last_doc_id < doc_id {
            writer.start_document(0)?;
            writer.finish_document()?;
            *last_doc_id += 1;
        }
        Ok(())
    }

    /// Writes the term vectors of the document that just finished.
    ///
    /// Equivalent to `TermVectorsConsumer.finishDocument(int)`. `term_pool` is
    /// the postings hash's byte pool, which holds the term text.
    ///
    /// Nothing at all happens while no field of the segment stores term
    /// vectors. Once one does, every document from then on writes a frame, and
    /// every document *before* it is back-filled with an empty one.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] when `doc_id` does not follow the
    /// document last written — Lucene asserts `lastDocID == docID` — and
    /// propagates any error raised by the codec writer.
    pub fn finish_document(&mut self, doc_id: i32, term_pool: &ByteBlockPool) -> Result<()> {
        if !self.has_vectors {
            return Ok(());
        }

        // "Fields in term vectors are UTF16 sorted"
        // (`TermVectorsConsumer.java:123-124`): `ArrayUtil.introSort` on
        // `TermsHashPerField.compareTo`, i.e. on `String.compareTo`.
        let Self {
            fields,
            pending_fields,
            ..
        } = self;
        pending_fields.sort_by(|left, right| {
            compare_utf16(fields[*left].field_name(), fields[*right].field_name())
        });

        self.init_term_vectors_writer()?;

        let Self {
            writer,
            fields,
            pending_fields,
            terms_hash,
            flush_term,
            last_doc_id,
            ..
        } = self;
        let writer = writer.as_mut().ok_or_else(|| {
            LuceneError::IllegalState(
                "the term-vectors writer was not created by initTermVectorsWriter".to_string(),
            )
        })?;

        Self::fill(writer.as_mut(), last_doc_id, doc_id)?;
        if *last_doc_id != doc_id {
            return Err(LuceneError::IllegalState(format!(
                "term vectors must be written in increasing doc order: lastDocID={last_doc_id} docID={doc_id}"
            )));
        }

        writer.start_document(pending_fields.len() as i32)?;
        let streams = terms_hash.pool();
        for index in pending_fields.iter() {
            fields[*index].finish_document(writer.as_mut(), streams, term_pool, flush_term)?;
        }
        writer.finish_document()?;
        *last_doc_id += 1;

        // `super.reset()` drops the position and offset streams of the document
        // that just finished; `resetFields()` empties the pending list.
        self.terms_hash.reset();
        self.reset_fields();
        Ok(())
    }

    /// Empties the list of fields pending for the current document.
    ///
    /// Equivalent to `TermVectorsConsumer.resetFields()`.
    fn reset_fields(&mut self) {
        self.pending_fields.clear();
    }

    /// Pads the stream to `max_doc` frames, finishes the writer and closes it.
    ///
    /// Equivalent to
    /// `TermVectorsConsumer.flush(Map, SegmentWriteState, Sorter.DocMap, NormsProducer)`
    /// for an unsorted segment. With no writer — no document of the segment ever
    /// had vectors — nothing is written and no file is created.
    ///
    /// The writer is closed even when padding or finishing it failed, mirroring
    /// Java's `try`/`finally`; the first error wins.
    ///
    /// # Errors
    ///
    /// Propagates any error raised while writing the trailing frames, finishing
    /// the writer or closing it.
    pub fn flush(&mut self, max_doc: i32) -> Result<()> {
        let Some(mut writer) = self.writer.take() else {
            return Ok(());
        };
        let filled = Self::fill(writer.as_mut(), &mut self.last_doc_id, max_doc);
        let finished = filled.and_then(|()| writer.finish(max_doc));
        let closed = writer.close();
        finished.and(closed)
    }

    /// Discards the segment's term vectors.
    ///
    /// Equivalent to `TermVectorsConsumer.abort()`, which resets the buffers,
    /// closes the writer with `IOUtils.closeWhileHandlingException` and resets
    /// again. Calling this without a writer is a no-op.
    pub fn abort(&mut self) {
        self.aborted = true;
        for field in &mut self.fields {
            field.release_accounting();
        }
        self.fields.clear();
        self.field_index.clear();
        self.pending_fields.clear();
        self.terms_hash.reset();
        if let Some(mut writer) = self.writer.take() {
            let _ = writer.close();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::atomic::Ordering;

    use super::*;
    use crate::codecs::{register_codec, Lucene104Codec};
    use crate::index::IndexOptions;
    use crate::store::{ByteBuffersDirectory, DataInput, TrackingDirectoryWrapper};
    use crate::util::{Accountable, Version};

    fn codec() -> Arc<dyn Codec> {
        let _ = register_codec("Lucene104", Lucene104Codec::new());
        crate::codecs::default_codec().expect("Lucene104 codec is registered")
    }

    /// Builds a consumer over a fresh in-memory segment.
    fn consumer() -> (TermVectorsConsumer, Arc<TrackingDirectoryWrapper>) {
        let codec = codec();
        let tracking = Arc::new(TrackingDirectoryWrapper::new(Box::new(
            ByteBuffersDirectory::new(),
        )));
        let directory: Arc<dyn Directory> = Arc::clone(&tracking) as Arc<dyn Directory>;
        let info = SegmentInfo::new(
            Arc::clone(&directory),
            Version::LATEST,
            Some(Version::LATEST),
            "_0".to_string(),
            -1,
            false,
            false,
            Arc::clone(&codec),
            HashMap::new(),
            [3u8; 16],
            HashMap::new(),
            Default::default(),
        )
        .expect("segment info");
        let consumer = TermVectorsConsumer::new(
            codec,
            Arc::clone(&tracking) as Arc<dyn Directory>,
            info,
            Arc::new(AtomicI64::new(0)),
        );
        (consumer, tracking)
    }

    fn vector_field_info(name: &str, number: i32) -> FieldInfo {
        let mut info = FieldInfo::new(name, number);
        info.index_options = IndexOptions::DOCS_AND_FREQS_AND_POSITIONS;
        info.set_store_term_vectors().expect("store term vectors");
        info
    }

    fn token(start: i32, end: i32) -> InvertedToken<'static> {
        InvertedToken {
            start_offset: start,
            end_offset: end,
            payload: None,
            term_freq: 1,
            has_term_freq_attribute: false,
        }
    }

    // -- Consumer lifecycle ------------------------------------------------

    #[test]
    fn a_segment_whose_fields_never_asked_for_vectors_writes_nothing() {
        let (mut consumer, tracking) = consumer();
        let term_pool = ByteBlockPool::new(Arc::new(AtomicI64::new(0)));
        consumer.start_document();
        consumer
            .finish_document(0, &term_pool)
            .expect("no vectors, nothing to do");
        assert!(!consumer.has_writer());
        consumer.flush(1).expect("flush");
        assert!(tracking.get_created_files().is_empty());
    }

    #[test]
    fn flushing_without_a_writer_is_a_no_op() {
        let (mut consumer, tracking) = consumer();
        consumer.set_has_vectors();
        consumer.flush(10).expect("flush");
        assert!(tracking.get_created_files().is_empty());
        assert_eq!(consumer.last_doc_id(), 0);
    }

    #[test]
    fn the_first_vector_document_back_fills_every_document_before_it() {
        let (mut consumer, _tracking) = consumer();
        let term_pool = ByteBlockPool::new(Arc::new(AtomicI64::new(0)));
        consumer.set_has_vectors();
        consumer.start_document();
        // Nothing was collected, yet doc 3 still gets its frame and docs 0-2
        // get an empty one each.
        consumer.finish_document(3, &term_pool).expect("finish");
        assert!(consumer.has_writer());
        assert_eq!(consumer.last_doc_id(), 4);
        consumer.flush(6).expect("flush");
        assert_eq!(
            consumer.last_doc_id(),
            6,
            "the tail is padded up to max_doc"
        );
    }

    #[test]
    fn a_document_written_out_of_order_is_rejected() {
        let (mut consumer, _tracking) = consumer();
        let term_pool = ByteBlockPool::new(Arc::new(AtomicI64::new(0)));
        consumer.set_has_vectors();
        consumer.start_document();
        consumer.finish_document(4, &term_pool).expect("finish");
        consumer.start_document();
        let error = consumer
            .finish_document(2, &term_pool)
            .expect_err("doc 2 was already covered by the back-fill");
        assert!(matches!(error, LuceneError::IllegalState(_)), "{error:?}");
    }

    #[test]
    fn an_aborted_consumer_refuses_to_create_a_second_writer() {
        let (mut consumer, _tracking) = consumer();
        let term_pool = ByteBlockPool::new(Arc::new(AtomicI64::new(0)));
        consumer.set_has_vectors();
        consumer.start_document();
        consumer.finish_document(0, &term_pool).expect("finish");
        assert!(consumer.has_writer());

        consumer.abort();
        assert!(consumer.is_aborted());
        assert!(!consumer.has_writer());

        let error = consumer
            .init_term_vectors_writer()
            .expect_err("a discarded segment must not grow a second set of files");
        assert!(matches!(error, LuceneError::AlreadyClosed(_)), "{error:?}");
    }

    #[test]
    fn aborting_hands_every_accounted_byte_back() {
        let bytes_used = Arc::new(AtomicI64::new(0));
        let codec = codec();
        let tracking = Arc::new(TrackingDirectoryWrapper::new(Box::new(
            ByteBuffersDirectory::new(),
        )));
        let directory: Arc<dyn Directory> = Arc::clone(&tracking) as Arc<dyn Directory>;
        let info = SegmentInfo::new(
            Arc::clone(&directory),
            Version::LATEST,
            Some(Version::LATEST),
            "_0".to_string(),
            -1,
            false,
            false,
            Arc::clone(&codec),
            HashMap::new(),
            [3u8; 16],
            HashMap::new(),
            Default::default(),
        )
        .expect("segment info");
        let mut consumer =
            TermVectorsConsumer::new(codec, directory, info, Arc::clone(&bytes_used));
        let index = consumer.add_field(&vector_field_info("body", 0));
        consumer.fields[index].do_vectors = true;
        consumer.fields[index].do_vector_positions = true;

        let mut term_pool = ByteBlockPool::new(Arc::clone(&bytes_used));
        let mut state = FieldInvertState::new(
            10,
            "body".to_string(),
            IndexOptions::DOCS_AND_FREQS_AND_POSITIONS,
        );
        for term in 0..64 {
            let text_start = term_pool
                .add_bytes_ref(format!("term{term}").as_bytes())
                .expect("intern");
            state.set_position(term);
            consumer
                .add(index, &state, text_start, &token(0, 4))
                .expect("add");
        }
        let charged_before = bytes_used.load(Ordering::Acquire);
        let term_pool_bytes = term_pool.ram_bytes_used();
        assert!(charged_before > term_pool_bytes);

        consumer.abort();
        assert_eq!(
            bytes_used.load(Ordering::Acquire),
            term_pool_bytes,
            "only the caller's own term pool may still be charged"
        );
    }

    #[test]
    fn add_field_returns_the_same_index_for_a_field_it_already_knows() {
        let (mut consumer, _tracking) = consumer();
        let info = vector_field_info("body", 0);
        let first = consumer.add_field(&info);
        let again = consumer.add_field(&info);
        assert_eq!(first, again);
        assert_eq!(consumer.field(first).field_name(), "body");
        assert!(consumer.field_by_name("body").is_some());
        assert!(consumer.field_by_name("missing").is_none());
    }

    #[test]
    fn setting_has_vectors_is_sticky() {
        let (mut consumer, _tracking) = consumer();
        assert!(!consumer.has_vectors());
        consumer.set_has_vectors();
        assert!(consumer.has_vectors());
        consumer.start_document();
        assert!(consumer.has_vectors(), "a new document does not clear it");
    }

    // -- Per-field encoding ------------------------------------------------

    /// Drives one per-field table directly, bypassing the field type, so that
    /// the raw streams can be inspected byte for byte.
    fn buffered_field(
        positions: bool,
        offsets: bool,
        payloads: bool,
    ) -> (
        TermVectorsConsumerPerField,
        ByteBlockPool,
        ByteBlockPool,
        FieldInvertState,
    ) {
        let bytes_used = Arc::new(AtomicI64::new(0));
        let mut field =
            TermVectorsConsumerPerField::new(vector_field_info("body", 0), Arc::clone(&bytes_used));
        field.do_vectors = true;
        field.do_vector_positions = positions;
        field.do_vector_offsets = offsets;
        field.do_vector_payloads = payloads;
        let streams = ByteBlockPool::new(Arc::clone(&bytes_used));
        let term_pool = ByteBlockPool::new(Arc::clone(&bytes_used));
        let state = FieldInvertState::new(
            10,
            "body".to_string(),
            IndexOptions::DOCS_AND_FREQS_AND_POSITIONS,
        );
        (field, streams, term_pool, state)
    }

    #[test]
    fn offsets_are_written_as_the_gap_from_the_previous_end_offset() {
        // The postings writer stores the previous *start* offset; the
        // term-vectors writer stores the previous *end* offset, and
        // `TermVectorsWriter.addProx` decodes it that way. Writing the postings
        // convention here would silently shift every offset but the first.
        let (mut field, mut streams, mut term_pool, mut state) = buffered_field(false, true, false);
        let text_start = term_pool.add_bytes_ref(b"alpha").expect("intern");
        field
            .add(&mut streams, &state, text_start, &token(0, 5))
            .expect("first occurrence");
        state.set_position(1);
        field
            .add(&mut streams, &state, text_start, &token(20, 25))
            .expect("second occurrence");

        let mut reader = ByteSliceReader::new();
        field.base.init_reader(&mut reader, 0, OFFSETS_STREAM);
        let mut input = PooledSliceReader::new(reader, &streams);
        assert_eq!(input.read_v_int().expect("start gap"), 0);
        assert_eq!(input.read_v_int().expect("length"), 5);
        assert_eq!(
            input.read_v_int().expect("start gap"),
            15,
            "20 - 5, the previous end offset"
        );
        assert_eq!(input.read_v_int().expect("length"), 5);
        assert!(input.eof());
    }

    #[test]
    fn positions_are_written_as_a_shifted_delta_with_a_payload_bit() {
        let (mut field, mut streams, mut term_pool, mut state) = buffered_field(true, false, true);
        let text_start = term_pool.add_bytes_ref(b"alpha").expect("intern");
        let payload = InvertedToken {
            payload: Some(b"pay"),
            ..token(0, 5)
        };
        field
            .add(&mut streams, &state, text_start, &payload)
            .expect("first occurrence");
        state.set_position(7);
        field
            .add(&mut streams, &state, text_start, &token(0, 5))
            .expect("second occurrence");

        let mut reader = ByteSliceReader::new();
        field.base.init_reader(&mut reader, 0, POSITIONS_STREAM);
        let mut input = PooledSliceReader::new(reader, &streams);
        assert_eq!(input.read_v_int().expect("code"), 1, "(0 << 1) | 1");
        assert_eq!(input.read_v_int().expect("payload length"), 3);
        let mut bytes = [0u8; 3];
        input.read_bytes(&mut bytes, 0, 3).expect("payload bytes");
        assert_eq!(&bytes, b"pay");
        assert_eq!(input.read_v_int().expect("code"), 14, "(7 - 0) << 1");
        assert!(input.eof());
        assert!(field.has_payloads);
    }

    #[test]
    fn an_empty_payload_does_not_set_the_payload_bit() {
        let (mut field, mut streams, mut term_pool, state) = buffered_field(true, false, true);
        let text_start = term_pool.add_bytes_ref(b"alpha").expect("intern");
        let empty = InvertedToken {
            payload: Some(b""),
            ..token(0, 5)
        };
        field
            .add(&mut streams, &state, text_start, &empty)
            .expect("occurrence");
        let mut reader = ByteSliceReader::new();
        field.base.init_reader(&mut reader, 0, POSITIONS_STREAM);
        let mut input = PooledSliceReader::new(reader, &streams);
        assert_eq!(input.read_v_int().expect("code"), 0);
        assert!(input.eof());
        assert!(!field.has_payloads);
    }

    #[test]
    fn a_position_delta_that_overflows_the_shift_round_trips() {
        // `IndexWriter.MAX_POSITION` allows a delta whose `delta << 1`
        // overflows an `i32`; Java wraps and recovers it with `>>>`.
        let (mut field, mut streams, mut term_pool, mut state) = buffered_field(true, false, false);
        let text_start = term_pool.add_bytes_ref(b"alpha").expect("intern");
        let far = i32::MAX - 128;
        field
            .add(&mut streams, &state, text_start, &token(0, 5))
            .expect("first occurrence");
        state.set_position(far);
        field
            .add(&mut streams, &state, text_start, &token(0, 5))
            .expect("second occurrence");

        let mut reader = ByteSliceReader::new();
        field.base.init_reader(&mut reader, 0, POSITIONS_STREAM);
        let mut input = PooledSliceReader::new(reader, &streams);
        assert_eq!(input.read_v_int().expect("code"), 0);
        let code = input.read_v_int().expect("code");
        assert_eq!(
            ((code as u32) >> 1) as i32,
            far,
            "the unsigned shift must recover the delta"
        );
    }

    #[test]
    fn repeated_occurrences_accumulate_the_frequency() {
        let (mut field, mut streams, mut term_pool, mut state) = buffered_field(true, false, false);
        let text_start = term_pool.add_bytes_ref(b"alpha").expect("intern");
        for position in 0..5 {
            state.set_position(position);
            field
                .add(&mut streams, &state, text_start, &token(0, 5))
                .expect("occurrence");
        }
        assert_eq!(field.num_terms(), 1);
        assert_eq!(field.base.posting(0).term_freq, 5);
    }

    #[test]
    fn a_custom_term_frequency_is_rejected_alongside_positions_or_offsets() {
        for (positions, offsets, expected) in [(true, false, "positions"), (false, true, "offsets")]
        {
            let (mut field, mut streams, mut term_pool, state) =
                buffered_field(positions, offsets, false);
            let text_start = term_pool.add_bytes_ref(b"alpha").expect("intern");
            let custom = InvertedToken {
                term_freq: 4,
                has_term_freq_attribute: true,
                ..token(0, 5)
            };
            let error = field
                .add(&mut streams, &state, text_start, &custom)
                .expect_err("a custom frequency has no positions to go with");
            assert!(
                matches!(&error, LuceneError::IllegalArgument(message)
                    if message.contains(&format!("cannot index term vector {expected}"))),
                "{error:?}"
            );
        }
    }

    #[test]
    fn a_custom_term_frequency_is_accepted_without_positions_and_offsets() {
        let (mut field, mut streams, mut term_pool, state) = buffered_field(false, false, false);
        let text_start = term_pool.add_bytes_ref(b"alpha").expect("intern");
        let custom = InvertedToken {
            term_freq: 4,
            has_term_freq_attribute: true,
            ..token(0, 5)
        };
        field
            .add(&mut streams, &state, text_start, &custom)
            .expect("a bare frequency is fine");
        assert_eq!(field.base.posting(0).term_freq, 4);
    }

    #[test]
    fn a_field_is_only_pending_when_it_has_both_vectors_and_terms() {
        let (mut field, mut streams, mut term_pool, state) = buffered_field(false, false, false);
        assert!(!field.finish(), "no term was buffered yet");
        let text_start = term_pool.add_bytes_ref(b"alpha").expect("intern");
        field
            .add(&mut streams, &state, text_start, &token(0, 5))
            .expect("occurrence");
        assert!(field.finish());
        field.do_vectors = false;
        assert!(!field.finish(), "the field did not ask for vectors");
    }
}
