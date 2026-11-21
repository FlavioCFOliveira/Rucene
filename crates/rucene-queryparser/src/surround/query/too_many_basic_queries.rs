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

//! The port of `TooManyBasicQueries.java`, of the Java package
//! `org.apache.lucene.queryparser.surround.query`.

use std::sync::Arc;

use rucene::error::{JavaThrowable, LuceneError, LuceneErrorKind};

/// Exception thrown when `BasicQueryFactory` would exceed the limit of query
/// clauses.
///
/// Equivalent to `org.apache.lucene.queryparser.surround.query.TooManyBasicQueries`,
/// a subclass of `java.io.IOException` (so that it is usable from within
/// `Query.rewrite()`); it is thrown as a [`LuceneError`] of kind
/// [`LuceneErrorKind::Throwable`].
#[derive(Debug, Clone)]
pub struct TooManyBasicQueries {
    message: String,
}

impl TooManyBasicQueries {
    /// Equivalent to `new TooManyBasicQueries(int)`.
    pub fn new(max_basic_queries: i32) -> Self {
        Self {
            message: format!("Exceeded maximum of {max_basic_queries} basic queries."),
        }
    }

    /// This exception as the [`LuceneError`] it is thrown as.
    pub fn error(self) -> LuceneError {
        LuceneError::from(LuceneErrorKind::Throwable(Arc::new(self)))
    }
}

impl JavaThrowable for TooManyBasicQueries {
    fn java_class(&self) -> &'static str {
        "org.apache.lucene.queryparser.surround.query.TooManyBasicQueries"
    }

    fn get_message(&self) -> Option<&str> {
        Some(&self.message)
    }

    fn is_io_exception(&self) -> bool {
        true
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
