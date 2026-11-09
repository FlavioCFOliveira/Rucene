//! Port of `lucene/core/src/test/org/apache/lucene/util/TestFrequencyTrackingRingBuffer.java`.

use std::collections::HashMap;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use rucene::error::Result;
use rucene::util::{Accountable, FrequencyTrackingRingBuffer};
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_equals, class, ram_used_unmodelled, random, test, unchecked};

/// `TestFrequencyTrackingRingBuffer`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.util.TestFrequencyTrackingRingBuffer",
        vec![
            test("test", test_),
            test("testRamBytesUsed", test_ram_bytes_used),
        ],
    )
}

/// `assertBuffer(buffer, maxSize, sentinel, items)`.
fn assert_buffer(
    buffer: &FrequencyTrackingRingBuffer,
    max_size: i32,
    sentinel: i32,
    items: &[i32],
) {
    let recent_items: Vec<i32> = if items.len() <= max_size as usize {
        let mut recent_items = Vec::new();
        for _ in items.len()..max_size as usize {
            recent_items.push(sentinel);
        }
        recent_items.extend_from_slice(items);
        recent_items
    } else {
        items[items.len() - max_size as usize..].to_vec()
    };
    let mut expected_frequencies: HashMap<i32, i32> = HashMap::new();
    for item in recent_items {
        match expected_frequencies.get(&item).copied() {
            None => {
                expected_frequencies.insert(item, 1);
            }
            Some(freq) => {
                expected_frequencies.insert(item, freq + 1);
            }
        }
    }
    assert_equals(&expected_frequencies, &buffer.as_frequency_map());
}

fn test_() -> Result<()> {
    let iterations = LuceneTestCase::at_least(100);
    for _ in 0..iterations {
        let max_size = 2 + random().next_int_bound(100);
        let numitems = random().next_int_bound(5000);
        let maxitem = 1 + random().next_int_bound(100);
        let mut items: Vec<i32> = Vec::new();
        let sentinel = random().next_int_bound(200);
        let mut buffer = unchecked(FrequencyTrackingRingBuffer::new(
            max_size as usize,
            sentinel,
        ));
        for _ in 0..numitems {
            let item = random().next_int_bound(maxitem);
            items.push(item);
            buffer.add(item);
        }
        assert_buffer(&buffer, max_size, sentinel, &items);
    }
    Ok(())
}

fn test_ram_bytes_used() -> Result<()> {
    let max_size = 2 + random().next_int_bound(10000);
    let sentinel = random().next_int();
    let mut buffer = unchecked(FrequencyTrackingRingBuffer::new(
        max_size as usize,
        sentinel,
    ));
    for _ in 0..10000 {
        buffer.add(random().next_int());
    }
    assert_equals(
        &ram_used_unmodelled("FrequencyTrackingRingBuffer"),
        &buffer.ram_bytes_used(),
    );
    Ok(())
}
