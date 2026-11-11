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

//! Port of `org.apache.lucene.codecs.blocktreeords.OrdsBlockTreeTermsWriter`.

use std::cell::RefCell;

use rucene::codecs::codec_util::{write_footer, write_index_header};
use rucene::codecs::{
    BlockTermState, FieldsConsumer, Lucene103BlockTreeTermsWriter, NormsProducer,
    PostingsWriterBase, SegmentWriteState,
};
use rucene::error::{LuceneError, Result};
use rucene::index::index_file_names::segment_file_name;
use rucene::index::{FieldInfo, FieldInfos, Fields, IndexOptions, TermsEnum};
use rucene::store::{ByteBuffersDataOutput, DataOutput, IndexOutput};
use rucene::util::fst::{Builder, BytesRefFSTEnum, InputType, Util, FST};
use rucene::util::{BytesRef, FixedBitSet, IOUtils, IntsRefBuilder};

use super::fst_ords_outputs::{FSTOrdsOutputs, Output};

/// Port of `OrdsBlockTreeTermsWriter.FST_OUTPUTS`.
pub(crate) const FST_OUTPUTS: FSTOrdsOutputs = FSTOrdsOutputs;

/// Suggested default value for the `minItemsInBlock` parameter to
/// [`OrdsBlockTreeTermsWriter::new`].
pub const DEFAULT_MIN_BLOCK_SIZE: i32 = 25;

/// Suggested default value for the `maxItemsInBlock` parameter to
/// [`OrdsBlockTreeTermsWriter::new`].
pub const DEFAULT_MAX_BLOCK_SIZE: i32 = 48;

pub(crate) const OUTPUT_FLAGS_NUM_BITS: u32 = 2;
#[allow(dead_code)]
pub(crate) const OUTPUT_FLAGS_MASK: i64 = 0x3;
pub(crate) const OUTPUT_FLAG_IS_FLOOR: i64 = 0x1;
pub(crate) const OUTPUT_FLAG_HAS_TERMS: i64 = 0x2;

/// Extension of terms file.
pub(crate) const TERMS_EXTENSION: &str = "tio";

pub(crate) const TERMS_CODEC_NAME: &str = "OrdsBlockTreeTerms";

/// Initial terms format.
pub const VERSION_START: i32 = 1;

/// Current terms format.
pub const VERSION_CURRENT: i32 = VERSION_START;

/// Extension of terms index file.
pub(crate) const TERMS_INDEX_EXTENSION: &str = "tipo";

pub(crate) const TERMS_INDEX_CODEC_NAME: &str = "OrdsBlockTreeIndex";

/// Port of the private record `OrdsBlockTreeTermsWriter.FieldMetaData`.
struct FieldMetaData {
    field_info: FieldInfo,
    root_code: Output,
    num_terms: i64,
    index_start_fp: i64,
    sum_total_term_freq: i64,
    sum_doc_freq: i64,
    doc_count: i32,
    min_term: Vec<u8>,
    max_term: Vec<u8>,
}

/// This is just like `Lucene103BlockTreeTermsWriter`, except it also stores
/// a version per term, and adds a method to its `TermsEnum` implementation
/// to `seekExact` only if the version is >= the specified version. The
/// version is added to the terms index to avoid seeking if no term in the
/// block has a high enough version. The term blocks file is `.tiv` and the
/// terms index extension is `.tipv`.
///
/// (Lucene's javadoc, kept as is: the files this writer writes are `.tio`
/// and `.tipo`, and what it stores per term is its ordinal.)
///
/// Port of `org.apache.lucene.codecs.blocktreeords.OrdsBlockTreeTermsWriter`.
pub struct OrdsBlockTreeTermsWriter {
    out: Box<dyn IndexOutput>,
    index_out: Box<dyn IndexOutput>,
    max_doc: i32,
    min_items_in_block: i32,
    max_items_in_block: i32,
    postings_writer: Box<dyn PostingsWriterBase>,
    field_infos: FieldInfos,
    fields: Vec<FieldMetaData>,
    scratch_bytes: ByteBuffersDataOutput,
    scratch_ints_ref: IntsRefBuilder,
    closed: bool,
}

