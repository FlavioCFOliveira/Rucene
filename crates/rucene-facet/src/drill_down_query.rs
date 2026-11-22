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

//! Port of `DrillDownQuery.java`, of the Java package
//! `org.apache.lucene.facet`.

use std::any::Any;
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use rucene::error::Result;
use rucene::index::Term;
use rucene::internal::hppc::IntHashSet;
use rucene::search::boolean_query::{BooleanQuery, Builder as BooleanQueryBuilder};
use rucene::search::match_all_docs_query::MatchAllDocsQuery;
use rucene::search::{IndexSearcher, Occur, Query, QueryVisitor, TermQuery};
use rucene::util::java_collections::java_identity_hash_code_stand_in;
use rucene::util::BytesRef;

use crate::facets_config::FacetsConfig;
use crate::label_and_value::objects_hash;

/// A dimension's `BooleanQuery.Builder`, shared between a query and its
/// clones as Java shares the builder objects; compared by identity as
/// `BooleanQuery.Builder` does not override `equals`.
pub type SharedBuilder = Arc<Mutex<BooleanQueryBuilder>>;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Creates a drill-down term.
///
/// Equivalent to `DrillDownQuery.term(String, String, String...)`.
pub fn term(field: &str, dim: &str, path: &[&str]) -> Result<Term> {
    Ok(Term::new(
        field.to_string(),
        BytesRef::from_char_sequence(&FacetsConfig::path_to_string_dim(dim, path)?),
    ))
}

/// The lazily built dimension queries.
struct Built {
    built_dim_queries: Vec<Option<Arc<dyn Query>>>,
    dirty_dim_query_index: IntHashSet,
}

/// A `Query` for drill-down over facet categories.
///
/// Equivalent to `org.apache.lucene.facet.DrillDownQuery`.
pub struct DrillDownQuery {
    config: Arc<FacetsConfig>,
    base_query: Option<Arc<dyn Query>>,
    dim_queries: Vec<SharedBuilder>,
    /// `LinkedHashMap<String, Integer>`: insertion order.
    drill_down_dims: Vec<(String, i32)>,
    built: Mutex<Built>,
}

impl fmt::Debug for DrillDownQuery {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DrillDownQuery")
            .field("drill_down_dims", &self.drill_down_dims)
            .finish_non_exhaustive()
    }
}

impl DrillDownQuery {
    /// Used by `clone()` and `DrillSideways`.
    ///
    /// Equivalent to the package-private `DrillDownQuery(FacetsConfig, Query,
    /// List, Map)`.
    pub(crate) fn from_parts(
        config: Arc<FacetsConfig>,
        base_query: Option<Arc<dyn Query>>,
        dim_queries: Vec<SharedBuilder>,
        drill_down_dims: Vec<(String, i32)>,
    ) -> Self {
        let n = dim_queries.len();
        let mut dirty = IntHashSet::new();
        for i in 0..n {
            dirty.add(i as i32);
        }
        Self {
            config,
            base_query,
            dim_queries,
            drill_down_dims,
            built: Mutex::new(Built {
                built_dim_queries: vec![None; n],
                dirty_dim_query_index: dirty,
            }),
        }
    }

    /// Used by `DrillSideways`.
    ///
    /// Equivalent to the package-private `DrillDownQuery(FacetsConfig, Query,
    /// DrillDownQuery)`.
    pub(crate) fn with_filter(config: Arc<FacetsConfig>, filter: Arc<dyn Query>, other: &DrillDownQuery) -> Result<Self> {
        let mut bq = BooleanQueryBuilder::new();
        bq.add(
            other
                .base_query
                .clone()
                .unwrap_or_else(|| Arc::new(MatchAllDocsQuery::instance())),
            Occur::MUST,
        )?;
        bq.add(filter, Occur::FILTER)?;
        let base_query: Arc<dyn Query> = Arc::new(bq.build());
        Ok(Self::from_parts(
            config,
            Some(base_query),
            other.dim_queries.clone(),
            other.drill_down_dims.clone(),
        ))
    }

    /// Creates a new `DrillDownQuery` without a base query, to perform a pure
    /// browsing query (equivalent to using `MatchAllDocsQuery` as base).
    ///
    /// Equivalent to `DrillDownQuery(FacetsConfig)`.
    pub fn new(config: Arc<FacetsConfig>) -> Self {
        Self::with_base_query(config, None)
    }

    /// Creates a new `DrillDownQuery` over the given base query.
    ///
    /// Equivalent to `DrillDownQuery(FacetsConfig, Query)`.
    pub fn with_base_query(config: Arc<FacetsConfig>, base_query: Option<Arc<dyn Query>>) -> Self {
        Self {
            config,
            base_query,
            dim_queries: Vec::new(),
            drill_down_dims: Vec::new(),
            built: Mutex::new(Built {
                built_dim_queries: Vec::new(),
                dirty_dim_query_index: IntHashSet::new(),
            }),
        }
    }

    /// Adds one dimension of drill downs; if you pass the same dimension more
    /// than once it is OR'd with the previous constraints on that dimension,
    /// and all dimensions are AND'd against each other and the base query.
    ///
    /// Equivalent to `add(String, String...)`.
    pub fn add(&mut self, dim: &str, path: &[&str]) -> Result<()> {
        let indexed_field = self.config.get_dim_config(dim).index_field_name;
        self.add_query(dim, Arc::new(TermQuery::new(term(&indexed_field, dim, path)?)))
    }

