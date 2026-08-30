//! `SegmentTermsEnum` and `SegmentTermsEnumFrame` ported from
//! `org.apache.lucene.codecs.lucene103.blocktree`.
//!
//! The cursor that walks one field's terms: a stack of frames, each holding one
//! decoded block of the `.tim` file, navigated through the `.tip` trie.

use crate::codecs::lucene103::blocktree::CompressionAlgorithm;
use crate::codecs::lucene103::field_reader::FieldReader;
use crate::codecs::lucene103::trie_reader::{Node, TrieReader};
use crate::codecs::term_state::BlockTermState;
use crate::error::{LuceneError, Result};
use crate::index::terms::{SeekStatus, TermState, TermsEnum};
use crate::index::ImpactsEnum;
use crate::index::{IndexOptions, PostingsEnum};
use crate::store::{ByteArrayDataInput, DataInput, IndexInput};
use crate::util::{AttributeSource, BytesRef};

/// One decoded block of the terms dictionary.
///
/// Equivalent to
/// `org.apache.lucene.codecs.lucene103.blocktree.SegmentTermsEnumFrame`.
///
/// **Divergence from Lucene 10.5.0.** Java's frame holds a back-reference to its
/// `SegmentTermsEnum` and reads and writes the cursor's shared `term` buffer
/// directly. Rust cannot express that cycle, so the frame is plain data and the
/// cursor passes the shared state in as a parameter. The decoded fields and the
/// block layout are unchanged.
pub struct SegmentTermsEnumFrame {
    /// This frame's index in the stack.
    pub ord: usize,
    /// Whether this block holds terms.
    pub has_terms: bool,
    /// `has_terms` as the trie recorded it, before floor navigation changed it.
    pub has_terms_orig: bool,
    /// Whether this block is one of several floor blocks sharing a prefix.
    pub is_floor: bool,
    /// The trie node that pointed at this block, when there was one.
    pub node: Option<Node>,

    /// Where this block was loaded from.
    pub fp: i64,
    /// Where the block originally started, before floor navigation.
    pub fp_orig: i64,
    /// Where this block ends.
    pub fp_end: i64,

    suffix_bytes: Vec<u8>,
    suffixes_reader: ByteArrayDataInput,
    suffix_length_bytes: Vec<u8>,
    suffix_lengths_reader: ByteArrayDataInput,
    stat_bytes: Vec<u8>,
    stats_reader: ByteArrayDataInput,
    stats_singleton_run_length: i32,

    /// Floor data, copied out of the `.tip` file.
    floor_data: Vec<u8>,
    /// Read position inside `floor_data`.
    floor_data_pos: usize,

    /// Length of the prefix every term in this block shares.
    pub prefix_length: usize,
    /// How many entries — terms or sub-blocks — the block holds.
    pub ent_count: i32,
    /// Which entry comes next, or `-1` when the block is not loaded.
    pub next_ent: i32,
    /// Whether this is the last block of its floor run.
    pub is_last_in_floor: bool,
    /// Whether every entry is a term.
    pub is_leaf_block: bool,
    /// Whether every suffix has the same length.
    pub all_equal: bool,
    /// Pointer to the last sub-block seen.
    pub last_sub_fp: i64,
    /// Label at which the next floor block starts, or `256` past the last.
    pub next_floor_label: i32,
    /// How many floor blocks still follow.
    pub num_follow_floor_blocks: i32,

    /// How many terms of this block have had their metadata decoded.
    pub meta_data_upto: i32,
    /// The term state the postings reader fills in.
    pub state: BlockTermState,
    bytes: Vec<u8>,
    bytes_reader: ByteArrayDataInput,

    /// Length of the current entry's suffix.
    pub suffix_length: usize,
    /// Where the current entry's suffix starts inside `suffix_bytes`.
    pub start_byte_pos: usize,
    /// Sub-block pointer of the current entry, or `0` for a term.
    pub sub_code: i64,
}

