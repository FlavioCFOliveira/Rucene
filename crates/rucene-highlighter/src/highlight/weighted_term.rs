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

//! Port of `WeightedTerm.java`, of the Java package
//! `org.apache.lucene.search.highlight`.

/// Lightweight class to hold term and a weight value used for scoring this
/// term.
///
/// Equivalent to `org.apache.lucene.search.highlight.WeightedTerm`. Java
/// compares and hashes it by identity (it overrides neither `equals` nor
/// `hashCode`); the port gives it no `PartialEq` either.
#[derive(Debug, Clone)]
pub struct WeightedTerm {
    pub(crate) weight: f32, // multiplier
    pub(crate) term: String, // stemmed form
}

impl WeightedTerm {
    /// Equivalent to `new WeightedTerm(float, String)`.
    pub fn new(weight: f32, term: impl Into<String>) -> Self {
        Self {
            weight,
            term: term.into(),
        }
    }

    /// Equivalent to `getTerm()`.
    pub fn get_term(&self) -> &str {
        &self.term
    }

    /// Equivalent to `getWeight()`.
    pub fn get_weight(&self) -> f32 {
        self.weight
    }

    /// Equivalent to `setTerm(String)`.
    pub fn set_term(&mut self, term: impl Into<String>) {
        self.term = term.into();
    }

    /// Equivalent to `setWeight(float)`.
    pub fn set_weight(&mut self, weight: f32) {
        self.weight = weight;
    }
}