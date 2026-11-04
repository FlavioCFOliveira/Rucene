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

//! Port of `org.apache.lucene.codecs.memory.FSTTermsReader`.

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use rucene::codecs::codec_util::{check_index_header, checksum_entire_file};
use rucene::codecs::{BlockTermState, FieldsProducer, PostingsReaderBase, SegmentReadState};
use rucene::error::{LuceneError, Result};
use rucene::index::index_file_names::segment_file_name;
use rucene::index::index_reader::java_object_to_string;
use rucene::index::{
    FieldInfo, Fields, ImpactsEnum, IndexOptions, PostingsEnum, SeekStatus, SegmentInfo, TermState,
    Terms, TermsEnum,
};
use rucene::store::{ByteArrayDataInput, FileTypeHint, IndexInput, PreloadHint};
use rucene::util::attribute::AttributeSource;
use rucene::util::automaton::{AutomatonType, ByteRunnable, CompiledAutomaton};
use rucene::util::fst::{
    target_has_arcs, Arc as FstArc, BytesReader, BytesRefFSTEnum, OffHeapFSTStore, Outputs, Util,
    FST,
};
use rucene::util::java_collections::java_compare_to;
use rucene::util::{BytesRef, IOUtils};

use super::fst_term_outputs::{FSTTermOutputs, TermData};
use super::fst_terms_writer::{
    TERMS_CODEC_NAME, TERMS_EXTENSION, TERMS_VERSION_CURRENT, TERMS_VERSION_START,
};
use crate::blockterms::fixed_gap_terms_index_reader::{field_info_by_number, seek_dir};

/// FST-based terms dictionary reader.
///
/// The FST index maps each term and its ord, and during seek the ord is used
/// to fetch metadata from a single block. The term dictionary is fully
/// memory resident.
///
/// Port of `org.apache.lucene.codecs.memory.FSTTermsReader`. Java's
/// `TreeMap<String, TermsReader>` is a map plus the names in
/// `String.compareTo` order.
///
/// `close()` closes the postings reader and the terms input when this reader
/// is their last holder; one a live enum still shares is released when that
/// enum is dropped (see `Lucene103BlockTreeTermsReader`).
pub struct FSTTermsReader {
    fields: HashMap<String, Arc<TermsReader>>,
    field_names: Vec<String>,
    postings_reader: Option<Arc<dyn PostingsReaderBase>>,
    fst_terms_input: Option<Arc<Box<dyn IndexInput>>>,
}

impl FSTTermsReader {
    /// Port of `FSTTermsReader(SegmentReadState, PostingsReaderBase)`.
    ///
    /// The postings reader moves into the constructor, so when it fails it
    /// also closes it, which Java's caller (`FSTPostingsFormat`) does with
    /// `IOUtils.closeWhileHandlingException(postingsReader)`.
    ///
    /// # Errors
    ///
    /// `CorruptIndexException` for a bad header, checksum or field summary,
    /// or a duplicate field; I/O errors.
    pub fn new(
        state: &SegmentReadState<'_>,
        mut postings_reader: Box<dyn PostingsReaderBase>,
    ) -> Result<Self> {
        let terms_file_name = segment_file_name(
            &state.segment_info.name,
            &state.segment_suffix,
            TERMS_EXTENSION,
        );
        let context = state.context.with_hints(&[
            Arc::new(FileTypeHint::Data),
            Arc::new(PreloadHint::instance()),
        ]);
        let opened = match state
            .directory
            .open_input(&terms_file_name, context.as_ref())
        {
            Ok(mut input) => match Self::read(state, postings_reader.as_mut(), input.as_mut()) {
                Ok(fields) => Ok((input, fields)),
                Err(e) => {
                    IOUtils::close_while_handling_exception([IOUtils::closeable(|| {
                        input.close()
                    })])?;
                    Err(e)
                }
            },
            Err(e) => Err(e),
        };
        match opened {
            Ok((input, fields)) => {
                let mut field_names: Vec<String> = fields.keys().cloned().collect();
                field_names.sort_by(|a, b| java_compare_to(a, b));
                let postings_reader: Arc<dyn PostingsReaderBase> = Arc::from(postings_reader);
                let fields = fields
                    .into_iter()
                    .map(|(name, mut reader)| {
                        reader.postings_reader = Some(Arc::clone(&postings_reader));
                        (name, Arc::new(reader))
                    })
                    .collect();
                Ok(Self {
                    fields,
                    field_names,
                    postings_reader: Some(postings_reader),
                    fst_terms_input: Some(Arc::new(input)),
                })
            }
            Err(e) => {
                IOUtils::close_while_handling_exception([IOUtils::closeable(|| {
                    postings_reader.close()
                })])?;
                Err(e)
            }
        }
    }

