//! Port of `org.apache.lucene.util.automaton.LevenshteinAutomata`.

use std::collections::HashSet;

use crate::error::{LuceneError, Result};
use crate::util::UnicodeUtil;

use super::automata::Automata;
use super::automaton::{Automaton, MAX_CODE_POINT};
use super::lev1_parametric_description::Lev1ParametricDescription;
use super::lev1t_parametric_description::Lev1TParametricDescription;
use super::lev2_parametric_description::Lev2ParametricDescription;
use super::lev2t_parametric_description::Lev2TParametricDescription;
use super::operations::Operations;

/// Maximum edit distance this class can generate an automaton for.
pub const MAXIMUM_SUPPORTED_DISTANCE: i32 = 2;

/// Bit masks used by [`unpack`]; `MASKS[i]` has the low `i + 1` bits set.
const MASKS: [u64; 63] = {
    let mut masks = [0u64; 63];
    let mut i = 0;
    while i < 63 {
        masks[i] = (1u64 << (i + 1)) - 1;
        i += 1;
    }
    masks
};

/// Reads the `index`th value of `bits_per_value` bits out of the packed `data`.
///
/// Equivalent to `LevenshteinAutomata.ParametricDescription.unpack`.
pub fn unpack(data: &[u64], index: i32, bits_per_value: i32) -> i32 {
    let bit_loc = i64::from(bits_per_value) * i64::from(index);
    let data_loc = (bit_loc >> 6) as usize;
    let bit_start = (bit_loc & 63) as i32;
    if bit_start + bits_per_value <= 64 {
        // not split
        ((data[data_loc] >> bit_start) & MASKS[(bits_per_value - 1) as usize]) as i32
    } else {
        // split
        let part = 64 - bit_start;
        (((data[data_loc] >> bit_start) & MASKS[(part - 1) as usize])
            + ((data[1 + data_loc] & MASKS[(bits_per_value - part - 1) as usize]) << part))
            as i32
    }
}

/// Describes the structure of a Levenshtein DFA for some degree `n`.
///
/// Equivalent to `LevenshteinAutomata.ParametricDescription`.
///
/// There are four components of a parametric description, all parameterized on the
/// length of the word `w`:
///
/// 1. the number of states: [`ParametricDescription::size`];
/// 2. the set of final states: [`ParametricDescription::is_accept`];
/// 3. the transition function: [`ParametricDescription::transition`];
/// 4. the minimal boundary function: [`ParametricDescription::get_position`].
pub trait ParametricDescription {
    /// The length of the word this description was built for.
    fn w(&self) -> i32;

    /// The edit distance this description encodes.
    fn n(&self) -> i32;

    /// The minimal number of errors for each parametric state.
    fn min_errors(&self) -> &'static [i32];

    /// Returns the number of states needed to compute a Levenshtein DFA.
    fn size(&self) -> i32 {
        self.min_errors().len() as i32 * (self.w() + 1)
    }

    /// Returns true if `abs_state` in any Levenshtein DFA is an accept (final) state.
    fn is_accept(&self, abs_state: i32) -> bool {
        // decode absState -> state, offset
        let state = abs_state / (self.w() + 1);
        let offset = abs_state % (self.w() + 1);
        debug_assert!(offset >= 0);
        self.w() - offset + self.min_errors()[state as usize] <= self.n()
    }

    /// Returns the position in the input word for a given state.
    ///
    /// This is the minimal boundary for the state.
    fn get_position(&self, abs_state: i32) -> i32 {
        abs_state % (self.w() + 1)
    }

    /// Returns the state number for a transition from the given state, assuming
    /// `position` and characteristic vector `vector`.
    fn transition(&self, abs_state: i32, position: i32, vector: i32) -> i32;
}

/// Constructs DFAs that match a word within some edit distance.
///
/// Equivalent to `org.apache.lucene.util.automaton.LevenshteinAutomata`. Implements
/// the algorithm described in Schulz and Mihov, *Fast String Correction with
/// Levenshtein Automata*.
pub struct LevenshteinAutomata {
    /// Input word.
    word: Vec<i32>,
    /// The automata alphabet.
    alphabet: Vec<i32>,
    /// The maximum symbol in the alphabet (e.g. 255 for UTF-8 or 10FFFF for UTF-32).
    alpha_max: i32,
    /// The ranges outside of the alphabet.
    range_lower: Vec<i32>,
    range_upper: Vec<i32>,
    num_ranges: usize,
    descriptions: Vec<Option<Box<dyn ParametricDescription>>>,
}

impl LevenshteinAutomata {
    /// Creates a new `LevenshteinAutomata` for some input string.
    ///
    /// Optionally counts transpositions as a primitive edit.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] if the word contains a symbol above
    /// `Character.MAX_CODE_POINT`.
    pub fn from_str(input: &str, with_transpositions: bool) -> Result<Self> {
        let word: Vec<i32> = input.chars().map(|c| c as i32).collect();
        Self::new(word, MAX_CODE_POINT, with_transpositions)
    }

