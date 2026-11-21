//! In-memory HNSW graph representation.
//!
//! Equivalent to `org.apache.lucene.util.hnsw.OnHeapHnswGraph`.

#![deny(unsafe_code)]

use std::sync::atomic::{AtomicI32, AtomicI64, Ordering};
use std::sync::{Arc, OnceLock};

use crate::error::LuceneError;
use crate::util::{Accountable, RamUsageEstimator};

use super::neighbor::{NeighborArray, OnHeapMemoryUsageListener};
use super::{ArrayNodesIterator, DenseNodesIterator, HnswGraph, HnswLock, NodesIterator, Result};

const INIT_SIZE: usize = 128;

/// One `(level, node)` slot: Java's `graph[node][level]`, a `NeighborArray`
/// reference that starts `null` and is assigned by `addNode`.
type LevelSlot = OnceLock<NeighborArray>;

/// One node's levels: Java's `graph[node]`, `null` until the node is added.
type NodeSlot = OnceLock<Box<[LevelSlot]>>;

/// An in-memory HNSW graph where all nodes and connections are held on the
/// heap.
///
/// Equivalent to `org.apache.lucene.util.hnsw.OnHeapHnswGraph`.
///
/// Lucene lets the workers of `HnswConcurrentMergeBuilder` share one graph:
/// a worker writes the arrays of the node it inserts and publishes the node's
/// slot without any lock, locks a neighbour's array through the 512 striped
/// [`HnswLock`]s while it adds the back-link, and copies a neighbour list under
/// `HnswLock.read` while it searches (`HnswGraphBuilder.java:417-442`,
/// `OnHeapHnswGraph.java:150-157`, `HnswConcurrentMergeBuilder.java:249-263`).
/// The builders of this port take the same locks at the same points, and the
/// graph hands out its arrays as Java's does; a [`NeighborArray`] keeps its
/// storage in atomics, so a write Java makes without a lock is not a data race
/// (see that type).
///
/// # Divergence from Lucene 10.5.0: set-once slots
///
/// A node's slot, and each of its level slots, is a [`OnceLock`] set once, so
/// publishing a node to the other workers is a release store that their
/// acquire loads observe; Java's `graph[node] = ...` is a plain store, which
/// Rust cannot make through a shared graph. `size` and `maxNodeId` are
/// atomics, as in Lucene, and the entry node is one packed atomic word,
/// Lucene's `AtomicReference<EntryNode>`.
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

