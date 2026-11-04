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

//! Port of `org.apache.lucene.tests.mockfile.FilterFileSystemProvider`.
//!
//! **Divergence.** Java's `FilterFileSystemProvider` is an abstract class
//! whose methods subclasses override and reach again through `super`. Rust
//! has no inheritance, so the class is three pieces: [`FilterState`], its
//! fields; the trait [`FilterFileSystemProvider`], whose methods default to
//! the class's bodies and which a mock filesystem overrides; and the free
//! functions of this module, the same bodies, which an override calls where
//! Java calls `super`. [`impl_file_system_provider!`](crate::impl_file_system_provider)
//! makes such a type a `java.nio.file.spi.FileSystemProvider` by forwarding
//! every provider method to the trait.

use std::any::Any;
use std::io;
use std::sync::{Arc, OnceLock};

use rucene::store::java_nio_file::{
    same_file_system, throw_unchecked, AccessMode, AsynchronousFileChannel, AttributeValue,
    BasicFileAttributes, CopyOption, DirectoryStream, DirectoryStreamFilter, FileChannel,
    FileStore, FileSystem, FileSystemProvider, InputStream, LinkOption, OpenOption, OutputStream,
    Path, SeekableByteChannel,
};

use super::filter_directory_stream::FilterDirectoryStream;
use super::filter_file_system::FilterFileSystem;
use super::filter_path::FilterPath;

/// The fields of `FilterFileSystemProvider`.
pub struct FilterState {
    /// `delegate`: the underlying `FileSystemProvider`.
    delegate: Arc<dyn FileSystemProvider>,
    /// `fileSystem`: the underlying `FileSystem` instance, the singleton
    /// filesystem of this provider.
    file_system: OnceLock<Arc<FilterFileSystem>>,
    /// `scheme`: the URI scheme for this provider.
    scheme: String,
}

impl FilterState {
    /// The state of `FilterFileSystemProvider(String scheme, FileSystem
    /// delegateInstance)`: the delegate is `delegateInstance.provider()`, and
    /// the filesystem is created by [`install`].
    pub fn new(scheme: &str, delegate_instance: &Arc<dyn FileSystem>) -> FilterState {
        FilterState {
            delegate: delegate_instance.provider(),
            file_system: OnceLock::new(),
            scheme: scheme.to_string(),
        }
    }

    /// The state of `FilterFileSystemProvider(String scheme,
    /// FileSystemProvider delegate)`: "You must set the singleton
    /// `filesystem` yourself" ([`set_file_system`](Self::set_file_system)).
    pub fn with_provider(scheme: &str, delegate: Arc<dyn FileSystemProvider>) -> FilterState {
        FilterState {
            delegate,
            file_system: OnceLock::new(),
            scheme: scheme.to_string(),
        }
    }

    /// `delegate`.
    pub fn delegate(&self) -> &Arc<dyn FileSystemProvider> {
        &self.delegate
    }

    /// `fileSystem`, `None` until set.
    pub fn file_system(&self) -> Option<&Arc<FilterFileSystem>> {
        self.file_system.get()
    }

    /// Sets the singleton `fileSystem`.
    ///
    /// # Panics
    ///
    /// When it was already set.
    pub fn set_file_system(&self, file_system: Arc<FilterFileSystem>) {
        if self.file_system.set(file_system).is_err() {
            throw_unchecked(
                "java.lang.IllegalStateException",
                Some("filesystem already set"),
            )
        }
    }

    /// `scheme`.
    pub fn scheme(&self) -> &str {
        &self.scheme
    }

    /// The singleton filesystem.
    ///
    /// # Panics
    ///
    /// `IllegalStateException` ("subclass did not initialize singleton
    /// filesystem") when it was not set.
    pub fn require_file_system(&self) -> &Arc<FilterFileSystem> {
        match self.file_system.get() {
            Some(fs) => fs,
            None => throw_unchecked(
                "java.lang.IllegalStateException",
                Some("subclass did not initialize singleton filesystem"),
            ),
        }
    }

