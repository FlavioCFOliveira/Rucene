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

//! Port of `org.apache.lucene.tests.store.RawDirectoryWrapper`: delegates
//! all operations, even optional ones, to the wrapped directory.
//!
//! This class is used for the most realistic testing, still with a
//! `CheckIndex` on close. For asserts and evil things, use
//! [`MockDirectoryWrapper`](super::MockDirectoryWrapper) instead.

use std::collections::HashSet;
use std::sync::Arc;

use rucene::error::Result;
use rucene::store::{
    BufferedChecksumIndexInput, Directory, IOContext, IndexInput, IndexOutput, Lock,
};

use super::BaseDirectoryWrapper;

/// Port of `org.apache.lucene.tests.store.RawDirectoryWrapper` (`final`,
/// `extends BaseDirectoryWrapper`): [`BaseDirectoryWrapper`] that also
/// forwards `copyFrom` and `openChecksumInput` to the delegate.
pub struct RawDirectoryWrapper {
    base: BaseDirectoryWrapper,
}

impl RawDirectoryWrapper {
    /// `RawDirectoryWrapper(delegate)`.
    pub fn new(delegate: Arc<dyn Directory>) -> Self {
        RawDirectoryWrapper {
            base: BaseDirectoryWrapper::new(delegate),
        }
    }

    /// The [`BaseDirectoryWrapper`] this class extends: `isOpen()`,
    /// `getDelegate()` and the check-on-close settings.
    pub fn base(&self) -> &BaseDirectoryWrapper {
        &self.base
    }
}

impl Directory for RawDirectoryWrapper {
    /// `instanceof` (see [`Directory::as_any`]).
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn list_all(&self) -> Result<Vec<String>> {
        self.base.list_all()
    }

    fn delete_file(&self, name: &str) -> Result<()> {
        self.base.delete_file(name)
    }

    fn file_length(&self, name: &str) -> Result<i64> {
        self.base.file_length(name)
    }

    fn create_output(&self, name: &str, context: &dyn IOContext) -> Result<Box<dyn IndexOutput>> {
        self.base.create_output(name, context)
    }

    fn create_temp_output(
        &self,
        prefix: &str,
        suffix: &str,
        context: &dyn IOContext,
    ) -> Result<Box<dyn IndexOutput>> {
        self.base.create_temp_output(prefix, suffix, context)
    }

    fn sync(&self, names: &[String]) -> Result<()> {
        self.base.sync(names)
    }

    fn sync_metadata(&self) -> Result<()> {
        self.base.sync_metadata()
    }

    fn rename(&self, source: &str, dest: &str) -> Result<()> {
        self.base.rename(source, dest)
    }

    fn open_input(&self, name: &str, context: &dyn IOContext) -> Result<Box<dyn IndexInput>> {
        self.base.open_input(name, context)
    }

    /// `openChecksumInput(name)`: `in.openChecksumInput(name)`.
    fn open_checksum_input(&self, name: &str) -> Result<Box<BufferedChecksumIndexInput>> {
        self.base.get_delegate().open_checksum_input(name)
    }

    fn obtain_lock(&self, name: &str) -> Result<Box<dyn Lock>> {
        self.base.obtain_lock(name)
    }

    /// `BaseDirectoryWrapper.close()`.
    fn close(&self) -> Result<()> {
        self.base.close()
    }

    /// `copyFrom(from, src, dest, context)`: `in.copyFrom(from, src, dest,
    /// context)`.
    fn copy_from(
        &self,
        from: &dyn Directory,
        src: &str,
        dest: &str,
        context: &dyn IOContext,
    ) -> Result<()> {
        self.base.get_delegate().copy_from(from, src, dest, context)
    }

    fn get_pending_deletions(&self) -> Result<HashSet<String>> {
        self.base.get_pending_deletions()
    }

    fn ensure_open(&self) -> Result<()> {
        self.base.ensure_open()
    }

    /// `FilterDirectory.toString()`: `RawDirectoryWrapper(<in>)`.
    fn directory_to_string(&self) -> String {
        format!(
            "RawDirectoryWrapper({})",
            self.base.get_delegate().directory_to_string()
        )
    }

    /// A `FilterDirectory`, over its `in`.
    fn as_filter_directory(&self) -> Option<&dyn Directory> {
        Some(self.base.get_delegate().as_ref())
    }
}

impl std::fmt::Display for RawDirectoryWrapper {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.directory_to_string())
    }
}
