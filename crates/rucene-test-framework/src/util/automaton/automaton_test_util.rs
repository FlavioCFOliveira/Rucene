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

//! Port of `org.apache.lucene.tests.util.automaton.AutomatonTestUtil`:
//! utilities for testing automata.
//!
//! Capable of generating random regular expressions and automata, and also
//! provides a number of very basic unoptimized implementations (`*Simple`,
//! `*Slow`) for testing. The original brics implementations below are from
//! dk.brics.automaton, Copyright (c) 2001-2009 Anders Moeller, BSD license
//! (redistribution in source form retains this notice; see Lucene's
//! `AutomatonTestUtil.java` for the full text).

use std::collections::{HashMap, HashSet, VecDeque};

use randomizedtesting::jdk::random::Random;
use rucene::util::automaton::{
    Automaton, AutomatonBuilder, Operations, RegExp, Transition, TransitionAccessor,
    MAX_CODE_POINT, MIN_CODE_POINT,
};
use rucene::util::unicode_util::{UnicodeUtil, UNI_SUR_HIGH_END, UNI_SUR_HIGH_START};

use crate::util::TestUtil;

/// `UnicodeUtil.UNI_SUR_LOW_END`.
const UNI_SUR_LOW_END: i32 = 0xDFFF;

/// Port of `org.apache.lucene.tests.util.automaton.AutomatonTestUtil`.
pub struct AutomatonTestUtil;

impl AutomatonTestUtil {
    /// `DEFAULT_MAX_DETERMINIZED_STATES`: the default maximum number of
    /// states that `Operations.determinize` should create.
    pub const DEFAULT_MAX_DETERMINIZED_STATES: i32 = 1_000_000;

    /// `MAX_RECURSION_LEVEL`: the maximum level of recursion allowed in
    /// recursive operations.
    pub const MAX_RECURSION_LEVEL: i32 = 1000;

    /// `randomRegexp(r)`: a random regular expression, including the full
    /// unicode range, that `new RegExp(regexp, RegExp.NONE)` accepts.
    pub fn random_regexp<R: Random + ?Sized>(r: &R) -> String {
        loop {
            let regexp = Self::random_regexp_string(r);
            // we will also generate some undefined unicode queries
            if !UnicodeUtil::valid_utf16_string(&regexp, regexp.len()) {
                continue;
            }
            let regexp =
                String::from_utf16(&regexp).expect("INVARIANT: valid UTF-16 is a Rust string");
            if RegExp::with_flags(&regexp, RegExp::NONE).is_ok() {
                return regexp;
            }
        }
    }

    /// `randomRegexpString(r)`, as UTF-16 code units.
    fn random_regexp_string<R: Random + ?Sized>(r: &R) -> Vec<u16> {
        let end = r.next_int_bound(20) as usize;
        if end == 0 {
            // allow 0 length
            return Vec::new();
        }
        let mut buffer = vec![0u16; end];
        let mut i = 0;
        while i < end {
            let t = r.next_int_bound(15);
            if t == 0 && i < end - 1 {
                // Make a surrogate pair
                // High surrogate
                buffer[i] = TestUtil::next_int(r, 0xd800, 0xdbff) as u16;
                i += 1;
                // Low surrogate
                buffer[i] = TestUtil::next_int(r, 0xdc00, 0xdfff) as u16;
            } else if t <= 1 {
                buffer[i] = r.next_int_bound(0x80) as u16;
            } else if t == 2 {
                buffer[i] = TestUtil::next_int(r, 0x80, 0x800) as u16;
            } else if t == 3 {
                buffer[i] = TestUtil::next_int(r, 0x800, 0xd7ff) as u16;
            } else if t == 4 {
                buffer[i] = TestUtil::next_int(r, 0xe000, 0xffff) as u16;
            } else {
                buffer[i] = u16::from(b".?*+()-[]|"[t as usize - 5]);
            }
            i += 1;
        }
        buffer
    }