    /// `toDelegate(path)`: the delegate path of a path of this provider's
    /// filesystem.
    ///
    /// # Panics
    ///
    /// `ProviderMismatchException` ("mismatch, expected: class …, got: class
    /// …") for a path of another filter filesystem, and ("mismatch, expected:
    /// FilterPath, got: class sun.nio.fs.UnixPath") for a path of a
    /// filesystem that is not a filter.
    pub fn to_delegate(&self, path: &Path) -> Path {
        let fs = self.require_file_system();
        to_delegate_of(fs, path)
    }
}

/// `toDelegate(path)` against the filter filesystem `fs`, shared by
/// [`FilterState::to_delegate`] and `FilterPath.toDelegate`.
pub(crate) fn to_delegate_of(fs: &Arc<FilterFileSystem>, path: &Path) -> Path {
    let own: Arc<dyn FileSystem> = Arc::clone(fs) as Arc<dyn FileSystem>;
    if let Some(other) = path
        .get_file_system()
        .as_any()
        .downcast_ref::<FilterFileSystem>()
    {
        if !same_file_system(path.get_file_system(), &own) {
            throw_unchecked(
                "java.nio.file.ProviderMismatchException",
                Some(&format!(
                    "mismatch, expected: class {}, got: class {}",
                    fs.provider().provider_class_name(),
                    other.provider().provider_class_name()
                )),
            )
        }
        return path.with_file_system(Arc::clone(fs.get_delegate()));
    }
    throw_unchecked(
        "java.nio.file.ProviderMismatchException",
        Some("mismatch, expected: FilterPath, got: class sun.nio.fs.UnixPath"),
    )
}

/// Creates the singleton filesystem of `this`, `new FilterFileSystem(this,
/// delegateInstance)`, as the constructor `FilterFileSystemProvider(String,
/// FileSystem)` does.
pub fn install<T: FilterFileSystemProvider + 'static>(
    this: &Arc<T>,
    delegate_instance: Arc<dyn FileSystem>,
) {
    let parent: Arc<dyn FilterFileSystemProvider> =
        Arc::clone(this) as Arc<dyn FilterFileSystemProvider>;
    this.filter()
        .set_file_system(Arc::new(FilterFileSystem::new(parent, delegate_instance)));
}

/// Port of `org.apache.lucene.tests.mockfile.FilterFileSystemProvider`: a provider that contains another provider,
/// which it uses as its basic source of data, possibly transforming the data
/// along the way or providing additional functionality.
///
/// Every method defaults to the Java body; the free functions of this module
/// are the same bodies, for an override to call as `super`.
pub trait FilterFileSystemProvider: FileSystemProvider {
    /// The fields of the class.
    fn filter(&self) -> &FilterState;

    /// `getClass().getName()`.
    fn class_name(&self) -> &'static str;

