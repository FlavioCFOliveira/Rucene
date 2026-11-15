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

//! The port of `StandardSyntaxParser.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.standard.parser`: the parser
//! JavaCC generates from `StandardSyntaxParser.jj`.
//!
//! The regions between `// @generated begin` and `// @generated end` are
//! written by `tools/translate/javacc_java_to_rust.py` from the generated
//! Java; the rest is the grammar's `PARSER_BEGIN` user code (the
//! constructor, `parse`, `parseFloat` and `parseInt`), translated by hand
//! (the translator pins its Java tokens), and the helpers the translator's
//! rules for the grammar's actions call.
//!
//! Java's class is rendered as the state [`StandardSyntaxParser`] and the
//! trait [`StandardSyntaxParserOps`], whose provided methods are the public
//! members; the private productions are free functions.

use std::sync::{Arc, Mutex, PoisonError};

use rucene::analysis::StringCharReader;
use rucene::error::{LuceneError, LuceneErrorKind, Result};
use rucene::search::fuzzy_query;

use super::escape_query_syntax_impl::EscapeQuerySyntaxImpl;
use super::parse_exception::ParseException;
use super::standard_syntax_parser_constants::{self as qpc, StandardSyntaxParserConstants};
use super::standard_syntax_parser_token_manager::StandardSyntaxParserTokenManager;
use super::token::{Token, TokenRef};
use crate::charstream::char_stream::CharStream;
use crate::charstream::fast_char_stream::FastCharStream;
use crate::flexible::core::messages::query_parser_messages::QueryParserMessages;
use crate::flexible::core::nodes::and_query_node::AndQueryNode;
use crate::flexible::core::nodes::boolean_query_node::BooleanQueryNode;
use crate::flexible::core::nodes::boost_query_node::BoostQueryNode;
use crate::flexible::core::nodes::field_query_node::FieldQueryNode;
use crate::flexible::core::nodes::fuzzy_query_node::FuzzyQueryNode;
use crate::flexible::core::nodes::group_query_node::GroupQueryNode;
use crate::flexible::core::nodes::modifier_query_node::{
    Modifier, ModifierQueryNode, MODIFIER_QUERY_NODE_CLASS,
};
use crate::flexible::core::nodes::or_query_node::OrQueryNode;
use crate::flexible::core::nodes::query_node::{class_cast, instance_of, QueryNodeRef};
use crate::flexible::core::nodes::quoted_field_query_node::QuotedFieldQueryNode;
use crate::flexible::core::nodes::slop_query_node::SlopQueryNode;
use crate::flexible::core::parser::syntax_parser::SyntaxParser;
use crate::flexible::core::query_node_parse_exception::QueryNodeParseException;
use crate::flexible::messages::message_impl::MessageImpl;
use crate::flexible::standard::nodes::interval_query_node::IntervalQueryNode;
use crate::flexible::standard::nodes::intervalfn::after::After;
use crate::flexible::standard::nodes::intervalfn::analyzed_text::AnalyzedText;
use crate::flexible::standard::nodes::intervalfn::at_least::AtLeast;
use crate::flexible::standard::nodes::intervalfn::before::Before;
use crate::flexible::standard::nodes::intervalfn::contained_by::ContainedBy;
use crate::flexible::standard::nodes::intervalfn::containing::Containing;
use crate::flexible::standard::nodes::intervalfn::extend::Extend;
use crate::flexible::standard::nodes::intervalfn::fuzzy_term::FuzzyTerm;
use crate::flexible::standard::nodes::intervalfn::interval_function::IntervalFunction;
use crate::flexible::standard::nodes::intervalfn::max_gaps::MaxGaps;
use crate::flexible::standard::nodes::intervalfn::max_width::MaxWidth;
use crate::flexible::standard::nodes::intervalfn::non_overlapping::NonOverlapping;
use crate::flexible::standard::nodes::intervalfn::not_contained_by::NotContainedBy;
use crate::flexible::standard::nodes::intervalfn::not_containing::NotContaining;
use crate::flexible::standard::nodes::intervalfn::not_within::NotWithin;
use crate::flexible::standard::nodes::intervalfn::or::Or;
use crate::flexible::standard::nodes::intervalfn::ordered::Ordered;
use crate::flexible::standard::nodes::intervalfn::overlapping::Overlapping;
use crate::flexible::standard::nodes::intervalfn::phrase::Phrase;
use crate::flexible::standard::nodes::intervalfn::unordered::Unordered;
use crate::flexible::standard::nodes::intervalfn::unordered_no_overlaps::UnorderedNoOverlaps;
use crate::flexible::standard::nodes::intervalfn::wildcard::Wildcard;
use crate::flexible::standard::nodes::intervalfn::within::Within;
use crate::flexible::standard::nodes::min_should_match_node::MinShouldMatchNode;
use crate::flexible::standard::nodes::regexp_query_node::RegexpQueryNode;
use crate::flexible::standard::nodes::term_range_query_node::TermRangeQueryNode;
use crate::jdk::char_sequence::CharSequence;
use crate::jdk::class::JavaClass;
use crate::jdk::object::JavaObject;

/// Parser for the standard Lucene syntax
///
/// Equivalent to `org.apache.lucene.queryparser.flexible.standard.parser.StandardSyntaxParser`,
/// which implements `SyntaxParser`: the JavaCC state; the methods are those
/// of [`StandardSyntaxParserOps`]. The `SyntaxParser` a `QueryParserHelper`
/// holds is shared (`&self`), so it is implemented by a
/// `Mutex<StandardSyntaxParser>`, through which `parse` rewrites the state
/// as Java's does (concurrent calls, which Java's unsynchronized parser
/// does not support, are serialized).
#[derive(Debug)]
pub struct StandardSyntaxParser {
    /// The fields JavaCC generates.
    pub(crate) jj: JjState,
}

impl StandardSyntaxParser {
    /// Equivalent to `new StandardSyntaxParser()`:
    /// `this(new FastCharStream(Reader.nullReader()))`, a reader at its end.
    pub fn new() -> Self {
        Self::with_char_stream(Box::new(FastCharStream::new(Box::new(
            StringCharReader::new(""),
        ))))
    }

    /// Constructor with user supplied CharStream.
    ///
    /// Equivalent to `new StandardSyntaxParser(CharStream)`.
    pub fn with_char_stream(stream: Box<dyn CharStream>) -> Self {
        Self {
            jj: JjState::with_char_stream(stream),
        }
    }

    /// Constructor with generated Token Manager.
    ///
    /// Equivalent to `new StandardSyntaxParser(StandardSyntaxParserTokenManager)`.
    pub fn with_token_manager(tm: StandardSyntaxParserTokenManager) -> Self {
        Self {
            jj: JjState::with_token_manager(tm),
        }
    }

    /// Equivalent to the static `parseFloat(Token token)`:
    /// `Float.parseFloat(token.image)`.
    ///
    /// # Errors
    ///
    /// The `NumberFormatException` of `Float.parseFloat`, and the
    /// `NullPointerException` it throws for a `null` image.
    pub fn parse_float(token: &TokenRef) -> Result<f32> {
        let image = token.read(|o| o.image.clone());
        crate::javacc::parse_float(crate::javacc::nn_str(image.as_deref())?)
    }

    /// Equivalent to the static `parseInt(Token token)`:
    /// `Integer.parseInt(token.image)`.
    ///
    /// # Errors
    ///
    /// The `NumberFormatException` of `Integer.parseInt`, `Cannot parse null
    /// string: null` for a `null` image.
    pub fn parse_int(token: &TokenRef) -> Result<i32> {
        match token.read(|o| o.image.clone()) {
            Some(image) => crate::javacc::parse_int(&image),
            None => Err(LuceneError::number_format("Cannot parse null string: null")),
        }
    }
}

impl Default for StandardSyntaxParser {
    fn default() -> Self {
        Self::new()
    }
}

impl StandardSyntaxParserConstants for StandardSyntaxParser {}

impl StandardSyntaxParserOps for StandardSyntaxParser {
    fn standard_syntax_parser(&self) -> &StandardSyntaxParser {
        self
    }

    fn standard_syntax_parser_mut(&mut self) -> &mut StandardSyntaxParser {
        self
    }
}

impl SyntaxParser for Mutex<StandardSyntaxParser> {
    /// Equivalent to `parse(CharSequence query, CharSequence field)` (see
    /// [`StandardSyntaxParserOps::parse`]).
    fn parse(&self, query: &CharSequence, field: Option<&CharSequence>) -> Result<QueryNodeRef> {
        self.lock()
            .unwrap_or_else(PoisonError::into_inner)
            .parse(query, field)
    }
}

/// `Throwable.getMessage()` of a thrown error.
fn throwable_message(error: &LuceneError) -> Option<String> {
    match error.kind() {
        LuceneErrorKind::Throwable(t) => t.get_message().map(str::to_string),
        LuceneErrorKind::Unchecked { message, .. } => message.clone(),
        LuceneErrorKind::AssertionError(message) => Some(message.clone()),
        other => Some(other.to_string()),
    }
}

/// The children list of a node a production builds from its
/// `ArrayList<QueryNode>`: the node's constructor hands the list to
/// `QueryNodeImpl.set`, which calls `removeFromParent()` on each element
/// first.
///
/// # Errors
///
/// The `NullPointerException` of a `null` element.
fn query_nodes(list: Vec<Option<QueryNodeRef>>) -> Result<Vec<QueryNodeRef>> {
    list.into_iter().map(crate::javacc::nn_owned).collect()
}

/// The sources of an interval function a production builds from its
/// `ArrayList<IntervalFunction>`.
///
/// **Divergence.** Java's interval functions keep a `null` element of the
/// list and throw the `NullPointerException` when they use it
/// (`AtLeast.java`, `Ordered.java` and the other list functions); the port's
/// functions hold non-null sources, so the exception is thrown here. No
/// production of the grammar returns `null`, so no list holds one.
///
/// # Errors
///
/// The `NullPointerException` of a `null` element.
fn interval_functions(
    list: Vec<Option<Arc<dyn IntervalFunction>>>,
) -> Result<Vec<Arc<dyn IntervalFunction>>> {
    list.into_iter().map(crate::javacc::nn_owned).collect()
}

/// `node instanceof class` of a nullable node reference: `false` for `null`.
fn is_instance(node: &Option<QueryNodeRef>, class: &JavaClass) -> bool {
    node.as_ref()
        .is_some_and(|n| instance_of(n.as_ref(), class))
}

/// The reference cast `(class) node`: `null` passes.
///
/// # Errors
///
/// The `ClassCastException` of a node of another class.
fn cast_node(node: Option<QueryNodeRef>, class: &JavaClass) -> Result<Option<QueryNodeRef>> {
    match &node {
        Some(n) if !instance_of(n.as_ref(), class) => Err(class_cast(n.as_ref(), class)),
        _ => Ok(node),
    }
}

/// `((ModifierQueryNode) node).getModifier()`.
///
/// # Errors
///
/// The `NullPointerException` of a `null` node, the `ClassCastException` of
/// a node that is not a `ModifierQueryNode`.
fn modifier_of(node: &Option<QueryNodeRef>) -> Result<Modifier> {
    let node = crate::javacc::nn(node.as_ref())?;
    match node.as_modifier_query_node() {
        Some(m) => Ok(m.get_modifier()),
        None => Err(class_cast(node.as_ref(), &MODIFIER_QUERY_NODE_CLASS)),
    }
}

/// The behaviour of Java's `StandardSyntaxParser`: its public methods,
/// provided for every type that embeds a [`StandardSyntaxParser`].
///
/// Equivalent to the methods of
/// `org.apache.lucene.queryparser.flexible.standard.parser.StandardSyntaxParser`.
pub trait StandardSyntaxParserOps {
    /// The state of the parser.
    fn standard_syntax_parser(&self) -> &StandardSyntaxParser;

    /// The state of the parser, for writing.
    fn standard_syntax_parser_mut(&mut self) -> &mut StandardSyntaxParser;

    /// The JavaCC state.
    fn jj(&self) -> &JjState {
        &self.standard_syntax_parser().jj
    }

    /// The JavaCC state, for writing.
    fn jj_mut(&mut self) -> &mut JjState {
        &mut self.standard_syntax_parser_mut().jj
    }

    /// Reinitialise.
    ///
    /// Equivalent to `ReInit(CharStream)`.
    fn re_init(&mut self, stream: Box<dyn CharStream>) {
        re_init(self, stream);
    }

    /// Equivalent to `TopLevelQuery(CharSequence field)`.
    ///
    /// # Errors
    ///
    /// The `ParseException` of a syntax error, the `TokenMgrError` of a
    /// lexical one, and the exceptions of the actions.
    fn top_level_query(&mut self, field: Option<CharSequence>) -> Result<Option<QueryNodeRef>> {
        top_level_query(self, field)
    }

    /// Parses a query string, returning a `QueryNode`.
    ///
    /// Equivalent to `parse(CharSequence query, CharSequence field)`: a
    /// `ParseException` gets the query (`setQuery`); a `java.lang.Error`
    /// (the token manager's `TokenMgrError`) is rethrown as a
    /// `QueryNodeParseException` whose message is
    /// `INVALID_SYNTAX_CANNOT_PARSE` with the query and the error's message.
    ///
    /// **Divergence.** Java also catches the `StackOverflowError` of a query
    /// nested deeper than the JVM's stack allows, as a `java.lang.Error`; the
    /// port's recursion exhausts the native stack and the process aborts.
    /// Rust offers no recoverable stack overflow, and a depth limit would be
    /// a bound Lucene does not have.
    ///
    /// # Errors
    ///
    /// The `ParseException` (with the query) of a syntax error, the
    /// `QueryNodeParseException` of a lexical one, and the other exceptions
    /// of the actions, which pass through (the `NumberFormatException` of a
    /// number that does not parse).
    fn parse(
        &mut self,
        query: &CharSequence,
        field: Option<&CharSequence>,
    ) -> Result<QueryNodeRef> {
        self.re_init(Box::new(FastCharStream::new(Box::new(
            StringCharReader::new(query.to_string()),
        ))));
        match top_level_query(self, field.cloned()) {
            // `TopLevelQuery` never returns `null`.
            Ok(node) => crate::javacc::nn_owned(node),
            Err(tme) => {
                // catch (ParseException tme)
                if let Some(pe) = ParseException::of(&tme) {
                    let mut pe = pe.clone();
                    pe.query_node_parse_exception_mut()
                        .set_query(Some(query.clone()));
                    return Err(pe.error());
                }
                // catch (Error tme)
                if crate::javacc::is_error(&tme) {
                    let message = MessageImpl::with_args(
                        QueryParserMessages::INVALID_SYNTAX_CANNOT_PARSE.get(),
                        vec![
                            JavaObject::from(query),
                            JavaObject::from(throwable_message(&tme)),
                        ],
                    );
                    let mut e = QueryNodeParseException::with_cause(tme);
                    e.set_query(Some(query.clone()));
                    e.set_non_localized_message(Arc::new(message));
                    return Err(e.error());
                }
                Err(tme)
            }
        }
    }

    // @generated begin productions
    /// `ReInit(StandardSyntaxParserTokenManager)` of the generated Java.
    #[allow(
        unused_mut,
        unused_variables,
        unused_parens,
        unused_assignments,
        unused_labels,
        clippy::all
    )]
    fn re_init_with_token_manager(&mut self, mut tm: StandardSyntaxParserTokenManager) {
        let this = self;
        this.jj_mut().token_source = tm;
        this.jj_mut().token = Some(TokenRef::new(Token::new()));
        this.jj_mut().jj_ntk = -1;
        this.jj_mut().jj_gen = 0;
        {
            let mut i: i32 = 0;
            'l1: loop {
                if !(i < 34) {
                    break;
                }
                'l1_body: {
                    let __i1 = (i) as usize;
                    this.jj_mut().jj_la1[__i1] = -1;
                }
                i = i.wrapping_add(1);
            }
        }
        {
            let mut i: i32 = 0;
            'l2: loop {
                if !(i < (this.jj().jj_2_rtns.len() as i32)) {
                    break;
                }
                'l2_body: {
                    let __i2 = (i) as usize;
                    this.jj_mut().jj_2_rtns[__i2] = Some(JJCallsRef::new(JJCalls::default()));
                }
                i = i.wrapping_add(1);
            }
        }
    }

    /// `getNextToken()` of the generated Java.
    #[allow(
        unused_mut,
        unused_variables,
        unused_parens,
        unused_assignments,
        unused_labels,
        clippy::all
    )]
    fn get_next_token(&mut self) -> Result<Option<TokenRef>> {
        let this = self;
        if (crate::javacc::nn(this.jj().token.as_ref())?.read(|o| o.next.clone())).is_some() {
            this.jj_mut().token =
                crate::javacc::nn(this.jj().token.as_ref())?.read(|o| o.next.clone());
        } else {
            let __o1 = this.jj().token.clone();
            let __a2 = this.jj_mut().token_source.get_next_token()?;
            crate::javacc::nn(__o1.as_ref())?.lock().next = __a2.clone();
            this.jj_mut().token = __a2;
        }
        this.jj_mut().jj_ntk = -1;
        this.jj_mut().jj_gen = this.jj().jj_gen.wrapping_add(1);
        return Ok(this.jj().token.clone());
    }

    /// `getToken(int)` of the generated Java.
    #[allow(
        unused_mut,
        unused_variables,
        unused_parens,
        unused_assignments,
        unused_labels,
        clippy::all
    )]
    fn get_token(&mut self, mut index: i32) -> Result<Option<TokenRef>> {
        let this = self;
        let mut t: Option<TokenRef> = this.jj().token.clone();
        {
            let mut i: i32 = 0;
            'l1: loop {
                if !(i < index) {
                    break;
                }
                'l1_body: {
                    if (crate::javacc::nn(t.as_ref())?.read(|o| o.next.clone())).is_some() {
                        t = crate::javacc::nn(t.as_ref())?.read(|o| o.next.clone());
                    } else {
                        let __o1 = t.clone();
                        let __a2 = this.jj_mut().token_source.get_next_token()?;
                        crate::javacc::nn(__o1.as_ref())?.lock().next = __a2.clone();
                        t = __a2;
                    }
                }
                i = i.wrapping_add(1);
            }
        }
        return Ok(t.clone());
    }

    /// `generateParseException()` of the generated Java.
    #[allow(
        unused_mut,
        unused_variables,
        unused_parens,
        unused_assignments,
        unused_labels,
        clippy::all
    )]
    fn generate_parse_exception(&mut self) -> Result<ParseException> {
        let this = self;
        this.jj_mut().jj_expentries.clear();
        let mut la1tokens: Vec<bool> = vec![false; (57) as usize];
        if this.jj().jj_kind >= 0 {
            let __i1 = (this.jj().jj_kind) as usize;
            la1tokens[__i1] = true;
            this.jj_mut().jj_kind = -1;
        }
        {
            let mut i: i32 = 0;
            'l1: loop {
                if !(i < 34) {
                    break;
                }
                'l1_body: {
                    if (this.jj().jj_la1[(i) as usize]) == this.jj().jj_gen {
                        {
                            let mut j: i32 = 0;
                            'l2: loop {
                                if !(j < 32) {
                                    break;
                                }
                                'l2_body: {
                                    if ((JJ_LA1_0[(i) as usize]) & (1i32.wrapping_shl((j) as u32)))
                                        != 0
                                    {
                                        let __i2 = (j) as usize;
                                        la1tokens[__i2] = true;
                                    }
                                    if ((JJ_LA1_1[(i) as usize]) & (1i32.wrapping_shl((j) as u32)))
                                        != 0
                                    {
                                        let __i3 = (32i32.wrapping_add(j)) as usize;
                                        la1tokens[__i3] = true;
                                    }
                                }
                                j = j.wrapping_add(1);
                            }
                        }
                    }
                }
                i = i.wrapping_add(1);
            }
        }
        {
            let mut i: i32 = 0;
            'l3: loop {
                if !(i < 57) {
                    break;
                }
                'l3_body: {
                    if la1tokens[(i) as usize] {
                        this.jj_mut().jj_expentry = Some(vec![0; (1) as usize]);
                        let __i4 = (0) as usize;
                        crate::javacc::nn_mut(this.jj_mut().jj_expentry.as_mut())?[__i4] = i;
                        let __p5 = crate::javacc::nn_owned(this.jj().jj_expentry.clone())?;
                        this.jj_mut().jj_expentries.push(__p5);
                    }
                }
                i = i.wrapping_add(1);
            }
        }
        this.jj_mut().jj_endpos = 0;
        jj_rescan_token(this)?;
        jj_add_error_token(this, 0, 0)?;
        let mut exptokseq: Vec<Vec<i32>> =
            vec![Vec::new(); (this.jj_mut().jj_expentries.len() as i32) as usize];
        {
            let mut i: i32 = 0;
            'l4: loop {
                if !(i < (this.jj_mut().jj_expentries.len() as i32)) {
                    break;
                }
                'l4_body: {
                    let __i6 = (i) as usize;
                    exptokseq[__i6] = this.jj_mut().jj_expentries[(i) as usize].clone();
                }
                i = i.wrapping_add(1);
            }
        }
        return Ok(ParseException::with_tokens(
            crate::javacc::nn(this.jj().token.clone().as_ref())?.clone(),
            exptokseq.clone(),
            &qpc::TOKEN_IMAGE,
        )?);
    }

    /// `trace_enabled()` of the generated Java.
    #[allow(
        unused_mut,
        unused_variables,
        unused_parens,
        unused_assignments,
        unused_labels,
        clippy::all
    )]
    fn trace_enabled(&mut self) -> bool {
        let this = self;
        return this.jj().trace_enabled;
    }

    /// `enable_tracing()` of the generated Java.
    #[allow(
        unused_mut,
        unused_variables,
        unused_parens,
        unused_assignments,
        unused_labels,
        clippy::all
    )]
    fn enable_tracing(&mut self) {
        let this = self;
    }

    /// `disable_tracing()` of the generated Java.
    #[allow(
        unused_mut,
        unused_variables,
        unused_parens,
        unused_assignments,
        unused_labels,
        clippy::all
    )]
    fn disable_tracing(&mut self) {
        let this = self;
    }
    // @generated end productions
}

