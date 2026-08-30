//! Small utilities ported from `org.apache.lucene.util`.
//!
//! | Rucene | Apache Lucene Core 10.5.0 |
//! | --- | --- |
//! | [`MapOfSets`] | `MapOfSets<K, V>` |
//! | [`ToStringUtils`] | `ToStringUtils` |
//! | [`StrictStringTokenizer`] | `StrictStringTokenizer` |
//! | [`SentinelIntSet`] | `SentinelIntSet` |
//! | [`TermAndVector`] | `TermAndVector` |

#![deny(unsafe_code)]

use std::collections::{HashMap, HashSet};
use std::fmt::{self, Display, Formatter, Write as _};
use std::hash::Hash;

use crate::error::{LuceneError, Result};
use crate::util::vector_util::l2normalize;
use crate::util::{BitUtil, BytesRef, BytesRefBuilder, RamUsageEstimator};

// ---------------------------------------------------------------------------
// MapOfSets
// ---------------------------------------------------------------------------

/// Keeps sets of values associated with keys.
///
/// Port of `org.apache.lucene.util.MapOfSets`. **Not thread safe**, exactly as
/// Lucene warns.
#[derive(Debug, Default, Clone)]
pub struct MapOfSets<K, V> {
    the_map: HashMap<K, HashSet<V>>,
}

impl<K: Eq + Hash, V: Eq + Hash> MapOfSets<K, V> {
    /// Creates an instance backed by `m`.
    ///
    /// Equivalent to `new MapOfSets(Map)`.
    pub fn new(m: HashMap<K, HashSet<V>>) -> Self {
        Self { the_map: m }
    }

    /// Returns direct access to the backing map.
    pub fn get_map(&self) -> &HashMap<K, HashSet<V>> {
        &self.the_map
    }

    /// Returns mutable direct access to the backing map.
    pub fn get_map_mut(&mut self) -> &mut HashMap<K, HashSet<V>> {
        &mut self.the_map
    }

    /// Adds `val` to the set associated with `key`, creating the set when
    /// needed, and returns the size of that set afterwards.
    pub fn put(&mut self, key: K, val: V) -> usize {
        // Java sizes the new set at 23; `HashSet::with_capacity` is the
        // equivalent hint.
        let the_set = self
            .the_map
            .entry(key)
            .or_insert_with(|| HashSet::with_capacity(23));
        the_set.insert(val);
        the_set.len()
    }

    /// Adds every value of `vals` to the set associated with `key`, creating the
    /// set when needed, and returns the size of that set afterwards.
    pub fn put_all<I: IntoIterator<Item = V>>(&mut self, key: K, vals: I) -> usize {
        let the_set = self
            .the_map
            .entry(key)
            .or_insert_with(|| HashSet::with_capacity(23));
        the_set.extend(vals);
        the_set.len()
    }
}

// ---------------------------------------------------------------------------
// ToStringUtils
// ---------------------------------------------------------------------------

/// Helpers that ease implementing `Display`/`Debug`.
///
/// Port of `org.apache.lucene.util.ToStringUtils`.
pub struct ToStringUtils;

impl ToStringUtils {
    /// Appends `b[i]=<signed byte>` for every byte, comma separated.
    ///
    /// Equivalent to `ToStringUtils.byteArray(StringBuilder, byte[])`. Java
    /// renders a `byte`, which is signed.
    pub fn byte_array(buffer: &mut String, bytes: &[u8]) {
        for (i, b) in bytes.iter().enumerate() {
            let _ = write!(buffer, "b[{}]={}", i, *b as i8);
            if i < bytes.len() - 1 {
                buffer.push(',');
            }
        }
    }

    /// Returns `x` in hex with a `0x` prefix and all leading zeroes.
    ///
    /// Unlike `Long.toHexString`, the result is always 16 hex digits wide.
    pub fn long_hex(x: i64) -> String {
        format!("0x{:016x}", x as u64)
    }

    /// Renders a [`BytesRef`] as its UTF-8 text followed by its hex bytes, for
    /// example `hello [68 65 6c 6c 6f]`.
    ///
    /// When the content is not valid UTF-8, only the hex form is returned, as
    /// `BytesRef::to_string` produces.
    pub fn bytes_ref_to_string(b: Option<&BytesRef>) -> String {
        match b {
            None => "null".to_string(),
            Some(b) => match b.utf8_to_string() {
                Ok(text) => format!("{text} {b}"),
                Err(_) => b.to_string(),
            },
        }
    }

    /// Renders the content of a [`BytesRefBuilder`].
    pub fn bytes_ref_builder_to_string(b: &BytesRefBuilder) -> String {
        Self::bytes_ref_to_string(Some(&b.get()))
    }