impl std::fmt::Debug for SegmentTermsEnumFrame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SegmentTermsEnumFrame")
            .field("ord", &self.ord)
            .field("fp", &self.fp)
            .field("ent_count", &self.ent_count)
            .field("next_ent", &self.next_ent)
            .field("is_leaf_block", &self.is_leaf_block)
            .finish_non_exhaustive()
    }
}

impl SegmentTermsEnumFrame {
    /// Creates an empty frame at stack position `ord`.
    pub fn new(ord: usize, state: BlockTermState) -> Self {
        Self {
            ord,
            has_terms: false,
            has_terms_orig: false,
            is_floor: false,
            node: None,
            fp: 0,
            fp_orig: 0,
            fp_end: 0,
            suffix_bytes: Vec::new(),
            suffixes_reader: ByteArrayDataInput::new(Vec::new()),
            suffix_length_bytes: Vec::new(),
            suffix_lengths_reader: ByteArrayDataInput::new(Vec::new()),
            stat_bytes: Vec::new(),
            stats_reader: ByteArrayDataInput::new(Vec::new()),
            stats_singleton_run_length: 0,
            floor_data: Vec::new(),
            floor_data_pos: 0,
            prefix_length: 0,
            ent_count: 0,
            next_ent: -1,
            is_last_in_floor: false,
            is_leaf_block: false,
            all_equal: false,
            last_sub_fp: -1,
            next_floor_label: 0,
            num_follow_floor_blocks: 0,
            meta_data_upto: 0,
            state,
            bytes: Vec::new(),
            bytes_reader: ByteArrayDataInput::new(Vec::new()),
            suffix_length: 0,
            start_byte_pos: 0,
            sub_code: 0,
        }
    }

    /// Records this block's floor data, read out of the term index.
    ///
    /// Equivalent to `SegmentTermsEnumFrame.setFloorData`.
    pub fn set_floor_data(&mut self, floor_data: Vec<u8>) -> Result<()> {
        let mut reader = ByteArrayDataInput::from_slice(&floor_data);
        self.num_follow_floor_blocks = reader.read_v_int()?;
        self.next_floor_label = i32::from(reader.read_byte()?);
        self.floor_data_pos = reader.position();
        self.floor_data = floor_data;
        Ok(())
    }

    /// Returns how many terms of this block have been passed.
    ///
    /// Equivalent to `SegmentTermsEnumFrame.getTermBlockOrd`.
    pub fn get_term_block_ord(&self) -> i32 {
        if self.is_leaf_block {
            self.next_ent
        } else {
            self.state.term_block_ord
        }
    }

