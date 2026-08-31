//! Port of `org.apache.lucene.util.automaton.Lev1ParametricDescription`.
//!
//! The tables in this file are code-generated in Lucene by
//! `gradle/generation/moman/createAutomata.py` (the moman/finenight package) and are
//! reproduced here verbatim: the packed data *is* the automaton, so any change to it
//! changes which terms a fuzzy query matches.

use super::levenshtein_automata::{unpack, ParametricDescription};

/// Packed transition data, 1 bits per value.
#[rustfmt::skip]
const OFFSET_INCRS0: &[u64] = &[
    0x0,
];

/// Packed transition data, 1 bits per value.
#[rustfmt::skip]
const OFFSET_INCRS1: &[u64] = &[
    0x38,
];

/// Packed transition data, 2 bits per value.
#[rustfmt::skip]
const OFFSET_INCRS2: &[u64] = &[
    0x5555528000,
];

/// Packed transition data, 2 bits per value.
#[rustfmt::skip]
const OFFSET_INCRS3: &[u64] = &[
    0x555555e80a0f0000, 0x5555,
];

/// Packed transition data, 2 bits per value.
#[rustfmt::skip]
const TO_STATES0: &[u64] = &[
    0x2,
];

/// Packed transition data, 2 bits per value.
#[rustfmt::skip]
const TO_STATES1: &[u64] = &[
    0xa43,
];

/// Packed transition data, 3 bits per value.
#[rustfmt::skip]
const TO_STATES2: &[u64] = &[
    0x4da292442420003,
];

/// Packed transition data, 3 bits per value.
#[rustfmt::skip]
const TO_STATES3: &[u64] = &[
    0x14d0812112018003, 0xb1a29b46d48a49,
];

/// Minimal number of errors for each parametric state.
#[rustfmt::skip]
const MIN_ERRORS: &[i32] = &[0, 1, 0, -1, -1];

/// Parametric description for generating a Levenshtein automaton of degree 1.
///
/// Equivalent to `org.apache.lucene.util.automaton.Lev1ParametricDescription`.
#[derive(Clone, Debug)]
pub struct Lev1ParametricDescription {
    w: i32,
}

impl Lev1ParametricDescription {
    /// Creates the parametric description for a word of length `w`.
    pub fn new(w: i32) -> Self {
        Self { w }
    }
}

impl ParametricDescription for Lev1ParametricDescription {
    fn w(&self) -> i32 {
        self.w
    }

    fn n(&self) -> i32 {
        1
    }

    fn min_errors(&self) -> &'static [i32] {
        MIN_ERRORS
    }

    fn transition(&self, abs_state: i32, position: i32, vector: i32) -> i32 {
        debug_assert!(abs_state != -1, "null absState should never be passed in");

        // decode absState -> state, offset
        let w = self.w;
        let mut state = abs_state / (w + 1);
        let mut offset = abs_state % (w + 1);
        debug_assert!(offset >= 0);

        if position == w {
            if state < 2 {
                let loc = vector * 2 + state;
                offset += unpack(OFFSET_INCRS0, loc, 1);
                state = unpack(TO_STATES0, loc, 2) - 1;
            }
        } else if position == w - 1 {
            if state < 3 {
                let loc = vector * 3 + state;
                offset += unpack(OFFSET_INCRS1, loc, 1);
                state = unpack(TO_STATES1, loc, 2) - 1;
            }
        } else if position == w - 2 {
            if state < 5 {
                let loc = vector * 5 + state;
                offset += unpack(OFFSET_INCRS2, loc, 2);
                state = unpack(TO_STATES2, loc, 3) - 1;
            }
        } else if state < 5 {
            let loc = vector * 5 + state;
            offset += unpack(OFFSET_INCRS3, loc, 2);
            state = unpack(TO_STATES3, loc, 3) - 1;
        }

        if state == -1 {
            // null state
            -1
        } else {
            // translate back to abs
            state * (w + 1) + offset
        }
    }
}
