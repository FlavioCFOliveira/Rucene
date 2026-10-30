//! Term-vectors format base traits.
//!
//! Equivalent to `org.apache.lucene.codecs.TermVectorsFormat`,
//! `TermVectorsReader` and `TermVectorsWriter`.

use std::fmt;

use crate::error::{LuceneError, Result};
use crate::index::leaf_reader::TermVectors;
use crate::store::{DataInput, Directory, IOContext};
use crate::util::{Accountable, BytesRef};

use std::sync::Arc;

use super::postings::MergeState;
use super::stub::{FieldInfo, FieldInfos, Fields, SegmentInfo};
use crate::index::merge::{DocIDMerger, DocIDMergerSub, DocMap};
use crate::index::postings_enum::{POSTINGS_ENUM_OFFSETS, POSTINGS_ENUM_PAYLOADS};
use crate::search::doc_id_set_iterator::NO_MORE_DOCS;

/// Controls the format of term vectors.
///
/// Lucene Core equivalent: `org.apache.lucene.codecs.TermVectorsFormat`.
pub trait TermVectorsFormat: Send + Sync + fmt::Debug {
    /// Returns this format's SPI name.
    fn name(&self) -> &str;

    /// Returns a [`TermVectorsReader`] to read term vectors.
    fn vectors_reader(
        &self,
        directory: &dyn Directory,
        segment_info: &SegmentInfo,
        field_infos: &FieldInfos,
        context: &dyn IOContext,
    ) -> Result<Box<dyn TermVectorsReader>>;

    /// Returns a [`TermVectorsWriter`] to write term vectors.
    ///
    /// The directory is shared, not borrowed: as Java's writer keeps its
    /// `Directory` reference, the writer may create, read back and delete
    /// files after this call returns — the fields index streams to temporary
    /// files it only turns into the `.fdx`/`.tvx` at `finish`
    /// (`FieldsIndexWriter.java:83-87, 105-181`).
    fn vectors_writer(
        &self,
        directory: &std::sync::Arc<dyn Directory>,
        segment_info: &SegmentInfo,
        context: &dyn IOContext,
    ) -> Result<Box<dyn TermVectorsWriter>>;
}

/// Codec API for reading term vectors.
///
/// Lucene Core equivalent: `org.apache.lucene.codecs.TermVectorsReader`, which
/// extends `TermVectors` (`TermVectorsReader.java:28`): `get` and `prefetch`
/// are inherited from [`TermVectors`], so a codec reader is itself the
/// `TermVectors` a `CodecReader` hands out.
pub trait TermVectorsReader: TermVectors {
    /// Checks consistency of this reader.
    fn check_integrity(&self) -> Result<()>;

    /// Creates a clone that one caller at a time may use to read term vectors.
    fn clone_reader(&self) -> Box<dyn TermVectorsReader>;

    /// Returns an instance optimized for merging.
    ///
    /// The default implementation returns a clone of `self`.
    fn get_merge_instance(&self) -> Box<dyn TermVectorsReader> {
        self.clone_reader()
    }

    /// Returns this reader as [`std::any::Any`], for the one caller that has to
    /// recognise a concrete reader type: the compressing term-vectors writer,
    /// which can only copy raw chunks out of a reader of its own format.
    ///
    /// Divergence: Lucene writes `mergeState.termVectorsReaders[readerIndex]
    /// instanceof Lucene90CompressingTermVectorsReader`
    /// (`Lucene90CompressingTermVectorsWriter.java:956-971`). Rust has no
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

/// Codec API for writing term vectors.
///
/// Lucene Core equivalent: `org.apache.lucene.codecs.TermVectorsWriter`.
pub trait TermVectorsWriter: Send + Sync + fmt::Debug + Accountable {
    /// Called before writing the term vectors of a document.
    fn start_document(&mut self, num_vector_fields: i32) -> Result<()>;

    /// Called after a document and all its fields have been added.
    fn finish_document(&mut self) -> Result<()> {
        Ok(())
    }

    /// Called before writing the terms of a field.
    fn start_field(
        &mut self,
        info: &FieldInfo,
        num_terms: i32,
        positions: bool,
        offsets: bool,
        payloads: bool,
    ) -> Result<()>;

    /// Called after a field and all its terms have been added.
    fn finish_field(&mut self) -> Result<()> {
        Ok(())
    }

    /// Adds a term and its term frequency.
    fn start_term(&mut self, term: &BytesRef, freq: i32) -> Result<()>;

    /// Called after a term and all its positions have been added.
    fn finish_term(&mut self) -> Result<()> {
        Ok(())
    }

