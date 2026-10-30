//! In-memory HNSW graph representation.
//!
//! Equivalent to `org.apache.lucene.util.hnsw.OnHeapHnswGraph`.

#![deny(unsafe_code)]

use std::sync::atomic::{AtomicI32, AtomicI64, Ordering};
use std::sync::{Arc, OnceLock, RwLock, RwLockReadGuard, RwLockWriteGuard};

use crate::error::LuceneError;
use crate::util::{Accountable, RamUsageEstimator};

use super::neighbor::{NeighborArray, OnHeapMemoryUsageListener};
use super::{ArrayNodesIterator, DenseNodesIterator, HnswGraph, NodesIterator, Result};

const INIT_SIZE: usize = 128;

/// One `(level, node)` slot: Java's `graph[node][level]`, a `NeighborArray`
/// reference that starts `null` and is assigned by `addNode`.
type LevelSlot = OnceLock<RwLock<NeighborArray>>;

/// One node's levels: Java's `graph[node]`, `null` until the node is added.
type NodeSlot = OnceLock<Box<[LevelSlot]>>;

/// An in-memory HNSW graph where all nodes and connections are held on the
/// heap.
///
/// Equivalent to `org.apache.lucene.util.hnsw.OnHeapHnswGraph`.
///
/// # Divergence from Lucene 10.5.0: one lock per neighbour array
///
/// Lucene lets the workers of `HnswConcurrentMergeBuilder` share one graph:
/// a worker writes the arrays of the node it inserts and publishes the node's
/// slot without any lock, locks a neighbour's array through the 512 striped
/// `HnswLock`s while it adds the back-link, and copies a neighbour list under
/// `HnswLock.read` while it searches (`HnswGraphBuilder.java:417-442`,
/// `OnHeapHnswGraph.java:150-157`, `HnswConcurrentMergeBuilder.java:249-263`).
/// The unlocked writes are a data race in Rust, so this port makes the graph
/// itself thread-safe instead:
///
/// * every `NeighborArray` sits behind its own [`RwLock`], so the readers and
///   writers of one `(level, node)` exclude each other, the new node's own
///   array included, where Lucene takes no lock for it;
/// * a node's slot, and each of its level slots, is a [`OnceLock`] set once,
///   so publishing a node to the other workers is a release store that their
///   acquire loads observe; Java's `graph[node] = ...` is a plain store;
/// * `size` and `maxNodeId` are atomics, as in Lucene, and the entry node is
///   one packed atomic word, Lucene's `AtomicReference<EntryNode>`.
///
/// The striped [`HnswLock`](super::HnswLock) therefore guards nothing in this
/// port's build. Only locking differs: the same arrays are read and written in
/// the same order, so a single-threaded build produces the same graph.
///
/// Growing the outer array, or a node's level array, needs exclusive access
/// ([`add_node`](Self::add_node)); Lucene's concurrent merge never grows
/// either, because it sizes the graph up front and inserts each node from its
/// top level down, and the shared insertion path
/// ([`add_node_shared`](Self::add_node_shared)) refuses to.
#[derive(Debug)]
pub struct OnHeapHnswGraph {
    entry_node: AtomicEntryNode,
    graph: Vec<NodeSlot>,
    size: AtomicI32,
    max_node_id: AtomicI32,
    nsize: i32,
    nsize0: i32,
    no_growth: bool,
    // cursor state for the `HnswGraph` trait (not thread-safe)
    upto: i32,
    cur: Option<NeighborArray>,
    /// Running total of the on-heap memory the graph has taken.
    ///
    /// Equivalent to Lucene's `LongAdder graphRamBytesUsed`; every
    /// [`NeighborArray`] this graph creates is handed a listener that adds to
    /// it, so it is shared and must be atomic.
    graph_ram_bytes_used: Arc<AtomicI64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct EntryNode {
    node: i32,
    level: i32,
}

impl EntryNode {
    fn pack(self) -> i64 {
        (i64::from(self.node) << 32) | i64::from(self.level as u32)
    }

