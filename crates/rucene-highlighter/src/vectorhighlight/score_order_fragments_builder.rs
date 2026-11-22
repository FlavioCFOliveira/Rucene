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

//! Port of `ScoreOrderFragmentsBuilder.java`, of the Java package
//! `org.apache.lucene.search.vectorhighlight`.

use std::cmp::Ordering;
use std::ops::{Deref, DerefMut};

use crate::vectorhighlight::base_fragments_builder::{BaseFragmentsBuilder, BaseFragmentsBuilderImpl};
use crate::vectorhighlight::boundary_scanner::BoundaryScanner;
use crate::vectorhighlight::field_frag_list::WeightedFragInfo;

/// An implementation of FragmentsBuilder that outputs score-order fragments.
///
/// Equivalent to `org.apache.lucene.search.vectorhighlight.ScoreOrderFragmentsBuilder`.
#[derive(Debug, Default)]
pub struct ScoreOrderFragmentsBuilder {
    base: BaseFragmentsBuilder,
}

impl ScoreOrderFragmentsBuilder {
    /// a constructor.
    ///
    /// Equivalent to `new ScoreOrderFragmentsBuilder()`.
    pub fn new() -> Self {
        Self {
            base: BaseFragmentsBuilder::new(),
        }
    }

    /// a constructor.
    ///
    /// Equivalent to `new ScoreOrderFragmentsBuilder(String[], String[])`.
    pub fn with_tags(pre_tags: Vec<String>, post_tags: Vec<String>) -> Self {
        Self {
            base: BaseFragmentsBuilder::with_tags(pre_tags, post_tags),
        }
    }

    /// Equivalent to `new ScoreOrderFragmentsBuilder(BoundaryScanner)`.
    pub fn with_scanner(bs: Box<dyn BoundaryScanner>) -> Self {
        Self {
            base: BaseFragmentsBuilder::with_scanner(bs),
        }
    }

    /// Equivalent to `new ScoreOrderFragmentsBuilder(String[], String[],
    /// BoundaryScanner)`.
    pub fn with_tags_and_scanner(
        pre_tags: Vec<String>,
        post_tags: Vec<String>,
        bs: Box<dyn BoundaryScanner>,
    ) -> Self {
        Self {
            base: BaseFragmentsBuilder::with_tags_and_scanner(pre_tags, post_tags, bs),
        }
    }
}

impl Deref for ScoreOrderFragmentsBuilder {
    type Target = BaseFragmentsBuilder;

    fn deref(&self) -> &BaseFragmentsBuilder {
        &self.base
    }
}

impl DerefMut for ScoreOrderFragmentsBuilder {
    fn deref_mut(&mut self) -> &mut BaseFragmentsBuilder {
        &mut self.base
    }
}

impl BaseFragmentsBuilderImpl for ScoreOrderFragmentsBuilder {
    fn base(&self) -> &BaseFragmentsBuilder {
        &self.base
    }

    /// Sort by score the list of WeightedFragInfo.
    fn get_weighted_frag_info_list(&self, mut src: Vec<WeightedFragInfo>) -> Vec<WeightedFragInfo> {
        src.sort_by(|o1, o2| ScoreComparator.compare(o1, o2));
        src
    }
}

/// Comparator for `WeightedFragInfo` by boost, breaking ties by offset.
///
/// Equivalent to `org.apache.lucene.search.vectorhighlight.ScoreOrderFragmentsBuilder.ScoreComparator`.
#[derive(Debug, Clone, Copy, Default)]
pub struct ScoreComparator;

impl ScoreComparator {
    /// Equivalent to `compare(WeightedFragInfo, WeightedFragInfo)`.
    pub fn compare(&self, o1: &WeightedFragInfo, o2: &WeightedFragInfo) -> Ordering {
        if o1.get_total_boost() > o2.get_total_boost() {
            return Ordering::Less;
        } else if o1.get_total_boost() < o2.get_total_boost() {
            return Ordering::Greater;
        }
        // if same score then check startOffset
        else if o1.get_start_offset() < o2.get_start_offset() {
            return Ordering::Less;
        } else if o1.get_start_offset() > o2.get_start_offset() {
            return Ordering::Greater;
        }
        Ordering::Equal
    }
}