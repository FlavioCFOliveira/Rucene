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

//! Emulation of `java.util.BitSet`, the reference set of
//! `BaseBitSetTestCase` and `BaseDocIdSetTestCase`.
//!
//! **Divergence.** An emulation from the JDK 25 specification of the members
//! the suites call (see [`crate::jdk`]): a growable vector of 64-bit words,
//! `size()` the number of bits of the words held. The growth policy is the
//! JDK's (`ensureCapacity`: the larger of twice the words held and the words
//! required), so `size()` answers as the JDK does for the sets the suites
//! build.

/// `java.util.BitSet`: a vector of bits that grows as needed.
#[derive(Clone, Debug, Default)]
pub struct JavaBitSet {
    words: Vec<u64>,
}

/// `wordIndex(bitIndex)`.
fn word_index(bit_index: usize) -> usize {
    bit_index >> 6
}

impl JavaBitSet {
    /// `new BitSet()`: 64 bits.
    pub fn new() -> Self {
        JavaBitSet { words: vec![0; 1] }
    }

    /// `new BitSet(nbits)`: room for `nbits` bits.
    pub fn with_capacity(nbits: usize) -> Self {
        let words = if nbits == 0 {
            0
        } else {
            word_index(nbits - 1) + 1
        };
        JavaBitSet {
            words: vec![0; words],
        }
    }

    /// `ensureCapacity(wordsRequired)`.
    fn ensure_capacity(&mut self, words_required: usize) {
        if self.words.len() < words_required {
            let request = (2 * self.words.len()).max(words_required);
            self.words.resize(request, 0);
        }
    }

    /// `size()`: the number of bits of space actually in use.
    pub fn size(&self) -> usize {
        self.words.len() * 64
    }

    /// `get(bitIndex)`.
    pub fn get(&self, bit_index: usize) -> bool {
        let w = word_index(bit_index);
        w < self.words.len() && (self.words[w] & (1u64 << (bit_index & 63))) != 0
    }

    /// `set(bitIndex)`.
    pub fn set(&mut self, bit_index: usize) {
        let w = word_index(bit_index);
        self.ensure_capacity(w + 1);
        self.words[w] |= 1u64 << (bit_index & 63);
    }

    /// `set(fromIndex, toIndex)`.
    pub fn set_range(&mut self, from: usize, to: usize) {
        for i in from..to {
            self.set(i);
        }
    }

    /// `clear(bitIndex)`.
    pub fn clear(&mut self, bit_index: usize) {
        let w = word_index(bit_index);
        if w < self.words.len() {
            self.words[w] &= !(1u64 << (bit_index & 63));
        }
    }

    /// `clear(fromIndex, toIndex)`.
    pub fn clear_range(&mut self, from: usize, to: usize) {
        for i in from..to.min(self.size()) {
            self.clear(i);
        }
    }

    /// `clear()`.
    pub fn clear_all(&mut self) {
        self.words.iter_mut().for_each(|w| *w = 0);
    }

    /// `wordsInUse`: the words up to the highest one holding a set bit.
    fn words_in_use(&self) -> usize {
        self.words
            .iter()
            .rposition(|&w| w != 0)
            .map_or(0, |i| i + 1)
    }

    /// `flip(bitIndex)`.
    pub fn flip(&mut self, bit_index: usize) {
        let w = word_index(bit_index);
        self.ensure_capacity(w + 1);
        self.words[w] ^= 1u64 << (bit_index & 63);
    }

    /// `flip(fromIndex, toIndex)`.
    pub fn flip_range(&mut self, from: usize, to: usize) {
        if from == to {
            return;
        }
        self.ensure_capacity(word_index(to - 1) + 1);
        for i in from..to {
            self.words[word_index(i)] ^= 1u64 << (i & 63);
        }
    }

    /// `and(set)`.
    pub fn and(&mut self, set: &JavaBitSet) {
        for (i, w) in self.words.iter_mut().enumerate() {
            *w &= set.words.get(i).copied().unwrap_or(0);
        }
    }

