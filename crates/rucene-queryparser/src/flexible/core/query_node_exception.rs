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

//! Port of `QueryNodeException.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.core`.

use std::any::Any;
use std::sync::{Arc, OnceLock, PoisonError, RwLock};

use rucene::error::{JavaThrowable, LuceneError, LuceneErrorKind, Result};

use super::messages::query_parser_messages::QueryParserMessages;
use super::query_node_parse_exception::QueryNodeParseException;
use crate::flexible::messages::message::Message;
use crate::flexible::messages::message_impl::MessageImpl;
use crate::flexible::messages::nls_exception::NLSException;
use crate::jdk::locale::Locale;

/// The binary name of `QueryNodeException`.
pub const QUERY_NODE_EXCEPTION_CLASS: &str =
    "org.apache.lucene.queryparser.flexible.core.QueryNodeException";

/// The view of a subclass of `QueryNodeException` declared outside this
/// module: the `QueryNodeException` part of an exception, `None` when the
/// exception is not of that subclass.
pub type QueryNodeExceptionView = for<'a> fn(&'a dyn Any) -> Option<&'a QueryNodeException>;

/// The registered subclass views (see [`QueryNodeException::register_subclass`]).
static SUBCLASS_VIEWS: RwLock<Vec<QueryNodeExceptionView>> = RwLock::new(Vec::new());

/// `Throwable.toString()` of a thrown error: the class then `": " +
/// getLocalizedMessage()`, or the overriding `toString()` of a
/// `QueryNodeException`.
pub fn throwable_to_string(error: &LuceneError) -> String {
    if let Some(e) = QueryNodeException::of(error) {
        if let Ok(s) = e.to_java_string() {
            return s;
        }
    }
    let with = |class: &str, message: &str| format!("{class}: {message}");
    match error.kind() {
        LuceneErrorKind::Throwable(t) => match t.get_message() {
            Some(m) => with(t.java_class(), m),
            None => t.java_class().to_string(),
        },
        LuceneErrorKind::Unchecked { class, message, .. } => match message {
            Some(m) => with(class, m),
            None => class.to_string(),
        },
        LuceneErrorKind::IllegalArgument { message, .. } => {
            with("java.lang.IllegalArgumentException", message)
        }
        LuceneErrorKind::IllegalState { message, .. } => {
            with("java.lang.IllegalStateException", message)
        }
        LuceneErrorKind::UnsupportedOperation { message, .. } => {
            with("java.lang.UnsupportedOperationException", message)
        }
        LuceneErrorKind::StringIndexOutOfBounds(message) => {
            with("java.lang.StringIndexOutOfBoundsException", message)
        }
        LuceneErrorKind::Arithmetic(message) => with("java.lang.ArithmeticException", message),
        LuceneErrorKind::AssertionError(message) => with("java.lang.AssertionError", message),
        LuceneErrorKind::Io(e) => rucene::store::java_nio_file::java_to_string(e),
        other => format!("{}: {other}", other.java_simple_name()),
    }
}

/// This exception should be thrown if something wrong happens when dealing
/// with [`QueryNode`](super::nodes::query_node::QueryNode)s.
///
/// It also supports NLS messages.
///
/// Equivalent to `org.apache.lucene.queryparser.flexible.core.QueryNodeException`,
/// a checked `Exception` that implements `NLSException`; it is thrown as a
/// [`LuceneError`] of kind [`LuceneErrorKind::Throwable`]. A subclass embeds
/// it and sets its own class name ([`set_java_class`](Self::set_java_class)).
///
/// **Divergences.** Java's `getMessage()` localizes the message with the
/// default locale at each call; the port localizes it on the first call
/// ([`JavaThrowable::get_message`]) and keeps the text, so a later change of
/// the default locale is not reflected (the messages of
/// `QueryParserMessages` exist in one locale only, so none of them changes);
/// a failing lookup, which Java throws from `getMessage()`, gives no message.
/// The `Display` of the [`LuceneError`] is `Throwable.toString()`'s default
/// form; the class's own `toString()` is [`to_java_string`](Self::to_java_string).
#[derive(Debug, Clone)]
pub struct QueryNodeException {
    class: &'static str,
    detail_message: Option<String>,
    cause: Option<Arc<LuceneError>>,
    message: Arc<dyn Message>,
    localized: OnceLock<Option<String>>,
}

impl QueryNodeException {
    /// The initial value of the protected field `message`: `new
    /// MessageImpl(QueryParserMessages.EMPTY_MESSAGE)`.
    fn empty_message() -> Arc<dyn Message> {
        Arc::new(MessageImpl::new(QueryParserMessages::EMPTY_MESSAGE.get()))
    }