    /// `getRandomCodePoint(r, min, max)`: a random code point of `[min,
    /// max]`, avoiding surrogates.
    ///
    /// # Panics
    ///
    /// `IllegalArgumentException` (`transition accepts only surrogates:
    /// min=… max=…`) when the range holds only surrogates.
    fn get_random_code_point<R: Random + ?Sized>(r: &R, min: i32, max: i32) -> i32 {
        let (high_start, high_end) = (UNI_SUR_HIGH_START as i32, UNI_SUR_HIGH_END as i32);
        let only_surrogates = || panic!("transition accepts only surrogates: min={min} max={max}");
        let code = if max < high_start || min > high_end {
            // easy: entire range is before or after surrogates
            min + r.next_int_bound(max - min + 1)
        } else if min >= high_start {
            if max > UNI_SUR_LOW_END {
                // after surrogates
                1 + UNI_SUR_LOW_END + r.next_int_bound(max - UNI_SUR_LOW_END)
            } else {
                only_surrogates()
            }
        } else if max <= UNI_SUR_LOW_END {
            if min < high_start {
                // before surrogates
                min + r.next_int_bound(high_start - min)
            } else {
                only_surrogates()
            }
        } else {
            // range includes all surrogates
            let gap1 = high_start - min;
            let gap2 = max - UNI_SUR_LOW_END;
            let c = r.next_int_bound(gap1 + gap2);
            if c < gap1 {
                min + c
            } else {
                UNI_SUR_LOW_END + c - gap1 + 1
            }
        };
        java_assert_unchecked!(
            code >= min && code <= max && (code < high_start || code > UNI_SUR_LOW_END),
            "code={code} min={min} max={max}"
        );
        code
    }

    /// `randomSingleAutomaton(random)`: the automaton of a random regular
    /// expression, complemented half of the time; a regular expression too
    /// complex to complement is replaced by another.
    ///
    /// Public in the port for the oracle, which compares it by seed (Java
    /// keeps it private).
    ///
    /// # Panics
    ///
    /// When `RegExp.toAutomaton()` fails, as Java's unchecked exception.
    pub fn random_single_automaton<R: Random + ?Sized>(random: &R) -> Automaton {
        loop {
            let regexp = Self::random_regexp(random);
            let mut a1 = RegExp::with_flags(&regexp, RegExp::NONE)
                .and_then(|re| re.to_automaton())
                .unwrap_or_else(|e| panic!("{e}"));
            if random.next_boolean() {
                match Operations::complement(&a1, Self::DEFAULT_MAX_DETERMINIZED_STATES) {
                    Ok(complement) => a1 = complement,
                    // This can (rarely) happen if the random regexp is too
                    // hard; just try again...
                    Err(_) => continue,
                }
            }
            return a1;
        }
    }

    /// `randomAutomaton(random)`: a random NFA/DFA for testing, two random
    /// automata combined by concatenation, union, intersection or difference.
    ///
    /// # Panics
    ///
    /// `TooComplexToDeterminizeException` of `Operations.minus`, which Java
    /// does not catch here.
    pub fn random_automaton<R: Random + ?Sized>(random: &R) -> Automaton {
        // get two random Automata from regexps
        let a1 = Self::random_single_automaton(random);
        let a2 = Self::random_single_automaton(random);

        // combine them in random ways
        match random.next_int_bound(4) {
            0 => Operations::concatenate(&[a1, a2]),
            1 => Operations::union(&[a1, a2]),
            2 => Operations::intersection(&a1, &a2),
            _ => Operations::minus(&a1, &a2, Self::DEFAULT_MAX_DETERMINIZED_STATES)
                .unwrap_or_else(|e| panic!("{e}")),
        }
    }