impl OrdsBlockTreeTermsWriter {
    /// Create a new writer. The number of items (terms or sub-blocks) per
    /// block will aim to be between `min_items_in_block` and
    /// `max_items_in_block`, though in some cases the blocks may be smaller
    /// than the min.
    ///
    /// Port of `OrdsBlockTreeTermsWriter(SegmentWriteState,
    /// PostingsWriterBase, int, int)`. The postings writer moves into the
    /// constructor, so when it fails it also closes it, which Java's caller
    /// (`BlockTreeOrdsPostingsFormat.fieldsConsumer`) does with
    /// `IOUtils.closeWhileHandlingException(postingsWriter)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for invalid block sizes; the I/O errors of
    /// creating the files, writing their headers and the postings writer's
    /// `init`.
    pub fn new(
        state: &SegmentWriteState<'_>,
        mut postings_writer: Box<dyn PostingsWriterBase>,
        min_items_in_block: i32,
        max_items_in_block: i32,
    ) -> Result<Self> {
        match Self::open(
            state,
            postings_writer.as_mut(),
            min_items_in_block,
            max_items_in_block,
        ) {
            Ok((out, index_out, max_doc)) => Ok(Self {
                out,
                index_out,
                max_doc,
                min_items_in_block,
                max_items_in_block,
                postings_writer,
                field_infos: state.field_infos.clone(),
                fields: Vec::new(),
                scratch_bytes: ByteBuffersDataOutput::new_resettable_instance(),
                scratch_ints_ref: IntsRefBuilder::new(),
                closed: false,
            }),
            Err(e) => {
                IOUtils::close_while_handling_exception([IOUtils::closeable(|| {
                    postings_writer.close()
                })])?;
                Err(e)
            }
        }
    }

    #[allow(clippy::type_complexity)]
    fn open(
        state: &SegmentWriteState<'_>,
        postings_writer: &mut dyn PostingsWriterBase,
        min_items_in_block: i32,
        max_items_in_block: i32,
    ) -> Result<(Box<dyn IndexOutput>, Box<dyn IndexOutput>, i32)> {
        Lucene103BlockTreeTermsWriter::validate_settings(min_items_in_block, max_items_in_block)?;

        let max_doc = state.segment_info.max_doc()?;

        let terms_file_name = segment_file_name(
            &state.segment_info.name,
            &state.segment_suffix,
            TERMS_EXTENSION,
        );
        let mut out = state
            .directory
            .create_output(&terms_file_name, state.context)?;
        let mut index_out: Option<Box<dyn IndexOutput>> = None;
        let init = (|| -> Result<()> {
            write_index_header(
                out.as_mut(),
                TERMS_CODEC_NAME,
                VERSION_CURRENT,
                &state.segment_info.id(),
                &state.segment_suffix,
            )?;

            let terms_index_file_name = segment_file_name(
                &state.segment_info.name,
                &state.segment_suffix,
                TERMS_INDEX_EXTENSION,
            );
            let index = index_out.insert(
                state
                    .directory
                    .create_output(&terms_index_file_name, state.context)?,
            );
            write_index_header(
                index.as_mut(),
                TERMS_INDEX_CODEC_NAME,
                VERSION_CURRENT,
                &state.segment_info.id(),
                &state.segment_suffix,
            )?;

            // have consumer write its format/header
            postings_writer.init(out.as_mut(), state)
        })();
        match init {
            Ok(()) => {
                let index_out = index_out.expect("INVARIANT: created before init succeeds");
                Ok((out, index_out, max_doc))
            }
            Err(e) => {
                IOUtils::close_while_handling_exception([
                    IOUtils::closeable(|| out.close()),
                    IOUtils::closeable(|| index_out.as_mut().map_or(Ok(()), |o| o.close())),
                ])?;
                Err(e)
            }
        }
    }
}

/// Port of `OrdsBlockTreeTermsWriter.encodeOutput(long, boolean, boolean)`.
pub(crate) fn encode_output(fp: i64, has_terms: bool, is_floor: bool) -> i64 {
    debug_assert!(fp < (1i64 << 62));
    (fp << 2)
        | (if has_terms { OUTPUT_FLAG_HAS_TERMS } else { 0 })
        | (if is_floor { OUTPUT_FLAG_IS_FLOOR } else { 0 })
}

impl FieldsConsumer for OrdsBlockTreeTermsWriter {
    fn write(&mut self, fields: &dyn Fields, norms: Option<&dyn NormsProducer>) -> Result<()> {
        let mut last_field: Option<String> = None;
        for field in fields.iterator() {
            debug_assert!(last_field.as_deref().is_none_or(|last| {
                rucene::util::java_collections::java_compare_to(last, &field)
                    == std::cmp::Ordering::Less
            }));
            last_field = Some(field.clone());

            let Some(terms) = fields.terms(&field)? else {
                continue;
            };

            let mut terms_enum = terms.iterator()?;

            let field_info = self
                .field_infos
                .field_info(&field)
                .cloned()
                .ok_or_else(|| {
                    LuceneError::unchecked(
                        "java.lang.NullPointerException",
                        "Cannot invoke \"org.apache.lucene.index.FieldInfo.getIndexOptions()\" \
                         because \"fieldInfo\" is null",
                    )
                })?;
            let mut terms_writer = TermsWriter::new(self, field_info)?;
            while let Some(term) = terms_enum.next()? {
                terms_writer.write(self, &term, terms_enum.as_mut(), norms)?;
            }

            terms_writer.finish(self)?;
        }
        Ok(())
    }

