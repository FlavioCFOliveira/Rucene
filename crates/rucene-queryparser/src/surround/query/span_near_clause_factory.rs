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

//! The port of `SpanNearClauseFactory.java`, of the Java package
//! `org.apache.lucene.queryparser.surround.query`.

use std::sync::Arc;

use rucene::error::{LuceneError, LuceneErrorKind, Result};
use rucene::index::{IndexReader, Term};
use rucene::search::match_no_docs_query::MatchNoDocsQuery;
use rucene::search::Query;
use rucene_queries::spans::field_masking_span_query::FieldMaskingSpanQuery;
use rucene_queries::spans::span_containing_query::SpanContainingQuery;
use rucene_queries::spans::span_first_query::SpanFirstQuery;
use rucene_queries::spans::span_near_query::SpanNearQuery;
use rucene_queries::spans::span_not_query::SpanNotQuery;
use rucene_queries::spans::span_or_query::SpanOrQuery;
use rucene_queries::spans::span_position_range_query::SpanPositionRangeQuery;
use rucene_queries::spans::span_query::SpanQuery;
use rucene_queries::spans::span_term_query::SpanTermQuery;
use rucene_queries::spans::span_within_query::SpanWithinQuery;

use super::basic_query_factory::BasicQueryFactory;

/// `HashMap.DEFAULT_INITIAL_CAPACITY`.
const DEFAULT_INITIAL_CAPACITY: usize = 16;
/// `HashMap.DEFAULT_LOAD_FACTOR`.
const DEFAULT_LOAD_FACTOR: f32 = 0.75;
/// `HashMap.TREEIFY_THRESHOLD`.
const TREEIFY_THRESHOLD: usize = 8;
/// `HashMap.MIN_TREEIFY_CAPACITY`.
const MIN_TREEIFY_CAPACITY: usize = 64;

/// One chain of [`SpanQueryWeights`]: `(hash, key, value)` in insertion
/// order.
type Bucket = Vec<(i32, Arc<dyn SpanQuery>, f32)>;

/// The `HashMap<SpanQuery, Float>` behind `weightBySpanQuery`, with Java's
/// iteration order: `makeSpanClause` builds its `SpanOrQuery` from the key
/// set in that order.
///
/// Reproduces what decides `java.util.HashMap`'s order for keys hashed by
/// `Query.hashCode()` and compared by `Query.equals`: the spreading
/// `h ^ (h >>> 16)`, a power-of-two table allocated at 16, insertion-ordered
/// chains, the doubling resize at a load factor of 0.75 with its
/// order-preserving split, the extra resize of `treeifyBin` on a short
/// table, and `clear()` keeping the table length. `put` on a present key
/// keeps the first key object and replaces the value, as Java does.
///
/// **Divergence.** A chain of more than `TREEIFY_THRESHOLD` colliding keys
/// on a table of at least `MIN_TREEIFY_CAPACITY` becomes a red-black tree in
/// Java, whose chain is re-headed at the tree root; this map keeps the chain,
/// as [`JavaHashMap`](rucene::util::java_collections::JavaHashMap) does.
#[derive(Debug, Default)]
struct SpanQueryWeights {
    table: Vec<Bucket>,
    size: usize,
    threshold: usize,
}

impl SpanQueryWeights {
    /// `HashMap.hash(Object)`.
    fn hash(sq: &dyn SpanQuery) -> i32 {
        let h = sq.query_hash();
        h ^ ((h as u32) >> 16) as i32
    }

    fn index(hash: i32, n: usize) -> usize {
        (hash as u32 as usize) & (n - 1)
    }

    /// `HashMap.resize()`.
    fn resize(&mut self) {
        let old_cap = self.table.len();
        let new_cap = if old_cap == 0 {
            DEFAULT_INITIAL_CAPACITY
        } else {
            old_cap << 1
        };
        self.threshold = (new_cap as f32 * DEFAULT_LOAD_FACTOR) as usize;
        let old = std::mem::take(&mut self.table);
        let mut table: Vec<Bucket> = Vec::with_capacity(new_cap);
        table.resize_with(new_cap, Vec::new);
        for bucket in old {
            for entry in bucket {
                table[Self::index(entry.0, new_cap)].push(entry);
            }
        }
        self.table = table;
    }

    /// `HashMap.get(Object)`.
    fn get(&self, sq: &dyn SpanQuery) -> Option<f32> {
        if self.table.is_empty() {
            return None;
        }
        let hash = Self::hash(sq);
        self.table[Self::index(hash, self.table.len())]
            .iter()
            .find(|(h, k, _)| *h == hash && k.query_eq(sq))
            .map(|(_, _, v)| *v)
    }

    /// `HashMap.put(K, V)`.
    fn put(&mut self, sq: Arc<dyn SpanQuery>, w: f32) {
        if self.table.is_empty() {
            self.resize();
        }
        let hash = Self::hash(sq.as_ref());
        let n = self.table.len();
        let bucket = &mut self.table[Self::index(hash, n)];
        for entry in bucket.iter_mut() {
            if entry.0 == hash && entry.1.query_eq(sq.as_ref()) {
                entry.2 = w;
                return;
            }
        }
        bucket.push((hash, sq, w));
        if bucket.len() > TREEIFY_THRESHOLD && n < MIN_TREEIFY_CAPACITY {
            self.resize();
        }
        self.size += 1;
        if self.size > self.threshold {
            self.resize();
        }
    }

