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

//! The types declared in `Judge.java`, of the Java package
//! `org.apache.lucene.benchmark.quality`.

use crate::jdk::PrintWriter;
use crate::quality::quality_query::QualityQuery;

/// Equivalent to `org.apache.lucene.benchmark.quality.Judge` (`public
/// interface Judge`): judge if a document is relevant for a quality query.
pub trait Judge: Send + Sync {
    /// `isRelevant(String docName, QualityQuery query)`: judge if document
    /// `doc_name` is relevant for the given quality query.
    fn is_relevant(&self, doc_name: Option<&str>, query: &QualityQuery) -> bool;

    /// `validateData(QualityQuery[] qq, PrintWriter logger)`: validate that
    /// queries and this Judge match each other. To be perfectly valid, this
    /// Judge must have some data for each and every input quality query, and
    /// must not have any data on any other quality query. **Note**: the
    /// quality benchmark run would not fail in case of imperfect validity,
    /// just a warning message would be logged. If `logger` is not `None`,
    /// validation issues are logged.
    fn validate_data(&self, qq: &[QualityQuery], logger: Option<&PrintWriter>) -> bool;

    /// `maxRecall(QualityQuery query)`: the maximal recall for the input
    /// quality query. It is the number of relevant docs this Judge "knows"
    /// for the query.
    fn max_recall(&self, query: &QualityQuery) -> i32;
}
