//! Port of `org.apache.lucene.util.hnsw.UpdateGraphsUtils`.

use crate::error::Result;
use crate::internal::hppc::IntHashSet;
use crate::search::NO_MORE_DOCS;
use crate::util::TernaryLongHeap;

use super::HnswGraph;

/// Utility for updating a big graph with smaller graphs.
///
/// Equivalent to `org.apache.lucene.util.hnsw.UpdateGraphsUtils`. This is used
/// during the merging of segments containing HNSW graphs.
pub struct UpdateGraphsUtils;

impl UpdateGraphsUtils {
    /// Finds the nodes of the graph that best cover it.
    ///
    /// This is reminiscent of an edge-cover problem; here, rather than choosing
    /// edges, we pick nodes and increment a count at their neighbours.
    ///
    /// # Errors
    ///
    /// Returns an error if reading the graph fails.
    pub fn compute_join_set(graph: &mut dyn HnswGraph) -> Result<IntHashSet> {
        // coverage for the current node
        let mut k: i32;
        let size = graph.size();
        let mut heap = TernaryLongHeap::new(size.max(1) as usize)?;
        let mut j = IntHashSet::new();
        let mut stale = vec![false; size as usize];
        let mut counts = vec![0i16; size as usize];
        let mut g_exit = 0i64;
        for v in 0..size {
            graph.seek(0, v)?;
            let degree = graph.neighbor_count();
            k = if degree < 9 { 2 } else { ceil_div(degree, 4) };
            g_exit += i64::from(k);
            let gain = k + degree;
            heap.push(encode(gain, v));
        }

        let mut g_tot = 0i64;
        while g_tot < g_exit && heap.size() > 0 {
            let el = heap.pop()?;
            let gain = decode_value1(el);
            let v = decode_value2(el);
            graph.seek(0, v)?;
            let degree = graph.neighbor_count();
            let mut ns = Vec::with_capacity(degree.max(0) as usize);
            loop {
                let u = graph.next_neighbor()?;
                if u == NO_MORE_DOCS {
                    break;
                }
                ns.push(u);
            }
            k = if degree < 9 { 2 } else { ceil_div(degree, 4) };
            if stale[v as usize] {
                // if stale, recalculate the gain
                let mut new_gain = 0.max(k - i32::from(counts[v as usize]));
                for &u in &ns {
                    if i32::from(counts[u as usize]) < k && !j.contains(u) {
                        new_gain += 1;
                    }
                }
                if new_gain > 0 {
                    heap.push(encode(new_gain, v));
                    stale[v as usize] = false;
                }
            } else {
                j.add(v);
                g_tot += i64::from(gain);
                let mark_neighbours_stale = i32::from(counts[v as usize]) < k;
                for &u in &ns {
                    if mark_neighbours_stale {
                        stale[u as usize] = true;
                    }
                    if i32::from(counts[u as usize]) < (k - 1) {
                        // make the neighbours of u stale
                        graph.seek(0, u)?;
                        loop {
                            let uu = graph.next_neighbor()?;
                            if uu == NO_MORE_DOCS {
                                break;
                            }
                            stale[uu as usize] = true;
                        }
                    }
                    counts[u as usize] = counts[u as usize].wrapping_add(1);
                }
            }
        }
        Ok(j)
    }
}

/// `java.lang.Math.ceilDiv(int, int)` for the non-negative degrees used here.
fn ceil_div(x: i32, y: i32) -> i32 {
    let q = x / y;
    if (x % y != 0) && ((x ^ y) >= 0) {
        q + 1
    } else {
        q
    }
}

fn encode(value1: i32, value2: i32) -> i64 {
    ((-(value1 as i64)) << 32) | i64::from(value2 as u32)
}

fn decode_value1(encoded: i64) -> i32 {
    -((encoded >> 32) as i32)
}

fn decode_value2(encoded: i64) -> i32 {
    (encoded & 0xFFFF_FFFF) as i32
}
