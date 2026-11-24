// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.text.RBBISetBuilder`: the character categories of the RBBI
//! rule compiler.
//!
//! Starting with the rules' parse tree and the list of all `UnicodeSet`s
//! they reference, it builds the set of non-overlapping ranges of
//! characters (character categories), the trie mapping characters to
//! categories, and the `leafChar` nodes that tie the sets to the
//! categories.

use rucene::error::Result;

use crate::jdk::OutputStream;
use crate::r#impl::assert::Assert;
use crate::text::rbbi_node::{self, NodeId, LEAF_CHAR, OP_OR, VAR_REF};
use crate::text::rbbi_rule_builder::{IntPair, RBBIRuleBuilder};
use crate::text::rule_based_break_iterator::ByteArrayOutputStream;
use crate::util::code_point_trie::{CodePointTrie, Type as TrieType, ValueWidth};
use crate::util::mutable_code_point_trie::MutableCodePointTrie;

/// The index of a range in the set builder's range arena.
type RangeId = usize;

/// A range of characters that all belong to the same sets.
///
/// Equivalent to the package-private static nested class
/// `RBBISetBuilder.RangeDescriptor`; its `fNext` link is an arena index.
#[derive(Debug, Clone, Default)]
pub(crate) struct RangeDescriptor {
    /// `fStartChar`: inclusive.
    f_start_char: i32,
    /// `fEndChar`: inclusive.
    f_end_char: i32,
    /// `fNum`: the value assigned to this range.
    f_num: i32,
    /// `fIncludesDict`: if the range includes characters from the original
    /// set "dictionary".
    f_includes_dict: bool,
    /// `fFirstInGroup`: first range in a group with the same fNum.
    f_first_in_group: bool,
    /// `fIncludesSets`: the `uset` nodes of the sets including this range.
    f_includes_sets: Vec<NodeId>,
    /// `fNext`: next range in the list.
    f_next: Option<RangeId>,
}

/// Builds the character categories and the trie of a rule compilation.
///
/// Equivalent to the package-private class `com.ibm.icu.text.RBBISetBuilder`;
/// its `fRB` back reference is the `rb` parameter of the methods that use
/// it, and its linked list of ranges is an arena.
#[derive(Debug, Default)]
pub(crate) struct RBBISetBuilder {
    /// The `RangeDescriptor`s.
    ranges: Vec<RangeDescriptor>,
    /// `fRangeList`: the head of the range list.
    f_range_list: Option<RangeId>,
    /// `fTrie`: the mapping trie, while being built.
    f_trie: Option<MutableCodePointTrie>,
    /// `fFrozenTrie`: the mapping trie, after it is frozen.
    f_frozen_trie: Option<CodePointTrie>,
    /// `fGroupCount`: groups (categories) built from the ranges.
    f_group_count: i32,
    /// `fDictCategoriesStart`: first category that is "dictionary".
    f_dict_categories_start: i32,
    /// `fSawBOF`.
    f_saw_bof: bool,
}

/// `MAX_CHAR_CATEGORIES_FOR_8BITS_TRIE`.
const MAX_CHAR_CATEGORIES_FOR_8BITS_TRIE: i32 = 255;

impl RBBISetBuilder {
    /// `RBBISetBuilder(RBBIRuleBuilder rb)`.
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// `RangeDescriptor.split(int where)`: splits a range in two at
    /// `where_`; the new range holds the upper part.
    fn split(&mut self, r: RangeId, where_: i32) -> Result<()> {
        Assert::assrt(where_ > self.ranges[r].f_start_char && where_ <= self.ranges[r].f_end_char)?;
        let mut nr = self.ranges[r].clone();

        // RangeDescriptor copy constructor copies all fields.
        //  Only need to update those that are different after the split.
        nr.f_start_char = where_;
        self.ranges[r].f_end_char = where_ - 1;
        nr.f_next = self.ranges[r].f_next;
        self.ranges.push(nr);
        self.ranges[r].f_next = Some(self.ranges.len() - 1);

        // TODO:  fIncludesSets is not updated with the sets that the new range is in.
        Ok(())
    }

