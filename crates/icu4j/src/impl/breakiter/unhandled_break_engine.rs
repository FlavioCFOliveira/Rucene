// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.breakiter.UnhandledBreakEngine`.

use std::any::Any;
use std::sync::{Arc, Mutex};

use rucene_analysis_common::jdk::character_iterator::CharacterIterator;

use crate::lang::u_character::UCharacter;
use crate::lang::u_property::UProperty;
use crate::r#impl::breakiter::dictionary_break_engine::{uset_contains, DequeI};
use crate::r#impl::breakiter::language_break_engine::LanguageBreakEngine;
use crate::r#impl::character_iteration;
use crate::text::unicode_set::UnicodeSet;

/// The engine for the scripts no other engine handles: it skips over runs
/// of their characters without finding breaks.
///
/// Equivalent to the final class
/// `com.ibm.icu.impl.breakiter.UnhandledBreakEngine`. The `volatile
/// UnicodeSet fHandled`, replaced as a whole by `handleChar` (thread safety
/// is by copy-on-write), is a lock-held `Arc<UnicodeSet>`: a reader clones
/// the `Arc` and reads its own snapshot, as Java's readers read the field
/// once.
#[derive(Debug)]
pub struct UnhandledBreakEngine {
    /// `fHandled`: the set of characters handled by this engine.
    f_handled: Mutex<Arc<UnicodeSet>>,
}

impl Default for UnhandledBreakEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl UnhandledBreakEngine {
    /// `UnhandledBreakEngine()`.
    pub fn new() -> Self {
        Self {
            f_handled: Mutex::new(Arc::new(UnicodeSet::new())),
        }
    }

    /// The current `fHandled` (a volatile read).
    fn handled(&self) -> Arc<UnicodeSet> {
        self.f_handled
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// `handleChar(int c)`: adds the whole script of `c` to the handled
    /// set.
    ///
    /// Note that the same `UnhandledBreakEngine` instance is shared among
    /// all break iterators that encounter a character from an unhandled
    /// script; the replacement set is built aside and published at once.
    pub fn handle_char(&self, c: i32) {
        let original_set = self.handled();
        if !uset_contains(&original_set, c) {
            let script = UCharacter::get_int_property_value(c, UProperty::SCRIPT);
            let mut new_set = UnicodeSet::new();
            new_set
                .apply_int_property_value(UProperty::SCRIPT, script)
                .unwrap_or_else(|e| panic!("{e}"));
            new_set
                .add_all(&original_set)
                .unwrap_or_else(|e| panic!("{e}"));
            *self.f_handled.lock().unwrap_or_else(|e| e.into_inner()) = Arc::new(new_set);
        }
    }
}

impl LanguageBreakEngine for UnhandledBreakEngine {
    fn handles(&self, c: i32) -> bool {
        uset_contains(&self.handled(), c)
    }

    fn find_breaks(
        &self,
        text: &mut dyn CharacterIterator,
        _start_pos: i32,
        end_pos: i32,
        _found_breaks: &mut DequeI,
        _is_phrase_breaking: bool,
    ) -> i32 {
        let uniset = self.handled();
        let mut c = character_iteration::current32(text);
        while text.get_index() < end_pos && uset_contains(&uniset, c) {
            character_iteration::next32(text);
            c = character_iteration::current32(text);
        }
        0
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
