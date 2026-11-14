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

//! Port of `SpanDisiPriorityQueue.java`, of the Java package
//! `org.apache.lucene.queries.spans`.
//!
//! # Adaptation: the heap holds positions
//!
//! As in the core's [`DisiPriorityQueue`](rucene::search::DisiPriorityQueue),
//! Java's heap of wrapper references becomes a heap of **positions** into the
//! wrappers this queue owns, and the `topList` linked through
//! `SpanDisiWrapper.next` becomes a vector of positions in the same traversal
//! order. The heap layout, the sift loops and the `topList` order are
//! unchanged.

use crate::spans::span_disi_wrapper::SpanDisiWrapper;

/// Equivalent to the package-private static `SpanDisiPriorityQueue.leftNode(int)`.
pub(crate) fn left_node(node: usize) -> usize {
    ((node + 1) << 1) - 1
}

/// Equivalent to the package-private static `SpanDisiPriorityQueue.rightNode(int)`.
pub(crate) fn right_node(left_node: usize) -> usize {
    left_node + 1
}

/// Equivalent to the package-private static `SpanDisiPriorityQueue.parentNode(int)`,
/// `((node + 1) >>> 1) - 1`, which is `-1` for the root.
pub(crate) fn parent_node(node: usize) -> isize {
    (((node + 1) >> 1) as isize) - 1
}

/// A priority queue of span iterators that orders by current doc ID.
///
/// Equivalent to the package-private final
/// `org.apache.lucene.queries.spans.SpanDisiPriorityQueue`. This
/// specialization is needed over a generic priority queue because the
/// pluggable comparison function makes the rebalancing quite slow.
#[derive(Debug)]
pub(crate) struct SpanDisiPriorityQueue {
    /// The wrappers added to this queue, which Java's heap references.
    wrappers: Vec<SpanDisiWrapper>,
    /// Positions into `wrappers`; Java's `heap` array, `size` long.
    heap: Vec<usize>,
    max_size: usize,
}

impl SpanDisiPriorityQueue {
    /// Equivalent to `SpanDisiPriorityQueue(int)`.
    pub(crate) fn new(max_size: usize) -> Self {
        Self {
            wrappers: Vec::with_capacity(max_size),
            heap: Vec::with_capacity(max_size),
            max_size,
        }
    }

    /// Equivalent to `SpanDisiPriorityQueue.size()`.
    pub(crate) fn size(&self) -> usize {
        self.heap.len()
    }

    /// Returns the position of the top wrapper, or `None` when empty.
    ///
    /// Equivalent to `SpanDisiPriorityQueue.top()`, which reads `heap[0]`.
    pub(crate) fn top(&self) -> Option<usize> {
        self.heap.first().copied()
    }

    /// The wrapper at `position`.
    pub(crate) fn wrapper(&self, position: usize) -> &SpanDisiWrapper {
        &self.wrappers[position]
    }

    /// The wrapper at `position`, mutably.
    pub(crate) fn wrapper_mut(&mut self, position: usize) -> &mut SpanDisiWrapper {
        &mut self.wrappers[position]
    }

    /// Returns the positions of the wrappers which are on the current doc, in
    /// the order Java's linked list traverses them.
    ///
    /// Equivalent to `SpanDisiPriorityQueue.topList()`, which builds its list
    /// by prepending: the traversal starts at the last wrapper prepended, so
    /// the vector collected in prepend order is returned reversed.
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
    /// Equivalent to the private `SpanDisiPriorityQueue.prepend(SpanDisiWrapper,
    /// SpanDisiWrapper)`; see the module documentation.
    fn prepend(w1: usize, order: &mut Vec<usize>) {
        order.push(w1);
    }

    /// Equivalent to the private
    /// `SpanDisiPriorityQueue.topList(SpanDisiWrapper, SpanDisiWrapper[], int, int)`.
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
    /// Equivalent to `SpanDisiPriorityQueue.add(SpanDisiWrapper)`.
    ///
    /// # Panics
    ///
    /// Panics when the queue is full, which is the
    /// `ArrayIndexOutOfBoundsException` Java raises.
    pub(crate) fn add(&mut self, entry: SpanDisiWrapper) -> usize {
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
    /// Equivalent to `SpanDisiPriorityQueue.pop()`.
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
    /// Equivalent to `SpanDisiPriorityQueue.updateTop()`.
    pub(crate) fn update_top(&mut self) -> Option<usize> {
        let size = self.heap.len();
        self.down_heap(size);
        self.top()
    }

    /// Replaces the top wrapper and rebalances.
    ///
    /// Equivalent to the package-private
    /// `SpanDisiPriorityQueue.updateTop(SpanDisiWrapper)`.
    pub(crate) fn update_top_with(&mut self, top_replacement: usize) -> Option<usize> {
        if self.heap.is_empty() {
            self.heap.push(top_replacement);
        } else {
            self.heap[0] = top_replacement;
        }
        self.update_top()
    }

    /// Equivalent to the package-private `SpanDisiPriorityQueue.upHeap(int)`.
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

    /// Equivalent to the package-private `SpanDisiPriorityQueue.downHeap(int)`.
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
    /// Equivalent to `SpanDisiPriorityQueue.iterator()`, over
    /// `Arrays.asList(heap).subList(0, size)`.
    pub(crate) fn iter(&self) -> impl Iterator<Item = &SpanDisiWrapper> + '_ {
        self.heap
            .iter()
            .map(move |&position| &self.wrappers[position])
    }
}
