// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.text.RBBIRuleScanner`: the parser of break-iterator rule
//! source.

use std::collections::HashMap;
use std::sync::Arc;

use rucene::error::{LuceneError, Result};

use crate::jdk::{units, ParsePosition};
use crate::lang::u_character::UCharacter;
use crate::lang::u_property::UProperty;
use crate::r#impl::assert::Assert;
use crate::r#impl::utility::Utility;
use crate::text::rbbi_node::{
    child, NodeId, END_MARK, LOOK_AHEAD, OP_CAT, OP_L_PAREN, OP_OR, OP_PLUS, OP_QUESTION, OP_STAR,
    OP_START, PREC_L_PAREN, PREC_OP_CAT, PREC_START, SET_REF, TAG, USET, VAR_REF,
};
use crate::text::rbbi_rule_builder::RBBIRuleBuilder;
use crate::text::rbbi_rule_parse_table::{RBBIRuleParseTable as T, G_RULE_PARSE_STATE_TABLE};
use crate::text::rbbi_symbol_table::{RBBISymbolTable, RBBISymbolTableView};
use crate::text::unicode_set::UnicodeSet;
use crate::text::utf16::UTF16;

/// `kStackSize`.
const K_STACK_SIZE: usize = 100; // The size of the state stack for
                                 //   rules parsing.  Corresponds roughly
                                 //   to the depth of parentheses nesting
                                 //   that is allowed in the rules.

/// `gRuleSet_rule_char_pattern`.
const G_RULE_SET_RULE_CHAR_PATTERN: &str = "[^[\\p{Z}\\u0020-\\u007f]-[\\p{L}]-[\\p{N}]]";
/// `gRuleSet_name_char_pattern`.
const G_RULE_SET_NAME_CHAR_PATTERN: &str = "[_\\p{L}\\p{N}]";
/// `gRuleSet_digit_char_pattern`.
const G_RULE_SET_DIGIT_CHAR_PATTERN: &str = "[0-9]";
/// `gRuleSet_name_start_char_pattern`.
const G_RULE_SET_NAME_START_CHAR_PATTERN: &str = "[_\\p{L}]";
/// `gRuleSet_white_space_pattern`.
const G_RULE_SET_WHITE_SPACE_PATTERN: &str = "[\\p{Pattern_White_Space}]";
/// `kAny`.
const K_ANY: &str = "any";

/// `chNEL`: NEL newline variant.
const CH_NEL: i32 = 0x85;
/// `chLS`: Unicode Line Separator.
const CH_LS: i32 = 0x2028;

/// One character of the rule source with its escape status.
///
/// Equivalent to the package-private static nested class
/// `RBBIRuleScanner.RBBIRuleChar`.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct RBBIRuleChar {
    /// `fChar`.
    pub(crate) f_char: i32,
    /// `fEscaped`.
    pub(crate) f_escaped: bool,
}

/// The rule parser: scans the rule source and builds the parse trees.
///
/// Equivalent to the package-private class `com.ibm.icu.text.RBBIRuleScanner`.
/// Its `fRB` back reference is the `rb` parameter of the methods that use
/// it.
#[derive(Debug)]
pub(crate) struct RBBIRuleScanner {
    /// `fScanIndex`: index of current character being processed in the
    /// rule input string.
    f_scan_index: i32,
    /// `fNextIndex`: index of the next character, which is the first
    /// character not yet scanned.
    f_next_index: i32,
    /// `fQuoteMode`: scan is in a 'quoted region'.
    f_quote_mode: bool,
    /// `fLineNum`: line number in input file.
    f_line_num: i32,
    /// `fCharNum`: char position within the line.
    f_char_num: i32,
    /// `fLastChar`: previous char, needed to count CR-LF as a single line,
    /// not two.
    f_last_char: i32,
    /// `fC`: current char for parse state machine processing.
    f_c: RBBIRuleChar,
    /// `fStack`: state stack, holds state pushes.
    f_stack: [i16; K_STACK_SIZE],
    /// `fStackPtr`.
    f_stack_ptr: i32,
    /// `fNodeStack`: node stack, holds nodes created during parse of a
    /// rule.
    f_node_stack: Vec<Option<NodeId>>,
    /// `fNodeStackPtr`.
    f_node_stack_ptr: i32,
    /// `fReverseRule`: true if the rule currently being parsed began with
    /// '!' (a reverse rule).
    f_reverse_rule: bool,
    /// `fLookAheadRule`: true if the rule includes a '/' somewhere within
    /// it.
    f_look_ahead_rule: bool,
    /// `fNoChainInRule`: true if the current rule starts with a '^'.
    f_no_chain_in_rule: bool,
    /// `fSymbolTable`: symbol table, holds definitions of $variables
    /// occurring in the rules.
    pub(crate) f_symbol_table: RBBISymbolTable,
    /// `fSetTable`: the UnicodeSets of the rules, keyed by their source
    /// text, so that the same set used in several rules is shared.
    f_set_table: HashMap<Vec<u16>, NodeId>,
    /// `fRuleSets`: predefined UnicodeSets that are always available for
    /// use as char class matchers.
    f_rule_sets: Vec<Option<UnicodeSet>>,
    /// `fRuleNum`: currently parsing rule number, starting at 1.
    f_rule_num: i32,
    /// `fOptionStart`: string index of the start of an option (the
    /// `!!option` keyword).
    f_option_start: i32,
}

