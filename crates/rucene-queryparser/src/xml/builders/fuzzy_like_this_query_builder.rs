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

//! Port of `FuzzyLikeThisQueryBuilder.java`, of the Java package
//! `org.apache.lucene.queryparser.xml.builders`.

use std::sync::Arc;

use rucene::analysis::Analyzer;
use rucene::error::Result;
use rucene::search::boost_query::BoostQuery;
use rucene::search::fuzzy_query::DEFAULT_MAX_EDITS;
use rucene::search::query::Query;
use rucene_sandbox::queries::fuzzy_like_this_query::FuzzyLikeThisQuery;

use crate::jdk::dom::Element;
use crate::xml::dom_utils::DOMUtils;
use crate::xml::query_builder::QueryBuilder;

/// `DEFAULT_MAX_NUM_TERMS`.
const DEFAULT_MAX_NUM_TERMS: i32 = 50;
/// `DEFAULT_MIN_SIMILARITY`: `FuzzyQuery.defaultMaxEdits`.
const DEFAULT_MIN_SIMILARITY: f32 = DEFAULT_MAX_EDITS as f32;
/// `DEFAULT_PREFIX_LENGTH`.
const DEFAULT_PREFIX_LENGTH: i32 = 1;
/// `DEFAULT_IGNORE_TF`.
const DEFAULT_IGNORE_TF: bool = false;

/// Builder for `FuzzyLikeThisQuery`
///
/// Equivalent to
/// `org.apache.lucene.queryparser.xml.builders.FuzzyLikeThisQueryBuilder`.
///
/// A `Field` without a `fieldName` (its own or an ancestor's) and a `None`
/// analyzer reach `FuzzyLikeThisQuery` as Java's `null`s
/// (`FuzzyLikeThisQueryBuilder.java:48,57,60`); the query answers them as
/// Java does when it rewrites.
#[derive(Debug)]
pub struct FuzzyLikeThisQueryBuilder {
    analyzer: Option<Arc<dyn Analyzer>>,
}

impl FuzzyLikeThisQueryBuilder {
    /// Equivalent to `new FuzzyLikeThisQueryBuilder(Analyzer analyzer)`.
    pub fn new(analyzer: Option<Arc<dyn Analyzer>>) -> Self {
        Self { analyzer }
    }
}

impl QueryBuilder for FuzzyLikeThisQueryBuilder {
    /// Equivalent to `getQuery(Element e)`: a `FuzzyLikeThisQuery` of the
    /// `Field` descendants, with the `maxNumTerms`, `ignoreTF` and `boost`
    /// attributes and each field's `minSimilarity` and `prefixLength`.
    fn get_query(&self, e: &Element) -> Result<Arc<dyn Query>> {
        let nl = e.get_elements_by_tag_name("Field");
        let max_num_terms = DOMUtils::get_attribute_int(e, "maxNumTerms", DEFAULT_MAX_NUM_TERMS)?;
        let mut fbq = FuzzyLikeThisQuery::new(max_num_terms, self.analyzer.clone());
        fbq.set_ignore_tf(DOMUtils::get_attribute_boolean(
            e,
            "ignoreTF",
            DEFAULT_IGNORE_TF,
        ));
        let nl_len = nl.get_length();
        for i in 0..nl_len {
            let Some(node) = nl.item(i) else {
                continue;
            };
            let field_elem = Element::cast(&node)?;
            let min_similarity = DOMUtils::get_attribute_float(
                &field_elem,
                "minSimilarity",
                DEFAULT_MIN_SIMILARITY,
            )?;
            let prefix_length =
                DOMUtils::get_attribute_int(&field_elem, "prefixLength", DEFAULT_PREFIX_LENGTH)?;
            let field_name = DOMUtils::get_attribute_with_inheritance(&field_elem, "fieldName");
            let value = DOMUtils::get_text(&field_elem);
            fbq.add_terms(Some(value), field_name, min_similarity, prefix_length)?;
        }
        let fbq = Arc::new(fbq);
        let mut q: Arc<dyn Query> = fbq.clone();
        let boost = DOMUtils::get_attribute_float(e, "boost", 1.0)?;
        if boost != 1.0 {
            q = Arc::new(BoostQuery::new(fbq, boost)?);
        }
        Ok(q)
    }
}