    /// `getClass().getSimpleName()`.
    fn simple_name(&self) -> &'static str {
        let name = self.class_name();
        name.rsplit(['.', '$']).next().unwrap_or(name)
    }

    /// `wrapPath(path)`: wraps a delegate path with provider-specific
    /// behavior, a [`FilterPath`] by default.
    fn wrap_path(&self, path: &Path) -> Path {
        FilterPath::new(path, self.filter().require_file_system())
    }

    /// `onClose()`: overridden to trigger some behavior when the filesystem
    /// is closed. This is always called for each `FilterFileSystemProvider`
    /// in the chain.
    fn on_close(&self) {}

    /// `getScheme()`.
    fn get_scheme(&self) -> String {
        self.filter().scheme().to_string()
    }

    /// `getFileSystem(URI)`.
    fn get_file_system(&self) -> Arc<dyn FileSystem> {
        let fs: Arc<dyn FileSystem> =
            Arc::clone(self.filter().require_file_system()) as Arc<dyn FileSystem>;
        fs
    }

    /// `newFileSystem(URI, Map)` / `newFileSystem(Path, Map)`: the singleton.
    fn new_file_system(&self) -> io::Result<Arc<dyn FileSystem>> {
        Ok(FilterFileSystemProvider::get_file_system(self))
    }

    /// `getPath(URI)`: `wrapPath(delegate.getPath(uri))`.
    fn get_path(&self, uri: &str) -> Path {
        get_path(self, uri)
    }

    /// `FilterPath.resolve(Path)`: `wrap(delegate.resolve(toDelegate(other)))`.
    fn resolve(&self, base: &Path, other: &Path) -> Path {
        resolve(self, base, other)
    }

    /// `createDirectory(dir, attrs...)`.
    fn create_directory(&self, dir: &Path) -> io::Result<()> {
        create_directory(self, dir)
    }

    /// `delete(path)`.
    fn delete(&self, path: &Path) -> io::Result<()> {
        delete(self, path)
    }

    /// `copy(source, target, options...)`.
    fn copy(&self, source: &Path, target: &Path, options: &[CopyOption]) -> io::Result<()> {
        copy(self, source, target, options)
    }

    /// `move(source, target, options...)`.
    fn r#move(&self, source: &Path, target: &Path, options: &[CopyOption]) -> io::Result<()> {
        r#move(self, source, target, options)
    }

    /// `isSameFile(path, path2)`.
    fn is_same_file(&self, path: &Path, path2: &Path) -> io::Result<bool> {
        let f = self.filter();
        f.delegate()
            .is_same_file(&f.to_delegate(path), &f.to_delegate(path2))
    }

    /// `isHidden(path)`.
    fn is_hidden(&self, path: &Path) -> io::Result<bool> {
        let f = self.filter();
        f.delegate().is_hidden(&f.to_delegate(path))
    }

    /// `getFileStore(path)`: the delegate's store, unwrapped.
    fn get_file_store(&self, path: &Path) -> io::Result<Arc<dyn FileStore>> {
        let f = self.filter();
        f.delegate().get_file_store(&f.to_delegate(path))
    }

    /// `checkAccess(path, modes...)`.
    fn check_access(&self, path: &Path, modes: &[AccessMode]) -> io::Result<()> {
        let f = self.filter();
        f.delegate().check_access(&f.to_delegate(path), modes)
    }

    /// `readAttributes(path, BasicFileAttributes.class, options...)`.
    fn read_attributes(
        &self,
        path: &Path,
        options: &[LinkOption],
    ) -> io::Result<BasicFileAttributes> {
        let f = self.filter();
        f.delegate().read_attributes(&f.to_delegate(path), options)
    }

    /// `readAttributes(path, String, options...)`.
    fn read_attributes_by_name(
        &self,
        path: &Path,
        attributes: &str,
        options: &[LinkOption],
    ) -> io::Result<Vec<(String, AttributeValue)>> {
        let f = self.filter();
        f.delegate()
            .read_attributes_by_name(&f.to_delegate(path), attributes, options)
    }

    /// `setAttribute(path, attribute, value, options...)`.
    fn set_attribute(
        &self,
        path: &Path,
        attribute: &str,
        value: &AttributeValue,
        options: &[LinkOption],
    ) -> io::Result<()> {
        set_attribute(self, path, attribute, value, options)
    }

    /// `newInputStream(path, options...)`.
    fn new_input_stream(
        &self,
        path: &Path,
        options: &[OpenOption],
    ) -> io::Result<Box<dyn InputStream>> {
        new_input_stream(self, path, options)
    }

    /// `newOutputStream(path, options...)`.
    fn new_output_stream(
        &self,
        path: &Path,
        options: &[OpenOption],
    ) -> io::Result<Box<dyn OutputStream>> {
        new_output_stream(self, path, options)
    }

    /// `newFileChannel(path, options, attrs...)`.
    fn new_file_channel(
        &self,
        path: &Path,
        options: &[OpenOption],
    ) -> io::Result<Arc<dyn FileChannel>> {
        new_file_channel(self, path, options)
    }

    /// `newAsynchronousFileChannel(path, options, executor, attrs...)`.
    fn new_asynchronous_file_channel(
        &self,
        path: &Path,
        options: &[OpenOption],
    ) -> io::Result<Arc<dyn AsynchronousFileChannel>> {
        new_asynchronous_file_channel(self, path, options)
    }

    /// `newByteChannel(path, options, attrs...)`.
    fn new_byte_channel(
        &self,
        path: &Path,
        options: &[OpenOption],
    ) -> io::Result<Box<dyn SeekableByteChannel>> {
        new_byte_channel(self, path, options)
    }

    /// `newDirectoryStream(dir, filter)`: the filter sees wrapped paths, and
    /// the stream hands out wrapped paths.
    fn new_directory_stream(
        &self,
        dir: &Path,
        filter: DirectoryStreamFilter,
    ) -> io::Result<Box<dyn DirectoryStream>> {
        new_directory_stream(self, dir, filter)
    }

    /// `createSymbolicLink(link, target, attrs...)`.
    fn create_symbolic_link(&self, link: &Path, target: &Path) -> io::Result<()> {
        create_symbolic_link(self, link, target)
    }

    /// `createLink(link, existing)`.
    fn create_link(&self, link: &Path, existing: &Path) -> io::Result<()> {
        create_link(self, link, existing)
    }

    /// `deleteIfExists(path)`.
    fn delete_if_exists(&self, path: &Path) -> io::Result<bool> {
        delete_if_exists(self, path)
    }

    /// `readSymbolicLink(link)`: the delegate's path, not wrapped, as Lucene
    /// returns it.
    fn read_symbolic_link(&self, link: &Path) -> io::Result<Path> {
        let f = self.filter();
        f.delegate().read_symbolic_link(&f.to_delegate(link))
    }

    /// `toString()`: `getClass().getSimpleName() + "(" + delegate + ")"`.
    fn provider_to_string(&self) -> String {
        format!(
            "{}({})",
            self.simple_name(),
            self.filter().delegate().provider_to_string()
        )
    }
}

