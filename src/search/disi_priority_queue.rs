//! Iterator heaps, ported from `org.apache.lucene.search.DisiPriorityQueue`,
//! `DisiPriorityQueue2` and `DisiPriorityQueueN`.
//!
//! # Adaptation: the heap holds positions, not wrappers
//!
//! Java's heap stores [`DisiWrapper`] references, and the code around it keeps
//! its own references to the very same objects: it mutates `w.doc` and then
//! asks the heap to rebalance, walks the heap while holding a wrapper of its
//! own, and moves a wrapper between the heap and a second collection. Rust
//! forbids that aliasing.
//!
//! This port therefore keeps the wrappers in one array owned by the caller and
//! stores **positions into that array** in the heap. Every operation that needs
//! to compare doc IDs receives the array, so the ordering is computed from
//! exactly the same values Java reads through its references. The heap layout,
//! the sift-up and sift-down loops, the bulk heapify and the `topList`
//! traversal order are unchanged, which matters because the order in which the
//! top list is walked decides the order in which sub-scores are summed.

#![deny(unsafe_code)]

use crate::search::disi_wrapper::DisiWrapper;

/// Returns the position of the left child of `node`.
///
/// Equivalent to the package-private `DisiPriorityQueueN.leftNode(int)`.
pub fn left_node(node: usize) -> usize {
    ((node + 1) << 1) - 1
}

/// Returns the position of the right sibling of `left_node`.
///
/// Equivalent to the package-private `DisiPriorityQueueN.rightNode(int)`.
pub fn right_node(left_node: usize) -> usize {
    left_node + 1
}

/// Returns the position of the parent of `node`, or `-1` for the root.
///
/// Equivalent to the package-private `DisiPriorityQueueN.parentNode(int)`,
/// which returns `-1` for node `0`; the signed return type spells that out.
pub fn parent_node(node: usize) -> i32 {
    (((node + 1) >> 1) as i32) - 1
}

/// A priority queue of [`DisiWrapper`]s that orders by current doc ID.
///
/// Equivalent to the `abstract sealed class
/// org.apache.lucene.search.DisiPriorityQueue`, which permits exactly
/// [`DisiPriorityQueue2`] and [`DisiPriorityQueueN`]. The specialisation exists
/// because the pluggable comparison function of the general-purpose
/// [`PriorityQueue`](crate::util::PriorityQueue) makes rebalancing slow.
///
/// Rust has no `sealed` keyword; the enum reproduces it exactly, since no
/// implementation outside this module can be added.
#[derive(Debug)]
pub enum DisiPriorityQueue {
    /// The specialisation for two entries or less.
    Two(DisiPriorityQueue2),
    /// The general heap.
    N(DisiPriorityQueueN),
}

impl DisiPriorityQueue {
    /// Creates a queue of the given maximum size.
    ///
    /// Equivalent to `DisiPriorityQueue.ofMaxSize(int)`.
    pub fn of_max_size(max_size: usize) -> Self {
        if max_size <= 2 {
            Self::Two(DisiPriorityQueue2::new())
        } else {
            Self::N(DisiPriorityQueueN::new(max_size))
        }
    }

    /// Returns the number of entries in this heap.
    ///
    /// Equivalent to `DisiPriorityQueue.size()`.
    pub fn size(&self) -> usize {
        match self {
            Self::Two(q) => q.size(),
            Self::N(q) => q.size(),
        }
    }

    /// Returns the position of the top value in this heap, or `None` if the
    /// heap is empty.
    ///
    /// Equivalent to `DisiPriorityQueue.top()`.
    pub fn top(&self) -> Option<usize> {
        match self {
            Self::Two(q) => q.top(),
            Self::N(q) => q.top(),
        }
    }

    /// Returns the position of the 2nd least value in this heap, or `None` if
    /// the heap holds less than 2 values.
    ///
    /// Equivalent to `DisiPriorityQueue.top2()`.
    pub fn top2(&self, wrappers: &[DisiWrapper]) -> Option<usize> {
        match self {
            Self::Two(q) => q.top2(),
            Self::N(q) => q.top2(wrappers),
        }
    }

