//! Iteration-order-faithful ports of the `java.util` collections whose order
//! Lucene 10.5.0 writes into an index file.
//!
//! | Rucene | Apache Lucene Core 10.5.0 |
//! | --- | --- |
//! | [`JavaHashMap`] | the `java.util.HashMap` behind `FieldInfo.attributes`, `SegmentInfo.attributes`, `SegmentInfos.userData` and `ReadersAndUpdates.pendingDVUpdates` |
//! | [`JavaHashSet`] | the `java.util.HashSet` behind `SegmentInfo.setFiles` |
//!
//! # Why the order is part of the file
//!
//! `DataOutput.writeMapOfStrings` and `DataOutput.writeSetOfStrings` write a
//! count and then walk the collection in *iteration order*
//! (`DataOutput.java:304-310` and `:321-326`). Nothing in the format fixes that
//! order, so the bytes a writer produces are whatever its map happened to
//! iterate. Reproducing Lucene's files byte for byte therefore means
//! reproducing the iteration order of the exact `java.util` implementation
//! Lucene holds at each of those call sites — see the module-level notes on
//! [`crate::index::segment_info::SegmentInfo`] for the sites where that
//! implementation is *not* a `HashMap` and the order is consequently not
//! reproducible at all.
//!
//! [`std::collections::HashMap`] cannot stand in: its iteration order comes
//! from a per-process random seed, so it is not even stable between two runs of
//! Rucene itself.
//!
//! # What is reproduced
//!
//! Everything that decides `java.util.HashMap`'s iteration order:
//!
//! * the spreading function `h ^ (h >>> 16)` over `String.hashCode()`
//!   (`HashMap.hash`), ported as [`spread`];
//! * a table whose length is always a power of two, with the bucket index
//!   `hash & (n - 1)`;
//! * insertion-ordered chains within a bucket, appended at the tail;
//! * the capacity policy — a lazily allocated table of 16 for
//!   `new HashMap<>()`, `tableSizeFor` for `new HashMap<>(int)`, and the
//!   pre-sizing `new HashMap<>(Map)` performs — because the table length is
//!   what the bucket index is taken modulo, so two maps holding the same
//!   entries iterate differently when their capacity histories differ;
//! * the doubling resize, including its threshold arithmetic and its
//!   order-preserving split of each bucket;
//! * the extra resize `treeifyBin` performs when a bucket grows past
//!   `TREEIFY_THRESHOLD` while the table is still shorter than
//!   `MIN_TREEIFY_CAPACITY`.
//!
//! Every one of those is checked against a running JVM by
//! `tests/portability/java_collections.rs`, which drives
//! `JavaCollectionsFixture` over randomised key sets and compares the orders
//! entry by entry.
//!
//! # What is not reproduced
//!
//! When a bucket reaches `TREEIFY_THRESHOLD + 1` entries *and* the table is at
//! least `MIN_TREEIFY_CAPACITY` long, `java.util.HashMap` converts that bucket
//! into a red-black tree and `moveRootToFront` re-heads the bucket's chain at
//! the tree root, so the chain is no longer in insertion order. This port keeps
//! the chain.
//!
//! Reaching that state needs keys whose `String.hashCode` collides: every key
//! past the eighth in one bucket makes `treeifyBin` double the table, so the
//! table reaches 64 long before any load factor would take it there. Measured
//! against a JVM by
//! `java_collections::colliding_buckets_agree_with_the_jvm_up_to_the_treeify_boundary`,
//! the orders agree for up to **ten** keys sharing a bucket and diverge at the
//! eleventh — not at the 49 entries a load-factor resize to 64 would need. The
//! keys Lucene's own code puts in these maps are fixed strings that do not
//! collide, so no `.fnm`, `.si` or `segments_N` Lucene writes can reach it;
//! eleven colliding keys in a caller-supplied
//! `IndexWriter.setLiveCommitData` can. The condition is observable through
//! [`JavaHashMap::is_treeified`], and the divergence past it is measured by
//! `java_collections::treeified_bucket_is_the_one_shape_the_port_does_not_order_like_java`
//! rather than assumed away.

#![deny(unsafe_code)]

use std::cmp::Ordering;
use std::collections::HashMap;
use std::fmt;

/// `HashMap.DEFAULT_INITIAL_CAPACITY`, the table length a lazily allocated
/// table gets.
pub const DEFAULT_INITIAL_CAPACITY: usize = 1 << 4;

/// `HashMap.MAXIMUM_CAPACITY`.
pub const MAXIMUM_CAPACITY: usize = 1 << 30;

/// `HashMap.DEFAULT_LOAD_FACTOR`.
pub const DEFAULT_LOAD_FACTOR: f32 = 0.75;

/// `HashMap.TREEIFY_THRESHOLD`.
pub const TREEIFY_THRESHOLD: usize = 8;

/// `HashMap.MIN_TREEIFY_CAPACITY`.
pub const MIN_TREEIFY_CAPACITY: usize = 64;

/// `java.lang.String.hashCode()`.
///
/// The sum `s[0]*31^(n-1) + s[1]*31^(n-2) + ... + s[n-1]` over the string's
/// UTF-16 code units, in wrapping 32-bit arithmetic. Iterating Rust `char`s
/// instead would disagree on every string outside the basic multilingual plane,
/// where one `char` is two code units.
pub fn java_string_hash_code(s: &str) -> i32 {
    let mut h: i32 = 0;
    for unit in s.encode_utf16() {
        h = h.wrapping_mul(31).wrapping_add(i32::from(unit));
    }
    h
}

/// `HashMap.hash(Object)`: `(h = key.hashCode()) ^ (h >>> 16)`.
///
/// The XOR folds the high bits down, so they still influence the bucket index
/// after the mask `hash & (n - 1)` has thrown them away.
pub fn spread(key: &str) -> i32 {
    let h = java_string_hash_code(key);
    h ^ ((h as u32) >> 16) as i32
}

/// `HashMap.tableSizeFor(int)`: the smallest power of two `>= cap`.
///
/// Java computes it as `-1 >>> Integer.numberOfLeadingZeros(cap - 1)`, whose
/// shift distance is masked to five bits, so `cap` of 0 or 1 both leave the
/// intermediate at `-1` and return 1.
pub fn table_size_for(cap: i32) -> i32 {
    let leading = (cap.wrapping_sub(1) as u32).leading_zeros() & 31;
    let n = (u32::MAX >> leading) as i32;
    if n < 0 {
        1
    } else if n as usize >= MAXIMUM_CAPACITY {
        MAXIMUM_CAPACITY as i32
    } else {
        n + 1
    }
}

/// The bucket index `(n - 1) & hash`.
fn index_for(hash: i32, n: usize) -> usize {
    (hash as u32 as usize) & (n - 1)
}

/// One `HashMap.Node`.
#[derive(Clone, Debug)]
struct Entry<V> {
    hash: i32,
    key: String,
    value: V,
}

/// A `java.util.HashMap<String, V>` with Java's iteration order.
///
/// Equivalent to `java.util.HashMap` for the operations Lucene performs on the
/// maps whose order reaches an index file: the `Map<String, String>` maps it
/// serialises, and `ReadersAndUpdates.pendingDVUpdates`, whose field order is
/// the order the doc-values update generations are written in. The type is
/// deliberately `String`-keyed: every such map is keyed by `String`, and a
/// generic key would need a port of `Object.hashCode` per key type that nothing
/// would call.
///
/// The load factor is fixed at [`DEFAULT_LOAD_FACTOR`]; Lucene never constructs
/// one of these maps with any other.
#[derive(Clone)]
pub struct JavaHashMap<V> {
    /// `HashMap.table`. Empty stands for Java's `null` table, which is what a
    /// freshly constructed map has until its first insertion.
    table: Vec<Vec<Entry<V>>>,
    /// `HashMap.size`.
    size: usize,
    /// `HashMap.threshold`. Before the table exists this holds the *initial
    /// capacity* rather than a resize point, exactly as Java overloads it.
    threshold: i32,
    /// Set once a bucket has reached the shape Java would have converted into a
    /// red-black tree; see the module docs.
    treeified: bool,
}

impl<V> Default for JavaHashMap<V> {
    fn default() -> Self {
        Self::new()
    }
}

impl<V> JavaHashMap<V> {
    /// `new HashMap<>()`: no table, no threshold. The table is allocated with
    /// [`DEFAULT_INITIAL_CAPACITY`] entries on the first insertion.
    pub fn new() -> Self {
        JavaHashMap {
            table: Vec::new(),
            size: 0,
            threshold: 0,
            treeified: false,
        }
    }

    /// `new HashMap<>(int initialCapacity)`, which sets
    /// `threshold = tableSizeFor(initialCapacity)` and allocates a table of
    /// exactly that length on the first insertion.
    pub fn with_initial_capacity(initial_capacity: i32) -> Self {
        assert!(
            initial_capacity >= 0,
            "illegal initial capacity: {initial_capacity}"
        );
        JavaHashMap {
            table: Vec::new(),
            size: 0,
            threshold: table_size_for(initial_capacity),
            treeified: false,
        }
    }

    /// `HashMap.size()`.
    pub fn len(&self) -> usize {
        self.size
    }

    /// `HashMap.isEmpty()`.
    pub fn is_empty(&self) -> bool {
        self.size == 0
    }

    /// The current table length, or 0 while the table is still unallocated.
    ///
    /// Exposed because it is the modulus the bucket index is taken against, and
    /// so the one piece of internal state the JVM differential test has to be
    /// able to compare.
    pub fn table_length(&self) -> usize {
        self.table.len()
    }

    /// Whether any bucket has reached the shape `java.util.HashMap` would have
    /// turned into a red-black tree, past which this port's iteration order is
    /// no longer Java's. See the module docs.
    pub fn is_treeified(&self) -> bool {
        self.treeified
    }

    /// `HashMap.get(Object)`.
    pub fn get(&self, key: &str) -> Option<&V> {
        if self.table.is_empty() {
            return None;
        }
        let hash = spread(key);
        let bucket = &self.table[index_for(hash, self.table.len())];
        bucket
            .iter()
            .find(|e| e.hash == hash && e.key == key)
            .map(|e| &e.value)
    }

    /// `HashMap.containsKey(Object)`.
    pub fn contains_key(&self, key: &str) -> bool {
        self.get(key).is_some()
    }

