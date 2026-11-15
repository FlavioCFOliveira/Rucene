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

//! Port of `AnalyzerQueryNodeProcessor.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.standard.processors`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use rucene::analysis::{
    Analyzer, CachingTokenFilter, CharTermAttribute, PositionIncrementAttribute, SharedTokenStream,
    TokenStream,
};
use rucene::error::{LuceneError, Result};
use rucene::util::atomic_ref_cell::AtomicRefCell;

use crate::flexible::core::messages::query_parser_messages::QueryParserMessages;
use crate::flexible::core::nodes::boolean_query_node::{
    BooleanQueryNode, BOOLEAN_QUERY_NODE_CLASS,
};
use crate::flexible::core::nodes::field_query_node::FieldQueryNode;
use crate::flexible::core::nodes::fuzzy_query_node::FUZZY_QUERY_NODE_CLASS;
use crate::flexible::core::nodes::group_query_node::GroupQueryNode;
use crate::flexible::core::nodes::modifier_query_node::{Modifier, ModifierQueryNode};
use crate::flexible::core::nodes::no_token_found_query_node::NoTokenFoundQueryNode;
use crate::flexible::core::nodes::query_node::{instance_of, QueryNode, QueryNodeRef};
use crate::flexible::core::nodes::quoted_field_query_node::QUOTED_FIELD_QUERY_NODE_CLASS;
use crate::flexible::core::nodes::range_query_node::RANGE_QUERY_NODE_CLASS;
use crate::flexible::core::nodes::tokenized_phrase_query_node::TokenizedPhraseQueryNode;
use crate::flexible::core::processors::query_node_processor_impl::{
    QueryNodeProcessorImpl, QueryNodeProcessorImplOps,
};
use crate::flexible::messages::nls::NLS;
use crate::flexible::standard::config::standard_query_config_handler::{
    ConfigurationKeys, Operator,
};
use crate::flexible::standard::nodes::multi_phrase_query_node::MultiPhraseQueryNode;
use crate::flexible::standard::nodes::synonym_query_node::SynonymQueryNode;
use crate::flexible::standard::nodes::wildcard_query_node::WILDCARD_QUERY_NODE_CLASS;
use crate::jdk::char_sequence::CharSequence;

/// This processor verifies if `ConfigurationKeys.ANALYZER` is defined in the
/// `QueryConfigHandler`. If it is and the analyzer is not `null`, it looks
/// for every `FieldQueryNode` that is not `WildcardQueryNode`,
/// `FuzzyQueryNode` or `RangeQueryNode` contained in the query node tree,
/// then it applies the analyzer to that `FieldQueryNode` object.
///
/// If the analyzer return only one term, the returned term is set to the
/// `FieldQueryNode` and it's returned.
///
/// If the analyzer return more than one term, a `TokenizedPhraseQueryNode`
/// or `MultiPhraseQueryNode` is created, whether there is one or more terms
/// at the same position, and it's returned.
///
/// If no term is returned by the analyzer a `NoTokenFoundQueryNode` object
/// is returned.
///
/// Equivalent to `org.apache.lucene.queryparser.flexible.standard.processors.AnalyzerQueryNodeProcessor`.
#[derive(Default)]
pub struct AnalyzerQueryNodeProcessor {
    base: QueryNodeProcessorImpl,
    analyzer: Mutex<Option<Arc<dyn Analyzer>>>,
    position_increments_enabled: AtomicBool,
    default_operator: Mutex<Option<Operator>>,
}

impl std::fmt::Debug for AnalyzerQueryNodeProcessor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AnalyzerQueryNodeProcessor")
            .field("base", &self.base)
            .field(
                "position_increments_enabled",
                &self.position_increments_enabled.load(Ordering::SeqCst),
            )
            .finish_non_exhaustive()
    }
}

/// The token stream's current position increment: the attribute's value, or
/// `1` without the attribute (`posIncrAtt == null`).
fn position_increment(buffer: &CachingTokenFilter, has_pos_incr_att: bool) -> i32 {
    if !has_pos_incr_att {
        return 1;
    }
    buffer
        .attribute_source()
        .get_attribute_interface::<dyn PositionIncrementAttribute>()
        .map_or(1, |att| att.get_position_increment())
}

/// `termAtt.toString()`: the current term.
fn term_text(buffer: &CachingTokenFilter) -> Option<String> {
    buffer
        .attribute_source()
        .get_attribute_interface::<dyn CharTermAttribute>()
        .map(|att| att.buffer()[..att.length()].iter().collect())
}