    fn close(&mut self) -> Result<()> {
        if self.closed {
            return Ok(());
        }
        self.closed = true;

        let written = (|| -> Result<()> {
            let dir_start = self.out.file_pointer();
            let index_dir_start = self.index_out.file_pointer();

            self.out.write_v_int(self.fields.len() as i32)?;

            for field in &self.fields {
                self.out.write_v_int(field.field_info.number)?;
                debug_assert!(field.num_terms > 0);
                self.out.write_v_long(field.num_terms)?;
                let root_code = field.root_code.bytes();
                self.out.write_v_int(root_code.len() as i32)?;
                self.out.write_bytes(root_code, 0, root_code.len())?;
                if field.field_info.get_index_options() != IndexOptions::DOCS {
                    self.out.write_v_long(field.sum_total_term_freq)?;
                }
                self.out.write_v_long(field.sum_doc_freq)?;
                self.out.write_v_int(field.doc_count)?;
                self.index_out.write_v_long(field.index_start_fp)?;
                write_bytes_ref(self.out.as_mut(), &field.min_term)?;
                write_bytes_ref(self.out.as_mut(), &field.max_term)?;
            }
            self.out.write_long(dir_start)?;
            write_footer(self.out.as_mut())?;
            self.index_out.write_long(index_dir_start)?;
            write_footer(self.index_out.as_mut())
        })();
        let out = &mut self.out;
        let index_out = &mut self.index_out;
        let postings_writer = &mut self.postings_writer;
        let objects = [
            IOUtils::closeable(|| out.close()),
            IOUtils::closeable(|| index_out.close()),
            IOUtils::closeable(|| postings_writer.close()),
        ];
        match written {
            // if (success) IOUtils.close(out, indexOut, postingsWriter)
            Ok(()) => IOUtils::close(objects),
            // else IOUtils.closeWhileHandlingException(out, indexOut, postingsWriter)
            Err(e) => {
                IOUtils::close_while_handling_exception(objects)?;
                Err(e)
            }
        }
    }
}

/// Port of `OrdsBlockTreeTermsWriter.writeBytesRef(IndexOutput, BytesRef)`.
fn write_bytes_ref(out: &mut dyn IndexOutput, bytes: &[u8]) -> Result<()> {
    out.write_v_int(bytes.len() as i32)?;
    out.write_bytes(bytes, 0, bytes.len())
}

/// Port of the private static classes `PendingEntry`, `PendingTerm` and
/// `PendingBlock`: Java's `isTerm` flag and downcasts are the variants.
enum PendingEntry {
    Term(PendingTerm),
    Block(PendingBlock),
}

/// Port of `OrdsBlockTreeTermsWriter.PendingTerm`.
struct PendingTerm {
    term_bytes: Vec<u8>,
    // stats + metadata
    state: BlockTermState,
}

/// Port of the private record `OrdsBlockTreeTermsWriter.SubIndex`.
struct SubIndex {
    index: FST<FSTOrdsOutputs>,
    term_ord_start: i64,
}

/// Port of `OrdsBlockTreeTermsWriter.PendingBlock`.
struct PendingBlock {
    prefix: Vec<u8>,
    fp: i64,
    index: Option<FST<FSTOrdsOutputs>>,
    sub_indices: Option<Vec<SubIndex>>,
    has_terms: bool,
    is_floor: bool,
    floor_lead_byte: i32,
    tot_floor_term_count: i64,
    total_term_count: i64,
}

impl PendingBlock {
    fn new(
        prefix: Vec<u8>,
        fp: i64,
        has_terms: bool,
        total_term_count: i64,
        is_floor: bool,
        floor_lead_byte: i32,
        sub_indices: Option<Vec<SubIndex>>,
    ) -> Self {
        debug_assert!(total_term_count > 0);
        Self {
            prefix,
            fp,
            index: None,
            sub_indices,
            has_terms,
            is_floor,
            floor_lead_byte,
            tot_floor_term_count: 0,
            total_term_count,
        }
    }
}

