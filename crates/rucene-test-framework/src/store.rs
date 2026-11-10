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

//! Port of `org.apache.lucene.tests.store`.

/// Implements `DataInput` and `IndexInput` for a subclass of
/// `MockIndexInputWrapper` held as `inner`: every method forwards to the
/// wrapper, `close()` runs `$close` with `$this` bound to `self`, and the
/// type reports itself as its `FilterIndexInput` class.
macro_rules! forward_mock_index_input {
    ($ty:ident, |$this:ident| $close:expr) => {
        impl rucene::store::DataInput for $ty {
            fn to_java_string(&self) -> String {
                self.inner.to_java_string()
            }
            fn read_byte(&mut self) -> rucene::error::Result<u8> {
                self.inner.read_byte()
            }
            fn read_bytes(
                &mut self,
                b: Option<&mut [u8]>,
                offset: usize,
                len: usize,
            ) -> rucene::error::Result<()> {
                self.inner.read_bytes(b, offset, len)
            }
            fn read_bytes_buffered(
                &mut self,
                b: Option<&mut [u8]>,
                offset: usize,
                len: usize,
                use_buffer: bool,
            ) -> rucene::error::Result<()> {
                self.inner.read_bytes_buffered(b, offset, len, use_buffer)
            }
            fn skip_bytes(&mut self, num_bytes: i64) -> rucene::error::Result<()> {
                self.inner.skip_bytes(num_bytes)
            }
            fn read_floats(
                &mut self,
                floats: Option<&mut [f32]>,
                offset: usize,
                len: usize,
            ) -> rucene::error::Result<()> {
                self.inner.read_floats(floats, offset, len)
            }
            fn read_short(&mut self) -> rucene::error::Result<i16> {
                self.inner.read_short()
            }
            fn read_int(&mut self) -> rucene::error::Result<i32> {
                self.inner.read_int()
            }
            fn read_long(&mut self) -> rucene::error::Result<i64> {
                self.inner.read_long()
            }
            fn read_string(&mut self) -> rucene::error::Result<String> {
                self.inner.read_string()
            }
            fn read_v_int(&mut self) -> rucene::error::Result<i32> {
                self.inner.read_v_int()
            }
            fn read_v_long(&mut self) -> rucene::error::Result<i64> {
                self.inner.read_v_long()
            }
            fn read_z_int(&mut self) -> rucene::error::Result<i32> {
                self.inner.read_z_int()
            }
            fn read_z_long(&mut self) -> rucene::error::Result<i64> {
                self.inner.read_z_long()
            }
            fn read_map_of_strings(
                &mut self,
            ) -> rucene::error::Result<rucene::util::java_collections::JavaStringMap> {
                self.inner.read_map_of_strings()
            }
            fn read_set_of_strings(
                &mut self,
            ) -> rucene::error::Result<rucene::util::java_collections::JavaStringSet> {
                self.inner.read_set_of_strings()
            }
        }

        impl rucene::store::IndexInput for $ty {
            fn close(&mut self) -> rucene::error::Result<()> {
                let $this = self;
                $close
            }
            fn file_pointer(&self) -> i64 {
                self.inner.file_pointer()
            }
            fn length(&self) -> i64 {
                self.inner.length()
            }
            fn seek(&mut self, pos: i64) -> rucene::error::Result<()> {
                self.inner.seek(pos)
            }
            fn slice(
                &self,
                slice_description: &str,
                offset: i64,
                length: i64,
            ) -> rucene::error::Result<Box<dyn rucene::store::IndexInput>> {
                self.inner.slice(slice_description, offset, length)
            }
            fn slice_with_context(
                &self,
                slice_description: &str,
                offset: i64,
                length: i64,
                context: &dyn rucene::store::IOContext,
            ) -> rucene::error::Result<Box<dyn rucene::store::IndexInput>> {
                self.inner
                    .slice_with_context(slice_description, offset, length, context)
            }
            fn clone_input(&self) -> rucene::error::Result<Box<dyn rucene::store::IndexInput>> {
                self.inner.clone_input()
            }
            fn resource_description(&self) -> &str {
                self.inner.resource_description()
            }
            fn prefetch(&self, offset: i64, length: i64) -> rucene::error::Result<()> {
                self.inner.prefetch(offset, length)
            }
            fn update_io_context(
                &self,
                context: &dyn rucene::store::IOContext,
            ) -> rucene::error::Result<()> {
                self.inner.update_io_context(context)
            }
            fn is_loaded(&self) -> Option<bool> {
                self.inner.is_loaded()
            }
            fn as_filter_index_input(
                &self,
            ) -> Option<(std::any::TypeId, &dyn rucene::store::IndexInput)> {
                self.inner
                    .as_filter_index_input()
                    .map(|(_, delegate)| (std::any::TypeId::of::<$ty>(), delegate))
            }
        }

        impl std::fmt::Debug for $ty {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                std::fmt::Debug::fmt(&self.inner, f)
            }
        }
    };
}

pub mod base_chunked_directory_test_case;
pub mod base_directory_test_case;
pub mod base_directory_wrapper;
pub mod base_lock_factory_test_case;
pub mod corrupting_index_output;
pub mod mock_directory_wrapper;
pub mod mock_index_input_wrapper;
pub mod mock_index_output_wrapper;
pub mod raw_directory_wrapper;
pub mod serial_io_counting_directory;
pub mod slow_closing_mock_index_input_wrapper;
pub mod slow_opening_mock_index_input_wrapper;

pub use base_directory_wrapper::BaseDirectoryWrapper;
pub use corrupting_index_output::CorruptingIndexOutput;
pub use mock_directory_wrapper::{
    fake_io_exception, Failure, MockDirectoryWrapper, PlainFailure, Throttling, FAKE_IO_EXCEPTION,
};
pub use mock_index_input_wrapper::MockIndexInputWrapper;
pub use mock_index_output_wrapper::MockIndexOutputWrapper;
pub use raw_directory_wrapper::RawDirectoryWrapper;
pub use serial_io_counting_directory::SerialIOCountingDirectory;
pub use slow_closing_mock_index_input_wrapper::SlowClosingMockIndexInputWrapper;
pub use slow_opening_mock_index_input_wrapper::SlowOpeningMockIndexInputWrapper;
