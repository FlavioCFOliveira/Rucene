//! Port of `org.apache.lucene.internal.hppc.IntHashSet`.

use super::macros::define_hash_set;

define_hash_set! {
    set = IntHashSet,
    key = i32,
    cursor = IntCursor,
    key_zero = 0,
    hash_key = BitMixer::mix_phi_i32,
    mix_key = BitMixer::mix_i32,
    size_of_keys = super::support::size_of_int_array,
    base_ram_bytes_used = 48,
    java_class = "IntHashSet",
    java_key = "int",
}

impl IntHashSet {
    /// New instance copying elements from another collection.
    ///
    /// Equivalent of Java's `IntHashSet(Collection<Integer>)`, which sizes the
    /// set for the collection before copying. `LongHashSet` and `CharHashSet`
    /// have no such constructor in Lucene, and none is added here.
    pub fn from_collection<I>(collection: I) -> Self
    where
        I: IntoIterator<Item = i32>,
        I::IntoIter: ExactSizeIterator,
    {
        let iter = collection.into_iter();
        let mut set = Self::with_expected_elements(iter.len() as i32);
        set.add_all_collection(iter);
        set
    }

    /// Adds all elements from the given collection to this set.
    ///
    /// Equivalent of Java's `addAll(Collection<Integer>)`, which — unlike
    /// [`Self::add_all_array`] — does not pre-size the set. Returns the number
    /// of elements actually added (not previously present in the set).
    pub fn add_all_collection<I: IntoIterator<Item = i32>>(&mut self, collection: I) -> i32 {
        let mut count = 0;
        for element in collection {
            if self.add(element) {
                count += 1;
            }
        }
        count
    }
}
