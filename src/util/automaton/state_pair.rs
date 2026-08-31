//! Port of `org.apache.lucene.util.automaton.StatePair`.

/// Pair of states.
///
/// Equivalent to `org.apache.lucene.util.automaton.StatePair`.
#[derive(Clone, Copy, Debug)]
pub struct StatePair {
    /// The state in the product automaton these two states map to; `-1` until it
    /// has been assigned. Only Mike knows what it does (do not expose).
    pub(crate) s: i32,
    /// First state.
    pub s1: i32,
    /// Second state.
    pub s2: i32,
}

impl StatePair {
    /// Constructs a new state pair.
    pub fn new(s1: i32, s2: i32) -> Self {
        Self { s: -1, s1, s2 }
    }

    /// Constructs a new state pair that already maps to the product state `s`.
    pub(crate) fn with_state(s: i32, s1: i32, s2: i32) -> Self {
        Self { s, s1, s2 }
    }
}

impl PartialEq for StatePair {
    fn eq(&self, other: &Self) -> bool {
        self.s1 == other.s1 && self.s2 == other.s2
    }
}

impl Eq for StatePair {}

impl std::hash::Hash for StatePair {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        // Don't use s1 ^ s2 since it's vulnerable to the case where s1 == s2 always
        // --> hashCode = 0, e.g. if you call AutomatonTestUtil.sameLanguage passing
        // the same automaton against itself.
        state.write_i32(self.s1.wrapping_mul(31).wrapping_add(self.s2));
    }
}

impl std::fmt::Display for StatePair {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "StatePair(s1={} s2={})", self.s1, self.s2)
    }
}
