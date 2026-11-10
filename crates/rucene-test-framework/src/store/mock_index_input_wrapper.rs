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

//! Port of `org.apache.lucene.tests.store.MockIndexInputWrapper`: the input
//! [`MockDirectoryWrapper`] hands out, which keeps track of when it has been
//! closed.
//!
//! Java's `ensureOpen()` and `ensureAccessible()` throw a `RuntimeException`
//! from every method; here a method that returns a `Result` returns it as an
//! `Err`, and one that cannot fail (`getFilePointer()`, `length()`,
//! `isLoaded()`) panics with it.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Once};
use std::thread::ThreadId;

use rucene::error::{LuceneError, Result};
use rucene::internal::tests_hooks::TestSecrets;
use rucene::store::java_nio_file::throw_unchecked;
use rucene::store::{DataInput, IOContext, IndexInput};
use rucene::util::java_collections::{JavaStringMap, JavaStringSet};

use super::mock_directory_wrapper::{runtime_exception, try_with_resources, MockDirectoryWrapper};
use super::slow_closing_mock_index_input_wrapper::SlowClosingMockIndexInputWrapper;
use super::slow_opening_mock_index_input_wrapper::SlowOpeningMockIndexInputWrapper;

/// The state of an input that its directory reaches too: `crash()` closes
/// every open input (Java's `openFileHandles` holds the inputs themselves).
pub(crate) struct InputCtl {
    id: u64,
    name: String,
    /// `closed`.
    closed: AtomicBool,
    /// Whether the input is a [`SlowClosingMockIndexInputWrapper`], whose
    /// `close()` first sleeps 50 ms.
    slow_closing: AtomicBool,
}

impl InputCtl {
    /// `crash()`'s `f.close()` of an open input: `close()` as Java runs it,
    /// with the delegate's `in.close()` left to the input (see the docs of
    /// [`MockDirectoryWrapper`]'s module).
    pub(crate) fn close_from_crash(&self, dir: &MockDirectoryWrapper) -> Result<()> {
        if self.slow_closing.load(Ordering::SeqCst) {
            SlowClosingMockIndexInputWrapper::sleep_before_close()?;
        }
        if self.closed.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        dir.remove_index_input_id(self.id, &self.name);
        dir.maybe_throw_deterministic_exception()
    }
}

/// Registers the wrappers as test filter inputs, once: the `static {}`
/// blocks of `MockIndexInputWrapper`, `SlowClosingMockIndexInputWrapper` and
/// `SlowOpeningMockIndexInputWrapper`
/// (`TestSecrets.getFilterInputIndexAccess().addTestFilterType(...)`).
fn register_test_filter_types() {
    static REGISTERED: Once = Once::new();
    REGISTERED.call_once(|| {
        let access = TestSecrets::get_filter_input_index_access()
            .expect("INVARIANT: FilterIndexInput registers its accessor on first use");
        access.add_test_filter_type(std::any::TypeId::of::<MockIndexInputWrapper>());
        access.add_test_filter_type(std::any::TypeId::of::<SlowClosingMockIndexInputWrapper>());
        access.add_test_filter_type(std::any::TypeId::of::<SlowOpeningMockIndexInputWrapper>());
    });
}

/// Port of `org.apache.lucene.tests.store.MockIndexInputWrapper` (`extends
/// FilterIndexInput`): used by [`MockDirectoryWrapper`] to create an input
/// that keeps track of when it has been closed.
pub struct MockIndexInputWrapper {
    resource_description: String,
    r#in: Box<dyn IndexInput>,
    /// `dir`. A registered input releases its handle once closed.
    dir: Option<MockDirectoryWrapper>,
    name: String,
    ctl: Arc<InputCtl>,
    /// Which input we were cloned from (its state), or `None` if we are not
    /// a clone.
    parent: Option<Arc<InputCtl>>,
    confined: bool,
    thread: ThreadId,
}

