//! `TrieReader` ported from `org.apache.lucene.codecs.lucene103.blocktree`.
//!
//! Walks the term-index trie stored in the `.tip` file: each node carries a
//! label, an optional output file pointer into the `.tim` file, and the encoded
//! location of its children.

use crate::error::{LuceneError, Result};
use crate::store::RandomAccessInput;

/// Sentinel for a node that carries no output pointer.
pub const NO_OUTPUT: i64 = -1;
/// Sentinel for a node whose block is not a floor block.
pub const NO_FLOOR_DATA: i64 = -1;

/// A node with no children.
pub const SIGN_NO_CHILDREN: i32 = 0x00;
/// A node with exactly one child and an output pointer.
pub const SIGN_SINGLE_CHILD_WITH_OUTPUT: i32 = 0x01;
/// A node with exactly one child and no output pointer.
pub const SIGN_SINGLE_CHILD_WITHOUT_OUTPUT: i32 = 0x02;
/// A node with several children.
pub const SIGN_MULTI_CHILDREN: i32 = 0x03;

/// Flag bit marking a leaf node whose block holds terms.
pub const LEAF_NODE_HAS_TERMS: i32 = 1 << 5;
/// Flag bit marking a leaf node whose block is a floor block.
pub const LEAF_NODE_HAS_FLOOR: i32 = 1 << 6;
/// Flag bit marking a non-leaf node whose block holds terms.
pub const NON_LEAF_NODE_HAS_TERMS: i64 = 1 << 1;
/// Flag bit marking a non-leaf node whose block is a floor block.
pub const NON_LEAF_NODE_HAS_FLOOR: i64 = 1;

/// Masks selecting the low `n + 1` bytes of a long.
const BYTES_MINUS_1_MASK: [u64; 8] = [
    0xFF,
    0xFFFF,
    0xFF_FFFF,
    0xFFFF_FFFF,
    0xFF_FFFF_FFFF,
    0xFFFF_FFFF_FFFF,
    0xFF_FFFF_FFFF_FFFF,
    0xFFFF_FFFF_FFFF_FFFF,
];

/// How a node's child labels are laid out, which decides how a label is looked
/// up.
///
/// Equivalent to `TrieBuilder.ChildSaveStrategy`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChildSaveStrategy {
    /// Labels beyond the first are stored one byte each, ascending.
    Array,
    /// A presence bitset over the label range.
    Bits,
    /// The maximum label, then the labels that are *absent* from the range.
    ReverseArray,
}

impl ChildSaveStrategy {
    /// Returns the strategy a node's header code names.
    ///
    /// Equivalent to `ChildSaveStrategy.byCode(int)`.
    pub fn by_code(code: i32) -> Result<Self> {
        match code {
            0 => Ok(Self::ReverseArray),
            1 => Ok(Self::Array),
            2 => Ok(Self::Bits),
            _ => Err(LuceneError::IllegalArgument(format!(
                "illegal child save strategy code: {code}"
            ))),
        }
    }

