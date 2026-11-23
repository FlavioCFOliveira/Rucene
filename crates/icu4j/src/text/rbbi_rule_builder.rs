// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.text.RBBIRuleBuilder`: the top level of the RBBI rule
//! compiler.

use std::collections::{BTreeSet, HashMap};

use rucene::error::Result;

use crate::jdk::{to_rust_string, DataOutputStream, OutputStream};
use crate::r#impl::assert::Assert;
use crate::r#impl::icu_binary::ICUBinary;
use crate::r#impl::icu_debug::ICUDebug;
use crate::r#impl::rbbi_data_wrapper::RBBIDataWrapper;
use crate::text::rbbi_node::{NodeId, RBBINodes};
use crate::text::rbbi_rule_scanner::RBBIRuleScanner;
use crate::text::rbbi_set_builder::RBBISetBuilder;
use crate::text::rbbi_table_builder::RBBITableBuilder;

/// A pair of ints, used to bundle pairs of states or pairs of character
/// classes.
///
/// Equivalent to the package-private static nested class
/// `RBBIRuleBuilder.IntPair`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct IntPair {
    /// `first`.
    pub(crate) first: i32,
    /// `second`.
    pub(crate) second: i32,
}

impl IntPair {
    /// `IntPair(int f, int s)`.
    pub(crate) fn new(f: i32, s: i32) -> Self {
        Self { first: f, second: s }
    }
}

/// The compiler of break-iterator rules into binary rule data.
///
/// Equivalent to the package-private class `com.ibm.icu.text.RBBIRuleBuilder`.
/// It owns the node arena of the compilation; the scanner, set builder and
/// table builder it creates take it as a parameter in place of their
/// `fRB` back reference (each is taken out of its field while it runs).
#[derive(Debug)]
pub(crate) struct RBBIRuleBuilder {
    //   This is the main class for building (compiling) break rules into the tables
    //    required by the runtime RBBI engine.
    //
    /// `fDebugEnv`: the debug environment string.
    pub(crate) f_debug_env: Option<String>,
    /// `fRules`: the rules source.
    pub(crate) f_rules: Vec<u16>,
    /// `fStrippedRules`: the rules, with comments blanked out.
    pub(crate) f_stripped_rules: Vec<u16>,
    /// `fScanner`.
    pub(crate) f_scanner: Option<RBBIRuleScanner>,

    //
    //  There are four separate parse trees generated, one for each of the
    //    forward rules, reverse rules, safe forward rules and safe reverse rules.
    //  This array references the root of each of the trees.
    //  Only fForwardTree data is actually used to generate a state table.
    //  The other three are retained for back compatibility with old rule files.
    //
    /// `fTreeRoots`.
    pub(crate) f_tree_roots: [Option<NodeId>; 4],
    /// `fDefaultTree`: for rules not qualified with a !, the tree to which
    /// they belong to.
    pub(crate) f_default_tree: usize,
    /// `fChainRules`: true for chained Unicode TR style rules.
    pub(crate) f_chain_rules: bool,
    /// `fLookAheadHardBreak`: true if look ahead rules are to break as
    /// soon as the look ahead finishes.
    pub(crate) f_look_ahead_hard_break: bool,
    /// `fSetBuilder`: the set and range builder for this rule set.
    pub(crate) f_set_builder: Option<RBBISetBuilder>,
    /// `fUSetNodes`: list of all uset nodes that were created during
    /// parsing.
    pub(crate) f_u_set_nodes: Vec<NodeId>,
    /// `fForwardTable`: the forward state table data.
    pub(crate) f_forward_table: Option<RBBITableBuilder>,
    /// `fStatusSets`: maps the sets of status (tag) values to their index
    /// in fRuleStatusVals.
    pub(crate) f_status_sets: HashMap<BTreeSet<i32>, i32>,
    /// `fRuleStatusVals`: the status values themselves; the runtime
    /// format of the status values table.
    pub(crate) f_rule_status_vals: Vec<i32>,
    /// The nodes of the parse trees.
    pub(crate) nodes: RBBINodes,
}