    /// Adds a term position and offsets.
    fn add_position(
        &mut self,
        position: i32,
        start_offset: i32,
        end_offset: i32,
        payload: Option<&BytesRef>,
    ) -> Result<()>;

    /// Consumes `num_prox` positions and offsets straight from the indexer's
    /// buffers.
    ///
    /// Lucene Core equivalent: `TermVectorsWriter.addProx(int, DataInput,
    /// DataInput)`. This is the expert entry point
    /// `TermVectorsConsumerPerField.finishDocument` uses: rather than decoding
    /// its RAM buffers itself, the indexer hands the two byte streams over so
    /// that a format able to write all positions and then all offsets can do so
    /// without an intermediate representation. The default implementation
    /// decodes them and calls [`Self::add_position`], exactly as Java's does.
    ///
    /// `positions` carries, per occurrence, a variable-length
    /// `positionDelta << 1` with the low bit set when a payload follows, then
    /// the payload length and its bytes. `offsets` carries, per occurrence, the
    /// gap from the previous *end* offset to this start offset, then the token
    /// length. Either may be absent, in which case the corresponding values are
    /// reported as `-1`, which is what Lucene passes on.
    ///
    /// # Errors
    ///
    /// Propagates any error raised while reading the two streams or while
    /// writing the positions.
    fn add_prox(
        &mut self,
        num_prox: i32,
        mut positions: Option<&mut dyn DataInput>,
        mut offsets: Option<&mut dyn DataInput>,
    ) -> Result<()> {
        let mut position: i32 = 0;
        let mut last_offset: i32 = 0;
        // Java reuses one `BytesRefBuilder` across the whole term; so does this,
        // which is why the buffer lives outside the loop.
        let mut payload = BytesRef::default();

        for _ in 0..num_prox {
            let mut has_payload = false;
            match positions.as_deref_mut() {
                None => position = -1,
                Some(input) => {
                    let code = input.read_v_int()?;
                    // Java shifts an `int` right with `>>>`, and the matching
                    // `pos << 1` in `TermVectorsConsumerPerField.writeProx`
                    // overflows silently for a position delta above
                    // `Integer.MAX_VALUE / 2` — which `IndexWriter.MAX_POSITION`
                    // still allows. Going through `u32` reproduces both halves
                    // of that round trip instead of aborting a debug build.
                    position = position.wrapping_add(((code as u32) >> 1) as i32);
                    if code & 1 != 0 {
                        let length = input.read_v_int()?;
                        let length = usize::try_from(length).map_err(|_| {
                            LuceneError::CorruptIndex(format!(
                                "negative term-vector payload length: {length}"
                            ))
                        })?;
                        payload.bytes.clear();
                        payload.bytes.resize(length, 0);
                        input.read_bytes(&mut payload.bytes, 0, length)?;
                        payload.offset = 0;
                        payload.length = length;
                        has_payload = true;
                    }
                }
            }

            let (start_offset, end_offset) = match offsets.as_deref_mut() {
                None => (-1, -1),
                Some(input) => {
                    let start = last_offset.wrapping_add(input.read_v_int()?);
                    let end = start.wrapping_add(input.read_v_int()?);
                    last_offset = end;
                    (start, end)
                }
            };

            let this_payload = if has_payload { Some(&payload) } else { None };
            self.add_position(position, start_offset, end_offset, this_payload)?;
        }
        Ok(())
    }

    /// Called before close, passing the number of documents written.
    fn finish(&mut self, num_docs: i32) -> Result<()>;

