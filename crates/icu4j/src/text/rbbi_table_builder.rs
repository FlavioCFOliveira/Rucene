// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.text.RBBITableBuilder`: builds the state tables of the RBBI
//! rule compiler from the parse trees (Aho, Sethi, Ullman, "Compilers",
//! section 3.9).

use std::collections::BTreeSet;

use rucene::error::Result;

use crate::r#impl::assert::Assert;
use crate::r#impl::rbbi_data_wrapper::{RBBIDataWrapper, RBBIStateTable};
use crate::text::rbbi_node::{
    self, child, NodeId, RBBINodes, END_MARK, LEAF_CHAR, LOOK_AHEAD, OP_CAT, OP_OR, OP_PLUS, OP_QUESTION,
    OP_STAR, SET_REF, TAG,
};
use crate::text::rbbi_rule_builder::{IntPair, RBBIRuleBuilder};

/// One state of the DFA under construction.
///
/// Equivalent to the package-private static nested class
/// `RBBITableBuilder.RBBIStateDescriptor`.
#[derive(Debug, Clone)]
pub(crate) struct RBBIStateDescriptor {
    /// `fMarked`.
    f_marked: bool,
    /// `fAccepting`.
    f_accepting: i32,
    /// `fLookAhead`.
    f_look_ahead: i32,
    /// `fTagVals`.
    f_tag_vals: BTreeSet<i32>,
    /// `fTagsIdx`.
    f_tags_idx: i32,
    /// `fPositions`: set of parse tree positions associated with this
    /// state.
    f_positions: BTreeSet<NodeId>,
    /// `fDtran`: transitions out of this state, indexed by input character
    /// category.
    f_dtran: Vec<i32>,
}

impl RBBIStateDescriptor {
    /// `RBBIStateDescriptor(int maxInputSymbol)`.
    fn new(max_input_symbol: i32) -> Self {
        Self {
            f_marked: false,
            f_accepting: 0,
            f_look_ahead: 0,
            f_tag_vals: BTreeSet::new(),
            f_tags_idx: 0,
            f_positions: BTreeSet::new(),
            f_dtran: vec![0; (max_input_symbol + 1) as usize],
        }
    }
}

/// `MAX_STATE_FOR_8BITS_TABLE`.
const MAX_STATE_FOR_8BITS_TABLE: i32 = 255;

/// Builds the forward and the safe reverse state tables.
///
/// Equivalent to the package-private class
/// `com.ibm.icu.text.RBBITableBuilder`; its `fRB` back reference is the
/// `rb` parameter of the methods that use it.
#[derive(Debug)]
pub(crate) struct RBBITableBuilder {
    /// `fRootIx`: which of the builder's trees this table is for.
    f_root_ix: usize,
    /// `fDStates`: the DFA states.
    f_d_states: Vec<RBBIStateDescriptor>,
    /// `fSafeTable`: the safe reverse table.
    f_safe_table: Option<Vec<Vec<i16>>>,
    /// `fLookAheadRuleMap`: map from rule number (fVal in look ahead nodes)
    /// to sequential lookahead index.
    f_look_ahead_rule_map: Vec<i32>,
    /// `fLASlotsInUse`: count of the look-ahead slots in use.
    f_la_slots_in_use: i32,
}

impl RBBITableBuilder {
    /// `RBBITableBuilder(RBBIRuleBuilder rb, int rootNodeIx)`.
    pub(crate) fn new(root_node_ix: usize) -> Self {
        Self {
            f_root_ix: root_node_ix,
            f_d_states: Vec::new(),
            f_safe_table: None,
            f_look_ahead_rule_map: Vec::new(),
            f_la_slots_in_use: RBBIDataWrapper::ACCEPTING_UNCONDITIONAL,
        }
    }

    /// The root of this table's tree.
    fn root(&self, rb: &RBBIRuleBuilder) -> NodeId {
        child(rb.f_tree_roots[self.f_root_ix])
    }

    /// `buildForwardTable()`: builds the DFA of the forward rules.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for a too deeply nested tree;
    /// `IllegalStateException` (`Assert.assrt`) for an internal
    /// inconsistency.
    pub(crate) fn build_forward_table(&mut self, rb: &mut RBBIRuleBuilder) -> Result<()> {
        // If there were no rules, just return.  This situation can easily arise
        //   for the reverse rules.
        let Some(root) = rb.f_tree_roots[self.f_root_ix] else {
            return Ok(());
        };

        //
        // Walk through the tree, replacing any references to $variables with a copy of the
        //   parse tree for the substition expression.
        //
        let root = rb.nodes.flatten_variables(root, 0)?;
        rb.f_tree_roots[self.f_root_ix] = Some(root);
        if rb.f_debug_env.as_deref().is_some_and(|e| e.contains("ftree")) {
            println!("Parse tree after flattening variable references.");
            rb.nodes.print_tree(root, true);
        }

        //
        // If the rules contained any references to {bof}
        //   add a {bof} <cat> <former root of tree> to the
        //   tree.  Means that all matches must start out with the
        //   {bof} fake character.
        //
        if rb.set_builder().saw_bof() {
            let bof_top = rb.nodes.new_node(OP_CAT)?;
            let bof_leaf = rb.nodes.new_node(LEAF_CHAR)?;
            rb.nodes[bof_top].f_left_child = Some(bof_leaf);
            rb.nodes[bof_top].f_right_child = rb.f_tree_roots[self.f_root_ix];
            rb.nodes[bof_leaf].f_parent = Some(bof_top);
            rb.nodes[bof_leaf].f_val = 2; // Reserved value for {bof}.
            rb.f_tree_roots[self.f_root_ix] = Some(bof_top);
        }

        //
        // Add a unique right-end marker to the expression.
        //   Appears as a cat-node, left child being the original tree,
        //   right child being the end marker.
        //
        let cn = rb.nodes.new_node(OP_CAT)?;
        let old_root = self.root(rb);
        rb.nodes[cn].f_left_child = Some(old_root);
        rb.nodes[old_root].f_parent = Some(cn);
        let end_marker_node = rb.nodes.new_node(END_MARK)?;
        rb.nodes[cn].f_right_child = Some(end_marker_node);
        rb.nodes[end_marker_node].f_parent = Some(cn);
        rb.f_tree_roots[self.f_root_ix] = Some(cn);

        //
        //  Replace all references to UnicodeSets with the tree for the equivalent
        //      expression.
        //
        rb.nodes.flatten_sets(cn)?;
        if rb.f_debug_env.as_deref().is_some_and(|e| e.contains("stree")) {
            println!("Parse tree after flattening Unicode Set references.");
            rb.nodes.print_tree(cn, true);
        }

        //
        // calculate the functions nullable, firstpos, lastpos and followpos on
        // nodes in the parse tree.
        //    See the algorithm description in Aho.
        //    Understanding how this works by looking at the code alone will be
        //       nearly impossible.
        //
        calc_nullable(&mut rb.nodes, Some(cn));
        calc_first_pos(&mut rb.nodes, Some(cn));
        calc_last_pos(&mut rb.nodes, Some(cn));
        calc_follow_pos(&mut rb.nodes, Some(cn));
        if rb.f_debug_env.as_deref().is_some_and(|e| e.contains("pos")) {
            println!();
            print_pos_sets(&rb.nodes, Some(cn));
        }

        //
        //  For "chained" rules, modify the followPos sets
        //
        if rb.f_chain_rules {
            calc_chained_follow_pos(&mut rb.nodes, cn, end_marker_node);
        }

        //
        //  BOF (start of input) test fixup.
        //
        if rb.set_builder().saw_bof() {
            self.bof_fixup(rb)?;
        }

        //
        // Build the DFA state transition tables.
        //
        self.build_state_table(rb)?;
        self.map_look_ahead_rules(rb);
        self.flag_accepting_states(rb);
        self.flag_look_ahead_states(rb);
        self.flag_tagged_states(rb);

        //
        // Update the global table of rule status {tag} values
        // The rule builder has a global vector of status values that are common
        //    for all tables.  Merge the ones from this table into the global set.
        //
        self.merge_rule_status_vals(rb);
        Ok(())
    }