    /// Returns the position of `target_label` among the node's children, or
    /// `None` when the node has no such child.
    ///
    /// Equivalent to `ChildSaveStrategy.lookup`.
    pub fn lookup(
        self,
        target_label: i32,
        input: &mut dyn RandomAccessInput,
        offset: i64,
        strategy_bytes: i32,
        min_label: i32,
    ) -> Result<Option<i32>> {
        match self {
            Self::Bits => {
                let bit_index = target_label - min_label;
                if bit_index >= (strategy_bytes << 3) {
                    return Ok(None);
                }
                let word_index = bit_index >> 6;
                let word_fp = offset + i64::from(word_index << 3);
                let word = input.read_long_at(word_fp)? as u64;
                let mask = 1u64 << bit_index;
                if word & mask == 0 {
                    return Ok(None);
                }
                let mut pos = 0i32;
                let mut fp = offset;
                while fp < word_fp {
                    pos += (input.read_long_at(fp)? as u64).count_ones() as i32;
                    fp += 8;
                }
                pos += (word & (mask - 1)).count_ones() as i32;
                Ok(Some(pos))
            }
            Self::Array => {
                let mut low = 0i32;
                let mut high = strategy_bytes - 1;
                while low <= high {
                    let mid = (low + high) >> 1;
                    let mid_label = i32::from(input.read_byte_at(offset + i64::from(mid))?);
                    match mid_label.cmp(&target_label) {
                        std::cmp::Ordering::Less => low = mid + 1,
                        std::cmp::Ordering::Greater => high = mid - 1,
                        std::cmp::Ordering::Equal => return Ok(Some(mid + 1)),
                    }
                }
                Ok(None)
            }
            Self::ReverseArray => {
                let mut offset = offset;
                let max_label = i32::from(input.read_byte_at(offset)?);
                offset += 1;
                if target_label >= max_label {
                    return Ok(if target_label == max_label {
                        Some(max_label - min_label - strategy_bytes + 1)
                    } else {
                        None
                    });
                }
                if strategy_bytes == 1 {
                    return Ok(Some(target_label - min_label));
                }
                // The stored bytes are the labels that are *missing* from the
                // range, so a hit means the child is absent.
                let mut low = 0i32;
                let mut high = strategy_bytes - 2;
                while low <= high {
                    let mid = (low + high) >> 1;
                    let mid_label = i32::from(input.read_byte_at(offset + i64::from(mid))?);
                    match mid_label.cmp(&target_label) {
                        std::cmp::Ordering::Less => low = mid + 1,
                        std::cmp::Ordering::Greater => high = mid - 1,
                        std::cmp::Ordering::Equal => return Ok(None),
                    }
                }
                Ok(Some(target_label - min_label - low))
            }
        }
    }
}

/// One node of the term-index trie.
///
/// Equivalent to `TrieReader.Node`.
#[derive(Clone, Copy, Debug, Default)]
pub struct Node {
    /// Distance back to the single child, for a single-child node.
    child_delta_fp: i64,
    /// Where the child-lookup data starts, for a multi-child node.
    strategy_fp: i64,
    /// Which lookup strategy the child labels use.
    child_save_strategy: i32,
    /// How many bytes the child-lookup data occupies.
    strategy_bytes: i32,
    /// How many bytes each child's delta pointer occupies.
    children_delta_fp_bytes: i32,
    /// Which of the four node shapes this is.
    sign: i32,
    /// This node's own position in the `.tip` file.
    fp: i64,
    /// The smallest child label.
    min_children_label: i32,
    /// The label on the arc that reached this node.
    pub label: i32,
    /// Position of this node's block in the `.tim` file, or [`NO_OUTPUT`].
    pub output_fp: i64,
    /// Whether the block this node points at holds terms.
    pub has_terms: bool,
    /// Where the floor data starts, or [`NO_FLOOR_DATA`].
    pub floor_data_fp: i64,
}

impl Node {
    /// Returns whether the node points at a block.
    ///
    /// Equivalent to `Node.hasOutput()`.
    pub fn has_output(&self) -> bool {
        self.output_fp != NO_OUTPUT
    }

    /// Returns whether the block this node points at is a floor block.
    ///
    /// Equivalent to `Node.isFloor()`.
    pub fn is_floor(&self) -> bool {
        self.floor_data_fp != NO_FLOOR_DATA
    }
}

/// Reads the term-index trie of one field.
///
/// Equivalent to `org.apache.lucene.codecs.lucene103.blocktree.TrieReader`.
pub struct TrieReader {
    input: Box<dyn RandomAccessInput>,
    /// The trie's root node, loaded on construction.
    pub root: Node,
}

impl std::fmt::Debug for TrieReader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TrieReader")
            .field("root", &self.root)
            .finish_non_exhaustive()
    }
}

impl TrieReader {
    /// Opens the trie whose root sits at `root_fp` in `input`.
    pub fn new(mut input: Box<dyn RandomAccessInput>, root_fp: i64) -> Result<Self> {
        let mut root = Node::default();
        Self::load_node(input.as_mut(), &mut root, root_fp)?;
        Ok(Self { input, root })
    }

    /// Returns the underlying input, for reading a node's floor data.
    pub fn input(&mut self) -> &mut dyn RandomAccessInput {
        self.input.as_mut()
    }

