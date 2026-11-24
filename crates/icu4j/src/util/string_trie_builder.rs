// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.util.StringTrieBuilder`.

use std::collections::HashMap;

use rucene::error::{LuceneError, Result};

/// Build options for `BytesTrieBuilder` and `CharsTrieBuilder`.
///
/// Equivalent to the enum `com.ibm.icu.util.StringTrieBuilder.Option`,
/// prefixed with its enclosing class because `Option` names a type of the
/// Rust prelude.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StringTrieBuilderOption {
    /// `FAST`: builds a trie quickly.
    Fast,
    /// `SMALL`: builds a trie more slowly, attempting to generate a shorter
    /// but equivalent serialization.
    Small,
}

/// The build state (`StringTrieBuilder.State`, private).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Adding,
    BuildingFast,
    BuildingSmall,
    Built,
}

/// The index of a node in the builder's node arena: a Java object
/// reference, compared by identity.
pub(crate) type NodeId = usize;

/// The kind of a node, with the fields of its Java class.
#[derive(Debug, Clone)]
enum NodeKind {
    /// `ValueNode`.
    Value,
    /// `IntermediateValueNode`.
    IntermediateValue { next: NodeId },
    /// `LinearMatchNode`: a run of units of the builder's `strings`.
    LinearMatch {
        string_offset: i32,
        length: i32,
        next: NodeId,
        hash: i32,
    },
    /// `DynamicBranchNode`: the branch units and the node of each.
    DynamicBranch { chars: Vec<u16>, equal: Vec<NodeId> },
    /// `ListBranchNode`.
    ListBranch {
        hash: i32,
        first_edge_number: i32,
        equal: Vec<Option<NodeId>>,
        length: i32,
        values: Vec<i32>,
        units: Vec<u16>,
    },
    /// `SplitBranchNode`.
    SplitBranch {
        hash: i32,
        first_edge_number: i32,
        unit: u16,
        less_than: NodeId,
        greater_or_equal: NodeId,
    },
    /// `BranchHeadNode`.
    BranchHead { length: i32, next: NodeId },
}

/// A node: `Node.offset`, the `ValueNode` fields `hasValue` and `value`
/// (unused by the branch nodes, which do not extend `ValueNode`), and the
/// fields of its class.
#[derive(Debug, Clone)]
struct Node {
    offset: i32,
    has_value: bool,
    value: i32,
    kind: NodeKind,
}

impl Node {
    fn new(kind: NodeKind) -> Self {
        Self {
            offset: 0,
            has_value: false,
            value: 0,
            kind,
        }
    }

    /// `getClass()`, for `Node.equals`.
    fn class_id(&self) -> u8 {
        match self.kind {
            NodeKind::Value => 0,
            NodeKind::IntermediateValue { .. } => 1,
            NodeKind::LinearMatch { .. } => 2,
            NodeKind::DynamicBranch { .. } => 3,
            NodeKind::ListBranch { .. } => 4,
            NodeKind::SplitBranch { .. } => 5,
            NodeKind::BranchHead { .. } => 6,
        }
    }
}

/// Base class for string trie builder classes: the trie under
/// construction, as a graph of nodes deduplicated by value, and the
/// shared `strings` of the linear-match nodes.
///
/// Equivalent to the abstract class `com.ibm.icu.util.StringTrieBuilder`.
/// Its abstract methods, which the serializing subclass implements, are
/// the trait [`StringTrieBuilderSpi`]; its protected `addImpl`, `buildImpl`
/// and `clearImpl` take that subclass.
///
/// **Divergence.** Java's nodes are objects that refer to each other and
/// are compared by identity (`next == o.next`) and, in the deduplicating
/// `HashMap<Node, Node>`, by their `equals`/`hashCode` overrides; here the
/// nodes live in an arena owned by the builder and refer to each other by
/// index, an index standing for a Java reference, and the map is a table of
/// node indexes by hash code whose lookups call the same `equals`, so that
/// the same nodes are shared and the same serialization is written.
#[derive(Debug)]
pub struct StringTrieBuilder {
    state: State,
    /// `strings` (deprecated, protected): strings and sub-strings for
    /// linear-match nodes.
    pub strings: Vec<u16>,
    root: Option<NodeId>,
    /// `nodes`: hash table for finding nodes with the same value.
    nodes: HashMap<i32, Vec<NodeId>>,
    /// `lookupFinalValueNode`.
    lookup_final_value_node: NodeId,
    arena: Vec<Node>,
}

impl Default for StringTrieBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// The abstract methods of `StringTrieBuilder` (protected in Java), which a
/// serializing subclass such as `CharsTrieBuilder` implements, plus access
/// to the base class's state.
pub trait StringTrieBuilderSpi {
    /// The `StringTrieBuilder` part of the subclass.
    fn base(&self) -> &StringTrieBuilder;

