// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.coll.UVector64`.

/// A growable array of `long` values.
///
/// Equivalent to `com.ibm.icu.impl.coll.UVector64`: same initial capacity
/// (32) and growth policy, so [`get_buffer`](Self::get_buffer) exposes the
/// same capacity as Java's `getBuffer()`.
#[derive(Debug, Clone)]
pub struct UVector64 {
    buffer: Vec<i64>,
    length: i32,
}

impl Default for UVector64 {
    fn default() -> Self {
        Self::new()
    }
}

impl UVector64 {
    /// `UVector64()`.
    pub fn new() -> Self {
        Self {
            buffer: vec![0; 32],
            length: 0,
        }
    }

    /// `isEmpty()`.
    pub fn is_empty(&self) -> bool {
        self.length == 0
    }

    /// `size()`.
    pub fn size(&self) -> i32 {
        self.length
    }

    /// `elementAti(int)`.
    pub fn element_ati(&self, i: i32) -> i64 {
        self.buffer[i as usize]
    }

    /// `getBuffer()`: the whole backing array (its length is the capacity).
    pub fn get_buffer(&self) -> &[i64] {
        &self.buffer
    }

    /// `addElement(long)`.
    pub fn add_element(&mut self, e: i64) {
        self.ensure_append_capacity();
        self.buffer[self.length as usize] = e;
        self.length += 1;
    }

    /// `setElementAt(long, int)`.
    pub fn set_element_at(&mut self, elem: i64, index: i32) {
        self.buffer[index as usize] = elem;
    }

    /// `insertElementAt(long, int)`.
    pub fn insert_element_at(&mut self, elem: i64, index: i32) {
        self.ensure_append_capacity();
        let (index, length) = (index as usize, self.length as usize);
        self.buffer.copy_within(index..length, index + 1);
        self.buffer[index] = elem;
        self.length += 1;
    }

    /// `removeAllElements()`.
    pub fn remove_all_elements(&mut self) {
        self.length = 0;
    }

    fn ensure_append_capacity(&mut self) {
        if self.length as usize >= self.buffer.len() {
            let len = self.buffer.len();
            let new_capacity = if len <= 0xffff { 4 * len } else { 2 * len };
            self.buffer.resize(new_capacity, 0);
        }
    }
}
