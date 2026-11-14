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
//! `sun.text.BreakDictionary` answers for.
//!
//! **Data source and divergence from JDK 25 (decided on rmp task #447).**
//! JDK 25 reads its compiled `thai_dict`, the resource
//! `sun/text/resources/ext/thai_dict` of the module `jdk.localedata`. This
//! data is GPL (with the Classpath Exception), so nothing of it is in this
//! crate: the dictionary is read **at run time from a JDK installed on the
//! machine**, once per process ([`ThaiDictionarySource`],
//! [`thai_dictionary_source`]). The lookup order:
//!
//! 1. `$JAVA_HOME/lib/modules`, when `JAVA_HOME` is set and not empty;
//! 2. `<home>/lib/modules` of the `java` executable found on `PATH`, symbolic
//!    links resolved (`<home>/bin/java`);
//! 3. the first of these whose image holds a readable `thai_dict` wins. The
//!    image is read with [`jimage`](super::jimage) and the dictionary data
//!    parsed as `BreakDictionary.setupDictionary` reads it; the JDK's version
//!    is not checked (the dictionary is the one of whichever JDK is found).
//! 4. Otherwise (no JDK, or its image or dictionary cannot be read), the
//!    machine is built from the Thai word list of ICU 79.1
//!    (`icu4c/source/data/brkitr/dictionaries/thaidict.txt` at tag
//!    `release-79.1`, commit `ddc6b66926bfac6036144ca9902992a0d6d50af9`,
//!    git blob `854b36e60f3fc2f01135ad3a23c153ea5643794d`, SHA-256
//!    `3166abde40c0f44ab91c28f5ce96d7d1472cb7882e1c0bda0a72f8f69dba4274`,
//!    26,383 words), embedded as
//!    `crates/rucene-analysis-common/resources/icu/thaidict.txt` under the
//!    Unicode License V3 (`resources/icu/LICENSE`). This fallback is the
//!    divergence: the two lists differ, so a Thai run can split into
//!    different words than on the JDK (measured on #447: 24.66% of 10,000
//!    seeded sentences split identically; Lucene's Thai test texts all do).
//!
//! Whichever the source, the machine answers as `BreakDictionary` does:
//! state 0 is both the start state and the "no word" (error) state; a
//! character of the dictionary's alphabet leads to the next state of the
//! words read so far, or to 0; any other character (the JDK's column 0)
//! leads to -1 ("end of word") from a state that ends a word, and to 0
//! otherwise.

use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use super::jimage;

/// The embedded ICU word list (UTF-8 with a byte order mark, one word per
/// line, `#` comment lines).
static WORD_LIST: &[u8] = include_bytes!("../../resources/icu/thaidict.txt");

/// The jimage name of the JDK's Thai dictionary: the module of
/// `sun.text.resources.ext.BreakIteratorResources_th` and the data name
/// `BreakIteratorInfo_th` gives for `WordDictionary`, as
/// `BreakIteratorResourceBundle.handleGetObject` resolves it.
const JDK_THAI_DICT: &str = "/jdk.localedata/sun/text/resources/ext/thai_dict";

/// Where the Thai dictionary of this process comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThaiDictionarySource {
    /// The JDK's own `thai_dict`, read from the run-time image at `image`
    /// (a `lib/modules` file).
    Jdk {
        /// The `lib/modules` file the dictionary was read from.
        image: PathBuf,
    },
    /// The embedded ICU 79.1 word list (no JDK found, or its dictionary
    /// could not be read).
    Icu,
}

/// The source of the Thai dictionary [`get_word_instance`] uses for `th`,
/// found once per process (see the module documentation for the lookup
/// order).
///
/// [`get_word_instance`]: super::break_iterator::get_word_instance
pub fn thai_dictionary_source() -> ThaiDictionarySource {
    THAI.0.clone()
}

/// The Thai dictionary of the process and its source.
static THAI: LazyLock<(ThaiDictionarySource, BreakDictionary)> = LazyLock::new(|| {
    for home in java_home_candidates() {
        let image = home.join("lib").join("modules");
        if let Ok(Some(data)) = jimage::read_resource(&image, JDK_THAI_DICT) {
            if let Some(dictionary) = JdkDictionary::parse(&data) {
                return (
                    ThaiDictionarySource::Jdk { image },
                    BreakDictionary::Jdk(dictionary),
                );
            }
        }
    }
    (ThaiDictionarySource::Icu, BreakDictionary::icu())
});

/// The embedded ICU dictionary, built once (the fallback, and the source a
/// test forces).
static ICU: LazyLock<BreakDictionary> = LazyLock::new(BreakDictionary::icu);

/// The JDK homes to try, in order: `JAVA_HOME`, then the home of `java` on
/// `PATH`.
fn java_home_candidates() -> Vec<PathBuf> {
    let mut homes = Vec::new();
    if let Some(home) = std::env::var_os("JAVA_HOME").filter(|h| !h.is_empty()) {
        homes.push(PathBuf::from(home));
    }
    if let Some(home) = java_on_path_home() {
        if !homes.contains(&home) {
            homes.push(home);
        }
    }
    homes
}