    /// The `StringTrieBuilder` part of the subclass, mutably.
    fn base_mut(&mut self) -> &mut StringTrieBuilder;

    /// `matchNodesCanHaveValues()`.
    fn match_nodes_can_have_values(&self) -> bool;

    /// `getMaxBranchLinearSubNodeLength()`.
    fn get_max_branch_linear_sub_node_length(&self) -> i32;

    /// `getMinLinearMatch()`.
    fn get_min_linear_match(&self) -> i32;

    /// `getMaxLinearMatchLength()`.
    fn get_max_linear_match_length(&self) -> i32;

    /// `write(int unit)`: writes one unit; returns the new offset.
    fn write_unit(&mut self, unit: i32) -> i32;

    /// `write(int offset, int length)`: writes `length` units of `strings`
    /// from `offset`; returns the new offset.
    fn write_offset_length(&mut self, offset: i32, length: i32) -> i32;

    /// `writeValueAndFinal(int i, boolean isFinal)`.
    fn write_value_and_final(&mut self, i: i32, is_final: bool) -> i32;

    /// `writeValueAndType(boolean hasValue, int value, int node)`.
    fn write_value_and_type(&mut self, has_value: bool, value: i32, node: i32) -> i32;

    /// `writeDeltaTo(int jumpTarget)`.
    fn write_delta_to(&mut self, jump_target: i32) -> i32;
}

impl StringTrieBuilder {
    /// `StringTrieBuilder()` (deprecated, protected).
    pub fn new() -> Self {
        let mut b = Self {
            state: State::Adding,
            strings: Vec::new(),
            root: None,
            nodes: HashMap::new(),
            lookup_final_value_node: 0,
            arena: Vec::new(),
        };
        b.lookup_final_value_node = b.alloc(Node::new(NodeKind::Value));
        b
    }

    fn alloc(&mut self, node: Node) -> NodeId {
        self.arena.push(node);
        self.arena.len() - 1
    }

    fn node(&self, id: NodeId) -> &Node {
        &self.arena[id]
    }

    fn node_mut(&mut self, id: NodeId) -> &mut Node {
        &mut self.arena[id]
    }

    /// `addImpl(CharSequence s, int value)` (deprecated, protected).
    ///
    /// # Errors
    ///
    /// `IllegalStateException("Cannot add (string, value) pairs after
    /// build().")`; `IllegalArgumentException("Duplicate string.")`.
    ///
    /// # Panics
    ///
    /// `IndexOutOfBoundsException("The maximum string length is 0xffff.")`.
    pub fn add_impl(b: &mut dyn StringTrieBuilderSpi, s: &[u16], value: i32) -> Result<()> {
        if b.base().state != State::Adding {
            // Cannot add elements after building.
            return Err(LuceneError::illegal_state(
                "Cannot add (string, value) pairs after build().",
            ));
        }
        if s.len() > 0xffff {
            // Too long: Limited by iterator internals, and by builder recursion depth.
            crate::jdk::index_out_of_bounds(Some(
                "The maximum string length is 0xffff.".to_string(),
            ));
        }
        let root = match b.base().root {
            None => b.base_mut().create_suffix_node(s, 0, value),
            Some(root) => node_add(b, root, s, 0, value)?,
        };
        b.base_mut().root = Some(root);
        Ok(())
    }

    /// `buildImpl(Option buildOption)` (final, deprecated, protected).
    ///
    /// # Errors
    ///
    /// `IllegalStateException("Builder failed and must be clear()ed.")`.
    ///
    /// # Panics
    ///
    /// `IndexOutOfBoundsException("No (string, value) pairs were added.")`.
    pub fn build_impl(
        b: &mut dyn StringTrieBuilderSpi,
        build_option: StringTrieBuilderOption,
    ) -> Result<()> {
        match b.base().state {
            State::Adding => {
                if b.base().root.is_none() {
                    crate::jdk::index_out_of_bounds(Some(
                        "No (string, value) pairs were added.".to_string(),
                    ));
                }
                if build_option == StringTrieBuilderOption::Fast {
                    b.base_mut().state = State::BuildingFast;
                    // Building "fast" is somewhat faster (25..50% in some test)
                    // because it makes registerNode() return the input node
                    // rather than checking for duplicates.
                    // As a result, we sometimes write larger trie serializations.
                    //
                    // In either case we need to fix-up linear-match nodes (for their maximum length)
                    // and branch nodes (turning dynamic branch nodes into trees of
                    // runtime-equivalent nodes), but the HashMap/hashCode()/equals() are omitted for
                    // nodes other than final values.
                } else {
                    b.base_mut().state = State::BuildingSmall;
                }
            }
            State::BuildingFast | State::BuildingSmall => {
                // Building must have failed.
                return Err(LuceneError::illegal_state(
                    "Builder failed and must be clear()ed.",
                ));
            }
            State::Built => return Ok(()), // Nothing more to do.
        }
        // Implementation note:
        // We really build three versions of the trie.
        // The first is a fully dynamic trie, built successively by addImpl().
        // Then we call root.register() to turn it into a tree of nodes
        // which is 1:1 equivalent to the runtime data structure.
        // Finally, root.markRightEdgesFirst() and root.write() write that serialized form.
        let root = b.base().root.expect("checked above");
        let root = node_register(b, root);
        b.base_mut().root = Some(root);
        node_mark_right_edges_first(b.base_mut(), root, -1);
        node_write(b, root);
        b.base_mut().state = State::Built;
        Ok(())
    }

