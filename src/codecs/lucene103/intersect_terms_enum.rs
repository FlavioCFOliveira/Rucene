//! `IntersectTermsEnum` and `IntersectTermsEnumFrame` ported from
//! `org.apache.lucene.codecs.lucene103.blocktree`.
//!
//! Walks only the terms an automaton accepts, pruning whole blocks whose
//! prefix no transition can reach — which is what makes a wildcard, regexp or
//! fuzzy query cost far less than a full scan.

use crate::codecs::lucene103::blocktree::CompressionAlgorithm;
use crate::codecs::lucene103::field_reader::FieldReader;
use crate::codecs::lucene103::trie_reader::{Node, TrieReader};
use crate::codecs::term_state::BlockTermState;
use crate::error::{LuceneError, Result};
use crate::index::terms::{SeekStatus, TermState, TermsEnum};
use crate::index::{ImpactsEnum, IndexOptions, PostingsEnum};
use crate::store::{ByteArrayDataInput, DataInput, IndexInput};
use crate::util::automaton::{ByteRunnable, Transition, TransitionAccessor};
use crate::util::{AttributeSource, BytesRef};

/// One block of the terms dictionary, seen through the automaton.
///
/// Equivalent to
/// `org.apache.lucene.codecs.lucene103.blocktree.IntersectTermsEnumFrame`.
pub struct IntersectTermsEnumFrame {
    /// This frame's index in the stack.
    pub ord: usize,
    /// Where this block was loaded from.
    pub fp: i64,
    /// Where the block originally started.
    pub fp_orig: i64,
    /// Where this block ends.
    pub fp_end: i64,
    /// Pointer to the last sub-block seen.
    pub last_sub_fp: i64,

    /// Automaton state this block's prefix reaches.
    pub state: i32,
    /// The state just before the last label.
    pub last_state: i32,
    /// The transition currently being followed out of `state`.
    pub transition: Transition,
    /// Which transition out of `state` is current.
    pub transition_index: i32,
    /// How many transitions leave `state`.
    pub transition_count: i32,

    /// How many terms of this block have had their metadata decoded.
    pub meta_data_upto: i32,
    /// The term state the postings reader fills in.
    pub state_data: BlockTermState,

    suffix_bytes: Vec<u8>,
    suffixes_reader: ByteArrayDataInput,
    suffix_length_bytes: Vec<u8>,
    suffix_lengths_reader: ByteArrayDataInput,
    stat_bytes: Vec<u8>,
    stats_reader: ByteArrayDataInput,
    stats_singleton_run_length: i32,
    bytes: Vec<u8>,
    bytes_reader: ByteArrayDataInput,

    /// Floor data, copied out of the term index.
    floor_data: Vec<u8>,
    floor_data_pos: usize,

    /// Length of the prefix every term in this block shares.
    pub prefix: usize,
    /// How many entries the block holds.
    pub ent_count: i32,
    /// Which entry comes next.
    pub next_ent: i32,
    /// Whether this is the last block of its floor run.
    pub is_last_in_floor: bool,
    /// Whether every entry is a term.
    pub is_leaf_block: bool,
    /// How many floor blocks still follow.
    pub num_follow_floor_blocks: i32,
    /// Label at which the next floor block starts.
    pub next_floor_label: i32,

    /// Length of the current entry's suffix.
    pub suffix: usize,
    /// Where the current entry's suffix starts inside `suffix_bytes`.
    pub start_byte_pos: usize,
    /// The trie node that pointed at this block.
    pub node: Option<Node>,
}

impl std::fmt::Debug for IntersectTermsEnumFrame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IntersectTermsEnumFrame")
            .field("ord", &self.ord)
            .field("fp", &self.fp)
            .field("state", &self.state)
            .finish_non_exhaustive()
    }
}