    /// Reads the floor data of `node` into a byte vector.
    ///
    /// Equivalent to `Node.floorData(TrieReader)`, which in Java simply seeks
    /// the shared input; here the bytes are copied out so the caller does not
    /// hold the input across a later seek.
    pub fn floor_data(&mut self, node: &Node, len: usize) -> Result<Vec<u8>> {
        if !node.is_floor() {
            return Err(LuceneError::IllegalState(
                "node has no floor data".to_string(),
            ));
        }
        let mut buf = vec![0u8; len];
        self.input
            .read_bytes_at(node.floor_data_fp, &mut buf, 0, len)?;
        Ok(buf)
    }

    /// Loads the node stored at `fp`.
    ///
    /// Equivalent to `TrieReader.load`.
    fn load_node(input: &mut dyn RandomAccessInput, node: &mut Node, fp: i64) -> Result<()> {
        node.fp = fp;
        let term_flags_long = input.read_long_at(fp)?;
        let term_flags = term_flags_long as i32;
        node.sign = term_flags & 0x03;
        match node.sign {
            SIGN_NO_CHILDREN => Self::load_leaf_node(input, node, fp, term_flags, term_flags_long),
            SIGN_MULTI_CHILDREN => {
                Self::load_multi_children_node(input, node, fp, term_flags, term_flags_long)
            }
            sign => {
                Self::load_single_child_node(input, node, fp, sign, term_flags, term_flags_long)
            }
        }
    }

    /// Layout: `[floor data] [output fp] [1b x | 1b floor | 1b terms | 3b fp bytes | 2b sign]`.
    fn load_leaf_node(
        input: &mut dyn RandomAccessInput,
        node: &mut Node,
        fp: i64,
        term: i32,
        term_long: i64,
    ) -> Result<()> {
        let fp_bytes_minus_1 = ((term >> 2) & 0x07) as usize;
        node.output_fp = if fp_bytes_minus_1 <= 6 {
            (((term_long as u64) >> 8) & BYTES_MINUS_1_MASK[fp_bytes_minus_1]) as i64
        } else {
            input.read_long_at(fp + 1)?
        };
        node.has_terms = term & LEAF_NODE_HAS_TERMS != 0;
        node.floor_data_fp = if term & LEAF_NODE_HAS_FLOOR != 0 {
            fp + 2 + fp_bytes_minus_1 as i64
        } else {
            NO_FLOOR_DATA
        };
        Ok(())
    }

    /// Layout: `[floor data] [encoded output fp] [child fp] [1B label]
    /// [3b output fp bytes | 3b child fp bytes | 2b sign]`.
    fn load_single_child_node(
        input: &mut dyn RandomAccessInput,
        node: &mut Node,
        fp: i64,
        sign: i32,
        term: i32,
        term_long: i64,
    ) -> Result<()> {
        let child_delta_fp_bytes_minus_1 = ((term >> 2) & 0x07) as usize;
        let l = if child_delta_fp_bytes_minus_1 <= 5 {
            (term_long as u64) >> 16
        } else {
            input.read_long_at(fp + 2)? as u64
        };
        node.child_delta_fp = (l & BYTES_MINUS_1_MASK[child_delta_fp_bytes_minus_1]) as i64;
        node.min_children_label = (term >> 8) & 0xFF;

        if sign == SIGN_SINGLE_CHILD_WITHOUT_OUTPUT {
            node.output_fp = NO_OUTPUT;
            node.floor_data_fp = NO_FLOOR_DATA;
            node.has_terms = false;
            return Ok(());
        }
        if sign != SIGN_SINGLE_CHILD_WITH_OUTPUT {
            return Err(LuceneError::corrupt_index(
                format!("unexpected trie node sign: {sign}"),
                "term index",
            ));
        }
        let encoded_output_fp_bytes_minus_1 = ((term >> 5) & 0x07) as usize;
        let offset = fp + child_delta_fp_bytes_minus_1 as i64 + 3;
        let encoded_fp = (input.read_long_at(offset)? as u64)
            & BYTES_MINUS_1_MASK[encoded_output_fp_bytes_minus_1];
        node.output_fp = (encoded_fp >> 2) as i64;
        node.has_terms = encoded_fp as i64 & NON_LEAF_NODE_HAS_TERMS != 0;
        node.floor_data_fp = if encoded_fp as i64 & NON_LEAF_NODE_HAS_FLOOR != 0 {
            offset + encoded_output_fp_bytes_minus_1 as i64 + 1
        } else {
            NO_FLOOR_DATA
        };
        Ok(())
    }

