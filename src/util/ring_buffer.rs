//! Circular buffers ported from `org.apache.lucene.util`.
//!
//! | Rucene | Apache Lucene Core 10.5.0 |
//! | --- | --- |
//! | [`RollingBuffer`] / [`Resettable`] | `RollingBuffer<T>` and its nested `Resettable` |
//! | [`FrequencyTrackingRingBuffer`] | `FrequencyTrackingRingBuffer` |

#![deny(unsafe_code)]

use std::collections::HashMap;

use crate::error::{LuceneError, Result};
use crate::util::{Accountable, ArrayUtil, RamUsageEstimator};

// ---------------------------------------------------------------------------
// RollingBuffer
// ---------------------------------------------------------------------------

/// An instance a [`RollingBuffer`] can recycle.
///
/// Port of the nested interface `RollingBuffer.Resettable`.
pub trait Resettable {
    /// Returns this instance to its initial state.
    fn reset(&mut self);
}

/// Behaves like a forever-growing `T[]` while internally reusing instances of
/// `T` through a circular buffer.
///
/// Port of `org.apache.lucene.util.RollingBuffer`.
///
/// **Divergence from Lucene 10.5.0.** Java declares the class abstract with an
/// abstract `newInstance()`; the subclass supplies the factory. Rust has no
/// inheritance, so the factory is a closure held by the buffer — the same
/// injected behaviour, made explicit.
pub struct RollingBuffer<T, F>
where
    T: Resettable,
    F: FnMut() -> T,
{
    buffer: Vec<T>,
    new_instance: F,
    /// Next array index to write to.
    next_write: usize,
    /// Next position to write.
    next_pos: i32,
    /// How many valid positions are held in the array.
    count: usize,
}

impl<T, F> RollingBuffer<T, F>
where
    T: Resettable,
    F: FnMut() -> T,
{
    /// Creates a buffer of eight pre-built instances, as Java's constructor
    /// does.
    pub fn new(mut new_instance: F) -> Self {
        let buffer = (0..8).map(|_| new_instance()).collect();
        Self {
            buffer,
            new_instance,
            next_write: 0,
            next_pos: 0,
            count: 0,
        }
    }

    /// Resets every live instance and rewinds the buffer.
    pub fn reset(&mut self) {
        // Java decrements first and wraps inside the loop.
        let len = self.buffer.len();
        let mut next_write = self.next_write as isize - 1;
        while self.count > 0 {
            if next_write == -1 {
                next_write = len as isize - 1;
            }
            self.buffer[next_write as usize].reset();
            next_write -= 1;
            self.count -= 1;
        }
        self.next_write = 0;
        self.next_pos = 0;
        self.count = 0;
    }

    /// For assertions: whether `pos` is inside the live window.
    fn in_bounds(&self, pos: i32) -> bool {
        pos < self.next_pos && pos >= self.next_pos - self.count as i32
    }

    /// Maps an absolute position to its slot in the circular buffer.
    fn get_index(&self, pos: i32) -> usize {
        let mut index = self.next_write as isize - (self.next_pos - pos) as isize;
        if index < 0 {
            index += self.buffer.len() as isize;
        }
        index as usize
    }

    /// Returns the instance for this absolute position.
    ///
    /// The position may be arbitrarily far in the future but must not precede
    /// the last [`RollingBuffer::free_before`].
    pub fn get(&mut self, pos: i32) -> &mut T {
        while pos >= self.next_pos {
            if self.count == self.buffer.len() {
                let old_len = self.buffer.len();
                let new_len = ArrayUtil::oversize(
                    1 + self.count,
                    RamUsageEstimator::NUM_BYTES_OBJECT_REF as usize,
                )
                .max(1 + self.count);
                // Java copies the ring into a fresh array so that the live
                // window starts at index 0; rotating the Vec left by
                // `next_write` has exactly that effect.
                self.buffer.rotate_left(self.next_write);
                for _ in old_len..new_len {
                    let instance = (self.new_instance)();
                    self.buffer.push(instance);
                }
                self.next_write = old_len;
            }
            if self.next_write == self.buffer.len() {
                self.next_write = 0;
            }
            // The slot has already been reset.
            self.next_write += 1;
            self.next_pos += 1;
            self.count += 1;
        }
        debug_assert!(
            self.in_bounds(pos),
            "pos={} nextPos={} count={}",
            pos,
            self.next_pos,
            self.count
        );
        let index = self.get_index(pos);
        &mut self.buffer[index]
    }

    /// Returns the maximum position looked up, or `-1` since the last reset.
    pub fn get_max_pos(&self) -> i32 {
        self.next_pos - 1
    }

    /// Returns how many positions are live in the buffer.
    pub fn get_buffer_size(&self) -> usize {
        self.count
    }

    /// Releases every position before `pos`.
    pub fn free_before(&mut self, pos: i32) {
        let to_free = self.count as i32 - (self.next_pos - pos);
        debug_assert!(to_free >= 0);
        debug_assert!(
            to_free <= self.count as i32,
            "toFree={to_free} count={}",
            self.count
        );
        let mut index = self.next_write as isize - self.count as isize;
        if index < 0 {
            index += self.buffer.len() as isize;
        }
        let mut index = index as usize;
        for _ in 0..to_free {
            if index == self.buffer.len() {
                index = 0;
            }
            self.buffer[index].reset();
            index += 1;
        }
        self.count -= to_free as usize;
    }
}

