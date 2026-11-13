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

//! The timeout of a suggestion, ported from
//! `org.apache.lucene.analysis.hunspell.SuggestionTimeoutException`.

use std::sync::Arc;

use rucene::error::{JavaThrowable, LuceneError, LuceneErrorKind};

/// An exception thrown when
/// [`Hunspell::suggest`](super::hunspell::Hunspell::suggest) call takes too
/// long, if [`TimeoutPolicy::ThrowException`](super::timeout_policy::TimeoutPolicy::ThrowException)
/// is used.
///
/// Equivalent to `org.apache.lucene.analysis.hunspell.SuggestionTimeoutException`,
/// a `RuntimeException`, carried as [`LuceneErrorKind::Throwable`]; build one
/// with [`error`](Self::error) and read it back with [`of`](Self::of).
#[derive(Debug, Clone)]
pub struct SuggestionTimeoutException {
    message: Option<String>,
    partial_result: Option<Vec<String>>,
}

impl SuggestionTimeoutException {
    /// Equivalent to `SuggestionTimeoutException(String, List<String>)`.
    pub fn new(message: Option<String>, partial_result: Option<Vec<String>>) -> Self {
        Self {
            message,
            partial_result,
        }
    }

    /// `throw new SuggestionTimeoutException(message, partialResult)`, as a
    /// [`LuceneError`].
    pub fn error(message: Option<String>, partial_result: Option<Vec<String>>) -> LuceneError {
        LuceneError::from(LuceneErrorKind::Throwable(Arc::new(Self::new(
            message,
            partial_result,
        ))))
    }

    /// The `SuggestionTimeoutException` `error` stands for, if it is one.
    pub fn of(error: &LuceneError) -> Option<&SuggestionTimeoutException> {
        match error.kind() {
            LuceneErrorKind::Throwable(throwable) => throwable.as_any().downcast_ref(),
            _ => None,
        }
    }

    /// Partial result calculated by
    /// [`Hunspell::suggest`](super::hunspell::Hunspell::suggest) before the
    /// time limit was exceeded, or `None` if there's none available.
    ///
    /// Equivalent to `getPartialResult()`.
    pub fn get_partial_result(&self) -> Option<&[String]> {
        self.partial_result.as_deref()
    }

    /// Equivalent to `getMessage()`.
    pub fn get_message(&self) -> Option<&str> {
        self.message.as_deref()
    }
}

impl JavaThrowable for SuggestionTimeoutException {
    fn java_class(&self) -> &'static str {
        "org.apache.lucene.analysis.hunspell.SuggestionTimeoutException"
    }

    fn get_message(&self) -> Option<&str> {
        self.message.as_deref()
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