    /// Returns the positions of the scorers which are on the current doc, in
    /// the order in which Java's linked list is traversed.
    ///
    /// Equivalent to `DisiPriorityQueue.topList()`.
    pub fn top_list(&self, wrappers: &[DisiWrapper]) -> Vec<usize> {
        match self {
            Self::Two(q) => q.top_list(wrappers),
            Self::N(q) => q.top_list(wrappers),
        }
    }

    /// Adds an entry to this queue and returns the position of the new top
    /// entry.
    ///
    /// Equivalent to `DisiPriorityQueue.add(DisiWrapper)`.
    ///
    /// # Panics
    ///
    /// Panics when the queue is already full, which is the misuse Java reports
    /// with an `IllegalStateException` or an `ArrayIndexOutOfBoundsException`.
    pub fn add(&mut self, wrappers: &[DisiWrapper], entry: usize) -> usize {
        match self {
            Self::Two(q) => q.add(wrappers, entry),
            Self::N(q) => q.add(wrappers, entry),
        }
    }

    /// Bulk add.
    ///
    /// Equivalent to `DisiPriorityQueue.addAll(DisiWrapper[], int, int)`; the
    /// Java array plus offset and length become a slice.
    ///
    /// # Panics
    ///
    /// Panics when the entries do not fit, which is the
    /// `IndexOutOfBoundsException` Java throws.
    pub fn add_all(&mut self, wrappers: &[DisiWrapper], entries: &[usize]) {
        match self {
            Self::Two(q) => q.add_all(wrappers, entries),
            Self::N(q) => q.add_all(wrappers, entries),
        }
    }

    /// Removes the top entry and returns its position.
    ///
    /// Equivalent to `DisiPriorityQueue.pop()`; the wrapper array is passed in
    /// because the sift-down that follows the removal compares doc IDs.
    pub fn pop(&mut self, wrappers: &[DisiWrapper]) -> Option<usize> {
        match self {
            Self::Two(q) => q.pop(),
            Self::N(q) => q.pop(wrappers),
        }
    }

    /// Rebalances this heap and returns the position of the top entry.
    ///
    /// Equivalent to `DisiPriorityQueue.updateTop()`.
    pub fn update_top(&mut self, wrappers: &[DisiWrapper]) -> Option<usize> {
        match self {
            Self::Two(q) => q.update_top(wrappers),
            Self::N(q) => q.update_top(wrappers),
        }
    }

    /// Replaces the top entry with `replacement`, rebalances the heap and
    /// returns the position of the new top entry.
    ///
    /// Equivalent to the package-private
    /// `DisiPriorityQueue.updateTop(DisiWrapper)`.
    pub fn update_top_with(
        &mut self,
        wrappers: &[DisiWrapper],
        replacement: usize,
    ) -> Option<usize> {
        match self {
            Self::Two(q) => q.update_top_with(wrappers, replacement),
            Self::N(q) => q.update_top_with(wrappers, replacement),
        }
    }

    /// Clears the heap.
    ///
    /// Equivalent to `DisiPriorityQueue.clear()`.
    pub fn clear(&mut self) {
        match self {
            Self::Two(q) => q.clear(),
            Self::N(q) => q.clear(),
        }
    }

    /// Returns the entries of this heap in heap-array order.
    ///
    /// Equivalent to `DisiPriorityQueue.iterator()`, which Java inherits from
    /// `Iterable<DisiWrapper>` and which walks the backing array.
    pub fn entries(&self) -> &[usize] {
        match self {
            Self::Two(q) => q.entries(),
            Self::N(q) => q.entries(),
        }
    }
}

/// A [`DisiPriorityQueue`] of two entries or less.
///
/// Equivalent to the `final class org.apache.lucene.search.DisiPriorityQueue2`,
/// whose `top` and `top2` fields become positions `0` and `1` of a two-slot
/// array here.
#[derive(Debug, Default)]
pub struct DisiPriorityQueue2 {
    heap: Vec<usize>,
}

impl DisiPriorityQueue2 {
    /// Creates an empty queue.
    ///
    /// Equivalent to `new DisiPriorityQueue2()`.
    pub fn new() -> Self {
        Self {
            heap: Vec::with_capacity(2),
        }
    }

