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

//! The state tables of JDK 25's rule-based break iterators: the word and
//! line iterators of the root locale, the word and line iterators of Thai
//! (before their dictionary splits Thai runs) and the sentence iterator,
//! forward and backward.
//!
//! JDK 25 compiles, when the JDK is built, the rule sets of
//! `sun.text.resources.BreakIteratorRules` (`WordBreakRules`,
//! `LineBreakRules`, `SentenceBreakRules`) and
//! `sun.text.resources.ext.BreakIteratorRules_th` (`WordBreakRules`,
//! `LineBreakRules`) into state tables over character categories, which
//! `sun.text.RuleBasedBreakIterator.handleNext` runs: from the current
//! boundary it feeds code points to the table (an "ignore" character leaves
//! the state as it is) until the table stops or the text ends, and the
//! boundary is the end of the last "end" state reached, or, for a rule with a
//! `/`, the position of the `/`.
//!
//! This module rebuilds those tables from the rules' meaning, read from the
//! JDK 25 sources as specification; no JDK rule text and no compiled table is
//! reproduced:
//!
//! * the two word and the two line rule sets (no `*?`) are written here as regular expressions
//!   over the rules' character classes and compiled by subset construction,
//!   the state flags ("end", "lookahead") being those of the NFA positions a
//!   state holds, as the JDK's builder merges them;
//! * the sentence rules, all of the form `.*?…` (shortest match, with the
//!   builder's "backfilling" of the loop state into the states that follow
//!   it), are the state machine the JDK's builder produces for them, written
//!   by hand: `sentence_rules` documents each state;
//! * the backward tables, which `handlePrevious` runs from a position back to
//!   a position the rules always break at, are what the JDK's builder makes
//!   of the forward table: a machine over pairs of adjacent categories that
//!   continues while the forward table has some state taking the left
//!   category into a state with a transition on the right one, and stops at
//!   the first pair it never continues across (see `pair_table`); for the
//!   sentence rules, the backward-only (`!`) rules come first (see
//!   `sentence_rules`).
//!
//! Character classes use JDK 25's general categories (`Character.getType`,
//! Unicode 16.0, the data the JDK's build reads), as the JDK's builder
//! reads them (`type_segments`: a gap before a `UnicodeData.txt` `First>`
//! range of the same category joins the range), and supplementary code
//! points are looked up in a table built as the JDK builds its
//! `SupplementaryCharacterData` (`supplementary_table`: an ignore range
//! loses its last code point, a single-code-point one vanishes).
//!
//! **Verification.** Java models of these tables, of `handleNext`,
//! `handlePrevious` and the movements built on them were checked against
//! JDK 25.0.5 (`BreakIterator.getWordInstance(Locale.ROOT)`,
//! `getSentenceInstance(Locale.ROOT)` and `getWordInstance(th)`, the last on
//! the JDK's own dictionary) on pseudo-random strings with random sequences
//! of `first`, `last`, `next`, `previous`, `following`, `preceding` and
//! `isBoundary`, with no difference: `tools/translate/BrkCheck.java`,
//! `DbbiCheck.java` and `BrkBackCheck.java` (see `tools/translate/README.md`).
//! The port's own comparison with JDK 25 belongs to its tests.

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::sync::LazyLock;

use super::character_type_data::TYPES;

/// `RuleBasedBreakIterator.IGNORE`: the category of an ignore character.
pub(crate) const IGNORE: i32 = -1;

/// `RuleBasedBreakIterator.START_STATE`.
pub(crate) const START_STATE: usize = 1;

/// `RuleBasedBreakIterator.STOP_STATE`.
pub(crate) const STOP_STATE: usize = 0;

// General categories, as `Character.getType` numbers them.
const LU: u8 = 1;
const LL: u8 = 2;
const LT: u8 = 3;
const LM: u8 = 4;
const LO: u8 = 5;
const MN: u8 = 6;
const ME: u8 = 7;
const MC: u8 = 8;
const ND: u8 = 9;
const NL: u8 = 10;
const NO: u8 = 11;
const ZS: u8 = 12;
const ZL: u8 = 13;
const ZP: u8 = 14;
const CC: u8 = 15;
const CF: u8 = 16;
const PD: u8 = 20;
const PS: u8 = 21;
const PE: u8 = 22;
const PC: u8 = 23;
const SC: u8 = 26;
const PI: u8 = 29;
const PF: u8 = 30;

/// The bit of general category `t` in a [`Set::Types`] mask.
const fn ty(t: u8) -> u32 {
    1 << t
}

/// `[:L:]`.
const LETTER: u32 = ty(LU) | ty(LL) | ty(LT) | ty(LM) | ty(LO);

/// `[:N:]`.
const NUMBER: u32 = ty(ND) | ty(NL) | ty(NO);

/// A set of code points, as the rules' `[]` expressions describe one.
#[derive(Debug, Clone)]
enum Set {
    /// Every code point.
    All,
    /// The code points of the general categories of the mask.
    Types(u32),
    /// The code points `first..=last`.
    Range(u32, u32),
    /// The union of the sets.
    Union(Vec<Set>),
    /// The first set without the second.
    Diff(Box<Set>, Box<Set>),
}

impl Set {
    /// A single code point.
    fn ch(c: u32) -> Set {
        Set::Range(c, c)
    }

    /// The code points of `chars`.
    fn chars(chars: &[u32]) -> Set {
        Set::Union(chars.iter().map(|&c| Set::ch(c)).collect())
    }

    /// `self` without `other`.
    fn minus(self, other: Set) -> Set {
        Set::Diff(Box::new(self), Box::new(other))
    }

    /// Whether code point `c`, of general category `t`, is in the set.
    fn contains(&self, c: u32, t: u8) -> bool {
        match self {
            Set::All => true,
            Set::Types(mask) => mask & ty(t) != 0,
            Set::Range(first, last) => (*first..=*last).contains(&c),
            Set::Union(sets) => sets.iter().any(|s| s.contains(c, t)),
            Set::Diff(a, b) => a.contains(c, t) && !b.contains(c, t),
        }
    }

    /// Adds to `points` every code point at which membership can change
    /// other than at a general-category boundary.
    fn breakpoints(&self, points: &mut BTreeSet<u32>) {
        match self {
            Set::All | Set::Types(_) => {}
            Set::Range(first, last) => {
                points.insert(*first);
                points.insert(last + 1);
            }
            Set::Union(sets) => sets.iter().for_each(|s| s.breakpoints(points)),
            Set::Diff(a, b) => {
                a.breakpoints(points);
                b.breakpoints(points);
            }
        }
    }
}

/// The code points of the `<..., First>` lines of Unicode 16.0's
/// `UnicodeData.txt`: the starts of the ranges it lists by their first and
/// last code points (CJK ideographs and extensions, Hangul syllables,
/// surrogates, Tangut, private use).
const UNICODE_DATA_RANGE_STARTS: &[u32] = &[
    0x3400, 0x4E00, 0xAC00, 0xD800, 0xDB80, 0xDC00, 0xE000, 0x17000, 0x18D00, 0x20000, 0x2A700,
    0x2B740, 0x2B820, 0x2CEB0, 0x2EBF0, 0x30000, 0x31350, 0xF0000, 0x100000,
];

/// The segments of the code space over which the general category is
/// constant, as the JDK's break-iterator builder sees the categories:
/// `(first code point, category)` in order, unassigned gaps being category
/// 0 (`Character.getType`), except a gap just before a `<..., First>` range
/// of `UnicodeData.txt` whose preceding code point has the range's
/// category: the gap takes that category.
///
/// The exception is how `build.tools.generatebreakiteratordata.CharacterCategory`
/// (`storeNewData`) of JDK 25 reads `UnicodeData.txt`, the data of the
/// JDK's compiled break-iterator tables: a `First>` line of the same
/// category as the line before continues the current run without checking
/// for a gap (only a line that is neither `First>` nor `Last>` is), so the
/// gap ends up in the run. Observed on JDK 25.0.5: `U+2A6E0..U+2A6FF`, the
/// gap between CJK Extensions B and C, joins letters in a word like an
/// ideograph (`tests/break_iterator_jdk_oracle.rs`).
fn type_segments() -> Vec<(u32, u8)> {
    let mut segments = Vec::with_capacity(TYPES.len() * 2);
    let mut next = 0u32;
    for &(first, last, t) in TYPES {
        if first > next {
            segments.push((next, 0));
        }
        segments.push((first, t));
        next = last + 1;
    }
    if next <= 0x10FFFF {
        segments.push((next, 0));
    }
    for &start in UNICODE_DATA_RANGE_STARTS {
        let Ok(i) = segments.binary_search_by_key(&start, |&(first, _)| first) else {
            continue;
        };
        // a gap (category 0) right before the range, after a code point of
        // the range's category
        if i >= 2 && segments[i - 1].1 == 0 && segments[i - 2].1 == segments[i].1 {
            segments[i - 1].1 = segments[i].1;
        }
    }
    segments
}

/// The category assignment of a table: `ranges` maps code points (from each
/// start to the next) to a category, or [`IGNORE`].
struct Categories {
    ranges: Vec<(u32, i32)>,
    /// The number of categories.
    count: usize,
}

/// Partitions the code space by `classify`, which answers a category key
/// for a code point and its general category (`None` for an ignore
/// character); each distinct key becomes a category, numbered in order of
/// first appearance. Membership only changes at general-category boundaries
/// and at `points`.
fn categorize<K: Ord + Clone>(
    points: &BTreeSet<u32>,
    classify: impl Fn(u32, u8) -> Option<K>,
) -> (Categories, Vec<K>) {
    let types = type_segments();
    let mut starts: BTreeMap<u32, u8> = BTreeMap::new();
    for &(first, t) in &types {
        starts.insert(first, t);
    }
    // the general category in force at each breakpoint
    for &p in points {
        if p > 0x10FFFF {
            continue;
        }
        let index = types.partition_point(|&(first, _)| first <= p) - 1;
        starts.entry(p).or_insert(types[index].1);
    }
    let mut keys: Vec<K> = Vec::new();
    let mut ids: BTreeMap<K, i32> = BTreeMap::new();
    let mut ranges: Vec<(u32, i32)> = Vec::new();
    for (&first, &t) in &starts {
        let category = match classify(first, t) {
            None => IGNORE,
            Some(key) => *ids.entry(key.clone()).or_insert_with(|| {
                keys.push(key);
                (keys.len() - 1) as i32
            }),
        };
        if ranges.last().map(|&(_, c)| c) != Some(category) {
            ranges.push((first, category));
        }
    }
    (
        Categories {
            ranges,
            count: keys.len(),
        },
        keys,
    )
}