    fn read(
        state: &SegmentReadState<'_>,
        postings_reader: &mut dyn PostingsReaderBase,
        input: &mut dyn IndexInput,
    ) -> Result<HashMap<String, TermsReader>> {
        check_index_header(
            input,
            TERMS_CODEC_NAME,
            TERMS_VERSION_START,
            TERMS_VERSION_CURRENT,
            &state.segment_info.id(),
            &state.segment_suffix,
        )?;
        checksum_entire_file(input)?;
        postings_reader.init(input, state)?;
        seek_dir(input)?;

        let mut fields = HashMap::new();
        let num_fields = input.read_v_int()?;
        for _ in 0..num_fields {
            let field_number = input.read_v_int()?;
            let field_info = field_info_by_number(state, field_number)?;
            let num_terms = input.read_v_long()?;
            let sum_total_term_freq = input.read_v_long()?;
            // if frequencies are omitted, sumTotalTermFreq=sumDocFreq and we only write one value
            let sum_doc_freq = if field_info.get_index_options() == IndexOptions::DOCS {
                sum_total_term_freq
            } else {
                input.read_v_long()?
            };
            let doc_count = input.read_v_int()?;
            let current = TermsReader::new(
                field_info,
                input,
                num_terms,
                sum_total_term_freq,
                sum_doc_freq,
                doc_count,
            )?;
            let name = field_info.name.clone();
            let previous = fields.insert(name, current);
            check_field_summary(
                state.segment_info,
                input,
                &fields[&field_info.name],
                previous.is_some(),
            )?;
        }
        Ok(fields)
    }
}

/// Port of `FSTTermsReader.checkFieldSummary`.
fn check_field_summary(
    info: &SegmentInfo,
    input: &dyn IndexInput,
    field: &TermsReader,
    has_previous: bool,
) -> Result<()> {
    let max_doc = info.max_doc()?;
    // #docs with field must be <= #docs
    if field.doc_count < 0 || field.doc_count > max_doc {
        return Err(LuceneError::corrupt_index(
            format!("invalid docCount: {} maxDoc: {max_doc}", field.doc_count),
            input.resource_description(),
        ));
    }
    // #postings must be >= #docs with field
    if field.sum_doc_freq < i64::from(field.doc_count) {
        return Err(LuceneError::corrupt_index(
            format!(
                "invalid sumDocFreq: {} docCount: {}",
                field.sum_doc_freq, field.doc_count
            ),
            input.resource_description(),
        ));
    }
    // #positions must be >= #postings
    if field.sum_total_term_freq < field.sum_doc_freq {
        return Err(LuceneError::corrupt_index(
            format!(
                "invalid sumTotalTermFreq: {} sumDocFreq: {}",
                field.sum_total_term_freq, field.sum_doc_freq
            ),
            input.resource_description(),
        ));
    }
    if has_previous {
        return Err(LuceneError::corrupt_index(
            format!("duplicate fields: {}", field.field_info.name),
            input.resource_description(),
        ));
    }
    Ok(())
}

impl fmt::Display for FSTTermsReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.postings_reader {
            Some(p) => write!(
                f,
                "FSTTermsReader(fields={},delegate={p})",
                self.fields.len()
            ),
            None => write!(
                f,
                "FSTTermsReader(fields={},delegate=null)",
                self.fields.len()
            ),
        }
    }
}

impl Fields for FSTTermsReader {
    fn iterator(&self) -> Box<dyn Iterator<Item = String> + '_> {
        Box::new(self.field_names.iter().cloned())
    }

    fn terms(&self, field: &str) -> Result<Option<Box<dyn Terms>>> {
        Ok(self
            .fields
            .get(field)
            .map(|t| Box::new(SharedTermsReader(Arc::clone(t))) as Box<dyn Terms>))
    }

    fn size(&self) -> i32 {
        self.fields.len() as i32
    }
}

impl FieldsProducer for FSTTermsReader {
    fn check_integrity(&self) -> Result<()> {
        match &self.postings_reader {
            Some(p) => p.check_integrity(),
            None => Err(LuceneError::unchecked(
                "java.lang.NullPointerException",
                "Cannot invoke \"org.apache.lucene.codecs.PostingsReaderBase.checkIntegrity()\"",
            )),
        }
    }

    fn get_merge_instance(&self) -> Result<Box<dyn FieldsProducer>> {
        Ok(Box::new(Self {
            fields: self.fields.clone(),
            field_names: self.field_names.clone(),
            postings_reader: self.postings_reader.clone(),
            fst_terms_input: self.fst_terms_input.clone(),
        }))
    }

