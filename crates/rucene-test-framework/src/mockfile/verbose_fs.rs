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

//! Port of `org.apache.lucene.tests.mockfile.VerboseFS`.

use std::io;
use std::panic::{catch_unwind, resume_unwind, AssertUnwindSafe};
use std::sync::{Arc, OnceLock};

use rucene::store::java_nio_file::{
    java_to_string, options_to_string, AsynchronousFileChannel, AttributeValue, CopyOption,
    FileChannel, FileSystem, LinkOption, OpenOption, OutputStream, Path, SeekableByteChannel,
};
use rucene::util::InfoStream;

use super::filter_file_system_provider::{
    self as ffsp, install, FilterFileSystemProvider, FilterState,
};

/// Port of `org.apache.lucene.tests.mockfile.VerboseFS`: a filesystem that
/// records all major destructive filesystem activities.
pub struct VerboseFS {
    filter: FilterState,
    /// `infoStream`: the component for messages is named `FS`.
    info_stream: Arc<dyn InfoStream>,
    /// `root`: the current directory, absolute and normalized, as a path of
    /// this filesystem; set by the constructor once the filesystem exists.
    root: OnceLock<Path>,
}

/// What a wrapped call threw: Java's `Throwable exception`.
enum Thrown {
    Io(io::Error),
    Unchecked(Box<dyn std::any::Any + Send>),
}

impl Thrown {
    /// `Throwable.toString()`.
    fn to_java_string(&self) -> String {
        match self {
            Thrown::Io(error) => java_to_string(error),
            Thrown::Unchecked(payload) => {
                randomizedtesting::jdk::exception::panic_message(payload.as_ref())
            }
        }
    }
}

/// Runs `call`, catching what it throws, `try { ... } catch (Throwable t)`.
fn attempt<T>(call: impl FnOnce() -> io::Result<T>) -> Result<T, Thrown> {
    match catch_unwind(AssertUnwindSafe(call)) {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(error)) => Err(Thrown::Io(error)),
        Err(payload) => Err(Thrown::Unchecked(payload)),
    }
}

/// `IOUtils.rethrowAlways(exception)`.
fn rethrow<T>(thrown: Thrown) -> io::Result<T> {
    match thrown {
        Thrown::Io(error) => Err(error),
        Thrown::Unchecked(payload) => resume_unwind(payload),
    }
}

impl VerboseFS {
    /// `VerboseFS(delegate, infoStream)`, with the scheme `verbose://`: records
    /// major filesystem write activities (create, delete, etc) to the
    /// `InfoStream`.
    pub fn new(delegate: Arc<dyn FileSystem>, info_stream: Arc<dyn InfoStream>) -> Arc<VerboseFS> {
        let this = Arc::new(VerboseFS {
            filter: FilterState::new("verbose://", &delegate),
            info_stream,
            root: OnceLock::new(),
        });
        install(&this, delegate);
        // `this.getFileSystem(null).getPath(".").toAbsolutePath().normalize()`.
        let root = FilterFileSystemProvider::get_file_system(&*this)
            .get_path(".", &[])
            .to_absolute_path()
            .normalize();
        let _ = this.root.set(root);
        this
    }

    /// `sop(text, exception)`: records the message, and rethrows the exception
    /// if there is one.
    fn sop<T>(&self, text: String, outcome: Result<T, Thrown>) -> io::Result<T> {
        match outcome {
            Ok(value) => {
                if self.info_stream.is_enabled("FS") {
                    self.info_stream.message("FS", &text);
                }
                Ok(value)
            }
            Err(thrown) => {
                if self.info_stream.is_enabled("FS") {
                    self.info_stream.message(
                        "FS",
                        &format!("{text} (FAILED: {})", thrown.to_java_string()),
                    );
                }
                rethrow(thrown)
            }
        }
    }

    /// `path(path)`: `root.relativize(path.toAbsolutePath().normalize())`.
    fn path(&self, path: &Path) -> String {
        self.root
            .get()
            .expect("INVARIANT: the constructor sets root")
            .relativize(&path.to_absolute_path().normalize())
            .to_string()
    }

    fn contains_destructive(options: &[OpenOption]) -> bool {
        options.contains(&OpenOption::Append)
            || options.contains(&OpenOption::Write)
            || options.contains(&OpenOption::DeleteOnClose)
    }

