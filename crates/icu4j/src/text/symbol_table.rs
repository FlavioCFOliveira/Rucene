// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.text.SymbolTable`.

use std::sync::Arc;

use rucene::error::Result;

use crate::jdk::ParsePosition;
use crate::text::unicode_matcher::UnicodeMatcher;
use crate::text::unicode_set::XSymbolTable;

/// `SYMBOL_REF`: the character preceding a symbol reference name.
pub const SYMBOL_REF: u16 = b'$' as u16;

/// An interface that defines both lookup protocol and parsing of symbolic
/// names.
///
/// Equivalent to the interface `com.ibm.icu.text.SymbolTable`.
/// [`as_x_symbol_table`](Self::as_x_symbol_table) carries Java's
/// `symbols instanceof UnicodeSet.XSymbolTable` test and cast.
pub trait SymbolTable: Send + Sync {
    /// `lookup(String s)`: the value of a symbol, `None` when undefined.
    fn lookup(&self, s: &[u16]) -> Option<Vec<u16>>;

    /// `lookupMatcher(int ch)`: the matcher a stand-in character stands
    /// for, `None` when none.
    fn lookup_matcher(&self, ch: i32) -> Option<Arc<dyn UnicodeMatcher>>;

    /// `parseReference(String text, ParsePosition pos, int limit)`: the
    /// name of a symbol at `pos`, advancing `pos` past it; `None` when
    /// no valid name is there.
    ///
    /// # Errors
    ///
    /// The errors the implementation throws.
    fn parse_reference(&self, text: &[u16], pos: &mut ParsePosition, limit: i32) -> Result<Option<Vec<u16>>>;

    /// This table as an `XSymbolTable`, `None` when it is not one.
    fn as_x_symbol_table(&self) -> Option<&dyn XSymbolTable> {
        None
    }
}