// ---------------------------------------------------------------------------
// FrequencyTrackingRingBuffer
// ---------------------------------------------------------------------------

/// A ring buffer that tracks the frequency of the integers it contains.
///
/// Typically used to track the hash codes of popular recently-used items. It
/// requires 22 bytes per entry on average (between 16 and 28).
///
/// Port of `org.apache.lucene.util.FrequencyTrackingRingBuffer`.
#[derive(Debug, Clone)]
pub struct FrequencyTrackingRingBuffer {
    max_size: usize,
    buffer: Vec<i32>,
    position: usize,
    frequencies: IntBag,
}

impl FrequencyTrackingRingBuffer {
    /// Creates a ring buffer holding at most `max_size` items, initially filled
    /// with `max_size` copies of `sentinel`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `max_size` is below 2.
    pub fn new(max_size: usize, sentinel: i32) -> Result<Self> {
        if max_size < 2 {
            return Err(LuceneError::IllegalArgument(
                "maxSize must be at least 2".to_string(),
            ));
        }
        let mut frequencies = IntBag::new(max_size);
        let buffer = vec![sentinel; max_size];
        for _ in 0..max_size {
            frequencies.add(sentinel);
        }
        debug_assert_eq!(frequencies.frequency(sentinel) as usize, max_size);
        Ok(Self {
            max_size,
            buffer,
            position: 0,
            frequencies,
        })
    }

    /// Adds an item, evicting the oldest entry when the buffer is full.
    pub fn add(&mut self, i: i32) {
        // Remove the previous value.
        let removed = self.buffer[self.position];
        let removed_from_bag = self.frequencies.remove(removed);
        debug_assert!(removed_from_bag);
        // Add the new value.
        self.buffer[self.position] = i;
        self.frequencies.add(i);
        // Advance the position.
        self.position += 1;
        if self.position == self.max_size {
            self.position = 0;
        }
    }

    /// Returns the frequency of `key` in the ring buffer.
    pub fn frequency(&self, key: i32) -> i32 {
        self.frequencies.frequency(key)
    }

    /// Returns the tracked frequencies as a map.
    ///
    /// Package-private in Java, where it exists for tests.
    pub fn as_frequency_map(&self) -> HashMap<i32, i32> {
        self.frequencies.as_map()
    }
}

impl Accountable for FrequencyTrackingRingBuffer {
    fn ram_bytes_used(&self) -> i64 {
        RamUsageEstimator::align_object_size(
            RamUsageEstimator::NUM_BYTES_OBJECT_HEADER
                + 2 * RamUsageEstimator::NUM_BYTES_OBJECT_REF,
        ) + self.frequencies.ram_bytes_used()
            + RamUsageEstimator::size_of_int(&self.buffer)
    }
}

/// A bag of integers whose maximum size is known up front, so the backing
/// storage never has to be resized.
///
/// Port of the nested class `FrequencyTrackingRingBuffer.IntBag`.
#[derive(Debug, Clone)]
struct IntBag {
    keys: Vec<i32>,
    freqs: Vec<i32>,
    mask: usize,
}

