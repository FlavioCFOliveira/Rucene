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

//! The state tables of JDK 25's rule-based break iterators: the word
//! iterator of the root locale, the word iterator of Thai (before its
//! dictionary splits Thai runs) and the sentence iterator, forward and
//! backward.
//!
//! JDK 25 compiles, when the JDK is built, the rule sets of
//! `sun.text.resources.BreakIteratorRules` (`WordBreakRules`,
//! `SentenceBreakRules`) and `sun.text.resources.ext.BreakIteratorRules_th`
//! (`WordBreakRules`) into state tables over character categories, which
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
//! * the two word rule sets (no `*?`) are written here as regular expressions
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
//! Unicode 16.0, the data the JDK's build reads).
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

/// The segments of the code space over which the general category is
/// constant: `(first code point, category)` in order, unassigned gaps being
/// category 0, as `Character.getType` answers.
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
}

impl RuleTable {
    /// `lookupCategory(int)`: the category of code point `c`, or
    /// [`IGNORE`].
    pub(crate) fn lookup_category(&self, c: i32) -> i32 {
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
    let (categories, masks) = categorize(&points, |c, t| {
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
    RuleTable {
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

        RuleTable {
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
