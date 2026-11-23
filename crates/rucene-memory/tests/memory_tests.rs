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

//! Port of the test classes of Lucene 10.5.0 `lucene/memory/src/test`
//! (rmp #629), each run as a suite by the ported runner:
//! `org.apache.lucene.index.memory.TestSlicedIntBlockPool`,
//! `TestMemoryIndex` and `TestMemoryIndexAgainstDirectory`.

// The ports keep Java's indexed loops and counters line by line.
#![allow(clippy::needless_range_loop)]

mod memory_support;

mod index {
    pub mod memory {
        pub mod test_memory_index;
        pub mod test_memory_index_against_directory;
        pub mod test_sliced_int_block_pool;
    }
}

use memory_support::core_support::run;

#[test]
fn test_sliced_int_block_pool() {
    run(index::memory::test_sliced_int_block_pool::class_def);
}

#[test]
fn test_memory_index() {
    run(index::memory::test_memory_index::class_def);
}

#[test]
fn test_memory_index_against_directory() {
    run(index::memory::test_memory_index_against_directory::class_def);
}