    fn close(&mut self) -> Result<()> {
        // try { IOUtils.close(postingsReader, fstTermsInput) } finally { fields.clear() }
        self.fields.clear();
        self.field_names.clear();
        let mut postings_reader = self.postings_reader.take();
        let mut input = self.fst_terms_input.take();
        IOUtils::close([
            postings_reader
                .as_mut()
                .and_then(Arc::get_mut)
                .and_then(|r| IOUtils::closeable(move || r.close())),
            input
                .as_mut()
                .and_then(Arc::get_mut)
                .and_then(|i| IOUtils::closeable(move || i.close())),
        ])
    }
}

/// Port of the inner class `FSTTermsReader.TermsReader`.
pub(crate) struct TermsReader {
    field_info: FieldInfo,
    num_terms: i64,
    sum_total_term_freq: i64,
    sum_doc_freq: i64,
    doc_count: i32,
    dict: Arc<FST<FSTTermOutputs>>,
    postings_reader: Option<Arc<dyn PostingsReaderBase>>,
}

impl TermsReader {
    fn new(
        field_info: &FieldInfo,
        input: &mut dyn IndexInput,
        num_terms: i64,
        sum_total_term_freq: i64,
        sum_doc_freq: i64,
        doc_count: i32,
    ) -> Result<Self> {
        let outputs = FSTTermOutputs::new(field_info);
        let fst_metadata = FST::read_metadata(input, outputs)?;
        let off_heap_fst_store =
            OffHeapFSTStore::new(input.clone_input()?, input.file_pointer(), &fst_metadata);
        let size = off_heap_fst_store.size();
        let dict = FST::from_reader(fst_metadata, Box::new(off_heap_fst_store));
        input.skip_bytes(size)?;
        Ok(Self {
            field_info: field_info.clone(),
            num_terms,
            sum_total_term_freq,
            sum_doc_freq,
            doc_count,
            dict: Arc::new(dict),
            postings_reader: None,
        })
    }

    fn postings_reader(&self) -> &Arc<dyn PostingsReaderBase> {
        self.postings_reader
            .as_ref()
            .expect("INVARIANT: the reader's constructor sets the postings reader")
    }
}

impl fmt::Display for TermsReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "FSTTerms(terms={},postings={},positions={},docs={})",
            self.num_terms, self.sum_doc_freq, self.sum_total_term_freq, self.doc_count
        )
    }
}

/// The `Terms` the reader hands out: its `TermsReader` itself.
struct SharedTermsReader(Arc<TermsReader>);

impl fmt::Display for SharedTermsReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&*self.0, f)
    }
}

impl Terms for SharedTermsReader {
    fn iterator(&self) -> Result<Box<dyn TermsEnum>> {
        Ok(Box::new(SegmentTermsEnum::new(Arc::clone(&self.0))?))
    }

    fn intersect(
        &self,
        compiled: &CompiledAutomaton,
        start_term: Option<&BytesRef>,
    ) -> Result<Box<dyn TermsEnum>> {
        if compiled.automaton_type != AutomatonType::Normal {
            return Err(LuceneError::IllegalArgument(
                "please use CompiledAutomaton.getTermsEnum instead".to_string(),
            ));
        }
        Ok(Box::new(IntersectTermsEnum::new(
            Arc::clone(&self.0),
            compiled.clone(),
            start_term,
        )?))
    }

    fn has_freqs(&self) -> bool {
        self.0
            .field_info
            .get_index_options()
            .subsumes(IndexOptions::DOCS_AND_FREQS)
    }

    fn has_offsets(&self) -> bool {
        self.0
            .field_info
            .get_index_options()
            .subsumes(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS)
    }

    fn has_positions(&self) -> bool {
        self.0
            .field_info
            .get_index_options()
            .subsumes(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS)
    }

    fn has_payloads(&self) -> bool {
        self.0.field_info.has_payloads()
    }

    fn size(&self) -> Result<i64> {
        Ok(self.0.num_terms)
    }

    fn sum_total_term_freq(&self) -> Result<i64> {
        Ok(self.0.sum_total_term_freq)
    }

    fn sum_doc_freq(&self) -> Result<i64> {
        Ok(self.0.sum_doc_freq)
    }

    fn doc_count(&self) -> Result<i32> {
        Ok(self.0.doc_count)
    }
}

/// Port of the abstract inner class `TermsReader.BaseTermsEnum`: the state
/// both enums share (current term stats, decoded metadata, undecoded
/// metadata and its reader).
struct BaseState {
    terms: Arc<TermsReader>,
    attributes: AttributeSource,
    /// Current term stats + decoded metadata (customized by PBF).
    state: BlockTermState,
    /// Current term stats + undecoded metadata (long[] & byte[]).
    meta: Option<TermData>,
    /// `bytesReader`: the bytes it was last reset over and its position.
    bytes_reader: Option<Arc<[u8]>>,
    bytes_reader_pos: usize,
}