/// Port of `PendingBlock.compileIndex(List<PendingBlock>,
/// ByteBuffersDataOutput, IntsRefBuilder)`, called on `blocks[0]`.
fn compile_index(
    blocks: &mut [PendingBlock],
    scratch_bytes: &mut ByteBuffersDataOutput,
    scratch_ints_ref: &mut IntsRefBuilder,
) -> Result<()> {
    let (first, rest) = blocks
        .split_first_mut()
        .expect("INVARIANT: writeBlocks compiles a non-empty list");
    debug_assert!(
        (first.is_floor && !rest.is_empty()) || (!first.is_floor && rest.is_empty()),
        "isFloor={} blocks={}",
        first.is_floor,
        rest.len() + 1
    );

    debug_assert_eq!(scratch_bytes.size(), 0);

    // TODO: try writing the leading vLong in MSB order
    // (opposite of what Lucene does today), for better
    // outputs sharing in the FST
    let mut last_sum_total_term_count = 0i64;
    let mut sum_total_term_count = first.total_term_count;
    scratch_bytes.write_v_long(encode_output(first.fp, first.has_terms, first.is_floor))?;
    if first.is_floor {
        scratch_bytes.write_v_int(rest.len() as i32)?;
        for sub in rest.iter() {
            debug_assert!(sub.floor_lead_byte != -1);
            scratch_bytes.write_byte(sub.floor_lead_byte as u8)?;
            scratch_bytes.write_v_long(sum_total_term_count - last_sum_total_term_count)?;
            last_sum_total_term_count = sum_total_term_count;
            sum_total_term_count += sub.total_term_count;
            debug_assert!(sub.fp > first.fp);
            scratch_bytes.write_v_long(((sub.fp - first.fp) << 1) | i64::from(sub.has_terms))?;
        }
    }

    let mut fst_compiler = Builder::new(InputType::Byte1, FST_OUTPUTS).build()?;
    let bytes = scratch_bytes.to_array_copy()?;
    debug_assert!(!bytes.is_empty());
    let prefix = BytesRef::new(first.prefix.clone());
    fst_compiler.add(
        &Util::to_ints_ref(&prefix, scratch_ints_ref),
        FST_OUTPUTS.new_output(bytes, 0, i64::MAX - (sum_total_term_count - 1)),
    )?;
    scratch_bytes.reset()?;

    // Copy over index for all sub-blocks

    let mut term_ord_offset = 0i64;
    for block in std::iter::once(&mut *first).chain(rest.iter_mut()) {
        if let Some(sub_indices) = block.sub_indices.take() {
            for sub_index in &sub_indices {
                append(
                    &mut fst_compiler,
                    &sub_index.index,
                    term_ord_offset + sub_index.term_ord_start,
                    scratch_ints_ref,
                )?;
            }
        }
        term_ord_offset += block.total_term_count;
    }
    first.tot_floor_term_count = term_ord_offset;

    debug_assert_eq!(sum_total_term_count, first.tot_floor_term_count);

    let metadata = fst_compiler.compile()?;
    first.index = FST::from_fst_reader(metadata, fst_compiler.into_fst_reader()?);
    debug_assert!(first.sub_indices.is_none());
    Ok(())
}

/// Port of `PendingBlock.append(FSTCompiler, FST, long, IntsRefBuilder)`.
// TODO: maybe we could add bulk-add method to
// Builder?  Takes FST and unions it w/ current
// FST.
fn append(
    fst_compiler: &mut rucene::util::fst::FSTCompiler<'static, FSTOrdsOutputs>,
    sub_index: &FST<FSTOrdsOutputs>,
    term_ord_offset: i64,
    scratch_ints_ref: &mut IntsRefBuilder,
) -> Result<()> {
    let mut sub_index_enum = BytesRefFSTEnum::new(sub_index)?;
    while let Some((input, output)) = sub_index_enum
        .next()?
        .map(|index_ent| (index_ent.input.clone(), index_ent.output.clone()))
    {
        // long blockTermCount = output.endOrd - output.startOrd + 1;
        let new_output = FST_OUTPUTS.new_output(
            output.bytes().to_vec(),
            term_ord_offset + output.start_ord(),
            output.end_ord() - term_ord_offset,
        );
        fst_compiler.add(&Util::to_ints_ref(&input, scratch_ints_ref), new_output)?;
    }
    Ok(())
}

