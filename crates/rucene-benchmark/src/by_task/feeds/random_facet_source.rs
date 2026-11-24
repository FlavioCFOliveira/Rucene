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

//! The types declared in `RandomFacetSource.java`, of the Java package
//! `org.apache.lucene.benchmark.byTask.feeds`.

use std::any::Any;
use std::sync::{Arc, Mutex};

use rucene::error::{LuceneError, Result};
use rucene::util::java_random::JavaRandom;
use rucene_facet::facet_field::FacetField;
use rucene_facet::facets_config::FacetsConfig;

use crate::by_task::feeds::content_items_source::{ContentItemsSource, ContentItemsSourceBase};
use crate::by_task::feeds::facet_source::{self, FacetSource};
use crate::by_task::utils::config::Config;
use crate::jdk;

#[derive(Debug)]
struct State {
    random: JavaRandom,
    max_doc_facets: i32,
    max_facet_depth: i32,
    max_dims: i32,
    max_value: i32,
}

/// Equivalent to `org.apache.lucene.benchmark.byTask.feeds.RandomFacetSource`:
/// simple implementation of a random facet source.
///
/// Supports the following parameters:
///
/// * **rand.seed** - defines the seed to initialize `Random` with
///   (default: **13**).
/// * **max.doc.facet.dims** - Max number of random dimensions to create
///   (default: **5**); actual number of dimensions would be anything between
///   1 and that number.
/// * **max.doc.facets** - maximal #facets per doc (default: **10**). Actual
///   number of facets in a certain doc would be anything between 1 and that
///   number.
/// * **max.facet.depth** - maximal #components in a facet (default:
///   **3**). Actual number of components in a certain facet would be
///   anything between 1 and that number.
#[derive(Debug)]
pub struct RandomFacetSource {
    base: ContentItemsSourceBase,
    state: Mutex<State>,
}

impl Default for RandomFacetSource {
    fn default() -> Self {
        Self::new()
    }
}

impl RandomFacetSource {
    /// `new RandomFacetSource()`: `random` is `null` until `setConfig`;
    /// `maxValue = maxDocFacets * maxFacetDepth` is 0.
    pub fn new() -> Self {
        RandomFacetSource {
            base: ContentItemsSourceBase::new(),
            state: Mutex::new(State {
                random: JavaRandom::new(0),
                max_doc_facets: 0,
                max_facet_depth: 0,
                max_dims: 0,
                max_value: 0,
            }),
        }
    }
}

impl ContentItemsSource for RandomFacetSource {
    fn items_base(&self) -> &ContentItemsSourceBase {
        &self.base
    }

    fn java_simple_name(&self) -> &'static str {
        "RandomFacetSource"
    }

    fn close(&self) -> Result<()> {
        // nothing to do here
        Ok(())
    }

    fn reset_inputs(&self) -> Result<()> {
        facet_source::reset_inputs(self)
    }

    fn set_config(&self, config: Arc<Config>) -> Result<()> {
        self.base.set_config(Arc::clone(&config))?;
        let random = JavaRandom::new(i64::from(config.get_int("rand.seed", 13)?));
        let max_doc_facets = config.get_int("max.doc.facets", 10)?;
        let max_dims = config.get_int("max.doc.facets.dims", 5)?;
        let max_facet_depth = config.get_int("max.facet.depth", 3)?;
        let mut s = jdk::lock(&self.state);
        s.random = random;
        s.max_doc_facets = max_doc_facets;
        s.max_dims = max_dims;
        s.max_facet_depth = max_facet_depth;
        if max_facet_depth < 2 {
            return Err(LuceneError::illegal_argument(format!(
                "max.facet.depth must be at least 2; got: {max_facet_depth}"
            )));
        }
        s.max_value = max_doc_facets * max_facet_depth;
        Ok(())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl FacetSource for RandomFacetSource {
    fn get_next_facets(&self, facets: &mut Vec<FacetField>) -> Result<()> {
        facets.clear();
        let mut s = jdk::lock(&self.state);
        let max_doc_facets = s.max_doc_facets;
        let num_facets = 1 + s.random.next_int_bound(max_doc_facets); // at least one facet to each doc
        for _ in 0..num_facets {
            let depth = if s.max_facet_depth == 2 {
                2
            } else {
                let bound = s.max_facet_depth - 2;
                2 + s.random.next_int_bound(bound) // depth < 2 is not useful
            };
            let max_dims = s.max_dims;
            let dim = s.random.next_int_bound(max_dims).to_string();
            let mut components = Vec::with_capacity((depth - 1) as usize);
            for _ in 0..depth - 1 {
                let max_value = s.max_value;
                components.push(s.random.next_int_bound(max_value).to_string());
                self.base.add_item();
            }
            let refs: Vec<&str> = components.iter().map(String::as_str).collect();
            let ff = FacetField::new(&dim, &refs)?;
            let len = jdk::utf16_len(&ff.to_string()) as i64;
            facets.push(ff);
            self.base.add_bytes(len); // very rough approximation
        }
        Ok(())
    }

    fn configure(&self, config: &FacetsConfig) {
        let max_dims = jdk::lock(&self.state).max_dims;
        for i in 0..max_dims {
            config.set_hierarchical(&i.to_string(), true);
            config.set_multi_valued(&i.to_string(), true);
        }
    }
}