/// A forward state table: what `RuleBasedBreakIterator.setupTables` reads
/// from the JDK's compiled data (`charCategoryTable`, `stateTable`,
/// `endStates`, `lookaheadStates`, and, for a dictionary iterator, the
/// `categoryFlags` of `DictionaryBasedBreakIterator`).
#[derive(Debug)]
pub(crate) struct RuleTable {
    /// `(first code point, category)`, in order.
    ranges: Vec<(u32, i32)>,
    /// The category of every BMP code point, the role of the JDK's
    /// `charCategoryTable` (`RuleBasedBreakIterator.lookupCategory`: a
    /// `CompactByteArray` read in constant time for `c < 0x10000`).
    ///
    /// PERF (rmp #680): built once from `ranges`, so a BMP lookup is one
    /// load instead of a binary search over the ranges per character.
    bmp: Box<[i16]>,
    /// The number of categories, the width of `state_table`.
    num_categories: usize,
    /// The next state by state and category; state 0 is `STOP_STATE`.
    state_table: Vec<u16>,
    /// Whether each state is an accepting ("end") state.
    end_states: Vec<bool>,
    /// Whether each state is a lookahead state.
    lookahead_states: Vec<bool>,
    /// The next state by state and category when moving backward
    /// (`backwardsStateTable`); state 0 stops, state 1 starts.
    backwards_state_table: Vec<u16>,
    /// Whether each category holds dictionary characters
    /// (`DictionaryBasedBreakIterator.categoryFlags`); empty for a table
    /// without a dictionary.
    category_flags: Vec<bool>,
    /// The categories of the supplementary code points, as the JDK's
    /// `SupplementaryCharacterData` table (see [`supplementary_table`]).
    supplementary: Vec<i32>,
    /// The category of a code point the rules do not mention (the JDK's
    /// category 0), which the supplementary table's default entries name.
    default_category: i32,
}

/// The category of each BMP code point (`0..0x10000`), from the category runs
/// `ranges` (each run lasts until the next one starts).
fn bmp_table(ranges: &[(u32, i32)]) -> Box<[i16]> {
    let mut table = vec![0i16; 0x10000];
    for (i, &(first, category)) in ranges.iter().enumerate() {
        if first >= 0x10000 {
            break;
        }
        let end = ranges
            .get(i + 1)
            .map_or(0x10000, |&(next, _)| next.min(0x10000));
        let category = i16::try_from(category)
            .expect("INVARIANT: a rule table has fewer than 2^15 categories");
        table[first as usize..end as usize].fill(category);
    }
    table.into_boxed_slice()
}

/// The encoding of a category in [`supplementary_table`]: the JDK's value
/// byte, `0` for the default category, `-1` (`0xFF`) for [`IGNORE`], and
/// `1 +` the port's number for the others.
fn supplementary_byte(category: i32, default_category: i32) -> Option<i64> {
    if category == IGNORE {
        Some(-1)
    } else if category == default_category {
        None
    } else {
        Some(i64::from(category) + 1)
    }
}

/// The table `SupplementaryCharacterData.getValue` searches, built from the
/// category runs `ranges` as the JDK's break-iterator builder builds it
/// (`build.tools.generatebreakiteratordata.RuleBasedBreakIteratorBuilder`
/// appending each category's supplementary ranges to its
/// `SupplementaryCharacterData`, then `generateTable`), derived from the
/// JDK 25 sources read as a specification.
///
/// Two consequences of that construction are the JDK's observable
/// behaviour and reproduced here (observed on JDK 25.0.5 by
/// `tests/break_iterator_jdk_oracle.rs`):
///
/// * an entry is the long `((start << 24) + end) << 8) + value` with the
///   ignore value the byte `-1`, so for an ignore range the addition borrows
///   from `end`: the range loses its last code point (`U+1343F`, the last
///   of the Egyptian format controls `U+13430..U+1343F`, is not ignored);
/// * a default entry follows a range only if the range is not a single
///   code point (`end != start`) and a gap follows it; for an ignore range
///   whose `end` the borrow moved below `start`, the default entry at
///   `start` hides the range (`U+11001`, a single nonspacing mark, is not
///   ignored).
fn supplementary_table(ranges: &[(u32, i32)], default_category: i32) -> Vec<i32> {
    const UPPER_LIMIT: i64 = 0x10FFFF + 1;
    const CODEPOINT_MASK: i64 = 0x1F_FFFF;
    const MIN_SUPPLEMENTARY_CODE_POINT: i64 = 0x10000;
    let compose_entry =
        |code_point: i64, value: i64| -> i32 { ((code_point as i32) << 8) | (value as i32 & 0xFF) };
    // `appendElement(start, end, value)` for each supplementary range
    let mut temp: Vec<i64> = Vec::new();
    for (i, &(first, category)) in ranges.iter().enumerate() {
        let last = ranges.get(i + 1).map_or(0x10FFFF, |&(next, _)| next - 1);
        if i64::from(last) < MIN_SUPPLEMENTARY_CODE_POINT {
            continue;
        }
        let Some(value) = supplementary_byte(category, default_category) else {
            continue;
        };
        let start = i64::from(first).max(MIN_SUPPLEMENTARY_CODE_POINT);
        temp.push((((start << 24) + i64::from(last)) << 8) + value);
    }
    // `generateTable()`
    if temp.is_empty() {
        return vec![compose_entry(0, 0), compose_entry(UPPER_LIMIT, 0)];
    }
    temp.sort_unstable();
    let mut table: Vec<i32> = Vec::with_capacity(temp.len() * 2 + 3);
    let mut data = temp[0];
    let mut start = (data >> 32) & CODEPOINT_MASK;
    let mut end = (data >> 8) & CODEPOINT_MASK;
    if start != MIN_SUPPLEMENTARY_CODE_POINT {
        table.push(compose_entry(MIN_SUPPLEMENTARY_CODE_POINT, 0));
    }
    table.push(compose_entry(start, data));
    for &next in &temp[1..] {
        data = next;
        let next_start = (data >> 32) & CODEPOINT_MASK;
        if end != start && end != next_start - 1 {
            table.push(compose_entry(end + 1, 0));
        }
        table.push(compose_entry(next_start, data));
        start = next_start;
        end = (data >> 8) & CODEPOINT_MASK;
    }
    end += 1;
    table.push(compose_entry(end, 0));
    if end < UPPER_LIMIT {
        table.push(compose_entry(UPPER_LIMIT, 0));
    }
    table
}

/// `SupplementaryCharacterData.getValue(index)` over `table`, decoded to a
/// category (see [`supplementary_byte`]).
fn supplementary_category(table: &[i32], index: i32, default_category: i32) -> i32 {
    let mut i = 0usize;
    let mut j = table.len() - 1;
    loop {
        let k = (i + j) / 2;
        let start = table[k] >> 8;
        let end = table[k + 1] >> 8;
        if index < start {
            j = k;
        } else if index > end - 1 {
            i = k;
        } else {
            return match table[k] & 0xFF {
                0xFF => IGNORE,
                0 => default_category,
                v => v - 1,
            };
        }
    }
}

impl RuleTable {
    /// `lookupCategory(int)`: the category of code point `c`, or
    /// [`IGNORE`].
    ///
    /// A supplementary code point is looked up in the JDK's
    /// `SupplementaryCharacterData` table ([`supplementary_table`]).
    #[inline]
    pub(crate) fn lookup_category(&self, c: i32) -> i32 {
        if let Some(&category) = usize::try_from(c).ok().and_then(|c| self.bmp.get(c)) {
            return i32::from(category);
        }
        if (0x10000..=0x10FFFF).contains(&c) {
            return supplementary_category(&self.supplementary, c, self.default_category);
        }
        let c = c.clamp(0, 0x10FFFF) as u32;
        let index = self.ranges.partition_point(|&(first, _)| first <= c) - 1;
        self.ranges[index].1
    }

    /// `lookupState(int, int)`.
    pub(crate) fn lookup_state(&self, state: usize, category: i32) -> usize {
        usize::from(self.state_table[state * self.num_categories + category as usize])
    }

    /// `endStates[state]`.
    pub(crate) fn is_end_state(&self, state: usize) -> bool {
        self.end_states[state]
    }

    /// `lookaheadStates[state]`.
    pub(crate) fn is_lookahead_state(&self, state: usize) -> bool {
        self.lookahead_states[state]
    }

    /// `lookupBackwardState(int, int)`.
    pub(crate) fn lookup_backward_state(&self, state: usize, category: i32) -> usize {
        usize::from(self.backwards_state_table[state * self.num_categories + category as usize])
    }

    /// `categoryFlags[category]`: whether a category holds dictionary
    /// characters.
    pub(crate) fn is_dictionary_category(&self, category: i32) -> bool {
        self.category_flags
            .get(category as usize)
            .copied()
            .unwrap_or(false)
    }
}

// ------------------------------------------------- the word rule compiler

/// A rule, or part of one, as a regular expression over classes.
#[derive(Debug, Clone)]
enum Re {
    /// One code point of class `n` of the rule set.
    Class(usize),
    /// `.`: any code point that is not an ignore character.
    Any,
    Seq(Vec<Re>),
    Alt(Vec<Re>),
    /// `{}`.
    Opt(Box<Re>),
    /// `*`.
    Star(Box<Re>),
    /// `/`: the break goes here if the rest of the rule matches.
    Slash,
}

fn cls(n: usize) -> Re {
    Re::Class(n)
}