impl BaseState {
    fn new(terms: Arc<TermsReader>) -> Result<Self> {
        let state = terms.postings_reader().new_term_state()?;
        Ok(Self {
            terms,
            attributes: AttributeSource::new(),
            state,
            meta: None,
            bytes_reader: None,
            bytes_reader_pos: 0,
        })
    }

    /// `if (meta.bytes != null) bytesReader.reset(meta.bytes, 0,
    /// meta.bytes.length); postingsReader.decodeTerm(bytesReader, fieldInfo,
    /// state, true)`. Java's reader keeps the bytes and position of its last
    /// reset when the term has no bytes.
    fn decode(&mut self) -> Result<()> {
        let meta = self.meta.as_ref().ok_or_else(|| {
            LuceneError::unchecked(
                "java.lang.NullPointerException",
                "Cannot read field \"bytes\" because \"this.meta\" is null",
            )
        })?;
        if let Some(bytes) = &meta.bytes {
            self.bytes_reader = Some(Arc::clone(bytes));
            self.bytes_reader_pos = 0;
        }
        let bytes: &[u8] = self.bytes_reader.as_deref().unwrap_or(&[]);
        let mut input = ByteArrayDataInput::new(bytes);
        input.set_position(self.bytes_reader_pos);
        let terms = &self.terms;
        terms.postings_reader().decode_term(
            &mut input,
            &terms.field_info,
            &mut self.state,
            true,
        )?;
        self.bytes_reader_pos = input.position();
        Ok(())
    }

    fn total_term_freq(&self) -> i64 {
        if self.state.total_term_freq == -1 {
            i64::from(self.state.doc_freq)
        } else {
            self.state.total_term_freq
        }
    }

    fn postings(
        &self,
        reuse: Option<Box<dyn PostingsEnum>>,
        flags: i32,
    ) -> Result<Box<dyn PostingsEnum>> {
        self.terms
            .postings_reader()
            .postings(&self.terms.field_info, &self.state, reuse, flags)
    }

    fn impacts(&self, flags: i32) -> Result<Box<dyn ImpactsEnum>> {
        self.terms
            .postings_reader()
            .impacts(&self.terms.field_info, &self.state, flags)
    }
}

fn unsupported() -> LuceneError {
    LuceneError::UnsupportedOperation(String::new())
}

/// Port of the private inner class `TermsReader.SegmentTermsEnum`: iterates
/// through all terms in this field.
struct SegmentTermsEnum {
    base: BaseState,
    /// Current term, null when enum ends or unpositioned.
    term: Option<BytesRef>,
    fst_enum: BytesRefFSTEnum<'static, FSTTermOutputs>,
    /// True when current term's metadata is decoded.
    decoded: bool,
    /// True when current enum is 'positioned' by seekExact(TermState).
    seek_pending: bool,
}

impl fmt::Display for SegmentTermsEnum {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&java_object_to_string(
            "org.apache.lucene.codecs.memory.FSTTermsReader$TermsReader$SegmentTermsEnum",
            self,
        ))
    }
}

impl SegmentTermsEnum {
    fn new(terms: Arc<TermsReader>) -> Result<Self> {
        let fst_enum = BytesRefFSTEnum::new_shared(Arc::clone(&terms.dict))?;
        Ok(Self {
            base: BaseState::new(terms)?,
            term: None,
            fst_enum,
            decoded: false,
            seek_pending: false,
        })
    }

    /// Port of `decodeMetaData()`: lets the postings reader decode metadata
    /// from long[] and byte[].
    fn decode_meta_data(&mut self) -> Result<()> {
        if !self.decoded && !self.seek_pending {
            self.base.decode()?;
            self.decoded = true;
        }
        Ok(())
    }

    /// Port of `updateEnum(InputOutput)`: updates current enum according to
    /// FSTEnum.
    fn update_enum(&mut self, pair: Option<(BytesRef, TermData)>) {
        match pair {
            None => self.term = None,
            Some((input, output)) => {
                self.term = Some(input);
                self.base.state.doc_freq = output.doc_freq;
                self.base.state.total_term_freq = output.total_term_freq;
                self.base.meta = Some(output);
            }
        }
        self.decoded = false;
        self.seek_pending = false;
    }
}

/// An `InputOutput` of the FST enum, copied.
fn pair(
    io: Option<rucene::util::fst::BytesRefInputOutput<'_, TermData>>,
) -> Option<(BytesRef, TermData)> {
    io.map(|io| (io.input.clone(), io.output.clone()))
}

