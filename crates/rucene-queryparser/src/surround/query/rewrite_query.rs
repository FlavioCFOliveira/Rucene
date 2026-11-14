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

//! The port of `RewriteQuery.java`, of the Java package
//! `org.apache.lucene.queryparser.surround.query`.

use std::sync::Arc;

use rucene::error::Result;
use rucene::util::java_collections::java_string_hash_code;

use super::basic_query_factory::BasicQueryFactory;
use super::srnd_query::SrndQuery;

/// The fields and the shared behaviour of the abstract class
/// `RewriteQuery<SQ extends SrndQuery>`, a Lucene `Query` that rewrites a
/// surround query: the surround query, the field name and the basic query
/// factory.
///
/// Equivalent to the package-private
/// `org.apache.lucene.queryparser.surround.query.RewriteQuery`. Its
/// subclasses, `DistanceRewriteQuery` and `SimpleTermRewriteQuery`, hold one
/// and implement `Query`, each keeping the surround query also as the type
/// its `rewrite` needs.
#[derive(Debug)]
pub(crate) struct RewriteQuery {
    /// `protected final SQ srndQuery`.
    pub(crate) srnd_query: Arc<dyn SrndQuery>,
    /// `protected final String fieldName`.
    pub(crate) field_name: String,
    /// `protected final BasicQueryFactory qf`.
    pub(crate) qf: Arc<BasicQueryFactory>,
}

impl RewriteQuery {
    /// Equivalent to the constructor `RewriteQuery(SQ, String,
    /// BasicQueryFactory)`.
    ///
    /// # Errors
    ///
    /// The `NullPointerException` of `Objects.requireNonNull` for a `null`
    /// field name.
    pub(crate) fn new(
        srnd_query: Arc<dyn SrndQuery>,
        field_name: Option<&str>,
        qf: &Arc<BasicQueryFactory>,
    ) -> Result<Self> {
        Ok(Self {
            srnd_query,
            field_name: crate::javacc::nn_str(field_name)?.to_string(),
            qf: Arc::clone(qf),
        })
    }

    /// Equivalent to `toString(String)`, with `class` the subclass name.
    pub(crate) fn to_string(&self, class: &str, field: &str) -> String {
        format!(
            "{class}{}({}, {}, {})",
            if field.is_empty() {
                String::new()
            } else {
                format!("(unused: {field})")
            },
            self.field_name,
            self.srnd_query.srnd_to_string(),
            self.qf.java_to_string()
        )
    }

    /// Equivalent to `hashCode()`, with `class_hash` the subclass's
    /// `classHash()`.
    pub(crate) fn hash_code(&self, class_hash: i32) -> i32 {
        class_hash
            ^ java_string_hash_code(&self.field_name)
            ^ self.qf.java_hash_code()
            ^ self.srnd_query.srnd_hash_code()
    }

    /// Equivalent to the private `equalsTo(RewriteQuery<?>)`.
    pub(crate) fn equals_to(&self, other: &RewriteQuery) -> bool {
        self.field_name == other.field_name
            && self.qf.java_equals(&other.qf)
            && self.srnd_query.srnd_equals(other.srnd_query.as_ref())
    }
}