    /// `bofFixup()`: adds the start of the main tree's matches to the
    /// follow positions of the `{bof}` leaf.
    ///
    /// # Errors
    ///
    /// `IllegalStateException` (`Assert.assrt`) when the tree does not have
    /// the expected shape.
    fn bof_fixup(&mut self, rb: &mut RBBIRuleBuilder) -> Result<()> {
        //
        //   The parse tree looks like this ...
        //         fTree root  --.       <cat>
        //                               /     \
        //                            <cat>   <#end node>
        //                           /     \
        //                     <bofNode>   rest
        //                               of tree
        //
        //    We will be adding things to the followPos set of the <bofNode>
        //
        let root = self.root(rb);
        let left = child(rb.nodes[root].f_left_child);
        let bof_node = child(rb.nodes[left].f_left_child);
        Assert::assrt(rb.nodes[bof_node].f_type == LEAF_CHAR)?;
        Assert::assrt(rb.nodes[bof_node].f_val == 2)?;

        // Get all nodes that can be the start a match of the user-written rules
        //  (excluding the fake bofNode)
        //  We want the nodes that can start a match in the
        //     part labeled "rest of tree"
        //
        let rest = child(rb.nodes[left].f_right_child);
        let match_start_nodes: Vec<NodeId> = rb.nodes[rest].f_first_pos_set.iter().copied().collect();
        for start_node in match_start_nodes {
            if rb.nodes[start_node].f_type != LEAF_CHAR {
                continue;
            }

            if rb.nodes[start_node].f_val == rb.nodes[bof_node].f_val {
                //  We found a leaf node corresponding to a {bof} that was
                //    explicitly written into a rule.
                //  Add everything from the followPos set of this node to the
                //    followPos set of the fake bofNode at the start of the tree.
                //
                let follow = rb.nodes[start_node].f_follow_pos.clone();
                rb.nodes[bof_node].f_follow_pos.extend(follow);
            }
        }
        Ok(())
    }

    /// `buildStateTable()`: builds the DFA states and transitions.
    ///
    /// # Errors
    ///
    /// `IllegalStateException` (`Assert.assrt`) for an internal
    /// inconsistency.
    fn build_state_table(&mut self, rb: &mut RBBIRuleBuilder) -> Result<()> {
        //
        // Add a dummy state 0 - the stop state.  Not from Aho.
        let last_input_symbol = rb.set_builder().get_num_char_categories() - 1;
        let fail_state = RBBIStateDescriptor::new(last_input_symbol);
        self.f_d_states.push(fail_state);

        // initially, the only unmarked state in Dstates is firstpos(root),
        //       where toot is the root of the syntax tree for (r)#;
        let mut initial_state = RBBIStateDescriptor::new(last_input_symbol);
        let root = self.root(rb);
        initial_state.f_positions.extend(rb.nodes[root].f_first_pos_set.iter().copied());
        self.f_d_states.push(initial_state);

        // while there is an unmarked state T in Dstates do begin
        loop {
            let mut t: Option<usize> = None;
            for tx in 1..self.f_d_states.len() {
                if !self.f_d_states[tx].f_marked {
                    t = Some(tx);
                    break;
                }
            }
            let Some(t) = t else {
                break;
            };

            // mark T;
            self.f_d_states[t].f_marked = true;

            // for each input symbol a do begin
            for a in 1..=last_input_symbol {
                // let U be the set of positions that are in followpos(p)
                //    for some position p in T
                //    such that the symbol at position p is a;
                let mut u: Option<BTreeSet<NodeId>> = None;
                for &p in &self.f_d_states[t].f_positions {
                    if rb.nodes[p].f_type == LEAF_CHAR && rb.nodes[p].f_val == a {
                        u.get_or_insert_with(BTreeSet::new).extend(rb.nodes[p].f_follow_pos.iter().copied());
                    }
                }

                // if U is not empty and not in DStates then
                let mut ux = 0;
                let mut u_in_dstates = false;
                if let Some(u) = u {
                    Assert::assrt(!u.is_empty())?;
                    for ix in 0..self.f_d_states.len() {
                        if u == self.f_d_states[ix].f_positions {
                            ux = ix;
                            u_in_dstates = true;
                            break;
                        }
                    }

                    // Add U as an unmarked state to Dstates
                    if !u_in_dstates {
                        let mut new_state = RBBIStateDescriptor::new(last_input_symbol);
                        new_state.f_positions = u;
                        self.f_d_states.push(new_state);
                        ux = self.f_d_states.len() - 1;
                    }

                    // Dtran[T, a] := U;
                    self.f_d_states[t].f_dtran[a as usize] = ux as i32;
                }
            }
        }
        Ok(())
    }