    /// Renders a byte slice the way [`Self::bytes_ref_to_string`] does.
    pub fn bytes_to_string(b: &[u8]) -> String {
        Self::bytes_ref_to_string(Some(&BytesRef::new(b.to_vec())))
    }
}

// ---------------------------------------------------------------------------
// StrictStringTokenizer
// ---------------------------------------------------------------------------

/// Splits a string on a delimiter without silently skipping empty tokens.
///
/// Port of `org.apache.lucene.util.StrictStringTokenizer`, used for parsing
/// version strings. It is package-private in Java and public here because Rust
/// has no package visibility between sibling modules.
#[derive(Debug, Clone)]
pub struct StrictStringTokenizer {
    s: Vec<char>,
    delimiter: char,
    /// Position of the next token, or `None` once the input is exhausted
    /// (Java's `pos < 0`).
    pos: Option<usize>,
}

impl StrictStringTokenizer {
    /// Creates a tokenizer over `s` splitting on `delimiter`.
    pub fn new(s: &str, delimiter: char) -> Self {
        Self {
            s: s.chars().collect(),
            delimiter,
            pos: Some(0),
        }
    }

    /// Returns the next token.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] once the input is exhausted, which
    /// is Java's `IllegalStateException("no more tokens")`.
    pub fn next_token(&mut self) -> Result<String> {
        let Some(pos) = self.pos else {
            return Err(LuceneError::IllegalState("no more tokens".to_string()));
        };

        match self.s[pos..].iter().position(|&c| c == self.delimiter) {
            Some(rel) => {
                let pos1 = pos + rel;
                let s1: String = self.s[pos..pos1].iter().collect();
                self.pos = Some(pos1 + 1);
                Ok(s1)
            }
            None => {
                let s1: String = self.s[pos..].iter().collect();
                self.pos = None;
                Ok(s1)
            }
        }
    }

    /// Returns whether another token is available.
    pub fn has_more_tokens(&self) -> bool {
        self.pos.is_some()
    }
}

// ---------------------------------------------------------------------------
// SentinelIntSet
// ---------------------------------------------------------------------------

/// A native `i32` hash set that reserves one value to mean "empty".
///
/// Port of `org.apache.lucene.util.SentinelIntSet`. The internal fields are
/// public in Java "to enable more efficient use at the expense of better O-O
/// principles"; that is reproduced here.
///
/// To iterate the values held in the set, skip every slot equal to
/// [`SentinelIntSet::empty_val`].
#[derive(Debug, Clone)]
pub struct SentinelIntSet {
    /// A power-of-two over-sized array holding the values and the empty slots.
    pub keys: Vec<i32>,
    /// Number of values in the set.
    pub count: usize,
    /// The value used for EMPTY.
    pub empty_val: i32,
    /// The count at which a rehash is performed.
    pub rehash_count: usize,
}

impl SentinelIntSet {
    /// Creates a set able to hold at least `size` elements without rehashing.
    ///
    /// `empty_val` is the integer used for EMPTY and must never be inserted.
    pub fn new(size: usize, empty_val: i32) -> Self {
        let mut tsize = (BitUtil::next_highest_power_of_two(size as i32).max(1)) as usize;
        let mut rehash_count = tsize - (tsize >> 2);
        if size >= rehash_count {
            // Must be able to hold `size` without rehashing.
            tsize <<= 1;
            rehash_count = tsize - (tsize >> 2);
        }
        let keys = vec![if empty_val != 0 { empty_val } else { 0 }; tsize];
        Self {
            keys,
            count: 0,
            empty_val,
            rehash_count,
        }
    }

    /// Empties the set.
    pub fn clear(&mut self) {
        let empty_val = self.empty_val;
        self.keys.iter_mut().for_each(|k| *k = empty_val);
        self.count = 0;
    }

    /// Returns the hash for `key`.
    ///
    /// The default returns the key itself, which is not appropriate for general
    /// purpose use but is fine for Lucene doc ids. Java expects subclasses to
    /// override it; this port exposes it as an overridable strategy through
    /// [`SentinelIntSetHash`].
    pub fn hash(&self, key: i32) -> i32 {
        key
    }

    /// The number of integers in this set.
    pub fn size(&self) -> usize {
        self.count
    }

    /// Returns whether the set holds no value.
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Returns the slot for `key`, whether or not it is present.
    pub fn get_slot(&self, key: i32) -> usize {
        debug_assert_ne!(key, self.empty_val);
        let h = self.hash(key);
        let mask = self.keys.len() - 1;
        let mut s = (h as usize) & mask;
        if self.keys[s] == key || self.keys[s] == self.empty_val {
            return s;
        }

        let increment = ((h >> 7) | 1) as usize;
        loop {
            s = (s + increment) & mask;
            if self.keys[s] == key || self.keys[s] == self.empty_val {
                return s;
            }
        }
    }