    /// Expert: add a custom drill-down subQuery.
    ///
    /// Equivalent to `add(String, Query)`.
    pub fn add_query(&mut self, dim: &str, sub_query: Arc<dyn Query>) -> Result<()> {
        let mut built = lock(&self.built);
        debug_assert!(self.dim_queries.len() == built.built_dim_queries.len());
        debug_assert!(self.drill_down_dims.len() == self.dim_queries.len());
        if !self.drill_down_dims.iter().any(|(d, _)| d == dim) {
            let index = self.drill_down_dims.len() as i32;
            self.drill_down_dims.push((dim.to_string(), index));
            self.dim_queries.push(Arc::new(Mutex::new(BooleanQueryBuilder::new())));
            built.built_dim_queries.push(None);
        }
        let index = self
            .drill_down_dims
            .iter()
            .find(|(d, _)| d == dim)
            .map(|(_, i)| *i)
            .expect("INVARIANT: added above");
        lock(&self.dim_queries[index as usize]).add(sub_query, Occur::SHOULD)?;
        built.dirty_dim_query_index.add(index);
        Ok(())
    }

    /// Equivalent to `clone()`: shares the dimension builders.
    pub fn clone_query(&self) -> DrillDownQuery {
        Self::from_parts(
            Arc::clone(&self.config),
            self.base_query.clone(),
            self.dim_queries.clone(),
            self.drill_down_dims.clone(),
        )
    }

    /// Equivalent to `equalsTo(DrillDownQuery)`.
    fn equals_to(&self, other: &DrillDownQuery) -> bool {
        let base_eq = match (&self.base_query, &other.base_query) {
            (None, None) => true,
            (Some(a), Some(b)) => a.query_eq(&**b),
            _ => false,
        };
        base_eq
            && self.dim_queries.len() == other.dim_queries.len()
            && self
                .dim_queries
                .iter()
                .zip(&other.dim_queries)
                .all(|(a, b)| Arc::ptr_eq(a, b))
    }

    /// Equivalent to `getBooleanQuery()`.
    fn get_boolean_query(&self) -> Result<BooleanQuery> {
        let mut bq = BooleanQueryBuilder::new();
        if let Some(base_query) = &self.base_query {
            bq.add(Arc::clone(base_query), Occur::MUST)?;
        }
        for query in self.get_drill_down_queries() {
            bq.add(query, Occur::FILTER)?;
        }
        Ok(bq.build())
    }

    /// Returns the internal baseQuery of the DrillDownQuery.
    ///
    /// Equivalent to `getBaseQuery()`.
    pub fn get_base_query(&self) -> Option<&Arc<dyn Query>> {
        self.base_query.as_ref()
    }

    /// Returns the dimension queries added either via `add(String, Query)` or
    /// `add(String, String...)`.
    ///
    /// Equivalent to `getDrillDownQueries()`.
    pub fn get_drill_down_queries(&self) -> Vec<Arc<dyn Query>> {
        let mut built = lock(&self.built);
        let dirty: Vec<i32> = built.dirty_dim_query_index.iter().map(|c| c.value).collect();
        for dirty_dim_index in dirty {
            let query: Arc<dyn Query> = Arc::new(lock(&self.dim_queries[dirty_dim_index as usize]).build());
            built.built_dim_queries[dirty_dim_index as usize] = Some(query);
        }
        built.dirty_dim_query_index.clear();
        built
            .built_dim_queries
            .iter()
            .map(|q| Arc::clone(q.as_ref().expect("INVARIANT: every dim query is built")))
            .collect()
    }

    /// Equivalent to the package-private `getDims()`.
    pub(crate) fn get_dims(&self) -> &[(String, i32)] {
        &self.drill_down_dims
    }
}

impl Query for DrillDownQuery {
    fn to_query_string(&self, field: &str) -> String {
        self.get_boolean_query()
            .map(|q| q.to_query_string(field))
            .unwrap_or_default()
    }

    fn visit(&self, visitor: &mut dyn QueryVisitor) {
        visitor.visit_leaf(self);
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn rewrite(&self, _index_searcher: &IndexSearcher) -> Result<Option<Arc<dyn Query>>> {
        let rewritten = self.get_boolean_query()?;
        if rewritten.clauses().is_empty() {
            return Ok(Some(Arc::new(MatchAllDocsQuery::instance())));
        }
        Ok(Some(Arc::new(rewritten)))
    }

    fn query_eq(&self, other: &dyn Query) -> bool {
        self.same_class_as(other)
            && other
                .as_any()
                .downcast_ref::<DrillDownQuery>()
                .is_some_and(|o| self.equals_to(o))
    }

    fn query_hash(&self) -> i32 {
        // `classHash() + Objects.hash(baseQuery, dimQueries)`; a builder's
        // hash is its identity hash.
        let base = self.base_query.as_ref().map_or(0, |q| q.query_hash());
        let mut list_hash = 1i32;
        for _ in &self.dim_queries {
            list_hash = list_hash.wrapping_mul(31).wrapping_add(java_identity_hash_code_stand_in(
                "org.apache.lucene.search.BooleanQuery$Builder",
            ));
        }
        self.class_hash().wrapping_add(objects_hash(&[base, list_hash]))
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.facet.DrillDownQuery"
    }
}
