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

//! The types declared in `ViterbiNBest.java`, of the Java package
//! `org.apache.lucene.analysis.morph`.

use std::sync::Arc;

use rucene::analysis::CharReader;
use rucene::error::Result;
use rucene::internal::hppc::{IntArrayList, IntIntHashMap};
use rucene::util::ArrayUtil;

use super::binary_dictionary::BinaryDictionarySubclass;
use super::connection_costs::ConnectionCosts;
use super::dictionary::Dictionary;
use super::morph_data::MorphData;
use super::token::TokenSubclass;
use super::token_info_fst::TokenInfoFST;
use super::token_type::TokenType;
use super::viterbi::{
    Position, Viterbi, ViterbiPosition, ViterbiSubclass, WrappedPositionArray, VERBOSE,
};

/// Java's `EnumMap<TokenType, Dictionary<? extends U>>`: one optional
/// dictionary per [`TokenType`], indexed by its ordinal.
pub type DictionaryMap<U> = [Option<Arc<dyn Dictionary<U>>>; 3];

/// `Viterbi` subclass for n-best path calculation.
///
/// Equivalent to `org.apache.lucene.analysis.morph.ViterbiNBest<T extends
/// Token, U extends MorphData>`, an abstract class extending
/// `Viterbi<T, ViterbiNBest.PositionNBest>`: this struct holds its fields
/// and the `Viterbi` it extends, and a concrete tokenizer holds a
/// `ViterbiNBest` and implements [`ViterbiNBestSubclass`] (and
/// [`ViterbiSubclass`], whose `backtrace_nbest` and `fixup_pending_list` it
/// implements with this module's final [`backtrace_nbest`] and
/// [`fixup_pending_list`]).
pub struct ViterbiNBest<T: TokenSubclass, U: ?Sized + MorphData> {
    /// The `Viterbi` this class extends.
    pub viterbi: Viterbi<T, PositionNBest>,
    /// Equivalent to the protected `ViterbiNBest.dictionaryMap`.
    pub dictionary_map: DictionaryMap<U>,
    /// Allowable cost difference for N-best output:
    n_best_cost: i32,
    /// Equivalent to the protected `ViterbiNBest.lattice` (`None` for
    /// `null`).
    pub lattice: Option<Lattice<U>>,
}

impl<T: TokenSubclass, U: ?Sized + MorphData> std::fmt::Debug for ViterbiNBest<T, U> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ViterbiNBest")
            .field("viterbi", &self.viterbi)
            .field("n_best_cost", &self.n_best_cost)
            .finish_non_exhaustive()
    }
}

impl<T: TokenSubclass, U: ?Sized + MorphData> ViterbiNBest<T, U> {
    /// Equivalent to the protected `ViterbiNBest(TokenInfoFST fst,
    /// FST.BytesReader fstReader, BinaryDictionary<? extends MorphData>
    /// dictionary, TokenInfoFST userFST, FST.BytesReader userFSTReader,
    /// Dictionary<? extends MorphData> userDictionary, ConnectionCosts
    /// costs)`, without the readers (see [`Viterbi`]).
    pub fn new(
        fst: Arc<TokenInfoFST>,
        dictionary: Arc<dyn BinaryDictionarySubclass<dyn MorphData>>,
        user_fst: Option<Arc<TokenInfoFST>>,
        user_dictionary: Option<Arc<dyn Dictionary<dyn MorphData>>>,
        costs: Arc<ConnectionCosts>,
    ) -> Self {
        Self {
            viterbi: Viterbi::new(fst, dictionary, user_fst, user_dictionary, costs),
            dictionary_map: [None, None, None],
            n_best_cost: 0,
            lattice: None,
        }
    }

    /// Equivalent to the protected `ViterbiNBest.setNBestCost(int)`.
    pub fn set_nbest_cost(&mut self, value: i32) {
        self.n_best_cost = value;
        self.viterbi.output_nbest = 0 < self.n_best_cost;
    }

