// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.text.RBBINode`: the nodes of the parse trees of the RBBI
//! rule compiler.

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;

use rucene::error::{LuceneError, Result};

use crate::r#impl::assert::Assert;
use crate::text::unicode_set::UnicodeSet;

/// The index of a node in its [`RBBINodes`] arena.
pub(crate) type NodeId = usize;

/// `setRef`.
pub(crate) const SET_REF: i32 = 0;
/// `uset`.
pub(crate) const USET: i32 = 1;
/// `varRef`.
pub(crate) const VAR_REF: i32 = 2;
/// `leafChar`.
pub(crate) const LEAF_CHAR: i32 = 3;
/// `lookAhead`.
pub(crate) const LOOK_AHEAD: i32 = 4;
/// `tag`.
pub(crate) const TAG: i32 = 5;
/// `endMark`.
pub(crate) const END_MARK: i32 = 6;
/// `opStart`.
pub(crate) const OP_START: i32 = 7;
/// `opCat`.
pub(crate) const OP_CAT: i32 = 8;
/// `opOr`.
pub(crate) const OP_OR: i32 = 9;
/// `opStar`.
pub(crate) const OP_STAR: i32 = 10;
/// `opPlus`.
pub(crate) const OP_PLUS: i32 = 11;
/// `opQuestion`.
pub(crate) const OP_QUESTION: i32 = 12;
/// `opBreak`.
#[allow(dead_code)] // as in ICU, only named by `nodeTypeNames`
pub(crate) const OP_BREAK: i32 = 13;
/// `opReverse`.
#[allow(dead_code)] // as in ICU, only named by `nodeTypeNames`
pub(crate) const OP_REVERSE: i32 = 14;
/// `opLParen`.
pub(crate) const OP_L_PAREN: i32 = 15;
/// `nodeTypeLimit`: for Assertion checking only.
pub(crate) const NODE_TYPE_LIMIT: i32 = 16;

/// `nodeTypeNames`.
pub(crate) const NODE_TYPE_NAMES: [&str; 16] = [
    "setRef",
    "uset",
    "varRef",
    "leafChar",
    "lookAhead",
    "tag",
    "endMark",
    "opStart",
    "opCat",
    "opOr",
    "opStar",
    "opPlus",
    "opQuestion",
    "opBreak",
    "opReverse",
    "opLParen",
];

// enum OpPrecedence {
/// `precZero`.
pub(crate) const PREC_ZERO: i32 = 0;
/// `precStart`.
pub(crate) const PREC_START: i32 = 1;
/// `precLParen`.
pub(crate) const PREC_L_PAREN: i32 = 2;
/// `precOpOr`.
pub(crate) const PREC_OP_OR: i32 = 3;
/// `precOpCat`.
pub(crate) const PREC_OP_CAT: i32 = 4;

/// `kRecursiveDepthLimit`.
const K_RECURSIVE_DEPTH_LIMIT: i32 = 3500;

/// `gLastSerial`: the serial number of the last node created.
static G_LAST_SERIAL: AtomicI32 = AtomicI32::new(0);

/// `++gLastSerial`.
fn next_serial() -> i32 {
    G_LAST_SERIAL.fetch_add(1, Ordering::Relaxed) + 1
}

