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

//! Port of `WeightedSpanTerm.java`, of the Java package
//! `org.apache.lucene.search.highlight`.

use std::ops::{Deref, DerefMut};

use crate::highlight::position_span::PositionSpan;
use crate::highlight::weighted_term::WeightedTerm;

/// Lightweight class to hold term, weight, and positions used for scoring
/// this term.
///
/// Equivalent to `org.apache.lucene.search.highlight.WeightedSpanTerm`, a
/// subclass of `WeightedTerm`: the base is reached through `Deref`.
#[derive(Debug, Clone)]
pub struct WeightedSpanTerm {
    base: WeightedTerm,
    pub(crate) position_sensitive: bool,
    position_spans: Vec<PositionSpan>,
}

impl WeightedSpanTerm {
    /// Equivalent to `new WeightedSpanTerm(float, String)`.
    pub fn new(weight: f32, term: impl Into<String>) -> Self {
        Self {
            base: WeightedTerm::new(weight, term),
            position_sensitive: false,
            position_spans: Vec::new(),
        }
    }

    /// Equivalent to `new WeightedSpanTerm(float, String, boolean)`.
    pub fn with_position_sensitive(
        weight: f32,
        term: impl Into<String>,
        position_sensitive: bool,
    ) -> Self {
        Self {
            base: WeightedTerm::new(weight, term),
            position_sensitive,
            position_spans: Vec::new(),
        }
    }

    /// Checks to see if this term is valid at `position`.
    ///
    /// Equivalent to `checkPosition(int)`.
    pub fn check_position(&self, position: i32) -> bool {
        // There would probably be a slight speed improvement if PositionSpans
        // where kept in some sort of priority queue - that way this method
        // could bail early without checking each PositionSpan.
        for pos_span in &self.position_spans {
            if (position >= pos_span.start) && (position <= pos_span.end) {
                return true;
            }
        }

        false
    }

    /// Equivalent to `addPositionSpans(List)`.
    pub fn add_position_spans(&mut self, position_spans: &[PositionSpan]) {
        self.position_spans.extend_from_slice(position_spans);
    }

    /// Equivalent to `isPositionSensitive()`.
    pub fn is_position_sensitive(&self) -> bool {
        self.position_sensitive
    }

    /// Equivalent to `setPositionSensitive(boolean)`.
    pub fn set_position_sensitive(&mut self, position_sensitive: bool) {
        self.position_sensitive = position_sensitive;
    }

    /// Equivalent to `getPositionSpans()`.
    pub fn get_position_spans(&self) -> &[PositionSpan] {
        &self.position_spans
    }

    /// Equivalent to `getPositionSpans()` for a caller that changes the list
    /// Java returns by reference.
    pub fn get_position_spans_mut(&mut self) -> &mut Vec<PositionSpan> {
        &mut self.position_spans
    }
}

impl Deref for WeightedSpanTerm {
    type Target = WeightedTerm;

    fn deref(&self) -> &WeightedTerm {
        &self.base
    }
}

impl DerefMut for WeightedSpanTerm {
    fn deref_mut(&mut self) -> &mut WeightedTerm {
        &mut self.base
    }
}