    /// `mapLookAheadRules()`: assigns the look-ahead result slots to the
    /// look-ahead rules.
    fn map_look_ahead_rules(&mut self, rb: &RBBIRuleBuilder) {
        self.f_look_ahead_rule_map = vec![0; (rb.scanner().num_rules() + 1) as usize];

        for sd in &self.f_d_states {
            let mut la_slot_for_state = 0;

            // Establish the look-ahead slot for this state, if the state covers
            // any look-ahead nodes - corresponding to the '/' in look-ahead rules.

            // If any of the look-ahead nodes already have a slot assigned, use it,
            // otherwise assign a new one.

            let mut saw_look_ahead_node = false;
            for &node in &sd.f_positions {
                if rb.nodes[node].f_type != LOOK_AHEAD {
                    continue;
                }
                saw_look_ahead_node = true;
                let rule_num = rb.nodes[node].f_val;
                debug_assert!((rule_num as usize) < self.f_look_ahead_rule_map.len());
                debug_assert!(rule_num > 0);
                let la_slot = self.f_look_ahead_rule_map[rule_num as usize];
                if la_slot != 0 {
                    if la_slot_for_state == 0 {
                        la_slot_for_state = la_slot;
                    } else {
                        // TODO: figure out if this can fail, change to setting an error code if so.
                        debug_assert!(la_slot == la_slot_for_state);
                    }
                }
            }
            if !saw_look_ahead_node {
                continue;
            }

            if la_slot_for_state == 0 {
                self.f_la_slots_in_use += 1;
                la_slot_for_state = self.f_la_slots_in_use;
            }

            // For each look ahead node covered by this state,
            // set the mapping from the node's rule number to the look ahead slot.
            // There can be multiple nodes/rule numbers going to the same la slot.

            for &node in &sd.f_positions {
                if rb.nodes[node].f_type != LOOK_AHEAD {
                    continue;
                }
                let rule_num = rb.nodes[node].f_val;
                let existing_val = self.f_look_ahead_rule_map[rule_num as usize];
                debug_assert!(existing_val == 0 || existing_val == la_slot_for_state);
                let _ = existing_val;
                self.f_look_ahead_rule_map[rule_num as usize] = la_slot_for_state;
            }
        }
    }

    /// `flagAcceptingStates()`: marks the states that contain an end
    /// marker as accepting.
    fn flag_accepting_states(&mut self, rb: &RBBIRuleBuilder) {
        let mut end_marker_nodes = Vec::new();
        rb.nodes.find_nodes(self.root(rb), &mut end_marker_nodes, END_MARK);

        for &end_marker in &end_marker_nodes {
            let em_val = rb.nodes[end_marker].f_val;
            for sd in self.f_d_states.iter_mut() {
                if sd.f_positions.contains(&end_marker) {
                    // Any non-zero value for fAccepting means this is an accepting node.
                    // The value is what will be returned to the user as the break status.
                    // If no other value was specified, force it to ACCEPTING_UNCONDITIONAL (1).

                    if sd.f_accepting == 0 {
                        // State hasn't been marked as accepting yet.  Do it now.
                        sd.f_accepting = self.f_look_ahead_rule_map[em_val as usize];
                        if sd.f_accepting == 0 {
                            sd.f_accepting = RBBIDataWrapper::ACCEPTING_UNCONDITIONAL;
                        }
                    }
                    if sd.f_accepting == RBBIDataWrapper::ACCEPTING_UNCONDITIONAL && em_val != 0 {
                        // Both lookahead and non-lookahead accepting for this state.
                        // Favor the look-ahead, because a look-ahead match needs to
                        // immediately stop the run-time engine. First match, not longest.
                        sd.f_accepting = self.f_look_ahead_rule_map[em_val as usize];
                    }
                    // implicit else:
                    // if sd.fAccepting already had a value other than 0 or 1, leave it be.
                }
            }
        }
    }

    /// `flagLookAheadStates()`: marks the states that contain a look-ahead
    /// node with its slot.
    fn flag_look_ahead_states(&mut self, rb: &RBBIRuleBuilder) {
        let mut look_ahead_nodes = Vec::new();
        rb.nodes.find_nodes(self.root(rb), &mut look_ahead_nodes, LOOK_AHEAD);
        for &look_ahead_node in &look_ahead_nodes {
            for sd in self.f_d_states.iter_mut() {
                if sd.f_positions.contains(&look_ahead_node) {
                    let lookahead_slot = self.f_look_ahead_rule_map[rb.nodes[look_ahead_node].f_val as usize];
                    debug_assert!(sd.f_look_ahead == 0 || sd.f_look_ahead == lookahead_slot);
                    sd.f_look_ahead = lookahead_slot;
                }
            }
        }
    }

    /// `flagTaggedStates()`: collects the tag values of each state.
    fn flag_tagged_states(&mut self, rb: &RBBIRuleBuilder) {
        let mut tag_nodes = Vec::new();
        rb.nodes.find_nodes(self.root(rb), &mut tag_nodes, TAG);
        for &tag_node in &tag_nodes {
            for sd in self.f_d_states.iter_mut() {
                // For each state  s (row) in the state table
                if sd.f_positions.contains(&tag_node) {
                    // if  s include the tag node t
                    sd.f_tag_vals.insert(rb.nodes[tag_node].f_val);
                }
            }
        }
    }

