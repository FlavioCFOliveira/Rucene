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

//! A resource loader over the local file system, ported from
//! `org.apache.lucene.analysis.util.FilesystemResourceLoader`.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use rucene::error::{LuceneError, Result};
use rucene::util::resource_loader::{
    ClasspathResourceLoader, ModuleResourceLoader, ResourceLoader,
};

/// Simple [`ResourceLoader`] that opens resource files from the local file
/// system, optionally resolving against a base directory.
///
/// This loader wraps a delegate [`ResourceLoader`] that is used to resolve
/// all files, the current base directory does not contain.
///
/// You can chain several `FilesystemResourceLoader`s to allow lookup of
/// files in more than one base directory.
///
/// Equivalent to `org.apache.lucene.analysis.util.FilesystemResourceLoader`.
/// Java's `newInstance` and `findClass` delegate to the delegate loader;
/// `rucene`'s `ResourceLoader` has no class half (see
/// `rucene::util::resource_loader`), so there is nothing to delegate here.
pub struct FilesystemResourceLoader {
    base_directory: PathBuf,
    delegate: Box<dyn ResourceLoader>,
}

impl std::fmt::Debug for FilesystemResourceLoader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FilesystemResourceLoader")
            .field("base_directory", &self.base_directory)
            .finish_non_exhaustive()
    }
}

impl FilesystemResourceLoader {
    /// Creates a resource loader that resolves resources against the given
    /// base directory. Files not found in file system and class lookups are
    /// delegated to the class loader whose resources are `class_loader`.
    ///
    /// Equivalent to `FilesystemResourceLoader(Path, ClassLoader)`,
    /// `this(baseDirectory, new ClasspathResourceLoader(delegate))`: a
    /// class loader is the table of the resources it holds
    /// ([`ClasspathResourceLoader::new`], for example
    /// [`class_loader_resources`](crate::resources::class_loader_resources)).
    ///
    /// # Errors
    ///
    /// As [`with_delegate`](Self::with_delegate).
    pub fn with_class_loader(
        base_directory: &Path,
        class_loader: HashMap<String, Arc<Vec<u8>>>,
    ) -> Result<Self> {
        Self::with_delegate(
            base_directory,
            Box::new(ClasspathResourceLoader::new(class_loader)),
        )
    }

    /// Creates a resource loader that resolves resources against the given
    /// base directory. Files not found in file system and class lookups are
    /// delegated to [`ModuleResourceLoader`].
    ///
    /// Equivalent to `FilesystemResourceLoader(Path, Module)`,
    /// `this(baseDirectory, new ModuleResourceLoader(delegate))`: a module is
    /// the directory root `rucene`'s [`ModuleResourceLoader`] reads below.
    ///
    /// # Errors
    ///
    /// As [`with_delegate`](Self::with_delegate).
    pub fn with_module(base_directory: &Path, module_root: &Path) -> Result<Self> {
        Self::with_delegate(
            base_directory,
            Box::new(ModuleResourceLoader::new(module_root)),
        )
    }

    /// Creates a resource loader that resolves resources against the given
    /// base directory. Files not found in file system and class lookups are
    /// delegated to the given delegate [`ResourceLoader`].
    ///
    /// Equivalent to `FilesystemResourceLoader(Path, ResourceLoader)`. Java's
    /// `IllegalArgumentException` for a `null` delegate, and its
    /// `NullPointerException` for a `null` base directory, have no
    /// counterpart: neither argument can be null here.
    ///
    /// # Errors
    ///
    /// Returns `IllegalArgumentException` (`LuceneErrorKind::IllegalArgument`)
    /// with Java's message, `<baseDirectory> is not a directory`, when
    /// `base_directory` is not a directory (`Files.isDirectory`, which
    /// follows symbolic links).
    pub fn with_delegate(base_directory: &Path, delegate: Box<dyn ResourceLoader>) -> Result<Self> {
        if !base_directory.is_dir() {
            return Err(LuceneError::illegal_argument(format!(
                "{} is not a directory",
                base_directory.display()
            )));
        }
        Ok(Self {
            base_directory: base_directory.to_path_buf(),
            delegate,
        })
    }
}

impl ResourceLoader for FilesystemResourceLoader {
    /// Equivalent to `openResource(String)`: `Files.newInputStream(
    /// baseDirectory.resolve(resource))`, and the delegate's `openResource`
    /// when the file does not exist (`FileNotFoundException` or
    /// `NoSuchFileException`); any other failure to open the file is
    /// returned.
    fn open_resource(&self, resource: &str) -> Result<Box<dyn Read + Send>> {
        match std::fs::File::open(self.base_directory.join(resource)) {
            Ok(file) => Ok(Box::new(file)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                self.delegate.open_resource(resource)
            }
            Err(error) => Err(LuceneError::from(error)),
        }
    }
}
