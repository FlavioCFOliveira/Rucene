// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.TextTrieMap`.

use std::sync::Mutex;

use rucene::error::{LuceneError, Result};

use crate::jdk::{append_code_point, char_count, code_point_at, is_high_surrogate, to_code_point};
use crate::lang::u_character::UCharacter;
use crate::text::unicode_set::UnicodeSet;

/// A trie mapping texts to lists of values, with longest-prefix and
/// all-prefix matching, optionally case-insensitive (by simple case
/// folding).
///
/// Equivalent to `com.ibm.icu.impl.TextTrieMap<V>`. Java's `find` is
/// `synchronized`; the trie is behind a `Mutex`, which `put` takes too (Rust
/// forbids unsynchronized mutation of a shared trie).
///
/// **Divergence.** Java hands result handlers and callers an
/// `Iterator<V>` over a node's live value list; the port hands handlers the
/// node's values as a slice while the trie is locked, and returns copies
/// from [`get`](Self::get) (crate precedent: trie iterators return copies).
#[derive(Debug)]
pub struct TextTrieMap<V> {
    root: Mutex<Node<V>>,
    ignore_case: bool,
}

/// The longest-match length and partial-match flag of a lookup.
///
/// Equivalent to `TextTrieMap.Output`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Output {
    /// `matchLength`.
    pub match_length: i32,
    /// `partialMatch`: whether the input ended inside a key.
    pub partial_match: bool,
}

/// A receiver of the values of every key that is a prefix of the input.
///
/// Equivalent to the interface `TextTrieMap.ResultHandler<V>`.
pub trait ResultHandler<V> {
    /// `handlePrefixMatch(int matchLength, Iterator<V> values)`: `false`
    /// stops the search.
    fn handle_prefix_match(&mut self, match_length: i32, values: &[V]) -> bool;
}

/// `TextTrieMap.LongestMatchHandler`.
struct LongestMatchHandler<V> {
    matches: Option<Vec<V>>,
    length: i32,
}

impl<V: Clone> ResultHandler<V> for LongestMatchHandler<V> {
    fn handle_prefix_match(&mut self, match_length: i32, values: &[V]) -> bool {
        if match_length > self.length {
            self.length = match_length;
            self.matches = Some(values.to_vec());
        }
        true
    }
}

/// Iteration over the units of a text, case-folded per code point when
/// ignoring case.
///
/// Equivalent to `TextTrieMap.CharIterator` (Java's `Character` results are
/// `Option<u16>`).
#[derive(Debug, Clone)]
pub struct CharIterator<'a> {
    ignore_case: bool,
    text: &'a [u16],
    next_idx: i32,
    start_idx: i32,
    remaining_char: Option<u16>,
}

impl<'a> CharIterator<'a> {
    fn new(text: &'a [u16], offset: i32, ignore_case: bool) -> Self {
        Self {
            ignore_case,
            text,
            next_idx: offset,
            start_idx: offset,
            remaining_char: None,
        }
    }

    /// `hasNext()`.
    pub fn has_next(&self) -> bool {
        !(self.next_idx == self.text.len() as i32 && self.remaining_char.is_none())
    }

    /// `next()`: the next unit, `None` at the end. When ignoring case the
    /// index advances by the length of the *folded* code point, as ICU's.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Option<u16> {
        if self.next_idx == self.text.len() as i32 && self.remaining_char.is_none() {
            return None;
        }
        if let Some(c) = self.remaining_char.take() {
            return Some(c);
        }
        if self.ignore_case {
            let cp = UCharacter::fold_case_cp(code_point_at(self.text, self.next_idx), true);
            self.next_idx += char_count(cp);
            let mut chars = Vec::with_capacity(2);
            // A folded code point is always valid.
            let _ = append_code_point(&mut chars, cp);
            if chars.len() == 2 {
                self.remaining_char = Some(chars[1]);
            }
            Some(chars[0])
        } else {
            let c = self.text[self.next_idx as usize];
            self.next_idx += 1;
            Some(c)
        }
    }

    /// `remove()`.
    ///
    /// # Errors
    ///
    /// Always `UnsupportedOperationException("remove() not supported")`.
    pub fn remove(&mut self) -> Result<()> {
        Err(LuceneError::unsupported_operation("remove() not supported"))
    }

    /// `nextIndex()`.
    pub fn next_index(&self) -> i32 {
        self.next_idx
    }

    /// `processedLength()`.
    ///
    /// # Errors
    ///
    /// `IllegalStateException("In the middle of surrogate pair")` when the
    /// second half of a folded supplementary code point is pending.
    pub fn processed_length(&self) -> Result<i32> {
        if self.remaining_char.is_some() {
            return Err(LuceneError::illegal_state(
                "In the middle of surrogate pair",
            ));
        }
        Ok(self.next_idx - self.start_idx)
    }
}

