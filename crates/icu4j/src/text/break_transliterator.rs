// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.text.BreakTransliterator`.

use std::sync::{Arc, Mutex, MutexGuard};

use rucene::error::{LuceneError, Result};
use rucene::util::java_collections::java_identity_hash_code_stand_in;
use rucene_analysis_common::jdk::character_iterator::{CharacterIterator, DONE};

use crate::jdk::units;
use crate::lang::u_character::UCharacter;
use crate::text::break_iterator::{self, BreakIterator, DONE as BI_DONE};
use crate::text::replaceable::Replaceable;
use crate::text::transliterator::{
    register_instance_in, transliterator_boilerplate, Position, Transliterator, TransliteratorBase,
};
use crate::text::transliterator_registry::TransliteratorRegistry;
use crate::text::unicode_filter::UnicodeFilter;
use crate::text::unicode_set::UnicodeSet;
use crate::text::utf16::UTF16;
use crate::util::u_locale::ULocale;

// java.lang.Character general category values
const UPPERCASE_LETTER: i32 = 1;
const LOWERCASE_LETTER: i32 = 2;
const TITLECASE_LETTER: i32 = 3;
const MODIFIER_LETTER: i32 = 4;
const OTHER_LETTER: i32 = 5;
const NON_SPACING_MARK: i32 = 6;
const ENCLOSING_MARK: i32 = 7;
const COMBINING_SPACING_MARK: i32 = 8;

/// `LETTER_OR_MARK_MASK`.
const LETTER_OR_MARK_MASK: i32 = (1 << UPPERCASE_LETTER)
    | (1 << LOWERCASE_LETTER)
    | (1 << TITLECASE_LETTER)
    | (1 << MODIFIER_LETTER)
    | (1 << OTHER_LETTER)
    | (1 << COMBINING_SPACING_MARK)
    | (1 << NON_SPACING_MARK)
    | (1 << ENCLOSING_MARK);

/// The mutable state of a `BreakTransliterator`: Java's fields `bi`,
/// `insertion`, `boundaries` and `boundaryCount`, which the `synchronized`
/// `handleTransliterate` mutates.
#[derive(Debug)]
struct BreakState {
    bi: Option<Box<dyn BreakIterator>>,
    insertion: Vec<u16>,
    boundaries: Vec<i32>,
    boundary_count: usize,
}

/// Inserts the specified characters at word breaks. To restrict it to
/// particular characters, use a filter. TODO: this is an internal class,
/// and only temporary. Remove it once we have \b notation in Transliterator.
///
/// Equivalent to the package-private final class
/// `com.ibm.icu.text.BreakTransliterator`.
///
/// **Divergence.** `getBreakIterator()` returns a clone of the iterator
/// (Java hands back the object it holds, `BreakTransliterator.java:52-56`),
/// as the crate's break-iterator service does: the iterator lives behind
/// the lock that stands for `synchronized`.
#[derive(Debug)]
pub struct BreakTransliterator {
    base: TransliteratorBase,
    state: Mutex<BreakState>,
}

impl BreakTransliterator {
    /// `BreakTransliterator(String ID, UnicodeFilter filter, BreakIterator
    /// bi, String insertion)`.
    ///
    /// # Errors
    ///
    /// The errors of `setFilter`.
    pub fn with_break_iterator(
        id: &[u16],
        filter: Option<&dyn UnicodeFilter>,
        bi: Option<Box<dyn BreakIterator>>,
        insertion: &[u16],
    ) -> Result<Self> {
        Ok(Self {
            base: TransliteratorBase::new(id.to_vec(), filter)?,
            state: Mutex::new(BreakState {
                bi,
                insertion: insertion.to_vec(),
                boundaries: vec![0; 50],
                boundary_count: 0,
            }),
        })
    }