impl RBBIRuleBuilder {
    /// `fForwardTree`.
    pub(crate) const F_FORWARD_TREE: usize = 0;
    /// `fReverseTree`.
    pub(crate) const F_REVERSE_TREE: usize = 1;
    /// `fSafeFwdTree`.
    pub(crate) const F_SAFE_FWD_TREE: usize = 2;
    /// `fSafeRevTree`.
    pub(crate) const F_SAFE_REV_TREE: usize = 3;

    //
    // Error codes from ICU4C.
    //
    /// `U_ILLEGAL_CHAR_FOUND`: character conversion: illegal input
    /// sequence/combination of input units.
    pub(crate) const U_ILLEGAL_CHAR_FOUND: i32 = 12;
    /// `U_BRK_ERROR_START`: start of codes indicating Break Iterator
    /// failures.
    #[allow(dead_code)]
    pub(crate) const U_BRK_ERROR_START: i32 = 0x10200;
    /// `U_BRK_INTERNAL_ERROR`: an internal error (bug) was detected.
    pub(crate) const U_BRK_INTERNAL_ERROR: i32 = 0x10201;
    /// `U_BRK_HEX_DIGITS_EXPECTED`: hex digits expected as part of a
    /// escaped char in a rule.
    pub(crate) const U_BRK_HEX_DIGITS_EXPECTED: i32 = 0x10202;
    /// `U_BRK_SEMICOLON_EXPECTED`: missing ';' at the end of a RBBI rule.
    #[allow(dead_code)]
    pub(crate) const U_BRK_SEMICOLON_EXPECTED: i32 = 0x10203;
    /// `U_BRK_RULE_SYNTAX`: syntax error in RBBI rule.
    pub(crate) const U_BRK_RULE_SYNTAX: i32 = 0x10204;
    /// `U_BRK_UNCLOSED_SET`: UnicodeSet writing an RBBI rule missing a
    /// closing ']'.
    #[allow(dead_code)]
    pub(crate) const U_BRK_UNCLOSED_SET: i32 = 0x10205;
    /// `U_BRK_ASSIGN_ERROR`: syntax error in RBBI rule assignment
    /// statement.
    pub(crate) const U_BRK_ASSIGN_ERROR: i32 = 0x10206;
    /// `U_BRK_VARIABLE_REDFINITION`: RBBI rule $Variable redefined.
    pub(crate) const U_BRK_VARIABLE_REDFINITION: i32 = 0x10207;
    /// `U_BRK_MISMATCHED_PAREN`: mis-matched parentheses in an RBBI rule.
    pub(crate) const U_BRK_MISMATCHED_PAREN: i32 = 0x10208;
    /// `U_BRK_NEW_LINE_IN_QUOTED_STRING`: missing closing quote in an RBBI
    /// rule.
    pub(crate) const U_BRK_NEW_LINE_IN_QUOTED_STRING: i32 = 0x10209;
    /// `U_BRK_UNDEFINED_VARIABLE`: use of an undefined $Variable in an RBBI
    /// rule.
    pub(crate) const U_BRK_UNDEFINED_VARIABLE: i32 = 0x1020a;
    /// `U_BRK_INIT_ERROR`: initialization failure. Probable missing ICU
    /// Data.
    #[allow(dead_code)]
    pub(crate) const U_BRK_INIT_ERROR: i32 = 0x1020b;
    /// `U_BRK_RULE_EMPTY_SET`: rule contains an empty Unicode Set.
    pub(crate) const U_BRK_RULE_EMPTY_SET: i32 = 0x1020c;
    /// `U_BRK_UNRECOGNIZED_OPTION`: !!option in RBBI rules not recognized.
    pub(crate) const U_BRK_UNRECOGNIZED_OPTION: i32 = 0x1020d;
    /// `U_BRK_MALFORMED_RULE_TAG`: the {nnn} tag on a rule is malformed.
    pub(crate) const U_BRK_MALFORMED_RULE_TAG: i32 = 0x1020e;
    /// `U_BRK_MALFORMED_SET`: a set expression is malformed.
    pub(crate) const U_BRK_MALFORMED_SET: i32 = 0x1020f;
    /// `U_BRK_ERROR_LIMIT`: this must always be the last value to indicate
    /// the limit for Break Iterator failures.
    #[allow(dead_code)]
    pub(crate) const U_BRK_ERROR_LIMIT: i32 = 0x10210;

