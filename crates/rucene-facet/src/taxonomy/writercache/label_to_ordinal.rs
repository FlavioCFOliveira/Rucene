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

//! Port of `LabelToOrdinal.java`, of the Java package
//! `org.apache.lucene.facet.taxonomy.writercache`.

use crate::taxonomy::facet_label::FacetLabel;

/// Abstract class for storing Label-&gt;Ordinal mappings in a taxonomy.
///
/// Equivalent to `org.apache.lucene.facet.taxonomy.writercache.LabelToOrdinal`;
/// its `protected int counter` is [`counter`](Self::counter).
pub trait LabelToOrdinal {
    /// The protected `counter` field.
    fn counter(&mut self) -> &mut i32;

    /// Return the maximal Ordinal assigned so far.
    ///
    /// Equivalent to `getMaxOrdinal()`.
    fn get_max_ordinal(&mut self) -> i32 {
        *self.counter()
    }

    /// Returns the next unassigned ordinal. The default behavior of this
    /// method is to simply increment a counter.
    ///
    /// Equivalent to `getNextOrdinal()`.
    fn get_next_ordinal(&mut self) -> i32 {
        let counter = self.counter();
        let next = *counter;
        *counter += 1;
        next
    }

    /// Adds a new label if it is not yet in the table.
    ///
    /// Equivalent to `addLabel(FacetLabel, int)`.
    fn add_label(&mut self, label: &FacetLabel, ordinal: i32);

    /// Returns the ordinal assigned to the given label, or
    /// [`INVALID_ORDINAL`] if the label cannot be found in this table.
    ///
    /// Equivalent to `getOrdinal(FacetLabel)`.
    fn get_ordinal(&mut self, label: &FacetLabel) -> i32;
}

/// Returned by [`LabelToOrdinal::get_ordinal`] when the label isn't
/// recognized.
///
/// Equivalent to `LabelToOrdinal.INVALID_ORDINAL`.
pub const INVALID_ORDINAL: i32 = -2;