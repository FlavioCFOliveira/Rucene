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

//! Port of `PointRangeQueryNodeProcessor.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.standard.processors`.

use std::sync::Arc;

use rucene::error::{LuceneError, Result};
use rucene::util::extra::ParseException;

use crate::flexible::core::messages::query_parser_messages::QueryParserMessages;
use crate::flexible::core::nodes::field_query_node::{FieldQueryNode, FIELD_QUERY_NODE_CLASS};
use crate::flexible::core::nodes::query_node::{class_cast, QueryNodeRef};
use crate::flexible::core::processors::query_node_processor_impl::{
    QueryNodeProcessorImpl, QueryNodeProcessorImplOps,
};
use crate::flexible::core::query_node_parse_exception::QueryNodeParseException;
use crate::flexible::core::util::string_utils::StringUtils;
use crate::flexible::messages::message_impl::MessageImpl;
use crate::flexible::standard::config::standard_query_config_handler::ConfigurationKeys;
use crate::flexible::standard::nodes::point_query_node::PointQueryNode;
use crate::flexible::standard::nodes::point_range_query_node::PointRangeQueryNode;
use crate::flexible::standard::nodes::term_range_query_node::TermRangeQueryNode;
use crate::jdk::class::{DOUBLE, FLOAT, INTEGER, LONG};
use crate::jdk::format::JavaNumber;
use crate::jdk::number_format::NumberFormat;
use crate::jdk::object::JavaObject;

/// This processor is used to convert `TermRangeQueryNode`s to
/// `PointRangeQueryNode`s. It looks for `ConfigurationKeys.POINTS_CONFIG`
/// set in the `FieldConfig` of every `TermRangeQueryNode` found. If
/// `ConfigurationKeys.POINTS_CONFIG` is found, it considers that
/// `TermRangeQueryNode` to be a numeric range query and convert it to
/// `PointRangeQueryNode`.
///
/// Equivalent to `org.apache.lucene.queryparser.flexible.standard.processors.PointRangeQueryNodeProcessor`.
#[derive(Debug, Default)]
pub struct PointRangeQueryNodeProcessor {
    base: QueryNodeProcessorImpl,
}

/// `numberFormat.parse(text)` of a bound with a text, the `ParseException`
/// rethrown as the `QueryNodeParseException` `COULD_NOT_PARSE_NUMBER`.
fn parse_bound(
    bound: &FieldQueryNode,
    text: &str,
    number_format: &Arc<dyn NumberFormat>,
) -> Result<JavaNumber> {
    match number_format.parse(text) {
        Ok(n) => Ok(n),
        Err(e) if ParseException::of(&e).is_some() => {
            Err(QueryNodeParseException::with_message_and_cause(
                Arc::new(MessageImpl::with_args(
                    QueryParserMessages::COULD_NOT_PARSE_NUMBER.get(),
                    vec![
                        JavaObject::from(bound.get_text_as_string()),
                        JavaObject::from(number_format.java_class().get_canonical_name()),
                    ],
                )),
                e,
            )
            .error())
        }
        Err(e) => Err(e),
    }
}

impl PointRangeQueryNodeProcessor {
    /// Constructs an empty [`PointRangeQueryNodeProcessor`] object.
    ///
    /// Equivalent to `new PointRangeQueryNodeProcessor()`.
    pub fn new() -> Self {
        // empty constructor
        Self::default()
    }
}

impl QueryNodeProcessorImplOps for PointRangeQueryNodeProcessor {
    fn query_node_processor_impl(&self) -> &QueryNodeProcessorImpl {
        &self.base
    }

    fn post_process_node(&self, node: QueryNodeRef) -> Result<QueryNodeRef> {
        let Some(term_range_node) = node.as_any().downcast_ref::<TermRangeQueryNode>() else {
            return Ok(node);
        };
        let Some(config) = self.get_query_config_handler() else {
            return Ok(node);
        };
        let range = term_range_node.abstract_range_query_node();
        let field_config = config
            .get_field_config(StringUtils::to_string(range.get_field()?.as_ref()).as_deref())?;
        let Some(numeric_config) = field_config.get(ConfigurationKeys::POINTS_CONFIG) else {
            return Ok(node);
        };
        let lower_bound = range.get_lower_bound()?;
        let lower = lower_bound
            .as_field_query_node()
            .ok_or_else(|| class_cast(lower_bound.as_ref(), &FIELD_QUERY_NODE_CLASS))?;
        let upper_bound = range.get_upper_bound()?;
        let upper = upper_bound
            .as_field_query_node()
            .ok_or_else(|| class_cast(upper_bound.as_ref(), &FIELD_QUERY_NODE_CLASS))?;

        let lower_text = lower.get_text_as_string();
        let upper_text = upper.get_text_as_string();
        let number_format = numeric_config.get_number_format();
        let mut lower_number: Option<JavaNumber> = None;
        let mut upper_number: Option<JavaNumber> = None;

        let lower_text = lower_text.ok_or_else(LuceneError::null_pointer)?;
        if !lower_text.is_empty() {
            lower_number = Some(parse_bound(lower, &lower_text, &number_format)?);
        }

        let upper_text = upper_text.ok_or_else(LuceneError::null_pointer)?;
        if !upper_text.is_empty() {
            upper_number = Some(parse_bound(upper, &upper_text, &number_format)?);
        }

        let type_ = numeric_config.get_type();
        if *type_ == INTEGER {
            upper_number = upper_number.map(|n| JavaNumber::Integer(n.int_value()));
            lower_number = lower_number.map(|n| JavaNumber::Integer(n.int_value()));
        } else if *type_ == LONG {
            upper_number = upper_number.map(|n| JavaNumber::Long(n.long_value()));
            lower_number = lower_number.map(|n| JavaNumber::Long(n.long_value()));
        } else if *type_ == DOUBLE {
            upper_number = upper_number.map(|n| JavaNumber::Double(n.double_value()));
            lower_number = lower_number.map(|n| JavaNumber::Double(n.double_value()));
        } else if *type_ == FLOAT {
            upper_number = upper_number.map(|n| JavaNumber::Float(n.float_value()));
            lower_number = lower_number.map(|n| JavaNumber::Float(n.float_value()));
        }

        let field = range.get_field()?;
        let lower_node =
            PointQueryNode::new(field.clone(), lower_number, Some(number_format.clone()));
        let upper_node = PointQueryNode::new(range.get_field()?, upper_number, Some(number_format));

        let lower_inclusive = range.is_lower_inclusive();
        let upper_inclusive = range.is_upper_inclusive();

        let point_range: QueryNodeRef = PointRangeQueryNode::new(
            Some(lower_node),
            Some(upper_node),
            lower_inclusive,
            upper_inclusive,
            Some(numeric_config),
        )?;
        Ok(point_range)
    }

    fn pre_process_node(&self, node: QueryNodeRef) -> Result<QueryNodeRef> {
        Ok(node)
    }

    fn set_children_order(&self, children: Vec<QueryNodeRef>) -> Result<Vec<QueryNodeRef>> {
        Ok(children)
    }
}