impl IntBag {
    fn new(max_size: usize) -> Self {
        // Load factor of 2/3.
        let mut capacity = (max_size * 3 / 2).max(2);
        // Round up to the next power of two: `Integer.highestOneBit(capacity - 1) << 1`.
        capacity = highest_one_bit(capacity - 1) << 1;
        debug_assert!(capacity > max_size);
        Self {
            keys: vec![0; capacity],
            freqs: vec![0; capacity],
            mask: capacity - 1,
        }
    }

    fn ram_bytes_used(&self) -> i64 {
        RamUsageEstimator::align_object_size(
            RamUsageEstimator::NUM_BYTES_OBJECT_HEADER
                + 2 * RamUsageEstimator::NUM_BYTES_OBJECT_REF
                + 4,
        ) + RamUsageEstimator::size_of_int(&self.keys)
            + RamUsageEstimator::size_of_int(&self.freqs)
    }

    /// Returns the frequency of `key` in the bag.
    fn frequency(&self, key: i32) -> i32 {
        let mut slot = (key as usize) & self.mask;
        loop {
            if self.keys[slot] == key {
                return self.freqs[slot];
            } else if self.freqs[slot] == 0 {
                return 0;
            }
            slot = (slot + 1) & self.mask;
        }
    }

    /// Increments the frequency of `key` by one and returns its new frequency.
    fn add(&mut self, key: i32) -> i32 {
        let mut slot = (key as usize) & self.mask;
        loop {
            if self.freqs[slot] == 0 {
                self.keys[slot] = key;
                self.freqs[slot] = 1;
                return 1;
            } else if self.keys[slot] == key {
                self.freqs[slot] += 1;
                return self.freqs[slot];
            }
            slot = (slot + 1) & self.mask;
        }
    }

    /// Decrements the frequency of `key` by one, doing nothing when it is
    /// absent. Returns whether the key was in the bag.
    fn remove(&mut self, key: i32) -> bool {
        let mut slot = (key as usize) & self.mask;
        loop {
            if self.freqs[slot] == 0 {
                // No such key in the bag.
                return false;
            } else if self.keys[slot] == key {
                self.freqs[slot] -= 1;
                if self.freqs[slot] == 0 {
                    self.relocate_adjacent_keys(slot);
                }
                return true;
            }
            slot = (slot + 1) & self.mask;
        }
    }

    fn relocate_adjacent_keys(&mut self, free_slot: usize) {
        let mut free_slot = free_slot;
        let mut slot = (free_slot + 1) & self.mask;
        loop {
            let freq = self.freqs[slot];
            if freq == 0 {
                // End of the collision chain: done.
                break;
            }
            let key = self.keys[slot];
            // The slot where `key` would be if there were no collisions.
            let expected_slot = (key as usize) & self.mask;
            // If the free slot is between the expected slot and the slot where
            // the key actually is, we can relocate there.
            if Self::between(expected_slot, slot, free_slot) {
                self.keys[free_slot] = key;
                self.freqs[free_slot] = freq;
                // `slot` is the new free slot.
                self.freqs[slot] = 0;
                free_slot = slot;
            }
            slot = (slot + 1) & self.mask;
        }
    }

    /// Given a chain of occupied slots between `chain_start` and `chain_end`,
    /// returns whether `slot` lies inside it.
    fn between(chain_start: usize, chain_end: usize, slot: usize) -> bool {
        if chain_start <= chain_end {
            chain_start <= slot && slot <= chain_end
        } else {
            // The chain wraps around the end of the array.
            slot >= chain_start || slot <= chain_end
        }
    }

    fn as_map(&self) -> HashMap<i32, i32> {
        let mut map = HashMap::new();
        for i in 0..self.keys.len() {
            if self.freqs[i] > 0 {
                map.insert(self.keys[i], self.freqs[i]);
            }
        }
        map
    }
}

/// `java.lang.Integer.highestOneBit`.
fn highest_one_bit(i: usize) -> usize {
    if i == 0 {
        0
    } else {
        1usize << (usize::BITS - 1 - i.leading_zeros())
    }
}