// @generated begin state
/// The JavaCC state of the parser: the fields JavaCC generates.
///
/// **Divergence.** Java declares these fields on the parser class itself;
/// they are gathered here so that the parser's generated code reaches them
/// through [`StandardSyntaxParserOps::jj`] whatever type implements the trait.
#[derive(Debug)]
pub struct JjState {
    /// `token_source`.
    pub token_source: StandardSyntaxParserTokenManager,
    /// `token`.
    pub token: Option<TokenRef>,
    /// `jj_nt`.
    pub jj_nt: Option<TokenRef>,
    /// `jj_ntk`.
    pub(crate) jj_ntk: i32,
    /// `jj_scanpos`.
    pub(crate) jj_scanpos: Option<TokenRef>,
    /// `jj_lastpos`.
    pub(crate) jj_lastpos: Option<TokenRef>,
    /// `jj_la`.
    pub(crate) jj_la: i32,
    /// `jj_gen`.
    pub(crate) jj_gen: i32,
    /// `jj_la1`.
    pub(crate) jj_la1: Vec<i32>,
    /// `jj_2_rtns`.
    pub(crate) jj_2_rtns: Vec<Option<JJCallsRef>>,
    /// `jj_rescan`.
    pub(crate) jj_rescan: bool,
    /// `jj_gc`.
    pub(crate) jj_gc: i32,
    /// `jj_expentries`.
    pub(crate) jj_expentries: Vec<Vec<i32>>,
    /// `jj_expentry`.
    pub(crate) jj_expentry: Option<Vec<i32>>,
    /// `jj_kind`.
    pub(crate) jj_kind: i32,
    /// `jj_lasttokens`.
    pub(crate) jj_lasttokens: Vec<i32>,
    /// `jj_endpos`.
    pub(crate) jj_endpos: i32,
    /// `trace_enabled`.
    pub(crate) trace_enabled: bool,
}

/// `jj_la1_0`, as `jj_la1_init_0()` sets it.
static JJ_LA1_0: [i32; 34] = [
    1065368576, 512, 256, 9216, 13312, 13312, 98304, 595593216, 1065355264, 2097152, 0, 58722304,
    58722304, 58722304, 58722304, 58722304, 50331648, 58720256, 16777216, 50331648, 58720256,
    50331648, 58720256, 1966080, 58720256, 50331648, 4194304, 528482304, 2097152, 4194304,
    402653184, 0, 0, 0,
];

/// `jj_la1_1`, as `jj_la1_init_1()` sets it.
static JJ_LA1_1: [i32; 34] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 16777216, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 13107200, 13107200, 3145728,
];

/// `JJCalls`, a node of the list of the lookahead calls `jj_save`
/// records.
///
/// Equivalent to `org.apache.lucene.queryparser.flexible.standard.parser.StandardSyntaxParser.JJCalls`, generated by JavaCC.
#[derive(Debug, Default)]
pub(crate) struct JJCalls {
    /// `gen`.
    pub(crate) gen: i32,
    /// `first`.
    pub(crate) first: Option<TokenRef>,
    /// `arg`.
    pub(crate) arg: i32,
    /// `next`.
    pub(crate) next: Option<JJCallsRef>,
}

/// A reference to a [`JJCalls`]: Java's object reference.
pub(crate) type JJCallsRef = crate::javacc::Shared<JJCalls>;

/// The control-flow throwable of the lookahead: `jj_scan_token` throws it
/// when the lookahead limit is reached with every token matched, and
/// `jj_2_*` and `jj_rescan_token` catch it.
///
/// Equivalent to `org.apache.lucene.queryparser.flexible.standard.parser.StandardSyntaxParser.LookaheadSuccess`, generated by JavaCC (a
/// `java.lang.Error` without a stack trace, one shared instance `jj_ls`).
#[derive(Debug, Clone, Copy)]
pub(crate) struct LookaheadSuccess;

/// What a lookahead routine raises: Java's `throw jj_ls`, or any other
/// throwable, which passes through.
#[derive(Debug)]
pub(crate) enum Raised {
    /// `jj_ls`.
    LookaheadSuccess(LookaheadSuccess),
    /// Any other throwable.
    Throwable(LuceneError),
}

impl From<LuceneError> for Raised {
    fn from(e: LuceneError) -> Self {
        Raised::Throwable(e)
    }
}

impl JjState {
    /// The state `StandardSyntaxParser(CharStream)` initializes.
    #[allow(unused_mut, clippy::all)]
    pub fn with_char_stream(stream: Box<dyn CharStream>) -> Self {
        let mut s = Self {
            token_source: StandardSyntaxParserTokenManager::new(stream),
            token: None,
            jj_nt: None,
            jj_ntk: 0,
            jj_scanpos: None,
            jj_lastpos: None,
            jj_la: 0,
            jj_gen: 0,
            jj_la1: vec![0; (34) as usize],
            jj_2_rtns: vec![None; (28) as usize],
            jj_rescan: false,
            jj_gc: 0,
            jj_expentries: Vec::new(),
            jj_expentry: None,
            jj_kind: -1,
            jj_lasttokens: vec![0; (100) as usize],
            jj_endpos: 0,
            trace_enabled: false,
        };
        s.reset();
        s
    }
}

impl JjState {
    /// The state `StandardSyntaxParser(StandardSyntaxParserTokenManager)` initializes.
    #[allow(unused_mut, clippy::all)]
    pub fn with_token_manager(stream: StandardSyntaxParserTokenManager) -> Self {
        let mut s = Self {
            token_source: stream,
            token: None,
            jj_nt: None,
            jj_ntk: 0,
            jj_scanpos: None,
            jj_lastpos: None,
            jj_la: 0,
            jj_gen: 0,
            jj_la1: vec![0; (34) as usize],
            jj_2_rtns: vec![None; (28) as usize],
            jj_rescan: false,
            jj_gc: 0,
            jj_expentries: Vec::new(),
            jj_expentry: None,
            jj_kind: -1,
            jj_lasttokens: vec![0; (100) as usize],
            jj_endpos: 0,
            trace_enabled: false,
        };
        s.reset();
        s
    }
}

impl JjState {
    /// The statements the constructors and `ReInit` run after setting
    /// `token_source`.
    #[allow(unused_labels)]
    pub(crate) fn reset(&mut self) {
        self.token = Some(TokenRef::new(Token::new()));
        self.jj_ntk = -1;
        self.jj_gen = 0;
        {
            let mut i: i32 = 0;
            'l1: loop {
                if !(i < 34) {
                    break;
                }
                'l1_body: {
                    let __i1 = (i) as usize;
                    self.jj_la1[__i1] = -1;
                }
                i = i.wrapping_add(1);
            }
        }
        {
            let mut i: i32 = 0;
            'l2: loop {
                if !(i < (self.jj_2_rtns.len() as i32)) {
                    break;
                }
                'l2_body: {
                    let __i2 = (i) as usize;
                    self.jj_2_rtns[__i2] = Some(JJCallsRef::new(JJCalls::default()));
                }
                i = i.wrapping_add(1);
            }
        }
    }
}

// @generated end state

// @generated begin private
/// `TopLevelQuery(CharSequence)` of the generated Java.
#[allow(unreachable_code)]
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
pub(crate) fn top_level_query<P: StandardSyntaxParserOps + ?Sized>(
    this: &mut P,
    mut field: Option<CharSequence>,
) -> Result<Option<QueryNodeRef>> {
    let mut q: Option<QueryNodeRef> = None;
    q = query(this, field.clone())?;
    let _ = jj_consume_token(this, 0)?;
    {
        if true {
            return Ok(q.clone());
        }
    }
    return Err(
        crate::javacc::java_error("Missing return statement in function".to_string()).into(),
    );
}

/// `Query(CharSequence)` of the generated Java.
#[allow(unreachable_code)]
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn query<P: StandardSyntaxParserOps + ?Sized>(
    this: &mut P,
    mut field: Option<CharSequence>,
) -> Result<Option<QueryNodeRef>> {
    let mut clauses: Vec<Option<QueryNodeRef>> = Vec::new();
    let mut node: Option<QueryNodeRef> = None;
    'label_1: loop {
        node = disj_query(this, field.clone())?;
        clauses.push(node.clone());
        'sw1: {
            match if (this.jj().jj_ntk == -1) {
                jj_ntk_f(this)?
            } else {
                this.jj().jj_ntk
            } {
                qpc::NOT
                | qpc::FN_PREFIX
                | qpc::PLUS
                | qpc::MINUS
                | qpc::QUOTED
                | qpc::NUMBER
                | qpc::TERM
                | qpc::REGEXPTERM
                | qpc::RANGEIN_START
                | qpc::RANGEEX_START
                | qpc::LPAREN => {}
                _ => {
                    let __i1 = (0) as usize;
                    this.jj_mut().jj_la1[__i1] = this.jj().jj_gen;
                    break 'label_1;
                }
            }
        }
    }
    if (clauses.len() as i32) == 1 {
        let mut first: Option<QueryNodeRef> = clauses[(0) as usize].clone();
        if (is_instance(&first, &MODIFIER_QUERY_NODE_CLASS))
            && ((modifier_of(&(cast_node(first.clone(), &MODIFIER_QUERY_NODE_CLASS)?))?)
                == Modifier::ModNot)
        {
            let __p2 = Some(
                BooleanQueryNode::new(Some(query_nodes(vec![first.clone()])?))? as QueryNodeRef,
            );
            let _ = std::mem::replace(&mut clauses[(0) as usize], __p2);
        }
    }
    {
        if true {
            return Ok(if (clauses.len() as i32) == 1 {
                clauses[(0) as usize].clone()
            } else {
                Some(BooleanQueryNode::new(Some(query_nodes(clauses.clone())?))? as QueryNodeRef)
            });
        }
    }
    return Err(
        crate::javacc::java_error("Missing return statement in function".to_string()).into(),
    );
}

/// `DisjQuery(CharSequence)` of the generated Java.
#[allow(unreachable_code)]
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn disj_query<P: StandardSyntaxParserOps + ?Sized>(
    this: &mut P,
    mut field: Option<CharSequence>,
) -> Result<Option<QueryNodeRef>> {
    let mut clauses: Vec<Option<QueryNodeRef>> = Vec::new();
    let mut node: Option<QueryNodeRef> = None;
    node = conj_query(this, field.clone())?;
    clauses.push(node.clone());
    'label_2: loop {
        'sw1: {
            match if (this.jj().jj_ntk == -1) {
                jj_ntk_f(this)?
            } else {
                this.jj().jj_ntk
            } {
                qpc::OR => {}
                _ => {
                    let __i1 = (1) as usize;
                    this.jj_mut().jj_la1[__i1] = this.jj().jj_gen;
                    break 'label_2;
                }
            }
        }
        let _ = jj_consume_token(this, qpc::OR)?;
        node = conj_query(this, field.clone())?;
        clauses.push(node.clone());
    }
    {
        if true {
            return Ok(if (clauses.len() as i32) == 1 {
                clauses[(0) as usize].clone()
            } else {
                Some(OrQueryNode::new(Some(query_nodes(clauses.clone())?))? as QueryNodeRef)
            });
        }
    }
    return Err(
        crate::javacc::java_error("Missing return statement in function".to_string()).into(),
    );
}

/// `ConjQuery(CharSequence)` of the generated Java.
#[allow(unreachable_code)]
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn conj_query<P: StandardSyntaxParserOps + ?Sized>(
    this: &mut P,
    mut field: Option<CharSequence>,
) -> Result<Option<QueryNodeRef>> {
    let mut clauses: Vec<Option<QueryNodeRef>> = Vec::new();
    let mut node: Option<QueryNodeRef> = None;
    node = mod_clause(this, field.clone())?;
    clauses.push(node.clone());
    'label_3: loop {
        'sw1: {
            match if (this.jj().jj_ntk == -1) {
                jj_ntk_f(this)?
            } else {
                this.jj().jj_ntk
            } {
                qpc::AND => {}
                _ => {
                    let __i1 = (2) as usize;
                    this.jj_mut().jj_la1[__i1] = this.jj().jj_gen;
                    break 'label_3;
                }
            }
        }
        let _ = jj_consume_token(this, qpc::AND)?;
        node = mod_clause(this, field.clone())?;
        clauses.push(node.clone());
    }
    {
        if true {
            return Ok(if (clauses.len() as i32) == 1 {
                clauses[(0) as usize].clone()
            } else {
                Some(AndQueryNode::new(Some(query_nodes(clauses.clone())?))? as QueryNodeRef)
            });
        }
    }
    return Err(
        crate::javacc::java_error("Missing return statement in function".to_string()).into(),
    );
}

/// `ModClause(CharSequence)` of the generated Java.
#[allow(unreachable_code)]
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn mod_clause<P: StandardSyntaxParserOps + ?Sized>(
    this: &mut P,
    mut field: Option<CharSequence>,
) -> Result<Option<QueryNodeRef>> {
    let mut q: Option<QueryNodeRef> = None;
    let mut modifier: Modifier = Modifier::ModNone;
    'sw1: {
        match if (this.jj().jj_ntk == -1) {
            jj_ntk_f(this)?
        } else {
            this.jj().jj_ntk
        } {
            qpc::NOT | qpc::PLUS | qpc::MINUS => 'sw2: {
                match if (this.jj().jj_ntk == -1) {
                    jj_ntk_f(this)?
                } else {
                    this.jj().jj_ntk
                } {
                    qpc::PLUS => {
                        let _ = jj_consume_token(this, qpc::PLUS)?;
                        modifier = Modifier::ModReq;
                    }
                    qpc::NOT | qpc::MINUS => {
                        'sw3: {
                            match if (this.jj().jj_ntk == -1) {
                                jj_ntk_f(this)?
                            } else {
                                this.jj().jj_ntk
                            } {
                                qpc::MINUS => {
                                    let _ = jj_consume_token(this, qpc::MINUS)?;
                                }
                                qpc::NOT => {
                                    let _ = jj_consume_token(this, qpc::NOT)?;
                                }
                                _ => {
                                    let __i1 = (3) as usize;
                                    this.jj_mut().jj_la1[__i1] = this.jj().jj_gen;
                                    let _ = jj_consume_token(this, -1)?;
                                    return Err(ParseException::new().error().into());
                                }
                            }
                        }
                        modifier = Modifier::ModNot;
                    }
                    _ => {
                        let __i2 = (4) as usize;
                        this.jj_mut().jj_la1[__i2] = this.jj().jj_gen;
                        let _ = jj_consume_token(this, -1)?;
                        return Err(ParseException::new().error().into());
                    }
                }
            }
            _ => {
                let __i3 = (5) as usize;
                this.jj_mut().jj_la1[__i3] = this.jj().jj_gen;
            }
        }
    }
    q = clause(this, field.clone())?;
    if modifier.clone() != Modifier::ModNone {
        q = Some(ModifierQueryNode::new(q.clone(), modifier.clone())? as QueryNodeRef);
    }
    {
        if true {
            return Ok(q.clone());
        }
    }
    return Err(
        crate::javacc::java_error("Missing return statement in function".to_string()).into(),
    );
}

/// `Clause(CharSequence)` of the generated Java.
#[allow(unreachable_code)]
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn clause<P: StandardSyntaxParserOps + ?Sized>(
    this: &mut P,
    mut field: Option<CharSequence>,
) -> Result<Option<QueryNodeRef>> {
    let mut q: Option<QueryNodeRef> = None;
    if jj_2_3(this, 2)? {
        q = field_range_expr(this, field.clone())?;
    } else {
        'sw1: {
            match if (this.jj().jj_ntk == -1) {
                jj_ntk_f(this)?
            } else {
                this.jj().jj_ntk
            } {
                qpc::FN_PREFIX
                | qpc::QUOTED
                | qpc::NUMBER
                | qpc::TERM
                | qpc::REGEXPTERM
                | qpc::RANGEIN_START
                | qpc::RANGEEX_START
                | qpc::LPAREN => {
                    if jj_2_1(this, 2)? {
                        field = field_name(this)?;
                        'sw2: {
                            match if (this.jj().jj_ntk == -1) {
                                jj_ntk_f(this)?
                            } else {
                                this.jj().jj_ntk
                            } {
                                qpc::OP_COLON => {
                                    let _ = jj_consume_token(this, qpc::OP_COLON)?;
                                }
                                qpc::OP_EQUAL => {
                                    let _ = jj_consume_token(this, qpc::OP_EQUAL)?;
                                }
                                _ => {
                                    let __i1 = (6) as usize;
                                    this.jj_mut().jj_la1[__i1] = this.jj().jj_gen;
                                    let _ = jj_consume_token(this, -1)?;
                                    return Err(ParseException::new().error().into());
                                }
                            }
                        }
                    } else {
                    }
                    if jj_2_2(this, 2)? {
                        q = term(this, field.clone())?;
                    } else {
                        'sw3: {
                            match if (this.jj().jj_ntk == -1) {
                                jj_ntk_f(this)?
                            } else {
                                this.jj().jj_ntk
                            } {
                                qpc::LPAREN => {
                                    q = grouping_expr(this, field.clone())?;
                                }
                                qpc::FN_PREFIX | qpc::QUOTED | qpc::NUMBER | qpc::TERM => {
                                    q = interval_expr(this, field.clone())?;
                                }
                                _ => {
                                    let __i2 = (7) as usize;
                                    this.jj_mut().jj_la1[__i2] = this.jj().jj_gen;
                                    let _ = jj_consume_token(this, -1)?;
                                    return Err(ParseException::new().error().into());
                                }
                            }
                        }
                    }
                }
                _ => {
                    let __i3 = (8) as usize;
                    this.jj_mut().jj_la1[__i3] = this.jj().jj_gen;
                    let _ = jj_consume_token(this, -1)?;
                    return Err(ParseException::new().error().into());
                }
            }
        }
    }
    {
        if true {
            return Ok(q.clone());
        }
    }
    return Err(
        crate::javacc::java_error("Missing return statement in function".to_string()).into(),
    );
}

/// `FieldName()` of the generated Java.
#[allow(unreachable_code)]
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn field_name<P: StandardSyntaxParserOps + ?Sized>(this: &mut P) -> Result<Option<CharSequence>> {
    let mut name: Option<TokenRef> = None;
    name = jj_consume_token(this, qpc::TERM)?;
    {
        if true {
            return Ok(Some(
                EscapeQuerySyntaxImpl::discard_escape_char(&CharSequence::from(
                    crate::javacc::nn_str(
                        crate::javacc::nn(name.as_ref())?
                            .read(|o| o.image.clone())
                            .as_deref(),
                    )?
                    .to_string(),
                ))?
                .into_char_sequence(),
            ));
        }
    }
    return Err(
        crate::javacc::java_error("Missing return statement in function".to_string()).into(),
    );
}

/// `GroupingExpr(CharSequence)` of the generated Java.
#[allow(unreachable_code)]
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn grouping_expr<P: StandardSyntaxParserOps + ?Sized>(
    this: &mut P,
    mut field: Option<CharSequence>,
) -> Result<Option<QueryNodeRef>> {
    let mut q: Option<QueryNodeRef> = None;
    let mut boost: Option<TokenRef> = None;
    let mut min_should_match: Option<TokenRef> = None;
    let _ = jj_consume_token(this, qpc::LPAREN)?;
    q = query(this, field.clone())?;
    let _ = jj_consume_token(this, qpc::RPAREN)?;
    'sw1: {
        match if (this.jj().jj_ntk == -1) {
            jj_ntk_f(this)?
        } else {
            this.jj().jj_ntk
        } {
            qpc::CARAT => {
                q = self::boost(this, q.clone())?;
            }
            _ => {
                let __i1 = (9) as usize;
                this.jj_mut().jj_la1[__i1] = this.jj().jj_gen;
            }
        }
    }
    'sw2: {
        match if (this.jj().jj_ntk == -1) {
            jj_ntk_f(this)?
        } else {
            this.jj().jj_ntk
        } {
            56 => {
                let _ = jj_consume_token(this, 56)?;
                min_should_match = jj_consume_token(this, qpc::NUMBER)?;
            }
            _ => {
                let __i2 = (10) as usize;
                this.jj_mut().jj_la1[__i2] = this.jj().jj_gen;
            }
        }
    }
    if min_should_match.clone().is_some() {
        q = Some(MinShouldMatchNode::new(
            StandardSyntaxParser::parse_int(&crate::javacc::nn_owned(min_should_match.clone())?)?,
            Some(GroupQueryNode::new(q.clone())?),
        )? as QueryNodeRef);
    } else {
        q = Some(GroupQueryNode::new(q.clone())?).map(|n| n as QueryNodeRef);
    }
    {
        if true {
            return Ok(q.clone());
        }
    }
    return Err(
        crate::javacc::java_error("Missing return statement in function".to_string()).into(),
    );
}