/// `FilterFileSystemProvider.getPath(URI)`.
pub fn get_path<P: FilterFileSystemProvider + ?Sized>(this: &P, uri: &str) -> Path {
    let f = this.filter();
    f.require_file_system();
    this.wrap_path(&f.delegate().get_path(uri))
}

/// `FilterPath.resolve(Path)`.
pub fn resolve<P: FilterFileSystemProvider + ?Sized>(this: &P, base: &Path, other: &Path) -> Path {
    let f = this.filter();
    let delegate_base = f.to_delegate(base);
    let delegate_other = f.to_delegate(other);
    let delegate_provider = delegate_base.get_file_system().provider();
    this.wrap_path(&delegate_provider.resolve(&delegate_base, &delegate_other))
}

/// `FilterFileSystemProvider.createDirectory`.
pub fn create_directory<P: FilterFileSystemProvider + ?Sized>(
    this: &P,
    dir: &Path,
) -> io::Result<()> {
    let f = this.filter();
    f.delegate().create_directory(&f.to_delegate(dir))
}

/// `FilterFileSystemProvider.delete`.
pub fn delete<P: FilterFileSystemProvider + ?Sized>(this: &P, path: &Path) -> io::Result<()> {
    let f = this.filter();
    f.delegate().delete(&f.to_delegate(path))
}

/// `FilterFileSystemProvider.copy`.
pub fn copy<P: FilterFileSystemProvider + ?Sized>(
    this: &P,
    source: &Path,
    target: &Path,
    options: &[CopyOption],
) -> io::Result<()> {
    let f = this.filter();
    f.delegate()
        .copy(&f.to_delegate(source), &f.to_delegate(target), options)
}

/// `FilterFileSystemProvider.move`.
pub fn r#move<P: FilterFileSystemProvider + ?Sized>(
    this: &P,
    source: &Path,
    target: &Path,
    options: &[CopyOption],
) -> io::Result<()> {
    let f = this.filter();
    f.delegate()
        .r#move(&f.to_delegate(source), &f.to_delegate(target), options)
}

/// `FilterFileSystemProvider.setAttribute`.
pub fn set_attribute<P: FilterFileSystemProvider + ?Sized>(
    this: &P,
    path: &Path,
    attribute: &str,
    value: &AttributeValue,
    options: &[LinkOption],
) -> io::Result<()> {
    let f = this.filter();
    f.delegate()
        .set_attribute(&f.to_delegate(path), attribute, value, options)
}

/// `FilterFileSystemProvider.newInputStream`.
pub fn new_input_stream<P: FilterFileSystemProvider + ?Sized>(
    this: &P,
    path: &Path,
    options: &[OpenOption],
) -> io::Result<Box<dyn InputStream>> {
    let f = this.filter();
    f.delegate().new_input_stream(&f.to_delegate(path), options)
}

/// `FilterFileSystemProvider.newOutputStream`.
pub fn new_output_stream<P: FilterFileSystemProvider + ?Sized>(
    this: &P,
    path: &Path,
    options: &[OpenOption],
) -> io::Result<Box<dyn OutputStream>> {
    let f = this.filter();
    f.delegate()
        .new_output_stream(&f.to_delegate(path), options)
}