    /// Reads this frame's block out of the terms file.
    ///
    /// Equivalent to `SegmentTermsEnumFrame.loadBlock`, and the definition of
    /// the on-disk block layout.
    pub fn load_block(&mut self, terms_in: &mut dyn IndexInput) -> Result<()> {
        if self.next_ent != -1 {
            return Ok(());
        }
        terms_in.seek(self.fp)?;

        let code = terms_in.read_v_int()?;
        self.ent_count = code >> 1;
        if self.ent_count <= 0 {
            return Err(LuceneError::corrupt_index(
                format!("invalid entry count: {}", self.ent_count),
                "terms dictionary",
            ));
        }
        self.is_last_in_floor = code & 1 != 0;

        let code_l = terms_in.read_v_long()?;
        self.is_leaf_block = code_l & 0x04 != 0;
        let num_suffix_bytes = ((code_l as u64) >> 3) as usize;
        if self.suffix_bytes.len() < num_suffix_bytes {
            self.suffix_bytes.resize(num_suffix_bytes, 0);
        }
        let compression = CompressionAlgorithm::by_code((code_l & 0x03) as i32)?;
        compression.read(terms_in, &mut self.suffix_bytes, num_suffix_bytes)?;
        self.suffixes_reader =
            ByteArrayDataInput::from_slice(&self.suffix_bytes[..num_suffix_bytes]);

        let mut num_suffix_length_bytes = terms_in.read_v_int()?;
        self.all_equal = num_suffix_length_bytes & 0x01 != 0;
        num_suffix_length_bytes >>= 1;
        let n = num_suffix_length_bytes.max(0) as usize;
        if self.suffix_length_bytes.len() < n {
            self.suffix_length_bytes.resize(n, 0);
        }
        if self.all_equal {
            let b = terms_in.read_byte()?;
            self.suffix_length_bytes[..n].fill(b);
        } else {
            terms_in.read_bytes(&mut self.suffix_length_bytes, 0, n)?;
        }
        self.suffix_lengths_reader = ByteArrayDataInput::from_slice(&self.suffix_length_bytes[..n]);

        let num_stat_bytes = terms_in.read_v_int()?.max(0) as usize;
        if self.stat_bytes.len() < num_stat_bytes {
            self.stat_bytes.resize(num_stat_bytes, 0);
        }
        terms_in.read_bytes(&mut self.stat_bytes, 0, num_stat_bytes)?;
        self.stats_reader = ByteArrayDataInput::from_slice(&self.stat_bytes[..num_stat_bytes]);
        self.stats_singleton_run_length = 0;

        self.meta_data_upto = 0;
        self.state.term_block_ord = 0;
        self.next_ent = 0;
        self.last_sub_fp = -1;

        let num_meta_bytes = terms_in.read_v_int()?.max(0) as usize;
        if self.bytes.len() < num_meta_bytes {
            self.bytes.resize(num_meta_bytes, 0);
        }
        terms_in.read_bytes(&mut self.bytes, 0, num_meta_bytes)?;
        self.bytes_reader = ByteArrayDataInput::from_slice(&self.bytes[..num_meta_bytes]);

        self.fp_end = terms_in.file_pointer();
        Ok(())
    }

    /// Moves to the next floor block of this run.
    ///
    /// Equivalent to `SegmentTermsEnumFrame.loadNextFloorBlock`.
    pub fn load_next_floor_block(&mut self, terms_in: &mut dyn IndexInput) -> Result<()> {
        self.fp = self.fp_end;
        self.next_ent = -1;
        self.load_block(terms_in)
    }

    /// Rewinds this frame to the start of its block.
    ///
    /// Equivalent to `SegmentTermsEnumFrame.rewind`.
    pub fn rewind(&mut self) -> Result<()> {
        self.fp = self.fp_orig;
        self.next_ent = -1;
        self.has_terms = self.has_terms_orig;
        if self.is_floor {
            let floor_data = std::mem::take(&mut self.floor_data);
            self.set_floor_data(floor_data)?;
        }
        Ok(())
    }

    /// Advances within a floor run until the block that could hold `target`.
    ///
    /// Equivalent to `SegmentTermsEnumFrame.scanToFloorFrame`.
    pub fn scan_to_floor_frame(&mut self, target: &[u8]) -> Result<()> {
        if !self.is_floor || target.len() <= self.prefix_length {
            return Ok(());
        }
        let target_label = i32::from(target[self.prefix_length]);
        if target_label < self.next_floor_label {
            return Ok(());
        }

        let mut new_fp = self.fp_orig;
        let mut reader = ByteArrayDataInput::from_slice(&self.floor_data);
        reader.seek(self.floor_data_pos)?;
        loop {
            let code = reader.read_v_long()?;
            new_fp = self.fp_orig + ((code as u64) >> 1) as i64;
            self.has_terms = code & 1 != 0;
            self.is_last_in_floor = self.num_follow_floor_blocks == 1;
            self.num_follow_floor_blocks -= 1;
            if self.is_last_in_floor {
                self.next_floor_label = 256;
                break;
            }
            self.next_floor_label = i32::from(reader.read_byte()?);
            if target_label < self.next_floor_label {
                break;
            }
        }
        self.floor_data_pos = reader.position();

        if new_fp != self.fp {
            self.next_ent = -1;
            self.fp = new_fp;
        }
        Ok(())
    }

