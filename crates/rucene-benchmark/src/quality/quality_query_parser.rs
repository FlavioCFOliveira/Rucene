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

//! The types declared in `QualityQueryParser.java`, of the Java package
//! `org.apache.lucene.benchmark.quality`.

use std::sync::Arc;

use rucene::error::Result;
use rucene::search::query::Query;

use crate::quality::quality_query::QualityQuery;

/// Equivalent to `org.apache.lucene.benchmark.quality.QualityQueryParser`
/// (`public interface QualityQueryParser`): parse a QualityQuery into a
/// Lucene query.
pub trait QualityQueryParser: Send + Sync {
    /// `parse(QualityQuery qq)`: parse a given QualityQuery into a Lucene
    /// query.
    ///
    /// # Errors
    ///
    /// `ParseException` if parsing failed.
    fn parse(&self, qq: &QualityQuery) -> Result<Arc<dyn Query>>;
}
