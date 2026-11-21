//! Port of `lucene/core/src/test/org/apache/lucene/util/automaton/TestLevenshteinAutomata.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::{Class, Method};
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::util::automaton::{
    Automata, Automaton, CharacterRunAutomaton, LevenshteinAutomata, Operations,
    DEFAULT_DETERMINIZE_WORK_LIMIT,
};
use rucene_test_framework::util::automaton::automaton_test_util::AutomatonTestUtil;

use crate::core_support::{assert_not_null, class, det, test, unchecked};

/// `TestLevenshteinAutomata`.
pub fn class_def() -> Arc<Class> {
    let methods: Vec<Method> = vec![
        test("testLev0", test_lev0),
        test("testLev1", test_lev1),
        test("testLev2", test_lev2),
        test("testNoWastedStates", test_no_wasted_states),
    ];
    class(
        "org.apache.lucene.util.automaton.TestLevenshteinAutomata",
        methods,
    )
}

fn test_lev0() -> Result<()> {
    assert_lev("", 0)?;
    assert_char_vectors(0)
}

fn test_lev1() -> Result<()> {
    assert_lev("", 1)?;
    assert_char_vectors(1)
}

fn test_lev2() -> Result<()> {
    assert_lev("", 2)?;
    assert_char_vectors(2)
}

/// `testNoWastedStates()` (LUCENE-3094).
fn test_no_wasted_states() -> Result<()> {
    let a = LevenshteinAutomata::from_str("abc", false)?
        .to_automaton(1)?
        .expect("distance 1 is supported");
    Assert::assert_false(Operations::has_dead_states_from_initial(&a));
    Ok(())
}

/// `assertCharVectors(n)`: tests all possible characteristic vectors for
/// some n. This exhaustively tests the parametric transitions tables.
fn assert_char_vectors(n: i32) -> Result<()> {
    let k = 2 * n + 1;
    // use k + 2 as the exponent: the formula generates different transitions
    // for w, w-1, w-2
    let limit = 2f64.powi(k + 2) as i32;
    for i in 0..limit {
        let encoded = format!("{i:b}");
        assert_lev(&encoded, n)?;
    }
    Ok(())
}

/// `assertLev(s, maxDistance)`: builds a DFA for some string, and checks
/// all Lev automata up to some maximum distance.
fn assert_lev(s: &str, max_distance: i32) -> Result<()> {
    let builder = LevenshteinAutomata::from_str(s, false)?;
    let tbuilder = LevenshteinAutomata::from_str(s, true)?;
    let mut automata: Vec<Automaton> = Vec::new();
    let mut tautomata: Vec<Automaton> = Vec::new();
    for n in 0..=max_distance {
        let a = builder.to_automaton(n)?;
        let ta = tbuilder.to_automaton(n)?;
        assert_not_null(&a);
        assert_not_null(&ta);
        automata.push(a.expect("asserted not null"));
        tautomata.push(ta.expect("asserted not null"));
        let n = n as usize;
        Assert::assert_true(automata[n].is_deterministic());
        Assert::assert_true(tautomata[n].is_deterministic());
        Assert::assert_true(AutomatonTestUtil::is_finite(&automata[n]));
        Assert::assert_true(AutomatonTestUtil::is_finite(&tautomata[n]));
        Assert::assert_false(Operations::has_dead_states_from_initial(&automata[n]));
        Assert::assert_false(Operations::has_dead_states_from_initial(&tautomata[n]));
        // check that the dfa for n-1 accepts a subset of the dfa for n
        if n > 0 {
            Assert::assert_true(AutomatonTestUtil::subset_of(
                &Operations::remove_dead_states(&automata[n - 1]),
                &Operations::remove_dead_states(&automata[n]),
            ));
            Assert::assert_true(AutomatonTestUtil::subset_of(
                &Operations::remove_dead_states(&automata[n - 1]),
                &Operations::remove_dead_states(&tautomata[n]),
            ));
            Assert::assert_true(AutomatonTestUtil::subset_of(
                &Operations::remove_dead_states(&tautomata[n - 1]),
                &Operations::remove_dead_states(&automata[n]),
            ));
            Assert::assert_true(AutomatonTestUtil::subset_of(
                &Operations::remove_dead_states(&tautomata[n - 1]),
                &Operations::remove_dead_states(&tautomata[n]),
            ));
            // assertNotSame(automata[n - 1], automata[n])
            Assert::assert_false(std::ptr::eq(&automata[n - 1], &automata[n]));
        }
        // check that Lev(N) is a subset of LevT(N)
        Assert::assert_true(AutomatonTestUtil::subset_of(
            &Operations::remove_dead_states(&automata[n]),
            &Operations::remove_dead_states(&tautomata[n]),
        ));
        // special checks for specific n
        match n {
            0 => {
                // easy, matches the string itself
                Assert::assert_true(AutomatonTestUtil::same_language(
                    &Automata::make_string(s),
                    &Operations::remove_dead_states(&automata[0]),
                ));
                Assert::assert_true(AutomatonTestUtil::same_language(
                    &Automata::make_string(s),
                    &Operations::remove_dead_states(&tautomata[0]),
                ));
            }
            1 => {
                // generate a lev1 naively, and check the accepted lang is the same.
                Assert::assert_true(AutomatonTestUtil::same_language(
                    &naive_lev1(s),
                    &Operations::remove_dead_states(&automata[1]),
                ));
                Assert::assert_true(AutomatonTestUtil::same_language(
                    &naive_lev1_t(s),
                    &Operations::remove_dead_states(&tautomata[1]),
                ));
            }
            _ => {
                assert_brute_force(s, &automata[n], n as i32);
                assert_brute_force_t(s, &tautomata[n], n as i32);
            }
        }
    }
    Ok(())
}