    /// The `finally` of the channel-opening methods: destructive options are
    /// recorded; otherwise only the exception is rethrown.
    fn channel_sop<T>(
        &self,
        method: &str,
        path: &Path,
        options: &[OpenOption],
        outcome: Result<T, Thrown>,
    ) -> io::Result<T> {
        if Self::contains_destructive(options) {
            self.sop(
                format!(
                    "{method}{}: {}",
                    options_to_string(options),
                    self.path(path)
                ),
                outcome,
            )
        } else {
            match outcome {
                Ok(value) => Ok(value),
                Err(thrown) => rethrow(thrown),
            }
        }
    }
}

impl FilterFileSystemProvider for VerboseFS {
    fn filter(&self) -> &FilterState {
        &self.filter
    }

    fn class_name(&self) -> &'static str {
        "org.apache.lucene.tests.mockfile.VerboseFS"
    }

    fn create_directory(&self, dir: &Path) -> io::Result<()> {
        let outcome = attempt(|| ffsp::create_directory(self, dir));
        self.sop(format!("createDirectory: {}", self.path(dir)), outcome)
    }

    fn delete(&self, path: &Path) -> io::Result<()> {
        let outcome = attempt(|| ffsp::delete(self, path));
        self.sop(format!("delete: {}", self.path(path)), outcome)
    }

    fn copy(&self, source: &Path, target: &Path, options: &[CopyOption]) -> io::Result<()> {
        let outcome = attempt(|| ffsp::copy(self, source, target, options));
        self.sop(
            format!(
                "copy{}: {} -> {}",
                options_to_string(options),
                self.path(source),
                self.path(target)
            ),
            outcome,
        )
    }

    fn r#move(&self, source: &Path, target: &Path, options: &[CopyOption]) -> io::Result<()> {
        let outcome = attempt(|| ffsp::r#move(self, source, target, options));
        self.sop(
            format!(
                "move{}: {} -> {}",
                options_to_string(options),
                self.path(source),
                self.path(target)
            ),
            outcome,
        )
    }

    fn set_attribute(
        &self,
        path: &Path,
        attribute: &str,
        value: &AttributeValue,
        options: &[LinkOption],
    ) -> io::Result<()> {
        let outcome = attempt(|| ffsp::set_attribute(self, path, attribute, value, options));
        self.sop(
            format!("setAttribute[{attribute}={value}]: {}", self.path(path)),
            outcome,
        )
    }

    fn new_output_stream(
        &self,
        path: &Path,
        options: &[OpenOption],
    ) -> io::Result<Box<dyn OutputStream>> {
        let outcome = attempt(|| ffsp::new_output_stream(self, path, options));
        self.sop(
            format!(
                "newOutputStream{}: {}",
                options_to_string(options),
                self.path(path)
            ),
            outcome,
        )
    }

    fn new_file_channel(
        &self,
        path: &Path,
        options: &[OpenOption],
    ) -> io::Result<Arc<dyn FileChannel>> {
        let outcome = attempt(|| ffsp::new_file_channel(self, path, options));
        self.channel_sop("newFileChannel", path, options, outcome)
    }

    fn new_asynchronous_file_channel(
        &self,
        path: &Path,
        options: &[OpenOption],
    ) -> io::Result<Arc<dyn AsynchronousFileChannel>> {
        let outcome = attempt(|| ffsp::new_asynchronous_file_channel(self, path, options));
        self.channel_sop("newAsynchronousFileChannel", path, options, outcome)
    }

    fn new_byte_channel(
        &self,
        path: &Path,
        options: &[OpenOption],
    ) -> io::Result<Box<dyn SeekableByteChannel>> {
        let outcome = attempt(|| ffsp::new_byte_channel(self, path, options));
        self.channel_sop("newByteChannel", path, options, outcome)
    }

    fn create_symbolic_link(&self, link: &Path, target: &Path) -> io::Result<()> {
        let outcome = attempt(|| ffsp::create_symbolic_link(self, link, target));
        self.sop(
            format!(
                "createSymbolicLink: {} -> {}",
                self.path(link),
                self.path(target)
            ),
            outcome,
        )
    }

    fn create_link(&self, link: &Path, existing: &Path) -> io::Result<()> {
        let outcome = attempt(|| ffsp::create_link(self, link, existing));
        self.sop(
            format!("createLink: {} -> {}", self.path(link), self.path(existing)),
            outcome,
        )
    }

    fn delete_if_exists(&self, path: &Path) -> io::Result<bool> {
        let outcome = attempt(|| ffsp::delete_if_exists(self, path));
        self.sop(format!("deleteIfExists: {}", self.path(path)), outcome)
    }
}

crate::impl_file_system_provider!(VerboseFS);