    /// `clearImpl()` (deprecated, protected).
    pub fn clear_impl(b: &mut dyn StringTrieBuilderSpi) {
        let base = b.base_mut();
        base.strings.clear();
        base.nodes.clear();
        base.root = None;
        base.state = State::Adding;
    }

    /// `registerNode(Node newNode)` (private, final): the equal node already
    /// registered, or `new_node` after registering it.
    fn register_node(&mut self, new_node: NodeId) -> NodeId {
        if self.state == State::BuildingFast {
            return new_node;
        }
        // BUILDING_SMALL
        if let Some(old_node) = self.nodes_get(new_node) {
            return old_node;
        }
        // If put() returns a non-null value from an equivalent, previously
        // registered node, then get() failed to find that and we will leak newNode.
        self.nodes_put(new_node);
        new_node
    }

    /// `registerFinalValue(int value)` (private, final).
    fn register_final_value(&mut self, value: i32) -> NodeId {
        // We always register final values because while ADDING
        // we do not know yet whether we will build fast or small.
        let lookup = self.lookup_final_value_node;
        {
            let n = self.node_mut(lookup);
            // setFinalValue(value)
            n.has_value = true;
            n.value = value;
        }
        if let Some(old_node) = self.nodes_get(lookup) {
            return old_node;
        }
        let mut node = Node::new(NodeKind::Value);
        node.has_value = true;
        node.value = value;
        let new_node = self.alloc(node);
        // If put() returns a non-null value from an equivalent, previously
        // registered node, then get() failed to find that and we will leak newNode.
        self.nodes_put(new_node);
        new_node
    }

    /// `nodes.get(node)`: the registered node equal to `node`.
    fn nodes_get(&self, node: NodeId) -> Option<NodeId> {
        let hash = node_hash_code(self, node);
        self.nodes
            .get(&hash)
            .and_then(|bucket| bucket.iter().copied().find(|&k| node_equals(self, k, node)))
    }

    /// `nodes.put(node, node)` for a node that is not registered.
    fn nodes_put(&mut self, node: NodeId) {
        let hash = node_hash_code(self, node);
        debug_assert!(self.nodes_get(node).is_none());
        self.nodes.entry(hash).or_default().push(node);
    }

    /// `createSuffixNode(CharSequence s, int start, int sValue)` (private):
    /// the final value, preceded by a linear-match node over the rest of
    /// `s` from `start`.
    fn create_suffix_node(&mut self, s: &[u16], start: i32, s_value: i32) -> NodeId {
        let mut node = self.register_final_value(s_value);
        if (start as usize) < s.len() {
            let offset = self.strings.len() as i32;
            self.strings.extend_from_slice(&s[start as usize..]);
            node = self.alloc(Node::new(NodeKind::LinearMatch {
                string_offset: offset,
                length: s.len() as i32 - start,
                next: node,
                hash: 0,
            }));
        }
        node
    }
}

/// `hashCode()` of a node.
fn node_hash_code(b: &StringTrieBuilder, id: NodeId) -> i32 {
    let n = b.node(id);
    match &n.kind {
        NodeKind::Value | NodeKind::DynamicBranch { .. } => {
            let mut hash = 0x111111i32;
            if n.has_value {
                hash = hash.wrapping_mul(37).wrapping_add(n.value);
            }
            hash
        }
        NodeKind::IntermediateValue { next } => {
            (0x222222i32.wrapping_mul(37).wrapping_add(n.value))
                .wrapping_mul(37)
                .wrapping_add(node_hash_code(b, *next))
        }
        NodeKind::LinearMatch { hash, .. } => *hash,
        NodeKind::ListBranch { hash, .. } => *hash,
        NodeKind::SplitBranch { hash, .. } => *hash,
        NodeKind::BranchHead { length, next } => {
            (0x666666i32.wrapping_mul(37).wrapping_add(*length))
                .wrapping_mul(37)
                .wrapping_add(node_hash_code(b, *next))
        }
    }
}

