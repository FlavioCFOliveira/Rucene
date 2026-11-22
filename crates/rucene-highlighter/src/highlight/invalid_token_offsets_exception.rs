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

//! Port of `InvalidTokenOffsetsException.java`, of the Java package
//! `org.apache.lucene.search.highlight`.

use std::any::Any;
use std::sync::Arc;

use rucene::error::{JavaThrowable, LuceneError, LuceneErrorKind};

/// Exception thrown if TokenStream Tokens are incompatible with provided text.
///
/// Equivalent to `org.apache.lucene.search.highlight.InvalidTokenOffsetsException`,
/// a checked `Exception`; it travels as a [`LuceneErrorKind::Throwable`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidTokenOffsetsException {
    message: String,
}

impl InvalidTokenOffsetsException {
    /// Equivalent to `new InvalidTokenOffsetsException(String)`.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    /// `throw new InvalidTokenOffsetsException(message)`: the exception as a
    /// [`LuceneError`].
    pub fn error(message: impl Into<String>) -> LuceneError {
        LuceneError::from(LuceneErrorKind::Throwable(Arc::new(Self::new(message))))
    }
}

impl JavaThrowable for InvalidTokenOffsetsException {
    fn java_class(&self) -> &'static str {
        "org.apache.lucene.search.highlight.InvalidTokenOffsetsException"
    }

    fn get_message(&self) -> Option<&str> {
        Some(&self.message)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}