/*
 * Licensed to the Apache Software Foundation (ASF) under one or more
 * contributor license agreements.  See the NOTICE file distributed with
 * this work for additional information regarding copyright ownership.
 * The ASF licenses this file to You under the Apache License, Version 2.0
 * (the "License"); you may not use this file except in compliance with
 * the License.  You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

//! Port of `DisiPriorityQueue.java`, of the Java package
//! `org.apache.lucene.queries.intervals`.
//!
//! # Adaptation: the heap holds positions
//!
//! As in the core's [`DisiPriorityQueue`](rucene::search::DisiPriorityQueue),
//! Java's heap of wrapper references becomes a heap of **positions** into the
//! wrappers this queue owns, and the `topList` linked through
//! `DisiWrapper.next` becomes a vector of positions in the same traversal
//! order. The heap layout, the sift loops and the `topList` order are
//! unchanged.

use crate::intervals::disi_wrapper::DisiWrapper;

/// Equivalent to the package-private static `DisiPriorityQueue.leftNode(int)`.
pub(crate) fn left_node(node: usize) -> usize {
    ((node + 1) << 1) - 1
}

/// Equivalent to the package-private static `DisiPriorityQueue.rightNode(int)`.
pub(crate) fn right_node(left_node: usize) -> usize {
    left_node + 1
}

/// Equivalent to the package-private static `DisiPriorityQueue.parentNode(int)`,
/// `((node + 1) >>> 1) - 1`, which is `-1` for the root.
pub(crate) fn parent_node(node: usize) -> isize {
    (((node + 1) >> 1) as isize) - 1
}

/// A priority queue of interval iterators that orders by current doc ID.
///
/// Equivalent to the final class
/// `org.apache.lucene.queries.intervals.DisiPriorityQueue`, the package's own
/// copy, distinct from the core's. This specialisation is needed over a
/// generic priority queue because the pluggable comparison function makes the
/// rebalancing quite slow.
#[derive(Debug)]
pub(crate) struct DisiPriorityQueue {
    /// The wrappers added to this queue, which Java's heap references.
    wrappers: Vec<DisiWrapper>,
    /// Positions into `wrappers`; Java's `heap` array, `size` long.
    heap: Vec<usize>,
    max_size: usize,
}

impl DisiPriorityQueue {
    /// Equivalent to `DisiPriorityQueue(int)`.
    pub(crate) fn new(max_size: usize) -> Self {
        Self {
            wrappers: Vec::with_capacity(max_size),
            heap: Vec::with_capacity(max_size),
            max_size,
        }
    }

    /// Equivalent to `DisiPriorityQueue.size()`.
    pub(crate) fn size(&self) -> usize {
        self.heap.len()
    }

    /// Returns the position of the top wrapper, or `None` when empty.
    ///
    /// Equivalent to `DisiPriorityQueue.top()`, which reads `heap[0]`.
    pub(crate) fn top(&self) -> Option<usize> {
        self.heap.first().copied()
    }

    /// The wrapper at `position`.
    pub(crate) fn wrapper(&self, position: usize) -> &DisiWrapper {
        &self.wrappers[position]
    }

    /// The wrapper at `position`, mutably.
    pub(crate) fn wrapper_mut(&mut self, position: usize) -> &mut DisiWrapper {
        &mut self.wrappers[position]
    }

    /// Returns the positions of the wrappers which are on the current doc, in
    /// the order Java's linked list traverses them.
    ///
    /// Equivalent to `DisiPriorityQueue.topList()`, which builds its list by
    /// prepending: the traversal starts at the last wrapper prepended, so the
    /// vector collected in prepend order is returned reversed.
    pub(crate) fn top_list(&self) -> Vec<usize> {
        let size = self.heap.len();
        let mut order = Vec::with_capacity(size);
        if size == 0 {
            return order;
        }
        let doc = self.wrappers[self.heap[0]].doc;
        order.push(self.heap[0]);
        if size >= 3 {
            self.collect_top_list(doc, 1, &mut order);
            self.collect_top_list(doc, 2, &mut order);
        } else if size == 2 && self.wrappers[self.heap[1]].doc == doc {
            Self::prepend(self.heap[1], &mut order);
        }
        order.reverse();
        order
    }

    /// Prepends `w1` (an iterator) to the list being built.
    ///
    /// Equivalent to the private `DisiPriorityQueue.prepend(DisiWrapper,
    /// DisiWrapper)`; see the module documentation.
    fn prepend(w1: usize, order: &mut Vec<usize>) {
        order.push(w1);
    }

    /// Equivalent to the private
    /// `DisiPriorityQueue.topList(DisiWrapper, DisiWrapper[], int, int)`.
    fn collect_top_list(&self, doc: i32, i: usize, order: &mut Vec<usize>) {
        let size = self.heap.len();
        let w = self.heap[i];
        if self.wrappers[w].doc == doc {
            Self::prepend(w, order);
            let left = left_node(i);
            let right = left + 1;
            if right < size {
                self.collect_top_list(doc, left, order);
                self.collect_top_list(doc, right, order);
            } else if left < size && self.wrappers[self.heap[left]].doc == doc {
                Self::prepend(self.heap[left], order);
            }
        }
    }

    /// Adds a wrapper and returns the position of the new top.
    ///
    /// Equivalent to `DisiPriorityQueue.add(DisiWrapper)`.
    ///
    /// # Panics
    ///
    /// Panics when the queue is full, which is the
    /// `ArrayIndexOutOfBoundsException` Java raises.
    pub(crate) fn add(&mut self, entry: DisiWrapper) -> usize {
        let size = self.heap.len();
        if size >= self.max_size {
            rucene::store::java_nio_file::throw_unchecked(
                "java.lang.ArrayIndexOutOfBoundsException",
                Some(&format!(
                    "Index {size} out of bounds for length {}",
                    self.max_size
                )),
            );
        }
        self.wrappers.push(entry);
        self.heap.push(self.wrappers.len() - 1);
        self.up_heap(size);
        self.heap[0]
    }

    /// Removes the top wrapper from the heap and returns its position.
    ///
    /// Equivalent to `DisiPriorityQueue.pop()`.
    pub(crate) fn pop(&mut self) -> Option<usize> {
        if self.heap.is_empty() {
            return None;
        }
        // `heap[0] = heap[--size]; heap[size] = null; downHeap(size)`.
        let result = self.heap.swap_remove(0);
        let size = self.heap.len();
        self.down_heap(size);
        Some(result)
    }

    /// Rebalances and returns the position of the top wrapper.
    ///
    /// Equivalent to `DisiPriorityQueue.updateTop()`.
    pub(crate) fn update_top(&mut self) -> Option<usize> {
        let size = self.heap.len();
        self.down_heap(size);
        self.top()
    }

    /// Replaces the top wrapper, given by its position, and rebalances.
    ///
    /// Equivalent to the package-private
    /// `DisiPriorityQueue.updateTop(DisiWrapper)`.
    pub(crate) fn update_top_with(&mut self, top_replacement: usize) -> Option<usize> {
        if self.heap.is_empty() {
            self.heap.push(top_replacement);
        } else {
            self.heap[0] = top_replacement;
        }
        self.update_top()
    }

    /// Equivalent to the package-private `DisiPriorityQueue.upHeap(int)`.
    fn up_heap(&mut self, mut i: usize) {
        let node = self.heap[i];
        let node_doc = self.wrappers[node].doc;
        let mut j = parent_node(i);
        while j >= 0 && node_doc < self.wrappers[self.heap[j as usize]].doc {
            self.heap[i] = self.heap[j as usize];
            i = j as usize;
            j = parent_node(j as usize);
        }
        self.heap[i] = node;
    }

    /// Equivalent to the package-private `DisiPriorityQueue.downHeap(int)`.
    fn down_heap(&mut self, size: usize) {
        if size == 0 {
            return;
        }
        let mut i = 0;
        let node = self.heap[0];
        let node_doc = self.wrappers[node].doc;
        let mut j = left_node(i);
        if j < size {
            let mut k = right_node(j);
            if k < size && self.wrappers[self.heap[k]].doc < self.wrappers[self.heap[j]].doc {
                j = k;
            }
            if self.wrappers[self.heap[j]].doc < node_doc {
                loop {
                    self.heap[i] = self.heap[j];
                    i = j;
                    j = left_node(i);
                    k = right_node(j);
                    if k < size && self.wrappers[self.heap[k]].doc < self.wrappers[self.heap[j]].doc
                    {
                        j = k;
                    }
                    if !(j < size && self.wrappers[self.heap[j]].doc < node_doc) {
                        break;
                    }
                }
                self.heap[i] = node;
            }
        }
    }

    /// Returns the wrappers in heap-array order.
    ///
    /// Equivalent to `DisiPriorityQueue.iterator()`, over
    /// `Arrays.asList(heap).subList(0, size)`.
    pub(crate) fn iter(&self) -> impl Iterator<Item = &DisiWrapper> + '_ {
        self.heap
            .iter()
            .map(move |&position| &self.wrappers[position])
    }
}