/// `IntervalExpr(CharSequence)` of the generated Java.
#[allow(unreachable_code)]
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn interval_expr<P: StandardSyntaxParserOps + ?Sized>(
    this: &mut P,
    mut field: Option<CharSequence>,
) -> Result<Option<QueryNodeRef>> {
    let mut source: Option<Arc<dyn IntervalFunction>> = None;
    source = interval_fun(this)?;
    {
        if true {
            return Ok(Some(IntervalQueryNode::new(
                if field.clone().is_none() {
                    None
                } else {
                    Some(crate::javacc::nn(field.as_ref())?.to_string())
                },
                source.clone(),
            )? as QueryNodeRef));
        }
    }
    return Err(
        crate::javacc::java_error("Missing return statement in function".to_string()).into(),
    );
}

/// `IntervalFun()` of the generated Java.
#[allow(unreachable_code)]
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn interval_fun<P: StandardSyntaxParserOps + ?Sized>(
    this: &mut P,
) -> Result<Option<Arc<dyn IntervalFunction>>> {
    let mut source: Option<Arc<dyn IntervalFunction>> = None;
    if jj_2_4(this, 2)? {
        source = interval_at_least(this)?;
        {
            if true {
                return Ok(source.clone());
            }
        }
    } else {
        if jj_2_5(this, 2)? {
            source = interval_max_width(this)?;
            {
                if true {
                    return Ok(source.clone());
                }
            }
        } else {
            if jj_2_6(this, 2)? {
                source = interval_max_gaps(this)?;
                {
                    if true {
                        return Ok(source.clone());
                    }
                }
            } else {
                if jj_2_7(this, 2)? {
                    source = interval_ordered(this)?;
                    {
                        if true {
                            return Ok(source.clone());
                        }
                    }
                } else {
                    if jj_2_8(this, 2)? {
                        source = interval_unordered(this)?;
                        {
                            if true {
                                return Ok(source.clone());
                            }
                        }
                    } else {
                        if jj_2_9(this, 2)? {
                            source = interval_unordered_no_overlaps(this)?;
                            {
                                if true {
                                    return Ok(source.clone());
                                }
                            }
                        } else {
                            if jj_2_10(this, 2)? {
                                source = interval_or(this)?;
                                {
                                    if true {
                                        return Ok(source.clone());
                                    }
                                }
                            } else {
                                if jj_2_11(this, 2)? {
                                    source = interval_wildcard(this)?;
                                    {
                                        if true {
                                            return Ok(source.clone());
                                        }
                                    }
                                } else {
                                    if jj_2_12(this, 2)? {
                                        source = interval_after(this)?;
                                        {
                                            if true {
                                                return Ok(source.clone());
                                            }
                                        }
                                    } else {
                                        if jj_2_13(this, 2)? {
                                            source = interval_before(this)?;
                                            {
                                                if true {
                                                    return Ok(source.clone());
                                                }
                                            }
                                        } else {
                                            if jj_2_14(this, 2)? {
                                                source = interval_phrase(this)?;
                                                {
                                                    if true {
                                                        return Ok(source.clone());
                                                    }
                                                }
                                            } else {
                                                if jj_2_15(this, 2)? {
                                                    source = interval_containing(this)?;
                                                    {
                                                        if true {
                                                            return Ok(source.clone());
                                                        }
                                                    }
                                                } else {
                                                    if jj_2_16(this, 2)? {
                                                        source = interval_not_containing(this)?;
                                                        {
                                                            if true {
                                                                return Ok(source.clone());
                                                            }
                                                        }
                                                    } else {
                                                        if jj_2_17(this, 2)? {
                                                            source = interval_contained_by(this)?;
                                                            {
                                                                if true {
                                                                    return Ok(source.clone());
                                                                }
                                                            }
                                                        } else {
                                                            if jj_2_18(this, 2)? {
                                                                source = interval_not_contained_by(
                                                                    this,
                                                                )?;
                                                                {
                                                                    if true {
                                                                        return Ok(source.clone());
                                                                    }
                                                                }
                                                            } else {
                                                                if jj_2_19(this, 2)? {
                                                                    source = interval_within(this)?;
                                                                    {
                                                                        if true {
                                                                            return Ok(
                                                                                source.clone()
                                                                            );
                                                                        }
                                                                    }
                                                                } else {
                                                                    if jj_2_20(this, 2)? {
                                                                        source =
                                                                            interval_not_within(
                                                                                this,
                                                                            )?;
                                                                        {
                                                                            if true {
                                                                                return Ok(
                                                                                    source.clone()
                                                                                );
                                                                            }
                                                                        }
                                                                    } else {
                                                                        if jj_2_21(this, 2)? {
                                                                            source = interval_overlapping(this)?;
                                                                            {
                                                                                if true {
                                                                                    return Ok(
                                                                                        source
                                                                                            .clone(
                                                                                            ),
                                                                                    );
                                                                                }
                                                                            }
                                                                        } else {
                                                                            if jj_2_22(this, 2)? {
                                                                                source = interval_non_overlapping(this)?;
                                                                                {
                                                                                    if true {
                                                                                        return Ok(source.clone());
                                                                                    }
                                                                                }
                                                                            } else {
                                                                                if jj_2_23(this, 2)?
                                                                                {
                                                                                    source = interval_extend(this)?;
                                                                                    {
                                                                                        if true {
                                                                                            return Ok(source.clone());
                                                                                        }
                                                                                    }
                                                                                } else {
                                                                                    if jj_2_24(
                                                                                        this, 2,
                                                                                    )? {
                                                                                        source = interval_fuzzy_term(this)?;
                                                                                        {
                                                                                            if true
                                                                                            {
                                                                                                return Ok(source.clone());
                                                                                            }
                                                                                        }
                                                                                    } else {
                                                                                        if jj_2_25(
                                                                                            this, 2,
                                                                                        )? {
                                                                                            source = interval_text(this)?;
                                                                                            {
                                                                                                if true {
                                                                                                    return Ok(source.clone());
                                                                                                }
                                                                                            }
                                                                                        } else {
                                                                                            let _ = jj_consume_token(this, -1)?;
                                                                                            return Err(ParseException::new().error().into());
                                                                                        }
                                                                                    }
                                                                                }
                                                                            }
                                                                        }
                                                                    }
                                                                }
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    return Err(
        crate::javacc::java_error("Missing return statement in function".to_string()).into(),
    );
}

/// `IntervalAtLeast()` of the generated Java.
#[allow(unreachable_code)]
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn interval_at_least<P: StandardSyntaxParserOps + ?Sized>(
    this: &mut P,
) -> Result<Option<Arc<dyn IntervalFunction>>> {
    let mut source: Option<Arc<dyn IntervalFunction>> = None;
    let mut sources: Vec<Option<Arc<dyn IntervalFunction>>> = Vec::new();
    let mut min_should_match: Option<TokenRef> = None;
    let _ = jj_consume_token(this, qpc::FN_PREFIX)?;
    let _ = jj_consume_token(this, qpc::ATLEAST)?;
    let _ = jj_consume_token(this, qpc::LPAREN)?;
    min_should_match = jj_consume_token(this, qpc::NUMBER)?;
    'label_4: loop {
        source = interval_fun(this)?;
        sources.push(source.clone());
        'sw1: {
            match if (this.jj().jj_ntk == -1) {
                jj_ntk_f(this)?
            } else {
                this.jj().jj_ntk
            } {
                qpc::FN_PREFIX | qpc::QUOTED | qpc::NUMBER | qpc::TERM => {}
                _ => {
                    let __i1 = (11) as usize;
                    this.jj_mut().jj_la1[__i1] = this.jj().jj_gen;
                    break 'label_4;
                }
            }
        }
    }
    let _ = jj_consume_token(this, qpc::RPAREN)?;
    {
        if true {
            return Ok(Some(Arc::new(AtLeast::new(
                StandardSyntaxParser::parse_int(&crate::javacc::nn_owned(
                    min_should_match.clone(),
                )?)?,
                Some(interval_functions(sources.clone())?),
            )?) as Arc<dyn IntervalFunction>));
        }
    }
    return Err(
        crate::javacc::java_error("Missing return statement in function".to_string()).into(),
    );
}

/// `IntervalMaxWidth()` of the generated Java.
#[allow(unreachable_code)]
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn interval_max_width<P: StandardSyntaxParserOps + ?Sized>(
    this: &mut P,
) -> Result<Option<Arc<dyn IntervalFunction>>> {
    let mut source: Option<Arc<dyn IntervalFunction>> = None;
    let mut max_width: Option<TokenRef> = None;
    let _ = jj_consume_token(this, qpc::FN_PREFIX)?;
    let _ = jj_consume_token(this, qpc::MAXWIDTH)?;
    let _ = jj_consume_token(this, qpc::LPAREN)?;
    max_width = jj_consume_token(this, qpc::NUMBER)?;
    source = interval_fun(this)?;
    let _ = jj_consume_token(this, qpc::RPAREN)?;
    {
        if true {
            return Ok(Some(Arc::new(MaxWidth::new(
                StandardSyntaxParser::parse_int(&crate::javacc::nn_owned(max_width.clone())?)?,
                source.clone(),
            )?) as Arc<dyn IntervalFunction>));
        }
    }
    return Err(
        crate::javacc::java_error("Missing return statement in function".to_string()).into(),
    );
}

/// `IntervalMaxGaps()` of the generated Java.
#[allow(unreachable_code)]
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn interval_max_gaps<P: StandardSyntaxParserOps + ?Sized>(
    this: &mut P,
) -> Result<Option<Arc<dyn IntervalFunction>>> {
    let mut source: Option<Arc<dyn IntervalFunction>> = None;
    let mut max_gaps: Option<TokenRef> = None;
    let _ = jj_consume_token(this, qpc::FN_PREFIX)?;
    let _ = jj_consume_token(this, qpc::MAXGAPS)?;
    let _ = jj_consume_token(this, qpc::LPAREN)?;
    max_gaps = jj_consume_token(this, qpc::NUMBER)?;
    source = interval_fun(this)?;
    let _ = jj_consume_token(this, qpc::RPAREN)?;
    {
        if true {
            return Ok(Some(Arc::new(MaxGaps::new(
                StandardSyntaxParser::parse_int(&crate::javacc::nn_owned(max_gaps.clone())?)?,
                source.clone(),
            )?) as Arc<dyn IntervalFunction>));
        }
    }
    return Err(
        crate::javacc::java_error("Missing return statement in function".to_string()).into(),
    );
}

/// `IntervalUnordered()` of the generated Java.
#[allow(unreachable_code)]
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn interval_unordered<P: StandardSyntaxParserOps + ?Sized>(
    this: &mut P,
) -> Result<Option<Arc<dyn IntervalFunction>>> {
    let mut source: Option<Arc<dyn IntervalFunction>> = None;
    let mut sources: Vec<Option<Arc<dyn IntervalFunction>>> = Vec::new();
    let _ = jj_consume_token(this, qpc::FN_PREFIX)?;
    let _ = jj_consume_token(this, qpc::UNORDERED)?;
    let _ = jj_consume_token(this, qpc::LPAREN)?;
    'label_5: loop {
        source = interval_fun(this)?;
        sources.push(source.clone());
        'sw1: {
            match if (this.jj().jj_ntk == -1) {
                jj_ntk_f(this)?
            } else {
                this.jj().jj_ntk
            } {
                qpc::FN_PREFIX | qpc::QUOTED | qpc::NUMBER | qpc::TERM => {}
                _ => {
                    let __i1 = (12) as usize;
                    this.jj_mut().jj_la1[__i1] = this.jj().jj_gen;
                    break 'label_5;
                }
            }
        }
    }
    let _ = jj_consume_token(this, qpc::RPAREN)?;
    {
        if true {
            return Ok(Some(
                Arc::new(Unordered::new(Some(interval_functions(sources.clone())?))?)
                    as Arc<dyn IntervalFunction>,
            ));
        }
    }
    return Err(
        crate::javacc::java_error("Missing return statement in function".to_string()).into(),
    );
}

/// `IntervalUnorderedNoOverlaps()` of the generated Java.
#[allow(unreachable_code)]
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn interval_unordered_no_overlaps<P: StandardSyntaxParserOps + ?Sized>(
    this: &mut P,
) -> Result<Option<Arc<dyn IntervalFunction>>> {
    let mut a: Option<Arc<dyn IntervalFunction>> = None;
    let mut b: Option<Arc<dyn IntervalFunction>> = None;
    let _ = jj_consume_token(this, qpc::FN_PREFIX)?;
    let _ = jj_consume_token(this, qpc::UNORDERED_NO_OVERLAPS)?;
    let _ = jj_consume_token(this, qpc::LPAREN)?;
    a = interval_fun(this)?;
    b = interval_fun(this)?;
    let _ = jj_consume_token(this, qpc::RPAREN)?;
    {
        if true {
            return Ok(Some(
                Arc::new(UnorderedNoOverlaps::new(a.clone(), b.clone())?)
                    as Arc<dyn IntervalFunction>,
            ));
        }
    }
    return Err(
        crate::javacc::java_error("Missing return statement in function".to_string()).into(),
    );
}

/// `IntervalOrdered()` of the generated Java.
#[allow(unreachable_code)]
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn interval_ordered<P: StandardSyntaxParserOps + ?Sized>(
    this: &mut P,
) -> Result<Option<Arc<dyn IntervalFunction>>> {
    let mut source: Option<Arc<dyn IntervalFunction>> = None;
    let mut sources: Vec<Option<Arc<dyn IntervalFunction>>> = Vec::new();
    let _ = jj_consume_token(this, qpc::FN_PREFIX)?;
    let _ = jj_consume_token(this, qpc::ORDERED)?;
    let _ = jj_consume_token(this, qpc::LPAREN)?;
    'label_6: loop {
        source = interval_fun(this)?;
        sources.push(source.clone());
        'sw1: {
            match if (this.jj().jj_ntk == -1) {
                jj_ntk_f(this)?
            } else {
                this.jj().jj_ntk
            } {
                qpc::FN_PREFIX | qpc::QUOTED | qpc::NUMBER | qpc::TERM => {}
                _ => {
                    let __i1 = (13) as usize;
                    this.jj_mut().jj_la1[__i1] = this.jj().jj_gen;
                    break 'label_6;
                }
            }
        }
    }
    let _ = jj_consume_token(this, qpc::RPAREN)?;
    {
        if true {
            return Ok(Some(
                Arc::new(Ordered::new(Some(interval_functions(sources.clone())?))?)
                    as Arc<dyn IntervalFunction>,
            ));
        }
    }
    return Err(
        crate::javacc::java_error("Missing return statement in function".to_string()).into(),
    );
}

/// `IntervalOr()` of the generated Java.
#[allow(unreachable_code)]
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn interval_or<P: StandardSyntaxParserOps + ?Sized>(
    this: &mut P,
) -> Result<Option<Arc<dyn IntervalFunction>>> {
    let mut source: Option<Arc<dyn IntervalFunction>> = None;
    let mut sources: Vec<Option<Arc<dyn IntervalFunction>>> = Vec::new();
    let _ = jj_consume_token(this, qpc::FN_PREFIX)?;
    let _ = jj_consume_token(this, qpc::FN_OR)?;
    let _ = jj_consume_token(this, qpc::LPAREN)?;
    'label_7: loop {
        source = interval_fun(this)?;
        sources.push(source.clone());
        'sw1: {
            match if (this.jj().jj_ntk == -1) {
                jj_ntk_f(this)?
            } else {
                this.jj().jj_ntk
            } {
                qpc::FN_PREFIX | qpc::QUOTED | qpc::NUMBER | qpc::TERM => {}
                _ => {
                    let __i1 = (14) as usize;
                    this.jj_mut().jj_la1[__i1] = this.jj().jj_gen;
                    break 'label_7;
                }
            }
        }
    }
    let _ = jj_consume_token(this, qpc::RPAREN)?;
    {
        if true {
            return Ok(Some(
                Arc::new(Or::new(Some(interval_functions(sources.clone())?))?)
                    as Arc<dyn IntervalFunction>,
            ));
        }
    }
    return Err(
        crate::javacc::java_error("Missing return statement in function".to_string()).into(),
    );
}

/// `IntervalPhrase()` of the generated Java.
#[allow(unreachable_code)]
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn interval_phrase<P: StandardSyntaxParserOps + ?Sized>(
    this: &mut P,
) -> Result<Option<Arc<dyn IntervalFunction>>> {
    let mut source: Option<Arc<dyn IntervalFunction>> = None;
    let mut sources: Vec<Option<Arc<dyn IntervalFunction>>> = Vec::new();
    let _ = jj_consume_token(this, qpc::FN_PREFIX)?;
    let _ = jj_consume_token(this, qpc::PHRASE)?;
    let _ = jj_consume_token(this, qpc::LPAREN)?;
    'label_8: loop {
        source = interval_fun(this)?;
        sources.push(source.clone());
        'sw1: {
            match if (this.jj().jj_ntk == -1) {
                jj_ntk_f(this)?
            } else {
                this.jj().jj_ntk
            } {
                qpc::FN_PREFIX | qpc::QUOTED | qpc::NUMBER | qpc::TERM => {}
                _ => {
                    let __i1 = (15) as usize;
                    this.jj_mut().jj_la1[__i1] = this.jj().jj_gen;
                    break 'label_8;
                }
            }
        }
    }
    let _ = jj_consume_token(this, qpc::RPAREN)?;
    {
        if true {
            return Ok(Some(
                Arc::new(Phrase::new(Some(interval_functions(sources.clone())?))?)
                    as Arc<dyn IntervalFunction>,
            ));
        }
    }
    return Err(
        crate::javacc::java_error("Missing return statement in function".to_string()).into(),
    );
}

/// `IntervalBefore()` of the generated Java.
#[allow(unreachable_code)]
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn interval_before<P: StandardSyntaxParserOps + ?Sized>(
    this: &mut P,
) -> Result<Option<Arc<dyn IntervalFunction>>> {
    let mut source: Option<Arc<dyn IntervalFunction>> = None;
    let mut reference: Option<Arc<dyn IntervalFunction>> = None;
    let _ = jj_consume_token(this, qpc::FN_PREFIX)?;
    let _ = jj_consume_token(this, qpc::BEFORE)?;
    let _ = jj_consume_token(this, qpc::LPAREN)?;
    source = interval_fun(this)?;
    reference = interval_fun(this)?;
    let _ = jj_consume_token(this, qpc::RPAREN)?;
    {
        if true {
            return Ok(Some(
                Arc::new(Before::new(source.clone(), reference.clone())?)
                    as Arc<dyn IntervalFunction>,
            ));
        }
    }
    return Err(
        crate::javacc::java_error("Missing return statement in function".to_string()).into(),
    );
}

/// `IntervalAfter()` of the generated Java.
#[allow(unreachable_code)]
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn interval_after<P: StandardSyntaxParserOps + ?Sized>(
    this: &mut P,
) -> Result<Option<Arc<dyn IntervalFunction>>> {
    let mut source: Option<Arc<dyn IntervalFunction>> = None;
    let mut reference: Option<Arc<dyn IntervalFunction>> = None;
    let _ = jj_consume_token(this, qpc::FN_PREFIX)?;
    let _ = jj_consume_token(this, qpc::AFTER)?;
    let _ = jj_consume_token(this, qpc::LPAREN)?;
    source = interval_fun(this)?;
    reference = interval_fun(this)?;
    let _ = jj_consume_token(this, qpc::RPAREN)?;
    {
        if true {
            return Ok(Some(
                Arc::new(After::new(source.clone(), reference.clone())?)
                    as Arc<dyn IntervalFunction>,
            ));
        }
    }
    return Err(
        crate::javacc::java_error("Missing return statement in function".to_string()).into(),
    );
}

/// `IntervalContaining()` of the generated Java.
#[allow(unreachable_code)]
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn interval_containing<P: StandardSyntaxParserOps + ?Sized>(
    this: &mut P,
) -> Result<Option<Arc<dyn IntervalFunction>>> {
    let mut big: Option<Arc<dyn IntervalFunction>> = None;
    let mut small: Option<Arc<dyn IntervalFunction>> = None;
    let _ = jj_consume_token(this, qpc::FN_PREFIX)?;
    let _ = jj_consume_token(this, qpc::CONTAINING)?;
    let _ = jj_consume_token(this, qpc::LPAREN)?;
    big = interval_fun(this)?;
    small = interval_fun(this)?;
    let _ = jj_consume_token(this, qpc::RPAREN)?;
    {
        if true {
            return Ok(Some(
                Arc::new(Containing::new(big.clone(), small.clone())?) as Arc<dyn IntervalFunction>
            ));
        }
    }
    return Err(
        crate::javacc::java_error("Missing return statement in function".to_string()).into(),
    );
}

/// `IntervalNotContaining()` of the generated Java.
#[allow(unreachable_code)]
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn interval_not_containing<P: StandardSyntaxParserOps + ?Sized>(
    this: &mut P,
) -> Result<Option<Arc<dyn IntervalFunction>>> {
    let mut minuend: Option<Arc<dyn IntervalFunction>> = None;
    let mut subtrahend: Option<Arc<dyn IntervalFunction>> = None;
    let _ = jj_consume_token(this, qpc::FN_PREFIX)?;
    let _ = jj_consume_token(this, qpc::NOT_CONTAINING)?;
    let _ = jj_consume_token(this, qpc::LPAREN)?;
    minuend = interval_fun(this)?;
    subtrahend = interval_fun(this)?;
    let _ = jj_consume_token(this, qpc::RPAREN)?;
    {
        if true {
            return Ok(Some(
                Arc::new(NotContaining::new(minuend.clone(), subtrahend.clone())?)
                    as Arc<dyn IntervalFunction>,
            ));
        }
    }
    return Err(
        crate::javacc::java_error("Missing return statement in function".to_string()).into(),
    );
}

/// `IntervalContainedBy()` of the generated Java.
#[allow(unreachable_code)]
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn interval_contained_by<P: StandardSyntaxParserOps + ?Sized>(
    this: &mut P,
) -> Result<Option<Arc<dyn IntervalFunction>>> {
    let mut big: Option<Arc<dyn IntervalFunction>> = None;
    let mut small: Option<Arc<dyn IntervalFunction>> = None;
    let _ = jj_consume_token(this, qpc::FN_PREFIX)?;
    let _ = jj_consume_token(this, qpc::CONTAINED_BY)?;
    let _ = jj_consume_token(this, qpc::LPAREN)?;
    small = interval_fun(this)?;
    big = interval_fun(this)?;
    let _ = jj_consume_token(this, qpc::RPAREN)?;
    {
        if true {
            return Ok(Some(
                Arc::new(ContainedBy::new(small.clone(), big.clone())?)
                    as Arc<dyn IntervalFunction>,
            ));
        }
    }
    return Err(
        crate::javacc::java_error("Missing return statement in function".to_string()).into(),
    );
}

/// `IntervalNotContainedBy()` of the generated Java.
#[allow(unreachable_code)]
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn interval_not_contained_by<P: StandardSyntaxParserOps + ?Sized>(
    this: &mut P,
) -> Result<Option<Arc<dyn IntervalFunction>>> {
    let mut big: Option<Arc<dyn IntervalFunction>> = None;
    let mut small: Option<Arc<dyn IntervalFunction>> = None;
    let _ = jj_consume_token(this, qpc::FN_PREFIX)?;
    let _ = jj_consume_token(this, qpc::NOT_CONTAINED_BY)?;
    let _ = jj_consume_token(this, qpc::LPAREN)?;
    small = interval_fun(this)?;
    big = interval_fun(this)?;
    let _ = jj_consume_token(this, qpc::RPAREN)?;
    {
        if true {
            return Ok(Some(
                Arc::new(NotContainedBy::new(small.clone(), big.clone())?)
                    as Arc<dyn IntervalFunction>,
            ));
        }
    }
    return Err(
        crate::javacc::java_error("Missing return statement in function".to_string()).into(),
    );
}

/// `IntervalWithin()` of the generated Java.
#[allow(unreachable_code)]
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn interval_within<P: StandardSyntaxParserOps + ?Sized>(
    this: &mut P,
) -> Result<Option<Arc<dyn IntervalFunction>>> {
    let mut source: Option<Arc<dyn IntervalFunction>> = None;
    let mut reference: Option<Arc<dyn IntervalFunction>> = None;
    let mut positions: Option<TokenRef> = None;
    let _ = jj_consume_token(this, qpc::FN_PREFIX)?;
    let _ = jj_consume_token(this, qpc::WITHIN)?;
    let _ = jj_consume_token(this, qpc::LPAREN)?;
    source = interval_fun(this)?;
    positions = jj_consume_token(this, qpc::NUMBER)?;
    reference = interval_fun(this)?;
    let _ = jj_consume_token(this, qpc::RPAREN)?;
    {
        if true {
            return Ok(Some(Arc::new(Within::new(
                source.clone(),
                StandardSyntaxParser::parse_int(&crate::javacc::nn_owned(positions.clone())?)?,
                reference.clone(),
            )?) as Arc<dyn IntervalFunction>));
        }
    }
    return Err(
        crate::javacc::java_error("Missing return statement in function".to_string()).into(),
    );
}

/// `IntervalExtend()` of the generated Java.
#[allow(unreachable_code)]
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn interval_extend<P: StandardSyntaxParserOps + ?Sized>(
    this: &mut P,
) -> Result<Option<Arc<dyn IntervalFunction>>> {
    let mut source: Option<Arc<dyn IntervalFunction>> = None;
    let mut before: Option<TokenRef> = None;
    let mut after: Option<TokenRef> = None;
    let _ = jj_consume_token(this, qpc::FN_PREFIX)?;
    let _ = jj_consume_token(this, qpc::EXTEND)?;
    let _ = jj_consume_token(this, qpc::LPAREN)?;
    source = interval_fun(this)?;
    before = jj_consume_token(this, qpc::NUMBER)?;
    after = jj_consume_token(this, qpc::NUMBER)?;
    let _ = jj_consume_token(this, qpc::RPAREN)?;
    {
        if true {
            return Ok(Some(Arc::new(Extend::new(
                source.clone(),
                StandardSyntaxParser::parse_int(&crate::javacc::nn_owned(before.clone())?)?,
                StandardSyntaxParser::parse_int(&crate::javacc::nn_owned(after.clone())?)?,
            )?) as Arc<dyn IntervalFunction>));
        }
    }
    return Err(
        crate::javacc::java_error("Missing return statement in function".to_string()).into(),
    );
}

/// `IntervalNotWithin()` of the generated Java.
#[allow(unreachable_code)]
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn interval_not_within<P: StandardSyntaxParserOps + ?Sized>(
    this: &mut P,
) -> Result<Option<Arc<dyn IntervalFunction>>> {
    let mut minuend: Option<Arc<dyn IntervalFunction>> = None;
    let mut subtrahend: Option<Arc<dyn IntervalFunction>> = None;
    let mut positions: Option<TokenRef> = None;
    let _ = jj_consume_token(this, qpc::FN_PREFIX)?;
    let _ = jj_consume_token(this, qpc::NOT_WITHIN)?;
    let _ = jj_consume_token(this, qpc::LPAREN)?;
    minuend = interval_fun(this)?;
    positions = jj_consume_token(this, qpc::NUMBER)?;
    subtrahend = interval_fun(this)?;
    let _ = jj_consume_token(this, qpc::RPAREN)?;
    {
        if true {
            return Ok(Some(Arc::new(NotWithin::new(
                minuend.clone(),
                StandardSyntaxParser::parse_int(&crate::javacc::nn_owned(positions.clone())?)?,
                subtrahend.clone(),
            )?) as Arc<dyn IntervalFunction>));
        }
    }
    return Err(
        crate::javacc::java_error("Missing return statement in function".to_string()).into(),
    );
}

/// `IntervalOverlapping()` of the generated Java.
#[allow(unreachable_code)]
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn interval_overlapping<P: StandardSyntaxParserOps + ?Sized>(
    this: &mut P,
) -> Result<Option<Arc<dyn IntervalFunction>>> {
    let mut source: Option<Arc<dyn IntervalFunction>> = None;
    let mut reference: Option<Arc<dyn IntervalFunction>> = None;
    let _ = jj_consume_token(this, qpc::FN_PREFIX)?;
    let _ = jj_consume_token(this, qpc::OVERLAPPING)?;
    let _ = jj_consume_token(this, qpc::LPAREN)?;
    source = interval_fun(this)?;
    reference = interval_fun(this)?;
    let _ = jj_consume_token(this, qpc::RPAREN)?;
    {
        if true {
            return Ok(Some(
                Arc::new(Overlapping::new(source.clone(), reference.clone())?)
                    as Arc<dyn IntervalFunction>,
            ));
        }
    }
    return Err(
        crate::javacc::java_error("Missing return statement in function".to_string()).into(),
    );
}

/// `IntervalNonOverlapping()` of the generated Java.
#[allow(unreachable_code)]
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn interval_non_overlapping<P: StandardSyntaxParserOps + ?Sized>(
    this: &mut P,
) -> Result<Option<Arc<dyn IntervalFunction>>> {
    let mut minuend: Option<Arc<dyn IntervalFunction>> = None;
    let mut subtrahend: Option<Arc<dyn IntervalFunction>> = None;
    let _ = jj_consume_token(this, qpc::FN_PREFIX)?;
    let _ = jj_consume_token(this, qpc::NON_OVERLAPPING)?;
    let _ = jj_consume_token(this, qpc::LPAREN)?;
    minuend = interval_fun(this)?;
    subtrahend = interval_fun(this)?;
    let _ = jj_consume_token(this, qpc::RPAREN)?;
    {
        if true {
            return Ok(Some(
                Arc::new(NonOverlapping::new(minuend.clone(), subtrahend.clone())?)
                    as Arc<dyn IntervalFunction>,
            ));
        }
    }
    return Err(
        crate::javacc::java_error("Missing return statement in function".to_string()).into(),
    );
}

/// `IntervalWildcard()` of the generated Java.
#[allow(unreachable_code)]
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn interval_wildcard<P: StandardSyntaxParserOps + ?Sized>(
    this: &mut P,
) -> Result<Option<Arc<dyn IntervalFunction>>> {
    let mut wildcard: Option<String> = None;
    let mut max_expansions: Option<TokenRef> = None;
    let _ = jj_consume_token(this, qpc::FN_PREFIX)?;
    let _ = jj_consume_token(this, qpc::WILDCARD)?;
    let _ = jj_consume_token(this, qpc::LPAREN)?;
    'sw1: {
        match if (this.jj().jj_ntk == -1) {
            jj_ntk_f(this)?
        } else {
            this.jj().jj_ntk
        } {
            qpc::NUMBER | qpc::TERM => {
                'sw2: {
                    match if (this.jj().jj_ntk == -1) {
                        jj_ntk_f(this)?
                    } else {
                        this.jj().jj_ntk
                    } {
                        qpc::TERM => {
                            let _ = jj_consume_token(this, qpc::TERM)?;
                        }
                        qpc::NUMBER => {
                            let _ = jj_consume_token(this, qpc::NUMBER)?;
                        }
                        _ => {
                            let __i1 = (16) as usize;
                            this.jj_mut().jj_la1[__i1] = this.jj().jj_gen;
                            let _ = jj_consume_token(this, -1)?;
                            return Err(ParseException::new().error().into());
                        }
                    }
                }
                wildcard = crate::javacc::nn(this.jj().token.as_ref())?.read(|o| o.image.clone());
            }
            qpc::QUOTED => {
                let _ = jj_consume_token(this, qpc::QUOTED)?;
                wildcard = Some(crate::javacc::substring(
                    &crate::javacc::nn_str(
                        crate::javacc::nn(this.jj().token.as_ref())?
                            .read(|o| o.image.clone())
                            .as_deref(),
                    )?,
                    1,
                    (crate::javacc::length(&crate::javacc::nn_str(
                        crate::javacc::nn(this.jj().token.as_ref())?
                            .read(|o| o.image.clone())
                            .as_deref(),
                    )?))
                    .wrapping_sub(1),
                )?);
            }
            _ => {
                let __i2 = (17) as usize;
                this.jj_mut().jj_la1[__i2] = this.jj().jj_gen;
                let _ = jj_consume_token(this, -1)?;
                return Err(ParseException::new().error().into());
            }
        }
    }
    'sw3: {
        match if (this.jj().jj_ntk == -1) {
            jj_ntk_f(this)?
        } else {
            this.jj().jj_ntk
        } {
            qpc::NUMBER => {
                max_expansions = jj_consume_token(this, qpc::NUMBER)?;
            }
            _ => {
                let __i3 = (18) as usize;
                this.jj_mut().jj_la1[__i3] = this.jj().jj_gen;
            }
        }
    }
    let _ = jj_consume_token(this, qpc::RPAREN)?;
    {
        if true {
            return Ok(Some(Arc::new(Wildcard::new(
                crate::javacc::nn_str(wildcard.clone().as_deref())?.to_string(),
                if max_expansions.clone().is_none() {
                    0
                } else {
                    StandardSyntaxParser::parse_int(&crate::javacc::nn_owned(
                        max_expansions.clone(),
                    )?)?
                },
            )) as Arc<dyn IntervalFunction>));
        }
    }
    return Err(
        crate::javacc::java_error("Missing return statement in function".to_string()).into(),
    );
}

/// `IntervalFuzzyTerm()` of the generated Java.
#[allow(unreachable_code)]
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn interval_fuzzy_term<P: StandardSyntaxParserOps + ?Sized>(
    this: &mut P,
) -> Result<Option<Arc<dyn IntervalFunction>>> {
    let mut term: Option<String> = None;
    let mut max_edits: Option<TokenRef> = None;
    let mut max_expansions: Option<TokenRef> = None;
    let _ = jj_consume_token(this, qpc::FN_PREFIX)?;
    let _ = jj_consume_token(this, qpc::FUZZYTERM)?;
    let _ = jj_consume_token(this, qpc::LPAREN)?;
    'sw1: {
        match if (this.jj().jj_ntk == -1) {
            jj_ntk_f(this)?
        } else {
            this.jj().jj_ntk
        } {
            qpc::NUMBER | qpc::TERM => {
                'sw2: {
                    match if (this.jj().jj_ntk == -1) {
                        jj_ntk_f(this)?
                    } else {
                        this.jj().jj_ntk
                    } {
                        qpc::TERM => {
                            let _ = jj_consume_token(this, qpc::TERM)?;
                        }
                        qpc::NUMBER => {
                            let _ = jj_consume_token(this, qpc::NUMBER)?;
                        }
                        _ => {
                            let __i1 = (19) as usize;
                            this.jj_mut().jj_la1[__i1] = this.jj().jj_gen;
                            let _ = jj_consume_token(this, -1)?;
                            return Err(ParseException::new().error().into());
                        }
                    }
                }
                term = crate::javacc::nn(this.jj().token.as_ref())?.read(|o| o.image.clone());
            }
            qpc::QUOTED => {
                let _ = jj_consume_token(this, qpc::QUOTED)?;
                term = Some(crate::javacc::substring(
                    &crate::javacc::nn_str(
                        crate::javacc::nn(this.jj().token.as_ref())?
                            .read(|o| o.image.clone())
                            .as_deref(),
                    )?,
                    1,
                    (crate::javacc::length(&crate::javacc::nn_str(
                        crate::javacc::nn(this.jj().token.as_ref())?
                            .read(|o| o.image.clone())
                            .as_deref(),
                    )?))
                    .wrapping_sub(1),
                )?);
            }
            _ => {
                let __i2 = (20) as usize;
                this.jj_mut().jj_la1[__i2] = this.jj().jj_gen;
                let _ = jj_consume_token(this, -1)?;
                return Err(ParseException::new().error().into());
            }
        }
    }
    if jj_2_26(this, 2)? {
        max_edits = jj_consume_token(this, qpc::NUMBER)?;
    } else {
    }
    if jj_2_27(this, 2)? {
        max_expansions = jj_consume_token(this, qpc::NUMBER)?;
    } else {
    }
    let _ = jj_consume_token(this, qpc::RPAREN)?;
    {
        if true {
            return Ok(Some(Arc::new(FuzzyTerm::new(
                crate::javacc::nn_str(term.clone().as_deref())?.to_string(),
                if max_edits.clone().is_none() {
                    None
                } else {
                    Some(StandardSyntaxParser::parse_int(&crate::javacc::nn_owned(
                        max_edits.clone(),
                    )?)?)
                },
                if max_expansions.clone().is_none() {
                    None
                } else {
                    Some(StandardSyntaxParser::parse_int(&crate::javacc::nn_owned(
                        max_expansions.clone(),
                    )?)?)
                },
            )) as Arc<dyn IntervalFunction>));
        }
    }
    return Err(
        crate::javacc::java_error("Missing return statement in function".to_string()).into(),
    );
}