/// `RamUsageEstimator.shallowSizeOfInstance(OnHeapHnswGraph.class)`, Java's
/// `OnHeapHnswGraph.RAM_BYTES_USED` (`OnHeapHnswGraph.java:37-38`), on a 64-bit
/// JVM with compressed oops: a 12-byte header, seven references (`entryNode`,
/// `graph`, `levelToNodes`, `size`, `maxNodeId`, `cur`, `graphRamBytesUsed`),
/// four `int`s (`lastFreezeSize`, `nsize`, `nsize0`, `upto`) and one
/// `boolean` (`noGrowth`), aligned to 8. Measured against
/// `lucene-core-10.5.0.jar`: 64. The size of this Rust struct is not the
/// Java object's.
const ON_HEAP_HNSW_GRAPH_SHALLOW_SIZE: i64 = 64;

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
        // `OnHeapHnswGraph.RAM_BYTES_USED`, plus the outer graph array
        // (`OnHeapHnswGraph.java:37-38, 87`).
        let ram_bytes_used =
            ON_HEAP_HNSW_GRAPH_SHALLOW_SIZE + RamUsageEstimator::shallow_size_of(&graph);
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

    /// Returns the slot of `node` at `level`.
    ///
    /// Java's `getNeighbors` only asserts the node and the level
    /// (`OnHeapHnswGraph.java:96-101`); without assertions an absent slot is
    /// its `ArrayIndexOutOfBoundsException` or `NullPointerException`, here a
    /// panic.
    fn level_slot(&self, level: i32, node: i32) -> Result<&NeighborArray> {
        let node_usize = node as usize;
        debug_assert!(node_usize < self.graph.len());
        // `graph[node][level]`: the JVM's `ArrayIndexOutOfBoundsException` or
        // `NullPointerException`.
        let Some(cell) = usize::try_from(node).ok().and_then(|n| self.graph.get(n)) else {
            return Err(crate::store::array_index_out_of_bounds(
                i64::from(node),
                self.graph.len(),
            ));
        };
        let levels = cell.get().ok_or_else(LuceneError::null_pointer)?;
        debug_assert!(
            (level as usize) < levels.len(),
            "level={level}, node has only {} levels",
            levels.len()
        );
        let Some(level_cell) = usize::try_from(level).ok().and_then(|l| levels.get(l)) else {
            return Err(crate::store::array_index_out_of_bounds(
                i64::from(level),
                levels.len(),
            ));
        };
        let slot = level_cell.get();
        debug_assert!(slot.is_some(), "node={node}, level={level}");
        // A `null` `NeighborArray` fails where the caller dereferences it.
        slot.ok_or_else(LuceneError::null_pointer)
    }

    /// Returns the `NeighborArray` for `node` at `level`.
    ///
    /// Equivalent to `org.apache.lucene.util.hnsw.OnHeapHnswGraph#getNeighbors`,
    /// which takes no lock: a caller that shares the graph with other workers
    /// holds `HnswLock.read` or `HnswLock.write` for `(level, node)` around
    /// its use of the array, as Lucene's builders do; the array is written
    /// through `&self` (see [`NeighborArray`]).
    ///
    /// # Errors
    ///
    /// Never; the node and the level are asserted, as in Java, and an absent
    /// slot panics.
    pub fn get_neighbors(&self, level: i32, node: i32) -> Result<&NeighborArray> {
        self.level_slot(level, node)
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
    /// Throws Java's `IllegalStateException` (as a panic) when `node` is
    /// beyond the size the graph was created with.
    pub fn add_node(&mut self, level: i32, node: i32) {
        let node_usize = node as usize;
        if node_usize >= self.graph.len() {
            if self.no_growth {
                // `throw new IllegalStateException(...)` (`OnHeapHnswGraph.java:140-144`).
                crate::store::java_nio_file::throw_unchecked(
                    "java.lang.IllegalStateException",
                    Some("The graph does not expect to grow when an initial size is given"),
                );
            }
            // `ArrayUtil.grow(graph, node + 1)` (`ArrayUtil.java:231-237`) sizes
            // the new array with `oversize(minSize, NUM_BYTES_OBJECT_REF)`.
            let new_len = crate::util::ArrayUtil::oversize(
                node_usize + 1,
                RamUsageEstimator::NUM_BYTES_OBJECT_REF as usize,
            );
            self.graph.resize_with(new_len, OnceLock::new);
        }

        debug_assert!(
            self.graph[node_usize]
                .get()
                .is_none_or(|levels| levels.len() >= level as usize),
            "node must be inserted from the top level: "
        );
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
        levels[level as usize] = OnceLock::from(array);
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
    /// Returns [`LuceneErrorKind::IllegalState`](crate::error::LuceneErrorKind::IllegalState) where [`add_node`](Self::add_node)
    /// would have to grow storage the other workers may be reading — the outer
    /// array, or the node's level array — and where the level was already
    /// added, which Java would overwrite under the other workers' feet.
    ///
    /// **Divergence from Lucene 10.5.0.** Java's `addNode` grows or overwrites
    /// in those cases; the slots here are published once to the concurrent
    /// workers, so they are refused instead.
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
            self.new_neighbor_array(level)
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

    /// Returns `graph.length`, the capacity of the outer array.
    ///
    /// Equivalent to reading the private field `OnHeapHnswGraph.graph`
    /// (`OnHeapHnswGraph.java:47`); public so that the test framework can
    /// model the Java object.
    pub fn graph_length(&self) -> usize {
        self.graph.len()
    }

    /// Returns `graph[node]`: `None` for Java's `null`, else the node's level
    /// slots, an unset slot standing for a `null` `graph[node][level]`.
    ///
    /// Equivalent to reading the private field `OnHeapHnswGraph.graph`
    /// (`OnHeapHnswGraph.java:47`); public so that the test framework can
    /// model the Java object.
    pub fn graph_node(&self, node: usize) -> Option<&[OnceLock<NeighborArray>]> {
        self.graph
            .get(node)
            .and_then(|slot| slot.get())
            .map(|levels| &**levels)
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
        let neighbors = self.get_neighbors(level, target)?.clone();
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
            // `cur.size()` on a `null` `cur`: the JVM's `NullPointerException`.
            .ok_or_else(LuceneError::null_pointer)?;
        self.upto += 1;
        if self.upto < cur.size() {
            Ok(cur.node(self.upto))
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
            // `levelToNodes[level]`: the JVM's `ArrayIndexOutOfBoundsException`.
            if level < 0 || level >= num_levels {
                return Err(crate::store::array_index_out_of_bounds(
                    i64::from(level),
                    num_levels as usize,
                ));
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
/// list out, under its stripe of the [`HnswLock`] when it has one.
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
/// builder with no lock — Java's `hnswLock == null`, whose searcher seeks
/// with `OnHeapHnswGraph.seek` — reads the array without one; Java walks the
/// array itself rather than a copy, and both yield the same neighbours in the
/// same order, since nothing writes the graph while that build searches it.
pub(crate) struct OnHeapGraphReader<'a> {
    graph: &'a OnHeapHnswGraph,
    /// `MergeSearcher.hnswLock`, `None` for a builder without one.
    hnsw_lock: Option<&'a HnswLock>,
    /// Java's `nodeBuffer`, owned by the caller so that it is reused across
    /// searches as Java reuses the searcher's.
    node_buffer: &'a mut Vec<i32>,
    upto: i32,
    size: i32,
}

impl<'a> OnHeapGraphReader<'a> {
    /// Creates a cursor over `graph` that copies into `node_buffer`, under
    /// `hnsw_lock`'s read stripes when it is given.
    pub(crate) fn new(
        graph: &'a OnHeapHnswGraph,
        hnsw_lock: Option<&'a HnswLock>,
        node_buffer: &'a mut Vec<i32>,
    ) -> Self {
        Self {
            graph,
            hnsw_lock,
            node_buffer,
            upto: -1,
            size: 0,
        }
    }
}

impl HnswGraph for OnHeapGraphReader<'_> {
    /// Equivalent to `MergeSearcher.graphSeek`: `Lock lock =
    /// hnswLock.read(level, targetNode); try { ... size =
    /// neighborArray.size(); System.arraycopy(neighborArray.nodes(), 0,
    /// nodeBuffer, 0, size); } finally { lock.unlock(); }`.
    fn seek(&mut self, level: i32, target: i32) -> Result<()> {
        {
            let _lock = self.hnsw_lock.map(|lock| lock.read(level, target));
            let neighbors = self.graph.get_neighbors(level, target)?;
            let size = neighbors.size();
            self.node_buffer.resize(size as usize, 0);
            neighbors.copy_nodes(0, &mut self.node_buffer[..]);
            self.size = size;
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
