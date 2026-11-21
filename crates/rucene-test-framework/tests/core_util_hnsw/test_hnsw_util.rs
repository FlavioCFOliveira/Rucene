//! Port of `lucene/core/src/test/org/apache/lucene/util/hnsw/TestHnswUtil.java`.

use std::sync::Arc;

use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::reflect::{Class, Method};
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::search::NO_MORE_DOCS;
use rucene::util::hnsw::{HnswGraph, HnswUtil, NodesIterator};
use rucene::util::FixedBitSet;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_equals, class, random, test};

/// `TestHnswUtil`.
pub fn class_def() -> Arc<Class> {
    let methods: Vec<Method> = vec![
        test("testTreeWithCycle", test_tree_with_cycle),
        test("testBackLinking", test_back_linking),
        test("testChain", test_chain),
        test("testTwoChains", test_two_chains),
        test("testLevels", test_levels),
        test("testLevelsNotRooted", test_levels_not_rooted),
        test("testRandom", test_random),
    ];
    class("org.apache.lucene.util.hnsw.TestHnswUtil", methods)
}

/// `int[][][]` with `null` entries.
type Nodes = Vec<Vec<Option<Vec<i32>>>>;

/// One level of `int[][]` literals, every node present.
fn level(nodes: &[&[i32]]) -> Vec<Option<Vec<i32>>> {
    nodes.iter().map(|n| Some(n.to_vec())).collect()
}

/// `HnswUtil.componentSizes(graph)`: `componentSizes(graph, 0)`.
fn component_sizes(graph: &mut MockGraph) -> Result<Vec<i32>> {
    HnswUtil::component_sizes(graph, 0)
}

fn test_tree_with_cycle() -> Result<()> {
    // nodes that link "forward" to each other, plus a cycle back to the root
    let nodes: Nodes = vec![level(&[
        &[1, 2], // node 0
        &[3, 4], // node 1
        &[5, 6], // node 2
        &[],
        &[],
        &[],
        &[0],
    ])];
    let mut graph = MockGraph::new(nodes);
    Assert::assert_true(HnswUtil::is_rooted(&mut graph)?);
    assert_equals(&vec![7], &component_sizes(&mut graph)?);
    Ok(())
}

fn test_back_linking() -> Result<()> {
    // nodes that link "backward" to each other
    let nodes: Nodes = vec![level(&[
        &[1, 2], // node 0
        &[3, 4], // node 1
        &[0],    // node 2
        &[1],
        &[1],
        &[1],
        &[1],
    ])];
    let mut graph = MockGraph::new(nodes);
    Assert::assert_false(HnswUtil::is_rooted(&mut graph)?);
    // [ {0, 1, 2, 3, 4}, {5}, {6}
    assert_equals(&vec![5, 1, 1], &component_sizes(&mut graph)?);
    Ok(())
}

fn test_chain() -> Result<()> {
    // nodes that link "backward" to each other
    let nodes: Nodes = vec![level(&[&[1], &[2], &[3], &[0]])];
    let mut graph = MockGraph::new(nodes);
    Assert::assert_true(HnswUtil::is_rooted(&mut graph)?);
    assert_equals(&vec![4], &component_sizes(&mut graph)?);
    Ok(())
}

fn test_two_chains() -> Result<()> {
    // nodes that link "backward" to each other
    let nodes: Nodes = vec![level(&[&[2], &[3], &[0], &[1]])];
    let mut graph = MockGraph::new(nodes);
    Assert::assert_false(HnswUtil::is_rooted(&mut graph)?);
    assert_equals(&vec![2, 2], &component_sizes(&mut graph)?);
    Ok(())
}

fn test_levels() -> Result<()> {
    let nodes: Nodes = vec![
        level(&[&[1, 2], &[3], &[0], &[0]]),
        vec![Some(vec![2]), None, Some(vec![0]), None],
        vec![Some(vec![]), None, None, None],
    ];
    let mut graph = MockGraph::new(nodes);
    // System.out.println(graph.toString());
    Assert::assert_true(HnswUtil::is_rooted(&mut graph)?);
    assert_equals(&vec![4], &component_sizes(&mut graph)?);
    Ok(())
}