    /// `RBBIRuleBuilder(String rules)`.
    ///
    /// # Errors
    ///
    /// The exceptions of the scanner's `UnicodeSet`s.
    pub(crate) fn new(rules: &[u16]) -> Result<Self> {
        Ok(Self {
            f_debug_env: if ICUDebug::enabled_arg("rbbi") {
                Some(ICUDebug::value("rbbi"))
            } else {
                None
            },
            f_rules: rules.to_vec(),
            f_stripped_rules: rules.to_vec(),
            f_scanner: Some(RBBIRuleScanner::new()?),
            f_tree_roots: [None; 4],
            f_default_tree: Self::F_FORWARD_TREE,
            f_chain_rules: false,
            f_look_ahead_hard_break: false,
            f_set_builder: Some(RBBISetBuilder::new()),
            f_u_set_nodes: Vec::new(),
            f_forward_table: None,
            f_status_sets: HashMap::new(),
            f_rule_status_vals: Vec::new(),
            nodes: RBBINodes::new(),
        })
    }

    /// `fScanner`.
    pub(crate) fn scanner(&self) -> &RBBIRuleScanner {
        self.f_scanner.as_ref().expect("the scanner is in place")
    }

    /// `fSetBuilder`.
    pub(crate) fn set_builder(&self) -> &RBBISetBuilder {
        self.f_set_builder.as_ref().expect("the set builder is in place")
    }

    /// `fSetBuilder`, mutable.
    fn set_builder_mut(&mut self) -> &mut RBBISetBuilder {
        self.f_set_builder.as_mut().expect("the set builder is in place")
    }

    /// `fForwardTable`.
    fn forward_table(&self) -> &RBBITableBuilder {
        self.f_forward_table.as_ref().expect("the forward table is built")
    }

    /// `align8(int i)`.
    fn align8(i: i32) -> i32 {
        (i + 7) & 0xfffffff8u32 as i32
    }

