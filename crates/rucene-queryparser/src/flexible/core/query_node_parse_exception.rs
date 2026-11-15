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

//! Port of `QueryNodeParseException.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.core`.

use std::any::Any;
use std::sync::{Arc, PoisonError, RwLock};

use rucene::error::{JavaThrowable, LuceneError, LuceneErrorKind};

use super::messages::query_parser_messages::QueryParserMessages;
use super::query_node_exception::QueryNodeException;
use crate::flexible::messages::message::Message;
use crate::flexible::messages::message_impl::MessageImpl;
use crate::flexible::messages::nls_exception::NLSException;
use crate::jdk::char_sequence::CharSequence;
use crate::jdk::object::JavaObject;

/// The binary name of `QueryNodeParseException`.
pub const QUERY_NODE_PARSE_EXCEPTION_CLASS: &str =
    "org.apache.lucene.queryparser.flexible.core.QueryNodeParseException";

/// The view of a subclass of `QueryNodeParseException` declared outside
/// this module (the flexible standard parser's `ParseException`).
pub type QueryNodeParseExceptionView =
    for<'a> fn(&'a dyn Any) -> Option<&'a QueryNodeParseException>;

/// The registered subclass views.
static SUBCLASS_VIEWS: RwLock<Vec<QueryNodeParseExceptionView>> = RwLock::new(Vec::new());

/// This should be thrown when an exception happens during the query parsing
/// from string to the query node tree.
///
/// Equivalent to
/// `org.apache.lucene.queryparser.flexible.core.QueryNodeParseException`,
/// which extends `QueryNodeException` (embedded here and reached through
/// [`query_node_exception`](Self::query_node_exception)); it is thrown as a
/// [`LuceneError`] of kind [`LuceneErrorKind::Throwable`].
#[derive(Debug, Clone)]
pub struct QueryNodeParseException {
    base: QueryNodeException,
    query: Option<CharSequence>,
    begin_column: i32,
    begin_line: i32,
    error_token: String,
}

impl QueryNodeParseException {
    fn of_base(mut base: QueryNodeException) -> Self {
        base.set_java_class(QUERY_NODE_PARSE_EXCEPTION_CLASS);
        Self {
            base,
            query: None,
            begin_column: -1,
            begin_line: -1,
            error_token: String::new(),
        }
    }

    /// Equivalent to `new QueryNodeParseException(Message message)`.
    pub fn new(message: Arc<dyn Message>) -> Self {
        Self::of_base(QueryNodeException::new(message))
    }

    /// Equivalent to `new QueryNodeParseException(Throwable throwable)`.
    pub fn with_cause(throwable: LuceneError) -> Self {
        Self::of_base(QueryNodeException::with_cause(throwable))
    }

    /// Equivalent to `new QueryNodeParseException(Message message, Throwable
    /// throwable)`.
    pub fn with_message_and_cause(message: Arc<dyn Message>, throwable: LuceneError) -> Self {
        Self::of_base(QueryNodeException::with_message_and_cause(
            message, throwable,
        ))
    }

    /// The `QueryNodeException` this exception extends.
    pub fn query_node_exception(&self) -> &QueryNodeException {
        &self.base
    }

    /// The `QueryNodeException` this exception extends, for writing its
    /// protected state.
    pub fn query_node_exception_mut(&mut self) -> &mut QueryNodeException {
        &mut self.base
    }

    /// Equivalent to `setQuery(CharSequence query)`: also replaces the
    /// message by `INVALID_SYNTAX_CANNOT_PARSE` with the query and `""`.
    pub fn set_query(&mut self, query: Option<CharSequence>) {
        self.base.set_message(Arc::new(MessageImpl::with_args(
            QueryParserMessages::INVALID_SYNTAX_CANNOT_PARSE.get(),
            vec![JavaObject::from(query.as_ref()), JavaObject::from("")],
        )));
        self.query = query;
    }

    /// Equivalent to `getQuery()`.
    pub fn get_query(&self) -> Option<&CharSequence> {
        self.query.as_ref()
    }

    /// Equivalent to the protected `setErrorToken(String errorToken)`.
    pub fn set_error_token(&mut self, error_token: impl Into<String>) {
        self.error_token = error_token.into();
    }

    /// Equivalent to `getErrorToken()`.
    pub fn get_error_token(&self) -> &str {
        &self.error_token
    }

    /// Equivalent to `setNonLocalizedMessage(Message message)`.
    pub fn set_non_localized_message(&mut self, message: Arc<dyn Message>) {
        self.base.set_message(message);
    }

    /// For EndOfLine and EndOfFile ("&lt;EOF&gt;") parsing problems the last
    /// char in the string is returned For the case where the parser is not
    /// able to figure out the line and column number -1 will be returned
    ///
    /// Equivalent to `getBeginLine()`.
    pub fn get_begin_line(&self) -> i32 {
        self.begin_line
    }

    /// For EndOfLine and EndOfFile ("&lt;EOF&gt;") parsing problems the last
    /// char in the string is returned For the case where the parser is not
    /// able to figure out the line and column number -1 will be returned
    ///
    /// Equivalent to `getBeginColumn()`.
    pub fn get_begin_column(&self) -> i32 {
        self.begin_column
    }

    /// Equivalent to the protected `setBeginLine(int beginLine)`.
    pub fn set_begin_line(&mut self, begin_line: i32) {
        self.begin_line = begin_line;
    }

    /// Equivalent to the protected `setBeginColumn(int beginColumn)`.
    pub fn set_begin_column(&mut self, begin_column: i32) {
        self.begin_column = begin_column;
    }

    /// This exception as the [`LuceneError`] it is thrown as.
    pub fn error(self) -> LuceneError {
        LuceneError::from(LuceneErrorKind::Throwable(Arc::new(self)))
    }

    /// The `QueryNodeParseException` part of `error`, when it is one or one
    /// of its subclasses (`catch (QueryNodeParseException e)`).
    pub fn of(error: &LuceneError) -> Option<&QueryNodeParseException> {
        match error.kind() {
            LuceneErrorKind::Throwable(t) => Self::view(t.as_any()),
            _ => None,
        }
    }

    /// The `QueryNodeParseException` part of an exception object.
    pub(crate) fn view(any: &dyn Any) -> Option<&QueryNodeParseException> {
        if let Some(e) = any.downcast_ref::<QueryNodeParseException>() {
            return Some(e);
        }
        let views = SUBCLASS_VIEWS
            .read()
            .unwrap_or_else(PoisonError::into_inner);
        views.iter().find_map(|view| view(any))
    }

    /// Registers a subclass of `QueryNodeParseException` declared outside
    /// this module, so that [`of`](Self::of) and
    /// [`QueryNodeException::of`] see through it.
    pub fn register_subclass(view: QueryNodeParseExceptionView) {
        SUBCLASS_VIEWS
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .push(view);
    }
}

impl NLSException for QueryNodeParseException {
    fn get_message_object(&self) -> Option<Arc<dyn Message>> {
        self.base.get_message_object()
    }
}

impl JavaThrowable for QueryNodeParseException {
    fn java_class(&self) -> &'static str {
        self.base.java_class()
    }

    fn get_message(&self) -> Option<&str> {
        self.base.get_message()
    }

    fn get_cause(&self) -> Option<&Arc<LuceneError>> {
        self.base.get_cause()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