    fn unpack(word: i64) -> Self {
        Self {
            node: (word >> 32) as i32,
            level: word as i32,
        }
    }
}

/// Lucene's `AtomicReference<EntryNode>`: the node and its level change
/// together, in one compare-and-set, so a reader never sees the node of one
/// entry with the level of another.
#[derive(Debug)]
struct AtomicEntryNode(AtomicI64);

impl AtomicEntryNode {
    fn new(node: i32, level: i32) -> Self {
        Self(AtomicI64::new(EntryNode { node, level }.pack()))
    }

    fn get(&self) -> EntryNode {
        // Acquire pairs with the release of `compare_and_set`, as Java's
        // volatile read pairs with its volatile write.
        EntryNode::unpack(self.0.load(Ordering::Acquire))
    }

    fn compare_and_set(&self, expected: EntryNode, new: EntryNode) -> bool {
        self.0
            .compare_exchange(
                expected.pack(),
                new.pack(),
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok()
    }
}

/// The error a poisoned neighbour-array lock turns into.
///
/// A lock is poisoned only when a thread panicked while it held the write
/// guard, which may have left the array half updated; Java has no such state.
fn poisoned(level: i32, node: i32) -> LuceneError {
    LuceneError::illegal_state(format!(
        "the neighbour array of node {node} at level {level} was poisoned by a panicking writer"
    ))
}

impl OnHeapHnswGraph {
    /// Creates an empty graph with the given `M`. `num_nodes` of `-1` means
    /// unbounded growth; a non-negative value fixes the eventual size.
    pub fn new(m: i32, num_nodes: i32) -> Self {
        let no_growth = num_nodes != -1;
        let num_nodes = if no_growth {
            num_nodes
        } else {
            INIT_SIZE as i32
        };
        let graph: Vec<NodeSlot> = (0..num_nodes.max(0) as usize)
            .map(|_| OnceLock::new())
            .collect();
        // Shallow estimate of the statically used on-heap memory, equivalent to
        // `OnHeapHnswGraph.RAM_BYTES_USED`, plus the outer graph array.
        let ram_bytes_used = RamUsageEstimator::shallow_size_of_instance::<Self>()
            + RamUsageEstimator::shallow_size_of(&graph);
        Self {
            entry_node: AtomicEntryNode::new(-1, 1),
            graph,
            size: AtomicI32::new(0),
            max_node_id: AtomicI32::new(-1),
            nsize: m + 1,
            nsize0: m * 2 + 1,
            no_growth,
            upto: -1,
            cur: None,
            graph_ram_bytes_used: Arc::new(AtomicI64::new(ram_bytes_used)),
        }
    }

    /// Returns the slot of `node` at `level`, or the error Java's assertions
    /// in `getNeighbors` stand for.
    fn level_slot(&self, level: i32, node: i32) -> Result<&RwLock<NeighborArray>> {
        let node_usize = node as usize;
        if node < 0 || node_usize >= self.graph.len() {
            return Err(LuceneError::IllegalArgument(format!(
                "node {} out of bounds (graph length {})",
                node,
                self.graph.len()
            )));
        }
        let levels = self.graph[node_usize]
            .get()
            .ok_or_else(|| LuceneError::IllegalArgument(format!("node {} has no levels", node)))?;
        let level_usize = level as usize;
        if level < 0 || level_usize >= levels.len() {
            return Err(LuceneError::IllegalArgument(format!(
                "level {} out of bounds for node {} (has {} levels)",
                level,
                node,
                levels.len()
            )));
        }
        levels[level_usize].get().ok_or_else(|| {
            LuceneError::IllegalArgument(format!(
                "node {} has no neighbor array at level {}",
                node, level
            ))
        })
    }