    /// `reverseOriginal(a, initialStates)`: the original brics `reverse()`,
    /// which also records the new initial states in `initial_states`.
    pub fn reverse_original(a: &Automaton, initial_states: Option<&mut HashSet<i32>>) -> Automaton {
        if Operations::is_empty(a) {
            return Automaton::new();
        }

        let num_states = a.get_num_states();

        // Build a new automaton with all edges reversed
        let mut builder = AutomatonBuilder::new();

        // Initial node; we'll add epsilon transitions in the end:
        builder.create_state();

        for _ in 0..num_states {
            builder.create_state();
        }

        // Old initial state becomes new accept state:
        builder.set_accept(1, true);

        let mut t = Transition::new();
        for s in 0..num_states {
            let num_transitions = a.get_num_transitions(s);
            a.init_transition(s, &mut t);
            for _ in 0..num_transitions {
                a.get_next_transition(&mut t);
                builder.add_transition_range(t.dest + 1, s + 1, t.min, t.max);
            }
        }

        let mut result = builder.finish();

        let mut initial_states = initial_states;
        for s in 0..num_states {
            if a.is_accept(s) {
                result.add_epsilon(0, s + 1);
                if let Some(initial) = initial_states.as_deref_mut() {
                    initial.insert(s + 1);
                }
            }
        }

        result.finish_state();

        result
    }

    /// `minimizeSimple(a)`: the simple, original brics implementation of
    /// Brzozowski's `minimize()`.
    pub fn minimize_simple(a: &Automaton) -> Automaton {
        let mut initial_set = HashSet::new();
        let a = Self::determinize_simple_from(
            &Self::reverse_original(a, Some(&mut initial_set)),
            &initial_set,
        );
        initial_set.clear();
        Self::determinize_simple_from(
            &Self::reverse_original(&a, Some(&mut initial_set)),
            &initial_set,
        )
    }

    /// `assertMinimalDFA(automaton)`: asserts that an automaton is a minimal
    /// DFA.
    ///
    /// # Panics
    ///
    /// The `AssertionError` of a failed assertion.
    pub fn assert_minimal_dfa(automaton: &Automaton) {
        Self::assert_clean_dfa(automaton);
        let minimized = Self::minimize_simple(automaton);
        assert_eq!(minimized.get_num_states(), automaton.get_num_states());
    }

    /// `assertCleanDFA(automaton)`: asserts that an automaton is a DFA with
    /// no dead states.
    ///
    /// # Panics
    ///
    /// The `AssertionError` of a failed assertion.
    pub fn assert_clean_dfa(automaton: &Automaton) {
        Self::assert_clean_nfa(automaton);
        assert!(automaton.is_deterministic(), "must be deterministic");
    }

    /// `assertCleanNFA(automaton)`: asserts that an automaton has no dead
    /// states.
    ///
    /// # Panics
    ///
    /// The `AssertionError` of a failed assertion.
    pub fn assert_clean_nfa(automaton: &Automaton) {
        assert!(
            !Operations::has_dead_states_from_initial(automaton),
            "has dead states reachable from initial"
        );
        assert!(
            !Operations::has_dead_states_to_accept(automaton),
            "has dead states leading to accept"
        );
        assert!(
            !Operations::has_dead_states(automaton),
            "has unreachable dead states (ghost states)"
        );
    }

    /// `determinizeSimple(a)`: the simple, original brics implementation of
    /// `determinize()`, from state 0.
    pub fn determinize_simple(a: &Automaton) -> Automaton {
        Self::determinize_simple_from(a, &HashSet::from([0]))
    }

    /// `determinizeSimple(a, initialset)`: determinizes `a` by subset
    /// construction from the set of initial states `initialset`.
    ///
    /// Java's sets are `HashSet`s; the states are numbered in the order the
    /// FIFO worklist meets the subsets and the start points are walked, which
    /// no hash order affects, so the result is Java's.
    pub fn determinize_simple_from(a: &Automaton, initialset: &HashSet<i32>) -> Automaton {
        if a.get_num_states() == 0 {
            return a.clone();
        }
        let points = a.get_start_points();
        // subset construction; a subset is keyed by its sorted states.
        let key = |s: &HashSet<i32>| {
            let mut v: Vec<i32> = s.iter().copied().collect();
            v.sort_unstable();
            v
        };
        let mut worklist: VecDeque<Vec<i32>> = VecDeque::new();
        let mut newstate: HashMap<Vec<i32>, i32> = HashMap::new();
        let initial = key(initialset);
        worklist.push_back(initial.clone());
        let mut result = AutomatonBuilder::new();
        result.create_state();
        newstate.insert(initial, 0);
        let mut t = Transition::new();
        while let Some(s) = worklist.pop_front() {
            let r = newstate[&s];
            if s.iter().any(|&q| a.is_accept(q)) {
                result.set_accept(r, true);
            }
            for n in 0..points.len() {
                let mut p = HashSet::new();
                for &q in &s {
                    let count = a.init_transition(q, &mut t);
                    for _ in 0..count {
                        a.get_next_transition(&mut t);
                        if t.min <= points[n] && points[n] <= t.max {
                            p.insert(t.dest);
                        }
                    }
                }
                let p = key(&p);

                let q = match newstate.get(&p) {
                    Some(&q) => q,
                    None => {
                        worklist.push_back(p.clone());
                        let q = result.create_state();
                        newstate.insert(p, q);
                        q
                    }
                };
                let min = points[n];
                let max = if n + 1 < points.len() {
                    points[n + 1] - 1
                } else {
                    MAX_CODE_POINT
                };
                result.add_transition_range(r, q, min, max);
            }
        }

        Operations::remove_dead_states(&result.finish())
    }

