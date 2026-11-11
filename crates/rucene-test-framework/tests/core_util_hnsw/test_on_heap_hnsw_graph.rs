//! Port of `lucene/core/src/test/org/apache/lucene/util/hnsw/TestOnHeapHnswGraph.java`:
//! `OnHeapHnswGraph`'s behavior specifically; for more complex tests, see
//! `HnswGraphTestCase`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::{Class, Method};
use rucene::error::Result;
use rucene::util::hnsw::{HnswGraph, OnHeapHnswGraph};
use rucene_test_framework::util::java_exceptions::expect_throws;

use crate::core_support::{assert_equals, class, expect_throws_panic, random, test};

/// `TestOnHeapHnswGraph`.
pub fn class_def() -> Arc<Class> {
    let methods: Vec<Method> = vec![
        test("testNoGrowth", test_no_growth),
        test("testIncompleteGraphThrow", test_incomplete_graph_throw),
        test("testGraphGrowth", test_graph_growth),
        test("testGraphBuildOutOfOrder", test_graph_build_out_of_order),
    ];
    class("org.apache.lucene.util.hnsw.TestOnHeapHnswGraph", methods)
}

/// `testNoGrowth()`: assert exception will be thrown when we add out of
/// bound node to a fixed size graph.
fn test_no_growth() -> Result<()> {
    let mut graph = OnHeapHnswGraph::new(10, 100);
    expect_throws_panic("java.lang.IllegalStateException", || graph.add_node(1, 100));
    Ok(())
}

/// `testIncompleteGraphThrow()`: assert exception will be thrown when we
/// call `getNodeOnLevel` for an incomplete graph.
fn test_incomplete_graph_throw() -> Result<()> {
    let mut graph = OnHeapHnswGraph::new(10, -1);
    graph.add_node(1, 0);
    graph.add_node(0, 0);
    assert_equals(&1, &graph.get_nodes_on_level(1)?.size());
    graph.add_node(0, 5);
    expect_throws("java.lang.IllegalStateException", || {
        graph.get_nodes_on_level(0)
    });
    Ok(())
}

fn test_graph_growth() -> Result<()> {
    let mut graph = OnHeapHnswGraph::new(10, -1);
    let max_level = 5;
    let mut level_to_nodes: Vec<Vec<i32>> = vec![Vec::new(); max_level as usize];
    for i in 0..101 {
        let level = random().next_int_bound(max_level);
        for l in (0..=level).rev() {
            graph.add_node(l, i);
            graph.try_set_new_entry_node(i, l);
            if l > graph.num_levels()? - 1 {
                let num_levels = graph.num_levels()?;
                graph.try_promote_new_entry_node(i, l, num_levels - 1);
            }
            level_to_nodes[l as usize].push(i);
        }
    }
    assert_graph_equals(&graph, &level_to_nodes)
}

fn test_graph_build_out_of_order() -> Result<()> {
    let mut graph = OnHeapHnswGraph::new(10, -1);
    let max_level = 5;
    let num_nodes = 100;
    let mut level_to_nodes: Vec<Vec<i32>> = vec![Vec::new(); max_level as usize];
    let mut insertions: Vec<i32> = (0..num_nodes).collect();
    // shuffle the insertion order
    for _ in 0..40 {
        let pos1 = random().next_int_bound(num_nodes) as usize;
        let pos2 = random().next_int_bound(num_nodes) as usize;
        insertions.swap(pos1, pos2);
    }

    for &i in &insertions {
        let level = random().next_int_bound(max_level);
        for l in (0..=level).rev() {
            graph.add_node(l, i);
            graph.try_set_new_entry_node(i, l);
            if l > graph.num_levels()? - 1 {
                let num_levels = graph.num_levels()?;
                graph.try_promote_new_entry_node(i, l, num_levels - 1);
            }
            level_to_nodes[l as usize].push(i);
        }
    }

    for nodes in level_to_nodes.iter_mut() {
        nodes.sort();
    }

    assert_graph_equals(&graph, &level_to_nodes)
}

/// `assertGraphEquals(graph, levelToNodes)`.
fn assert_graph_equals(graph: &OnHeapHnswGraph, level_to_nodes: &[Vec<i32>]) -> Result<()> {
    for l in 0..graph.num_levels()? {
        let mut nodes_iterator = graph.get_nodes_on_level(l)?;
        assert_equals(
            &(level_to_nodes[l as usize].len() as i32),
            &nodes_iterator.size(),
        );
        let mut idx = 0;
        while nodes_iterator.has_next() {
            assert_equals(&level_to_nodes[l as usize][idx], &nodes_iterator.next_int());
            idx += 1;
        }
    }
    Ok(())
}