    /// `mergeRuleStatusVals()`: merges the tag sets of the states into the
    /// builder's rule status table.
    fn merge_rule_status_vals(&mut self, rb: &mut RBBIRuleBuilder) {
        //
        //  The basic outline of what happens here is this...
        //
        //    for each state in this state table
        //       if the status tag list for this state is in the global statuses list
        //           record where and
        //           continue with the next state
        //       else
        //           add the tag list for this state to the global list.
        //

        // Pre-load a single tag of {0} into the table.
        //   We will need this as a default, for rule sets with no explicit tagging,
        //   or with explicit tagging of {0}.
        if rb.f_rule_status_vals.is_empty() {
            rb.f_rule_status_vals.push(1); // Num of statuses in group
            rb.f_rule_status_vals.push(0); //   and our single status of zero

            let s0: BTreeSet<i32> = BTreeSet::new(); // mapping for rules with no explicit tagging
            rb.f_status_sets.insert(s0, 0); //   (key is an empty set).

            let mut s1: BTreeSet<i32> = BTreeSet::new(); // mapping for rules with explicit tagging of {0}
            s1.insert(0);
            rb.f_status_sets.insert(s1, 0);
        }

        //    For each state, check whether the state's status tag values are
        //       already entered into the status values array, and add them if not.
        for sd in self.f_d_states.iter_mut() {
            let status_vals = &sd.f_tag_vals;
            let array_index_i = match rb.f_status_sets.get(status_vals) {
                Some(&i) => i,
                None => {
                    // This is the first encounter of this set of status values.
                    //   Add them to the statusSets map, This map associates
                    //   the set of status values with an index in the runtime status
                    //   values array.
                    let i = rb.f_rule_status_vals.len() as i32;
                    rb.f_status_sets.insert(status_vals.clone(), i);

                    // Add the new set of status values to the vector of values that
                    //   will eventually become the array used by the runtime engine.
                    rb.f_rule_status_vals.push(status_vals.len() as i32);
                    rb.f_rule_status_vals.extend(status_vals.iter().copied());
                    i
                }
            };

            // Save the runtime array index back into the state descriptor.
            sd.f_tags_idx = array_index_i;
        }
    }

    /// `findDuplCharClassFrom(IntPair categories)`: finds the next pair of
    /// character categories with identical columns, starting at
    /// `categories.first`; `false` when none.
    pub(crate) fn find_dupl_char_class_from(&self, rb: &RBBIRuleBuilder, categories: &mut IntPair) -> bool {
        let num_states = self.f_d_states.len();
        let num_cols = rb.set_builder().get_num_char_categories();

        let mut table_base = 0;
        let mut table_dupl = 0;
        while categories.first < num_cols - 1 {
            // Note: dictionary & non-dictionary columns cannot be merged.
            //       The limitSecond value prevents considering mixed pairs.
            //       Dictionary categories are >= DictCategoriesStart.
            //       Non dict categories are   <  DictCategoriesStart.
            let limit_second = if categories.first < rb.set_builder().get_dict_categories_start() {
                rb.set_builder().get_dict_categories_start()
            } else {
                num_cols
            };
            categories.second = categories.first + 1;
            while categories.second < limit_second {
                for state in 0..num_states {
                    let sd = &self.f_d_states[state];
                    table_base = sd.f_dtran[categories.first as usize];
                    table_dupl = sd.f_dtran[categories.second as usize];
                    if table_base != table_dupl {
                        break;
                    }
                }
                if table_base == table_dupl {
                    return true;
                }
                categories.second += 1;
            }
            categories.first += 1;
        }
        false
    }

    /// `removeColumn(int column)`: removes a column from the state table.
    pub(crate) fn remove_column(&mut self, column: i32) {
        for sd in self.f_d_states.iter_mut() {
            debug_assert!((column as usize) < sd.f_dtran.len());
            sd.f_dtran.remove(column as usize);
        }
    }

    /// `findDuplicateState(IntPair states)`: finds the next pair of
    /// equivalent states, starting at `states.first`.
    fn find_duplicate_state(&self, rb: &RBBIRuleBuilder, states: &mut IntPair) -> bool {
        let num_states = self.f_d_states.len() as i32;
        let num_cols = rb.set_builder().get_num_char_categories();

        while states.first < num_states - 1 {
            let first_sd = &self.f_d_states[states.first as usize];
            states.second = states.first + 1;
            while states.second < num_states {
                let dupl_sd = &self.f_d_states[states.second as usize];
                if first_sd.f_accepting != dupl_sd.f_accepting
                    || first_sd.f_look_ahead != dupl_sd.f_look_ahead
                    || first_sd.f_tags_idx != dupl_sd.f_tags_idx
                {
                    states.second += 1;
                    continue;
                }
                let mut rows_match = true;
                for col in 0..num_cols as usize {
                    let first_val = first_sd.f_dtran[col];
                    let dupl_val = dupl_sd.f_dtran[col];
                    if !((first_val == dupl_val)
                        || ((first_val == states.first || first_val == states.second)
                            && (dupl_val == states.first || dupl_val == states.second)))
                    {
                        rows_match = false;
                        break;
                    }
                }
                if rows_match {
                    return true;
                }
                states.second += 1;
            }
            states.first += 1;
        }
        false
    }

    /// `findDuplicateSafeState(IntPair states)`: as
    /// [`find_duplicate_state`](Self::find_duplicate_state) for the safe
    /// table.
    fn find_duplicate_safe_state(&self, states: &mut IntPair) -> bool {
        let safe = self.f_safe_table.as_ref().expect("built");
        let num_states = safe.len() as i32;

        while states.first < num_states - 1 {
            let first_row = &safe[states.first as usize];
            states.second = states.first + 1;
            while states.second < num_states {
                let dupl_row = &safe[states.second as usize];
                let mut rows_match = true;
                let num_cols = first_row.len();
                for col in 0..num_cols {
                    let first_val = first_row[col] as i32;
                    let dupl_val = dupl_row[col] as i32;
                    if !((first_val == dupl_val)
                        || ((first_val == states.first || first_val == states.second)
                            && (dupl_val == states.first || dupl_val == states.second)))
                    {
                        rows_match = false;
                        break;
                    }
                }
                if rows_match {
                    return true;
                }
                states.second += 1;
            }
            states.first += 1;
        }
        false
    }

    /// `removeState(IntPair duplStates)`: removes `duplStates.second`,
    /// redirecting its transitions to `duplStates.first`.
    fn remove_state(&mut self, rb: &RBBIRuleBuilder, dupl_states: &IntPair) {
        let keep_state = dupl_states.first;
        let dupl_state = dupl_states.second;
        debug_assert!(keep_state < dupl_state);
        debug_assert!((dupl_state as usize) < self.f_d_states.len());

        self.f_d_states.remove(dupl_state as usize);

        let num_cols = rb.set_builder().get_num_char_categories();
        for sd in self.f_d_states.iter_mut() {
            for col in 0..num_cols as usize {
                let existing_val = sd.f_dtran[col];
                let mut new_val = existing_val;
                if existing_val == dupl_state {
                    new_val = keep_state;
                } else if existing_val > dupl_state {
                    new_val = existing_val - 1;
                }
                sd.f_dtran[col] = new_val;
            }
        }
    }