impl MockIndexInputWrapper {
    /// `MockIndexInputWrapper(dir, name, delegate, parent, confined)`, with
    /// the resource description `MockIndexInputWrapper(name=<name>
    /// delegate=<delegate>)`.
    ///
    /// # Panics
    ///
    /// Java's `assert parent == null || parent.parent == null` (a clone's
    /// parent is not a clone), under `debug_assertions`.
    pub fn new(
        dir: &MockDirectoryWrapper,
        name: &str,
        delegate: Box<dyn IndexInput>,
        parent: Option<&MockIndexInputWrapper>,
        confined: bool,
    ) -> Self {
        Self::with_parent_ctl(
            dir,
            name,
            delegate,
            parent.map(|p| {
                // If we are a clone then our parent better not be a clone!
                java_assert_unchecked!(p.parent.is_none());
                Arc::clone(&p.ctl)
            }),
            confined,
        )
    }

    fn with_parent_ctl(
        dir: &MockDirectoryWrapper,
        name: &str,
        delegate: Box<dyn IndexInput>,
        parent: Option<Arc<InputCtl>>,
        confined: bool,
    ) -> Self {
        register_test_filter_types();
        MockIndexInputWrapper {
            resource_description: format!(
                "MockIndexInputWrapper(name={name} delegate={})",
                delegate.to_java_string()
            ),
            r#in: delegate,
            dir: Some(dir.handle()),
            name: name.to_string(),
            ctl: Arc::new(InputCtl {
                id: dir.next_handle_id(),
                name: name.to_string(),
                closed: AtomicBool::new(false),
                slow_closing: AtomicBool::new(false),
            }),
            parent,
            confined,
            thread: std::thread::current().id(),
        }
    }

    /// The identity `openFileHandles` knows this input by, and the state
    /// `crash()` closes.
    pub(crate) fn registration(&self) -> (u64, Arc<InputCtl>) {
        (self.ctl.id, Arc::clone(&self.ctl))
    }

    pub(crate) fn handle_id(&self) -> u64 {
        self.ctl.id
    }

    pub(crate) fn mark_slow_closing(&self) {
        self.ctl.slow_closing.store(true, Ordering::SeqCst);
    }

    /// The package-private field `name`.
    pub fn name(&self) -> &str {
        &self.name
    }

    fn is_closed(&self) -> bool {
        self.ctl.closed.load(Ordering::SeqCst)
    }

    fn dir(&self) -> &MockDirectoryWrapper {
        self.dir
            .as_ref()
            .expect("INVARIANT: an input keeps its directory until it is closed")
    }

