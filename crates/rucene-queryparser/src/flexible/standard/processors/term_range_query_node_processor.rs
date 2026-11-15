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

//! Port of `TermRangeQueryNodeProcessor.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.standard.processors`.

use rucene::document::DateTools;
use rucene::error::{LuceneError, Result};

use crate::flexible::core::nodes::field_query_node::FIELD_QUERY_NODE_CLASS;
use crate::flexible::core::nodes::query_node::{class_cast, QueryNodeRef};
use crate::flexible::core::processors::query_node_processor_impl::{
    QueryNodeProcessorImpl, QueryNodeProcessorImplOps,
};
use crate::flexible::standard::config::standard_query_config_handler::ConfigurationKeys;
use crate::flexible::standard::nodes::term_range_query_node::TermRangeQueryNode;
use crate::jdk::calendar::{self, Calendar};
use crate::jdk::char_sequence::CharSequence;
use crate::jdk::date_format;
use crate::jdk::locale::Locale;
use crate::jdk::time_zone::TimeZone;

/// This processors process `TermRangeQueryNode`s. It reads the lower and
/// upper bounds value from the `TermRangeQueryNode` object and try to parse
/// their values using a `DateFormat`. If the values cannot be parsed to a
/// date value, it will only create the `TermRangeQueryNode` using the
/// non-parsed values.
///
/// If a `ConfigurationKeys.LOCALE` is defined in the `QueryConfigHandler`
/// it will be used to parse the date, otherwise `Locale.getDefault()` will
/// be used.
///
/// If a `ConfigurationKeys.DATE_RESOLUTION` is defined and the
/// `DateTools.Resolution` is not `null` it will also be used to parse the
/// date value.
///
/// Equivalent to `org.apache.lucene.queryparser.flexible.standard.processors.TermRangeQueryNodeProcessor`.
///
/// **Divergence.** As in `FuzzyQueryNodeProcessor`, a bound without a field
/// throws a `NullPointerException` when the analyzer normalizes a text that
/// is not a date, where Java's `Analyzer.normalize` accepts a `null` field.
#[derive(Debug, Default)]
pub struct TermRangeQueryNodeProcessor {
    base: QueryNodeProcessorImpl,
}

impl TermRangeQueryNodeProcessor {
    /// Equivalent to `new TermRangeQueryNodeProcessor()`.
    pub fn new() -> Self {
        // empty constructor
        Self::default()
    }
}

impl QueryNodeProcessorImplOps for TermRangeQueryNodeProcessor {
    fn query_node_processor_impl(&self) -> &QueryNodeProcessorImpl {
        &self.base
    }

    fn post_process_node(&self, node: QueryNodeRef) -> Result<QueryNodeRef> {
        let Some(term_range_node) = node.as_any().downcast_ref::<TermRangeQueryNode>() else {
            return Ok(node);
        };
        let range = term_range_node.abstract_range_query_node();
        let upper_bound = range.get_upper_bound()?;
        let upper = upper_bound
            .as_field_query_node()
            .ok_or_else(|| class_cast(upper_bound.as_ref(), &FIELD_QUERY_NODE_CLASS))?;
        let lower_bound = range.get_lower_bound()?;
        let lower = lower_bound
            .as_field_query_node()
            .ok_or_else(|| class_cast(lower_bound.as_ref(), &FIELD_QUERY_NODE_CLASS))?;

        let mut inclusive = false;
        let config = self
            .get_query_config_handler()
            .ok_or_else(LuceneError::null_pointer)?;
        let locale = config
            .get(ConfigurationKeys::LOCALE)
            .unwrap_or_else(Locale::get_default);

        let time_zone = config
            .get(ConfigurationKeys::TIMEZONE)
            .unwrap_or_else(TimeZone::get_default);

        let field = range.get_field()?;
        let field_str = field.as_ref().map(CharSequence::to_string);

        let field_config = config.get_field_config(field_str.as_deref())?;
        // `if (fieldConfig != null)`: the configuration is never `null`.
        let date_res = field_config.get(ConfigurationKeys::DATE_RESOLUTION);

        if range.is_upper_inclusive() {
            inclusive = true;
        }

        let mut part1 = lower.get_text_as_string();
        let mut part2 = upper.get_text_as_string();

        let converted = (|| -> Result<()> {
            let df = date_format::get_date_instance(date_format::SHORT, &locale)?;
            df.set_lenient(true);

            if !part1
                .as_deref()
                .ok_or_else(LuceneError::null_pointer)?
                .is_empty()
            {
                let d1 = df.parse(part1.as_deref().unwrap_or_default())?;
                let resolution = date_res.ok_or_else(LuceneError::null_pointer)?;
                part1 = Some(DateTools::time_to_string(d1.get_time(), resolution));
                lower.set_text(part1.as_deref().map(CharSequence::from));
            }

            if !part2
                .as_deref()
                .ok_or_else(LuceneError::null_pointer)?
                .is_empty()
            {
                let mut d2 = df.parse(part2.as_deref().unwrap_or_default())?;
                if inclusive {
                    // The user can only specify the date, not the time, so make sure
                    // the time is set to the latest possible time of that date to
                    // really
                    // include all documents:
                    let mut cal = Calendar::get_instance(&time_zone, &locale)?;
                    cal.set_time(d2);
                    cal.set(calendar::HOUR_OF_DAY, 23);
                    cal.set(calendar::MINUTE, 59);
                    cal.set(calendar::SECOND, 59);
                    cal.set(calendar::MILLISECOND, 999);
                    d2 = cal.get_time()?;
                }
                let resolution = date_res.ok_or_else(LuceneError::null_pointer)?;
                part2 = Some(DateTools::time_to_string(d2.get_time(), resolution));
                upper.set_text(part2.as_deref().map(CharSequence::from));
            }
            Ok(())
        })();

        match converted {
            Ok(()) => {}
            // catch (Exception e): not a date
            Err(e) if crate::javacc::is_exception(&e) => {
                let analyzer = config.get(ConfigurationKeys::ANALYZER);
                if let Some(analyzer) = analyzer {
                    // because we call utf8ToString, this will only work with the default
                    // TermToBytesRefAttribute
                    let field = lower
                        .get_field_as_string()
                        .ok_or_else(LuceneError::null_pointer)?;
                    part1 = Some(
                        analyzer
                            .normalize(
                                &field,
                                part1.as_deref().ok_or_else(LuceneError::null_pointer)?,
                            )?
                            .utf8_to_string()?,
                    );
                    part2 = Some(
                        analyzer
                            .normalize(
                                &field,
                                part2.as_deref().ok_or_else(LuceneError::null_pointer)?,
                            )?
                            .utf8_to_string()?,
                    );
                    lower.set_text(part1.as_deref().map(CharSequence::from));
                    upper.set_text(part2.as_deref().map(CharSequence::from));
                }
            }
            Err(e) => return Err(e),
        }
        Ok(node)
    }

    fn pre_process_node(&self, node: QueryNodeRef) -> Result<QueryNodeRef> {
        Ok(node)
    }

    fn set_children_order(&self, children: Vec<QueryNodeRef>) -> Result<Vec<QueryNodeRef>> {
        Ok(children)
    }
}