fn seq(parts: Vec<Re>) -> Re {
    Re::Seq(parts)
}

fn opt(r: Re) -> Re {
    Re::Opt(Box::new(r))
}

fn star(r: Re) -> Re {
    Re::Star(Box::new(r))
}

/// The NFA position flag of an accepting position.
const FLAG_END: u8 = 1;
/// The NFA position flag of a lookahead position.
const FLAG_LOOKAHEAD: u8 = 2;

/// A Thompson NFA over the classes of a rule set.
#[derive(Default)]
struct Nfa {
    epsilon: Vec<Vec<usize>>,
    /// The transition of a position: `(class, target)`, class `None` for
    /// `.`.
    edge: Vec<Option<(Option<usize>, usize)>>,
    flags: Vec<u8>,
}

impl Nfa {
    fn add(&mut self) -> usize {
        self.epsilon.push(Vec::new());
        self.edge.push(None);
        self.flags.push(0);
        self.epsilon.len() - 1
    }

    /// Builds `r` from position `from`, returning its exit position; sets
    /// `slash` when `r` holds the rule's `/`.
    fn build(&mut self, r: &Re, from: usize, slash: &mut bool) -> usize {
        match r {
            Re::Class(n) => {
                let to = self.add();
                self.edge[from] = Some((Some(*n), to));
                to
            }
            Re::Any => {
                let to = self.add();
                self.edge[from] = Some((None, to));
                to
            }
            Re::Slash => {
                *slash = true;
                let to = self.add();
                self.epsilon[from].push(to);
                self.flags[to] = FLAG_LOOKAHEAD;
                to
            }
            Re::Seq(parts) => {
                let mut current = from;
                for part in parts {
                    let next = self.add();
                    self.epsilon[current].push(next);
                    current = self.build(part, next, slash);
                }
                current
            }
            Re::Alt(parts) => {
                let end = self.add();
                for part in parts {
                    let branch = self.add();
                    self.epsilon[from].push(branch);
                    let exit = self.build(part, branch, slash);
                    self.epsilon[exit].push(end);
                }
                end
            }
            Re::Opt(inner) => {
                let branch = self.add();
                self.epsilon[from].push(branch);
                let exit = self.build(inner, branch, slash);
                let end = self.add();
                self.epsilon[exit].push(end);
                self.epsilon[from].push(end);
                end
            }
            Re::Star(inner) => {
                let lp = self.add();
                self.epsilon[from].push(lp);
                let branch = self.add();
                self.epsilon[lp].push(branch);
                let exit = self.build(inner, branch, slash);
                self.epsilon[exit].push(lp);
                lp
            }
        }
    }

    fn closure(&self, set: &mut BTreeSet<usize>) {
        let mut queue: VecDeque<usize> = set.iter().copied().collect();
        while let Some(x) = queue.pop_front() {
            for &y in &self.epsilon[x] {
                if set.insert(y) {
                    queue.push_back(y);
                }
            }
        }
    }
}

/// Compiles a word rule set: `classes` are its `[]` expressions (each rule
/// refers to them by index), `ignore` the `<ignore>` set (its code points in
/// no class are the ignore characters), `dictionary` the class of the
/// `<dictionary>` characters, if any.
fn compile(classes: &[Set], ignore: &Set, dictionary: Option<usize>, rules: &[Re]) -> RuleTable {
    // the categories: code points with the same class memberships
    let mut points = BTreeSet::new();
    for class in classes {
        class.breakpoints(&mut points);
    }
    ignore.breakpoints(&mut points);
    let (mut categories, mut masks) = categorize(&points, |c, t| {
        let mask: u64 = classes
            .iter()
            .enumerate()
            .filter(|(_, class)| class.contains(c, t))
            .fold(0, |m, (i, _)| m | (1 << i));
        if mask == 0 && ignore.contains(c, t) {
            None
        } else {
            Some(mask)
        }
    });
    // The JDK's tables always have category 0, the code points in no class;
    // when every code point is in some class, the supplementary table's
    // default entries (see `supplementary_table`) still name it.
    if !masks.contains(&0) {
        masks.push(0);
        categories.count += 1;
    }

    // the NFA of every rule from one start position
    let mut nfa = Nfa::default();
    let start = nfa.add();
    for rule in rules {
        let s = nfa.add();
        nfa.epsilon[start].push(s);
        let mut slash = false;
        let exit = nfa.build(rule, s, &mut slash);
        nfa.flags[exit] |= FLAG_END | if slash { FLAG_LOOKAHEAD } else { 0 };
    }

    // subset construction: state 0 is STOP, state 1 START
    let mut states: Vec<BTreeSet<usize>> = vec![BTreeSet::new()];
    let mut ids: HashMap<BTreeSet<usize>, usize> = HashMap::new();
    let mut first = BTreeSet::from([start]);
    nfa.closure(&mut first);
    ids.insert(first.clone(), START_STATE);
    states.push(first);
    let mut state_table: Vec<u16> = vec![0; categories.count];
    let mut index = START_STATE;
    while index < states.len() {
        let set = states[index].clone();
        for &mask in &masks {
            let mut next = BTreeSet::new();
            for &x in &set {
                if let Some((class, target)) = nfa.edge[x] {
                    if class.is_none_or(|n| mask & (1 << n) != 0) {
                        next.insert(target);
                    }
                }
            }
            let id = if next.is_empty() {
                STOP_STATE
            } else {
                nfa.closure(&mut next);
                *ids.entry(next.clone()).or_insert_with(|| {
                    states.push(next);
                    states.len() - 1
                })
            };
            state_table.push(id as u16);
        }
        index += 1;
    }
    let flags: Vec<u8> = states
        .iter()
        .map(|set| set.iter().fold(0, |f, &x| f | nfa.flags[x]))
        .collect();
    let backwards_state_table = pair_table(&state_table, categories.count, None);
    // the category of the code points in no class (the JDK's category 0)
    let default_category = masks
        .iter()
        .position(|&m| m == 0)
        .expect("INVARIANT: category 0 was added above") as i32;
    let supplementary = supplementary_table(&categories.ranges, default_category);
    RuleTable {
        supplementary,
        default_category,
        bmp: bmp_table(&categories.ranges),
        ranges: categories.ranges,
        num_categories: categories.count,
        state_table,
        backwards_state_table,
        end_states: flags.iter().map(|f| f & FLAG_END != 0).collect(),
        lookahead_states: flags.iter().map(|f| f & FLAG_LOOKAHEAD != 0).collect(),
        category_flags: match dictionary {
            None => Vec::new(),
            Some(n) => masks.iter().map(|m| m & (1 << n) != 0).collect(),
        },
    }
}

/// The automatic part of a backward table, as the JDK's builder derives it
/// from the forward table `state_table` (`num_categories` wide).
///
/// Row 1 starts: it reads category `k` into row `2 + k`, the row of "the
/// character just read (moving backward) is of category `k`". Row `2 + x`
/// continues on category `y`, the character to the left, iff some forward
/// state takes `y` into a state with a transition on `x` (the forward rules
/// can read `y` then `x` without stopping): to row `2 + y`, or, where
/// `rule_start` (the start row of the backward-only rules) has a cell for
/// `y`, to that cell. A pair the forward table never reads across stops
/// (row 0): a sure break between `y` and `x`. Rows a caller appends follow
/// these `2 + num_categories` rows.
fn pair_table(state_table: &[u16], num_categories: usize, rule_start: Option<&[u16]>) -> Vec<u16> {
    let n = num_categories;
    let rows = state_table.len() / n;
    let mut back = vec![0u16; (2 + n) * n];
    for k in 0..n {
        back[n + k] = (2 + k) as u16;
    }
    for column in 0..n {
        for row in 0..rows {
            let next_row = usize::from(state_table[row * n + column]);
            if next_row == 0 {
                continue;
            }
            for next_column in 0..n {
                if state_table[next_row * n + next_column] != 0 {
                    back[(2 + next_column) * n + column] = (2 + column) as u16;
                }
            }
        }
    }
    if let Some(rule_start) = rule_start {
        for row in 1..2 + n {
            for k in 0..n {
                if rule_start[k] != 0 && back[row * n + k] != 0 {
                    back[row * n + k] = rule_start[k];
                }
            }
        }
    }
    back
}

/// U+3005, U+4E00-U+9FA5 and U+F900-U+FA2D, the rules' `<kanji>`.
fn kanji() -> Set {
    Set::Union(vec![
        Set::ch(0x3005),
        Set::Range(0x4E00, 0x9FA5),
        Set::Range(0xF900, 0xFA2D),
    ])
}