    /// `removeSafeState(IntPair duplStates)`: as
    /// [`remove_state`](Self::remove_state) for the safe table.
    fn remove_safe_state(&mut self, dupl_states: &IntPair) {
        let keep_state = dupl_states.first;
        let dupl_state = dupl_states.second;
        debug_assert!(keep_state < dupl_state);
        let safe = self.f_safe_table.as_mut().expect("built");
        debug_assert!((dupl_state as usize) < safe.len());

        safe.remove(dupl_state as usize);
        for row in safe.iter_mut() {
            for v in row.iter_mut() {
                let existing_val = *v as i32;
                let mut new_val = existing_val;
                if existing_val == dupl_state {
                    new_val = keep_state;
                } else if existing_val > dupl_state {
                    new_val = existing_val - 1;
                }
                *v = new_val as i16;
            }
        }
    }

    /// `removeDuplicateStates()`: removes all duplicate states; returns the
    /// number removed.
    pub(crate) fn remove_duplicate_states(&mut self, rb: &RBBIRuleBuilder) -> i32 {
        let mut dupls = IntPair::new(3, 0);
        let mut num_states_removed = 0;

        while self.find_duplicate_state(rb, &mut dupls) {
            // System.out.printf("Removing duplicate states (%d, %d)\n", dupls.first, dupls.second);
            self.remove_state(rb, &dupls);
            num_states_removed += 1;
        }
        num_states_removed
    }

    /// `getTableSize()`: the size of the forward table, in bytes, padded
    /// to 8.
    pub(crate) fn get_table_size(&self, rb: &RBBIRuleBuilder) -> i32 {
        if rb.f_tree_roots[self.f_root_ix].is_none() {
            return 0;
        }
        let mut size = RBBIStateTable::F_HEADER_SIZE; // The header, with no rows to the table.
        let num_rows = self.f_d_states.len() as i32;
        let num_cols = rb.set_builder().get_num_char_categories();
        let use_8_bits = num_rows <= MAX_STATE_FOR_8BITS_TABLE;
        let row_size = (if use_8_bits { 1 } else { 2 }) * (RBBIDataWrapper::NEXTSTATES + num_cols);
        size += num_rows * row_size;
        size = (size + 7) & !7; // round up to a multiple of 8 bytes
        size
    }

    /// `exportTable()`: the forward table in its runtime format.
    ///
    /// # Errors
    ///
    /// `IllegalStateException` (`Assert.assrt`) when a value does not fit
    /// the row width.
    pub(crate) fn export_table(&self, rb: &RBBIRuleBuilder) -> Result<RBBIStateTable> {
        let mut table = RBBIStateTable::new();
        if rb.f_tree_roots[self.f_root_ix].is_none() {
            return Ok(table);
        }

        Assert::assrt(rb.set_builder().get_num_char_categories() < 0x7fff && self.f_d_states.len() < 0x7fff)?;
        table.f_num_states = self.f_d_states.len() as i32;
        table.f_dict_categories_start = rb.set_builder().get_dict_categories_start();
        table.f_look_ahead_results_size = if self.f_la_slots_in_use == RBBIDataWrapper::ACCEPTING_UNCONDITIONAL {
            0
        } else {
            self.f_la_slots_in_use + 1
        };
        let use_8_bits = table.f_num_states <= MAX_STATE_FOR_8BITS_TABLE;

        // Size of table size in shorts.
        //  the "4" is the size of struct RBBIStateTableRow, the row header part only.
        let row_len = RBBIDataWrapper::NEXTSTATES + rb.set_builder().get_num_char_categories(); // Row Length in 16 bit words.
        let table_size;
        if use_8_bits {
            table_size = self.get_table_size(rb) - RBBIStateTable::F_HEADER_SIZE; // fTable length in bytes.
            table.f_table = vec![0; table_size as usize];
            table.f_row_len = row_len;
        } else {
            table_size = (self.get_table_size(rb) - RBBIStateTable::F_HEADER_SIZE) / 2;
            table.f_table = vec![0; table_size as usize];
            table.f_row_len = row_len * 2;
        }

        if rb.f_look_ahead_hard_break {
            table.f_flags |= RBBIDataWrapper::RBBI_LOOKAHEAD_HARD_BREAK;
        }
        if rb.set_builder().saw_bof() {
            table.f_flags |= RBBIDataWrapper::RBBI_BOF_REQUIRED;
        }
        if use_8_bits {
            table.f_flags |= RBBIDataWrapper::RBBI_8BITS_ROWS;
        }

        let num_char_categories = rb.set_builder().get_num_char_categories();
        for state in 0..table.f_num_states {
            let sd = &self.f_d_states[state as usize];
            let row = (state * row_len) as usize;
            if use_8_bits {
                Assert::assrt(0 <= sd.f_accepting && sd.f_accepting <= 255)?;
                Assert::assrt(0 <= sd.f_look_ahead && sd.f_look_ahead <= 255)?;
            } else {
                Assert::assrt(0 <= sd.f_accepting && sd.f_accepting <= 0xffff)?;
                Assert::assrt(0 <= sd.f_look_ahead && sd.f_look_ahead <= 0xffff)?;
            }
            table.f_table[row + RBBIDataWrapper::ACCEPTING as usize] = sd.f_accepting as u16;
            table.f_table[row + RBBIDataWrapper::LOOKAHEAD as usize] = sd.f_look_ahead as u16;
            table.f_table[row + RBBIDataWrapper::TAGSIDX as usize] = sd.f_tags_idx as u16;
            for col in 0..num_char_categories as usize {
                if use_8_bits {
                    Assert::assrt(0 <= sd.f_dtran[col] && sd.f_dtran[col] <= MAX_STATE_FOR_8BITS_TABLE)?;
                }
                table.f_table[row + RBBIDataWrapper::NEXTSTATES as usize + col] = sd.f_dtran[col] as u16;
            }
        }
        Ok(table)
    }