impl RBBIRuleScanner {
    /// `RBBIRuleScanner(RBBIRuleBuilder rb)`: the scanner and its
    /// predefined character classes.
    ///
    /// # Errors
    ///
    /// The exceptions of the `UnicodeSet` patterns.
    pub(crate) fn new() -> Result<Self> {
        // Set up the constant Unicode Sets.
        //     Note: These could be made static and shared among
        //           all instances of RBBIRuleScanners.
        let mut f_rule_sets: Vec<Option<UnicodeSet>> = (0..10).map(|_| None).collect();
        f_rule_sets[(T::kRuleSet_rule_char - 128) as usize] = Some(UnicodeSet::from_pattern(
            &units(G_RULE_SET_RULE_CHAR_PATTERN),
        )?);
        f_rule_sets[(T::kRuleSet_white_space - 128) as usize] = Some(UnicodeSet::from_pattern(
            &units(G_RULE_SET_WHITE_SPACE_PATTERN),
        )?);
        f_rule_sets[(T::kRuleSet_name_char - 128) as usize] = Some(UnicodeSet::from_pattern(
            &units(G_RULE_SET_NAME_CHAR_PATTERN),
        )?);
        f_rule_sets[(T::kRuleSet_name_start_char - 128) as usize] = Some(UnicodeSet::from_pattern(
            &units(G_RULE_SET_NAME_START_CHAR_PATTERN),
        )?);
        f_rule_sets[(T::kRuleSet_digit_char - 128) as usize] = Some(UnicodeSet::from_pattern(
            &units(G_RULE_SET_DIGIT_CHAR_PATTERN),
        )?);

        Ok(Self {
            f_scan_index: 0,
            f_next_index: 0,
            f_quote_mode: false,
            f_line_num: 1,
            f_char_num: 0,
            f_last_char: 0,
            f_c: RBBIRuleChar::default(),
            f_stack: [0; K_STACK_SIZE],
            f_stack_ptr: 0,
            f_node_stack: vec![None; K_STACK_SIZE],
            f_node_stack_ptr: 0,
            f_reverse_rule: false,
            f_look_ahead_rule: false,
            f_no_chain_in_rule: false,
            f_symbol_table: RBBISymbolTable::new(),
            f_set_table: HashMap::new(),
            f_rule_sets,
            f_rule_num: 0,
            f_option_start: 0,
        })
    }

    /// `fNodeStack[i]`, dereferenced.
    fn ns(&self, i: i32) -> NodeId {
        child(self.f_node_stack[i as usize])
    }

    /// `fRB.fRules.substring(begin, end)`.
    fn substring(rb: &RBBIRuleBuilder, begin: i32, end: i32) -> Vec<u16> {
        if begin < 0 || end > rb.f_rules.len() as i32 || begin > end {
            crate::jdk::string_index_out_of_bounds(if begin < 0 { begin } else { end - begin });
        }
        rb.f_rules[begin as usize..end as usize].to_vec()
    }