    /// `BreakTransliterator(String ID, UnicodeFilter filter)`: inserts a
    /// space at word breaks.
    ///
    /// # Errors
    ///
    /// The errors of `setFilter`.
    pub fn new(id: &[u16], filter: Option<&dyn UnicodeFilter>) -> Result<Self> {
        Self::with_break_iterator(id, filter, None, &[b' ' as u16])
    }

    fn state(&self) -> MutexGuard<'_, BreakState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// `getInsertion()`.
    pub fn get_insertion(&self) -> Vec<u16> {
        self.state().insertion.clone()
    }

    /// `setInsertion(String insertion)`.
    #[allow(dead_code)] // Java member with no caller in the closure
    pub fn set_insertion(&self, insertion: &[u16]) {
        self.state().insertion = insertion.to_vec();
    }

    /// `getBreakIterator()`: the break iterator, created (a Thai word
    /// iterator) on first use; a clone (see the type's divergence note).
    ///
    /// # Errors
    ///
    /// The errors of `BreakIterator.getWordInstance`.
    #[allow(dead_code)] // Java member with no caller in the closure
    pub fn get_break_iterator(&self) -> Result<Box<dyn BreakIterator>> {
        let mut st = self.state();
        Self::ensure_break_iterator(&mut st)?;
        Ok(st.bi.as_ref().expect("created above").clone_box())
    }

    /// The lazy part of `getBreakIterator()`.
    fn ensure_break_iterator(st: &mut BreakState) -> Result<()> {
        // Defer this work until we really need it.  We do this to
        // simplify our initialization of this class.
        if st.bi.is_none() {
            st.bi = Some(break_iterator::get_word_instance_ulocale(&ULocale::new(
                "th_TH",
            )?)?);
        }
        Ok(())
    }

    /// `setBreakIterator(BreakIterator bi)`.
    #[allow(dead_code)] // Java member with no caller in the closure
    pub fn set_break_iterator(&self, bi: Option<Box<dyn BreakIterator>>) {
        self.state().bi = bi;
    }

    /// `register()`: registers standard variants with the system. Called by
    /// Transliterator during initialization (on the registry under
    /// construction).
    pub(crate) fn register(registry: &TransliteratorRegistry) -> Result<()> {
        // false means that it is invisible
        let trans: Arc<dyn Transliterator> =
            Arc::new(BreakTransliterator::new(&units("Any-BreakInternal"), None)?);
        register_instance_in(registry, trans, false)
    }
}

impl Transliterator for BreakTransliterator {
    transliterator_boilerplate!();