    /// Equivalent to the protected `ViterbiNBest.getNBestCost()`.
    pub fn get_nbest_cost(&self) -> i32 {
        self.n_best_cost
    }

    /// Equivalent to `ViterbiNBest.getLatticeRootBase()`.
    ///
    /// # Panics
    ///
    /// Before the first n-best backtrace, where Java's `lattice.getRootBase()`
    /// throws `NullPointerException`.
    pub fn get_lattice_root_base(&self) -> i32 {
        self.lattice
            .as_ref()
            .expect("INVARIANT: Java's lattice.getRootBase() throws NullPointerException")
            .get_root_base()
    }

    /// Equivalent to `ViterbiNBest.probeDelta(int, int)`.
    ///
    /// # Panics
    ///
    /// Before the first n-best backtrace, where Java's `lattice.probeDelta`
    /// throws `NullPointerException`.
    pub fn probe_delta(&self, start: i32, end: i32) -> i32 {
        self.lattice
            .as_ref()
            .expect("INVARIANT: Java's lattice.probeDelta throws NullPointerException")
            .probe_delta(start, end)
    }
}

/// A concrete n-best tokenizer: a subclass of `ViterbiNBest`.
pub trait ViterbiNBestSubclass<T: TokenSubclass, U: ?Sized + MorphData>:
    ViterbiSubclass<T, PositionNBest>
{
    /// The `ViterbiNBest` this tokenizer extends.
    fn viterbi_nbest(&self) -> &ViterbiNBest<T, U>;

    /// The `ViterbiNBest` this tokenizer extends, for writing.
    fn viterbi_nbest_mut(&mut self) -> &mut ViterbiNBest<T, U>;

    /// Add n-best tokens to the pending list.
    ///
    /// Equivalent to the protected abstract `ViterbiNBest.registerNode(int, char[])`.
    fn register_node(&mut self, node: i32, fragment: &[u16]);
}

/// Equivalent to the protected final `ViterbiNBest.backtraceNBest(Position,
/// boolean)`, which a concrete tokenizer's [`ViterbiSubclass::backtrace_nbest`]
/// calls; `end_pos_data` is the position's `pos`.
///
/// # Errors
///
/// What reading the input raises.
pub fn backtrace_nbest<T, U, H>(
    host: &mut H,
    _reader: &mut dyn CharReader,
    end_pos_data: i32,
    use_eos: bool,
) -> Result<()>
where
    T: TokenSubclass,
    U: ?Sized + MorphData,
    H: ViterbiNBestSubclass<T, U> + ?Sized,
{
    let (fragment, best_cost, best_path) = {
        let nb = host.viterbi_nbest_mut();
        if nb.lattice.is_none() {
            nb.lattice = Some(Lattice::new());
        }
        let ViterbiNBest {
            viterbi,
            dictionary_map,
            lattice,
            ..
        } = nb;
        let lattice = lattice.as_mut().expect("INVARIANT: set above");

        let end_pos = end_pos_data;
        let fragment = viterbi.buffer.get_range(
            viterbi.last_back_trace_pos,
            (end_pos - viterbi.last_back_trace_pos) as usize,
        );
        lattice.setup(
            &fragment,
            dictionary_map,
            &mut viterbi.positions,
            viterbi.last_back_trace_pos,
            end_pos,
            use_eos,
        );
        lattice.mark_unreachable();
        lattice.calc_left_cost(&viterbi.costs);
        lattice.calc_right_cost(&viterbi.costs);

        let best_cost = lattice.best_cost();
        let best_path = lattice.best_path_node_list();
        (fragment, best_cost, best_path)
    };
    for node in best_path.iter() {
        host.register_node(node.value, &fragment);
    }

    let mut n = 2;
    loop {
        let (nbest, cost) = {
            let nb = host.viterbi_nbest_mut();
            let lattice = nb.lattice.as_mut().expect("INVARIANT: set above");
            let nbest = lattice.n_best_node_list(n);
            if nbest.is_empty() {
                break;
            }
            let cost = lattice.cost(nbest.get(0));
            (nbest, cost)
        };
        if best_cost.wrapping_add(host.viterbi_nbest().n_best_cost) < cost {
            break;
        }
        for node in nbest.iter() {
            host.register_node(node.value, &fragment);
        }
        n += 1;
    }
    if VERBOSE {
        if let Some(lattice) = host.viterbi_nbest().lattice.as_ref() {
            lattice.debug_print();
        }
    }
    Ok(())
}

