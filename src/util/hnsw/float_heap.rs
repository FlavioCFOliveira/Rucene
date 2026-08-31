//! Port of `org.apache.lucene.util.hnsw.FloatHeap`.

/// A bounded min heap that stores floats; the top element is the lowest value.
///
/// Equivalent to `org.apache.lucene.util.hnsw.FloatHeap`. A primitive priority queue
/// that maintains a partial ordering of its elements such that the least element can
/// always be found in constant time. The implementation is based on
/// [`LongHeap`](crate::util::LongHeap).
#[derive(Clone, Debug)]
pub struct FloatHeap {
    max_size: usize,
    heap: Vec<f32>,
    size: usize,
}

impl FloatHeap {
    /// Creates a heap holding at most `max_size` values.
    pub fn new(max_size: usize) -> Self {
        Self {
            max_size,
            heap: vec![0.0; max_size + 1],
            size: 0,
        }
    }

    /// Inserts a value into this heap.
    ///
    /// If the number of values would exceed the heap's `max_size`, the least value is
    /// discarded. Returns whether the value was added; it is not added when the heap
    /// is full and the new value is less than the top value.
    pub fn offer(&mut self, value: f32) -> bool {
        if self.size >= self.max_size {
            if value < self.heap[1] {
                return false;
            }
            self.update_top(value);
            return true;
        }
        self.push(value);
        true
    }

    /// Returns the values held by this heap, in heap order.
    pub fn get_heap(&self) -> Vec<f32> {
        self.heap[1..1 + self.size].to_vec()
    }

    /// Removes and returns the head of the heap, the smallest value.
    ///
    /// # Panics
    ///
    /// Panics if the heap is empty, matching Java's `IllegalStateException`.
    pub fn poll(&mut self) -> f32 {
        assert!(self.size > 0, "The heap is empty");
        let result = self.heap[1]; // save first value
        self.heap[1] = self.heap[self.size]; // move last to first
        self.size -= 1;
        self.down_heap(1); // adjust heap
        result
    }

    /// Retrieves, but does not remove, the head of this heap, the smallest value.
    pub fn peek(&self) -> f32 {
        self.heap[1]
    }

    /// Returns the number of elements in this heap.
    pub fn size(&self) -> usize {
        self.size
    }

    /// Removes every element from this heap.
    pub fn clear(&mut self) {
        self.size = 0;
    }

    fn push(&mut self, element: f32) {
        self.size += 1;
        self.heap[self.size] = element;
        self.up_heap(self.size);
    }

    fn update_top(&mut self, value: f32) -> f32 {
        self.heap[1] = value;
        self.down_heap(1);
        self.heap[1]
    }

    fn down_heap(&mut self, i: usize) {
        let mut i = i;
        let value = self.heap[i]; // save top value
        let mut j = i << 1; // find smaller child
        let mut k = j + 1;
        if k <= self.size && self.heap[k] < self.heap[j] {
            j = k;
        }
        while j <= self.size && self.heap[j] < value {
            self.heap[i] = self.heap[j]; // shift up child
            i = j;
            j = i << 1;
            k = j + 1;
            if k <= self.size && self.heap[k] < self.heap[j] {
                j = k;
            }
        }
        self.heap[i] = value; // install saved value
    }

    fn up_heap(&mut self, orig_pos: usize) {
        let mut i = orig_pos;
        let value = self.heap[i]; // save bottom value
        let mut j = i >> 1;
        while j > 0 && value < self.heap[j] {
            self.heap[i] = self.heap[j]; // shift parents down
            i = j;
            j >>= 1;
        }
        self.heap[i] = value; // install saved value
    }
}
