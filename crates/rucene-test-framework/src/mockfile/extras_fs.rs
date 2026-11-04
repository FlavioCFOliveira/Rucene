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

//! Port of `org.apache.lucene.tests.mockfile.ExtrasFS`.

use std::io;
use std::sync::Arc;

use rucene::store::java_nio_file::{FileSystem, Files, Path};

use super::filter_file_system_provider::{
    self as ffsp, install, FilterFileSystemProvider, FilterState,
};

/// `EXTRA_FILE_NAME`.
const EXTRA_FILE_NAME: &str = "extra0";

/// Port of `org.apache.lucene.tests.mockfile.ExtrasFS`: adds extra files or
/// subdirectories when directories are created.
///
/// Lucene shouldn't care about these, but sometimes operating systems create
/// special files themselves (`.DS_Store`, `thumbs.db`, `.nfsXXX`, ...), so we
/// add them and see what breaks. All other filesystem operations are
/// delegated as normal.
pub struct ExtrasFS {
    filter: FilterState,
    /// `active`: whether extra files are created.
    active: bool,
    /// `createDirectory`: whether directories are created instead of files.
    /// Ignored if `active` is `false`.
    create_directory: bool,
}

impl ExtrasFS {
    /// `ExtrasFS(delegate, active, createDirectory)`, with the scheme
    /// `extras://`.
    pub fn new(
        delegate: Arc<dyn FileSystem>,
        active: bool,
        create_directory: bool,
    ) -> Arc<ExtrasFS> {
        let this = Arc::new(ExtrasFS {
            filter: FilterState::new("extras://", &delegate),
            active,
            create_directory,
        });
        install(&this, delegate);
        this
    }

    /// `isExtra(fileName)`: whether `file_name` is one of the extra files this
    /// class adds.
    pub fn is_extra(file_name: &str) -> bool {
        file_name == EXTRA_FILE_NAME
    }
}

impl FilterFileSystemProvider for ExtrasFS {
    fn filter(&self) -> &FilterState {
        &self.filter
    }

    fn class_name(&self) -> &'static str {
        "org.apache.lucene.tests.mockfile.ExtrasFS"
    }

    /// `createDirectory(dir, attrs...)`: once the directory is created, "lets
    /// add a bogus file... if this fails, we don't care, its best effort".
    fn create_directory(&self, dir: &Path) -> io::Result<()> {
        ffsp::create_directory(self, dir)?;
        // ok, we created the directory successfully.
        if self.active {
            // `catch (Exception ignored)`: best effort.
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let target = dir.resolve_str(EXTRA_FILE_NAME);
                if self.create_directory {
                    let _ = ffsp::create_directory(self, &target);
                } else {
                    let _ = Files::create_file(&target);
                }
            }));
        }
        Ok(())
    }
}

crate::impl_file_system_provider!(ExtrasFS);