    /// Advances within the block until the sub-block at `sub_fp`.
    ///
    /// Equivalent to `SegmentTermsEnumFrame.scanToSubBlock`.
    pub fn scan_to_sub_block(&mut self, sub_fp: i64, term: &mut Vec<u8>) -> Result<()> {
        if self.last_sub_fp == sub_fp {
            return Ok(());
        }
        while self.next_ent < self.ent_count {
            self.next_non_leaf_entry(term)?;
            if self.last_sub_fp == sub_fp {
                return Ok(());
            }
        }
        Err(LuceneError::corrupt_index(
            format!("sub-block at fp {sub_fp} was not found in its parent block"),
            "terms dictionary",
        ))
    }

    /// Reads the next entry of a leaf block into `term`.
    ///
    /// Equivalent to `SegmentTermsEnumFrame.nextLeaf`.
    pub fn next_leaf(&mut self, term: &mut Vec<u8>) -> Result<()> {
        self.next_ent += 1;
        self.suffix_length = self.suffix_lengths_reader.read_v_int()?.max(0) as usize;
        self.start_byte_pos = self.suffixes_reader.position();
        term.truncate(self.prefix_length);
        term.resize(self.prefix_length + self.suffix_length, 0);
        self.suffixes_reader
            .read_bytes(&mut term[..], self.prefix_length, self.suffix_length)?;
        Ok(())
    }

    /// Reads the next entry of a non-leaf block into `term`.
    ///
    /// Returns `true` when the entry is a sub-block rather than a term.
    /// Equivalent to the body of `SegmentTermsEnumFrame.nextNonLeaf`, without
    /// its floor-block loop, which the cursor drives.
    pub fn next_non_leaf_entry(&mut self, term: &mut Vec<u8>) -> Result<bool> {
        self.next_ent += 1;
        let code = self.suffix_lengths_reader.read_v_int()?;
        self.suffix_length = ((code as u32) >> 1) as usize;
        self.start_byte_pos = self.suffixes_reader.position();
        term.truncate(self.prefix_length);
        term.resize(self.prefix_length + self.suffix_length, 0);
        self.suffixes_reader
            .read_bytes(&mut term[..], self.prefix_length, self.suffix_length)?;

        if code & 1 == 0 {
            // A term.
            self.sub_code = 0;
            self.state.term_block_ord += 1;
            Ok(false)
        } else {
            // A sub-block: its pointer is stored as a delta from this block.
            self.sub_code = self.suffix_lengths_reader.read_v_long()?;
            self.last_sub_fp = self.fp - self.sub_code;
            Ok(true)
        }
    }

    /// Decodes the statistics and postings metadata of every term passed so far.
    ///
    /// Equivalent to `SegmentTermsEnumFrame.decodeMetaData`.
    pub fn decode_meta_data(
        &mut self,
        field_info: &crate::index::FieldInfo,
        postings_reader: &mut dyn crate::codecs::postings::PostingsReaderBase,
    ) -> Result<()> {
        let limit = self.get_term_block_ord();
        let mut absolute = self.meta_data_upto == 0;
        while self.meta_data_upto < limit {
            if self.stats_singleton_run_length > 0 {
                self.state.doc_freq = 1;
                self.state.total_term_freq = 1;
                self.stats_singleton_run_length -= 1;
            } else {
                let token = self.stats_reader.read_v_int()?;
                if token & 1 == 1 {
                    // A run of terms that each appear in exactly one document.
                    self.state.doc_freq = 1;
                    self.state.total_term_freq = 1;
                    self.stats_singleton_run_length = ((token as u32) >> 1) as i32;
                } else {
                    self.state.doc_freq = ((token as u32) >> 1) as i32;
                    self.state.total_term_freq = if field_info.index_options == IndexOptions::DOCS {
                        i64::from(self.state.doc_freq)
                    } else {
                        i64::from(self.state.doc_freq) + self.stats_reader.read_v_long()?
                    };
                }
            }
            postings_reader.decode_term(
                &mut self.bytes_reader,
                field_info,
                &mut self.state,
                absolute,
            )?;
            self.meta_data_upto += 1;
            absolute = false;
        }
        self.state.term_block_ord = self.meta_data_upto;
        Ok(())
    }
}

