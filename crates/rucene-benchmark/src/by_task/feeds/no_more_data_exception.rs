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

//! The types declared in `NoMoreDataException.java`, of the Java package
//! `org.apache.lucene.benchmark.byTask.feeds`.

use std::any::Any;
use std::sync::Arc;

use rucene::error::{JavaThrowable, LuceneError, LuceneErrorKind};

/// Equivalent to `org.apache.lucene.benchmark.byTask.feeds.NoMoreDataException`
/// (`public class NoMoreDataException extends Exception`): exception
/// indicating there is no more data. Thrown by Docs Makers if
/// content.source.forever is false and docs sources of that maker where
/// exhausted. This is useful for iterating all document of a source, in case
/// we don't know in advance how many docs there are.
///
/// It travels as a [`LuceneError`] of the `Throwable` kind holding this
/// class; [`NoMoreDataException::is`] is Java's `catch
/// (NoMoreDataException e)`.
#[derive(Debug, Default)]
pub struct NoMoreDataException;

impl NoMoreDataException {
    /// `new NoMoreDataException()`, as the error that carries it.
    #[allow(clippy::new_ret_no_self)]
    pub fn new() -> LuceneError {
        LuceneError::from(LuceneErrorKind::Throwable(Arc::new(NoMoreDataException)))
    }

    /// Whether `error` is a `NoMoreDataException`.
    pub fn is(error: &LuceneError) -> bool {
        matches!(error.kind(), LuceneErrorKind::Throwable(t) if t.as_any().is::<NoMoreDataException>())
    }
}

impl JavaThrowable for NoMoreDataException {
    fn java_class(&self) -> &'static str {
        "org.apache.lucene.benchmark.byTask.feeds.NoMoreDataException"
    }

    fn get_message(&self) -> Option<&str> {
        None
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
