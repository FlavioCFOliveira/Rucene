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

//! Port of `org.apache.lucene.tests.index.SuppressingConcurrentMergeScheduler`.

use std::error::Error;
use std::sync::Arc;

use rucene::error::LuceneError;
use rucene::index::merge_scheduler::ConcurrentMergeScheduler;

/// The abstract `isOK(Throwable t)` a concrete subclass defines.
pub type IsOk = Arc<dyn Fn(&(dyn Error + 'static)) -> bool + Send + Sync>;

/// Port of `org.apache.lucene.tests.index.SuppressingConcurrentMergeScheduler`
/// (`abstract class extends ConcurrentMergeScheduler`): "A
/// `ConcurrentMergeScheduler` that ignores AlreadyClosedException."
///
/// Java's concrete subclasses define `isOK`; here the subclass instance is the
/// [`ConcurrentMergeScheduler`] [`create`](Self::create) returns, with the
/// `handleMergeException` override installed and `is_ok` as `isOK`.
pub struct SuppressingConcurrentMergeScheduler;

impl SuppressingConcurrentMergeScheduler {
    /// A new instance of the subclass whose `isOK(t)` is `is_ok(t)`.
    pub fn create(is_ok: IsOk) -> ConcurrentMergeScheduler {
        let cms = ConcurrentMergeScheduler::new();
        cms.set_handle_merge_exception_override(Some(Arc::new(move |exc: LuceneError| {
            Self::handle_merge_exception(&*is_ok, exc)
        })));
        cms
    }

    /// `handleMergeException(exc)`: "Suppress if any exception in the cause
    /// chain is OK. Rethrow the original otherwise."
    ///
    /// # Panics
    ///
    /// With the merge exception, as `super.handleMergeException(exc)` throws
    /// it, when no exception of the chain is OK.
    pub fn handle_merge_exception(
        is_ok: &dyn Fn(&(dyn Error + 'static)) -> bool,
        exc: LuceneError,
    ) {
        let mut cause: Option<&(dyn Error + 'static)> = Some(&exc);
        while let Some(c) = cause {
            // A `LuceneError` holds its cause as an `Arc<LuceneError>`; `isOK`
            // sees the exception itself, as Java's `getCause()` returns it.
            let c = match c.downcast_ref::<Arc<LuceneError>>() {
                Some(shared) => &**shared as &(dyn Error + 'static),
                None => c,
            };
            if is_ok(c) {
                return;
            }
            cause = c.source();
        }
        ConcurrentMergeScheduler::default_handle_merge_exception(exc);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rucene::error::LuceneErrorKind;
    use std::panic::{catch_unwind, AssertUnwindSafe};

    fn is_already_closed(t: &(dyn Error + 'static)) -> bool {
        matches!(
            t.downcast_ref::<LuceneError>(),
            Some(LuceneError {
                kind: LuceneErrorKind::AlreadyClosed { .. },
                ..
            })
        )
    }

    /// An OK exception anywhere in the cause chain is suppressed.
    #[test]
    fn suppresses_an_ok_cause() {
        SuppressingConcurrentMergeScheduler::handle_merge_exception(
            &is_already_closed,
            LuceneError::already_closed("closed"),
        );
        let wrapped = LuceneError::illegal_state_with_cause(
            "merge failed",
            Arc::new(LuceneError::already_closed("closed")),
        );
        SuppressingConcurrentMergeScheduler::handle_merge_exception(&is_already_closed, wrapped);
    }

    /// Otherwise the original is rethrown by `super.handleMergeException`.
    #[test]
    fn rethrows_otherwise() {
        let thrown = catch_unwind(AssertUnwindSafe(|| {
            SuppressingConcurrentMergeScheduler::handle_merge_exception(
                &is_already_closed,
                LuceneError::illegal_state("boom"),
            )
        }))
        .unwrap_err();
        let message = randomizedtesting::jdk::exception::panic_message(&*thrown);
        assert!(message.contains("boom"), "{message}");
    }

    /// The scheduler `create` returns carries the override.
    #[test]
    fn create_installs_the_override() {
        let cms = SuppressingConcurrentMergeScheduler::create(Arc::new(is_already_closed));
        cms.handle_merge_exception(LuceneError::already_closed("closed"));
        let thrown = catch_unwind(AssertUnwindSafe(|| {
            cms.handle_merge_exception(LuceneError::illegal_state("boom"))
        }));
        assert!(thrown.is_err());
    }
}