/// `IntervalText()` of the generated Java.
#[allow(unreachable_code)]
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn interval_text<P: StandardSyntaxParserOps + ?Sized>(
    this: &mut P,
) -> Result<Option<Arc<dyn IntervalFunction>>> {
    'sw1: {
        match if (this.jj().jj_ntk == -1) {
            jj_ntk_f(this)?
        } else {
            this.jj().jj_ntk
        } {
            qpc::QUOTED => {
                let _ = jj_consume_token(this, qpc::QUOTED)?;
                {
                    if true {
                        return Ok(Some(Arc::new(AnalyzedText::new(crate::javacc::substring(
                            &crate::javacc::nn_str(
                                crate::javacc::nn(this.jj().token.as_ref())?
                                    .read(|o| o.image.clone())
                                    .as_deref(),
                            )?,
                            1,
                            (crate::javacc::length(&crate::javacc::nn_str(
                                crate::javacc::nn(this.jj().token.as_ref())?
                                    .read(|o| o.image.clone())
                                    .as_deref(),
                            )?))
                            .wrapping_sub(1),
                        )?))
                            as Arc<dyn IntervalFunction>));
                    }
                }
            }
            qpc::NUMBER | qpc::TERM => {
                'sw2: {
                    match if (this.jj().jj_ntk == -1) {
                        jj_ntk_f(this)?
                    } else {
                        this.jj().jj_ntk
                    } {
                        qpc::TERM => {
                            let _ = jj_consume_token(this, qpc::TERM)?;
                        }
                        qpc::NUMBER => {
                            let _ = jj_consume_token(this, qpc::NUMBER)?;
                        }
                        _ => {
                            let __i1 = (21) as usize;
                            this.jj_mut().jj_la1[__i1] = this.jj().jj_gen;
                            let _ = jj_consume_token(this, -1)?;
                            return Err(ParseException::new().error().into());
                        }
                    }
                }
                {
                    if true {
                        return Ok(Some(Arc::new(AnalyzedText::new(
                            crate::javacc::nn_str(
                                crate::javacc::nn(this.jj().token.as_ref())?
                                    .read(|o| o.image.clone())
                                    .as_deref(),
                            )?
                            .to_string(),
                        ))
                            as Arc<dyn IntervalFunction>));
                    }
                }
            }
            _ => {
                let __i2 = (22) as usize;
                this.jj_mut().jj_la1[__i2] = this.jj().jj_gen;
                let _ = jj_consume_token(this, -1)?;
                return Err(ParseException::new().error().into());
            }
        }
    }
    return Err(
        crate::javacc::java_error("Missing return statement in function".to_string()).into(),
    );
}

/// `Boost(QueryNode)` of the generated Java.
#[allow(unreachable_code)]
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn boost<P: StandardSyntaxParserOps + ?Sized>(
    this: &mut P,
    mut node: Option<QueryNodeRef>,
) -> Result<Option<QueryNodeRef>> {
    let mut boost: Option<TokenRef> = None;
    let _ = jj_consume_token(this, qpc::CARAT)?;
    boost = jj_consume_token(this, qpc::NUMBER)?;
    {
        if true {
            return Ok(if node.clone().is_none() {
                node.clone()
            } else {
                Some(BoostQueryNode::new(
                    node.clone(),
                    StandardSyntaxParser::parse_float(&crate::javacc::nn_owned(boost.clone())?)?,
                )? as QueryNodeRef)
            });
        }
    }
    return Err(
        crate::javacc::java_error("Missing return statement in function".to_string()).into(),
    );
}

/// `FuzzyOp(CharSequence, Token, QueryNode)` of the generated Java.
#[allow(unreachable_code)]
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn fuzzy_op<P: StandardSyntaxParserOps + ?Sized>(
    this: &mut P,
    mut field: Option<CharSequence>,
    mut term: Option<TokenRef>,
    mut node: Option<QueryNodeRef>,
) -> Result<Option<QueryNodeRef>> {
    let mut similarity: Option<TokenRef> = None;
    let _ = jj_consume_token(this, qpc::TILDE)?;
    if jj_2_28(this, 2)? {
        similarity = jj_consume_token(this, qpc::NUMBER)?;
    } else {
    }
    let mut fms: f32 = (fuzzy_query::DEFAULT_MAX_EDITS as f32);
    if similarity.clone().is_some() {
        fms = StandardSyntaxParser::parse_float(&crate::javacc::nn_owned(similarity.clone())?)?;
        if fms < (0.0f32) {
            {
                if true {
                    return Err(ParseException::with_message(Arc::new(MessageImpl::new(
                        QueryParserMessages::INVALID_SYNTAX_FUZZY_LIMITS.get(),
                    )))
                    .error()
                    .into());
                }
            }
        } else {
            if (fms >= (1.0f32)) && (fms != ((fms as i32) as f32)) {
                {
                    if true {
                        return Err(ParseException::with_message(Arc::new(MessageImpl::new(
                            QueryParserMessages::INVALID_SYNTAX_FUZZY_EDITS.get(),
                        )))
                        .error()
                        .into());
                    }
                }
            }
        }
    }
    {
        if true {
            return Ok(Some(FuzzyQueryNode::new(
                field.clone(),
                Some(
                    EscapeQuerySyntaxImpl::discard_escape_char(&CharSequence::from(
                        crate::javacc::nn_str(
                            crate::javacc::nn(term.as_ref())?
                                .read(|o| o.image.clone())
                                .as_deref(),
                        )?
                        .to_string(),
                    ))?
                    .into_char_sequence(),
                ),
                fms,
                crate::javacc::nn(term.as_ref())?.read(|o| o.begin_column),
                crate::javacc::nn(term.as_ref())?.read(|o| o.end_column),
            ) as QueryNodeRef));
        }
    }
    return Err(
        crate::javacc::java_error("Missing return statement in function".to_string()).into(),
    );
}

