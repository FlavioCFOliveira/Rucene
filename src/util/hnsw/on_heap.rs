//! In-memory HNSW graph representation.
//!
//! Equivalent to `org.apache.lucene.util.hnsw.OnHeapHnswGraph`.

#![deny(unsafe_code)]

use std::sync::atomic::{AtomicI32, Ordering};

use crate::error::LuceneError;

use super::neighbor::NeighborArray;
use super::{ArrayNodesIterator, DenseNodesIterator, HnswGraph, NodesIterator, Result};

const INIT_SIZE: usize = 128;

/// An in-memory HNSW graph where all nodes and connections are held on the
/// heap.
///
/// Equivalent to `org.apache.lucene.util.hnsw.OnHeapHnswGraph`.
#[derive(Debug)]
pub struct OnHeapHnswGraph {
    entry_node: AtomicEntryNode,
    graph: Vec<Option<Vec<Option<NeighborArray>>>>,
    size: i32,
    max_node_id: i32,
    nsize: i32,
    nsize0: i32,
    no_growth: bool,
    // cursor state for the `HnswGraph` trait (not thread-safe)
    upto: i32,
    cur: Option<NeighborArray>,
}

#[derive(Debug, Clone, Copy)]
struct EntryNode {
    node: i32,
    level: i32,
}

#[derive(Debug)]
struct AtomicEntryNode {
    node: AtomicI32,
    level: AtomicI32,
}

impl AtomicEntryNode {
    fn new(node: i32, level: i32) -> Self {
        Self {
            node: AtomicI32::new(node),
            level: AtomicI32::new(level),
        }
    }

    fn get(&self) -> EntryNode {
        EntryNode {
            node: self.node.load(Ordering::Acquire),
            level: self.level.load(Ordering::Acquire),
        }
    }

    fn compare_and_set(&self, expected: EntryNode, new: EntryNode) -> bool {
        self.node
            .compare_exchange(expected.node, new.node, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
            && self
                .level
                .compare_exchange(
                    expected.level,
                    new.level,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                )
                .is_ok()
    }
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
        Self {
            entry_node: AtomicEntryNode::new(-1, 1),
            graph: (0..num_nodes.max(0) as usize).map(|_| None).collect(),
            size: 0,
            max_node_id: -1,
            nsize: m + 1,
            nsize0: m * 2 + 1,
            no_growth,
            upto: -1,
            cur: None,
        }
    }

    /// Returns the `NeighborArray` for `node` at `level`.
    pub fn get_neighbors(&self, level: i32, node: i32) -> Result<&NeighborArray> {
        let node_usize = node as usize;
        if node_usize >= self.graph.len() {
            return Err(LuceneError::IllegalArgument(format!(
                "node {} out of bounds (graph length {})",
                node,
                self.graph.len()
            )));
        }
        let levels = self.graph[node_usize]
            .as_ref()
            .ok_or_else(|| LuceneError::IllegalArgument(format!("node {} has no levels", node)))?;
        let level_usize = level as usize;
        if level_usize >= levels.len() {
            return Err(LuceneError::IllegalArgument(format!(
                "level {} out of bounds for node {} (has {} levels)",
                level,
                node,
                levels.len()
            )));
        }
        levels[level_usize].as_ref().ok_or_else(|| {
            LuceneError::IllegalArgument(format!(
                "node {} has no neighbor array at level {}",
                node, level
            ))
        })
    }

    /// Replaces the neighbor array for `node` at `level`.
    pub fn set_neighbors(&mut self, level: i32, node: i32, neighbors: NeighborArray) -> Result<()> {
        let node_usize = node as usize;
        if node_usize >= self.graph.len() {
            return Err(LuceneError::IllegalArgument(format!(
                "node {} out of bounds",
                node
            )));
        }
        let levels = self.graph[node_usize]
            .as_mut()
            .ok_or_else(|| LuceneError::IllegalArgument(format!("node {} has no levels", node)))?;
        let level_usize = level as usize;
        if level_usize >= levels.len() {
            return Err(LuceneError::IllegalArgument(format!(
                "level {} out of bounds for node {}",
                level, node
            )));
        }
        levels[level_usize] = Some(neighbors);
        Ok(())
    }

    /// Adds a node on the given level. Nodes must be added from their top
    /// level downward.
    pub fn add_node(&mut self, level: i32, node: i32) {
        let node_usize = node as usize;
        if node_usize >= self.graph.len() {
            if self.no_growth {
                panic!("The graph does not expect to grow when an initial size is given");
            }
            let new_len =
                crate::util::ArrayUtil::oversize(node_usize + 1, std::mem::size_of::<usize>())
                    .max(node_usize + 1);
            self.graph.resize(new_len, None);
        }

        if self.graph[node_usize].is_none() {
            self.graph[node_usize] = Some(Vec::with_capacity((level + 1) as usize));
            self.size += 1;
        }

        let levels = self.graph[node_usize].as_mut().unwrap();
        let target_len = (level + 1) as usize;
        if levels.len() < target_len {
            levels.resize(target_len, None);
        }

        levels[level as usize] = Some(NeighborArray::new(
            if level == 0 { self.nsize0 } else { self.nsize },
            true,
        ));
        self.max_node_id = self.max_node_id.max(node);
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
    pub fn node_exists_at_level(&self, level: i32, node: i32) -> bool {
        let node_usize = node as usize;
        if node_usize >= self.graph.len() {
            return false;
        }
        match &self.graph[node_usize] {
            None => false,
            Some(levels) => {
                let l = level as usize;
                l < levels.len() && levels[l].is_some()
            }
        }
    }

    /// Returns the number of levels currently in the graph.
    fn num_levels_internal(&self) -> i32 {
        self.entry_node.get().level + 1
    }

    /// Builds the level->nodes mapping used by `get_nodes_on_level`.
    fn build_level_to_nodes(&self) -> Vec<Vec<i32>> {
        let num_levels = self.num_levels_internal();
        let mut level_to_nodes: Vec<Vec<i32>> = (0..num_levels).map(|_| Vec::new()).collect();
        let mut non_null_count = 0i32;
        for (node, levels) in self.graph.iter().enumerate() {
            if levels.is_none() {
                continue;
            }
            non_null_count += 1;
            let levels = levels.as_ref().unwrap();
            for i in 1..levels.len() {
                if levels[i].is_some() {
                    level_to_nodes[i].push(node as i32);
                }
            }
            if non_null_count == self.size {
                break;
            }
        }
        level_to_nodes
    }
}

impl HnswGraph for OnHeapHnswGraph {
    fn seek(&mut self, level: i32, target: i32) -> Result<()> {
        self.cur = Some(self.get_neighbors(level, target)?.clone());
        self.upto = -1;
        Ok(())
    }

    fn size(&self) -> i32 {
        self.size
    }

    fn max_node_id(&self) -> i32 {
        if self.no_growth {
            self.graph.len() as i32 - 1
        } else {
            self.max_node_id
        }
    }

    fn next_neighbor(&mut self) -> Result<i32> {
        let cur = self
            .cur
            .as_ref()
            .ok_or_else(|| LuceneError::IllegalState("seek not called".to_string()))?;
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
        if self.size != self.max_node_id() + 1 {
            return Err(LuceneError::IllegalState(format!(
                "graph build not complete, size={} max_node_id={}",
                self.size,
                self.max_node_id()
            )));
        }
        if level == 0 {
            Ok(Box::new(DenseNodesIterator::new(self.size)))
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
