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

//! Port of `WildcardQueryNodeProcessor.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.standard.processors`.

use rucene::analysis::Analyzer;
use rucene::error::{LuceneError, Result};

use crate::flexible::core::nodes::query_node::{instance_of, QueryNodeRef};
use crate::flexible::core::nodes::quoted_field_query_node::QUOTED_FIELD_QUERY_NODE_CLASS;
use crate::flexible::core::processors::query_node_processor_impl::{
    QueryNodeProcessorImpl, QueryNodeProcessorImplOps,
};
use crate::flexible::core::util::unescaped_char_sequence::UnescapedCharSequence;
use crate::flexible::standard::config::standard_query_config_handler::ConfigurationKeys;
use crate::flexible::standard::nodes::prefix_wildcard_query_node::PrefixWildcardQueryNode;
use crate::flexible::standard::nodes::term_range_query_node::TERM_RANGE_QUERY_NODE_CLASS;
use crate::flexible::standard::nodes::wildcard_query_node::WildcardQueryNode;
use crate::jdk::char_sequence::CharSequence;

/// The UTF-16 unit of an ASCII character.
const fn unit(c: u8) -> u16 {
    c as u16
}

/// The `QueryNodeProcessorPipeline` processor that converts every
/// `FieldQueryNode` whose text holds an unescaped `*` or `?` (and is
/// neither a range bound nor quoted) into a `WildcardQueryNode` or a
/// `PrefixWildcardQueryNode`, simulating the old query parser.
///
/// Equivalent to `org.apache.lucene.queryparser.flexible.standard.processors.WildcardQueryNodeProcessor`.
#[derive(Debug, Default)]
pub struct WildcardQueryNodeProcessor {
    base: QueryNodeProcessorImpl,
}

impl WildcardQueryNodeProcessor {
    /// Equivalent to `new WildcardQueryNodeProcessor()`.
    pub fn new() -> Self {
        // empty constructor
        Self::default()
    }

    /// Equivalent to the private static `analyzeWildcard(Analyzer a, String
    /// field, String wildcard)`: the chunks between the matches of
    /// `WILDCARD_PATTERN`, `Pattern.compile("(\\.)|([?*]+)")`, normalized,
    /// and the runs of wildcard characters (group 2) kept as they are. As
    /// Lucene compiles it, group 1 matches a literal `.` (the pattern's
    /// `\\.` is the escaped dot of the regex, not a backslash and any
    /// character), and such a match is skipped without moving `last`.
    ///
    /// # Errors
    ///
    /// The exceptions of `Analyzer.normalize` and of the UTF-8 decoding.
    fn analyze_wildcard(a: &dyn Analyzer, field: Option<&str>, wildcard: &str) -> Result<String> {
        // best effort to not pass the wildcard characters through #normalize
        let units: Vec<u16> = wildcard.encode_utf16().collect();
        let mut sb = String::new();
        let mut last = 0usize;
        let mut i = 0usize;
        while i < units.len() {
            let c = units[i];
            if c == unit(b'.') {
                // continue if escaped char
                i += 1;
                continue;
            }
            if c == unit(b'?') || c == unit(b'*') {
                let start = i;
                let mut end = i + 1;
                while end < units.len() && (units[end] == unit(b'?') || units[end] == unit(b'*')) {
                    end += 1;
                }
                if start > 0 {
                    let chunk = String::from_utf16_lossy(&units[last..start]);
                    let normalized = a.normalize_nullable_field(field, &chunk)?;
                    sb.push_str(&normalized.utf8_to_string()?);
                }
                // append the wildcard character
                sb.push_str(&String::from_utf16_lossy(&units[start..end]));
                last = end;
                i = end;
                continue;
            }
            i += 1;
        }
        if last < units.len() {
            let chunk = String::from_utf16_lossy(&units[last..]);
            let normalized = a.normalize_nullable_field(field, &chunk)?;
            sb.push_str(&normalized.utf8_to_string()?);
        }
        Ok(sb)
    }

