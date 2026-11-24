// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.text.RBBISymbolTable`: the variables of a break-rule
//! source.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use rucene::error::{LuceneError, Result};

use crate::jdk::ParsePosition;
use crate::lang::u_character::UCharacter;
use crate::text::rbbi_node::{child, text_string, NodeId, RBBINodes, SET_REF, VAR_REF};
use crate::text::symbol_table::SymbolTable;
use crate::text::unicode_matcher::UnicodeMatcher;
use crate::text::unicode_set::UnicodeSet;
use crate::text::utf16::UTF16;

/// One variable definition.
///
/// Equivalent to the package-private static nested class
/// `RBBISymbolTable.RBBISymbolTableEntry`.
#[derive(Debug, Clone)]
pub(crate) struct RBBISymbolTableEntry {
    /// `key`.
    pub(crate) key: Vec<u16>,
    /// `val`.
    pub(crate) val: NodeId,
}

/// The symbol table of a rule scanner: variable names to their `varRef`
/// nodes, and the `SymbolTable` that `UnicodeSet` patterns resolve
/// `$variables` through.
///
/// Equivalent to the package-private class `com.ibm.icu.text.RBBISymbolTable`.
///
/// **Divergence.** Java's table holds its scanner and, inside the
/// `SymbolTable.lookup` callback `UnicodeSet` makes, throws the scanner's
/// `IllegalArgumentException` for a variable that is not a set
/// (`RBBISymbolTable.java`, `lookup`). The crate's `SymbolTable.lookup`
/// cannot fail, so the table records that error and the scanner raises it
/// as soon as the `UnicodeSet` constructor returns; Java's scanner catches
/// it there and raises the same error code at the same line and column
/// (`RBBIRuleScanner.scanSet`), so the outcome is identical.
#[derive(Debug, Default)]
pub(crate) struct RBBISymbolTable {
    /// `fHashTable`.
    pub(crate) f_hash_table: HashMap<Vec<u16>, RBBISymbolTableEntry>,
    /// `ffffString`: `"￿"`.
    ffff_string: Vec<u16>,
    /// `fCachedSetLookup`.
    f_cached_set_lookup: Mutex<Option<Arc<UnicodeSet>>>,
    /// The scanner error `lookup` raised, pending until the `UnicodeSet`
    /// constructor returns.
    pending_error: Mutex<Option<LuceneError>>,
}

impl RBBISymbolTable {
    /// `RBBISymbolTable(RBBIRuleScanner rs)`.
    pub(crate) fn new() -> Self {
        Self {
            f_hash_table: HashMap::new(),
            ffff_string: vec![0xffff],
            f_cached_set_lookup: Mutex::new(None),
            pending_error: Mutex::new(None),
        }
    }

    /// The error `lookup` recorded, taken.
    pub(crate) fn take_pending_error(&self) -> Option<LuceneError> {
        self.pending_error
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
    }

    /// `lookupNode(String key)`: the `varRef` node of a variable, if
    /// defined.
    pub(crate) fn lookup_node(&self, key: &[u16]) -> Option<NodeId> {
        self.f_hash_table.get(key).map(|el| el.val)
    }

    /// `addEntry(String key, RBBINode val)`: defines a variable; `false`
    /// when it is already defined, where Java's table raises the scanner's
    /// `U_BRK_VARIABLE_REDFINITION` (the scanner raises it on `false`).
    pub(crate) fn add_entry(&mut self, key: &[u16], val: NodeId) -> bool {
        if self.f_hash_table.contains_key(key) {
            return false;
        }
        let e = RBBISymbolTableEntry {
            key: key.to_vec(),
            val,
        };
        self.f_hash_table.insert(e.key.clone(), e);
        true
    }

    /// `rbbiSymtablePrint()`: prints the variable definitions (debugging).
    pub(crate) fn rbbi_symtable_print(&self, nodes: &RBBINodes) {
        print!(
            "Variable Definitions\nName               Node Val     String Val\n----------------------------------------------------------------------\n"
        );

        let syms: Vec<&RBBISymbolTableEntry> = self.f_hash_table.values().collect();

        for s in &syms {
            print!("  {}  ", String::from_utf16_lossy(&s.key));
            print!("  RBBINode@{}  ", nodes[s.val].f_serial_num);
            print!(
                "{}",
                text_string(&nodes[child(nodes[s.val].f_left_child)].f_text)
            );
            println!();
        }

        println!("\nParsed Variable Definitions\n");
        for s in &syms {
            print!("{}", String::from_utf16_lossy(&s.key));
            nodes.print_tree(child(nodes[s.val].f_left_child), true);
            println!();
        }
    }
}

