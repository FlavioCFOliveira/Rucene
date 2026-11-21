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

//! Port of `NumDocsValueSource.java`, of the Java package
//! `org.apache.lucene.queries.function.valuesource`.

use std::sync::Arc;

use rucene::error::Result;
use rucene::index::LeafReaderContext;

use crate::function::function_values::FunctionValues;
use crate::function::value_source::{
    class_hash, top_level_context, ValueSource, ValueSourceContext,
};
use crate::function::valuesource::doc_freq_value_source::ConstIntDocValues;

/// Returns the value of `IndexReader.numDocs()` for every document. This is
/// the number of documents excluding deletions.
///
/// Equivalent to `org.apache.lucene.queries.function.valuesource.NumDocsValueSource`.
#[derive(Debug, Clone, Copy, Default)]
pub struct NumDocsValueSource;

impl NumDocsValueSource {
    /// Equivalent to the implicit `NumDocsValueSource()`.
    pub fn new() -> Self {
        Self
    }

    /// Equivalent to `NumDocsValueSource.name()`.
    pub fn name(&self) -> String {
        "numdocs".to_string()
    }
}

impl ValueSource for NumDocsValueSource {
    fn description(&self) -> String {
        format!("{}()", self.name())
    }

    fn get_values<'a>(
        self: Arc<Self>,
        _context: &ValueSourceContext<'a>,
        reader_context: &LeafReaderContext,
    ) -> Result<Box<dyn FunctionValues + 'a>> {
        // Searcher has no numdocs so we must use the reader instead
        let num_docs = top_level_context(reader_context).reader().num_docs();
        Ok(Box::new(ConstIntDocValues::new(num_docs, self)))
    }

    /// `this.getClass() == o.getClass()`.
    fn value_source_eq(&self, other: &dyn ValueSource) -> bool {
        other.as_any().is::<NumDocsValueSource>()
    }

    /// `this.getClass().hashCode()`.
    fn value_source_hash(&self) -> i32 {
        class_hash("org.apache.lucene.queries.function.valuesource.NumDocsValueSource")
    }
}