    /// `or(set)`: grows to the words `set` uses.
    pub fn or(&mut self, set: &JavaBitSet) {
        let used = set.words_in_use();
        self.ensure_capacity(used);
        for i in 0..used {
            self.words[i] |= set.words[i];
        }
    }

    /// `xor(set)`: grows to the words `set` uses.
    pub fn xor(&mut self, set: &JavaBitSet) {
        let used = set.words_in_use();
        self.ensure_capacity(used);
        for i in 0..used {
            self.words[i] ^= set.words[i];
        }
    }

    /// `andNot(set)`.
    pub fn and_not(&mut self, set: &JavaBitSet) {
        for i in 0..self.words.len().min(set.words.len()) {
            self.words[i] &= !set.words[i];
        }
    }

    /// `cardinality()`.
    pub fn cardinality(&self) -> usize {
        self.words.iter().map(|w| w.count_ones() as usize).sum()
    }

    /// `nextSetBit(fromIndex)`: `-1` when there is none.
    pub fn next_set_bit(&self, from: usize) -> i32 {
        let mut u = word_index(from);
        if u >= self.words.len() {
            return -1;
        }
        let mut word = self.words[u] & (u64::MAX << (from & 63));
        loop {
            if word != 0 {
                return (u * 64 + word.trailing_zeros() as usize) as i32;
            }
            u += 1;
            if u == self.words.len() {
                return -1;
            }
            word = self.words[u];
        }
    }

    /// `nextClearBit(fromIndex)`.
    pub fn next_clear_bit(&self, from: usize) -> i32 {
        let mut u = word_index(from);
        if u >= self.words.len() {
            return from as i32;
        }
        let mut word = !self.words[u] & (u64::MAX << (from & 63));
        loop {
            if word != 0 {
                return (u * 64 + word.trailing_zeros() as usize) as i32;
            }
            u += 1;
            if u == self.words.len() {
                return (self.words.len() * 64) as i32;
            }
            word = !self.words[u];
        }
    }

    /// `previousSetBit(fromIndex)`: `-1` when there is none (and for
    /// `fromIndex == -1`).
    pub fn previous_set_bit(&self, from: i32) -> i32 {
        if from < 0 {
            return -1;
        }
        let from = from as usize;
        let mut u = word_index(from);
        if u >= self.words.len() {
            return self.last_set_bit();
        }
        let mut word = self.words[u] & (u64::MAX >> (63 - (from & 63)));
        loop {
            if word != 0 {
                return ((u + 1) * 64 - 1 - word.leading_zeros() as usize) as i32;
            }
            if u == 0 {
                return -1;
            }
            u -= 1;
            word = self.words[u];
        }
    }

    /// `length() - 1`: the highest set bit, `-1` when none is set.
    fn last_set_bit(&self) -> i32 {
        for u in (0..self.words.len()).rev() {
            if self.words[u] != 0 {
                return ((u + 1) * 64 - 1 - self.words[u].leading_zeros() as usize) as i32;
            }
        }
        -1
    }
}

/// The words up to the last non-zero one (`wordsInUse`).
fn words_in_use(words: &[u64]) -> &[u64] {
    let used = words.iter().rposition(|&w| w != 0).map_or(0, |i| i + 1);
    &words[..used]
}

impl PartialEq for JavaBitSet {
    /// `equals(Object)`: the same bits are set; the capacity (`size()`) does
    /// not take part, as Java compares the `wordsInUse` words only.
    fn eq(&self, other: &Self) -> bool {
        words_in_use(&self.words) == words_in_use(&other.words)
    }
}

impl Eq for JavaBitSet {}

impl std::fmt::Display for JavaBitSet {
    /// `toString()`: the indices of the set bits in ascending order, as
    /// `{i, j, k}`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("{")?;
        let mut first = true;
        for (u, &word) in self.words.iter().enumerate() {
            let mut w = word;
            while w != 0 {
                let bit = u * 64 + w.trailing_zeros() as usize;
                if !first {
                    f.write_str(", ")?;
                }
                write!(f, "{bit}")?;
                first = false;
                w &= w - 1;
            }
        }
        f.write_str("}")
    }
}
