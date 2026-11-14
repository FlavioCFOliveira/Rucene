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

//! The port of `SrndBooleanQuery.java`, of the Java package
//! `org.apache.lucene.queryparser.surround.query`.

use std::sync::Arc;

use rucene::error::{LuceneError, LuceneErrorKind, Result};
use rucene::search::boolean_query;
use rucene::search::{Occur, Query};

/// The static helpers that build the boolean queries of the surround
/// queries.
///
/// Equivalent to the package-private class
/// `org.apache.lucene.queryparser.surround.query.SrndBooleanQuery`.
pub(crate) struct SrndBooleanQuery;

impl SrndBooleanQuery {
    /// Equivalent to `addQueriesToBoolean(BooleanQuery.Builder, List<Query>,
    /// BooleanClause.Occur)`.
    ///
    /// # Errors
    ///
    /// The `TooManyClauses` of the builder.
    pub(crate) fn add_queries_to_boolean(
        bq: &mut boolean_query::Builder,
        queries: &[Arc<dyn Query>],
        occur: Occur,
    ) -> Result<()> {
        for q in queries {
            bq.add(Arc::clone(q), occur)?;
        }
        Ok(())
    }

    /// Equivalent to `makeBooleanQuery(List<Query>, BooleanClause.Occur)`.
    ///
    /// # Errors
    ///
    /// `AssertionError("Too few subqueries: " + size)` for fewer than two
    /// queries, and the `TooManyClauses` of the builder.
    pub(crate) fn make_boolean_query(
        queries: &[Arc<dyn Query>],
        occur: Occur,
    ) -> Result<Arc<dyn Query>> {
        if queries.len() <= 1 {
            return Err(LuceneError::from(LuceneErrorKind::AssertionError(format!(
                "Too few subqueries: {}",
                queries.len()
            ))));
        }
        let mut bq = boolean_query::Builder::new();
        Self::add_queries_to_boolean(&mut bq, queries, occur)?;
        Ok(Arc::new(bq.build()))
    }
}