    /// `getFiniteStringsRecursive(a, limit)`: the simple, original
    /// implementation of `getFiniteStrings`, the set of accepted strings,
    /// assuming at most `limit` (`< 0`: no limit) are accepted; past the
    /// limit, the first strings found.
    ///
    /// **Divergence.** Java returns a `Set<IntsRef>`; core's `IntsRef` has no
    /// `Hash`, so each string is its code points, the value an `IntsRef`
    /// compares by.
    pub fn get_finite_strings_recursive(a: &Automaton, limit: i32) -> HashSet<Vec<i32>> {
        let mut strings = HashSet::new();
        Self::get_finite_strings(
            a,
            0,
            &mut HashSet::new(),
            &mut strings,
            &mut Vec::new(),
            limit,
        );
        strings
    }

    /// `getFiniteStrings(a, s, pathstates, strings, path, limit)`: `false`
    /// if more than `limit` strings are found.
    fn get_finite_strings(
        a: &Automaton,
        s: i32,
        pathstates: &mut HashSet<i32>,
        strings: &mut HashSet<Vec<i32>>,
        path: &mut Vec<i32>,
        limit: i32,
    ) -> bool {
        pathstates.insert(s);
        let mut t = Transition::new();
        let count = a.init_transition(s, &mut t);
        for _ in 0..count {
            a.get_next_transition(&mut t);
            if pathstates.contains(&t.dest) {
                return false;
            }
            for n in t.min..=t.max {
                path.push(n);
                if a.is_accept(t.dest) {
                    strings.insert(path.clone());
                    if limit >= 0 && strings.len() as i64 > i64::from(limit) {
                        return false;
                    }
                }
                if !Self::get_finite_strings(a, t.dest, pathstates, strings, path, limit) {
                    return false;
                }
                path.pop();
            }
        }
        pathstates.remove(&s);
        true
    }

    /// `isFinite(a)`: whether the language of `a` (without dead states) is
    /// finite.
    ///
    /// # Panics
    ///
    /// `IllegalArgumentException` (`input automaton is too large: …`) past
    /// [`MAX_RECURSION_LEVEL`](Self::MAX_RECURSION_LEVEL) levels of
    /// recursion.
    pub fn is_finite(a: &Automaton) -> bool {
        if a.get_num_states() == 0 {
            return true;
        }
        let n = a.get_num_states() as usize;
        Self::is_finite_from(
            &mut Transition::new(),
            a,
            0,
            &mut vec![false; n],
            &mut vec![false; n],
            0,
        )
    }

    /// `isFinite(scratch, a, state, path, visited, level)`: whether no loop
    /// contains `state`.
    fn is_finite_from(
        scratch: &mut Transition,
        a: &Automaton,
        state: i32,
        path: &mut [bool],
        visited: &mut [bool],
        level: i32,
    ) -> bool {
        if level > Self::MAX_RECURSION_LEVEL {
            // `throw new IllegalArgumentException("input automaton is too large: " + level)`
            // (`AutomatonTestUtil.java:557-558`).
            rucene::store::java_nio_file::throw_unchecked(
                "java.lang.IllegalArgumentException",
                Some(&format!("input automaton is too large: {level}")),
            );
        }
        path[state as usize] = true;
        let num_transitions = a.init_transition(state, scratch);
        for t in 0..num_transitions {
            a.get_transition(state, t, scratch);
            let dest = scratch.dest as usize;
            if path[dest]
                || (!visited[dest]
                    && !Self::is_finite_from(scratch, a, dest as i32, path, visited, level + 1))
            {
                return false;
            }
        }
        path[state as usize] = false;
        visited[state as usize] = true;
        true
    }