    /// Writes one document's term vectors, field by field and term by term.
    ///
    /// Equivalent to `TermVectorsWriter.addAllDocVectors(Fields, MergeState)`.
    /// A document with no vectors is written as an empty one, so that document
    /// numbers stay aligned with the merged segment.
    fn add_all_doc_vectors(
        &mut self,
        vectors: Option<&dyn Fields>,
        merge_field_infos: &FieldInfos,
    ) -> Result<()> {
        let Some(vectors) = vectors else {
            self.start_document(0)?;
            self.finish_document()?;
            return Ok(());
        };

        let field_names: Vec<String> = vectors.iterator().collect();
        let mut num_fields = vectors.size();
        if num_fields == -1 {
            // Count them: `Fields.size()` is not a mandatory statistic.
            num_fields = field_names.len() as i32;
        }
        self.start_document(num_fields)?;

        for field_name in &field_names {
            let field_info = merge_field_infos.field_info(field_name).ok_or_else(|| {
                crate::error::LuceneError::illegal_state(format!(
                    "field \"{field_name}\" carries term vectors but is absent from the merged \
                     field infos"
                ))
            })?;
            let Some(terms) = vectors.terms(field_name)? else {
                // The field enumeration should not lie, but Lucene tolerates it.
                continue;
            };

            let has_positions = terms.has_positions();
            let has_offsets = terms.has_offsets();
            let has_payloads = terms.has_payloads();
            debug_assert!(!has_payloads || has_positions);

            let mut num_terms = terms.size()? as i32;
            if num_terms == -1 {
                // Count them: `Terms.size()` is not a mandatory statistic.
                num_terms = 0;
                let mut counting = terms.iterator()?;
                while counting.next()?.is_some() {
                    num_terms += 1;
                }
            }

            self.start_field(
                field_info,
                num_terms,
                has_positions,
                has_offsets,
                has_payloads,
            )?;
            let mut terms_enum = terms.iterator()?;
            while let Some(term) = terms_enum.next()? {
                let freq = terms_enum.total_term_freq()? as i32;
                self.start_term(&term, freq)?;

                if has_positions || has_offsets {
                    let mut postings = terms_enum
                        .postings(None, POSTINGS_ENUM_OFFSETS | POSTINGS_ENUM_PAYLOADS)?;
                    let doc_id = postings.next_doc()?;
                    debug_assert_ne!(doc_id, NO_MORE_DOCS);
                    for _ in 0..freq {
                        let pos = postings.next_position()?;
                        let start_offset = postings.start_offset();
                        let end_offset = postings.end_offset();
                        let payload = postings
                            .get_payload()?
                            .map(|bytes| BytesRef::new(bytes.to_vec()));
                        debug_assert!(!has_positions || pos >= 0);
                        self.add_position(pos, start_offset, end_offset, payload.as_ref())?;
                    }
                }
                self.finish_term()?;
            }
            self.finish_field()?;
        }
        self.finish_document()
    }

    /// Copies every document's term vectors from the segments being merged.
    ///
    /// Equivalent to `TermVectorsWriter.merge(MergeState)`.
    fn merge(&mut self, merge_state: &MergeState) -> Result<i32> {
        let mut readers: Vec<Option<Box<dyn TermVectorsReader>>> = Vec::new();
        let mut subs: Vec<TermVectorsMergeSub> = Vec::new();
        for (i, reader) in merge_state.term_vectors_readers.iter().enumerate() {
            if let Some(reader) = reader.as_ref() {
                // `TermVectorsWriter.java:203`.
                merge_state.check_aborted()?;
                reader.check_integrity()?;
                readers.push(Some(reader.clone_reader()));
            } else {
                readers.push(None);
            }
            subs.push(TermVectorsMergeSub {
                doc_map: Arc::clone(&merge_state.doc_maps[i]),
                reader_index: i,
                max_doc: merge_state.max_docs[i],
                doc_id: -1,
                mapped_doc_id: -1,
            });
        }

        let mut merger = DocIDMerger::new(subs, merge_state.needs_index_sort)?;
        let mut doc_count = 0i32;
        while let Some((reader_index, doc_id)) =
            merger.next_sub()?.map(|sub| (sub.reader_index, sub.doc_id))
        {
            let vectors = match readers[reader_index].as_ref() {
                None => None,
                Some(reader) => reader.get(doc_id)?,
            };
            self.add_all_doc_vectors(vectors.as_deref(), &merge_state.merge_field_infos)?;
            doc_count += 1;
        }
        self.finish(doc_count)?;
        Ok(doc_count)
    }

    /// Closes the writer and releases any resources.
    fn close(&mut self) -> Result<()>;
}

/// A minimal no-op term-vectors format.
#[derive(Debug, Copy, Clone, Default)]
pub struct EmptyTermVectorsFormat;

impl TermVectorsFormat for EmptyTermVectorsFormat {
    fn name(&self) -> &str {
        "EmptyTermVectors"
    }

    fn vectors_reader(
        &self,
        _directory: &dyn Directory,
        _segment_info: &SegmentInfo,
        _field_infos: &FieldInfos,
        _context: &dyn IOContext,
    ) -> Result<Box<dyn TermVectorsReader>> {
        Ok(Box::new(EmptyTermVectorsReader))
    }

    fn vectors_writer(
        &self,
        _directory: &std::sync::Arc<dyn Directory>,
        _segment_info: &SegmentInfo,
        _context: &dyn IOContext,
    ) -> Result<Box<dyn TermVectorsWriter>> {
        Ok(Box::new(EmptyTermVectorsWriter))
    }
}

/// A minimal no-op term-vectors reader.
#[derive(Debug, Copy, Clone, Default)]
pub struct EmptyTermVectorsReader;

impl TermVectorsReader for EmptyTermVectorsReader {
    fn check_integrity(&self) -> Result<()> {
        Ok(())
    }