/// Equivalent to the protected final `ViterbiNBest.fixupPendingList()`, which
/// a concrete tokenizer's [`ViterbiSubclass::fixup_pending_list`] calls.
///
/// # Errors
///
/// None; the signature is the hook's.
pub fn fixup_pending_list<T, U, H>(host: &mut H) -> Result<()>
where
    T: TokenSubclass,
    U: ?Sized + MorphData,
    H: ViterbiNBestSubclass<T, U> + ?Sized,
{
    let pending = &mut host.viterbi_nbest_mut().viterbi.pending;
    // Sort for removing same tokens.
    // USER token should be ahead from normal one.
    pending.sort_by(|a, b| {
        let (a, b) = (a.token(), b.token());
        let a_off = a.get_offset();
        let b_off = b.get_offset();
        if a_off != b_off {
            return a_off.wrapping_sub(b_off).cmp(&0);
        }
        let a_len = a.get_length();
        let b_len = b.get_length();
        if a_len != b_len {
            return a_len.wrapping_sub(b_len).cmp(&0);
        }
        // order of Type is KNOWN, UNKNOWN, USER,
        // so we use reversed comparison here.
        (b.get_type().ordinal() - a.get_type().ordinal()).cmp(&0)
    });

    // Remove same token.
    let mut i = 1;
    while i < pending.len() {
        let same = {
            let a = pending[i - 1].token();
            let b = pending[i].token();
            a.get_offset() == b.get_offset() && a.get_length() == b.get_length()
        };
        if same {
            pending.remove(i);
            // It is important to decrement "i" here, because a next may be removed.
            continue;
        }
        i += 1;
    }

    // offset=>position map
    let mut map = IntIntHashMap::new();
    for t in pending.iter() {
        let t = t.token();
        map.put(t.get_offset(), 0);
        map.put(t.get_offset() + t.get_length(), 0);
    }

    // Get unique and sorted list of all edge position of tokens.
    let mut offsets = map.keys().to_array();
    offsets.sort_unstable();

    // setup all value of map.  It specifies N-th position from begin.
    for (i, &offset) in offsets.iter().enumerate() {
        map.put(offset, i as i32);
    }

    // We got all position length now.
    for t in pending.iter_mut() {
        let (offset, length) = (t.token().get_offset(), t.token().get_length());
        t.token_mut()
            .set_position_length(map.get(offset + length) - map.get(offset));
    }

    // Make PENDING to be reversed order to fit its usage.
    // If you would like to speedup, you can try reversed order sort
    // at first of this function.
    pending.reverse();
    Ok(())
}

/// `Viterbi.Position` extension; this holds all forward pointers to
/// calculate n-best path.
///
/// Equivalent to `org.apache.lucene.analysis.morph.ViterbiNBest$PositionNBest`;
/// a `null` forward type is `None`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PositionNBest {
    position: Position,
    // Only used when finding 2nd best segmentation under a
    // too-long token:
    forward_count: i32,
    forward_pos: Vec<i32>,
    forward_id: Vec<i32>,
    forward_index: Vec<i32>,
    forward_type: Vec<Option<TokenType>>,
}

impl Default for PositionNBest {
    fn default() -> Self {
        Self {
            position: Position::default(),
            forward_count: 0,
            forward_pos: vec![0; 8],
            forward_id: vec![0; 8],
            forward_index: vec![0; 8],
            forward_type: vec![None; 8],
        }
    }
}