    /// `assertNoDetachedStates(a)`: checks that an automaton has no detached
    /// states that are unreachable from the initial state.
    ///
    /// # Panics
    ///
    /// The `AssertionError` (`automaton has … detached states`), under
    /// assertions.
    pub fn assert_no_detached_states(a: &Automaton) {
        let a2 = Operations::remove_dead_states(a);
        java_assert_unchecked!(
            a.get_num_states() == a2.get_num_states(),
            "automaton has {} detached states",
            a.get_num_states() - a2.get_num_states()
        );
    }

    /// `isDeterministicSlow(a)`: whether the automaton is deterministic.
    pub fn is_deterministic_slow(a: &Automaton) -> bool {
        let mut t = Transition::new();
        let num_states = a.get_num_states();
        for s in 0..num_states {
            let count = a.init_transition(s, &mut t);
            let mut last_max = -1;
            for _ in 0..count {
                a.get_next_transition(&mut t);
                if t.min <= last_max {
                    java_assert_unchecked!(!a.is_deterministic());
                    return false;
                }
                last_max = t.max;
            }
        }

        java_assert_unchecked!(a.is_deterministic());
        true
    }

    /// `sameLanguage(a1, a2)`: whether the two automata (determinized,
    /// without dead states) accept exactly the same language.
    ///
    /// # Panics
    ///
    /// As [`subset_of`](Self::subset_of).
    pub fn same_language(a1: &Automaton, a2: &Automaton) -> bool {
        if std::ptr::eq(a1, a2) {
            return true;
        }
        Self::subset_of(a2, a1) && Self::subset_of(a1, a2)
    }

    /// `subsetOf(a1, a2)`: whether the language of `a1` is a subset of the
    /// language of `a2`; both must be determinized without dead states.
    ///
    /// # Panics
    ///
    /// `IllegalArgumentException` (`a1 must be deterministic`, `a2 must be
    /// deterministic`).
    pub fn subset_of(a1: &Automaton, a2: &Automaton) -> bool {
        if !a1.is_deterministic() {
            panic!("a1 must be deterministic");
        }
        if !a2.is_deterministic() {
            panic!("a2 must be deterministic");
        }
        java_assert_unchecked!(!Operations::has_dead_states_from_initial(a1));
        java_assert_unchecked!(!Operations::has_dead_states_from_initial(a2));
        if a1.get_num_states() == 0 {
            // Empty language is always a subset of any other language
            return true;
        } else if a2.get_num_states() == 0 {
            return Operations::is_empty(a1);
        }

        // TODO: cutover to iterators instead
        let transitions1 = a1.get_sorted_transitions();
        let transitions2 = a2.get_sorted_transitions();
        let mut worklist = VecDeque::new();
        let mut visited = HashSet::new();
        worklist.push_back((0, 0));
        visited.insert((0, 0));
        while let Some((s1, s2)) = worklist.pop_front() {
            if a1.is_accept(s1) && !a2.is_accept(s2) {
                return false;
            }
            let t1 = &transitions1[s1 as usize];
            let t2 = &transitions2[s2 as usize];
            let mut b2 = 0;
            for tn1 in t1 {
                while b2 < t2.len() && t2[b2].max < tn1.min {
                    b2 += 1;
                }
                let (mut min1, mut max1) = (tn1.min, tn1.max);

                let mut n2 = b2;
                while n2 < t2.len() && tn1.max >= t2[n2].min {
                    if t2[n2].min > min1 {
                        return false;
                    }
                    if t2[n2].max < MAX_CODE_POINT {
                        min1 = t2[n2].max + 1;
                    } else {
                        min1 = MAX_CODE_POINT;
                        max1 = MIN_CODE_POINT;
                    }
                    let q = (tn1.dest, t2[n2].dest);
                    if visited.insert(q) {
                        worklist.push_back(q);
                    }
                    n2 += 1;
                }
                if min1 <= max1 {
                    return false;
                }
            }
        }
        true
    }
}