/// `equals(Object other)` of a node, per its class's override.
fn node_equals(b: &StringTrieBuilder, this: NodeId, other: NodeId) -> bool {
    if this == other {
        return true;
    }
    let (t, o) = (b.node(this), b.node(other));
    // Node.equals: this.getClass()==other.getClass()
    if t.class_id() != o.class_id() {
        return false;
    }
    // ValueNode.equals
    let value_equal =
        |t: &Node, o: &Node| t.has_value == o.has_value && (!t.has_value || t.value == o.value);
    match (&t.kind, &o.kind) {
        (NodeKind::Value, NodeKind::Value)
        | (NodeKind::DynamicBranch { .. }, NodeKind::DynamicBranch { .. }) => value_equal(t, o),
        (NodeKind::IntermediateValue { next: tn }, NodeKind::IntermediateValue { next: on }) => {
            value_equal(t, o) && tn == on
        }
        (
            NodeKind::LinearMatch {
                string_offset: ts,
                length: tl,
                next: tn,
                ..
            },
            NodeKind::LinearMatch {
                string_offset: os,
                length: ol,
                next: on,
                ..
            },
        ) => {
            if !value_equal(t, o) {
                return false;
            }
            if tl != ol || tn != on {
                return false;
            }
            let (mut i, mut j) = (*ts, *os);
            let limit = ts + tl;
            while i < limit {
                if b.strings[i as usize] != b.strings[j as usize] {
                    return false;
                }
                i += 1;
                j += 1;
            }
            true
        }
        (
            NodeKind::ListBranch {
                length,
                equal: te,
                values: tv,
                units: tu,
                ..
            },
            NodeKind::ListBranch {
                equal: oe,
                values: ov,
                units: ou,
                ..
            },
        ) => {
            for i in 0..*length as usize {
                if tu[i] != ou[i] || tv[i] != ov[i] || te[i] != oe[i] {
                    return false;
                }
            }
            true
        }
        (
            NodeKind::SplitBranch {
                unit: tu,
                less_than: tl,
                greater_or_equal: tg,
                ..
            },
            NodeKind::SplitBranch {
                unit: ou,
                less_than: ol,
                greater_or_equal: og,
                ..
            },
        ) => tu == ou && tl == ol && tg == og,
        (
            NodeKind::BranchHead {
                length: tl,
                next: tn,
            },
            NodeKind::BranchHead {
                length: ol,
                next: on,
            },
        ) => value_equal(t, o) && tl == ol && tn == on,
        _ => false,
    }
}

/// `ValueNode.setValue(int v)` (final).
fn set_value(b: &mut StringTrieBuilder, id: NodeId, v: i32) {
    let n = b.node_mut(id);
    debug_assert!(!n.has_value);
    n.has_value = true;
    n.value = v;
}

/// `add(StringTrieBuilder builder, CharSequence s, int start, int sValue)`
/// of a node, per its class's override: the node that replaces it.
fn node_add(
    b: &mut dyn StringTrieBuilderSpi,
    id: NodeId,
    s: &[u16],
    start: i32,
    s_value: i32,
) -> Result<NodeId> {
    let kind = b.base().node(id).kind.clone();
    match kind {
        NodeKind::Value => {
            // ValueNode.add
            if start as usize == s.len() {
                return Err(LuceneError::illegal_argument("Duplicate string."));
            }
            // Replace self with a node for the remaining string suffix and value.
            let base = b.base_mut();
            let node = base.create_suffix_node(s, start, s_value);
            let value = base.node(id).value;
            set_value(base, node, value);
            Ok(node)
        }
        NodeKind::LinearMatch {
            string_offset,
            length,
            next,
            ..
        } => linear_match_add(b, id, string_offset, length, next, s, start, s_value),
        NodeKind::DynamicBranch { .. } => {
            // DynamicBranchNode.add
            if start as usize == s.len() {
                if b.base().node(id).has_value {
                    return Err(LuceneError::illegal_argument("Duplicate string."));
                } else {
                    set_value(b.base_mut(), id, s_value);
                    return Ok(id);
                }
            }
            let c = s[start as usize];
            let start = start + 1;
            let i = dynamic_branch_find(b.base(), id, c);
            let existing = match &b.base().node(id).kind {
                NodeKind::DynamicBranch { chars, equal } => {
                    if (i as usize) < chars.len() && c == chars[i as usize] {
                        Some(equal[i as usize])
                    } else {
                        None
                    }
                }
                _ => unreachable!("the node is a DynamicBranchNode"),
            };
            match existing {
                Some(child) => {
                    let new_child = node_add(b, child, s, start, s_value)?;
                    if let NodeKind::DynamicBranch { equal, .. } =
                        &mut b.base_mut().node_mut(id).kind
                    {
                        equal[i as usize] = new_child;
                    }
                }
                None => {
                    let suffix = b.base_mut().create_suffix_node(s, start, s_value);
                    if let NodeKind::DynamicBranch { chars, equal } =
                        &mut b.base_mut().node_mut(id).kind
                    {
                        chars.insert(i as usize, c);
                        equal.insert(i as usize, suffix);
                    }
                }
            }
            Ok(id)
        }
        // Node.add: the other classes do not override it.
        _ => Ok(id),
    }
}