    /// Returns the `NeighborArray` for `node` at `level`, read-locked.
    ///
    /// Equivalent to `OnHeapHnswGraph#getNeighbors` read through
    /// `HnswLock.read(level, node)`, which is how Lucene's concurrent merge
    /// reads a neighbour list another worker may be writing; see the type
    /// docs for why every read takes the array's own lock here. Hold the
    /// guard briefly: a writer of the same array waits for it.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when the node or the level is
    /// out of range or not yet added, and [`LuceneError::IllegalState`] when a
    /// panicking writer poisoned the array's lock.
    pub fn get_neighbors(
        &self,
        level: i32,
        node: i32,
    ) -> Result<RwLockReadGuard<'_, NeighborArray>> {
        self.level_slot(level, node)?
            .read()
            .map_err(|_| poisoned(level, node))
    }

    /// Returns the `NeighborArray` for `node` at `level`, write-locked for
    /// modification.
    ///
    /// Java hands out the array itself from `getNeighbors` and its writers
    /// mutate it in place, under `HnswLock.write(level, node)` when another
    /// worker may read it; this is that pairing, through the array's own lock.
    ///
    /// # Errors
    ///
    /// As [`get_neighbors`](Self::get_neighbors).
    pub fn write_neighbors(
        &self,
        level: i32,
        node: i32,
    ) -> Result<RwLockWriteGuard<'_, NeighborArray>> {
        self.level_slot(level, node)?
            .write()
            .map_err(|_| poisoned(level, node))
    }

    /// Returns the `NeighborArray` for `node` at `level`, for modification
    /// through exclusive access, without locking.
    ///
    /// # Errors
    ///
    /// As [`get_neighbors`](Self::get_neighbors).
    pub fn get_neighbors_mut(&mut self, level: i32, node: i32) -> Result<&mut NeighborArray> {
        let node_usize = node as usize;
        let graph_len = self.graph.len();
        if node < 0 || node_usize >= graph_len {
            return Err(LuceneError::IllegalArgument(format!(
                "node {node} out of bounds (graph length {graph_len})"
            )));
        }
        let levels = self.graph[node_usize]
            .get_mut()
            .ok_or_else(|| LuceneError::IllegalArgument(format!("node {node} has no levels")))?;
        let level_usize = level as usize;
        let num_levels = levels.len();
        if level < 0 || level_usize >= num_levels {
            return Err(LuceneError::IllegalArgument(format!(
                "level {level} out of bounds for node {node} (has {num_levels} levels)"
            )));
        }
        levels[level_usize]
            .get_mut()
            .ok_or_else(|| {
                LuceneError::IllegalArgument(format!(
                    "node {node} has no neighbor array at level {level}"
                ))
            })?
            .get_mut()
            .map_err(|_| poisoned(level, node))
    }

    /// Replaces the neighbor array for `node` at `level`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when the node is out of range
    /// or has no level `level`.
    pub fn set_neighbors(&mut self, level: i32, node: i32, neighbors: NeighborArray) -> Result<()> {
        let node_usize = node as usize;
        if node < 0 || node_usize >= self.graph.len() {
            return Err(LuceneError::IllegalArgument(format!(
                "node {} out of bounds",
                node
            )));
        }
        let levels = self.graph[node_usize]
            .get_mut()
            .ok_or_else(|| LuceneError::IllegalArgument(format!("node {} has no levels", node)))?;
        let level_usize = level as usize;
        if level < 0 || level_usize >= levels.len() {
            return Err(LuceneError::IllegalArgument(format!(
                "level {} out of bounds for node {}",
                level, node
            )));
        }
        levels[level_usize] = OnceLock::from(RwLock::new(neighbors));
        Ok(())
    }