/// Port of `AutomatonTestUtil.RandomAcceptedStrings`: random strings accepted
/// by an automaton.
///
/// Once created, call [`get_random_accepted_string`] for a new string (in
/// UTF-32 code points).
///
/// Java keys `leadsToAccept` by the `Transition` objects of
/// `getSortedTransitions()` (identity, `Transition` has no `equals`); the port
/// keys it by each transition's position in that array.
///
/// [`get_random_accepted_string`]: RandomAcceptedStrings::get_random_accepted_string
pub struct RandomAcceptedStrings<'a> {
    leads_to_accept: HashSet<(i32, usize)>,
    a: &'a Automaton,
    transitions: Vec<Vec<Transition>>,
}

impl<'a> RandomAcceptedStrings<'a> {
    /// `new RandomAcceptedStrings(a)`.
    ///
    /// # Panics
    ///
    /// `IllegalArgumentException` (`this automaton accepts nothing`) for an
    /// automaton without states.
    pub fn new(a: &'a Automaton) -> Self {
        if a.get_num_states() == 0 {
            panic!("this automaton accepts nothing");
        }
        let transitions = a.get_sorted_transitions();

        let mut leads_to_accept = HashSet::new();
        let mut all_arriving: HashMap<i32, Vec<(i32, usize)>> = HashMap::new();

        let mut q = VecDeque::new();
        let mut seen = HashSet::new();

        // reverse map the transitions, so we can quickly look
        // up all arriving transitions to a given state
        let num_states = a.get_num_states();
        for s in 0..num_states {
            for (index, t) in transitions[s as usize].iter().enumerate() {
                all_arriving.entry(t.dest).or_default().push((s, index));
            }
            if a.is_accept(s) {
                q.push_back(s);
                seen.insert(s);
            }
        }

        // Breadth-first search, from accept states,
        // backwards:
        while let Some(s) = q.pop_front() {
            if let Some(arriving) = all_arriving.get(&s) {
                for &(from, index) in arriving {
                    if seen.insert(from) {
                        q.push_back(from);
                        leads_to_accept.insert((from, index));
                    }
                }
            }
        }
        RandomAcceptedStrings {
            leads_to_accept,
            a,
            transitions,
        }
    }

    /// `getRandomAcceptedString(r)`: a random accepted string, as code
    /// points.
    ///
    /// # Panics
    ///
    /// `RuntimeException` (`this automaton has dead states`), and the
    /// `IllegalArgumentException` of a transition that accepts only
    /// surrogates.
    pub fn get_random_accepted_string<R: Random + ?Sized>(&self, r: &R) -> Vec<i32> {
        let mut code_points = Vec::new();

        let mut s = 0;

        loop {
            if self.a.is_accept(s) {
                if self.a.get_num_transitions(s) == 0 {
                    // stop now
                    break;
                } else if r.next_boolean() {
                    break;
                }
            }

            if self.a.get_num_transitions(s) == 0 {
                panic!("this automaton has dead states");
            }

            let cheat = r.next_boolean();

            let transitions = &self.transitions[s as usize];
            let t = if cheat {
                // pick a transition that we know is the fastest
                // path to an accept state
                let to_accept: Vec<&Transition> = transitions
                    .iter()
                    .enumerate()
                    .filter(|(index, _)| self.leads_to_accept.contains(&(s, *index)))
                    .map(|(_, t)| t)
                    .collect();
                if to_accept.is_empty() {
                    // this is OK -- it means we jumped into a cycle
                    &transitions[r.next_int_bound(transitions.len() as i32) as usize]
                } else {
                    to_accept[r.next_int_bound(to_accept.len() as i32) as usize]
                }
            } else {
                &transitions[r.next_int_bound(transitions.len() as i32) as usize]
            };
            code_points.push(AutomatonTestUtil::get_random_code_point(r, t.min, t.max));
            s = t.dest;
        }
        code_points
    }
}