    /// `buildSafeReverseTable()`: builds the safe reverse table from the
    /// forward table's safe character pairs.
    pub(crate) fn build_safe_reverse_table(&mut self, rb: &RBBIRuleBuilder) {
        // Find safe char class pairs.

        // make a state table row for each trailing class, and map from class to row.

        // For each pair
        //   startRow[p][c1] = p[c2]   (startRow is the row for class c2's state)
        //   - with the starting state. (class c2 is the second char of the pair)
        // ...

        let mut safe_pairs: Vec<u16> = Vec::new();

        let num_char_classes = rb.set_builder().get_num_char_categories();
        let num_states = self.f_d_states.len();

        for c1 in 0..num_char_classes {
            for c2 in 0..num_char_classes {
                let mut wanted_end_state = -1;
                let mut end_state = 0;
                for start_state in 1..num_states {
                    let start_state_d = &self.f_d_states[start_state];
                    let s2 = start_state_d.f_dtran[c1 as usize];
                    let s2_state_d = &self.f_d_states[s2 as usize];
                    end_state = s2_state_d.f_dtran[c2 as usize];
                    if wanted_end_state < 0 {
                        wanted_end_state = end_state;
                    } else if wanted_end_state != end_state {
                        break;
                    }
                }
                if wanted_end_state == end_state {
                    safe_pairs.push(c1 as u16);
                    safe_pairs.push(c2 as u16);
                    // System.out.printf("(%d, %d) ", c1, c2);
                }
            }
            // System.out.printf("\n");
        }

        // Populate the initial safe table.
        //   The table as a whole is UVector<UnicodeString>
        //   Each row is represented by a UnicodeString, being used as a Vector<int16>.
        //   Row 0 is the stop state.
        //   Row 1 is the start state.
        //   Row 2 and beyond are other states, initially one per char class, but
        //     after initial construction, many of the states will be combined, compacting the
        //     table.)
        //   The String holds the nextState data only. The four leading fields of a row, fAccepting,
        //   fLookAhead, etc. are not needed for the safe table, and are omitted at this stage of building.

        debug_assert!(self.f_safe_table.is_none());
        let mut safe: Vec<Vec<i16>> = (0..num_char_classes + 2)
            .map(|_| vec![0i16; num_char_classes as usize])
            .collect();

        // From the start state, each input char class transitions to the state for that input.
        for char_class in 0..num_char_classes {
            // Note: +2 to skip the start & stop state rows.
            safe[1][char_class as usize] = (char_class + 2) as i16;
        }

        // Initially make every other state table row look like the start state row
        //    (except for the row header part, which will be fixed up later)
        let start_state = safe[1].clone();
        for row in 2..(num_char_classes + 2) as usize {
            safe[row].copy_from_slice(&start_state);
        }

        // Run through the safe pairs, set the next state to zero when pair has been seen.
        // Zero being the stop state, meaning we found a safe point.
        let mut pair_idx = 0;
        while pair_idx < safe_pairs.len() {
            let c1 = safe_pairs[pair_idx] as usize;
            let c2 = safe_pairs[pair_idx + 1] as usize;

            safe[c2 + 2][c1] = 0;
            pair_idx += 2;
        }
        self.f_safe_table = Some(safe);

        // Remove duplicate or redundant rows from the table.
        let mut states = IntPair::new(1, 0);
        while self.find_duplicate_safe_state(&mut states) {
            // System.out.printf("Removing duplicate safe states (%d, %d)\n", states.first, states.second);
            self.remove_safe_state(&states);
        }
    }

    /// `getSafeTableSize()`: the size of the safe table, in bytes, padded to
    /// 8.
    pub(crate) fn get_safe_table_size(&self) -> i32 {
        let Some(safe) = self.f_safe_table.as_ref() else {
            return 0;
        };
        let mut size = RBBIStateTable::F_HEADER_SIZE; // The header, with no rows to the table.
        let num_rows = safe.len() as i32;
        let num_cols = safe[0].len() as i32;
        let use_8_bits = num_rows <= MAX_STATE_FOR_8BITS_TABLE;

        let row_size = (if use_8_bits { 1 } else { 2 }) * (RBBIDataWrapper::NEXTSTATES + num_cols);
        size += num_rows * row_size;
        // TODO: there are redundant round-up. Figure out best place, get rid of the other.
        size = (size + 7) & !7; // round up to a multiple of 8 bytes
        size
    }

    /// `exportSafeTable()`: the safe reverse table in its runtime format.
    ///
    /// # Errors
    ///
    /// `IllegalStateException` (`Assert.assrt`) when a state does not fit
    /// an 8-bit row.
    ///
    /// # Panics
    ///
    /// `NullPointerException` when the safe table was not built.
    pub(crate) fn export_safe_table(&self) -> Result<RBBIStateTable> {
        let safe = self
            .f_safe_table
            .as_ref()
            .unwrap_or_else(|| panic!("java.lang.NullPointerException: fSafeTable"));
        let mut table = RBBIStateTable::new();
        table.f_num_states = safe.len() as i32;
        let use_8_bits = table.f_num_states <= MAX_STATE_FOR_8BITS_TABLE;
        let num_char_categories = safe[0].len();

        // Size of table size in shorts.
        //  the "4" is the size of struct RBBIStateTableRow, the row header part only.
        let row_len = RBBIDataWrapper::NEXTSTATES + num_char_categories as i32;
        // TODO: tableSize is basically numStates * numCharCategories,
        //       except for alignment padding. Clean up here, and in main exportTable().
        let mut table_size = self.get_safe_table_size() - RBBIStateTable::F_HEADER_SIZE; // fTable length in bytes.
        if use_8_bits {
            table.f_flags |= RBBIDataWrapper::RBBI_8BITS_ROWS;
            table.f_table = vec![0; table_size as usize];
            table.f_row_len = row_len;
        } else {
            table_size /= 2; // fTable length in shorts.
            table.f_table = vec![0; table_size as usize];
            table.f_row_len = row_len * 2;
        }

        for state in 0..table.f_num_states {
            let row_array = &safe[state as usize];
            let row = (state * row_len) as usize;

            for col in 0..num_char_categories {
                if use_8_bits {
                    Assert::assrt(row_array[col] as i32 <= MAX_STATE_FOR_8BITS_TABLE)?;
                }
                table.f_table[row + RBBIDataWrapper::NEXTSTATES as usize + col] = row_array[col] as u16;
            }
        }
        Ok(table)
    }