    /// Layout: `[floor data] [children fps] [strategy data] [1B children count if floor]
    /// [encoded output fp] [1B label] [5b strategy bytes | 2b strategy | 3b output fp bytes
    /// | 1b has output | 3b children fp bytes | 2b sign]`.
    fn load_multi_children_node(
        input: &mut dyn RandomAccessInput,
        node: &mut Node,
        fp: i64,
        term: i32,
        term_long: i64,
    ) -> Result<()> {
        node.children_delta_fp_bytes = ((term >> 2) & 0x07) + 1;
        node.child_save_strategy = (term >> 9) & 0x03;
        node.strategy_bytes = ((term >> 11) & 0x1F) + 1;
        node.min_children_label = (term >> 16) & 0xFF;

        if term & 0x20 == 0 {
            node.output_fp = NO_OUTPUT;
            node.has_terms = false;
            node.floor_data_fp = NO_FLOOR_DATA;
            node.strategy_fp = fp + 3;
            return Ok(());
        }

        let encoded_output_fp_bytes_minus_1 = ((term >> 6) & 0x07) as usize;
        let l = if encoded_output_fp_bytes_minus_1 <= 4 {
            (term_long as u64) >> 24
        } else {
            input.read_long_at(fp + 3)? as u64
        };
        let encoded_fp = l & BYTES_MINUS_1_MASK[encoded_output_fp_bytes_minus_1];
        node.output_fp = (encoded_fp >> 2) as i64;
        node.has_terms = encoded_fp as i64 & NON_LEAF_NODE_HAS_TERMS != 0;

        if encoded_fp as i64 & NON_LEAF_NODE_HAS_FLOOR != 0 {
            let offset = fp + 4 + encoded_output_fp_bytes_minus_1 as i64;
            let children_num = i64::from(input.read_byte_at(offset)?) + 1;
            node.strategy_fp = offset + 1;
            node.floor_data_fp = node.strategy_fp
                + i64::from(node.strategy_bytes)
                + children_num * i64::from(node.children_delta_fp_bytes);
        } else {
            node.floor_data_fp = NO_FLOOR_DATA;
            node.strategy_fp = fp + 4 + encoded_output_fp_bytes_minus_1 as i64;
        }
        Ok(())
    }

    /// Follows the arc labelled `target_label` out of `parent`, returning the
    /// child node or `None` when there is no such arc.
    ///
    /// Equivalent to `TrieReader.lookupChild`.
    pub fn lookup_child(&mut self, target_label: i32, parent: &Node) -> Result<Option<Node>> {
        let sign = parent.sign;
        if sign == SIGN_NO_CHILDREN {
            return Ok(None);
        }

        let mut child = Node::default();
        if sign != SIGN_MULTI_CHILDREN {
            if target_label != parent.min_children_label {
                return Ok(None);
            }
            child.label = target_label;
            Self::load_node(
                self.input.as_mut(),
                &mut child,
                parent.fp - parent.child_delta_fp,
            )?;
            return Ok(Some(child));
        }

        let min_label = parent.min_children_label;
        let position = if target_label == min_label {
            Some(0)
        } else if target_label > min_label {
            ChildSaveStrategy::by_code(parent.child_save_strategy)?.lookup(
                target_label,
                self.input.as_mut(),
                parent.strategy_fp,
                parent.strategy_bytes,
                min_label,
            )?
        } else {
            None
        };

        let Some(position) = position else {
            return Ok(None);
        };

        let bytes_per_entry = parent.children_delta_fp_bytes;
        let pos = parent.strategy_fp
            + i64::from(parent.strategy_bytes)
            + i64::from(bytes_per_entry) * i64::from(position);
        let delta = (self.input.read_long_at(pos)? as u64)
            & BYTES_MINUS_1_MASK[(bytes_per_entry - 1) as usize];
        child.label = target_label;
        Self::load_node(self.input.as_mut(), &mut child, parent.fp - delta as i64)?;
        Ok(Some(child))
    }
}
