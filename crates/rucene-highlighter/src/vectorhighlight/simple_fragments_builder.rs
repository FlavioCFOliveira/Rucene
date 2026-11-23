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

//! Port of `SimpleFragmentsBuilder.java`, of the Java package
//! `org.apache.lucene.search.vectorhighlight`.

use std::ops::{Deref, DerefMut};

use crate::vectorhighlight::base_fragments_builder::{
    BaseFragmentsBuilder, BaseFragmentsBuilderImpl,
};
use crate::vectorhighlight::boundary_scanner::BoundaryScanner;
use crate::vectorhighlight::field_frag_list::WeightedFragInfo;

/// A simple implementation of FragmentsBuilder.
///
/// Equivalent to `org.apache.lucene.search.vectorhighlight.SimpleFragmentsBuilder`.
#[derive(Debug, Default)]
pub struct SimpleFragmentsBuilder {
    base: BaseFragmentsBuilder,
}

impl SimpleFragmentsBuilder {
    /// a constructor.
    ///
    /// Equivalent to `new SimpleFragmentsBuilder()`.
    pub fn new() -> Self {
        Self {
            base: BaseFragmentsBuilder::new(),
        }
    }

    /// a constructor.
    ///
    /// Equivalent to `new SimpleFragmentsBuilder(String[], String[])`.
    pub fn with_tags(pre_tags: Vec<String>, post_tags: Vec<String>) -> Self {
        Self {
            base: BaseFragmentsBuilder::with_tags(pre_tags, post_tags),
        }
    }

    /// Equivalent to `new SimpleFragmentsBuilder(BoundaryScanner)`.
    pub fn with_scanner(bs: Box<dyn BoundaryScanner>) -> Self {
        Self {
            base: BaseFragmentsBuilder::with_scanner(bs),
        }
    }

    /// Equivalent to `new SimpleFragmentsBuilder(String[], String[],
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

impl Deref for SimpleFragmentsBuilder {
    type Target = BaseFragmentsBuilder;

    fn deref(&self) -> &BaseFragmentsBuilder {
        &self.base
    }
}

impl DerefMut for SimpleFragmentsBuilder {
    fn deref_mut(&mut self) -> &mut BaseFragmentsBuilder {
        &mut self.base
    }
}

impl BaseFragmentsBuilderImpl for SimpleFragmentsBuilder {
    fn base(&self) -> &BaseFragmentsBuilder {
        &self.base
    }

    /// do nothing. return the source list.
    fn get_weighted_frag_info_list(&self, src: Vec<WeightedFragInfo>) -> Vec<WeightedFragInfo> {
        src
    }
}