impl TermsEnum for SegmentTermsEnum {
    fn attributes(&mut self) -> &mut AttributeSource {
        &mut self.base.attributes
    }

    fn term(&self) -> Result<BytesRef> {
        // Java returns `null` when unpositioned; the port's `term()` cannot.
        Ok(self.term.clone().unwrap_or_default())
    }

    fn next(&mut self) -> Result<Option<BytesRef>> {
        if self.seek_pending {
            // previously positioned, but termOutputs not fetched
            self.seek_pending = false;
            let target = self.term.clone().unwrap_or_default();
            let status = self.seek_ceil(&target)?;
            // must positioned on valid term
            debug_assert_eq!(status, SeekStatus::FOUND);
        }
        let next = pair(self.fst_enum.next()?);
        self.update_enum(next);
        Ok(self.term.clone())
    }

    fn seek_exact(&mut self, target: &BytesRef) -> Result<bool> {
        let found = pair(self.fst_enum.seek_exact(target)?);
        self.update_enum(found);
        Ok(self.term.is_some())
    }

    fn seek_ceil(&mut self, target: &BytesRef) -> Result<SeekStatus> {
        let found = pair(self.fst_enum.seek_ceil(target)?);
        self.update_enum(found);
        Ok(match &self.term {
            None => SeekStatus::END,
            Some(term) if term == target => SeekStatus::FOUND,
            Some(_) => SeekStatus::NOT_FOUND,
        })
    }

    fn seek_term_state(&mut self, target: &BytesRef, other_state: &dyn TermState) -> Result<()> {
        if self.term.as_ref() != Some(target) {
            TermState::copy_from(&mut self.base.state, other_state);
            self.term = Some(BytesRef::deep_copy_of(target));
            self.seek_pending = true;
        }
        Ok(())
    }

    fn seek_ord(&mut self, _ord: i64) -> Result<()> {
        Err(unsupported())
    }

    fn ord(&self) -> Result<i64> {
        Err(unsupported())
    }

    fn doc_freq(&mut self) -> Result<i32> {
        Ok(self.base.state.doc_freq)
    }

    fn total_term_freq(&mut self) -> Result<i64> {
        Ok(self.base.total_term_freq())
    }

    fn term_state(&mut self) -> Result<Box<dyn TermState>> {
        self.decode_meta_data()?;
        Ok(Box::new(self.base.state))
    }

    fn postings(
        &mut self,
        reuse: Option<Box<dyn PostingsEnum>>,
        flags: i32,
    ) -> Result<Box<dyn PostingsEnum>> {
        self.decode_meta_data()?;
        self.base.postings(reuse, flags)
    }

    fn impacts(&mut self, flags: i32) -> Result<Box<dyn ImpactsEnum>> {
        self.decode_meta_data()?;
        self.base.impacts(flags)
    }
}

/// Port of the private inner class `IntersectTermsEnum.Frame`.
#[derive(Clone, Default)]
struct Frame {
    /// fst stats
    fst_arc: FstArc<TermData>,
    output: TermData,
    /// automaton stats
    fsa_state: i32,
}

impl Frame {
    fn new() -> Self {
        Self {
            fst_arc: FstArc::default(),
            output: TermData::default(),
            fsa_state: -1,
        }
    }
}

/// Port of the private inner class `TermsReader.IntersectTermsEnum`:
/// iterates intersect result with automaton (cannot seek!).
///
/// Java's frames are objects on a stack that the methods pass around; here
/// a frame is its index, and every method that takes Java's `(top, frame)`
/// pair works on `stack[level]` and `stack[level + 1]`, which is what Java
/// always passes.
struct IntersectTermsEnum {
    base: BaseState,
    /// Current term, null when enum ends or unpositioned.
    term: Option<Vec<u8>>,
    /// True when current term's metadata is decoded.
    decoded: bool,
    /// True when there is pending term when calling next().
    pending: bool,
    /// stack to record how current term is constructed, used to accumulate
    /// metadata or rewind term: level == term.length + 1, == 0 when term is
    /// null.
    stack: Vec<Frame>,
    level: i32,
    /// to which level the metadata is accumulated so that we can accumulate
    /// metadata lazily.
    meta_upto: i32,
    /// term dict fst
    fst: Arc<FST<FSTTermOutputs>>,
    fst_reader: Box<dyn BytesReader>,
    /// query automaton to intersect with
    compiled: CompiledAutomaton,
}

impl fmt::Display for IntersectTermsEnum {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&java_object_to_string(
            "org.apache.lucene.codecs.memory.FSTTermsReader$TermsReader$IntersectTermsEnum",
            self,
        ))
    }
}

