//! Code templates shared by the `org.apache.lucene.internal.hppc` containers.
//!
//! This module has no counterpart in Lucene. HPPC generates its specialised
//! containers from a single Velocity template per container *kind*, and Lucene
//! checked the generated Java files into its tree; the containers are therefore
//! byte-for-byte identical apart from the key and value types. Rust's
//! declarative macros play the role of that generator, so each container in
//! this module remains a distinct, monomorphic type with Lucene's own name —
//! exactly as in Lucene — without the body being written out nine times.
//!
//! Each macro is invoked exactly once, from the module named after the Lucene
//! class it produces.

/// Expands to a hash map of a primitive key type to a primitive value type.
///
/// Covers `IntIntHashMap`, `IntFloatHashMap`, `IntDoubleHashMap`,
/// `IntLongHashMap`, `LongIntHashMap` and `LongFloatHashMap`.
macro_rules! define_primitive_hash_map {
    (
        map = $map:ident,
        cursor = $cursor:ident,
        key = $kt:ty,
        value = $vt:ty,
        key_cursor = $kcur:ident,
        value_cursor = $vcur:ident,
        key_zero = $kzero:expr,
        value_zero = $vzero:expr,
        hash_key = $hash_key:path,
        mix_key = $mix_key:path,
        mix_value = $mix_value:path,
        eq_value = $eq_value:path,
        add_value = $add_value:path,
        size_of_keys = $size_of_keys:path,
        size_of_values = $size_of_values:path,
        base_ram_bytes_used = $base_ram:expr,
        java_class = $java:literal,
        java_key = $jkey:literal,
        java_value = $jvalue:literal,
        value_fmt = $vfmt:literal,
    ) => {
        use std::fmt::{self, Debug, Display, Formatter};
        use std::hash::{Hash, Hasher};
        use std::sync::atomic::{AtomicI32, Ordering};

        use super::abstract_iterator::AbstractIterator;
        use super::bit_mixer::BitMixer;
        use super::buffer_allocation_exception::BufferAllocationException;
        use super::hash_containers::{
            check_power_of_two, check_load_factor, expand_at_count, iteration_increment,
            min_buffer_size, next_buffer_size, next_iteration_seed, DEFAULT_EXPECTED_ELEMENTS,
            DEFAULT_LOAD_FACTOR, MAX_LOAD_FACTOR, MIN_LOAD_FACTOR,
        };
        use super::support::group_digits;
        use crate::util::Accountable;

        #[doc = concat!("Shallow size of a `", $java, "` instance, as `RamUsageEstimator.shallowSizeOfInstance` computes it.")]
        const BASE_RAM_BYTES_USED: i64 = $base_ram;

        #[doc = concat!("Port of `org.apache.lucene.internal.hppc.", $java, "`.")]
        ///
        #[doc = concat!("A hash map of `", $jkey, "` to `", $jvalue, "`, implemented using open addressing with linear probing for collision resolution.")]
        ///
        /// Lucene forked and trimmed this from HPPC 0.10.0
        #[doc = concat!("(`com.carrotsearch.hppc.", $java, "`).")]
        ///
        /// # Iteration order
        ///
        /// As in Lucene, the iteration order is deliberately randomised: every
        /// container starts from a distinct seed and every iterator advances it
        /// again, so no caller can come to depend on a particular order.
        pub struct $map {
            /// The array holding keys.
            pub keys: Vec<$kt>,

            /// The array holding values.
            pub values: Vec<$vt>,

            /// The number of stored keys (assigned key slots), excluding the
            /// special "empty" key, if any (use [`Self::size`] instead).
            assigned: i32,

            /// Mask for slot scans in [`Self::keys`].
            mask: i32,

            /// Expand (rehash) [`Self::keys`] when `assigned` hits this value.
            resize_at: i32,

            /// Special treatment for the "empty slot" key marker.
            has_empty_key: bool,

            /// The load factor for [`Self::keys`].
            load_factor: f64,

            /// Seed ensuring the hash iteration order differs from one
            /// iteration to another.
            ///
            /// Java uses a plain `int` field; this port uses an atomic so that
            /// iteration, which only borrows the map, can still advance the
            /// seed. Relaxed ordering suffices for exactly the reason Lucene
            /// gives: nothing depends on the value beyond each thread seeing a
            /// sequence of varying seeds.
            iteration_seed: AtomicI32,
        }

        impl $map {
            /// New instance with sane defaults.
            pub fn new() -> Self {
                Self::with_expected_elements(DEFAULT_EXPECTED_ELEMENTS)
            }

            /// New instance with sane defaults.
            ///
            /// `expected_elements` is the expected number of elements
            /// guaranteed not to cause buffer expansion (inclusive).
            pub fn with_expected_elements(expected_elements: i32) -> Self {
                Self::with_expected_elements_and_load_factor(
                    expected_elements,
                    DEFAULT_LOAD_FACTOR as f64,
                )
            }

            /// New instance with the provided defaults.
            ///
            /// # Panics
            ///
            /// Panics with a [`BufferAllocationException`] if `load_factor` is
            /// insane (zero, or full capacity), as Java's
            /// `verifyLoadFactor` does.
            pub fn with_expected_elements_and_load_factor(
                expected_elements: i32,
                load_factor: f64,
            ) -> Self {
                let mut map = Self {
                    keys: Vec::new(),
                    values: Vec::new(),
                    assigned: 0,
                    mask: 0,
                    resize_at: 0,
                    has_empty_key: false,
                    load_factor: Self::verify_load_factor(load_factor),
                    iteration_seed: AtomicI32::new(next_iteration_seed()),
                };
                map.ensure_capacity(expected_elements);
                map
            }

            #[doc = concat!("Creates a hash map from all key-value pairs of another `", $java, "`.")]
            ///
            /// Equivalent of Java's copy constructor, which rebuilds the table
            /// rather than copying its layout the way [`Clone`] does.
            pub fn from_map(map: &Self) -> Self {
                let mut copy = Self::with_expected_elements(map.size());
                copy.put_all(map.iter());
                copy
            }

            /// Associates `value` with `key`, returning the previous value, or
            #[doc = concat!("`", stringify!($vzero), "` if the key was absent.")]
            pub fn put(&mut self, key: $kt, value: $vt) -> $vt {
                debug_assert!(self.assigned < self.mask + 1);

                let mask = self.mask;
                if key == $kzero {
                    let previous_value = if self.has_empty_key {
                        self.values[(mask + 1) as usize]
                    } else {
                        $vzero
                    };
                    self.has_empty_key = true;
                    self.values[(mask + 1) as usize] = value;
                    previous_value
                } else {
                    let mut slot = Self::hash_key(key) & mask;

                    loop {
                        let existing = self.keys[slot as usize];
                        if existing == $kzero {
                            break;
                        }
                        if existing == key {
                            let previous_value = self.values[slot as usize];
                            self.values[slot as usize] = value;
                            return previous_value;
                        }
                        slot = (slot + 1) & mask;
                    }

                    if self.assigned == self.resize_at {
                        self.allocate_then_insert_then_rehash(slot, key, value);
                    } else {
                        self.keys[slot as usize] = key;
                        self.values[slot as usize] = value;
                    }

                    self.assigned += 1;
                    $vzero
                }
            }

            /// Puts every cursor of `iterable` into this map, returning the
            /// number of entries the map gained.
            pub fn put_all<I: IntoIterator<Item = $cursor>>(&mut self, iterable: I) -> i32 {
                let count = self.size();
                for c in iterable {
                    self.put(c.key, c.value);
                }
                self.size() - count
            }

            /// Trove-inspired API method, an equivalent of
            /// `if (!map.containsKey(key)) map.put(key, value);`.
            ///
            /// Returns `true` if `key` did not exist and `value` was placed in
            /// the map.
            pub fn put_if_absent(&mut self, key: $kt, value: $vt) -> bool {
                let key_index = self.index_of(key);
                if !self.index_exists(key_index) {
                    self.index_insert(key_index, key, value);
                    true
                } else {
                    false
                }
            }

            /// If `key` exists, its value is incremented by `increment_value`;
            /// otherwise `put_value` is inserted.
            ///
            /// Returns the current value associated with `key` (after changes).
            pub fn put_or_add(&mut self, key: $kt, put_value: $vt, increment_value: $vt) -> $vt {
                debug_assert!(self.assigned < self.mask + 1);

                let mut put_value = put_value;
                let key_index = self.index_of(key);
                if self.index_exists(key_index) {
                    put_value = $add_value(self.values[key_index as usize], increment_value);
                    self.index_replace(key_index, put_value);
                } else {
                    self.index_insert(key_index, key, put_value);
                }
                put_value
            }

            /// Adds `increment_value` to any existing value for `key`, or
            /// inserts `increment_value` if `key` did not previously exist.
            ///
            /// Returns the current value associated with `key` (after changes).
            pub fn add_to(&mut self, key: $kt, increment_value: $vt) -> $vt {
                self.put_or_add(key, increment_value, increment_value)
            }

            /// Removes `key`, returning the value it held, or
            #[doc = concat!("`", stringify!($vzero), "` if it was absent.")]
            pub fn remove(&mut self, key: $kt) -> $vt {
                let mask = self.mask;
                if key == $kzero {
                    if !self.has_empty_key {
                        return $vzero;
                    }
                    self.has_empty_key = false;
                    let previous_value = self.values[(mask + 1) as usize];
                    self.values[(mask + 1) as usize] = $vzero;
                    previous_value
                } else {
                    let mut slot = Self::hash_key(key) & mask;

                    loop {
                        let existing = self.keys[slot as usize];
                        if existing == $kzero {
                            break;
                        }
                        if existing == key {
                            let previous_value = self.values[slot as usize];
                            self.shift_conflicting_keys(slot);
                            return previous_value;
                        }
                        slot = (slot + 1) & mask;
                    }

                    $vzero
                }
            }

            /// Returns the value associated with `key`, or
            #[doc = concat!("`", stringify!($vzero), "` if the key is absent.")]
            pub fn get(&self, key: $kt) -> $vt {
                if key == $kzero {
                    if self.has_empty_key {
                        self.values[(self.mask + 1) as usize]
                    } else {
                        $vzero
                    }
                } else {
                    let mask = self.mask;
                    let mut slot = Self::hash_key(key) & mask;

                    loop {
                        let existing = self.keys[slot as usize];
                        if existing == $kzero {
                            break;
                        }
                        if existing == key {
                            return self.values[slot as usize];
                        }
                        slot = (slot + 1) & mask;
                    }

                    $vzero
                }
            }

            /// Returns the value associated with `key`, or `default_value` if
            /// the key is absent.
            pub fn get_or_default(&self, key: $kt, default_value: $vt) -> $vt {
                if key == $kzero {
                    if self.has_empty_key {
                        self.values[(self.mask + 1) as usize]
                    } else {
                        default_value
                    }
                } else {
                    let mask = self.mask;
                    let mut slot = Self::hash_key(key) & mask;

                    loop {
                        let existing = self.keys[slot as usize];
                        if existing == $kzero {
                            break;
                        }
                        if existing == key {
                            return self.values[slot as usize];
                        }
                        slot = (slot + 1) & mask;
                    }

                    default_value
                }
            }

            /// Returns `true` if `key` is present in this map.
            pub fn contains_key(&self, key: $kt) -> bool {
                if key == $kzero {
                    self.has_empty_key
                } else {
                    let mask = self.mask;
                    let mut slot = Self::hash_key(key) & mask;

                    loop {
                        let existing = self.keys[slot as usize];
                        if existing == $kzero {
                            break;
                        }
                        if existing == key {
                            return true;
                        }
                        slot = (slot + 1) & mask;
                    }

                    false
                }
            }

            /// Returns a logical "index" of `key`, usable to speed up follow-up
            /// logic.
            ///
            /// The result is non-negative when the key exists and the bitwise
            /// complement of the insertion slot when it does not. Indexes are
            /// valid only between modifications of the map.
            pub fn index_of(&self, key: $kt) -> i32 {
                let mask = self.mask;
                if key == $kzero {
                    if self.has_empty_key {
                        mask + 1
                    } else {
                        !(mask + 1)
                    }
                } else {
                    let mut slot = Self::hash_key(key) & mask;

                    loop {
                        let existing = self.keys[slot as usize];
                        if existing == $kzero {
                            break;
                        }
                        if existing == key {
                            return slot;
                        }
                        slot = (slot + 1) & mask;
                    }

                    !slot
                }
            }

            /// Returns `true` if `index`, as returned by [`Self::index_of`],
            /// corresponds to an existing key.
            pub fn index_exists(&self, index: i32) -> bool {
                debug_assert!(
                    index < 0 || index <= self.mask || (index == self.mask + 1 && self.has_empty_key)
                );

                index >= 0
            }

            /// Returns the value stored at an existing `index`.
            ///
            /// # Panics
            ///
            /// With debug assertions enabled, panics if `index` does not point
            /// at an existing key, exactly as Java's assertions do.
            pub fn index_get(&self, index: i32) -> $vt {
                debug_assert!(index >= 0, "The index must point at an existing key.");
                debug_assert!(
                    index <= self.mask || (index == self.mask + 1 && self.has_empty_key)
                );

                self.values[index as usize]
            }

            /// Replaces the value stored at an existing `index`, returning the
            /// previous one.
            ///
            /// # Panics
            ///
            /// With debug assertions enabled, panics if `index` does not point
            /// at an existing key.
            pub fn index_replace(&mut self, index: i32, new_value: $vt) -> $vt {
                debug_assert!(index >= 0, "The index must point at an existing key.");
                debug_assert!(
                    index <= self.mask || (index == self.mask + 1 && self.has_empty_key)
                );

                let previous_value = self.values[index as usize];
                self.values[index as usize] = new_value;
                previous_value
            }

            /// Inserts a key-value pair at an `index` that is not present in
            /// the map, avoiding a second hash computation.
            ///
            /// # Panics
            ///
            /// With debug assertions enabled, panics if `index` points at an
            /// existing key.
            pub fn index_insert(&mut self, index: i32, key: $kt, value: $vt) {
                debug_assert!(index < 0, "The index must not point at an existing key.");

                let index = !index;
                if key == $kzero {
                    debug_assert!(index == self.mask + 1);
                    self.values[index as usize] = value;
                    self.has_empty_key = true;
                } else {
                    debug_assert!(self.keys[index as usize] == $kzero);

                    if self.assigned == self.resize_at {
                        self.allocate_then_insert_then_rehash(index, key, value);
                    } else {
                        self.keys[index as usize] = key;
                        self.values[index as usize] = value;
                    }

                    self.assigned += 1;
                }
            }

            /// Removes the key at an existing `index`, returning its value.
            ///
            /// # Panics
            ///
            /// With debug assertions enabled, panics if `index` does not point
            /// at an existing key.
            pub fn index_remove(&mut self, index: i32) -> $vt {
                debug_assert!(index >= 0, "The index must point at an existing key.");
                debug_assert!(
                    index <= self.mask || (index == self.mask + 1 && self.has_empty_key)
                );

                let previous_value = self.values[index as usize];
                if index > self.mask {
                    debug_assert!(index == self.mask + 1);
                    self.has_empty_key = false;
                    self.values[index as usize] = $vzero;
                } else {
                    self.shift_conflicting_keys(index);
                }
                previous_value
            }

            /// Removes every entry, keeping the internal buffers.
            pub fn clear(&mut self) {
                self.assigned = 0;
                self.has_empty_key = false;

                self.keys.fill($kzero);
            }

            /// Removes every entry and releases the internal buffers, sizing
            /// them back down to the default capacity.
            pub fn release(&mut self) {
                self.assigned = 0;
                self.has_empty_key = false;

                self.keys = Vec::new();
                self.values = Vec::new();
                self.ensure_capacity(DEFAULT_EXPECTED_ELEMENTS);
            }

            /// Returns the number of entries in this map.
            pub fn size(&self) -> i32 {
                self.assigned + if self.has_empty_key { 1 } else { 0 }
            }

            /// Returns `true` if this map holds no entries.
            pub fn is_empty(&self) -> bool {
                self.size() == 0
            }

            /// Equivalent of Java's `hashCode()`, reproduced exactly.
            ///
            /// The value is order-independent, so it survives this container's
            /// randomised iteration order.
            pub fn hash_code(&self) -> i32 {
                let mut h: i32 = if self.has_empty_key {
                    0xDEAD_BEEF_u32 as i32
                } else {
                    0
                };
                for c in self.iter() {
                    h = h.wrapping_add($mix_key(c.key).wrapping_add($mix_value(c.value)));
                }
                h
            }

            /// Returns `true` if every key of `other` exists in this map with
            /// the same value.
            fn equal_elements(&self, other: &Self) -> bool {
                if other.size() != self.size() {
                    return false;
                }

                for c in other.iter() {
                    let key = c.key;
                    if !self.contains_key(key) || !$eq_value(self.get(key), c.value) {
                        return false;
                    }
                }

                true
            }

            /// Ensures this container can hold at least `expected_elements`
            /// entries without resizing its buffers.
            pub fn ensure_capacity(&mut self, expected_elements: i32) {
                if expected_elements > self.resize_at || self.keys.is_empty() {
                    let prev_keys = std::mem::take(&mut self.keys);
                    let prev_values = std::mem::take(&mut self.values);
                    self.allocate_buffers(min_buffer_size(expected_elements, self.load_factor));
                    if !prev_keys.is_empty() && !self.is_empty() {
                        self.rehash(&prev_keys, &prev_values);
                    }
                }
            }

            /// Provides the next iteration seed used to build the iteration
            /// starting slot and offset increment.
            fn next_iteration_seed(&self) -> i32 {
                let seed = BitMixer::mix_phi_i32(self.iteration_seed.load(Ordering::Relaxed));
                self.iteration_seed.store(seed, Ordering::Relaxed);
                seed
            }

            /// Returns an iterator over the entries of this map.
            pub fn iter(&self) -> EntryIterator<'_> {
                EntryIterator::new(self)
            }

            /// Returns a specialized view of the keys of this associated
            /// container.
            pub fn keys(&self) -> KeysContainer<'_> {
                KeysContainer { owner: self }
            }

            /// Returns a container with all values stored in this map.
            pub fn values(&self) -> ValuesContainer<'_> {
                ValuesContainer { owner: self }
            }

            /// Creates a hash map from two index-aligned arrays of key-value
            /// pairs.
            ///
            /// # Panics
            ///
            /// Panics if the two slices have different lengths, as Java's
            /// `IllegalArgumentException` does.
            #[allow(clippy::should_implement_trait)]
            pub fn from(keys: &[$kt], values: &[$vt]) -> Self {
                if keys.len() != values.len() {
                    panic!("Arrays of keys and values must have an identical length.");
                }

                let mut map = Self::with_expected_elements(keys.len() as i32);
                for (key, value) in keys.iter().zip(values.iter()) {
                    map.put(*key, *value);
                }

                map
            }

            /// Returns a hash code for `key`, distributing keys evenly across
            /// the entire integer range.
            fn hash_key(key: $kt) -> i32 {
                debug_assert!(key != $kzero); // Handled as a special case (empty slot marker).
                $hash_key(key)
            }

            /// Validates the load factor range and returns it.
            fn verify_load_factor(load_factor: f64) -> f64 {
                check_load_factor(load_factor, MIN_LOAD_FACTOR as f64, MAX_LOAD_FACTOR as f64);
                load_factor
            }

            /// Rehashes from old buffers into the current ones.
            fn rehash(&mut self, from_keys: &[$kt], from_values: &[$vt]) {
                debug_assert!(
                    from_keys.len() == from_values.len()
                        && check_power_of_two(from_keys.len() as i32 - 1)
                );

                let mask = self.mask;

                // Copy the zero element's slot, then rehash everything else.
                let mut from = from_keys.len() - 1;
                let last = self.keys.len() - 1;
                self.keys[last] = from_keys[from];
                self.values[last] = from_values[from];
                while from > 0 {
                    from -= 1;
                    let existing = from_keys[from];
                    if existing != $kzero {
                        let mut slot = Self::hash_key(existing) & mask;
                        while self.keys[slot as usize] != $kzero {
                            slot = (slot + 1) & mask;
                        }
                        self.keys[slot as usize] = existing;
                        self.values[slot as usize] = from_values[from];
                    }
                }
            }

            /// Allocates new internal buffers, atomically: either both
            /// allocations succeed, or the map is left untouched.
            ///
            /// # Panics
            ///
            /// Panics with a [`BufferAllocationException`] when the allocator
            /// cannot satisfy the request, which is where Java catches
            /// `OutOfMemoryError` and throws the same exception.
            fn allocate_buffers(&mut self, array_size: i32) {
                debug_assert!(array_size.count_ones() == 1);

                // An extra slot holds the value of the "empty" key.
                let length = array_size as usize + 1;
                let mut new_keys: Vec<$kt> = Vec::new();
                let mut new_values: Vec<$vt> = Vec::new();
                if new_keys.try_reserve_exact(length).is_err()
                    || new_values.try_reserve_exact(length).is_err()
                {
                    BufferAllocationException::new(format!(
                        "Not enough memory to allocate buffers for rehashing: {} -> {}",
                        group_digits((self.mask + 1) as i64),
                        group_digits(array_size as i64)
                    ))
                    .throw();
                }
                new_keys.resize(length, $kzero);
                new_values.resize(length, $vzero);
                self.keys = new_keys;
                self.values = new_values;

                self.resize_at = expand_at_count(array_size, self.load_factor);
                self.mask = array_size - 1;
            }

            /// Invoked when a new key-value pair must be inserted but there are
            /// not enough empty slots.
            ///
            /// New buffers are allocated first; once that succeeds the pending
            /// element is assigned into the previous buffer (possibly violating
            /// the invariant of having at least one empty slot) and all keys
            /// are rehashed into the new buffers.
            fn allocate_then_insert_then_rehash(
                &mut self,
                slot: i32,
                pending_key: $kt,
                pending_value: $vt,
            ) {
                debug_assert!(
                    self.assigned == self.resize_at
                        && self.keys[slot as usize] == $kzero
                        && pending_key != $kzero
                );

                // Try to allocate new buffers first. On failure we stay consistent.
                let mut prev_keys = std::mem::take(&mut self.keys);
                let mut prev_values = std::mem::take(&mut self.values);
                let next = next_buffer_size(self.mask + 1, self.size(), self.load_factor);
                self.allocate_buffers(next);
                debug_assert!(self.keys.len() > prev_keys.len());

                // We have succeeded at allocating new data so insert the
                // pending key/value at the free slot in the old arrays before
                // rehashing.
                prev_keys[slot as usize] = pending_key;
                prev_values[slot as usize] = pending_value;

                // Rehash old keys, including the pending key.
                self.rehash(&prev_keys, &prev_values);
            }

            /// Shifts all the slot-conflicting keys and values allocated to
            /// (and including) `gap_slot`.
            fn shift_conflicting_keys(&mut self, gap_slot: i32) {
                let mut gap_slot = gap_slot;
                let mask = self.mask;

                // Perform shifts of conflicting keys to fill in the gap.
                let mut distance = 0i32;
                loop {
                    distance += 1;
                    let slot = (gap_slot.wrapping_add(distance)) & mask;
                    let existing = self.keys[slot as usize];
                    if existing == $kzero {
                        break;
                    }

                    let ideal_slot = Self::hash_key(existing);
                    let shift = slot.wrapping_sub(ideal_slot) & mask;
                    if shift >= distance {
                        // Entry at this position was originally at or before the
                        // gap slot. Move the conflict-shifted entry to the gap's
                        // position and repeat the procedure for any entries to
                        // the right of the current position, treating it as the
                        // new gap.
                        let moved_value = self.values[slot as usize];
                        self.keys[gap_slot as usize] = existing;
                        self.values[gap_slot as usize] = moved_value;
                        gap_slot = slot;
                        distance = 0;
                    }
                }

                // Mark the last found gap slot without a conflict as empty.
                self.keys[gap_slot as usize] = $kzero;
                self.values[gap_slot as usize] = $vzero;
                self.assigned -= 1;
            }
        }

        impl Default for $map {
            fn default() -> Self {
                Self::new()
            }
        }

        impl Clone for $map {
            /// Clones this map, reusing the same hash function and array
            /// resizing strategy but drawing a fresh iteration seed, exactly as
            /// Java's `clone()` does.
            fn clone(&self) -> Self {
                Self {
                    keys: self.keys.clone(),
                    values: self.values.clone(),
                    assigned: self.assigned,
                    mask: self.mask,
                    resize_at: self.resize_at,
                    has_empty_key: self.has_empty_key,
                    load_factor: self.load_factor,
                    iteration_seed: AtomicI32::new(next_iteration_seed()),
                }
            }
        }

        impl PartialEq for $map {
            fn eq(&self, other: &Self) -> bool {
                std::ptr::eq(self, other) || self.equal_elements(other)
            }
        }

        impl Eq for $map {}

        impl Hash for $map {
            fn hash<H: Hasher>(&self, state: &mut H) {
                state.write_i32(self.hash_code());
            }
        }

        impl Accountable for $map {
            fn ram_bytes_used(&self) -> i64 {
                BASE_RAM_BYTES_USED
                    + $size_of_keys(self.keys.len())
                    + $size_of_values(self.values.len())
            }
        }

        impl Display for $map {
            /// Converts the contents of this map to a human-friendly string.
            fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
                f.write_str("[")?;
                let mut first = true;
                for cursor in self.iter() {
                    if !first {
                        f.write_str(", ")?;
                    }
                    write!(f, concat!("{}=>{", $vfmt, "}"), cursor.key, cursor.value)?;
                    first = false;
                }
                f.write_str("]")
            }
        }

        impl Debug for $map {
            fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
                write!(f, "{}{}", stringify!($map), self)
            }
        }

        impl<'a> IntoIterator for &'a $map {
            type Item = $cursor;
            type IntoIter = EntryIterator<'a>;

            fn into_iter(self) -> Self::IntoIter {
                self.iter()
            }
        }

        #[doc = concat!("Port of `", $java, ".", stringify!($cursor), "`.")]
        ///
        /// Forked by Lucene from HPPC, holding an `int` index together with the
        /// entry's key and value.
        #[derive(Debug, Clone, Copy, Default, PartialEq)]
        pub struct $cursor {
            /// The current key and value's index in the container this cursor
            /// belongs to.
            ///
            /// The meaning of this index is defined by the container (usually
            /// it will be an index in the underlying storage buffer).
            pub index: i32,

            /// The current key.
            pub key: $kt,

            /// The current value.
            pub value: $vt,
        }

        impl Display for $cursor {
            fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
                write!(
                    f,
                    concat!("[cursor, index: {}, key: {}, value: {", $vfmt, "}]"),
                    self.index, self.key, self.value
                )
            }
        }

        #[doc = concat!("Iterator over the entries of a [`", stringify!($map), "`].")]
        ///
        #[doc = concat!("Port of `", $java, ".EntryIterator`.")]
        pub struct EntryIterator<'a> {
            owner: &'a $map,
            increment: i32,
            index: i32,
            slot: i32,
        }

        impl<'a> EntryIterator<'a> {
            fn new(owner: &'a $map) -> Self {
                let seed = owner.next_iteration_seed();
                Self {
                    owner,
                    increment: iteration_increment(seed),
                    index: 0,
                    slot: seed & owner.mask,
                }
            }
        }

        impl AbstractIterator for EntryIterator<'_> {
            type Item = $cursor;

            fn fetch(&mut self) -> Option<$cursor> {
                let mask = self.owner.mask;
                while self.index <= mask {
                    self.index += 1;
                    self.slot = (self.slot + self.increment) & mask;
                    let existing = self.owner.keys[self.slot as usize];
                    if existing != $kzero {
                        return Some($cursor {
                            index: self.slot,
                            key: existing,
                            value: self.owner.values[self.slot as usize],
                        });
                    }
                }

                if self.index == mask + 1 && self.owner.has_empty_key {
                    let index = self.index;
                    self.index += 1;
                    return Some($cursor {
                        index,
                        key: $kzero,
                        value: self.owner.values[index as usize],
                    });
                }

                self.done()
            }
        }

        impl Iterator for EntryIterator<'_> {
            type Item = $cursor;

            fn next(&mut self) -> Option<$cursor> {
                self.fetch()
            }
        }

        #[doc = concat!("A view of the keys inside a [`", stringify!($map), "`].")]
        ///
        #[doc = concat!("Port of `", $java, ".KeysContainer`.")]
        #[derive(Debug, Clone, Copy)]
        pub struct KeysContainer<'a> {
            owner: &'a $map,
        }

        impl<'a> KeysContainer<'a> {
            /// Returns the number of keys in the view.
            pub fn size(&self) -> i32 {
                self.owner.size()
            }

            /// Copies every key into a freshly allocated array.
            pub fn to_array(&self) -> Vec<$kt> {
                let mut array = Vec::with_capacity(self.size() as usize);
                for cursor in self.iter() {
                    array.push(cursor.value);
                }
                array
            }

            /// Returns an iterator over the keys.
            pub fn iter(&self) -> KeysIterator<'a> {
                KeysIterator::new(self.owner)
            }
        }

        impl<'a> IntoIterator for KeysContainer<'a> {
            type Item = super::$kcur;
            type IntoIter = KeysIterator<'a>;

            fn into_iter(self) -> Self::IntoIter {
                KeysIterator::new(self.owner)
            }
        }

        #[doc = concat!("Iterator over the assigned keys of a [`", stringify!($map), "`].")]
        ///
        #[doc = concat!("Port of `", $java, ".KeysIterator`.")]
        pub struct KeysIterator<'a> {
            owner: &'a $map,
            increment: i32,
            index: i32,
            slot: i32,
        }

        impl<'a> KeysIterator<'a> {
            fn new(owner: &'a $map) -> Self {
                let seed = owner.next_iteration_seed();
                Self {
                    owner,
                    increment: iteration_increment(seed),
                    index: 0,
                    slot: seed & owner.mask,
                }
            }
        }

        impl AbstractIterator for KeysIterator<'_> {
            type Item = super::$kcur;

            fn fetch(&mut self) -> Option<super::$kcur> {
                let mask = self.owner.mask;
                while self.index <= mask {
                    self.index += 1;
                    self.slot = (self.slot + self.increment) & mask;
                    let existing = self.owner.keys[self.slot as usize];
                    if existing != $kzero {
                        return Some(super::$kcur {
                            index: self.slot,
                            value: existing,
                        });
                    }
                }

                if self.index == mask + 1 && self.owner.has_empty_key {
                    let index = self.index;
                    self.index += 1;
                    return Some(super::$kcur {
                        index,
                        value: $kzero,
                    });
                }

                self.done()
            }
        }

        impl Iterator for KeysIterator<'_> {
            type Item = super::$kcur;

            fn next(&mut self) -> Option<super::$kcur> {
                self.fetch()
            }
        }

        #[doc = concat!("A view over the values of a [`", stringify!($map), "`].")]
        ///
        #[doc = concat!("Port of `", $java, ".ValuesContainer`.")]
        #[derive(Debug, Clone, Copy)]
        pub struct ValuesContainer<'a> {
            owner: &'a $map,
        }

        impl<'a> ValuesContainer<'a> {
            /// Returns the number of values in the view.
            pub fn size(&self) -> i32 {
                self.owner.size()
            }

            /// Copies every value into a freshly allocated array.
            pub fn to_array(&self) -> Vec<$vt> {
                let mut array = Vec::with_capacity(self.size() as usize);
                for cursor in self.iter() {
                    array.push(cursor.value);
                }
                array
            }

            /// Returns an iterator over the values.
            pub fn iter(&self) -> ValuesIterator<'a> {
                ValuesIterator::new(self.owner)
            }
        }

        impl<'a> IntoIterator for ValuesContainer<'a> {
            type Item = super::$vcur;
            type IntoIter = ValuesIterator<'a>;

            fn into_iter(self) -> Self::IntoIter {
                ValuesIterator::new(self.owner)
            }
        }

        #[doc = concat!("Iterator over the assigned values of a [`", stringify!($map), "`].")]
        ///
        #[doc = concat!("Port of `", $java, ".ValuesIterator`.")]
        pub struct ValuesIterator<'a> {
            owner: &'a $map,
            increment: i32,
            index: i32,
            slot: i32,
        }

        impl<'a> ValuesIterator<'a> {
            fn new(owner: &'a $map) -> Self {
                let seed = owner.next_iteration_seed();
                Self {
                    owner,
                    increment: iteration_increment(seed),
                    index: 0,
                    slot: seed & owner.mask,
                }
            }
        }

        impl AbstractIterator for ValuesIterator<'_> {
            type Item = super::$vcur;

            fn fetch(&mut self) -> Option<super::$vcur> {
                let mask = self.owner.mask;
                while self.index <= mask {
                    self.index += 1;
                    self.slot = (self.slot + self.increment) & mask;
                    if self.owner.keys[self.slot as usize] != $kzero {
                        return Some(super::$vcur {
                            index: self.slot,
                            value: self.owner.values[self.slot as usize],
                        });
                    }
                }

                if self.index == mask + 1 && self.owner.has_empty_key {
                    let index = self.index;
                    self.index += 1;
                    return Some(super::$vcur {
                        index,
                        value: self.owner.values[index as usize],
                    });
                }

                self.done()
            }
        }

        impl Iterator for ValuesIterator<'_> {
            type Item = super::$vcur;

            fn next(&mut self) -> Option<super::$vcur> {
                self.fetch()
            }
        }
    };
}