/// `FieldRangeExpr(CharSequence)` of the generated Java.
#[allow(unreachable_code)]
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn field_range_expr<P: StandardSyntaxParserOps + ?Sized>(
    this: &mut P,
    mut field: Option<CharSequence>,
) -> Result<Option<QueryNodeRef>> {
    let mut operator: Option<TokenRef> = None;
    let mut term: Option<TokenRef> = None;
    let mut q_lower: Option<QueryNodeRef> = None;
    let mut q_upper: Option<QueryNodeRef> = None;
    let mut lower_inclusive: bool = false;
    let mut upper_inclusive: bool = false;
    field = field_name(this)?;
    'sw1: {
        match if (this.jj().jj_ntk == -1) {
            jj_ntk_f(this)?
        } else {
            this.jj().jj_ntk
        } {
            qpc::OP_LESSTHAN => {
                let _ = jj_consume_token(this, qpc::OP_LESSTHAN)?;
            }
            qpc::OP_LESSTHANEQ => {
                let _ = jj_consume_token(this, qpc::OP_LESSTHANEQ)?;
            }
            qpc::OP_MORETHAN => {
                let _ = jj_consume_token(this, qpc::OP_MORETHAN)?;
            }
            qpc::OP_MORETHANEQ => {
                let _ = jj_consume_token(this, qpc::OP_MORETHANEQ)?;
            }
            _ => {
                let __i1 = (23) as usize;
                this.jj_mut().jj_la1[__i1] = this.jj().jj_gen;
                let _ = jj_consume_token(this, -1)?;
                return Err(ParseException::new().error().into());
            }
        }
    }
    operator = this.jj().token.clone();
    'sw2: {
        match if (this.jj().jj_ntk == -1) {
            jj_ntk_f(this)?
        } else {
            this.jj().jj_ntk
        } {
            qpc::TERM => {
                let _ = jj_consume_token(this, qpc::TERM)?;
            }
            qpc::QUOTED => {
                let _ = jj_consume_token(this, qpc::QUOTED)?;
            }
            qpc::NUMBER => {
                let _ = jj_consume_token(this, qpc::NUMBER)?;
            }
            _ => {
                let __i2 = (24) as usize;
                this.jj_mut().jj_la1[__i2] = this.jj().jj_gen;
                let _ = jj_consume_token(this, -1)?;
                return Err(ParseException::new().error().into());
            }
        }
    }
    term = this.jj().token.clone();
    if (crate::javacc::nn(term.as_ref())?.read(|o| o.kind)) == qpc::QUOTED {
        let __o3 = term.clone();
        let __a4 = Some(crate::javacc::substring(
            &crate::javacc::nn_str(
                crate::javacc::nn(term.as_ref())?
                    .read(|o| o.image.clone())
                    .as_deref(),
            )?,
            1,
            (crate::javacc::length(&crate::javacc::nn_str(
                crate::javacc::nn(term.as_ref())?
                    .read(|o| o.image.clone())
                    .as_deref(),
            )?))
            .wrapping_sub(1),
        )?);
        crate::javacc::nn(__o3.as_ref())?.lock().image = __a4;
    }
    'sw3: {
        match crate::javacc::nn(operator.as_ref())?.read(|o| o.kind) {
            qpc::OP_LESSTHAN => {
                lower_inclusive = true;
                upper_inclusive = false;
                q_lower = Some(FieldQueryNode::new(
                    field.clone(),
                    Some(CharSequence::from("*".to_string())),
                    crate::javacc::nn(term.as_ref())?.read(|o| o.begin_column),
                    crate::javacc::nn(term.as_ref())?.read(|o| o.end_column),
                ) as QueryNodeRef);
                q_upper = Some(FieldQueryNode::new(
                    field.clone(),
                    Some(
                        EscapeQuerySyntaxImpl::discard_escape_char(&CharSequence::from(
                            crate::javacc::nn_str(
                                crate::javacc::nn(term.as_ref())?
                                    .read(|o| o.image.clone())
                                    .as_deref(),
                            )?
                            .to_string(),
                        ))?
                        .into_char_sequence(),
                    ),
                    crate::javacc::nn(term.as_ref())?.read(|o| o.begin_column),
                    crate::javacc::nn(term.as_ref())?.read(|o| o.end_column),
                ) as QueryNodeRef);
            }
            qpc::OP_LESSTHANEQ => {
                lower_inclusive = true;
                upper_inclusive = true;
                q_lower = Some(FieldQueryNode::new(
                    field.clone(),
                    Some(CharSequence::from("*".to_string())),
                    crate::javacc::nn(term.as_ref())?.read(|o| o.begin_column),
                    crate::javacc::nn(term.as_ref())?.read(|o| o.end_column),
                ) as QueryNodeRef);
                q_upper = Some(FieldQueryNode::new(
                    field.clone(),
                    Some(
                        EscapeQuerySyntaxImpl::discard_escape_char(&CharSequence::from(
                            crate::javacc::nn_str(
                                crate::javacc::nn(term.as_ref())?
                                    .read(|o| o.image.clone())
                                    .as_deref(),
                            )?
                            .to_string(),
                        ))?
                        .into_char_sequence(),
                    ),
                    crate::javacc::nn(term.as_ref())?.read(|o| o.begin_column),
                    crate::javacc::nn(term.as_ref())?.read(|o| o.end_column),
                ) as QueryNodeRef);
            }
            qpc::OP_MORETHAN => {
                lower_inclusive = false;
                upper_inclusive = true;
                q_lower = Some(FieldQueryNode::new(
                    field.clone(),
                    Some(
                        EscapeQuerySyntaxImpl::discard_escape_char(&CharSequence::from(
                            crate::javacc::nn_str(
                                crate::javacc::nn(term.as_ref())?
                                    .read(|o| o.image.clone())
                                    .as_deref(),
                            )?
                            .to_string(),
                        ))?
                        .into_char_sequence(),
                    ),
                    crate::javacc::nn(term.as_ref())?.read(|o| o.begin_column),
                    crate::javacc::nn(term.as_ref())?.read(|o| o.end_column),
                ) as QueryNodeRef);
                q_upper = Some(FieldQueryNode::new(
                    field.clone(),
                    Some(CharSequence::from("*".to_string())),
                    crate::javacc::nn(term.as_ref())?.read(|o| o.begin_column),
                    crate::javacc::nn(term.as_ref())?.read(|o| o.end_column),
                ) as QueryNodeRef);
            }
            qpc::OP_MORETHANEQ => {
                lower_inclusive = true;
                upper_inclusive = true;
                q_lower = Some(FieldQueryNode::new(
                    field.clone(),
                    Some(
                        EscapeQuerySyntaxImpl::discard_escape_char(&CharSequence::from(
                            crate::javacc::nn_str(
                                crate::javacc::nn(term.as_ref())?
                                    .read(|o| o.image.clone())
                                    .as_deref(),
                            )?
                            .to_string(),
                        ))?
                        .into_char_sequence(),
                    ),
                    crate::javacc::nn(term.as_ref())?.read(|o| o.begin_column),
                    crate::javacc::nn(term.as_ref())?.read(|o| o.end_column),
                ) as QueryNodeRef);
                q_upper = Some(FieldQueryNode::new(
                    field.clone(),
                    Some(CharSequence::from("*".to_string())),
                    crate::javacc::nn(term.as_ref())?.read(|o| o.begin_column),
                    crate::javacc::nn(term.as_ref())?.read(|o| o.end_column),
                ) as QueryNodeRef);
            }
            _ => {
                if true {
                    return Err(crate::javacc::java_error(crate::javacc::concat(&[
                        &"Unhandled case, operator=".to_string(),
                        &crate::javacc::string_value_of(
                            operator
                                .clone()
                                .as_ref()
                                .and_then(|t| t.read(|o| o.image.clone()))
                                .as_deref(),
                        ),
                    ]))
                    .into());
                }
            }
        }
    }
    {
        if true {
            return Ok(Some(TermRangeQueryNode::new(
                q_lower.clone(),
                q_upper.clone(),
                lower_inclusive,
                upper_inclusive,
            )? as QueryNodeRef));
        }
    }
    return Err(
        crate::javacc::java_error("Missing return statement in function".to_string()).into(),
    );
}

/// `Term(CharSequence)` of the generated Java.
#[allow(unreachable_code)]
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn term<P: StandardSyntaxParserOps + ?Sized>(
    this: &mut P,
    mut field: Option<CharSequence>,
) -> Result<Option<QueryNodeRef>> {
    let mut q: Option<QueryNodeRef> = None;
    let mut term: Option<TokenRef> = None;
    let mut fuzzy_slop: Option<TokenRef> = None;
    'sw1: {
        match if (this.jj().jj_ntk == -1) {
            jj_ntk_f(this)?
        } else {
            this.jj().jj_ntk
        } {
            qpc::REGEXPTERM => {
                term = jj_consume_token(this, qpc::REGEXPTERM)?;
                let mut v: Option<String> = Some(crate::javacc::substring(
                    &crate::javacc::nn_str(
                        crate::javacc::nn(term.as_ref())?
                            .read(|o| o.image.clone())
                            .as_deref(),
                    )?,
                    1,
                    (crate::javacc::length(&crate::javacc::nn_str(
                        crate::javacc::nn(term.as_ref())?
                            .read(|o| o.image.clone())
                            .as_deref(),
                    )?))
                    .wrapping_sub(1),
                )?);
                q = Some(RegexpQueryNode::new(
                    field.clone(),
                    &CharSequence::from(crate::javacc::nn_str(v.clone().as_deref())?.to_string()),
                    (0) as usize,
                    (crate::javacc::length(&crate::javacc::nn_str(v.as_deref())?)) as usize,
                )? as QueryNodeRef);
            }
            qpc::NUMBER | qpc::TERM => {
                'sw2: {
                    match if (this.jj().jj_ntk == -1) {
                        jj_ntk_f(this)?
                    } else {
                        this.jj().jj_ntk
                    } {
                        qpc::TERM => {
                            term = jj_consume_token(this, qpc::TERM)?;
                        }
                        qpc::NUMBER => {
                            term = jj_consume_token(this, qpc::NUMBER)?;
                        }
                        _ => {
                            let __i1 = (25) as usize;
                            this.jj_mut().jj_la1[__i1] = this.jj().jj_gen;
                            let _ = jj_consume_token(this, -1)?;
                            return Err(ParseException::new().error().into());
                        }
                    }
                }
                q = Some(FieldQueryNode::new(
                    field.clone(),
                    Some(
                        EscapeQuerySyntaxImpl::discard_escape_char(&CharSequence::from(
                            crate::javacc::nn_str(
                                crate::javacc::nn(term.as_ref())?
                                    .read(|o| o.image.clone())
                                    .as_deref(),
                            )?
                            .to_string(),
                        ))?
                        .into_char_sequence(),
                    ),
                    crate::javacc::nn(term.as_ref())?.read(|o| o.begin_column),
                    crate::javacc::nn(term.as_ref())?.read(|o| o.end_column),
                ) as QueryNodeRef);
                'sw3: {
                    match if (this.jj().jj_ntk == -1) {
                        jj_ntk_f(this)?
                    } else {
                        this.jj().jj_ntk
                    } {
                        qpc::TILDE => {
                            q = fuzzy_op(this, field.clone(), term.clone(), q.clone())?;
                        }
                        _ => {
                            let __i2 = (26) as usize;
                            this.jj_mut().jj_la1[__i2] = this.jj().jj_gen;
                        }
                    }
                }
            }
            qpc::RANGEIN_START | qpc::RANGEEX_START => {
                q = term_range_expr(this, field.clone())?;
            }
            qpc::QUOTED => {
                q = quoted_term(this, field.clone())?;
            }
            _ => {
                let __i3 = (27) as usize;
                this.jj_mut().jj_la1[__i3] = this.jj().jj_gen;
                let _ = jj_consume_token(this, -1)?;
                return Err(ParseException::new().error().into());
            }
        }
    }
    'sw4: {
        match if (this.jj().jj_ntk == -1) {
            jj_ntk_f(this)?
        } else {
            this.jj().jj_ntk
        } {
            qpc::CARAT => {
                q = boost(this, q.clone())?;
            }
            _ => {
                let __i4 = (28) as usize;
                this.jj_mut().jj_la1[__i4] = this.jj().jj_gen;
            }
        }
    }
    {
        if true {
            return Ok(q.clone());
        }
    }
    return Err(
        crate::javacc::java_error("Missing return statement in function".to_string()).into(),
    );
}

/// `QuotedTerm(CharSequence)` of the generated Java.
#[allow(unreachable_code)]
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn quoted_term<P: StandardSyntaxParserOps + ?Sized>(
    this: &mut P,
    mut field: Option<CharSequence>,
) -> Result<Option<QueryNodeRef>> {
    let mut q: Option<QueryNodeRef> = None;
    let mut term: Option<TokenRef> = None;
    let mut slop: Option<TokenRef> = None;
    term = jj_consume_token(this, qpc::QUOTED)?;
    let mut image: Option<String> = Some(crate::javacc::substring(
        &crate::javacc::nn_str(
            crate::javacc::nn(term.as_ref())?
                .read(|o| o.image.clone())
                .as_deref(),
        )?,
        1,
        (crate::javacc::length(&crate::javacc::nn_str(
            crate::javacc::nn(term.as_ref())?
                .read(|o| o.image.clone())
                .as_deref(),
        )?))
        .wrapping_sub(1),
    )?);
    q = Some(QuotedFieldQueryNode::new(
        field.clone(),
        Some(
            EscapeQuerySyntaxImpl::discard_escape_char(&CharSequence::from(
                crate::javacc::nn_str(image.clone().as_deref())?.to_string(),
            ))?
            .into_char_sequence(),
        ),
        (crate::javacc::nn(term.as_ref())?.read(|o| o.begin_column)).wrapping_add(1),
        (crate::javacc::nn(term.as_ref())?.read(|o| o.end_column)).wrapping_sub(1),
    ) as QueryNodeRef);
    'sw1: {
        match if (this.jj().jj_ntk == -1) {
            jj_ntk_f(this)?
        } else {
            this.jj().jj_ntk
        } {
            qpc::TILDE => {
                let _ = jj_consume_token(this, qpc::TILDE)?;
                slop = jj_consume_token(this, qpc::NUMBER)?;
                q = Some(SlopQueryNode::new(
                    q.clone(),
                    StandardSyntaxParser::parse_int(&crate::javacc::nn_owned(slop.clone())?)?,
                )? as QueryNodeRef);
            }
            _ => {
                let __i1 = (29) as usize;
                this.jj_mut().jj_la1[__i1] = this.jj().jj_gen;
            }
        }
    }
    {
        if true {
            return Ok(q.clone());
        }
    }
    return Err(
        crate::javacc::java_error("Missing return statement in function".to_string()).into(),
    );
}

/// `TermRangeExpr(CharSequence)` of the generated Java.
#[allow(unreachable_code)]
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn term_range_expr<P: StandardSyntaxParserOps + ?Sized>(
    this: &mut P,
    mut field: Option<CharSequence>,
) -> Result<Option<QueryNodeRef>> {
    let mut left: Option<TokenRef> = None;
    let mut right: Option<TokenRef> = None;
    let mut left_inclusive: bool = false;
    let mut right_inclusive: bool = false;
    'sw1: {
        match if (this.jj().jj_ntk == -1) {
            jj_ntk_f(this)?
        } else {
            this.jj().jj_ntk
        } {
            qpc::RANGEIN_START => {
                let _ = jj_consume_token(this, qpc::RANGEIN_START)?;
                left_inclusive = true;
            }
            qpc::RANGEEX_START => {
                let _ = jj_consume_token(this, qpc::RANGEEX_START)?;
            }
            _ => {
                let __i1 = (30) as usize;
                this.jj_mut().jj_la1[__i1] = this.jj().jj_gen;
                let _ = jj_consume_token(this, -1)?;
                return Err(ParseException::new().error().into());
            }
        }
    }
    'sw2: {
        match if (this.jj().jj_ntk == -1) {
            jj_ntk_f(this)?
        } else {
            this.jj().jj_ntk
        } {
            qpc::RANGE_GOOP => {
                let _ = jj_consume_token(this, qpc::RANGE_GOOP)?;
            }
            qpc::RANGE_QUOTED => {
                let _ = jj_consume_token(this, qpc::RANGE_QUOTED)?;
            }
            qpc::RANGE_TO => {
                let _ = jj_consume_token(this, qpc::RANGE_TO)?;
            }
            _ => {
                let __i2 = (31) as usize;
                this.jj_mut().jj_la1[__i2] = this.jj().jj_gen;
                let _ = jj_consume_token(this, -1)?;
                return Err(ParseException::new().error().into());
            }
        }
    }
    left = this.jj().token.clone();
    let _ = jj_consume_token(this, qpc::RANGE_TO)?;
    'sw3: {
        match if (this.jj().jj_ntk == -1) {
            jj_ntk_f(this)?
        } else {
            this.jj().jj_ntk
        } {
            qpc::RANGE_GOOP => {
                let _ = jj_consume_token(this, qpc::RANGE_GOOP)?;
            }
            qpc::RANGE_QUOTED => {
                let _ = jj_consume_token(this, qpc::RANGE_QUOTED)?;
            }
            qpc::RANGE_TO => {
                let _ = jj_consume_token(this, qpc::RANGE_TO)?;
            }
            _ => {
                let __i3 = (32) as usize;
                this.jj_mut().jj_la1[__i3] = this.jj().jj_gen;
                let _ = jj_consume_token(this, -1)?;
                return Err(ParseException::new().error().into());
            }
        }
    }
    right = this.jj().token.clone();
    'sw4: {
        match if (this.jj().jj_ntk == -1) {
            jj_ntk_f(this)?
        } else {
            this.jj().jj_ntk
        } {
            qpc::RANGEIN_END => {
                let _ = jj_consume_token(this, qpc::RANGEIN_END)?;
                right_inclusive = true;
            }
            qpc::RANGEEX_END => {
                let _ = jj_consume_token(this, qpc::RANGEEX_END)?;
            }
            _ => {
                let __i4 = (33) as usize;
                this.jj_mut().jj_la1[__i4] = this.jj().jj_gen;
                let _ = jj_consume_token(this, -1)?;
                return Err(ParseException::new().error().into());
            }
        }
    }
    if (crate::javacc::nn(left.as_ref())?.read(|o| o.kind)) == qpc::RANGE_QUOTED {
        let __o5 = left.clone();
        let __a6 = Some(crate::javacc::substring(
            &crate::javacc::nn_str(
                crate::javacc::nn(left.as_ref())?
                    .read(|o| o.image.clone())
                    .as_deref(),
            )?,
            1,
            (crate::javacc::length(&crate::javacc::nn_str(
                crate::javacc::nn(left.as_ref())?
                    .read(|o| o.image.clone())
                    .as_deref(),
            )?))
            .wrapping_sub(1),
        )?);
        crate::javacc::nn(__o5.as_ref())?.lock().image = __a6;
    }
    if (crate::javacc::nn(right.as_ref())?.read(|o| o.kind)) == qpc::RANGE_QUOTED {
        let __o7 = right.clone();
        let __a8 = Some(crate::javacc::substring(
            &crate::javacc::nn_str(
                crate::javacc::nn(right.as_ref())?
                    .read(|o| o.image.clone())
                    .as_deref(),
            )?,
            1,
            (crate::javacc::length(&crate::javacc::nn_str(
                crate::javacc::nn(right.as_ref())?
                    .read(|o| o.image.clone())
                    .as_deref(),
            )?))
            .wrapping_sub(1),
        )?);
        crate::javacc::nn(__o7.as_ref())?.lock().image = __a8;
    }
    let mut q_lower: Option<QueryNodeRef> = Some(FieldQueryNode::new(
        field.clone(),
        Some(
            EscapeQuerySyntaxImpl::discard_escape_char(&CharSequence::from(
                crate::javacc::nn_str(
                    crate::javacc::nn(left.as_ref())?
                        .read(|o| o.image.clone())
                        .as_deref(),
                )?
                .to_string(),
            ))?
            .into_char_sequence(),
        ),
        crate::javacc::nn(left.as_ref())?.read(|o| o.begin_column),
        crate::javacc::nn(left.as_ref())?.read(|o| o.end_column),
    ) as QueryNodeRef);
    let mut q_upper: Option<QueryNodeRef> = Some(FieldQueryNode::new(
        field.clone(),
        Some(
            EscapeQuerySyntaxImpl::discard_escape_char(&CharSequence::from(
                crate::javacc::nn_str(
                    crate::javacc::nn(right.as_ref())?
                        .read(|o| o.image.clone())
                        .as_deref(),
                )?
                .to_string(),
            ))?
            .into_char_sequence(),
        ),
        crate::javacc::nn(right.as_ref())?.read(|o| o.begin_column),
        crate::javacc::nn(right.as_ref())?.read(|o| o.end_column),
    ) as QueryNodeRef);
    {
        if true {
            return Ok(Some(TermRangeQueryNode::new(
                q_lower.clone(),
                q_upper.clone(),
                left_inclusive,
                right_inclusive,
            )? as QueryNodeRef));
        }
    }
    return Err(
        crate::javacc::java_error("Missing return statement in function".to_string()).into(),
    );
}