/// The word rules of the root locale (`WordBreakRules` of
/// `sun.text.resources.BreakIteratorRules`): a word is a run of letters
/// (with combining marks) that may hold "mid-word" punctuation (dashes,
/// connectors, `'`, `"`, `.`, U+00AD, U+2027) between letters and end in a
/// danda; words and numbers (digits with "mid-number" `'`, `"`, `,`, `.`,
/// U+066B between digits) may alternate, a number may follow a currency
/// sign or `#`/`.` and end in a "post-number" sign (`%`, `&`, `¢`, U+066A,
/// U+2030, U+2031); a run of whitespace may end in CR and a line
/// separator; runs of Katakana, of Hiragana (each with the CJK sound marks)
/// and of Kanji are one word each; a character with its combining marks is
/// one word; any other character is a word alone. Format characters
/// (`[:Cf:]`) are ignored.
pub(crate) fn word_rules() -> &'static RuleTable {
    static TABLE: LazyLock<RuleTable> = LazyLock::new(|| {
        let kata = Set::Union(vec![
            Set::Range(0x30A1, 0x30FA),
            Set::chars(&[0x30FD, 0x30FE]),
        ]);
        let hira = Set::Union(vec![
            Set::Range(0x3041, 0x3094),
            Set::chars(&[0x309D, 0x309E]),
        ]);
        let cjk_diacrit = Set::Union(vec![
            Set::Range(0x3099, 0x309C),
            Set::chars(&[0x30FB, 0x30FC]),
        ]);
        let enclosing = Set::Types(ty(MN) | ty(ME));
        let classes = [
            // 0 <enclosing>
            enclosing.clone(),
            // 1 <danda>
            Set::chars(&[0x0964, 0x0965]),
            // 2 <letter-base>
            Set::Types(LETTER | ty(MC)).minus(Set::Union(vec![
                kanji(),
                kata.clone(),
                hira.clone(),
                cjk_diacrit.clone(),
            ])),
            // 3 <digit-base>
            Set::Types(NUMBER),
            // 4 <mid-word>
            Set::Union(vec![
                Set::Types(ty(PD) | ty(PC)),
                Set::chars(&[0x00AD, 0x2027, '"' as u32, '\'' as u32, '.' as u32]),
            ]),
            // 5 <mid-num>
            Set::chars(&['"' as u32, '\'' as u32, ',' as u32, 0x066B, '.' as u32]),
            // 6 <pre-num>
            Set::Union(vec![
                Set::Types(ty(SC)),
                Set::chars(&['#' as u32, '.' as u32]),
            ])
            .minus(Set::ch(0x00A2)),
            // 7 <post-num>
            Set::chars(&['%' as u32, '&' as u32, 0x00A2, 0x066A, 0x2030, 0x2031]),
            // 8 <ls>
            Set::chars(&['\n' as u32, 0x000C, 0x2028, 0x2029]),
            // 9 <ws-base>
            Set::Union(vec![Set::Types(ty(ZS)), Set::ch('\t' as u32)]),
            // 10 \r
            Set::ch('\r' as u32),
            // 11 [<kata><cjk-diacrit>]
            Set::Union(vec![kata, cjk_diacrit.clone()]),
            // 12 [<hira><cjk-diacrit>]
            Set::Union(vec![hira, cjk_diacrit]),
            // 13 <kanji>
            kanji(),
            // 14 <base>
            Set::All
                .minus(enclosing)
                .minus(Set::Types(ty(CC) | ty(CF) | ty(ZL) | ty(ZP))),
        ];
        let enc = || cls(0);
        let let_ = || seq(vec![cls(2), star(enc())]);
        let dgt = || seq(vec![cls(3), star(enc())]);
        let ws = seq(vec![cls(9), star(enc())]);
        let word = || {
            seq(vec![
                let_(),
                star(let_()),
                star(seq(vec![cls(4), let_(), star(let_())])),
                opt(cls(1)),
            ])
        };
        let number = || {
            seq(vec![
                dgt(),
                star(dgt()),
                star(seq(vec![cls(5), dgt(), star(dgt())])),
            ])
        };
        let tail = || opt(seq(vec![number(), opt(cls(7))]));
        let rules = [
            Re::Any,
            seq(vec![opt(word()), star(seq(vec![number(), word()])), tail()]),
            seq(vec![cls(6), star(seq(vec![number(), word()])), tail()]),
            seq(vec![star(ws), opt(cls(10)), opt(cls(8))]),
            star(cls(11)),
            star(cls(12)),
            star(cls(13)),
            seq(vec![cls(14), enc(), star(enc())]),
        ];
        compile(&classes, &Set::Types(ty(CF)), None, &rules)
    });
    &TABLE
}

/// U+0E01-U+0E2E, U+0E30-U+0E3A, U+0E40-U+0E44 and U+0E47-U+0E4E, the Thai rules'
/// `<dictionary>` characters.
fn thai_dictionary_chars() -> Set {
    Set::Union(vec![
        Set::Range(0x0E01, 0x0E2E),
        Set::Range(0x0E30, 0x0E3A),
        Set::Range(0x0E40, 0x0E44),
        Set::Range(0x0E47, 0x0E4E),
    ])
}

/// The word rules of Thai (`WordBreakRules` of
/// `sun.text.resources.ext.BreakIteratorRules_th`): those of the root
/// locale without the combining-mark handling, letters excluding the Thai
/// `<dictionary>` characters, plus: a run of dictionary characters, which may
/// end in U+0E46 (maiyamok), optionally after U+0E2F (paiyannoi); a run of
/// dictionary characters ending in paiyannoi, the break going before what
/// follows unless that is U+0E25 followed by paiyannoi; and U+0E2F U+0E25
/// U+0E2F. Non-spacing and enclosing marks and format characters other
/// than dictionary characters are ignored. The dictionary
/// (`DictionaryBasedBreakIterator`) then splits each range of dictionary
/// characters this table returns.
pub(crate) fn thai_word_rules() -> &'static RuleTable {
    static TABLE: LazyLock<RuleTable> = LazyLock::new(|| {
        let ignore = Set::Types(ty(MN) | ty(ME) | ty(CF)).minus(thai_dictionary_chars());
        let kata = Set::Range(0x30A1, 0x30FA);
        let hira = Set::Range(0x3041, 0x3094);
        let cjk_diacrit = Set::Range(0x3099, 0x309C);
        let classes = [
            // 0 <dictionary>
            thai_dictionary_chars(),
            // 1 <paiyannoi>
            Set::ch(0x0E2F),
            // 2 <maiyamok>
            Set::ch(0x0E46),
            // 3 <danda>
            Set::chars(&[0x0964, 0x0965]),
            // 4 <let>
            Set::Types(LETTER | ty(MC)).minus(Set::Union(vec![
                kanji(),
                kata.clone(),
                hira.clone(),
                cjk_diacrit.clone(),
                thai_dictionary_chars(),
            ])),
            // 5 <dgt>
            Set::Types(NUMBER),
            // 6 <mid-word>
            Set::Union(vec![
                Set::Types(ty(PD)),
                Set::chars(&[0x00AD, 0x2027, '"' as u32, '\'' as u32, '.' as u32]),
            ]),
            // 7 <mid-num>
            Set::chars(&['"' as u32, '\'' as u32, ',' as u32, 0x066B, '.' as u32]),
            // 8 <pre-num>
            Set::Union(vec![
                Set::Types(ty(SC)),
                Set::chars(&['#' as u32, '.' as u32]),
            ])
            .minus(Set::ch(0x00A2)),
            // 9 <post-num>
            Set::chars(&['%' as u32, '&' as u32, 0x00A2, 0x066A, 0x2030, 0x2031]),
            // 10 <ls>
            Set::chars(&['\n' as u32, 0x000C, 0x2028, 0x2029]),
            // 11 <ws>
            Set::Union(vec![Set::Types(ty(ZS)), Set::ch('\t' as u32)]),
            // 12 \r
            Set::ch('\r' as u32),
            // 13 U+0E25
            Set::ch(0x0E25),
            // 14 [^[U+0E25 <ignore>]]
            Set::All.minus(Set::ch(0x0E25)).minus(ignore.clone()),
            // 15 [^[<paiyannoi><ignore>]]
            Set::All.minus(Set::ch(0x0E2F)).minus(ignore.clone()),
            // 16 [<kata><cjk-diacrit>]
            Set::Union(vec![kata, cjk_diacrit.clone()]),
            // 17 [<hira><cjk-diacrit>]
            Set::Union(vec![hira, cjk_diacrit]),
            // 18 <kanji>
            kanji(),
        ];
        let word = || {
            seq(vec![
                cls(4),
                star(cls(4)),
                star(seq(vec![cls(6), cls(4), star(cls(4))])),
                opt(cls(3)),
            ])
        };
        let number = || {
            seq(vec![
                cls(5),
                star(cls(5)),
                star(seq(vec![cls(7), cls(5), star(cls(5))])),
            ])
        };
        let tail = || opt(seq(vec![number(), opt(cls(9))]));
        let rules = [
            Re::Any,
            seq(vec![opt(word()), star(seq(vec![number(), word()])), tail()]),
            seq(vec![cls(8), star(seq(vec![number(), word()])), tail()]),
            seq(vec![
                cls(0),
                star(cls(0)),
                opt(seq(vec![opt(cls(1)), cls(2)])),
            ]),
            seq(vec![
                cls(0),
                star(cls(0)),
                cls(1),
                Re::Slash,
                Re::Alt(vec![cls(14), seq(vec![cls(13), cls(15)])]),
            ]),
            seq(vec![cls(1), cls(13), cls(1)]),
            seq(vec![star(cls(11)), opt(cls(12)), opt(cls(10))]),
            star(cls(16)),
            star(cls(17)),
            star(cls(18)),
        ];
        compile(&classes, &ignore, Some(0), &rules)
    });
    &TABLE
}

// ------------------------------------------------------ the sentence rules

/// The character categories of the sentence rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum SentenceCategory {
    /// U+2029, the paragraph separator.
    ParagraphSeparator,
    /// `<danda>`, which is also `<sent-start>`.
    Danda,
    /// `<period>`: `.` and U+FF0E.
    Period,
    /// `<term>`: `!`, `?`, U+3002, U+FF01, U+FF1F.
    Term,
    /// `<space>`: TAB, CR, FF, LF, U+2028 and `[:Zs:]`.
    Space,
    /// `"` and `'`, both `<start-punctuation>` and `<end>`.
    Quote,
    /// The rest of `<start-punctuation>`: `[:Ps::Pi:]`.
    Start,
    /// The rest of `<end>`: `[:Pe::Pf:]`.
    End,
    /// `<lc>`: `[:Ll:]`.
    Lowercase,
    /// `<notlc>`: the other letters.
    NotLowercase,
    /// `<digit>`: `[:N:]`.
    Digit,
    /// The rest of `<sent-start>`.
    SentenceStart,
    /// `,`, in no class but `.`.
    Other,
}