fn test_levels_not_rooted() -> Result<()> {
    let nodes: Nodes = vec![level(&[&[1], &[0], &[0]]), vec![Some(vec![]), None, None]];
    let mut graph = MockGraph::new(nodes);
    // System.out.println(graph.toString());
    Assert::assert_false(HnswUtil::is_rooted(&mut graph)?);
    assert_equals(&vec![2, 1], &component_sizes(&mut graph)?);
    Ok(())
}

fn test_random() -> Result<()> {
    let mut i = 0;
    while i < LuceneTestCase::at_least(10) {
        // test on a random directed graph comparing against a brute force algorithm
        let num_nodes = random().next_int_range(1, 100);
        let num_levels = (num_nodes as f64).ln().ceil() as i32;
        let mut nodes: Nodes = vec![Vec::new(); num_levels as usize];
        for level in (0..num_levels).rev() {
            nodes[level as usize] = vec![None; num_nodes as usize];
            for node in 0..num_nodes {
                if level > 0
                    && ((level == num_levels - 1 && node > 0)
                        || (level < num_levels - 1
                            && nodes[level as usize + 1][node as usize].is_none()))
                    && (random().next_float() as f64) > std::f64::consts::E.powi(-level)
                {
                    // this.next() is not on this level
                    continue;
                }
                let mut num_nbrs = random().next_int_bound((num_nodes + 7) / 8);
                if level == 0 {
                    num_nbrs *= 2;
                }
                nodes[level as usize][node as usize] = Some(vec![0; num_nbrs as usize]);
                for nbr in 0..num_nbrs as usize {
                    loop {
                        let random_nbr = random().next_int_bound(num_nodes);
                        if nodes[level as usize][random_nbr as usize].is_some() {
                            // allow self-linking; this doesn't arise in HNSW but it's valid more
                            // generally
                            nodes[level as usize][node as usize]
                                .as_mut()
                                .expect("just set")[nbr] = random_nbr;
                            break;
                        }
                        // nbr not on this level, try again
                    }
                }
            }
        }
        let rooted = is_rooted(&nodes);
        let mut graph = MockGraph::new(nodes);
        assert_equals(&rooted, &HnswUtil::is_rooted(&mut graph)?);
        i += 1;
    }
    Ok(())
}

/// `isRooted(nodes)`.
fn is_rooted(nodes: &Nodes) -> bool {
    for level in (0..nodes.len()).rev() {
        if !is_rooted_level(nodes, level) {
            return false;
        }
    }
    true
}

/// `isRooted(nodes, level)`: is every node on this level reachable from
/// the nodes on the level above (or node 0 on the top level)?
fn is_rooted_level(nodes: &Nodes, level: usize) -> bool {
    // check that the graph is rooted in the union of the entry nodes' trees
    let entry_points: Vec<Option<Vec<i32>>> = if level == nodes.len() - 1 {
        vec![nodes[level][0].clone()]
    } else {
        nodes[level + 1].clone()
    };
    let mut connected = FixedBitSet::new(nodes[level].len());
    let mut count = 0;
    for (entry_point, ep) in entry_points.iter().enumerate() {
        if ep.is_none() {
            // use nodes present on next higher level (or this level if top level) as entry points
            continue;
        }
        let mut stack: Vec<i32> = vec![entry_point as i32];
        while let Some(node) = stack.pop() {
            if connected.get(node as usize) {
                continue;
            }
            connected.set(node as usize);
            count += 1;
            for &nbr in nodes[level][node as usize].as_ref().expect("on this level") {
                stack.push(nbr);
            }
        }
    }
    count == level_size(&nodes[level])
}

