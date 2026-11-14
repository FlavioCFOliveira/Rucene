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

//! The type declared in `Hyphenation.java`, of the Java package
//! `org.apache.lucene.analysis.compound.hyphenation`.

/// This class represents a hyphenated word.
///
/// This class has been taken from the Apache FOP project
/// (<http://xmlgraphics.apache.org/fop/>). They have been slightly modified.
///
/// Equivalent to `org.apache.lucene.analysis.compound.hyphenation.Hyphenation`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hyphenation {
    hyphen_points: Vec<i32>,
}

impl Hyphenation {
    /// rawWord as made of alternating strings and [`Hyphen`](super::hyphen::Hyphen) instances
    ///
    /// Equivalent to the package-private `Hyphenation(int[] points)`.
    pub(crate) fn new(points: Vec<i32>) -> Self {
        Self {
            hyphen_points: points,
        }
    }

    /// Returns the number of hyphenation points in the word.
    ///
    /// Equivalent to `Hyphenation.length()`.
    pub fn length(&self) -> usize {
        self.hyphen_points.len()
    }

    /// Returns the hyphenation points.
    ///
    /// Equivalent to `Hyphenation.getHyphenationPoints()`.
    pub fn get_hyphenation_points(&self) -> &[i32] {
        &self.hyphen_points
    }
}