/// `LinearMatchNode.add(StringTrieBuilder builder, CharSequence s, int
/// start, int sValue)`.
#[allow(clippy::too_many_arguments)]
fn linear_match_add(
    b: &mut dyn StringTrieBuilderSpi,
    id: NodeId,
    mut string_offset: i32,
    mut length: i32,
    mut next: NodeId,
    s: &[u16],
    mut start: i32,
    s_value: i32,
) -> Result<NodeId> {
    let set_fields = |b: &mut dyn StringTrieBuilderSpi, so: i32, l: i32, n: NodeId| {
        if let NodeKind::LinearMatch {
            string_offset,
            length,
            next,
            ..
        } = &mut b.base_mut().node_mut(id).kind
        {
            *string_offset = so;
            *length = l;
            *next = n;
        }
    };
    if start as usize == s.len() {
        if b.base().node(id).has_value {
            return Err(LuceneError::illegal_argument("Duplicate string."));
        } else {
            set_value(b.base_mut(), id, s_value);
            return Ok(id);
        }
    }
    let limit = string_offset + length;
    let mut i = string_offset;
    while i < limit {
        if start as usize == s.len() {
            // s is a prefix with a new value. Split self into two linear-match nodes.
            let prefix_length = i - string_offset;
            let base = b.base_mut();
            let suffix_node = base.alloc(Node::new(NodeKind::LinearMatch {
                string_offset: i,
                length: length - prefix_length,
                next,
                hash: 0,
            }));
            set_value(base, suffix_node, s_value);
            length = prefix_length;
            next = suffix_node;
            set_fields(b, string_offset, length, next);
            return Ok(id);
        }
        let this_char = b.base().strings[i as usize];
        let new_char = s[start as usize];
        if this_char != new_char {
            // Mismatch, insert a branch node.
            let branch_node = b.base_mut().alloc(Node::new(NodeKind::DynamicBranch {
                chars: Vec::new(),
                equal: Vec::new(),
            }));
            // Reuse this node for one of the remaining substrings, if any.
            let result;
            let this_suffix_node;
            if i == string_offset {
                // Mismatch on first character, turn this node into a suffix.
                if b.base().node(id).has_value {
                    // Move the value for prefix length "start" to the new node.
                    let value = b.base().node(id).value;
                    set_value(b.base_mut(), branch_node, value);
                    let n = b.base_mut().node_mut(id);
                    n.value = 0;
                    n.has_value = false;
                }
                string_offset += 1;
                length -= 1;
                set_fields(b, string_offset, length, next);
                this_suffix_node = if length > 0 { id } else { next };
                // C++: if(length==0) { delete this; }
                result = branch_node;
            } else if i == limit - 1 {
                // Mismatch on last character, keep this node for the prefix.
                length -= 1;
                this_suffix_node = next;
                next = branch_node;
                set_fields(b, string_offset, length, next);
                result = id;
            } else {
                // Mismatch on intermediate character, keep this node for the prefix.
                let prefix_length = i - string_offset;
                i += 1; // Suffix start offset (after thisChar).
                this_suffix_node = b.base_mut().alloc(Node::new(NodeKind::LinearMatch {
                    string_offset: i,
                    length: length - (prefix_length + 1),
                    next,
                    hash: 0,
                }));
                length = prefix_length;
                next = branch_node;
                set_fields(b, string_offset, length, next);
                result = id;
            }
            let new_suffix_node = b.base_mut().create_suffix_node(s, start + 1, s_value);
            dynamic_branch_add_node(b.base_mut(), branch_node, this_char, this_suffix_node);
            dynamic_branch_add_node(b.base_mut(), branch_node, new_char, new_suffix_node);
            return Ok(result);
        }
        i += 1;
        start += 1;
    }
    // s matches all of this node's characters.
    next = node_add(b, next, s, start, s_value)?;
    set_fields(b, string_offset, length, next);
    Ok(id)
}

/// `DynamicBranchNode.add(char c, Node node)`: inserts `c` and its node
/// at their sorted position.
fn dynamic_branch_add_node(b: &mut StringTrieBuilder, id: NodeId, c: u16, node: NodeId) {
    let i = dynamic_branch_find(b, id, c) as usize;
    if let NodeKind::DynamicBranch { chars, equal } = &mut b.node_mut(id).kind {
        chars.insert(i, c);
        equal.insert(i, node);
    }
}

/// `DynamicBranchNode.find(char c)` (private): binary search.
fn dynamic_branch_find(b: &StringTrieBuilder, id: NodeId, c: u16) -> i32 {
    let NodeKind::DynamicBranch { chars, .. } = &b.node(id).kind else {
        unreachable!("the node is a DynamicBranchNode");
    };
    let mut start = 0i32;
    let mut limit = chars.len() as i32;
    while start < limit {
        let i = (start + limit) / 2;
        let middle_char = chars[i as usize];
        if c < middle_char {
            limit = i;
        } else if c == middle_char {
            return i;
        } else {
            start = i + 1;
        }
    }
    start
}

