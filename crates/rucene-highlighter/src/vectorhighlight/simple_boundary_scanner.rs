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

//! Port of `SimpleBoundaryScanner.java`, of the Java package
//! `org.apache.lucene.search.vectorhighlight`.

use std::collections::HashSet;

use crate::vectorhighlight::boundary_scanner::BoundaryScanner;

/// Simple boundary scanner implementation that divides fragments based on a
/// set of separator characters.
///
/// Equivalent to `org.apache.lucene.search.vectorhighlight.SimpleBoundaryScanner`.
/// A boundary character is a UTF-16 code unit (Java's `char`).
#[derive(Debug, Clone)]
pub struct SimpleBoundaryScanner {
    /// Equivalent to the protected field `maxScan`.
    pub max_scan: i32,
    /// Equivalent to the protected field `boundaryChars` (a `CharHashSet`).
    pub boundary_chars: HashSet<u16>,
}

impl SimpleBoundaryScanner {
    /// Equivalent to `SimpleBoundaryScanner.DEFAULT_MAX_SCAN`.
    pub const DEFAULT_MAX_SCAN: i32 = 20;

    /// Equivalent to `SimpleBoundaryScanner.DEFAULT_BOUNDARY_CHARS`.
    pub const DEFAULT_BOUNDARY_CHARS: [u16; 7] = [
        b'.' as u16,
        b',' as u16,
        b'!' as u16,
        b'?' as u16,
        b' ' as u16,
        b'\t' as u16,
        b'\n' as u16,
    ];

    /// Equivalent to `new SimpleBoundaryScanner()`.
    pub fn new() -> Self {
        Self::with_max_scan_and_chars(Self::DEFAULT_MAX_SCAN, &Self::DEFAULT_BOUNDARY_CHARS)
    }

    /// Equivalent to `new SimpleBoundaryScanner(int)`.
    pub fn with_max_scan(max_scan: i32) -> Self {
        Self::with_max_scan_and_chars(max_scan, &Self::DEFAULT_BOUNDARY_CHARS)
    }

    /// Equivalent to `new SimpleBoundaryScanner(Character[])`.
    pub fn with_boundary_chars(boundary_chars: &[u16]) -> Self {
        Self::with_max_scan_and_chars(Self::DEFAULT_MAX_SCAN, boundary_chars)
    }

    /// Equivalent to `new SimpleBoundaryScanner(int, char[])` and `new
    /// SimpleBoundaryScanner(int, Character[])`.
    pub fn with_max_scan_and_chars(max_scan: i32, boundary_chars: &[u16]) -> Self {
        Self {
            max_scan,
            boundary_chars: boundary_chars.iter().copied().collect(),
        }
    }

    /// Equivalent to `new SimpleBoundaryScanner(int, Set<Character>)`.
    pub fn with_max_scan_and_char_set(max_scan: i32, boundary_chars: &HashSet<u16>) -> Self {
        Self {
            max_scan,
            boundary_chars: boundary_chars.clone(),
        }
    }
}

impl Default for SimpleBoundaryScanner {
    fn default() -> Self {
        Self::new()
    }
}

impl BoundaryScanner for SimpleBoundaryScanner {
    fn find_start_offset(&self, buffer: &[u16], start: i32) -> i32 {
        // avoid illegal start offset
        if start > buffer.len() as i32 || start < 1 {
            return start;
        }
        let mut offset = start;
        let mut count = self.max_scan;
        while offset > 0 && count > 0 {
            // found?
            if self
                .boundary_chars
                .contains(&buffer[(offset - 1) as usize])
            {
                return offset;
            }
            offset -= 1;
            count -= 1;
        }
        // if we scanned up to the start of the text, return it, it's a
        // "boundary"
        if offset == 0 {
            return 0;
        }
        // not found
        start
    }

    fn find_end_offset(&self, buffer: &[u16], start: i32) -> i32 {
        // avoid illegal start offset
        if start > buffer.len() as i32 || start < 0 {
            return start;
        }
        let mut offset = start;
        let mut count = self.max_scan;
        while offset < buffer.len() as i32 && count > 0 {
            // found?
            if self.boundary_chars.contains(&buffer[offset as usize]) {
                return offset;
            }
            offset += 1;
            count -= 1;
        }
        // not found
        start
    }
}