    /// Creates the empty neighbour array `addNode` installs at `level`,
    /// charging its memory to the graph.
    fn new_neighbor_array(&self, level: i32) -> NeighborArray {
        let counter = Arc::clone(&self.graph_ram_bytes_used);
        NeighborArray::with_listener(
            if level == 0 { self.nsize0 } else { self.nsize },
            true,
            Some(OnHeapMemoryUsageListener::new(Arc::new(move |l| {
                debug_assert!(l > 0);
                counter.fetch_add(l, Ordering::Relaxed);
            }))),
        )
    }

    /// Adds a node on the given level. Nodes must be added from their top
    /// level downward.
    ///
    /// Equivalent to `org.apache.lucene.util.hnsw.OnHeapHnswGraph#addNode`,
    /// including growing the outer array (or panicking, where Java throws
    /// `IllegalStateException`, when the graph was given its size) and growing
    /// a node's level array exactly. Both growths replace storage the other
    /// workers of a concurrent merge would be reading, so this takes the graph
    /// exclusively; a worker uses [`add_node_shared`](Self::add_node_shared).
    ///
    /// # Panics
    ///
    /// Panics when `node` is beyond the size the graph was created with.
    pub fn add_node(&mut self, level: i32, node: i32) {
        let node_usize = node as usize;
        if node_usize >= self.graph.len() {
            if self.no_growth {
                panic!("The graph does not expect to grow when an initial size is given");
            }
            let new_len =
                crate::util::ArrayUtil::oversize(node_usize + 1, std::mem::size_of::<usize>())
                    .max(node_usize + 1);
            self.graph.resize_with(new_len, OnceLock::new);
        }

        let array = self.new_neighbor_array(level);
        let target_len = (level + 1) as usize;
        let slot = &mut self.graph[node_usize];
        if slot.get().is_none() {
            let levels: Box<[LevelSlot]> = (0..target_len).map(|_| OnceLock::new()).collect();
            let _ = slot.set(levels);
            self.size.fetch_add(1, Ordering::Relaxed);
        }
        let levels = slot
            .get_mut()
            .expect("INVARIANT: the node's level array was installed above");
        if levels.len() < target_len {
            // `ArrayUtil.growExact(graph[node], level + 1)`.
            let mut grown = std::mem::take(levels).into_vec();
            grown.resize_with(target_len, OnceLock::new);
            *levels = grown.into_boxed_slice();
        }
        levels[level as usize] = OnceLock::from(RwLock::new(array));
        self.max_node_id.fetch_max(node, Ordering::Relaxed);
    }

    /// Adds a node on the given level while other workers share the graph.
    ///
    /// This is `OnHeapHnswGraph#addNode` as `HnswConcurrentMergeBuilder`'s
    /// workers call it: on a graph sized up front, for a node no other worker
    /// inserts, from its top level downward. The node's slot and the level
    /// slot are each published once, so a worker that reaches the node
    /// through a neighbour list sees it complete.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] where [`add_node`](Self::add_node)
    /// would have to grow storage the other workers may be reading — the outer
    /// array, or the node's level array — and where the level was already
    /// added, which Java would overwrite under the other workers' feet.
    pub fn add_node_shared(&self, level: i32, node: i32) -> Result<()> {
        let node_usize = node as usize;
        if node < 0 || node_usize >= self.graph.len() {
            return Err(LuceneError::illegal_state(if self.no_growth {
                "The graph does not expect to grow when an initial size is given".to_string()
            } else {
                format!("node {node} is beyond a shared graph, which cannot grow")
            }));
        }
        let target_len = (level + 1) as usize;
        let levels = self.graph[node_usize].get_or_init(|| {
            self.size.fetch_add(1, Ordering::Relaxed);
            (0..target_len).map(|_| OnceLock::new()).collect()
        });
        if levels.len() < target_len {
            return Err(LuceneError::illegal_state(format!(
                "node {node} must be inserted from its top level: it has {} levels, not {target_len}",
                levels.len()
            )));
        }
        // The array is created, and its memory charged, only by the call that
        // installs it; a slot that is already set is refused untouched.
        let mut installed = false;
        levels[level as usize].get_or_init(|| {
            installed = true;
            RwLock::new(self.new_neighbor_array(level))
        });
        if !installed {
            return Err(LuceneError::illegal_state(format!(
                "node {node} was already added at level {level}"
            )));
        }
        self.max_node_id.fetch_max(node, Ordering::Relaxed);
        Ok(())
    }