/// `jj_2_1(int)` of the generated Java.
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn jj_2_1<P: StandardSyntaxParserOps + ?Sized>(this: &mut P, mut xla: i32) -> Result<bool> {
    this.jj_mut().jj_la = xla;
    let __a1 = this.jj().token.clone();
    this.jj_mut().jj_scanpos = __a1.clone();
    this.jj_mut().jj_lastpos = __a1;
    let __r2: Result<bool> = match jj_3_1(this) {
        Ok(v) => Ok(!v),
        Err(Raised::LookaheadSuccess(_)) => Ok(true),
        Err(Raised::Throwable(e)) => Err(e),
    };
    jj_save(this, 0, xla)?;
    return __r2;
}

/// `jj_2_2(int)` of the generated Java.
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn jj_2_2<P: StandardSyntaxParserOps + ?Sized>(this: &mut P, mut xla: i32) -> Result<bool> {
    this.jj_mut().jj_la = xla;
    let __a1 = this.jj().token.clone();
    this.jj_mut().jj_scanpos = __a1.clone();
    this.jj_mut().jj_lastpos = __a1;
    let __r2: Result<bool> = match jj_3_2(this) {
        Ok(v) => Ok(!v),
        Err(Raised::LookaheadSuccess(_)) => Ok(true),
        Err(Raised::Throwable(e)) => Err(e),
    };
    jj_save(this, 1, xla)?;
    return __r2;
}

/// `jj_2_3(int)` of the generated Java.
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn jj_2_3<P: StandardSyntaxParserOps + ?Sized>(this: &mut P, mut xla: i32) -> Result<bool> {
    this.jj_mut().jj_la = xla;
    let __a1 = this.jj().token.clone();
    this.jj_mut().jj_scanpos = __a1.clone();
    this.jj_mut().jj_lastpos = __a1;
    let __r2: Result<bool> = match jj_3_3(this) {
        Ok(v) => Ok(!v),
        Err(Raised::LookaheadSuccess(_)) => Ok(true),
        Err(Raised::Throwable(e)) => Err(e),
    };
    jj_save(this, 2, xla)?;
    return __r2;
}

/// `jj_2_4(int)` of the generated Java.
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn jj_2_4<P: StandardSyntaxParserOps + ?Sized>(this: &mut P, mut xla: i32) -> Result<bool> {
    this.jj_mut().jj_la = xla;
    let __a1 = this.jj().token.clone();
    this.jj_mut().jj_scanpos = __a1.clone();
    this.jj_mut().jj_lastpos = __a1;
    let __r2: Result<bool> = match jj_3_4(this) {
        Ok(v) => Ok(!v),
        Err(Raised::LookaheadSuccess(_)) => Ok(true),
        Err(Raised::Throwable(e)) => Err(e),
    };
    jj_save(this, 3, xla)?;
    return __r2;
}

/// `jj_2_5(int)` of the generated Java.
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn jj_2_5<P: StandardSyntaxParserOps + ?Sized>(this: &mut P, mut xla: i32) -> Result<bool> {
    this.jj_mut().jj_la = xla;
    let __a1 = this.jj().token.clone();
    this.jj_mut().jj_scanpos = __a1.clone();
    this.jj_mut().jj_lastpos = __a1;
    let __r2: Result<bool> = match jj_3_5(this) {
        Ok(v) => Ok(!v),
        Err(Raised::LookaheadSuccess(_)) => Ok(true),
        Err(Raised::Throwable(e)) => Err(e),
    };
    jj_save(this, 4, xla)?;
    return __r2;
}

/// `jj_2_6(int)` of the generated Java.
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn jj_2_6<P: StandardSyntaxParserOps + ?Sized>(this: &mut P, mut xla: i32) -> Result<bool> {
    this.jj_mut().jj_la = xla;
    let __a1 = this.jj().token.clone();
    this.jj_mut().jj_scanpos = __a1.clone();
    this.jj_mut().jj_lastpos = __a1;
    let __r2: Result<bool> = match jj_3_6(this) {
        Ok(v) => Ok(!v),
        Err(Raised::LookaheadSuccess(_)) => Ok(true),
        Err(Raised::Throwable(e)) => Err(e),
    };
    jj_save(this, 5, xla)?;
    return __r2;
}

/// `jj_2_7(int)` of the generated Java.
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn jj_2_7<P: StandardSyntaxParserOps + ?Sized>(this: &mut P, mut xla: i32) -> Result<bool> {
    this.jj_mut().jj_la = xla;
    let __a1 = this.jj().token.clone();
    this.jj_mut().jj_scanpos = __a1.clone();
    this.jj_mut().jj_lastpos = __a1;
    let __r2: Result<bool> = match jj_3_7(this) {
        Ok(v) => Ok(!v),
        Err(Raised::LookaheadSuccess(_)) => Ok(true),
        Err(Raised::Throwable(e)) => Err(e),
    };
    jj_save(this, 6, xla)?;
    return __r2;
}

/// `jj_2_8(int)` of the generated Java.
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn jj_2_8<P: StandardSyntaxParserOps + ?Sized>(this: &mut P, mut xla: i32) -> Result<bool> {
    this.jj_mut().jj_la = xla;
    let __a1 = this.jj().token.clone();
    this.jj_mut().jj_scanpos = __a1.clone();
    this.jj_mut().jj_lastpos = __a1;
    let __r2: Result<bool> = match jj_3_8(this) {
        Ok(v) => Ok(!v),
        Err(Raised::LookaheadSuccess(_)) => Ok(true),
        Err(Raised::Throwable(e)) => Err(e),
    };
    jj_save(this, 7, xla)?;
    return __r2;
}

/// `jj_2_9(int)` of the generated Java.
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn jj_2_9<P: StandardSyntaxParserOps + ?Sized>(this: &mut P, mut xla: i32) -> Result<bool> {
    this.jj_mut().jj_la = xla;
    let __a1 = this.jj().token.clone();
    this.jj_mut().jj_scanpos = __a1.clone();
    this.jj_mut().jj_lastpos = __a1;
    let __r2: Result<bool> = match jj_3_9(this) {
        Ok(v) => Ok(!v),
        Err(Raised::LookaheadSuccess(_)) => Ok(true),
        Err(Raised::Throwable(e)) => Err(e),
    };
    jj_save(this, 8, xla)?;
    return __r2;
}

/// `jj_2_10(int)` of the generated Java.
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn jj_2_10<P: StandardSyntaxParserOps + ?Sized>(this: &mut P, mut xla: i32) -> Result<bool> {
    this.jj_mut().jj_la = xla;
    let __a1 = this.jj().token.clone();
    this.jj_mut().jj_scanpos = __a1.clone();
    this.jj_mut().jj_lastpos = __a1;
    let __r2: Result<bool> = match jj_3_10(this) {
        Ok(v) => Ok(!v),
        Err(Raised::LookaheadSuccess(_)) => Ok(true),
        Err(Raised::Throwable(e)) => Err(e),
    };
    jj_save(this, 9, xla)?;
    return __r2;
}

/// `jj_2_11(int)` of the generated Java.
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn jj_2_11<P: StandardSyntaxParserOps + ?Sized>(this: &mut P, mut xla: i32) -> Result<bool> {
    this.jj_mut().jj_la = xla;
    let __a1 = this.jj().token.clone();
    this.jj_mut().jj_scanpos = __a1.clone();
    this.jj_mut().jj_lastpos = __a1;
    let __r2: Result<bool> = match jj_3_11(this) {
        Ok(v) => Ok(!v),
        Err(Raised::LookaheadSuccess(_)) => Ok(true),
        Err(Raised::Throwable(e)) => Err(e),
    };
    jj_save(this, 10, xla)?;
    return __r2;
}

/// `jj_2_12(int)` of the generated Java.
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn jj_2_12<P: StandardSyntaxParserOps + ?Sized>(this: &mut P, mut xla: i32) -> Result<bool> {
    this.jj_mut().jj_la = xla;
    let __a1 = this.jj().token.clone();
    this.jj_mut().jj_scanpos = __a1.clone();
    this.jj_mut().jj_lastpos = __a1;
    let __r2: Result<bool> = match jj_3_12(this) {
        Ok(v) => Ok(!v),
        Err(Raised::LookaheadSuccess(_)) => Ok(true),
        Err(Raised::Throwable(e)) => Err(e),
    };
    jj_save(this, 11, xla)?;
    return __r2;
}

/// `jj_2_13(int)` of the generated Java.
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn jj_2_13<P: StandardSyntaxParserOps + ?Sized>(this: &mut P, mut xla: i32) -> Result<bool> {
    this.jj_mut().jj_la = xla;
    let __a1 = this.jj().token.clone();
    this.jj_mut().jj_scanpos = __a1.clone();
    this.jj_mut().jj_lastpos = __a1;
    let __r2: Result<bool> = match jj_3_13(this) {
        Ok(v) => Ok(!v),
        Err(Raised::LookaheadSuccess(_)) => Ok(true),
        Err(Raised::Throwable(e)) => Err(e),
    };
    jj_save(this, 12, xla)?;
    return __r2;
}

/// `jj_2_14(int)` of the generated Java.
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn jj_2_14<P: StandardSyntaxParserOps + ?Sized>(this: &mut P, mut xla: i32) -> Result<bool> {
    this.jj_mut().jj_la = xla;
    let __a1 = this.jj().token.clone();
    this.jj_mut().jj_scanpos = __a1.clone();
    this.jj_mut().jj_lastpos = __a1;
    let __r2: Result<bool> = match jj_3_14(this) {
        Ok(v) => Ok(!v),
        Err(Raised::LookaheadSuccess(_)) => Ok(true),
        Err(Raised::Throwable(e)) => Err(e),
    };
    jj_save(this, 13, xla)?;
    return __r2;
}

/// `jj_2_15(int)` of the generated Java.
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn jj_2_15<P: StandardSyntaxParserOps + ?Sized>(this: &mut P, mut xla: i32) -> Result<bool> {
    this.jj_mut().jj_la = xla;
    let __a1 = this.jj().token.clone();
    this.jj_mut().jj_scanpos = __a1.clone();
    this.jj_mut().jj_lastpos = __a1;
    let __r2: Result<bool> = match jj_3_15(this) {
        Ok(v) => Ok(!v),
        Err(Raised::LookaheadSuccess(_)) => Ok(true),
        Err(Raised::Throwable(e)) => Err(e),
    };
    jj_save(this, 14, xla)?;
    return __r2;
}

/// `jj_2_16(int)` of the generated Java.
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn jj_2_16<P: StandardSyntaxParserOps + ?Sized>(this: &mut P, mut xla: i32) -> Result<bool> {
    this.jj_mut().jj_la = xla;
    let __a1 = this.jj().token.clone();
    this.jj_mut().jj_scanpos = __a1.clone();
    this.jj_mut().jj_lastpos = __a1;
    let __r2: Result<bool> = match jj_3_16(this) {
        Ok(v) => Ok(!v),
        Err(Raised::LookaheadSuccess(_)) => Ok(true),
        Err(Raised::Throwable(e)) => Err(e),
    };
    jj_save(this, 15, xla)?;
    return __r2;
}

/// `jj_2_17(int)` of the generated Java.
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn jj_2_17<P: StandardSyntaxParserOps + ?Sized>(this: &mut P, mut xla: i32) -> Result<bool> {
    this.jj_mut().jj_la = xla;
    let __a1 = this.jj().token.clone();
    this.jj_mut().jj_scanpos = __a1.clone();
    this.jj_mut().jj_lastpos = __a1;
    let __r2: Result<bool> = match jj_3_17(this) {
        Ok(v) => Ok(!v),
        Err(Raised::LookaheadSuccess(_)) => Ok(true),
        Err(Raised::Throwable(e)) => Err(e),
    };
    jj_save(this, 16, xla)?;
    return __r2;
}

/// `jj_2_18(int)` of the generated Java.
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn jj_2_18<P: StandardSyntaxParserOps + ?Sized>(this: &mut P, mut xla: i32) -> Result<bool> {
    this.jj_mut().jj_la = xla;
    let __a1 = this.jj().token.clone();
    this.jj_mut().jj_scanpos = __a1.clone();
    this.jj_mut().jj_lastpos = __a1;
    let __r2: Result<bool> = match jj_3_18(this) {
        Ok(v) => Ok(!v),
        Err(Raised::LookaheadSuccess(_)) => Ok(true),
        Err(Raised::Throwable(e)) => Err(e),
    };
    jj_save(this, 17, xla)?;
    return __r2;
}

/// `jj_2_19(int)` of the generated Java.
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn jj_2_19<P: StandardSyntaxParserOps + ?Sized>(this: &mut P, mut xla: i32) -> Result<bool> {
    this.jj_mut().jj_la = xla;
    let __a1 = this.jj().token.clone();
    this.jj_mut().jj_scanpos = __a1.clone();
    this.jj_mut().jj_lastpos = __a1;
    let __r2: Result<bool> = match jj_3_19(this) {
        Ok(v) => Ok(!v),
        Err(Raised::LookaheadSuccess(_)) => Ok(true),
        Err(Raised::Throwable(e)) => Err(e),
    };
    jj_save(this, 18, xla)?;
    return __r2;
}

/// `jj_2_20(int)` of the generated Java.
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn jj_2_20<P: StandardSyntaxParserOps + ?Sized>(this: &mut P, mut xla: i32) -> Result<bool> {
    this.jj_mut().jj_la = xla;
    let __a1 = this.jj().token.clone();
    this.jj_mut().jj_scanpos = __a1.clone();
    this.jj_mut().jj_lastpos = __a1;
    let __r2: Result<bool> = match jj_3_20(this) {
        Ok(v) => Ok(!v),
        Err(Raised::LookaheadSuccess(_)) => Ok(true),
        Err(Raised::Throwable(e)) => Err(e),
    };
    jj_save(this, 19, xla)?;
    return __r2;
}

/// `jj_2_21(int)` of the generated Java.
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn jj_2_21<P: StandardSyntaxParserOps + ?Sized>(this: &mut P, mut xla: i32) -> Result<bool> {
    this.jj_mut().jj_la = xla;
    let __a1 = this.jj().token.clone();
    this.jj_mut().jj_scanpos = __a1.clone();
    this.jj_mut().jj_lastpos = __a1;
    let __r2: Result<bool> = match jj_3_21(this) {
        Ok(v) => Ok(!v),
        Err(Raised::LookaheadSuccess(_)) => Ok(true),
        Err(Raised::Throwable(e)) => Err(e),
    };
    jj_save(this, 20, xla)?;
    return __r2;
}

/// `jj_2_22(int)` of the generated Java.
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn jj_2_22<P: StandardSyntaxParserOps + ?Sized>(this: &mut P, mut xla: i32) -> Result<bool> {
    this.jj_mut().jj_la = xla;
    let __a1 = this.jj().token.clone();
    this.jj_mut().jj_scanpos = __a1.clone();
    this.jj_mut().jj_lastpos = __a1;
    let __r2: Result<bool> = match jj_3_22(this) {
        Ok(v) => Ok(!v),
        Err(Raised::LookaheadSuccess(_)) => Ok(true),
        Err(Raised::Throwable(e)) => Err(e),
    };
    jj_save(this, 21, xla)?;
    return __r2;
}

/// `jj_2_23(int)` of the generated Java.
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn jj_2_23<P: StandardSyntaxParserOps + ?Sized>(this: &mut P, mut xla: i32) -> Result<bool> {
    this.jj_mut().jj_la = xla;
    let __a1 = this.jj().token.clone();
    this.jj_mut().jj_scanpos = __a1.clone();
    this.jj_mut().jj_lastpos = __a1;
    let __r2: Result<bool> = match jj_3_23(this) {
        Ok(v) => Ok(!v),
        Err(Raised::LookaheadSuccess(_)) => Ok(true),
        Err(Raised::Throwable(e)) => Err(e),
    };
    jj_save(this, 22, xla)?;
    return __r2;
}

/// `jj_2_24(int)` of the generated Java.
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn jj_2_24<P: StandardSyntaxParserOps + ?Sized>(this: &mut P, mut xla: i32) -> Result<bool> {
    this.jj_mut().jj_la = xla;
    let __a1 = this.jj().token.clone();
    this.jj_mut().jj_scanpos = __a1.clone();
    this.jj_mut().jj_lastpos = __a1;
    let __r2: Result<bool> = match jj_3_24(this) {
        Ok(v) => Ok(!v),
        Err(Raised::LookaheadSuccess(_)) => Ok(true),
        Err(Raised::Throwable(e)) => Err(e),
    };
    jj_save(this, 23, xla)?;
    return __r2;
}

/// `jj_2_25(int)` of the generated Java.
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn jj_2_25<P: StandardSyntaxParserOps + ?Sized>(this: &mut P, mut xla: i32) -> Result<bool> {
    this.jj_mut().jj_la = xla;
    let __a1 = this.jj().token.clone();
    this.jj_mut().jj_scanpos = __a1.clone();
    this.jj_mut().jj_lastpos = __a1;
    let __r2: Result<bool> = match jj_3_25(this) {
        Ok(v) => Ok(!v),
        Err(Raised::LookaheadSuccess(_)) => Ok(true),
        Err(Raised::Throwable(e)) => Err(e),
    };
    jj_save(this, 24, xla)?;
    return __r2;
}

/// `jj_2_26(int)` of the generated Java.
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn jj_2_26<P: StandardSyntaxParserOps + ?Sized>(this: &mut P, mut xla: i32) -> Result<bool> {
    this.jj_mut().jj_la = xla;
    let __a1 = this.jj().token.clone();
    this.jj_mut().jj_scanpos = __a1.clone();
    this.jj_mut().jj_lastpos = __a1;
    let __r2: Result<bool> = match jj_3_26(this) {
        Ok(v) => Ok(!v),
        Err(Raised::LookaheadSuccess(_)) => Ok(true),
        Err(Raised::Throwable(e)) => Err(e),
    };
    jj_save(this, 25, xla)?;
    return __r2;
}

/// `jj_2_27(int)` of the generated Java.
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn jj_2_27<P: StandardSyntaxParserOps + ?Sized>(this: &mut P, mut xla: i32) -> Result<bool> {
    this.jj_mut().jj_la = xla;
    let __a1 = this.jj().token.clone();
    this.jj_mut().jj_scanpos = __a1.clone();
    this.jj_mut().jj_lastpos = __a1;
    let __r2: Result<bool> = match jj_3_27(this) {
        Ok(v) => Ok(!v),
        Err(Raised::LookaheadSuccess(_)) => Ok(true),
        Err(Raised::Throwable(e)) => Err(e),
    };
    jj_save(this, 26, xla)?;
    return __r2;
}

/// `jj_2_28(int)` of the generated Java.
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn jj_2_28<P: StandardSyntaxParserOps + ?Sized>(this: &mut P, mut xla: i32) -> Result<bool> {
    this.jj_mut().jj_la = xla;
    let __a1 = this.jj().token.clone();
    this.jj_mut().jj_scanpos = __a1.clone();
    this.jj_mut().jj_lastpos = __a1;
    let __r2: Result<bool> = match jj_3_28(this) {
        Ok(v) => Ok(!v),
        Err(Raised::LookaheadSuccess(_)) => Ok(true),
        Err(Raised::Throwable(e)) => Err(e),
    };
    jj_save(this, 27, xla)?;
    return __r2;
}

/// `jj_3R_IntervalUnorderedNoOverlaps_465_3_17()` of the generated Java.
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn jj_3_r_interval_unordered_no_overlaps_465_3_17<P: StandardSyntaxParserOps + ?Sized>(
    this: &mut P,
) -> std::result::Result<bool, Raised> {
    if jj_scan_token(this, qpc::FN_PREFIX)? {
        return Ok(true);
    }
    if jj_scan_token(this, qpc::UNORDERED_NO_OVERLAPS)? {
        return Ok(true);
    }
    return Ok(false);
}

/// `jj_3R_IntervalContainedBy_557_3_25()` of the generated Java.
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn jj_3_r_interval_contained_by_557_3_25<P: StandardSyntaxParserOps + ?Sized>(
    this: &mut P,
) -> std::result::Result<bool, Raised> {
    if jj_scan_token(this, qpc::FN_PREFIX)? {
        return Ok(true);
    }
    if jj_scan_token(this, qpc::CONTAINED_BY)? {
        return Ok(true);
    }
    return Ok(false);
}