    /// `doParseActions(int action)`: performs a parse-table action; `false`
    /// stops the parse.
    ///
    /// # Errors
    ///
    /// The scanner's `IllegalArgumentException` for a rule error.
    fn do_parse_actions(&mut self, rb: &mut RBBIRuleBuilder, action: i32) -> Result<bool> {
        let n;

        let mut return_val = true;

        let action = action as i16;
        match action {
            T::doExprStart => {
                self.push_new_node(rb, OP_START)?;
                self.f_rule_num += 1;
            }

            T::doNoChain => {
                // Scanned a '^' while on the rule start state.
                self.f_no_chain_in_rule = true;
            }

            T::doExprOrOperator => {
                self.fix_op_stack(rb, PREC_OP_CAT)?;
                let operand_node = self.ns(self.f_node_stack_ptr);
                self.f_node_stack_ptr -= 1;
                let or_node = self.push_new_node(rb, OP_OR)?;
                rb.nodes[or_node].f_left_child = Some(operand_node);
                rb.nodes[operand_node].f_parent = Some(or_node);
            }

            T::doExprCatOperator => {
                // concatenation operator.
                // For the implicit concatenation of adjacent terms in an expression
                // that are
                //   not separated by any other operator. Action is invoked between the
                //   actions for the two terms.
                self.fix_op_stack(rb, PREC_OP_CAT)?;
                let operand_node = self.ns(self.f_node_stack_ptr);
                self.f_node_stack_ptr -= 1;
                let cat_node = self.push_new_node(rb, OP_CAT)?;
                rb.nodes[cat_node].f_left_child = Some(operand_node);
                rb.nodes[operand_node].f_parent = Some(cat_node);
            }

            T::doLParen => {
                // Open Paren.
                //   The openParen node is a dummy operation type with a low
                // precedence,
                //     which has the affect of ensuring that any real binary op that
                //     follows within the parens binds more tightly to the operands than
                //     stuff outside of the parens.
                self.push_new_node(rb, OP_L_PAREN)?;
            }

            T::doExprRParen => {
                self.fix_op_stack(rb, PREC_L_PAREN)?;
            }

            T::doNOP => {}

            T::doStartAssign => {
                // We've just scanned "$variable = "
                // The top of the node stack has the $variable ref node.

                // Save the start position of the RHS text in the StartExpression
                // node
                //   that precedes the $variableReference node on the stack.
                //   This will eventually be used when saving the full $variable
                // replacement
                //   text as a string.
                n = self.ns(self.f_node_stack_ptr - 1);
                rb.nodes[n].f_first_pos = self.f_next_index; // move past the '='

                // Push a new start-of-expression node; needed to keep parse of the
                //   RHS expression happy.
                self.push_new_node(rb, OP_START)?;
            }

            T::doEndAssign => {
                // We have reached the end of an assignment statement.
                //   Current scan char is the ';' that terminates the assignment.

                // Terminate expression, leaves expression parse tree rooted in TOS
                // node.
                self.fix_op_stack(rb, PREC_START)?;

                let start_expr_node = self.ns(self.f_node_stack_ptr - 2);
                let var_ref_node = self.ns(self.f_node_stack_ptr - 1);
                let rhs_expr_node = self.ns(self.f_node_stack_ptr);

                // Save original text of right side of assignment, excluding the
                // terminating ';'
                //  in the root of the node for the right-hand-side expression.
                rb.nodes[rhs_expr_node].f_first_pos = rb.nodes[start_expr_node].f_first_pos;
                rb.nodes[rhs_expr_node].f_last_pos = self.f_scan_index;
                // fRB.fRules.extractBetween(RHSExprNode.fFirstPos,
                // RHSExprNode.fLastPos, RHSExprNode.fText);
                let text = Self::substring(
                    rb,
                    rb.nodes[rhs_expr_node].f_first_pos,
                    rb.nodes[rhs_expr_node].f_last_pos,
                );
                rb.nodes[rhs_expr_node].f_text = Some(text);

                // Expression parse tree becomes l. child of the $variable reference
                // node.
                rb.nodes[var_ref_node].f_left_child = Some(rhs_expr_node);
                rb.nodes[rhs_expr_node].f_parent = Some(var_ref_node);

                // Make a symbol table entry for the $variableRef node.
                let key = rb.nodes[var_ref_node].f_text.clone().unwrap_or_default();
                if !self.f_symbol_table.add_entry(&key, var_ref_node) {
                    self.error(RBBIRuleBuilder::U_BRK_VARIABLE_REDFINITION)?;
                }

                // Clean up the stack.
                self.f_node_stack_ptr -= 3;
            }

            T::doEndOfRule => {
                self.fix_op_stack(rb, PREC_START)?; // Terminate expression, leaves expression

                if rb
                    .f_debug_env
                    .as_deref()
                    .is_some_and(|e| e.contains("rtree"))
                {
                    self.print_node_stack(rb, "end of rule");
                }
                Assert::assrt(self.f_node_stack_ptr == 1)?;
                let mut this_rule = self.ns(self.f_node_stack_ptr);

                // If this rule includes a look-ahead '/', add a endMark node to the
                //   expression tree.
                if self.f_look_ahead_rule {
                    let end_node = self.push_new_node(rb, END_MARK)?;
                    let cat_node = self.push_new_node(rb, OP_CAT)?;
                    self.f_node_stack_ptr -= 2;
                    rb.nodes[cat_node].f_left_child = Some(this_rule);
                    rb.nodes[cat_node].f_right_child = Some(end_node);
                    self.f_node_stack[self.f_node_stack_ptr as usize] = Some(cat_node);
                    rb.nodes[end_node].f_val = self.f_rule_num;
                    rb.nodes[end_node].f_look_ahead_end = true;
                    this_rule = cat_node;

                    // TODO: Disable chaining out of look-ahead (hard break) rules.
                    //   The break on rule match is forced, so there is no point in building up
                    //   the state table to chain into another rule for a longer match.
                }

                // Mark this node as being the root of a rule.
                rb.nodes[this_rule].f_rule_root = true;

                // Flag if chaining into this rule is wanted.
                //
                if rb.f_chain_rules && // If rule chaining is enabled globally via !!chain
                        !self.f_no_chain_in_rule
                {
                    //     and no '^' chain-in inhibit was on this rule
                    rb.nodes[this_rule].f_chain_in = true;
                }

                // All rule expressions are ORed together.
                // The ';' that terminates an expression really just functions as a
                // '|' with
                //   a low operator precedence.
                //
                // Each of the four sets of rules are collected separately.
                //  (forward, reverse, safe_forward, safe_reverse)
                //  OR this rule into the appropriate group of them.
                //

                let dest_rules = if self.f_reverse_rule {
                    RBBIRuleBuilder::F_SAFE_REV_TREE
                } else {
                    rb.f_default_tree
                };

                if let Some(prev_rules) = rb.f_tree_roots[dest_rules] {
                    // This is not the first rule encountered.
                    // OR previous stuff (from *destRules)
                    // with the current rule expression (on the Node Stack)
                    //  with the resulting OR expression going to *destRules
                    //
                    let this_rule = self.ns(self.f_node_stack_ptr);
                    let or_node = self.push_new_node(rb, OP_OR)?;
                    rb.nodes[or_node].f_left_child = Some(prev_rules);
                    rb.nodes[prev_rules].f_parent = Some(or_node);
                    rb.nodes[or_node].f_right_child = Some(this_rule);
                    rb.nodes[this_rule].f_parent = Some(or_node);
                    rb.f_tree_roots[dest_rules] = Some(or_node);
                } else {
                    // This is the first rule encountered (for this direction).
                    // Just move its parse tree from the stack to *destRules.
                    rb.f_tree_roots[dest_rules] = self.f_node_stack[self.f_node_stack_ptr as usize];
                }
                self.f_reverse_rule = false; // in preparation for the next rule.
                self.f_look_ahead_rule = false;
                self.f_no_chain_in_rule = false;
                self.f_node_stack_ptr = 0;
            }

            T::doRuleError => {
                self.error(RBBIRuleBuilder::U_BRK_RULE_SYNTAX)?;
                return_val = false;
            }

            T::doVariableNameExpectedErr => {
                self.error(RBBIRuleBuilder::U_BRK_RULE_SYNTAX)?;
            }

            //
            //  Unary operands + ? *
            //    These all appear after the operand to which they apply.
            //    When we hit one, the operand (may be a whole sub expression)
            //    will be on the top of the stack.
            //    Unary Operator becomes TOS, with the old TOS as its one child.
            T::doUnaryOpPlus => {
                let operand_node = self.ns(self.f_node_stack_ptr);
                self.f_node_stack_ptr -= 1;
                let plus_node = self.push_new_node(rb, OP_PLUS)?;
                rb.nodes[plus_node].f_left_child = Some(operand_node);
                rb.nodes[operand_node].f_parent = Some(plus_node);
            }

            T::doUnaryOpQuestion => {
                let operand_node = self.ns(self.f_node_stack_ptr);
                self.f_node_stack_ptr -= 1;
                let q_node = self.push_new_node(rb, OP_QUESTION)?;
                rb.nodes[q_node].f_left_child = Some(operand_node);
                rb.nodes[operand_node].f_parent = Some(q_node);
            }

            T::doUnaryOpStar => {
                let operand_node = self.ns(self.f_node_stack_ptr);
                self.f_node_stack_ptr -= 1;
                let star_node = self.push_new_node(rb, OP_STAR)?;
                rb.nodes[star_node].f_left_child = Some(operand_node);
                rb.nodes[operand_node].f_parent = Some(star_node);
            }

            T::doRuleChar => {
                // A "Rule Character" is any single character that is a literal part
                // of the regular expression. Like a, b and c in the expression "(abc*)
                // | [:L:]"
                // These are pretty uncommon in break rules; the terms are more commonly
                //  sets. To keep things uniform, treat these characters like as
                // sets that just happen to contain only one character.
                n = self.push_new_node(rb, SET_REF)?;
                let s = vec![self.f_c.f_char as u16];
                self.find_set_for(rb, &s, n, None)?;
                rb.nodes[n].f_first_pos = self.f_scan_index;
                rb.nodes[n].f_last_pos = self.f_next_index;
                let text = Self::substring(rb, rb.nodes[n].f_first_pos, rb.nodes[n].f_last_pos);
                rb.nodes[n].f_text = Some(text);
            }

            T::doDotAny => {
                // scanned a ".", meaning match any single character.
                n = self.push_new_node(rb, SET_REF)?;
                self.find_set_for(rb, &units(K_ANY), n, None)?;
                rb.nodes[n].f_first_pos = self.f_scan_index;
                rb.nodes[n].f_last_pos = self.f_next_index;
                let text = Self::substring(rb, rb.nodes[n].f_first_pos, rb.nodes[n].f_last_pos);
                rb.nodes[n].f_text = Some(text);
            }

            T::doSlash => {
                // Scanned a '/', which identifies a look-ahead break position in a
                // rule.
                n = self.push_new_node(rb, LOOK_AHEAD)?;
                rb.nodes[n].f_val = self.f_rule_num;
                rb.nodes[n].f_first_pos = self.f_scan_index;
                rb.nodes[n].f_last_pos = self.f_next_index;
                let text = Self::substring(rb, rb.nodes[n].f_first_pos, rb.nodes[n].f_last_pos);
                rb.nodes[n].f_text = Some(text);
                self.f_look_ahead_rule = true;
            }

            T::doStartTagValue => {
                // Scanned a '{', the opening delimiter for a tag value within a
                // rule.
                n = self.push_new_node(rb, TAG)?;
                rb.nodes[n].f_val = 0;
                rb.nodes[n].f_first_pos = self.f_scan_index;
                rb.nodes[n].f_last_pos = self.f_next_index;
            }

            T::doTagDigit => {
                // Just scanned a decimal digit that's part of a tag value
                n = self.ns(self.f_node_stack_ptr);
                let v = UCharacter::digit_radix(self.f_c.f_char as u16 as i32, 10);
                let update = (rb.nodes[n].f_val as i64) * 10 + v as i64;
                if update > i32::MAX as i64 {
                    self.error(RBBIRuleBuilder::U_BRK_RULE_SYNTAX)?;
                }
                rb.nodes[n].f_val = update as i32;
            }

            T::doTagValue => {
                n = self.ns(self.f_node_stack_ptr);
                rb.nodes[n].f_last_pos = self.f_next_index;
                let text = Self::substring(rb, rb.nodes[n].f_first_pos, rb.nodes[n].f_last_pos);
                rb.nodes[n].f_text = Some(text);
            }

            T::doTagExpectedError => {
                self.error(RBBIRuleBuilder::U_BRK_MALFORMED_RULE_TAG)?;
                return_val = false;
            }

            T::doOptionStart => {
                // Scanning a !!option. At the start of string.
                self.f_option_start = self.f_scan_index;
            }

            T::doOptionEnd => {
                let opt = Self::substring(rb, self.f_option_start, self.f_scan_index);
                if opt == units("chain") {
                    rb.f_chain_rules = true;
                } else if opt == units("forward") {
                    rb.f_default_tree = RBBIRuleBuilder::F_FORWARD_TREE;
                } else if opt == units("reverse") {
                    rb.f_default_tree = RBBIRuleBuilder::F_REVERSE_TREE;
                } else if opt == units("safe_forward") {
                    rb.f_default_tree = RBBIRuleBuilder::F_SAFE_FWD_TREE;
                } else if opt == units("safe_reverse") {
                    rb.f_default_tree = RBBIRuleBuilder::F_SAFE_REV_TREE;
                } else if opt == units("lookAheadHardBreak") {
                    rb.f_look_ahead_hard_break = true;
                } else if opt == units("quoted_literals_only") {
                    self.rule_set_mut(T::kRuleSet_rule_char).clear()?;
                } else if opt == units("unquoted_literals") {
                    self.rule_set_mut(T::kRuleSet_rule_char)
                        .apply_pattern(&units(G_RULE_SET_RULE_CHAR_PATTERN))?;
                } else {
                    self.error(RBBIRuleBuilder::U_BRK_UNRECOGNIZED_OPTION)?;
                }
            }

            T::doReverseDir => {
                self.f_reverse_rule = true;
            }

            T::doStartVariableName => {
                n = self.push_new_node(rb, VAR_REF)?;
                rb.nodes[n].f_first_pos = self.f_scan_index;
            }

            T::doEndVariableName => {
                let top = self.f_node_stack[self.f_node_stack_ptr as usize];
                match top {
                    Some(n) if rb.nodes[n].f_type == VAR_REF => {
                        rb.nodes[n].f_last_pos = self.f_scan_index;
                        let text = Self::substring(
                            rb,
                            rb.nodes[n].f_first_pos + 1,
                            rb.nodes[n].f_last_pos,
                        );
                        // Look the newly scanned name up in the symbol table
                        //   If there's an entry, set the l. child of the var ref to the
                        // replacement expression.
                        //   (We also pass through here when scanning assignments, but no harm
                        // is done, other
                        //    than a slight wasted effort that seems hard to avoid. Lookup will
                        // be null)
                        rb.nodes[n].f_left_child = self.f_symbol_table.lookup_node(&text);
                        rb.nodes[n].f_text = Some(text);
                    }
                    _ => {
                        self.error(RBBIRuleBuilder::U_BRK_INTERNAL_ERROR)?;
                    }
                }
            }

            T::doCheckVarDef => {
                n = self.ns(self.f_node_stack_ptr);
                if rb.nodes[n].f_left_child.is_none() {
                    self.error(RBBIRuleBuilder::U_BRK_UNDEFINED_VARIABLE)?;
                    return_val = false;
                }
            }

            T::doExprFinished => {}

            T::doRuleErrorAssignExpr => {
                self.error(RBBIRuleBuilder::U_BRK_ASSIGN_ERROR)?;
                return_val = false;
            }

            T::doExit => {
                return_val = false;
            }

            T::doScanUnicodeSet => {
                self.scan_set(rb)?;
            }

            _ => {
                self.error(RBBIRuleBuilder::U_BRK_INTERNAL_ERROR)?;
                return_val = false;
            }
        }
        Ok(return_val)
    }