    /// Attempts to set the entry node when the graph is empty.
    pub fn try_set_new_entry_node(&self, node: i32, level: i32) -> bool {
        let current = self.entry_node.get();
        if current.node == -1 {
            return self
                .entry_node
                .compare_and_set(current, EntryNode { node, level });
        }
        false
    }

    /// Attempts to promote `node` to the entry node.
    pub fn try_promote_new_entry_node(
        &self,
        node: i32,
        level: i32,
        expected_old_level: i32,
    ) -> bool {
        debug_assert!(level > expected_old_level);
        let current = self.entry_node.get();
        if current.level == expected_old_level {
            return self
                .entry_node
                .compare_and_set(current, EntryNode { node, level });
        }
        false
    }

    /// Returns whether `node` exists at `level`.
    ///
    /// Equivalent to `org.apache.lucene.util.hnsw.OnHeapHnswGraph#nodeExistAtLevel`,
    /// which asks only whether the node's level array reaches `level`: a node
    /// is inserted from its top level downward, so the slots below it are still
    /// empty while the insertion is in progress and must nonetheless count as
    /// existing.
    pub fn node_exists_at_level(&self, level: i32, node: i32) -> bool {
        let node_usize = node as usize;
        if node < 0 || node_usize >= self.graph.len() {
            return false;
        }
        match self.graph[node_usize].get() {
            None => false,
            Some(levels) => (level as usize) < levels.len(),
        }
    }

    /// Returns the number of levels currently in the graph.
    fn num_levels_internal(&self) -> i32 {
        self.entry_node.get().level + 1
    }

    /// Returns the number of nodes, Java's `size()`.
    ///
    /// `Relaxed` suffices for the counter itself, as it does for
    /// `max_node_id`: a reader that needs the final count reads it after the
    /// workers have been joined, and the join orders their increments first.
    fn size_internal(&self) -> i32 {
        self.size.load(Ordering::Relaxed)
    }

    /// Builds the level-to-nodes mapping used by `get_nodes_on_level`.
    ///
    /// Equivalent to the private
    /// `org.apache.lucene.util.hnsw.OnHeapHnswGraph#generateLevelToNodes`.
    ///
    /// **Divergence from Lucene 10.5.0.** Java memoises the mapping in the
    /// `levelToNodes` field and skips the walk when `lastFreezeSize` still
    /// equals `size()` (`OnHeapHnswGraph.java:273-296`). `getNodesOnLevel` is
    /// `&self` here — the trait method it implements takes a shared borrow —
    /// so the mapping is rebuilt and returned instead of cached. Only the cost
    /// differs: the walk is the same and produces the same lists, and
    /// `getNodesOnLevel` already refuses to run before the build is complete,
    /// so the mapping it sees cannot change between calls.
    fn build_level_to_nodes(&self) -> Vec<Vec<i32>> {
        let num_levels = self.num_levels_internal();
        let size = self.size_internal();
        let mut level_to_nodes: Vec<Vec<i32>> = (0..num_levels).map(|_| Vec::new()).collect();
        let mut non_null_count = 0i32;
        for (node, levels) in self.graph.iter().enumerate() {
            let Some(levels) = levels.get() else {
                continue;
            };
            non_null_count += 1;
            for i in 1..levels.len() {
                if levels[i].get().is_some() {
                    level_to_nodes[i].push(node as i32);
                }
            }
            if non_null_count == size {
                break;
            }
        }
        level_to_nodes
    }
}