    /// Returns the number of entries.
    ///
    /// Equivalent to `DisiPriorityQueue2.size()`.
    pub fn size(&self) -> usize {
        self.heap.len()
    }

    /// Returns the top entry, or `None` when empty.
    ///
    /// Equivalent to `DisiPriorityQueue2.top()`.
    pub fn top(&self) -> Option<usize> {
        self.heap.first().copied()
    }

    /// Returns the second entry, or `None` when there are fewer than two.
    ///
    /// Equivalent to `DisiPriorityQueue2.top2()`.
    pub fn top2(&self) -> Option<usize> {
        self.heap.get(1).copied()
    }

    /// Returns the entries on the current doc, in traversal order.
    ///
    /// Equivalent to `DisiPriorityQueue2.topList()`.
    pub fn top_list(&self, wrappers: &[DisiWrapper]) -> Vec<usize> {
        let mut order = Vec::with_capacity(2);
        if let Some(top) = self.top() {
            order.push(top);
            if let Some(top2) = self.top2() {
                if wrappers[top].doc == wrappers[top2].doc {
                    order.push(top2);
                }
            }
        }
        order.reverse();
        order
    }

    /// Adds an entry and returns the new top.
    ///
    /// Equivalent to `DisiPriorityQueue2.add(DisiWrapper)`.
    ///
    /// # Panics
    ///
    /// Panics when a third element is added, matching the
    /// `IllegalStateException` Java throws.
    pub fn add(&mut self, wrappers: &[DisiWrapper], entry: usize) -> usize {
        assert!(
            self.heap.len() < 2,
            "Trying to add a 3rd element to a DisiPriorityQueue configured with a max size of 2"
        );
        self.heap.push(entry);
        if self.heap.len() == 1 {
            entry
        } else {
            self.update_top(wrappers)
                .expect("INVARIANT: the heap holds two entries")
        }
    }

    /// Bulk add.
    ///
    /// Equivalent to `DisiPriorityQueue.addAll(DisiWrapper[], int, int)`, which
    /// `DisiPriorityQueue2` inherits unchanged.
    pub fn add_all(&mut self, wrappers: &[DisiWrapper], entries: &[usize]) {
        for &entry in entries {
            self.add(wrappers, entry);
        }
    }

    /// Removes and returns the top entry.
    ///
    /// Equivalent to `DisiPriorityQueue2.pop()`.
    pub fn pop(&mut self) -> Option<usize> {
        if self.heap.is_empty() {
            None
        } else {
            Some(self.heap.remove(0))
        }
    }

    /// Rebalances and returns the top entry.
    ///
    /// Equivalent to `DisiPriorityQueue2.updateTop()`.
    pub fn update_top(&mut self, wrappers: &[DisiWrapper]) -> Option<usize> {
        if self.heap.len() == 2 && wrappers[self.heap[1]].doc < wrappers[self.heap[0]].doc {
            self.heap.swap(0, 1);
        }
        self.top()
    }

    /// Replaces the top entry and rebalances.
    ///
    /// Equivalent to the package-private
    /// `DisiPriorityQueue2.updateTop(DisiWrapper)`.
    pub fn update_top_with(
        &mut self,
        wrappers: &[DisiWrapper],
        replacement: usize,
    ) -> Option<usize> {
        if self.heap.is_empty() {
            self.heap.push(replacement);
        } else {
            self.heap[0] = replacement;
        }
        self.update_top(wrappers)
    }

    /// Clears the heap.
    ///
    /// Equivalent to `DisiPriorityQueue2.clear()`.
    pub fn clear(&mut self) {
        self.heap.clear();
    }

    /// Returns the entries in heap-array order.
    ///
    /// Equivalent to `DisiPriorityQueue2.iterator()`.
    pub fn entries(&self) -> &[usize] {
        &self.heap
    }
}

/// The general [`DisiPriorityQueue`].
///
/// Equivalent to the `final class
/// org.apache.lucene.search.DisiPriorityQueueN`.
#[derive(Debug)]
pub struct DisiPriorityQueueN {
    heap: Vec<usize>,
    max_size: usize,
}

