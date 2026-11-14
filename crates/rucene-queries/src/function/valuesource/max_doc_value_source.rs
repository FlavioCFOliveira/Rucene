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

//! Port of `MaxDocValueSource.java`, of the Java package
//! `org.apache.lucene.queries.function.valuesource`.

use std::sync::Arc;

use rucene::error::Result;
use rucene::index::LeafReaderContext;
use rucene::search::index_searcher::IndexSearcher;

use crate::function::function_values::FunctionValues;
use crate::function::value_source::{class_hash, ValueSource, ValueSourceContext};
use crate::function::valuesource::doc_freq_value_source::{ConstIntDocValues, DocFreqValueSource};

/// Returns the value of `IndexReader.maxDoc()` for every document. This is
/// the number of documents including deletions.
///
/// Equivalent to `org.apache.lucene.queries.function.valuesource.MaxDocValueSource`.
#[derive(Debug, Clone, Copy, Default)]
pub struct MaxDocValueSource;

impl MaxDocValueSource {
    /// Equivalent to the implicit `MaxDocValueSource()`.
    pub fn new() -> Self {
        Self
    }

    /// Equivalent to `MaxDocValueSource.name()`.
    pub fn name(&self) -> String {
        "maxdoc".to_string()
    }
}

impl ValueSource for MaxDocValueSource {
    fn description(&self) -> String {
        format!("{}()", self.name())
    }

    /// `context.put("searcher", searcher)`.
    fn create_weight(
        &self,
        context: &ValueSourceContext<'_>,
        searcher: &IndexSearcher,
    ) -> Result<()> {
        context.put_searcher(searcher);
        Ok(())
    }

    fn get_values<'a>(
        self: Arc<Self>,
        context: &ValueSourceContext<'a>,
        _reader_context: &LeafReaderContext,
    ) -> Result<Box<dyn FunctionValues + 'a>> {
        let searcher = DocFreqValueSource::searcher(context)?;
        Ok(Box::new(ConstIntDocValues::new(
            searcher.get_index_reader().max_doc(),
            self,
        )))
    }

    /// `this.getClass() == o.getClass()`.
    fn value_source_eq(&self, other: &dyn ValueSource) -> bool {
        other.as_any().is::<MaxDocValueSource>()
    }

    /// `this.getClass().hashCode()`.
    fn value_source_hash(&self) -> i32 {
        class_hash("org.apache.lucene.queries.function.valuesource.MaxDocValueSource")
    }
}