impl IntersectTermsEnumFrame {
    /// Creates an empty frame at stack position `ord`.
    pub fn new(ord: usize) -> Self {
        Self {
            ord,
            fp: 0,
            fp_orig: 0,
            fp_end: 0,
            last_sub_fp: -1,
            state: 0,
            last_state: 0,
            transition: Transition::default(),
            transition_index: 0,
            transition_count: 0,
            meta_data_upto: 0,
            state_data: BlockTermState::default(),
            suffix_bytes: Vec::new(),
            suffixes_reader: ByteArrayDataInput::new(Vec::new()),
            suffix_length_bytes: Vec::new(),
            suffix_lengths_reader: ByteArrayDataInput::new(Vec::new()),
            stat_bytes: Vec::new(),
            stats_reader: ByteArrayDataInput::new(Vec::new()),
            stats_singleton_run_length: 0,
            bytes: Vec::new(),
            bytes_reader: ByteArrayDataInput::new(Vec::new()),
            floor_data: Vec::new(),
            floor_data_pos: 0,
            prefix: 0,
            ent_count: 0,
            next_ent: -1,
            is_last_in_floor: false,
            is_leaf_block: false,
            num_follow_floor_blocks: 0,
            next_floor_label: 0,
            suffix: 0,
            start_byte_pos: 0,
            node: None,
        }
    }

    /// Points this frame at automaton state `state`, priming its first
    /// transition.
    ///
    /// Equivalent to `IntersectTermsEnumFrame.setState`.
    pub fn set_state(&mut self, state: i32, automaton: &dyn TransitionAccessor) {
        self.state = state;
        self.transition_index = 0;
        self.transition_count = automaton.get_num_transitions(state);
        if self.transition_count != 0 {
            automaton.init_transition(state, &mut self.transition);
            automaton.get_next_transition(&mut self.transition);
        } else {
            // `min = -1` keeps the "label < min" test from firing, and
            // `max = -1` makes the very next label step past this frame.
            self.transition.min = -1;
            self.transition.max = -1;
        }
    }

    /// Records this block's floor data.
    pub fn set_floor_data(&mut self, floor_data: Vec<u8>) -> Result<()> {
        let mut reader = ByteArrayDataInput::from_slice(&floor_data);
        self.num_follow_floor_blocks = reader.read_v_int()?;
        self.next_floor_label = i32::from(reader.read_byte()?);
        self.floor_data_pos = reader.position();
        self.floor_data = floor_data;
        Ok(())
    }

    /// Reads this frame's block, which has the same layout as the one
    /// `SegmentTermsEnumFrame` reads.
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
        CompressionAlgorithm::by_code((code_l & 0x03) as i32)?.read(
            terms_in,
            &mut self.suffix_bytes,
            num_suffix_bytes,
        )?;
        self.suffixes_reader =
            ByteArrayDataInput::from_slice(&self.suffix_bytes[..num_suffix_bytes]);

