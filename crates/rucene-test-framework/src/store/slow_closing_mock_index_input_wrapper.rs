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

//! Port of `org.apache.lucene.tests.store.SlowClosingMockIndexInputWrapper`
//! (package-private in Java): hangs onto files a little bit longer (50 ms in
//! close). [`MockDirectoryWrapper`] acts like Windows (you cannot delete
//! files open elsewhere), so the idea is to make race conditions for tiny
//! files (like segments) easier to reproduce.

use std::sync::Arc;

use randomizedtesting::jdk::thread::Thread;
use rucene::error::{LuceneError, Result};
use rucene::store::IndexInput;
use rucene::util::concurrent::InterruptedException;

use super::mock_directory_wrapper::MockDirectoryWrapper;
use super::mock_index_input_wrapper::{InputCtl, MockIndexInputWrapper};

/// Port of `SlowClosingMockIndexInputWrapper` (`extends
/// MockIndexInputWrapper`): `close()` sleeps 50 ms, then closes.
pub struct SlowClosingMockIndexInputWrapper {
    inner: MockIndexInputWrapper,
}

impl SlowClosingMockIndexInputWrapper {
    /// `SlowClosingMockIndexInputWrapper(dir, name, delegate, confined)`.
    pub fn new(
        dir: &MockDirectoryWrapper,
        name: &str,
        delegate: Box<dyn IndexInput>,
        confined: bool,
    ) -> Self {
        let inner = MockIndexInputWrapper::new(dir, name, delegate, None, confined);
        inner.mark_slow_closing();
        SlowClosingMockIndexInputWrapper { inner }
    }

    pub(crate) fn registration(&self) -> (u64, Arc<InputCtl>) {
        self.inner.registration()
    }

    /// The `Thread.sleep(50)` of `close()`.
    ///
    /// # Errors
    ///
    /// `ThreadInterruptedException` when the thread is interrupted.
    pub(crate) fn sleep_before_close() -> Result<()> {
        Thread::try_sleep(50).map_err(|_| LuceneError::thread_interrupted(InterruptedException))
    }
}

forward_mock_index_input!(SlowClosingMockIndexInputWrapper, |this| {
    // `try { Thread.sleep(50) } catch (InterruptedException) { throw new
    // ThreadInterruptedException } finally { super.close() }`: a `finally`
    // that throws replaces the pending exception.
    let slept = SlowClosingMockIndexInputWrapper::sleep_before_close();
    let closed = this.inner.close_impl();
    closed.and(slept)
});