pub(crate) use define_primitive_hash_map;

/// Expands to a hash map of a primitive key type to an arbitrary value type.
///
/// Covers `IntObjectHashMap`, `LongObjectHashMap` and `CharObjectHashMap`.
macro_rules! define_object_hash_map {
    (
        map = $map:ident,
        cursor = $cursor:ident,
        key = $kt:ty,
        key_cursor = $kcur:ident,
        key_zero = $kzero:expr,
        hash_key = $hash_key:path,
        mix_key = $mix_key:path,
        size_of_keys = $size_of_keys:path,
        base_ram_bytes_used = $base_ram:expr,
        java_class = $java:literal,
        java_key = $jkey:literal,
    ) => {
        use std::fmt::{self, Debug, Display, Formatter};
        use std::hash::{Hash, Hasher};
        use std::sync::atomic::{AtomicI32, Ordering};

        use super::abstract_iterator::AbstractIterator;
        use super::bit_mixer::BitMixer;
        use super::buffer_allocation_exception::BufferAllocationException;
        use super::hash_containers::{
            check_power_of_two, check_load_factor, expand_at_count, iteration_increment,
            min_buffer_size, next_buffer_size, next_iteration_seed, DEFAULT_EXPECTED_ELEMENTS,
            DEFAULT_LOAD_FACTOR, MAX_LOAD_FACTOR, MIN_LOAD_FACTOR,
        };
        use super::support::{group_digits, shallow_size_of_object_array, value_hash};
        use super::ObjectCursor;
        use crate::util::{Accountable, RamUsageEstimator};

        #[doc = concat!("Shallow size of a `", $java, "` instance, as `RamUsageEstimator.shallowSizeOfInstance` computes it.")]
        const BASE_RAM_BYTES_USED: i64 = $base_ram;

        /// Message of the invariant that ties an assigned key slot to a present
        /// value slot.
        const ASSIGNED_SLOT_INVARIANT: &str =
            "INVARIANT: an assigned key slot always holds a value";

        #[doc = concat!("Port of `org.apache.lucene.internal.hppc.", $java, "`.")]
        ///
        #[doc = concat!("A hash map of `", $jkey, "` to objects, implemented using open addressing with linear probing for collision resolution.")]
        ///
        /// Lucene forked and trimmed this from HPPC 0.10.0
        #[doc = concat!("(`com.carrotsearch.hppc.", $java, "`).")]
        ///
        /// # Null values
        ///
        /// Java's `VType` is a nullable reference and the Java class documents
        /// that it supports null values. Rust's nullable reference is
        /// `Option<V>`, so a map that must store the absent value is
        #[doc = concat!("instantiated as `", stringify!($map), "<Option<T>>`. That keeps this type's")]
        /// own `Option` returns unambiguous: [`None`] always means *no such
        /// key*, never *a key mapped to null*.
        ///
        /// # Iteration order
        ///
        /// As in Lucene, the iteration order is deliberately randomised: every
        /// container starts from a distinct seed and every iterator advances it
        /// again, so no caller can come to depend on a particular order.
        pub struct $map<V> {
            /// The array holding keys.
            pub keys: Vec<$kt>,

            /// The array holding values.
            ///
            /// Java uses an `Object[]` whose unassigned slots hold `null`; this
            /// port uses `Option<V>`, where [`None`] marks an unassigned slot.
            /// A slot whose key is assigned always holds [`Some`].
            pub values: Vec<Option<V>>,

            /// The number of stored keys (assigned key slots), excluding the
            /// special "empty" key, if any (use [`Self::size`] instead).
            assigned: i32,

            /// Mask for slot scans in [`Self::keys`].
            mask: i32,

            /// Expand (rehash) [`Self::keys`] when `assigned` hits this value.
            resize_at: i32,

            /// Special treatment for the "empty slot" key marker.
            has_empty_key: bool,

            /// The load factor for [`Self::keys`].
            load_factor: f64,

            /// Seed ensuring the hash iteration order differs from one
            /// iteration to another.
            iteration_seed: AtomicI32,
        }

        impl<V> $map<V> {
            /// New instance with sane defaults.
            pub fn new() -> Self {
                Self::with_expected_elements(DEFAULT_EXPECTED_ELEMENTS)
            }

            /// New instance with sane defaults.
            ///
            /// `expected_elements` is the expected number of elements
            /// guaranteed not to cause buffer expansion (inclusive).
            pub fn with_expected_elements(expected_elements: i32) -> Self {
                Self::with_expected_elements_and_load_factor(
                    expected_elements,
                    DEFAULT_LOAD_FACTOR as f64,
                )
            }

            /// New instance with the provided defaults.
            ///
            /// # Panics
            ///
            /// Panics with a [`BufferAllocationException`] if `load_factor` is
            /// insane (zero, or full capacity).
            pub fn with_expected_elements_and_load_factor(
                expected_elements: i32,
                load_factor: f64,
            ) -> Self {
                let mut map = Self {
                    keys: Vec::new(),
                    values: Vec::new(),
                    assigned: 0,
                    mask: 0,
                    resize_at: 0,
                    has_empty_key: false,
                    load_factor: Self::verify_load_factor(load_factor),
                    iteration_seed: AtomicI32::new(next_iteration_seed()),
                };
                map.ensure_capacity(expected_elements);
                map
            }

            /// Associates `value` with `key`, returning the previous value if
            /// the key was present.
            pub fn put(&mut self, key: $kt, value: V) -> Option<V> {
                debug_assert!(self.assigned < self.mask + 1);

                let mask = self.mask;
                if key == $kzero {
                    let previous_value = if self.has_empty_key {
                        self.values[(mask + 1) as usize].take()
                    } else {
                        None
                    };
                    self.has_empty_key = true;
                    self.values[(mask + 1) as usize] = Some(value);
                    previous_value
                } else {
                    let mut slot = Self::hash_key(key) & mask;

                    loop {
                        let existing = self.keys[slot as usize];
                        if existing == $kzero {
                            break;
                        }
                        if existing == key {
                            return self.values[slot as usize].replace(value);
                        }
                        slot = (slot + 1) & mask;
                    }

                    if self.assigned == self.resize_at {
                        self.allocate_then_insert_then_rehash(slot, key, value);
                    } else {
                        self.keys[slot as usize] = key;
                        self.values[slot as usize] = Some(value);
                    }

                    self.assigned += 1;
                    None
                }
            }

            /// Puts every cursor of `iterable` into this map, returning the
            /// number of entries the map gained.
            pub fn put_all<I: IntoIterator<Item = $cursor<V>>>(&mut self, iterable: I) -> i32 {
                let count = self.size();
                for c in iterable {
                    self.put(c.key, c.value);
                }
                self.size() - count
            }

            /// Trove-inspired API method, an equivalent of
            /// `if (!map.containsKey(key)) map.put(key, value);`.
            ///
            /// Returns `true` if `key` did not exist and `value` was placed in
            /// the map.
            pub fn put_if_absent(&mut self, key: $kt, value: V) -> bool {
                let key_index = self.index_of(key);
                if !self.index_exists(key_index) {
                    self.index_insert(key_index, key, value);
                    true
                } else {
                    false
                }
            }

            /// Removes `key`, returning the value it held if it was present.
            pub fn remove(&mut self, key: $kt) -> Option<V> {
                let mask = self.mask;
                if key == $kzero {
                    if !self.has_empty_key {
                        return None;
                    }
                    self.has_empty_key = false;
                    self.values[(mask + 1) as usize].take()
                } else {
                    let mut slot = Self::hash_key(key) & mask;

                    loop {
                        let existing = self.keys[slot as usize];
                        if existing == $kzero {
                            break;
                        }
                        if existing == key {
                            let previous_value = self.values[slot as usize].take();
                            self.shift_conflicting_keys(slot);
                            return previous_value;
                        }
                        slot = (slot + 1) & mask;
                    }

                    None
                }
            }

            /// Returns the value associated with `key`, if any.
            pub fn get(&self, key: $kt) -> Option<&V> {
                let index = self.index_of(key);
                if index >= 0 {
                    self.values[index as usize].as_ref()
                } else {
                    None
                }
            }

            /// Returns a mutable borrow of the value associated with `key`, if
            /// any.
            ///
            /// Java's `get` hands back a reference through which the caller can
            /// mutate the stored object; under Rust's ownership rules that
            /// capability needs its own accessor, which is what this is.
            pub fn get_mut(&mut self, key: $kt) -> Option<&mut V> {
                let index = self.index_of(key);
                if index >= 0 {
                    self.values[index as usize].as_mut()
                } else {
                    None
                }
            }

            /// Returns the value associated with `key`, or `default_value` if
            /// the key is absent.
            pub fn get_or_default<'a>(&'a self, key: $kt, default_value: &'a V) -> &'a V {
                self.get(key).unwrap_or(default_value)
            }

            /// Returns `true` if `key` is present in this map.
            pub fn contains_key(&self, key: $kt) -> bool {
                if key == $kzero {
                    self.has_empty_key
                } else {
                    let mask = self.mask;
                    let mut slot = Self::hash_key(key) & mask;

                    loop {
                        let existing = self.keys[slot as usize];
                        if existing == $kzero {
                            break;
                        }
                        if existing == key {
                            return true;
                        }
                        slot = (slot + 1) & mask;
                    }

                    false
                }
            }

            /// Returns a logical "index" of `key`, usable to speed up follow-up
            /// logic.
            ///
            /// The result is non-negative when the key exists and the bitwise
            /// complement of the insertion slot when it does not. Indexes are
            /// valid only between modifications of the map.
            pub fn index_of(&self, key: $kt) -> i32 {
                let mask = self.mask;
                if key == $kzero {
                    if self.has_empty_key {
                        mask + 1
                    } else {
                        !(mask + 1)
                    }
                } else {
                    let mut slot = Self::hash_key(key) & mask;

                    loop {
                        let existing = self.keys[slot as usize];
                        if existing == $kzero {
                            break;
                        }
                        if existing == key {
                            return slot;
                        }
                        slot = (slot + 1) & mask;
                    }

                    !slot
                }
            }

            /// Returns `true` if `index`, as returned by [`Self::index_of`],
            /// corresponds to an existing key.
            pub fn index_exists(&self, index: i32) -> bool {
                debug_assert!(
                    index < 0 || index <= self.mask || (index == self.mask + 1 && self.has_empty_key)
                );

                index >= 0
            }

            /// Returns the value stored at an existing `index`.
            ///
            /// # Panics
            ///
            /// With debug assertions enabled, panics if `index` does not point
            /// at an existing key.
            pub fn index_get(&self, index: i32) -> &V {
                debug_assert!(index >= 0, "The index must point at an existing key.");
                debug_assert!(
                    index <= self.mask || (index == self.mask + 1 && self.has_empty_key)
                );

                self.values[index as usize]
                    .as_ref()
                    .expect(ASSIGNED_SLOT_INVARIANT)
            }

            /// Returns a mutable borrow of the value stored at an existing
            /// `index`.
            ///
            /// The mutable counterpart of [`Self::index_get`], for the reason
            /// given on [`Self::get_mut`].
            ///
            /// # Panics
            ///
            /// With debug assertions enabled, panics if `index` does not point
            /// at an existing key.
            pub fn index_get_mut(&mut self, index: i32) -> &mut V {
                debug_assert!(index >= 0, "The index must point at an existing key.");
                debug_assert!(
                    index <= self.mask || (index == self.mask + 1 && self.has_empty_key)
                );

                self.values[index as usize]
                    .as_mut()
                    .expect(ASSIGNED_SLOT_INVARIANT)
            }

            /// Replaces the value stored at an existing `index`, returning the
            /// previous one.
            ///
            /// # Panics
            ///
            /// With debug assertions enabled, panics if `index` does not point
            /// at an existing key.
            pub fn index_replace(&mut self, index: i32, new_value: V) -> V {
                debug_assert!(index >= 0, "The index must point at an existing key.");
                debug_assert!(
                    index <= self.mask || (index == self.mask + 1 && self.has_empty_key)
                );

                self.values[index as usize]
                    .replace(new_value)
                    .expect(ASSIGNED_SLOT_INVARIANT)
            }

            /// Inserts a key-value pair at an `index` that is not present in
            /// the map, avoiding a second hash computation.
            ///
            /// # Panics
            ///
            /// With debug assertions enabled, panics if `index` points at an
            /// existing key.
            pub fn index_insert(&mut self, index: i32, key: $kt, value: V) {
                debug_assert!(index < 0, "The index must not point at an existing key.");

                let index = !index;
                if key == $kzero {
                    debug_assert!(index == self.mask + 1);
                    self.values[index as usize] = Some(value);
                    self.has_empty_key = true;
                } else {
                    debug_assert!(self.keys[index as usize] == $kzero);

                    if self.assigned == self.resize_at {
                        self.allocate_then_insert_then_rehash(index, key, value);
                    } else {
                        self.keys[index as usize] = key;
                        self.values[index as usize] = Some(value);
                    }

                    self.assigned += 1;
                }
            }

            /// Removes the key at an existing `index`, returning its value.
            ///
            /// # Panics
            ///
            /// With debug assertions enabled, panics if `index` does not point
            /// at an existing key.
            pub fn index_remove(&mut self, index: i32) -> V {
                debug_assert!(index >= 0, "The index must point at an existing key.");
                debug_assert!(
                    index <= self.mask || (index == self.mask + 1 && self.has_empty_key)
                );

                let previous_value = self.values[index as usize]
                    .take()
                    .expect(ASSIGNED_SLOT_INVARIANT);
                if index > self.mask {
                    debug_assert!(index == self.mask + 1);
                    self.has_empty_key = false;
                } else {
                    self.shift_conflicting_keys(index);
                }
                previous_value
            }

            /// Removes every entry, keeping the internal buffers.
            ///
            /// Like Lucene, only the key buffer is reset; the value slots of
            /// the cleared entries keep whatever they held until they are
            /// overwritten, which is exactly what `Arrays.fill(keys, 0)` leaves
            /// behind in Java.
            pub fn clear(&mut self) {
                self.assigned = 0;
                self.has_empty_key = false;

                self.keys.fill($kzero);
            }

            /// Removes every entry and releases the internal buffers, sizing
            /// them back down to the default capacity.
            pub fn release(&mut self) {
                self.assigned = 0;
                self.has_empty_key = false;

                self.keys = Vec::new();
                self.values = Vec::new();
                self.ensure_capacity(DEFAULT_EXPECTED_ELEMENTS);
            }

            /// Returns the number of entries in this map.
            pub fn size(&self) -> i32 {
                self.assigned + if self.has_empty_key { 1 } else { 0 }
            }

            /// Returns `true` if this map holds no entries.
            pub fn is_empty(&self) -> bool {
                self.size() == 0
            }

            /// Ensures this container can hold at least `expected_elements`
            /// entries without resizing its buffers.
            pub fn ensure_capacity(&mut self, expected_elements: i32) {
                if expected_elements > self.resize_at || self.keys.is_empty() {
                    let prev_keys = std::mem::take(&mut self.keys);
                    let prev_values = std::mem::take(&mut self.values);
                    self.allocate_buffers(min_buffer_size(expected_elements, self.load_factor));
                    if !prev_keys.is_empty() && !self.is_empty() {
                        self.rehash(prev_keys, prev_values);
                    }
                }
            }

            /// Provides the next iteration seed used to build the iteration
            /// starting slot and offset increment.
            fn next_iteration_seed(&self) -> i32 {
                let seed = BitMixer::mix_phi_i32(self.iteration_seed.load(Ordering::Relaxed));
                self.iteration_seed.store(seed, Ordering::Relaxed);
                seed
            }

            /// Returns an iterator over the entries of this map.
            pub fn iter(&self) -> EntryIterator<'_, V> {
                EntryIterator::new(self)
            }

            /// Returns a specialized view of the keys of this associated
            /// container.
            pub fn keys(&self) -> KeysContainer<'_, V> {
                KeysContainer { owner: self }
            }

            /// Returns a container with all values stored in this map.
            pub fn values(&self) -> ValuesContainer<'_, V> {
                ValuesContainer { owner: self }
            }

            /// Creates a hash map from two index-aligned arrays of key-value
            /// pairs.
            ///
            /// # Panics
            ///
            /// Panics if the two collections have different lengths, as Java's
            /// `IllegalArgumentException` does.
            #[allow(clippy::should_implement_trait)]
            pub fn from(keys: &[$kt], values: Vec<V>) -> Self {
                if keys.len() != values.len() {
                    panic!("Arrays of keys and values must have an identical length.");
                }

                let mut map = Self::with_expected_elements(keys.len() as i32);
                for (key, value) in keys.iter().zip(values) {
                    map.put(*key, value);
                }

                map
            }

            /// Returns a hash code for `key`, distributing keys evenly across
            /// the entire integer range.
            fn hash_key(key: $kt) -> i32 {
                debug_assert!(key != $kzero); // Handled as a special case (empty slot marker).
                $hash_key(key)
            }

            /// Validates the load factor range and returns it.
            fn verify_load_factor(load_factor: f64) -> f64 {
                check_load_factor(load_factor, MIN_LOAD_FACTOR as f64, MAX_LOAD_FACTOR as f64);
                load_factor
            }

            /// Rehashes from old buffers into the current ones.
            fn rehash(&mut self, from_keys: Vec<$kt>, mut from_values: Vec<Option<V>>) {
                debug_assert!(
                    from_keys.len() == from_values.len()
                        && check_power_of_two(from_keys.len() as i32 - 1)
                );

                let mask = self.mask;

                // Copy the zero element's slot, then rehash everything else.
                let mut from = from_keys.len() - 1;
                let last = self.keys.len() - 1;
                self.keys[last] = from_keys[from];
                self.values[last] = from_values[from].take();
                while from > 0 {
                    from -= 1;
                    let existing = from_keys[from];
                    if existing != $kzero {
                        let mut slot = Self::hash_key(existing) & mask;
                        while self.keys[slot as usize] != $kzero {
                            slot = (slot + 1) & mask;
                        }
                        self.keys[slot as usize] = existing;
                        self.values[slot as usize] = from_values[from].take();
                    }
                }
            }

            /// Allocates new internal buffers, atomically: either both
            /// allocations succeed, or the map is left untouched.
            ///
            /// # Panics
            ///
            /// Panics with a [`BufferAllocationException`] when the allocator
            /// cannot satisfy the request, which is where Java catches
            /// `OutOfMemoryError` and throws the same exception.
            fn allocate_buffers(&mut self, array_size: i32) {
                debug_assert!(array_size.count_ones() == 1);

                // An extra slot holds the value of the "empty" key.
                let length = array_size as usize + 1;
                let mut new_keys: Vec<$kt> = Vec::new();
                let mut new_values: Vec<Option<V>> = Vec::new();
                if new_keys.try_reserve_exact(length).is_err()
                    || new_values.try_reserve_exact(length).is_err()
                {
                    BufferAllocationException::new(format!(
                        "Not enough memory to allocate buffers for rehashing: {} -> {}",
                        group_digits((self.mask + 1) as i64),
                        group_digits(array_size as i64)
                    ))
                    .throw();
                }
                new_keys.resize(length, $kzero);
                new_values.resize_with(length, || None);
                self.keys = new_keys;
                self.values = new_values;

                self.resize_at = expand_at_count(array_size, self.load_factor);
                self.mask = array_size - 1;
            }

            /// Invoked when a new key-value pair must be inserted but there are
            /// not enough empty slots.
            fn allocate_then_insert_then_rehash(
                &mut self,
                slot: i32,
                pending_key: $kt,
                pending_value: V,
            ) {
                debug_assert!(
                    self.assigned == self.resize_at
                        && self.keys[slot as usize] == $kzero
                        && pending_key != $kzero
                );

                let mut prev_keys = std::mem::take(&mut self.keys);
                let mut prev_values = std::mem::take(&mut self.values);
                let next = next_buffer_size(self.mask + 1, self.size(), self.load_factor);
                self.allocate_buffers(next);
                debug_assert!(self.keys.len() > prev_keys.len());

                // We have succeeded at allocating new data so insert the
                // pending key/value at the free slot in the old arrays before
                // rehashing.
                prev_keys[slot as usize] = pending_key;
                prev_values[slot as usize] = Some(pending_value);

                // Rehash old keys, including the pending key.
                self.rehash(prev_keys, prev_values);
            }

            /// Shifts all the slot-conflicting keys and values allocated to
            /// (and including) `gap_slot`.
            fn shift_conflicting_keys(&mut self, gap_slot: i32) {
                let mut gap_slot = gap_slot;
                let mask = self.mask;

                let mut distance = 0i32;
                loop {
                    distance += 1;
                    let slot = (gap_slot.wrapping_add(distance)) & mask;
                    let existing = self.keys[slot as usize];
                    if existing == $kzero {
                        break;
                    }

                    let ideal_slot = Self::hash_key(existing);
                    let shift = slot.wrapping_sub(ideal_slot) & mask;
                    if shift >= distance {
                        let moved_value = self.values[slot as usize].take();
                        self.keys[gap_slot as usize] = existing;
                        self.values[gap_slot as usize] = moved_value;
                        gap_slot = slot;
                        distance = 0;
                    }
                }

                // Mark the last found gap slot without a conflict as empty.
                self.keys[gap_slot as usize] = $kzero;
                self.values[gap_slot as usize] = None;
                self.assigned -= 1;
            }
        }

        impl<V: Hash> $map<V> {
            /// Equivalent of Java's `hashCode()`.
            ///
            /// The value is order-independent, so it survives this container's
            /// randomised iteration order. Java mixes `value.hashCode()`; Rust
            /// has no universal `hashCode`, so the value's contribution is
            /// derived from its [`Hash`] implementation instead.
            pub fn hash_code(&self) -> i32 {
                let mut h: i32 = if self.has_empty_key {
                    0xDEAD_BEEF_u32 as i32
                } else {
                    0
                };
                for c in self.iter() {
                    h = h.wrapping_add(
                        $mix_key(c.key).wrapping_add(BitMixer::mix_hash(value_hash(c.value))),
                    );
                }
                h
            }
        }

        impl<V: PartialEq> $map<V> {
            /// Returns `true` if every key of `other` exists in this map with
            /// the same value.
            fn equal_elements(&self, other: &Self) -> bool {
                if other.size() != self.size() {
                    return false;
                }

                for c in other.iter() {
                    let key = c.key;
                    if !self.contains_key(key) || self.get(key) != Some(c.value) {
                        return false;
                    }
                }

                true
            }
        }

        impl<V: Clone> $map<V> {
            #[doc = concat!("Creates a hash map from all key-value pairs of another `", $java, "`.")]
            ///
            /// Equivalent of Java's copy constructor, which rebuilds the table
            /// rather than copying its layout the way [`Clone`] does. Java
            /// copies references; Rust owns its values, so they are cloned.
            pub fn from_map(map: &Self) -> Self {
                let mut copy = Self::with_expected_elements(map.size());
                for c in map.iter() {
                    copy.put(c.key, c.value.clone());
                }
                copy
            }
        }

        impl<V> Default for $map<V> {
            fn default() -> Self {
                Self::new()
            }
        }

        impl<V: Clone> Clone for $map<V> {
            /// Clones this map, reusing the same hash function and array
            /// resizing strategy but drawing a fresh iteration seed, exactly as
            /// Java's `clone()` does.
            fn clone(&self) -> Self {
                Self {
                    keys: self.keys.clone(),
                    values: self.values.clone(),
                    assigned: self.assigned,
                    mask: self.mask,
                    resize_at: self.resize_at,
                    has_empty_key: self.has_empty_key,
                    load_factor: self.load_factor,
                    iteration_seed: AtomicI32::new(next_iteration_seed()),
                }
            }
        }

        impl<V: PartialEq> PartialEq for $map<V> {
            fn eq(&self, other: &Self) -> bool {
                std::ptr::eq(self, other) || self.equal_elements(other)
            }
        }

        impl<V: Eq> Eq for $map<V> {}

        impl<V: Hash> Hash for $map<V> {
            fn hash<H: Hasher>(&self, state: &mut H) {
                state.write_i32(self.hash_code());
            }
        }

        impl<V> Accountable for $map<V> {
            /// Equivalent of Java's `ramBytesUsed()`.
            ///
            /// Lucene's `sizeOfValues()` passes the *cursor* rather than the
            /// value to `RamUsageEstimator.sizeOfObject`, so every entry
            /// contributes the estimator's
            /// `UNKNOWN_DEFAULT_RAM_BYTES_USED` regardless of what is stored.
            /// This port reproduces that measurement rather than correcting it,
            /// which is also why the estimate needs no bound on `V`.
            fn ram_bytes_used(&self) -> i64 {
                BASE_RAM_BYTES_USED + $size_of_keys(self.keys.len()) + self.size_of_values()
            }
        }

        impl<V> $map<V> {
            /// Equivalent of Java's private `sizeOfValues()`.
            fn size_of_values(&self) -> i64 {
                shallow_size_of_object_array(self.values.len())
                    + self.size() as i64 * RamUsageEstimator::UNKNOWN_DEFAULT_RAM_BYTES_USED
            }
        }

        impl<V: Display> Display for $map<V> {
            /// Converts the contents of this map to a human-friendly string.
            fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
                f.write_str("[")?;
                let mut first = true;
                for cursor in self.iter() {
                    if !first {
                        f.write_str(", ")?;
                    }
                    write!(f, "{}=>{}", cursor.key, cursor.value)?;
                    first = false;
                }
                f.write_str("]")
            }
        }

        impl<V: Debug> Debug for $map<V> {
            fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
                write!(f, "{}[", stringify!($map))?;
                let mut first = true;
                for cursor in self.iter() {
                    if !first {
                        f.write_str(", ")?;
                    }
                    write!(f, "{}=>{:?}", cursor.key, cursor.value)?;
                    first = false;
                }
                f.write_str("]")
            }
        }

        impl<'a, V> IntoIterator for &'a $map<V> {
            type Item = $cursor<&'a V>;
            type IntoIter = EntryIterator<'a, V>;

            fn into_iter(self) -> Self::IntoIter {
                self.iter()
            }
        }

        #[doc = concat!("Port of `", $java, ".", stringify!($cursor), "`.")]
        ///
        /// Forked by Lucene from HPPC, holding an `int` index together with the
        /// entry's key and value. Iterating a map yields cursors whose `V` is a
        /// borrow of the stored value.
        #[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
        pub struct $cursor<V> {
            /// The current key and value's index in the container this cursor
            /// belongs to.
            ///
            /// The meaning of this index is defined by the container (usually
            /// it will be an index in the underlying storage buffer).
            pub index: i32,

            /// The current key.
            pub key: $kt,

            /// The current value.
            pub value: V,
        }

        impl<V: Display> Display for $cursor<V> {
            fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
                write!(
                    f,
                    "[cursor, index: {}, key: {}, value: {}]",
                    self.index, self.key, self.value
                )
            }
        }

        #[doc = concat!("Iterator over the entries of a [`", stringify!($map), "`].")]
        ///
        #[doc = concat!("Port of `", $java, ".EntryIterator`.")]
        pub struct EntryIterator<'a, V> {
            owner: &'a $map<V>,
            increment: i32,
            index: i32,
            slot: i32,
        }

        impl<'a, V> EntryIterator<'a, V> {
            fn new(owner: &'a $map<V>) -> Self {
                let seed = owner.next_iteration_seed();
                Self {
                    owner,
                    increment: iteration_increment(seed),
                    index: 0,
                    slot: seed & owner.mask,
                }
            }
        }

        impl<'a, V> AbstractIterator for EntryIterator<'a, V> {
            type Item = $cursor<&'a V>;

            fn fetch(&mut self) -> Option<$cursor<&'a V>> {
                let mask = self.owner.mask;
                while self.index <= mask {
                    self.index += 1;
                    self.slot = (self.slot + self.increment) & mask;
                    let existing = self.owner.keys[self.slot as usize];
                    if existing != $kzero {
                        return Some($cursor {
                            index: self.slot,
                            key: existing,
                            value: self.owner.values[self.slot as usize]
                                .as_ref()
                                .expect(ASSIGNED_SLOT_INVARIANT),
                        });
                    }
                }

                if self.index == mask + 1 && self.owner.has_empty_key {
                    let index = self.index;
                    self.index += 1;
                    return Some($cursor {
                        index,
                        key: $kzero,
                        value: self.owner.values[index as usize]
                            .as_ref()
                            .expect(ASSIGNED_SLOT_INVARIANT),
                    });
                }

                self.done()
            }
        }

        impl<'a, V> Iterator for EntryIterator<'a, V> {
            type Item = $cursor<&'a V>;

            fn next(&mut self) -> Option<$cursor<&'a V>> {
                self.fetch()
            }
        }

        #[doc = concat!("A view of the keys inside a [`", stringify!($map), "`].")]
        ///
        #[doc = concat!("Port of `", $java, ".KeysContainer`.")]
        #[derive(Debug)]
        pub struct KeysContainer<'a, V> {
            owner: &'a $map<V>,
        }

        impl<'a, V> KeysContainer<'a, V> {
            /// Returns the number of keys in the view.
            pub fn size(&self) -> i32 {
                self.owner.size()
            }

            /// Copies every key into a freshly allocated array.
            pub fn to_array(&self) -> Vec<$kt> {
                let mut array = Vec::with_capacity(self.size() as usize);
                for cursor in self.iter() {
                    array.push(cursor.value);
                }
                array
            }

            /// Returns an iterator over the keys.
            pub fn iter(&self) -> KeysIterator<'a, V> {
                KeysIterator::new(self.owner)
            }
        }

        impl<'a, V> IntoIterator for KeysContainer<'a, V> {
            type Item = super::$kcur;
            type IntoIter = KeysIterator<'a, V>;

            fn into_iter(self) -> Self::IntoIter {
                KeysIterator::new(self.owner)
            }
        }

        #[doc = concat!("Iterator over the assigned keys of a [`", stringify!($map), "`].")]
        ///
        #[doc = concat!("Port of `", $java, ".KeysIterator`.")]
        pub struct KeysIterator<'a, V> {
            owner: &'a $map<V>,
            increment: i32,
            index: i32,
            slot: i32,
        }

        impl<'a, V> KeysIterator<'a, V> {
            fn new(owner: &'a $map<V>) -> Self {
                let seed = owner.next_iteration_seed();
                Self {
                    owner,
                    increment: iteration_increment(seed),
                    index: 0,
                    slot: seed & owner.mask,
                }
            }
        }

        impl<V> AbstractIterator for KeysIterator<'_, V> {
            type Item = super::$kcur;

            fn fetch(&mut self) -> Option<super::$kcur> {
                let mask = self.owner.mask;
                while self.index <= mask {
                    self.index += 1;
                    self.slot = (self.slot + self.increment) & mask;
                    let existing = self.owner.keys[self.slot as usize];
                    if existing != $kzero {
                        return Some(super::$kcur {
                            index: self.slot,
                            value: existing,
                        });
                    }
                }

                if self.index == mask + 1 && self.owner.has_empty_key {
                    let index = self.index;
                    self.index += 1;
                    return Some(super::$kcur {
                        index,
                        value: $kzero,
                    });
                }

                self.done()
            }
        }

        impl<V> Iterator for KeysIterator<'_, V> {
            type Item = super::$kcur;

            fn next(&mut self) -> Option<super::$kcur> {
                self.fetch()
            }
        }

        #[doc = concat!("A view over the values of a [`", stringify!($map), "`].")]
        ///
        #[doc = concat!("Port of `", $java, ".ValuesContainer`.")]
        #[derive(Debug)]
        pub struct ValuesContainer<'a, V> {
            owner: &'a $map<V>,
        }

        impl<'a, V> ValuesContainer<'a, V> {
            /// Returns the number of values in the view.
            pub fn size(&self) -> i32 {
                self.owner.size()
            }

            /// Returns an iterator over the values.
            pub fn iter(&self) -> ValuesIterator<'a, V> {
                ValuesIterator::new(self.owner)
            }
        }

        impl<'a, V> IntoIterator for ValuesContainer<'a, V> {
            type Item = ObjectCursor<&'a V>;
            type IntoIter = ValuesIterator<'a, V>;

            fn into_iter(self) -> Self::IntoIter {
                ValuesIterator::new(self.owner)
            }
        }

        #[doc = concat!("Iterator over the assigned values of a [`", stringify!($map), "`].")]
        ///
        #[doc = concat!("Port of `", $java, ".ValuesIterator`.")]
        pub struct ValuesIterator<'a, V> {
            owner: &'a $map<V>,
            increment: i32,
            index: i32,
            slot: i32,
        }

        impl<'a, V> ValuesIterator<'a, V> {
            fn new(owner: &'a $map<V>) -> Self {
                let seed = owner.next_iteration_seed();
                Self {
                    owner,
                    increment: iteration_increment(seed),
                    index: 0,
                    slot: seed & owner.mask,
                }
            }
        }

        impl<'a, V> AbstractIterator for ValuesIterator<'a, V> {
            type Item = ObjectCursor<&'a V>;

            fn fetch(&mut self) -> Option<ObjectCursor<&'a V>> {
                let mask = self.owner.mask;
                while self.index <= mask {
                    self.index += 1;
                    self.slot = (self.slot + self.increment) & mask;
                    if self.owner.keys[self.slot as usize] != $kzero {
                        return Some(ObjectCursor {
                            index: self.slot,
                            value: self.owner.values[self.slot as usize]
                                .as_ref()
                                .expect(ASSIGNED_SLOT_INVARIANT),
                        });
                    }
                }

                if self.index == mask + 1 && self.owner.has_empty_key {
                    let index = self.index;
                    self.index += 1;
                    return Some(ObjectCursor {
                        index,
                        value: self.owner.values[index as usize]
                            .as_ref()
                            .expect(ASSIGNED_SLOT_INVARIANT),
                    });
                }

                self.done()
            }
        }

        impl<'a, V> Iterator for ValuesIterator<'a, V> {
            type Item = ObjectCursor<&'a V>;

            fn next(&mut self) -> Option<ObjectCursor<&'a V>> {
                self.fetch()
            }
        }
    };
}