/// `naiveLev1(s)`: an automaton that accepts all 1-character insertions,
/// deletions, and substitutions of s.
fn naive_lev1(s: &str) -> Automaton {
    det(Operations::determinize(
        &Operations::union(&[
            Automata::make_string(s),
            insertions_of(s),
            deletions_of(s),
            substitutions_of(s),
        ]),
        DEFAULT_DETERMINIZE_WORK_LIMIT,
    ))
}

/// `naiveLev1T(s)`: an automaton that accepts all 1-character insertions,
/// deletions, substitutions, and transpositions of s.
fn naive_lev1_t(s: &str) -> Automaton {
    det(Operations::determinize(
        &Operations::union(&[
            Automata::make_string(s),
            insertions_of(s),
            deletions_of(s),
            substitutions_of(s),
            transpositions_of(s),
        ]),
        DEFAULT_DETERMINIZE_WORK_LIMIT,
    ))
}

/// `insertionsOf(s)`: all 1-character insertions of s. The strings of this
/// test are binary digits, so byte indices are `char` indices.
fn insertions_of(s: &str) -> Automaton {
    let mut list = Vec::new();

    for i in 0..=s.len() {
        let pre = Automata::make_string(&s[..i]);
        let middle = Automata::make_any_char();
        let post = Automata::make_string(&s[i..]);
        list.push(Operations::concatenate(&[pre, middle, post]));
    }

    Operations::union(&list)
}

/// `deletionsOf(s)`: all 1-character deletions of s.
fn deletions_of(s: &str) -> Automaton {
    let mut list = Vec::new();

    for i in 0..s.len() {
        let pre = Automata::make_string(&s[..i]);
        let post = Automata::make_string(&s[i + 1..]);
        list.push(Operations::concatenate(&[pre, post]));
    }

    Operations::union(&list)
}

/// `substitutionsOf(s)`: all 1-character substitutions of s.
fn substitutions_of(s: &str) -> Automaton {
    let mut list = Vec::new();

    for i in 0..s.len() {
        let pre = Automata::make_string(&s[..i]);
        let middle = Automata::make_any_char();
        let post = Automata::make_string(&s[i + 1..]);
        list.push(Operations::concatenate(&[pre, middle, post]));
    }

    Operations::union(&list)
}

/// `transpositionsOf(s)`: all transpositions of s (transposing two adjacent
/// characters).
fn transpositions_of(s: &str) -> Automaton {
    if s.len() < 2 {
        return Automata::make_empty();
    }
    let b = s.as_bytes();
    let mut list = Vec::new();
    for i in 0..s.len() - 1 {
        let mut sb = String::new();
        sb.push_str(&s[..i]);
        sb.push(b[i + 1] as char);
        sb.push(b[i] as char);
        sb.push_str(&s[i + 2..]);
        if sb != s {
            list.push(Automata::make_string(&sb));
        }
    }
    Operations::union(&list)
}

