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

//! Port of `BoundaryScanner.java`, of the Java package
//! `org.apache.lucene.search.vectorhighlight`.

use std::fmt::Debug;

/// Finds fragment boundaries: pluggable into `BaseFragmentsBuilder`.
///
/// Equivalent to `org.apache.lucene.search.vectorhighlight.BoundaryScanner`.
/// The buffer is Java's `StringBuilder`, as UTF-16 code units.
pub trait BoundaryScanner: Debug + Send + Sync {
    /// Scan backward to find end offset.
    ///
    /// Equivalent to `findStartOffset(StringBuilder, int)`.
    fn find_start_offset(&self, buffer: &[u16], start: i32) -> i32;

    /// Scan forward to find start offset.
    ///
    /// Equivalent to `findEndOffset(StringBuilder, int)`.
    fn find_end_offset(&self, buffer: &[u16], start: i32) -> i32;
}
