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

    /// The package-private field `active`.
    pub fn active(&self) -> bool {
        self.active
    }

    /// The package-private field `createDirectory`.
    pub fn create_directory(&self) -> bool {
        self.create_directory
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
            // `catch (Exception ignored)`: best effort. An `Error` is not an
            // `Exception` and propagates.
            let attempt = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let target = dir.resolve_str(EXTRA_FILE_NAME);
                if self.create_directory {
                    let _ = ffsp::create_directory(self, &target);
                } else {
                    let _ = Files::create_file(&target);
                }
            }));
            if let Err(payload) = attempt {
                if super::is_java_error(&*payload) {
                    std::panic::resume_unwind(payload);
                }
            }
        }
        Ok(())
    }
}

crate::impl_file_system_provider!(ExtrasFS);

#[cfg(test)]
mod tests {
    use super::*;
    use randomizedtesting::jdk::exception::{Throwable, ASSERTION_ERROR, RUNTIME_EXCEPTION};
    use rucene::store::java_nio_file::default_file_system;

    /// A filter whose `createDirectory` of an `extra0` directory throws
    /// `class`.
    struct ThrowOnExtra {
        filter: FilterState,
        class: &'static str,
    }

    impl FilterFileSystemProvider for ThrowOnExtra {
        fn filter(&self) -> &FilterState {
            &self.filter
        }
        fn class_name(&self) -> &'static str {
            "ThrowOnExtra"
        }
        fn create_directory(&self, dir: &Path) -> io::Result<()> {
            if dir.to_string().ends_with(EXTRA_FILE_NAME) {
                Throwable::new(self.class, "extra refused").throw()
            }
            ffsp::create_directory(self, dir)
        }
    }

    crate::impl_file_system_provider!(ThrowOnExtra);

    fn extras_over_throwing(class: &'static str) -> Arc<ExtrasFS> {
        let default = default_file_system();
        let inner = Arc::new(ThrowOnExtra {
            filter: FilterState::new("throwonextra://", &default),
            class,
        });
        install(&inner, default);
        ExtrasFS::new(
            FilterFileSystemProvider::get_file_system(&*inner),
            true,
            true,
        )
    }

    fn fresh_dir(fs: &ExtrasFS, name: &str) -> Path {
        let base =
            std::env::temp_dir().join(format!("rucene-extras-fs-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        FilterFileSystemProvider::get_file_system(fs).get_path(&base.to_string_lossy(), &[])
    }

    /// `catch (Exception ignored)` (`ExtrasFS.java:69-73`) swallows an
    /// exception of the extra file's creation ...
    #[test]
    fn an_exception_creating_the_extra_is_ignored() {
        let fs = extras_over_throwing(RUNTIME_EXCEPTION);
        let dir = fresh_dir(&fs, "exception");
        FilterFileSystemProvider::create_directory(&*fs, &dir).unwrap();
        std::fs::remove_dir_all(dir.to_string()).unwrap();
    }

    /// ... but an `Error` is no `Exception`: it propagates.
    #[test]
    fn an_error_creating_the_extra_propagates() {
        let fs = extras_over_throwing(ASSERTION_ERROR);
        let dir = fresh_dir(&fs, "error");
        let payload = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = FilterFileSystemProvider::create_directory(&*fs, &dir);
        }))
        .unwrap_err();
        let t = Throwable::from_panic(payload);
        assert_eq!(t.class_name(), ASSERTION_ERROR);
        assert_eq!(t.message(), Some("extra refused"));
        std::fs::remove_dir_all(dir.to_string()).unwrap();
    }
}