/// The home of the first `java` executable on `PATH`: the executable's
/// canonical path (symbolic links resolved) is `<home>/bin/java`.
fn java_on_path_home() -> Option<PathBuf> {
    let exe = if cfg!(windows) { "java.exe" } else { "java" };
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(exe))
        .find(|candidate| candidate.is_file())
        .and_then(|java| std::fs::canonicalize(java).ok())
        .and_then(|java| java.parent().and_then(Path::parent).map(Path::to_path_buf))
}

/// A node of the ICU dictionary's trie: whether a word ends here, and the
/// next node of each following UTF-16 unit, sorted by unit.
#[derive(Debug, Default)]
pub(crate) struct Node {
    word_end: bool,
    children: Vec<(u16, u32)>,
}

/// The emulation of `sun.text.BreakDictionary` (see the module
/// documentation).
#[derive(Debug)]
pub(crate) enum BreakDictionary {
    /// The JDK's compiled dictionary.
    Jdk(JdkDictionary),
    /// The machine built from the ICU word list.
    Icu {
        nodes: Vec<Node>,
        /// The units of the alphabet: the characters with a column.
        alphabet: Vec<u16>,
    },
}

impl BreakDictionary {
    /// Builds the machine from the ICU word list: every line but blank ones
    /// and those whose first non-blank character is `#`, trimmed, is a word.
    fn icu() -> BreakDictionary {
        let text = std::str::from_utf8(WORD_LIST).expect("INVARIANT: the word list is UTF-8");
        let text = text.strip_prefix('\u{FEFF}').unwrap_or(text);
        let mut nodes = vec![Node::default()];
        let mut alphabet: Vec<u16> = Vec::new();
        for word in text
            .lines()
            .map(str::trim)
            .filter(|word| !word.is_empty() && !word.starts_with('#'))
        {
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
        BreakDictionary::Icu { nodes, alphabet }
    }

    /// The Thai dictionary of the process: the JDK's, else ICU's.
    pub(crate) fn thai() -> &'static BreakDictionary {
        &THAI.1
    }

    /// The ICU dictionary, whatever the process's source.
    pub(crate) fn icu_thai() -> &'static BreakDictionary {
        &ICU
    }

    /// `getNextState(row, 0)`: -1 if a word ends at `state`, else 0 (for
    /// the JDK's dictionary, its cell).
    pub(crate) fn end_of_word_state(&self, state: i32) -> i32 {
        match self {
            BreakDictionary::Jdk(d) => d.get_next_state(state, 0),
            BreakDictionary::Icu { nodes, .. } => {
                if nodes[state as usize].word_end {
                    -1
                } else {
                    0
                }
            }
        }
    }

    /// `getNextStateFromCharacter(row, ch)`: the state after `ch` (a code
    /// point) from `state`.
    pub(crate) fn get_next_state_from_character(&self, state: i32, ch: i32) -> i32 {
        match self {
            BreakDictionary::Jdk(d) => d.get_next_state_from_character(state, ch),
            BreakDictionary::Icu { nodes, alphabet } => {
                let in_alphabet = u16::try_from(ch)
                    .ok()
                    .filter(|unit| alphabet.binary_search(unit).is_ok());
                match in_alphabet {
                    None => self.end_of_word_state(state),
                    Some(unit) => {
                        let node = &nodes[state as usize];
                        match node.children.binary_search_by_key(&unit, |&(u, _)| u) {
                            Ok(i) => node.children[i].1 as i32,
                            Err(_) => 0,
                        }
                    }
                }
            }
        }
    }
}

/// `CompactByteArray`'s `BLOCKSHIFT` and the derived counts.
const BLOCKSHIFT: usize = 7;
const BLOCKCOUNT: usize = 1 << BLOCKSHIFT;
const INDEXCOUNT: usize = 1 << (16 - BLOCKSHIFT);
const BLOCKMASK: usize = BLOCKCOUNT - 1;

/// The JDK's compiled dictionary, the fields `BreakDictionary.setupDictionary`
/// reads (written from JDK 25's `sun.text.BreakDictionary`,
/// `sun.text.CompactByteArray` and `sun.text.SupplementaryCharacterData`
/// read as a specification).
#[derive(Debug)]
pub(crate) struct JdkDictionary {
    /// `columnMap`: `CompactByteArray(indices, values)`.
    column_indices: Vec<u16>,
    column_values: Vec<i8>,
    /// `supplementaryCharColumnMap`'s data table.
    supplementary: Vec<i32>,
    num_cols: i32,
    table: Vec<i16>,
    row_index: Vec<i16>,
    row_index_flags: Vec<i32>,
    row_index_flags_index: Vec<i16>,
    row_index_shifts: Vec<i8>,
}

