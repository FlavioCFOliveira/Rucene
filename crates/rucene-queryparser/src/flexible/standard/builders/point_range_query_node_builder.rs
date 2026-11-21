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

//! Port of `PointRangeQueryNodeBuilder.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.standard.builders`.

use std::sync::Arc;

use rucene::document::{DoublePoint, FloatPoint, IntPoint, LongPoint};
use rucene::error::{LuceneError, Result};
use rucene::search::Query;

use super::standard_query_builder::{cast, StandardQueryBuilder};
use crate::flexible::core::messages::query_parser_messages::QueryParserMessages;
use crate::flexible::core::nodes::query_node::QueryNodeRef;
use crate::flexible::core::query_node_exception::QueryNodeException;
use crate::flexible::core::util::string_utils::StringUtils;
use crate::flexible::messages::message_impl::MessageImpl;
use crate::flexible::standard::nodes::point_query_node::{PointQueryNode, POINT_QUERY_NODE_CLASS};
use crate::flexible::standard::nodes::point_range_query_node::{
    PointRangeQueryNode, POINT_RANGE_QUERY_NODE_CLASS,
};
use crate::jdk::class::{DOUBLE, FLOAT, INTEGER, LONG};
use crate::jdk::format::JavaNumber;
use crate::jdk::object::JavaObject;

/// Builds `PointValues` range queries out of `PointRangeQueryNode`s.
///
/// Equivalent to `org.apache.lucene.queryparser.flexible.standard.builders.PointRangeQueryNodeBuilder`.
#[derive(Debug, Default)]
pub struct PointRangeQueryNodeBuilder;

/// The JDK class name of a boxed number.
fn number_class(n: &JavaNumber) -> &'static str {
    match n {
        JavaNumber::Integer(_) => "java.lang.Integer",
        JavaNumber::Long(_) => "java.lang.Long",
        JavaNumber::Float(_) => "java.lang.Float",
        JavaNumber::Double(_) => "java.lang.Double",
    }
}

/// The `ClassCastException` of the cast of a boxed number to the boxed class
/// `to`, as JDK 25 words it for two classes of `java.base`.
fn number_class_cast(n: &JavaNumber, to: &str) -> LuceneError {
    let from = number_class(n);
    LuceneError::unchecked(
        "java.lang.ClassCastException",
        format!(
            "class {from} cannot be cast to class {to} ({from} and {to} are in module java.base \
             of loader 'bootstrap')"
        ),
    )
}

/// `PointRangeQuery.checkArgs`: the field of a points query.
fn points_field(field: Option<String>) -> Result<String> {
    field.ok_or_else(|| LuceneError::illegal_argument("field must not be null"))
}

impl PointRangeQueryNodeBuilder {
    /// Constructs a [`PointRangeQueryNodeBuilder`] object.
    ///
    /// Equivalent to `new PointRangeQueryNodeBuilder()`.
    pub fn new() -> Self {
        // empty constructor
        Self
    }
}

