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

//! Port of `BooleanQueryBuilder.java`, of the Java package
//! `org.apache.lucene.queryparser.xml.builders`.

use std::sync::Arc;

use rucene::error::Result;
use rucene::search::boolean_clause::{BooleanClause, Occur};
use rucene::search::boolean_query::Builder;
use rucene::search::boost_query::BoostQuery;
use rucene::search::query::Query;

use crate::jdk::dom::Element;
use crate::jdk::string::equals_ignore_case;
use crate::xml::dom_utils::DOMUtils;
use crate::xml::parser_exception::ParserException;
use crate::xml::query_builder::QueryBuilder;

/// Builder for `BooleanQuery`
///
/// Equivalent to `org.apache.lucene.queryparser.xml.builders.BooleanQueryBuilder`.
pub struct BooleanQueryBuilder {
    factory: Arc<dyn QueryBuilder>,
}

impl std::fmt::Debug for BooleanQueryBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("BooleanQueryBuilder")
    }
}

impl BooleanQueryBuilder {
    /// Equivalent to `new BooleanQueryBuilder(QueryBuilder factory)`.
    pub fn new(factory: Arc<dyn QueryBuilder>) -> Self {
        Self { factory }
    }

    /// Equivalent to the package-private static `getOccursValue(Element
    /// clauseElem)`: the `occurs` attribute (`should`, `must` and `mustNot`
    /// in any case, `filter` exactly).
    ///
    /// # Errors
    ///
    /// `ParserException("Invalid value for \"occurs\" attribute of clause:"
    /// + occs)` for any other value, an absent attribute (`""`) included.
    pub(crate) fn get_occurs_value(clause_elem: &Element) -> Result<Occur> {
        let occs = clause_elem.get_attribute("occurs");
        if equals_ignore_case("should", &occs) {
            Ok(Occur::SHOULD)
        } else if equals_ignore_case("must", &occs) {
            Ok(Occur::MUST)
        } else if equals_ignore_case("mustNot", &occs) {
            Ok(Occur::MUST_NOT)
        } else if "filter" == occs {
            Ok(Occur::FILTER)
        } else {
            Err(ParserException::with_message(format!(
                "Invalid value for \"occurs\" attribute of clause:{occs}"
            ))
            .error())
        }
    }
}

impl QueryBuilder for BooleanQueryBuilder {
    /// Equivalent to `getQuery(Element e)`: a `BooleanQuery` of the `Clause`
    /// children, each the first child element of the clause with its
    /// `occurs`; `minimumNumberShouldMatch` and `boost` from the attributes.
    fn get_query(&self, e: &Element) -> Result<Arc<dyn Query>> {
        let mut bq = Builder::new();
        bq.set_minimum_number_should_match(DOMUtils::get_attribute_int(
            e,
            "minimumNumberShouldMatch",
            0,
        )?);
        let nl = e.get_child_nodes();
        let nl_len = nl.get_length();
        for i in 0..nl_len {
            let Some(node) = nl.item(i) else {
                continue;
            };
            if node.get_node_name() == "Clause" {
                let clause_elem = Element::cast(&node)?;
                let occurs = Self::get_occurs_value(&clause_elem)?;
                let clause_query = DOMUtils::get_first_child_or_fail(&clause_elem)?;
                let q = self.factory.get_query(&clause_query)?;
                bq.add_clause(BooleanClause::new(q, occurs))?;
            }
        }
        let mut q: Arc<dyn Query> = Arc::new(bq.build());
        let boost = DOMUtils::get_attribute_float(e, "boost", 1.0)?;
        if boost != 1.0 {
            q = Arc::new(BoostQuery::new(q, boost)?);
        }
        Ok(q)
    }
}
