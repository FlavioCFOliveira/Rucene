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

//! The types declared in `ShapeFieldCache.java`, of the Java package
//! `org.apache.lucene.spatial.util`.

use std::sync::Arc;

/// Equivalent to `org.apache.lucene.spatial.util.ShapeFieldCache` (`public
/// class ShapeFieldCache<T extends Shape>`): bounded cache of shapes
/// associated with docIds. Note, multiple shapes can be associated with a
/// given docId.
///
/// WARNING: This class holds the data in an extremely inefficient manner as
/// all points are in memory as objects and they are stored in many
/// ArrayLists (one per document). So it works but doesn't scale. It will
/// be replaced in the future.
#[derive(Debug)]
pub struct ShapeFieldCache<T: ?Sized> {
    cache: Vec<Option<Vec<Arc<T>>>>,
    /// `defaultLength`.
    pub default_length: i32,
}

impl<T: ?Sized> ShapeFieldCache<T> {
    /// `ShapeFieldCache(int length, int defaultLength)`.
    pub fn new(length: i32, default_length: i32) -> Self {
        Self {
            cache: (0..length.max(0)).map(|_| None).collect(),
            default_length,
        }
    }

    /// `add(int docid, T s)`.
    pub fn add(&mut self, docid: i32, s: Arc<T>) {
        let default_length = self.default_length.max(0) as usize;
        self.cache[docid as usize]
            .get_or_insert_with(|| Vec::with_capacity(default_length))
            .push(s);
    }

    /// `getShapes(int docid)`: `None` (Java `null`) for a document without
    /// shapes.
    pub fn get_shapes(&self, docid: i32) -> Option<&[Arc<T>]> {
        self.cache[docid as usize].as_deref()
    }
}