impl Accountable for OnHeapHnswGraph {
    /// Provides an estimate of the current on-heap memory usage of the graph.
    ///
    /// This is not thread-safe, meaning the heap utilization while building the
    /// graph concurrently may be inaccurate. Its main purpose is during initial
    /// document indexing and flush.
    ///
    /// Equivalent to `org.apache.lucene.util.hnsw.OnHeapHnswGraph#ramBytesUsed`.
    fn ram_bytes_used(&self) -> i64 {
        self.graph_ram_bytes_used.load(Ordering::Relaxed)
    }
}

impl HnswGraph for OnHeapHnswGraph {
    fn seek(&mut self, level: i32, target: i32) -> Result<()> {
        let neighbors = self.get_neighbors_mut(level, target)?.clone();
        self.cur = Some(neighbors);
        self.upto = -1;
        Ok(())
    }

    fn size(&self) -> i32 {
        self.size_internal()
    }

    fn max_node_id(&self) -> i32 {
        if self.no_growth {
            self.graph.len() as i32 - 1
        } else {
            self.max_node_id.load(Ordering::Relaxed)
        }
    }

    fn next_neighbor(&mut self) -> Result<i32> {
        let cur = self
            .cur
            .as_ref()
            .ok_or_else(|| LuceneError::illegal_state("seek not called".to_string()))?;
        self.upto += 1;
        if self.upto < cur.size() {
            Ok(cur.nodes()[self.upto as usize])
        } else {
            Ok(super::NO_MORE_DOCS)
        }
    }

    fn num_levels(&self) -> Result<i32> {
        Ok(self.num_levels_internal())
    }

    fn max_conn(&self) -> i32 {
        self.nsize - 1
    }

    fn entry_node(&self) -> Result<i32> {
        Ok(self.entry_node.get().node)
    }

    fn get_nodes_on_level(&self, level: i32) -> Result<Box<dyn NodesIterator>> {
        let size = self.size_internal();
        if size != self.max_node_id() + 1 {
            return Err(LuceneError::illegal_state(format!(
                "graph build not complete, size={} max_node_id={}",
                size,
                self.max_node_id()
            )));
        }
        if level == 0 {
            Ok(Box::new(DenseNodesIterator::new(size)))
        } else {
            let num_levels = self.num_levels()?;
            if level < 0 || level >= num_levels {
                return Err(LuceneError::IllegalArgument(format!(
                    "level {} out of bounds (num_levels={})",
                    level, num_levels
                )));
            }
            let level_to_nodes = self.build_level_to_nodes();
            Ok(Box::new(ArrayNodesIterator::new(
                level_to_nodes[level as usize].clone(),
            )))
        }
    }

    fn neighbor_count(&self) -> i32 {
        self.cur.as_ref().map_or(0, |c| c.size())
    }
}

/// A cursor over a shared [`OnHeapHnswGraph`] that copies each neighbour
/// list out under its read lock.
///
/// Equivalent to the private
/// `org.apache.lucene.util.hnsw.HnswConcurrentMergeBuilder.MergeSearcher`,
/// whose `graphSeek` takes `HnswLock.read(level, targetNode)`, copies the
/// node's `NeighborArray` into a reusable `nodeBuffer` and releases the lock,
/// so that another worker can update the array while this one walks the copy
/// (`HnswConcurrentMergeBuilder.java:238-272`). Java overrides the searcher's
/// seek; Rust's [`HnswGraphSearcher`](super::HnswGraphSearcher) reads through
/// [`HnswGraph`], so the override becomes this view, and the searcher stays
/// the plain one.
///
/// Every builder in this port reads its graph through this view, because the
/// graph is shared behind an [`Arc`] and its own cursor needs `&mut`. A
/// single-threaded build reads Java's `OnHeapHnswGraph.seek` reference
/// instead of a copy; both yield the same neighbours in the same order, since
/// nothing writes the graph while that build searches it.
pub(crate) struct OnHeapGraphReader<'a> {
    graph: &'a OnHeapHnswGraph,
    /// Java's `nodeBuffer`, owned by the caller so that it is reused across
    /// searches as Java reuses the searcher's.
    node_buffer: &'a mut Vec<i32>,
    upto: i32,
    size: i32,
}

