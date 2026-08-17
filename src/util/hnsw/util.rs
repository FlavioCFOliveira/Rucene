//! Graph reachability and component helpers for HNSW graphs.
//!
//! Equivalent to `org.apache.lucene.util.hnsw.HnswUtil`.

#![deny(unsafe_code)]

use std::collections::VecDeque;

use crate::search::NO_MORE_DOCS;
use crate::util::FixedBitSet;

use super::{HnswGraph, Result};

/// A connected component of the graph.
///
/// Equivalent to `org.apache.lucene.util.hnsw.HnswUtil.Component`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Component {
    start: i32,
    size: i32,
}

impl Component {
    /// Creates a component description.
    pub fn new(start: i32, size: i32) -> Self {
        Self { start, size }
    }

    /// Returns the lowest-numbered node in the component.
    pub fn start(&self) -> i32 {
        self.start
    }

    /// Returns the number of nodes in the component.
    pub fn size(&self) -> i32 {
        self.size
    }
}

/// HNSW graph utilities.
///
/// Equivalent to `org.apache.lucene.util.hnsw.HnswUtil`.
pub struct HnswUtil;

impl HnswUtil {
    /// Returns true if every node on every level is reachable from the entry
    /// point.
    pub fn is_rooted(graph: &mut dyn HnswGraph) -> Result<bool> {
        for level in 0..graph.num_levels()? {
            if Self::components(graph, level)?.len() > 1 {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Returns the sizes of the distinct graph components on the given level.
    pub fn component_sizes(graph: &mut dyn HnswGraph, level: i32) -> Result<Vec<i32>> {
        Ok(Self::components(graph, level)?
            .into_iter()
            .map(|c| c.size())
            .collect())
    }

    /// Finds the connected components on a graph level.
    pub fn components(graph: &mut dyn HnswGraph, level: i32) -> Result<Vec<Component>> {
        let num_levels = graph.num_levels()?;
        if level < 0 || level >= num_levels {
            return Err(crate::error::LuceneError::IllegalArgument(format!(
                "level {} too large for graph with {} levels",
                level, num_levels
            )));
        }
        let graph_size = graph.max_node_id() + 1;
        let mut connected_nodes = FixedBitSet::new(graph_size.max(0) as usize);
        let mut components = Vec::new();
        let mut total = 0i32;

        // Start from the entry point(s) on the top level, otherwise from nodes
        // on the next higher level.
        if level == num_levels - 1 {
            let entry = graph.entry_node()?;
            let component = Self::mark_rooted(graph, level, &mut connected_nodes, entry)?;
            total += component.size();
        } else {
            let mut iter = graph.get_nodes_on_level(level + 1)?;
            while iter.has_next() {
                let entry = iter.next_int();
                let component = Self::mark_rooted(graph, level, &mut connected_nodes, entry)?;
                total += component.size();
            }
        }

        if total > 0 {
            components.push(Component::new(
                if level == num_levels - 1 {
                    graph.entry_node()?
                } else {
                    Self::next_unvisited(graph, level, &connected_nodes)?
                },
                total,
            ));
        }

        if level == 0 {
            let mut next_clear = connected_nodes.next_clear_bit(0);
            while next_clear != NO_MORE_DOCS as usize {
                let component =
                    Self::mark_rooted(graph, level, &mut connected_nodes, next_clear as i32)?;
                components.push(component);
                total += component.size();
                next_clear = connected_nodes.next_clear_bit(component.start() as usize);
            }
        } else {
            let mut iter = graph.get_nodes_on_level(level)?;
            while iter.has_next() {
                let node = iter.next_int();
                if connected_nodes.get(node as usize) {
                    continue;
                }
                let component = Self::mark_rooted(graph, level, &mut connected_nodes, node)?;
                components.push(component);
                total += component.size();
            }
        }

        debug_assert_eq!(
            total,
            graph.get_nodes_on_level(level)?.size(),
            "component total must equal level node count"
        );
        Ok(components)
    }

    fn mark_rooted(
        graph: &mut dyn HnswGraph,
        level: i32,
        connected_nodes: &mut FixedBitSet,
        entry_point: i32,
    ) -> Result<Component> {
        if connected_nodes.get(entry_point as usize) {
            return Ok(Component::new(entry_point, 0));
        }
        let mut stack: VecDeque<i32> = VecDeque::new();
        let mut nodes_in_stack = FixedBitSet::new(connected_nodes.length());
        stack.push_back(entry_point);
        let mut count = 0i32;
        while let Some(node) = stack.pop_back() {
            if connected_nodes.get(node as usize) {
                continue;
            }
            count += 1;
            connected_nodes.set(node as usize);
            graph.seek(level, node)?;
            loop {
                let friend = graph.next_neighbor()?;
                if friend == NO_MORE_DOCS {
                    break;
                }
                if !connected_nodes.get(friend as usize) && !nodes_in_stack.get(friend as usize) {
                    stack.push_back(friend);
                    nodes_in_stack.set(friend as usize);
                }
            }
        }
        Ok(Component::new(entry_point, count))
    }

    fn next_unvisited(
        _graph: &mut dyn HnswGraph,
        _level: i32,
        connected_nodes: &FixedBitSet,
    ) -> Result<i32> {
        Ok(connected_nodes.next_clear_bit(0) as i32)
    }
}

/// Extension trait for `FixedBitSet` to provide the next clear bit.
trait FixedBitSetExt {
    fn next_clear_bit(&self, from: usize) -> usize;
}

impl FixedBitSetExt for FixedBitSet {
    fn next_clear_bit(&self, from: usize) -> usize {
        let len = self.length();
        if from >= len {
            return NO_MORE_DOCS as usize;
        }
        let mut index = from;
        while index < len && self.get(index) {
            index += 1;
        }
        if index >= len {
            NO_MORE_DOCS as usize
        } else {
            index
        }
    }
}