/// A node in a parse tree of the RBBI rule compiler.
///
/// Equivalent to the package-private class `com.ibm.icu.text.RBBINode`.
///
/// **Divergence.** Java nodes point at each other (parent, children, and
/// the `HashSet<RBBINode>` position sets); the port keeps every node of one
/// compilation in an arena ([`RBBINodes`]) and links them by index. The
/// position sets hash nodes by identity in Java (`RBBINode` does not
/// override `hashCode`), so their iteration order is unspecified and
/// varies between runs; ICU only unions, compares and tests them, and the
/// port keeps them ordered by node index.
#[derive(Debug, Clone)]
pub(crate) struct RBBINode {
    /// `fType`.
    pub(crate) f_type: i32,
    /// `fParent`.
    pub(crate) f_parent: Option<NodeId>,
    /// `fLeftChild`.
    pub(crate) f_left_child: Option<NodeId>,
    /// `fRightChild`.
    pub(crate) f_right_child: Option<NodeId>,
    /// `fInputSet`: shared with copies of the node, as in Java.
    pub(crate) f_input_set: Option<Arc<UnicodeSet>>,
    /// `fPrecedence`.
    pub(crate) f_precedence: i32,
    /// `fText`: text corresponding to this node. May be lazily evaluated
    /// when (if) needed for some types of nodes.
    pub(crate) f_text: Option<Vec<u16>>,
    /// `fFirstPos`: position in the rule source string of the first text
    /// associated with the node. If there's a left child, this will be the
    /// same as that child's left pos.
    pub(crate) f_first_pos: i32,
    /// `fLastPos`: last position in the rule source string of any text
    /// associated with this node. If there's a right child, this will be
    /// the same as that child's last position.
    pub(crate) f_last_pos: i32,
    /// `fNullable`: see Aho DFA table generation algorithm.
    pub(crate) f_nullable: bool,
    /// `fVal`: value field usage depends on the node type. For leafChar:
    /// the character category number; for tag: the integer value of the
    /// tag.
    pub(crate) f_val: i32,
    /// `fLookAheadEnd`: for endMark nodes, set true if marking the end of a
    /// look-ahead rule.
    pub(crate) f_look_ahead_end: bool,
    /// `fRuleRoot`: true if this node is the root of a rule.
    pub(crate) f_rule_root: bool,
    /// `fChainIn`: true if chaining into this rule is allowed (no '^'
    /// present).
    pub(crate) f_chain_in: bool,
    /// `fFirstPosSet`.
    pub(crate) f_first_pos_set: BTreeSet<NodeId>,
    /// `fLastPosSet`.
    pub(crate) f_last_pos_set: BTreeSet<NodeId>,
    /// `fFollowPos`.
    pub(crate) f_follow_pos: BTreeSet<NodeId>,
    /// `fSerialNum`: debugging aid.
    pub(crate) f_serial_num: i32,
}

/// The nodes of one rule compilation, linked by index.
#[derive(Debug, Default)]
pub(crate) struct RBBINodes {
    nodes: Vec<RBBINode>,
}

impl std::ops::Index<NodeId> for RBBINodes {
    type Output = RBBINode;

    fn index(&self, id: NodeId) -> &RBBINode {
        &self.nodes[id]
    }
}

impl std::ops::IndexMut<NodeId> for RBBINodes {
    fn index_mut(&mut self, id: NodeId) -> &mut RBBINode {
        &mut self.nodes[id]
    }
}

/// `n.fLeftChild` of a node whose child Java dereferences.
///
/// # Panics
///
/// `NullPointerException` when the child is absent.
pub(crate) fn child(c: Option<NodeId>) -> NodeId {
    c.unwrap_or_else(|| panic!("java.lang.NullPointerException: RBBINode child"))
}

impl RBBINodes {
    /// An empty arena.
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// `new RBBINode(int t)`.
    ///
    /// # Errors
    ///
    /// `IllegalStateException` (`Assert.assrt`) for a type out of range.
    pub(crate) fn new_node(&mut self, t: i32) -> Result<NodeId> {
        Assert::assrt(t < NODE_TYPE_LIMIT)?;
        let f_precedence = if t == OP_CAT {
            PREC_OP_CAT
        } else if t == OP_OR {
            PREC_OP_OR
        } else if t == OP_START {
            PREC_START
        } else if t == OP_L_PAREN {
            PREC_L_PAREN
        } else {
            PREC_ZERO
        };
        self.nodes.push(RBBINode {
            f_type: t,
            f_parent: None,
            f_left_child: None,
            f_right_child: None,
            f_input_set: None,
            f_precedence,
            f_text: None,
            f_first_pos: 0,
            f_last_pos: 0,
            f_nullable: false,
            f_val: 0,
            f_look_ahead_end: false,
            f_rule_root: false,
            f_chain_in: false,
            f_first_pos_set: BTreeSet::new(),
            f_last_pos_set: BTreeSet::new(),
            f_follow_pos: BTreeSet::new(),
            f_serial_num: next_serial(),
        });
        Ok(self.nodes.len() - 1)
    }

