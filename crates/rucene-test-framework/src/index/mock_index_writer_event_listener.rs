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

//! Port of `org.apache.lucene.tests.index.MockIndexWriterEventListener`: an
//! `IndexWriterEventListener` that records whether a merge on full flush began
//! and ended.

use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};

use rucene::index::index_writer_config::IndexWriterEventListener;
use rucene::index::merge_policy::MergeSpecification;

/// Port of `org.apache.lucene.tests.index.MockIndexWriterEventListener`
/// (`implements IndexWriterEventListener`).
#[derive(Debug, Default)]
pub struct MockIndexWriterEventListener {
    begin_merge_called: AtomicBool,
    end_merge_called: AtomicBool,
}

impl MockIndexWriterEventListener {
    /// `new MockIndexWriterEventListener()`.
    pub fn new() -> Self {
        Self::default()
    }

    /// `isEventsRecorded()`: whether both events were seen.
    pub fn is_events_recorded(&self) -> bool {
        self.begin_merge_called.load(Ordering::Relaxed)
            && self.end_merge_called.load(Ordering::Relaxed)
    }
}

impl IndexWriterEventListener for MockIndexWriterEventListener {
    /// `beginMergeOnFullFlush(merge)`.
    fn begin_merge_on_full_flush(&self, _merge: &MergeSpecification) {
        self.begin_merge_called.store(true, Ordering::Relaxed);
    }

    /// `endMergeOnFullFlush(merge)`.
    fn end_merge_on_full_flush(&self, _merge: &MergeSpecification) {
        self.end_merge_called.store(true, Ordering::Relaxed);
    }
}

impl fmt::Display for MockIndexWriterEventListener {
    /// `Object.toString()`: the class name and, for the identity hash, the
    /// object's address, as `java_object_to_string` renders every object
    /// without a `toString()` of its own.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&rucene::index::index_reader::java_object_to_string(
            "org.apache.lucene.tests.index.MockIndexWriterEventListener",
            self,
        ))
    }
}
