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

//! Port of `PositionSpan.java`, of the Java package
//! `org.apache.lucene.search.highlight`.

/// Utility class to store a Span.
///
/// Equivalent to `org.apache.lucene.search.highlight.PositionSpan`.
#[derive(Debug, Clone, Copy)]
pub struct PositionSpan {
    pub(crate) start: i32,
    pub(crate) end: i32,
}

impl PositionSpan {
    /// Equivalent to `new PositionSpan(int, int)`.
    pub fn new(start: i32, end: i32) -> Self {
        Self { start, end }
    }

    /// Equivalent to reading the package-private field `start`.
    pub fn start(&self) -> i32 {
        self.start
    }

    /// Equivalent to reading the package-private field `end`.
    pub fn end(&self) -> i32 {
        self.end
    }
}