    /// `HashMap.put(K, V)`, returning the previous value as Java does.
    pub fn insert(&mut self, key: String, value: V) -> Option<V> {
        if self.table.is_empty() {
            self.resize();
        }
        let hash = spread(&key);
        let n = self.table.len();
        let index = index_for(hash, n);
        let bucket = &mut self.table[index];
        for entry in bucket.iter_mut() {
            if entry.hash == hash && entry.key == key {
                return Some(std::mem::replace(&mut entry.value, value));
            }
        }
        bucket.push(Entry { hash, key, value });
        // `putVal` counts the links it walked before appending, so a bucket that
        // already held `TREEIFY_THRESHOLD` nodes triggers `treeifyBin` — which
        // resizes instead of building a tree while the table is short.
        if bucket.len() > TREEIFY_THRESHOLD {
            if n < MIN_TREEIFY_CAPACITY {
                self.resize();
            } else {
                self.treeified = true;
            }
        }
        self.size += 1;
        if self.size as i32 > self.threshold {
            self.resize();
        }
        None
    }

    /// `HashMap.get(Object)`, returning the value for in-place change.
    pub fn get_mut(&mut self, key: &str) -> Option<&mut V> {
        if self.table.is_empty() {
            return None;
        }
        let hash = spread(key);
        let n = self.table.len();
        self.table[index_for(hash, n)]
            .iter_mut()
            .find(|e| e.hash == hash && e.key == key)
            .map(|e| &mut e.value)
    }

    /// `HashMap.computeIfAbsent(K, Function)` with a mapping function that
    /// never returns `null`: returns the value mapped to `key`, inserting
    /// `make()` first when there is none.
    ///
    /// Its insertion is not `put`'s, and the difference is visible in the
    /// iteration order, so it is ported separately from JDK 21's
    /// `HashMap.computeIfAbsent`:
    ///
    /// * the table is resized *before* the lookup when `size > threshold` (or
    ///   when it is still unallocated), never after the insertion — so the
    ///   entry that crosses the threshold does not trigger the resize itself;
    /// * the new node becomes the head of its bucket
    ///   (`tab[i] = newNode(hash, key, v, first)`), not its tail;
    /// * `treeifyBin` runs when the bucket already held
    ///   `TREEIFY_THRESHOLD - 1` nodes, one node earlier than in `putVal`.
    pub fn compute_if_absent(&mut self, key: &str, make: impl FnOnce() -> V) -> &mut V {
        if self.size as i32 > self.threshold || self.table.is_empty() {
            self.resize();
        }
        let hash = spread(key);
        let n = self.table.len();
        let index = index_for(hash, n);
        if let Some(position) = self.table[index]
            .iter()
            .position(|e| e.hash == hash && e.key == key)
        {
            return &mut self.table[index][position].value;
        }
        let bin_count = self.table[index].len();
        self.table[index].insert(
            0,
            Entry {
                hash,
                key: key.to_string(),
                value: make(),
            },
        );
        if bin_count >= TREEIFY_THRESHOLD - 1 {
            if n < MIN_TREEIFY_CAPACITY {
                self.resize();
            } else {
                self.treeified = true;
            }
        }
        self.size += 1;
        let index = index_for(hash, self.table.len());
        let position = self.table[index]
            .iter()
            .position(|e| e.hash == hash && e.key == key)
            .expect("INVARIANT: the entry was just inserted");
        &mut self.table[index][position].value
    }

    /// `HashMap.remove(Object)`.
    pub fn remove(&mut self, key: &str) -> Option<V> {
        if self.table.is_empty() {
            return None;
        }
        let hash = spread(key);
        let n = self.table.len();
        let bucket = &mut self.table[index_for(hash, n)];
        let position = bucket.iter().position(|e| e.hash == hash && e.key == key)?;
        let removed = bucket.remove(position);
        self.size -= 1;
        Some(removed.value)
    }

    /// `HashMap.clear()`, which keeps the table it has and empties its buckets.
    pub fn clear(&mut self) {
        for bucket in self.table.iter_mut() {
            bucket.clear();
        }
        self.size = 0;
        self.treeified = false;
    }

    /// Iterates the entries in `HashMap.entrySet()` order: buckets by ascending
    /// index, and within a bucket the chain from head to tail.
    pub fn iter(&self) -> impl Iterator<Item = (&String, &V)> {
        self.table
            .iter()
            .flat_map(|bucket| bucket.iter().map(|e| (&e.key, &e.value)))
    }

    /// Iterates the keys in `HashMap.keySet()` order.
    pub fn keys(&self) -> impl Iterator<Item = &String> {
        self.iter().map(|(k, _)| k)
    }

    /// Iterates the values in `HashMap.values()` order.
    pub fn values(&self) -> impl Iterator<Item = &V> {
        self.iter().map(|(_, v)| v)
    }

    /// Iterates the entries in `HashMap.entrySet()` order, with each value
    /// open to in-place change (`Map.Entry.setValue`).
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (&String, &mut V)> {
        self.table
            .iter_mut()
            .flat_map(|bucket| bucket.iter_mut().map(|e| (&e.key, &mut e.value)))
    }

    /// Iterates the values in `HashMap.values()` order, each open to in-place
    /// change.
    pub fn values_mut(&mut self) -> impl Iterator<Item = &mut V> {
        self.iter_mut().map(|(_, v)| v)
    }

    /// `HashMap.resize()`.
    ///
    /// The new capacity is the old one doubled, or the threshold when the table
    /// was still unallocated and a capacity had been requested, or
    /// [`DEFAULT_INITIAL_CAPACITY`] otherwise.
    ///
    /// Rehashing preserves each bucket's chain order. Java splits a bucket into
    /// a low and a high list and keeps the relative order of both; since every
    /// entry of old bucket `j` lands in new bucket `j` or `j + oldCap`, and no
    /// two old buckets can feed one new bucket, walking the old table in order
    /// and appending reproduces exactly those chains. The order in which the
    /// *old* buckets are visited is for the same reason unobservable — a
    /// negative control that reverses it does not change any iteration order.
    fn resize(&mut self) {
        let old_cap = self.table.len();
        let old_thr = self.threshold;
        let new_cap: usize;
        let mut new_thr: i32 = 0;

        if old_cap > 0 {
            if old_cap >= MAXIMUM_CAPACITY {
                self.threshold = i32::MAX;
                return;
            }
            new_cap = old_cap << 1;
            if new_cap < MAXIMUM_CAPACITY && old_cap >= DEFAULT_INITIAL_CAPACITY {
                new_thr = old_thr << 1;
            }
        } else if old_thr > 0 {
            new_cap = old_thr as usize;
        } else {
            new_cap = DEFAULT_INITIAL_CAPACITY;
            new_thr = (DEFAULT_LOAD_FACTOR * DEFAULT_INITIAL_CAPACITY as f32) as i32;
        }

        if new_thr == 0 {
            let ft = new_cap as f32 * DEFAULT_LOAD_FACTOR;
            new_thr = if new_cap < MAXIMUM_CAPACITY && ft < MAXIMUM_CAPACITY as f32 {
                ft as i32
            } else {
                i32::MAX
            };
        }
        self.threshold = new_thr;

        let old_table = std::mem::take(&mut self.table);
        let mut new_table: Vec<Vec<Entry<V>>> = Vec::with_capacity(new_cap);
        new_table.resize_with(new_cap, Vec::new);
        for bucket in old_table {
            for entry in bucket {
                let index = index_for(entry.hash, new_cap);
                new_table[index].push(entry);
            }
        }
        self.table = new_table;
    }

    /// `HashMap.putMapEntries(Map, boolean)`, the body shared by the copy
    /// constructor and `putAll`.
    ///
    /// `size` is the source map's size, which Java reads once up front and uses
    /// to pre-size the table before inserting anything. Doing that changes the
    /// resulting capacity, and therefore the iteration order, relative to the
    /// same entries inserted one by one.
    fn put_map_entries<I>(&mut self, entries: I, size: usize)
    where
        I: IntoIterator<Item = (String, V)>,
    {
        if size == 0 {
            return;
        }
        if self.table.is_empty() {
            // `Math.ceil(s / (double) loadFactor)`. JDK 18 and earlier computed
            // `(int)((float) s / loadFactor + 1.0f)`, which sizes a 12-entry
            // copy to 32 instead of 16 and so iterates in a different order;
            // the JVM differential test discriminates between the two at sizes
            // 3, 12 and 24.
            let dt = (size as f64 / DEFAULT_LOAD_FACTOR as f64).ceil();
            let t = if dt < MAXIMUM_CAPACITY as f64 {
                dt as i32
            } else {
                MAXIMUM_CAPACITY as i32
            };
            if t > self.threshold {
                self.threshold = table_size_for(t);
            }
        } else {
            while size as i32 > self.threshold && self.table.len() < MAXIMUM_CAPACITY {
                self.resize();
            }
        }
        for (key, value) in entries {
            self.insert(key, value);
        }
    }

    /// `HashMap.putAll(Map)`.
    pub fn put_all<I>(&mut self, entries: I)
    where
        I: IntoIterator<Item = (String, V)>,
        I::IntoIter: ExactSizeIterator,
    {
        let iter = entries.into_iter();
        let size = iter.len();
        self.put_map_entries(iter, size);
    }
}

impl<V: Clone> JavaHashMap<V> {
    /// `new HashMap<>(Map)`.
    ///
    /// Pre-sizes the table from the source's size and then inserts the source's
    /// entries **in the source's iteration order**, which is what makes the
    /// result reproducible: the copy of a copy is still determined.
    pub fn copy_of(source: &JavaHashMap<V>) -> Self {
        Self::copy_of_entries(source.iter().map(|(k, v)| (k.clone(), v.clone())))
    }

    /// `new HashMap<>(Map)` over any map whose iteration order is already
    /// known, which is how a `HashMap` is copied out of one of the other
    /// `java.util.Map` implementations Lucene holds — see [`JavaStringMap`].
    pub fn copy_of_entries<I>(entries: I) -> Self
    where
        I: IntoIterator<Item = (String, V)>,
    {
        let entries: Vec<(String, V)> = entries.into_iter().collect();
        let size = entries.len();
        let mut copy = JavaHashMap::new();
        copy.put_map_entries(entries, size);
        copy
    }

    /// The entries as a [`std::collections::HashMap`], for the callers that
    /// only look things up and never write the map out.
    pub fn to_std(&self) -> HashMap<String, V> {
        self.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
    }
}

