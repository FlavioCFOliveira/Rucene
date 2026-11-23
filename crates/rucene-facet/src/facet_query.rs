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

//! Port of `FacetQuery.java`, of the Java package `org.apache.lucene.facet`.

use std::any::Any;
use std::sync::Arc;

use rucene::error::Result;
use rucene::index::Term;
use rucene::search::{IndexSearcher, Query, QueryVisitor, ScoreMode, TermQuery, Weight};

use crate::facets_config::{default_dim_config, DimConfig, FacetsConfig};

/// A term `Query` over a `FacetField`.
///
/// Equivalent to `org.apache.lucene.facet.FacetQuery`, a `TermQuery`
/// subclass: it delegates to the [`TermQuery`] it extends, and compares
/// equal only to another `FacetQuery` (`sameClassAs`). It dereferences to
/// that `TermQuery`, so the inherited public members (`getTerm()`,
/// `getTermStates()`) are reachable on it, as in Java.
#[derive(Debug, Clone)]
pub struct FacetQuery {
    inner: TermQuery,
}

impl FacetQuery {
    /// Creates a new `FacetQuery` filtering the query on the given
    /// dimension.
    ///
    /// Equivalent to `FacetQuery(FacetsConfig, String, String...)`.
    pub fn new(facets_config: &FacetsConfig, dimension: &str, path: &[&str]) -> Result<Self> {
        Ok(Self {
            inner: TermQuery::new(to_term(
                &facets_config.get_dim_config(dimension),
                dimension,
                path,
            )?),
        })
    }

    /// Creates a new `FacetQuery` filtering the query on the given dimension.
    /// NOTE: uses the default `FacetsConfig.DEFAULT_DIM_CONFIG`.
    ///
    /// Equivalent to `FacetQuery(String, String...)`.
    pub fn with_default_config(dimension: &str, path: &[&str]) -> Result<Self> {
        Ok(Self {
            inner: TermQuery::new(to_term(&default_dim_config(), dimension, path)?),
        })
    }

    /// The `TermQuery` this query extends.
    pub fn term_query(&self) -> &TermQuery {
        &self.inner
    }
}

impl std::ops::Deref for FacetQuery {
    type Target = TermQuery;

    fn deref(&self) -> &TermQuery {
        &self.inner
    }
}

/// Equivalent to `FacetQuery.toTerm(DimConfig, String, String...)`.
pub(crate) fn to_term(dim_config: &DimConfig, dimension: &str, path: &[&str]) -> Result<Term> {
    Ok(Term::new(
        dim_config.index_field_name(),
        rucene::util::BytesRef::from_char_sequence(&FacetsConfig::path_to_string_dim(
            dimension, path,
        )?),
    ))
}

impl Query for FacetQuery {
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

    fn as_term_query(&self) -> Option<&TermQuery> {
        Some(&self.inner)
    }

    fn query_eq(&self, other: &dyn Query) -> bool {
        self.same_class_as(other)
            && other
                .as_any()
                .downcast_ref::<FacetQuery>()
                .is_some_and(|o| self.inner.get_term() == o.inner.get_term())
    }

    fn query_hash(&self) -> i32 {
        // `classHash() ^ term.hashCode()`.
        self.class_hash() ^ self.inner.get_term().hash_code()
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.facet.FacetQuery"
    }
}