/// `jj_3R_FieldRangeExpr_744_3_11()` of the generated Java.
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn jj_3_r_field_range_expr_744_3_11<P: StandardSyntaxParserOps + ?Sized>(
    this: &mut P,
) -> std::result::Result<bool, Raised> {
    if jj_3_r_field_name_344_3_9(this)? {
        return Ok(true);
    }
    let mut xsp: Option<TokenRef> = None;
    xsp = this.jj().jj_scanpos.clone();
    if jj_scan_token(this, 17)? {
        this.jj_mut().jj_scanpos = xsp.clone();
        if jj_scan_token(this, 18)? {
            this.jj_mut().jj_scanpos = xsp.clone();
            if jj_scan_token(this, 19)? {
                this.jj_mut().jj_scanpos = xsp.clone();
                if jj_scan_token(this, 20)? {
                    return Ok(true);
                }
            }
        }
    }
    return Ok(false);
}

/// `jj_3R_IntervalWildcard_647_3_19()` of the generated Java.
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn jj_3_r_interval_wildcard_647_3_19<P: StandardSyntaxParserOps + ?Sized>(
    this: &mut P,
) -> std::result::Result<bool, Raised> {
    if jj_scan_token(this, qpc::FN_PREFIX)? {
        return Ok(true);
    }
    if jj_scan_token(this, qpc::WILDCARD)? {
        return Ok(true);
    }
    return Ok(false);
}

/// `jj_3R_QuotedTerm_834_5_47()` of the generated Java.
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn jj_3_r_quoted_term_834_5_47<P: StandardSyntaxParserOps + ?Sized>(
    this: &mut P,
) -> std::result::Result<bool, Raised> {
    if jj_scan_token(this, qpc::TILDE)? {
        return Ok(true);
    }
    return Ok(false);
}

/// `jj_3R_IntervalUnordered_454_3_16()` of the generated Java.
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn jj_3_r_interval_unordered_454_3_16<P: StandardSyntaxParserOps + ?Sized>(
    this: &mut P,
) -> std::result::Result<bool, Raised> {
    if jj_scan_token(this, qpc::FN_PREFIX)? {
        return Ok(true);
    }
    if jj_scan_token(this, qpc::UNORDERED)? {
        return Ok(true);
    }
    return Ok(false);
}

/// `jj_3R_IntervalNotContaining_546_3_24()` of the generated Java.
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn jj_3_r_interval_not_containing_546_3_24<P: StandardSyntaxParserOps + ?Sized>(
    this: &mut P,
) -> std::result::Result<bool, Raised> {
    if jj_scan_token(this, qpc::FN_PREFIX)? {
        return Ok(true);
    }
    if jj_scan_token(this, qpc::NOT_CONTAINING)? {
        return Ok(true);
    }
    return Ok(false);
}

/// `jj_3R_QuotedTerm_829_3_43()` of the generated Java.
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn jj_3_r_quoted_term_829_3_43<P: StandardSyntaxParserOps + ?Sized>(
    this: &mut P,
) -> std::result::Result<bool, Raised> {
    if jj_scan_token(this, qpc::QUOTED)? {
        return Ok(true);
    }
    let mut xsp: Option<TokenRef> = None;
    xsp = this.jj().jj_scanpos.clone();
    if jj_3_r_quoted_term_834_5_47(this)? {
        this.jj_mut().jj_scanpos = xsp.clone();
    }
    return Ok(false);
}

/// `jj_3R_IntervalNonOverlapping_636_3_30()` of the generated Java.
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn jj_3_r_interval_non_overlapping_636_3_30<P: StandardSyntaxParserOps + ?Sized>(
    this: &mut P,
) -> std::result::Result<bool, Raised> {
    if jj_scan_token(this, qpc::FN_PREFIX)? {
        return Ok(true);
    }
    if jj_scan_token(this, qpc::NON_OVERLAPPING)? {
        return Ok(true);
    }
    return Ok(false);
}

/// `jj_3R_IntervalMaxGaps_442_3_14()` of the generated Java.
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn jj_3_r_interval_max_gaps_442_3_14<P: StandardSyntaxParserOps + ?Sized>(
    this: &mut P,
) -> std::result::Result<bool, Raised> {
    if jj_scan_token(this, qpc::FN_PREFIX)? {
        return Ok(true);
    }
    if jj_scan_token(this, qpc::MAXGAPS)? {
        return Ok(true);
    }
    return Ok(false);
}

/// `jj_3_28()` of the generated Java.
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn jj_3_28<P: StandardSyntaxParserOps + ?Sized>(this: &mut P) -> std::result::Result<bool, Raised> {
    if jj_scan_token(this, qpc::NUMBER)? {
        return Ok(true);
    }
    return Ok(false);
}

/// `jj_3R_IntervalContaining_535_3_23()` of the generated Java.
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn jj_3_r_interval_containing_535_3_23<P: StandardSyntaxParserOps + ?Sized>(
    this: &mut P,
) -> std::result::Result<bool, Raised> {
    if jj_scan_token(this, qpc::FN_PREFIX)? {
        return Ok(true);
    }
    if jj_scan_token(this, qpc::CONTAINING)? {
        return Ok(true);
    }
    return Ok(false);
}

/// `jj_3R_FieldName_344_3_9()` of the generated Java.
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn jj_3_r_field_name_344_3_9<P: StandardSyntaxParserOps + ?Sized>(
    this: &mut P,
) -> std::result::Result<bool, Raised> {
    if jj_scan_token(this, qpc::TERM)? {
        return Ok(true);
    }
    return Ok(false);
}

/// `jj_3R_IntervalOverlapping_626_3_29()` of the generated Java.
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn jj_3_r_interval_overlapping_626_3_29<P: StandardSyntaxParserOps + ?Sized>(
    this: &mut P,
) -> std::result::Result<bool, Raised> {
    if jj_scan_token(this, qpc::FN_PREFIX)? {
        return Ok(true);
    }
    if jj_scan_token(this, qpc::OVERLAPPING)? {
        return Ok(true);
    }
    return Ok(false);
}

/// `jj_3R_Term_806_10_41()` of the generated Java.
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn jj_3_r_term_806_10_41<P: StandardSyntaxParserOps + ?Sized>(
    this: &mut P,
) -> std::result::Result<bool, Raised> {
    if jj_3_r_fuzzy_op_717_3_45(this)? {
        return Ok(true);
    }
    return Ok(false);
}

/// `jj_3R_Term_808_8_37()` of the generated Java.
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn jj_3_r_term_808_8_37<P: StandardSyntaxParserOps + ?Sized>(
    this: &mut P,
) -> std::result::Result<bool, Raised> {
    if jj_3_r_quoted_term_829_3_43(this)? {
        return Ok(true);
    }
    return Ok(false);
}

/// `jj_3R_FuzzyOp_717_3_45()` of the generated Java.
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn jj_3_r_fuzzy_op_717_3_45<P: StandardSyntaxParserOps + ?Sized>(
    this: &mut P,
) -> std::result::Result<bool, Raised> {
    if jj_scan_token(this, qpc::TILDE)? {
        return Ok(true);
    }
    return Ok(false);
}

/// `jj_3R_Term_810_5_38()` of the generated Java.
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn jj_3_r_term_810_5_38<P: StandardSyntaxParserOps + ?Sized>(
    this: &mut P,
) -> std::result::Result<bool, Raised> {
    if jj_3_r_boost_700_3_44(this)? {
        return Ok(true);
    }
    return Ok(false);
}

/// `jj_3R_Term_807_8_36()` of the generated Java.
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn jj_3_r_term_807_8_36<P: StandardSyntaxParserOps + ?Sized>(
    this: &mut P,
) -> std::result::Result<bool, Raised> {
    if jj_3_r_term_range_expr_855_3_42(this)? {
        return Ok(true);
    }
    return Ok(false);
}

/// `jj_3_2()` of the generated Java.
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn jj_3_2<P: StandardSyntaxParserOps + ?Sized>(this: &mut P) -> std::result::Result<bool, Raised> {
    if jj_3_r_term_798_3_10(this)? {
        return Ok(true);
    }
    return Ok(false);
}

/// `jj_3R_IntervalMaxWidth_430_3_13()` of the generated Java.
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn jj_3_r_interval_max_width_430_3_13<P: StandardSyntaxParserOps + ?Sized>(
    this: &mut P,
) -> std::result::Result<bool, Raised> {
    if jj_scan_token(this, qpc::FN_PREFIX)? {
        return Ok(true);
    }
    if jj_scan_token(this, qpc::MAXWIDTH)? {
        return Ok(true);
    }
    return Ok(false);
}

/// `jj_3R_Term_804_8_35()` of the generated Java.
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn jj_3_r_term_804_8_35<P: StandardSyntaxParserOps + ?Sized>(
    this: &mut P,
) -> std::result::Result<bool, Raised> {
    let mut xsp: Option<TokenRef> = None;
    xsp = this.jj().jj_scanpos.clone();
    if jj_scan_token(this, 25)? {
        this.jj_mut().jj_scanpos = xsp.clone();
        if jj_scan_token(this, 24)? {
            return Ok(true);
        }
    }
    xsp = this.jj().jj_scanpos.clone();
    if jj_3_r_term_806_10_41(this)? {
        this.jj_mut().jj_scanpos = xsp.clone();
    }
    return Ok(false);
}

/// `jj_3R_IntervalAfter_524_3_20()` of the generated Java.
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn jj_3_r_interval_after_524_3_20<P: StandardSyntaxParserOps + ?Sized>(
    this: &mut P,
) -> std::result::Result<bool, Raised> {
    if jj_scan_token(this, qpc::FN_PREFIX)? {
        return Ok(true);
    }
    if jj_scan_token(this, qpc::AFTER)? {
        return Ok(true);
    }
    return Ok(false);
}

/// `jj_3_1()` of the generated Java.
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn jj_3_1<P: StandardSyntaxParserOps + ?Sized>(this: &mut P) -> std::result::Result<bool, Raised> {
    if jj_3_r_field_name_344_3_9(this)? {
        return Ok(true);
    }
    let mut xsp: Option<TokenRef> = None;
    xsp = this.jj().jj_scanpos.clone();
    if jj_scan_token(this, 15)? {
        this.jj_mut().jj_scanpos = xsp.clone();
        if jj_scan_token(this, 16)? {
            return Ok(true);
        }
    }
    return Ok(false);
}

/// `jj_3_3()` of the generated Java.
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn jj_3_3<P: StandardSyntaxParserOps + ?Sized>(this: &mut P) -> std::result::Result<bool, Raised> {
    if jj_3_r_field_range_expr_744_3_11(this)? {
        return Ok(true);
    }
    return Ok(false);
}

/// `jj_3R_Term_799_8_34()` of the generated Java.
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn jj_3_r_term_799_8_34<P: StandardSyntaxParserOps + ?Sized>(
    this: &mut P,
) -> std::result::Result<bool, Raised> {
    if jj_scan_token(this, qpc::REGEXPTERM)? {
        return Ok(true);
    }
    return Ok(false);
}

/// `jj_3R_IntervalNotWithin_611_3_28()` of the generated Java.
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn jj_3_r_interval_not_within_611_3_28<P: StandardSyntaxParserOps + ?Sized>(
    this: &mut P,
) -> std::result::Result<bool, Raised> {
    if jj_scan_token(this, qpc::FN_PREFIX)? {
        return Ok(true);
    }
    if jj_scan_token(this, qpc::NOT_WITHIN)? {
        return Ok(true);
    }
    return Ok(false);
}

/// `jj_3R_IntervalBefore_513_3_21()` of the generated Java.
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn jj_3_r_interval_before_513_3_21<P: StandardSyntaxParserOps + ?Sized>(
    this: &mut P,
) -> std::result::Result<bool, Raised> {
    if jj_scan_token(this, qpc::FN_PREFIX)? {
        return Ok(true);
    }
    if jj_scan_token(this, qpc::BEFORE)? {
        return Ok(true);
    }
    return Ok(false);
}

/// `jj_3R_Term_798_3_10()` of the generated Java.
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn jj_3_r_term_798_3_10<P: StandardSyntaxParserOps + ?Sized>(
    this: &mut P,
) -> std::result::Result<bool, Raised> {
    let mut xsp: Option<TokenRef> = None;
    xsp = this.jj().jj_scanpos.clone();
    if jj_3_r_term_799_8_34(this)? {
        this.jj_mut().jj_scanpos = xsp.clone();
        if jj_3_r_term_804_8_35(this)? {
            this.jj_mut().jj_scanpos = xsp.clone();
            if jj_3_r_term_807_8_36(this)? {
                this.jj_mut().jj_scanpos = xsp.clone();
                if jj_3_r_term_808_8_37(this)? {
                    return Ok(true);
                }
            }
        }
    }
    xsp = this.jj().jj_scanpos.clone();
    if jj_3_r_term_810_5_38(this)? {
        this.jj_mut().jj_scanpos = xsp.clone();
    }
    return Ok(false);
}

/// `jj_3R_IntervalAtLeast_418_3_12()` of the generated Java.
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn jj_3_r_interval_at_least_418_3_12<P: StandardSyntaxParserOps + ?Sized>(
    this: &mut P,
) -> std::result::Result<bool, Raised> {
    if jj_scan_token(this, qpc::FN_PREFIX)? {
        return Ok(true);
    }
    if jj_scan_token(this, qpc::ATLEAST)? {
        return Ok(true);
    }
    return Ok(false);
}

/// `jj_3R_Boost_700_3_44()` of the generated Java.
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn jj_3_r_boost_700_3_44<P: StandardSyntaxParserOps + ?Sized>(
    this: &mut P,
) -> std::result::Result<bool, Raised> {
    if jj_scan_token(this, qpc::CARAT)? {
        return Ok(true);
    }
    return Ok(false);
}

/// `jj_3_25()` of the generated Java.
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn jj_3_25<P: StandardSyntaxParserOps + ?Sized>(this: &mut P) -> std::result::Result<bool, Raised> {
    if jj_3_r_interval_text_685_5_33(this)? {
        return Ok(true);
    }
    return Ok(false);
}

/// `jj_3_24()` of the generated Java.
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn jj_3_24<P: StandardSyntaxParserOps + ?Sized>(this: &mut P) -> std::result::Result<bool, Raised> {
    if jj_3_r_interval_fuzzy_term_666_3_32(this)? {
        return Ok(true);
    }
    return Ok(false);
}

/// `jj_3_23()` of the generated Java.
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn jj_3_23<P: StandardSyntaxParserOps + ?Sized>(this: &mut P) -> std::result::Result<bool, Raised> {
    if jj_3_r_interval_extend_595_3_31(this)? {
        return Ok(true);
    }
    return Ok(false);
}

/// `jj_3_22()` of the generated Java.
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn jj_3_22<P: StandardSyntaxParserOps + ?Sized>(this: &mut P) -> std::result::Result<bool, Raised> {
    if jj_3_r_interval_non_overlapping_636_3_30(this)? {
        return Ok(true);
    }
    return Ok(false);
}

/// `jj_3_21()` of the generated Java.
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn jj_3_21<P: StandardSyntaxParserOps + ?Sized>(this: &mut P) -> std::result::Result<bool, Raised> {
    if jj_3_r_interval_overlapping_626_3_29(this)? {
        return Ok(true);
    }
    return Ok(false);
}

/// `jj_3R_IntervalPhrase_501_3_22()` of the generated Java.
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn jj_3_r_interval_phrase_501_3_22<P: StandardSyntaxParserOps + ?Sized>(
    this: &mut P,
) -> std::result::Result<bool, Raised> {
    if jj_scan_token(this, qpc::FN_PREFIX)? {
        return Ok(true);
    }
    if jj_scan_token(this, qpc::PHRASE)? {
        return Ok(true);
    }
    return Ok(false);
}

/// `jj_3_20()` of the generated Java.
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn jj_3_20<P: StandardSyntaxParserOps + ?Sized>(this: &mut P) -> std::result::Result<bool, Raised> {
    if jj_3_r_interval_not_within_611_3_28(this)? {
        return Ok(true);
    }
    return Ok(false);
}

/// `jj_3R_IntervalExtend_595_3_31()` of the generated Java.
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn jj_3_r_interval_extend_595_3_31<P: StandardSyntaxParserOps + ?Sized>(
    this: &mut P,
) -> std::result::Result<bool, Raised> {
    if jj_scan_token(this, qpc::FN_PREFIX)? {
        return Ok(true);
    }
    if jj_scan_token(this, qpc::EXTEND)? {
        return Ok(true);
    }
    return Ok(false);
}

/// `jj_3_19()` of the generated Java.
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn jj_3_19<P: StandardSyntaxParserOps + ?Sized>(this: &mut P) -> std::result::Result<bool, Raised> {
    if jj_3_r_interval_within_579_3_27(this)? {
        return Ok(true);
    }
    return Ok(false);
}

/// `jj_3_18()` of the generated Java.
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn jj_3_18<P: StandardSyntaxParserOps + ?Sized>(this: &mut P) -> std::result::Result<bool, Raised> {
    if jj_3_r_interval_not_contained_by_568_3_26(this)? {
        return Ok(true);
    }
    return Ok(false);
}

/// `jj_3R_IntervalText_686_5_40()` of the generated Java.
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn jj_3_r_interval_text_686_5_40<P: StandardSyntaxParserOps + ?Sized>(
    this: &mut P,
) -> std::result::Result<bool, Raised> {
    let mut xsp: Option<TokenRef> = None;
    xsp = this.jj().jj_scanpos.clone();
    if jj_scan_token(this, 25)? {
        this.jj_mut().jj_scanpos = xsp.clone();
        if jj_scan_token(this, 24)? {
            return Ok(true);
        }
    }
    return Ok(false);
}

/// `jj_3_17()` of the generated Java.
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn jj_3_17<P: StandardSyntaxParserOps + ?Sized>(this: &mut P) -> std::result::Result<bool, Raised> {
    if jj_3_r_interval_contained_by_557_3_25(this)? {
        return Ok(true);
    }
    return Ok(false);
}

/// `jj_3R_IntervalText_685_5_39()` of the generated Java.
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn jj_3_r_interval_text_685_5_39<P: StandardSyntaxParserOps + ?Sized>(
    this: &mut P,
) -> std::result::Result<bool, Raised> {
    if jj_scan_token(this, qpc::QUOTED)? {
        return Ok(true);
    }
    return Ok(false);
}

/// `jj_3R_IntervalText_685_5_33()` of the generated Java.
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn jj_3_r_interval_text_685_5_33<P: StandardSyntaxParserOps + ?Sized>(
    this: &mut P,
) -> std::result::Result<bool, Raised> {
    let mut xsp: Option<TokenRef> = None;
    xsp = this.jj().jj_scanpos.clone();
    if jj_3_r_interval_text_685_5_39(this)? {
        this.jj_mut().jj_scanpos = xsp.clone();
        if jj_3_r_interval_text_686_5_40(this)? {
            return Ok(true);
        }
    }
    return Ok(false);
}

/// `jj_3_16()` of the generated Java.
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn jj_3_16<P: StandardSyntaxParserOps + ?Sized>(this: &mut P) -> std::result::Result<bool, Raised> {
    if jj_3_r_interval_not_containing_546_3_24(this)? {
        return Ok(true);
    }
    return Ok(false);
}

/// `jj_3_15()` of the generated Java.
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn jj_3_15<P: StandardSyntaxParserOps + ?Sized>(this: &mut P) -> std::result::Result<bool, Raised> {
    if jj_3_r_interval_containing_535_3_23(this)? {
        return Ok(true);
    }
    return Ok(false);
}

/// `jj_3_14()` of the generated Java.
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn jj_3_14<P: StandardSyntaxParserOps + ?Sized>(this: &mut P) -> std::result::Result<bool, Raised> {
    if jj_3_r_interval_phrase_501_3_22(this)? {
        return Ok(true);
    }
    return Ok(false);
}

/// `jj_3_13()` of the generated Java.
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn jj_3_13<P: StandardSyntaxParserOps + ?Sized>(this: &mut P) -> std::result::Result<bool, Raised> {
    if jj_3_r_interval_before_513_3_21(this)? {
        return Ok(true);
    }
    return Ok(false);
}

/// `jj_3_12()` of the generated Java.
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn jj_3_12<P: StandardSyntaxParserOps + ?Sized>(this: &mut P) -> std::result::Result<bool, Raised> {
    if jj_3_r_interval_after_524_3_20(this)? {
        return Ok(true);
    }
    return Ok(false);
}

/// `jj_3_11()` of the generated Java.
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn jj_3_11<P: StandardSyntaxParserOps + ?Sized>(this: &mut P) -> std::result::Result<bool, Raised> {
    if jj_3_r_interval_wildcard_647_3_19(this)? {
        return Ok(true);
    }
    return Ok(false);
}

/// `jj_3_10()` of the generated Java.
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn jj_3_10<P: StandardSyntaxParserOps + ?Sized>(this: &mut P) -> std::result::Result<bool, Raised> {
    if jj_3_r_interval_or_489_3_18(this)? {
        return Ok(true);
    }
    return Ok(false);
}