        let mut num_suffix_length_bytes = terms_in.read_v_int()?;
        let all_equal = num_suffix_length_bytes & 0x01 != 0;
        num_suffix_length_bytes >>= 1;
        let n = num_suffix_length_bytes.max(0) as usize;
        if self.suffix_length_bytes.len() < n {
            self.suffix_length_bytes.resize(n, 0);
        }
        if all_equal {
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
        self.state_data.term_block_ord = 0;
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
    pub fn load_next_floor_block(&mut self, terms_in: &mut dyn IndexInput) -> Result<()> {
        self.fp = self.fp_end;
        self.next_ent = -1;
        self.load_block(terms_in)
    }

    /// Reads the next entry, returning whether it is a sub-block.
    ///
    /// Equivalent to `IntersectTermsEnumFrame.next`.
    pub fn next(&mut self) -> Result<bool> {
        if self.is_leaf_block {
            self.next_leaf()?;
            Ok(false)
        } else {
            self.next_non_leaf()
        }
    }

    fn next_leaf(&mut self) -> Result<()> {
        self.next_ent += 1;
        self.suffix = self.suffix_lengths_reader.read_v_int()?.max(0) as usize;
        self.start_byte_pos = self.suffixes_reader.position();
        self.suffixes_reader
            .seek(self.start_byte_pos + self.suffix)?;
        Ok(())
    }

    fn next_non_leaf(&mut self) -> Result<bool> {
        self.next_ent += 1;
        let code = self.suffix_lengths_reader.read_v_int()?;
        self.suffix = ((code as u32) >> 1) as usize;
        self.start_byte_pos = self.suffixes_reader.position();
        self.suffixes_reader
            .seek(self.start_byte_pos + self.suffix)?;
        if code & 1 == 0 {
            self.state_data.term_block_ord += 1;
            Ok(false)
        } else {
            self.last_sub_fp = self.fp - self.suffix_lengths_reader.read_v_long()?;
            Ok(true)
        }
    }

    /// Returns the bytes of the current entry's suffix.
    pub fn suffix_slice(&self) -> &[u8] {
        &self.suffix_bytes[self.start_byte_pos..self.start_byte_pos + self.suffix]
    }

    /// Returns the byte at `index` of the suffix buffer.
    pub fn suffix_byte(&self, index: usize) -> u8 {
        self.suffix_bytes[index]
    }

    /// Decodes the statistics and postings metadata of every term passed so far.
    pub fn decode_meta_data(
        &mut self,
        field_info: &crate::index::FieldInfo,
        postings_reader: &mut dyn crate::codecs::postings::PostingsReaderBase,
    ) -> Result<()> {
        let limit = if self.is_leaf_block {
            self.next_ent
        } else {
            self.state_data.term_block_ord
        };
        let mut absolute = self.meta_data_upto == 0;
        while self.meta_data_upto < limit {
            if self.stats_singleton_run_length > 0 {
                self.state_data.doc_freq = 1;
                self.state_data.total_term_freq = 1;
                self.stats_singleton_run_length -= 1;
            } else {
                let token = self.stats_reader.read_v_int()?;
                if token & 1 == 1 {
                    self.state_data.doc_freq = 1;
                    self.state_data.total_term_freq = 1;
                    self.stats_singleton_run_length = ((token as u32) >> 1) as i32;
                } else {
                    self.state_data.doc_freq = ((token as u32) >> 1) as i32;
                    self.state_data.total_term_freq =
                        if field_info.index_options == IndexOptions::DOCS {
                            i64::from(self.state_data.doc_freq)
                        } else {
                            i64::from(self.state_data.doc_freq) + self.stats_reader.read_v_long()?
                        };
                }
            }
            postings_reader.decode_term(
                &mut self.bytes_reader,
                field_info,
                &mut self.state_data,
                absolute,
            )?;
            self.meta_data_upto += 1;
            absolute = false;
        }
        Ok(())
    }
}

/// Walks only the terms an automaton accepts.
///
/// Equivalent to
/// `org.apache.lucene.codecs.lucene103.blocktree.IntersectTermsEnum`.
///
/// **Divergence from Lucene 10.5.0.** Java signals the end of the walk by
/// throwing a stack-trace-free `NoMoreTermsException` out of `popPushNext`, so
/// the deeply nested loop can unwind in one step. Rust returns `Ok(None)`
/// through the same path instead, which the labelled loops below make explicit.
/// The traversal order and the terms produced are unchanged.
pub struct IntersectTermsEnum {
    field: FieldReader,
    terms_in: Box<dyn IndexInput>,
    trie: TrieReader,
    run_automaton: Box<dyn ByteRunnable>,
    automaton: Box<dyn TransitionAccessor>,
    common_suffix: Option<BytesRef>,
    stack: Vec<IntersectTermsEnumFrame>,
    /// Index of the frame the cursor is in, or `None` once exhausted.
    current: Option<usize>,
    term: Vec<u8>,
    saved_start_term: Option<Vec<u8>>,
    attributes: AttributeSource,
}

impl std::fmt::Debug for IntersectTermsEnum {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IntersectTermsEnum")
            .field("field", &self.field.field_info().name)
            .finish_non_exhaustive()
    }
}