impl StandardQueryBuilder for PointRangeQueryNodeBuilder {
    /// Equivalent to `build(QueryNode queryNode)`.
    ///
    /// # Errors
    ///
    /// The `QueryNodeException` `UNSUPPORTED_NUMERIC_DATA_TYPE` of a type
    /// other than `Integer`, `Long`, `Float` and `Double`, the
    /// `ClassCastException` of a bound of another class, and the
    /// `IllegalArgumentException` of a `null` field.
    fn build(&self, query_node: &QueryNodeRef) -> Result<Option<Arc<dyn Query>>> {
        let numeric_range_node: &PointRangeQueryNode =
            cast(query_node, &POINT_RANGE_QUERY_NODE_CLASS)?;
        let range = numeric_range_node.abstract_range_query_node();
        let lower_bound = range.get_lower_bound()?;
        let upper_bound = range.get_upper_bound()?;
        let lower_numeric_node: &PointQueryNode = cast(&lower_bound, &POINT_QUERY_NODE_CLASS)?;
        let upper_numeric_node: &PointQueryNode = cast(&upper_bound, &POINT_QUERY_NODE_CLASS)?;

        let lower_number = lower_numeric_node.get_value();
        let upper_number = upper_numeric_node.get_value();

        let points_config = numeric_range_node
            .get_points_config()
            .ok_or_else(LuceneError::null_pointer)?;
        let number_type = points_config.get_type();
        let field = StringUtils::to_string(range.get_field()?.as_ref());
        let min_inclusive = range.is_lower_inclusive();
        let max_inclusive = range.is_upper_inclusive();

        // TODO: push down cleaning up of crazy nulls and inclusive/exclusive elsewhere
        if *number_type == INTEGER {
            let mut lower = match lower_number {
                None => i32::MIN,
                Some(JavaNumber::Integer(v)) => v,
                Some(n) => return Err(number_class_cast(&n, "java.lang.Integer")),
            };
            if !min_inclusive {
                lower = lower.wrapping_add(1);
            }
            let mut upper = match upper_number {
                None => i32::MAX,
                Some(JavaNumber::Integer(v)) => v,
                Some(n) => return Err(number_class_cast(&n, "java.lang.Integer")),
            };
            if !max_inclusive {
                upper = upper.wrapping_sub(1);
            }
            Ok(Some(IntPoint::new_range_query(
                &points_field(field)?,
                &[lower],
                &[upper],
            )?))
        } else if *number_type == LONG {
            let mut lower = match lower_number {
                None => i64::MIN,
                Some(JavaNumber::Long(v)) => v,
                Some(n) => return Err(number_class_cast(&n, "java.lang.Long")),
            };
            if !min_inclusive {
                lower = lower.wrapping_add(1);
            }
            let mut upper = match upper_number {
                None => i64::MAX,
                Some(JavaNumber::Long(v)) => v,
                Some(n) => return Err(number_class_cast(&n, "java.lang.Long")),
            };
            if !max_inclusive {
                upper = upper.wrapping_sub(1);
            }
            Ok(Some(LongPoint::new_range_query(
                &points_field(field)?,
                &[lower],
                &[upper],
            )?))
        } else if *number_type == FLOAT {
            let mut lower = match lower_number {
                None => f32::NEG_INFINITY,
                Some(JavaNumber::Float(v)) => v,
                Some(n) => return Err(number_class_cast(&n, "java.lang.Float")),
            };
            if !min_inclusive {
                lower = lower.next_up();
            }
            let mut upper = match upper_number {
                None => f32::INFINITY,
                Some(JavaNumber::Float(v)) => v,
                Some(n) => return Err(number_class_cast(&n, "java.lang.Float")),
            };
            if !max_inclusive {
                upper = upper.next_down();
            }
            Ok(Some(FloatPoint::new_range_query(
                &points_field(field)?,
                &[lower],
                &[upper],
            )?))
        } else if *number_type == DOUBLE {
            let mut lower = match lower_number {
                None => f64::NEG_INFINITY,
                Some(JavaNumber::Double(v)) => v,
                Some(n) => return Err(number_class_cast(&n, "java.lang.Double")),
            };
            if !min_inclusive {
                lower = lower.next_up();
            }
            let mut upper = match upper_number {
                None => f64::INFINITY,
                Some(JavaNumber::Double(v)) => v,
                Some(n) => return Err(number_class_cast(&n, "java.lang.Double")),
            };
            if !max_inclusive {
                upper = upper.next_down();
            }
            Ok(Some(DoublePoint::new_range_query(
                &points_field(field)?,
                &[lower],
                &[upper],
            )?))
        } else {
            Err(QueryNodeException::new(Arc::new(MessageImpl::with_args(
                QueryParserMessages::UNSUPPORTED_NUMERIC_DATA_TYPE.get(),
                vec![JavaObject::class(number_type)],
            )))
            .error())
        }
    }
}
