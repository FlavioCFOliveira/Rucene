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

//! Port of `org.apache.lucene.facet.taxonomy.writercache.TestCharBlockArray`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene_facet::taxonomy::writercache::char_block_array::CharBlockArray;
use rucene_test_framework::util::lucene_test_case::TEST_NIGHTLY;
use rucene_test_framework::util::LuceneTestCase;

use crate::facet_support::core_support::test;
use crate::facet_support::facet_class;

/// `TestCharBlockArray`.
pub fn class_def() -> Arc<Class> {
    facet_class(
        "org.apache.lucene.facet.taxonomy.writercache.TestCharBlockArray",
        vec![test("testArray", test_array)],
    )
}

/// `random().nextBytes(buffer)`, `1 + random().nextInt(50)`, then the
/// UTF-8 decoding with REPLACE of the first `size` bytes (both sides of the
/// comparison receive the same string; the draws are Java's).
fn random_string(buffer: &mut [u8; 50]) -> Vec<u16> {
    let random = LuceneTestCase::random();
    random.next_bytes(buffer);
    let size = 1 + random.next_int_bound(50) as usize;
    String::from_utf8_lossy(&buffer[..size])
        .encode_utf16()
        .collect()
}

fn test_array() -> Result<()> {
    let mut array = CharBlockArray::new()?;
    let mut builder: Vec<u16> = Vec::new();
    let n = if *TEST_NIGHTLY { 100 * 1000 } else { 1000 };
    let mut buffer = [0u8; 50];
    for _ in 0..n {
        let s = random_string(&mut buffer);
        // array.append(String)
        array.append_str(&String::from_utf16_lossy(&s))?;
        builder.extend_from_slice(&s);
    }
    for _ in 0..n {
        let s = random_string(&mut buffer);
        // array.append((CharSequence) s)
        array.append_char_sequence(&s)?;
        builder.extend_from_slice(&s);
    }
    for _ in 0..n {
        let s = random_string(&mut buffer);
        for &c in &s {
            array.append_char(c)?;
        }
        builder.extend_from_slice(&s);
    }
    assert_equals_internal(
        "GrowingCharArray<->StringBuilder mismatch.",
        &builder,
        &array,
    );
    Ok(())
}

fn assert_equals_internal(msg: &str, expected: &[u16], actual: &CharBlockArray) {
    Assert::assert_equals_msg(Some(msg), expected.len(), actual.length());
    for (i, &c) in expected.iter().enumerate() {
        Assert::assert_equals_msg(Some(msg), c, actual.char_at(i));
    }
}