/// `new HashMap<>()` followed by one `put` per item, in the iterator's order.
///
/// This is the shape of every map Lucene builds literally — the fixtures'
/// `new HashMap<>(); map.put(...)` and the codec attribute maps alike. The
/// pre-sized copy constructor is [`JavaHashMap::copy_of`]; the two do **not**
/// produce the same iteration order, because they end up with different table
/// lengths.
impl<V> FromIterator<(String, V)> for JavaHashMap<V> {
    fn from_iter<I: IntoIterator<Item = (String, V)>>(iter: I) -> Self {
        let mut map = JavaHashMap::new();
        for (key, value) in iter {
            map.insert(key, value);
        }
        map
    }
}

impl<V> Extend<(String, V)> for JavaHashMap<V> {
    fn extend<I: IntoIterator<Item = (String, V)>>(&mut self, iter: I) {
        for (key, value) in iter {
            self.insert(key, value);
        }
    }
}

/// `AbstractMap.equals`, which compares contents and ignores iteration order.
impl<V: PartialEq> PartialEq for JavaHashMap<V> {
    fn eq(&self, other: &Self) -> bool {
        self.size == other.size
            && self
                .iter()
                .all(|(k, v)| other.get(k).is_some_and(|other_v| other_v == v))
    }
}

impl<V: Eq> Eq for JavaHashMap<V> {}

/// Formats like [`std::collections::HashMap`], but in Java's iteration order.
impl<V: fmt::Debug> fmt::Debug for JavaHashMap<V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_map().entries(self.iter()).finish()
    }
}

impl<V> JavaHashMap<V> {
    /// Consumes the map and yields its keys in iteration order.
    pub fn into_keys(self) -> impl Iterator<Item = String> {
        self.table
            .into_iter()
            .flat_map(|bucket| bucket.into_iter().map(|e| e.key))
    }
}

impl<V> IntoIterator for JavaHashMap<V> {
    type Item = (String, V);
    type IntoIter = std::vec::IntoIter<(String, V)>;

    /// Consumes the map and yields its entries in `HashMap.entrySet()` order.
    fn into_iter(self) -> Self::IntoIter {
        let entries: Vec<(String, V)> = self
            .table
            .into_iter()
            .flat_map(|bucket| bucket.into_iter().map(|e| (e.key, e.value)))
            .collect();
        entries.into_iter()
    }
}

impl<V: Clone> From<&JavaHashMap<V>> for HashMap<String, V> {
    fn from(map: &JavaHashMap<V>) -> Self {
        map.to_std()
    }
}

/// A `java.util.HashMap<Integer, V>` with Java's iteration order.
///
/// `Integer.hashCode()` is the value itself, so the bucket of key `k` is
/// `(k ^ (k >>> 16)) & (n - 1)`; everything else — lazy allocation, the load
/// factor, the resize that keeps each chain's order, `clear` keeping the table —
/// is [`JavaHashMap`]'s. Lucene keys `SegmentCommitInfo.dvUpdatesFiles` by
/// field number with such a map, and `SegmentInfos.write` writes it in this
/// order (`SegmentInfos.java:696-702`).
///
/// Like [`JavaHashMap`], a bucket that reaches the tree shape is flagged by
/// [`JavaIntHashMap::is_treeified`] and no longer ordered like Java past it.
#[derive(Clone, Debug)]
pub struct JavaIntHashMap<V> {
    table: Vec<Vec<(i32, V)>>,
    size: usize,
    threshold: i32,
    treeified: bool,
}

impl<V> Default for JavaIntHashMap<V> {
    fn default() -> Self {
        Self::new()
    }
}

/// `HashMap.hash(Object)` of an `Integer`.
fn spread_int(key: i32) -> i32 {
    key ^ ((key as u32) >> 16) as i32
}

impl<V> JavaIntHashMap<V> {
    /// `new HashMap<>()`.
    pub fn new() -> Self {
        Self {
            table: Vec::new(),
            size: 0,
            threshold: 0,
            treeified: false,
        }
    }

    /// `new HashMap<>(int initialCapacity)`.
    pub fn with_initial_capacity(initial_capacity: i32) -> Self {
        assert!(
            initial_capacity >= 0,
            "illegal initial capacity: {initial_capacity}"
        );
        Self {
            table: Vec::new(),
            size: 0,
            threshold: table_size_for(initial_capacity),
            treeified: false,
        }
    }

    /// `HashMap.newHashMap(int numMappings)`: an initial capacity of
    /// `ceil(numMappings / 0.75)`, so that many mappings fit without a resize.
    pub fn new_hash_map(num_mappings: usize) -> Self {
        Self::with_initial_capacity(
            (num_mappings as f64 / f64::from(DEFAULT_LOAD_FACTOR)).ceil() as i32,
        )
    }

    /// `HashMap.size()`.
    pub fn len(&self) -> usize {
        self.size
    }

    /// `HashMap.isEmpty()`.
    pub fn is_empty(&self) -> bool {
        self.size == 0
    }

    /// The current table length, or 0 while the table is unallocated.
    pub fn table_length(&self) -> usize {
        self.table.len()
    }

    /// See [`JavaHashMap::is_treeified`].
    pub fn is_treeified(&self) -> bool {
        self.treeified
    }

    /// `HashMap.get(Object)`.
    pub fn get(&self, key: i32) -> Option<&V> {
        if self.table.is_empty() {
            return None;
        }
        self.table[index_for(spread_int(key), self.table.len())]
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, v)| v)
    }

    /// `HashMap.containsKey(Object)`.
    pub fn contains_key(&self, key: i32) -> bool {
        self.get(key).is_some()
    }

    /// `HashMap.put(K, V)`, returning the previous value as Java does.
    pub fn insert(&mut self, key: i32, value: V) -> Option<V> {
        if self.table.is_empty() {
            self.resize();
        }
        let n = self.table.len();
        let bucket = &mut self.table[index_for(spread_int(key), n)];
        if let Some(entry) = bucket.iter_mut().find(|(k, _)| *k == key) {
            return Some(std::mem::replace(&mut entry.1, value));
        }
        bucket.push((key, value));
        if bucket.len() > TREEIFY_THRESHOLD {
            if n < MIN_TREEIFY_CAPACITY {
                self.resize();
            } else {
                self.treeified = true;
            }
        }
        self.size += 1;
        if self.size as i32 > self.threshold {
            self.resize();
        }
        None
    }

    /// `HashMap.clear()`, which keeps the table it has.
    pub fn clear(&mut self) {
        for bucket in self.table.iter_mut() {
            bucket.clear();
        }
        self.size = 0;
        self.treeified = false;
    }

    /// Iterates the entries in `HashMap.entrySet()` order.
    pub fn iter(&self) -> impl Iterator<Item = (i32, &V)> {
        self.table
            .iter()
            .flat_map(|bucket| bucket.iter().map(|(k, v)| (*k, v)))
    }

    /// Iterates the values in `HashMap.values()` order.
    pub fn values(&self) -> impl Iterator<Item = &V> {
        self.iter().map(|(_, v)| v)
    }

    /// `HashMap.resize()`; see [`JavaHashMap`]'s, which this mirrors.
    fn resize(&mut self) {
        let old_cap = self.table.len();
        let old_thr = self.threshold;
        let new_cap: usize;
        let mut new_thr: i32 = 0;
        if old_cap > 0 {
            if old_cap >= MAXIMUM_CAPACITY {
                self.threshold = i32::MAX;
                return;
            }
            new_cap = old_cap << 1;
            if new_cap < MAXIMUM_CAPACITY && old_cap >= DEFAULT_INITIAL_CAPACITY {
                new_thr = old_thr << 1;
            }
        } else if old_thr > 0 {
            new_cap = old_thr as usize;
        } else {
            new_cap = DEFAULT_INITIAL_CAPACITY;
            new_thr = (DEFAULT_LOAD_FACTOR * DEFAULT_INITIAL_CAPACITY as f32) as i32;
        }
        if new_thr == 0 {
            let ft = new_cap as f32 * DEFAULT_LOAD_FACTOR;
            new_thr = if new_cap < MAXIMUM_CAPACITY && ft < MAXIMUM_CAPACITY as f32 {
                ft as i32
            } else {
                i32::MAX
            };
        }
        self.threshold = new_thr;
        let old_table = std::mem::take(&mut self.table);
        let mut new_table: Vec<Vec<(i32, V)>> = Vec::with_capacity(new_cap);
        new_table.resize_with(new_cap, Vec::new);
        for bucket in old_table {
            for (key, value) in bucket {
                new_table[index_for(spread_int(key), new_cap)].push((key, value));
            }
        }
        self.table = new_table;
    }
}

/// A `java.util.HashMap<Long, V>` with Java's iteration order, treeified bins
/// included.
///
/// `Long.hashCode()` is `(int) (value ^ (value >>> 32))`, spread by
/// `h ^ (h >>> 16)`; the bucket is that hash modulo the table length.
/// Everything that decides the iteration order is reproduced from the JDK's
/// `java.util.HashMap` (identical in JDK 21 and 25):
///
/// * lazy allocation, the load factor and the doubling resize, which keeps the
///   order of each list bin and splits a tree bin with `TreeNode.split`;
/// * `treeifyBin` once a bin holds more than `TREEIFY_THRESHOLD` (8) nodes —
///   a resize instead while the table is shorter than `MIN_TREEIFY_CAPACITY`
///   (64);
/// * the red-black tree of a treeified bin, ordered by hash and then by
///   `Long.compareTo` (so `tieBreakOrder` is never reached), with the linked
///   list that iteration walks: `putTreeVal` links a new node right after its
///   tree parent, `treeify`, `putTreeVal` and `removeTreeNode` move the root to
///   the front of the bin (`moveRootToFront`), `removeTreeNode` unlinks the
///   node and untreeifies a tree that became too small, and `split` untreeifies
///   a half of at most `UNTREEIFY_THRESHOLD` (6) nodes;
/// * `remove` unlinking from a list bin, `clear` keeping the table, and
///   `putAll` pre-sizing.
///
/// Lucene keys `SnapshotDeletionPolicy.refCounts` and `indexCommits` by commit
/// generation with such maps, and `PersistentSnapshotDeletionPolicy.persist`
/// writes `refCounts` in this order (`PersistentSnapshotDeletionPolicy.java:176-180`).
#[derive(Clone, Debug)]
pub struct JavaLongHashMap<V> {
    table: Vec<LongBin<V>>,
    size: usize,
    threshold: i32,
}

/// One bin of a [`JavaLongHashMap`]: a plain chain, or a treeified one.
#[derive(Clone, Debug)]
enum LongBin<V> {
    /// `Node` chain, head to tail.
    List(Vec<(i64, V)>),
    /// `TreeNode` bin.
    Tree(TreeBin<V>),
}