pub(crate) use define_object_hash_map;

/// Expands to a hash set of a primitive key type.
///
/// Covers `IntHashSet`, `LongHashSet` and `CharHashSet`.
macro_rules! define_hash_set {
    (
        set = $set:ident,
        key = $kt:ty,
        cursor = $cur:ident,
        key_zero = $kzero:expr,
        hash_key = $hash_key:path,
        mix_key = $mix_key:path,
        size_of_keys = $size_of_keys:path,
        base_ram_bytes_used = $base_ram:expr,
        java_class = $java:literal,
        java_key = $jkey:literal,
    ) => {
        use std::fmt::{self, Debug, Formatter};
        use std::hash::{Hash, Hasher};
        use std::sync::atomic::{AtomicI32, Ordering};

        use super::abstract_iterator::AbstractIterator;
        use super::bit_mixer::BitMixer;
        use super::buffer_allocation_exception::BufferAllocationException;
        use super::hash_containers::{
            check_power_of_two, check_load_factor, expand_at_count, iteration_increment,
            min_buffer_size, next_buffer_size, next_iteration_seed, DEFAULT_EXPECTED_ELEMENTS,
            DEFAULT_LOAD_FACTOR, MAX_LOAD_FACTOR, MIN_LOAD_FACTOR,
        };
        use super::support::group_digits;
        use crate::util::Accountable;

        #[doc = concat!("Shallow size of a `", $java, "` instance, as `RamUsageEstimator.shallowSizeOfInstance` computes it.")]
        const BASE_RAM_BYTES_USED: i64 = $base_ram;

        #[doc = concat!("Port of `org.apache.lucene.internal.hppc.", $java, "`.")]
        ///
        #[doc = concat!("A hash set of `", $jkey, "`s, implemented using open addressing with linear probing for collision resolution.")]
        ///
        /// Lucene forked and trimmed this from HPPC 0.10.0
        #[doc = concat!("(`com.carrotsearch.hppc.", $java, "`).")]
        ///
        /// # Iteration order
        ///
        /// As in Lucene, the iteration order is deliberately randomised: every
        /// container starts from a distinct seed and every iterator advances it
        /// again, so no caller can come to depend on a particular order.
        pub struct $set {
            /// The hash array holding keys.
            pub keys: Vec<$kt>,

            /// The number of stored keys (assigned key slots), excluding the
            /// special "empty" key, if any (use [`Self::size`] instead).
            assigned: i32,

            /// Mask for slot scans in [`Self::keys`].
            mask: i32,

            /// Expand (rehash) [`Self::keys`] when `assigned` hits this value.
            resize_at: i32,

            /// Special treatment for the "empty slot" key marker.
            has_empty_key: bool,

            /// The load factor for [`Self::keys`].
            load_factor: f64,

            /// Seed ensuring the hash iteration order differs from one
            /// iteration to another.
            iteration_seed: AtomicI32,
        }

        impl $set {
            /// New instance with sane defaults.
            pub fn new() -> Self {
                Self::with_expected_elements(DEFAULT_EXPECTED_ELEMENTS)
            }

            /// New instance with sane defaults.
            ///
            /// `expected_elements` is the expected number of elements
            /// guaranteed not to cause a rehash (inclusive).
            pub fn with_expected_elements(expected_elements: i32) -> Self {
                Self::with_expected_elements_and_load_factor(
                    expected_elements,
                    DEFAULT_LOAD_FACTOR as f64,
                )
            }

            /// New instance with the provided defaults.
            ///
            /// # Panics
            ///
            /// Panics with a [`BufferAllocationException`] if `load_factor` is
            /// insane (zero, or full capacity).
            pub fn with_expected_elements_and_load_factor(
                expected_elements: i32,
                load_factor: f64,
            ) -> Self {
                let mut set = Self {
                    keys: Vec::new(),
                    assigned: 0,
                    mask: 0,
                    resize_at: 0,
                    has_empty_key: false,
                    load_factor: Self::verify_load_factor(load_factor),
                    iteration_seed: AtomicI32::new(next_iteration_seed()),
                };
                set.ensure_capacity(expected_elements);
                set
            }

            /// New instance copying elements from another set.
            ///
            /// Equivalent of Java's copy constructor, which rebuilds the table
            /// rather than copying its layout the way [`Clone`] does.
            pub fn from_set(set: &Self) -> Self {
                let mut copy = Self::with_expected_elements(set.size());
                copy.add_all_set(set);
                copy
            }

            /// Adds `key` to this set, returning `true` if it was not already
            /// present.
            pub fn add(&mut self, key: $kt) -> bool {
                if key == $kzero {
                    debug_assert!(self.keys[(self.mask + 1) as usize] == $kzero);
                    let added = !self.has_empty_key;
                    self.has_empty_key = true;
                    added
                } else {
                    let mask = self.mask;
                    let mut slot = Self::hash_key(key) & mask;

                    loop {
                        let existing = self.keys[slot as usize];
                        if existing == $kzero {
                            break;
                        }
                        if key == existing {
                            return false;
                        }
                        slot = (slot + 1) & mask;
                    }

                    if self.assigned == self.resize_at {
                        self.allocate_then_insert_then_rehash(slot, key);
                    } else {
                        self.keys[slot as usize] = key;
                    }

                    self.assigned += 1;
                    true
                }
            }

            /// Adds all elements from the given array to this set.
            ///
            /// Equivalent of Java's varargs `addAll`, which pre-sizes the set
            /// for the whole batch. Returns the number of elements actually
            /// added (not previously present in the set).
            pub fn add_all_array(&mut self, elements: &[$kt]) -> i32 {
                self.ensure_capacity(elements.len() as i32);
                let mut count = 0;
                for e in elements {
                    if self.add(*e) {
                        count += 1;
                    }
                }
                count
            }

            /// Adds all elements from the given set to this set.
            ///
            /// Returns the number of elements actually added (not previously
            /// present in the set).
            pub fn add_all_set(&mut self, set: &Self) -> i32 {
                self.ensure_capacity(set.size());
                self.add_all_cursors(set.iter())
            }

            /// Adds all elements from the given cursors to this set.
            ///
            /// Returns the number of elements actually added (not previously
            /// present in the set).
            pub fn add_all_cursors<I: IntoIterator<Item = super::$cur>>(
                &mut self,
                iterable: I,
            ) -> i32 {
                let mut count = 0;
                for cursor in iterable {
                    if self.add(cursor.value) {
                        count += 1;
                    }
                }
                count
            }

            /// Copies every element into a freshly allocated array.
            pub fn to_array(&self) -> Vec<$kt> {
                let mut cloned = vec![$kzero; self.size() as usize];
                let mut j = 0usize;
                if self.has_empty_key {
                    cloned[j] = $kzero;
                    j += 1;
                }

                let seed = self.next_iteration_seed();
                let inc = iteration_increment(seed);
                let mask = self.mask;
                let mut slot = seed & mask;
                let mut i = 0;
                while i <= mask {
                    let existing = self.keys[slot as usize];
                    if existing != $kzero {
                        cloned[j] = existing;
                        j += 1;
                    }
                    i += 1;
                    slot = (slot + inc) & mask;
                }

                cloned
            }

            /// Removes `key`, returning `true` if it was present.
            pub fn remove(&mut self, key: $kt) -> bool {
                if key == $kzero {
                    let had_empty_key = self.has_empty_key;
                    self.has_empty_key = false;
                    had_empty_key
                } else {
                    let mask = self.mask;
                    let mut slot = Self::hash_key(key) & mask;

                    loop {
                        let existing = self.keys[slot as usize];
                        if existing == $kzero {
                            break;
                        }
                        if key == existing {
                            self.shift_conflicting_keys(slot);
                            return true;
                        }
                        slot = (slot + 1) & mask;
                    }
                    false
                }
            }

            /// Removes all keys present in `other`, returning the number of
            /// elements actually removed.
            pub fn remove_all(&mut self, other: &Self) -> i32 {
                let before = self.size();

                // Try to iterate over the smaller set or over the container
                // that isn't implementing an efficient `contains` lookup.
                if other.size() >= self.size() {
                    if self.has_empty_key && other.contains($kzero) {
                        self.has_empty_key = false;
                    }

                    let max = self.mask;
                    let mut slot = 0;
                    while slot <= max {
                        let existing = self.keys[slot as usize];
                        if existing != $kzero && other.contains(existing) {
                            // Shift, do not increment slot.
                            self.shift_conflicting_keys(slot);
                        } else {
                            slot += 1;
                        }
                    }
                } else {
                    for c in other.iter() {
                        self.remove(c.value);
                    }
                }

                before - self.size()
            }

            /// Returns `true` if `key` is present in this set.
            pub fn contains(&self, key: $kt) -> bool {
                if key == $kzero {
                    self.has_empty_key
                } else {
                    let mask = self.mask;
                    let mut slot = Self::hash_key(key) & mask;
                    loop {
                        let existing = self.keys[slot as usize];
                        if existing == $kzero {
                            break;
                        }
                        if key == existing {
                            return true;
                        }
                        slot = (slot + 1) & mask;
                    }
                    false
                }
            }

            /// Removes every element, keeping the internal buffer.
            pub fn clear(&mut self) {
                self.assigned = 0;
                self.has_empty_key = false;
                self.keys.fill($kzero);
            }

            /// Removes every element and releases the internal buffer, sizing
            /// it back down to the default capacity.
            pub fn release(&mut self) {
                self.assigned = 0;
                self.has_empty_key = false;
                self.keys = Vec::new();
                self.ensure_capacity(DEFAULT_EXPECTED_ELEMENTS);
            }

            /// Returns `true` if this set holds no elements.
            pub fn is_empty(&self) -> bool {
                self.size() == 0
            }

            /// Ensures this container can hold at least `expected_elements`
            /// elements without resizing its buffers.
            pub fn ensure_capacity(&mut self, expected_elements: i32) {
                if expected_elements > self.resize_at || self.keys.is_empty() {
                    let prev_keys =
                        self.allocate_buffers(min_buffer_size(expected_elements, self.load_factor));
                    if !prev_keys.is_empty() && !self.is_empty() {
                        self.rehash(&prev_keys);
                    }
                }
            }

            /// Returns the number of elements in this set.
            pub fn size(&self) -> i32 {
                self.assigned + if self.has_empty_key { 1 } else { 0 }
            }

            /// Equivalent of Java's `hashCode()`, reproduced exactly.
            ///
            /// The value is order-independent, so it survives this container's
            /// randomised iteration order.
            pub fn hash_code(&self) -> i32 {
                let mut h: i32 = if self.has_empty_key {
                    0xDEAD_BEEF_u32 as i32
                } else {
                    0
                };
                let mut slot = self.mask;
                while slot >= 0 {
                    let existing = self.keys[slot as usize];
                    if existing != $kzero {
                        h = h.wrapping_add($mix_key(existing));
                    }
                    slot -= 1;
                }
                h
            }

            /// Returns `true` if all keys of `other` exist in this container.
            fn same_keys(&self, other: &Self) -> bool {
                if other.size() != self.size() {
                    return false;
                }

                for c in other.iter() {
                    if !self.contains(c.value) {
                        return false;
                    }
                }

                true
            }

            /// Returns an iterator over the elements of this set.
            pub fn iter(&self) -> EntryIterator<'_> {
                EntryIterator::new(self)
            }

            /// Provides the next iteration seed used to build the iteration
            /// starting slot and offset increment.
            fn next_iteration_seed(&self) -> i32 {
                let seed = BitMixer::mix_phi_i32(self.iteration_seed.load(Ordering::Relaxed));
                self.iteration_seed.store(seed, Ordering::Relaxed);
                seed
            }

            /// Creates a set from an array of elements, copied into the
            /// internal buffer.
            #[allow(clippy::should_implement_trait)]
            pub fn from(elements: &[$kt]) -> Self {
                let mut set = Self::with_expected_elements(elements.len() as i32);
                set.add_all_array(elements);
                set
            }

            /// Returns a hash code for `key`, distributing keys evenly across
            /// the entire integer range.
            fn hash_key(key: $kt) -> i32 {
                debug_assert!(key != $kzero); // Handled as a special case (empty slot marker).
                $hash_key(key)
            }

            /// Returns a logical "index" of `key`, usable to speed up follow-up
            /// logic.
            ///
            /// The semantics of these indexes are not strictly defined; they
            /// may well not be contiguous, and they are valid only between
            /// modifications of the set. The result is non-negative when the
            /// key exists and the bitwise complement of the insertion slot when
            /// it does not.
            pub fn index_of(&self, key: $kt) -> i32 {
                let mask = self.mask;
                if key == $kzero {
                    if self.has_empty_key {
                        mask + 1
                    } else {
                        !(mask + 1)
                    }
                } else {
                    let mut slot = Self::hash_key(key) & mask;

                    loop {
                        let existing = self.keys[slot as usize];
                        if existing == $kzero {
                            break;
                        }
                        if key == existing {
                            return slot;
                        }
                        slot = (slot + 1) & mask;
                    }

                    !slot
                }
            }

            /// Returns `true` if `index`, as returned by [`Self::index_of`],
            /// corresponds to an existing key.
            pub fn index_exists(&self, index: i32) -> bool {
                debug_assert!(
                    index < 0 || index <= self.mask || (index == self.mask + 1 && self.has_empty_key)
                );

                index >= 0
            }

            /// Returns the exact key currently stored at an existing `index`.
            ///
            /// # Panics
            ///
            /// With debug assertions enabled, panics if `index` does not point
            /// at an existing key.
            pub fn index_get(&self, index: i32) -> $kt {
                debug_assert!(index >= 0, "The index must point at an existing key.");
                debug_assert!(
                    index <= self.mask || (index == self.mask + 1 && self.has_empty_key)
                );

                self.keys[index as usize]
            }

            /// Replaces the existing key at `index` with an equivalent one and
            /// returns the previous key.
            ///
            /// # Panics
            ///
            /// With debug assertions enabled, panics if `index` does not point
            /// at an existing key, or if `equivalent_key` is not equivalent to
            /// the key currently stored there.
            pub fn index_replace(&mut self, index: i32, equivalent_key: $kt) -> $kt {
                debug_assert!(index >= 0, "The index must point at an existing key.");
                debug_assert!(
                    index <= self.mask || (index == self.mask + 1 && self.has_empty_key)
                );
                debug_assert!(self.keys[index as usize] == equivalent_key);

                let previous_value = self.keys[index as usize];
                self.keys[index as usize] = equivalent_key;
                previous_value
            }

            /// Inserts a key at an `index` that is not present in the set,
            /// avoiding a second hash computation.
            ///
            /// # Panics
            ///
            /// With debug assertions enabled, panics if `index` points at an
            /// existing key.
            pub fn index_insert(&mut self, index: i32, key: $kt) {
                debug_assert!(index < 0, "The index must not point at an existing key.");

                let index = !index;
                if key == $kzero {
                    debug_assert!(index == self.mask + 1);
                    debug_assert!(self.keys[index as usize] == $kzero);
                    self.has_empty_key = true;
                } else {
                    debug_assert!(self.keys[index as usize] == $kzero);

                    if self.assigned == self.resize_at {
                        self.allocate_then_insert_then_rehash(index, key);
                    } else {
                        self.keys[index as usize] = key;
                    }

                    self.assigned += 1;
                }
            }

            /// Removes the key at an existing `index`.
            ///
            /// # Panics
            ///
            /// With debug assertions enabled, panics if `index` does not point
            /// at an existing key.
            pub fn index_remove(&mut self, index: i32) {
                debug_assert!(index >= 0, "The index must point at an existing key.");
                debug_assert!(
                    index <= self.mask || (index == self.mask + 1 && self.has_empty_key)
                );

                if index > self.mask {
                    self.has_empty_key = false;
                } else {
                    self.shift_conflicting_keys(index);
                }
            }

            /// Validates the load factor range and returns it.
            fn verify_load_factor(load_factor: f64) -> f64 {
                check_load_factor(load_factor, MIN_LOAD_FACTOR as f64, MAX_LOAD_FACTOR as f64);
                load_factor
            }

            /// Rehashes from an old buffer into the current one.
            fn rehash(&mut self, from_keys: &[$kt]) {
                debug_assert!(check_power_of_two(from_keys.len() as i32 - 1));

                let mask = self.mask;
                let mut i = from_keys.len() - 1;
                while i > 0 {
                    i -= 1;
                    let existing = from_keys[i];
                    if existing != $kzero {
                        let mut slot = Self::hash_key(existing) & mask;
                        while self.keys[slot as usize] != $kzero {
                            slot = (slot + 1) & mask;
                        }
                        self.keys[slot as usize] = existing;
                    }
                }
            }

            /// Allocates a new internal buffer and returns the previous one.
            ///
            /// # Panics
            ///
            /// Panics with a [`BufferAllocationException`] when the allocator
            /// cannot satisfy the request, which is where Java catches
            /// `OutOfMemoryError` and throws the same exception.
            fn allocate_buffers(&mut self, array_size: i32) -> Vec<$kt> {
                debug_assert!(array_size.count_ones() == 1);

                // An extra slot stands for the "empty" key.
                let length = array_size as usize + 1;
                let mut new_keys: Vec<$kt> = Vec::new();
                if new_keys.try_reserve_exact(length).is_err() {
                    let held = if self.keys.is_empty() {
                        0
                    } else {
                        self.size() as i64
                    };
                    BufferAllocationException::new(format!(
                        "Not enough memory to allocate buffers for rehashing: {} -> {}",
                        group_digits(held),
                        group_digits(array_size as i64)
                    ))
                    .throw();
                }
                new_keys.resize(length, $kzero);
                let prev_keys = std::mem::replace(&mut self.keys, new_keys);

                self.resize_at = expand_at_count(array_size, self.load_factor);
                self.mask = array_size - 1;
                prev_keys
            }

            /// Invoked when a new key must be inserted but there are not enough
            /// empty slots.
            fn allocate_then_insert_then_rehash(&mut self, slot: i32, pending_key: $kt) {
                debug_assert!(
                    self.assigned == self.resize_at
                        && self.keys[slot as usize] == $kzero
                        && pending_key != $kzero
                );

                let next = next_buffer_size(self.mask + 1, self.size(), self.load_factor);
                let mut prev_keys = self.allocate_buffers(next);
                debug_assert!(self.keys.len() > prev_keys.len());

                // We have succeeded at allocating new data so insert the
                // pending key at the free slot in the old array before
                // rehashing.
                prev_keys[slot as usize] = pending_key;

                // Rehash old keys, including the pending key.
                self.rehash(&prev_keys);
            }

            /// Shifts all the slot-conflicting keys allocated to (and
            /// including) `gap_slot`.
            fn shift_conflicting_keys(&mut self, gap_slot: i32) {
                let mut gap_slot = gap_slot;
                let mask = self.mask;

                let mut distance = 0i32;
                loop {
                    distance += 1;
                    let slot = (gap_slot.wrapping_add(distance)) & mask;
                    let existing = self.keys[slot as usize];
                    if existing == $kzero {
                        break;
                    }

                    let ideal_slot = Self::hash_key(existing);
                    let shift = slot.wrapping_sub(ideal_slot) & mask;
                    if shift >= distance {
                        self.keys[gap_slot as usize] = existing;
                        gap_slot = slot;
                        distance = 0;
                    }
                }

                // Mark the last found gap slot without a conflict as empty.
                self.keys[gap_slot as usize] = $kzero;
                self.assigned -= 1;
            }
        }

        impl Default for $set {
            fn default() -> Self {
                Self::new()
            }
        }

        impl Clone for $set {
            /// Clones this set, reusing the same hash function and array
            /// resizing strategy but drawing a fresh iteration seed, exactly as
            /// Java's `clone()` does.
            fn clone(&self) -> Self {
                Self {
                    keys: self.keys.clone(),
                    assigned: self.assigned,
                    mask: self.mask,
                    resize_at: self.resize_at,
                    has_empty_key: self.has_empty_key,
                    load_factor: self.load_factor,
                    iteration_seed: AtomicI32::new(next_iteration_seed()),
                }
            }
        }

        impl PartialEq for $set {
            fn eq(&self, other: &Self) -> bool {
                std::ptr::eq(self, other) || self.same_keys(other)
            }
        }

        impl Eq for $set {}

        impl Hash for $set {
            fn hash<H: Hasher>(&self, state: &mut H) {
                state.write_i32(self.hash_code());
            }
        }

        impl Accountable for $set {
            fn ram_bytes_used(&self) -> i64 {
                BASE_RAM_BYTES_USED + $size_of_keys(self.keys.len())
            }
        }

        impl Debug for $set {
            fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
                write!(f, "{}[", stringify!($set))?;
                let mut first = true;
                for cursor in self.iter() {
                    if !first {
                        f.write_str(", ")?;
                    }
                    write!(f, "{}", cursor.value)?;
                    first = false;
                }
                f.write_str("]")
            }
        }

        impl<'a> IntoIterator for &'a $set {
            type Item = super::$cur;
            type IntoIter = EntryIterator<'a>;

            fn into_iter(self) -> Self::IntoIter {
                self.iter()
            }
        }

        #[doc = concat!("Iterator over the elements of a [`", stringify!($set), "`].")]
        ///
        #[doc = concat!("Port of `", $java, ".EntryIterator`.")]
        pub struct EntryIterator<'a> {
            owner: &'a $set,
            increment: i32,
            index: i32,
            slot: i32,
        }

        impl<'a> EntryIterator<'a> {
            fn new(owner: &'a $set) -> Self {
                let seed = owner.next_iteration_seed();
                Self {
                    owner,
                    increment: iteration_increment(seed),
                    index: 0,
                    slot: seed & owner.mask,
                }
            }
        }

        impl AbstractIterator for EntryIterator<'_> {
            type Item = super::$cur;

            fn fetch(&mut self) -> Option<super::$cur> {
                let mask = self.owner.mask;
                while self.index <= mask {
                    self.index += 1;
                    self.slot = (self.slot + self.increment) & mask;
                    let existing = self.owner.keys[self.slot as usize];
                    if existing != $kzero {
                        return Some(super::$cur {
                            index: self.slot,
                            value: existing,
                        });
                    }
                }

                if self.index == mask + 1 && self.owner.has_empty_key {
                    let index = self.index;
                    self.index += 1;
                    return Some(super::$cur {
                        index,
                        value: $kzero,
                    });
                }

                self.done()
            }
        }

        impl Iterator for EntryIterator<'_> {
            type Item = super::$cur;

            fn next(&mut self) -> Option<super::$cur> {
                self.fetch()
            }
        }
    };
}