    /// `handleTransliterate(Replaceable text, Position pos, boolean
    /// incremental)`.
    fn handle_transliterate(
        &self,
        text: &dyn Replaceable,
        pos: &mut Position,
        incremental: bool,
    ) -> Result<()> {
        let mut guard = self.state(); // synchronized
        let st = &mut *guard;
        st.boundary_count = 0;
        let mut boundary;
        Self::ensure_break_iterator(st)?;
        let BreakState {
            bi,
            insertion,
            boundaries,
            boundary_count,
        } = st;
        let bi = bi.as_mut().expect("created above");
        bi.set_text(Some(Box::new(ReplaceableCharacterIterator::new(
            text, pos.start, pos.limit, pos.start,
        )?)));
        // TODO: fix clumsy workaround used below.
        /*
        char[] tempBuffer = new char[text.length()];
        text.getChars(0, text.length(), tempBuffer, 0);
        bi.setText(new StringCharacterIterator(new String(tempBuffer), pos.start, pos.limit, pos.start));
        */
        // end debugging

        // To make things much easier, we will stack the boundaries, and then insert at the end.
        // generally, we won't need too many, since we will be filtered.

        boundary = bi.first();
        while boundary != BI_DONE && boundary < pos.limit {
            if boundary == 0 {
                boundary = bi.next();
                continue;
            }
            // HACK: Check to see that preceding item was a letter

            let cp = UTF16::char_at_replaceable(text, boundary - 1);
            let type_ = UCharacter::get_type(cp);
            //System.out.println(Integer.toString(cp,16) + " (before): " + type);
            if ((1 << type_) & LETTER_OR_MARK_MASK) == 0 {
                boundary = bi.next();
                continue;
            }

            let cp = UTF16::char_at_replaceable(text, boundary);
            let type_ = UCharacter::get_type(cp);
            //System.out.println(Integer.toString(cp,16) + " (after): " + type);
            if ((1 << type_) & LETTER_OR_MARK_MASK) == 0 {
                boundary = bi.next();
                continue;
            }

            if *boundary_count >= boundaries.len() {
                // realloc if necessary
                let mut temp = vec![0; boundaries.len() * 2];
                temp[..boundaries.len()].copy_from_slice(boundaries);
                *boundaries = temp;
            }

            boundaries[*boundary_count] = boundary;
            *boundary_count += 1;
            //System.out.println(boundary);
            boundary = bi.next();
        }

        let mut delta = 0;
        let mut last_boundary = 0;

        if *boundary_count != 0 {
            // if we found something, adjust
            delta = *boundary_count as i32 * insertion.len() as i32;
            last_boundary = boundaries[*boundary_count - 1];

            // we do this from the end backwards, so that we don't have to keep updating.

            while *boundary_count > 0 {
                *boundary_count -= 1;
                boundary = boundaries[*boundary_count];
                text.replace(boundary, boundary, insertion);
            }
        }

        // Now fix up the return values
        pos.context_limit += delta;
        pos.limit += delta;
        pos.start = if incremental {
            last_boundary + delta
        } else {
            pos.limit
        };
        Ok(())
    }

    /// `addSourceTargetSet(UnicodeSet inputFilter, UnicodeSet sourceSet,
    /// UnicodeSet targetSet)`.
    fn add_source_target_set(
        &self,
        input_filter: &UnicodeSet,
        _source_set: &mut UnicodeSet,
        target_set: &mut UnicodeSet,
    ) -> Result<()> {
        let my_filter = self.get_filter_as_unicode_set(input_filter)?;
        // Doesn't actually modify the source characters, so leave them alone.
        // add the characters inserted
        if my_filter.size() != 0 {
            target_set.add_all_str(&self.get_insertion())?;
        }
        Ok(())
    }
}

/// Replaceable character iterator.
///
/// Equivalent to the nested class
/// `BreakTransliterator.ReplaceableCharacterIterator`.
///
/// **Divergence.** Java reads the `Replaceable` it is given
/// (`BreakTransliterator.java:278`, `:294`); the break iterator keeps its
/// character iterator past the call that creates it, so the iterator here
/// holds a snapshot of the text's units (the text does not change while
/// `handleTransliterate` iterates). `equals` compares the snapshot's
/// identity where Java compares the `Replaceable` (which does not override
/// `equals`), and `hashCode` uses the identity-hash stand-in for the text.
#[derive(Debug, Clone)]
pub struct ReplaceableCharacterIterator {
    text: Arc<[u16]>,
    begin: i32,
    end: i32,
    // invariant: begin <= pos <= end
    pos: i32,
}

/// The units of a `Replaceable`.
fn snapshot(text: &dyn Replaceable) -> Arc<[u16]> {
    let len = text.length();
    let mut units = vec![0u16; len as usize];
    text.get_chars(0, len, &mut units, 0);
    Arc::from(units)
}

impl ReplaceableCharacterIterator {
    /// `ReplaceableCharacterIterator(Replaceable text, int begin, int end,
    /// int pos)`: constructs an iterator over the given range of the given
    /// string, with the index set at the specified position.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for an invalid range or position (Java's
    /// `null` check is the non-optional parameter).
    pub fn new(text: &dyn Replaceable, begin: i32, end: i32, pos: i32) -> Result<Self> {
        if begin < 0 || begin > end || end > text.length() {
            return Err(LuceneError::illegal_argument("Invalid substring range"));
        }

        if pos < begin || pos > end {
            return Err(LuceneError::illegal_argument("Invalid position"));
        }

        Ok(Self {
            text: snapshot(text),
            begin,
            end,
            pos,
        })
    }