/// The `TreeNode`s of one treeified bin, in an arena.
#[derive(Clone, Debug)]
struct TreeBin<V> {
    nodes: Vec<Option<TreeNode<V>>>,
    /// `tab[index]`: the head of the bin's linked list.
    first: Option<usize>,
}

/// `HashMap.TreeNode`: the tree links and the linked-list links.
#[derive(Clone, Debug)]
struct TreeNode<V> {
    hash: i32,
    key: i64,
    value: V,
    parent: Option<usize>,
    left: Option<usize>,
    right: Option<usize>,
    prev: Option<usize>,
    next: Option<usize>,
    red: bool,
}

/// `HashMap.UNTREEIFY_THRESHOLD`.
const UNTREEIFY_THRESHOLD: usize = 6;

impl<V> Default for JavaLongHashMap<V> {
    fn default() -> Self {
        Self::new()
    }
}

/// `HashMap.hash(Object)` of a `Long`: `Long.hashCode()` spread.
fn spread_long(key: i64) -> i32 {
    let h = (key ^ ((key as u64) >> 32) as i64) as i32;
    h ^ ((h as u32) >> 16) as i32
}

/// The `dir` of `treeify` and `putTreeVal` for a key `(h, k)` against a node
/// `(ph, pk)`: by hash, then by `Long.compareTo`. Distinct `Long`s never compare
/// equal, so `tieBreakOrder` is never consulted.
fn tree_dir(h: i32, k: i64, ph: i32, pk: i64) -> i32 {
    if ph > h {
        -1
    } else if ph < h {
        1
    } else {
        match k.cmp(&pk) {
            std::cmp::Ordering::Less => -1,
            std::cmp::Ordering::Greater => 1,
            std::cmp::Ordering::Equal => 0,
        }
    }
}

impl<V> TreeBin<V> {
    fn node(&self, i: usize) -> &TreeNode<V> {
        self.nodes[i]
            .as_ref()
            .expect("INVARIANT: a linked tree node is live")
    }

    fn node_mut(&mut self, i: usize) -> &mut TreeNode<V> {
        self.nodes[i]
            .as_mut()
            .expect("INVARIANT: a linked tree node is live")
    }

    /// The entries in linked-list order, from `tab[index]`.
    fn list_order(&self) -> Vec<usize> {
        let mut out = Vec::new();
        let mut at = self.first;
        while let Some(i) = at {
            out.push(i);
            at = self.node(i).next;
        }
        out
    }

    /// `TreeNode.root()`.
    fn root_of(&self, mut i: usize) -> usize {
        while let Some(p) = self.node(i).parent {
            i = p;
        }
        i
    }

    /// Builds a bin from a list chain and treeifies it:
    /// `treeifyBin`'s relinking followed by `TreeNode.treeify`.
    fn from_list(entries: Vec<(i64, V)>) -> Self {
        let n = entries.len();
        let nodes = entries
            .into_iter()
            .enumerate()
            .map(|(i, (key, value))| {
                Some(TreeNode {
                    hash: spread_long(key),
                    key,
                    value,
                    parent: None,
                    left: None,
                    right: None,
                    prev: i.checked_sub(1),
                    next: (i + 1 < n).then_some(i + 1),
                    red: false,
                })
            })
            .collect();
        let mut bin = Self {
            nodes,
            first: (n > 0).then_some(0),
        };
        bin.treeify();
        bin
    }

    /// `TreeNode.treeify(tab)`: builds the tree over the list, in list order,
    /// then moves the root to the front.
    fn treeify(&mut self) {
        let mut root: Option<usize> = None;
        let mut at = self.first;
        while let Some(x) = at {
            at = self.node(x).next;
            {
                let node = self.node_mut(x);
                node.left = None;
                node.right = None;
            }
            match root {
                None => {
                    let node = self.node_mut(x);
                    node.parent = None;
                    node.red = false;
                    root = Some(x);
                }
                Some(r) => {
                    let (h, k) = (self.node(x).hash, self.node(x).key);
                    let mut p = r;
                    loop {
                        let dir = tree_dir(h, k, self.node(p).hash, self.node(p).key);
                        let xp = p;
                        let child = if dir <= 0 {
                            self.node(p).left
                        } else {
                            self.node(p).right
                        };
                        match child {
                            Some(c) => p = c,
                            None => {
                                self.node_mut(x).parent = Some(xp);
                                if dir <= 0 {
                                    self.node_mut(xp).left = Some(x);
                                } else {
                                    self.node_mut(xp).right = Some(x);
                                }
                                root = Some(self.balance_insertion(r, x));
                                break;
                            }
                        }
                    }
                }
            }
        }
        if let Some(r) = root {
            self.move_root_to_front(r);
        }
    }

    /// `TreeNode.moveRootToFront(tab, root)`.
    fn move_root_to_front(&mut self, root: usize) {
        let first = self.first;
        if Some(root) != first {
            self.first = Some(root);
            let rp = self.node(root).prev;
            let rn = self.node(root).next;
            if let Some(rn) = rn {
                self.node_mut(rn).prev = rp;
            }
            if let Some(rp) = rp {
                self.node_mut(rp).next = rn;
            }
            if let Some(first) = first {
                self.node_mut(first).prev = Some(root);
            }
            let node = self.node_mut(root);
            node.next = first;
            node.prev = None;
        }
    }

    /// `TreeNode.find(h, k, kc)` from `p`.
    fn find(&self, mut p: Option<usize>, h: i32, k: i64) -> Option<usize> {
        while let Some(i) = p {
            let node = self.node(i);
            let (pl, pr) = (node.left, node.right);
            if node.hash > h {
                p = pl;
            } else if node.hash < h {
                p = pr;
            } else if node.key == k {
                return Some(i);
            } else if pl.is_none() {
                p = pr;
            } else if pr.is_none() {
                p = pl;
            } else {
                p = if k < node.key { pl } else { pr };
            }
        }
        None
    }

    /// `TreeNode.putTreeVal`: returns the existing node of `key`, or links a
    /// new one after its tree parent and rebalances.
    fn put_tree_val(&mut self, key: i64, value: V) -> Result<(), (usize, V)> {
        let h = spread_long(key);
        let first = self.first.expect("INVARIANT: a tree bin is never empty");
        let root = self.root_of(first);
        let mut p = root;
        loop {
            let (ph, pk) = (self.node(p).hash, self.node(p).key);
            if ph == h && pk == key {
                return Err((p, value));
            }
            let dir = tree_dir(h, key, ph, pk);
            let xp = p;
            let child = if dir <= 0 {
                self.node(p).left
            } else {
                self.node(p).right
            };
            match child {
                Some(c) => p = c,
                None => {
                    let xpn = self.node(xp).next;
                    let x = self.nodes.len();
                    self.nodes.push(Some(TreeNode {
                        hash: h,
                        key,
                        value,
                        parent: Some(xp),
                        left: None,
                        right: None,
                        prev: Some(xp),
                        next: xpn,
                        red: false,
                    }));
                    if dir <= 0 {
                        self.node_mut(xp).left = Some(x);
                    } else {
                        self.node_mut(xp).right = Some(x);
                    }
                    self.node_mut(xp).next = Some(x);
                    if let Some(xpn) = xpn {
                        self.node_mut(xpn).prev = Some(x);
                    }
                    let root = self.balance_insertion(root, x);
                    self.move_root_to_front(root);
                    return Ok(());
                }
            }
        }
    }

    /// `TreeNode.removeTreeNode(map, tab, movable = true)`. Returns the
    /// removed value, and whether the bin must be untreeified ("too small").
    fn remove_tree_node(&mut self, this: usize) -> (V, bool) {
        let first = self.first.expect("INVARIANT: a tree bin is never empty");
        let succ = self.node(this).next;
        let pred = self.node(this).prev;
        let mut first_now = Some(first);
        match pred {
            None => {
                self.first = succ;
                first_now = succ;
            }
            Some(pred) => self.node_mut(pred).next = succ,
        }
        if let Some(succ) = succ {
            self.node_mut(succ).prev = pred;
        }
        let Some(first_now) = first_now else {
            let removed = self.nodes[this].take().expect("INVARIANT: live").value;
            return (removed, false);
        };
        let mut root = first_now;
        if self.node(root).parent.is_some() {
            root = self.root_of(root);
        }
        let too_small = {
            let r = self.node(root);
            r.right.is_none()
                || match r.left {
                    None => true,
                    Some(rl) => self.node(rl).left.is_none(),
                }
        };
        if too_small {
            let removed = self.nodes[this].take().expect("INVARIANT: live").value;
            return (removed, true);
        }
        let p = this;
        let pl = self.node(p).left;
        let pr = self.node(p).right;
        let replacement;
        if let (Some(pl), Some(pr)) = (pl, pr) {
            let mut s = pr;
            while let Some(sl) = self.node(s).left {
                s = sl;
            }
            let c = self.node(s).red;
            let pred_red = self.node(p).red;
            self.node_mut(s).red = pred_red;
            self.node_mut(p).red = c;
            let sr = self.node(s).right;
            let pp = self.node(p).parent;
            if s == pr {
                self.node_mut(p).parent = Some(s);
                self.node_mut(s).right = Some(p);
            } else {
                let sp = self.node(s).parent;
                self.node_mut(p).parent = sp;
                if let Some(sp) = sp {
                    if self.node(sp).left == Some(s) {
                        self.node_mut(sp).left = Some(p);
                    } else {
                        self.node_mut(sp).right = Some(p);
                    }
                }
                self.node_mut(s).right = Some(pr);
                self.node_mut(pr).parent = Some(s);
            }
            self.node_mut(p).left = None;
            self.node_mut(p).right = sr;
            if let Some(sr) = sr {
                self.node_mut(sr).parent = Some(p);
            }
            self.node_mut(s).left = Some(pl);
            self.node_mut(pl).parent = Some(s);
            self.node_mut(s).parent = pp;
            match pp {
                None => root = s,
                Some(pp) => {
                    if self.node(pp).left == Some(p) {
                        self.node_mut(pp).left = Some(s);
                    } else {
                        self.node_mut(pp).right = Some(s);
                    }
                }
            }
            replacement = sr.unwrap_or(p);
        } else if let Some(pl) = pl {
            replacement = pl;
        } else if let Some(pr) = pr {
            replacement = pr;
        } else {
            replacement = p;
        }
        if replacement != p {
            let pp = self.node(p).parent;
            self.node_mut(replacement).parent = pp;
            match pp {
                None => {
                    root = replacement;
                    self.node_mut(replacement).red = false;
                }
                Some(pp) => {
                    if self.node(pp).left == Some(p) {
                        self.node_mut(pp).left = Some(replacement);
                    } else {
                        self.node_mut(pp).right = Some(replacement);
                    }
                }
            }
            let node = self.node_mut(p);
            node.left = None;
            node.right = None;
            node.parent = None;
        }
        let r = if self.node(p).red {
            root
        } else {
            self.balance_deletion(root, Some(replacement))
        };
        if replacement == p {
            // detach
            let pp = self.node(p).parent;
            self.node_mut(p).parent = None;
            if let Some(pp) = pp {
                if self.node(pp).left == Some(p) {
                    self.node_mut(pp).left = None;
                } else if self.node(pp).right == Some(p) {
                    self.node_mut(pp).right = None;
                }
            }
        }
        self.move_root_to_front(r);
        let removed = self.nodes[this].take().expect("INVARIANT: live").value;
        (removed, false)
    }

