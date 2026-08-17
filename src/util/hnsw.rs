//! HNSW graph utilities for approximate nearest-neighbor search.
//!
//! Equivalent to `org.apache.lucene.util.hnsw`.
//!
//! These types provide the in-memory graph representation and construction
//! primitives used by the KNN-vectors codecs (`Lucene99HnswVectorsFormat`).

#![deny(unsafe_code)]

use crate::error::Result;

/// Iterator over the node ordinals present on a given graph level.
///
/// Equivalent to `org.apache.lucene.util.hnsw.HnswGraph.NodesIterator`.
pub trait NodesIterator {
    /// Returns the number of nodes the iterator will yield.
    fn size(&self) -> i32;

    /// Returns the next node ordinal, or `NO_MORE_DOCS` when exhausted.
    fn next_int(&mut self) -> i32;

    /// Returns `true` while there are still nodes to consume.
    fn has_next(&self) -> bool;

    /// Copies as many remaining node ordinals as fit into `dest`.
    ///
    /// Returns the number of values written.
    fn consume(&mut self, dest: &mut [i32]) -> usize;
}

/// Node iterator that enumerates the contiguous range `[0, size)`.
///
/// Equivalent to `org.apache.lucene.util.hnsw.HnswGraph.DenseNodesIterator`.
#[derive(Clone, Debug)]
pub struct DenseNodesIterator {
    size: i32,
    cur: i32,
}

impl DenseNodesIterator {
    /// Creates an iterator over `[0, size)`.
    pub fn new(size: i32) -> Self {
        Self { size, cur: 0 }
    }
}

impl NodesIterator for DenseNodesIterator {
    fn size(&self) -> i32 {
        self.size
    }

    fn next_int(&mut self) -> i32 {
        if self.cur < self.size {
            let v = self.cur;
            self.cur += 1;
            v
        } else {
            crate::search::NO_MORE_DOCS
        }
    }

    fn has_next(&self) -> bool {
        self.cur < self.size
    }

    fn consume(&mut self, dest: &mut [i32]) -> usize {
        let remaining = (self.size - self.cur) as usize;
        let n = remaining.min(dest.len());
        for (i, slot) in dest.iter_mut().enumerate().take(n) {
            *slot = self.cur + i as i32;
        }
        self.cur += n as i32;
        n
    }
}

/// Node iterator backed by an arbitrary array of ordinals.
///
/// Equivalent to `org.apache.lucene.util.hnsw.HnswGraph.ArrayNodesIterator`.
#[derive(Clone, Debug)]
pub struct ArrayNodesIterator {
    nodes: Vec<i32>,
    size: i32,
    cur: i32,
}

impl ArrayNodesIterator {
    /// Creates an iterator over `nodes`.
    pub fn new(nodes: Vec<i32>) -> Self {
        let size = nodes.len() as i32;
        Self {
            nodes,
            size,
            cur: 0,
        }
    }
}

impl NodesIterator for ArrayNodesIterator {
    fn size(&self) -> i32 {
        self.size
    }

    fn next_int(&mut self) -> i32 {
        if self.cur < self.size {
            let v = self.nodes[self.cur as usize];
            self.cur += 1;
            v
        } else {
            crate::search::NO_MORE_DOCS
        }
    }

    fn has_next(&self) -> bool {
        self.cur < self.size
    }

    fn consume(&mut self, dest: &mut [i32]) -> usize {
        let remaining = (self.size - self.cur) as usize;
        let n = remaining.min(dest.len());
        let start = self.cur as usize;
        dest[..n].copy_from_slice(&self.nodes[start..start + n]);
        self.cur += n as i32;
        n
    }
}

/// Marker returned by `HnswGraph::next_neighbor` when a neighbor list is
/// exhausted.
pub const NO_MORE_DOCS: i32 = crate::search::NO_MORE_DOCS;

/// Sentinel value used when the maximum number of connections is unknown.
pub const UNKNOWN_MAX_CONN: i32 = -1;

/// Hierarchical Navigable Small World graph.
///
/// Equivalent to `org.apache.lucene.util.hnsw.HnswGraph`.
///
/// The graph may be searched concurrently, but updates are not thread-safe.
pub trait HnswGraph: Send + Sync {
    /// Positions the neighbor iterator on `target` at `level`.
    fn seek(&mut self, level: i32, target: i32) -> Result<()>;

    /// Returns the number of nodes in the graph.
    fn size(&self) -> i32;

    /// Returns the maximum node id, inclusive.
    fn max_node_id(&self) -> i32 {
        self.size() - 1
    }

    /// Returns the next neighbor ordinal, or `NO_MORE_DOCS`.
    fn next_neighbor(&mut self) -> Result<i32>;

    /// Returns the current number of levels.
    fn num_levels(&self) -> i32;

    /// Returns `M`, the maximum number of connections per node.
    fn max_conn(&self) -> i32;

    /// Returns the entry node on the top level.
    fn entry_node(&self) -> i32;

    /// Returns an iterator over the nodes on `level`.
    fn get_nodes_on_level(&self, level: i32) -> Result<Box<dyn NodesIterator>>;

    /// Returns the number of neighbors of the current target node.
    fn neighbor_count(&self) -> i32;
}