/// `register(StringTrieBuilder builder)` of a node, per its class's
/// override.
#[allow(clippy::needless_late_init)] // declare-then-assign mirrors the Java source
fn node_register(b: &mut dyn StringTrieBuilderSpi, id: NodeId) -> NodeId {
    let kind = b.base().node(id).kind.clone();
    match kind {
        NodeKind::LinearMatch {
            string_offset,
            mut length,
            next,
            ..
        } => {
            // LinearMatchNode.register
            let mut next = node_register(b, next);
            // Break the linear-match sequence into chunks of at most kMaxLinearMatchLength.
            let max_linear_match_length = b.get_max_linear_match_length();
            while length > max_linear_match_length {
                let next_offset = string_offset + length - max_linear_match_length;
                length -= max_linear_match_length;
                let base = b.base_mut();
                let suffix_node = base.alloc(Node::new(NodeKind::LinearMatch {
                    string_offset: next_offset,
                    length: max_linear_match_length,
                    next,
                    hash: 0,
                }));
                linear_match_set_hash_code(base, suffix_node);
                next = base.register_node(suffix_node);
            }
            if let NodeKind::LinearMatch {
                length: l, next: n, ..
            } = &mut b.base_mut().node_mut(id).kind
            {
                *l = length;
                *n = next;
            }
            let result;
            if b.base().node(id).has_value && !b.match_nodes_can_have_values() {
                let base = b.base_mut();
                let intermediate_value = base.node(id).value;
                {
                    let n = base.node_mut(id);
                    n.value = 0;
                    n.has_value = false;
                }
                linear_match_set_hash_code(base, id);
                let registered = base.register_node(id);
                let mut iv = Node::new(NodeKind::IntermediateValue { next: registered });
                // IntermediateValueNode(int v, Node nextNode): setValue(v)
                iv.has_value = true;
                iv.value = intermediate_value;
                result = base.alloc(iv);
            } else {
                linear_match_set_hash_code(b.base_mut(), id);
                result = id;
            }
            b.base_mut().register_node(result)
        }
        NodeKind::DynamicBranch { chars, .. } => {
            // DynamicBranchNode.register
            let sub_node = dynamic_branch_register_range(b, id, 0, chars.len() as i32);
            let head = b.base_mut().alloc(Node::new(NodeKind::BranchHead {
                length: chars.len() as i32,
                next: sub_node,
            }));
            let mut result = head;
            if b.base().node(id).has_value {
                let value = b.base().node(id).value;
                if b.match_nodes_can_have_values() {
                    set_value(b.base_mut(), head, value);
                } else {
                    let base = b.base_mut();
                    let registered = base.register_node(head);
                    let mut iv = Node::new(NodeKind::IntermediateValue { next: registered });
                    iv.has_value = true;
                    iv.value = value;
                    result = base.alloc(iv);
                }
            }
            b.base_mut().register_node(result)
        }
        // Node.register: the other classes return this.
        _ => id,
    }
}

/// `DynamicBranchNode.register(StringTrieBuilder builder, int start, int
/// limit)` (private).
fn dynamic_branch_register_range(
    b: &mut dyn StringTrieBuilderSpi,
    id: NodeId,
    mut start: i32,
    limit: i32,
) -> NodeId {
    let (chars, equal) = match &b.base().node(id).kind {
        NodeKind::DynamicBranch { chars, equal } => (chars.clone(), equal.clone()),
        _ => unreachable!("the node is a DynamicBranchNode"),
    };
    let length = limit - start;
    if length > b.get_max_branch_linear_sub_node_length() {
        // Branch on the middle unit.
        let middle = start + length / 2;
        let less_than = dynamic_branch_register_range(b, id, start, middle);
        let greater_or_equal = dynamic_branch_register_range(b, id, middle, limit);
        let base = b.base_mut();
        let middle_unit = chars[middle as usize];
        let hash = ((0x555555i32
            .wrapping_mul(37)
            .wrapping_add(middle_unit as i32))
        .wrapping_mul(37)
        .wrapping_add(node_hash_code(base, less_than)))
        .wrapping_mul(37)
        .wrapping_add(node_hash_code(base, greater_or_equal));
        let split = base.alloc(Node::new(NodeKind::SplitBranch {
            hash,
            first_edge_number: 0,
            unit: middle_unit,
            less_than,
            greater_or_equal,
        }));
        return base.register_node(split);
    }
    let list_node = b.base_mut().alloc(Node::new(NodeKind::ListBranch {
        hash: 0x444444i32.wrapping_mul(37).wrapping_add(length),
        first_edge_number: 0,
        equal: vec![None; length as usize],
        length: 0,
        values: vec![0; length as usize],
        units: vec![0; length as usize],
    }));
    loop {
        let c = chars[start as usize];
        let node = equal[start as usize];
        if matches!(b.base().node(node).kind, NodeKind::Value) {
            // Final value.
            let value = b.base().node(node).value;
            list_branch_add_value(b.base_mut(), list_node, c as i32, value);
        } else {
            let registered = node_register(b, node);
            list_branch_add_node(b.base_mut(), list_node, c as i32, registered);
        }
        start += 1;
        if start >= limit {
            break;
        }
    }
    b.base_mut().register_node(list_node)
}