/// The states of the sentence table; their numbers are the table's.
mod sentence_state {
    /// The loop of the `.*?` every sentence rule starts with, also the start
    /// state: end state.
    pub(super) const LOOP: usize = 1;
    /// After U+2029: end state, stops.
    pub(super) const PARAGRAPH: usize = 2;
    /// After a danda, and its whitespace: end state.
    pub(super) const DANDA: usize = 3;
    /// After a terminator, and further terminators, periods and ending
    /// punctuation: end state.
    pub(super) const TERM: usize = 4;
    /// After the terminator's whitespace: end state.
    pub(super) const TERM_SPACE: usize = 5;
    /// After a period, and further periods and ending punctuation:
    /// lookahead state.
    pub(super) const PERIOD: usize = 6;
    /// After a quotation mark that follows a period: lookahead state.
    pub(super) const PERIOD_QUOTE: usize = 7;
    /// After one whitespace character following the period: lookahead
    /// state.
    pub(super) const SPACE: usize = 8;
    /// After two or more whitespace characters following the period:
    /// lookahead state.
    pub(super) const SPACES: usize = 9;
    /// After starting punctuation or a sentence-starting character that
    /// follows the period: no flag.
    pub(super) const OPENING: usize = 10;
    /// The same after a quotation mark or after two or more whitespace
    /// characters, a state the builder does not backfill: no flag.
    pub(super) const OPENING_UNFILLED: usize = 11;
    /// A sentence break before the letter just read (end and lookahead
    /// state): stops.
    pub(super) const BREAK: usize = 12;
    /// The number of states, `STOP_STATE` included.
    pub(super) const COUNT: usize = 13;
}

/// The sentence rules (`SentenceBreakRules` of
/// `sun.text.resources.BreakIteratorRules`): a sentence runs to the first of
///
/// * a paragraph separator (included);
/// * a danda and its whitespace;
/// * a terminator (`!`, `?`, U+3002, U+FF01, U+FF1F) followed by any
///   terminators, periods and ending punctuation, whitespace and one
///   paragraph separator;
/// * a period followed by any periods and ending punctuation and whitespace,
///   when what follows starts a sentence: a letter other than a lowercase one
///   after at least one whitespace character, or starting punctuation or a
///   sentence-starting character followed by a letter (the break then goes
///   before the starting punctuation).
///
/// The rules all start with the non-greedy `.*?`, for which the JDK's builder
/// copies the loop state's transitions into the empty cells of the states
/// that follow it ("backfilling"), except into end states and into states
/// it creates by merging two states of the same rule. The table here is the
/// one the builder produces, with two consequences of that construction:
/// after two or more whitespace characters following a period a lowercase
/// letter also starts a sentence (the state for "more whitespace" is
/// created while the builder loops the whitespace class, and joins the
/// next step's states), and after a quotation mark or two or more
/// whitespace characters following a period, a sentence-starting character
/// leads to a state that is not backfilled, which stops on anything but more
/// sentence-starting characters or a letter.
///
/// The backward table holds, besides the pair machine of `pair_table`, the
/// two backward-only (`!`) rules, read right to left: a sentence-starting
/// character (or, for the second rule, also a lowercase letter or a digit),
/// then any starting punctuation, whitespace and ending punctuation, then a
/// period (first rule only) or a terminator, after which the machine stops.
/// The rules' start row is the table's start state: on a sentence-starting
/// character it enters both rules, on a lowercase letter or a digit the
/// second; every other cell of it, and every cell the rules leave empty in
/// their states (but the final one), is the pair machine's, whose own cells
/// on those four categories enter the rules too.
pub(crate) fn sentence_rules() -> &'static RuleTable {
    use sentence_state::*;
    use SentenceCategory as K;

    static TABLE: LazyLock<RuleTable> = LazyLock::new(|| {
        let explicit: &[u32] = &[
            0x2029,
            0x0964,
            0x0965,
            '.' as u32,
            0xFF0E,
            '!' as u32,
            '?' as u32,
            0x3002,
            0xFF01,
            0xFF1F,
            '\t' as u32,
            '\r' as u32,
            0x000C,
            '\n' as u32,
            0x2028,
            '"' as u32,
            '\'' as u32,
            ',' as u32,
        ];
        let points: BTreeSet<u32> = explicit.iter().flat_map(|&c| [c, c + 1]).collect();
        let (categories, keys) = categorize(&points, |c, t| {
            Some(match c {
                0x2029 => K::ParagraphSeparator,
                0x0964 | 0x0965 => K::Danda,
                0x2E | 0xFF0E => K::Period,
                0x21 | 0x3F | 0x3002 | 0xFF01 | 0xFF1F => K::Term,
                0x09 | 0x0D | 0x0C | 0x0A | 0x2028 => K::Space,
                0x22 | 0x27 => K::Quote,
                0x2C => K::Other,
                _ => match t {
                    MN | ME | CF => return None,
                    ZS => K::Space,
                    PS | PI => K::Start,
                    PE | PF => K::End,
                    LL => K::Lowercase,
                    LU | LT | LM | LO => K::NotLowercase,
                    ND | NL | NO => K::Digit,
                    _ => K::SentenceStart,
                },
            })
        });
        // the transition of the `.*?` loop, which every backfilled state
        // takes where its own rules have none
        let from_loop = |k: K| match k {
            K::ParagraphSeparator => PARAGRAPH,
            K::Danda => DANDA,
            K::Period => PERIOD,
            K::Term => TERM,
            _ => LOOP,
        };
        let opening = |k: K| matches!(k, K::Start | K::SentenceStart | K::Danda | K::Quote);
        let letter = |k: K| matches!(k, K::Lowercase | K::NotLowercase);
        let next = |state: usize, k: K| -> usize {
            match state {
                LOOP => from_loop(k),
                PARAGRAPH | BREAK => STOP_STATE,
                DANDA => {
                    if k == K::Space {
                        DANDA
                    } else {
                        STOP_STATE
                    }
                }
                TERM => match k {
                    K::Term | K::Period | K::End | K::Quote => TERM,
                    K::Space => TERM_SPACE,
                    K::ParagraphSeparator => PARAGRAPH,
                    _ => STOP_STATE,
                },
                TERM_SPACE => match k {
                    K::Space => TERM_SPACE,
                    K::ParagraphSeparator => PARAGRAPH,
                    _ => STOP_STATE,
                },
                PERIOD => match k {
                    K::Period | K::End => PERIOD,
                    K::Quote => PERIOD_QUOTE,
                    K::Space => SPACE,
                    k if opening(k) => OPENING,
                    k => from_loop(k),
                },
                PERIOD_QUOTE => match k {
                    K::Period | K::End => PERIOD,
                    K::Quote => PERIOD_QUOTE,
                    K::Space => SPACE,
                    k if opening(k) => OPENING_UNFILLED,
                    k if letter(k) => BREAK,
                    k => from_loop(k),
                },
                SPACE => match k {
                    K::Space => SPACES,
                    K::NotLowercase => BREAK,
                    k if opening(k) => OPENING,
                    k => from_loop(k),
                },
                SPACES => match k {
                    K::Space => SPACES,
                    k if letter(k) => BREAK,
                    k if opening(k) => OPENING_UNFILLED,
                    k => from_loop(k),
                },
                OPENING => match k {
                    k if opening(k) => OPENING,
                    k if letter(k) => BREAK,
                    k => from_loop(k),
                },
                OPENING_UNFILLED => match k {
                    k if opening(k) => OPENING,
                    k if letter(k) => BREAK,
                    _ => STOP_STATE,
                },
                _ => unreachable!("INVARIANT: a sentence state"),
            }
        };
        let mut state_table = vec![0u16; COUNT * categories.count];
        for state in 1..COUNT {
            for (category, &k) in keys.iter().enumerate() {
                state_table[state * categories.count + category] = next(state, k) as u16;
            }
        }
        let end = [LOOP, PARAGRAPH, DANDA, TERM, TERM_SPACE, BREAK];
        let lookahead = [PERIOD, PERIOD_QUOTE, SPACE, SPACES, BREAK];

        // the backward table: the pair rows, then the rows of the `!` rules
        let n = categories.count;
        // both rules after their first character (sentence-starting) and any
        // starting punctuation; after whitespace; after ending punctuation
        let both_first = 2 + n;
        let both_space = both_first + 1;
        let both_end = both_first + 2;
        // the second rule alone (first character a lowercase letter or a
        // digit), likewise
        let second_first = both_first + 3;
        let second_space = both_first + 4;
        let second_end = both_first + 5;
        // a rule matched: stops
        let matched = both_first + 6;
        let rows = both_first + 7;
        let rule_start: Vec<u16> = keys
            .iter()
            .map(|&k| match k {
                K::SentenceStart | K::Danda => both_first as u16,
                K::Lowercase | K::Digit => second_first as u16,
                _ => 0,
            })
            .collect();
        let mut backwards_state_table = pair_table(&state_table, n, Some(&rule_start));
        backwards_state_table.resize(rows * n, 0);
        for (category, &k) in keys.iter().enumerate() {
            // the start row: the rules' start, the pair machine's start
            // where the rules have no cell
            if rule_start[category] != 0 {
                backwards_state_table[n + category] = rule_start[category];
            }
            let fallback = backwards_state_table[n + category];
            let both = [both_first, both_space, both_end];
            let second = [second_first, second_space, second_end];
            let target = |state: usize| -> u16 {
                let (first, space, end, ends_on_period) = if both.contains(&state) {
                    (both_first, both_space, both_end, true)
                } else {
                    (second_first, second_space, second_end, false)
                };
                let target = match k {
                    K::Start if state == first => first,
                    K::Quote if state == first => first,
                    K::Quote => end,
                    K::Space if state != end => space,
                    K::End => end,
                    K::Term => matched,
                    K::Period if ends_on_period => matched,
                    _ => return fallback,
                };
                target as u16
            };
            for state in both.into_iter().chain(second) {
                backwards_state_table[state * n + category] = target(state);
            }
        }

        // `,` is in no class but `.`, as a code point the rules do not
        // mention (the JDK's category 0)
        let default_category = keys
            .iter()
            .position(|&k| k == K::Other)
            .expect("INVARIANT: `,` has a category") as i32;
        let supplementary = supplementary_table(&categories.ranges, default_category);
        RuleTable {
            supplementary,
            default_category,
            bmp: bmp_table(&categories.ranges),
            ranges: categories.ranges,
            num_categories: categories.count,
            state_table,
            backwards_state_table,
            end_states: (0..COUNT).map(|s| end.contains(&s)).collect(),
            lookahead_states: (0..COUNT).map(|s| lookahead.contains(&s)).collect(),
            category_flags: Vec::new(),
        }
    });
    &TABLE
}

