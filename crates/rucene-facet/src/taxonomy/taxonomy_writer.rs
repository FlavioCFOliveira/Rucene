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

//! Port of `TaxonomyWriter.java`, of the Java package
//! `org.apache.lucene.facet.taxonomy`.

use rucene::error::Result;
use rucene::index::TwoPhaseCommit;

use crate::taxonomy::facet_label::FacetLabel;

/// `TaxonomyWriter` is the interface which the faceted-search library uses to
/// dynamically build the taxonomy at indexing time.
///
/// Equivalent to `org.apache.lucene.facet.taxonomy.TaxonomyWriter`, which
/// extends `Closeable` and `TwoPhaseCommit`; `close()` is
/// [`close`](Self::close).
pub trait TaxonomyWriter: TwoPhaseCommit + Send + Sync {
    /// Adds a category to the taxonomy, returning its ordinal.
    ///
    /// Equivalent to `addCategory(FacetLabel)`.
    fn add_category(&self, category_path: &FacetLabel) -> Result<i32>;

    /// Returns the ordinal of the parent category of the category with the
    /// given ordinal.
    ///
    /// Equivalent to `getParent(int)`.
    fn get_parent(&self, ordinal: i32) -> Result<i32>;

    /// Returns the number of categories in the taxonomy.
    ///
    /// Equivalent to `getSize()`.
    fn get_size(&self) -> Result<i32>;

    /// Sets the commit user data iterable.
    ///
    /// Equivalent to `setLiveCommitData(Iterable)`.
    fn set_live_commit_data(&self, commit_user_data: Option<Vec<(String, String)>>) -> Result<()>;

    /// Returns the commit user data iterable that was set on
    /// [`set_live_commit_data`](Self::set_live_commit_data).
    ///
    /// Equivalent to `getLiveCommitData()`.
    fn get_live_commit_data(&self) -> Result<Vec<(String, String)>>;

    /// Equivalent to `Closeable.close()`.
    fn close(&self) -> Result<()>;
}