    /// `RangeDescriptor.isDictionaryRange()`: whether one of the range's
    /// sets is the variable `$dictionary`.
    fn is_dictionary_range(&self, rb: &RBBIRuleBuilder, r: RangeId) -> bool {
        for &uset_node in &self.ranges[r].f_includes_sets {
            let mut set_name: Vec<u16> = Vec::new();
            if let Some(set_ref) = rb.nodes[uset_node].f_parent {
                if let Some(var_ref) = rb.nodes[set_ref].f_parent {
                    if rb.nodes[var_ref].f_type == VAR_REF {
                        set_name = rb.nodes[var_ref].f_text.clone().unwrap_or_default();
                    }
                }
            }
            if set_name == crate::jdk::units("dictionary") {
                return true;
            }
        }
        false
    }

    /// The ids of the ranges, in list order.
    fn range_ids(&self) -> Vec<RangeId> {
        let mut ids = Vec::new();
        let mut r = self.f_range_list;
        while let Some(id) = r {
            ids.push(id);
            r = self.ranges[id].f_next;
        }
        ids
    }

    /// `buildRanges()`: builds the ranges from the sets, numbers the
    /// groups, and adds the `leafChar` category nodes to the sets.
    ///
    /// # Errors
    ///
    /// `IllegalStateException` (`Assert.assrt`) for an internal
    /// inconsistency.
    pub(crate) fn build_ranges(&mut self, rb: &mut RBBIRuleBuilder) -> Result<()> {
        if rb.f_debug_env.as_deref().is_some_and(|e| e.contains("usets")) {
            self.print_sets(rb);
        }

        //  Initialize the process by creating a single range encompassing all characters
        //  that is in no sets.
        //
        self.ranges.push(RangeDescriptor {
            f_start_char: 0,
            f_end_char: 0x10ffff,
            ..RangeDescriptor::default()
        });
        self.f_range_list = Some(self.ranges.len() - 1);

        //
        //  Find the set of non-overlapping ranges of characters
        //
        let uset_nodes = rb.f_u_set_nodes.clone();
        for &uset_node in &uset_nodes {
            let input_set = rb.nodes[uset_node]
                .f_input_set
                .clone()
                .unwrap_or_else(|| panic!("java.lang.NullPointerException: fInputSet"));
            let input_set_range_count = input_set.get_range_count();
            let mut input_set_range_index = 0;
            let mut rl_range = self.f_range_list.expect("range list");

            loop {
                if input_set_range_index >= input_set_range_count {
                    break;
                }
                let input_set_range_begin = input_set.get_range_start(input_set_range_index);
                let input_set_range_end = input_set.get_range_end(input_set_range_index);

                // skip over ranges from the range list that are completely
                //   below the current range from the input unicode set.
                while self.ranges[rl_range].f_end_char < input_set_range_begin {
                    rl_range = rbbi_node::child(self.ranges[rl_range].f_next);
                }

                // If the start of the range from the range list is before with
                //   the start of the range from the unicode set, split the range list range
                //   in two, with one part being before (wholly outside of) the unicode set
                //   and the other containing the rest.
                //   Then continue the loop; the post-split current range will then be skipped
                //     over
                if self.ranges[rl_range].f_start_char < input_set_range_begin {
                    self.split(rl_range, input_set_range_begin)?;
                    continue;
                }

                // Same thing at the end of the ranges...
                // If the end of the range from the range list doesn't coincide with
                //   the end of the range from the unicode set, split the range list
                //   range in two.  The first part of the split range will be
                //   wholly inside the Unicode set.
                if self.ranges[rl_range].f_end_char > input_set_range_end {
                    self.split(rl_range, input_set_range_end + 1)?;
                }

                // The current rlRange is now entirely within the UnicodeSet range.
                // Add this unicode set to the list of sets for this rlRange
                if !self.ranges[rl_range].f_includes_sets.contains(&uset_node) {
                    self.ranges[rl_range].f_includes_sets.push(uset_node);
                }

                // Advance over ranges that we are finished with.
                if input_set_range_end == self.ranges[rl_range].f_end_char {
                    input_set_range_index += 1;
                }
                match self.ranges[rl_range].f_next {
                    Some(next) => rl_range = next,
                    None => {
                        // Java's rlRange becomes null; the next use (if any) throws.
                        if input_set_range_index < input_set_range_count {
                            panic!("java.lang.NullPointerException: rlRange");
                        }
                        break;
                    }
                }
            }
        }

        if rb.f_debug_env.as_deref().is_some_and(|e| e.contains("range")) {
            self.print_ranges(rb);
        }

        //
        //  Group the above ranges, with each group consisting of one or more
        //    ranges that are in exactly the same set of original UnicodeSets.
        //    The groups are numbered, and these group numbers are the set of
        //    input symbols recognized by the run-time state machine.
        //
        //    Numbering: # 0  (state table column 0) is unused.
        //               # 1  is reserved - table column 1 is for end-of-input
        //               # 2  is reserved - table column 2 is for beginning-of-input
        //               # 3  is the first range list.
        //
        let mut dict_group_count = 0;

        let ids = self.range_ids();
        for (pos, &rl_range) in ids.iter().enumerate() {
            for &rl_search_range in &ids[..pos] {
                if self.ranges[rl_range].f_includes_sets == self.ranges[rl_search_range].f_includes_sets {
                    self.ranges[rl_range].f_num = self.ranges[rl_search_range].f_num;
                    self.ranges[rl_range].f_includes_dict = self.ranges[rl_search_range].f_includes_dict;
                    break;
                }
            }
            if self.ranges[rl_range].f_num == 0 {
                self.ranges[rl_range].f_first_in_group = true;
                if self.is_dictionary_range(rb, rl_range) {
                    dict_group_count += 1;
                    self.ranges[rl_range].f_num = dict_group_count;
                    self.ranges[rl_range].f_includes_dict = true;
                } else {
                    self.f_group_count += 1;
                    self.ranges[rl_range].f_num = self.f_group_count + 2;
                    let sets = self.ranges[rl_range].f_includes_sets.clone();
                    Self::add_val_to_sets(rb, &sets, self.f_group_count + 2)?;
                }
            }
        }

        // Move the character categories for Dictionary Ranges up, so that they
        // immediately follow the non-dictionary ranges.

        self.f_dict_categories_start = self.f_group_count + 3;
        for &rl_range in &ids {
            if self.ranges[rl_range].f_includes_dict {
                self.ranges[rl_range].f_num += self.f_dict_categories_start - 1;
                if self.ranges[rl_range].f_first_in_group {
                    let sets = self.ranges[rl_range].f_includes_sets.clone();
                    Self::add_val_to_sets(rb, &sets, self.ranges[rl_range].f_num)?;
                }
            }
        }
        self.f_group_count += dict_group_count;

        // Handle input sets that contain the special string {eof}.
        //   Column 1 of the state table is reserved for EOF on input.
        //   Column 2 is reserved for before-the-start-input.
        //            (This column can be optimized away later if there are no rule
        //             references to {bof}.)
        //   Add this column value (1 or 2) to the equivalent expression
        //     subtree for each UnicodeSet that contains the string {eof}
        //   Because {bof} and {eof} are not a characters in the normal sense,
        //   they doesn't affect the computation of ranges or TRIE.

        let eof_string = crate::jdk::units("eof");
        let bof_string = crate::jdk::units("bof");

        for &uset_node in &uset_nodes {
            let input_set = rb.nodes[uset_node]
                .f_input_set
                .clone()
                .unwrap_or_else(|| panic!("java.lang.NullPointerException: fInputSet"));
            if input_set.contains_str(&eof_string) {
                Self::add_val_to_set(rb, uset_node, 1)?;
            }
            if input_set.contains_str(&bof_string) {
                Self::add_val_to_set(rb, uset_node, 2)?;
                self.f_saw_bof = true;
            }
        }

        if rb.f_debug_env.as_deref().is_some_and(|e| e.contains("rgroup")) {
            self.print_range_groups(rb);
        }
        if rb.f_debug_env.as_deref().is_some_and(|e| e.contains("esets")) {
            self.print_sets(rb);
        }
        Ok(())
    }