/// A code point of each of the categories 1, 2, ... of JDK 25's root line
/// table, in the JDK's numbering (see [`compile_jdk`]): read from
/// `BreakIterator.getLineInstance(Locale.ROOT)` with
/// `RuleBasedBreakIterator.lookupCategory`, the first code point of each.
const LINE_CATEGORY_ORDER: [u32; 17] = [
    0x0030, 0x0028, 0x0022, 0x000D, 0x002D, 0x002C, 0x0300, 0x0023, 0x3099, 0x0024, 0x0029, 0x0000,
    0x00A0, 0x0003, 0x0021, 0x005B, 0x3042,
];

/// A code point of each of the categories 1, 2, ... of JDK 25's Thai line
/// table, as [`LINE_CATEGORY_ORDER`] for
/// `BreakIterator.getLineInstance(th)`.
const THAI_LINE_CATEGORY_ORDER: [u32; 24] = [
    0x0000, 0x3042, 0x00A0, 0x0E2F, 0x0022, 0xFEFF, 0x002C, 0x0030, 0x0072, 0x002D, 0x006B, 0x00AD,
    0x000D, 0x0065, 0x0E01, 0x0E25, 0x0062, 0x0003, 0x0061, 0x0023, 0x0021, 0x0964, 0x0024, 0x3099,
];

/// The small Kana, CJK sound marks and other characters both line rule sets
/// put in `<post-word>` besides general categories `Pe` and `Pf`: `"`, `!`,
/// `%`, `.`, `,`, `:`, `;`, `?`, U+00A2, U+00B0, U+066A, U+2030-U+2034,
/// U+2103, U+2105, U+2109, U+3001, U+3002, U+3005, the small Hiragana and
/// Katakana, U+3099-U+309E, U+30FC-U+30FE, U+FF01, U+FF0E and U+FF1F.
fn line_post_word_common() -> Set {
    Set::Union(vec![
        Set::Types(ty(PE) | ty(PF)),
        Set::chars(&[
            '"' as u32, '!' as u32, '%' as u32, '.' as u32, ',' as u32, ':' as u32, ';' as u32,
            '?' as u32, 0x00A2, 0x00B0, 0x066A, 0x2103, 0x2105, 0x2109, 0x3001, 0x3002, 0x3005,
            0x3041, 0x3043, 0x3045, 0x3047, 0x3049, 0x3063, 0x3083, 0x3085, 0x3087, 0x308E, 0x30A1,
            0x30A3, 0x30A5, 0x30A7, 0x30A9, 0x30C3, 0x30E3, 0x30E5, 0x30E7, 0x30EE, 0x30F5, 0x30F6,
            0xFF01, 0xFF0E, 0xFF1F,
        ]),
        Set::Range(0x2030, 0x2034),
        Set::Range(0x3099, 0x309E),
        Set::Range(0x30FC, 0x30FE),
    ])
}

/// The line rules of the root locale (`LineBreakRules` of
/// `sun.text.resources.BreakIteratorRules`), which
/// `BreakIterator.getLineInstance` runs for every locale but Thai: a line
/// may break between "words", a word being an optional run of prefix
/// characters (currency signs but U+00A2, `Ps`, `Pi`, `"`, `'`), a core (a run of
/// characters the rules do not otherwise mention, one CJK ideograph, Kana or
/// Hangul syllable, or a number: prefixes and dashes, then digits with
/// single `.` or `,` between them) and a suffix (a run of dashes or a run of
/// `<post-word>` characters). Words joined by "glue" (U+00A0, U+0F0C,
/// U+2007, U+2011, U+202F, U+FEFF, which spaces may surround) or by `$` or
/// `'` stay together; a word keeps its trailing spaces, then enclosing
/// marks, a `()` pair with `<post-word>` characters, enclosing marks, a CR
/// and one hard break character (U+0003, tab, LF, FF, U+2028, U+2029); CR
/// LF is one piece. Format characters and the control characters other
/// than the hard breaks and CR are ignored where no class claims them (the
/// controls being spaces, a space they are).
pub(crate) fn line_rules() -> &'static RuleTable {
    static TABLE: LazyLock<RuleTable> = LazyLock::new(|| {
        let brk = Set::chars(&[0x0003, '\t' as u32, '\n' as u32, 0x000C, 0x2028, 0x2029]);
        let cr = Set::ch('\r' as u32);
        let ignore = Set::Union(vec![
            Set::Types(ty(CF)),
            Set::Types(ty(CC)).minus(Set::Union(vec![brk.clone(), cr.clone()])),
        ]);
        let danda = Set::chars(&[0x0964, 0x0965]);
        let glue = Set::chars(&[0x00A0, 0x0F0C, 0x2007, 0x2011, 0x202F, 0xFEFF]);
        let space = Set::Types(ty(ZS) | ty(CC)).minus(Set::Union(vec![
            glue.clone(),
            brk.clone(),
            cr.clone(),
        ]));
        let dash = Set::Union(vec![Set::Types(ty(PD)), Set::ch(0x00AD)]).minus(glue.clone());
        // `[:Sc::Ps::Pi:^[\u00a2]\"\']`: a `^` takes away the one item
        // that follows it, the quotes are added
        let pre_word = Set::Union(vec![
            Set::Types(ty(SC) | ty(PS) | ty(PI)).minus(Set::ch(0x00A2)),
            Set::chars(&['"' as u32, '\'' as u32]),
        ]);
        let post_word = Set::Union(vec![
            line_post_word_common(),
            Set::chars(&[0xFF05, 0xFF0C, 0xFF1A, 0xFF1B]),
        ]);
        let kanji = Set::Union(vec![
            Set::Range(0x4E00, 0x9FA5),
            Set::Range(0xAC00, 0xD7A3),
            Set::Range(0xF900, 0xFA2D),
            Set::Range(0xFA30, 0xFA6A),
            Set::Range(0x3041, 0x3094),
            Set::Range(0x30A1, 0x30FA),
        ])
        .minus(Set::Union(vec![post_word.clone(), ignore.clone()]));
        let mid_num = Set::chars(&['.' as u32, ',' as u32]);
        let char_ = Set::All.minus(Set::Union(vec![
            brk.clone(),
            space.clone(),
            dash.clone(),
            kanji.clone(),
            glue.clone(),
            ignore.clone(),
            pre_word.clone(),
            post_word.clone(),
            mid_num.clone(),
            cr.clone(),
            danda.clone(),
        ]));
        let pre_word_dash = Set::Union(vec![pre_word.clone(), dash.clone()]);
        let classes = [
            brk,
            Set::Types(ty(MN) | ty(ME)),
            glue,
            space,
            dash,
            pre_word,
            post_word,
            kanji,
            Set::Types(ty(ND) | ty(NO)),
            mid_num,
            char_,
            Set::ch('(' as u32),
            Set::ch(')' as u32),
            Set::chars(&['$' as u32, '\'' as u32]),
            cr,
            pre_word_dash,
        ];
        let names = [
            "<break>",
            "<enclosing>",
            "<glue>",
            "<space>",
            "<dash>",
            "<pre-word>",
            "<post-word>",
            "<kanji>",
            "<digit>",
            "<mid-num>",
            "<char>",
            "<hack1>",
            "<hack2>",
            "<hack3>",
            "\r",
            // `[<pre-word><dash>]`
            "<pre-word-dash>",
        ];
        let macros = [
            (
                "<number>",
                "(<pre-word-dash>*<digit><digit>*(<mid-num><digit><digit>*)*)",
            ),
            ("<word-core>", "(<char>*|<kanji>|<number>)"),
            ("<word-suffix>", "((<dash><dash>*|<post-word>*))"),
            ("<word>", "(<pre-word>*<word-core><word-suffix>)"),
        ];
        let rules = [
            "<word>(((<space>*<glue><glue>*{<space>})|<hack3>)<word>)*<space>*{<enclosing>*}\
             {<hack1><hack2><post-word>*}{<enclosing>*}{\r}{<break>}",
            "\r<break>",
        ];
        compile_jdk(
            &classes,
            &names,
            &ignore,
            None,
            &macros,
            &rules,
            &LINE_CATEGORY_ORDER,
        )
    });
    &TABLE
}

