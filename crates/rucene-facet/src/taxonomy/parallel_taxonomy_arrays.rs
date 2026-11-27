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

//! Port of `ParallelTaxonomyArrays.java`, of the Java package
//! `org.apache.lucene.facet.taxonomy`.

/// Returns 3 arrays for traversing the taxonomy: `parents`, `children` and
/// `siblings`.
///
/// Equivalent to `org.apache.lucene.facet.taxonomy.ParallelTaxonomyArrays`.
pub trait ParallelTaxonomyArrays: Send + Sync {
    /// Returns the parents array, where `parents[i]` denotes the parent of
    /// category ordinal `i`.
    ///
    /// Equivalent to `parents()`.
    fn parents(&self) -> &dyn IntArray;

    /// Returns the children array, where `children[i]` denotes a child of
    /// category ordinal `i`.
    ///
    /// Equivalent to `children()`.
    fn children(&self) -> &dyn IntArray;

    /// Returns the siblings array, where `siblings[i]` denotes the sibling of
    /// category ordinal `i`.
    ///
    /// Equivalent to `siblings()`.
    fn siblings(&self) -> &dyn IntArray;
}

/// Abstraction that looks like an int[], but read-only.
///
/// Equivalent to `org.apache.lucene.facet.taxonomy.ParallelTaxonomyArrays.IntArray`.
pub trait IntArray: Send + Sync {
    /// `ia.get(i) == array[i]`.
    ///
    /// Equivalent to `get(int)`.
    fn get(&self, i: i32) -> i32;

    /// `ia.length() == array.length`.
    ///
    /// Equivalent to `length()`.
    fn length(&self) -> i32;
}