    /// Expert: specify a custom maximum possible symbol (`alpha_max`); the default
    /// is `Character.MAX_CODE_POINT`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] if the word contains a symbol above
    /// `alpha_max`.
    pub fn new(word: Vec<i32>, alpha_max: i32, with_transpositions: bool) -> Result<Self> {
        // calculate the alphabet
        let mut set: HashSet<i32> = HashSet::new();
        for &v in &word {
            if v > alpha_max {
                return Err(LuceneError::IllegalArgument(format!(
                    "alphaMax exceeded by symbol {} in word",
                    v
                )));
            }
            set.insert(v);
        }
        let mut alphabet: Vec<i32> = set.into_iter().collect();
        alphabet.sort_unstable();

        let mut range_lower = vec![0i32; alphabet.len() + 2];
        let mut range_upper = vec![0i32; alphabet.len() + 2];
        // calculate the unicode range intervals that exclude the alphabet
        // these are the ranges for all unicode characters not in the alphabet
        let mut num_ranges = 0usize;
        let mut lower = 0i32;
        for &higher in &alphabet {
            if higher > lower {
                range_lower[num_ranges] = lower;
                range_upper[num_ranges] = higher - 1;
                num_ranges += 1;
            }
            lower = higher + 1;
        }
        // add the final endpoint
        if lower <= alpha_max {
            range_lower[num_ranges] = lower;
            range_upper[num_ranges] = alpha_max;
            num_ranges += 1;
        }

        let w = word.len() as i32;
        let descriptions: Vec<Option<Box<dyn ParametricDescription>>> = vec![
            // for n=0, we do not need to go through the trouble
            None,
            Some(if with_transpositions {
                Box::new(Lev1TParametricDescription::new(w)) as Box<dyn ParametricDescription>
            } else {
                Box::new(Lev1ParametricDescription::new(w)) as Box<dyn ParametricDescription>
            }),
            Some(if with_transpositions {
                Box::new(Lev2TParametricDescription::new(w)) as Box<dyn ParametricDescription>
            } else {
                Box::new(Lev2ParametricDescription::new(w)) as Box<dyn ParametricDescription>
            }),
        ];

        Ok(Self {
            word,
            alphabet,
            alpha_max,
            range_lower,
            range_upper,
            num_ranges,
            descriptions,
        })
    }

    /// The maximum symbol in the alphabet this instance was built for.
    pub fn alpha_max(&self) -> i32 {
        self.alpha_max
    }

    /// Computes a DFA that accepts all strings within an edit distance of `n`.
    ///
    /// All automata have the following properties: they are deterministic, they have
    /// no transitions to dead states, and they are not minimal (some transitions
    /// could be combined). Returns `None` when `n` exceeds
    /// [`MAXIMUM_SUPPORTED_DISTANCE`].
    ///
    /// # Errors
    ///
    /// Returns an error if the word cannot be rendered back to a string.
    pub fn to_automaton(&self, n: i32) -> Result<Option<Automaton>> {
        self.to_automaton_with_prefix(n, "")
    }

    /// Computes a DFA that accepts all strings within an edit distance of `n`,
    /// matching the specified exact prefix.
    ///
    /// Returns `None` when `n` exceeds [`MAXIMUM_SUPPORTED_DISTANCE`].
    ///
    /// # Errors
    ///
    /// Returns an error if the word cannot be rendered back to a string.
    pub fn to_automaton_with_prefix(&self, n: i32, prefix: &str) -> Result<Option<Automaton>> {
        if n == 0 {
            let word = UnicodeUtil::new_string(&self.word, 0, self.word.len())?;
            let mut s = String::with_capacity(prefix.len() + word.len());
            s.push_str(prefix);
            s.push_str(&word);
            return Ok(Some(Automata::make_string(&s)));
        }

        if n as usize >= self.descriptions.len() {
            return Ok(None);
        }

        let range = 2 * n + 1;
        let description = self.descriptions[n as usize]
            .as_ref()
            .expect("INVARIANT: only index 0 is None and n != 0 here");
        // the number of states is based on the length of the word and n
        let num_states = description.size();
        let num_transitions = num_states * (1 + 2 * n).min(self.alphabet.len() as i32);
        let prefix_states = prefix.chars().count() as i32;

        let mut a = Automaton::with_capacity(
            (num_states + prefix_states) as usize,
            num_transitions.max(0) as usize,
        );

        // Insert prefix
        let mut last_state = a.create_state();
        for cp in prefix.chars() {
            let state = a.create_state();
            a.add_transition_range(last_state, state, cp as i32, cp as i32);
            last_state = state;
        }

        let state_offset = last_state;
        a.set_accept(last_state, description.is_accept(0));

        // create all states, and mark as accept states if appropriate
        for i in 1..num_states {
            let state = a.create_state();
            a.set_accept(state, description.is_accept(i));
        }

        // NOTE (Lucene): this creates bogus states/transitions (states are final,
        // have self loops, and can't be reached from an init state).

        // create transitions from state to state
        for k in 0..num_states {
            let xpos = description.get_position(k);
            if xpos < 0 {
                continue;
            }
            let end = xpos + (self.word.len() as i32 - xpos).min(range);

            for x in 0..self.alphabet.len() {
                let ch = self.alphabet[x];
                // get the characteristic vector at this position wrt ch
                let cvec = self.get_vector(ch, xpos, end);
                let dest = description.transition(k, xpos, cvec);
                if dest >= 0 {
                    a.add_transition(state_offset + k, state_offset + dest, ch);
                }
            }
            // add transitions for all other chars in unicode
            // by definition, their characteristic vectors are always 0,
            // because they do not exist in the input string.
            let dest = description.transition(k, xpos, 0); // by definition
            if dest >= 0 {
                for r in 0..self.num_ranges {
                    a.add_transition_range(
                        state_offset + k,
                        state_offset + dest,
                        self.range_lower[r],
                        self.range_upper[r],
                    );
                }
            }
        }

        a.finish_state();
        let automaton = Operations::remove_dead_states(&a);
        debug_assert!(automaton.is_deterministic());
        Ok(Some(automaton))
    }

    /// Gets the characteristic vector `X(x, V)` where `V` is `word[pos..end]`.
    pub fn get_vector(&self, x: i32, pos: i32, end: i32) -> i32 {
        let mut vector = 0i32;
        for i in pos..end {
            vector <<= 1;
            if self.word[i as usize] == x {
                vector |= 1;
            }
        }
        vector
    }
}
