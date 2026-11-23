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

//! Port of `FacetLabel.java`, of the Java package
//! `org.apache.lucene.facet.taxonomy`.

use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use rucene::error::{LuceneError, Result};
use rucene::util::byte_block_pool::BYTE_BLOCK_SIZE;
use rucene::util::java_collections::{java_compare_to, java_string_hash_code};

/// Holds a sequence of string components, specifying the hierarchical name of
/// a category.
///
/// Equivalent to `org.apache.lucene.facet.taxonomy.FacetLabel`. Java's
/// `components` array is shared between a label and the prefixes
/// [`subpath`](Self::subpath) makes of it; the port shares it through an
/// [`Arc`].
#[derive(Clone)]
pub struct FacetLabel {
    /// The components of this `FacetLabel`. Note that this array may be shared
    /// with other `FacetLabel` instances, e.g. as a result of
    /// [`subpath`](Self::subpath), therefore you should traverse the array up
    /// to [`length`](Self::length) for this path's components.
    pub components: Arc<[String]>,
    /// The number of components of this `FacetLabel`.
    pub length: usize,
}

/// `String.length()`: the number of UTF-16 code units.
pub(crate) fn java_length(s: &str) -> usize {
    s.encode_utf16().count()
}

/// `Arrays.toString(Object[])` over strings.
pub(crate) fn arrays_to_string<S: AsRef<str>>(values: &[S]) -> String {
    let mut out = String::from("[");
    for (i, value) in values.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        out.push_str(value.as_ref());
    }
    out.push(']');
    out
}

/// `String.substring(0, end)` in UTF-16 code units.
pub(crate) fn java_substring_to(s: &str, end: usize) -> Result<String> {
    let units: Vec<u16> = s.encode_utf16().collect();
    if end > units.len() {
        return Err(LuceneError::unchecked(
            "java.lang.StringIndexOutOfBoundsException",
            format!("begin 0, end {end}, length {}", units.len()),
        ));
    }
    Ok(String::from_utf16_lossy(&units[..end]))
}

impl FacetLabel {
    /// The maximum number of characters a `FacetLabel` can have.
    ///
    /// Equivalent to `FacetLabel.MAX_CATEGORY_PATH_LENGTH`.
    pub const MAX_CATEGORY_PATH_LENGTH: usize = (BYTE_BLOCK_SIZE - 2) / 4;

    /// Equivalent to the private `FacetLabel(FacetLabel, int)`.
    fn prefix_of(copy_from: &FacetLabel, prefix_len: usize) -> Self {
        // `FacetLabel.java:52-58`.
        debug_assert!(
            prefix_len <= copy_from.components.len(),
            "prefixLen cannot be negative nor larger than the given components' length: prefixLen={} components.length={}",
            prefix_len,
            copy_from.components.len()
        );
        Self {
            components: Arc::clone(&copy_from.components),
            length: prefix_len,
        }
    }

    /// Construct from the given path components.
    ///
    /// Equivalent to `FacetLabel(String...)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for an empty component or a path longer
    /// than [`MAX_CATEGORY_PATH_LENGTH`](Self::MAX_CATEGORY_PATH_LENGTH).
    pub fn new<S: Into<String>>(components: impl IntoIterator<Item = S>) -> Result<Self> {
        let components: Arc<[String]> = components.into_iter().map(Into::into).collect();
        let length = components.len();
        let label = Self { components, length };
        label.check_components()?;
        Ok(label)
    }

    /// Construct from the given nullable path components.
    ///
    /// Equivalent to `FacetLabel(String...)` with Java's nullable `String`:
    /// `None` is Java's `null`, rejected by `checkComponents()` with the
    /// components printed by `Arrays.toString` (`null` for `None`).
    ///
    /// # Errors
    ///
    /// As [`new`](Self::new); also `IllegalArgumentException` for a `null`
    /// component.
    pub fn new_nullable(components: &[Option<&str>]) -> Result<Self> {
        // `checkComponents()` (`FacetLabel.java:76-83`) throws at the first
        // null or empty component, before the length check.
        if components.iter().any(|c| c.is_none_or(str::is_empty)) {
            return Err(LuceneError::illegal_argument(format!(
                "empty or null components not allowed: {}",
                arrays_to_string(
                    &components
                        .iter()
                        .map(|c| c.unwrap_or("null"))
                        .collect::<Vec<&str>>()
                )
            )));
        }
        Self::new(components.iter().flatten().copied())
    }

    /// Construct from the dimension plus the given path components.
    ///
    /// Equivalent to `FacetLabel(String, String[])`.
    ///
    /// # Errors
    ///
    /// As [`new`](Self::new).
    pub fn with_dim(dim: &str, path: &[impl AsRef<str>]) -> Result<Self> {
        let mut components = Vec::with_capacity(1 + path.len());
        components.push(dim.to_string());
        components.extend(path.iter().map(|p| p.as_ref().to_string()));
        Self::new(components)
    }