impl<'a> OnHeapGraphReader<'a> {
    /// Creates a cursor over `graph` that copies into `node_buffer`.
    pub(crate) fn new(graph: &'a OnHeapHnswGraph, node_buffer: &'a mut Vec<i32>) -> Self {
        Self {
            graph,
            node_buffer,
            upto: -1,
            size: 0,
        }
    }
}

impl HnswGraph for OnHeapGraphReader<'_> {
    /// Equivalent to `MergeSearcher.graphSeek`.
    fn seek(&mut self, level: i32, target: i32) -> Result<()> {
        {
            let neighbors = self.graph.get_neighbors(level, target)?;
            self.node_buffer.clear();
            self.node_buffer.extend_from_slice(neighbors.nodes());
            self.size = neighbors.size();
        }
        self.upto = -1;
        Ok(())
    }

    fn size(&self) -> i32 {
        HnswGraph::size(self.graph)
    }

    fn max_node_id(&self) -> i32 {
        HnswGraph::max_node_id(self.graph)
    }

    /// Equivalent to `MergeSearcher.graphNextNeighbor`.
    fn next_neighbor(&mut self) -> Result<i32> {
        self.upto += 1;
        if self.upto < self.size {
            Ok(self.node_buffer[self.upto as usize])
        } else {
            Ok(super::NO_MORE_DOCS)
        }
    }

    fn num_levels(&self) -> Result<i32> {
        HnswGraph::num_levels(self.graph)
    }

    fn max_conn(&self) -> i32 {
        HnswGraph::max_conn(self.graph)
    }

    fn entry_node(&self) -> Result<i32> {
        HnswGraph::entry_node(self.graph)
    }

    fn get_nodes_on_level(&self, level: i32) -> Result<Box<dyn NodesIterator>> {
        HnswGraph::get_nodes_on_level(self.graph, level)
    }

    fn neighbor_count(&self) -> i32 {
        self.size
    }
}

/// An empty HNSW graph.
///
/// Equivalent to `org.apache.lucene.util.hnsw.HnswGraph.EMPTY`.
#[derive(Debug, Copy, Clone)]
pub struct EmptyHnswGraph;

impl HnswGraph for EmptyHnswGraph {
    fn seek(&mut self, _level: i32, _target: i32) -> Result<()> {
        Ok(())
    }

    fn size(&self) -> i32 {
        0
    }

    fn next_neighbor(&mut self) -> Result<i32> {
        Ok(super::NO_MORE_DOCS)
    }

    fn num_levels(&self) -> Result<i32> {
        Ok(0)
    }

    fn max_conn(&self) -> i32 {
        super::UNKNOWN_MAX_CONN
    }

    fn entry_node(&self) -> Result<i32> {
        Ok(0)
    }

    fn get_nodes_on_level(&self, _level: i32) -> Result<Box<dyn NodesIterator>> {
        Ok(Box::new(DenseNodesIterator::new(0)))
    }

    fn neighbor_count(&self) -> i32 {
        0
    }
}

// -----------------------------------------------------------------------------
// Object contract: `OnHeapHnswGraph.toString()`
// -----------------------------------------------------------------------------

impl std::fmt::Display for OnHeapHnswGraph {
    /// Port of `OnHeapHnswGraph.toString()`.
    ///
    /// Java calls `numLevels()` and `entryNode()`, whose `OnHeapHnswGraph`
    /// overrides cannot fail; this reads the same two values through the
    /// infallible internals so that `Display` needs no fallback.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "OnHeapHnswGraph(size={}, numLevels={}, entryNode={})",
            self.size_internal(),
            self.num_levels_internal(),
            self.entry_node.get().node
        )
    }
}