    /// `ensureOpen()`'s exception, if any.
    fn open_error(&self) -> Option<&'static str> {
        // TODO: not great this is a volatile read (closed) ... we should deploy heavy JVM voodoo like
        // SwitchPoint to avoid this
        if self.is_closed() {
            return Some("Abusing closed IndexInput!");
        }
        if let Some(parent) = &self.parent {
            if parent.closed.load(Ordering::SeqCst) {
                return Some("Abusing clone of a closed IndexInput!");
            }
        }
        None
    }

    /// `ensureAccessible()`'s exception, if any.
    fn access_error(&self) -> Option<&'static str> {
        if self.confined && self.thread != std::thread::current().id() {
            return Some("Abusing from another thread!");
        }
        None
    }

    fn ensure_open(&self) -> Result<()> {
        match self.open_error() {
            Some(message) => Err(runtime_exception(message.to_string(), None)),
            None => Ok(()),
        }
    }

    fn ensure_open_and_accessible(&self) -> Result<()> {
        self.ensure_open()?;
        match self.access_error() {
            Some(message) => Err(runtime_exception(message.to_string(), None)),
            None => Ok(()),
        }
    }

    fn ensure_open_unchecked(&self) {
        if let Some(message) = self.open_error() {
            throw_unchecked("java.lang.RuntimeException", Some(message));
        }
    }

    fn ensure_accessible_unchecked(&self) {
        if let Some(message) = self.access_error() {
            throw_unchecked("java.lang.RuntimeException", Some(message));
        }
    }

    /// `close()`: a second close only closes the delegate again (it does not
    /// mask double-close bugs); the first unregisters a non-clone from its
    /// directory and runs `maybeThrowDeterministicException`, then closes the
    /// delegate.
    pub(crate) fn close_impl(&mut self) -> Result<()> {
        if self.ctl.closed.swap(true, Ordering::SeqCst) {
            if self.parent.is_none() {
                self.dir = None;
            }
            return self.r#in.close(); // don't mask double-close bugs
        }
        // Pending resolution on LUCENE-686 we may want to
        // remove the conditional check so we also track that
        // all clones get closed:
        let body = {
            let dir = self.dir();
            if self.parent.is_none() {
                dir.remove_index_input_id(self.ctl.id, &self.name);
            }
            dir.maybe_throw_deterministic_exception()
        };
        let closed = self.r#in.close();
        if self.parent.is_none() {
            self.dir = None;
        }
        try_with_resources(body, [closed])
    }

    /// `toString()`: `MockIndexInputWrapper(<in>)`.
    fn java_to_string(&self) -> String {
        format!("MockIndexInputWrapper({})", self.r#in.to_java_string())
    }

    /// `clone()` and the two `slice` methods: a new wrapper over `delegate`
    /// whose parent is this input (or this input's parent).
    fn derive(&self, name: &str, delegate: Box<dyn IndexInput>) -> MockIndexInputWrapper {
        let parent = Arc::clone(self.parent.as_ref().unwrap_or(&self.ctl));
        MockIndexInputWrapper::with_parent_ctl(
            self.dir(),
            name,
            delegate,
            Some(parent),
            self.confined,
        )
    }

    /// `slice(sliceDescription, offset, length[, context])`: both forms call
    /// the delegate's three-argument `slice`.
    fn slice_impl(
        &self,
        slice_description: &str,
        offset: i64,
        length: i64,
    ) -> Result<MockIndexInputWrapper> {
        self.ensure_open()?;
        if self.dir().verbose_clone() {
            println!("java.lang.Exception: slice: {}", self.java_to_string());
        }
        self.dir().increment_input_clone_count();
        let slice = self.r#in.slice(slice_description, offset, length)?;
        Ok(self.derive(slice_description, slice))
    }
}

impl DataInput for MockIndexInputWrapper {
    /// `toString()`: `MockIndexInputWrapper(<in>)`.
    fn to_java_string(&self) -> String {
        self.java_to_string()
    }

    fn read_byte(&mut self) -> Result<u8> {
        self.ensure_open_and_accessible()?;
        self.r#in.read_byte()
    }

    fn read_bytes(&mut self, b: Option<&mut [u8]>, offset: usize, len: usize) -> Result<()> {
        self.ensure_open_and_accessible()?;
        self.r#in.read_bytes(b, offset, len)
    }

    fn read_bytes_buffered(
        &mut self,
        b: Option<&mut [u8]>,
        offset: usize,
        len: usize,
        use_buffer: bool,
    ) -> Result<()> {
        self.ensure_open_and_accessible()?;
        self.r#in.read_bytes_buffered(b, offset, len, use_buffer)
    }