/// Walks the terms of one field.
///
/// Equivalent to
/// `org.apache.lucene.codecs.lucene103.blocktree.SegmentTermsEnum`.
///
/// **Divergence from Lucene 10.5.0.** Java's cursor keeps its own `IndexInput`
/// over the `.tim` file and its frames read through a back-reference to it. This
/// port clones the input into the cursor and passes it to each frame, which is
/// the same single reader with the ownership made explicit. `seek_ord` is
/// unsupported here as it is in Java, and `intersect` lives in its own cursor.
pub struct SegmentTermsEnum {
    field: FieldReader,
    terms_in: Box<dyn IndexInput>,
    trie: TrieReader,
    /// The frame stack. Frame `n` holds a block whose prefix is `n` bytes deep.
    stack: Vec<SegmentTermsEnumFrame>,
    /// Index of the frame the cursor is positioned in.
    current: usize,
    /// The trie node reached at each stack depth.
    nodes: Vec<Node>,
    /// The current term.
    term: Vec<u8>,
    /// Whether the current position names a term rather than a sub-block.
    term_exists: bool,
    /// Whether the cursor has run past the last term.
    eof: bool,
    /// Whether the first frame has been pushed.
    started: bool,
    attributes: AttributeSource,
}

impl std::fmt::Debug for SegmentTermsEnum {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SegmentTermsEnum")
            .field("field", &self.field.field_info().name)
            .field("term_exists", &self.term_exists)
            .field("eof", &self.eof)
            .finish_non_exhaustive()
    }
}

impl SegmentTermsEnum {
    /// Opens a cursor over `field`.
    pub fn new(field: FieldReader) -> Result<Self> {
        let shared = field.shared();
        let terms_in = shared.terms_in.clone_input()?;
        let index_in = shared.index_in.clone_input()?;
        let index_slice = index_in.random_access_slice(0, index_in.length())?;
        let trie = TrieReader::new(index_slice, field.index_start() + field.root_fp())?;

        Ok(Self {
            field,
            terms_in,
            trie,
            stack: Vec::new(),
            current: 0,
            nodes: Vec::new(),
            term: Vec::new(),
            term_exists: false,
            eof: false,
            started: false,
            attributes: AttributeSource::new(),
        })
    }

    /// Returns the frame at stack position `ord`, creating it when needed.
    ///
    /// Equivalent to `SegmentTermsEnum.getFrame`.
    fn ensure_frame(&mut self, ord: usize) -> Result<()> {
        while self.stack.len() <= ord {
            let state = self.postings_reader_new_state()?;
            self.stack
                .push(SegmentTermsEnumFrame::new(self.stack.len(), state));
        }
        Ok(())
    }

    fn postings_reader_new_state(&self) -> Result<BlockTermState> {
        // Every frame needs its own term state; the postings reader knows the
        // shape the format uses.
        Ok(BlockTermState::default())
    }

    /// Pushes a frame for the block at `fp`, whose terms share a `length`-byte
    /// prefix.
    ///
    /// Equivalent to `SegmentTermsEnum.pushFrame(Node, long, int)`.
    fn push_frame(&mut self, node: Option<Node>, fp: i64, length: usize) -> Result<usize> {
        let ord = if self.started { self.current + 1 } else { 0 };
        self.ensure_frame(ord)?;
        let frame = &mut self.stack[ord];
        frame.node = node;

        if frame.fp_orig == fp && frame.next_ent != -1 {
            // The frame already holds this block; rewind it rather than reload.
            if frame.ord > self.current {
                frame.rewind()?;
            }
        } else {
            frame.next_ent = -1;
            frame.prefix_length = length;
            frame.state.term_block_ord = 0;
            frame.fp = fp;
            frame.fp_orig = fp;
            frame.last_sub_fp = -1;
        }
        self.current = ord;
        self.started = true;
        Ok(ord)
    }