impl ViterbiPosition for PositionNBest {
    fn position(&self) -> &Position {
        &self.position
    }

    fn position_mut(&mut self) -> &mut Position {
        &mut self.position
    }

    /// Equivalent to `PositionNBest.reset()`.
    fn reset(&mut self) {
        self.position.reset();
        // forwardCount naturally resets after it runs:
        debug_assert!(
            self.forward_count == 0,
            "pos={} forwardCount={}",
            self.position.get_pos(),
            self.forward_count
        );
    }
}

impl PositionNBest {
    /// Equivalent to the private `PositionNBest.growForward()`.
    fn grow_forward(&mut self) {
        let min = (1 + self.forward_count) as usize;
        self.forward_pos = ArrayUtil::grow_int(&self.forward_pos, min);
        self.forward_id = ArrayUtil::grow_int(&self.forward_id, min);
        self.forward_index = ArrayUtil::grow_int(&self.forward_index, min);

        // NOTE: sneaky: grow separately because
        // ArrayUtil.grow will otherwise pick a different
        // length than the int[]s we just grew:
        self.forward_type.resize(self.forward_pos.len(), None);
    }

    /// Equivalent to `PositionNBest.addForward(int, int, int, TokenType)`.
    pub fn add_forward(
        &mut self,
        forward_pos: i32,
        forward_index: i32,
        forward_id: i32,
        forward_type: Option<TokenType>,
    ) {
        if self.forward_count as usize == self.forward_id.len() {
            self.grow_forward();
        }
        let c = self.forward_count as usize;
        self.forward_pos[c] = forward_pos;
        self.forward_index[c] = forward_index;
        self.forward_id[c] = forward_id;
        self.forward_type[c] = forward_type;
        self.forward_count += 1;
    }

    /// Equivalent to `PositionNBest.getForwardCount()`.
    pub fn get_forward_count(&self) -> i32 {
        self.forward_count
    }

    /// Equivalent to `PositionNBest.setForwardCount(int)`.
    pub fn set_forward_count(&mut self, forward_count: i32) {
        self.forward_count = forward_count;
    }

    /// Equivalent to `PositionNBest.getForwardType(int)`.
    pub fn get_forward_type(&self, index: i32) -> Option<TokenType> {
        self.forward_type[index as usize]
    }

    /// Equivalent to `PositionNBest.getForwardID(int)`.
    pub fn get_forward_id(&self, index: i32) -> i32 {
        self.forward_id[index as usize]
    }

    /// Equivalent to `PositionNBest.getForwardPos(int)`.
    pub fn get_forward_pos(&self, index: i32) -> i32 {
        self.forward_pos[index as usize]
    }
}

/// Yet another lattice data structure for keeping n-best path.
///
/// Equivalent to the protected static class
/// `org.apache.lucene.analysis.morph.ViterbiNBest$Lattice<U extends MorphData>`;
/// the dictionary map it keeps is a copy of the tokenizer's (shared
/// dictionaries), and a `null` node dictionary type is `None`.
pub struct Lattice<U: ?Sized + MorphData> {
    fragment: Vec<u16>,
    dictionary_map: DictionaryMap<U>,
    use_eos: bool,

    root_capacity: usize,
    root_size: i32,
    root_base: i32,

    // root pointers of node chain by leftChain_ that have same start offset.
    l_root: Vec<i32>,
    // root pointers of node chain by rightChain_ that have same end offset.
    r_root: Vec<i32>,

    capacity: usize,
    node_count: i32,

    // The variables below are elements of lattice node that indexed by node number.
    node_dic_type: Vec<Option<TokenType>>,
    node_word_id: Vec<i32>,
    // nodeMark - -1:excluded, 0:unused, 1:bestpath, 2:2-best-path, ... N:N-best-path
    node_mark: Vec<i32>,
    node_left_id: Vec<i32>,
    node_right_id: Vec<i32>,
    node_word_cost: Vec<i32>,
    node_left_cost: Vec<i32>,
    node_right_cost: Vec<i32>,
    // nodeLeftNode, nodeRightNode - are left/right node number with minimum cost path.
    node_left_node: Vec<i32>,
    node_right_node: Vec<i32>,
    // nodeLeft, nodeRight - start/end offset
    node_left: Vec<i32>,
    node_right: Vec<i32>,
    node_left_chain: Vec<i32>,
    node_right_chain: Vec<i32>,
}