fn assert_brute_force(input: &str, dfa: &Automaton, distance: i32) {
    let ra = unchecked(CharacterRunAutomaton::new(dfa.clone()));
    let max_len = input.len() as i32 + distance + 1;
    let max_num = 2f64.powi(max_len) as i32;
    for i in 0..max_num {
        let encoded = format!("{i:b}");
        let accepts = ra.run(&encoded);
        if accepts {
            Assert::assert_true(get_distance(input, &encoded) <= distance);
        } else {
            Assert::assert_true(get_distance(input, &encoded) > distance);
        }
    }
}

fn assert_brute_force_t(input: &str, dfa: &Automaton, distance: i32) {
    let ra = unchecked(CharacterRunAutomaton::new(dfa.clone()));
    let max_len = input.len() as i32 + distance + 1;
    let max_num = 2f64.powi(max_len) as i32;
    for i in 0..max_num {
        let encoded = format!("{i:b}");
        let accepts = ra.run(&encoded);
        if accepts {
            Assert::assert_true(get_t_distance(input, &encoded) <= distance);
        } else {
            Assert::assert_true(get_t_distance(input, &encoded) > distance);
        }
    }
}

/// `getDistance(target, other)`: Levenshtein distance (see
/// `org.apache.commons.lang.StringUtils#getLevenshteinDistance`), over the
/// UTF-16 code units of the strings.
fn get_distance(target: &str, other: &str) -> i32 {
    let sa: Vec<u16> = target.encode_utf16().collect();
    let t: Vec<u16> = other.encode_utf16().collect();
    let n = sa.len();
    let mut p = vec![0i32; n + 1]; // 'previous' cost array, horizontally
    let mut d = vec![0i32; n + 1]; // cost array, horizontally

    let m = t.len();
    if n == 0 || m == 0 {
        if n == m {
            return 0;
        } else {
            return n.max(m) as i32;
        }
    }

    for (i, v) in p.iter_mut().enumerate() {
        *v = i as i32;
    }

    for j in 1..=m {
        let t_j = t[j - 1];
        d[0] = j as i32;

        for i in 1..=n {
            let cost = if sa[i - 1] == t_j { 0 } else { 1 };
            // minimum of cell to the left+1, to the top+1, diagonally left and up +cost
            d[i] = (d[i - 1] + 1).min(p[i] + 1).min(p[i - 1] + cost);
        }

        // copy current distance counts to 'previous row' distance counts
        std::mem::swap(&mut p, &mut d);
    }

    // our last action in the above loop was to switch d and p, so p now
    // actually has the most recent cost counts
    p[n].abs()
}

/// `getTDistance(target, other)`: the distance with transpositions.
fn get_t_distance(target: &str, other: &str) -> i32 {
    let sa: Vec<u16> = target.encode_utf16().collect();
    let t: Vec<u16> = other.encode_utf16().collect();
    let n = sa.len();
    let m = t.len();
    let mut d = vec![vec![0i32; m + 1]; n + 1]; // cost array

    if n == 0 || m == 0 {
        if n == m {
            return 0;
        } else {
            return n.max(m) as i32;
        }
    }

    for (i, row) in d.iter_mut().enumerate() {
        row[0] = i as i32;
    }

    for j in 0..=m {
        d[0][j] = j as i32;
    }

    for j in 1..=m {
        let t_j = t[j - 1];

        for i in 1..=n {
            let cost = if sa[i - 1] == t_j { 0 } else { 1 };
            // minimum of cell to the left+1, to the top+1, diagonally left and up +cost
            d[i][j] = (d[i - 1][j] + 1)
                .min(d[i][j - 1] + 1)
                .min(d[i - 1][j - 1] + cost);
            // transposition
            if i > 1 && j > 1 && sa[i - 1] == t[j - 2] && sa[i - 2] == t[j - 1] {
                d[i][j] = d[i][j].min(d[i - 2][j - 2] + cost);
            }
        }
    }

    // our last action in the above loop was to switch d and p, so p now
    // actually has the most recent cost counts
    d[n][m].abs()
}