impl DisiPriorityQueueN {
    /// Creates an empty queue of the given maximum size.
    ///
    /// Equivalent to `new DisiPriorityQueueN(int)`.
    pub fn new(max_size: usize) -> Self {
        Self {
            heap: Vec::with_capacity(max_size),
            max_size,
        }
    }

    /// Returns the number of entries.
    ///
    /// Equivalent to `DisiPriorityQueueN.size()`.
    pub fn size(&self) -> usize {
        self.heap.len()
    }

    /// Returns the top entry, or `None` when empty.
    ///
    /// Equivalent to `DisiPriorityQueueN.top()`, which reads `heap[0]`.
    pub fn top(&self) -> Option<usize> {
        self.heap.first().copied()
    }

    /// Returns the second least entry.
    ///
    /// Equivalent to `DisiPriorityQueueN.top2()`.
    pub fn top2(&self, wrappers: &[DisiWrapper]) -> Option<usize> {
        match self.heap.len() {
            0 | 1 => None,
            2 => Some(self.heap[1]),
            _ => {
                if wrappers[self.heap[1]].doc <= wrappers[self.heap[2]].doc {
                    Some(self.heap[1])
                } else {
                    Some(self.heap[2])
                }
            }
        }
    }

    /// Returns the entries on the current doc, in traversal order.
    ///
    /// Equivalent to `DisiPriorityQueueN.topList()`. Java builds a linked list
    /// by prepending, so the traversal starts at the last entry prepended; the
    /// vector returned here is in exactly that order.
    pub fn top_list(&self, wrappers: &[DisiWrapper]) -> Vec<usize> {
        let size = self.heap.len();
        let mut order = Vec::with_capacity(size);
        if size == 0 {
            return order;
        }
        let doc = wrappers[self.heap[0]].doc;
        order.push(self.heap[0]);
        if size >= 3 {
            self.collect_top_list(wrappers, doc, 1, &mut order);
            self.collect_top_list(wrappers, doc, 2, &mut order);
        } else if size == 2 && wrappers[self.heap[1]].doc == doc {
            order.push(self.heap[1]);
        }
        order.reverse();
        order
    }

    /// Equivalent to the private
    /// `DisiPriorityQueueN.topList(DisiWrapper, DisiWrapper[], int, int)`.
    fn collect_top_list(
        &self,
        wrappers: &[DisiWrapper],
        doc: i32,
        i: usize,
        order: &mut Vec<usize>,
    ) {
        let size = self.heap.len();
        let w = self.heap[i];
        if wrappers[w].doc == doc {
            order.push(w);
            let left = left_node(i);
            let right = right_node(left);
            if right < size {
                self.collect_top_list(wrappers, doc, left, order);
                self.collect_top_list(wrappers, doc, right, order);
            } else if left < size && wrappers[self.heap[left]].doc == doc {
                order.push(self.heap[left]);
            }
        }
    }

    /// Adds an entry and returns the new top.
    ///
    /// Equivalent to `DisiPriorityQueueN.add(DisiWrapper)`.
    ///
    /// # Panics
    ///
    /// Panics when the queue is full, which is the
    /// `ArrayIndexOutOfBoundsException` Java raises.
    pub fn add(&mut self, wrappers: &[DisiWrapper], entry: usize) -> usize {
        assert!(
            self.heap.len() < self.max_size,
            "Cannot add an element to a queue with no remaining capacity"
        );
        self.heap.push(entry);
        let last = self.heap.len() - 1;
        self.up_heap(wrappers, last);
        self.heap[0]
    }

