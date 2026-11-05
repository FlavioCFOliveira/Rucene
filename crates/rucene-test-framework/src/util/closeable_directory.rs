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

//! Port of `org.apache.lucene.tests.util.CloseableDirectory`: attempts to
//! close a [`BaseDirectoryWrapper`].

use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::store::BaseDirectoryWrapper;
use crate::util::TestRuleMarkFailure;

/// What [`CloseableDirectory`] reads of the `BaseDirectoryWrapper` it
/// watches: `isOpen()` and `toString()`.
///
/// Java's `CloseableDirectory` keeps a reference to the wrapper itself. A
/// `MockDirectoryWrapper` copy that is alive when the wrapper is closed keeps
/// its delegate from being closed (see its module docs), so the wrappers
/// `LuceneTestCase.wrapDirectory` registers are watched through a
/// [`DirectoryOpenProbe`], which shares only their open flag.
pub trait DirectoryOpenState: fmt::Display + Send + Sync {
    /// `isOpen()`.
    fn is_open(&self) -> bool;
}

impl DirectoryOpenState for BaseDirectoryWrapper {
    fn is_open(&self) -> bool {
        BaseDirectoryWrapper::is_open(self)
    }
}

/// The open flag of a directory wrapper and its `toString()` at the time the
/// probe was taken.
pub struct DirectoryOpenProbe {
    is_open: Arc<AtomicBool>,
    rendered: String,
}

impl DirectoryOpenProbe {
    /// A probe over the wrapper's shared open flag.
    pub fn new(is_open: Arc<AtomicBool>, rendered: String) -> Self {
        DirectoryOpenProbe { is_open, rendered }
    }
}

impl DirectoryOpenState for DirectoryOpenProbe {
    fn is_open(&self) -> bool {
        self.is_open.load(Ordering::SeqCst)
    }
}

impl fmt::Display for DirectoryOpenProbe {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.rendered)
    }
}

/// Port of `org.apache.lucene.tests.util.CloseableDirectory` (package-private
/// in Java).
pub struct CloseableDirectory {
    dir: Arc<dyn DirectoryOpenState>,
    failure_marker: Arc<TestRuleMarkFailure>,
}

impl CloseableDirectory {
    /// `new CloseableDirectory(dir, failureMarker)`.
    pub fn new<D: DirectoryOpenState + 'static>(
        dir: Arc<D>,
        failure_marker: Arc<TestRuleMarkFailure>,
    ) -> Self {
        CloseableDirectory {
            dir: dir as Arc<dyn DirectoryOpenState>,
            failure_marker,
        }
    }

    /// `close()`: fails the test (`Directory not closed: …`) when no other
    /// test failure happened and the directory is still open. The delegate
    /// is not closed (LUCENE-4058).
    ///
    /// # Panics
    ///
    /// The `AssertionError` of `Assert.fail`.
    pub fn close(&self) {
        // We only attempt to check open/closed state if there were no other test
        // failures.
        if self.failure_marker.was_successful() && self.dir.is_open() {
            randomizedtesting::junit::Assert::fail(&format!("Directory not closed: {}", self.dir));
        }
        // TODO: perform real close of the delegate: LUCENE-4058
        // dir.close();
    }
}

impl randomizedtesting::jdk::io::Closeable for CloseableDirectory {
    /// [`CloseableDirectory::close`].
    fn close(&self) -> std::io::Result<()> {
        CloseableDirectory::close(self);
        Ok(())
    }
}