/// `FilterFileSystemProvider.newFileChannel`.
pub fn new_file_channel<P: FilterFileSystemProvider + ?Sized>(
    this: &P,
    path: &Path,
    options: &[OpenOption],
) -> io::Result<Arc<dyn FileChannel>> {
    let f = this.filter();
    f.delegate().new_file_channel(&f.to_delegate(path), options)
}

/// `FilterFileSystemProvider.newAsynchronousFileChannel`.
pub fn new_asynchronous_file_channel<P: FilterFileSystemProvider + ?Sized>(
    this: &P,
    path: &Path,
    options: &[OpenOption],
) -> io::Result<Arc<dyn AsynchronousFileChannel>> {
    let f = this.filter();
    f.delegate()
        .new_asynchronous_file_channel(&f.to_delegate(path), options)
}

/// `FilterFileSystemProvider.newByteChannel`.
pub fn new_byte_channel<P: FilterFileSystemProvider + ?Sized>(
    this: &P,
    path: &Path,
    options: &[OpenOption],
) -> io::Result<Box<dyn SeekableByteChannel>> {
    let f = this.filter();
    f.delegate().new_byte_channel(&f.to_delegate(path), options)
}

/// The filter a filter provider hands its delegate: `filter.accept(wrapPath(entry))`.
pub fn wrapped_filter(
    wrap: Arc<dyn Fn(&Path) -> Path + Send + Sync>,
    filter: DirectoryStreamFilter,
) -> DirectoryStreamFilter {
    Arc::new(move |entry: &Path| filter(&wrap(entry)))
}

/// The `wrapPath` of `this` as a shareable function.
pub fn wrapper_of<P: FilterFileSystemProvider + ?Sized>(
    this: &P,
) -> Arc<dyn Fn(&Path) -> Path + Send + Sync> {
    let fs = Arc::clone(this.filter().require_file_system());
    Arc::new(move |path: &Path| fs.wrap_path(path))
}

/// `FilterFileSystemProvider.newDirectoryStream`.
pub fn new_directory_stream<P: FilterFileSystemProvider + ?Sized>(
    this: &P,
    dir: &Path,
    filter: DirectoryStreamFilter,
) -> io::Result<Box<dyn DirectoryStream>> {
    let f = this.filter();
    let wrapped = wrapped_filter(wrapper_of(this), filter);
    let stream = f
        .delegate()
        .new_directory_stream(&f.to_delegate(dir), wrapped)?;
    Ok(Box::new(FilterDirectoryStream::new(
        stream,
        Arc::clone(f.require_file_system()),
    )))
}

/// `FilterFileSystemProvider.createSymbolicLink`.
pub fn create_symbolic_link<P: FilterFileSystemProvider + ?Sized>(
    this: &P,
    link: &Path,
    target: &Path,
) -> io::Result<()> {
    let f = this.filter();
    f.delegate()
        .create_symbolic_link(&f.to_delegate(link), &f.to_delegate(target))
}

/// `FilterFileSystemProvider.createLink`.
pub fn create_link<P: FilterFileSystemProvider + ?Sized>(
    this: &P,
    link: &Path,
    existing: &Path,
) -> io::Result<()> {
    let f = this.filter();
    f.delegate()
        .create_link(&f.to_delegate(link), &f.to_delegate(existing))
}

/// `FilterFileSystemProvider.deleteIfExists`.
pub fn delete_if_exists<P: FilterFileSystemProvider + ?Sized>(
    this: &P,
    path: &Path,
) -> io::Result<bool> {
    let f = this.filter();
    f.delegate().delete_if_exists(&f.to_delegate(path))
}

/// The upcast of a filter provider to its `Any`, for `instanceof` tests.
pub fn as_any<P: FilterFileSystemProvider + 'static>(this: &P) -> &dyn Any {
    this
}

