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

//! Port of `org.apache.lucene.tests.index.MismatchedDirectoryReader`: a
//! `DirectoryReader` that wraps all its subreaders with
//! [`MismatchedLeafReader`].

use std::sync::Arc;

use randomizedtesting::jdk::random::Random;
use rucene::error::Result;
use rucene::index::directory_reader::DirectoryReader;
use rucene::index::filter_directory_reader::{
    DirectoryReaderWrapper, FilterDirectoryReader, SubReaderWrapper,
};
use rucene::index::LeafReader;

use super::{AssertingDirectoryReader, MismatchedLeafReader};

/// Port of `MismatchedDirectoryReader.MismatchedSubReaderWrapper`: wraps each
/// leaf in a [`MismatchedLeafReader`] drawing on the one shared `random`.
#[derive(Clone)]
pub struct MismatchedSubReaderWrapper {
    random: Arc<dyn Random>,
}

impl MismatchedSubReaderWrapper {
    /// `new MismatchedSubReaderWrapper(random)`.
    pub fn new(random: Arc<dyn Random>) -> Self {
        MismatchedSubReaderWrapper { random }
    }
}

impl SubReaderWrapper for MismatchedSubReaderWrapper {
    fn wrap(&self, reader: Arc<dyn LeafReader>) -> Result<Arc<dyn LeafReader>> {
        Ok(Arc::new(MismatchedLeafReader::new(
            reader,
            self.random.as_ref(),
        )?))
    }
}

/// Port of `org.apache.lucene.tests.index.MismatchedDirectoryReader`: the
/// [`DirectoryReaderWrapper`] that stands for the subclass. Its
/// `getReaderCacheHelper()` returns the wrapped reader's helper.
///
/// As in Java, `doWrapDirectoryReader` rewraps a reopened reader as an
/// [`AssertingDirectoryReader`], not as another `MismatchedDirectoryReader`
/// (`MismatchedDirectoryReader.java:47-50`).
#[derive(Debug, Default, Clone, Copy)]
pub struct MismatchedDirectoryReader;

impl MismatchedDirectoryReader {
    /// `new MismatchedDirectoryReader(in, random)`.
    ///
    /// # Errors
    ///
    /// What wrapping a leaf in a [`MismatchedLeafReader`] raises.
    // The Java constructor: the subclass is the wrapper parameterising core's
    // single `FilterDirectoryReader`, which is what it builds.
    #[allow(clippy::new_ret_no_self)]
    pub fn new(
        r#in: Arc<dyn DirectoryReader>,
        random: Arc<dyn Random>,
    ) -> Result<FilterDirectoryReader> {
        FilterDirectoryReader::new(
            r#in,
            Arc::new(MismatchedSubReaderWrapper::new(random)),
            Arc::new(MismatchedDirectoryReader),
        )
    }
}

impl DirectoryReaderWrapper for MismatchedDirectoryReader {
    /// `doWrapDirectoryReader(in)`: `new AssertingDirectoryReader(in)`.
    fn wrap(&self, inner: Arc<dyn DirectoryReader>) -> Result<Arc<dyn DirectoryReader>> {
        Ok(Arc::new(AssertingDirectoryReader::new(inner)?))
    }

    fn subclass_simple_name(&self) -> &'static str {
        "MismatchedDirectoryReader"
    }
}