    /// `printStates()`: debug.
    pub(crate) fn print_states(&self, rb: &RBBIRuleBuilder) {
        let ncat = rb.set_builder().get_num_char_categories();
        println!("state |           i n p u t     s y m b o l s ");
        print!("      | Acc  LA    Tag");
        for c in 0..ncat {
            rbbi_node::print_int(c, 4);
        }
        println!();
        print!("      |---------------");
        for _ in 0..ncat {
            print!("----");
        }
        println!();

        for (n, sd) in self.f_d_states.iter().enumerate() {
            rbbi_node::print_int(n as i32, 5);
            print!(" | ");

            rbbi_node::print_int(sd.f_accepting, 3);
            rbbi_node::print_int(sd.f_look_ahead, 4);
            rbbi_node::print_int(sd.f_tags_idx, 6);
            print!(" ");
            for c in 0..ncat as usize {
                rbbi_node::print_int(sd.f_dtran[c], 4);
            }
            println!();
        }
        print!("\n\n");
    }

    /// `printReverseTable()`: debug.
    pub(crate) fn print_reverse_table(&self) {
        println!("    Safe Reverse Table ");
        let Some(safe) = self.f_safe_table.as_ref() else {
            println!("   --- nullptr ---");
            return;
        };
        let num_char_categories = safe[0].len();
        println!("state |           i n p u t     s y m b o l s ");
        print!("      | Acc  LA    Tag");
        for c in 0..num_char_categories {
            print!(" {c:2}");
        }
        println!();
        print!("      |---------------");
        for _ in 0..num_char_categories {
            print!("---");
        }
        println!();

        for (n, row_array) in safe.iter().enumerate() {
            print!("  {n:3} | ");
            print!("{:3} {:3} {:5} ", 0, 0, 0); // Accepting, LookAhead, Tags
            for c in 0..num_char_categories {
                print!(" {:2}", row_array[c]);
            }
            println!();
        }
        print!("\n\n");
    }

    /// `printRuleStatusTable()`: debug.
    pub(crate) fn print_rule_status_table(&self, rb: &RBBIRuleBuilder) {
        let mut next_record = 0usize;
        let tbl = &rb.f_rule_status_vals;

        println!("index |  tags ");
        println!("-------------------");

        while next_record < tbl.len() {
            let this_record = next_record;
            next_record = this_record + tbl[this_record] as usize + 1;
            rbbi_node::print_int(this_record as i32, 7);
            for &val in &tbl[this_record + 1..next_record] {
                rbbi_node::print_int(val, 7);
            }
            println!();
        }
        print!("\n\n");
    }
}

/// `calcNullable(RBBINode n)`: the `nullable` function of Aho's algorithm.
fn calc_nullable(nodes: &mut RBBINodes, n: Option<NodeId>) {
    let Some(n) = n else {
        return;
    };
    let t = nodes[n].f_type;
    if t == SET_REF || t == END_MARK {
        // These are non-empty leaf node types.
        nodes[n].f_nullable = false;
        return;
    }

    if t == LOOK_AHEAD || t == TAG {
        // Lookahead marker node.  It's a leaf, so no recursion on children.
        // It's nullable because it does not match any literal text from the input stream.
        nodes[n].f_nullable = true;
        return;
    }

    // The node is not a leaf.
    //  Calculate nullable on its children.
    let (l, r) = (nodes[n].f_left_child, nodes[n].f_right_child);
    calc_nullable(nodes, l);
    calc_nullable(nodes, r);

    // Apply functions from table 3.40 in Aho
    nodes[n].f_nullable = if t == OP_OR {
        nodes[child(l)].f_nullable || nodes[child(r)].f_nullable
    } else if t == OP_CAT {
        nodes[child(l)].f_nullable && nodes[child(r)].f_nullable
    } else {
        t == OP_STAR || t == OP_QUESTION
    };
}

/// `calcFirstPos(RBBINode n)`: the `firstpos` function of Aho's algorithm.
fn calc_first_pos(nodes: &mut RBBINodes, n: Option<NodeId>) {
    let Some(n) = n else {
        return;
    };
    let t = nodes[n].f_type;
    if t == LEAF_CHAR || t == END_MARK || t == LOOK_AHEAD || t == TAG {
        // These are non-empty leaf node types.
        nodes[n].f_first_pos_set.insert(n);
        return;
    }

    // The node is not a leaf.
    //  Calculate firstPos on its children.
    let (l, r) = (nodes[n].f_left_child, nodes[n].f_right_child);
    calc_first_pos(nodes, l);
    calc_first_pos(nodes, r);

    // Apply functions from table 3.40 in Aho
    if t == OP_OR {
        let ls = nodes[child(l)].f_first_pos_set.clone();
        let rs = nodes[child(r)].f_first_pos_set.clone();
        nodes[n].f_first_pos_set.extend(ls);
        nodes[n].f_first_pos_set.extend(rs);
    } else if t == OP_CAT {
        let ls = nodes[child(l)].f_first_pos_set.clone();
        nodes[n].f_first_pos_set.extend(ls);
        if nodes[child(l)].f_nullable {
            let rs = nodes[child(r)].f_first_pos_set.clone();
            nodes[n].f_first_pos_set.extend(rs);
        }
    } else if t == OP_STAR || t == OP_QUESTION || t == OP_PLUS {
        let ls = nodes[child(l)].f_first_pos_set.clone();
        nodes[n].f_first_pos_set.extend(ls);
    }
}