impl IntersectTermsEnum {
    fn new(
        terms: Arc<TermsReader>,
        compiled: CompiledAutomaton,
        start_term: Option<&BytesRef>,
    ) -> Result<Self> {
        let fst = Arc::clone(&terms.dict);
        let fst_reader = fst.get_owned_bytes_reader()?;
        let mut e = Self {
            base: BaseState::new(terms)?,
            term: None,
            decoded: false,
            pending: false,
            stack: (0..16).map(|_| Frame::new()).collect(),
            level: -1,
            meta_upto: 0,
            fst,
            fst_reader,
            compiled,
        };
        if e.compiled.get_byte_runnable().is_none() {
            return Err(LuceneError::unchecked(
                "java.lang.NullPointerException",
                "Cannot invoke \"org.apache.lucene.util.automaton.ByteRunnable.step(int, int)\" \
                 because \"this.fsa\" is null",
            ));
        }

        let f = e.new_frame();
        e.load_virtual_frame(f);
        e.level += 1;
        let f = e.new_frame();
        e.load_first_frame(f);
        e.push_frame(f);

        e.base.meta = None;
        e.meta_upto = 1;
        e.decoded = false;
        e.pending = false;

        match start_term {
            None => {
                e.pending = e.is_accept(e.level as usize);
            }
            Some(start_term) => {
                e.do_seek_ceil(start_term)?;
                let top = e.level as usize;
                e.pending = e.term.as_deref() != Some(start_term.slice())
                    && e.is_valid(top)
                    && e.is_accept(top);
            }
        }
        Ok(e)
    }

    fn fsa(&self) -> &dyn ByteRunnable {
        self.compiled
            .get_byte_runnable()
            .expect("INVARIANT: the constructor checked the byte runnable")
    }

    fn term_ref(&self) -> Option<BytesRef> {
        self.term.as_ref().map(|t| BytesRef::new(t.clone()))
    }

    /// Port of `decodeMetaData()`.
    fn decode_meta_data(&mut self) -> Result<()> {
        debug_assert!(self.term.is_some());
        if !self.decoded {
            self.base.decode()?;
            self.decoded = true;
        }
        Ok(())
    }

    /// Port of `loadMetaData()`: lazily accumulate meta data, when we got a
    /// accepted term.
    fn load_meta_data(&mut self) {
        let outputs = self.fst.outputs().clone();
        let mut last = self.meta_upto as usize;
        while self.meta_upto != self.level {
            self.meta_upto += 1;
            let next = self.meta_upto as usize;
            self.stack[next].output =
                outputs.add(&self.stack[next].output, &self.stack[last].output);
            last = next;
        }
        let last = &self.stack[last];
        let meta = if last.fst_arc.is_final() {
            outputs.add(&last.output, last.fst_arc.next_final_output())
        } else {
            last.output.clone()
        };
        self.base.state.doc_freq = meta.doc_freq;
        self.base.state.total_term_freq = meta.total_term_freq;
        self.base.meta = Some(meta);
    }

    /// Port of `doSeekCeil(BytesRef)`; the term it returns is the enum's.
    fn do_seek_ceil(&mut self, target: &BytesRef) -> Result<()> {
        let mut frame: Option<usize> = None;
        let mut upto = 0usize;
        let limit = target.length;
        while upto < limit {
            // to target prefix, or ceil label (rewind prefix)
            let f = self.new_frame();
            let label = i32::from(target.bytes[target.offset + upto]);
            frame = if self.load_ceil_frame(label)? {
                Some(f)
            } else {
                None
            };
            match frame {
                Some(f) if self.stack[f].fst_arc.label() == label => {}
                _ => break,
            }
            // target must be fetched from automaton
            debug_assert!(self.is_valid(frame.expect("INVARIANT: matched Some above")));
            self.push_frame(frame.expect("INVARIANT: matched Some above"));
            upto += 1;
        }
        if upto == limit {
            // got target
            return Ok(());
        }
        if let Some(f) = frame {
            // got larger term('s prefix)
            self.push_frame(f);
            if !self.is_accept(f) {
                self.next()?;
            }
            return Ok(());
        }
        while self.level > 0 {
            // got target's prefix, advance to larger term
            let mut f = self.pop_frame();
            while self.level > 0 && !self.can_rewind(f) {
                f = self.pop_frame();
            }
            if self.load_next_frame()? {
                self.push_frame(f);
                if !self.is_accept(f) {
                    self.next()?;
                }
                return Ok(());
            }
        }
        // `return null` leaves the term as it is: the popped frames emptied it.
        Ok(())
    }

