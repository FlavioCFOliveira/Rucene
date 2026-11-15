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

//! Port of `TermsQueryBuilder.java`, of the Java package
//! `org.apache.lucene.queryparser.xml.builders`.

use std::sync::Arc;

use rucene::analysis::tokenattributes::TermToBytesRefAttribute;
use rucene::analysis::Analyzer;
use rucene::error::{LuceneError, Result};
use rucene::index::Term;
use rucene::search::boolean_clause::{BooleanClause, Occur};
use rucene::search::boolean_query::Builder;
use rucene::search::boost_query::BoostQuery;
use rucene::search::query::Query;
use rucene::search::term_query::TermQuery;

use super::close_after;
use crate::flexible::core::query_node_exception::throwable_to_string;
use crate::jdk::dom::Element;
use crate::xml::dom_utils::DOMUtils;
use crate::xml::query_builder::QueryBuilder;

/// Builds a BooleanQuery from all of the terms found in the XML element
/// using the choice of analyzer
///
/// Equivalent to `org.apache.lucene.queryparser.xml.builders.TermsQueryBuilder`.
#[derive(Debug)]
pub struct TermsQueryBuilder {
    analyzer: Option<Arc<dyn Analyzer>>,
}

impl TermsQueryBuilder {
    /// Equivalent to `new TermsQueryBuilder(Analyzer analyzer)`.
    pub fn new(analyzer: Option<Arc<dyn Analyzer>>) -> Self {
        Self { analyzer }
    }
}

impl QueryBuilder for TermsQueryBuilder {
    /// Equivalent to `getQuery(Element e)`: a `BooleanQuery` of a `SHOULD`
    /// `TermQuery` per token the analyzer produces from the element's text,
    /// with the `minimumNumberShouldMatch` attribute, always wrapped in a
    /// `BoostQuery` of the `boost` attribute.
    ///
    /// # Errors
    ///
    /// `RuntimeException("Error constructing terms from index:" + ioe)` for
    /// an `IOException` of the analysis.
    fn get_query(&self, e: &Element) -> Result<Arc<dyn Query>> {
        let field_name = DOMUtils::get_attribute_with_inheritance_or_fail(e, "fieldName")?;
        let text = DOMUtils::get_non_blank_text_or_fail(e)?;
        let mut bq = Builder::new();
        bq.set_minimum_number_should_match(DOMUtils::get_attribute_int(
            e,
            "minimumNumberShouldMatch",
            0,
        )?);
        let result = (|| -> Result<()> {
            let ts = self
                .analyzer
                .as_ref()
                .ok_or_else(rucene::error::LuceneError::null_pointer)?
                .token_stream_from_str(&field_name, &text)?;
            let mut ts = ts.borrow_mut();
            let body = (|| -> Result<()> {
                ts.attribute_source_mut()
                    .add_attribute_interface::<dyn TermToBytesRefAttribute>()?;
                ts.reset()?;
                while ts.increment_token()? {
                    let bytes = ts
                        .attribute_source()
                        .get_attribute_interface::<dyn TermToBytesRefAttribute>()
                        .expect("INVARIANT: TermToBytesRefAttribute was added above")
                        .get_bytes_ref();
                    let term = Term::new(field_name.clone(), bytes);
                    bq.add_clause(BooleanClause::new(
                        Arc::new(TermQuery::new(term)),
                        Occur::SHOULD,
                    ))?;
                }
                ts.end()?;
                Ok(())
            })();
            let closed = ts.close();
            close_after(body, closed)
        })();
        result.map_err(|ioe| {
            if ioe.is_io_exception() {
                LuceneError::unchecked(
                    "java.lang.RuntimeException",
                    format!(
                        "Error constructing terms from index:{}",
                        throwable_to_string(&ioe)
                    ),
                )
            } else {
                ioe
            }
        })?;
        let q: Arc<dyn Query> = Arc::new(bq.build());
        let boost = DOMUtils::get_attribute_float(e, "boost", 1.0)?;
        Ok(Arc::new(BoostQuery::new(q, boost)?))
    }
}