/// `calcLastPos(RBBINode n)`: the `lastpos` function of Aho's algorithm.
fn calc_last_pos(nodes: &mut RBBINodes, n: Option<NodeId>) {
    let Some(n) = n else {
        return;
    };
    let t = nodes[n].f_type;
    if t == LEAF_CHAR || t == END_MARK || t == LOOK_AHEAD || t == TAG {
        // These are non-empty leaf node types.
        nodes[n].f_last_pos_set.insert(n);
        return;
    }

    // The node is not a leaf.
    //  Calculate lastPos on its children.
    let (l, r) = (nodes[n].f_left_child, nodes[n].f_right_child);
    calc_last_pos(nodes, l);
    calc_last_pos(nodes, r);

    // Apply functions from table 3.40 in Aho
    if t == OP_OR {
        let ls = nodes[child(l)].f_last_pos_set.clone();
        let rs = nodes[child(r)].f_last_pos_set.clone();
        nodes[n].f_last_pos_set.extend(ls);
        nodes[n].f_last_pos_set.extend(rs);
    } else if t == OP_CAT {
        let rs = nodes[child(r)].f_last_pos_set.clone();
        nodes[n].f_last_pos_set.extend(rs);
        if nodes[child(r)].f_nullable {
            let ls = nodes[child(l)].f_last_pos_set.clone();
            nodes[n].f_last_pos_set.extend(ls);
        }
    } else if t == OP_STAR || t == OP_QUESTION || t == OP_PLUS {
        let ls = nodes[child(l)].f_last_pos_set.clone();
        nodes[n].f_last_pos_set.extend(ls);
    }
}

/// `calcFollowPos(RBBINode n)`: the `followpos` function of Aho's
/// algorithm.
fn calc_follow_pos(nodes: &mut RBBINodes, n: Option<NodeId>) {
    let Some(n) = n else {
        return;
    };
    let t = nodes[n].f_type;
    if t == LEAF_CHAR || t == END_MARK {
        return;
    }

    let (l, r) = (nodes[n].f_left_child, nodes[n].f_right_child);
    calc_follow_pos(nodes, l);
    calc_follow_pos(nodes, r);

    // Aho rule #1
    if t == OP_CAT {
        let last: Vec<NodeId> = nodes[child(l)].f_last_pos_set.iter().copied().collect();
        let first = nodes[child(r)].f_first_pos_set.clone();
        for i in last {
            nodes[i].f_follow_pos.extend(first.iter().copied());
        }
    }

    // Aho rule #2
    if t == OP_STAR || t == OP_PLUS {
        let last: Vec<NodeId> = nodes[n].f_last_pos_set.iter().copied().collect();
        let first = nodes[n].f_first_pos_set.clone();
        for i in last {
            nodes[i].f_follow_pos.extend(first.iter().copied());
        }
    }
}

/// `addRuleRootNodes(List<RBBINode> dest, RBBINode node)`: collects the
/// rule roots of a tree.
fn add_rule_root_nodes(nodes: &RBBINodes, dest: &mut Vec<NodeId>, node: Option<NodeId>) {
    let Some(node) = node else {
        return;
    };
    if nodes[node].f_rule_root {
        dest.push(node);
        // Note: rules cannot nest. If we found a rule start node,
        //       no child node can also be a start node.
        return;
    }
    add_rule_root_nodes(nodes, dest, nodes[node].f_left_child);
    add_rule_root_nodes(nodes, dest, nodes[node].f_right_child);
}

/// `calcChainedFollowPos(RBBINode tree, RBBINode endMarkNode)`: for
/// chained rules, adds the starts of the chain-in rules to the follow
/// positions of the leaves that can end a match.
fn calc_chained_follow_pos(nodes: &mut RBBINodes, tree: NodeId, end_mark_node: NodeId) {
    let mut leaf_nodes = Vec::new();

    // get a list all leaf nodes
    nodes.find_nodes(tree, &mut leaf_nodes, LEAF_CHAR);

    // Collect all leaf nodes that can start matches for rules
    // with inbound chaining enabled, which is the union of the
    // firstPosition sets from each of the rule root nodes.

    let mut rule_root_nodes = Vec::new();
    add_rule_root_nodes(nodes, &mut rule_root_nodes, Some(tree));

    let mut match_start_nodes: BTreeSet<NodeId> = BTreeSet::new();
    for &node in &rule_root_nodes {
        if nodes[node].f_chain_in {
            match_start_nodes.extend(nodes[node].f_first_pos_set.iter().copied());
        }
    }

    // Iterate over all leaf nodes,
    //
    for &end_node in &leaf_nodes {
        // Identify leaf nodes that correspond to overall rule match positions.
        // These include the endMarkNode in their followPos sets.
        //
        // Note: do not consider other end marker nodes, those that are added to
        //       look-ahead rules. These can't chain; a match immediately stops
        //       further matching. This leaves exactly one end marker node, the one
        //       at the end of the complete tree.

        if !nodes[end_node].f_follow_pos.contains(&end_mark_node) {
            continue;
        }

        // We've got a node that can end a match.

        // Now iterate over the nodes that can start a match, looking for ones
        //   with the same char class as our ending node.
        for &start_node in &match_start_nodes {
            if nodes[start_node].f_type != LEAF_CHAR {
                continue;
            }

            if nodes[end_node].f_val == nodes[start_node].f_val {
                // The end val (character class) of one possible match is the
                //   same as the start of another.

                // Add all nodes from the followPos of the start node to the
                //  followPos set of the end node, which will have the effect of
                //  letting matches transition from a match state at endNode
                //  to the second char of a match starting with startNode.
                let follow = nodes[start_node].f_follow_pos.clone();
                nodes[end_node].f_follow_pos.extend(follow);
            }
        }
    }
}

/// `printPosSets(RBBINode n)`: debug.
fn print_pos_sets(nodes: &RBBINodes, n: Option<NodeId>) {
    let Some(n) = n else {
        return;
    };
    nodes.print_node(Some(n));
    print!("         Nullable:  {}", nodes[n].f_nullable);

    print!("         firstpos:  ");
    print_set(nodes, &nodes[n].f_first_pos_set);

    print!("         lastpos:   ");
    print_set(nodes, &nodes[n].f_last_pos_set);

    print!("         followpos: ");
    print_set(nodes, &nodes[n].f_follow_pos);

    print_pos_sets(nodes, nodes[n].f_left_child);
    print_pos_sets(nodes, nodes[n].f_right_child);
}

/// `printSet(Collection<RBBINode> s)`: debug.
fn print_set(nodes: &RBBINodes, s: &BTreeSet<NodeId>) {
    for &n in s {
        rbbi_node::print_int(nodes[n].f_serial_num, 8);
    }
    println!();
}