/// Makes a [`FilterFileSystemProvider`] a
/// `java.nio.file.spi.FileSystemProvider`: every provider method forwards to
/// the type's [`FilterFileSystemProvider`] method, and `exists` and
/// `readAttributesIfExists` keep the JDK's defaults, as Lucene's class does
/// not override them.
#[macro_export]
macro_rules! impl_file_system_provider {
    ($t:ty) => {
        impl ::rucene::store::java_nio_file::FileSystemProvider for $t {
            fn get_scheme(&self) -> String {
                <Self as $crate::mockfile::FilterFileSystemProvider>::get_scheme(self)
            }
            fn get_file_system(&self) -> ::std::sync::Arc<dyn ::rucene::store::java_nio_file::FileSystem> {
                <Self as $crate::mockfile::FilterFileSystemProvider>::get_file_system(self)
            }
            fn get_path(&self, uri: &str) -> ::rucene::store::java_nio_file::Path {
                <Self as $crate::mockfile::FilterFileSystemProvider>::get_path(self, uri)
            }
            fn resolve(
                &self,
                base: &::rucene::store::java_nio_file::Path,
                other: &::rucene::store::java_nio_file::Path,
            ) -> ::rucene::store::java_nio_file::Path {
                <Self as $crate::mockfile::FilterFileSystemProvider>::resolve(self, base, other)
            }
            fn create_directory(&self, dir: &::rucene::store::java_nio_file::Path) -> ::std::io::Result<()> {
                <Self as $crate::mockfile::FilterFileSystemProvider>::create_directory(self, dir)
            }
            fn delete(&self, path: &::rucene::store::java_nio_file::Path) -> ::std::io::Result<()> {
                <Self as $crate::mockfile::FilterFileSystemProvider>::delete(self, path)
            }
            fn delete_if_exists(&self, path: &::rucene::store::java_nio_file::Path) -> ::std::io::Result<bool> {
                <Self as $crate::mockfile::FilterFileSystemProvider>::delete_if_exists(self, path)
            }
            fn copy(
                &self,
                source: &::rucene::store::java_nio_file::Path,
                target: &::rucene::store::java_nio_file::Path,
                options: &[::rucene::store::java_nio_file::CopyOption],
            ) -> ::std::io::Result<()> {
                <Self as $crate::mockfile::FilterFileSystemProvider>::copy(self, source, target, options)
            }
            fn r#move(
                &self,
                source: &::rucene::store::java_nio_file::Path,
                target: &::rucene::store::java_nio_file::Path,
                options: &[::rucene::store::java_nio_file::CopyOption],
            ) -> ::std::io::Result<()> {
                <Self as $crate::mockfile::FilterFileSystemProvider>::r#move(self, source, target, options)
            }
            fn is_same_file(
                &self,
                path: &::rucene::store::java_nio_file::Path,
                path2: &::rucene::store::java_nio_file::Path,
            ) -> ::std::io::Result<bool> {
                <Self as $crate::mockfile::FilterFileSystemProvider>::is_same_file(self, path, path2)
            }
            fn is_hidden(&self, path: &::rucene::store::java_nio_file::Path) -> ::std::io::Result<bool> {
                <Self as $crate::mockfile::FilterFileSystemProvider>::is_hidden(self, path)
            }
            fn get_file_store(
                &self,
                path: &::rucene::store::java_nio_file::Path,
            ) -> ::std::io::Result<::std::sync::Arc<dyn ::rucene::store::java_nio_file::FileStore>> {
                <Self as $crate::mockfile::FilterFileSystemProvider>::get_file_store(self, path)
            }
            fn check_access(
                &self,
                path: &::rucene::store::java_nio_file::Path,
                modes: &[::rucene::store::java_nio_file::AccessMode],
            ) -> ::std::io::Result<()> {
                <Self as $crate::mockfile::FilterFileSystemProvider>::check_access(self, path, modes)
            }
            fn read_attributes(
                &self,
                path: &::rucene::store::java_nio_file::Path,
                options: &[::rucene::store::java_nio_file::LinkOption],
            ) -> ::std::io::Result<::rucene::store::java_nio_file::BasicFileAttributes> {
                <Self as $crate::mockfile::FilterFileSystemProvider>::read_attributes(self, path, options)
            }
            fn read_attributes_by_name(
                &self,
                path: &::rucene::store::java_nio_file::Path,
                attributes: &str,
                options: &[::rucene::store::java_nio_file::LinkOption],
            ) -> ::std::io::Result<Vec<(String, ::rucene::store::java_nio_file::AttributeValue)>> {
                <Self as $crate::mockfile::FilterFileSystemProvider>::read_attributes_by_name(
                    self, path, attributes, options,
                )
            }
            fn set_attribute(
                &self,
                path: &::rucene::store::java_nio_file::Path,
                attribute: &str,
                value: &::rucene::store::java_nio_file::AttributeValue,
                options: &[::rucene::store::java_nio_file::LinkOption],
            ) -> ::std::io::Result<()> {
                <Self as $crate::mockfile::FilterFileSystemProvider>::set_attribute(
                    self, path, attribute, value, options,
                )
            }
            fn new_input_stream(
                &self,
                path: &::rucene::store::java_nio_file::Path,
                options: &[::rucene::store::java_nio_file::OpenOption],
            ) -> ::std::io::Result<Box<dyn ::rucene::store::java_nio_file::InputStream>> {
                <Self as $crate::mockfile::FilterFileSystemProvider>::new_input_stream(self, path, options)
            }
            fn new_output_stream(
                &self,
                path: &::rucene::store::java_nio_file::Path,
                options: &[::rucene::store::java_nio_file::OpenOption],
            ) -> ::std::io::Result<Box<dyn ::rucene::store::java_nio_file::OutputStream>> {
                <Self as $crate::mockfile::FilterFileSystemProvider>::new_output_stream(self, path, options)
            }
            fn new_file_channel(
                &self,
                path: &::rucene::store::java_nio_file::Path,
                options: &[::rucene::store::java_nio_file::OpenOption],
            ) -> ::std::io::Result<::std::sync::Arc<dyn ::rucene::store::java_nio_file::FileChannel>> {
                <Self as $crate::mockfile::FilterFileSystemProvider>::new_file_channel(self, path, options)
            }
            fn new_asynchronous_file_channel(
                &self,
                path: &::rucene::store::java_nio_file::Path,
                options: &[::rucene::store::java_nio_file::OpenOption],
            ) -> ::std::io::Result<
                ::std::sync::Arc<dyn ::rucene::store::java_nio_file::AsynchronousFileChannel>,
            > {
                <Self as $crate::mockfile::FilterFileSystemProvider>::new_asynchronous_file_channel(
                    self, path, options,
                )
            }
            fn new_byte_channel(
                &self,
                path: &::rucene::store::java_nio_file::Path,
                options: &[::rucene::store::java_nio_file::OpenOption],
            ) -> ::std::io::Result<Box<dyn ::rucene::store::java_nio_file::SeekableByteChannel>> {
                <Self as $crate::mockfile::FilterFileSystemProvider>::new_byte_channel(self, path, options)
            }
            fn new_directory_stream(
                &self,
                dir: &::rucene::store::java_nio_file::Path,
                filter: ::rucene::store::java_nio_file::DirectoryStreamFilter,
            ) -> ::std::io::Result<Box<dyn ::rucene::store::java_nio_file::DirectoryStream>> {
                <Self as $crate::mockfile::FilterFileSystemProvider>::new_directory_stream(self, dir, filter)
            }
            fn create_symbolic_link(
                &self,
                link: &::rucene::store::java_nio_file::Path,
                target: &::rucene::store::java_nio_file::Path,
            ) -> ::std::io::Result<()> {
                <Self as $crate::mockfile::FilterFileSystemProvider>::create_symbolic_link(self, link, target)
            }
            fn create_link(
                &self,
                link: &::rucene::store::java_nio_file::Path,
                existing: &::rucene::store::java_nio_file::Path,
            ) -> ::std::io::Result<()> {
                <Self as $crate::mockfile::FilterFileSystemProvider>::create_link(self, link, existing)
            }
            fn read_symbolic_link(
                &self,
                link: &::rucene::store::java_nio_file::Path,
            ) -> ::std::io::Result<::rucene::store::java_nio_file::Path> {
                <Self as $crate::mockfile::FilterFileSystemProvider>::read_symbolic_link(self, link)
            }
            fn provider_class_name(&self) -> String {
                <Self as $crate::mockfile::FilterFileSystemProvider>::class_name(self).to_string()
            }
            fn provider_to_string(&self) -> String {
                <Self as $crate::mockfile::FilterFileSystemProvider>::provider_to_string(self)
            }
            fn as_any(&self) -> &dyn ::std::any::Any {
                self
            }
        }
    };
}
