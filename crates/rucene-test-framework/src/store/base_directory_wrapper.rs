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

//! Port of `org.apache.lucene.tests.store.BaseDirectoryWrapper`: calls
//! `CheckIndex` on close.
//!
//! Ported ahead of rmp #343 under the cycle rule: `CloseableDirectory` (rmp
//! #342) calls `isOpen()`.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::Arc;

use rucene::error::{LuceneError, Result};
use rucene::index::check_index::Level;
use rucene::index::directory_reader::index_exists;
use rucene::store::{Directory, IOContext, IndexInput, IndexOutput, Lock};

use crate::util::TestUtil;

/// Port of `org.apache.lucene.tests.store.BaseDirectoryWrapper` (`extends
/// FilterDirectory`).
///
/// **Divergence.** Java's `close()` runs `TestUtil.checkIndex(this, …)` over
/// the wrapper itself; core's `CheckIndex` takes an owned `Arc<dyn
/// Directory>`, which a wrapper cannot hand out for itself from `&mut self`,
/// so the check runs over the delegate, which this base class only forwards
/// to. The delegate is shared (`Arc`) for that check; closing it needs the
/// wrapper to hold the only reference, as Java's `in.close()` owns its
/// delegate.
pub struct BaseDirectoryWrapper {
    delegate: Arc<dyn Directory>,
    check_index_on_close: AtomicBool,
    level_for_check_on_close: AtomicI32,
    is_open: AtomicBool,
}

impl BaseDirectoryWrapper {
    /// `BaseDirectoryWrapper(delegate)`.
    pub fn new(delegate: Arc<dyn Directory>) -> Self {
        BaseDirectoryWrapper {
            delegate,
            check_index_on_close: AtomicBool::new(true),
            level_for_check_on_close: AtomicI32::new(Level::MIN_LEVEL_FOR_SLOW_CHECKS),
            is_open: AtomicBool::new(true),
        }
    }

    /// `getDelegate()`.
    pub fn get_delegate(&self) -> &Arc<dyn Directory> {
        &self.delegate
    }

    /// `isOpen()`.
    pub fn is_open(&self) -> bool {
        self.is_open.load(Ordering::SeqCst)
    }

    /// `setCheckIndexOnClose(value)`: whether `CheckIndex` runs on close.
    pub fn set_check_index_on_close(&self, value: bool) {
        self.check_index_on_close.store(value, Ordering::SeqCst);
    }

    /// `getCheckIndexOnClose()`.
    pub fn get_check_index_on_close(&self) -> bool {
        self.check_index_on_close.load(Ordering::SeqCst)
    }

    /// `setCrossCheckTermVectorsOnClose(value)`: `true` enables the slow
    /// checks (`MIN_LEVEL_FOR_SLOW_CHECKS`), `false` keeps the integrity
    /// checks (`MIN_LEVEL_FOR_INTEGRITY_CHECKS`).
    pub fn set_cross_check_term_vectors_on_close(&self, value: bool) {
        // If true, we are enabling slow checks.
        let level = if value {
            Level::MIN_LEVEL_FOR_SLOW_CHECKS
        } else {
            Level::MIN_LEVEL_FOR_INTEGRITY_CHECKS
        };
        self.level_for_check_on_close.store(level, Ordering::SeqCst);
    }

    /// `getLevelForCheckOnClose()`.
    pub fn get_level_for_check_on_close(&self) -> i32 {
        self.level_for_check_on_close.load(Ordering::SeqCst)
    }
}

impl Directory for BaseDirectoryWrapper {
    fn list_all(&self) -> Result<Vec<String>> {
        self.delegate.list_all()
    }

    fn delete_file(&self, name: &str) -> Result<()> {
        self.delegate.delete_file(name)
    }

    fn file_length(&self, name: &str) -> Result<i64> {
        self.delegate.file_length(name)
    }

    fn create_output(&self, name: &str, context: &dyn IOContext) -> Result<Box<dyn IndexOutput>> {
        self.delegate.create_output(name, context)
    }

    fn create_temp_output(
        &self,
        prefix: &str,
        suffix: &str,
        context: &dyn IOContext,
    ) -> Result<Box<dyn IndexOutput>> {
        self.delegate.create_temp_output(prefix, suffix, context)
    }

    fn sync(&self, names: &[String]) -> Result<()> {
        self.delegate.sync(names)
    }

    fn sync_metadata(&self) -> Result<()> {
        self.delegate.sync_metadata()
    }

    fn rename(&self, source: &str, dest: &str) -> Result<()> {
        self.delegate.rename(source, dest)
    }

    fn open_input(&self, name: &str, context: &dyn IOContext) -> Result<Box<dyn IndexInput>> {
        self.delegate.open_input(name, context)
    }

    fn obtain_lock(&self, name: &str) -> Result<Box<dyn Lock>> {
        self.delegate.obtain_lock(name)
    }

    /// `close()`: the first close runs `TestUtil.checkIndex` at
    /// [`get_level_for_check_on_close`](Self::get_level_for_check_on_close)
    /// when enabled and an index exists, then closes the delegate.
    fn close(&mut self) -> Result<()> {
        if self.is_open.swap(false, Ordering::SeqCst)
            && self.get_check_index_on_close()
            && index_exists(self)?
        {
            TestUtil::check_index_with_level(
                Arc::clone(&self.delegate),
                self.get_level_for_check_on_close(),
            )?;
        }
        Arc::get_mut(&mut self.delegate)
            .ok_or_else(|| LuceneError::IllegalState {
                message: "BaseDirectoryWrapper: the delegate is still shared and cannot be closed"
                    .to_string(),
                cause: None,
            })?
            .close()
    }

    fn get_pending_deletions(&self) -> Result<HashSet<String>> {
        self.delegate.get_pending_deletions()
    }

    fn ensure_open(&self) -> Result<()> {
        self.delegate.ensure_open()
    }

    /// `FilterDirectory.toString()`: `getClass().getSimpleName() + "(" +
    /// in.toString() + ")"`.
    fn directory_to_string(&self) -> String {
        format!(
            "BaseDirectoryWrapper({})",
            self.delegate.directory_to_string()
        )
    }

    /// A `FilterDirectory`, over its `in`.
    fn as_filter_directory(&self) -> Option<&dyn Directory> {
        Some(self.delegate.as_ref())
    }
}

impl std::fmt::Display for BaseDirectoryWrapper {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.directory_to_string())
    }
}