    /// `buildTrie()`: builds the mutable trie of the categories.
    ///
    /// # Errors
    ///
    /// The exceptions of the trie.
    pub(crate) fn build_trie(&mut self) -> Result<()> {
        let mut trie = MutableCodePointTrie::new(
            0, // Initial value for all code points.
            0, // Error value for out-of-range input.
        );

        for rl_range in self.range_ids() {
            let r = &self.ranges[rl_range];
            trie.set_range(
                r.f_start_char, // Range start
                r.f_end_char,   // Range end (inclusive)
                r.f_num,        // value for range
            )?;
        }
        self.f_trie = Some(trie);
        Ok(())
    }

    /// `mergeCategories(IntPair categories)`: merges category
    /// `categories.second` into `categories.first`.
    pub(crate) fn merge_categories(&mut self, categories: &IntPair) {
        debug_assert!(categories.first >= 1);
        debug_assert!(categories.second > categories.first);
        debug_assert!(
            (categories.first < self.f_dict_categories_start && categories.second < self.f_dict_categories_start)
                || (categories.first >= self.f_dict_categories_start
                    && categories.second >= self.f_dict_categories_start)
        );
        for rd in self.range_ids() {
            let range_num = self.ranges[rd].f_num;
            if range_num == categories.second {
                self.ranges[rd].f_num = categories.first;
            } else if range_num > categories.second {
                self.ranges[rd].f_num -= 1;
            }
        }
        self.f_group_count -= 1;
        if categories.second <= self.f_dict_categories_start {
            self.f_dict_categories_start -= 1;
        }
    }

