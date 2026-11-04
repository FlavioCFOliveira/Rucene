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

//! Port of `org.apache.lucene.tests.mockfile.DisableFsyncFS`.

use std::io;
use std::sync::Arc;

use rucene::store::java_nio_file::{
    AsynchronousFileChannel, FileChannel, FileSystem, OpenOption, Path,
};

use super::filter_asynchronous_file_channel::{
    AsynchronousFileChannelOverrides, FilterAsynchronousFileChannel,
};
use super::filter_file_channel::{FileChannelOverrides, FilterFileChannel};
use super::filter_file_system_provider::{
    self as ffsp, install, FilterFileSystemProvider, FilterState,
};

/// Port of `org.apache.lucene.tests.mockfile.DisableFsyncFS`: disables actual
/// calls to fsync. All other filesystem operations are passed thru as normal.
pub struct DisableFsyncFS {
    filter: FilterState,
}

impl DisableFsyncFS {
    /// `DisableFsyncFS(delegate)`, with the scheme `disablefsync://`.
    pub fn new(delegate: Arc<dyn FileSystem>) -> Arc<DisableFsyncFS> {
        let this = Arc::new(DisableFsyncFS {
            filter: FilterState::new("disablefsync://", &delegate),
        });
        install(&this, delegate);
        this
    }
}

/// The anonymous `FilterFileChannel` whose `force` does nothing.
struct NoForce;

impl FileChannelOverrides for NoForce {
    fn force(&self, _delegate: &dyn FileChannel, _meta_data: bool) -> io::Result<()> {
        Ok(())
    }
}

/// The anonymous `FilterAsynchronousFileChannel` whose `force` does nothing.
struct NoAsyncForce;

impl AsynchronousFileChannelOverrides for NoAsyncForce {
    fn force(&self, _delegate: &dyn AsynchronousFileChannel, _meta_data: bool) -> io::Result<()> {
        Ok(())
    }
}

impl FilterFileSystemProvider for DisableFsyncFS {
    fn filter(&self) -> &FilterState {
        &self.filter
    }

    fn class_name(&self) -> &'static str {
        "org.apache.lucene.tests.mockfile.DisableFsyncFS"
    }

    fn new_file_channel(
        &self,
        path: &Path,
        options: &[OpenOption],
    ) -> io::Result<Arc<dyn FileChannel>> {
        Ok(Arc::new(FilterFileChannel::with_overrides(
            ffsp::new_file_channel(self, path, options)?,
            Box::new(NoForce),
            "org.apache.lucene.tests.mockfile.DisableFsyncFS$1",
        )))
    }

    fn new_asynchronous_file_channel(
        &self,
        path: &Path,
        options: &[OpenOption],
    ) -> io::Result<Arc<dyn AsynchronousFileChannel>> {
        Ok(Arc::new(FilterAsynchronousFileChannel::with_overrides(
            ffsp::new_asynchronous_file_channel(self, path, options)?,
            Box::new(NoAsyncForce),
            "org.apache.lucene.tests.mockfile.DisableFsyncFS$2",
        )))
    }
}

crate::impl_file_system_provider!(DisableFsyncFS);