impl<U: ?Sized + MorphData> std::fmt::Debug for Lattice<U> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Lattice")
            .field("root_size", &self.root_size)
            .field("root_base", &self.root_base)
            .field("node_count", &self.node_count)
            .finish_non_exhaustive()
    }
}

impl<U: ?Sized + MorphData> Default for Lattice<U> {
    fn default() -> Self {
        Self::new()
    }
}

impl<U: ?Sized + MorphData> Lattice<U> {
    /// Equivalent to the implicit `Lattice()`.
    pub fn new() -> Self {
        Self {
            fragment: Vec::new(),
            dictionary_map: [None, None, None],
            use_eos: false,
            root_capacity: 0,
            root_size: 0,
            root_base: 0,
            l_root: Vec::new(),
            r_root: Vec::new(),
            capacity: 0,
            node_count: 0,
            node_dic_type: Vec::new(),
            node_word_id: Vec::new(),
            node_mark: Vec::new(),
            node_left_id: Vec::new(),
            node_right_id: Vec::new(),
            node_word_cost: Vec::new(),
            node_left_cost: Vec::new(),
            node_right_cost: Vec::new(),
            node_left_node: Vec::new(),
            node_right_node: Vec::new(),
            node_left: Vec::new(),
            node_right: Vec::new(),
            node_left_chain: Vec::new(),
            node_right_chain: Vec::new(),
        }
    }

    /// Equivalent to `Lattice.getNodeLeft(int)`.
    pub fn get_node_left(&self, node: i32) -> i32 {
        self.node_left[node as usize]
    }

    /// Equivalent to `Lattice.getNodeRight(int)`.
    pub fn get_node_right(&self, node: i32) -> i32 {
        self.node_right[node as usize]
    }

    /// Equivalent to `Lattice.getNodeDicType(int)`.
    pub fn get_node_dic_type(&self, node: i32) -> Option<TokenType> {
        self.node_dic_type[node as usize]
    }

    /// Equivalent to `Lattice.getNodeWordID(int)`.
    pub fn get_node_word_id(&self, node: i32) -> i32 {
        self.node_word_id[node as usize]
    }

    /// Equivalent to `Lattice.getRootBase()`.
    pub fn get_root_base(&self) -> i32 {
        self.root_base
    }

    /// Equivalent to the private `Lattice.setupRoot(int, int)`.
    fn setup_root(&mut self, base_offset: i32, last_offset: i32) {
        debug_assert!(base_offset <= last_offset);
        let size = (last_offset - base_offset + 1) as usize;
        if self.root_capacity < size {
            let oversize = ArrayUtil::oversize(size, 4);
            self.l_root = vec![0; oversize];
            self.r_root = vec![0; oversize];
            self.root_capacity = oversize;
        }
        self.l_root[..size].fill(-1);
        self.r_root[..size].fill(-1);
        self.root_size = size as i32;
        self.root_base = base_offset;
    }

    /// Reserve at least N nodes.
    ///
    /// Equivalent to the private `Lattice.reserve(int)`.
    fn reserve(&mut self, n: usize) {
        if self.capacity < n {
            let oversize = ArrayUtil::oversize(n, 4);
            self.node_dic_type = vec![None; oversize];
            self.node_word_id = vec![0; oversize];
            self.node_mark = vec![0; oversize];
            self.node_left_id = vec![0; oversize];
            self.node_right_id = vec![0; oversize];
            self.node_word_cost = vec![0; oversize];
            self.node_left_cost = vec![0; oversize];
            self.node_right_cost = vec![0; oversize];
            self.node_left_node = vec![0; oversize];
            self.node_right_node = vec![0; oversize];
            self.node_left = vec![0; oversize];
            self.node_right = vec![0; oversize];
            self.node_left_chain = vec![0; oversize];
            self.node_right_chain = vec![0; oversize];
            self.capacity = oversize;
        }
    }

