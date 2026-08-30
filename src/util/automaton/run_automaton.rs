//! Port of `org.apache.lucene.util.automaton.RunAutomaton` and
//! `org.apache.lucene.util.automaton.ByteRunnable`.

use crate::error::{LuceneError, Result};
use crate::util::FixedBitSet;

use super::automaton::{append_char_string, Automaton, Transition};

/// A runnable automaton accepting a byte array as input.
///
/// Equivalent to `org.apache.lucene.util.automaton.ByteRunnable`.
///
/// # Divergence from Lucene 10.5.0
///
/// Lucene names the two methods `getSize()` and `run(byte[], int, int)`. This port
/// keeps the pre-existing Rucene names [`ByteRunnable::size`] and
/// [`ByteRunnable::run`] (the latter taking a slice, which already carries the
/// offset and length); [`ByteRunnable::run_range`] is the literal counterpart of the
/// Java signature.
pub trait ByteRunnable {
    /// Returns the state obtained by reading the given byte from the given state.
    ///
    /// Returns `-1` if there is no such state.
    fn step(&self, state: i32, c: i32) -> i32;

    /// Returns the acceptance status for the given state.
    fn is_accept(&self, state: i32) -> bool;

    /// Returns the number of states this automaton has.
    ///
    /// Note this may not be an accurate number in the case of an NFA.
    fn size(&self) -> i32;

    /// Returns true if the given byte slice is accepted by this automaton.
    fn run(&self, s: &[u8]) -> bool {
        self.run_range(s, 0, s.len())
    }

    /// Returns true if `s[offset..offset + length]` is accepted by this automaton.
    fn run_range(&self, s: &[u8], offset: usize, length: usize) -> bool {
        let mut p = 0i32;
        let l = offset + length;
        for &b in &s[offset..l] {
            p = self.step(p, i32::from(b));
            if p == -1 {
                return false;
            }
        }
        self.is_accept(p)
    }
}

/// Finite-state automaton with a fast run operation. The initial state is always 0.
///
/// Equivalent to `org.apache.lucene.util.automaton.RunAutomaton`. Lucene declares it
/// abstract and derives `ByteRunAutomaton` and `CharacterRunAutomaton` from it; this
/// port makes it a concrete struct that those two types wrap, because Rust has no
/// implementation inheritance.
#[derive(Clone, Debug)]
pub struct RunAutomaton {
    automaton: Automaton,
    alphabet_size: i32,
    size: i32,
    accept: FixedBitSet,
    /// `delta(state, c) = transitions[state * points.len() + get_char_class(c)]`
    transitions: Vec<i32>,
    /// Char interval start points.
    points: Vec<i32>,
    /// Map from char number to class.
    classmap: Vec<i32>,
}

impl RunAutomaton {
    /// Constructs a new `RunAutomaton` from a deterministic [`Automaton`].
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] if the automaton is not
    /// deterministic.
    pub fn new(a: Automaton, alphabet_size: i32) -> Result<Self> {
        if !a.is_deterministic() {
            return Err(LuceneError::IllegalArgument(
                "Automaton must be deterministic".to_string(),
            ));
        }
        let points = a.get_start_points();
        let size = a.get_num_states().max(1);
        let mut accept = FixedBitSet::new(size as usize);
        let mut transitions = vec![-1i32; size as usize * points.len()];
        let mut transition = Transition::new();
        for n in 0..size {
            if a.is_accept(n) {
                accept.set(n as usize);
            }
            transition.source = n;
            transition.transition_upto = -1;
            for (c, &point) in points.iter().enumerate() {
                let dest = a.next(&mut transition, point);
                debug_assert!(dest == -1 || dest < size);
                transitions[n as usize * points.len() + c] = dest;
            }
        }

        // Set alphabet table for optimal run performance.
        let mut classmap = vec![0i32; 256.min(alphabet_size).max(0) as usize];
        let mut i = 0usize;
        for (j, slot) in classmap.iter_mut().enumerate() {
            if i + 1 < points.len() && j as i32 == points[i + 1] {
                i += 1;
            }
            *slot = i as i32;
        }

        Ok(Self {
            automaton: a,
            alphabet_size,
            size,
            accept,
            transitions,
            points,
            classmap,
        })
    }

    /// Returns the number of states in the automaton.
    pub fn get_size(&self) -> i32 {
        self.size
    }

    /// Returns the number of states in the automaton.
    ///
    /// Rucene name for `getSize()`.
    pub fn size(&self) -> i32 {
        self.size
    }

    /// Returns the acceptance status for the given state.
    pub fn is_accept(&self, state: i32) -> bool {
        self.accept.get(state as usize)
    }

    /// Returns the array of codepoint class interval start points.
    pub fn get_char_intervals(&self) -> &[i32] {
        &self.points
    }

    /// Returns the source automaton used to build this table.
    pub fn automaton(&self) -> &Automaton {
        &self.automaton
    }

    /// Gets the character class of the given codepoint.
    pub(crate) fn get_char_class(&self, c: i32) -> i32 {
        // binary search
        let mut a = 0usize;
        let mut b = self.points.len();
        while b - a > 1 {
            let d = (a + b) >> 1;
            match self.points[d].cmp(&c) {
                std::cmp::Ordering::Greater => b = d,
                std::cmp::Ordering::Less => a = d,
                std::cmp::Ordering::Equal => return d as i32,
            }
        }
        a as i32
    }

    /// Returns the state obtained by reading the given char from the given state.
    ///
    /// Returns `-1` if no such state is obtained. (If the original [`Automaton`] had
    /// no dead states, `-1` is returned here if and only if a dead state is entered
    /// in an equivalent automaton with a total transition function.)
    pub fn step(&self, state: i32, c: i32) -> i32 {
        debug_assert!(c < self.alphabet_size);
        if c as usize >= self.classmap.len() {
            self.transitions[state as usize * self.points.len() + self.get_char_class(c) as usize]
        } else {
            self.transitions
                [state as usize * self.points.len() + self.classmap[c as usize] as usize]
        }
    }
}

impl std::fmt::Display for RunAutomaton {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut b = String::new();
        b.push_str("initial state: 0\n");
        for i in 0..self.size {
            b.push_str(&format!("state {}", i));
            if self.accept.get(i as usize) {
                b.push_str(" [accept]:\n");
            } else {
                b.push_str(" [reject]:\n");
            }
            for j in 0..self.points.len() {
                let k = self.transitions[i as usize * self.points.len() + j];
                if k != -1 {
                    let min = self.points[j];
                    let max = if j + 1 < self.points.len() {
                        self.points[j + 1] - 1
                    } else {
                        self.alphabet_size
                    };
                    b.push(' ');
                    append_char_string(min, &mut b);
                    if min != max {
                        b.push('-');
                        append_char_string(max, &mut b);
                    }
                    b.push_str(&format!(" -> {}\n", k));
                }
            }
        }
        f.write_str(&b)
    }
}

impl PartialEq for RunAutomaton {
    fn eq(&self, other: &Self) -> bool {
        self.alphabet_size == other.alphabet_size
            && self.size == other.size
            && self.points == other.points
            && self.accept.get_bits() == other.accept.get_bits()
            && self.transitions == other.transitions
    }
}

impl Eq for RunAutomaton {}

impl ByteRunnable for RunAutomaton {
    fn step(&self, state: i32, c: i32) -> i32 {
        RunAutomaton::step(self, state, c)
    }

    fn is_accept(&self, state: i32) -> bool {
        RunAutomaton::is_accept(self, state)
    }

    fn size(&self) -> i32 {
        self.size
    }
}