    /// `fRuleSets[charClass - 128]`, dereferenced.
    fn rule_set_mut(&mut self, char_class: i16) -> &mut UnicodeSet {
        self.f_rule_sets[(char_class - 128) as usize]
            .as_mut()
            .unwrap_or_else(|| panic!("java.lang.NullPointerException: fRuleSets"))
    }

    /// The exception `error(int e)` throws: `IllegalArgumentException("Error
    /// " + e + " at line " + fLineNum + " column " + fCharNum)`.
    fn error_value(&self, e: i32) -> LuceneError {
        LuceneError::illegal_argument(format!(
            "Error {} at line {} column {}",
            e, self.f_line_num, self.f_char_num
        ))
    }

    /// `error(int e)`: report an error; always throws.
    ///
    /// # Errors
    ///
    /// Always, with [`error_value`](Self::error_value).
    fn error(&self, e: i32) -> Result<()> {
        Err(self.error_value(e))
    }

    /// `fixOpStack(int p)`: when an operator is scanned, pops the operators
    /// of higher precedence off the node stack, making them operands.
    ///
    /// # Errors
    ///
    /// The scanner's errors for a bad operator node or mismatched parens.
    fn fix_op_stack(&mut self, rb: &mut RBBIRuleBuilder, p: i32) -> Result<()> {
        let mut n;
        // printNodeStack("entering fixOpStack()");
        loop {
            n = self.ns(self.f_node_stack_ptr - 1); // an operator node
            if rb.nodes[n].f_precedence == 0 {
                print!("RBBIRuleScanner.fixOpStack, bad operator node");
                self.error(RBBIRuleBuilder::U_BRK_INTERNAL_ERROR)?;
                return Ok(());
            }

            if rb.nodes[n].f_precedence < p || rb.nodes[n].f_precedence <= PREC_L_PAREN {
                // The most recent operand goes with the current operator,
                //   not with the previously stacked one.
                break;
            }
            // Stack operator is a binary op ( '|' or concatenation)
            //   TOS operand becomes right child of this operator.
            //   Resulting subexpression becomes the TOS operand.
            let tos = self.ns(self.f_node_stack_ptr);
            rb.nodes[n].f_right_child = Some(tos);
            rb.nodes[tos].f_parent = Some(n);
            self.f_node_stack_ptr -= 1;
            // printNodeStack("looping in fixOpStack()   ");
        }

        if p <= PREC_L_PAREN {
            // Scan is at a right paren or end of expression.
            //  The scanned item must match the stack, or else there was an
            // error.
            //  Discard the left paren (or start expr) node from the stack,
            //  leaving the completed (sub)expression as TOS.
            if rb.nodes[n].f_precedence != p {
                // Right paren encountered matched start of expression node, or
                // end of expression matched with a left paren node.
                self.error(RBBIRuleBuilder::U_BRK_MISMATCHED_PAREN)?;
            }
            self.f_node_stack[(self.f_node_stack_ptr - 1) as usize] =
                self.f_node_stack[self.f_node_stack_ptr as usize];
            self.f_node_stack_ptr -= 1;
            // Delete the now-discarded LParen or Start node.
            // delete n;
        }
        // printNodeStack("leaving fixOpStack()");
        Ok(())
    }