/// A big-endian reader over the dictionary data (`ByteBuffer.wrap`).
struct Data<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl Data<'_> {
    fn take(&mut self, n: usize) -> Option<&[u8]> {
        let s = self.bytes.get(self.pos..self.pos.checked_add(n)?)?;
        self.pos += n;
        Some(s)
    }
    fn int(&mut self) -> Option<i32> {
        Some(i32::from_be_bytes(self.take(4)?.try_into().ok()?))
    }
    fn len(&mut self) -> Option<usize> {
        usize::try_from(self.int()?).ok()
    }
    fn shorts(&mut self) -> Option<Vec<i16>> {
        let n = self.len()?;
        let b = self.take(n.checked_mul(2)?)?;
        Some(
            b.chunks(2)
                .map(|c| i16::from_be_bytes([c[0], c[1]]))
                .collect(),
        )
    }
    fn ints(&mut self) -> Option<Vec<i32>> {
        let n = self.len()?;
        let b = self.take(n.checked_mul(4)?)?;
        Some(
            b.chunks(4)
                .map(|c| i32::from_be_bytes([c[0], c[1], c[2], c[3]]))
                .collect(),
        )
    }
    fn bytes(&mut self) -> Option<Vec<i8>> {
        let n = self.len()?;
        Some(self.take(n)?.iter().map(|&b| b as i8).collect())
    }
}

impl JdkDictionary {
    /// `setupDictionary(name, data)`; `None` where Java throws
    /// (`MissingResourceException` for an unsupported version, a wrong size
    /// or corrupted data, `IllegalArgumentException` for a bad column map).
    fn parse(bytes: &[u8]) -> Option<JdkDictionary> {
        let mut bb = Data { bytes, pos: 0 };
        // supportedVersion
        if bb.int()? != 1 {
            return None;
        }
        let len = bb.len()?;
        if bb.pos + len != bytes.len() {
            return None;
        }
        let indices = bb.shorts()?;
        let values = bb.bytes()?;
        // `new CompactByteArray(indexArray, newValues)`
        if indices.len() != INDEXCOUNT
            || indices
                .iter()
                .any(|&i| i < 0 || i as usize >= values.len() + BLOCKCOUNT)
        {
            return None;
        }
        let num_cols = bb.int()?;
        let _num_col_groups = bb.int()?;
        let row_index = bb.shorts()?;
        let row_index_flags_index = bb.shorts()?;
        let row_index_flags = bb.ints()?;
        let row_index_shifts = bb.bytes()?;
        let table = bb.shorts()?;
        let supplementary = bb.ints()?;
        if supplementary.len() < 2 || bb.pos != bytes.len() {
            return None;
        }
        Some(JdkDictionary {
            column_indices: indices.into_iter().map(|i| i as u16).collect(),
            column_values: values,
            supplementary,
            num_cols,
            table,
            row_index,
            row_index_flags,
            row_index_flags_index,
            row_index_shifts,
        })
    }

    /// `CompactByteArray.elementAt(char)`.
    fn column_of(&self, ch: u16) -> i32 {
        let index = ch as usize;
        let block = self.column_indices[index >> BLOCKSHIFT] as usize;
        i32::from(self.column_values[block + (index & BLOCKMASK)])
    }

    /// `SupplementaryCharacterData.getValue(int)`.
    fn supplementary_column(&self, index: i32) -> i32 {
        let table = &self.supplementary;
        let (mut i, mut j) = (0usize, table.len() - 1);
        loop {
            let k = (i + j) / 2;
            let start = table[k] >> 8;
            let end = table[k + 1] >> 8;
            if index < start {
                j = k;
            } else if index > end - 1 {
                i = k;
            } else {
                let v = table[k] & 0xFF;
                return if v == 0xFF { -1 } else { v };
            }
        }
    }

    /// `getNextStateFromCharacter(row, ch)`.
    fn get_next_state_from_character(&self, row: i32, ch: i32) -> i32 {
        let col = if ch < 0x10000 {
            self.column_of(ch as u16)
        } else {
            self.supplementary_column(ch)
        };
        self.get_next_state(row, col)
    }

    /// `getNextState(row, col)`.
    fn get_next_state(&self, row: i32, col: i32) -> i32 {
        if self.cell_is_populated(row, col) {
            let r = i32::from(self.row_index[row as usize]);
            let c = col + i32::from(self.row_index_shifts[row as usize]);
            i32::from(self.table[(r * self.num_cols + c) as usize])
        } else {
            0
        }
    }

    /// `cellIsPopulated(row, col)`.
    fn cell_is_populated(&self, row: i32, col: i32) -> bool {
        let flags_index = i32::from(self.row_index_flags_index[row as usize]);
        if flags_index < 0 {
            col == -flags_index
        } else {
            let flags = self.row_index_flags[(flags_index + (col >> 5)) as usize];
            (flags & (1 << (col & 0x1f))) != 0
        }
    }
}
