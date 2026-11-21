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

//! The type declared in `ByteVector.java`, of the Java package
//! `org.apache.lucene.analysis.compound.hyphenation`.

/// This class implements a simple byte vector with access to the underlying
/// array. This class has been taken from the Apache FOP project
/// (<http://xmlgraphics.apache.org/fop/>). They have been slightly modified.
///
/// An index past the end of the array panics where Java throws
/// `ArrayIndexOutOfBoundsException`.
///
/// Equivalent to `org.apache.lucene.analysis.compound.hyphenation.ByteVector`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ByteVector {
    block_size: usize,
    /// The encapsulated array
    array: Vec<i8>,
    /// Points to next free item
    n: usize,
}

/// Capacity increment size
const DEFAULT_BLOCK_SIZE: usize = 2048;

impl Default for ByteVector {
    fn default() -> Self {
        Self::new()
    }
}

impl ByteVector {
    /// Equivalent to `ByteVector()`.
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_BLOCK_SIZE as i32)
    }

    /// Equivalent to `ByteVector(int capacity)`.
    pub fn with_capacity(capacity: i32) -> Self {
        let block_size = if capacity > 0 {
            capacity as usize
        } else {
            DEFAULT_BLOCK_SIZE
        };
        Self {
            block_size,
            array: vec![0; block_size],
            n: 0,
        }
    }

    /// Equivalent to `ByteVector(byte[] a)`.
    pub fn from_array(a: Vec<i8>) -> Self {
        let n = 0;
        Self {
            block_size: DEFAULT_BLOCK_SIZE,
            array: a,
            n,
        }
    }

    /// Equivalent to `ByteVector(byte[] a, int capacity)`.
    pub fn from_array_with_capacity(a: Vec<i8>, capacity: i32) -> Self {
        let block_size = if capacity > 0 {
            capacity as usize
        } else {
            DEFAULT_BLOCK_SIZE
        };
        let n = 0;
        Self {
            block_size,
            array: a,
            n,
        }
    }

    /// Equivalent to `ByteVector.getArray()`.
    pub fn get_array(&self) -> &[i8] {
        &self.array
    }

    /// Equivalent to `ByteVector.getArray()`, for writing into the array.
    pub fn get_array_mut(&mut self) -> &mut [i8] {
        &mut self.array
    }

    /// return number of items in array
    ///
    /// Equivalent to `ByteVector.length()`.
    pub fn length(&self) -> usize {
        self.n
    }

    /// returns current capacity of array
    ///
    /// Equivalent to `ByteVector.capacity()`.
    pub fn capacity(&self) -> usize {
        self.array.len()
    }

    /// Equivalent to `ByteVector.put(int, byte)`.
    pub fn put(&mut self, index: usize, val: i8) {
        self.array[index] = val;
    }

    /// Equivalent to `ByteVector.get(int)`.
    pub fn get(&self, index: usize) -> i8 {
        self.array[index]
    }

    /// This is to implement memory allocation in the array. Like malloc().
    ///
    /// Equivalent to `ByteVector.alloc(int)`: the array grows by one block when
    /// `n + size` reaches its length, whatever `size`.
    pub fn alloc(&mut self, size: usize) -> usize {
        let index = self.n;
        let len = self.array.len();
        if self.n + size >= len {
            let mut aux = vec![0; len + self.block_size];
            aux[..len].copy_from_slice(&self.array[..len]);
            self.array = aux;
        }
        self.n += size;
        index
    }

    /// Equivalent to `ByteVector.trimToSize()`.
    pub fn trim_to_size(&mut self) {
        if self.n < self.array.len() {
            let aux = self.array[..self.n].to_vec();
            self.array = aux;
        }
    }
}