    /// `findSetFor(String s, RBBINode node, UnicodeSet setToAdopt)`: hooks
    /// `node` to the `uset` node of the set named by its source text `s`,
    /// creating it (from `set_to_adopt`, `.` for `"any"`, or the single
    /// character) the first time.
    ///
    /// # Errors
    ///
    /// `IllegalStateException` (`Assert.assrt`) for an internal
    /// inconsistency.
    fn find_set_for(
        &mut self,
        rb: &mut RBBIRuleBuilder,
        s: &[u16],
        node: NodeId,
        set_to_adopt: Option<UnicodeSet>,
    ) -> Result<()> {
        // First check whether we've already cached a set for this string.
        // If so, just use the cached set in the new node.
        //   delete any set provided by the caller, since we own it.
        if let Some(&el) = self.f_set_table.get(s) {
            rb.nodes[node].f_left_child = Some(el);
            Assert::assrt(rb.nodes[el].f_type == USET)?;
            return Ok(());
        }

        // Haven't seen this set before.
        // If the caller didn't provide us with a prebuilt set,
        //   create a new UnicodeSet now.
        let set_to_adopt = match set_to_adopt {
            Some(set) => set,
            None => {
                if s == units(K_ANY).as_slice() {
                    UnicodeSet::new_range(0x000000, 0x10ffff)?
                } else {
                    let c = UTF16::char_at(s, 0);
                    UnicodeSet::new_range(c, c)?
                }
            }
        };

        // Make a new uset node to refer to this UnicodeSet
        // This new uset node becomes the child of the caller's setReference
        // node.
        let uset_node = rb.nodes.new_node(USET)?;
        rb.nodes[uset_node].f_input_set = Some(Arc::new(set_to_adopt));
        rb.nodes[uset_node].f_parent = Some(node);
        rb.nodes[node].f_left_child = Some(uset_node);
        rb.nodes[uset_node].f_text = Some(s.to_vec());

        // Add the new uset node to the list of all uset nodes.
        rb.f_u_set_nodes.push(uset_node);

        // Add the new set to the set hash table.
        self.f_set_table.insert(s.to_vec(), uset_node);
        Ok(())
    }

