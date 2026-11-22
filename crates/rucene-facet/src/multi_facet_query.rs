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

//! Port of `MultiFacetQuery.java`, of the Java package
//! `org.apache.lucene.facet`.

use std::any::Any;
use std::sync::Arc;

use rucene::error::Result;
use rucene::search::term_in_set_query::TermInSetQuery;
use rucene::search::{IndexSearcher, Query, QueryVisitor, ScoreMode, Weight};
use rucene::util::BytesRef;

use crate::facets_config::{default_dim_config, FacetsConfig};

/// A multi-terms `Query` over a `FacetField`.
///
/// Equivalent to `org.apache.lucene.facet.MultiFacetQuery`, a
/// `TermInSetQuery` subclass: it delegates to the [`TermInSetQuery`] it
/// extends and compares equal only to another `MultiFacetQuery`.
#[derive(Debug, Clone)]
pub struct MultiFacetQuery {
    inner: TermInSetQuery,
}

impl MultiFacetQuery {
    /// Creates a new `MultiFacetQuery` filtering the query on the given
    /// dimension.
    ///
    /// Equivalent to `MultiFacetQuery(FacetsConfig, String, String[]...)`.
    pub fn new(facets_config: &FacetsConfig, dimension: &str, paths: &[&[&str]]) -> Result<Self> {
        Ok(Self {
            inner: TermInSetQuery::new(
                facets_config.get_dim_config(dimension).index_field_name,
                to_terms(dimension, paths)?,
            )?,
        })
    }

    /// Creates a new `MultiFacetQuery` filtering the query on the given
    /// dimension. NOTE: uses the default `FacetsConfig.DEFAULT_DIM_CONFIG`.
    ///
    /// Equivalent to `MultiFacetQuery(String, String[]...)`.
    pub fn with_default_config(dimension: &str, paths: &[&[&str]]) -> Result<Self> {
        Ok(Self {
            inner: TermInSetQuery::new(
                default_dim_config().index_field_name,
                to_terms(dimension, paths)?,
            )?,
        })
    }

    /// The `TermInSetQuery` this query extends.
    pub fn term_in_set_query(&self) -> &TermInSetQuery {
        &self.inner
    }
}

/// Equivalent to `MultiFacetQuery.toTerms(String, String[]...)`.
pub(crate) fn to_terms(dimension: &str, paths: &[&[&str]]) -> Result<Vec<BytesRef>> {
    let mut terms = Vec::with_capacity(paths.len());
    for path in paths {
        terms.push(BytesRef::from_char_sequence(&FacetsConfig::path_to_string_dim(
            dimension, path,
        )?));
    }
    Ok(terms)
}

impl Query for MultiFacetQuery {
    fn to_query_string(&self, field: &str) -> String {
        self.inner.to_query_string(field)
    }

    fn visit(&self, visitor: &mut dyn QueryVisitor) {
        self.inner.visit(visitor)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn create_weight(
        &self,
        searcher: &IndexSearcher,
        score_mode: ScoreMode,
        boost: f32,
    ) -> Result<Arc<dyn Weight>> {
        self.inner.create_weight(searcher, score_mode, boost)
    }

    fn rewrite(&self, searcher: &IndexSearcher) -> Result<Option<Arc<dyn Query>>> {
        self.inner.rewrite(searcher)
    }

    fn is_multi_term_query(&self) -> bool {
        self.inner.is_multi_term_query()
    }

    fn as_multi_term_query(&self) -> Option<&dyn rucene::search::multi_term_query::MultiTermQuery> {
        self.inner.as_multi_term_query()
    }

    fn query_eq(&self, other: &dyn Query) -> bool {
        self.same_class_as(other)
            && other
                .as_any()
                .downcast_ref::<MultiFacetQuery>()
                .is_some_and(|o| self.inner.query_eq_ignoring_class(&o.inner))
    }

    fn query_hash(&self) -> i32 {
        self.inner.query_hash_with_class_hash(self.class_hash())
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.facet.MultiFacetQuery"
    }
}