    /// `freezeTrieIfNotYet()`: freezes the trie, 8-bit values when the
    /// categories fit.
    ///
    /// # Errors
    ///
    /// The exceptions of the trie build.
    fn freeze_trie_if_not_yet(&mut self) -> Result<()> {
        if self.f_frozen_trie.is_none() {
            let use_8_bits = self.get_num_char_categories() <= MAX_CHAR_CATEGORIES_FOR_8BITS_TRIE;
            let mut trie = self
                .f_trie
                .take()
                .unwrap_or_else(|| panic!("java.lang.NullPointerException: fTrie"));
            self.f_frozen_trie = Some(trie.build_immutable(
                Some(TrieType::Fast),
                Some(if use_8_bits { ValueWidth::Bits8 } else { ValueWidth::Bits16 }),
            )?);
        }
        Ok(())
    }

    /// `getTrieSize()`: the size of the serialized trie, in bytes.
    ///
    /// # Errors
    ///
    /// The exceptions of the trie.
    pub(crate) fn get_trie_size(&mut self) -> Result<i32> {
        self.freeze_trie_if_not_yet()?;
        self.f_frozen_trie
            .as_ref()
            .expect("frozen above")
            .to_binary(&mut ByteArrayOutputStream::new())
    }

    /// `serializeTrie(OutputStream os)`: writes the serialized trie.
    ///
    /// # Errors
    ///
    /// The `IOException` of the stream.
    pub(crate) fn serialize_trie(&mut self, os: &mut dyn OutputStream) -> Result<()> {
        self.freeze_trie_if_not_yet()?;
        self.f_frozen_trie.as_ref().expect("frozen above").to_binary(os)?;
        Ok(())
    }

    /// `addValToSets(List<RBBINode> sets, int val)`.
    fn add_val_to_sets(rb: &mut RBBIRuleBuilder, sets: &[NodeId], val: i32) -> Result<()> {
        for &uset_node in sets {
            Self::add_val_to_set(rb, uset_node, val)?;
        }
        Ok(())
    }

    /// `addValToSet(RBBINode usetNode, int val)`: adds a `leafChar` node for
    /// category `val` to the expression of a set, or-ed with what is there.
    fn add_val_to_set(rb: &mut RBBIRuleBuilder, uset_node: NodeId, val: i32) -> Result<()> {
        let leaf_node = rb.nodes.new_node(LEAF_CHAR)?;
        rb.nodes[leaf_node].f_val = val;
        match rb.nodes[uset_node].f_left_child {
            None => {
                rb.nodes[uset_node].f_left_child = Some(leaf_node);
                rb.nodes[leaf_node].f_parent = Some(uset_node);
            }
            Some(left) => {
                // There are already input symbols present for this set.
                // Set up an OR node, with the previous stuff as the left child
                //   and the new value as the right child.
                let or_node = rb.nodes.new_node(OP_OR)?;
                rb.nodes[or_node].f_left_child = Some(left);
                rb.nodes[or_node].f_right_child = Some(leaf_node);
                rb.nodes[left].f_parent = Some(or_node);
                rb.nodes[leaf_node].f_parent = Some(or_node);
                rb.nodes[uset_node].f_left_child = Some(or_node);
                rb.nodes[or_node].f_parent = Some(uset_node);
            }
        }
        Ok(())
    }

