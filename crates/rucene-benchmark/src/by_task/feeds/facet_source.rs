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

//! The types declared in `FacetSource.java`, of the Java package
//! `org.apache.lucene.benchmark.byTask.feeds`.

use rucene::error::Result;
use rucene_facet::facet_field::FacetField;
use rucene_facet::facets_config::FacetsConfig;

use crate::by_task::feeds::content_items_source::ContentItemsSource;

/// Equivalent to `org.apache.lucene.benchmark.byTask.feeds.FacetSource`
/// (`public abstract class FacetSource extends ContentItemsSource`): source
/// items for facets.
///
/// For supported configuration parameters see [`ContentItemsSource`].
/// `FacetSource.resetInputs()` is [`reset_inputs`], which every
/// implementation's [`ContentItemsSource::reset_inputs`] calls.
pub trait FacetSource: ContentItemsSource {
    /// `getNextFacets(List<FacetField> facets)`: fills the next facets
    /// content items in the given list. Implementations must account for
    /// multi-threading, as multiple threads can call this method
    /// simultaneously.
    ///
    /// # Errors
    ///
    /// `NoMoreDataException` or an `IOException`.
    fn get_next_facets(&self, facets: &mut Vec<FacetField>) -> Result<()>;

    /// `configure(FacetsConfig config)`.
    fn configure(&self, config: &FacetsConfig);
}

/// `FacetSource.resetInputs()`: prints the statistics, re-initiates the
/// source (properties by round may have changed), then
/// `ContentItemsSource.resetInputs()`.
///
/// # Errors
///
/// The exception of `setConfig`.
pub fn reset_inputs<S: FacetSource + ?Sized>(source: &S) -> Result<()> {
    source.print_statistics("facets");
    // re-initiate since properties by round may have changed.
    let config = source.get_config().expect("setConfig was called");
    source.set_config(config)?;
    source.items_base().reset_inputs();
    Ok(())
}