    /// `TreeNode.untreeify(map)`: the list, in order.
    fn untreeify(self) -> Vec<(i64, V)> {
        let order = self.list_order();
        let mut nodes = self.nodes;
        order
            .into_iter()
            .map(|i| {
                let node = nodes[i].take().expect("INVARIANT: live");
                (node.key, node.value)
            })
            .collect()
    }

    /// `TreeNode.rotateLeft(root, p)`.
    fn rotate_left(&mut self, mut root: usize, p: usize) -> usize {
        if let Some(r) = self.node(p).right {
            let rl = self.node(r).left;
            self.node_mut(p).right = rl;
            if let Some(rl) = rl {
                self.node_mut(rl).parent = Some(p);
            }
            let pp = self.node(p).parent;
            self.node_mut(r).parent = pp;
            match pp {
                None => {
                    root = r;
                    self.node_mut(r).red = false;
                }
                Some(pp) => {
                    if self.node(pp).left == Some(p) {
                        self.node_mut(pp).left = Some(r);
                    } else {
                        self.node_mut(pp).right = Some(r);
                    }
                }
            }
            self.node_mut(r).left = Some(p);
            self.node_mut(p).parent = Some(r);
        }
        root
    }

    /// `TreeNode.rotateRight(root, p)`.
    fn rotate_right(&mut self, mut root: usize, p: usize) -> usize {
        if let Some(l) = self.node(p).left {
            let lr = self.node(l).right;
            self.node_mut(p).left = lr;
            if let Some(lr) = lr {
                self.node_mut(lr).parent = Some(p);
            }
            let pp = self.node(p).parent;
            self.node_mut(l).parent = pp;
            match pp {
                None => {
                    root = l;
                    self.node_mut(l).red = false;
                }
                Some(pp) => {
                    if self.node(pp).right == Some(p) {
                        self.node_mut(pp).right = Some(l);
                    } else {
                        self.node_mut(pp).left = Some(l);
                    }
                }
            }
            self.node_mut(l).right = Some(p);
            self.node_mut(p).parent = Some(l);
        }
        root
    }

    fn is_red(&self, i: Option<usize>) -> bool {
        i.is_some_and(|i| self.node(i).red)
    }

    /// `TreeNode.balanceInsertion(root, x)`.
    fn balance_insertion(&mut self, mut root: usize, mut x: usize) -> usize {
        self.node_mut(x).red = true;
        loop {
            let Some(mut xp) = self.node(x).parent else {
                self.node_mut(x).red = false;
                return x;
            };
            if !self.node(xp).red {
                return root;
            }
            let Some(mut xpp) = self.node(xp).parent else {
                return root;
            };
            let xppl = self.node(xpp).left;
            if Some(xp) == xppl {
                let xppr = self.node(xpp).right;
                if self.is_red(xppr) {
                    self.node_mut(xppr.expect("red implies present")).red = false;
                    self.node_mut(xp).red = false;
                    self.node_mut(xpp).red = true;
                    x = xpp;
                } else {
                    let mut xp_opt = Some(xp);
                    let mut xpp_opt = Some(xpp);
                    if Some(x) == self.node(xp).right {
                        x = xp;
                        root = self.rotate_left(root, x);
                        xp_opt = self.node(x).parent;
                        xpp_opt = xp_opt.and_then(|xp| self.node(xp).parent);
                    }
                    if let Some(xp2) = xp_opt {
                        xp = xp2;
                        self.node_mut(xp).red = false;
                        if let Some(xpp2) = xpp_opt {
                            xpp = xpp2;
                            self.node_mut(xpp).red = true;
                            root = self.rotate_right(root, xpp);
                        }
                    }
                }
            } else if self.is_red(xppl) {
                self.node_mut(xppl.expect("red implies present")).red = false;
                self.node_mut(xp).red = false;
                self.node_mut(xpp).red = true;
                x = xpp;
            } else {
                let mut xp_opt = Some(xp);
                let mut xpp_opt = Some(xpp);
                if Some(x) == self.node(xp).left {
                    x = xp;
                    root = self.rotate_right(root, x);
                    xp_opt = self.node(x).parent;
                    xpp_opt = xp_opt.and_then(|xp| self.node(xp).parent);
                }
                if let Some(xp2) = xp_opt {
                    xp = xp2;
                    self.node_mut(xp).red = false;
                    if let Some(xpp2) = xpp_opt {
                        xpp = xpp2;
                        self.node_mut(xpp).red = true;
                        root = self.rotate_left(root, xpp);
                    }
                }
            }
        }
    }

    /// `TreeNode.balanceDeletion(root, x)`.
    fn balance_deletion(&mut self, mut root: usize, mut x: Option<usize>) -> usize {
        loop {
            let Some(xi) = x else {
                return root;
            };
            if xi == root {
                return root;
            }
            let Some(mut xp) = self.node(xi).parent else {
                self.node_mut(xi).red = false;
                return xi;
            };
            if self.node(xi).red {
                self.node_mut(xi).red = false;
                return root;
            }
            let xpl = self.node(xp).left;
            if xpl == Some(xi) {
                let mut xpr = self.node(xp).right;
                let mut xp_opt = Some(xp);
                if self.is_red(xpr) {
                    self.node_mut(xpr.expect("red implies present")).red = false;
                    self.node_mut(xp).red = true;
                    root = self.rotate_left(root, xp);
                    xp_opt = self.node(xi).parent;
                    xpr = xp_opt.and_then(|xp| self.node(xp).right);
                }
                match xpr {
                    None => x = xp_opt,
                    Some(xpr_i) => {
                        let sl = self.node(xpr_i).left;
                        let sr = self.node(xpr_i).right;
                        if !self.is_red(sr) && !self.is_red(sl) {
                            self.node_mut(xpr_i).red = true;
                            x = xp_opt;
                        } else {
                            let mut xpr2 = Some(xpr_i);
                            if !self.is_red(sr) {
                                if let Some(sl) = sl {
                                    self.node_mut(sl).red = false;
                                }
                                self.node_mut(xpr_i).red = true;
                                root = self.rotate_right(root, xpr_i);
                                xp_opt = self.node(xi).parent;
                                xpr2 = xp_opt.and_then(|xp| self.node(xp).right);
                            }
                            if let Some(xpr2) = xpr2 {
                                let red = xp_opt.is_some_and(|xp| self.node(xp).red);
                                self.node_mut(xpr2).red = red;
                                if let Some(sr) = self.node(xpr2).right {
                                    self.node_mut(sr).red = false;
                                }
                            }
                            if let Some(xp2) = xp_opt {
                                xp = xp2;
                                self.node_mut(xp).red = false;
                                root = self.rotate_left(root, xp);
                            }
                            x = Some(root);
                        }
                    }
                }
            } else {
                let mut xpl = xpl;
                let mut xp_opt = Some(xp);
                if self.is_red(xpl) {
                    self.node_mut(xpl.expect("red implies present")).red = false;
                    self.node_mut(xp).red = true;
                    root = self.rotate_right(root, xp);
                    xp_opt = self.node(xi).parent;
                    xpl = xp_opt.and_then(|xp| self.node(xp).left);
                }
                match xpl {
                    None => x = xp_opt,
                    Some(xpl_i) => {
                        let sl = self.node(xpl_i).left;
                        let sr = self.node(xpl_i).right;
                        if !self.is_red(sl) && !self.is_red(sr) {
                            self.node_mut(xpl_i).red = true;
                            x = xp_opt;
                        } else {
                            let mut xpl2 = Some(xpl_i);
                            if !self.is_red(sl) {
                                if let Some(sr) = sr {
                                    self.node_mut(sr).red = false;
                                }
                                self.node_mut(xpl_i).red = true;
                                root = self.rotate_left(root, xpl_i);
                                xp_opt = self.node(xi).parent;
                                xpl2 = xp_opt.and_then(|xp| self.node(xp).left);
                            }
                            if let Some(xpl2) = xpl2 {
                                let red = xp_opt.is_some_and(|xp| self.node(xp).red);
                                self.node_mut(xpl2).red = red;
                                if let Some(sl) = self.node(xpl2).left {
                                    self.node_mut(sl).red = false;
                                }
                            }
                            if let Some(xp2) = xp_opt {
                                xp = xp2;
                                self.node_mut(xp).red = false;
                                root = self.rotate_right(root, xp);
                            }
                            x = Some(root);
                        }
                    }
                }
            }
        }
    }
}

impl<V> LongBin<V> {
    /// The entries in iteration order.
    fn entries(&self) -> Vec<(i64, &V)> {
        match self {
            Self::List(entries) => entries.iter().map(|(k, v)| (*k, v)).collect(),
            Self::Tree(tree) => tree
                .list_order()
                .into_iter()
                .map(|i| {
                    let node = tree.node(i);
                    (node.key, &node.value)
                })
                .collect(),
        }
    }
}

impl<V> JavaLongHashMap<V> {
    /// `new HashMap<>()`.
    pub fn new() -> Self {
        Self {
            table: Vec::new(),
            size: 0,
            threshold: 0,
        }
    }

    /// `HashMap.size()`.
    pub fn len(&self) -> usize {
        self.size
    }

    /// `HashMap.isEmpty()`.
    pub fn is_empty(&self) -> bool {
        self.size == 0
    }

    /// The current table length, or 0 while the table is unallocated.
    pub fn table_length(&self) -> usize {
        self.table.len()
    }

    /// Whether the bin `key` falls in is treeified.
    pub fn is_treeified_at(&self, key: i64) -> bool {
        !self.table.is_empty()
            && matches!(
                self.table[index_for(spread_long(key), self.table.len())],
                LongBin::Tree(_)
            )
    }