    /// Equivalent to the private `isWildcard(CharSequence text)`.
    fn is_wildcard(text: Option<&CharSequence>) -> Result<bool> {
        let Some(text) = text else {
            return Ok(false);
        };
        if text.is_empty() {
            return Ok(false);
        }
        // If a un-escaped '*' or '?' if found return true
        // start at the end since it's more common to put wildcards at the end
        for i in (0..text.length()).rev() {
            let c = text.char_at(i)?;
            if (c == unit(b'*') || c == unit(b'?'))
                && !UnescapedCharSequence::was_escaped_in(text, i)?
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Equivalent to the private `isPrefixWildcard(CharSequence text)`.
    fn is_prefix_wildcard(text: Option<&CharSequence>) -> Result<bool> {
        let Some(text) = text else {
            return Ok(false);
        };
        if text.is_empty() || !Self::is_wildcard(Some(text))? {
            return Ok(false);
        }
        let length = text.length();
        // Validate last character is a '*' and was not escaped
        // If single '*' is is a wildcard not prefix to simulate old queryparser
        if text.char_at(length - 1)? != unit(b'*') {
            return Ok(false);
        }
        if UnescapedCharSequence::was_escaped_in(text, length - 1)? {
            return Ok(false);
        }
        if length == 1 {
            return Ok(false);
        }
        // Only make a prefix if there is only one single star at the end and no '?' or '*' characters
        // If single wildcard return false to mimic old queryparser
        for i in 0..length {
            let c = text.char_at(i)?;
            if c == unit(b'?') {
                return Ok(false);
            }
            if c == unit(b'*') && !UnescapedCharSequence::was_escaped_in(text, i)? {
                return Ok(i == length - 1);
            }
        }
        Ok(false)
    }
}

impl QueryNodeProcessorImplOps for WildcardQueryNodeProcessor {
    fn query_node_processor_impl(&self) -> &QueryNodeProcessorImpl {
        &self.base
    }

    fn post_process_node(&self, node: QueryNodeRef) -> Result<QueryNodeRef> {
        // the old Lucene Parser ignores FuzzyQueryNode that are also PrefixWildcardQueryNode or
        // WildcardQueryNode
        // we do the same here, also ignore empty terms
        let Some(fqn) = node.as_field_query_node() else {
            return Ok(node);
        };
        let mut text = fqn.get_text();
        // do not process wildcards for TermRangeQueryNode children and
        // QuotedFieldQueryNode to reproduce the old parser behavior
        let parent_is_range = node
            .get_parent()
            .is_some_and(|p| instance_of(p.as_ref(), &TERM_RANGE_QUERY_NODE_CLASS));
        if parent_is_range
            || instance_of(node.as_ref(), &QUOTED_FIELD_QUERY_NODE_CLASS)
            || text
                .as_ref()
                .ok_or_else(LuceneError::null_pointer)?
                .is_empty()
        {
            // Ignore empty terms
            return Ok(node);
        }

        // Code below simulates the old lucene parser behavior for wildcards

        if Self::is_wildcard(text.as_ref())? {
            let analyzer = self
                .get_query_config_handler()
                .ok_or_else(LuceneError::null_pointer)?
                .get(ConfigurationKeys::ANALYZER);
            if let Some(analyzer) = analyzer {
                // the field may be `null` (an empty `MULTI_FIELDS`)
                let field = fqn.get_field_as_string();
                let wildcard = text
                    .as_ref()
                    .map(CharSequence::to_string)
                    .unwrap_or_default();
                text = Some(CharSequence::from(Self::analyze_wildcard(
                    analyzer.as_ref(),
                    field.as_deref(),
                    &wildcard,
                )?));
            }

            if Self::is_prefix_wildcard(text.as_ref())? {
                let prefix: QueryNodeRef = PrefixWildcardQueryNode::new(
                    fqn.get_field(),
                    text,
                    fqn.get_begin(),
                    fqn.get_end(),
                );
                return Ok(prefix);
            } else {
                let wildcard: QueryNodeRef =
                    WildcardQueryNode::new(fqn.get_field(), text, fqn.get_begin(), fqn.get_end());
                return Ok(wildcard);
            }
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
