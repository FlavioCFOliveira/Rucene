//! A `java.util.HashSet<E>` whose iteration order is Java's, for the ported
//! tests whose random draws or results follow the order a `HashSet` iterates
//! (`HashSet<BytesRef>`, `HashSet<Integer>`, `HashSet<String>`,
//! `HashSet<IntsRef>`).
//!
//! It reproduces what decides `java.util.HashMap`'s order: the spreading
//! function `h ^ (h >>> 16)` over the element's `hashCode()`, a power-of-two
//! table lazily allocated at 16 entries (or `tableSizeFor` of the requested
//! capacity), bucket index `hash & (n - 1)`, insertion-ordered chains
//! appended at the tail, the doubling resize at a 0.75 load factor with its
//! order-preserving lo/hi split, and the resize `treeifyBin` performs when a
//! chain reaches nine entries in a table shorter than 64. Like
//! `rucene::util::java_collections::JavaHashMap`, it keeps a treeified
//! bucket as a chain.

#![allow(dead_code)]

use rucene::util::string_helper::IntsRef;
use rucene::util::BytesRef;

/// `Object.hashCode()` of the Java counterpart of a value.
pub trait JavaHashCode {
    /// The Java `hashCode()`.
    fn java_hash_code(&self) -> i32;
}

impl JavaHashCode for i32 {
    /// `Integer.hashCode()`.
    fn java_hash_code(&self) -> i32 {
        *self
    }
}

impl JavaHashCode for i64 {
    /// `Long.hashCode(value)`: `(int) (value ^ (value >>> 32))`.
    fn java_hash_code(&self) -> i32 {
        (*self ^ ((*self as u64) >> 32) as i64) as i32
    }
}

impl JavaHashCode for BytesRef {
    /// `BytesRef.hashCode()`.
    fn java_hash_code(&self) -> i32 {
        self.hash_code()
    }
}

impl JavaHashCode for IntsRef {
    /// `IntsRef.hashCode()`.
    fn java_hash_code(&self) -> i32 {
        self.hash_code()
    }
}

impl JavaHashCode for String {
    /// `String.hashCode()` over the UTF-16 code units.
    fn java_hash_code(&self) -> i32 {
        self.encode_utf16()
            .fold(0i32, |h, c| h.wrapping_mul(31).wrapping_add(c as i32))
    }
}

const DEFAULT_INITIAL_CAPACITY: usize = 16;
const MAXIMUM_CAPACITY: usize = 1 << 30;
const TREEIFY_THRESHOLD: usize = 8;
const MIN_TREEIFY_CAPACITY: usize = 64;

/// `HashMap.hash(key)`.
fn spread(h: i32) -> i32 {
    h ^ ((h as u32) >> 16) as i32
}

/// `HashMap.tableSizeFor(cap)`.
fn table_size_for(cap: usize) -> usize {
    cap.max(1).next_power_of_two().min(MAXIMUM_CAPACITY)
}

/// Port of `java.util.HashSet<E>` (backed by `java.util.HashMap<E, Object>`)
/// with Java's iteration order.
#[derive(Clone, Debug)]
pub struct JavaHashSet<T> {
    table: Vec<Vec<(i32, T)>>,
    initial_capacity: usize,
    size: usize,
    threshold: usize,
}

impl<T: JavaHashCode + PartialEq> Default for JavaHashSet<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: JavaHashCode + PartialEq> JavaHashSet<T> {
    /// `new HashSet<>()`.
    pub fn new() -> Self {
        JavaHashSet {
            table: Vec::new(),
            initial_capacity: DEFAULT_INITIAL_CAPACITY,
            size: 0,
            threshold: 0,
        }
    }

    /// `new HashSet<>(initialCapacity)`.
    pub fn with_capacity(initial_capacity: usize) -> Self {
        JavaHashSet {
            table: Vec::new(),
            initial_capacity: table_size_for(initial_capacity),
            size: 0,
            threshold: 0,
        }
    }

    /// `new HashSet<>(collection)`: `new HashMap<>(Math.max((int) (c.size() /
    /// .75f) + 1, 16))`, then `addAll(c)`.
    pub fn from_collection(c: impl IntoIterator<Item = T>) -> Self
    where
        T: Clone,
    {
        let items: Vec<T> = c.into_iter().collect();
        let cap = ((items.len() as f32 / 0.75f32) as usize + 1).max(16);
        let mut set = Self::with_capacity(cap);
        for item in items {
            set.add(item);
        }
        set
    }

