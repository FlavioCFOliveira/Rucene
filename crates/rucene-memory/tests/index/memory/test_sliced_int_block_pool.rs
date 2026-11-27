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

//! Port of `TestSlicedIntBlockPool.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::util::concurrent::Counter;
use rucene::util::int_block_pool::{IntBlockAllocator, INT_BLOCK_SIZE};
use rucene_memory::memory_index::{SliceReader, SliceWriter, SlicedIntBlockPool};
use rucene_test_framework::util::LuceneTestCase;

use crate::memory_support::core_support::{class, random, test};

/// `TestSlicedIntBlockPool`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.memory.TestSlicedIntBlockPool",
        vec![
            test("testSingleWriterReader", test_single_writer_reader),
            test("testMultipleWriterReader", test_multiple_writer_reader),
        ],
    )
}

const INT_BYTES: i64 = 4;

fn test_single_writer_reader() -> Result<()> {
    let bytes_used = Arc::new(Counter::new_counter());
    let mut sliced_int_block_pool = SlicedIntBlockPool::new(Box::new(ByteTrackingAllocator::new(
        Arc::clone(&bytes_used),
    )));

    for _j in 0..2 {
        let mut writer = SliceWriter::default();
        let start = writer.start_new_slice(&mut sliced_int_block_pool)?;
        let num = LuceneTestCase::at_least(100);
        for i in 0..num {
            writer.write_int(&mut sliced_int_block_pool, i)?;
        }

        let upto = writer.get_current_offset();
        let mut reader = SliceReader::default();
        reader.reset(start, upto);
        for i in 0..num {
            Assert::assert_equals(i, reader.read_int(&sliced_int_block_pool));
        }
        Assert::assert_true(reader.end_of_slice());
        if random().next_boolean() {
            sliced_int_block_pool.reset(true, false);
            Assert::assert_equals(0i64, bytes_used.get());
        } else {
            sliced_int_block_pool.reset(true, true);
            Assert::assert_equals(INT_BLOCK_SIZE as i64 * INT_BYTES, bytes_used.get());
        }
    }
    Ok(())
}

fn test_multiple_writer_reader() -> Result<()> {
    let bytes_used = Arc::new(Counter::new_counter());
    let mut sliced_int_block_pool = SlicedIntBlockPool::new(Box::new(ByteTrackingAllocator::new(
        Arc::clone(&bytes_used),
    )));
    for _j in 0..2 {
        let mut holders: Vec<StartEndAndValues> = Vec::new();
        let num = LuceneTestCase::at_least(4);
        for _i in 0..num {
            holders.push(StartEndAndValues::new(random().next_int_bound(1000)));
        }
        let mut writer = SliceWriter::default();
        let mut reader = SliceReader::default();

        let num_values_to_write = LuceneTestCase::at_least(10000);
        for _i in 0..num_values_to_write {
            let index = random().next_int_bound(holders.len() as i32) as usize;
            let values = &mut holders[index];
            if values.value_count == 0 {
                values.start = writer.start_new_slice(&mut sliced_int_block_pool)?;
            } else {
                writer.reset(values.end);
            }
            let value = values.next_value();
            writer.write_int(&mut sliced_int_block_pool, value)?;
            values.end = writer.get_current_offset();
            if random().next_int_bound(5) == 0 {
                // pick one and reader the ints
                let pick = random().next_int_bound(holders.len() as i32) as usize;
                assert_reader(&mut reader, &sliced_int_block_pool, &holders[pick]);
            }
        }

        while !holders.is_empty() {
            let index = random().next_int_bound(holders.len() as i32) as usize;
            let values = holders.remove(index);
            assert_reader(&mut reader, &sliced_int_block_pool, &values);
        }
        if random().next_boolean() {
            sliced_int_block_pool.reset(true, false);
            Assert::assert_equals(0i64, bytes_used.get());
        } else {
            sliced_int_block_pool.reset(true, true);
            Assert::assert_equals(INT_BLOCK_SIZE as i64 * INT_BYTES, bytes_used.get());
        }
    }
    Ok(())
}

/// `ByteTrackingAllocator extends IntBlockPool.Allocator`.
struct ByteTrackingAllocator {
    block_size: usize,
    bytes_used: Arc<Counter>,
}

impl ByteTrackingAllocator {
    fn new(bytes_used: Arc<Counter>) -> Self {
        Self::with_block_size(INT_BLOCK_SIZE, bytes_used)
    }

    fn with_block_size(block_size: usize, bytes_used: Arc<Counter>) -> Self {
        Self {
            block_size,
            bytes_used,
        }
    }
}

impl IntBlockAllocator for ByteTrackingAllocator {
    fn block_size(&self) -> usize {
        self.block_size
    }

    fn get_int_block(&mut self) -> Vec<i32> {
        self.bytes_used
            .add_and_get(self.block_size as i64 * INT_BYTES);
        vec![0; self.block_size]
    }

    fn recycle_int_blocks(&mut self, _blocks: &mut [Option<Vec<i32>>], start: usize, end: usize) {
        self.bytes_used
            .add_and_get(-(((end - start) * self.block_size) as i64 * INT_BYTES));
    }
}

fn assert_reader(reader: &mut SliceReader, pool: &SlicedIntBlockPool, values: &StartEndAndValues) {
    reader.reset(values.start, values.end);
    for i in 0..values.value_count {
        Assert::assert_equals(values.value_offset + i, reader.read_int(pool));
    }
    Assert::assert_true(reader.end_of_slice());
}

struct StartEndAndValues {
    value_offset: i32,
    value_count: i32,
    start: i32,
    end: i32,
}

impl StartEndAndValues {
    fn new(value_offset: i32) -> Self {
        Self {
            value_offset,
            value_count: 0,
            start: 0,
            end: 0,
        }
    }

    fn next_value(&mut self) -> i32 {
        let v = self.value_offset + self.value_count;
        self.value_count += 1;
        v
    }
}
