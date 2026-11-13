/*
 * Licensed to the Apache Software Foundation (ASF) under one or more
 * contributor license agreements.  See the NOTICE file distributed with
 * this work for additional information regarding copyright ownership.
 * The ASF licenses this file to You under the Apache License, Version 2.0
 * (the "License"); you may not use this file except in compliance with
 * the License.  You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

//! The word dictionary of the Thai word iterator: the state machine
//! `sun.text.BreakDictionary` answers for, built from a Unicode-licensed
//! word list.
//!
//! **Divergence from JDK 25 (data source, approved on rmp task #447).** The
//! JDK reads its own compiled `thai_dict`; this emulation builds the same
//! kind of machine from the Thai word list of ICU 79.1
//! (`icu4c/source/data/brkitr/dictionaries/thaidict.txt` at tag
//! `release-79.1`, commit `ddc6b66926bfac6036144ca9902992a0d6d50af9`, git
//! blob `854b36e60f3fc2f01135ad3a23c153ea5643794d`, SHA-256
//! `3166abde40c0f44ab91c28f5ce96d7d1472cb7882e1c0bda0a72f8f69dba4274`,
//! 26,383 words), embedded as
//! `crates/rucene-analysis-common/resources/icu/thaidict.txt` under the
//! Unicode License V3 (`resources/icu/LICENSE`). The two lists differ, so a
//! Thai run can split into different words than on the JDK; the algorithm
//! that splits it ([`DictionaryBasedBreakIterator`](super::break_iterator))
//! is the JDK's.
//!
//! The machine answers as `BreakDictionary` does: state 0 is both the start
//! state and the "no word" (error) state; a character of the dictionary's
//! alphabet leads to the next state of the words read so far, or to 0; any
//! other character (the JDK's column 0) leads to -1 ("end of word") from a
//! state that ends a word, and to 0 otherwise.

use std::sync::LazyLock;

/// The embedded word list (UTF-8 with a byte order mark, one word per line,
/// `#` comment lines).
static WORD_LIST: &[u8] = include_bytes!("../../resources/icu/thaidict.txt");

/// A node of the dictionary's trie: whether a word ends here, and the next
/// node of each following UTF-16 unit, sorted by unit.
#[derive(Debug, Default)]
struct Node {
    word_end: bool,
    children: Vec<(u16, u32)>,
}

/// The emulation of `sun.text.BreakDictionary` (see the module
/// documentation).
#[derive(Debug)]
pub(crate) struct BreakDictionary {
    nodes: Vec<Node>,
    /// The units of the alphabet: the characters with a column.
    alphabet: Vec<u16>,
}

impl BreakDictionary {
    /// Builds the machine from the word list: every line but blank ones and
    /// those whose first non-blank character is `#`, trimmed, is a word.
    fn load() -> BreakDictionary {
        let text = std::str::from_utf8(WORD_LIST).expect("INVARIANT: the word list is UTF-8");
        let text = text.strip_prefix('\u{FEFF}').unwrap_or(text);
        let mut nodes = vec![Node::default()];
        let mut alphabet: Vec<u16> = Vec::new();
        for line in text.lines() {
            let word = line.trim();
            if word.is_empty() || word.starts_with('#') {
                continue;
            }
            let mut node = 0usize;
            for unit in word.encode_utf16() {
                alphabet.push(unit);
                let next = match nodes[node]
                    .children
                    .binary_search_by_key(&unit, |&(u, _)| u)
                {
                    Ok(i) => nodes[node].children[i].1 as usize,
                    Err(i) => {
                        let id = nodes.len();
                        nodes.push(Node::default());
                        nodes[node].children.insert(i, (unit, id as u32));
                        id
                    }
                };
                node = next;
            }
            nodes[node].word_end = true;
        }
        alphabet.sort_unstable();
        alphabet.dedup();
        BreakDictionary { nodes, alphabet }
    }

    /// The Thai dictionary, built once.
    pub(crate) fn thai() -> &'static BreakDictionary {
        static DICTIONARY: LazyLock<BreakDictionary> = LazyLock::new(BreakDictionary::load);
        &DICTIONARY
    }

    /// `getNextState(row, 0)`: -1 if a word ends at `state`, else 0.
    pub(crate) fn end_of_word_state(&self, state: i32) -> i32 {
        if self.nodes[state as usize].word_end {
            -1
        } else {
            0
        }
    }

    /// `getNextStateFromCharacter(row, ch)`: the state after `ch` (a code
    /// point) from `state`.
    pub(crate) fn get_next_state_from_character(&self, state: i32, ch: i32) -> i32 {
        let in_alphabet = u16::try_from(ch)
            .ok()
            .filter(|unit| self.alphabet.binary_search(unit).is_ok());
        match in_alphabet {
            None => self.end_of_word_state(state),
            Some(unit) => {
                let node = &self.nodes[state as usize];
                match node.children.binary_search_by_key(&unit, |&(u, _)| u) {
                    Ok(i) => node.children[i].1 as i32,
                    Err(_) => 0,
                }
            }
        }
    }
}
