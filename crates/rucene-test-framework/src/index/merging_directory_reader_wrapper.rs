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

//! Port of `org.apache.lucene.tests.index.MergingDirectoryReaderWrapper`: a
//! `DirectoryReader` wrapper that uses the merge instances of the wrapped
//! `CodecReader`s.

use std::sync::Arc;

use rucene::error::{LuceneError, Result};
use rucene::index::directory_reader::DirectoryReader;
use rucene::index::filter_directory_reader::{
    DirectoryReaderWrapper, FilterDirectoryReader, SubReaderWrapper,
};
use rucene::index::LeafReader;

use super::MergingCodecReader;

/// The anonymous `SubReaderWrapper` of `MergingDirectoryReaderWrapper`:
/// `new MergingCodecReader((CodecReader) reader)`.
#[derive(Debug, Default, Clone, Copy)]
struct MergingSubReaderWrapper;

impl SubReaderWrapper for MergingSubReaderWrapper {
    /// **Divergence (cast message).** A leaf that is not a `CodecReader`
    /// fails the cast with Java's `ClassCastException`, without its message
    /// (`class X cannot be cast to class ...`), which names run-time classes
    /// this port does not have.
    fn wrap(&self, reader: Arc<dyn LeafReader>) -> Result<Arc<dyn LeafReader>> {
        match reader.as_codec_reader() {
            Some(reader) => Ok(Arc::new(MergingCodecReader::new(reader))),
            None => Err(LuceneError::Unchecked {
                class: "java.lang.ClassCastException",
                message: None,
                cause: None,
            }),
        }
    }
}

/// Port of `org.apache.lucene.tests.index.MergingDirectoryReaderWrapper`
/// (`final`, `extends FilterDirectoryReader`): the [`DirectoryReaderWrapper`]
/// that stands for the subclass. NOTE: as in Java, it fails to work if the
/// leaves of the wrapped directory are not codec readers. Its
/// `getReaderCacheHelper()` returns the wrapped reader's helper (it doesn't
/// change the content: can delegate).
#[derive(Debug, Default, Clone, Copy)]
pub struct MergingDirectoryReaderWrapper;

impl MergingDirectoryReaderWrapper {
    /// `new MergingDirectoryReaderWrapper(in)`: wraps the given directory.
    ///
    /// # Errors
    ///
    /// The `ClassCastException` of a leaf that is not a codec reader.
    // The Java constructor: the subclass is the wrapper parameterising core's
    // single `FilterDirectoryReader`, which is what it builds.
    #[allow(clippy::new_ret_no_self)]
    pub fn new(r#in: Arc<dyn DirectoryReader>) -> Result<FilterDirectoryReader> {
        FilterDirectoryReader::new(
            r#in,
            Arc::new(MergingSubReaderWrapper),
            Arc::new(MergingDirectoryReaderWrapper),
        )
    }
}

impl DirectoryReaderWrapper for MergingDirectoryReaderWrapper {
    /// `doWrapDirectoryReader(in)`: `new MergingDirectoryReaderWrapper(in)`.
    fn wrap(&self, inner: Arc<dyn DirectoryReader>) -> Result<Arc<dyn DirectoryReader>> {
        Ok(Arc::new(MergingDirectoryReaderWrapper::new(inner)?))
    }

    fn subclass_simple_name(&self) -> &'static str {
        "MergingDirectoryReaderWrapper"
    }
}