/// One output written through two `DataOutput` handles, for
/// `root.index.save(indexOut, indexOut)`: the FST's metadata and then its
/// bytes go to the same file, in that order.
struct AliasedOutput<'c, 'o>(&'c RefCell<&'o mut (dyn IndexOutput + 'static)>);

impl DataOutput for AliasedOutput<'_, '_> {
    fn write_byte(&mut self, b: u8) -> Result<()> {
        self.0.borrow_mut().write_byte(b)
    }

    fn write_bytes(&mut self, b: &[u8], offset: usize, len: usize) -> Result<()> {
        self.0.borrow_mut().write_bytes(b, offset, len)
    }
}

/// Port of the inner class `OrdsBlockTreeTermsWriter.TermsWriter`; the outer
/// writer is the `w` its methods receive.
struct TermsWriter {
    field_info: FieldInfo,
    num_terms: i64,
    docs_seen: FixedBitSet,
    sum_total_term_freq: i64,
    sum_doc_freq: i64,
    index_start_fp: i64,

    // Records index into pending where the current prefix at that
    // length "started"; for example, if current term starts with 't',
    // startsByPrefix[0] is the index into pending for the first
    // term/sub-block starting with 't'.  We use this to figure out when
    // to write a new block:
    last_term: Vec<u8>,
    prefix_starts: Vec<i32>,

    // Pending stack of terms and blocks.  As terms arrive (in sorted order)
    // we append to this stack, and once the top of the stack has enough
    // terms starting with a common prefix, we write a new block with
    // those terms and replace those terms in the stack with a new block:
    pending: Vec<PendingEntry>,

    // Reused in writeBlocks:
    new_blocks: Vec<PendingBlock>,

    first_pending_term: Option<Vec<u8>>,
    last_pending_term: Option<Vec<u8>>,

    suffix_writer: ByteBuffersDataOutput,
    stats_writer: ByteBuffersDataOutput,
    meta_writer: ByteBuffersDataOutput,
}

impl TermsWriter {
    /// Port of `TermsWriter(FieldInfo)`.
    fn new(w: &mut OrdsBlockTreeTermsWriter, field_info: FieldInfo) -> Result<Self> {
        let docs_seen = FixedBitSet::new(w.max_doc as usize);
        w.postings_writer.set_field(&field_info)?;
        Ok(Self {
            field_info,
            num_terms: 0,
            docs_seen,
            sum_total_term_freq: 0,
            sum_doc_freq: 0,
            index_start_fp: 0,
            last_term: Vec::new(),
            prefix_starts: vec![0; 8],
            pending: Vec::new(),
            new_blocks: Vec::new(),
            first_pending_term: None,
            last_pending_term: None,
            suffix_writer: ByteBuffersDataOutput::new_resettable_instance(),
            stats_writer: ByteBuffersDataOutput::new_resettable_instance(),
            meta_writer: ByteBuffersDataOutput::new_resettable_instance(),
        })
    }

    /// Writes the top count entries in pending, using prevTerm to compute
    /// the prefix.
    fn write_blocks(
        &mut self,
        w: &mut OrdsBlockTreeTermsWriter,
        prefix_length: usize,
        count: usize,
    ) -> Result<()> {
        debug_assert!(count > 0);

        // Root block better write all remaining pending entries:
        debug_assert!(prefix_length > 0 || count == self.pending.len());

        let mut last_suffix_lead_label = -1i32;

        // True if we saw at least one term in this block (we record if a block
        // only points to sub-blocks in the terms index so we can avoid seeking
        // to it when we are looking for a term):
        let mut has_terms = false;
        let mut has_sub_blocks = false;

        let start = self.pending.len() - count;
        let end = self.pending.len();
        let mut next_block_start = start;
        let mut next_floor_lead_label = -1i32;

        for i in start..end {
            let suffix_lead_label = match &self.pending[i] {
                PendingEntry::Term(term) => {
                    if term.term_bytes.len() == prefix_length {
                        // Suffix is 0, i.e. prefix 'foo' and term is
                        // 'foo' so the term has empty string suffix
                        // in this block
                        debug_assert_eq!(last_suffix_lead_label, -1);
                        -1
                    } else {
                        i32::from(term.term_bytes[prefix_length])
                    }
                }
                PendingEntry::Block(block) => {
                    debug_assert!(block.prefix.len() > prefix_length);
                    i32::from(block.prefix[prefix_length])
                }
            };

            if suffix_lead_label != last_suffix_lead_label {
                let items_in_block = i - next_block_start;
                if items_in_block >= w.min_items_in_block as usize
                    && end - next_block_start > w.max_items_in_block as usize
                {
                    // The count is too large for one block, so we must break it into "floor"
                    // blocks, where we record the leading label of the suffix of the first term
                    // in each floor block, so at search time we can jump to the right floor
                    // block.  We just use a naive greedy segmenter here: make a new floor block
                    // as soon as we have at least minItemsInBlock.  This is not always best: it
                    // often produces a too-small block as the final block:
                    let is_floor = items_in_block < count;
                    let block = self.write_block(
                        w,
                        prefix_length,
                        is_floor,
                        next_floor_lead_label,
                        next_block_start,
                        i,
                        has_terms,
                        has_sub_blocks,
                    )?;
                    self.new_blocks.push(block);

                    has_terms = false;
                    has_sub_blocks = false;
                    next_floor_lead_label = suffix_lead_label;
                    next_block_start = i;
                }

                last_suffix_lead_label = suffix_lead_label;
            }

            match &self.pending[i] {
                PendingEntry::Term(_) => has_terms = true,
                PendingEntry::Block(_) => has_sub_blocks = true,
            }
        }

        // Write last block, if any:
        if next_block_start < end {
            let items_in_block = end - next_block_start;
            let is_floor = items_in_block < count;
            let block = self.write_block(
                w,
                prefix_length,
                is_floor,
                next_floor_lead_label,
                next_block_start,
                end,
                has_terms,
                has_sub_blocks,
            )?;
            self.new_blocks.push(block);
        }

        debug_assert!(!self.new_blocks.is_empty());

        debug_assert!(self.new_blocks[0].is_floor || self.new_blocks.len() == 1);

        compile_index(
            &mut self.new_blocks,
            &mut w.scratch_bytes,
            &mut w.scratch_ints_ref,
        )?;

        // Remove slice from the top of the pending stack, that we just wrote:
        let new_len = self.pending.len() - count;
        self.pending.truncate(new_len);

        // Append new block
        let first_block = self.new_blocks.swap_remove(0);
        self.pending.push(PendingEntry::Block(first_block));

        self.new_blocks.clear();
        Ok(())
    }

    /// Writes the specified slice (start is inclusive, end is exclusive)
    /// from pending stack as a new block. If isFloor is true, there were too
    /// many (more than maxItemsInBlock) entries sharing the same prefix, and
    /// so we broke it into multiple floor blocks where we record the
    /// starting label of the suffix of each floor block.
    #[allow(clippy::too_many_arguments)]
    fn write_block(
        &mut self,
        w: &mut OrdsBlockTreeTermsWriter,
        prefix_length: usize,
        is_floor: bool,
        floor_lead_label: i32,
        start: usize,
        end: usize,
        has_terms: bool,
        has_sub_blocks: bool,
    ) -> Result<PendingBlock> {
        debug_assert!(end > start);

        let start_fp = w.out.file_pointer();

        let has_floor_lead_label = is_floor && floor_lead_label != -1;

        let mut prefix = Vec::with_capacity(prefix_length + usize::from(has_floor_lead_label));
        prefix.extend_from_slice(&self.last_term[..prefix_length]);

        // Write block header:
        let num_entries = (end - start) as i32;
        let mut code = num_entries << 1;
        if end == self.pending.len() {
            // Last block:
            code |= 1;
        }
        w.out.write_v_int(code)?;

        let sub_indices;

        // We optimize the leaf block case (block has only terms), writing a more
        // compact format in this case:
        let is_leaf_block = !has_sub_blocks;

        // Number of terms in this block and all sub-blocks (recursively)
        let mut total_term_count: i64;

        let mut absolute = true;

        let index_options = self.field_info.get_index_options();
        if is_leaf_block {
            // Only terms:
            sub_indices = None;
            for i in start..end {
                let PendingEntry::Term(term) = &self.pending[i] else {
                    unreachable!("INVARIANT: a leaf block holds only terms (i={i})");
                };
                debug_assert!(term.term_bytes.starts_with(&prefix));
                let state = &term.state;
                let suffix = term.term_bytes.len() - prefix_length;
                // For leaf block we write suffix straight
                self.suffix_writer.write_v_int(suffix as i32)?;
                self.suffix_writer
                    .write_bytes(&term.term_bytes, prefix_length, suffix)?;
                debug_assert!(
                    floor_lead_label == -1
                        || i32::from(term.term_bytes[prefix_length]) >= floor_lead_label
                );

                // Write term stats, to separate byte[] blob:
                self.stats_writer.write_v_int(state.doc_freq)?;
                if index_options != IndexOptions::DOCS {
                    debug_assert!(
                        state.total_term_freq >= i64::from(state.doc_freq),
                        "{} vs {}",
                        state.total_term_freq,
                        state.doc_freq
                    );
                    self.stats_writer
                        .write_v_long(state.total_term_freq - i64::from(state.doc_freq))?;
                }

                // Write term meta data
                w.postings_writer.encode_term(
                    &mut self.meta_writer,
                    &self.field_info,
                    state,
                    absolute,
                )?;
                absolute = false;
            }
            total_term_count = (end - start) as i64;
        } else {
            // Mixed terms and sub-blocks:
            let mut subs = Vec::new();
            total_term_count = 0;
            for i in start..end {
                match &mut self.pending[i] {
                    PendingEntry::Term(term) => {
                        debug_assert!(term.term_bytes.starts_with(&prefix));
                        let state = &term.state;
                        let suffix = term.term_bytes.len() - prefix_length;
                        // For non-leaf block we borrow 1 bit to record
                        // if entry is term or sub-block
                        self.suffix_writer.write_v_int((suffix as i32) << 1)?;
                        self.suffix_writer
                            .write_bytes(&term.term_bytes, prefix_length, suffix)?;
                        debug_assert!(
                            floor_lead_label == -1
                                || i32::from(term.term_bytes[prefix_length]) >= floor_lead_label
                        );

                        // Write term stats, to separate byte[] blob:
                        self.stats_writer.write_v_int(state.doc_freq)?;
                        if index_options != IndexOptions::DOCS {
                            debug_assert!(state.total_term_freq >= i64::from(state.doc_freq));
                            self.stats_writer
                                .write_v_long(state.total_term_freq - i64::from(state.doc_freq))?;
                        }

                        // TODO: now that terms dict "sees" these longs,
                        // we can explore better column-stride encodings
                        // to encode all long[0]s for this block at
                        // once, all long[1]s, etc., e.g. using
                        // Simple64.  Alternatively, we could interleave
                        // stats + meta ... no reason to have them
                        // separate anymore:

                        // Write term meta data
                        w.postings_writer.encode_term(
                            &mut self.meta_writer,
                            &self.field_info,
                            state,
                            absolute,
                        )?;
                        absolute = false;

                        total_term_count += 1;
                    }
                    PendingEntry::Block(block) => {
                        debug_assert!(block.prefix.starts_with(&prefix));
                        let suffix = block.prefix.len() - prefix_length;

                        debug_assert!(suffix > 0);

                        // For non-leaf block we borrow 1 bit to record
                        // if entry is term or sub-block
                        self.suffix_writer.write_v_int(((suffix as i32) << 1) | 1)?;
                        self.suffix_writer
                            .write_bytes(&block.prefix, prefix_length, suffix)?;

                        debug_assert!(
                            floor_lead_label == -1
                                || i32::from(block.prefix[prefix_length]) >= floor_lead_label
                        );

                        debug_assert!(block.fp < start_fp);

                        self.suffix_writer.write_v_long(start_fp - block.fp)?;
                        self.suffix_writer
                            .write_v_long(block.tot_floor_term_count)?;
                        // `new SubIndex(block.index, totalTermCount)`: the
                        // block leaves the pending stack right after this
                        // block is written, so its index moves here.
                        let index = block
                            .index
                            .take()
                            .expect("INVARIANT: a pending block's index is compiled");
                        subs.push(SubIndex {
                            index,
                            term_ord_start: total_term_count,
                        });
                        total_term_count += block.tot_floor_term_count;
                    }
                }
            }

            debug_assert!(!subs.is_empty());
            sub_indices = Some(subs);
        }

        // TODO: we could block-write the term suffix pointers;
        // this would take more space but would enable binary
        // search on lookup

        // Write suffixes byte[] blob to terms dict output:
        w.out
            .write_v_int(((self.suffix_writer.size() << 1) as i32) | i32::from(is_leaf_block))?;
        self.suffix_writer.copy_to(w.out.as_mut())?;
        self.suffix_writer.reset()?;

        // Write term stats byte[] blob
        w.out.write_v_int(self.stats_writer.size() as i32)?;
        self.stats_writer.copy_to(w.out.as_mut())?;
        self.stats_writer.reset()?;

        // Write term meta data byte[] blob
        w.out.write_v_int(self.meta_writer.size() as i32)?;
        self.meta_writer.copy_to(w.out.as_mut())?;
        self.meta_writer.reset()?;

        if has_floor_lead_label {
            // We already allocated to length+1 above:
            prefix.push(floor_lead_label as u8);
        }

        Ok(PendingBlock::new(
            prefix,
            start_fp,
            has_terms,
            total_term_count,
            is_floor,
            floor_lead_label,
            sub_indices,
        ))
    }

    /// Writes one term's worth of postings.
    fn write(
        &mut self,
        w: &mut OrdsBlockTreeTermsWriter,
        text: &BytesRef,
        terms_enum: &mut dyn TermsEnum,
        norms: Option<&dyn NormsProducer>,
    ) -> Result<()> {
        let state = w
            .postings_writer
            .write_term(text, terms_enum, &mut self.docs_seen, norms)?;
        if let Some(state) = state {
            debug_assert!(state.doc_freq != 0);
            debug_assert!(
                self.field_info.get_index_options() == IndexOptions::DOCS
                    || state.total_term_freq >= i64::from(state.doc_freq)
            );
            self.sum_doc_freq += i64::from(state.doc_freq);
            self.sum_total_term_freq += state.total_term_freq;
            self.push_term(w, text.slice())?;

            let term_bytes = text.slice().to_vec();
            if self.first_pending_term.is_none() {
                self.first_pending_term = Some(term_bytes.clone());
            }
            self.last_pending_term = Some(term_bytes.clone());
            self.pending
                .push(PendingEntry::Term(PendingTerm { term_bytes, state }));
            self.num_terms += 1;
        }
        Ok(())
    }

    /// Pushes the new term to the top of the stack, and writes new blocks.
    fn push_term(&mut self, w: &mut OrdsBlockTreeTermsWriter, text: &[u8]) -> Result<()> {
        let limit = self.last_term.len().min(text.len());

        // Find common prefix between last term and current term:
        let mut pos = 0;
        while pos < limit && self.last_term[pos] == text[pos] {
            pos += 1;
        }

        // Close the "abandoned" suffix now:
        let mut i = self.last_term.len();
        while i > pos {
            i -= 1;
            // How many items on top of the stack share the current suffix
            // we are closing:
            let prefix_top_size = self.pending.len() as i32 - self.prefix_starts[i];
            if prefix_top_size >= w.min_items_in_block {
                self.write_blocks(w, i + 1, prefix_top_size as usize)?;
                self.prefix_starts[i] -= prefix_top_size - 1;
            }
        }

        if self.prefix_starts.len() < text.len() {
            self.prefix_starts.resize(text.len(), 0);
        }

        // Init new tail:
        for i in pos..text.len() {
            self.prefix_starts[i] = self.pending.len() as i32;
        }

        self.last_term.clear();
        self.last_term.extend_from_slice(text);
        Ok(())
    }

    /// Finishes all terms in this field.
    fn finish(&mut self, w: &mut OrdsBlockTreeTermsWriter) -> Result<()> {
        if self.num_terms > 0 {
            // TODO: if pending.size() is already 1 with a non-zero prefix length
            // we can save writing a "degenerate" root block, but we have to
            // fix all the places that assume the root block's prefix is the empty string:
            let count = self.pending.len();
            self.write_blocks(w, 0, count)?;

            // We better have one final "root" block:
            debug_assert!(
                self.pending.len() == 1 && matches!(self.pending[0], PendingEntry::Block(_)),
                "pending.size()={}",
                self.pending.len()
            );
            let PendingEntry::Block(root) = &self.pending[0] else {
                unreachable!("INVARIANT: writeBlocks(0, ...) leaves the root block");
            };
            debug_assert!(root.prefix.is_empty());
            let index = root
                .index
                .as_ref()
                .expect("INVARIANT: the root block's index is compiled");
            debug_assert!(index.empty_output().is_some());

            // Write FST to index
            self.index_start_fp = w.index_out.file_pointer();
            {
                let cell = RefCell::new(w.index_out.as_mut());
                index.save(&mut AliasedOutput(&cell), &mut AliasedOutput(&cell))?;
            }

            let min_term = self
                .first_pending_term
                .clone()
                .expect("INVARIANT: numTerms > 0");
            let max_term = self
                .last_pending_term
                .clone()
                .expect("INVARIANT: numTerms > 0");

            let root_code = index
                .empty_output()
                .cloned()
                .expect("INVARIANT: the root block's index accepts the empty string");
            debug_assert!(self.num_terms > 0);
            w.fields.push(FieldMetaData {
                field_info: self.field_info.clone(),
                root_code,
                num_terms: self.num_terms,
                index_start_fp: self.index_start_fp,
                sum_total_term_freq: self.sum_total_term_freq,
                sum_doc_freq: self.sum_doc_freq,
                doc_count: self.docs_seen.cardinality() as i32,
                min_term,
                max_term,
            });
        } else {
            debug_assert_eq!(self.docs_seen.cardinality(), 0);
        }
        Ok(())
    }
}