/// `TextTrieMap.Node`: a run of units, the values of the key ending here,
/// and the children ordered by first unit (Java's `LinkedList`s are
/// `Vec`s; `null` lists are `None`).
#[derive(Debug)]
struct Node<V> {
    text: Vec<u16>,
    values: Option<Vec<V>>,
    children: Option<Vec<Node<V>>>,
}

impl<V> Node<V> {
    fn root() -> Self {
        Self {
            text: Vec::new(),
            values: None,
            children: None,
        }
    }

    fn char_count(&self) -> i32 {
        self.text.len() as i32
    }

    fn add_from_iter(&mut self, chitr: &mut CharIterator<'_>, value: V) {
        let mut buf = Vec::new();
        while chitr.has_next() {
            if let Some(c) = chitr.next() {
                buf.push(c);
            }
        }
        self.add(&buf, 0, value);
    }

    fn find_match(
        &self,
        chitr: &mut CharIterator<'_>,
        output: &mut Option<&mut Output>,
    ) -> Option<&Node<V>> {
        let children = self.children.as_ref()?;
        if !chitr.has_next() {
            if let Some(o) = output.as_deref_mut() {
                o.partial_match = true;
            }
            return None;
        }
        let ch = chitr.next()?;
        for child in children {
            if ch < child.text[0] {
                break;
            }
            if ch == child.text[0] {
                if child.match_following(chitr, output) {
                    return Some(child);
                }
                break;
            }
        }
        None
    }

    fn put_lead_code_points(&self, output: &mut UnicodeSet) -> Result<()> {
        let Some(children) = &self.children else {
            return Ok(());
        };
        for child in children {
            let c0 = child.text[0];
            if !is_high_surrogate(c0) {
                output.add(c0 as i32)?;
            } else if child.char_count() >= 2 {
                output.add(code_point_at(&child.text, 0))?;
            } else if let Some(grandchildren) = &child.children {
                // Iterate over all grandchildren.
                for grandchild in grandchildren {
                    let c1 = grandchild.text[0];
                    let cp = to_code_point(c0, c1);
                    output.add(cp)?;
                }
            }
        }
        Ok(())
    }

    fn add(&mut self, text: &[u16], offset: usize, value: V) {
        if text.len() == offset {
            self.values.get_or_insert_with(Vec::new).push(value);
            return;
        }
        let Some(children) = self.children.as_mut() else {
            self.children = Some(vec![Node {
                text: text[offset..].to_vec(),
                values: Some(vec![value]),
                children: None,
            }]);
            return;
        };
        // walk children
        let mut insert_at = children.len();
        for (i, next) in children.iter_mut().enumerate() {
            if text[offset] < next.text[0] {
                insert_at = i;
                break;
            }
            if text[offset] == next.text[0] {
                let match_len = next.len_matches(text, offset);
                if match_len == next.text.len() {
                    // matched
                    next.add(text, offset + match_len, value);
                } else {
                    // partial match, create a branch
                    next.split(match_len);
                    next.add(text, offset + match_len, value);
                }
                return;
            }
        }
        // add a new child to this node
        children.insert(
            insert_at,
            Node {
                text: text[offset..].to_vec(),
                values: Some(vec![value]),
                children: None,
            },
        );
    }

    fn match_following(
        &self,
        chitr: &mut CharIterator<'_>,
        output: &mut Option<&mut Output>,
    ) -> bool {
        let mut matched = true;
        let mut idx = 1;
        while idx < self.text.len() {
            if !chitr.has_next() {
                if let Some(o) = output.as_deref_mut() {
                    o.partial_match = true;
                }
                matched = false;
                break;
            }
            let ch = chitr.next();
            if ch != Some(self.text[idx]) {
                matched = false;
                break;
            }
            idx += 1;
        }
        matched
    }