    fn clone_reader(&self) -> Box<dyn TermVectorsReader> {
        Box::new(*self)
    }
}

impl TermVectors for EmptyTermVectorsReader {
    fn get(&self, _doc: i32) -> Result<Option<Box<dyn Fields>>> {
        Ok(None)
    }
}

/// A minimal no-op term-vectors writer.
#[derive(Debug, Copy, Clone, Default)]
pub struct EmptyTermVectorsWriter;

/// Holds nothing, so it accounts for nothing.
///
/// Java's `TermVectorsWriter` is `Accountable`, so every writer answers
/// `ramBytesUsed()`.
impl Accountable for EmptyTermVectorsWriter {
    fn ram_bytes_used(&self) -> i64 {
        0
    }
}

impl TermVectorsWriter for EmptyTermVectorsWriter {
    fn start_document(&mut self, _num_vector_fields: i32) -> Result<()> {
        Ok(())
    }

    fn start_field(
        &mut self,
        _info: &FieldInfo,
        _num_terms: i32,
        _positions: bool,
        _offsets: bool,
        _payloads: bool,
    ) -> Result<()> {
        Ok(())
    }

    fn start_term(&mut self, _term: &BytesRef, _freq: i32) -> Result<()> {
        Ok(())
    }

    fn add_position(
        &mut self,
        _position: i32,
        _start_offset: i32,
        _end_offset: i32,
        _payload: Option<&BytesRef>,
    ) -> Result<()> {
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

    // -- add_prox ------------------------------------------------------------

    /// Records what [`TermVectorsWriter::add_prox`] forwards to
    /// [`TermVectorsWriter::add_position`].
    #[derive(Debug, Default)]
    struct RecordingWriter {
        positions: Vec<(i32, i32, i32, Option<Vec<u8>>)>,
    }

    impl Accountable for RecordingWriter {
        fn ram_bytes_used(&self) -> i64 {
            0
        }
    }

    impl TermVectorsWriter for RecordingWriter {
        fn start_document(&mut self, _num_vector_fields: i32) -> Result<()> {
            Ok(())
        }

        fn start_field(
            &mut self,
            _info: &FieldInfo,
            _num_terms: i32,
            _positions: bool,
            _offsets: bool,
            _payloads: bool,
        ) -> Result<()> {
            Ok(())
        }

        fn start_term(&mut self, _term: &BytesRef, _freq: i32) -> Result<()> {
            Ok(())
        }

        fn add_position(
            &mut self,
            position: i32,
            start_offset: i32,
            end_offset: i32,
            payload: Option<&BytesRef>,
        ) -> Result<()> {
            self.positions.push((
                position,
                start_offset,
                end_offset,
                payload.map(|value| value.slice().to_vec()),
            ));
            Ok(())
        }

        fn finish(&mut self, _num_docs: i32) -> Result<()> {
            Ok(())
        }

        fn close(&mut self) -> Result<()> {
            Ok(())
        }
    }

    /// Builds the position stream `TermVectorsConsumerPerField.writeProx`
    /// writes: `delta << 1`, with the low bit set when a payload follows.
    fn position_stream(entries: &[(i32, Option<&[u8]>)]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut last = 0i32;
        for (position, payload) in entries {
            let code = ((*position - last) as u32) << 1;
            match payload {
                Some(bytes) if !bytes.is_empty() => {
                    write_v_int(&mut out, (code | 1) as i32);
                    write_v_int(&mut out, bytes.len() as i32);
                    out.extend_from_slice(bytes);
                }
                _ => write_v_int(&mut out, code as i32),
            }
            last = *position;
        }
        out
    }

    /// Builds the offset stream: the gap from the previous *end* offset, then
    /// the token length.
    fn offset_stream(entries: &[(i32, i32)]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut last_end = 0i32;
        for (start, end) in entries {
            write_v_int(&mut out, start - last_end);
            write_v_int(&mut out, end - start);
            last_end = *end;
        }
        out
    }

    fn write_v_int(out: &mut Vec<u8>, value: i32) {
        let mut remaining = value as u32;
        while remaining & !0x7F != 0 {
            out.push(((remaining & 0x7F) | 0x80) as u8);
            remaining >>= 7;
        }
        out.push(remaining as u8);
    }

    #[test]
    fn add_prox_decodes_positions_offsets_and_payloads() {
        let positions = position_stream(&[(0, Some(b"one")), (3, None), (9, Some(b"two"))]);
        let offsets = offset_stream(&[(0, 5), (10, 14), (20, 30)]);
        let mut writer = RecordingWriter::default();
        let mut positions = crate::store::ByteArrayDataInput::new(&positions);
        let mut offsets = crate::store::ByteArrayDataInput::new(&offsets);
        writer
            .add_prox(3, Some(&mut positions), Some(&mut offsets))
            .expect("add prox");
        assert_eq!(
            writer.positions,
            vec![
                (0, 0, 5, Some(b"one".to_vec())),
                (3, 10, 14, None),
                (9, 20, 30, Some(b"two".to_vec())),
            ]
        );
    }

    #[test]
    fn add_prox_reports_minus_one_for_the_stream_it_was_not_given() {
        let mut writer = RecordingWriter::default();
        let positions_src = position_stream(&[(0, None), (4, None)]);
        let mut positions = crate::store::ByteArrayDataInput::new(&positions_src);
        writer
            .add_prox(2, Some(&mut positions), None)
            .expect("positions only");
        assert_eq!(
            writer.positions,
            vec![(0, -1, -1, None), (4, -1, -1, None)],
            "an absent offsets stream reports -1, exactly as Java does"
        );

        let mut writer = RecordingWriter::default();
        let offsets_src = offset_stream(&[(0, 5), (7, 11)]);
        let mut offsets = crate::store::ByteArrayDataInput::new(&offsets_src);
        writer
            .add_prox(2, None, Some(&mut offsets))
            .expect("offsets only");
        assert_eq!(
            writer.positions,
            vec![(-1, 0, 5, None), (-1, 7, 11, None)],
            "an absent positions stream reports -1"
        );
    }

    #[test]
    fn add_prox_recovers_a_delta_that_overflowed_the_shift() {
        // `IndexWriter.MAX_POSITION` allows a delta whose `delta << 1`
        // overflows; the encoder wraps and the decoder must shift back
        // unsigned.
        let far = i32::MAX - 128;
        let mut writer = RecordingWriter::default();
        let positions_src = position_stream(&[(far, None)]);
        let mut positions = crate::store::ByteArrayDataInput::new(&positions_src);
        writer
            .add_prox(1, Some(&mut positions), None)
            .expect("add prox");
        assert_eq!(writer.positions, vec![(far, -1, -1, None)]);
    }

    #[test]
    fn add_prox_rejects_a_negative_payload_length() {
        // A corrupt buffer must produce an error, never a huge allocation.
        let mut bytes = Vec::new();
        write_v_int(&mut bytes, 1); // (0 << 1) | 1: a payload follows
        write_v_int(&mut bytes, -1); // its length
        let mut writer = RecordingWriter::default();
        let mut positions = crate::store::ByteArrayDataInput::new(&bytes);
        let error = writer
            .add_prox(1, Some(&mut positions), None)
            .expect_err("a negative length is not a length");
        assert!(matches!(error, LuceneError::CorruptIndex(_)), "{error:?}");
    }

    #[test]
    fn add_prox_writes_nothing_for_zero_occurrences() {
        let mut writer = RecordingWriter::default();
        writer.add_prox(0, None, None).expect("add prox");
        assert!(writer.positions.is_empty());
    }

    #[test]
    fn empty_term_vectors_format_name() {
        let format = EmptyTermVectorsFormat;
        assert_eq!(format.name(), "EmptyTermVectors");
    }

    #[test]
    fn empty_term_vectors_reader_methods() {
        let reader = EmptyTermVectorsReader;
        reader.check_integrity().unwrap();
        assert!(reader.get(0).unwrap().is_none());
        let _clone = reader.clone_reader();
    }

    #[test]
    fn empty_term_vectors_writer_methods() {
        let mut writer = EmptyTermVectorsWriter;
        writer.start_document(1).unwrap();
        let info = FieldInfo::default();
        writer.start_field(&info, 1, true, true, true).unwrap();
        let term = BytesRef::new(b"term".to_vec());
        writer.start_term(&term, 1).unwrap();
        writer.add_position(0, 0, 4, Some(&term)).unwrap();
        writer.finish_term().unwrap();
        writer.finish_field().unwrap();
        writer.finish_document().unwrap();
        writer.finish(1).unwrap();
        writer.close().unwrap();
    }
}

/// One sub-reader's term vectors, as the merger walks them.
///
/// Equivalent to `TermVectorsWriter.TermVectorsMergeSub`. The reader lives in
/// the merge's own list rather than in the sub, so it need not be cloned per
/// document.
struct TermVectorsMergeSub {
    doc_map: DocMap,
    reader_index: usize,
    max_doc: i32,
    doc_id: i32,
    mapped_doc_id: i32,
}

impl DocIDMergerSub for TermVectorsMergeSub {
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