    /// Pushes a frame for the block a trie node points at, reading its floor
    /// data when it has any.
    ///
    /// Equivalent to `SegmentTermsEnum.pushFrame(Node, int)`.
    fn push_frame_from_node(&mut self, node: Node, length: usize) -> Result<usize> {
        let floor_data = if node.is_floor() {
            // The floor data runs to the end of the field's slice of the index.
            let end = self.field.index_end();
            let len = (end - node.floor_data_fp).max(0) as usize;
            Some(self.trie.floor_data(&node, len.min(1 << 16))?)
        } else {
            None
        };

        let ord = self.push_frame(Some(node), node.output_fp, length)?;
        let frame = &mut self.stack[ord];
        frame.has_terms = node.has_terms;
        frame.has_terms_orig = node.has_terms;
        frame.is_floor = node.is_floor();
        if let Some(floor_data) = floor_data {
            frame.set_floor_data(floor_data)?;
        }
        Ok(ord)
    }

    /// Loads the current frame's block.
    fn load_current_block(&mut self) -> Result<()> {
        let terms_in = &mut *self.terms_in;
        self.stack[self.current].load_block(terms_in)
    }

    /// Positions the cursor at the first term of the field.
    fn start(&mut self) -> Result<()> {
        let root = self.trie.root;
        self.nodes.clear();
        self.nodes.push(root);
        self.push_frame_from_node(root, 0)?;
        self.load_current_block()
    }

    /// Returns the field this cursor walks.
    pub fn field_reader(&self) -> &FieldReader {
        &self.field
    }

    /// Returns the term state of the current term, decoding its metadata first.
    ///
    /// Equivalent to `SegmentTermsEnum.decodeMetaData` followed by reading the
    /// frame's state.
    pub fn block_term_state(
        &mut self,
        postings_reader: &mut dyn crate::codecs::postings::PostingsReaderBase,
    ) -> Result<BlockTermState> {
        let field_info = self.field.field_info().clone();
        let frame = &mut self.stack[self.current];
        frame.decode_meta_data(&field_info, postings_reader)?;
        Ok(frame.state.clone())
    }
}

impl TermsEnum for SegmentTermsEnum {
    fn attributes(&mut self) -> &mut AttributeSource {
        &mut self.attributes
    }

    fn next(&mut self) -> Result<Option<BytesRef>> {
        if !self.started {
            self.start()?;
        }
        if self.eof {
            return Ok(None);
        }

        loop {
            // Exhausted the current block: either move to the next floor block,
            // or pop back to the parent.
            while self.stack[self.current].next_ent == self.stack[self.current].ent_count {
                if !self.stack[self.current].is_last_in_floor {
                    let terms_in = &mut *self.terms_in;
                    self.stack[self.current].load_next_floor_block(terms_in)?;
                    break;
                }
                if self.stack[self.current].ord == 0 {
                    self.eof = true;
                    self.term.clear();
                    self.term_exists = false;
                    return Ok(None);
                }
                let last_fp = self.stack[self.current].fp_orig;
                self.current -= 1;
                if self.stack[self.current].next_ent == -1
                    || self.stack[self.current].last_sub_fp != last_fp
                {
                    let target = self.term.clone();
                    self.stack[self.current].scan_to_floor_frame(&target)?;
                    let terms_in = &mut *self.terms_in;
                    self.stack[self.current].load_block(terms_in)?;
                    let mut term = std::mem::take(&mut self.term);
                    let result = self.stack[self.current].scan_to_sub_block(last_fp, &mut term);
                    self.term = term;
                    result?;
                }
            }

            let is_leaf = self.stack[self.current].is_leaf_block;
            if is_leaf {
                let mut term = std::mem::take(&mut self.term);
                let result = self.stack[self.current].next_leaf(&mut term);
                self.term = term;
                result?;
                self.term_exists = true;
                return Ok(Some(BytesRef::new(self.term.clone())));
            }

            let mut term = std::mem::take(&mut self.term);
            let is_sub_block = self.stack[self.current].next_non_leaf_entry(&mut term);
            self.term = term;
            if is_sub_block? {
                // Descend into the sub-block and keep going.
                let sub_fp = self.stack[self.current].last_sub_fp;
                let length = self.term.len();
                self.push_frame(None, sub_fp, length)?;
                self.load_current_block()?;
            } else {
                self.term_exists = true;
                return Ok(Some(BytesRef::new(self.term.clone())));
            }
        }
    }