    /// `stripRules(String rules)`: the rules without their white space
    /// (Pattern_White_Space); comments were blanked by the scan.
    pub(crate) fn strip_rules(rules: &[u16]) -> Vec<u16> {
        let mut stripped_rules = Vec::new();
        let rules_length = rules.len() as i32;

        let mut idx = 0;
        while idx < rules_length {
            let cp = crate::jdk::code_point_at(rules, idx);
            let white_space = UCharacter::has_binary_property(cp, UProperty::PATTERN_WHITE_SPACE);
            if !white_space {
                crate::jdk::append_code_point(&mut stripped_rules, cp)
                    .unwrap_or_else(|e| panic!("{e}"));
            }
            idx = crate::jdk::offset_by_code_points(rules, idx, 1);
        }
        stripped_rules
    }

    /// `nextCharLL()`: the next code point of the rules, `-1` at the end;
    /// counts lines and columns.
    ///
    /// # Errors
    ///
    /// The scanner's errors for an unpaired surrogate or a new line inside
    /// a quoted string.
    fn next_char_ll(&mut self, rb: &mut RBBIRuleBuilder) -> Result<i32> {
        if self.f_next_index >= rb.f_rules.len() as i32 {
            return Ok(-1);
        }
        let ch = UTF16::char_at(&rb.f_rules, self.f_next_index);
        if ch < 0x10000 && (0xD800..=0xDFFF).contains(&ch) {
            self.error(RBBIRuleBuilder::U_ILLEGAL_CHAR_FOUND)?;
        }
        self.f_next_index = UTF16::move_code_point_offset(&rb.f_rules, self.f_next_index, 1);

        if ch == '\r' as i32
            || ch == CH_NEL
            || ch == CH_LS
            || (ch == '\n' as i32 && self.f_last_char != '\r' as i32)
        {
            // Character is starting a new line. Bump up the line number, and
            // reset the column to 0.
            self.f_line_num += 1;
            self.f_char_num = 0;
            if self.f_quote_mode {
                self.error(RBBIRuleBuilder::U_BRK_NEW_LINE_IN_QUOTED_STRING)?;
                self.f_quote_mode = false;
            }
        } else {
            // Character is not starting a new line. Except in the case of a
            // LF following a CR, increment the column position.
            if ch != '\n' as i32 {
                self.f_char_num += 1;
            }
        }
        self.f_last_char = ch;
        Ok(ch)
    }

    /// `nextChar(RBBIRuleChar c)`: the next character of the rules into
    /// `fC`, handling quotes, comments and escapes.
    ///
    /// # Errors
    ///
    /// The scanner's errors.
    fn next_char(&mut self, rb: &mut RBBIRuleBuilder) -> Result<()> {
        // Unicode Character constants needed for the processing done by nextChar(),
        //   in hex because literals wont work on EBCDIC machines.

        self.f_scan_index = self.f_next_index;
        self.f_c.f_char = self.next_char_ll(rb)?;
        self.f_c.f_escaped = false;

        //
        //  check for '' sequence.
        //  These are recognized in all contexts, whether in quoted text or not.
        //
        if self.f_c.f_char == '\'' as i32 {
            if self.f_next_index < rb.f_rules.len() as i32
                && UTF16::char_at(&rb.f_rules, self.f_next_index) == '\'' as i32
            {
                self.f_c.f_char = self.next_char_ll(rb)?; // get nextChar officially so character counts
                self.f_c.f_escaped = true; //   stay correct.
            } else {
                // Single quote, by itself.
                //   Toggle quoting mode.
                //   Return either '('  or ')', because quotes cause a grouping of the quoted text.
                self.f_quote_mode = !self.f_quote_mode;
                if self.f_quote_mode {
                    self.f_c.f_char = '(' as i32;
                } else {
                    self.f_c.f_char = ')' as i32;
                }
                self.f_c.f_escaped = false;
                return Ok(());
            }
        }

        if self.f_c.f_char == -1 {
            return Ok(());
        }
        if self.f_quote_mode {
            self.f_c.f_escaped = true;
        } else {
            // We are not in a 'quoted region' of the source.
            //
            if self.f_c.f_char == '#' as i32 {
                // Start of a comment. Consume the rest of it.
                //  The new-line char that terminates the comment is always returned.
                //  It will be treated as white-space, and serves to break up anything
                //    that might otherwise incorrectly clump together with a comment in
                //    the middle (a variable name, for example.)
                let comment_start = self.f_scan_index;
                loop {
                    self.f_c.f_char = self.next_char_ll(rb)?;
                    if self.f_c.f_char == -1
                        || self.f_c.f_char == '\r' as i32
                        || self.f_c.f_char == '\n' as i32
                        || self.f_c.f_char == CH_NEL
                        || self.f_c.f_char == CH_LS
                    {
                        break;
                    }
                }
                for i in comment_start..self.f_next_index - 1 {
                    rb.f_stripped_rules[i as usize] = ' ' as u16;
                }
            }
            if self.f_c.f_char == -1 {
                return Ok(());
            }

            //
            //  Backslash escapes are not recognized in quoted text.
            //
            if self.f_c.f_char == '\\' as i32 {
                self.f_c.f_escaped = true;
                let cp_and_length = Utility::unescape_and_length_at(&rb.f_rules, self.f_next_index);
                if cp_and_length < 0 {
                    self.error(RBBIRuleBuilder::U_BRK_HEX_DIGITS_EXPECTED)?;
                }
                self.f_c.f_char = Utility::cp_from_code_point_and_length(cp_and_length);
                let length = Utility::length_from_code_point_and_length(cp_and_length);

                self.f_char_num += length;
                self.f_next_index += length;
            }
        }
        // putc(fC.fChar, stdout);
        Ok(())
    }