    /// `flattenData(OutputStream os)`: writes the binary rule data.
    ///
    /// # Errors
    ///
    /// The `IOException` of the stream; `IllegalStateException`
    /// (`Assert.assrt`) for an internal inconsistency.
    fn flatten_data(&mut self, os: &mut dyn OutputStream) -> Result<()> {
        // Remove whitespace from the rules to make it smaller.
        // The rule parser has already removed comments.
        let stripped_rules = RBBIRuleScanner::strip_rules(&self.f_stripped_rules);

        // Calculate the size of each section in the data in bytes.
        //   Sizes here are padded up to a multiple of 8 for better memory alignment.
        //   Sections sizes actually stored in the header are for the actual data
        //     without the padding.
        //
        let header_size = RBBIDataWrapper::DH_SIZE * 4; // align8(sizeof(RBBIDataHeader));
        let forward_table_size = Self::align8(self.forward_table().get_table_size(self));
        let reverse_table_size = Self::align8(self.forward_table().get_safe_table_size());
        let trie_size = Self::align8(self.set_builder_mut().get_trie_size()?);
        let status_table_size = Self::align8(self.f_rule_status_vals.len() as i32 * 4);

        let stripped_rules_utf8: Vec<u8> = to_rust_string(&stripped_rules).into_bytes();
        let rules_size = Self::align8(stripped_rules_utf8.len() as i32 + 1);

        let total_size =
            header_size + forward_table_size + reverse_table_size + status_table_size + trie_size + rules_size;
        let mut output_pos = 0; // Track stream position, starting from RBBIDataHeader.

        let mut header = [0i32; RBBIDataWrapper::DH_SIZE as usize];
        {
            let mut dos = DataOutputStream::new(os);
            //
            // Write out an ICU Data Header
            //
            ICUBinary::write_header(RBBIDataWrapper::DATA_FORMAT, RBBIDataWrapper::FORMAT_VERSION, 0, &mut dos)?;

            //
            // Write out the RBBIDataHeader
            //
            header[RBBIDataWrapper::DH_MAGIC as usize] = 0xb1a0;
            header[RBBIDataWrapper::DH_FORMATVERSION as usize] = RBBIDataWrapper::FORMAT_VERSION;
            header[RBBIDataWrapper::DH_LENGTH as usize] = total_size; // fLength, the total size of all rule sections.
            header[RBBIDataWrapper::DH_CATCOUNT as usize] = self.set_builder().get_num_char_categories(); // fCatCount.

            header[RBBIDataWrapper::DH_FTABLE as usize] = header_size; // fFTable
            header[RBBIDataWrapper::DH_FTABLELEN as usize] = forward_table_size; // fTableLen

            header[RBBIDataWrapper::DH_RTABLE as usize] = header[RBBIDataWrapper::DH_FTABLE as usize] + forward_table_size; // fRTable
            header[RBBIDataWrapper::DH_RTABLELEN as usize] = reverse_table_size; // fRTableLen

            header[RBBIDataWrapper::DH_TRIE as usize] =
                header[RBBIDataWrapper::DH_RTABLE as usize] + header[RBBIDataWrapper::DH_RTABLELEN as usize]; // fTrie
            header[RBBIDataWrapper::DH_TRIELEN as usize] = self.set_builder_mut().get_trie_size()?; // fTrieLen
            header[RBBIDataWrapper::DH_STATUSTABLE as usize] = header[RBBIDataWrapper::DH_TRIE as usize] + trie_size;
            header[RBBIDataWrapper::DH_STATUSTABLELEN as usize] = status_table_size; // fStatusTableLen
            header[RBBIDataWrapper::DH_RULESOURCE as usize] =
                header[RBBIDataWrapper::DH_STATUSTABLE as usize] + status_table_size;
            header[RBBIDataWrapper::DH_RULESOURCELEN as usize] = stripped_rules_utf8.len() as i32;
            for h in header {
                dos.write_int(h)?; // Write the header ints
                output_pos += 4;
            }

            // Write out the actual state tables.
            let table = self.forward_table().export_table(self)?;
            debug_assert!(output_pos == header[RBBIDataWrapper::DH_FTABLE as usize]);
            output_pos += table.put(&mut dos)?;

            let table = self.forward_table().export_safe_table()?;
            Assert::assrt(output_pos == header[RBBIDataWrapper::DH_RTABLE as usize])?;
            output_pos += table.put(&mut dos)?;

            // write out the Trie table
            Assert::assrt(output_pos == header[RBBIDataWrapper::DH_TRIE as usize])?;
        }
        self.set_builder_mut().serialize_trie(os)?;
        let mut dos = DataOutputStream::new(os);
        output_pos += header[RBBIDataWrapper::DH_TRIELEN as usize];
        while output_pos % 8 != 0 {
            // pad to an 8 byte boundary
            dos.write_byte(0)?;
            output_pos += 1;
        }

        // Write out the status {tag} table.
        Assert::assrt(output_pos == header[RBBIDataWrapper::DH_STATUSTABLE as usize])?;
        for &val in &self.f_rule_status_vals {
            dos.write_int(val)?;
            output_pos += 4;
        }

        while output_pos % 8 != 0 {
            // pad to an 8 byte boundary
            dos.write_byte(0)?;
            output_pos += 1;
        }

        // Write out the stripped rules (rules with extra spaces removed
        //   These go last in the data area, even though they are not last in the header.
        Assert::assrt(output_pos == header[RBBIDataWrapper::DH_RULESOURCE as usize])?;
        for &b in &stripped_rules_utf8 {
            dos.write_byte(b as i32)?;
        }
        dos.write_byte(0)?; // Null termination
        output_pos += stripped_rules_utf8.len() as i32 + 1;
        while output_pos % 8 != 0 {
            // pad to an 8 byte boundary
            dos.write_byte(0)?;
            output_pos += 1;
        }
        Ok(())
    }

