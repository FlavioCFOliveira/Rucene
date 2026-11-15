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

//! Port of `PointQueryNodeProcessor.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.standard.processors`.

use std::sync::Arc;

use rucene::error::{LuceneError, Result};
use rucene::util::extra::ParseException;

use crate::flexible::core::messages::query_parser_messages::QueryParserMessages;
use crate::flexible::core::nodes::query_node::{instance_of, QueryNodeRef};
use crate::flexible::core::nodes::range_query_node::RANGE_QUERY_NODE_CLASS;
use crate::flexible::core::processors::query_node_processor_impl::{
    QueryNodeProcessorImpl, QueryNodeProcessorImplOps,
};
use crate::flexible::core::query_node_parse_exception::QueryNodeParseException;
use crate::flexible::messages::message_impl::MessageImpl;
use crate::flexible::standard::config::standard_query_config_handler::ConfigurationKeys;
use crate::flexible::standard::nodes::point_query_node::PointQueryNode;
use crate::flexible::standard::nodes::point_range_query_node::PointRangeQueryNode;
use crate::jdk::class::{DOUBLE, FLOAT, INTEGER, LONG};
use crate::jdk::format::JavaNumber;
use crate::jdk::object::JavaObject;

/// This processor is used to convert `FieldQueryNode`s to
/// `PointRangeQueryNode`s. It looks for `ConfigurationKeys.POINTS_CONFIG`
/// set in the `FieldConfig` of every `FieldQueryNode` found. If
/// `ConfigurationKeys.POINTS_CONFIG` is found, it considers that
/// `FieldQueryNode` to be a numeric query and convert it to
/// `PointRangeQueryNode` with upper and lower inclusive and lower and upper
/// equals to the value represented by the `FieldQueryNode` converted to
/// `Number`. It means that **field:1** is converted to **field:[1 TO 1]**.
///
/// Note that `FieldQueryNode`s children of a `RangeQueryNode` are ignored.
///
/// Equivalent to `org.apache.lucene.queryparser.flexible.standard.processors.PointQueryNodeProcessor`.
#[derive(Debug, Default)]
pub struct PointQueryNodeProcessor {
    base: QueryNodeProcessorImpl,
}

impl PointQueryNodeProcessor {
    /// Constructs a [`PointQueryNodeProcessor`] object.
    ///
    /// Equivalent to `new PointQueryNodeProcessor()`.
    pub fn new() -> Self {
        // empty constructor
        Self::default()
    }
}

impl QueryNodeProcessorImplOps for PointQueryNodeProcessor {
    fn query_node_processor_impl(&self) -> &QueryNodeProcessorImpl {
        &self.base
    }

    fn post_process_node(&self, node: QueryNodeRef) -> Result<QueryNodeRef> {
        let Some(field_node) = node.as_field_query_node() else {
            return Ok(node);
        };
        let parent_is_range = node
            .get_parent()
            .is_some_and(|p| instance_of(p.as_ref(), &RANGE_QUERY_NODE_CLASS));
        if parent_is_range {
            return Ok(node);
        }
        let Some(config) = self.get_query_config_handler() else {
            return Ok(node);
        };
        let field_config = config.get_field_config(field_node.get_field_as_string().as_deref())?;
        // `if (fieldConfig != null)`, then `if (numericConfig != null)`
        let Some(numeric_config) =
            field_config.and_then(|c| c.get(ConfigurationKeys::POINTS_CONFIG))
        else {
            return Ok(node);
        };
        let number_format = numeric_config.get_number_format();
        let text = field_node.get_text_as_string();
        let mut number: JavaNumber;
        if !text
            .as_deref()
            .ok_or_else(LuceneError::null_pointer)?
            .is_empty()
        {
            let text = text.as_deref().unwrap_or_default();
            number = match number_format.parse(text) {
                Ok(n) => n,
                Err(e) if ParseException::of(&e).is_some() => {
                    return Err(QueryNodeParseException::with_message_and_cause(
                        Arc::new(MessageImpl::with_args(
                            QueryParserMessages::COULD_NOT_PARSE_NUMBER.get(),
                            vec![
                                JavaObject::from(field_node.get_text_as_string()),
                                JavaObject::from(number_format.java_class().get_canonical_name()),
                            ],
                        )),
                        e,
                    )
                    .error());
                }
                Err(e) => return Err(e),
            };
            let type_ = numeric_config.get_type();
            if *type_ == INTEGER {
                number = JavaNumber::Integer(number.int_value());
            } else if *type_ == LONG {
                number = JavaNumber::Long(number.long_value());
            } else if *type_ == DOUBLE {
                number = JavaNumber::Double(number.double_value());
            } else if *type_ == FLOAT {
                number = JavaNumber::Float(number.float_value());
            }
        } else {
            return Err(
                QueryNodeParseException::new(Arc::new(MessageImpl::with_args(
                    QueryParserMessages::NUMERIC_CANNOT_BE_EMPTY.get(),
                    vec![JavaObject::from(field_node.get_field_as_string())],
                )))
                .error(),
            );
        }
        let lower_node = PointQueryNode::new(
            field_node.get_field(),
            Some(number),
            Some(number_format.clone()),
        );
        let upper_node =
            PointQueryNode::new(field_node.get_field(), Some(number), Some(number_format));
        let range: QueryNodeRef = PointRangeQueryNode::new(
            Some(lower_node),
            Some(upper_node),
            true,
            true,
            Some(numeric_config),
        )?;
        Ok(range)
    }

    fn pre_process_node(&self, node: QueryNodeRef) -> Result<QueryNodeRef> {
        Ok(node)
    }

    fn set_children_order(&self, children: Vec<QueryNodeRef>) -> Result<Vec<QueryNodeRef>> {
        Ok(children)
    }
}
