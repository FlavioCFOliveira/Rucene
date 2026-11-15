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

//! Port of `StandardQueryTreeBuilder.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.standard.builders`.

use std::ops::Deref;
use std::sync::Arc;

use rucene::error::Result;
use rucene::search::Query;

use super::boolean_query_node_builder::BooleanQueryNodeBuilder;
use super::boost_query_node_builder::BoostQueryNodeBuilder;
use super::dummy_query_node_builder::DummyQueryNodeBuilder;
use super::field_query_node_builder::FieldQueryNodeBuilder;
use super::fuzzy_query_node_builder::FuzzyQueryNodeBuilder;
use super::group_query_node_builder::GroupQueryNodeBuilder;
use super::interval_query_node_builder::IntervalQueryNodeBuilder;
use super::match_all_docs_query_node_builder::MatchAllDocsQueryNodeBuilder;
use super::match_no_docs_query_node_builder::MatchNoDocsQueryNodeBuilder;
use super::min_should_match_node_builder::MinShouldMatchNodeBuilder;
use super::modifier_query_node_builder::ModifierQueryNodeBuilder;
use super::multi_phrase_query_node_builder::MultiPhraseQueryNodeBuilder;
use super::phrase_query_node_builder::PhraseQueryNodeBuilder;
use super::point_range_query_node_builder::PointRangeQueryNodeBuilder;
use super::prefix_wildcard_query_node_builder::PrefixWildcardQueryNodeBuilder;
use super::regexp_query_node_builder::RegexpQueryNodeBuilder;
use super::slop_query_node_builder::SlopQueryNodeBuilder;
use super::standard_query_builder::{query_of, StandardQueryBuilder};
use super::synonym_query_node_builder::SynonymQueryNodeBuilder;
use super::term_range_query_node_builder::TermRangeQueryNodeBuilder;
use super::wildcard_query_node_builder::WildcardQueryNodeBuilder;
use crate::flexible::core::builders::query_builder::QueryBuilder;
use crate::flexible::core::builders::query_tree_builder::QueryTreeBuilder;
use crate::flexible::core::nodes::boolean_query_node::BOOLEAN_QUERY_NODE_CLASS;
use crate::flexible::core::nodes::boost_query_node::BOOST_QUERY_NODE_CLASS;
use crate::flexible::core::nodes::field_query_node::FIELD_QUERY_NODE_CLASS;
use crate::flexible::core::nodes::fuzzy_query_node::FUZZY_QUERY_NODE_CLASS;
use crate::flexible::core::nodes::group_query_node::GROUP_QUERY_NODE_CLASS;
use crate::flexible::core::nodes::match_all_docs_query_node::MATCH_ALL_DOCS_QUERY_NODE_CLASS;
use crate::flexible::core::nodes::match_no_docs_query_node::MATCH_NO_DOCS_QUERY_NODE_CLASS;
use crate::flexible::core::nodes::modifier_query_node::MODIFIER_QUERY_NODE_CLASS;
use crate::flexible::core::nodes::query_node::QueryNodeRef;
use crate::flexible::core::nodes::slop_query_node::SLOP_QUERY_NODE_CLASS;
use crate::flexible::core::nodes::tokenized_phrase_query_node::TOKENIZED_PHRASE_QUERY_NODE_CLASS;
use crate::flexible::standard::nodes::interval_query_node::INTERVAL_QUERY_NODE_CLASS;
use crate::flexible::standard::nodes::min_should_match_node::MIN_SHOULD_MATCH_NODE_CLASS;
use crate::flexible::standard::nodes::multi_phrase_query_node::MULTI_PHRASE_QUERY_NODE_CLASS;
use crate::flexible::standard::nodes::point_query_node::POINT_QUERY_NODE_CLASS;
use crate::flexible::standard::nodes::point_range_query_node::POINT_RANGE_QUERY_NODE_CLASS;
use crate::flexible::standard::nodes::prefix_wildcard_query_node::PREFIX_WILDCARD_QUERY_NODE_CLASS;
use crate::flexible::standard::nodes::regexp_query_node::REGEXP_QUERY_NODE_CLASS;
use crate::flexible::standard::nodes::synonym_query_node::SYNONYM_QUERY_NODE_CLASS;
use crate::flexible::standard::nodes::term_range_query_node::TERM_RANGE_QUERY_NODE_CLASS;
use crate::flexible::standard::nodes::wildcard_query_node::WILDCARD_QUERY_NODE_CLASS;

/// This query tree builder only defines the necessary map to build a
/// [`Query`] tree object. It should be used to generate a [`Query`] tree
/// object from a query node tree processed by a
/// `StandardQueryNodeProcessorPipeline`.
///
/// Equivalent to `org.apache.lucene.queryparser.flexible.standard.builders.StandardQueryTreeBuilder`,
/// which extends `QueryTreeBuilder` (embedded, and reached by
/// dereference) and implements `StandardQueryBuilder`.
#[derive(Debug)]
pub struct StandardQueryTreeBuilder {
    base: QueryTreeBuilder,
}