    /// Port of `loadVirtualFrame(Frame)`: virtual frame, never pop.
    fn load_virtual_frame(&mut self, f: usize) {
        let frame = &mut self.stack[f];
        frame.output = TermData::default();
        frame.fsa_state = -1;
    }

    /// Port of `loadFirstFrame(Frame)`: load frame for start arc(node) on fst.
    fn load_first_frame(&mut self, f: usize) {
        let fst = Arc::clone(&self.fst);
        let frame = &mut self.stack[f];
        fst.get_first_arc(&mut frame.fst_arc);
        frame.output = frame.fst_arc.output().clone();
        frame.fsa_state = 0;
    }

    /// Port of `loadExpandFrame(top, frame)`: load frame for target arc(node)
    /// on fst; `true` is Java's frame returned, `false` its `null`.
    fn load_expand_frame(&mut self) -> Result<bool> {
        let level = self.level as usize;
        if !self.can_grow(level) {
            return Ok(false);
        }
        let (lo, hi) = self.stack.split_at_mut(level + 1);
        let (top, frame) = (&lo[level], &mut hi[0]);
        self.fst.read_first_real_target_arc(
            top.fst_arc.target(),
            &mut frame.fst_arc,
            self.fst_reader.as_mut(),
        )?;
        let fsa = self
            .compiled
            .get_byte_runnable()
            .expect("INVARIANT: the constructor checked the byte runnable");
        frame.fsa_state = fsa.step(top.fsa_state, frame.fst_arc.label());
        if frame.fsa_state == -1 {
            return self.load_next_frame();
        }
        frame.output = frame.fst_arc.output().clone();
        Ok(true)
    }

    /// Port of `loadNextFrame(top, frame)`: load frame for sibling arc(node)
    /// on fst.
    fn load_next_frame(&mut self) -> Result<bool> {
        let level = self.level as usize;
        if !self.can_rewind(level + 1) {
            return Ok(false);
        }
        let (lo, hi) = self.stack.split_at_mut(level + 1);
        let (top, frame) = (&lo[level], &mut hi[0]);
        let fsa = self
            .compiled
            .get_byte_runnable()
            .expect("INVARIANT: the constructor checked the byte runnable");
        while !frame.fst_arc.is_last() {
            self.fst
                .read_next_real_arc(&mut frame.fst_arc, self.fst_reader.as_mut())?;
            frame.fsa_state = fsa.step(top.fsa_state, frame.fst_arc.label());
            if frame.fsa_state != -1 {
                break;
            }
        }
        if frame.fsa_state == -1 {
            return Ok(false);
        }
        frame.output = frame.fst_arc.output().clone();
        Ok(true)
    }

    /// Port of `loadCeilFrame(label, top, frame)`: load frame for target arc
    /// (node) on fst, so that `arc.label >= label` and
    /// `!fsa.reject(arc.label)`.
    fn load_ceil_frame(&mut self, label: i32) -> Result<bool> {
        let level = self.level as usize;
        let (lo, hi) = self.stack.split_at_mut(level + 1);
        let (top, frame) = (&lo[level], &mut hi[0]);
        if !Util::read_ceil_arc(
            label,
            &self.fst,
            &top.fst_arc,
            &mut frame.fst_arc,
            self.fst_reader.as_mut(),
        )? {
            return Ok(false);
        }
        let fsa = self
            .compiled
            .get_byte_runnable()
            .expect("INVARIANT: the constructor checked the byte runnable");
        frame.fsa_state = fsa.step(top.fsa_state, frame.fst_arc.label());
        if frame.fsa_state == -1 {
            return self.load_next_frame();
        }
        frame.output = frame.fst_arc.output().clone();
        Ok(true)
    }

    /// reach a term both fst&fsa accepts
    fn is_accept(&self, f: usize) -> bool {
        let frame = &self.stack[f];
        self.fsa().is_accept(frame.fsa_state) && frame.fst_arc.is_final()
    }

    /// reach a prefix both fst&fsa won't reject
    fn is_valid(&self, f: usize) -> bool {
        self.stack[f].fsa_state != -1
    }

    /// can walk forward on both fst&fsa
    fn can_grow(&self, f: usize) -> bool {
        let frame = &self.stack[f];
        frame.fsa_state != -1 && target_has_arcs(&frame.fst_arc)
    }

    /// can jump to sibling
    fn can_rewind(&self, f: usize) -> bool {
        !self.stack[f].fst_arc.is_last()
    }

    fn push_frame(&mut self, f: usize) {
        let label = self.stack[f].fst_arc.label();
        self.grow(label);
        self.level += 1;
    }