    /// The empty label, `new FacetLabel()`.
    pub fn empty() -> Self {
        Self {
            components: Arc::from(Vec::<String>::new()),
            length: 0,
        }
    }

    /// Equivalent to `checkComponents()`.
    fn check_components(&self) -> Result<()> {
        let mut len: i64 = 0;
        for comp in self.components.iter() {
            if comp.is_empty() {
                return Err(LuceneError::illegal_argument(format!(
                    "empty or null components not allowed: {}",
                    arrays_to_string(&self.components)
                )));
            }
            len += java_length(comp) as i64;
        }
        len += self.components.len() as i64 - 1; // add separators
        if len > Self::MAX_CATEGORY_PATH_LENGTH as i64 {
            return Err(LuceneError::illegal_argument(format!(
                "category path exceeds maximum allowed path length: max={} len={} path={}...",
                Self::MAX_CATEGORY_PATH_LENGTH,
                len,
                java_substring_to(&arrays_to_string(&self.components), 30)?
            )));
        }
        Ok(())
    }

    /// Compares this path with another `FacetLabel` for lexicographic order.
    ///
    /// Equivalent to `compareTo(FacetLabel)`.
    pub fn compare_to(&self, other: &FacetLabel) -> i32 {
        let len = self.length.min(other.length);
        for i in 0..len {
            match java_compare_to(&self.components[i], &other.components[i]) {
                Ordering::Less => return -1,   // this is 'before'
                Ordering::Greater => return 1, // this is 'after'
                Ordering::Equal => {}
            }
        }
        // one is a prefix of the other
        self.length as i32 - other.length as i32
    }

    /// Equivalent to `hashCode()`.
    pub fn hash_code(&self) -> i32 {
        if self.length == 0 {
            return 0;
        }
        let mut hash = self.length as i32;
        for i in 0..self.length {
            hash = hash
                .wrapping_mul(31)
                .wrapping_add(java_string_hash_code(&self.components[i]));
        }
        hash
    }

    /// Calculate a 64-bit hash function for this path.
    ///
    /// Equivalent to `longHashCode()`.
    pub fn long_hash_code(&self) -> i64 {
        if self.length == 0 {
            return 0;
        }
        let mut hash = self.length as i64;
        for i in 0..self.length {
            hash = hash
                .wrapping_mul(65599)
                .wrapping_add(java_string_hash_code(&self.components[i]) as i64);
        }
        hash
    }

    /// Returns a sub-path of this path up to `length` components.
    ///
    /// Equivalent to `subpath(int)`; a negative Java length is any value at
    /// or above [`length`](Self::length) here's mirror, `None`.
    pub fn subpath(&self, length: usize) -> FacetLabel {
        if length >= self.length {
            self.clone()
        } else {
            Self::prefix_of(self, length)
        }
    }

    /// Equivalent to `subpath(int)` with a Java `int` argument, negative
    /// included.
    pub fn subpath_i32(&self, length: i32) -> FacetLabel {
        if length < 0 {
            self.clone()
        } else {
            self.subpath(length as usize)
        }
    }

    /// Returns the last component of this path.
    ///
    /// Equivalent to `lastComponent()`, which reads the last element of the
    /// whole (possibly shared) `components` array.
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException("components is empty")`.
    pub fn last_component(&self) -> Result<&str> {
        if self.components.is_empty() {
            return Err(LuceneError::unsupported_operation("components is empty"));
        }
        Ok(&self.components[self.components.len() - 1])
    }

    /// The first `length` components.
    pub fn path(&self) -> &[String] {
        &self.components[..self.length]
    }
}

impl PartialEq for FacetLabel {
    /// Equivalent to `equals(Object)`.
    fn eq(&self, other: &Self) -> bool {
        if self.length != other.length {
            return false; // not same length, cannot be equal
        }
        // CategoryPaths are more likely to differ at the last components, so
        // start from last-first
        (0..self.length)
            .rev()
            .all(|i| self.components[i] == other.components[i])
    }
}

impl Eq for FacetLabel {}

impl Hash for FacetLabel {
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write_i32(self.hash_code());
    }
}

impl PartialOrd for FacetLabel {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for FacetLabel {
    fn cmp(&self, other: &Self) -> Ordering {
        self.compare_to(other).cmp(&0)
    }
}

impl fmt::Display for FacetLabel {
    /// Equivalent to `toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.length == 0 {
            return f.write_str("FacetLabel: []");
        }
        write!(f, "FacetLabel: {}", arrays_to_string(self.path()))
    }
}

impl fmt::Debug for FacetLabel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}
