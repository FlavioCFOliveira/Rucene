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

//! Port of `org.apache.lucene.tests.index.AssertingDirectoryReader`: a
//! `DirectoryReader` that wraps all its subreaders with
//! [`AssertingLeafReader`].
//!
//! As the other `FilterDirectoryReader` subclasses of this port
//! (`rucene::index::filter_directory_reader::SoftDeletesDirectoryReaderWrapper`),
//! the Java subclass is the [`DirectoryReaderWrapper`] that parameterises
//! core's single [`FilterDirectoryReader`]: [`AssertingDirectoryReader::new`]
//! builds one, and a reopen rewraps the new reader the same way
//! (`doWrapDirectoryReader`).

use std::sync::Arc;

use rucene::error::Result;
use rucene::index::directory_reader::DirectoryReader;
use rucene::index::filter_directory_reader::{
    DirectoryReaderWrapper, FilterDirectoryReader, SubReaderWrapper,
};
use rucene::index::LeafReader;

use super::AssertingLeafReader;

/// Port of `AssertingDirectoryReader.AssertingSubReaderWrapper`: wraps each
/// leaf in an [`AssertingLeafReader`].
#[derive(Debug, Default, Clone, Copy)]
pub struct AssertingSubReaderWrapper;

impl SubReaderWrapper for AssertingSubReaderWrapper {
    fn wrap(&self, reader: Arc<dyn LeafReader>) -> Result<Arc<dyn LeafReader>> {
        Ok(Arc::new(AssertingLeafReader::new(reader)?))
    }
}

/// Port of `org.apache.lucene.tests.index.AssertingDirectoryReader`: the
/// [`DirectoryReaderWrapper`] that stands for the subclass. Its
/// `getReaderCacheHelper()` returns the wrapped reader's helper, which is
/// [`DirectoryReaderWrapper::uses_delegating_cache_helper`]'s default.
#[derive(Debug, Default, Clone, Copy)]
pub struct AssertingDirectoryReader;

impl AssertingDirectoryReader {
    /// `new AssertingDirectoryReader(in)`.
    ///
    /// # Errors
    ///
    /// What wrapping a leaf in an [`AssertingLeafReader`] raises.
    // The Java constructor: the subclass is the wrapper parameterising core's
    // single `FilterDirectoryReader`, which is what it builds.
    #[allow(clippy::new_ret_no_self)]
    pub fn new(r#in: Arc<dyn DirectoryReader>) -> Result<FilterDirectoryReader> {
        FilterDirectoryReader::new(
            r#in,
            Arc::new(AssertingSubReaderWrapper),
            Arc::new(AssertingDirectoryReader),
        )
    }
}

impl DirectoryReaderWrapper for AssertingDirectoryReader {
    /// `doWrapDirectoryReader(in)`: `new AssertingDirectoryReader(in)`.
    fn wrap(&self, inner: Arc<dyn DirectoryReader>) -> Result<Arc<dyn DirectoryReader>> {
        Ok(Arc::new(AssertingDirectoryReader::new(inner)?))
    }

    fn subclass_simple_name(&self) -> &'static str {
        "AssertingDirectoryReader"
    }
}