    /// Equivalent to `new QueryNodeException(Message message)`: the detail
    /// message is the message's key.
    pub fn new(message: Arc<dyn Message>) -> Self {
        Self {
            class: QUERY_NODE_EXCEPTION_CLASS,
            detail_message: message.get_key().map(str::to_string),
            cause: None,
            message,
            localized: OnceLock::new(),
        }
    }

    /// Equivalent to `new QueryNodeException(Throwable throwable)`: the
    /// detail message is the cause's `toString()`.
    pub fn with_cause(throwable: LuceneError) -> Self {
        Self {
            class: QUERY_NODE_EXCEPTION_CLASS,
            detail_message: Some(throwable_to_string(&throwable)),
            cause: Some(Arc::new(throwable)),
            message: Self::empty_message(),
            localized: OnceLock::new(),
        }
    }

    /// Equivalent to `new QueryNodeException(Message message, Throwable
    /// throwable)`.
    pub fn with_message_and_cause(message: Arc<dyn Message>, throwable: LuceneError) -> Self {
        Self {
            class: QUERY_NODE_EXCEPTION_CLASS,
            detail_message: message.get_key().map(str::to_string),
            cause: Some(Arc::new(throwable)),
            message,
            localized: OnceLock::new(),
        }
    }

    /// The runtime class of the exception, `getClass().getName()`; a
    /// subclass's constructor sets its own.
    pub fn set_java_class(&mut self, class: &'static str) {
        self.class = class;
    }

    /// Writes the protected field `message`.
    pub fn set_message(&mut self, message: Arc<dyn Message>) {
        self.message = message;
        self.localized = OnceLock::new();
    }

    /// Equivalent to `getLocalizedMessage()`: the message in the default
    /// locale.
    ///
    /// # Errors
    ///
    /// The exceptions of the message lookup.
    pub fn get_localized_message(&self) -> Result<String> {
        self.get_localized_message_with_locale(&Locale::get_default())
    }

    /// Equivalent to `getLocalizedMessage(Locale locale)`.
    ///
    /// # Errors
    ///
    /// The exceptions of the message lookup.
    pub fn get_localized_message_with_locale(&self, locale: &Locale) -> Result<String> {
        self.message.get_localized_message_with_locale(locale)
    }

    /// Equivalent to `toString()`: the message key, `": "` and the localized
    /// message.
    ///
    /// # Errors
    ///
    /// The exceptions of the message lookup.
    pub fn to_java_string(&self) -> Result<String> {
        Ok(format!(
            "{}: {}",
            self.message.get_key().unwrap_or("null"),
            self.get_localized_message()?
        ))
    }

    /// This exception as the [`LuceneError`] it is thrown as.
    pub fn error(self) -> LuceneError {
        LuceneError::from(LuceneErrorKind::Throwable(Arc::new(self)))
    }

    /// The `QueryNodeException` part of `error`, when it is one or one of
    /// its subclasses (`catch (QueryNodeException e)`).
    pub fn of(error: &LuceneError) -> Option<&QueryNodeException> {
        let LuceneErrorKind::Throwable(t) = error.kind() else {
            return None;
        };
        let any = t.as_any();
        if let Some(e) = any.downcast_ref::<QueryNodeException>() {
            return Some(e);
        }
        if let Some(e) = QueryNodeParseException::view(any) {
            return Some(e.query_node_exception());
        }
        let views = SUBCLASS_VIEWS
            .read()
            .unwrap_or_else(PoisonError::into_inner);
        views.iter().find_map(|view| view(any))
    }

    /// Registers a subclass of `QueryNodeException` declared outside this
    /// module and not a subclass of `QueryNodeParseException` (whose
    /// subclasses register with
    /// [`QueryNodeParseException::register_subclass`]), so that
    /// [`of`](Self::of) sees through it.
    pub fn register_subclass(view: QueryNodeExceptionView) {
        SUBCLASS_VIEWS
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .push(view);
    }
}

impl NLSException for QueryNodeException {
    fn get_message_object(&self) -> Option<Arc<dyn Message>> {
        Some(Arc::clone(&self.message))
    }
}

impl JavaThrowable for QueryNodeException {
    fn java_class(&self) -> &'static str {
        self.class
    }

    /// `getMessage()`: `getLocalizedMessage()` (see the divergences of the
    /// type).
    fn get_message(&self) -> Option<&str> {
        self.localized
            .get_or_init(|| self.get_localized_message().ok())
            .as_deref()
    }

    fn get_cause(&self) -> Option<&Arc<LuceneError>> {
        self.cause.as_ref()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl QueryNodeException {
    /// The detail message of `Throwable` (the key of the message, or the
    /// cause's `toString()`), which `getMessage()` overrides.
    pub fn get_detail_message(&self) -> Option<&str> {
        self.detail_message.as_deref()
    }
}
