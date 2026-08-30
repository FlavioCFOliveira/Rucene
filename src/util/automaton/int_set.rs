//! Port of `org.apache.lucene.util.automaton.IntSet`,
//! `org.apache.lucene.util.automaton.FrozenIntSet` and
//! `org.apache.lucene.util.automaton.StateSet`.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use crate::internal::hppc::BitMixer;

/// A set of integers, used to represent a set of NFA states while determinizing.
///
/// Equivalent to `org.apache.lucene.util.automaton.IntSet`.
pub trait IntSet {
    /// Returns an array representation of this int set's values.
    ///
    /// Values are valid for indices `[0, size())`. If this is a mutable int set,
    /// then changes to the set are not guaranteed to be visible in this array.
    fn get_array(&self) -> &[i32];

    /// The number of values in this set.
    ///
    /// Guaranteed to be less than or equal to the length of the slice returned by
    /// [`IntSet::get_array`].
    fn size(&self) -> usize;

    /// The 64-bit hash code of this set, on which equality is also predicated.
    fn long_hash_code(&self) -> i64;
}

/// An immutable snapshot of an [`IntSet`], associated with one determinized state.
///
/// Equivalent to `org.apache.lucene.util.automaton.FrozenIntSet`.
#[derive(Clone, Debug, Eq)]
pub struct FrozenIntSet {
    /// The (sorted) values of this set.
    pub(crate) values: Vec<i32>,
    /// The determinized state this set was frozen for.
    pub(crate) state: i32,
    /// Cached hash code, carried over from the set this was frozen from.
    pub(crate) hash_code: i64,
}

impl FrozenIntSet {
    /// Creates a frozen set from already-sorted `values` with a precomputed hash.
    pub fn new(values: Vec<i32>, hash_code: i64, state: i32) -> Self {
        Self {
            values,
            state,
            hash_code,
        }
    }

    /// Creates a frozen set holding the single value `value`.
    pub fn singleton(value: i32, state: i32) -> Self {
        Self {
            values: vec![value],
            state,
            hash_code: i64::from(BitMixer::mix_i32(value)) + 1,
        }
    }

    /// The determinized state this set was frozen for.
    pub fn state(&self) -> i32 {
        self.state
    }

    /// Sets the determinized state this set is associated with.
    ///
    /// The state takes no part in equality or hashing, exactly as in Lucene, where
    /// `IntSet.equals` only inspects the values.
    pub fn set_state(&mut self, state: i32) {
        self.state = state;
    }
}

impl IntSet for FrozenIntSet {
    fn get_array(&self) -> &[i32] {
        &self.values
    }

    fn size(&self) -> usize {
        self.values.len()
    }

    fn long_hash_code(&self) -> i64 {
        self.hash_code
    }
}

impl PartialEq for FrozenIntSet {
    fn eq(&self, other: &Self) -> bool {
        self.hash_code == other.hash_code && self.values == other.values
    }
}

impl Hash for FrozenIntSet {
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write_i64(self.hash_code);
    }
}

impl std::fmt::Display for FrozenIntSet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.values)
    }
}

/// A mutable, reference-counted set of NFA states.
///
/// Equivalent to `org.apache.lucene.util.automaton.StateSet`. Adding a state that
/// is already present increases its reference count; removing it decreases the
/// count and only drops the state once the count reaches zero.
#[derive(Clone, Debug, Default)]
pub struct StateSet {
    inner: HashMap<i32, i32>,
    hash_code: i64,
    hash_updated: bool,
    array_updated: bool,
    array_cache: Vec<i32>,
}

impl StateSet {
    /// Creates an empty set with room for `capacity` states.
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: HashMap::with_capacity(capacity),
            hash_code: 0,
            hash_updated: true,
            array_updated: true,
            array_cache: Vec::new(),
        }
    }

    /// Adds the state into this set; if it is already there, increases its
    /// reference count by one.
    pub fn incr(&mut self, state: i32) {
        let count = self.inner.entry(state).or_insert(0);
        *count += 1;
        if *count == 1 {
            self.key_changed();
        }
    }

    /// Decreases the reference count of the state; when the count reaches zero the
    /// state is removed from this set.
    ///
    /// # Panics
    ///
    /// Panics if `state` is not a member of this set.
    pub fn decr(&mut self, state: i32) {
        let count = self
            .inner
            .get_mut(&state)
            .expect("INVARIANT: decr is only called for states previously incr'd");
        *count -= 1;
        if *count == 0 {
            self.inner.remove(&state);
            self.key_changed();
        }
    }

    /// Removes every state from this set.
    pub fn reset(&mut self) {
        self.inner.clear();
        self.key_changed();
    }

    /// Creates a snapshot of this int set associated with a given state.
    ///
    /// The snapshot does not retain any frequency information about the elements of
    /// this set, only existence.
    pub fn freeze(&mut self, state: i32) -> FrozenIntSet {
        FrozenIntSet::new(self.get_array().to_vec(), self.long_hash_code(), state)
    }

    fn key_changed(&mut self) {
        self.hash_updated = false;
        self.array_updated = false;
    }

    /// Returns the sorted array of the states in this set, recomputing it if the
    /// set changed since the last call.
    pub fn get_array(&mut self) -> &[i32] {
        if self.array_updated {
            return &self.array_cache;
        }
        self.array_cache.clear();
        self.array_cache.extend(self.inner.keys().copied());
        // The array must be sorted since "equals" depends on it.
        self.array_cache.sort_unstable();
        self.array_updated = true;
        &self.array_cache
    }

    /// The number of states in this set.
    pub fn size(&self) -> usize {
        self.inner.len()
    }

    /// Returns the hash code of this set, recomputing it if the set changed since
    /// the last call.
    pub fn long_hash_code(&mut self) -> i64 {
        if self.hash_updated {
            return self.hash_code;
        }
        let mut hash_code = self.inner.len() as i64;
        for key in self.inner.keys() {
            hash_code = hash_code.wrapping_add(i64::from(BitMixer::mix_i32(*key)));
        }
        self.hash_code = hash_code;
        self.hash_updated = true;
        hash_code
    }
}