/// `boolean hasNext = buffer.incrementToken(); assert hasNext == true; term =
/// termAtt.toString();` inside `try { ... } catch (IOException e) { }`: the
/// term, `None` when an `IOException` interrupted it ("safe to ignore,
/// because we know the number of tokens").
fn next_term(buffer: &mut CachingTokenFilter) -> Result<Option<String>> {
    match buffer.increment_token() {
        Ok(has_next) => {
            debug_assert!(has_next);
            Ok(term_text(buffer))
        }
        Err(e) if e.is_io_exception() => Ok(None),
        Err(e) => Err(e),
    }
}

/// `node.add(child)` of a child that may be `null`: the
/// `IllegalArgumentException` (`NODE_ACTION_NOT_SUPPORTED`) Java's
/// `QueryNodeImpl.add` throws for `null`.
fn add_child(node: &dyn QueryNode, child: Option<QueryNodeRef>) -> Result<()> {
    match child {
        Some(child) => node.add(child),
        None => Err(LuceneError::illegal_argument(NLS::get_localized_message(
            QueryParserMessages::NODE_ACTION_NOT_SUPPORTED
                .get()
                .as_deref(),
        )?)),
    }
}

/// The outcome of a try-with-resources body followed by the resource's
/// `close()`: the body's failure wins and carries a close failure as
/// suppressed; otherwise the close failure, if any, is the outcome.
fn close_after<T>(result: Result<T>, closed: Result<()>) -> Result<T> {
    match (result, closed) {
        (Ok(value), Ok(())) => Ok(value),
        (Ok(_), Err(close_error)) => Err(close_error),
        (Err(e), Ok(())) => Err(e),
        (Err(mut e), Err(close_error)) => {
            e.add_suppressed(close_error);
            Err(e)
        }
    }
}

/// What the first pass over the tokens counted.
#[derive(Debug, Default)]
struct Counts {
    has_pos_incr_att: bool,
    num_tokens: i32,
    position_count: i32,
    several_tokens_at_same_position: bool,
}

impl AnalyzerQueryNodeProcessor {
    /// Equivalent to `new AnalyzerQueryNodeProcessor()`.
    pub fn new() -> Self {
        // empty constructor
        Self::default()
    }