/// The table as the `SymbolTable` of one `UnicodeSet` parse: the table, the
/// node arena it refers to, and the scanner error `lookup` raises.
pub(crate) struct RBBISymbolTableView<'a> {
    /// The table.
    pub(crate) table: &'a RBBISymbolTable,
    /// The nodes the table refers to.
    pub(crate) nodes: &'a RBBINodes,
    /// `fRuleScanner.error(RBBIRuleBuilder.U_BRK_MALFORMED_SET)` at the
    /// scanner's current line and column.
    pub(crate) malformed_set_error: LuceneError,
}

impl SymbolTable for RBBISymbolTableView<'_> {
    /// `lookup(String s)`: the text of variable `s`; for a set, `"￿"`,
    /// the set being cached for the following `lookupMatcher(0xffff)`.
    fn lookup(&self, s: &[u16]) -> Option<Vec<u16>> {
        let el = self.table.f_hash_table.get(s)?;

        // Walk through any chain of variable assignments that ultimately resolve to a Set Ref.
        let mut var_ref_node = el.val;
        while self.nodes[child(self.nodes[var_ref_node].f_left_child)].f_type == VAR_REF {
            var_ref_node = child(self.nodes[var_ref_node].f_left_child);
        }

        let expr_node = child(self.nodes[var_ref_node].f_left_child); // Root node of expression for variable
        let ret_string;
        let mut cache = self
            .table
            .f_cached_set_lookup
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if self.nodes[expr_node].f_type == SET_REF {
            // The $variable refers to a single UnicodeSet
            //   return the ffffString, which will subsequently be interpreted as a
            //   stand-in character for the set by UnicodeSet.applyPattern(), which
            //   will call us back via lookupMatcher(), returning the actual set.
            let uset_node = child(self.nodes[expr_node].f_left_child);
            *cache = self.nodes[uset_node].f_input_set.clone();
            ret_string = self.table.ffff_string.clone();
        } else {
            // The variable is not a single set.  There are two possibilities.
            //   1. The variable is not a set at all, but a string.
            //   2. The variable is a complex expression.
            //   Java's scanner throws U_BRK_MALFORMED_SET here; the port records it.
            let mut pending = self
                .table
                .pending_error
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            if pending.is_none() {
                *pending = Some(self.malformed_set_error.duplicate());
            }
            ret_string = self.nodes[expr_node].f_text.clone().unwrap_or_default();
            *cache = None;
        }
        Some(ret_string)
    }

    /// `lookupMatcher(int ch)`: the set cached by the preceding `lookup`
    /// for `0xffff`.
    fn lookup_matcher(&self, ch: i32) -> Option<Arc<dyn UnicodeMatcher>> {
        let mut ret_val: Option<Arc<UnicodeSet>> = None;
        if ch == 0xffff {
            let mut cache = self
                .table
                .f_cached_set_lookup
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            ret_val = cache.take();
        }
        ret_val.map(|s| s as Arc<dyn UnicodeMatcher>)
    }

    /// `parseReference(String text, ParsePosition pos, int limit)`: the
    /// identifier at `pos` (empty when there is none), moving `pos` past it.
    fn parse_reference(
        &self,
        text: &[u16],
        pos: &mut ParsePosition,
        limit: i32,
    ) -> Result<Option<Vec<u16>>> {
        let start = pos.get_index();
        let mut i = start;
        let result: Vec<u16> = Vec::new();
        while i < limit {
            let c = UTF16::char_at(text, i);
            if (i == start && !UCharacter::is_unicode_identifier_start(c))
                || !UCharacter::is_unicode_identifier_part(c)
            {
                break;
            }
            i += UTF16::get_char_count(c);
        }
        if i == start {
            // No valid name chars
            return Ok(Some(result)); // Indicate failure with empty string
        }
        pos.set_index(i);
        Ok(Some(text[start as usize..i as usize].to_vec()))
    }
}