impl StandardQueryTreeBuilder {
    /// Equivalent to `new StandardQueryTreeBuilder()`: a builder for each
    /// node class of the standard parser.
    pub fn new() -> Self {
        let base = QueryTreeBuilder::new();
        base.set_builder_for_class(
            &GROUP_QUERY_NODE_CLASS,
            Arc::new(GroupQueryNodeBuilder::new()),
        );
        base.set_builder_for_class(
            &FIELD_QUERY_NODE_CLASS,
            Arc::new(FieldQueryNodeBuilder::new()),
        );
        base.set_builder_for_class(
            &BOOLEAN_QUERY_NODE_CLASS,
            Arc::new(BooleanQueryNodeBuilder::new()),
        );
        base.set_builder_for_class(
            &FUZZY_QUERY_NODE_CLASS,
            Arc::new(FuzzyQueryNodeBuilder::new()),
        );
        base.set_builder_for_class(
            &POINT_QUERY_NODE_CLASS,
            Arc::new(DummyQueryNodeBuilder::new()),
        );
        base.set_builder_for_class(
            &POINT_RANGE_QUERY_NODE_CLASS,
            Arc::new(PointRangeQueryNodeBuilder::new()),
        );
        base.set_builder_for_class(
            &BOOST_QUERY_NODE_CLASS,
            Arc::new(BoostQueryNodeBuilder::new()),
        );
        base.set_builder_for_class(
            &MODIFIER_QUERY_NODE_CLASS,
            Arc::new(ModifierQueryNodeBuilder::new()),
        );
        base.set_builder_for_class(
            &WILDCARD_QUERY_NODE_CLASS,
            Arc::new(WildcardQueryNodeBuilder::new()),
        );
        base.set_builder_for_class(
            &TOKENIZED_PHRASE_QUERY_NODE_CLASS,
            Arc::new(PhraseQueryNodeBuilder::new()),
        );
        base.set_builder_for_class(
            &MATCH_NO_DOCS_QUERY_NODE_CLASS,
            Arc::new(MatchNoDocsQueryNodeBuilder::new()),
        );
        base.set_builder_for_class(
            &PREFIX_WILDCARD_QUERY_NODE_CLASS,
            Arc::new(PrefixWildcardQueryNodeBuilder::new()),
        );
        base.set_builder_for_class(
            &TERM_RANGE_QUERY_NODE_CLASS,
            Arc::new(TermRangeQueryNodeBuilder::new()),
        );
        base.set_builder_for_class(
            &REGEXP_QUERY_NODE_CLASS,
            Arc::new(RegexpQueryNodeBuilder::new()),
        );
        base.set_builder_for_class(
            &SLOP_QUERY_NODE_CLASS,
            Arc::new(SlopQueryNodeBuilder::new()),
        );
        base.set_builder_for_class(
            &SYNONYM_QUERY_NODE_CLASS,
            Arc::new(SynonymQueryNodeBuilder::new()),
        );
        base.set_builder_for_class(
            &MULTI_PHRASE_QUERY_NODE_CLASS,
            Arc::new(MultiPhraseQueryNodeBuilder::new()),
        );
        base.set_builder_for_class(
            &MATCH_ALL_DOCS_QUERY_NODE_CLASS,
            Arc::new(MatchAllDocsQueryNodeBuilder::new()),
        );
        base.set_builder_for_class(
            &MIN_SHOULD_MATCH_NODE_CLASS,
            Arc::new(MinShouldMatchNodeBuilder::new()),
        );
        base.set_builder_for_class(
            &INTERVAL_QUERY_NODE_CLASS,
            Arc::new(IntervalQueryNodeBuilder::new()),
        );
        Self { base }
    }

    /// The `QueryTreeBuilder` this builder extends.
    pub fn query_tree_builder(&self) -> &QueryTreeBuilder {
        &self.base
    }
}

impl Default for StandardQueryTreeBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl Deref for StandardQueryTreeBuilder {
    type Target = QueryTreeBuilder;

    fn deref(&self) -> &QueryTreeBuilder {
        &self.base
    }
}

impl StandardQueryBuilder for StandardQueryTreeBuilder {
    /// Equivalent to `build(QueryNode queryNode)`: `(Query)
    /// super.build(queryNode)`.
    fn build(&self, query_node: &QueryNodeRef) -> Result<Option<Arc<dyn Query>>> {
        QueryBuilder::build(&self.base, query_node)?
            .map(|obj| query_of(&obj))
            .transpose()
    }

    fn as_query_tree_builder(&self) -> Option<&QueryTreeBuilder> {
        Some(&self.base)
    }
}
