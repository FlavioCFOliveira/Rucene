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

//! Port of `FacetSetDecoder.java`, of the Java package
//! `org.apache.lucene.facet.facetset`.

use rucene::document::{IntPoint, LongPoint};
use rucene::util::BytesRef;

/// A functional interface for decoding facet set values into comparable
/// `long` ones.
///
/// Equivalent to `org.apache.lucene.facet.facetset.FacetSetDecoder`; a
/// closure or function of the same shape (Java's lambdas and method
/// references) implements it.
pub trait FacetSetDecoder: Send + Sync {
    /// Decodes the facet set dimension values into the given destination
    /// buffer and returns the number of bytes read.
    ///
    /// Equivalent to `decode(BytesRef, int, long[])`.
    fn decode(&self, bytes_ref: &BytesRef, start: usize, dest: &mut [i64]) -> usize;
}

impl<F> FacetSetDecoder for F
where
    F: Fn(&BytesRef, usize, &mut [i64]) -> usize + Send + Sync,
{
    fn decode(&self, bytes_ref: &BytesRef, start: usize, dest: &mut [i64]) -> usize {
        self(bytes_ref, start, dest)
    }
}

/// An implementation of `decode` for facet set dimension values which were
/// encoded with `LongPoint`.
///
/// Equivalent to `FacetSetDecoder.decodeLongs(BytesRef, int, long[])`.
pub fn decode_longs(bytes_ref: &BytesRef, start: usize, dest: &mut [i64]) -> usize {
    LongPoint::unpack(bytes_ref, start, dest);
    dest.len() * 8
}

/// An implementation of `decode` for facet set dimension values which were
/// encoded with `IntPoint`.
///
/// Equivalent to `FacetSetDecoder.decodeInts(BytesRef, int, long[])`.
pub fn decode_ints(bytes_ref: &BytesRef, start: usize, dest: &mut [i64]) -> usize {
    let mut offset = start;
    for slot in dest.iter_mut() {
        *slot = IntPoint::decode_dimension(&bytes_ref.bytes, offset) as i64;
        offset += 4;
    }
    dest.len() * 4
}