    /// `setText(Replaceable text)`: reset this iterator to point to a new
    /// string. This package-visible method is used by other java.text
    /// classes that want to avoid allocating new
    /// ReplaceableCharacterIterator objects every time their setText method
    /// is called.
    #[allow(dead_code)] // Java member with no caller in the closure
    pub fn set_text(&mut self, text: &dyn Replaceable) {
        self.text = snapshot(text);
        self.begin = 0;
        self.end = text.length();
        self.pos = 0;
    }

    /// `equals(Object obj)`: compares the equality of two
    /// ReplaceableCharacterIterator objects.
    #[allow(dead_code)] // Java member with no caller in the closure
    pub fn equals(&self, that: &ReplaceableCharacterIterator) -> bool {
        if std::ptr::eq(self, that) {
            return true;
        }

        if self.hash_code() != that.hash_code() {
            return false;
        }
        if !Arc::ptr_eq(&self.text, &that.text) {
            return false;
        }
        if self.pos != that.pos || self.begin != that.begin || self.end != that.end {
            return false;
        }
        true
    }

    /// `hashCode()`: computes a hashcode for this iterator.
    #[allow(dead_code)] // Java member with no caller in the closure
    pub fn hash_code(&self) -> i32 {
        java_identity_hash_code_stand_in("com.ibm.icu.text.Replaceable")
            ^ self.pos
            ^ self.begin
            ^ self.end
    }
}

impl CharacterIterator for ReplaceableCharacterIterator {
    /// `first()`: implements `CharacterIterator.first()` for `String`.
    fn first(&mut self) -> u16 {
        self.pos = self.begin;
        self.current()
    }

    /// `last()`: implements `CharacterIterator.last()` for `String`.
    fn last(&mut self) -> u16 {
        if self.end != self.begin {
            self.pos = self.end - 1;
        } else {
            self.pos = self.end;
        }
        self.current()
    }

    /// `current()`: implements `CharacterIterator.current()` for `String`.
    fn current(&self) -> u16 {
        if self.pos >= self.begin && self.pos < self.end {
            self.text[self.pos as usize]
        } else {
            DONE
        }
    }

    /// `next()`: implements `CharacterIterator.next()` for `String`.
    fn next(&mut self) -> u16 {
        if self.pos < self.end - 1 {
            self.pos += 1;
            self.text[self.pos as usize]
        } else {
            self.pos = self.end;
            DONE
        }
    }

    /// `previous()`: implements `CharacterIterator.previous()` for
    /// `String`.
    fn previous(&mut self) -> u16 {
        if self.pos > self.begin {
            self.pos -= 1;
            self.text[self.pos as usize]
        } else {
            DONE
        }
    }

    /// `setIndex(int p)`: implements `CharacterIterator.setIndex()` for
    /// `String`.
    fn set_index(&mut self, p: i32) -> Result<u16> {
        if p < self.begin || p > self.end {
            return Err(LuceneError::illegal_argument("Invalid index"));
        }
        self.pos = p;
        Ok(self.current())
    }

    /// `getBeginIndex()`.
    fn get_begin_index(&self) -> i32 {
        self.begin
    }

    /// `getEndIndex()`.
    fn get_end_index(&self) -> i32 {
        self.end
    }

    /// `getIndex()`.
    fn get_index(&self) -> i32 {
        self.pos
    }

    /// `clone()`: creates a copy of this iterator (sharing the text, as
    /// Java's shallow `super.clone()` does).
    fn clone_box(&self) -> Box<dyn CharacterIterator> {
        Box::new(self.clone())
    }
}