/// `jj_3_9()` of the generated Java.
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn jj_3_9<P: StandardSyntaxParserOps + ?Sized>(this: &mut P) -> std::result::Result<bool, Raised> {
    if jj_3_r_interval_unordered_no_overlaps_465_3_17(this)? {
        return Ok(true);
    }
    return Ok(false);
}

/// `jj_3R_IntervalOr_489_3_18()` of the generated Java.
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn jj_3_r_interval_or_489_3_18<P: StandardSyntaxParserOps + ?Sized>(
    this: &mut P,
) -> std::result::Result<bool, Raised> {
    if jj_scan_token(this, qpc::FN_PREFIX)? {
        return Ok(true);
    }
    if jj_scan_token(this, qpc::FN_OR)? {
        return Ok(true);
    }
    return Ok(false);
}

/// `jj_3_8()` of the generated Java.
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn jj_3_8<P: StandardSyntaxParserOps + ?Sized>(this: &mut P) -> std::result::Result<bool, Raised> {
    if jj_3_r_interval_unordered_454_3_16(this)? {
        return Ok(true);
    }
    return Ok(false);
}

/// `jj_3_7()` of the generated Java.
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn jj_3_7<P: StandardSyntaxParserOps + ?Sized>(this: &mut P) -> std::result::Result<bool, Raised> {
    if jj_3_r_interval_ordered_477_3_15(this)? {
        return Ok(true);
    }
    return Ok(false);
}

/// `jj_3_6()` of the generated Java.
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn jj_3_6<P: StandardSyntaxParserOps + ?Sized>(this: &mut P) -> std::result::Result<bool, Raised> {
    if jj_3_r_interval_max_gaps_442_3_14(this)? {
        return Ok(true);
    }
    return Ok(false);
}

/// `jj_3_5()` of the generated Java.
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn jj_3_5<P: StandardSyntaxParserOps + ?Sized>(this: &mut P) -> std::result::Result<bool, Raised> {
    if jj_3_r_interval_max_width_430_3_13(this)? {
        return Ok(true);
    }
    return Ok(false);
}

/// `jj_3_4()` of the generated Java.
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn jj_3_4<P: StandardSyntaxParserOps + ?Sized>(this: &mut P) -> std::result::Result<bool, Raised> {
    if jj_3_r_interval_at_least_418_3_12(this)? {
        return Ok(true);
    }
    return Ok(false);
}

/// `jj_3_27()` of the generated Java.
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn jj_3_27<P: StandardSyntaxParserOps + ?Sized>(this: &mut P) -> std::result::Result<bool, Raised> {
    if jj_scan_token(this, qpc::NUMBER)? {
        return Ok(true);
    }
    return Ok(false);
}

/// `jj_3R_IntervalWithin_579_3_27()` of the generated Java.
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn jj_3_r_interval_within_579_3_27<P: StandardSyntaxParserOps + ?Sized>(
    this: &mut P,
) -> std::result::Result<bool, Raised> {
    if jj_scan_token(this, qpc::FN_PREFIX)? {
        return Ok(true);
    }
    if jj_scan_token(this, qpc::WITHIN)? {
        return Ok(true);
    }
    return Ok(false);
}

/// `jj_3_26()` of the generated Java.
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn jj_3_26<P: StandardSyntaxParserOps + ?Sized>(this: &mut P) -> std::result::Result<bool, Raised> {
    if jj_scan_token(this, qpc::NUMBER)? {
        return Ok(true);
    }
    return Ok(false);
}

/// `jj_3R_TermRangeExpr_856_6_46()` of the generated Java.
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn jj_3_r_term_range_expr_856_6_46<P: StandardSyntaxParserOps + ?Sized>(
    this: &mut P,
) -> std::result::Result<bool, Raised> {
    if jj_scan_token(this, qpc::RANGEIN_START)? {
        return Ok(true);
    }
    return Ok(false);
}

/// `jj_3R_IntervalOrdered_477_3_15()` of the generated Java.
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn jj_3_r_interval_ordered_477_3_15<P: StandardSyntaxParserOps + ?Sized>(
    this: &mut P,
) -> std::result::Result<bool, Raised> {
    if jj_scan_token(this, qpc::FN_PREFIX)? {
        return Ok(true);
    }
    if jj_scan_token(this, qpc::ORDERED)? {
        return Ok(true);
    }
    return Ok(false);
}

/// `jj_3R_IntervalFuzzyTerm_666_3_32()` of the generated Java.
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn jj_3_r_interval_fuzzy_term_666_3_32<P: StandardSyntaxParserOps + ?Sized>(
    this: &mut P,
) -> std::result::Result<bool, Raised> {
    if jj_scan_token(this, qpc::FN_PREFIX)? {
        return Ok(true);
    }
    if jj_scan_token(this, qpc::FUZZYTERM)? {
        return Ok(true);
    }
    return Ok(false);
}

/// `jj_3R_TermRangeExpr_855_3_42()` of the generated Java.
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn jj_3_r_term_range_expr_855_3_42<P: StandardSyntaxParserOps + ?Sized>(
    this: &mut P,
) -> std::result::Result<bool, Raised> {
    let mut xsp: Option<TokenRef> = None;
    xsp = this.jj().jj_scanpos.clone();
    if jj_3_r_term_range_expr_856_6_46(this)? {
        this.jj_mut().jj_scanpos = xsp.clone();
        if jj_scan_token(this, 28)? {
            return Ok(true);
        }
    }
    xsp = this.jj().jj_scanpos.clone();
    if jj_scan_token(this, 55)? {
        this.jj_mut().jj_scanpos = xsp.clone();
        if jj_scan_token(this, 54)? {
            this.jj_mut().jj_scanpos = xsp.clone();
            if jj_scan_token(this, 51)? {
                return Ok(true);
            }
        }
    }
    return Ok(false);
}

/// `jj_3R_IntervalNotContainedBy_568_3_26()` of the generated Java.
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn jj_3_r_interval_not_contained_by_568_3_26<P: StandardSyntaxParserOps + ?Sized>(
    this: &mut P,
) -> std::result::Result<bool, Raised> {
    if jj_scan_token(this, qpc::FN_PREFIX)? {
        return Ok(true);
    }
    if jj_scan_token(this, qpc::NOT_CONTAINED_BY)? {
        return Ok(true);
    }
    return Ok(false);
}

/// `ReInit(CharStream)` of the generated Java.
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
pub(crate) fn re_init<P: StandardSyntaxParserOps + ?Sized>(
    this: &mut P,
    mut stream: Box<dyn CharStream>,
) {
    this.jj_mut().token_source.re_init(stream);
    this.jj_mut().token = Some(TokenRef::new(Token::new()));
    this.jj_mut().jj_ntk = -1;
    this.jj_mut().jj_gen = 0;
    {
        let mut i: i32 = 0;
        'l1: loop {
            if !(i < 34) {
                break;
            }
            'l1_body: {
                let __i1 = (i) as usize;
                this.jj_mut().jj_la1[__i1] = -1;
            }
            i = i.wrapping_add(1);
        }
    }
    {
        let mut i: i32 = 0;
        'l2: loop {
            if !(i < (this.jj().jj_2_rtns.len() as i32)) {
                break;
            }
            'l2_body: {
                let __i2 = (i) as usize;
                this.jj_mut().jj_2_rtns[__i2] = Some(JJCallsRef::new(JJCalls::default()));
            }
            i = i.wrapping_add(1);
        }
    }
}

/// `jj_consume_token(int)` of the generated Java.
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn jj_consume_token<P: StandardSyntaxParserOps + ?Sized>(
    this: &mut P,
    mut kind: i32,
) -> Result<Option<TokenRef>> {
    let mut old_token: Option<TokenRef> = None;
    let __a1 = this.jj().token.clone();
    old_token = __a1.clone();
    if (crate::javacc::nn((__a1).as_ref())?.read(|o| o.next.clone())).is_some() {
        this.jj_mut().token = crate::javacc::nn(this.jj().token.as_ref())?.read(|o| o.next.clone());
    } else {
        let __o2 = this.jj().token.clone();
        let __a3 = this.jj_mut().token_source.get_next_token()?;
        crate::javacc::nn(__o2.as_ref())?.lock().next = __a3.clone();
        this.jj_mut().token = __a3;
    }
    this.jj_mut().jj_ntk = -1;
    if (crate::javacc::nn(this.jj().token.as_ref())?.read(|o| o.kind)) == kind {
        this.jj_mut().jj_gen = this.jj().jj_gen.wrapping_add(1);
        this.jj_mut().jj_gc = this.jj().jj_gc.wrapping_add(1);
        if this.jj().jj_gc > 100 {
            this.jj_mut().jj_gc = 0;
            {
                let mut i: i32 = 0;
                'l1: loop {
                    if !(i < (this.jj().jj_2_rtns.len() as i32)) {
                        break;
                    }
                    'l1_body: {
                        let mut c: Option<JJCallsRef> = this.jj().jj_2_rtns[(i) as usize].clone();
                        'l2: loop {
                            if !(c.clone().is_some()) {
                                break;
                            }
                            if (crate::javacc::nn(c.as_ref())?.read(|o| o.gen)) < this.jj().jj_gen {
                                let __o4 = c.clone();
                                let __a5 = None;
                                crate::javacc::nn(__o4.as_ref())?.lock().first = __a5;
                            }
                            c = crate::javacc::nn(c.as_ref())?.read(|o| o.next.clone());
                        }
                    }
                    i = i.wrapping_add(1);
                }
            }
        }
        return Ok(this.jj().token.clone());
    }
    this.jj_mut().token = old_token.clone();
    this.jj_mut().jj_kind = kind;
    return Err(this.generate_parse_exception()?.error().into());
}

/// `jj_scan_token(int)` of the generated Java.
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn jj_scan_token<P: StandardSyntaxParserOps + ?Sized>(
    this: &mut P,
    mut kind: i32,
) -> std::result::Result<bool, Raised> {
    if crate::javacc::opt_ptr_eq(&this.jj().jj_scanpos.clone(), &this.jj().jj_lastpos.clone()) {
        this.jj_mut().jj_la = this.jj().jj_la.wrapping_sub(1);
        if (crate::javacc::nn(this.jj().jj_scanpos.as_ref())?.read(|o| o.next.clone())).is_none() {
            let __o1 = this.jj().jj_scanpos.clone();
            let __a2 = this.jj_mut().token_source.get_next_token()?;
            crate::javacc::nn(__o1.as_ref())?.lock().next = __a2.clone();
            let __a3 = __a2;
            this.jj_mut().jj_scanpos = __a3.clone();
            this.jj_mut().jj_lastpos = __a3;
        } else {
            let __a4 = crate::javacc::nn(this.jj().jj_scanpos.as_ref())?.read(|o| o.next.clone());
            this.jj_mut().jj_scanpos = __a4.clone();
            this.jj_mut().jj_lastpos = __a4;
        }
    } else {
        this.jj_mut().jj_scanpos =
            crate::javacc::nn(this.jj().jj_scanpos.as_ref())?.read(|o| o.next.clone());
    }
    if this.jj().jj_rescan {
        let mut i: i32 = 0;
        let mut tok: Option<TokenRef> = this.jj().token.clone();
        'l1: loop {
            if !(tok.clone().is_some()
                && (!crate::javacc::opt_ptr_eq(&tok.clone(), &this.jj().jj_scanpos.clone())))
            {
                break;
            }
            i = i.wrapping_add(1);
            tok = crate::javacc::nn(tok.as_ref())?.read(|o| o.next.clone());
        }
        if tok.clone().is_some() {
            jj_add_error_token(this, kind, i)?;
        }
    }
    if (crate::javacc::nn(this.jj().jj_scanpos.as_ref())?.read(|o| o.kind)) != kind {
        return Ok(true);
    }
    if (this.jj().jj_la == 0)
        && (crate::javacc::opt_ptr_eq(&this.jj().jj_scanpos.clone(), &this.jj().jj_lastpos.clone()))
    {
        return Err(Raised::LookaheadSuccess(LookaheadSuccess));
    }
    return Ok(false);
}

/// `jj_ntk_f()` of the generated Java.
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn jj_ntk_f<P: StandardSyntaxParserOps + ?Sized>(this: &mut P) -> Result<i32> {
    let __a1 = crate::javacc::nn(this.jj().token.as_ref())?.read(|o| o.next.clone());
    this.jj_mut().jj_nt = __a1.clone();
    if (__a1).is_none() {
        let __o2 = this.jj().token.clone();
        let __a3 = this.jj_mut().token_source.get_next_token()?;
        crate::javacc::nn(__o2.as_ref())?.lock().next = __a3.clone();
        let __a4 = crate::javacc::nn((__a3).as_ref())?.read(|o| o.kind);
        this.jj_mut().jj_ntk = __a4;
        return Ok((__a4));
    } else {
        let __a5 = crate::javacc::nn(this.jj().jj_nt.as_ref())?.read(|o| o.kind);
        this.jj_mut().jj_ntk = __a5;
        return Ok((__a5));
    }
}

/// `jj_add_error_token(int, int)` of the generated Java.
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn jj_add_error_token<P: StandardSyntaxParserOps + ?Sized>(
    this: &mut P,
    mut kind: i32,
    mut pos: i32,
) -> Result<()> {
    if pos >= 100 {
        return Ok(());
    }
    if pos == this.jj().jj_endpos.wrapping_add(1) {
        let __v1 = this.jj().jj_endpos;
        this.jj_mut().jj_endpos = __v1.wrapping_add(1);
        let __i2 = (__v1) as usize;
        this.jj_mut().jj_lasttokens[__i2] = kind;
    } else {
        if this.jj().jj_endpos != 0 {
            this.jj_mut().jj_expentry = Some(vec![0; (this.jj().jj_endpos) as usize]);
            {
                let mut i: i32 = 0;
                'l1: loop {
                    if !(i < this.jj().jj_endpos) {
                        break;
                    }
                    'l1_body: {
                        let __i3 = (i) as usize;
                        crate::javacc::nn_mut(this.jj_mut().jj_expentry.as_mut())?[__i3] =
                            this.jj().jj_lasttokens[(i) as usize];
                    }
                    i = i.wrapping_add(1);
                }
            }
            let mut __k4 = 0usize;
            'l2: loop {
                if __k4 >= this.jj_mut().jj_expentries.len() {
                    break;
                }
                let oldentry: Vec<i32> = this.jj_mut().jj_expentries[__k4].clone();
                __k4 += 1;
                if (oldentry.len() as i32)
                    == (crate::javacc::nn(this.jj().jj_expentry.as_ref())?.len() as i32)
                {
                    let mut is_matched: bool = true;
                    {
                        let mut i: i32 = 0;
                        'l3: loop {
                            if !(i
                                < (crate::javacc::nn(this.jj().jj_expentry.as_ref())?.len() as i32))
                            {
                                break;
                            }
                            'l3_body: {
                                if (oldentry[(i) as usize])
                                    != (crate::javacc::nn(this.jj().jj_expentry.as_ref())?
                                        [(i) as usize])
                                {
                                    is_matched = false;
                                    break 'l3;
                                }
                            }
                            i = i.wrapping_add(1);
                        }
                    }
                    if is_matched {
                        let __p5 = crate::javacc::nn_owned(this.jj().jj_expentry.clone())?;
                        this.jj_mut().jj_expentries.push(__p5);
                        break 'l2;
                    }
                }
            }
            if pos != 0 {
                let __a6 = pos;
                this.jj_mut().jj_endpos = __a6;
                let __i7 = ((__a6).wrapping_sub(1)) as usize;
                this.jj_mut().jj_lasttokens[__i7] = kind;
            }
        }
    }
    Ok(())
}

/// `jj_rescan_token()` of the generated Java.
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn jj_rescan_token<P: StandardSyntaxParserOps + ?Sized>(this: &mut P) -> Result<()> {
    this.jj_mut().jj_rescan = true;
    {
        let mut i: i32 = 0;
        'l1: loop {
            if !(i < 28) {
                break;
            }
            'l1_body: {
                let __caught1: Option<Raised> = (|| -> std::result::Result<(), Raised> {
                    let mut p: Option<JJCallsRef> = this.jj().jj_2_rtns[(i) as usize].clone();
                    'l2: loop {
                        'l2_body: {
                            if (crate::javacc::nn(p.as_ref())?.read(|o| o.gen)) > this.jj().jj_gen {
                                this.jj_mut().jj_la =
                                    crate::javacc::nn(p.as_ref())?.read(|o| o.arg);
                                let __a2 = crate::javacc::nn(p.as_ref())?.read(|o| o.first.clone());
                                this.jj_mut().jj_scanpos = __a2.clone();
                                this.jj_mut().jj_lastpos = __a2;
                                'sw3: {
                                    match i {
                                        0 => {
                                            let _ = jj_3_1(this)?;
                                        }
                                        1 => {
                                            let _ = jj_3_2(this)?;
                                        }
                                        2 => {
                                            let _ = jj_3_3(this)?;
                                        }
                                        3 => {
                                            let _ = jj_3_4(this)?;
                                        }
                                        4 => {
                                            let _ = jj_3_5(this)?;
                                        }
                                        5 => {
                                            let _ = jj_3_6(this)?;
                                        }
                                        6 => {
                                            let _ = jj_3_7(this)?;
                                        }
                                        7 => {
                                            let _ = jj_3_8(this)?;
                                        }
                                        8 => {
                                            let _ = jj_3_9(this)?;
                                        }
                                        9 => {
                                            let _ = jj_3_10(this)?;
                                        }
                                        10 => {
                                            let _ = jj_3_11(this)?;
                                        }
                                        11 => {
                                            let _ = jj_3_12(this)?;
                                        }
                                        12 => {
                                            let _ = jj_3_13(this)?;
                                        }
                                        13 => {
                                            let _ = jj_3_14(this)?;
                                        }
                                        14 => {
                                            let _ = jj_3_15(this)?;
                                        }
                                        15 => {
                                            let _ = jj_3_16(this)?;
                                        }
                                        16 => {
                                            let _ = jj_3_17(this)?;
                                        }
                                        17 => {
                                            let _ = jj_3_18(this)?;
                                        }
                                        18 => {
                                            let _ = jj_3_19(this)?;
                                        }
                                        19 => {
                                            let _ = jj_3_20(this)?;
                                        }
                                        20 => {
                                            let _ = jj_3_21(this)?;
                                        }
                                        21 => {
                                            let _ = jj_3_22(this)?;
                                        }
                                        22 => {
                                            let _ = jj_3_23(this)?;
                                        }
                                        23 => {
                                            let _ = jj_3_24(this)?;
                                        }
                                        24 => {
                                            let _ = jj_3_25(this)?;
                                        }
                                        25 => {
                                            let _ = jj_3_26(this)?;
                                        }
                                        26 => {
                                            let _ = jj_3_27(this)?;
                                        }
                                        27 => {
                                            let _ = jj_3_28(this)?;
                                        }
                                        _ => {}
                                    }
                                }
                            }
                            p = crate::javacc::nn(p.as_ref())?.read(|o| o.next.clone());
                        }
                        if !(p.clone().is_some()) {
                            break;
                        }
                    }
                    Ok(())
                })()
                .err();
                if let Some(__e) = __caught1 {
                    if matches!(__e, Raised::LookaheadSuccess(_)) {
                    } else {
                        if let Raised::Throwable(t) = __e {
                            return Err(t);
                        }
                    }
                }
            }
            i = i.wrapping_add(1);
        }
    }
    this.jj_mut().jj_rescan = false;
    Ok(())
}

/// `jj_save(int, int)` of the generated Java.
#[allow(
    unused_mut,
    unused_variables,
    unused_parens,
    unused_assignments,
    unused_labels,
    clippy::all
)]
fn jj_save<P: StandardSyntaxParserOps + ?Sized>(
    this: &mut P,
    mut index: i32,
    mut xla: i32,
) -> Result<()> {
    let mut p: Option<JJCallsRef> = this.jj().jj_2_rtns[(index) as usize].clone();
    'l1: loop {
        if !((crate::javacc::nn(p.as_ref())?.read(|o| o.gen)) > this.jj().jj_gen) {
            break;
        }
        if (crate::javacc::nn(p.as_ref())?.read(|o| o.next.clone())).is_none() {
            let __o1 = p.clone();
            let __a2 = Some(JJCallsRef::new(JJCalls::default()));
            crate::javacc::nn(__o1.as_ref())?.lock().next = __a2.clone();
            p = __a2;
            break 'l1;
        }
        p = crate::javacc::nn(p.as_ref())?.read(|o| o.next.clone());
    }
    let __o3 = p.clone();
    let __a4 = this
        .jj()
        .jj_gen
        .wrapping_add(xla)
        .wrapping_sub(this.jj().jj_la);
    crate::javacc::nn(__o3.as_ref())?.lock().gen = __a4;
    let __o5 = p.clone();
    let __a6 = this.jj().token.clone();
    crate::javacc::nn(__o5.as_ref())?.lock().first = __a6;
    let __o7 = p.clone();
    let __a8 = xla;
    crate::javacc::nn(__o7.as_ref())?.lock().arg = __a8;
    Ok(())
}
// @generated end private
