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

//! Port of `PointRangeQueryBuilder.java`, of the Java package
//! `org.apache.lucene.queryparser.xml.builders`.

use std::sync::Arc;

use rucene::document::{DoublePoint, FloatPoint, IntPoint, LongPoint};
use rucene::error::{LuceneError, Result, NUMBER_FORMAT_EXCEPTION};
use rucene::search::query::Query;

use crate::jdk::dom::Element;
use crate::jdk::locale::Locale;
use crate::jdk::string::to_lower_case_str;
use crate::xml::dom_utils::DOMUtils;
use crate::xml::parser_exception::ParserException;
use crate::xml::query_builder::QueryBuilder;

/// Creates a range query across 1D `PointValues`. The table below specifies
/// the required attributes and the defaults if optional attributes are
/// omitted: `fieldName` (required, inherited), `lowerTerm` (the type's
/// minimum when omitted), `upperTerm` (the type's maximum when omitted),
/// `type` (`int`, `long`, `float` or `double`; `int` by default).
///
/// A `ParserException` will be thrown if an error occurs parsing the
/// supplied `lowerTerm` or `upperTerm` into the numeric type specified by
/// `type`.
///
/// Equivalent to
/// `org.apache.lucene.queryparser.xml.builders.PointRangeQueryBuilder`.
#[derive(Debug, Default)]
pub struct PointRangeQueryBuilder;

impl PointRangeQueryBuilder {
    /// Equivalent to `new PointRangeQueryBuilder()`.
    pub fn new() -> Self {
        Self
    }
}

/// `Long.parseLong(String)`.
fn parse_long(s: &str) -> Result<i64> {
    rucene::util::extra::java_parse_long(s)
        .ok_or_else(|| LuceneError::number_format_for_input_string(s, 10))
}

/// `Double.parseDouble(String)`.
fn parse_double(s: &str) -> Result<f64> {
    if s.trim_matches(|c: char| c <= '\u{20}').is_empty() {
        return Err(LuceneError::number_format("empty String"));
    }
    rucene::util::java_fmt::parse_double(s)
        .ok_or_else(|| LuceneError::number_format(format!("For input string: \"{s}\"")))
}

impl QueryBuilder for PointRangeQueryBuilder {
    /// Equivalent to `getQuery(Element e)`.
    ///
    /// # Errors
    ///
    /// `ParserException("type attribute must be one of: [long, int, double,
    /// float]")`, and `ParserException("Could not parse lowerTerm or
    /// upperTerm into a number", nfe)` for a `NumberFormatException`.
    fn get_query(&self, e: &Element) -> Result<Arc<dyn Query>> {
        let field = DOMUtils::get_attribute_with_inheritance_or_fail(e, "fieldName")?;
        let lower_term = DOMUtils::get_attribute_string(e, "lowerTerm", None);
        let upper_term = DOMUtils::get_attribute_string(e, "upperTerm", None);
        let type_ = to_lower_case_str(
            &DOMUtils::get_attribute_string(e, "type", Some("int")).unwrap_or_default(),
            &Locale::get_default(),
        );
        let result = (|| -> Result<Arc<dyn Query>> {
            match type_.as_str() {
                "int" => {
                    let lower = match &lower_term {
                        None => i32::MIN,
                        Some(t) => crate::javacc::parse_int(t)?,
                    };
                    let upper = match &upper_term {
                        None => i32::MAX,
                        Some(t) => crate::javacc::parse_int(t)?,
                    };
                    IntPoint::new_range_query(&field, &[lower], &[upper])
                }
                "long" => {
                    let lower = match &lower_term {
                        None => i64::MIN,
                        Some(t) => parse_long(t)?,
                    };
                    let upper = match &upper_term {
                        None => i64::MAX,
                        Some(t) => parse_long(t)?,
                    };
                    LongPoint::new_range_query(&field, &[lower], &[upper])
                }
                "double" => {
                    let lower = match &lower_term {
                        None => f64::NEG_INFINITY,
                        Some(t) => parse_double(t)?,
                    };
                    let upper = match &upper_term {
                        None => f64::INFINITY,
                        Some(t) => parse_double(t)?,
                    };
                    DoublePoint::new_range_query(&field, &[lower], &[upper])
                }
                "float" => {
                    let lower = match &lower_term {
                        None => f32::NEG_INFINITY,
                        Some(t) => crate::javacc::parse_float(t)?,
                    };
                    let upper = match &upper_term {
                        None => f32::INFINITY,
                        Some(t) => crate::javacc::parse_float(t)?,
                    };
                    FloatPoint::new_range_query(&field, &[lower], &[upper])
                }
                _ => Err(ParserException::with_message(
                    "type attribute must be one of: [long, int, double, float]",
                )
                .error()),
            }
        })();
        result.map_err(|e| {
            let is_nfe = matches!(
                e.kind(),
                rucene::error::LuceneErrorKind::Unchecked { class, .. } if *class == NUMBER_FORMAT_EXCEPTION
            );
            if is_nfe {
                ParserException::with_message_and_cause("Could not parse lowerTerm or upperTerm into a number", e)
                    .error()
            } else {
                e
            }
        })
    }
}