    /// `parse()`: parses the rule source into the builder's trees.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for a rule syntax error.
    pub(crate) fn parse(&mut self, rb: &mut RBBIRuleBuilder) -> Result<()> {
        let mut state: i32;
        let mut table_el;

        state = 1;
        self.next_char(rb)?;
        //
        // Main loop for the rule parsing state machine.
        //   Runs once per state transition.
        //   Each time through optionally performs, depending on the state table,
        //      - an advance to the next input char
        //      - an action to be performed.
        //      - pushing or popping a state to/from the local state return stack.
        //
        loop {
            // Quit if state == 0 (the state of the RBBI parse table
            if state == 0 {
                break;
            }

            // Find the state table element that matches the input char from the rule, or the
            //    class of the input character.  Start with the first table row for this
            //    state, then linearly scan forward until we find a row that matches the
            //    character.  The last row for each state always matches all characters, so
            //    the search will stop there, if not before.
            //
            table_el = &G_RULE_PARSE_STATE_TABLE[state as usize];
            if rb
                .f_debug_env
                .as_deref()
                .is_some_and(|e| e.contains("scan"))
            {
                println!(
                    "char, line, col = ('{}', {}, {}    state = {}",
                    String::from_utf16_lossy(&[self.f_c.f_char as u16]),
                    self.f_line_num,
                    self.f_char_num,
                    table_el.f_state_name.unwrap_or("null")
                );
            }

            let mut table_row = state as usize;
            loop {
                table_el = &G_RULE_PARSE_STATE_TABLE[table_row];
                if rb
                    .f_debug_env
                    .as_deref()
                    .is_some_and(|e| e.contains("scan"))
                {
                    print!(".");
                }
                if table_el.f_char_class < 127
                    && !self.f_c.f_escaped
                    && table_el.f_char_class as i32 == self.f_c.f_char
                {
                    // Table row specified an individual character, not a set, and
                    //   the input character is not escaped, and
                    //   the input character matched it.
                    break;
                }
                if table_el.f_char_class == 255 {
                    // Table row specified default, match anything character class.
                    break;
                }
                if table_el.f_char_class == 254 && self.f_c.f_escaped {
                    // Table row specified "escaped" and the char was escaped.
                    break;
                }
                if table_el.f_char_class == 253
                    && self.f_c.f_escaped
                    && (self.f_c.f_char == 0x50 || self.f_c.f_char == 0x70)
                {
                    // Table row specified "escaped P" and the char is either 'p' or 'P'.
                    break;
                }
                if table_el.f_char_class == 252 && self.f_c.f_char == -1 {
                    // Table row specified eof and we hit eof on the input.
                    break;
                }

                if table_el.f_char_class >= 128
                    && table_el.f_char_class < 240
                    && // Table specs a char class &&
                    !self.f_c.f_escaped
                    && //   char is not escaped &&
                    self.f_c.f_char != -1
                {
                    //   char is not EOF
                    let uniset = self.f_rule_sets[(table_el.f_char_class - 128) as usize]
                        .as_ref()
                        .unwrap_or_else(|| panic!("java.lang.NullPointerException: fRuleSets"));
                    if uniset.contains(self.f_c.f_char)? {
                        // Table row specified a character class, or set of characters,
                        //   and the current char matches it.
                        break;
                    }
                }
                table_row += 1;
            }

            if rb
                .f_debug_env
                .as_deref()
                .is_some_and(|e| e.contains("scan"))
            {
                println!();
            }
            //
            // We've found the row of the state table that matches the current input
            //   character from the rules string.
            // Perform any action specified  by this row in the state table.
            if !self.do_parse_actions(rb, table_el.f_action as i32)? {
                // Break out of the state machine loop if the
                //   the action signalled some kind of error, or
                //   the action was to exit, occurs on normal end-of-rules-input.
                break;
            }

            if table_el.f_push_state != 0 {
                self.f_stack_ptr += 1;
                if self.f_stack_ptr as usize >= K_STACK_SIZE {
                    println!("RBBIRuleScanner.parse() - state stack overflow.");
                    self.error(RBBIRuleBuilder::U_BRK_INTERNAL_ERROR)?;
                }
                self.f_stack[self.f_stack_ptr as usize] = table_el.f_push_state;
            }

            if table_el.f_next_char {
                self.next_char(rb)?;
            }

            // Get the next state from the table entry, or from the
            //   state stack if the next state was specified as "pop".
            if table_el.f_next_state != 255 {
                state = table_el.f_next_state as i32;
            } else {
                state = self.f_stack[self.f_stack_ptr as usize] as i32;
                self.f_stack_ptr -= 1;
                if self.f_stack_ptr < 0 {
                    println!("RBBIRuleScanner.parse() - state stack underflow.");
                    self.error(RBBIRuleBuilder::U_BRK_INTERNAL_ERROR)?;
                }
            }
        }

        // If there are no forward rules throw an error.
        //
        if rb.f_tree_roots[RBBIRuleBuilder::F_FORWARD_TREE].is_none() {
            self.error(RBBIRuleBuilder::U_BRK_RULE_SYNTAX)?;
        }

        //
        // Parsing of the input RBBI rules is complete.
        // We now have a parse tree for the rule expressions
        // and a list of all UnicodeSets that are referenced.
        //
        if rb
            .f_debug_env
            .as_deref()
            .is_some_and(|e| e.contains("symbols"))
        {
            self.f_symbol_table.rbbi_symtable_print(&rb.nodes);
        }
        if rb
            .f_debug_env
            .as_deref()
            .is_some_and(|e| e.contains("ptree"))
        {
            println!("Completed Forward Rules Parse Tree...");
            rb.nodes.print_tree(
                child(rb.f_tree_roots[RBBIRuleBuilder::F_FORWARD_TREE]),
                true,
            );
            println!("\nCompleted Reverse Rules Parse Tree...");
            rb.nodes.print_tree(
                child(rb.f_tree_roots[RBBIRuleBuilder::F_REVERSE_TREE]),
                true,
            );
            println!("\nCompleted Safe Point Forward Rules Parse Tree...");
            match rb.f_tree_roots[RBBIRuleBuilder::F_SAFE_FWD_TREE] {
                None => println!("  -- null -- "),
                Some(t) => rb.nodes.print_tree(t, true),
            }
            println!("\nCompleted Safe Point Reverse Rules Parse Tree...");
            match rb.f_tree_roots[RBBIRuleBuilder::F_SAFE_REV_TREE] {
                None => println!("  -- null -- "),
                Some(t) => rb.nodes.print_tree(t, true),
            }
        }
        Ok(())
    }

