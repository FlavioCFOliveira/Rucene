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
use crate::util::{AttributeSource, BytesRef, StringHelper};

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

    // Java's frame keeps a `ByteArrayDataInput` next to each of these buffers
    // (`IntersectTermsEnumFrame.java:48`, `:51`, `:55`, `:90`) and resets it
    // over the buffer it sits beside (`:190`, `:203`, `:211`, `:224`). A struct
    // holding both a buffer and a view of that same buffer is a self-reference
    // Rust forbids, so the frame keeps the view's two fields — position and
    // limit — beside the buffer instead, and [`ByteArrayDataInput`] is rebuilt
    // around them for each read. The state is exactly Java's, only spelled
    // inline; the frame already stores `floor_data_pos` this way.
    suffix_bytes: Vec<u8>,
    /// Read position inside `suffix_bytes`.
    suffixes_pos: usize,
    /// Where this block's suffix bytes end inside `suffix_bytes`.
    suffixes_limit: usize,
    suffix_length_bytes: Vec<u8>,
    /// Read position inside `suffix_length_bytes`.
    suffix_lengths_pos: usize,
    /// Where this block's suffix lengths end inside `suffix_length_bytes`.
    suffix_lengths_limit: usize,
    stat_bytes: Vec<u8>,
    /// Read position inside `stat_bytes`.
    stats_pos: usize,
    /// Where this block's statistics end inside `stat_bytes`.
    stats_limit: usize,
    stats_singleton_run_length: i32,
    bytes: Vec<u8>,
    /// Read position inside `bytes`.
    bytes_pos: usize,
    /// Where this block's term metadata ends inside `bytes`.
    bytes_limit: usize,

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
            suffixes_pos: 0,
            suffixes_limit: 0,
            suffix_length_bytes: Vec::new(),
            suffix_lengths_pos: 0,
            suffix_lengths_limit: 0,
            stat_bytes: Vec::new(),
            stats_pos: 0,
            stats_limit: 0,
            stats_singleton_run_length: 0,
            bytes: Vec::new(),
            bytes_pos: 0,
            bytes_limit: 0,
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

    /// Runs `f` over this block's suffix bytes and saves the read position
    /// back.
    ///
    /// Stands in for Java's `suffixesReader` field, which cannot be a field
    /// here; see the note beside `suffix_bytes`.
    fn with_suffixes<T>(
        &mut self,
        f: impl FnOnce(&mut ByteArrayDataInput<'_>) -> Result<T>,
    ) -> Result<T> {
        let mut reader = ByteArrayDataInput::with_range(&self.suffix_bytes, 0, self.suffixes_limit);
        reader.set_position(self.suffixes_pos);
        let out = f(&mut reader);
        let position = reader.position();
        self.suffixes_pos = position;
        out
    }

    /// Runs `f` over this block's suffix lengths and saves the read position
    /// back.
    ///
    /// Stands in for Java's `suffixLengthsReader` field.
    fn with_suffix_lengths<T>(
        &mut self,
        f: impl FnOnce(&mut ByteArrayDataInput<'_>) -> Result<T>,
    ) -> Result<T> {
        let mut reader =
            ByteArrayDataInput::with_range(&self.suffix_length_bytes, 0, self.suffix_lengths_limit);
        reader.set_position(self.suffix_lengths_pos);
        let out = f(&mut reader);
        let position = reader.position();
        self.suffix_lengths_pos = position;
        out
    }

    /// Runs `f` over this block's term statistics and saves the read position
    /// back.
    ///
    /// Stands in for Java's `statsReader` field.
    fn with_stats<T>(
        &mut self,
        f: impl FnOnce(&mut ByteArrayDataInput<'_>) -> Result<T>,
    ) -> Result<T> {
        let mut reader = ByteArrayDataInput::with_range(&self.stat_bytes, 0, self.stats_limit);
        reader.set_position(self.stats_pos);
        let out = f(&mut reader);
        let position = reader.position();
        self.stats_pos = position;
        out
    }

    /// Records this block's floor data.
    pub fn set_floor_data(&mut self, floor_data: Vec<u8>) -> Result<()> {
        let mut reader = ByteArrayDataInput::new(&floor_data);
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
        self.suffixes_pos = 0;
        self.suffixes_limit = num_suffix_bytes;

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
        self.suffix_lengths_pos = 0;
        self.suffix_lengths_limit = n;

        let num_stat_bytes = terms_in.read_v_int()?.max(0) as usize;
        if self.stat_bytes.len() < num_stat_bytes {
            self.stat_bytes.resize(num_stat_bytes, 0);
        }
        terms_in.read_bytes(&mut self.stat_bytes, 0, num_stat_bytes)?;
        self.stats_pos = 0;
        self.stats_limit = num_stat_bytes;
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
        self.bytes_pos = 0;
        self.bytes_limit = num_meta_bytes;

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
        self.suffix = self.with_suffix_lengths(|r| r.read_v_int())?.max(0) as usize;
        self.start_byte_pos = self.suffixes_pos;
        let suffix = self.suffix as i64;
        self.with_suffixes(|r| r.skip_bytes(suffix))?;
        Ok(())
    }

    fn next_non_leaf(&mut self) -> Result<bool> {
        self.next_ent += 1;
        let code = self.with_suffix_lengths(|r| r.read_v_int())?;
        self.suffix = ((code as u32) >> 1) as usize;
        self.start_byte_pos = self.suffixes_pos;
        let suffix = self.suffix as i64;
        self.with_suffixes(|r| r.skip_bytes(suffix))?;
        if code & 1 == 0 {
            self.state_data.term_block_ord += 1;
            Ok(false)
        } else {
            self.last_sub_fp = self.fp - self.with_suffix_lengths(|r| r.read_v_long())?;
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

    /// Returns how many terms of this block have been passed.
    ///
    /// Equivalent to
    /// `org.apache.lucene.codecs.lucene103.blocktree.IntersectTermsEnumFrame#getTermBlockOrd`.
    pub fn get_term_block_ord(&self) -> i32 {
        if self.is_leaf_block {
            self.next_ent
        } else {
            self.state_data.term_block_ord
        }
    }

    /// Decodes the statistics and postings metadata of every term passed so far.
    pub fn decode_meta_data(
        &mut self,
        field_info: &crate::index::FieldInfo,
        postings_reader: &dyn crate::codecs::postings::PostingsReaderBase,
    ) -> Result<()> {
        let limit = self.get_term_block_ord();
        let mut absolute = self.meta_data_upto == 0;
        while self.meta_data_upto < limit {
            if self.stats_singleton_run_length > 0 {
                self.state_data.doc_freq = 1;
                self.state_data.total_term_freq = 1;
                self.stats_singleton_run_length -= 1;
            } else {
                let token = self.with_stats(|r| r.read_v_int())?;
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
                            i64::from(self.state_data.doc_freq)
                                + self.with_stats(|r| r.read_v_long())?
                        };
                }
            }
            // `bytes` and `state_data` are borrowed at once, which Java gets
            // for free by aliasing; the position is written back whether or not
            // the decode succeeded, exactly as Java's shared reader leaves it.
            let Self {
                bytes,
                bytes_pos,
                bytes_limit,
                state_data,
                ..
            } = self;
            let mut reader = ByteArrayDataInput::with_range(bytes, 0, *bytes_limit);
            reader.set_position(*bytes_pos);
            let decoded =
                postings_reader.decode_term(&mut reader, field_info, state_data, absolute);
            *bytes_pos = reader.position();
            decoded?;
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
    /// The trie node reached at each depth of the current term.
    nodes: Vec<Node>,
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
        let terms_in = field.shared().terms_in.clone_input()?;
        let trie = field.new_reader()?;

        let mut this = Self {
            field,
            terms_in,
            trie,
            run_automaton,
            automaton,
            common_suffix,
            stack: (0..5).map(IntersectTermsEnumFrame::new).collect(),
            nodes: vec![Node::default(); 5],
            current: None,
            term: Vec::new(),
            saved_start_term: None,
            attributes: AttributeSource::new(),
        };
        this.start()?;
        this.set_saved_start_term(start_term.as_ref());
        if let Some(start_term) = start_term.as_ref() {
            this.seek_to_start_term(start_term)?;
        }
        Ok(this)
    }

    /// Remembers the term the walk was started from.
    ///
    /// Java keeps this only to assert, later, that the walk never steps back
    /// before the term it was seeded with; this port keeps the same copy for
    /// the same purpose.
    ///
    /// Equivalent to
    /// `org.apache.lucene.codecs.lucene103.blocktree.IntersectTermsEnum#setSavedStartTerm`.
    fn set_saved_start_term(&mut self, start_term: Option<&BytesRef>) {
        self.saved_start_term = start_term.map(|t| t.slice().to_vec());
    }

    /// Returns the trie node slot at depth `ord`, growing the array first.
    ///
    /// Equivalent to
    /// `org.apache.lucene.codecs.lucene103.blocktree.IntersectTermsEnum#getNode`.
    fn get_node(&mut self, ord: usize) -> &mut Node {
        if ord >= self.nodes.len() {
            self.nodes.resize(ord + 1, Node::default());
        }
        &mut self.nodes[ord]
    }

    /// Runs the current entry's suffix through the automaton, from the state
    /// the current block's prefix reaches.
    ///
    /// Equivalent to
    /// `org.apache.lucene.codecs.lucene103.blocktree.IntersectTermsEnum#getState`.
    fn get_state(&self) -> i32 {
        let cur = self.current.expect("get_state with no current frame");
        let frame = &self.stack[cur];
        let mut state = frame.state;
        for idx in 0..frame.suffix {
            state = self.run_automaton.step(
                state,
                i32::from(frame.suffix_byte(frame.start_byte_pos + idx)),
            );
            debug_assert!(state != -1);
        }
        state
    }

    /// Positions the cursor just before `target`, so that the first
    /// [`TermsEnum::next`] returns the term after it.
    ///
    /// This is a seek *floor*: it is only ever used for the initial seek, and
    /// it walks the blocks rather than the index, exactly as Java does.
    ///
    /// Equivalent to
    /// `org.apache.lucene.codecs.lucene103.blocktree.IntersectTermsEnum#seekToStartTerm`.
    fn seek_to_start_term(&mut self, target: &BytesRef) -> Result<()> {
        debug_assert_eq!(self.current, Some(0));
        let target_bytes = target.slice();

        for _idx in 0..=target_bytes.len() {
            loop {
                let cur = self.current.expect("seek_to_start_term lost its frame");

                // Remember enough of the frame to be able to step back onto the
                // entry before the one that overshot the target.
                let sav_next_ent = self.stack[cur].next_ent;
                let save_pos = self.stack[cur].suffixes_pos;
                let save_length_pos = self.stack[cur].suffix_lengths_pos;
                let save_start_byte_pos = self.stack[cur].start_byte_pos;
                let save_suffix = self.stack[cur].suffix;
                let save_last_sub_fp = self.stack[cur].last_sub_fp;
                let save_term_block_ord = self.stack[cur].state_data.term_block_ord;

                let is_sub_block = self.stack[cur].next()?;
                self.copy_term();

                if is_sub_block
                    && StringHelper::starts_with_bytes(
                        target_bytes,
                        &BytesRef::new(self.term.clone()),
                    )
                {
                    // Recurse.
                    let state = self.get_state();
                    self.push_frame(state)?;
                    break;
                }

                match self.term.as_slice().cmp(target_bytes) {
                    std::cmp::Ordering::Less => {
                        if self.stack[cur].next_ent == self.stack[cur].ent_count {
                            if !self.stack[cur].is_last_in_floor {
                                // Advance to the next floor block.
                                let terms_in = &mut *self.terms_in;
                                self.stack[cur].load_next_floor_block(terms_in)?;
                                continue;
                            }
                            return Ok(());
                        }
                        continue;
                    }
                    std::cmp::Ordering::Equal => return Ok(()),
                    std::cmp::Ordering::Greater => {
                        // Fall back to the prior entry: the contract is that the
                        // first next() returns the term after the requested one.
                        let frame = &mut self.stack[cur];
                        frame.next_ent = sav_next_ent;
                        frame.last_sub_fp = save_last_sub_fp;
                        frame.start_byte_pos = save_start_byte_pos;
                        frame.suffix = save_suffix;
                        frame.suffixes_pos = save_pos;
                        frame.suffix_lengths_pos = save_length_pos;
                        frame.state_data.term_block_ord = save_term_block_ord;
                        // The last entry may have been a sub-block; there is no
                        // need to recurse into it, because the first next() will
                        // skip that frame anyway.
                        self.copy_term();
                        return Ok(());
                    }
                }
            }
        }

        Err(LuceneError::illegal_state(
            "seek_to_start_term ran past the target term".to_string(),
        ))
    }

    /// Pushes the root frame over the trie root.
    fn start(&mut self) -> Result<()> {
        let root = self.trie.root;
        *self.get_node(0) = root;
        let frame = &mut self.stack[0];
        frame.ord = 0;
        frame.prefix = 0;
        frame.fp = root.output_fp;
        frame.fp_orig = root.output_fp;
        frame.next_ent = -1;
        frame.node = Some(root);
        if root.is_floor() {
            let end = self.field.index_end() - self.field.index_start();
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
            LuceneError::illegal_state("parent frame has no trie node".to_string())
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
            *self.get_node(1 + idx) = node;
            idx += 1;
        }

        let floor = if node.is_floor() {
            let end = self.field.index_end() - self.field.index_start();
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

impl IntersectTermsEnum {
    /// Catches the current frame up on the metadata decode it defers.
    ///
    /// Equivalent to the `currentFrame.decodeMetaData()` call every statistics
    /// and postings accessor of `IntersectTermsEnum` makes before reading
    /// `state`.
    fn decode_current_meta_data(&mut self) -> Result<()> {
        let Some(cur) = self.current else {
            return Ok(());
        };
        let field_info = self.field.field_info().clone();
        let reader = std::sync::Arc::clone(&self.field.shared().postings_reader);
        self.stack[cur].decode_meta_data(&field_info, reader.as_ref())
    }
}

impl std::fmt::Display for IntersectTermsEnum {
    /// `org.apache.lucene.codecs.lucene103.blocktree.IntersectTermsEnum` declares no `toString()` in Lucene 10.5.0, so
    /// `java.lang.Object`'s applies: the binary class name, `@`, and the
    /// identity hash. See
    /// [`java_object_to_string`](crate::index::index_reader::java_object_to_string)
    /// for why only the hexadecimal suffix diverges.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&crate::index::index_reader::java_object_to_string(
            "org.apache.lucene.codecs.lucene103.blocktree.IntersectTermsEnum",
            self,
        ))
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

    fn doc_freq(&mut self) -> Result<i32> {
        // As in `SegmentTermsEnum`, Java decodes the frame's metadata before
        // reading `state`; without it the frame still holds the zero it was
        // built with.
        self.decode_current_meta_data()?;
        let cur = self
            .current
            .ok_or_else(|| LuceneError::illegal_state("no current term".to_string()))?;
        Ok(self.stack[cur].state_data.doc_freq)
    }

    fn total_term_freq(&mut self) -> Result<i64> {
        self.decode_current_meta_data()?;
        let cur = self
            .current
            .ok_or_else(|| LuceneError::illegal_state("no current term".to_string()))?;
        Ok(self.stack[cur].state_data.total_term_freq)
    }

    fn impacts(&mut self, flags: i32) -> Result<Box<dyn ImpactsEnum>> {
        self.decode_current_meta_data()?;
        let cur = self
            .current
            .ok_or_else(|| LuceneError::illegal_state("no current term".to_string()))?;
        let field_info = self.field.field_info().clone();
        let state = self.stack[cur].state_data;
        let reader = std::sync::Arc::clone(&self.field.shared().postings_reader);
        reader.impacts(&field_info, &state, flags)
    }

    fn postings(
        &mut self,
        reuse: Option<Box<dyn PostingsEnum>>,
        flags: i32,
    ) -> Result<Box<dyn PostingsEnum>> {
        self.decode_current_meta_data()?;
        let cur = self
            .current
            .ok_or_else(|| LuceneError::illegal_state("no current term".to_string()))?;
        let field_info = self.field.field_info().clone();
        let state = self.stack[cur].state_data;
        let reader = std::sync::Arc::clone(&self.field.shared().postings_reader);
        reader.postings(&field_info, &state, reuse, flags)
    }

    fn term_state(&mut self) -> Result<Box<dyn TermState>> {
        let cur = self
            .current
            .ok_or_else(|| LuceneError::illegal_state("no current term".to_string()))?;
        Ok(Box::new(self.stack[cur].state_data.clone()))
    }
}
