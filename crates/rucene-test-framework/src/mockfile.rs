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

//! Port of `org.apache.lucene.tests.mockfile`: support for testing and
//! debugging with virtual filesystems.
//!
//! The primary classes are:
//!
//! * [`LeakFS`]: fails tests if they leave open file handles.
//! * [`VerboseFS`]: prints destructive filesystem operations to infostream.
//! * [`WindowsFS`]: acts like windows.
//! * [`DisableFsyncFS`]: makes actual fsync calls a no-op.
//! * [`ExtrasFS`]: adds 'bonus' files to directories.
//! * [`ShuffleFS`]: directory listings in an unpredictable but deterministic
//!   order.
//!
//! Every filesystem wraps a `java.nio.file` provider of the store's layer
//! ([`rucene::store::java_nio_file`]) and hands out paths of its own
//! filesystem, so an `FSDirectory` opened on such a path performs every file
//! operation through the wrapper.

pub mod disable_fsync_fs;
pub mod extras_fs;
pub mod filter_asynchronous_file_channel;
pub mod filter_directory_stream;
pub mod filter_file_channel;
pub mod filter_file_store;
pub mod filter_file_system;
pub mod filter_file_system_provider;
pub mod filter_input_stream2;
pub mod filter_output_stream2;
pub mod filter_path;
pub mod filter_seekable_byte_channel;
pub mod handle_limit_fs;
pub mod handle_tracking_fs;
pub mod leak_fs;
pub mod mock_file_system_test_case;
pub mod shuffle_fs;
pub mod verbose_fs;
pub mod virus_checking_fs;
pub mod windows_fs;
pub mod windows_path;

pub use disable_fsync_fs::DisableFsyncFS;
pub use extras_fs::ExtrasFS;
pub use filter_asynchronous_file_channel::FilterAsynchronousFileChannel;
pub use filter_directory_stream::FilterDirectoryStream;
pub use filter_file_channel::FilterFileChannel;
pub use filter_file_store::FilterFileStore;
pub use filter_file_system::FilterFileSystem;
pub use filter_file_system_provider::{FilterFileSystemProvider, FilterState};
pub use filter_input_stream2::FilterInputStream2;
pub use filter_output_stream2::FilterOutputStream2;
pub use filter_path::FilterPath;
pub use filter_seekable_byte_channel::FilterSeekableByteChannel;
pub use handle_limit_fs::{HandleLimitFS, MaxOpenHandles};
pub use handle_tracking_fs::{Handle, HandleTrackingFS, HandleTrackingState};
pub use leak_fs::LeakFS;
pub use mock_file_system_test_case::MockFileSystemTestCase;
pub use shuffle_fs::ShuffleFS;
pub use verbose_fs::VerboseFS;
pub use virus_checking_fs::VirusCheckingFS;
pub use windows_fs::WindowsFS;
pub use windows_path::WindowsPath;

/// `Object.toString()` of an instance of `class_name`: the class name, `@`,
/// and an identity hash of the instance.
pub(crate) fn identity_string<T: ?Sized>(class_name: &str, object: &T) -> String {
    let address = std::ptr::from_ref(object).cast::<()>() as usize;
    format!("{class_name}@{:x}", (address >> 3) as u32)
}
