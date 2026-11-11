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

//! Port of `org.apache.lucene.tests.util.QuickPatchThreadsFilter`.

use randomizedtesting::jdk::thread::Thread;
use randomizedtesting::ThreadFilter;

/// Port of `QuickPatchThreadsFilter`: "this is a hack to temporarily get
/// rid of certain threads which are created by the JVM and the leak
/// detector cannot handle."
///
/// **Divergence.** Java rejects the IBM J9 `ClassCache Reaper` and
/// `java.util.Timer$TimerImpl` threads (`isJ9`, never true for a Rust
/// process) and the daemon `ForkJoinPool.commonPool-worker` threads (a
/// `ForkJoinWorkerThread`, which a Rust process does not start); neither
/// kind of thread exists here, so only the `JNA Cleaner` check remains.
#[derive(Debug, Default, Clone, Copy)]
pub struct QuickPatchThreadsFilter;

impl QuickPatchThreadsFilter {
    /// `QuickPatchThreadsFilter.class.getName()`.
    pub const CLASS_NAME: &'static str = "org.apache.lucene.tests.util.QuickPatchThreadsFilter";
}

impl ThreadFilter for QuickPatchThreadsFilter {
    /// `reject(t)`: "also filter out JNA Cleaner threads, which is static
    /// per-JVM and not under the control of a test suite."
    fn reject(&self, t: &Thread) -> bool {
        t.name() == "JNA Cleaner"
    }
}