    /// `new RBBINode(RBBINode other)`: a copy without links, not a rule
    /// root.
    pub(crate) fn copy_node(&mut self, other: NodeId) -> NodeId {
        let o = &self.nodes[other];
        let n = RBBINode {
            f_type: o.f_type,
            f_parent: None,
            f_left_child: None,
            f_right_child: None,
            f_input_set: o.f_input_set.clone(),
            f_precedence: o.f_precedence,
            f_text: o.f_text.clone(),
            f_first_pos: o.f_first_pos,
            f_last_pos: o.f_last_pos,
            f_nullable: o.f_nullable,
            f_val: o.f_val,
            f_look_ahead_end: false,
            f_rule_root: false,
            f_chain_in: o.f_chain_in,
            f_first_pos_set: o.f_first_pos_set.clone(),
            f_last_pos_set: o.f_last_pos_set.clone(),
            f_follow_pos: o.f_follow_pos.clone(),
            f_serial_num: next_serial(),
        };
        self.nodes.push(n);
        self.nodes.len() - 1
    }

    /// `cloneTree()`: makes a deep copy of a node tree, flattening
    /// variable references; `uset` nodes are not copied but shared.
    pub(crate) fn clone_tree(&mut self, this: NodeId) -> NodeId {
        let n;
        let t = self.nodes[this].f_type;
        if t == VAR_REF {
            // If the current node is a variable reference, skip over it
            // and clone the definition of the variable instead.
            n = self.clone_tree(child(self.nodes[this].f_left_child));
        } else if t == USET {
            n = this;
        } else {
            n = self.copy_node(this);
            if let Some(l) = self.nodes[this].f_left_child {
                let c = self.clone_tree(l);
                self.nodes[n].f_left_child = Some(c);
                self.nodes[c].f_parent = Some(n);
            }
            if let Some(r) = self.nodes[this].f_right_child {
                let c = self.clone_tree(r);
                self.nodes[n].f_right_child = Some(c);
                self.nodes[c].f_parent = Some(n);
            }
        }
        n
    }

    /// `flattenVariables(int depth)`: replaces each variable reference
    /// with a copy of the variable's definition; returns the (possibly
    /// replaced) node.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("The input is too long")` past depth 3500.
    pub(crate) fn flatten_variables(&mut self, this: NodeId, depth: i32) -> Result<NodeId> {
        if depth > K_RECURSIVE_DEPTH_LIMIT {
            return Err(LuceneError::illegal_argument("The input is too long"));
        }
        if self.nodes[this].f_type == VAR_REF {
            let ret_node = self.clone_tree(child(self.nodes[this].f_left_child));
            self.nodes[ret_node].f_rule_root = self.nodes[this].f_rule_root;
            self.nodes[ret_node].f_chain_in = self.nodes[this].f_chain_in;
            return Ok(ret_node);
        }

        if let Some(l) = self.nodes[this].f_left_child {
            let c = self.flatten_variables(l, depth + 1)?;
            self.nodes[this].f_left_child = Some(c);
            self.nodes[c].f_parent = Some(this);
        }
        if let Some(r) = self.nodes[this].f_right_child {
            let c = self.flatten_variables(r, depth + 1)?;
            self.nodes[this].f_right_child = Some(c);
            self.nodes[c].f_parent = Some(this);
        }
        Ok(this)
    }

    /// `flattenSets()`: replaces each `setRef` with a copy of its set's
    /// expression tree.
    ///
    /// # Errors
    ///
    /// `IllegalStateException` (`Assert.assrt`) when called on a `setRef`.
    pub(crate) fn flatten_sets(&mut self, this: NodeId) -> Result<()> {
        Assert::assrt(self.nodes[this].f_type != SET_REF)?;

        if let Some(l) = self.nodes[this].f_left_child {
            if self.nodes[l].f_type == SET_REF {
                let set_ref_node = l;
                let uset_node = child(self.nodes[set_ref_node].f_left_child);
                let repl_tree = child(self.nodes[uset_node].f_left_child);
                let c = self.clone_tree(repl_tree);
                self.nodes[this].f_left_child = Some(c);
                self.nodes[c].f_parent = Some(this);
            } else {
                self.flatten_sets(l)?;
            }
        }

        if let Some(r) = self.nodes[this].f_right_child {
            if self.nodes[r].f_type == SET_REF {
                let set_ref_node = r;
                let uset_node = child(self.nodes[set_ref_node].f_left_child);
                let repl_tree = child(self.nodes[uset_node].f_left_child);
                let c = self.clone_tree(repl_tree);
                self.nodes[this].f_right_child = Some(c);
                self.nodes[c].f_parent = Some(this);
                // delete setRefNode;
            } else {
                self.flatten_sets(r)?;
            }
        }
        Ok(())
    }

