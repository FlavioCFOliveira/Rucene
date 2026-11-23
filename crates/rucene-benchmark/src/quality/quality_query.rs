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

//! The types declared in `QualityQuery.java`, of the Java package
//! `org.apache.lucene.benchmark.quality`.

use std::cmp::Ordering;

use rucene::util::java_collections::JavaHashMap;

/// Equivalent to `org.apache.lucene.benchmark.quality.QualityQuery`
/// (`public class QualityQuery implements Comparable<QualityQuery>`): a
/// QualityQuery has an ID and some name-value pairs.
///
/// The ID allows to map the quality query with its judgements.
///
/// The name-value pairs are used by a
/// [`QualityQueryParser`](crate::quality::quality_query_parser::QualityQueryParser)
/// to create a Lucene `Query`.
///
/// It is very likely that name-value-pairs would be mapped into fields in a
/// Lucene query, but it is up to the QualityQueryParser how to map - e.g.
/// all values in a single field, or each pair as its own field, etc., - and
/// this of course must match the way the searched index was constructed.
#[derive(Clone, Debug)]
pub struct QualityQuery {
    query_id: String,
    /// `nameValPairs`: the `HashMap` the readers build, in its iteration
    /// order.
    name_val_pairs: JavaHashMap<String>,
}

impl QualityQuery {
    /// `QualityQuery(String queryID, Map<String, String> nameValPairs)`:
    /// create a QualityQuery with given ID and name-value pairs.
    pub fn new(query_id: String, name_val_pairs: JavaHashMap<String>) -> Self {
        QualityQuery { query_id, name_val_pairs }
    }

    /// `getNames()`: return all the names of name-value-pairs in this
    /// QualityQuery.
    pub fn get_names(&self) -> Vec<String> {
        self.name_val_pairs.keys().cloned().collect()
    }

    /// `getValue(String name)`: return the value of a certain name-value
    /// pair.
    pub fn get_value(&self, name: &str) -> Option<&str> {
        self.name_val_pairs.get(name).map(String::as_str)
    }

    /// `getQueryID()`: return the ID of this query. The ID allows to map the
    /// quality query with its judgements.
    pub fn get_query_id(&self) -> &str {
        &self.query_id
    }

    /// `compareTo(QualityQuery other)`: for a nicer sort of input queries
    /// before running them. Try first as ints, fall back to string if not
    /// int.
    pub fn compare_to(&self, other: &QualityQuery) -> i32 {
        // compare as ints when ids ints
        match (
            rucene::util::extra::java_parse_int(&self.query_id),
            rucene::util::extra::java_parse_int(&other.query_id),
        ) {
            (Some(n), Some(n_other)) => n.wrapping_sub(n_other),
            // fall back to string comparison
            _ => match self.query_id.encode_utf16().cmp(other.query_id.encode_utf16()) {
                Ordering::Less => -1,
                Ordering::Equal => 0,
                Ordering::Greater => 1,
            },
        }
    }
}

/// `Arrays.sort(qq)` by [`QualityQuery::compare_to`] (a stable sort, as
/// Java's for objects).
pub fn sort_quality_queries(qq: &mut [QualityQuery]) {
    qq.sort_by(|a, b| a.compare_to(b).cmp(&0));
}