    fn pop_frame(&mut self) -> usize {
        self.shrink();
        self.level -= 1;
        self.meta_upto = if self.meta_upto > self.level {
            self.level
        } else {
            self.meta_upto
        };
        (self.level + 1) as usize
    }

    fn new_frame(&mut self) -> usize {
        if (self.level + 1) as usize == self.stack.len() {
            let new_len = rucene::util::ArrayUtil::oversize(
                (self.level + 2) as usize,
                rucene::util::RamUsageEstimator::NUM_BYTES_OBJECT_REF as usize,
            );
            self.stack.resize_with(new_len, Frame::new);
        }
        (self.level + 1) as usize
    }

    fn grow(&mut self, label: i32) {
        match &mut self.term {
            None => self.term = Some(Vec::new()),
            Some(term) => term.push(label as u8),
        }
    }

    fn shrink(&mut self) {
        if let Some(term) = &mut self.term {
            if term.is_empty() {
                self.term = None;
            } else {
                term.pop();
            }
        }
    }
}

impl TermsEnum for IntersectTermsEnum {
    fn attributes(&mut self) -> &mut AttributeSource {
        &mut self.base.attributes
    }

    fn term(&self) -> Result<BytesRef> {
        // Java returns `null` when unpositioned; the port's `term()` cannot.
        Ok(self.term_ref().unwrap_or_default())
    }

    fn seek_ceil(&mut self, target: &BytesRef) -> Result<SeekStatus> {
        self.decoded = false;
        self.do_seek_ceil(target)?;
        self.load_meta_data();
        Ok(match &self.term {
            None => SeekStatus::END,
            Some(term) if term.as_slice() == target.slice() => SeekStatus::FOUND,
            Some(_) => SeekStatus::NOT_FOUND,
        })
    }

    fn next(&mut self) -> Result<Option<BytesRef>> {
        if self.pending {
            self.pending = false;
            self.load_meta_data();
            return Ok(self.term_ref());
        }
        self.decoded = false;
        'dfs: while self.level > 0 {
            let f = self.new_frame();
            if self.load_expand_frame()? {
                // has valid target
                self.push_frame(f);
                if self.is_accept(f) {
                    // gotcha
                    break;
                }
                continue; // check next target
            }
            let mut f = self.pop_frame();
            while self.level > 0 {
                if self.load_next_frame()? {
                    // has valid sibling
                    self.push_frame(f);
                    if self.is_accept(f) {
                        // gotcha
                        break 'dfs;
                    }
                    continue 'dfs; // check next target
                }
                f = self.pop_frame();
            }
            return Ok(None);
        }
        self.load_meta_data();
        Ok(self.term_ref())
    }

    fn seek_ord(&mut self, _ord: i64) -> Result<()> {
        Err(unsupported())
    }

    fn ord(&self) -> Result<i64> {
        Err(unsupported())
    }

    fn doc_freq(&mut self) -> Result<i32> {
        Ok(self.base.state.doc_freq)
    }

    fn total_term_freq(&mut self) -> Result<i64> {
        Ok(self.base.total_term_freq())
    }

    fn term_state(&mut self) -> Result<Box<dyn TermState>> {
        self.decode_meta_data()?;
        Ok(Box::new(self.base.state))
    }

    fn postings(
        &mut self,
        reuse: Option<Box<dyn PostingsEnum>>,
        flags: i32,
    ) -> Result<Box<dyn PostingsEnum>> {
        self.decode_meta_data()?;
        self.base.postings(reuse, flags)
    }

    fn impacts(&mut self, flags: i32) -> Result<Box<dyn ImpactsEnum>> {
        self.decode_meta_data()?;
        self.base.impacts(flags)
    }
}

/// Port of the static `FSTTermsReader.walk(FST)`, a debugging helper that
/// visits every node breadth first.
#[allow(dead_code)]
pub(crate) fn walk<O: Outputs>(fst: &FST<O>) -> Result<()> {
    let mut queue: std::collections::VecDeque<FstArc<O::Output>> =
        std::collections::VecDeque::new();
    let mut seen = std::collections::HashSet::new();
    let mut reader = fst.get_bytes_reader()?;
    let mut start_arc = FstArc::default();
    fst.get_first_arc(&mut start_arc);
    queue.push_back(start_arc);
    while let Some(mut arc) = queue.pop_front() {
        let node = arc.target();
        if target_has_arcs(&arc) && seen.insert(node) {
            fst.read_first_real_target_arc(node, &mut arc, reader.as_mut())?;
            loop {
                queue.push_back(arc.clone());
                if arc.is_last() {
                    break;
                }
                fst.read_next_real_arc(&mut arc, reader.as_mut())?;
            }
        }
    }
    Ok(())
}