    fn analyzer(&self) -> Option<Arc<dyn Analyzer>> {
        self.analyzer
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn default_operator(&self) -> Option<Operator> {
        *self
            .default_operator
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// The body of the `try (TokenStream source = ...)` block: the buffer is
    /// created (and kept in `slot`, which the `finally` block closes), reset,
    /// read through once to count the tokens and positions, and reset again.
    fn count_tokens(
        source: &Arc<AtomicRefCell<dyn TokenStream>>,
        slot: &mut Option<CachingTokenFilter>,
        counts: &mut Counts,
    ) -> Result<()> {
        let buffer = slot.insert(CachingTokenFilter::new(Box::new(SharedTokenStream::new(
            Arc::clone(source),
        ))));
        buffer.reset()?;

        if buffer
            .attribute_source()
            .has_attribute_interface::<dyn PositionIncrementAttribute>()
        {
            counts.has_pos_incr_att = true;
        }

        let counted = (|| -> Result<()> {
            while buffer.increment_token()? {
                counts.num_tokens += 1;
                let position_increment = position_increment(buffer, counts.has_pos_incr_att);
                if position_increment != 0 {
                    counts.position_count += position_increment;
                } else {
                    counts.several_tokens_at_same_position = true;
                }
            }
            Ok(())
        })();
        match counted {
            Ok(()) => {}
            // ignore
            Err(e) if e.is_io_exception() => {}
            Err(e) => return Err(e),
        }

        // rewind the buffer stream
        buffer.reset() // will never through on subsequent reset calls
    }

    /// The `try` block of `postProcessNode` for a node to analyze; `slot`
    /// receives the buffer the `finally` block closes.
    fn analyze(
        &self,
        node: &QueryNodeRef,
        field_node: &FieldQueryNode,
        slot: &mut Option<CachingTokenFilter>,
    ) -> Result<QueryNodeRef> {
        let text = field_node.get_text_as_string();
        let field = field_node.get_field_as_string();
        let mut counts = Counts::default();

        let analyzer = self.analyzer().ok_or_else(LuceneError::null_pointer)?;
        // try (TokenStream source = this.analyzer.tokenStream(field, text)) { ... }
        // catch (IOException e) { throw new RuntimeException(e); }
        let analyzed = (|| -> Result<()> {
            let text = text.as_deref().ok_or_else(LuceneError::null_pointer)?;
            // the field may be `null` (an empty `MULTI_FIELDS`)
            let source = analyzer.token_stream_from_str_nullable_field(field.as_deref(), text)?;
            let body = Self::count_tokens(&source, slot, &mut counts);
            let closed = source.borrow_mut().close();
            close_after(body, closed)
        })();
        if let Err(e) = analyzed {
            if e.is_io_exception() {
                return Err(LuceneError::runtime_exception(e));
            }
            return Err(e);
        }
        let buffer = slot.as_mut().ok_or_else(LuceneError::null_pointer)?;
        let has_pos_incr_att = counts.has_pos_incr_att;
        let num_tokens = counts.num_tokens;
        let position_count = counts.position_count;
        let several_tokens_at_same_position = counts.several_tokens_at_same_position;
        let field_cs = field.as_deref().map(CharSequence::from);

        if !buffer
            .attribute_source()
            .has_attribute_interface::<dyn CharTermAttribute>()
        {
            let no_token: QueryNodeRef = NoTokenFoundQueryNode::new();
            return Ok(no_token);
        }

        if num_tokens == 0 {
            let no_token: QueryNodeRef = NoTokenFoundQueryNode::new();
            Ok(no_token)
        } else if num_tokens == 1 {
            let term = next_term(buffer)?;
            field_node.set_text(term.map(CharSequence::from));
            Ok(Arc::clone(node))
        } else if several_tokens_at_same_position
            || !instance_of(node.as_ref(), &QUOTED_FIELD_QUERY_NODE_CLASS)
        {
            if position_count == 1 || !instance_of(node.as_ref(), &QUOTED_FIELD_QUERY_NODE_CLASS) {
                // no phrase query:

                if position_count == 1 {
                    // simple case: only one position, with synonyms
                    let mut children: Vec<QueryNodeRef> = Vec::new();

                    for _ in 0..num_tokens {
                        let term = next_term(buffer)?;
                        children.push(FieldQueryNode::new(
                            field_cs.clone(),
                            term.map(CharSequence::from),
                            -1,
                            -1,
                        ));
                    }
                    let synonyms: QueryNodeRef = SynonymQueryNode::new(Some(children))?;
                    let group: QueryNodeRef = GroupQueryNode::new(Some(synonyms))?;
                    Ok(group)
                } else {
                    // multiple positions
                    let mut q: QueryNodeRef = BooleanQueryNode::new(Some(Vec::new()))?;
                    let mut current_query: Option<QueryNodeRef> = None;
                    for _ in 0..num_tokens {
                        let term = next_term(buffer)?;
                        if has_pos_incr_att && position_increment(buffer, true) == 0 {
                            let current_is_boolean = current_query.as_ref().is_some_and(|c| {
                                instance_of(c.as_ref(), &BOOLEAN_QUERY_NODE_CLASS)
                            });
                            if !current_is_boolean {
                                let t = current_query.take();
                                let synonyms: QueryNodeRef =
                                    SynonymQueryNode::new(Some(Vec::new()))?;
                                add_child(synonyms.as_ref(), t)?;
                                current_query = Some(synonyms);
                            }
                            let current = current_query
                                .as_ref()
                                .ok_or_else(LuceneError::null_pointer)?;
                            let term_node: QueryNodeRef = FieldQueryNode::new(
                                field_cs.clone(),
                                term.map(CharSequence::from),
                                -1,
                                -1,
                            );
                            current.add(term_node)?;
                        } else {
                            if let Some(current) = current_query.take() {
                                if self.default_operator() == Some(Operator::Or) {
                                    q.add(current)?;
                                } else {
                                    let required: QueryNodeRef =
                                        ModifierQueryNode::new(Some(current), Modifier::ModReq)?;
                                    q.add(required)?;
                                }
                            }
                            current_query = Some(FieldQueryNode::new(
                                field_cs.clone(),
                                term.map(CharSequence::from),
                                -1,
                                -1,
                            ));
                        }
                    }
                    if self.default_operator() == Some(Operator::Or) {
                        add_child(q.as_ref(), current_query)?;
                    } else {
                        let required: QueryNodeRef =
                            ModifierQueryNode::new(current_query, Modifier::ModReq)?;
                        q.add(required)?;
                    }

                    if instance_of(q.as_ref(), &BOOLEAN_QUERY_NODE_CLASS) {
                        q = GroupQueryNode::new(Some(q))?;
                    }
                    Ok(q)
                }
            } else {
                // phrase query:
                let mpq = MultiPhraseQueryNode::new();

                let mut multi_terms: Vec<Arc<FieldQueryNode>> = Vec::new();
                let mut position = -1i32;
                let mut term_group_count = 0i32;
                for _ in 0..num_tokens {
                    let mut position_increment_value = 1;
                    let term = match buffer.increment_token() {
                        Ok(has_next) => {
                            debug_assert!(has_next);
                            let term = term_text(buffer);
                            if has_pos_incr_att {
                                position_increment_value = position_increment(buffer, true);
                            }
                            term
                        }
                        // safe to ignore, because we know the number of tokens
                        Err(e) if e.is_io_exception() => None,
                        Err(e) => return Err(e),
                    };

                    if position_increment_value > 0 && !multi_terms.is_empty() {
                        for term_node in &multi_terms {
                            if self.position_increments_enabled.load(Ordering::SeqCst) {
                                term_node.set_position_increment(position);
                            } else {
                                term_node.set_position_increment(term_group_count);
                            }

                            let term_node: QueryNodeRef = term_node.clone();
                            mpq.add(term_node)?;
                        }

                        // Only increment once for each "group" of
                        // terms that were in the same position:
                        term_group_count += 1;

                        multi_terms.clear();
                    }

                    position = position.wrapping_add(position_increment_value);
                    multi_terms.push(FieldQueryNode::new(
                        field_cs.clone(),
                        term.map(CharSequence::from),
                        -1,
                        -1,
                    ));
                }

                for term_node in &multi_terms {
                    if self.position_increments_enabled.load(Ordering::SeqCst) {
                        term_node.set_position_increment(position);
                    } else {
                        term_node.set_position_increment(term_group_count);
                    }

                    let term_node: QueryNodeRef = term_node.clone();
                    mpq.add(term_node)?;
                }

                let mpq: QueryNodeRef = mpq;
                Ok(mpq)
            }
        } else {
            let pq = TokenizedPhraseQueryNode::new();

            let mut position = -1i32;

            for i in 0..num_tokens {
                let mut position_increment_value = 1;
                let term = match buffer.increment_token() {
                    Ok(has_next) => {
                        debug_assert!(has_next);
                        let term = term_text(buffer);
                        if has_pos_incr_att {
                            position_increment_value = position_increment(buffer, true);
                        }
                        term
                    }
                    // safe to ignore, because we know the number of tokens
                    Err(e) if e.is_io_exception() => None,
                    Err(e) => return Err(e),
                };

                let new_field_node =
                    FieldQueryNode::new(field_cs.clone(), term.map(CharSequence::from), -1, -1);

                if self.position_increments_enabled.load(Ordering::SeqCst) {
                    position = position.wrapping_add(position_increment_value);
                    new_field_node.set_position_increment(position);
                } else {
                    new_field_node.set_position_increment(i);
                }

                let new_field_node: QueryNodeRef = new_field_node;
                pq.add(new_field_node)?;
            }

            let pq: QueryNodeRef = pq;
            Ok(pq)
        }
    }
}

impl QueryNodeProcessorImplOps for AnalyzerQueryNodeProcessor {
    fn query_node_processor_impl(&self) -> &QueryNodeProcessorImpl {
        &self.base
    }

    fn process(&self, query_tree: QueryNodeRef) -> Result<QueryNodeRef> {
        let config = self
            .get_query_config_handler()
            .ok_or_else(LuceneError::null_pointer)?;
        let analyzer = config.get(ConfigurationKeys::ANALYZER);

        if let Some(analyzer) = analyzer {
            *self.analyzer.lock().unwrap_or_else(PoisonError::into_inner) = Some(analyzer);
            self.position_increments_enabled
                .store(false, Ordering::SeqCst);
            let position_increments_enabled =
                config.get(ConfigurationKeys::ENABLE_POSITION_INCREMENTS);
            let default_operator = config.get(ConfigurationKeys::DEFAULT_OPERATOR);
            *self
                .default_operator
                .lock()
                .unwrap_or_else(PoisonError::into_inner) =
                Some(default_operator.unwrap_or(Operator::Or));

            if let Some(position_increments_enabled) = position_increments_enabled {
                self.position_increments_enabled
                    .store(position_increments_enabled, Ordering::SeqCst);
            }

            if self.analyzer().is_some() {
                return QueryNodeProcessorImpl::process(self, query_tree);
            }
        }

        Ok(query_tree)
    }

    fn post_process_node(&self, node: QueryNodeRef) -> Result<QueryNodeRef> {
        let Some(field_node) = node.as_field_query_node() else {
            return Ok(node);
        };
        let parent_is_range = node
            .get_parent()
            .is_some_and(|p| instance_of(p.as_ref(), &RANGE_QUERY_NODE_CLASS));
        if instance_of(node.as_ref(), &WILDCARD_QUERY_NODE_CLASS)
            || instance_of(node.as_ref(), &FUZZY_QUERY_NODE_CLASS)
            || parent_is_range
        {
            return Ok(node);
        }

        let mut buffer: Option<CachingTokenFilter> = None;
        let result = self.analyze(&node, field_node, &mut buffer);
        // finally
        if let Some(mut buffer) = buffer {
            if let Err(e) = buffer.close() {
                // safe to ignore an IOException
                if !e.is_io_exception() {
                    return Err(e);
                }
            }
        }
        result
    }

    fn pre_process_node(&self, node: QueryNodeRef) -> Result<QueryNodeRef> {
        Ok(node)
    }

    fn set_children_order(&self, children: Vec<QueryNodeRef>) -> Result<Vec<QueryNodeRef>> {
        Ok(children)
    }
}