    /// `findNodes(List<RBBINode> dest, int kind)`: appends, in pre-order,
    /// every node of the tree of `kind`.
    pub(crate) fn find_nodes(&self, this: NodeId, dest: &mut Vec<NodeId>, kind: i32) {
        if self.nodes[this].f_type == kind {
            dest.push(this);
        }
        if let Some(l) = self.nodes[this].f_left_child {
            self.find_nodes(l, dest, kind);
        }
        if let Some(r) = self.nodes[this].f_right_child {
            self.find_nodes(r, dest, kind);
        }
    }

    /// `printNode(RBBINode n)`: prints one node (debugging).
    pub(crate) fn print_node(&self, n: Option<NodeId>) {
        match n {
            None => println!(" -- null --"),
            Some(n) => {
                let node = &self.nodes[n];
                print_int(node.f_serial_num, 10);
                print_string(NODE_TYPE_NAMES[node.f_type as usize], 11);
                print_int(node.f_parent.map_or(0, |p| self.nodes[p].f_serial_num), 11);
                print_int(
                    node.f_left_child.map_or(0, |p| self.nodes[p].f_serial_num),
                    11,
                );
                print_int(
                    node.f_right_child.map_or(0, |p| self.nodes[p].f_serial_num),
                    12,
                );
                print_int(node.f_first_pos, 12);
                print_int(node.f_val, 7);

                if node.f_type == VAR_REF {
                    print!(" {}", text_string(&node.f_text));
                }
            }
        }
        println!();
    }

    /// `printTree(boolean printHeading)`: prints a tree (debugging).
    pub(crate) fn print_tree(&self, this: NodeId, print_heading: bool) {
        if print_heading {
            println!("-------------------------------------------------------------------");
            println!("    Serial       type     Parent  LeftChild  RightChild    position  value");
        }
        self.print_node(Some(this));
        // Only dump the definition under the variable reference if asked to.
        // Unconditionally dump children of all other node types.
        if self.nodes[this].f_type != VAR_REF {
            if let Some(l) = self.nodes[this].f_left_child {
                self.print_tree(l, false);
            }

            if let Some(r) = self.nodes[this].f_right_child {
                self.print_tree(r, false);
            }
        }
    }
}

/// `fText` as Java's string concatenation renders it (`"null"` for none).
pub(crate) fn text_string(t: &Option<Vec<u16>>) -> String {
    match t {
        None => "null".to_string(),
        Some(t) => String::from_utf16_lossy(t),
    }
}

/// `printString(String s, int minWidth)`: print a string, padded on the
/// left to `min_width`; a negative width adds `-min_width` spaces of
/// left padding first.
pub(crate) fn print_string(s: &str, min_width: i32) {
    let mut i = min_width;
    while i < 0 {
        // negative width means pad leading spaces, not fixed width.
        print!(" ");
        i += 1;
    }
    let mut i = s.encode_utf16().count() as i32;
    while i < min_width {
        print!(" ");
        i += 1;
    }
    print!("{s}");
}

/// `printInt(int i, int minWidth)`: print an int, padded on the left to a
/// minimum width, always with at least one leading space.
pub(crate) fn print_int(i: i32, min_width: i32) {
    let s = i.to_string();
    let len = s.len() as i32;
    print_string(&s, min_width.max(len + 1));
}

/// `printHex(int i, int minWidth)`: `Integer.toString(i, 16)` zero-padded
/// to five digits, then padded to `min_width`.
pub(crate) fn print_hex(i: i32, min_width: i32) {
    let s = if i < 0 {
        format!("-{:x}", (i as i64).unsigned_abs())
    } else {
        format!("{i:x}")
    };
    let leading_zeroes = &"00000"[..(5 - s.len() as i32).max(0) as usize];
    let s = format!("{leading_zeroes}{s}");
    print_string(&s, min_width);
}