    /// Equivalent to the private `Lattice.setupNodePool(int)`.
    fn setup_node_pool(&mut self, n: usize) {
        self.reserve(n);
        self.node_count = 0;
    }

    /// Equivalent to the private `Lattice.addNode(TokenType, int, int, int)`.
    ///
    /// # Panics
    ///
    /// For a word whose type has no dictionary in the map, where Java throws
    /// `NullPointerException`.
    fn add_node(
        &mut self,
        dic_type: Option<TokenType>,
        word_id: i32,
        left: i32,
        right: i32,
    ) -> i32 {
        debug_assert!((self.node_count as usize) < self.capacity);
        debug_assert!(left == -1 || right == -1 || left < right);
        debug_assert!(left == -1 || (0 <= left && left < self.root_size));
        debug_assert!(right == -1 || (0 <= right && right < self.root_size));

        let node = self.node_count;
        self.node_count += 1;
        let n = node as usize;

        self.node_dic_type[n] = dic_type;
        self.node_word_id[n] = word_id;
        self.node_mark[n] = 0;

        if word_id < 0 {
            self.node_word_cost[n] = 0;
            self.node_left_cost[n] = 0;
            self.node_right_cost[n] = 0;
            self.node_left_id[n] = 0;
            self.node_right_id[n] = 0;
        } else {
            let dic = dic_type
                .and_then(|t| self.dictionary_map[t.ordinal() as usize].as_ref())
                .expect("INVARIANT: Java's dic.getWordCost throws NullPointerException");
            self.node_word_cost[n] = dic.get_word_cost(word_id);
            self.node_left_id[n] = dic.get_left_id(word_id);
            self.node_right_id[n] = dic.get_right_id(word_id);
        }

        self.node_left[n] = left;
        self.node_right[n] = right;
        if 0 <= left {
            self.node_left_chain[n] = self.l_root[left as usize];
            self.l_root[left as usize] = node;
        } else {
            self.node_left_chain[n] = -1;
        }
        if 0 <= right {
            self.node_right_chain[n] = self.r_root[right as usize];
            self.r_root[right as usize] = node;
        } else {
            self.node_right_chain[n] = -1;
        }
        node
    }

    /// Sum of positions.get(i).count in [beg, end) range.
    ///
    /// Equivalent to the private `Lattice.positionCount(WrappedPositionArray, int, int)`.
    fn position_count(
        positions: &mut WrappedPositionArray<PositionNBest>,
        beg: i32,
        end: i32,
    ) -> i32 {
        let mut count = 0;
        for i in beg..end {
            count += positions.get(i).position().get_count();
        }
        count
    }