pub(crate) use define_hash_set;

/// Expands to an array-backed list of a primitive element type.
///
/// Covers `IntArrayList`, `LongArrayList` and `FloatArrayList`.
macro_rules! define_array_list {
    (
        list = $list:ident,
        element = $et:ty,
        cursor = $cur:ident,
        zero = $zero:expr,
        bytes_per_element = $bpe:expr,
        mix = $mix:path,
        eq = $eq:path,
        sort = $sort:path,
        size_of_elements = $size_of_elements:path,
        base_ram_bytes_used = $base_ram:expr,
        java_class = $java:literal,
        java_element = $jelem:literal,
        element_fmt = $efmt:literal,
    ) => {
        use std::fmt::{self, Debug, Display, Formatter};
        use std::hash::{Hash, Hasher};

        use super::abstract_iterator::AbstractIterator;
        use super::hash_containers::DEFAULT_EXPECTED_ELEMENTS;
        use super::support::grow;
        use crate::util::Accountable;

        #[doc = concat!("Shallow size of a `", $java, "` instance, as `RamUsageEstimator.shallowSizeOfInstance` computes it.")]
        const BASE_RAM_BYTES_USED: i64 = $base_ram;

        #[doc = concat!("Port of `org.apache.lucene.internal.hppc.", $java, "`.")]
        ///
        #[doc = concat!("An array-backed list of `", $jelem, "`.")]
        ///
        /// Lucene forked and trimmed this from HPPC 0.10.0
        #[doc = concat!("(`com.carrotsearch.hppc.", $java, "`).")]
        ///
        /// Indices and sizes are `i32` throughout, as in Java, so that the
        /// bounds checks and the `-1` returned by the search methods keep their
        /// original meaning.
        #[derive(Clone)]
        pub struct $list {
            /// Internal array for storing the list.
            ///
            /// The array may be larger than the current size
            /// ([`Self::size`]).
            pub buffer: Vec<$et>,

            /// Current number of elements stored in [`Self::buffer`].
            pub elements_count: i32,
        }

        impl $list {
            /// An immutable empty buffer.
            ///
            /// Equivalent of Java's `EMPTY_ARRAY`, which exists so that
            /// [`Self::release`] can drop the buffer without allocating.
            pub const EMPTY_ARRAY: &'static [$et] = &[];

            /// New instance with sane defaults.
            pub fn new() -> Self {
                Self::with_expected_elements(DEFAULT_EXPECTED_ELEMENTS)
            }

            /// New instance sized for `expected_elements` without expansion.
            pub fn with_expected_elements(expected_elements: i32) -> Self {
                Self {
                    buffer: vec![$zero; expected_elements as usize],
                    elements_count: 0,
                }
            }

            /// Creates a new list from the elements of another list, in its
            /// iteration order.
            pub fn from_list(list: &Self) -> Self {
                let mut copy = Self::with_expected_elements(list.size());
                copy.add_all(list);
                copy
            }

            /// Appends an element to the list.
            pub fn add(&mut self, e1: $et) {
                self.ensure_buffer_space(1);
                self.buffer[self.elements_count as usize] = e1;
                self.elements_count += 1;
            }

            /// Appends all elements from a range of the given array.
            pub fn add_range(&mut self, elements: &[$et], start: i32, length: i32) {
                debug_assert!(length >= 0, "Length must be >= 0");

                self.ensure_buffer_space(length);
                let to = self.elements_count as usize;
                self.buffer[to..to + length as usize]
                    .copy_from_slice(&elements[start as usize..(start + length) as usize]);
                self.elements_count += length;
            }

            /// Appends every element of the given array.
            ///
            /// Equivalent of Java's varargs `add`, which is handy but costly in
            /// tight loops because of the anonymous array it passes.
            pub fn add_array(&mut self, elements: &[$et]) {
                self.add_range(elements, 0, elements.len() as i32);
            }

            /// Adds all elements from another list, returning how many were
            /// added.
            pub fn add_all(&mut self, list: &Self) -> i32 {
                let size = list.size();
                self.ensure_buffer_space(size);

                for cursor in list.iter() {
                    self.add(cursor.value);
                }

                size
            }

            /// Adds all elements from the given cursors, returning how many
            /// were added.
            pub fn add_all_cursors<I: IntoIterator<Item = super::$cur>>(
                &mut self,
                iterable: I,
            ) -> i32 {
                let mut size = 0;
                for cursor in iterable {
                    self.add(cursor.value);
                    size += 1;
                }
                size
            }

            /// Inserts an element at `index`, shifting the tail right.
            ///
            /// # Panics
            ///
            /// With debug assertions enabled, panics if `index` is out of the
            /// inclusive range `[0, size()]`, as Java's assertion does.
            pub fn insert(&mut self, index: i32, e1: $et) {
                debug_assert!(
                    index >= 0 && index <= self.size(),
                    "Index out of bounds [0, size()]."
                );

                self.ensure_buffer_space(1);
                self.buffer
                    .copy_within(index as usize..self.elements_count as usize, index as usize + 1);
                self.buffer[index as usize] = e1;
                self.elements_count += 1;
            }

            /// Returns the element at `index`.
            ///
            /// # Panics
            ///
            /// With debug assertions enabled, panics if `index` is out of the
            /// half-open range `[0, size())`, as Java's assertion does.
            pub fn get(&self, index: i32) -> $et {
                debug_assert!(
                    index >= 0 && index < self.size(),
                    "Index out of bounds [0, size())."
                );

                self.buffer[index as usize]
            }

            /// Replaces the element at `index`, returning the previous one.
            ///
            /// # Panics
            ///
            /// With debug assertions enabled, panics if `index` is out of the
            /// half-open range `[0, size())`.
            pub fn set(&mut self, index: i32, e1: $et) -> $et {
                debug_assert!(
                    index >= 0 && index < self.size(),
                    "Index out of bounds [0, size())."
                );

                let v = self.buffer[index as usize];
                self.buffer[index as usize] = e1;
                v
            }

            /// Removes the element at `index` and returns it.
            ///
            /// # Panics
            ///
            /// With debug assertions enabled, panics if `index` is out of the
            /// half-open range `[0, size())`.
            pub fn remove_at(&mut self, index: i32) -> $et {
                debug_assert!(
                    index >= 0 && index < self.size(),
                    "Index out of bounds [0, size())."
                );

                let v = self.buffer[index as usize];
                self.elements_count -= 1;
                self.buffer
                    .copy_within(index as usize + 1..self.elements_count as usize + 1, index as usize);
                v
            }

            /// Removes and returns the last element of this list.
            ///
            /// # Panics
            ///
            /// With debug assertions enabled, panics if the list is empty.
            pub fn remove_last(&mut self) -> $et {
                debug_assert!(!self.is_empty(), "List is empty");

                self.elements_count -= 1;
                self.buffer[self.elements_count as usize]
            }

            /// Removes the elements whose indexes lie in
            /// `[from_index, to_index)`.
            ///
            /// # Panics
            ///
            /// With debug assertions enabled, panics if either bound is out of
            /// range or if `from_index` is greater than `to_index`.
            pub fn remove_range(&mut self, from_index: i32, to_index: i32) {
                debug_assert!(
                    from_index >= 0 && from_index <= self.size(),
                    "fromIndex out of bounds [0, size())."
                );
                debug_assert!(
                    to_index >= 0 && to_index <= self.size(),
                    "toIndex out of bounds [0, size()]."
                );
                debug_assert!(from_index <= to_index, "fromIndex must be <= toIndex");

                self.buffer
                    .copy_within(to_index as usize..self.elements_count as usize, from_index as usize);
                let count = to_index - from_index;
                self.elements_count -= count;
            }

            /// Removes the first element equal to `e`, reporting whether one
            /// was removed.
            pub fn remove_element(&mut self, e: $et) -> bool {
                self.remove_first(e) != -1
            }

            /// Removes the first element equal to `e1`, returning its deleted
            /// position or `-1` if it was not found.
            pub fn remove_first(&mut self, e1: $et) -> i32 {
                let index = self.index_of(e1);
                if index >= 0 {
                    self.remove_at(index);
                }
                index
            }

            /// Removes the last element equal to `e1`, returning its deleted
            /// position or `-1` if it was not found.
            ///
            /// Named apart from [`Self::remove_last`] because Rust has no
            /// overloading; Java calls both `removeLast`.
            pub fn remove_last_element(&mut self, e1: $et) -> i32 {
                let index = self.last_index_of(e1);
                if index >= 0 {
                    self.remove_at(index);
                }
                index
            }

            /// Removes every occurrence of `e`, returning how many were
            /// removed.
            pub fn remove_all(&mut self, e: $et) -> i32 {
                let mut to = 0;
                for from in 0..self.elements_count as usize {
                    if $eq(e, self.buffer[from]) {
                        continue;
                    }
                    if to != from {
                        let moved = self.buffer[from];
                        self.buffer[to] = moved;
                    }
                    to += 1;
                }
                let deleted = self.elements_count - to as i32;
                self.elements_count = to as i32;
                deleted
            }

            /// Returns `true` if the list holds an element equal to `e1`.
            pub fn contains(&self, e1: $et) -> bool {
                self.index_of(e1) >= 0
            }

            /// Returns the index of the first element equal to `e1`, or `-1`.
            pub fn index_of(&self, e1: $et) -> i32 {
                for i in 0..self.elements_count {
                    if $eq(e1, self.buffer[i as usize]) {
                        return i;
                    }
                }

                -1
            }

            /// Returns the index of the last element equal to `e1`, or `-1`.
            pub fn last_index_of(&self, e1: $et) -> i32 {
                let mut i = self.elements_count - 1;
                while i >= 0 {
                    if $eq(e1, self.buffer[i as usize]) {
                        return i;
                    }
                    i -= 1;
                }

                -1
            }

            /// Returns `true` if this list holds no elements.
            pub fn is_empty(&self) -> bool {
                self.elements_count == 0
            }

            /// Ensures this container can hold at least `expected_elements`
            /// elements without resizing its buffer.
            pub fn ensure_capacity(&mut self, expected_elements: i32) {
                if expected_elements > self.buffer.len() as i32 {
                    self.ensure_buffer_space(expected_elements - self.size());
                }
            }

            /// Ensures the internal buffer has enough free slots to store
            /// `expected_additions` more elements, growing it if needed.
            pub fn ensure_buffer_space(&mut self, expected_additions: i32) {
                if self.elements_count + expected_additions > self.buffer.len() as i32 {
                    grow(
                        &mut self.buffer,
                        self.elements_count + expected_additions,
                        $bpe,
                    );
                }
            }

            /// Truncates or expands the list to `new_size`.
            ///
            /// A truncation does not reallocate the buffer (use
            /// [`Self::trim_to_size`] for that) but does reset the truncated
            /// values to zero. An expansion initialises the new elements to
            /// zero, matching the JVM's array defaults.
            pub fn resize(&mut self, new_size: i32) {
                if new_size <= self.buffer.len() as i32 {
                    if new_size < self.elements_count {
                        self.buffer[new_size as usize..self.elements_count as usize].fill($zero);
                    } else {
                        self.buffer[self.elements_count as usize..new_size as usize].fill($zero);
                    }
                } else {
                    self.ensure_capacity(new_size);
                }
                self.elements_count = new_size;
            }

            /// Returns the number of elements in this list.
            pub fn size(&self) -> i32 {
                self.elements_count
            }

            /// Trims the internal buffer to the current size.
            pub fn trim_to_size(&mut self) {
                if self.size() != self.buffer.len() as i32 {
                    self.buffer = self.to_array();
                }
            }

            /// Sets the number of stored elements to zero, resetting the
            /// storage array to default values.
            ///
            /// To clear the list without cleaning the buffer, set
            /// [`Self::elements_count`] to zero directly, exactly as in Java.
            pub fn clear(&mut self) {
                self.buffer[0..self.elements_count as usize].fill($zero);
                self.elements_count = 0;
            }

            /// Sets the number of stored elements to zero and releases the
            /// internal storage array.
            pub fn release(&mut self) {
                self.buffer = Self::EMPTY_ARRAY.to_vec();
                self.elements_count = 0;
            }

            /// Returns an array sized to match exactly the number of elements
            /// of this list.
            pub fn to_array(&self) -> Vec<$et> {
                self.buffer[0..self.elements_count as usize].to_vec()
            }

            /// Equivalent of Java's `hashCode()`, reproduced exactly.
            pub fn hash_code(&self) -> i32 {
                let mut h: i32 = 1;
                for i in 0..self.elements_count as usize {
                    h = h.wrapping_mul(31).wrapping_add($mix(self.buffer[i]));
                }
                h
            }

            /// Compares index-aligned elements against another list.
            fn equal_elements(&self, other: &Self) -> bool {
                let max = self.size();
                if other.size() != max {
                    return false;
                }

                for i in 0..max {
                    if !$eq(self.get(i), other.get(i)) {
                        return false;
                    }
                }

                true
            }

            /// Sorts the elements in this list and returns it, for chaining.
            pub fn sort(&mut self) -> &mut Self {
                $sort(&mut self.buffer[0..self.elements_count as usize]);
                self
            }

            /// Reverses the elements in this list and returns it, for chaining.
            pub fn reverse(&mut self) -> &mut Self {
                self.buffer[0..self.elements_count as usize].reverse();
                self
            }

            /// Returns an iterator over the elements of this list.
            pub fn iter(&self) -> ValueIterator<'_> {
                ValueIterator::new(&self.buffer, self.size())
            }

            /// Creates a list from an array of elements, copied into the
            /// internal buffer.
            #[allow(clippy::should_implement_trait)]
            pub fn from(elements: &[$et]) -> Self {
                let mut list = Self::with_expected_elements(elements.len() as i32);
                list.add_array(elements);
                list
            }
        }

        impl Default for $list {
            fn default() -> Self {
                Self::new()
            }
        }

        impl PartialEq for $list {
            fn eq(&self, other: &Self) -> bool {
                std::ptr::eq(self, other) || self.equal_elements(other)
            }
        }

        impl Hash for $list {
            fn hash<H: Hasher>(&self, state: &mut H) {
                state.write_i32(self.hash_code());
            }
        }

        impl Accountable for $list {
            fn ram_bytes_used(&self) -> i64 {
                BASE_RAM_BYTES_USED + $size_of_elements(self.buffer.len())
            }
        }

        impl Display for $list {
            /// Converts the contents of this list to a human-friendly string,
            /// the way `java.util.Arrays.toString` does.
            fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
                f.write_str("[")?;
                for i in 0..self.elements_count as usize {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, concat!("{", $efmt, "}"), self.buffer[i])?;
                }
                f.write_str("]")
            }
        }

        impl Debug for $list {
            fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
                write!(f, "{}", stringify!($list))?;
                f.write_str("[")?;
                for i in 0..self.elements_count as usize {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{:?}", self.buffer[i])?;
                }
                f.write_str("]")
            }
        }

        impl<'a> IntoIterator for &'a $list {
            type Item = super::$cur;
            type IntoIter = ValueIterator<'a>;

            fn into_iter(self) -> Self::IntoIter {
                self.iter()
            }
        }

        #[doc = concat!("Iterator over the elements of a [`", stringify!($list), "`].")]
        ///
        #[doc = concat!("Port of `", $java, ".ValueIterator`.")]
        pub struct ValueIterator<'a> {
            buffer: &'a [$et],
            size: i32,
            index: i32,
        }

        impl<'a> ValueIterator<'a> {
            /// Creates an iterator over the first `size` elements of `buffer`.
            fn new(buffer: &'a [$et], size: i32) -> Self {
                Self {
                    buffer,
                    size,
                    index: -1,
                }
            }
        }

        impl AbstractIterator for ValueIterator<'_> {
            type Item = super::$cur;

            fn fetch(&mut self) -> Option<super::$cur> {
                if self.index + 1 == self.size {
                    return self.done();
                }

                self.index += 1;
                Some(super::$cur {
                    index: self.index,
                    value: self.buffer[self.index as usize],
                })
            }
        }

        impl Iterator for ValueIterator<'_> {
            type Item = super::$cur;

            fn next(&mut self) -> Option<super::$cur> {
                self.fetch()
            }
        }
    };
}