    /// `HashMap.get(Object)`.
    pub fn get(&self, key: i64) -> Option<&V> {
        if self.table.is_empty() {
            return None;
        }
        match &self.table[index_for(spread_long(key), self.table.len())] {
            LongBin::List(entries) => entries.iter().find(|(k, _)| *k == key).map(|(_, v)| v),
            LongBin::Tree(tree) => {
                let root = tree.root_of(tree.first?);
                tree.find(Some(root), spread_long(key), key)
                    .map(|i| &tree.node(i).value)
            }
        }
    }

    /// `HashMap.containsKey(Object)`.
    pub fn contains_key(&self, key: i64) -> bool {
        self.get(key).is_some()
    }

    /// `HashMap.put(K, V)` — `putVal` — returning the previous value.
    pub fn insert(&mut self, key: i64, value: V) -> Option<V> {
        if self.table.is_empty() {
            self.resize();
        }
        let n = self.table.len();
        let index = index_for(spread_long(key), n);
        let treeify = match &mut self.table[index] {
            LongBin::Tree(tree) => match tree.put_tree_val(key, value) {
                Ok(()) => false,
                Err((existing, value)) => {
                    return Some(std::mem::replace(&mut tree.node_mut(existing).value, value));
                }
            },
            LongBin::List(entries) => {
                if let Some(entry) = entries.iter_mut().find(|(k, _)| *k == key) {
                    return Some(std::mem::replace(&mut entry.1, value));
                }
                entries.push((key, value));
                // `binCount >= TREEIFY_THRESHOLD - 1`: the bin now holds more
                // than `TREEIFY_THRESHOLD` nodes.
                entries.len() > TREEIFY_THRESHOLD
            }
        };
        if treeify {
            self.treeify_bin(index);
        }
        self.size += 1;
        if self.size as i32 > self.threshold {
            self.resize();
        }
        None
    }

    /// `HashMap.treeifyBin(tab, hash)`: a resize while the table is shorter
    /// than `MIN_TREEIFY_CAPACITY`, otherwise the bin becomes a tree.
    fn treeify_bin(&mut self, index: usize) {
        if self.table.len() < MIN_TREEIFY_CAPACITY {
            self.resize();
        } else {
            let bin = std::mem::replace(&mut self.table[index], LongBin::List(Vec::new()));
            self.table[index] = match bin {
                LongBin::List(entries) => LongBin::Tree(TreeBin::from_list(entries)),
                tree => tree,
            };
        }
    }

    /// `HashMap.remove(Object)` — `removeNode(..., movable = true)`.
    pub fn remove(&mut self, key: i64) -> Option<V> {
        if self.table.is_empty() {
            return None;
        }
        let n = self.table.len();
        let index = index_for(spread_long(key), n);
        let removed = match &mut self.table[index] {
            LongBin::List(entries) => {
                let position = entries.iter().position(|(k, _)| *k == key)?;
                entries.remove(position).1
            }
            LongBin::Tree(tree) => {
                let first = tree.first?;
                let node = if tree.node(first).key == key {
                    first
                } else {
                    let root = tree.root_of(first);
                    tree.find(Some(root), spread_long(key), key)?
                };
                let (value, too_small) = tree.remove_tree_node(node);
                if too_small || tree.first.is_none() {
                    let bin = std::mem::replace(&mut self.table[index], LongBin::List(Vec::new()));
                    if let LongBin::Tree(tree) = bin {
                        self.table[index] = LongBin::List(tree.untreeify());
                    }
                }
                value
            }
        };
        self.size -= 1;
        Some(removed)
    }

    /// `HashMap.clear()`, which keeps the table it has.
    pub fn clear(&mut self) {
        for bin in self.table.iter_mut() {
            *bin = LongBin::List(Vec::new());
        }
        self.size = 0;
    }

    /// Iterates the entries in `HashMap.entrySet()` order.
    pub fn iter(&self) -> impl Iterator<Item = (i64, &V)> {
        self.table.iter().flat_map(|bin| bin.entries())
    }

    /// Iterates the keys in `HashMap.keySet()` order.
    pub fn keys(&self) -> impl Iterator<Item = i64> + '_ {
        self.iter().map(|(k, _)| k)
    }

    /// Iterates the values in `HashMap.values()` order.
    pub fn values(&self) -> impl Iterator<Item = &V> {
        self.iter().map(|(_, v)| v)
    }

    /// `HashMap.putAll(Map)`: `putMapEntries` pre-sizes an unallocated table
    /// from the source's size (or doubles an allocated one until it fits),
    /// then inserts the entries in the source's iteration order.
    pub fn put_all<I>(&mut self, entries: I)
    where
        I: IntoIterator<Item = (i64, V)>,
        I::IntoIter: ExactSizeIterator,
    {
        let entries = entries.into_iter();
        let size = entries.len();
        if size == 0 {
            return;
        }
        if self.table.is_empty() {
            let dt = (size as f64 / DEFAULT_LOAD_FACTOR as f64).ceil();
            let t = if dt < MAXIMUM_CAPACITY as f64 {
                dt as i32
            } else {
                MAXIMUM_CAPACITY as i32
            };
            if t > self.threshold {
                self.threshold = table_size_for(t);
            }
        } else {
            while size as i32 > self.threshold && self.table.len() < MAXIMUM_CAPACITY {
                self.resize();
            }
        }
        for (key, value) in entries {
            self.insert(key, value);
        }
    }

    /// `HashMap.resize()`: list bins keep their order when split; tree bins
    /// split with `TreeNode.split`.
    fn resize(&mut self) {
        let old_cap = self.table.len();
        let old_thr = self.threshold;
        let new_cap: usize;
        let mut new_thr: i32 = 0;
        if old_cap > 0 {
            if old_cap >= MAXIMUM_CAPACITY {
                self.threshold = i32::MAX;
                return;
            }
            new_cap = old_cap << 1;
            if new_cap < MAXIMUM_CAPACITY && old_cap >= DEFAULT_INITIAL_CAPACITY {
                new_thr = old_thr << 1;
            }
        } else if old_thr > 0 {
            new_cap = old_thr as usize;
        } else {
            new_cap = DEFAULT_INITIAL_CAPACITY;
            new_thr = (DEFAULT_LOAD_FACTOR * DEFAULT_INITIAL_CAPACITY as f32) as i32;
        }
        if new_thr == 0 {
            let ft = new_cap as f32 * DEFAULT_LOAD_FACTOR;
            new_thr = if new_cap < MAXIMUM_CAPACITY && ft < MAXIMUM_CAPACITY as f32 {
                ft as i32
            } else {
                i32::MAX
            };
        }
        self.threshold = new_thr;
        let old_table = std::mem::take(&mut self.table);
        let mut new_table: Vec<LongBin<V>> = Vec::with_capacity(new_cap);
        new_table.resize_with(new_cap, || LongBin::List(Vec::new()));
        for (j, bin) in old_table.into_iter().enumerate() {
            match bin {
                LongBin::List(entries) => {
                    // preserve order
                    for (key, value) in entries {
                        if let LongBin::List(target) =
                            &mut new_table[index_for(spread_long(key), new_cap)]
                        {
                            target.push((key, value));
                        }
                    }
                }
                LongBin::Tree(tree) => {
                    let (lo, hi) = Self::split(tree, old_cap);
                    new_table[j] = lo;
                    new_table[j + old_cap] = hi;
                }
            }
        }
        self.table = new_table;
    }

    /// `TreeNode.split(map, tab, index, bit)`: relinks into lo and hi lists
    /// preserving order; a half of at most `UNTREEIFY_THRESHOLD` nodes is
    /// untreeified, a larger one is treeified again unless every node stayed
    /// on its side, in which case the tree is kept as it is.
    fn split(tree: TreeBin<V>, bit: usize) -> (LongBin<V>, LongBin<V>) {
        let mut lo: Vec<(i64, V)> = Vec::new();
        let mut hi: Vec<(i64, V)> = Vec::new();
        let order = tree.list_order();
        let all_lo = order
            .iter()
            .all(|&i| (tree.node(i).hash as u32 as usize) & bit == 0);
        let all_hi = order
            .iter()
            .all(|&i| (tree.node(i).hash as u32 as usize) & bit != 0);
        let count = order.len();
        if (all_lo || all_hi) && count > UNTREEIFY_THRESHOLD {
            // `else is already treeified`: the same nodes, list and tree.
            let kept = LongBin::Tree(tree);
            let empty = LongBin::List(Vec::new());
            return if all_lo { (kept, empty) } else { (empty, kept) };
        }
        let mut nodes = tree.nodes;
        for i in order {
            let node = nodes[i].take().expect("INVARIANT: live");
            if (node.hash as u32 as usize) & bit == 0 {
                lo.push((node.key, node.value));
            } else {
                hi.push((node.key, node.value));
            }
        }
        let rebuild = |entries: Vec<(i64, V)>| {
            if entries.len() <= UNTREEIFY_THRESHOLD {
                LongBin::List(entries)
            } else {
                LongBin::Tree(TreeBin::from_list(entries))
            }
        };
        (rebuild(lo), rebuild(hi))
    }
}

/// A `java.util.HashSet<String>` with Java's iteration order.
///
/// `java.util.HashSet` is a `HashMap` whose values are all one shared sentinel
/// (`HashSet.PRESENT`), so its iteration order is its backing map's and this
/// port is the same wrapper over [`JavaHashMap`].
#[derive(Clone, Default)]
pub struct JavaHashSet {
    map: JavaHashMap<()>,
}

impl JavaHashSet {
    /// `new HashSet<>()`.
    pub fn new() -> Self {
        JavaHashSet {
            map: JavaHashMap::new(),
        }
    }

    /// `new HashSet<>(int initialCapacity)`.
    pub fn with_initial_capacity(initial_capacity: i32) -> Self {
        JavaHashSet {
            map: JavaHashMap::with_initial_capacity(initial_capacity),
        }
    }

    /// `HashSet.add(E)`, returning whether the set changed.
    pub fn insert(&mut self, value: String) -> bool {
        self.map.insert(value, ()).is_none()
    }

    /// `HashSet.remove(Object)`, returning whether the set changed.
    pub fn remove(&mut self, value: &str) -> bool {
        self.map.remove(value).is_some()
    }

    /// `HashSet.contains(Object)`.
    pub fn contains(&self, value: &str) -> bool {
        self.map.contains_key(value)
    }

    /// `HashSet.size()`.
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// `HashSet.isEmpty()`.
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// `HashSet.clear()`.
    pub fn clear(&mut self) {
        self.map.clear();
    }