/// The line rules of Thai (`LineBreakRules` of
/// `sun.text.resources.ext.BreakIteratorRules_th`), run with the Thai
/// dictionary: those of the root locale with the Thai `<dictionary>`
/// characters as a word core (a run, which the dictionary then splits) and
/// U+0E2F U+0E25 U+0E2F as another, U+0E46 (maiyamok) a `<post-word>`
/// character, Mn, Me and Cf (but dictionary characters) ignored, no
/// enclosing-mark, `()`, `$` or `'` handling, spaces part of the word
/// suffix and words joined only by glue (`<nbsp>`); a line keeps a CR and a
/// hard break, or U+0E2F, CR and, as the JDK's rule spells it, the optional
/// letters `break`, or U+0E2F and a hard break; U+0E2F ending a line goes
/// with it unless followed by U+0E25 U+0E2F (the break then goes before
/// U+0E2F).
pub(crate) fn thai_line_rules() -> &'static RuleTable {
    static TABLE: LazyLock<RuleTable> = LazyLock::new(|| {
        let dictionary = thai_dictionary_chars();
        let ignore = Set::Types(ty(MN) | ty(ME) | ty(CF)).minus(dictionary.clone());
        let danda = Set::chars(&[0x0964, 0x0965]);
        let brk = Set::chars(&[0x0003, '\t' as u32, '\n' as u32, 0x000C, 0x2028, 0x2029]);
        let cr = Set::ch('\r' as u32);
        let nbsp = Set::chars(&[0x00A0, 0x0F0C, 0x2007, 0x2011, 0x202F, 0xFEFF]);
        let space = Set::Types(ty(ZS) | ty(CC)).minus(Set::Union(vec![
            nbsp.clone(),
            brk.clone(),
            cr.clone(),
        ]));
        let dash = Set::Union(vec![Set::Types(ty(PD)), Set::ch(0x00AD)]).minus(nbsp.clone());
        let paiyannoi = Set::ch(0x0E2F);
        let maiyamok = Set::ch(0x0E46);
        // `[:Sc::Ps::Pi:^\u00a2\"]`: a `^` takes away the one item that
        // follows it, `"` is added
        let pre_word = Set::Union(vec![
            Set::Types(ty(SC) | ty(PS) | ty(PI)).minus(Set::ch(0x00A2)),
            Set::ch('"' as u32),
        ]);
        let post_word = Set::Union(vec![line_post_word_common(), maiyamok.clone()]);
        let kanji = Set::Union(vec![
            Set::Range(0x4E00, 0x9FA5),
            Set::Range(0xF900, 0xFA2D),
            Set::Range(0x3041, 0x3094),
            Set::Range(0x30A1, 0x30FA),
        ])
        .minus(Set::Union(vec![post_word.clone(), ignore.clone()]));
        let mid_num = Set::chars(&['.' as u32, ',' as u32]);
        let char_ = Set::All.minus(Set::Union(vec![
            brk.clone(),
            space.clone(),
            dash.clone(),
            kanji.clone(),
            nbsp.clone(),
            ignore.clone(),
            pre_word.clone(),
            post_word.clone(),
            mid_num.clone(),
            cr.clone(),
            danda.clone(),
            dictionary.clone(),
            paiyannoi.clone(),
            maiyamok,
        ]));
        let pre_word_dash = Set::Union(vec![pre_word.clone(), dash.clone()]);
        let classes = [
            dictionary,
            brk,
            nbsp,
            space,
            dash,
            paiyannoi,
            pre_word,
            post_word,
            kanji,
            Set::Types(ty(ND) | ty(NO)),
            mid_num,
            char_,
            cr,
            Set::ch(0x0E25),
            Set::All.minus(Set::ch(0x0E25)).minus(ignore.clone()),
            Set::All.minus(Set::ch(0x0E2F)).minus(ignore.clone()),
            Set::ch('b' as u32),
            Set::ch('r' as u32),
            Set::ch('e' as u32),
            Set::ch('a' as u32),
            Set::ch('k' as u32),
            pre_word_dash,
        ];
        let names = [
            "<dictionary>",
            "<break>",
            "<nbsp>",
            "<space>",
            "<dash>",
            "<paiyannoi>",
            "<pre-word>",
            "<post-word>",
            "<kanji>",
            "<digit>",
            "<mid-num>",
            "<char>",
            "\r",
            "\u{e25}",
            // `[^[\u0e25<ignore>]]`
            "<not-0e25>",
            // `[^[<paiyannoi><ignore>]]`
            "<not-paiyannoi>",
            // the letters of the rule's literal `{break}`
            "b",
            "r",
            "e",
            "a",
            "k",
            // `[<pre-word><dash>]`
            "<pre-word-dash>",
        ];
        let macros = [
            ("<thai-etc>", "(<paiyannoi>\u{e25}<paiyannoi>)"),
            (
                "<number>",
                "(<pre-word-dash>*<digit><digit>*(<mid-num><digit><digit>*)*)",
            ),
            (
                "<word-core>",
                "(<char>*|<kanji>|<number>|<dictionary><dictionary>*|<thai-etc>)",
            ),
            ("<word-suffix>", "((<dash><dash>*|<post-word>*)<space>*)"),
            ("<word>", "(<pre-word>*<word-core><word-suffix>)"),
        ];
        let rules = [
            "<word>(<nbsp><nbsp>*<word>)*{({\r}{<break>}|<paiyannoi>\r{break}|<paiyannoi><break>)}",
            "<word>(<nbsp><nbsp>*<word>)*<paiyannoi>/(<not-0e25>|\u{e25}<not-paiyannoi>)",
        ];
        compile_jdk(
            &classes,
            &names,
            &ignore,
            Some(0),
            &macros,
            &rules,
            &THAI_LINE_CATEGORY_ORDER,
        )
    });
    &TABLE
}

/// A token of a rule as the JDK's builder reads it, after the variable
/// substitutions: a character class (a `[]` expression or a literal
/// character, by its index in the rule set's classes) or an operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tok {
    Class(usize),
    Open,
    Bar,
    Close,
    Star,
    LBrace,
    RBrace,
    Slash,
}

/// Tokenizes `rule` after expanding `macros` (`(name, text)`, in definition
/// order: the JDK substitutes a definition into the text that follows it,
/// so expanding the last first is the same): `<name>` and every other
/// character but the operators `(`, `|`, `)`, `*`, `{`, `}` and `/` are
/// classes, looked up in `classes` by name (a literal character by itself).
fn tokenize(rule: &str, macros: &[(&str, &str)], classes: &[&str]) -> Vec<Tok> {
    let mut text = rule.to_string();
    for (name, body) in macros.iter().rev() {
        text = text.replace(name, body);
    }
    let class = |name: &str| {
        Tok::Class(
            classes
                .iter()
                .position(|c| *c == name)
                .unwrap_or_else(|| panic!("INVARIANT: class {name:?} is defined")),
        )
    };
    let mut tokens = Vec::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        tokens.push(match c {
            '(' => Tok::Open,
            '|' => Tok::Bar,
            ')' => Tok::Close,
            '*' => Tok::Star,
            '{' => Tok::LBrace,
            '}' => Tok::RBrace,
            '/' => Tok::Slash,
            '<' => {
                let mut name = String::from('<');
                for d in chars.by_ref() {
                    name.push(d);
                    if d == '>' {
                        break;
                    }
                }
                class(&name)
            }
            other => class(&other.to_string()),
        });
    }
    tokens
}

/// `RuleBasedBreakIteratorBuilder.END_STATE_FLAG`.
const JDK_END_STATE_FLAG: u16 = 0x8000;
/// `RuleBasedBreakIteratorBuilder.DONT_LOOP_FLAG`.
const JDK_DONT_LOOP_FLAG: u16 = 0x4000;
/// `RuleBasedBreakIteratorBuilder.LOOKAHEAD_STATE_FLAG`.
const JDK_LOOKAHEAD_STATE_FLAG: u16 = 0x2000;
/// `RuleBasedBreakIteratorBuilder.ALL_FLAGS`.
const JDK_ALL_FLAGS: u16 = JDK_END_STATE_FLAG | JDK_DONT_LOOP_FLAG | JDK_LOOKAHEAD_STATE_FLAG;

/// The rows a state-table update applies to: the decision point list
/// itself (which the update may extend while it runs), or a list of its own.
#[derive(Clone, Copy)]
enum Rows<'a> {
    DecisionPoints,
    List(&'a [usize]),
}

/// The forward state-table construction of the JDK's
/// `RuleBasedBreakIteratorBuilder` (`parseRule`, `updateStateTable`,
/// `mergeStates`, `searchMergeList`), written from the JDK 25 build tool's
/// source read as specification, for rules without `*?` and `.`.
///
/// The builder is not a regular-expression compiler: it lays each rule out
/// as a line of states, makes the table deterministic as it goes by merging
/// colliding states, and makes a `(...)*` group repeat by copying the
/// transitions its entry states had when the `)` is read into its exit
/// states. Where a group can be entered through an empty alternative the
/// copied transitions miss what follows the empty path, so the line tables,
/// whose `<word>` matches the empty string, differ from what their rules
/// mean as regular expressions; this reproduces the JDK's tables.
struct JdkTableBuilder {
    /// `numCategories`.
    n: usize,
    /// `tempStateTable`: each row has `n` cells and a flag cell.
    table: Vec<Vec<u16>>,
    /// `decisionPointList`.
    dpl: Vec<usize>,
    /// `decisionPointStack`.
    stack: Vec<Vec<usize>>,
    /// `mergeList`: `(old row, new row, combined row)`.
    merge_list: Vec<[usize; 3]>,
}

impl JdkTableBuilder {
    fn new(n: usize) -> Self {
        Self {
            n,
            // state 0 means "stop", state 1 starts
            table: vec![vec![0; n + 1], vec![0; n + 1]],
            dpl: Vec::new(),
            stack: Vec::new(),
            merge_list: Vec::new(),
        }
    }

    /// `parseRule(rule, true)`; `categories[k]` are the categories of
    /// class `k`.
    fn parse_rule(&mut self, rule: &[Tok], categories: &[Vec<usize>]) {
        let n = self.n;
        let mut p = 0;
        let mut current_state = 1;
        let mut last_state = current_state;
        let mut pending: &[usize] = &[];
        self.stack = Vec::new();
        self.dpl = vec![current_state];
        let mut saw_early_break = false;
        current_state = self.table.len() - 1;
        let star_follows = |p: usize| rule.get(p + 1) == Some(&Tok::Star);
        while p < rule.len() {
            match rule[p] {
                Tok::Class(k) => {
                    pending = &categories[k];
                    if !pending.is_empty() {
                        if star_follows(p) {
                            self.stack.push(self.dpl.clone());
                        }
                        let new_state = self.table.len();
                        let mut state = vec![0; n + 1];
                        if saw_early_break {
                            state[n] = JDK_DONT_LOOP_FLAG;
                        }
                        self.table.push(state);
                        self.update_state_table(Rows::DecisionPoints, pending, new_state);
                        self.dpl.clear();
                        last_state = current_state;
                        loop {
                            current_state += 1;
                            self.dpl.push(current_state);
                            if current_state + 1 >= self.table.len() {
                                break;
                            }
                        }
                    }
                }
                Tok::LBrace => self.stack.push(self.dpl.clone()),
                tok @ (Tok::RBrace | Tok::Star) => {
                    if tok == Tok::Star {
                        let mut i = last_state + 1;
                        while i < self.table.len() {
                            self.update_state_table(Rows::List(&[i]), pending, last_state + 1);
                            i += 1;
                        }
                    }
                    let mut temp = self.stack.pop().expect("INVARIANT: balanced rule");
                    temp.append(&mut self.dpl);
                    self.dpl = temp;
                }
                Tok::Open => {
                    self.table.push(vec![0; n + 1]);
                    last_state = current_state;
                    current_state += 1;
                    self.dpl.insert(0, current_state);
                    self.stack.push(self.dpl.clone());
                    self.stack.push(Vec::new());
                }
                Tok::Bar => {
                    let mut one_down = self.stack.pop().expect("INVARIANT: inside ()");
                    let two_down = self.stack.last().expect("INVARIANT: inside ()").clone();
                    one_down.extend_from_slice(&self.dpl);
                    self.stack.push(one_down);
                    self.dpl = two_down;
                }
                Tok::Close => {
                    let mut exit_points = self.stack.pop().expect("INVARIANT: inside ()");
                    exit_points.append(&mut self.dpl);
                    self.dpl = exit_points;
                    if !star_follows(p) {
                        self.stack.pop();
                    } else {
                        let exit_points = self.dpl.clone();
                        let mut temp = self.stack.pop().expect("INVARIANT: inside ()");
                        let temp_state_num = temp[0];
                        let temp_state = self.table[temp_state_num].clone();
                        temp.extend_from_slice(&self.dpl);
                        self.dpl = temp;
                        for (i, &target) in temp_state.iter().enumerate() {
                            if usize::from(target) > temp_state_num {
                                self.update_state_table(
                                    Rows::List(&exit_points),
                                    &[i],
                                    usize::from(target),
                                );
                            }
                        }
                        last_state = current_state;
                        current_state = self.table.len() - 1;
                        p += 1;
                    }
                }
                Tok::Slash => {
                    saw_early_break = true;
                    for &row in &self.dpl {
                        self.table[row][n] |= JDK_LOOKAHEAD_STATE_FLAG;
                    }
                }
            }
            p += 1;
        }
        let flags = JDK_END_STATE_FLAG
            | if saw_early_break {
                JDK_LOOKAHEAD_STATE_FLAG
            } else {
                0
            };
        for &row in &self.dpl {
            self.table[row][n] |= flags;
        }
    }

