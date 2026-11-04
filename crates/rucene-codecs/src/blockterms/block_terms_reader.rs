/*
 * Licensed to the Apache Software Foundation (ASF) under one or more
 * contributor license agreements.  See the NOTICE file distributed with
 * this work for additional information regarding copyright ownership.
 * The ASF licenses this file to You under the Apache License, Version 2.0
 * (the "License"); you may not use this file except in compliance with
 * the License.  You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

//! Port of `org.apache.lucene.codecs.blockterms.BlockTermsReader`.

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use rucene::codecs::codec_util::{check_index_header, checksum_entire_file, retrieve_checksum};
use rucene::codecs::{BlockTermState, FieldsProducer, PostingsReaderBase, SegmentReadState};
use rucene::error::{LuceneError, Result};
use rucene::index::index_file_names::segment_file_name;
use rucene::index::index_reader::java_object_to_string;
use rucene::index::{
    FieldInfo, Fields, ImpactsEnum, IndexOptions, PostingsEnum, SeekStatus, TermState, Terms,
    TermsEnum,
};
use rucene::store::{ByteArrayDataInput, DataInput, IndexInput};
use rucene::util::attribute::AttributeSource;
use rucene::util::java_collections::java_compare_to;
use rucene::util::BytesRef;

use super::block_terms_writer::{CODEC_NAME, TERMS_EXTENSION, VERSION_CURRENT, VERSION_START};
use super::fixed_gap_terms_index_reader::{field_info_by_number, seek_dir};
use super::terms_index_reader_base::{FieldIndexEnum, TermsIndexReaderBase};

/// What the reader shares with its field readers and enums: Java's `in`,
/// `postingsReader` and `indexReader`, which the inner classes reach
/// through their outer instance.
struct Shared {
    input: Arc<Box<dyn IndexInput>>,
    postings_reader: Arc<dyn PostingsReaderBase>,
    index_reader: Arc<dyn TermsIndexReaderBase>,
}

/// Handles a terms dict, but decouples all details of doc/freqs/positions
/// reading to an instance of [`PostingsReaderBase`]. This class is reusable
/// for codecs that use a different format for docs/freqs/positions (though
/// codecs are also free to make their own terms dict impl).
///
/// This class also interacts with an instance of
/// [`TermsIndexReaderBase`], to abstract away the specific implementation of
/// the terms dict index.
///
/// Port of `org.apache.lucene.codecs.blockterms.BlockTermsReader`. Java's
/// `TreeMap<String, FieldReader>` is a map plus the names in
/// `String.compareTo` order.
///
/// `close()` closes the index reader, `in` and the postings reader, in that
/// order, each when this reader is its last holder; as for
/// `Lucene103BlockTreeTermsReader`, one a live enum still shares is released
/// when that enum is dropped.
pub struct BlockTermsReader {
    shared: Option<Arc<Shared>>,
    fields: HashMap<String, Arc<FieldReaderData>>,
    field_names: Vec<String>,
}

impl BlockTermsReader {
    /// Port of `BlockTermsReader(TermsIndexReaderBase, PostingsReaderBase,
    /// SegmentReadState)`.
    ///
    /// The two readers move into the constructor, so when it fails it also
    /// closes them, which Java's callers (`LuceneFixedGap` and the other
    /// blockterms assemblies) do in their `finally`: the postings reader
    /// first, then the index reader.
    ///
    /// # Errors
    ///
    /// `CorruptIndexException` for a bad header, footer, field count or
    /// field statistics, or a duplicate field; I/O errors.
    pub fn new(
        mut index_reader: Box<dyn TermsIndexReaderBase>,
        mut postings_reader: Box<dyn PostingsReaderBase>,
        state: &SegmentReadState<'_>,
    ) -> Result<Self> {
        let opened = Self::open(postings_reader.as_mut(), state);
        match opened {
            Ok((input, fields, field_names)) => Ok(Self {
                shared: Some(Arc::new(Shared {
                    input: Arc::new(input),
                    postings_reader: Arc::from(postings_reader),
                    index_reader: Arc::from(index_reader),
                })),
                fields,
                field_names,
            }),
            Err(e) => {
                let postings = postings_reader.close();
                let index = index_reader.close();
                postings?;
                index?;
                Err(e)
            }
        }
    }

    #[allow(clippy::type_complexity)]
    fn open(
        postings_reader: &mut dyn PostingsReaderBase,
        state: &SegmentReadState<'_>,
    ) -> Result<(
        Box<dyn IndexInput>,
        HashMap<String, Arc<FieldReaderData>>,
        Vec<String>,
    )> {
        let filename = segment_file_name(
            &state.segment_info.name,
            &state.segment_suffix,
            TERMS_EXTENSION,
        );
        let mut input = state.directory.open_input(&filename, state.context)?;
        match Self::read(postings_reader, state, input.as_mut()) {
            Ok((fields, field_names)) => Ok((input, fields, field_names)),
            Err(e) => {
                input.close()?;
                Err(e)
            }
        }
    }

    #[allow(clippy::type_complexity)]
    fn read(
        postings_reader: &mut dyn PostingsReaderBase,
        state: &SegmentReadState<'_>,
        input: &mut dyn IndexInput,
    ) -> Result<(HashMap<String, Arc<FieldReaderData>>, Vec<String>)> {
        check_index_header(
            input,
            CODEC_NAME,
            VERSION_START,
            VERSION_CURRENT,
            &state.segment_info.id(),
            &state.segment_suffix,
        )?;

        // Have PostingsReader init itself
        postings_reader.init(input, state)?;

        // NOTE: data file is too costly to verify checksum against all the bytes on open,
        // but for now we at least verify proper structure of the checksum footer: which looks
        // for FOOTER_MAGIC + algorithmID. This is cheap and can detect some forms of corruption
        // such as file truncation.
        retrieve_checksum(input)?;

        // Read per-field details
        seek_dir(input)?;

        let num_fields = input.read_v_int()?;
        if num_fields < 0 {
            return Err(LuceneError::corrupt_index(
                format!("invalid number of fields: {num_fields}"),
                input.resource_description(),
            ));
        }
        let max_doc = state.segment_info.max_doc()?;
        let mut fields = HashMap::new();
        for _ in 0..num_fields {
            let field = input.read_v_int()?;
            let num_terms = input.read_v_long()?;
            debug_assert!(num_terms >= 0);
            let terms_start_pointer = input.read_v_long()?;
            let field_info = field_info_by_number(state, field)?;
            let sum_total_term_freq = input.read_v_long()?;
            // when frequencies are omitted, sumDocFreq=totalTermFreq and we only write one value
            let sum_doc_freq = if field_info.get_index_options() == IndexOptions::DOCS {
                sum_total_term_freq
            } else {
                input.read_v_long()?
            };
            let doc_count = input.read_v_int()?;
            if doc_count < 0 || doc_count > max_doc {
                // #docs with field must be <= #docs
                return Err(LuceneError::corrupt_index(
                    format!("invalid docCount: {doc_count} maxDoc: {max_doc}"),
                    input.resource_description(),
                ));
            }
            if sum_doc_freq < i64::from(doc_count) {
                // #postings must be >= #docs with field
                return Err(LuceneError::corrupt_index(
                    format!("invalid sumDocFreq: {sum_doc_freq} docCount: {doc_count}"),
                    input.resource_description(),
                ));
            }
            if sum_total_term_freq < sum_doc_freq {
                // #positions must be >= #postings
                return Err(LuceneError::corrupt_index(
                    format!(
                        "invalid sumTotalTermFreq: {sum_total_term_freq} sumDocFreq: {sum_doc_freq}"
                    ),
                    input.resource_description(),
                ));
            }
            debug_assert!(num_terms > 0);
            let data = FieldReaderData {
                field_info: field_info.clone(),
                num_terms,
                terms_start_pointer,
                sum_total_term_freq,
                sum_doc_freq,
                doc_count,
            };
            if fields
                .insert(field_info.name.clone(), Arc::new(data))
                .is_some()
            {
                return Err(LuceneError::corrupt_index(
                    format!("duplicate fields: {}", field_info.name),
                    input.resource_description(),
                ));
            }
        }
        let mut field_names: Vec<String> = fields.keys().cloned().collect();
        field_names.sort_by(|a, b| java_compare_to(a, b));
        Ok((fields, field_names))
    }

    fn shared(&self) -> Result<&Arc<Shared>> {
        self.shared
            .as_ref()
            .ok_or_else(|| LuceneError::already_closed("this BlockTermsReader is closed"))
    }
}

impl fmt::Display for BlockTermsReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.shared {
            Some(shared) => write!(
                f,
                "BlockTermsReader(index={},delegate={})",
                shared.index_reader, shared.postings_reader
            ),
            None => write!(f, "BlockTermsReader(index=null,delegate=null)"),
        }
    }
}

impl Fields for BlockTermsReader {
    fn iterator(&self) -> Box<dyn Iterator<Item = String> + '_> {
        Box::new(self.field_names.iter().cloned())
    }

    fn terms(&self, field: &str) -> Result<Option<Box<dyn Terms>>> {
        let Some(data) = self.fields.get(field) else {
            return Ok(None);
        };
        Ok(Some(Box::new(FieldReader {
            shared: Arc::clone(self.shared()?),
            data: Arc::clone(data),
        })))
    }

    fn size(&self) -> i32 {
        self.fields.len() as i32
    }
}

impl FieldsProducer for BlockTermsReader {
    fn check_integrity(&self) -> Result<()> {
        let shared = self.shared()?;
        // verify terms
        let mut input = shared.input.clone_input()?;
        checksum_entire_file(input.as_mut())?;
        // verify postings
        shared.postings_reader.check_integrity()
    }

    fn get_merge_instance(&self) -> Result<Box<dyn FieldsProducer>> {
        Ok(Box::new(Self {
            shared: self.shared.clone(),
            fields: self.fields.clone(),
            field_names: self.field_names.clone(),
        }))
    }

    fn close(&mut self) -> Result<()> {
        self.fields.clear();
        self.field_names.clear();
        let Some(mut shared) = self.shared.take() else {
            return Ok(());
        };
        let Some(shared) = Arc::get_mut(&mut shared) else {
            return Ok(());
        };
        // try { try { IOUtils.close(indexReader) } finally { IOUtils.close(in) } }
        // finally { IOUtils.close(postingsReader) }
        let index = Arc::get_mut(&mut shared.index_reader).map_or(Ok(()), |r| r.close());
        let input = Arc::get_mut(&mut shared.input).map_or(Ok(()), |i| i.close());
        let postings = Arc::get_mut(&mut shared.postings_reader).map_or(Ok(()), |r| r.close());
        // A `finally` that throws replaces the exception of its `try`.
        postings.and(input).and(index)
    }
}

/// The fields of `FieldReader`.
struct FieldReaderData {
    field_info: FieldInfo,
    num_terms: i64,
    terms_start_pointer: i64,
    sum_total_term_freq: i64,
    sum_doc_freq: i64,
    doc_count: i32,
}

/// Port of the inner class `BlockTermsReader.FieldReader`.
struct FieldReader {
    shared: Arc<Shared>,
    data: Arc<FieldReaderData>,
}

impl Terms for FieldReader {
    fn iterator(&self) -> Result<Box<dyn TermsEnum>> {
        Ok(Box::new(SegmentTermsEnum::new(
            Arc::clone(&self.shared),
            Arc::clone(&self.data),
        )?))
    }

    fn has_freqs(&self) -> bool {
        self.data
            .field_info
            .get_index_options()
            .subsumes(IndexOptions::DOCS_AND_FREQS)
    }

    fn has_offsets(&self) -> bool {
        self.data
            .field_info
            .get_index_options()
            .subsumes(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS)
    }

    fn has_positions(&self) -> bool {
        self.data
            .field_info
            .get_index_options()
            .subsumes(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS)
    }

    fn has_payloads(&self) -> bool {
        self.data.field_info.has_payloads()
    }

    fn size(&self) -> Result<i64> {
        Ok(self.data.num_terms)
    }

    fn sum_total_term_freq(&self) -> Result<i64> {
        Ok(self.data.sum_total_term_freq)
    }

    fn sum_doc_freq(&self) -> Result<i64> {
        Ok(self.data.sum_doc_freq)
    }

    fn doc_count(&self) -> Result<i32> {
        Ok(self.data.doc_count)
    }
}

/// Java's `BytesRefBuilder term`: `setLength` keeps the bytes of the
/// backing array, and `grow` zero-fills the new ones.
#[derive(Default)]
struct TermBuilder {
    bytes: Vec<u8>,
    length: usize,
}

impl TermBuilder {
    fn set_length(&mut self, length: usize) {
        if self.bytes.len() < length {
            self.bytes.resize(length, 0);
        }
        self.length = length;
    }

    fn copy_bytes(&mut self, src: &[u8]) {
        self.set_length(src.len());
        self.bytes[..src.len()].copy_from_slice(src);
    }

    fn get(&self) -> &[u8] {
        &self.bytes[..self.length]
    }

    /// `byteAt(i)`: Java reads the backing array, which `ArrayUtil.grow`
    /// oversizes; past it Java throws `ArrayIndexOutOfBoundsException`. The
    /// port's array is exactly as long as the longest term it held, so an
    /// index past it (which only a corrupt block prefix can produce) raises
    /// the same exception.
    fn byte_at(&self, i: usize) -> Result<u8> {
        self.bytes.get(i).copied().ok_or_else(|| {
            LuceneError::unchecked(
                "java.lang.ArrayIndexOutOfBoundsException",
                format!("Index {i} out of bounds for length {}", self.bytes.len()),
            )
        })
    }
}

/// A `ByteArrayDataInput` over a buffer the enum owns: the buffer, the
/// window's limit and the read position.
#[derive(Default)]
struct OwnedArrayInput {
    bytes: Vec<u8>,
    limit: usize,
    pos: usize,
}

impl OwnedArrayInput {
    /// `in.readBytes(buffer, 0, len); reader.reset(buffer, 0, len)`.
    fn fill(&mut self, input: &mut dyn IndexInput, len: usize) -> Result<()> {
        if self.bytes.len() < len {
            self.bytes.resize(len, 0);
        }
        input.read_bytes(&mut self.bytes, 0, len)?;
        self.limit = len;
        self.pos = 0;
        Ok(())
    }

    fn with<R>(&mut self, f: impl FnOnce(&mut ByteArrayDataInput<'_>) -> Result<R>) -> Result<R> {
        let mut input = ByteArrayDataInput::with_range(&self.bytes, 0, self.limit);
        input.set_position(self.pos);
        let result = f(&mut input);
        self.pos = input.position();
        result
    }

    fn read_v_int(&mut self) -> Result<i32> {
        self.with(|i| i.read_v_int())
    }

    fn read_v_long(&mut self) -> Result<i64> {
        self.with(|i| i.read_v_long())
    }

    fn skip_bytes(&mut self, n: usize) -> Result<()> {
        self.with(|i| i.skip_bytes(n as i64))
    }

    /// `readBytes(term.bytes(), offset, len)`.
    fn read_into(&mut self, term: &mut TermBuilder, offset: usize, len: usize) -> Result<()> {
        self.with(|i| i.read_bytes(&mut term.bytes, offset, len))
    }
}

/// `target.bytes[target.offset + i] & 0xFF`, with the
/// `ArrayIndexOutOfBoundsException` of an index past the backing array.
fn target_byte(target: &BytesRef, i: usize) -> Result<i32> {
    target
        .bytes
        .get(target.offset + i)
        .map(|&b| i32::from(b))
        .ok_or_else(|| {
            LuceneError::unchecked(
                "java.lang.ArrayIndexOutOfBoundsException",
                format!(
                    "Index {} out of bounds for length {}",
                    target.offset + i,
                    target.bytes.len()
                ),
            )
        })
}

/// Port of the private inner class `FieldReader.SegmentTermsEnum`: iterates
/// through terms in this field.
struct SegmentTermsEnum {
    shared: Arc<Shared>,
    field: Arc<FieldReaderData>,
    attributes: AttributeSource,
    input: Box<dyn IndexInput>,
    state: BlockTermState,
    do_ord: bool,
    index_enum: Option<Box<dyn FieldIndexEnum>>,
    term: TermBuilder,

    /// This is true if indexEnum is "still" seek'd to the index term for
    /// the current term. We set it to true on seeking, and then it remains
    /// valid until next() is called enough times to load another terms
    /// block.
    index_is_current: bool,

    /// True if we've already called .next() on the indexEnum, to "bracket"
    /// the current block of terms.
    did_index_next: bool,

    /// Next index term, bracketing the current block of terms; this is only
    /// valid if didIndexNext is true. Java aliases the index enum's term,
    /// which only moves when `didIndexNext` is reset, so a copy is the same.
    next_index_term: Option<BytesRef>,

    /// True after seekExact(TermState), do defer seeking. If the app then
    /// calls next() (which is not "typical"), then we'll do the real seek.
    seek_pending: bool,

    term_suffixes: OwnedArrayInput,

    /// Common prefix used for all terms in this block.
    term_block_prefix: usize,

    /// How many terms in current block.
    block_term_count: i32,

    freq_reader: OwnedArrayInput,
    meta_data_upto: i32,
    bytes_reader: OwnedArrayInput,
}

impl fmt::Display for SegmentTermsEnum {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&java_object_to_string(
            "org.apache.lucene.codecs.blockterms.BlockTermsReader$FieldReader$SegmentTermsEnum",
            self,
        ))
    }
}

/// The `IllegalStateException` of a seek without a terms index.
fn no_index() -> LuceneError {
    LuceneError::illegal_state("terms index was not loaded")
}

impl SegmentTermsEnum {
    fn new(shared: Arc<Shared>, field: Arc<FieldReaderData>) -> Result<Self> {
        let mut input = shared.input.clone_input()?;
        input.seek(field.terms_start_pointer)?;
        let index_enum = shared.index_reader.get_field_enum(&field.field_info);
        let do_ord = shared.index_reader.supports_ord();
        let mut state = shared.postings_reader.new_term_state()?;
        state.total_term_freq = -1;
        state.ord = -1;
        Ok(Self {
            shared,
            field,
            attributes: AttributeSource::new(),
            input,
            state,
            do_ord,
            index_enum,
            term: TermBuilder::default(),
            index_is_current: false,
            did_index_next: false,
            next_index_term: None,
            seek_pending: false,
            term_suffixes: OwnedArrayInput::default(),
            term_block_prefix: 0,
            block_term_count: 0,
            freq_reader: OwnedArrayInput::default(),
            meta_data_upto: 0,
            bytes_reader: OwnedArrayInput::default(),
        })
    }

    /// Reads the suffix of the current term into `term`, after the block's
    /// common prefix: `term.setLength(termBlockPrefix + suffix);
    /// term.grow(term.length()); termSuffixesReader.readBytes(term.bytes(),
    /// termBlockPrefix, suffix)`.
    fn fill_term(&mut self, suffix: usize) -> Result<()> {
        let prefix = self.term_block_prefix;
        self.term.set_length(prefix + suffix);
        self.term_suffixes.read_into(&mut self.term, prefix, suffix)
    }

    /// Port of `_next()`: decodes only the term bytes of the next term. If
    /// caller then asks for metadata, ie docFreq, totalTermFreq or pulls a
    /// D/&PEnum, we then (lazily) decode all metadata up to the current
    /// term.
    fn next_term(&mut self) -> Result<bool> {
        if self.state.term_block_ord == self.block_term_count && !self.next_block()? {
            self.index_is_current = false;
            return Ok(false);
        }

        // TODO: cutover to something better for these ints!  simple64?
        let suffix = self.term_suffixes.read_v_int()? as usize;
        self.fill_term(suffix)?;
        self.state.term_block_ord += 1;

        // NOTE: meaningless in the non-ord case
        self.state.ord += 1;
        Ok(true)
    }

    /// Port of `nextBlock()`: does initial decode of next block of terms;
    /// this doesn't actually decode the docFreq, totalTermFreq, postings
    /// details (frq/prx offset, etc.) metadata; it just loads them as byte[]
    /// blobs which are then decoded on-demand if the metadata is ever
    /// requested for any term in this block. This enables terms-only
    /// intensive consumes (eg certain MTQs, respelling) to not pay the price
    /// of decoding metadata they won't use.
    fn next_block(&mut self) -> Result<bool> {
        // TODO: we still lazy-decode the byte[] for each
        // term (the suffix), but, if we decoded
        // all N terms up front then seeking could do a fast
        // bsearch w/in the block...

        self.state.block_file_pointer = self.input.file_pointer();
        self.block_term_count = self.input.read_v_int()?;
        if self.block_term_count == 0 {
            return Ok(false);
        }
        self.term_block_prefix = self.input.read_v_int()? as usize;

        // term suffixes:
        let len = self.input.read_v_int()? as usize;
        self.term_suffixes.fill(self.input.as_mut(), len)?;

        // docFreq, totalTermFreq
        let len = self.input.read_v_int()? as usize;
        self.freq_reader.fill(self.input.as_mut(), len)?;

        // metadata
        let len = self.input.read_v_int()? as usize;
        self.bytes_reader.fill(self.input.as_mut(), len)?;

        self.meta_data_upto = 0;

        self.state.term_block_ord = 0;

        self.index_is_current = false;
        Ok(true)
    }

    /// Port of `decodeMetaData()`.
    fn decode_meta_data(&mut self) -> Result<()> {
        if !self.seek_pending {
            // TODO: cutover to random-access API
            // here.... really stupid that we have to decode N
            // wasted term metadata just to get to the N+1th
            // that we really need...

            // lazily catch up on metadata decode:
            let limit = self.state.term_block_ord;
            let mut absolute = self.meta_data_upto == 0;
            // TODO: better API would be "jump straight to term=N"???
            let field_info = &self.field.field_info;
            while self.meta_data_upto < limit {
                // TODO: we could make "tiers" of metadata, ie,
                // decode docFreq/totalTF but don't decode postings
                // metadata; this way caller could get
                // docFreq/totalTF w/o paying decode cost for
                // postings

                // TODO: if docFreq were bulk decoded we could
                // just skipN here:

                // docFreq, totalTermFreq
                self.state.doc_freq = self.freq_reader.read_v_int()?;
                if field_info.get_index_options() == IndexOptions::DOCS {
                    // all postings have tf=1
                    self.state.total_term_freq = i64::from(self.state.doc_freq);
                } else {
                    self.state.total_term_freq =
                        i64::from(self.state.doc_freq) + self.freq_reader.read_v_long()?;
                }
                // metadata
                let postings_reader = &self.shared.postings_reader;
                let state = &mut self.state;
                self.bytes_reader
                    .with(|i| postings_reader.decode_term(i, field_info, state, absolute))?;
                self.meta_data_upto += 1;
                absolute = false;
            }
        }
        Ok(())
    }

    fn term_ref(&self) -> BytesRef {
        BytesRef::new(self.term.get().to_vec())
    }
}

impl TermsEnum for SegmentTermsEnum {
    fn attributes(&mut self) -> &mut AttributeSource {
        &mut self.attributes
    }

    // TODO: we may want an alternate mode here which is
    // "if you are about to return NOT_FOUND I won't use
    // the terms data from that"; eg FuzzyTermsEnum will
    // (usually) just immediately call seek again if we
    // return NOT_FOUND so it's a waste for us to fill in
    // the term that was actually NOT_FOUND
    fn seek_ceil(&mut self, target: &BytesRef) -> Result<SeekStatus> {
        if self.index_enum.is_none() {
            return Err(no_index());
        }

        let mut do_seek = true;

        // See if we can avoid seeking, because target term
        // is after current term but before next index term:
        if self.index_is_current {
            let cmp = self.term.get().cmp(target.slice());

            if cmp == std::cmp::Ordering::Equal {
                // Already at the requested term
                return Ok(SeekStatus::FOUND);
            } else if cmp == std::cmp::Ordering::Less {
                // Target term is after current term
                if !self.did_index_next {
                    let index_enum = self.index_enum.as_mut().ok_or_else(no_index)?;
                    if index_enum.next()? == -1 {
                        self.next_index_term = None;
                    } else {
                        self.next_index_term = index_enum.term();
                    }
                    self.did_index_next = true;
                }

                if self
                    .next_index_term
                    .as_ref()
                    .is_none_or(|next| target.slice() < next.slice())
                {
                    // Optimization: requested term is within the
                    // same term block we are now in; skip seeking
                    // (but do scanning):
                    do_seek = false;
                }
            }
        }

        if do_seek {
            // Ask terms index to find biggest indexed term (=
            // first term in a block) that's <= our text:
            let index_enum = self.index_enum.as_mut().ok_or_else(no_index)?;
            let fp = index_enum.seek(target)?;
            self.input.seek(fp)?;
            let result = self.next_block()?;

            // Block must exist since, at least, the indexed term
            // is in the block:
            debug_assert!(result);

            self.index_is_current = true;
            self.did_index_next = false;

            let index_enum = self.index_enum.as_mut().ok_or_else(no_index)?;
            if self.do_ord {
                self.state.ord = index_enum.ord()? - 1;
            }

            let index_term = index_enum.term().unwrap_or_default();
            self.term.copy_bytes(index_term.slice());
        } else if self.state.term_block_ord == self.block_term_count && !self.next_block()? {
            self.index_is_current = false;
            return Ok(SeekStatus::END);
        }

        self.seek_pending = false;

        let mut common = 0usize;

        // Scan within block.  We could do this by calling
        // _next() and testing the resulting term, but this
        // is wasteful.  Instead, we first confirm the
        // target matches the common prefix of this block,
        // and then we scan the term bytes directly from the
        // termSuffixesreader's byte[], saving a copy into
        // the BytesRef term per term.  Only when we return
        // do we then copy the bytes into the term.

        loop {
            // First, see if target term matches common prefix
            // in this block:
            if common < self.term_block_prefix {
                let cmp = i32::from(self.term.byte_at(common)?) - target_byte(target, common)?;
                if cmp < 0 {
                    // TODO: maybe we should store common prefix
                    // in block header?  (instead of relying on
                    // last term of previous block)

                    // Target's prefix is after the common block
                    // prefix, so term cannot be in this block
                    // but it could be in next block.  We
                    // must scan to end-of-block to set common
                    // prefix for next block:
                    if self.state.term_block_ord < self.block_term_count {
                        while self.state.term_block_ord < self.block_term_count - 1 {
                            self.state.term_block_ord += 1;
                            self.state.ord += 1;
                            let skip = self.term_suffixes.read_v_int()? as usize;
                            self.term_suffixes.skip_bytes(skip)?;
                        }
                        let suffix = self.term_suffixes.read_v_int()? as usize;
                        self.fill_term(suffix)?;
                    }
                    self.state.ord += 1;

                    if !self.next_block()? {
                        self.index_is_current = false;
                        return Ok(SeekStatus::END);
                    }
                    common = 0;
                } else if cmp > 0 {
                    // Target's prefix is before the common prefix
                    // of this block, so we position to start of
                    // block and return NOT_FOUND:
                    debug_assert_eq!(self.state.term_block_ord, 0);

                    let suffix = self.term_suffixes.read_v_int()? as usize;
                    self.fill_term(suffix)?;
                    return Ok(SeekStatus::NOT_FOUND);
                } else {
                    common += 1;
                }

                continue;
            }

            // Test every term in this block
            loop {
                self.state.term_block_ord += 1;
                self.state.ord += 1;

                let suffix = self.term_suffixes.read_v_int()? as usize;

                // We know the prefix matches, so just compare the new suffix:
                let term_len = self.term_block_prefix + suffix;
                let mut byte_pos = self.term_suffixes.pos;

                let mut next = false;
                let limit = target.offset
                    + if term_len < target.length {
                        term_len
                    } else {
                        target.length
                    };
                let mut target_pos = target.offset + self.term_block_prefix;
                while target_pos < limit {
                    let suffix_byte =
                        self.term_suffixes
                            .bytes
                            .get(byte_pos)
                            .copied()
                            .ok_or_else(|| {
                                LuceneError::unchecked(
                                    "java.lang.ArrayIndexOutOfBoundsException",
                                    format!(
                                        "Index {byte_pos} out of bounds for length {}",
                                        self.term_suffixes.bytes.len()
                                    ),
                                )
                            })?;
                    byte_pos += 1;
                    let cmp =
                        i32::from(suffix_byte) - target_byte(target, target_pos - target.offset)?;
                    target_pos += 1;
                    if cmp < 0 {
                        // Current term is still before the target;
                        // keep scanning
                        next = true;
                        break;
                    } else if cmp > 0 {
                        // Done!  Current term is after target. Stop
                        // here, fill in real term, return NOT_FOUND.
                        self.fill_term(suffix)?;
                        return Ok(SeekStatus::NOT_FOUND);
                    }
                }

                if !next && target.length <= term_len {
                    self.fill_term(suffix)?;

                    if target.length == term_len {
                        // Done!  Exact match.  Stop here, fill in
                        // real term, return FOUND.
                        return Ok(SeekStatus::FOUND);
                    } else {
                        return Ok(SeekStatus::NOT_FOUND);
                    }
                }

                if self.state.term_block_ord == self.block_term_count {
                    // Must pre-fill term for next block's common prefix
                    self.fill_term(suffix)?;
                    break;
                } else {
                    self.term_suffixes.skip_bytes(suffix)?;
                }
            }

            // The purpose of the terms dict index is to seek
            // the enum to the closest index term before the
            // term we are looking for.  So, we should never
            // cross another index term (besides the first
            // one) while we are scanning:

            debug_assert!(self.index_is_current);

            if !self.next_block()? {
                self.index_is_current = false;
                return Ok(SeekStatus::END);
            }
            common = 0;
        }
    }

    fn next(&mut self) -> Result<Option<BytesRef>> {
        // If seek was previously called and the term was cached,
        // usually caller is just going to pull a D/&PEnum or get
        // docFreq, etc.  But, if they then call next(),
        // this method catches up all internal state so next()
        // works properly:
        if self.seek_pending {
            debug_assert!(!self.index_is_current);
            self.input.seek(self.state.block_file_pointer)?;
            let pending_seek_count = self.state.term_block_ord;
            let result = self.next_block()?;

            let sav_ord = self.state.ord;

            // Block must exist since seek(TermState) was called w/ a
            // TermState previously returned by this enum when positioned
            // on a real term:
            debug_assert!(result);

            while self.state.term_block_ord < pending_seek_count {
                let next_result = self.next_term()?;
                debug_assert!(next_result);
            }
            self.seek_pending = false;
            self.state.ord = sav_ord;
        }
        Ok(if self.next_term()? {
            Some(self.term_ref())
        } else {
            None
        })
    }

    fn term(&self) -> Result<BytesRef> {
        Ok(self.term_ref())
    }

    fn doc_freq(&mut self) -> Result<i32> {
        self.decode_meta_data()?;
        Ok(self.state.doc_freq)
    }

    fn total_term_freq(&mut self) -> Result<i64> {
        self.decode_meta_data()?;
        Ok(self.state.total_term_freq)
    }

    fn postings(
        &mut self,
        reuse: Option<Box<dyn PostingsEnum>>,
        flags: i32,
    ) -> Result<Box<dyn PostingsEnum>> {
        self.decode_meta_data()?;
        self.shared
            .postings_reader
            .postings(&self.field.field_info, &self.state, reuse, flags)
    }

    fn impacts(&mut self, flags: i32) -> Result<Box<dyn ImpactsEnum>> {
        self.decode_meta_data()?;
        self.shared
            .postings_reader
            .impacts(&self.field.field_info, &self.state, flags)
    }

    fn seek_term_state(&mut self, target: &BytesRef, other_state: &dyn TermState) -> Result<()> {
        let other = other_state.as_any().downcast_ref::<BlockTermState>();
        debug_assert!(other.is_some());
        debug_assert!(!self.do_ord || other.is_some_and(|o| o.ord < self.field.num_terms));
        if let Some(other) = other {
            self.state.copy_from(other);
        }
        self.seek_pending = true;
        self.index_is_current = false;
        self.term.copy_bytes(target.slice());
        Ok(())
    }

    fn term_state(&mut self) -> Result<Box<dyn TermState>> {
        self.decode_meta_data()?;
        Ok(Box::new(self.state))
    }

    fn seek_ord(&mut self, ord: i64) -> Result<()> {
        let Some(index_enum) = self.index_enum.as_mut() else {
            return Err(no_index());
        };

        debug_assert!(ord < self.field.num_terms);

        // TODO: if ord is in same terms block and
        // after current ord, we should avoid this seek just
        // like we do in the seek(BytesRef) case
        let fp = index_enum.seek_ord(ord)?;
        self.input.seek(fp)?;
        let result = self.next_block()?;

        // Block must exist since ord < numTerms:
        debug_assert!(result);

        self.index_is_current = true;
        self.did_index_next = false;
        self.seek_pending = false;

        let index_enum = self.index_enum.as_mut().ok_or_else(no_index)?;
        self.state.ord = index_enum.ord()? - 1;
        debug_assert!(self.state.ord >= -1, "ord={}", self.state.ord);
        let index_term = index_enum.term().unwrap_or_default();
        self.term.copy_bytes(index_term.slice());

        // Now, scan:
        let mut left = (ord - self.state.ord) as i32;
        while left > 0 {
            let term = self.next_term()?;
            debug_assert!(term);
            left -= 1;
            debug_assert!(self.index_is_current);
        }
        Ok(())
    }

    fn ord(&self) -> Result<i64> {
        if !self.do_ord {
            return Err(LuceneError::UnsupportedOperation(String::new()));
        }
        Ok(self.state.ord)
    }
}