/// `levelSize(nodes)`.
fn level_size(nodes: &[Option<Vec<i32>>]) -> i32 {
    nodes.iter().filter(|n| n.is_some()).count() as i32
}

/// `MockGraph extends HnswGraph`: an empty graph value.
struct MockGraph {
    nodes: Nodes,
    current_level: usize,
    current_node: usize,
    current_neighbor: usize,
}

impl MockGraph {
    fn new(nodes: Nodes) -> Self {
        MockGraph {
            nodes,
            current_level: 0,
            current_node: 0,
            current_neighbor: 0,
        }
    }
}

impl std::fmt::Display for MockGraph {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for level in (0..self.nodes.len()).rev() {
            write!(f, "\nLEVEL {level}\n")?;
            for (node, nbrs) in self.nodes[level].iter().enumerate() {
                if let Some(nbrs) = nbrs {
                    writeln!(
                        f,
                        "  {node}:{}",
                        crate::core_support::arrays_to_string(nbrs)
                    )?;
                }
            }
        }
        Ok(())
    }
}

impl HnswGraph for MockGraph {
    fn next_neighbor(&mut self) -> Result<i32> {
        let nbrs = self.nodes[self.current_level][self.current_node]
            .as_ref()
            .expect("sought node is on its level");
        if self.current_neighbor >= nbrs.len() {
            Ok(NO_MORE_DOCS)
        } else {
            let n = nbrs[self.current_neighbor];
            self.current_neighbor += 1;
            Ok(n)
        }
    }

    fn seek(&mut self, level: i32, target: i32) -> Result<()> {
        assert!(level >= 0 && (level as usize) < self.nodes.len());
        assert!(
            target >= 0 && (target as usize) < self.nodes[level as usize].len(),
            "target out of range: {target} for level {level}; should be less than {}",
            self.nodes[level as usize].len()
        );
        assert!(
            self.nodes[level as usize][target as usize].is_some(),
            "target {target} not on level {level}"
        );
        self.current_level = level as usize;
        self.current_node = target as usize;
        self.current_neighbor = 0;
        Ok(())
    }

    fn size(&self) -> i32 {
        self.nodes[0].len() as i32
    }

    fn num_levels(&self) -> Result<i32> {
        Ok(self.nodes.len() as i32)
    }

    fn entry_node(&self) -> Result<i32> {
        Ok(0)
    }

    fn neighbor_count(&self) -> i32 {
        self.nodes[self.current_level][self.current_node]
            .as_ref()
            .map_or(0, |n| n.len() as i32)
    }

    fn max_conn(&self) -> i32 {
        rucene::util::hnsw::UNKNOWN_MAX_CONN
    }

    fn get_nodes_on_level(&self, level: i32) -> Result<Box<dyn NodesIterator>> {
        let level_nodes = self.nodes[level as usize].clone();
        let count = level_nodes.iter().filter(|n| n.is_some()).count() as i32;
        Ok(Box::new(MockNodesIterator {
            nodes: level_nodes,
            final_count: count,
            cur: -1,
            cur_count: 0,
        }))
    }
}

/// The anonymous `NodesIterator` of `MockGraph.getNodesOnLevel`.
struct MockNodesIterator {
    nodes: Vec<Option<Vec<i32>>>,
    final_count: i32,
    cur: i32,
    cur_count: i32,
}

impl NodesIterator for MockNodesIterator {
    fn size(&self) -> i32 {
        self.final_count
    }

    fn next_int(&mut self) -> i32 {
        while self.cur_count < self.final_count {
            self.cur += 1;
            if self.nodes[self.cur as usize].is_some() {
                self.cur_count += 1;
                return self.cur;
            }
        }
        Throwable::new("java.lang.IllegalStateException", "exhausted").throw()
    }

    fn has_next(&self) -> bool {
        self.cur_count < self.final_count
    }

    fn consume(&mut self, _dest: &mut [i32]) -> usize {
        Throwable::new("java.lang.UnsupportedOperationException", String::new()).throw()
    }
}
