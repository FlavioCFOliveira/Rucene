/*
 * Copyright 2005 Google Inc.
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

//! The types declared in `MutableInteger.java`, of the Java package
//! `com.google.common.geometry`.

use std::fmt;
use std::hash::{Hash, Hasher};

/// Equivalent to `com.google.common.geometry.MutableInteger` (`public
/// class`): an `int` that can be changed in place. Not thread-safe.
///
/// Java caches the boxed `Integer` of the value (`cachedIntegerValue`); a
/// Rust `i32` needs no box, so [`integer_value`](Self::integer_value) returns
/// the value itself, which is what the cached `Integer` holds.
#[derive(Clone, Copy, Debug, Eq)]
pub struct MutableInteger {
    value: i32,
}

impl MutableInteger {
    /// `MutableInteger(int)`.
    pub fn new(i: i32) -> Self {
        Self { value: i }
    }

    /// `intValue()`.
    pub fn int_value(&self) -> i32 {
        self.value
    }

    /// `integerValue()`.
    pub fn integer_value(&self) -> i32 {
        self.value
    }

    /// `setValue(int)`.
    pub fn set_value(&mut self, value: i32) {
        self.value = value;
    }

    /// `increment()`.
    pub fn increment(&mut self) {
        self.add(1);
    }

    /// `add(int)`: Java `int` arithmetic, wrapping on overflow.
    pub fn add(&mut self, amount: i32) {
        self.set_value(self.value.wrapping_add(amount));
    }

    /// `decrement()`.
    pub fn decrement(&mut self) {
        self.subtract(1);
    }

    /// `subtract(int)`: `add(amount * -1)`.
    pub fn subtract(&mut self, amount: i32) {
        self.add(amount.wrapping_mul(-1));
    }

    /// `hashCode()`: `Integer.hashCode()`, the value.
    pub fn hash_code(&self) -> i32 {
        self.integer_value()
    }
}

impl PartialEq for MutableInteger {
    /// `equals(Object)`.
    fn eq(&self, other: &Self) -> bool {
        other.value == self.value
    }
}

impl Hash for MutableInteger {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.hash_code().hash(state);
    }
}

impl fmt::Display for MutableInteger {
    /// `toString()`: `String.valueOf(value)`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.value)
    }
}
