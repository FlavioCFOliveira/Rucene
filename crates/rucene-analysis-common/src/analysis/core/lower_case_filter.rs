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

//! The `analysis/common` reference to `rucene`'s `LowerCaseFilter`, ported from
//! `org.apache.lucene.analysis.core.LowerCaseFilter`.

use rucene::analysis::{new_lower_case_filter, TokenStream};
use rucene::error::Result;
use rucene::util::attribute::{AttributeReflector, AttributeSource, Unwrappable};
use rucene::util::IgnoreRandomChains;

/// Normalizes token text to lower case.
///
/// This class moved to Lucene Core, but a reference in the `analysis/common`
/// module is preserved for documentation purposes and consistency with filter
/// factory.
///
/// See also `rucene::analysis::LowerCaseFilter` and `LowerCaseFilterFactory`.
///
/// Equivalent to `org.apache.lucene.analysis.core.LowerCaseFilter`, which
/// extends `org.apache.lucene.analysis.LowerCaseFilter` and declares nothing
/// but its constructor: the filter holds the core filter it extends and
/// delegates every method to it, so the two are distinct types, as the two
/// Java classes are.
#[derive(Debug)]
pub struct LowerCaseFilter {
    /// The `org.apache.lucene.analysis.LowerCaseFilter` this class extends.
    inner: rucene::analysis::LowerCaseFilter,
}

impl LowerCaseFilter {
    /// `@IgnoreRandomChains(reason = "clones of core's filters")`.
    pub const IGNORE_RANDOM_CHAINS: IgnoreRandomChains =
        IgnoreRandomChains::new("clones of core's filters");

    /// Create a new LowerCaseFilter, that normalizes token text to lower case:
    /// `input` is the TokenStream to filter.
    ///
    /// Equivalent to `LowerCaseFilter(TokenStream)`, `super(in)`.
    ///
    /// # Errors
    ///
    /// What the stream's attribute factory raises for an interface it cannot
    /// implement, as Java's `addAttribute` throws.
    pub fn new(input: Box<dyn TokenStream>) -> Result<Self> {
        Ok(Self {
            inner: new_lower_case_filter(input)?,
        })
    }

    /// The `org.apache.lucene.analysis.LowerCaseFilter` this filter extends.
    pub fn as_core(&self) -> &rucene::analysis::LowerCaseFilter {
        &self.inner
    }
}

/// Every method is the inherited one: Java's subclass declares only its
/// constructor.
impl TokenStream for LowerCaseFilter {
    fn increment_token(&mut self) -> Result<bool> {
        self.inner.increment_token()
    }

    fn end(&mut self) -> Result<()> {
        self.inner.end()
    }

    fn reset(&mut self) -> Result<()> {
        self.inner.reset()
    }

    fn close(&mut self) -> Result<()> {
        self.inner.close()
    }

    fn attribute_source(&self) -> &AttributeSource {
        self.inner.attribute_source()
    }

    fn attribute_source_mut(&mut self) -> &mut AttributeSource {
        self.inner.attribute_source_mut()
    }

    fn reflect_with(&self, reflector: &mut dyn AttributeReflector) {
        self.inner.reflect_with(reflector)
    }

    fn as_unwrappable(&self) -> Option<&dyn Unwrappable<dyn TokenStream>> {
        self.inner.as_unwrappable()
    }
}
