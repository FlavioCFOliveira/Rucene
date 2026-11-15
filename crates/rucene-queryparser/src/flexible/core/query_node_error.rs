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

//! Port of `QueryNodeError.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.core`.

use std::any::Any;
use std::sync::Arc;

use rucene::error::{JavaThrowable, LuceneError, LuceneErrorKind};

use super::query_node_exception::throwable_to_string;
use crate::flexible::messages::message::Message;
use crate::flexible::messages::nls_exception::NLSException;

/// The binary name of `QueryNodeError`.
pub const QUERY_NODE_ERROR_CLASS: &str =
    "org.apache.lucene.queryparser.flexible.core.QueryNodeError";

/// Error class with NLS support
///
/// Equivalent to `org.apache.lucene.queryparser.flexible.core.QueryNodeError`,
/// a `java.lang.Error` that implements `NLSException`; it is thrown as a
/// [`LuceneError`] of kind [`LuceneErrorKind::Throwable`].
#[derive(Debug, Clone)]
pub struct QueryNodeError {
    message: Option<Arc<dyn Message>>,
    detail_message: Option<String>,
    cause: Option<Arc<LuceneError>>,
}

impl QueryNodeError {
    /// Equivalent to `new QueryNodeError(Message message)`: the detail
    /// message is the message's key.
    pub fn new(message: Arc<dyn Message>) -> Self {
        Self {
            detail_message: message.get_key().map(str::to_string),
            message: Some(message),
            cause: None,
        }
    }

    /// Equivalent to `new QueryNodeError(Throwable throwable)`: no message
    /// object, the detail message is the cause's `toString()`.
    pub fn with_cause(throwable: LuceneError) -> Self {
        Self {
            message: None,
            detail_message: Some(throwable_to_string(&throwable)),
            cause: Some(Arc::new(throwable)),
        }
    }

    /// Equivalent to `new QueryNodeError(Message message, Throwable
    /// throwable)`.
    pub fn with_message_and_cause(message: Arc<dyn Message>, throwable: LuceneError) -> Self {
        Self {
            detail_message: message.get_key().map(str::to_string),
            message: Some(message),
            cause: Some(Arc::new(throwable)),
        }
    }

    /// This error as the [`LuceneError`] it is thrown as.
    pub fn error(self) -> LuceneError {
        LuceneError::from(LuceneErrorKind::Throwable(Arc::new(self)))
    }

    /// The `QueryNodeError` `error` stands for, when it is one.
    pub fn of(error: &LuceneError) -> Option<&QueryNodeError> {
        match error.kind() {
            LuceneErrorKind::Throwable(t) => t.as_any().downcast_ref(),
            _ => None,
        }
    }
}

impl NLSException for QueryNodeError {
    fn get_message_object(&self) -> Option<Arc<dyn Message>> {
        self.message.clone()
    }
}

impl JavaThrowable for QueryNodeError {
    fn java_class(&self) -> &'static str {
        QUERY_NODE_ERROR_CLASS
    }

    fn get_message(&self) -> Option<&str> {
        self.detail_message.as_deref()
    }

    fn get_cause(&self) -> Option<&Arc<LuceneError>> {
        self.cause.as_ref()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