    /// Iterates in `HashSet.iterator()` order.
    pub fn iter(&self) -> impl Iterator<Item = &String> {
        self.map.keys()
    }

    /// The current backing table length; see [`JavaHashMap::table_length`].
    pub fn table_length(&self) -> usize {
        self.map.table_length()
    }

    /// See [`JavaHashMap::is_treeified`].
    pub fn is_treeified(&self) -> bool {
        self.map.is_treeified()
    }

    /// The elements as a [`std::collections::HashSet`], for the callers that
    /// only test membership and never write the set out.
    pub fn to_std(&self) -> std::collections::HashSet<String> {
        self.iter().cloned().collect()
    }
}

/// `new HashSet<>()` followed by one `add` per item, in the iterator's order.
///
/// This is the shape `SegmentInfo.setFiles` builds
/// (`SegmentInfo.java:309-320`). Java's collection constructor
/// `new HashSet<>(Collection)` pre-sizes its backing map instead and so reaches
/// a different table length; that shape is [`JavaHashSet::from_collection`].
impl FromIterator<String> for JavaHashSet {
    fn from_iter<I: IntoIterator<Item = String>>(iter: I) -> Self {
        let mut set = JavaHashSet::new();
        for value in iter {
            set.insert(value);
        }
        set
    }
}

impl JavaHashSet {
    /// `new HashSet<>(Collection)`.
    ///
    /// The backing map is pre-sized from the collection's size; the exact
    /// expression is checked against the running JVM by the differential test,
    /// because it has changed between JDK releases.
    pub fn from_collection<I>(values: I) -> Self
    where
        I: IntoIterator<Item = String>,
        I::IntoIter: ExactSizeIterator,
    {
        let values = values.into_iter();
        // `HashMap.newHashMap(Math.max(c.size(), 12))`, i.e.
        // `new HashMap<>((int) Math.ceil(max(size, 12) / 0.75))`.
        let requested = (values.len() as i32).max(12);
        let capacity = (requested as f64 / DEFAULT_LOAD_FACTOR as f64).ceil() as i32;
        let mut set = JavaHashSet::with_initial_capacity(capacity);
        for value in values {
            set.insert(value);
        }
        set
    }
}

impl Extend<String> for JavaHashSet {
    fn extend<I: IntoIterator<Item = String>>(&mut self, iter: I) {
        for value in iter {
            self.insert(value);
        }
    }
}

impl IntoIterator for JavaHashSet {
    type Item = String;
    type IntoIter = std::vec::IntoIter<String>;

    fn into_iter(self) -> Self::IntoIter {
        let values: Vec<String> = self.map.into_keys().collect();
        values.into_iter()
    }
}

impl<'a> IntoIterator for &'a JavaHashSet {
    type Item = &'a String;
    type IntoIter = Box<dyn Iterator<Item = &'a String> + 'a>;

    fn into_iter(self) -> Self::IntoIter {
        Box::new(self.iter())
    }
}

/// `AbstractSet.equals`, which compares contents and ignores iteration order.
impl PartialEq for JavaHashSet {
    fn eq(&self, other: &Self) -> bool {
        self.map == other.map
    }
}

impl Eq for JavaHashSet {}

impl fmt::Debug for JavaHashSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_set().entries(self.iter()).finish()
    }
}

impl From<&JavaHashSet> for std::collections::HashSet<String> {
    fn from(set: &JavaHashSet) -> Self {
        set.to_std()
    }
}

// ---------------------------------------------------------------------------
// What `writeMapOfStrings` and `writeSetOfStrings` accept
// ---------------------------------------------------------------------------

/// A `Map<String, String>` whose iteration order is defined.
///
/// `DataOutput.writeMapOfStrings` takes a `Map<String, String>` and walks it
/// (`DataOutput.java:304-310`); this trait is that parameter, narrowed to the
/// maps whose order is something a reader could reproduce. A
/// [`std::collections::HashMap`] deliberately does not implement it.
pub trait JavaOrderedMap {
    /// `Map.size()`.
    fn entry_count(&self) -> usize;
    /// `Map.entrySet()`, in iteration order.
    fn entries(&self) -> Box<dyn Iterator<Item = (&String, &String)> + '_>;
}

/// A `Set<String>` whose iteration order is defined; the set counterpart of
/// [`JavaOrderedMap`], and the parameter of
/// `DataOutput.writeSetOfStrings` (`DataOutput.java:321-326`).
pub trait JavaOrderedSet {
    /// `Set.size()`.
    fn value_count(&self) -> usize;
    /// `Set.iterator()`, in iteration order.
    fn values(&self) -> Box<dyn Iterator<Item = &String> + '_>;
}

impl JavaOrderedMap for JavaHashMap<String> {
    fn entry_count(&self) -> usize {
        self.len()
    }
    fn entries(&self) -> Box<dyn Iterator<Item = (&String, &String)> + '_> {
        Box::new(self.iter())
    }
}

impl JavaOrderedSet for JavaHashSet {
    fn value_count(&self) -> usize {
        self.len()
    }
    fn values(&self) -> Box<dyn Iterator<Item = &String> + '_> {
        Box::new(self.iter())
    }
}

// ---------------------------------------------------------------------------
// The maps and sets DataInput hands back
// ---------------------------------------------------------------------------

/// `java.lang.String.compareTo(String)`.
///
/// Compares UTF-16 code units and falls back to the length difference, which is
/// the order a `java.util.TreeMap<String, ?>` iterates in. It is **not** the
/// order [`str`] compares in: Rust orders by UTF-8 bytes, i.e. by code point,
/// so it puts U+E000 before U+1D400 while Java, comparing the surrogate 0xD835
/// against 0xE000, puts U+1D400 first.
pub fn java_compare_to(a: &str, b: &str) -> Ordering {
    let mut left = a.encode_utf16();
    let mut right = b.encode_utf16();
    loop {
        match (left.next(), right.next()) {
            (Some(x), Some(y)) if x == y => continue,
            (Some(x), Some(y)) => return x.cmp(&y),
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
        }
    }
}

/// A `Map<String, String>` with the iteration order of whichever `java.util`
/// implementation Lucene is holding.
///
/// `DataInput.readMapOfStrings` does not hand back a `HashMap`: it returns
/// `Collections.emptyMap()` for a count of zero, `Collections.singletonMap` for
/// one, an unmodifiable `TreeMap` for two to ten and an unmodifiable `HashMap`
/// beyond that (`DataInput.java:261-276`). A `FieldInfo` read from a `.fnm`
/// therefore holds a **sorted** attribute map, and writing that `FieldInfo`
/// back out writes its attributes in sorted order — so a port that turned every
/// map into a `HashMap` on the way in would not rewrite the file it just read.
#[derive(Clone)]
pub enum JavaStringMap {
    /// `Collections.emptyMap()`, `Collections.singletonMap` or a `TreeMap`:
    /// all three iterate in ascending [`java_compare_to`] key order.
    Sorted(Vec<(String, String)>),
    /// A `java.util.HashMap`, in bucket order.
    Hash(JavaHashMap<String>),
}

impl Default for JavaStringMap {
    fn default() -> Self {
        JavaStringMap::Sorted(Vec::new())
    }
}

impl JavaStringMap {
    /// The map `DataInput.readMapOfStrings` builds for `entries` read in file
    /// order (`DataInput.java:261-276`).
    pub fn from_read(entries: Vec<(String, String)>) -> Self {
        if entries.len() > 10 {
            // `new HashMap<>()` then one `put` per entry, in file order.
            JavaStringMap::Hash(entries.into_iter().collect())
        } else {
            // Empty, singleton and `TreeMap` alike iterate sorted.
            let mut sorted = entries;
            sorted.sort_by(|(a, _), (b, _)| java_compare_to(a, b));
            // A repeated key is a later `put` over an earlier one, so the last
            // value read wins, as it does in the `TreeMap` Java fills.
            sorted.dedup_by(|(later_key, later_value), (kept_key, kept_value)| {
                if later_key == kept_key {
                    *kept_value = std::mem::take(later_value);
                    true
                } else {
                    false
                }
            });
            JavaStringMap::Sorted(sorted)
        }
    }

    /// Iterates in the underlying implementation's order.
    pub fn iter(&self) -> Box<dyn Iterator<Item = (&String, &String)> + '_> {
        match self {
            JavaStringMap::Sorted(entries) => Box::new(entries.iter().map(|(k, v)| (k, v))),
            JavaStringMap::Hash(map) => Box::new(map.iter()),
        }
    }

    /// `Map.get(Object)`.
    pub fn get(&self, key: &str) -> Option<&String> {
        match self {
            JavaStringMap::Sorted(entries) => entries
                .binary_search_by(|(k, _)| java_compare_to(k, key))
                .ok()
                .map(|index| &entries[index].1),
            JavaStringMap::Hash(map) => map.get(key),
        }
    }

    /// `Map.size()`.
    pub fn len(&self) -> usize {
        match self {
            JavaStringMap::Sorted(entries) => entries.len(),
            JavaStringMap::Hash(map) => map.len(),
        }
    }

    /// `Map.isEmpty()`.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The entries as a [`std::collections::HashMap`], for the callers that
    /// only look things up and never write the map out.
    pub fn to_std(&self) -> HashMap<String, String> {
        self.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
    }

    /// `new HashMap<>(this)`, which is what `putAttribute` does before it puts.
    pub fn to_java_hash_map(&self) -> JavaHashMap<String> {
        JavaHashMap::copy_of_entries(self.iter().map(|(k, v)| (k.clone(), v.clone())))
    }
}

impl JavaStringMap {
    /// `Map.copyOf(Map)`, as `SegmentInfo`'s constructor, `setDiagnostics` and
    /// `addDiagnostics` all use (`SegmentInfo.java:87`, `:137`, `:142`).
    ///
    /// **Divergence from Lucene 10.5.0, and an unavoidable one.**
    /// `Map.copyOf` returns a `java.util.ImmutableCollections.MapN`, whose
    /// iteration order is decided by a `SALT32L` drawn once per JVM at class
    /// initialisation. Two runs of *Lucene itself* therefore write the `.si`
    /// diagnostics of one identical segment in different byte orders — measured
    /// by `seal_bytes::lucenes_own_si_diagnostics_order_is_not_stable_between_jvms`
    /// — so there is no order here for a port to reproduce. This one sorts, the
    /// order a `TreeMap` would give and the order
    /// `DataInput.readMapOfStrings` gives the map back for two to ten entries,
    /// which makes Rucene's own output byte-stable. Nothing in the format
    /// depends on the order, so no reader can tell.
    pub fn immutable_copy_of<I>(entries: I) -> Self
    where
        I: IntoIterator<Item = (String, String)>,
    {
        let entries: Vec<(String, String)> = entries.into_iter().collect();
        let mut sorted = entries;
        sorted.sort_by(|(a, _), (b, _)| java_compare_to(a, b));
        sorted.dedup_by(|(later_key, later_value), (kept_key, kept_value)| {
            if later_key == kept_key {
                *kept_value = std::mem::take(later_value);
                true
            } else {
                false
            }
        });
        JavaStringMap::Sorted(sorted)
    }
}