    fn seek_ceil(&mut self, text: &BytesRef) -> Result<SeekStatus> {
        // Walk the trie as far as the target's bytes take us, then scan the
        // block that run of bytes lands in.
        let target = text.slice().to_vec();
        self.eof = false;
        self.started = false;
        self.current = 0;

        let mut node = self.trie.root;
        self.nodes.clear();
        self.nodes.push(node);
        let mut depth = 0usize;
        let mut best = (node, 0usize);

        while depth < target.len() {
            let Some(child) = self.trie.lookup_child(i32::from(target[depth]), &node)? else {
                break;
            };
            depth += 1;
            node = child;
            self.nodes.push(node);
            if node.has_output() {
                best = (node, depth);
            }
        }

        self.term.clear();
        self.term.extend_from_slice(&target[..best.1]);
        self.push_frame_from_node(best.0, best.1)?;
        self.load_current_block()?;
        self.stack[self.current].scan_to_floor_frame(&target)?;
        let terms_in = &mut *self.terms_in;
        self.stack[self.current].load_block(terms_in)?;

        // Scan forward until a term at or after the target.
        loop {
            match self.next()? {
                None => return Ok(SeekStatus::END),
                Some(term) => match term.slice().cmp(target.as_slice()) {
                    std::cmp::Ordering::Less => continue,
                    std::cmp::Ordering::Equal => return Ok(SeekStatus::FOUND),
                    std::cmp::Ordering::Greater => return Ok(SeekStatus::NOT_FOUND),
                },
            }
        }
    }

    fn seek_ord(&mut self, _ord: i64) -> Result<()> {
        Err(LuceneError::UnsupportedOperation(
            "the block-tree terms dictionary does not support seeking by ordinal".to_string(),
        ))
    }

    fn term(&self) -> Result<BytesRef> {
        Ok(BytesRef::new(self.term.clone()))
    }

    fn ord(&self) -> Result<i64> {
        Err(LuceneError::UnsupportedOperation(
            "the block-tree terms dictionary does not track term ordinals".to_string(),
        ))
    }

    fn doc_freq(&self) -> Result<i32> {
        Ok(self.stack[self.current].state.doc_freq)
    }

    fn total_term_freq(&self) -> Result<i64> {
        Ok(self.stack[self.current].state.total_term_freq)
    }

    fn impacts(&mut self, _flags: i32) -> Result<Box<dyn ImpactsEnum>> {
        Err(LuceneError::UnsupportedOperation(
            "impacts need the postings reader, which the field's producer supplies".to_string(),
        ))
    }

    fn postings(
        &mut self,
        _reuse: Option<Box<dyn PostingsEnum>>,
        _flags: i32,
    ) -> Result<Box<dyn PostingsEnum>> {
        Err(LuceneError::UnsupportedOperation(
            "postings need the postings reader, which the field's producer supplies".to_string(),
        ))
    }

    fn term_state(&mut self) -> Result<Box<dyn TermState>> {
        Ok(Box::new(self.stack[self.current].state.clone()))
    }
}
