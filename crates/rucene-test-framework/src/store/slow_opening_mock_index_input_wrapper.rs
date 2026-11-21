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

//! Port of `org.apache.lucene.tests.store.SlowOpeningMockIndexInputWrapper`
//! (package-private in Java): takes a while to open files, which gives
//! `testThreadInterruptDeadlock` a chance to find file leaks if opening an
//! input throws an exception.

use std::sync::Arc;

use randomizedtesting::jdk::thread::Thread;
use rucene::error::{LuceneError, Result};
use rucene::store::IndexInput;
use rucene::util::concurrent::InterruptedException;

use super::mock_directory_wrapper::MockDirectoryWrapper;
use super::mock_index_input_wrapper::{InputCtl, MockIndexInputWrapper};

/// Port of `SlowOpeningMockIndexInputWrapper` (`extends
/// MockIndexInputWrapper`): the constructor sleeps 50 ms.
pub struct SlowOpeningMockIndexInputWrapper {
    inner: MockIndexInputWrapper,
}

impl SlowOpeningMockIndexInputWrapper {
    /// `SlowOpeningMockIndexInputWrapper(dir, name, delegate, confined)`:
    /// sleeps 50 ms after construction.
    ///
    /// # Errors
    ///
    /// `ThreadInterruptedException` when the thread is interrupted during the
    /// sleep, after closing the input (ignoring what that close throws: we
    /// did not open successfully).
    pub fn new(
        dir: &MockDirectoryWrapper,
        name: &str,
        delegate: Box<dyn IndexInput>,
        confined: bool,
    ) -> Result<Self> {
        let mut inner = MockIndexInputWrapper::new(dir, name, delegate, None, confined);
        if Thread::try_sleep(50).is_err() {
            // we didnt open successfully
            let _ignore = inner.close_impl();
            return Err(LuceneError::thread_interrupted(InterruptedException));
        }
        Ok(SlowOpeningMockIndexInputWrapper { inner })
    }

    pub(crate) fn registration(&self) -> (u64, Arc<InputCtl>) {
        self.inner.registration()
    }
}

forward_mock_index_input!(SlowOpeningMockIndexInputWrapper, |this| this
    .inner
    .close_impl());
