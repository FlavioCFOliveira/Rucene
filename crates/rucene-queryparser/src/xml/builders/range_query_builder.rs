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

//! Port of `RangeQueryBuilder.java`, of the Java package
//! `org.apache.lucene.queryparser.xml.builders`.

use std::sync::Arc;

use rucene::error::{LuceneError, Result};
use rucene::search::query::Query;
use rucene::search::term_range_query::TermRangeQuery;

use crate::jdk::dom::Element;
use crate::xml::dom_utils::DOMUtils;
use crate::xml::query_builder::QueryBuilder;

/// Builder for `TermRangeQuery`
///
/// Equivalent to `org.apache.lucene.queryparser.xml.builders.RangeQueryBuilder`.
#[derive(Debug, Default)]
pub struct RangeQueryBuilder;

impl RangeQueryBuilder {
    /// Equivalent to `new RangeQueryBuilder()`.
    pub fn new() -> Self {
        Self
    }
}

impl QueryBuilder for RangeQueryBuilder {
    /// Equivalent to `getQuery(Element e)`: `TermRangeQuery.newStringRange`
    /// of the `fieldName` attribute (inherited), the `lowerTerm` and
    /// `upperTerm` attributes (`""` when absent) and the `includeLower` and
    /// `includeUpper` ones (true by default).
    ///
    /// # Errors
    ///
    /// `NullPointerException("field must not be null")` (`MultiTermQuery`'s
    /// check) without a field name.
    fn get_query(&self, e: &Element) -> Result<Arc<dyn Query>> {
        let field_name = DOMUtils::get_attribute_with_inheritance(e, "fieldName");
        let lower_term = e.get_attribute("lowerTerm");
        let upper_term = e.get_attribute("upperTerm");
        let include_lower = DOMUtils::get_attribute_boolean(e, "includeLower", true);
        let include_upper = DOMUtils::get_attribute_boolean(e, "includeUpper", true);
        let field_name = field_name
            .ok_or_else(|| LuceneError::null_pointer_with_message("field must not be null"))?;
        Ok(Arc::new(TermRangeQuery::new_string_range(
            field_name,
            Some(&lower_term),
            Some(&upper_term),
            include_lower,
            include_upper,
        )?))
    }
}