    /// `updateStateTable(rows, pendingChars, newValue)`.
    fn update_state_table(&mut self, rows: Rows<'_>, pending: &[usize], new_value: usize) {
        let mut new_values = vec![0u16; self.n + 1];
        for &c in pending {
            new_values[c] = new_value as u16;
        }
        match rows {
            Rows::DecisionPoints => {
                // the list grows as merged states join it
                let mut i = 0;
                while i < self.dpl.len() {
                    let row = self.dpl[i];
                    self.merge_states(row, &new_values, rows);
                    i += 1;
                }
            }
            Rows::List(list) => {
                for &row in list {
                    self.merge_states(row, &new_values, rows);
                }
            }
        }
    }

    /// `mergeStates(rowNum, newValues, rowsBeingUpdated)` (no looping
    /// states: the rules have no `*?`).
    fn merge_states(&mut self, row_num: usize, new_values: &[u16], rows: Rows<'_>) {
        let n = self.n;
        for i in 0..=n {
            let old = self.table[row_num][i];
            let new = new_values[i];
            if old == new {
                continue;
            } else if old == 0 {
                self.table[row_num][i] = new;
            } else if i == n {
                self.table[row_num][i] = (new & JDK_ALL_FLAGS) | old;
            } else if new != 0 {
                let (old_row, new_row) = (usize::from(old), usize::from(new));
                let combined = self.search_merge_list(old_row, new_row);
                if combined != 0 {
                    self.table[row_num][i] = combined as u16;
                    continue;
                }
                let combined = self.table.len();
                self.merge_list.push([old_row, new_row, combined]);
                let copy = self.table[old_row].clone();
                self.table.push(copy);
                self.table[row_num][i] = combined as u16;
                let has = |list: &[usize]| list.contains(&old_row) || list.contains(&new_row);
                if has(&self.dpl) && !self.dpl.contains(&combined) {
                    self.dpl.push(combined);
                }
                let (in_rows, combined_in_rows) = match rows {
                    Rows::DecisionPoints => (has(&self.dpl), self.dpl.contains(&combined)),
                    Rows::List(list) => (has(list), list.contains(&combined)),
                };
                if in_rows && !combined_in_rows {
                    self.dpl.push(combined);
                }
                for dpl in &mut self.stack {
                    if has(dpl) && !dpl.contains(&combined) {
                        dpl.push(combined);
                    }
                }
                let values = self.table[new_row].clone();
                self.merge_states(combined, &values, rows);
            }
        }
    }

    /// `searchMergeList(a, b)`.
    fn search_merge_list(&self, a: usize, b: usize) -> usize {
        for &[x, y, combined] in &self.merge_list {
            if (x == a && y == b) || (x == b && y == a) {
                return combined;
            }
            if combined == a && (x == b || y == b) {
                return combined;
            }
            if combined == b && (x == a || y == a) {
                return combined;
            }
        }
        0
    }
}

/// Compiles a rule set as the JDK's builder does (see [`JdkTableBuilder`]):
/// `classes` are its `[]` expressions and literal characters, `names` their
/// names in `rules` (after `macros`), `ignore` and `dictionary` as for
/// [`compile`], and `order` a code point of each of the JDK's categories 1,
/// 2, ... in turn.
///
/// The JDK numbers its categories in the order its builder's `Hashtable`
/// of rule expressions enumerates them, and the numbering shapes the table:
/// `mergeStates` walks the columns in order, which numbers the merged
/// states, and a repeating group only copies transitions to states numbered
/// after its entry state. `order` was read from JDK 25's compiled tables
/// (`RuleBasedBreakIterator.lookupCategory` of each code point; see
/// [`line_rules`]). The table keeps the states reachable from the start state;
/// the JDK's builder also merges equivalent states, which changes no
/// boundary (nor the backward table, derived from which category pairs
/// some state reads in sequence).
fn compile_jdk(
    classes: &[Set],
    names: &[&str],
    ignore: &Set,
    dictionary: Option<usize>,
    macros: &[(&str, &str)],
    rules: &[&str],
    order: &[u32],
) -> RuleTable {
    let mut points = BTreeSet::new();
    for class in classes {
        class.breakpoints(&mut points);
    }
    ignore.breakpoints(&mut points);
    let (mut categories, mut masks) = categorize(&points, |c, t| {
        let mask: u64 = classes
            .iter()
            .enumerate()
            .filter(|(_, class)| class.contains(c, t))
            .fold(0, |m, (i, _)| m | (1 << i));
        if mask == 0 && ignore.contains(c, t) {
            None
        } else {
            Some(mask)
        }
    });
    if !masks.contains(&0) {
        masks.push(0);
        categories.count += 1;
    }
    let n = categories.count;
    let class_categories: Vec<Vec<usize>> = (0..classes.len())
        .map(|k| (0..n).filter(|&cat| masks[cat] & (1 << k) != 0).collect())
        .collect();

    // the JDK's numbering of the categories: its column 0 is the code points
    // in no class, the others are named by their representatives
    assert_eq!(
        order.len() + 1,
        n,
        "INVARIANT: one representative per JDK category"
    );
    let mut perm = vec![usize::MAX; n];
    perm[masks
        .iter()
        .position(|&m| m == 0)
        .expect("INVARIANT: category 0 was added above")] = 0;
    for (k, &rep) in order.iter().enumerate() {
        let at = categories
            .ranges
            .partition_point(|&(first, _)| first <= rep)
            - 1;
        let category = categories.ranges[at].1;
        assert!(
            category >= 0 && perm[category as usize] == usize::MAX,
            "INVARIANT: U+{rep:04X} names a category of its own"
        );
        perm[category as usize] = k + 1;
    }
    let class_categories: Vec<Vec<usize>> = class_categories
        .iter()
        .map(|cats| {
            let mut jdk: Vec<usize> = cats.iter().map(|&c| perm[c]).collect();
            jdk.sort_unstable();
            jdk
        })
        .collect();
    let mut builder = JdkTableBuilder::new(n);
    for rule in rules {
        let tokens = tokenize(rule, macros, names);
        builder.parse_rule(&tokens, &class_categories);
    }
    // back to this module's numbering
    for row in &mut builder.table {
        let jdk = row.clone();
        for (c, cell) in row[..n].iter_mut().enumerate() {
            *cell = jdk[perm[c]];
        }
    }

    // the states reachable from the start state, in their order
    let table = builder.table;
    let mut reachable = vec![false; table.len()];
    reachable[STOP_STATE] = true;
    reachable[START_STATE] = true;
    let mut stack = vec![START_STATE];
    while let Some(row) = stack.pop() {
        for &target in &table[row][..n] {
            let target = usize::from(target);
            if target != 0 && !reachable[target] {
                reachable[target] = true;
                stack.push(target);
            }
        }
    }
    let mut number = vec![0usize; table.len()];
    let mut count = 0;
    for (row, &keep) in reachable.iter().enumerate() {
        if keep {
            number[row] = count;
            count += 1;
        }
    }
    let mut state_table = Vec::with_capacity(count * n);
    let mut end_states = Vec::with_capacity(count);
    let mut lookahead_states = Vec::with_capacity(count);
    for (row, cells) in table.iter().enumerate() {
        if !reachable[row] {
            continue;
        }
        state_table.extend(cells[..n].iter().map(|&t| number[usize::from(t)] as u16));
        end_states.push(cells[n] & JDK_END_STATE_FLAG != 0);
        lookahead_states.push(cells[n] & JDK_LOOKAHEAD_STATE_FLAG != 0);
    }
    let backwards_state_table = pair_table(&state_table, n, None);
    let default_category = masks
        .iter()
        .position(|&m| m == 0)
        .expect("INVARIANT: category 0 was added above") as i32;
    let supplementary = supplementary_table(&categories.ranges, default_category);
    RuleTable {
        supplementary,
        default_category,
        bmp: bmp_table(&categories.ranges),
        ranges: categories.ranges,
        num_categories: n,
        state_table,
        backwards_state_table,
        end_states,
        lookahead_states,
        category_flags: match dictionary {
            None => Vec::new(),
            Some(d) => masks.iter().map(|m| m & (1 << d) != 0).collect(),
        },
    }
}
