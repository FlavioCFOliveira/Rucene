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

//! Port of `org.apache.lucene.tests.mockfile.ShuffleFS`.

use std::io;
use std::sync::Arc;

use randomizedtesting::jdk::random::JdkRandom;
use randomizedtesting::jdk::util::shuffle;
use rucene::store::java_nio_file::{DirectoryStream, DirectoryStreamFilter, FileSystem, Path};

use super::filter_file_system_provider::{
    self as ffsp, install, FilterFileSystemProvider, FilterState,
};
use super::identity_string;

/// Port of `org.apache.lucene.tests.mockfile.ShuffleFS`: gives an
/// unpredictable, but deterministic order to directory listings. This can be
/// useful if for instance, you have build servers on linux but developers are
/// using macs.
pub struct ShuffleFS {
    filter: FilterState,
    /// `seed`.
    seed: i64,
}

impl ShuffleFS {
    /// `ShuffleFS(delegate, seed)`, with the scheme `shuffle://`.
    pub fn new(delegate: Arc<dyn FileSystem>, seed: i64) -> Arc<ShuffleFS> {
        let this = Arc::new(ShuffleFS {
            filter: FilterState::new("shuffle://", &delegate),
            seed,
        });
        install(&this, delegate);
        this
    }
}

/// The anonymous `DirectoryStream` `ShuffleFS` returns: the shuffled listing,
/// iterable any number of times, and a `close()` that does nothing.
struct ShuffledDirectoryStream {
    contents: Vec<Path>,
}

impl DirectoryStream for ShuffledDirectoryStream {
    fn iterator(&mut self) -> Box<dyn Iterator<Item = io::Result<Path>> + Send + '_> {
        Box::new(self.contents.clone().into_iter().map(Ok))
    }

    fn close(&mut self) -> io::Result<()> {
        Ok(())
    }

    fn stream_to_string(&self) -> String {
        identity_string("org.apache.lucene.tests.mockfile.ShuffleFS$1", self)
    }
}

impl FilterFileSystemProvider for ShuffleFS {
    fn filter(&self) -> &FilterState {
        &self.filter
    }

    fn class_name(&self) -> &'static str {
        "org.apache.lucene.tests.mockfile.ShuffleFS"
    }

    /// `newDirectoryStream(dir, filter)`: reads the complete listing, sorts it
    /// by file name ("sort first based only on filename"), then shuffles it
    /// with `new Random(seed)` ("sort based on current class seed").
    fn new_directory_stream(
        &self,
        dir: &Path,
        filter: DirectoryStreamFilter,
    ) -> io::Result<Box<dyn DirectoryStream>> {
        let mut stream = ffsp::new_directory_stream(self, dir, filter)?;
        let mut contents: Vec<Path> = Vec::new();
        let mut failure = None;
        for path in stream.iterator() {
            match path {
                Ok(path) => contents.push(path),
                Err(error) => {
                    failure = Some(error);
                    break;
                }
            }
        }
        let closed = stream.close();
        if let Some(error) = failure {
            return Err(error);
        }
        closed?;
        // `path1.getFileName().toString().compareTo(...)`: UTF-16 order.
        contents.sort_by(|a, b| {
            let a = a.get_file_name().map(|n| n.to_string()).unwrap_or_default();
            let b = b.get_file_name().map(|n| n.to_string()).unwrap_or_default();
            a.encode_utf16().cmp(b.encode_utf16())
        });
        shuffle(&mut contents, &JdkRandom::new(self.seed));
        Ok(Box::new(ShuffledDirectoryStream { contents }))
    }
}

crate::impl_file_system_provider!(ShuffleFS);