    /// Returns the slot for `key`, or `-slot - 1` when it is not present.
    pub fn find(&self, key: i32) -> isize {
        debug_assert_ne!(key, self.empty_val);
        let h = self.hash(key);
        let mask = self.keys.len() - 1;
        let mut s = (h as usize) & mask;
        if self.keys[s] == key {
            return s as isize;
        }
        if self.keys[s] == self.empty_val {
            return -(s as isize) - 1;
        }

        let increment = ((h >> 7) | 1) as usize;
        loop {
            s = (s + increment) & mask;
            if self.keys[s] == key {
                return s as isize;
            }
            if self.keys[s] == self.empty_val {
                return -(s as isize) - 1;
            }
        }
    }

    /// Returns whether the set contains `key`.
    pub fn exists(&self, key: i32) -> bool {
        self.find(key) >= 0
    }

    /// Inserts `key` and returns the slot it landed in, rehashing when the set
    /// would otherwise become more than 75% full.
    pub fn put(&mut self, key: i32) -> usize {
        let s = self.find(key);
        if s < 0 {
            self.count += 1;
            let slot = if self.count >= self.rehash_count {
                self.rehash();
                self.get_slot(key)
            } else {
                (-s - 1) as usize
            };
            self.keys[slot] = key;
            return slot;
        }
        s as usize
    }

    /// Doubles the key array and refills it with the old values.
    pub fn rehash(&mut self) {
        let new_size = self.keys.len() << 1;
        let old_keys = std::mem::replace(&mut self.keys, vec![0; new_size]);
        if self.empty_val != 0 {
            let empty_val = self.empty_val;
            self.keys.iter_mut().for_each(|k| *k = empty_val);
        }

        for key in old_keys {
            if key == self.empty_val {
                continue;
            }
            let new_slot = self.get_slot(key);
            self.keys[new_slot] = key;
        }
        self.rehash_count = new_size - (new_size >> 2);
    }

    /// Returns the memory footprint of this set in bytes.
    pub fn ram_bytes_used(&self) -> i64 {
        RamUsageEstimator::align_object_size(4 * 3 + RamUsageEstimator::NUM_BYTES_OBJECT_REF)
            + RamUsageEstimator::size_of_int(&self.keys)
    }
}

/// The hashing strategy of a [`SentinelIntSet`].
///
/// Java says "consider extending and over-riding `hash(int)` if the values
/// might be poor hash keys". Rust has no inheritance, so a caller that needs a
/// different hash implements this trait and drives the set through
/// [`SentinelIntSetHash::hash`] — the identity default matches Lucene's.
pub trait SentinelIntSetHash {
    /// Returns the hash for `key`.
    fn hash(&self, key: i32) -> i32 {
        key
    }
}

impl SentinelIntSetHash for SentinelIntSet {}

// ---------------------------------------------------------------------------
// TermAndVector
// ---------------------------------------------------------------------------

/// A word2vec unit: a term together with its vector.
///
/// Port of the record `org.apache.lucene.util.TermAndVector`, which Lucene
/// 10.5.0 marks `@lucene.experimental`.
#[derive(Debug, Clone, PartialEq)]
pub struct TermAndVector {
    term: BytesRef,
    vector: Vec<f32>,
}

impl TermAndVector {
    /// Creates a term/vector pair.
    pub fn new(term: BytesRef, vector: Vec<f32>) -> Self {
        Self { term, vector }
    }

    /// Returns the term.
    pub fn term(&self) -> &BytesRef {
        &self.term
    }

    /// Returns the vector.
    pub fn vector(&self) -> &[f32] {
        &self.vector
    }

    /// Returns the number of dimensions.
    pub fn size(&self) -> usize {
        self.vector.len()
    }

    /// Returns a copy whose vector is normalised by its L2 norm.
    ///
    /// # Errors
    ///
    /// Propagates the error [`l2normalize`] raises for a zero vector. Java's
    /// `VectorUtil.l2normalize(float[])` throws `IllegalArgumentException` in
    /// the same case.
    pub fn normalize_vector(&self) -> Result<Self> {
        let mut vector = self.vector.clone();
        l2normalize(&mut vector, true)?;
        Ok(Self {
            term: self.term.clone(),
            vector,
        })
    }
}

impl Display for TermAndVector {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let term = self
            .term
            .utf8_to_string()
            .unwrap_or_else(|_| self.term.to_string());
        write!(f, "{term} [")?;
        if !self.vector.is_empty() {
            for v in &self.vector[..self.vector.len() - 1] {
                write!(f, "{v:.3},")?;
            }
            write!(f, "{:.3}]", self.vector[self.vector.len() - 1])?;
        }
        Ok(())
    }
}
