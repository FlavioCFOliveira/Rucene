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

//! Port of `FuzzyConfig.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.standard.config`.

use rucene::search::fuzzy_query::{DEFAULT_MAX_EDITS, DEFAULT_PREFIX_LENGTH};

use crate::flexible::core::nodes::query_node_impl::Mutable;

/// Configuration parameters for `FuzzyQuery`s
///
/// Equivalent to
/// `org.apache.lucene.queryparser.flexible.standard.config.FuzzyConfig`. It
/// is configured by reference (`StandardQueryParser.setFuzzyMinSim` changes
/// the object the configuration holds), so its setters take `&self`.
#[derive(Debug)]
pub struct FuzzyConfig {
    prefix_length: Mutable<i32>,
    min_similarity: Mutable<f32>,
}

impl Default for FuzzyConfig {
    fn default() -> Self {
        Self::new()
    }
}

impl FuzzyConfig {
    /// Equivalent to `new FuzzyConfig()`: `FuzzyQuery.defaultPrefixLength`
    /// and `FuzzyQuery.defaultMaxEdits`.
    pub fn new() -> Self {
        Self {
            prefix_length: Mutable::new(DEFAULT_PREFIX_LENGTH),
            min_similarity: Mutable::new(DEFAULT_MAX_EDITS as f32),
        }
    }

    /// Equivalent to `getPrefixLength()`.
    pub fn get_prefix_length(&self) -> i32 {
        self.prefix_length.get()
    }

    /// Equivalent to `setPrefixLength(int prefixLength)`.
    pub fn set_prefix_length(&self, prefix_length: i32) {
        self.prefix_length.set(prefix_length);
    }

    /// Equivalent to `getMinSimilarity()`.
    pub fn get_min_similarity(&self) -> f32 {
        self.min_similarity.get()
    }

    /// Equivalent to `setMinSimilarity(float minSimilarity)`.
    pub fn set_min_similarity(&self, min_similarity: f32) {
        self.min_similarity.set(min_similarity);
    }
}