impl IntersectTermsEnum {
    /// Opens a cursor that yields only the terms `run_automaton` accepts.
    pub fn new(
        field: FieldReader,
        automaton: Box<dyn TransitionAccessor>,
        run_automaton: Box<dyn ByteRunnable>,
        common_suffix: Option<BytesRef>,
        start_term: Option<BytesRef>,
    ) -> Result<Self> {
        let shared = field.shared();
        let terms_in = shared.terms_in.clone_input()?;
        let index_in = shared.index_in.clone_input()?;
        let index_slice = index_in.random_access_slice(0, index_in.length())?;
        let trie = TrieReader::new(index_slice, field.index_start() + field.root_fp())?;

        let mut this = Self {
            field,
            terms_in,
            trie,
            run_automaton,
            automaton,
            common_suffix,
            stack: (0..5).map(IntersectTermsEnumFrame::new).collect(),
            current: None,
            term: Vec::new(),
            saved_start_term: start_term.map(|t| t.slice().to_vec()),
            attributes: AttributeSource::new(),
        };
        this.start()?;
        Ok(this)
    }

    /// Pushes the root frame over the trie root.
    fn start(&mut self) -> Result<()> {
        let root = self.trie.root;
        let frame = &mut self.stack[0];
        frame.ord = 0;
        frame.prefix = 0;
        frame.fp = root.output_fp;
        frame.fp_orig = root.output_fp;
        frame.next_ent = -1;
        frame.node = Some(root);
        if root.is_floor() {
            let end = self.field.index_end();
            let len = (end - root.floor_data_fp).max(0) as usize;
            let floor = self.trie.floor_data(&root, len.min(1 << 16))?;
            self.stack[0].set_floor_data(floor)?;
        }
        let automaton = &*self.automaton;
        self.stack[0].set_state(0, automaton);
        let terms_in = &mut *self.terms_in;
        self.stack[0].load_block(terms_in)?;
        self.current = Some(0);
        Ok(())
    }

    /// Descends into the sub-block the current frame points at, following the
    /// trie arcs for the bytes the term gained.
    ///
    /// Equivalent to `IntersectTermsEnum.pushFrame`.
    fn push_frame(&mut self, state: i32) -> Result<()> {
        let cur = self.current.expect("push_frame with no current frame");
        let ord = cur + 1;
        while self.stack.len() <= ord {
            self.stack
                .push(IntersectTermsEnumFrame::new(self.stack.len()));
        }

        let (fp, prefix, parent_node, parent_prefix) = {
            let parent = &self.stack[cur];
            (
                parent.last_sub_fp,
                parent.prefix + parent.suffix,
                parent.node,
                parent.prefix,
            )
        };

        // Walk the trie down the bytes this sub-block added to the prefix.
        let mut node = parent_node.ok_or_else(|| {
            LuceneError::IllegalState("parent frame has no trie node".to_string())
        })?;
        let mut idx = parent_prefix;
        while idx < prefix {
            let target = i32::from(self.term[idx]);
            node = self.trie.lookup_child(target, &node)?.ok_or_else(|| {
                LuceneError::corrupt_index(
                    format!("term index has no arc for label {target}"),
                    "term index",
                )
            })?;
            idx += 1;
        }

        let floor = if node.is_floor() {
            let end = self.field.index_end();
            let len = (end - node.floor_data_fp).max(0) as usize;
            Some(self.trie.floor_data(&node, len.min(1 << 16))?)
        } else {
            None
        };

        let automaton = &*self.automaton;
        let frame = &mut self.stack[ord];
        frame.ord = ord;
        frame.fp = fp;
        frame.fp_orig = fp;
        frame.prefix = prefix;
        frame.next_ent = -1;
        frame.node = Some(node);
        frame.set_state(state, automaton);
        if let Some(floor) = floor {
            frame.set_floor_data(floor)?;
        }
        let terms_in = &mut *self.terms_in;
        self.stack[ord].load_block(terms_in)?;
        self.current = Some(ord);
        Ok(())
    }

    /// Copies the current entry's suffix onto the term.
    ///
    /// Equivalent to `IntersectTermsEnum.copyTerm`.
    fn copy_term(&mut self) {
        let cur = self.current.expect("copy_term with no current frame");
        let (prefix, start, len) = {
            let frame = &self.stack[cur];
            (frame.prefix, frame.start_byte_pos, frame.suffix)
        };
        self.term.truncate(prefix);
        self.term.resize(prefix + len, 0);
        let suffix: Vec<u8> = self.stack[cur].suffix_bytes[start..start + len].to_vec();
        self.term[prefix..prefix + len].copy_from_slice(&suffix);
    }