    /// Bulk add, heapifying in one pass.
    ///
    /// Equivalent to `DisiPriorityQueueN.addAll(DisiWrapper[], int, int)`.
    ///
    /// # Panics
    ///
    /// Panics when the entries do not fit, matching Java's
    /// `IndexOutOfBoundsException`.
    pub fn add_all(&mut self, wrappers: &[DisiWrapper], entries: &[usize]) {
        // Nothing to do if empty:
        if entries.is_empty() {
            return;
        }

        // Fail early if we're going to over-fill:
        assert!(
            self.heap.len() + entries.len() <= self.max_size,
            "Cannot add {} elements to a queue with remaining capacity {}",
            entries.len(),
            self.max_size - self.heap.len()
        );

        // Copy the entries over to our heap array:
        self.heap.extend_from_slice(entries);
        let size = self.heap.len();

        // Heapify in bulk:
        let first_leaf_index = size >> 1;
        for root_index in (0..first_leaf_index).rev() {
            let mut parent_index = root_index;
            let parent = self.heap[parent_index];
            while parent_index < first_leaf_index {
                let mut child_index = left_node(parent_index);
                let right_child_index = right_node(child_index);
                let mut child = self.heap[child_index];
                if right_child_index < size
                    && wrappers[self.heap[right_child_index]].doc < wrappers[child].doc
                {
                    child = self.heap[right_child_index];
                    child_index = right_child_index;
                }
                if wrappers[child].doc >= wrappers[parent].doc {
                    break;
                }
                self.heap[parent_index] = child;
                parent_index = child_index;
            }
            self.heap[parent_index] = parent;
        }
    }

    /// Removes and returns the top entry.
    ///
    /// Equivalent to `DisiPriorityQueueN.pop()`; the wrapper array is passed in
    /// because the sift-down that follows the removal compares doc IDs.
    pub fn pop(&mut self, wrappers: &[DisiWrapper]) -> Option<usize> {
        if self.heap.is_empty() {
            return None;
        }
        // `heap[0] = heap[--size]` followed by `downHeap(size)`; `swap_remove`
        // performs the very same move.
        let result = self.heap.swap_remove(0);
        self.down_heap(wrappers);
        Some(result)
    }

    /// Rebalances and returns the top entry.
    ///
    /// Equivalent to `DisiPriorityQueueN.updateTop()`.
    pub fn update_top(&mut self, wrappers: &[DisiWrapper]) -> Option<usize> {
        self.down_heap(wrappers);
        self.top()
    }

    /// Replaces the top entry and rebalances.
    ///
    /// Equivalent to the package-private
    /// `DisiPriorityQueueN.updateTop(DisiWrapper)`.
    pub fn update_top_with(
        &mut self,
        wrappers: &[DisiWrapper],
        replacement: usize,
    ) -> Option<usize> {
        if self.heap.is_empty() {
            self.heap.push(replacement);
        } else {
            self.heap[0] = replacement;
        }
        self.update_top(wrappers)
    }

    /// Clears the heap.
    ///
    /// Equivalent to `DisiPriorityQueueN.clear()`.
    pub fn clear(&mut self) {
        self.heap.clear();
    }

    /// Returns the entries in heap-array order.
    ///
    /// Equivalent to `DisiPriorityQueueN.iterator()`.
    pub fn entries(&self) -> &[usize] {
        &self.heap
    }

    /// Equivalent to the package-private `DisiPriorityQueueN.upHeap(int)`.
    fn up_heap(&mut self, wrappers: &[DisiWrapper], mut i: usize) {
        let node = self.heap[i];
        let node_doc = wrappers[node].doc;
        let mut j = parent_node(i);
        while j >= 0 && node_doc < wrappers[self.heap[j as usize]].doc {
            self.heap[i] = self.heap[j as usize];
            i = j as usize;
            j = parent_node(i);
        }
        self.heap[i] = node;
    }

    /// Equivalent to the package-private `DisiPriorityQueueN.downHeap(int)`,
    /// called with the current size.
    fn down_heap(&mut self, wrappers: &[DisiWrapper]) {
        let size = self.heap.len();
        if size == 0 {
            return;
        }
        let mut i = 0;
        let node = self.heap[0];
        let node_doc = wrappers[node].doc;
        let mut j = left_node(i);
        if j < size {
            let mut k = right_node(j);
            if k < size && wrappers[self.heap[k]].doc < wrappers[self.heap[j]].doc {
                j = k;
            }
            if wrappers[self.heap[j]].doc < node_doc {
                loop {
                    self.heap[i] = self.heap[j];
                    i = j;
                    j = left_node(i);
                    k = right_node(j);
                    if k < size && wrappers[self.heap[k]].doc < wrappers[self.heap[j]].doc {
                        j = k;
                    }
                    if !(j < size && wrappers[self.heap[j]].doc < node_doc) {
                        break;
                    }
                }
                self.heap[i] = node;
            }
        }
    }
}
