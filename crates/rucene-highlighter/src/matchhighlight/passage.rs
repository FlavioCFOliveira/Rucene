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

//! Port of `Passage.java`, of the Java package
//! `org.apache.lucene.search.matchhighlight`.

use std::fmt;

use rucene::error::{LuceneError, Result};

use crate::matchhighlight::offset_range::{HasOffsetRange, OffsetRange};

/// A passage is a fragment of source text, scored and possibly with a list
/// of sub-offsets (markers) to be highlighted. The markers can overlap or
/// be nested, but they're always contained within the passage.
///
/// Equivalent to `org.apache.lucene.search.matchhighlight.Passage`, a
/// subclass of `OffsetRange`.
#[derive(Debug, Clone)]
pub struct Passage {
    /// The start offset (the inherited field `from`).
    pub from: i32,
    /// The end offset (the inherited field `to`).
    pub to: i32,
    /// Equivalent to the public field `markers`.
    pub markers: Vec<OffsetRange>,
}

impl Passage {
    /// Equivalent to `new Passage(int, int, List)`.
    pub fn new(from: i32, to: i32, markers: Vec<OffsetRange>) -> Self {
        debug_assert!(
            from <= to,
            "A non-empty offset range is required: {from}-{to}"
        );
        Self { from, to, markers }
    }

    /// Equivalent to the inherited `length()`.
    pub fn length(&self) -> i32 {
        self.to - self.from
    }

    /// Passages can't be sliced as it could split previously determined
    /// highlight markers.
    ///
    /// Equivalent to `slice(int, int)`.
    ///
    /// # Errors
    ///
    /// Always, as Java throws `RuntimeException("Passages.slice() does not
    /// make sense?")`.
    pub fn slice(&self, _from: i32, _to: i32) -> Result<OffsetRange> {
        Err(LuceneError::unchecked(
            "java.lang.RuntimeException",
            "Passages.slice() does not make sense?",
        ))
    }

    /// This passage's range, as Java reads a `Passage` as an `OffsetRange`.
    pub fn as_offset_range(&self) -> OffsetRange {
        OffsetRange::new(self.from, self.to)
    }
}

impl fmt::Display for Passage {
    /// Equivalent to `toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[[from={}, to={}], markers=[", self.from, self.to)?;
        for (i, m) in self.markers.iter().enumerate() {
            if i > 0 {
                f.write_str(", ")?;
            }
            write!(f, "{m}")?;
        }
        f.write_str("]]")
    }
}

impl PartialEq for Passage {
    /// The inherited `equals(Object)`: equal offsets.
    fn eq(&self, other: &Self) -> bool {
        self.from == other.from && self.to == other.to
    }
}

impl HasOffsetRange for Passage {
    fn range_from(&self) -> i32 {
        self.from
    }

    fn range_to(&self) -> i32 {
        self.to
    }
}