    /// Equivalent to the package-private `Lattice.setup(char[], EnumMap,
    /// WrappedPositionArray, int, int, boolean)`.
    pub(crate) fn setup(
        &mut self,
        fragment: &[u16],
        dictionary_map: &DictionaryMap<U>,
        positions: &mut WrappedPositionArray<PositionNBest>,
        prev_offset: i32,
        end_offset: i32,
        use_eos: bool,
    ) {
        debug_assert!(positions.get(prev_offset).position().get_count() == 1);

        self.fragment = fragment.to_vec();
        self.dictionary_map = dictionary_map.clone();
        self.use_eos = use_eos;

        // Initialize lRoot and rRoot.
        self.setup_root(prev_offset, end_offset);

        // "+ 2" for first/last record.
        let n = Self::position_count(positions, prev_offset + 1, end_offset + 1) + 2;
        self.setup_node_pool(n as usize);

        // substitute for BOS = 0
        let (first_type, first_id) = {
            let first = positions.get(prev_offset).position();
            (first.get_back_type(0), first.get_back_id(0))
        };
        if self.add_node(first_type, first_id, -1, 0) != 0 {
            debug_assert!(false);
        }

        // EOS = 1
        if self.add_node(Some(TokenType::Known), -1, end_offset - self.root_base, -1) != 1 {
            debug_assert!(false);
        }

        let mut offset = end_offset;
        while prev_offset < offset {
            let right = offset - self.root_base;
            // optimize: exclude disconnected nodes.
            if 0 <= self.l_root[right as usize] {
                let back: Vec<(Option<TokenType>, i32, i32)> = {
                    let pos = positions.get(offset).position();
                    (0..pos.get_count())
                        .map(|i| {
                            (
                                pos.get_back_type(i),
                                pos.get_back_id(i),
                                pos.get_back_pos(i),
                            )
                        })
                        .collect()
                };
                for (back_type, back_id, back_pos) in back {
                    self.add_node(back_type, back_id, back_pos - self.root_base, right);
                }
            }
            offset -= 1;
        }
    }

    /// set mark = -1 for unreachable nodes.
    ///
    /// Equivalent to the package-private `Lattice.markUnreachable()`.
    pub(crate) fn mark_unreachable(&mut self) {
        for index in 1..self.root_size - 1 {
            if self.r_root[index as usize] < 0 {
                let mut node = self.l_root[index as usize];
                while 0 <= node {
                    self.node_mark[node as usize] = -1;
                    node = self.node_left_chain[node as usize];
                }
            }
        }
    }

    /// Equivalent to the private `Lattice.connectionCost(ConnectionCosts, int, int)`.
    fn connection_cost(&self, costs: &ConnectionCosts, left: i32, right: i32) -> i32 {
        let left_id = self.node_left_id[right as usize];
        if left_id == 0 && !self.use_eos {
            0
        } else {
            costs.get(self.node_right_id[left as usize], left_id)
        }
    }

    /// Equivalent to the package-private `Lattice.calcLeftCost(ConnectionCosts)`.
    pub(crate) fn calc_left_cost(&mut self, costs: &ConnectionCosts) {
        for index in 0..self.root_size {
            let mut node = self.l_root[index as usize];
            while 0 <= node {
                if 0 <= self.node_mark[node as usize] {
                    let mut least_node = -1;
                    let mut least_cost = i32::MAX;
                    let mut left_node = self.r_root[index as usize];
                    while 0 <= left_node {
                        if 0 <= self.node_mark[left_node as usize] {
                            let cost = self.node_left_cost[left_node as usize]
                                .wrapping_add(self.node_word_cost[left_node as usize])
                                .wrapping_add(self.connection_cost(costs, left_node, node));
                            if cost < least_cost {
                                least_cost = cost;
                                least_node = left_node;
                            }
                        }
                        left_node = self.node_right_chain[left_node as usize];
                    }
                    debug_assert!(0 <= least_node);
                    self.node_left_node[node as usize] = least_node;
                    self.node_left_cost[node as usize] = least_cost;
                }
                node = self.node_left_chain[node as usize];
            }
        }
    }

    /// Equivalent to the package-private `Lattice.calcRightCost(ConnectionCosts)`.
    pub(crate) fn calc_right_cost(&mut self, costs: &ConnectionCosts) {
        let mut index = self.root_size - 1;
        while 0 <= index {
            let mut node = self.r_root[index as usize];
            while 0 <= node {
                if 0 <= self.node_mark[node as usize] {
                    let mut least_node = -1;
                    let mut least_cost = i32::MAX;
                    let mut right_node = self.l_root[index as usize];
                    while 0 <= right_node {
                        if 0 <= self.node_mark[right_node as usize] {
                            let cost = self.node_right_cost[right_node as usize]
                                .wrapping_add(self.node_word_cost[right_node as usize])
                                .wrapping_add(self.connection_cost(costs, node, right_node));
                            if cost < least_cost {
                                least_cost = cost;
                                least_node = right_node;
                            }
                        }
                        right_node = self.node_left_chain[right_node as usize];
                    }
                    debug_assert!(0 <= least_node);
                    self.node_right_node[node as usize] = least_node;
                    self.node_right_cost[node as usize] = least_cost;
                }
                node = self.node_right_chain[node as usize];
            }
            index -= 1;
        }
    }