    /// Advances to the next entry, popping exhausted frames first.
    ///
    /// Returns `None` once the walk is over, which is how this port replaces
    /// Java's `NoMoreTermsException`.
    ///
    /// Equivalent to `IntersectTermsEnum.popPushNext`.
    fn pop_push_next(&mut self) -> Result<Option<bool>> {
        loop {
            let cur = match self.current {
                Some(cur) => cur,
                None => return Ok(None),
            };
            if self.stack[cur].next_ent != self.stack[cur].ent_count {
                break;
            }
            if !self.stack[cur].is_last_in_floor {
                let terms_in = &mut *self.terms_in;
                self.stack[cur].load_next_floor_block(terms_in)?;
                break;
            }
            if self.stack[cur].ord == 0 {
                self.current = None;
                return Ok(None);
            }
            self.current = Some(cur - 1);
        }
        let cur = self.current.expect("current frame vanished");
        Ok(Some(self.stack[cur].next()?))
    }

    /// Returns whether the current entry's suffix ends with the common suffix
    /// every accepted term must have.
    fn matches_common_suffix(&self) -> bool {
        let Some(common_suffix) = &self.common_suffix else {
            return true;
        };
        let cur = self.current.expect("no current frame");
        let frame = &self.stack[cur];
        let common = common_suffix.slice();
        let term_len = frame.prefix + frame.suffix;
        if term_len < common.len() {
            return false;
        }

        // The common suffix may straddle the block prefix and the entry suffix.
        let len_in_prefix = common.len() as i64 - frame.suffix as i64;
        let mut common_pos = 0usize;
        if len_in_prefix > 0 {
            let len_in_prefix = len_in_prefix as usize;
            let mut term_pos = frame.prefix - len_in_prefix;
            while term_pos < frame.prefix {
                if self.term[term_pos] != common[common_pos] {
                    return false;
                }
                term_pos += 1;
                common_pos += 1;
            }
            let mut suffix_pos = frame.start_byte_pos;
            while common_pos < common.len() {
                if frame.suffix_byte(suffix_pos) != common[common_pos] {
                    return false;
                }
                suffix_pos += 1;
                common_pos += 1;
            }
        } else {
            let mut suffix_pos = frame.start_byte_pos + frame.suffix - common.len();
            while common_pos < common.len() {
                if frame.suffix_byte(suffix_pos) != common[common_pos] {
                    return false;
                }
                suffix_pos += 1;
                common_pos += 1;
            }
        }
        true
    }
}

impl TermsEnum for IntersectTermsEnum {
    fn attributes(&mut self) -> &mut AttributeSource {
        &mut self.attributes
    }