    /// `HashMap.clear()`: the table keeps its length.
    fn clear(&mut self) {
        for bucket in &mut self.table {
            bucket.clear();
        }
        self.size = 0;
    }

    /// `keySet().iterator()`.
    fn keys(&self) -> impl Iterator<Item = &Arc<dyn SpanQuery>> {
        self.table.iter().flatten().map(|(_, k, _)| k)
    }
}

/// `q instanceof SpanQuery`, with the query as a [`SpanQuery`]: the span
/// queries of `rucene-queries`.
fn as_span_query(q: &Arc<dyn Query>) -> Option<Arc<dyn SpanQuery>> {
    let any = q.as_any();
    macro_rules! try_span {
        ($($t:ty),*) => {
            $(if let Some(s) = any.downcast_ref::<$t>() {
                return Some(Arc::new(s.clone()));
            })*
        };
    }
    try_span!(
        SpanNearQuery,
        SpanOrQuery,
        SpanNotQuery,
        SpanTermQuery,
        SpanFirstQuery,
        SpanContainingQuery,
        SpanWithinQuery,
        SpanPositionRangeQuery,
        FieldMaskingSpanQuery
    );
    None
}

/// Factory for the clauses of a span near query: weighted terms and
/// weighted sub-queries, made into one span clause.
///
/// Equivalent to `org.apache.lucene.queryparser.surround.query.SpanNearClauseFactory`.
#[derive(Debug)]
pub struct SpanNearClauseFactory {
    reader: Arc<dyn IndexReader>,
    field_name: String,
    weight_by_span_query: SpanQueryWeights,
    qf: Arc<BasicQueryFactory>,
}

impl SpanNearClauseFactory {
    /// Equivalent to `new SpanNearClauseFactory(IndexReader, String,
    /// BasicQueryFactory)`.
    pub fn new(reader: Arc<dyn IndexReader>, field_name: &str, qf: Arc<BasicQueryFactory>) -> Self {
        Self {
            reader,
            field_name: field_name.to_string(),
            weight_by_span_query: SpanQueryWeights::default(),
            qf,
        }
    }

    /// Equivalent to `getIndexReader()`.
    pub fn get_index_reader(&self) -> &Arc<dyn IndexReader> {
        &self.reader
    }

    /// Equivalent to `getFieldName()`.
    pub fn get_field_name(&self) -> &str {
        &self.field_name
    }

    /// Equivalent to `getBasicQueryFactory()`.
    pub fn get_basic_query_factory(&self) -> &Arc<BasicQueryFactory> {
        &self.qf
    }

    /// Equivalent to `size()`.
    pub fn size(&self) -> i32 {
        self.weight_by_span_query.size as i32
    }

    /// Equivalent to `clear()`.
    pub fn clear(&mut self) {
        self.weight_by_span_query.clear();
    }

    /// Equivalent to the protected `addSpanQueryWeighted(SpanQuery, float)`.
    pub fn add_span_query_weighted(&mut self, sq: Arc<dyn SpanQuery>, weight: f32) {
        let w = match self.weight_by_span_query.get(sq.as_ref()) {
            Some(w) => w + weight,
            None => weight,
        };
        self.weight_by_span_query.put(sq, w);
    }

    /// Equivalent to `addTermWeighted(Term, float)`.
    ///
    /// # Errors
    ///
    /// The `TooManyBasicQueries` of the factory.
    pub fn add_term_weighted(&mut self, t: Term, weight: f32) -> Result<()> {
        let stq = self.qf.new_span_term_query(t)?;
        /* CHECKME: wrap in Hashable...? */
        self.add_span_query_weighted(Arc::new(stq), weight);
        Ok(())
    }

    /// Equivalent to `addSpanQuery(Query)`.
    ///
    /// # Errors
    ///
    /// `AssertionError("Expected SpanQuery: " + q.toString(getFieldName()))`
    /// for a query that is neither a `MatchNoDocsQuery` nor a span query.
    pub fn add_span_query(&mut self, q: Arc<dyn Query>) -> Result<()> {
        if q.as_any().is::<MatchNoDocsQuery>() {
            return Ok(());
        }
        let Some(sq) = as_span_query(&q) else {
            return Err(LuceneError::from(LuceneErrorKind::AssertionError(format!(
                "Expected SpanQuery: {}",
                q.to_query_string(self.get_field_name())
            ))));
        };
        let boost = 1.0f32;
        self.add_span_query_weighted(sq, boost);
        Ok(())
    }

    /// Equivalent to `makeSpanClause()`.
    ///
    /// # Errors
    ///
    /// The errors of `new SpanOrQuery`.
    pub fn make_span_clause(&self) -> Result<Arc<dyn SpanQuery>> {
        let span_queries: Vec<Arc<dyn SpanQuery>> =
            self.weight_by_span_query.keys().cloned().collect();
        if span_queries.len() == 1 {
            Ok(Arc::clone(&span_queries[0]))
        } else {
            Ok(Arc::new(SpanOrQuery::new(span_queries)?))
        }
    }
}
