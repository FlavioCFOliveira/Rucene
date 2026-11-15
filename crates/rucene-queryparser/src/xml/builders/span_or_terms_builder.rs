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

//! Port of `SpanOrTermsBuilder.java`, of the Java package
//! `org.apache.lucene.queryparser.xml.builders`.

use std::sync::Arc;

use rucene::analysis::tokenattributes::TermToBytesRefAttribute;
use rucene::analysis::Analyzer;
use rucene::error::Result;
use rucene::index::Term;
use rucene::search::query::Query;
use rucene_queries::spans::span_or_query::SpanOrQuery;
use rucene_queries::spans::span_query::SpanQuery;
use rucene_queries::spans::span_term_query::SpanTermQuery;

use super::close_after;
use crate::xml::parser_exception::ParserException;

use super::span_builder_base::SpanBuilderBase;
use super::span_query_builder::SpanQueryBuilder;
use crate::jdk::dom::Element;
use crate::xml::dom_utils::DOMUtils;
use crate::xml::query_builder::QueryBuilder;

/// Builder that analyzes the text into a `SpanOrQuery`
///
/// Equivalent to `org.apache.lucene.queryparser.xml.builders.SpanOrTermsBuilder`, which extends `SpanBuilderBase`.
pub struct SpanOrTermsBuilder {
    analyzer: Option<Arc<dyn Analyzer>>,
}

impl std::fmt::Debug for SpanOrTermsBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SpanOrTermsBuilder")
    }
}

impl SpanOrTermsBuilder {
    /// Equivalent to `new SpanOrTermsBuilder(Analyzer analyzer)`.
    pub fn new(analyzer: Option<Arc<dyn Analyzer>>) -> Self {
        Self { analyzer }
    }
}

impl QueryBuilder for SpanOrTermsBuilder {
    fn get_query(&self, e: &Element) -> Result<Arc<dyn Query>> {
        SpanBuilderBase::get_query(self, e)
    }
}

impl SpanQueryBuilder for SpanOrTermsBuilder {
    /// Equivalent to `getSpanQuery(Element e)`: a `SpanOrQuery` of a
    /// `SpanTermQuery` per token the analyzer produces from the element's
    /// text, in the `fieldName` attribute (inherited).
    ///
    /// # Errors
    ///
    /// `ParserException("IOException parsing value:" + value, ioe)` for an
    /// `IOException` of the analysis.
    fn get_span_query(&self, e: &Element) -> Result<Arc<dyn SpanQuery>> {
        let field_name = DOMUtils::get_attribute_with_inheritance_or_fail(e, "fieldName")?;
        let value = DOMUtils::get_non_blank_text_or_fail(e)?;
        let result = (|| -> Result<Arc<dyn SpanQuery>> {
            let ts = self
                .analyzer
                .as_ref()
                .ok_or_else(rucene::error::LuceneError::null_pointer)?
                .token_stream_from_str(&field_name, &value)?;
            let mut ts = ts.borrow_mut();
            let body = (|| -> Result<Arc<dyn SpanQuery>> {
                ts.attribute_source_mut()
                    .add_attribute_interface::<dyn TermToBytesRefAttribute>()?;
                let mut clauses_list: Vec<Arc<dyn SpanQuery>> = Vec::new();
                ts.reset()?;
                while ts.increment_token()? {
                    let bytes = ts
                        .attribute_source()
                        .get_attribute_interface::<dyn TermToBytesRefAttribute>()
                        .expect("INVARIANT: TermToBytesRefAttribute was added above")
                        .get_bytes_ref();
                    let stq = SpanTermQuery::new(Term::new(field_name.clone(), bytes));
                    clauses_list.push(Arc::new(stq));
                }
                ts.end()?;
                Ok(Arc::new(SpanOrQuery::new(clauses_list)?))
            })();
            let closed = ts.close();
            close_after(body, closed)
        })();
        result.map_err(|ioe| {
            if ioe.is_io_exception() {
                ParserException::with_message_and_cause(
                    format!("IOException parsing value:{value}"),
                    ioe,
                )
                .error()
            } else {
                ioe
            }
        })
    }
}
