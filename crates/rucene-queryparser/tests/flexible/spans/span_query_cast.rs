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

//! The cast `(SpanQuery) obj` of the span helpers of
//! `org.apache.lucene.queryparser.flexible.spans`, over a built object.
//!
//! The span builders implement `StandardQueryBuilder`, so the object a
//! node's tag holds is the built query as a `Query` (`query_object`). Rust
//! cannot cast an `Arc<dyn Query>` to an `Arc<dyn SpanQuery>`; the cast is a
//! downcast to the concrete span classes these builders build
//! (`SpanTermQuery`, `SpanOrQuery`), the precedent of
//! `SpanMultiTermQueryWrapper`'s `to_span_query` in `rucene-queries`. Any
//! other object gives Java's `ClassCastException`.

use std::sync::Arc;

use rucene::error::{LuceneError, Result};
use rucene_queries::spans::span_or_query::SpanOrQuery;
use rucene_queries::spans::span_query::SpanQuery;
use rucene_queries::spans::span_term_query::SpanTermQuery;
use rucene_queryparser::flexible::core::nodes::query_node::JavaObjectRef;
use rucene_queryparser::flexible::standard::builders::standard_query_builder::query_of;

/// `(SpanQuery) obj`; `None` for Java's `null`.
///
/// # Errors
///
/// The `ClassCastException` of an object that is no span query.
pub fn span_query_of(obj: Option<&JavaObjectRef>) -> Result<Option<Arc<dyn SpanQuery>>> {
    let Some(obj) = obj else {
        return Ok(None);
    };
    let query = query_of(obj)?;
    if let Some(q) = query.as_any().downcast_ref::<SpanTermQuery>() {
        return Ok(Some(Arc::new(q.clone())));
    }
    if let Some(q) = query.as_any().downcast_ref::<SpanOrQuery>() {
        return Ok(Some(Arc::new(q.clone())));
    }
    Err(LuceneError::unchecked(
        "java.lang.ClassCastException",
        format!(
            "class {} cannot be cast to class org.apache.lucene.queries.spans.SpanQuery",
            query.java_class_name()
        ),
    ))
}