    /// `compileRules(String rules, OutputStream os)`: compiles `rules` and
    /// writes the binary rule data to `os`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for a rule syntax error; the
    /// `IOException` of the stream.
    pub(crate) fn compile_rules(rules: &[u16], os: &mut dyn OutputStream) -> Result<()> {
        //
        // Read the input rules, generate a parse tree, symbol table,
        // and list of all Unicode Sets referenced by the rules.
        //
        let mut builder = RBBIRuleBuilder::new(rules)?;
        builder.build(os)
    }

    /// `build(OutputStream os)`: compiles the rules of this builder.
    ///
    /// # Errors
    ///
    /// As [`compile_rules`](Self::compile_rules).
    pub(crate) fn build(&mut self, os: &mut dyn OutputStream) -> Result<()> {
        let mut scanner = self.f_scanner.take().expect("the scanner is in place");
        let parsed = scanner.parse(self);
        self.f_scanner = Some(scanner);
        parsed?;

        //
        // UnicodeSet processing.
        //    Munge the Unicode Sets to create a set of character categories.
        //    Generate the mapping tables (TRIE) from input code points to
        //    the character categories.
        //
        let mut set_builder = self.f_set_builder.take().expect("the set builder is in place");
        let ranges = set_builder.build_ranges(self);
        self.f_set_builder = Some(set_builder);
        ranges?;

        //
        //   Generate the DFA state transition table.
        //
        let mut forward_table = RBBITableBuilder::new(Self::F_FORWARD_TREE);
        let built = forward_table.build_forward_table(self);
        self.f_forward_table = Some(forward_table);
        built?;

        //
        //   Optimize the DFA state transition table.
        //
        self.optimize_tables();
        let mut forward_table = self.f_forward_table.take().expect("the forward table is built");
        forward_table.build_safe_reverse_table(self);
        self.f_forward_table = Some(forward_table);

        if self.f_debug_env.as_deref().is_some_and(|e| e.contains("states")) {
            self.forward_table().print_states(self);
            self.forward_table().print_rule_status_table(self);
            self.forward_table().print_reverse_table();
        }

        //    Generate the mapping tables (TRIE) from input code points to
        //    the character categories.
        //
        self.set_builder_mut().build_trie()?;

        //
        //   Package up the compiled data, writing it to an output stream
        //      in the serialization format.  This is the same as the ICU4C runtime format.
        //
        self.flatten_data(os)
    }

    /// `optimizeTables()`: merges duplicate character categories and
    /// removes duplicate states until nothing changes.
    fn optimize_tables(&mut self) {
        let mut forward_table = self.f_forward_table.take().expect("the forward table is built");
        loop {
            let mut did_something = false;

            // Begin looking for duplicates with char class 3.
            // Classes 0, 1 and 2 are special; they are unused, {bof} and {eof} respectively,
            // and should not have other categories merged into them.
            let mut dupl_pair = IntPair::new(3, 0);
            while forward_table.find_dupl_char_class_from(self, &mut dupl_pair) {
                self.set_builder_mut().merge_categories(&dupl_pair);
                forward_table.remove_column(dupl_pair.second);
                did_something = true;
            }
            while forward_table.remove_duplicate_states(self) > 0 {
                did_something = true;
            }
            if !did_something {
                break;
            }
        }
        self.f_forward_table = Some(forward_table);
    }
}
