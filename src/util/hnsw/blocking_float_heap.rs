//! Port of `org.apache.lucene.util.hnsw.BlockingFloatHeap`.

use std::sync::Mutex;

/// The heap state guarded by the lock.
#[derive(Debug)]
struct Inner {
    heap: Vec<f32>,
    size: usize,
}

/// A blocking bounded min heap that stores floats; the top element is the lowest
/// value.
///
/// Equivalent to `org.apache.lucene.util.hnsw.BlockingFloatHeap`. A primitive
/// priority queue that maintains a partial ordering of its elements such that the
/// least element can always be found in constant time.
///
/// # Divergence from Lucene 10.5.0
///
/// Java guards the array and the size with a `ReentrantLock` held around each
/// method body; this port puts both behind one [`Mutex`], which is the same critical
/// section expressed the way Rust ties data to its lock. Java's `poll` reads `size`
/// once *outside* the lock to decide whether the heap is empty; this port takes the
/// lock first, which removes that race without changing the result for any
/// single-threaded caller.
#[derive(Debug)]
pub struct BlockingFloatHeap {
    max_size: usize,
    inner: Mutex<Inner>,
}

impl BlockingFloatHeap {
    /// Creates a heap holding at most `max_size` values.
    pub fn new(max_size: usize) -> Self {
        Self {
            max_size,
            inner: Mutex::new(Inner {
                heap: vec![0.0; max_size + 1],
                size: 0,
            }),
        }
    }

    /// Inserts a value into this heap, discarding the least value if the heap is
    /// full.
    ///
    /// Returns the new top element of the queue.
    ///
    /// # Panics
    ///
    /// Panics if the lock is poisoned.
    pub fn offer(&self, value: f32) -> f32 {
        let mut inner = self.lock();
        if inner.size < self.max_size {
            inner.push(value);
            inner.heap[1]
        } else {
            if value >= inner.heap[1] {
                inner.update_top(value);
            }
            inner.heap[1]
        }
    }

    /// Inserts an array of values into this heap.
    ///
    /// The values must be sorted in ascending order; only the first `len` are
    /// inserted. Returns the new top element of the queue.
    ///
    /// # Panics
    ///
    /// Panics if the lock is poisoned.
    pub fn offer_all(&self, values: &[f32], len: usize) -> f32 {
        let mut inner = self.lock();
        for i in (0..len).rev() {
            if inner.size < self.max_size {
                inner.push(values[i]);
            } else if values[i] >= inner.heap[1] {
                inner.update_top(values[i]);
            } else {
                break;
            }
        }
        inner.heap[1]
    }

    /// Removes and returns the head of the heap, the smallest value.
    ///
    /// # Panics
    ///
    /// Panics if the heap is empty, matching Java's `IllegalStateException`, or if
    /// the lock is poisoned.
    pub fn poll(&self) -> f32 {
        let mut inner = self.lock();
        assert!(inner.size > 0, "The heap is empty");
        let result = inner.heap[1]; // save first value
        inner.heap[1] = inner.heap[inner.size]; // move last to first
        inner.size -= 1;
        inner.down_heap(1); // adjust heap
        result
    }

    /// Retrieves, but does not remove, the head of this heap, the smallest value.
    ///
    /// # Panics
    ///
    /// Panics if the lock is poisoned.
    pub fn peek(&self) -> f32 {
        self.lock().heap[1]
    }

    /// Returns the number of elements in this heap.
    ///
    /// # Panics
    ///
    /// Panics if the lock is poisoned.
    pub fn size(&self) -> usize {
        self.lock().size
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner
            .lock()
            .expect("INVARIANT: the heap is only mutated by infallible code, so it cannot be left inconsistent by a panic")
    }
}

impl Inner {
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