    /// `size()`.
    pub fn size(&self) -> usize {
        self.size
    }

    /// `isEmpty()`.
    pub fn is_empty(&self) -> bool {
        self.size == 0
    }

    /// `contains(o)`.
    pub fn contains(&self, value: &T) -> bool {
        if self.table.is_empty() {
            return false;
        }
        let hash = spread(value.java_hash_code());
        let index = (hash as u32 as usize) & (self.table.len() - 1);
        self.table[index]
            .iter()
            .any(|(h, v)| *h == hash && v == value)
    }

    /// `add(e)`: `true` when the set did not hold `e`.
    pub fn add(&mut self, value: T) -> bool {
        if self.table.is_empty() {
            self.resize();
        }
        let hash = spread(value.java_hash_code());
        let n = self.table.len();
        let index = (hash as u32 as usize) & (n - 1);
        let bucket = &mut self.table[index];
        if bucket.iter().any(|(h, v)| *h == hash && *v == value) {
            return false;
        }
        let bin_count = bucket.len();
        bucket.push((hash, value));
        // `putVal`: `binCount >= TREEIFY_THRESHOLD - 1` counts the nodes
        // walked before the append, the first one excluded.
        if bin_count >= TREEIFY_THRESHOLD && n < MIN_TREEIFY_CAPACITY {
            self.resize();
        }
        self.size += 1;
        if self.size > self.threshold {
            self.resize();
        }
        true
    }

    /// `remove(o)`: `true` when the set held `o`.
    pub fn remove(&mut self, value: &T) -> bool {
        if self.table.is_empty() {
            return false;
        }
        let hash = spread(value.java_hash_code());
        let index = (hash as u32 as usize) & (self.table.len() - 1);
        let bucket = &mut self.table[index];
        match bucket.iter().position(|(h, v)| *h == hash && v == value) {
            Some(pos) => {
                bucket.remove(pos);
                self.size -= 1;
                true
            }
            None => false,
        }
    }

    /// `addAll(c)`.
    pub fn add_all(&mut self, c: impl IntoIterator<Item = T>) {
        for v in c {
            self.add(v);
        }
    }

    /// `removeIf(filter)` / `iterator().remove()` over the iteration order.
    pub fn retain(&mut self, mut keep: impl FnMut(&T) -> bool) {
        for bucket in &mut self.table {
            let before = bucket.len();
            bucket.retain(|(_, v)| keep(v));
            self.size -= before - bucket.len();
        }
    }

    /// `clear()`.
    pub fn clear(&mut self) {
        for bucket in &mut self.table {
            bucket.clear();
        }
        self.size = 0;
    }

    /// The elements in Java's iteration order.
    pub fn iter(&self) -> impl Iterator<Item = &T> {
        self.table.iter().flat_map(|b| b.iter().map(|(_, v)| v))
    }

    /// `toArray()` / `new ArrayList<>(set)`.
    pub fn to_vec(&self) -> Vec<T>
    where
        T: Clone,
    {
        self.iter().cloned().collect()
    }

    /// `HashMap.resize()`.
    fn resize(&mut self) {
        let old_cap = self.table.len();
        let new_cap = if old_cap == 0 {
            self.initial_capacity
        } else if old_cap >= MAXIMUM_CAPACITY {
            self.threshold = usize::MAX;
            return;
        } else {
            old_cap * 2
        };
        self.threshold = if new_cap < MAXIMUM_CAPACITY {
            (new_cap as f32 * 0.75f32) as usize
        } else {
            usize::MAX
        };
        let mut new_table: Vec<Vec<(i32, T)>> = (0..new_cap).map(|_| Vec::new()).collect();
        for (j, bucket) in std::mem::take(&mut self.table).into_iter().enumerate() {
            for (hash, v) in bucket {
                // The lo/hi split keeps each chain's relative order.
                let index = if old_cap != 0 && (hash as u32 as usize) & old_cap == 0 {
                    j
                } else if old_cap != 0 {
                    j + old_cap
                } else {
                    (hash as u32 as usize) & (new_cap - 1)
                };
                new_table[index].push((hash, v));
            }
        }
        self.table = new_table;
    }
}

impl<T: JavaHashCode + PartialEq> PartialEq for JavaHashSet<T> {
    /// `AbstractSet.equals`: same size, and every element of one is in the
    /// other.
    fn eq(&self, other: &Self) -> bool {
        self.size == other.size && self.iter().all(|v| other.contains(v))
    }
}