    /// `printNodeStack(String title)`: debug.
    fn print_node_stack(&self, rb: &RBBIRuleBuilder, title: &str) {
        println!("{title}.  Dumping node stack...\n");
        let mut i = self.f_node_stack_ptr;
        while i > 0 {
            rb.nodes.print_tree(self.ns(i), true);
            i -= 1;
        }
    }

    /// `pushNewNode(int nodeType)`: creates a node of `node_type` and
    /// pushes it onto the node stack.
    ///
    /// # Errors
    ///
    /// The scanner's `U_BRK_INTERNAL_ERROR` on stack overflow.
    fn push_new_node(&mut self, rb: &mut RBBIRuleBuilder, node_type: i32) -> Result<NodeId> {
        self.f_node_stack_ptr += 1;
        if self.f_node_stack_ptr as usize >= K_STACK_SIZE {
            println!("RBBIRuleScanner.pushNewNode - stack overflow.");
            self.error(RBBIRuleBuilder::U_BRK_INTERNAL_ERROR)?;
        }
        let n = rb.nodes.new_node(node_type)?;
        self.f_node_stack[self.f_node_stack_ptr as usize] = Some(n);
        Ok(n)
    }

    /// `scanSet()`: scans a `UnicodeSet` pattern at the scan index and
    /// pushes a `setRef` node for it.
    ///
    /// # Errors
    ///
    /// The scanner's `U_BRK_MALFORMED_SET` when the pattern does not parse
    /// (any exception of the `UnicodeSet` constructor), and
    /// `U_BRK_RULE_EMPTY_SET` for a set without code points.
    fn scan_set(&mut self, rb: &mut RBBIRuleBuilder) -> Result<()> {
        let mut pos = ParsePosition::new(self.f_scan_index);

        let start_pos = self.f_scan_index;
        let parsed = {
            let view = RBBISymbolTableView {
                table: &self.f_symbol_table,
                nodes: &rb.nodes,
                malformed_set_error: self.error_value(RBBIRuleBuilder::U_BRK_MALFORMED_SET),
            };
            UnicodeSet::from_pattern_pos_options(
                &rb.f_rules,
                &mut pos,
                Some(&view),
                UnicodeSet::IGNORE_SPACE,
            )
        };
        let pending = self.f_symbol_table.take_pending_error();
        let uset = match parsed {
            Ok(uset) if pending.is_none() => uset,
            _ => {
                // TODO:  catch only IllegalArgumentException.
                // Java's UnicodeSet constructor exceptions all become U_BRK_MALFORMED_SET.
                return self.error(RBBIRuleBuilder::U_BRK_MALFORMED_SET);
            }
        };

        // Verify that the set contains at least one code point.
        //
        let mut temp_set = UnicodeSet::from_set(&uset);
        // Use tempSet to handle the case that the UnicodeSet contains
        // only string element, such as [{ab}] and treat it as empty set.
        temp_set.remove_all_strings()?;
        if temp_set.is_empty() {
            // This set is empty.
            //  Make it an error, because it almost certainly is not what the user wanted.
            //  Also, avoids having to think about corner cases in the tree manipulation code
            //   that occurs later on.
            //  TODO:  this shouldn't be an error; it does happen.
            self.error(RBBIRuleBuilder::U_BRK_RULE_EMPTY_SET)?;
        }

        // Advance the RBBI parse position over the UnicodeSet pattern.
        //   Don't just set fScanIndex because the line/char positions maintained
        //   for error reporting would be thrown off.
        let i = pos.get_index();
        loop {
            if self.f_next_index >= i {
                break;
            }
            self.next_char_ll(rb)?;
        }

        let n = self.push_new_node(rb, SET_REF)?;
        rb.nodes[n].f_first_pos = start_pos;
        rb.nodes[n].f_last_pos = self.f_next_index;
        let text = Self::substring(rb, rb.nodes[n].f_first_pos, rb.nodes[n].f_last_pos);
        rb.nodes[n].f_text = Some(text.clone());
        //  findSetFor() serves several purposes here:
        //     - Adopts storage for the UnicodeSet, will be responsible for deleting.
        //     - Maintains collection of all sets in use, needed later for establishing
        //          character categories for run time engine.
        //     - Eliminates duplicate sets.
        //     - Creates the new uset node if necessary (if this isn't a duplicate.)
        self.find_set_for(rb, &text, n, Some(uset))
    }

    /// `numRules()`: the number of rules parsed.
    pub(crate) fn num_rules(&self) -> i32 {
        self.f_rule_num
    }
}