/// `ListBranchNode.add(int c, int value)`: adds a unit with a final value.
fn list_branch_add_value(b: &mut StringTrieBuilder, id: NodeId, c: i32, value: i32) {
    if let NodeKind::ListBranch {
        hash,
        equal,
        length,
        values,
        units,
        ..
    } = &mut b.node_mut(id).kind
    {
        let l = *length as usize;
        units[l] = c as u16;
        equal[l] = None;
        values[l] = value;
        *length += 1;
        *hash = (hash.wrapping_mul(37).wrapping_add(c))
            .wrapping_mul(37)
            .wrapping_add(value);
    }
}

/// `ListBranchNode.add(int c, Node node)`: adds a unit with a sub-node.
fn list_branch_add_node(b: &mut StringTrieBuilder, id: NodeId, c: i32, node: NodeId) {
    let node_hash = node_hash_code(b, node);
    if let NodeKind::ListBranch {
        hash,
        equal,
        length,
        values,
        units,
        ..
    } = &mut b.node_mut(id).kind
    {
        let l = *length as usize;
        units[l] = c as u16;
        equal[l] = Some(node);
        values[l] = 0;
        *length += 1;
        *hash = (hash.wrapping_mul(37).wrapping_add(c))
            .wrapping_mul(37)
            .wrapping_add(node_hash);
    }
}

/// `LinearMatchNode.setHashCode()` (private).
fn linear_match_set_hash_code(b: &mut StringTrieBuilder, id: NodeId) {
    let (string_offset, length, next) = match b.node(id).kind {
        NodeKind::LinearMatch {
            string_offset,
            length,
            next,
            ..
        } => (string_offset, length, next),
        _ => unreachable!("the node is a LinearMatchNode"),
    };
    let mut h = (0x333333i32.wrapping_mul(37).wrapping_add(length))
        .wrapping_mul(37)
        .wrapping_add(node_hash_code(b, next));
    let n = b.node(id);
    if n.has_value {
        h = h.wrapping_mul(37).wrapping_add(n.value);
    }
    for i in string_offset..string_offset + length {
        h = h
            .wrapping_mul(37)
            .wrapping_add(b.strings[i as usize] as i32);
    }
    if let NodeKind::LinearMatch { hash, .. } = &mut b.node_mut(id).kind {
        *hash = h;
    }
}

/// `markRightEdgesFirst(int edgeNumber)` of a node, per its class's
/// override.
fn node_mark_right_edges_first(b: &mut StringTrieBuilder, id: NodeId, mut edge_number: i32) -> i32 {
    let kind = b.node(id).kind.clone();
    let offset = b.node(id).offset;
    match kind {
        NodeKind::IntermediateValue { next }
        | NodeKind::LinearMatch { next, .. }
        | NodeKind::BranchHead { next, .. } => {
            if offset == 0 {
                edge_number = node_mark_right_edges_first(b, next, edge_number);
                b.node_mut(id).offset = edge_number;
            }
            edge_number
        }
        NodeKind::ListBranch { equal, length, .. } => {
            if offset == 0 {
                if let NodeKind::ListBranch {
                    first_edge_number, ..
                } = &mut b.node_mut(id).kind
                {
                    *first_edge_number = edge_number;
                }
                let mut step = 0;
                let mut i = length;
                loop {
                    i -= 1;
                    if let Some(edge) = equal[i as usize] {
                        edge_number = node_mark_right_edges_first(b, edge, edge_number - step);
                    }
                    // For all but the rightmost edge, decrement the edge number.
                    step = 1;
                    if i <= 0 {
                        break;
                    }
                }
                b.node_mut(id).offset = edge_number;
            }
            edge_number
        }
        NodeKind::SplitBranch {
            less_than,
            greater_or_equal,
            ..
        } => {
            if offset == 0 {
                if let NodeKind::SplitBranch {
                    first_edge_number, ..
                } = &mut b.node_mut(id).kind
                {
                    *first_edge_number = edge_number;
                }
                edge_number = node_mark_right_edges_first(b, greater_or_equal, edge_number);
                edge_number = node_mark_right_edges_first(b, less_than, edge_number - 1);
                b.node_mut(id).offset = edge_number;
            }
            edge_number
        }
        // Node.markRightEdgesFirst
        NodeKind::Value | NodeKind::DynamicBranch { .. } => {
            if offset == 0 {
                b.node_mut(id).offset = edge_number;
            }
            edge_number
        }
    }
}