    fn len_matches(&self, text: &[u16], offset: usize) -> usize {
        let text_len = text.len() - offset;
        let limit = self.text.len().min(text_len);
        let mut len = 0;
        while len < limit {
            if self.text[len] != text[offset + len] {
                break;
            }
            len += 1;
        }
        len
    }

    fn split(&mut self, offset: usize) {
        let child_text = self.text[offset..].to_vec();
        self.text.truncate(offset);
        let child = Node {
            text: child_text,
            values: self.values.take(),
            children: self.children.take(),
        };
        self.children = Some(vec![child]);
    }
}

impl<V> TextTrieMap<V> {
    /// `new TextTrieMap(boolean ignoreCase)`.
    pub fn new(ignore_case: bool) -> Self {
        Self {
            root: Mutex::new(Node::root()),
            ignore_case,
        }
    }

    /// `put(CharSequence text, V val)`: adds `val` under `text`.
    pub fn put(&self, text: &[u16], val: V) -> &Self {
        let mut chitr = CharIterator::new(text, 0, self.ignore_case);
        self.root
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .add_from_iter(&mut chitr, val);
        self
    }

    /// `find(CharSequence text, ResultHandler<V> handler)`.
    ///
    /// # Errors
    ///
    /// As [`find_from`](Self::find_from).
    pub fn find(&self, text: &[u16], handler: &mut dyn ResultHandler<V>) -> Result<()> {
        self.find_internal(text, 0, handler, None)
    }

    /// `find(CharSequence text, int offset, ResultHandler<V> handler)`:
    /// calls `handler` with the values of every key that is a prefix of
    /// `text` from `offset`, shortest first.
    ///
    /// # Errors
    ///
    /// `IllegalStateException("In the middle of surrogate pair")` when a
    /// key ends inside a folded supplementary code point.
    pub fn find_from(
        &self,
        text: &[u16],
        offset: i32,
        handler: &mut dyn ResultHandler<V>,
    ) -> Result<()> {
        self.find_internal(text, offset, handler, None)
    }

    fn find_internal(
        &self,
        text: &[u16],
        offset: i32,
        handler: &mut dyn ResultHandler<V>,
        mut output: Option<&mut Output>,
    ) -> Result<()> {
        let mut chitr = CharIterator::new(text, offset, self.ignore_case);
        let root = self.root.lock().unwrap_or_else(|e| e.into_inner());
        let mut node: &Node<V> = &root;
        loop {
            if let Some(values) = &node.values {
                if !handler.handle_prefix_match(chitr.processed_length()?, values) {
                    return Ok(());
                }
            }
            match node.find_match(&mut chitr, &mut output) {
                Some(next) => node = next,
                None => return Ok(()),
            }
        }
    }

    /// `putLeadCodePoints(UnicodeSet output)`: adds the first code point of
    /// every key.
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException` when `output` is frozen.
    pub fn put_lead_code_points(&self, output: &mut UnicodeSet) -> Result<()> {
        self.root
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .put_lead_code_points(output)
    }
}

impl<V: Clone> TextTrieMap<V> {
    /// `get(String text)`.
    ///
    /// # Errors
    ///
    /// As [`find_from`](Self::find_from).
    pub fn get(&self, text: &[u16]) -> Result<Option<Vec<V>>> {
        self.get_from(text, 0, None)
    }

    /// `get(CharSequence text, int start, Output output)` (and
    /// `get(CharSequence, int)` with `output` `None`): the values of the
    /// longest key that is a prefix of `text` from `start`, `None` when
    /// none is; `output` receives the match length and the partial-match
    /// flag.
    ///
    /// # Errors
    ///
    /// As [`find_from`](Self::find_from).
    pub fn get_from(
        &self,
        text: &[u16],
        start: i32,
        mut output: Option<&mut Output>,
    ) -> Result<Option<Vec<V>>> {
        let mut handler = LongestMatchHandler {
            matches: None,
            length: 0,
        };
        self.find_internal(text, start, &mut handler, output.as_deref_mut())?;
        if let Some(o) = output {
            o.match_length = handler.length;
        }
        Ok(handler.matches)
    }
}