    /// Mark all nodes that have same text and different par-of-speech or
    /// reading.
    ///
    /// Equivalent to the package-private `Lattice.markSameSpanNode(int, int)`.
    pub(crate) fn mark_same_span_node(&mut self, ref_node: i32, value: i32) {
        let left = self.node_left[ref_node as usize];
        let right = self.node_right[ref_node as usize];
        let mut node = self.l_root[left as usize];
        while 0 <= node {
            if self.node_right[node as usize] == right {
                self.node_mark[node as usize] = value;
            }
            node = self.node_left_chain[node as usize];
        }
    }

    /// Equivalent to the package-private `Lattice.bestPathNodeList()`.
    pub(crate) fn best_path_node_list(&mut self) -> IntArrayList {
        let mut list = IntArrayList::new();
        let mut node = self.node_right_node[0];
        while node != 1 {
            list.add(node);
            self.mark_same_span_node(node, 1);
            node = self.node_right_node[node as usize];
        }
        list
    }

    /// Equivalent to the private `Lattice.cost(int)`.
    fn cost(&self, node: i32) -> i32 {
        let n = node as usize;
        self.node_left_cost[n]
            .wrapping_add(self.node_word_cost[n])
            .wrapping_add(self.node_right_cost[n])
    }

    /// Equivalent to the package-private `Lattice.nBestNodeList(int)`.
    pub(crate) fn n_best_node_list(&mut self, n_best: i32) -> IntArrayList {
        let mut list = IntArrayList::new();
        let mut least_cost = i32::MAX;
        let mut least_left = -1;
        let mut least_right = -1;
        for node in 2..self.node_count {
            if self.node_mark[node as usize] == 0 {
                let cost = self.cost(node);
                if cost < least_cost {
                    least_cost = cost;
                    least_left = self.node_left[node as usize];
                    least_right = self.node_right[node as usize];
                    list.clear();
                    list.add(node);
                } else if cost == least_cost
                    && (self.node_left[node as usize] != least_left
                        || self.node_right[node as usize] != least_right)
                {
                    list.add(node);
                }
            }
        }
        for node in list.iter() {
            self.mark_same_span_node(node.value, n_best);
        }
        list
    }

    /// Equivalent to the package-private `Lattice.bestCost()`.
    pub(crate) fn best_cost(&self) -> i32 {
        self.node_left_cost[1]
    }

    /// Equivalent to the package-private `Lattice.probeDelta(int, int)`.
    pub(crate) fn probe_delta(&self, start: i32, end: i32) -> i32 {
        let left = start - self.root_base;
        let right = end - self.root_base;
        if left < 0 || self.root_size < right {
            return i32::MAX;
        }
        let mut probed_cost = i32::MAX;
        let mut node = self.l_root[left as usize];
        while 0 <= node {
            if self.node_right[node as usize] == right {
                probed_cost = probed_cost.min(self.cost(node));
            }
            node = self.node_left_chain[node as usize];
        }
        probed_cost.wrapping_sub(self.best_cost())
    }

    /// Equivalent to the package-private `Lattice.debugPrint()`: prints the
    /// nodes to the standard output when [`VERBOSE`].
    pub(crate) fn debug_print(&self) {
        if VERBOSE {
            for node in 0..self.node_count {
                let n = node as usize;
                println!(
                    "DEBUG NODE: node={node}, mark={}, cost={}, left={}, right={}",
                    self.node_mark[n],
                    self.cost(node),
                    self.node_left[n],
                    self.node_right[n]
                );
            }
        }
    }
}