/// `writeUnlessInsideRightEdge(int firstRight, int lastRight,
/// StringTrieBuilder builder)` (final): writes the node unless it is part
/// of the right edge of the branch being written.
fn node_write_unless_inside_right_edge(
    b: &mut dyn StringTrieBuilderSpi,
    id: NodeId,
    first_right: i32,
    last_right: i32,
) {
    // Note: Edge numbers are negative, lastRight<=firstRight.
    // If offset>0 then this node and its sub-nodes have been written already
    // and we need not write them again.
    // If this node is part of the unwritten right branch edge,
    // then we wait until that is written.
    let offset = b.base().node(id).offset;
    if offset < 0 && (offset < last_right || first_right < offset) {
        node_write(b, id);
    }
}

/// `write(StringTrieBuilder builder)` of a node, per its class's override.
#[allow(clippy::needless_late_init)] // declare-then-assign mirrors the Java source
fn node_write(b: &mut dyn StringTrieBuilderSpi, id: NodeId) {
    let node = b.base().node(id).clone();
    match node.kind {
        NodeKind::Value | NodeKind::DynamicBranch { .. } => {
            // ValueNode.write
            let offset = b.write_value_and_final(node.value, true);
            b.base_mut().node_mut(id).offset = offset;
        }
        NodeKind::IntermediateValue { next } => {
            node_write(b, next);
            let offset = b.write_value_and_final(node.value, false);
            b.base_mut().node_mut(id).offset = offset;
        }
        NodeKind::LinearMatch {
            string_offset,
            length,
            next,
            ..
        } => {
            node_write(b, next);
            b.write_offset_length(string_offset, length);
            let t = b.get_min_linear_match() + length - 1;
            let offset = b.write_value_and_type(node.has_value, node.value, t);
            b.base_mut().node_mut(id).offset = offset;
        }
        NodeKind::ListBranch {
            first_edge_number,
            equal,
            length,
            values,
            units,
            ..
        } => {
            // Write the sub-nodes in reverse order: The jump lengths are deltas from
            // after their own positions, so if we wrote the minUnit sub-node first,
            // then its jump delta would be larger.
            // Instead we write the minUnit sub-node last, for a shorter delta.
            let mut unit_number = length - 1;
            let right_edge = equal[unit_number as usize];
            let right_edge_number = match right_edge {
                None => first_edge_number,
                Some(r) => b.base().node(r).offset,
            };
            loop {
                unit_number -= 1;
                if let Some(e) = equal[unit_number as usize] {
                    node_write_unless_inside_right_edge(b, e, first_edge_number, right_edge_number);
                }
                if unit_number <= 0 {
                    break;
                }
            }
            // The maxUnit sub-node is written as the very last one because we do
            // not jump for it at all.
            unit_number = length - 1;
            match right_edge {
                None => {
                    b.write_value_and_final(values[unit_number as usize], true);
                }
                Some(r) => node_write(b, r),
            }
            let mut offset = b.write_unit(units[unit_number as usize] as i32);
            b.base_mut().node_mut(id).offset = offset;
            // Write the rest of this node's unit-value pairs.
            loop {
                unit_number -= 1;
                if unit_number < 0 {
                    break;
                }
                let value;
                let is_final;
                match equal[unit_number as usize] {
                    None => {
                        // Write the final value for the one string ending with this unit.
                        value = values[unit_number as usize];
                        is_final = true;
                    }
                    Some(e) => {
                        // Write the delta to the start position of the sub-node.
                        let e_offset = b.base().node(e).offset;
                        debug_assert!(e_offset > 0);
                        value = offset - e_offset;
                        is_final = false;
                    }
                }
                b.write_value_and_final(value, is_final);
                offset = b.write_unit(units[unit_number as usize] as i32);
                b.base_mut().node_mut(id).offset = offset;
            }
        }
        NodeKind::SplitBranch {
            first_edge_number,
            unit,
            less_than,
            greater_or_equal,
            ..
        } => {
            // Encode the less-than branch first.
            let ge_offset = b.base().node(greater_or_equal).offset;
            node_write_unless_inside_right_edge(b, less_than, first_edge_number, ge_offset);
            // Encode the greater-or-equal branch last because we do not jump for it at all.
            node_write(b, greater_or_equal);
            // Write this node.
            let lt_offset = b.base().node(less_than).offset;
            debug_assert!(lt_offset > 0);
            b.write_delta_to(lt_offset);
            let offset = b.write_unit(unit as i32);
            b.base_mut().node_mut(id).offset = offset;
        }
        NodeKind::BranchHead { length, next } => {
            node_write(b, next);
            let offset = if length <= b.get_min_linear_match() {
                b.write_value_and_type(node.has_value, node.value, length - 1)
            } else {
                b.write_unit(length - 1);
                b.write_value_and_type(node.has_value, node.value, 0)
            };
            b.base_mut().node_mut(id).offset = offset;
        }
    }
}