impl JavaOrderedMap for JavaStringMap {
    fn entry_count(&self) -> usize {
        self.len()
    }
    fn entries(&self) -> Box<dyn Iterator<Item = (&String, &String)> + '_> {
        self.iter()
    }
}

impl fmt::Display for JavaStringMap {
    /// Renders the map the way `java.util.AbstractMap.toString()` does:
    /// `{key=value, key=value}`, in the map's own iteration order, and `{}`
    /// when it is empty.
    ///
    /// Every `java.util.Map` Lucene holds inherits that body — `HashMap`,
    /// `TreeMap` and the `Collections.unmodifiableMap` wrapper alike — so a
    /// `toString()` that appends a map, as `SegmentInfo.toStringVerbose()`
    /// appends its diagnostics and its attributes, prints exactly this.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("{")?;
        for (i, (key, value)) in self.iter().enumerate() {
            if i > 0 {
                f.write_str(", ")?;
            }
            write!(f, "{key}={value}")?;
        }
        f.write_str("}")
    }
}

impl From<JavaHashMap<String>> for JavaStringMap {
    fn from(map: JavaHashMap<String>) -> Self {
        JavaStringMap::Hash(map)
    }
}

/// `new HashMap<>()` then one `put` per item, wrapped as [`JavaStringMap::Hash`].
impl FromIterator<(String, String)> for JavaStringMap {
    fn from_iter<I: IntoIterator<Item = (String, String)>>(iter: I) -> Self {
        JavaStringMap::Hash(iter.into_iter().collect())
    }
}

/// `AbstractMap.equals`, which compares contents and ignores iteration order.
impl PartialEq for JavaStringMap {
    fn eq(&self, other: &Self) -> bool {
        self.len() == other.len()
            && self
                .iter()
                .all(|(k, v)| other.get(k).is_some_and(|other_v| other_v == v))
    }
}

impl Eq for JavaStringMap {}

impl fmt::Debug for JavaStringMap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_map().entries(self.iter()).finish()
    }
}

/// A `Set<String>` with the iteration order of whichever `java.util`
/// implementation Lucene is holding; the set counterpart of [`JavaStringMap`].
///
/// `DataInput.readSetOfStrings` dispatches the same way: empty, singleton,
/// `TreeSet` for two to ten, `HashSet` beyond (`DataInput.java:283-296`).
#[derive(Clone)]
pub enum JavaStringSet {
    /// Empty, singleton or `TreeSet`: all iterate in ascending
    /// [`java_compare_to`] order.
    Sorted(Vec<String>),
    /// A `java.util.HashSet`, in bucket order.
    Hash(JavaHashSet),
}

impl Default for JavaStringSet {
    fn default() -> Self {
        JavaStringSet::Sorted(Vec::new())
    }
}

impl JavaStringSet {
    /// The set `DataInput.readSetOfStrings` builds for `values` read in file
    /// order (`DataInput.java:283-296`).
    pub fn from_read(values: Vec<String>) -> Self {
        if values.len() > 10 {
            JavaStringSet::Hash(values.into_iter().collect())
        } else {
            let mut sorted = values;
            sorted.sort_by(|a, b| java_compare_to(a, b));
            sorted.dedup();
            JavaStringSet::Sorted(sorted)
        }
    }

    /// Iterates in the underlying implementation's order.
    pub fn iter(&self) -> Box<dyn Iterator<Item = &String> + '_> {
        match self {
            JavaStringSet::Sorted(values) => Box::new(values.iter()),
            JavaStringSet::Hash(set) => Box::new(set.iter()),
        }
    }

    /// `Set.contains(Object)`.
    pub fn contains(&self, value: &str) -> bool {
        match self {
            JavaStringSet::Sorted(values) => values
                .binary_search_by(|v| java_compare_to(v, value))
                .is_ok(),
            JavaStringSet::Hash(set) => set.contains(value),
        }
    }

    /// `Set.size()`.
    pub fn len(&self) -> usize {
        match self {
            JavaStringSet::Sorted(values) => values.len(),
            JavaStringSet::Hash(set) => set.len(),
        }
    }

    /// `Set.isEmpty()`.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The elements as a [`std::collections::HashSet`].
    pub fn to_std(&self) -> std::collections::HashSet<String> {
        self.iter().cloned().collect()
    }
}

impl JavaOrderedSet for JavaStringSet {
    fn value_count(&self) -> usize {
        self.len()
    }
    fn values(&self) -> Box<dyn Iterator<Item = &String> + '_> {
        self.iter()
    }
}

impl From<JavaHashSet> for JavaStringSet {
    fn from(set: JavaHashSet) -> Self {
        JavaStringSet::Hash(set)
    }
}

impl FromIterator<String> for JavaStringSet {
    fn from_iter<I: IntoIterator<Item = String>>(iter: I) -> Self {
        JavaStringSet::Hash(iter.into_iter().collect())
    }
}

impl IntoIterator for JavaStringSet {
    type Item = String;
    type IntoIter = std::vec::IntoIter<String>;

    fn into_iter(self) -> Self::IntoIter {
        match self {
            JavaStringSet::Sorted(values) => values.into_iter(),
            JavaStringSet::Hash(set) => set.into_iter().collect::<Vec<_>>().into_iter(),
        }
    }
}

impl<'a> IntoIterator for &'a JavaStringSet {
    type Item = &'a String;
    type IntoIter = Box<dyn Iterator<Item = &'a String> + 'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl PartialEq for JavaStringSet {
    fn eq(&self, other: &Self) -> bool {
        self.len() == other.len() && self.iter().all(|v| other.contains(v))
    }
}

impl Eq for JavaStringSet {}

impl fmt::Debug for JavaStringSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_set().entries(self.iter()).finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_size_for_matches_javas_edge_cases() {
        // `-1 >>> numberOfLeadingZeros(cap - 1)` with the shift masked to five
        // bits: 0 and 1 both leave the intermediate at -1.
        assert_eq!(table_size_for(0), 1);
        assert_eq!(table_size_for(1), 1);
        assert_eq!(table_size_for(2), 2);
        assert_eq!(table_size_for(3), 4);
        assert_eq!(table_size_for(4), 4);
        assert_eq!(table_size_for(5), 8);
        assert_eq!(table_size_for(1 << 29), 1 << 29);
        assert_eq!(table_size_for((1 << 29) + 1), 1 << 30);
        assert_eq!(table_size_for(i32::MAX), MAXIMUM_CAPACITY as i32);
    }

    #[test]
    fn string_hash_code_uses_utf16_code_units() {
        assert_eq!(java_string_hash_code(""), 0);
        assert_eq!(java_string_hash_code("a"), 97);
        // "Aa" and "BB" are the canonical `String.hashCode` collision.
        assert_eq!(java_string_hash_code("Aa"), java_string_hash_code("BB"));
        // U+1D400 is a surrogate pair: 0xD835, 0xDC00.
        assert_eq!(
            java_string_hash_code("\u{1D400}"),
            0xD835i32.wrapping_mul(31).wrapping_add(0xDC00)
        );
    }

    #[test]
    fn default_constructor_allocates_sixteen_on_first_put() {
        let mut map: JavaHashMap<String> = JavaHashMap::new();
        assert_eq!(map.table_length(), 0);
        map.insert("k".to_string(), "v".to_string());
        assert_eq!(map.table_length(), DEFAULT_INITIAL_CAPACITY);
    }

    #[test]
    fn copy_of_presizes_and_so_changes_the_table_length() {
        let two: JavaHashMap<String> = [
            ("PerFieldPostingsFormat.format", "Lucene104"),
            ("PerFieldPostingsFormat.suffix", "0"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
        assert_eq!(two.table_length(), 16);
        // `new HashMap<>(m)` with two entries pre-sizes to
        // `tableSizeFor((int)(2 / 0.75f + 1.0f))` == `tableSizeFor(3)` == 4.
        let copy = JavaHashMap::copy_of(&two);
        assert_eq!(copy.table_length(), 4);
        assert_eq!(copy, two);
    }

    #[test]
    fn resize_preserves_the_chain_order_within_a_bucket() {
        // "Aa" and "BB" have the same `String.hashCode`, so they share a bucket
        // at every table length; the one inserted first stays first.
        let mut map: JavaHashMap<i32> = JavaHashMap::new();
        map.insert("Aa".to_string(), 1);
        map.insert("BB".to_string(), 2);
        let before: Vec<&String> = map.keys().collect();
        assert_eq!(before, vec!["Aa", "BB"]);
        for i in 0..40 {
            map.insert(format!("filler{i}"), i);
        }
        assert!(map.table_length() > DEFAULT_INITIAL_CAPACITY);
        let after: Vec<String> = map
            .keys()
            .filter(|k| k.as_str() == "Aa" || k.as_str() == "BB")
            .cloned()
            .collect();
        assert_eq!(after, vec!["Aa".to_string(), "BB".to_string()]);
    }

    #[test]
    fn removing_and_reinserting_moves_the_key_to_the_tail_of_its_bucket() {
        let mut map: JavaHashMap<i32> = JavaHashMap::new();
        map.insert("Aa".to_string(), 1);
        map.insert("BB".to_string(), 2);
        assert_eq!(map.remove("Aa"), Some(1));
        map.insert("Aa".to_string(), 3);
        let keys: Vec<&String> = map.keys().collect();
        assert_eq!(keys, vec!["BB", "Aa"]);
    }

    #[test]
    fn set_is_its_backing_map() {
        let set: JavaHashSet = ["_9.fdt", "_9.fdx", "_9.nvd"]
            .into_iter()
            .map(str::to_string)
            .collect();
        assert_eq!(set.len(), 3);
        assert_eq!(set.table_length(), DEFAULT_INITIAL_CAPACITY);
        assert!(set.contains("_9.fdx"));
        assert!(!set.contains("_9.tim"));
    }
}