    /// `getNumCharCategories()`: the number of character categories,
    /// including the reserved 0, 1 and 2.
    pub(crate) fn get_num_char_categories(&self) -> i32 {
        self.f_group_count + 3
    }

    /// `getDictCategoriesStart()`: the first dictionary category.
    pub(crate) fn get_dict_categories_start(&self) -> i32 {
        self.f_dict_categories_start
    }

    /// `sawBOF()`: whether some rule references `{bof}`.
    pub(crate) fn saw_bof(&self) -> bool {
        self.f_saw_bof
    }

    /// `getFirstChar(int category)`: the first character of a category,
    /// `-1` when none.
    #[allow(dead_code)]
    pub(crate) fn get_first_char(&self, category: i32) -> i32 {
        let mut ret_val = -1;
        for rl_range in self.range_ids() {
            if self.ranges[rl_range].f_num == category {
                ret_val = self.ranges[rl_range].f_start_char;
                break;
            }
        }
        ret_val
    }

    /// The set name of a `uset` node: its `$variable`, or `anon`.
    fn set_name(rb: &RBBIRuleBuilder, uset_node: NodeId, anon: &str) -> String {
        let mut set_name = anon.to_string();
        if let Some(set_ref) = rb.nodes[uset_node].f_parent {
            if let Some(var_ref) = rb.nodes[set_ref].f_parent {
                if rb.nodes[var_ref].f_type == VAR_REF {
                    set_name = rbbi_node::text_string(&rb.nodes[var_ref].f_text);
                }
            }
        }
        set_name
    }

    /// `printRanges()`: debug.
    fn print_ranges(&self, rb: &RBBIRuleBuilder) {
        print!("\n\n Nonoverlapping Ranges ...\n");
        for rl_range in self.range_ids() {
            let r = &self.ranges[rl_range];
            print!("{:04x}-{:04x} ", r.f_start_char, r.f_end_char);

            for &uset_node in &r.f_includes_sets {
                print!("{}", Self::set_name(rb, uset_node, "anon"));
                print!("  ");
            }
            println!();
        }
    }

    /// `printRangeGroups()`: debug.
    fn print_range_groups(&self, rb: &RBBIRuleBuilder) {
        print!("\nRanges grouped by Unicode Set Membership...\n");
        let ids = self.range_ids();
        for (pos, &rl_range) in ids.iter().enumerate() {
            let r = &self.ranges[rl_range];
            if r.f_first_in_group {
                let group_num = r.f_num;
                if group_num < 10 {
                    print!(" ");
                }
                print!("{group_num} ");

                if group_num >= self.f_dict_categories_start {
                    print!(" <DICT> ");
                }

                for &uset_node in &r.f_includes_sets {
                    print!("{}", Self::set_name(rb, uset_node, "anon"));
                    print!(" ");
                }

                let mut i = 0;
                for &t_range in &ids[pos..] {
                    let t = &self.ranges[t_range];
                    if t.f_num == r.f_num {
                        if i % 5 == 0 {
                            print!("\n    ");
                        }
                        i += 1;
                        rbbi_node::print_hex(t.f_start_char, -1);
                        print!("-");
                        rbbi_node::print_hex(t.f_end_char, 0);
                    }
                }
                println!();
            }
        }
        println!();
    }

    /// `printSets()`: debug.
    fn print_sets(&self, rb: &RBBIRuleBuilder) {
        print!("\n\nUnicode Sets List\n------------------\n");
        for (i, &uset_node) in rb.f_u_set_nodes.iter().enumerate() {
            //System.out.print(" " + i + "   ");
            rbbi_node::print_int(2, i as i32);
            let set_name = Self::set_name(rb, uset_node, "anonymous");
            print!("  {set_name}");
            print!("   ");
            print!("{}", rbbi_node::text_string(&rb.nodes[uset_node].f_text));
            println!();
            if let Some(l) = rb.nodes[uset_node].f_left_child {
                rb.nodes.print_tree(l, true);
            }
        }
        println!();
    }
}