    /// `skipBytes(numBytes)`: `IndexInput.skipBytes`, over this wrapper's
    /// `getFilePointer()` and `seek`.
    fn skip_bytes(&mut self, num_bytes: i64) -> Result<()> {
        self.ensure_open_and_accessible()?;
        if num_bytes < 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "numBytes must be >= 0, got {num_bytes}"
            )));
        }
        let skip_to = self.file_pointer() + num_bytes;
        self.seek(skip_to)
    }

    fn read_floats(&mut self, floats: Option<&mut [f32]>, offset: usize, len: usize) -> Result<()> {
        self.ensure_open_and_accessible()?;
        self.r#in.read_floats(floats, offset, len)
    }

    fn read_short(&mut self) -> Result<i16> {
        self.ensure_open_and_accessible()?;
        self.r#in.read_short()
    }

    fn read_int(&mut self) -> Result<i32> {
        self.ensure_open_and_accessible()?;
        self.r#in.read_int()
    }

    fn read_long(&mut self) -> Result<i64> {
        self.ensure_open_and_accessible()?;
        self.r#in.read_long()
    }

    fn read_string(&mut self) -> Result<String> {
        self.ensure_open_and_accessible()?;
        self.r#in.read_string()
    }

    fn read_v_int(&mut self) -> Result<i32> {
        self.ensure_open_and_accessible()?;
        self.r#in.read_v_int()
    }

    fn read_v_long(&mut self) -> Result<i64> {
        self.ensure_open_and_accessible()?;
        self.r#in.read_v_long()
    }

    fn read_z_int(&mut self) -> Result<i32> {
        self.ensure_open_and_accessible()?;
        self.r#in.read_z_int()
    }

    fn read_z_long(&mut self) -> Result<i64> {
        self.ensure_open_and_accessible()?;
        self.r#in.read_z_long()
    }

    fn read_map_of_strings(&mut self) -> Result<JavaStringMap> {
        self.ensure_open_and_accessible()?;
        self.r#in.read_map_of_strings()
    }

    fn read_set_of_strings(&mut self) -> Result<JavaStringSet> {
        self.ensure_open_and_accessible()?;
        self.r#in.read_set_of_strings()
    }
}

impl IndexInput for MockIndexInputWrapper {
    fn close(&mut self) -> Result<()> {
        self.close_impl()
    }

    fn file_pointer(&self) -> i64 {
        self.ensure_open_unchecked();
        self.ensure_accessible_unchecked();
        self.r#in.file_pointer()
    }

    fn length(&self) -> i64 {
        self.ensure_open_unchecked();
        self.r#in.length()
    }

    fn seek(&mut self, pos: i64) -> Result<()> {
        self.ensure_open_and_accessible()?;
        self.r#in.seek(pos)
    }

    fn slice(
        &self,
        slice_description: &str,
        offset: i64,
        length: i64,
    ) -> Result<Box<dyn IndexInput>> {
        Ok(Box::new(self.slice_impl(
            slice_description,
            offset,
            length,
        )?))
    }

    fn slice_with_context(
        &self,
        slice_description: &str,
        offset: i64,
        length: i64,
        _context: &dyn IOContext,
    ) -> Result<Box<dyn IndexInput>> {
        Ok(Box::new(self.slice_impl(
            slice_description,
            offset,
            length,
        )?))
    }

    /// `clone()`: counts the clone in the directory and wraps the
    /// delegate's clone, with this input (or its parent) as the parent.
    fn clone_input(&self) -> Result<Box<dyn IndexInput>> {
        self.ensure_open()?;
        if self.dir().verbose_clone() {
            println!("java.lang.Exception: clone: {}", self.java_to_string());
        }
        self.dir().increment_input_clone_count();
        let iiclone = self.r#in.clone_input()?;
        // Pending resolution on LUCENE-686 we may want to
        // uncomment this code so that we also track that all
        // clones get closed (see Lucene's source).
        Ok(Box::new(self.derive(&self.name.clone(), iiclone)))
    }

    fn resource_description(&self) -> &str {
        &self.resource_description
    }

    fn prefetch(&self, offset: i64, length: i64) -> Result<()> {
        self.ensure_open_and_accessible()?;
        self.r#in.prefetch(offset, length)
    }

    fn update_io_context(&self, context: &dyn IOContext) -> Result<()> {
        self.ensure_open_and_accessible()?;
        self.r#in.update_io_context(context)
    }

    fn is_loaded(&self) -> Option<bool> {
        self.ensure_open_unchecked();
        self.ensure_accessible_unchecked();
        self.r#in.is_loaded()
    }

    /// A `FilterIndexInput` of class `MockIndexInputWrapper`, over its `in`.
    fn as_filter_index_input(&self) -> Option<(std::any::TypeId, &dyn IndexInput)> {
        Some((
            std::any::TypeId::of::<MockIndexInputWrapper>(),
            self.r#in.as_ref(),
        ))
    }
}

impl std::fmt::Debug for MockIndexInputWrapper {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.java_to_string())
    }
}