pub(crate) use define_array_list;

/// Expands to an array-backed list with a maximum size limit.
///
/// Covers `MaxSizedIntArrayList` and `MaxSizedFloatArrayList`.
///
/// # Adaptation
///
/// In Java these extend their unbounded counterpart and override a single
/// method, `ensureBufferSpace`, which every growing operation calls through
/// virtual dispatch. Rust has no inheritance, so the bounded list *contains*
/// the unbounded one:
///
/// * read-only inherited behaviour is reached through [`std::ops::Deref`];
/// * every operation that can grow the buffer is restated here so that it goes
///   through the bounded `ensure_buffer_space`, which is precisely what the
///   override achieves in Java;
/// * every other mutator is forwarded verbatim.
///
/// [`std::ops::DerefMut`] is deliberately **not** implemented: it would hand
/// out the unbounded `add`, `insert` and `resize`, defeating the very limit
/// this type exists to enforce. For the same reason the inherited `buffer` and
/// `elements_count` fields are readable but not writable through this type.
macro_rules! define_max_sized_array_list {
    (
        list = $list:ident,
        base = $base:ident,
        element = $et:ty,
        cursor = $cur:ident,
        bytes_per_element = $bpe:expr,
        mix = $mix:path,
        size_of_elements = $size_of_elements:path,
        base_ram_bytes_used = $base_ram:expr,
        java_class = $java:literal,
        java_base = $jbase:literal,
        element_fmt = $efmt:literal,
    ) => {
        use std::fmt::{self, Debug, Display, Formatter};
        use std::hash::{Hash, Hasher};
        use std::ops::Deref;

        use super::hash_containers::DEFAULT_EXPECTED_ELEMENTS;
        use super::support::grow_in_range;
        use super::$base;
        use crate::util::Accountable;

        #[doc = concat!("Shallow size of a `", $java, "` instance, as `RamUsageEstimator.shallowSizeOfInstance` computes it.")]
        const BASE_RAM_BYTES_USED: i64 = $base_ram;

        #[doc = concat!("Port of `org.apache.lucene.internal.hppc.", $java, "`.")]
        ///
        #[doc = concat!("An array-backed list of the same element type as [`", $jbase, "`], with a maximum size limit.")]
        ///
        /// Growing past that limit is a programming error and panics, which is
        /// how Java's unchecked `IllegalStateException` behaves.
        #[derive(Clone)]
        pub struct $list {
            list: $base,
            max_size: i32,
        }

        impl $list {
            /// New instance with sane defaults, limited to `max_size` elements.
            pub fn new(max_size: i32) -> Self {
                Self::with_expected_elements(max_size, DEFAULT_EXPECTED_ELEMENTS)
            }

            /// New instance limited to `max_size` elements and sized for
            /// `expected_elements` without expansion.
            ///
            /// # Panics
            ///
            /// With debug assertions enabled, panics if `expected_elements`
            /// exceeds `max_size`, as Java's assertion does.
            pub fn with_expected_elements(max_size: i32, expected_elements: i32) -> Self {
                debug_assert!(
                    expected_elements <= max_size,
                    "expectedElements must be <= maxSize"
                );
                Self {
                    list: $base::with_expected_elements(expected_elements),
                    max_size,
                }
            }

            /// Creates a new list from the elements of another list, in its
            /// iteration order, keeping its maximum size.
            pub fn from_list(list: &Self) -> Self {
                let mut copy = Self {
                    list: $base::with_expected_elements(list.size()),
                    max_size: list.max_size,
                };
                copy.add_all(&list.list);
                copy
            }

            /// The maximum number of elements this list may hold.
            ///
            /// Java reaches the field directly from inside the package; Rust
            /// needs an accessor for it to be readable at all.
            pub fn max_size(&self) -> i32 {
                self.max_size
            }

            /// Ensures the internal buffer has enough free slots to store
            /// `expected_additions` more elements, growing it if needed and
            /// never beyond [`Self::max_size`].
            ///
            /// # Panics
            ///
            /// Panics if the request would grow the list beyond its maximum
            /// size, where Java throws `IllegalStateException`.
            pub fn ensure_buffer_space(&mut self, expected_additions: i32) {
                if self.list.elements_count + expected_additions > self.max_size {
                    panic!("Cannot grow beyond maxSize: {}", self.max_size);
                }
                if self.list.elements_count + expected_additions > self.list.buffer.len() as i32 {
                    grow_in_range(
                        &mut self.list.buffer,
                        self.list.elements_count + expected_additions,
                        self.max_size,
                        $bpe,
                    );
                }
            }

            /// Appends an element to the list.
            pub fn add(&mut self, e1: $et) {
                self.ensure_buffer_space(1);
                self.list.buffer[self.list.elements_count as usize] = e1;
                self.list.elements_count += 1;
            }

            /// Appends all elements from a range of the given array.
            pub fn add_range(&mut self, elements: &[$et], start: i32, length: i32) {
                debug_assert!(length >= 0, "Length must be >= 0");

                self.ensure_buffer_space(length);
                let to = self.list.elements_count as usize;
                self.list.buffer[to..to + length as usize]
                    .copy_from_slice(&elements[start as usize..(start + length) as usize]);
                self.list.elements_count += length;
            }

            /// Appends every element of the given array.
            pub fn add_array(&mut self, elements: &[$et]) {
                self.add_range(elements, 0, elements.len() as i32);
            }

            /// Adds all elements from another list, returning how many were
            /// added.
            pub fn add_all(&mut self, list: &$base) -> i32 {
                let size = list.size();
                self.ensure_buffer_space(size);

                for cursor in list.iter() {
                    self.add(cursor.value);
                }

                size
            }

            /// Adds all elements from the given cursors, returning how many
            /// were added.
            pub fn add_all_cursors<I: IntoIterator<Item = super::$cur>>(
                &mut self,
                iterable: I,
            ) -> i32 {
                let mut size = 0;
                for cursor in iterable {
                    self.add(cursor.value);
                    size += 1;
                }
                size
            }

            /// Inserts an element at `index`, shifting the tail right.
            pub fn insert(&mut self, index: i32, e1: $et) {
                debug_assert!(
                    index >= 0 && index <= self.size(),
                    "Index out of bounds [0, size()]."
                );

                self.ensure_buffer_space(1);
                let count = self.list.elements_count as usize;
                self.list
                    .buffer
                    .copy_within(index as usize..count, index as usize + 1);
                self.list.buffer[index as usize] = e1;
                self.list.elements_count += 1;
            }

            /// Ensures this container can hold at least `expected_elements`
            /// elements without resizing its buffer.
            pub fn ensure_capacity(&mut self, expected_elements: i32) {
                if expected_elements > self.list.buffer.len() as i32 {
                    self.ensure_buffer_space(expected_elements - self.size());
                }
            }

            /// Truncates or expands the list to `new_size`.
            pub fn resize(&mut self, new_size: i32) {
                if new_size <= self.list.buffer.len() as i32 {
                    self.list.resize(new_size);
                } else {
                    self.ensure_capacity(new_size);
                    self.list.elements_count = new_size;
                }
            }

            /// Replaces the element at `index`, returning the previous one.
            pub fn set(&mut self, index: i32, e1: $et) -> $et {
                self.list.set(index, e1)
            }

            /// Removes the element at `index` and returns it.
            pub fn remove_at(&mut self, index: i32) -> $et {
                self.list.remove_at(index)
            }

            /// Removes and returns the last element of this list.
            pub fn remove_last(&mut self) -> $et {
                self.list.remove_last()
            }

            /// Removes the elements whose indexes lie in
            /// `[from_index, to_index)`.
            pub fn remove_range(&mut self, from_index: i32, to_index: i32) {
                self.list.remove_range(from_index, to_index);
            }

            /// Removes the first element equal to `e`, reporting whether one
            /// was removed.
            pub fn remove_element(&mut self, e: $et) -> bool {
                self.list.remove_element(e)
            }

            /// Removes the first element equal to `e1`, returning its deleted
            /// position or `-1` if it was not found.
            pub fn remove_first(&mut self, e1: $et) -> i32 {
                self.list.remove_first(e1)
            }

            /// Removes the last element equal to `e1`, returning its deleted
            /// position or `-1` if it was not found.
            pub fn remove_last_element(&mut self, e1: $et) -> i32 {
                self.list.remove_last_element(e1)
            }

            /// Removes every occurrence of `e`, returning how many were
            /// removed.
            pub fn remove_all(&mut self, e: $et) -> i32 {
                self.list.remove_all(e)
            }

            /// Sets the number of stored elements to zero, resetting the
            /// storage array to default values.
            pub fn clear(&mut self) {
                self.list.clear();
            }

            /// Sets the number of stored elements to zero and releases the
            /// internal storage array.
            pub fn release(&mut self) {
                self.list.release();
            }

            /// Trims the internal buffer to the current size.
            pub fn trim_to_size(&mut self) {
                self.list.trim_to_size();
            }

            /// Sorts the elements in this list and returns it, for chaining.
            pub fn sort(&mut self) -> &mut Self {
                self.list.sort();
                self
            }

            /// Reverses the elements in this list and returns it, for chaining.
            pub fn reverse(&mut self) -> &mut Self {
                self.list.reverse();
                self
            }

            /// Equivalent of Java's `hashCode()`, reproduced exactly: it mixes
            /// the maximum size in before the elements.
            pub fn hash_code(&self) -> i32 {
                let mut h: i32 = 1;
                h = h.wrapping_mul(31).wrapping_add(self.max_size);
                for i in 0..self.list.elements_count as usize {
                    h = h.wrapping_mul(31).wrapping_add($mix(self.list.buffer[i]));
                }
                h
            }
        }

        impl Deref for $list {
            type Target = $base;

            /// Grants read-only access to everything inherited from
            #[doc = concat!("[`", stringify!($base), "`].")]
            fn deref(&self) -> &Self::Target {
                &self.list
            }
        }

        impl PartialEq for $list {
            /// Two bounded lists are equal only when they agree on both their
            /// maximum size and their elements.
            fn eq(&self, other: &Self) -> bool {
                self.max_size == other.max_size && self.list == other.list
            }
        }

        impl Hash for $list {
            fn hash<H: Hasher>(&self, state: &mut H) {
                state.write_i32(self.hash_code());
            }
        }

        impl Accountable for $list {
            fn ram_bytes_used(&self) -> i64 {
                BASE_RAM_BYTES_USED + $size_of_elements(self.list.buffer.len())
            }
        }

        impl Display for $list {
            fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
                Display::fmt(&self.list, f)
            }
        }

        impl Debug for $list {
            fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
                write!(f, "{}[", stringify!($list))?;
                for i in 0..self.list.elements_count as usize {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, concat!("{", $efmt, "}"), self.list.buffer[i])?;
                }
                write!(f, "], maxSize: {}", self.max_size)
            }
        }

        impl<'a> IntoIterator for &'a $list {
            type Item = super::$cur;
            type IntoIter = <&'a $base as IntoIterator>::IntoIter;

            fn into_iter(self) -> Self::IntoIter {
                self.list.iter()
            }
        }
    };
}

pub(crate) use define_max_sized_array_list;