    fn next(&mut self) -> Result<Option<BytesRef>> {
        let Some(mut is_sub_block) = self.pop_push_next()? else {
            return Ok(None);
        };

        'next_term: loop {
            let Some(cur) = self.current else {
                return Ok(None);
            };

            let state;
            let last_state;

            if self.stack[cur].suffix != 0 {
                let label = i32::from(self.stack[cur].suffix_byte(self.stack[cur].start_byte_pos));

                // Below the current transition's range: skip entries until one
                // reaches it, then re-examine.
                if label < self.stack[cur].transition.min {
                    let min_trans = self.stack[cur].transition.min;
                    while self.stack[cur].next_ent < self.stack[cur].ent_count {
                        is_sub_block = self.stack[cur].next()?;
                        let pos = self.stack[cur].start_byte_pos;
                        if i32::from(self.stack[cur].suffix_byte(pos)) >= min_trans {
                            continue 'next_term;
                        }
                    }
                    match self.pop_push_next()? {
                        Some(v) => is_sub_block = v,
                        None => return Ok(None),
                    }
                    continue 'next_term;
                }

                // Above the current transition's range: step to the next
                // transition, or pop when there is none left.
                while label > self.stack[cur].transition.max {
                    if self.stack[cur].transition_index >= self.stack[cur].transition_count - 1 {
                        if self.stack[cur].ord == 0 {
                            self.current = None;
                            return Ok(None);
                        }
                        self.current = Some(cur - 1);
                        match self.pop_push_next()? {
                            Some(v) => is_sub_block = v,
                            None => return Ok(None),
                        }
                        continue 'next_term;
                    }
                    self.stack[cur].transition_index += 1;
                    self.automaton
                        .get_next_transition(&mut self.stack[cur].transition);
                    if label < self.stack[cur].transition.min {
                        let min_trans = self.stack[cur].transition.min;
                        while self.stack[cur].next_ent < self.stack[cur].ent_count {
                            is_sub_block = self.stack[cur].next()?;
                            let pos = self.stack[cur].start_byte_pos;
                            if i32::from(self.stack[cur].suffix_byte(pos)) >= min_trans {
                                continue 'next_term;
                            }
                        }
                        match self.pop_push_next()? {
                            Some(v) => is_sub_block = v,
                            None => return Ok(None),
                        }
                        continue 'next_term;
                    }
                }

                if self.common_suffix.is_some() && !is_sub_block && !self.matches_common_suffix() {
                    match self.pop_push_next()? {
                        Some(v) => is_sub_block = v,
                        None => return Ok(None),
                    }
                    continue 'next_term;
                }

                // Run the rest of the suffix through the automaton.
                let mut running_last = self.stack[cur].state;
                let mut running = self.stack[cur].transition.dest;
                let end = self.stack[cur].start_byte_pos + self.stack[cur].suffix;
                let mut idx = self.stack[cur].start_byte_pos + 1;
                let mut dead = false;
                while idx < end {
                    running_last = running;
                    running = self
                        .run_automaton
                        .step(running, i32::from(self.stack[cur].suffix_byte(idx)));
                    if running == -1 {
                        dead = true;
                        break;
                    }
                    idx += 1;
                }
                if dead {
                    match self.pop_push_next()? {
                        Some(v) => is_sub_block = v,
                        None => return Ok(None),
                    }
                    continue 'next_term;
                }
                state = running;
                last_state = running_last;
            } else {
                state = self.stack[cur].state;
                last_state = self.stack[cur].last_state;
            }

            if is_sub_block {
                self.copy_term();
                self.push_frame(state)?;
                if let Some(new_cur) = self.current {
                    self.stack[new_cur].last_state = last_state;
                }
            } else if self.run_automaton.is_accept(state) {
                self.copy_term();
                return Ok(Some(BytesRef::new(self.term.clone())));
            }

            match self.pop_push_next()? {
                Some(v) => is_sub_block = v,
                None => return Ok(None),
            }
        }
    }

    fn seek_ceil(&mut self, _text: &BytesRef) -> Result<SeekStatus> {
        Err(LuceneError::UnsupportedOperation(
            "IntersectTermsEnum only walks forward over the terms the automaton accepts"
                .to_string(),
        ))
    }

    fn seek_ord(&mut self, _ord: i64) -> Result<()> {
        Err(LuceneError::UnsupportedOperation(
            "IntersectTermsEnum does not support seeking by ordinal".to_string(),
        ))
    }

    fn term(&self) -> Result<BytesRef> {
        Ok(BytesRef::new(self.term.clone()))
    }

    fn ord(&self) -> Result<i64> {
        Err(LuceneError::UnsupportedOperation(
            "IntersectTermsEnum does not track term ordinals".to_string(),
        ))
    }

    fn doc_freq(&self) -> Result<i32> {
        let cur = self
            .current
            .ok_or_else(|| LuceneError::IllegalState("no current term".to_string()))?;
        Ok(self.stack[cur].state_data.doc_freq)
    }

    fn total_term_freq(&self) -> Result<i64> {
        let cur = self
            .current
            .ok_or_else(|| LuceneError::IllegalState("no current term".to_string()))?;
        Ok(self.stack[cur].state_data.total_term_freq)
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
        let cur = self
            .current
            .ok_or_else(|| LuceneError::IllegalState("no current term".to_string()))?;
        Ok(Box::new(self.stack[cur].state_data.clone()))
    }
}
