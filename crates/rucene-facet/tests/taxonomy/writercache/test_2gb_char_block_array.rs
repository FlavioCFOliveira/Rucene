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

//! Port of `org.apache.lucene.facet.taxonomy.writercache.Test2GBCharBlockArray`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene_facet::taxonomy::writercache::char_block_array::CharBlockArray;
use rucene_test_framework::util::lucene_test_case::Monster;
use rucene_test_framework::util::test_util::TestUtil;
use rucene_test_framework::util::LuceneTestCase;

use crate::facet_support::core_support::{class_annotated, test};

/// `@Monster("uses lots of space and takes a few minutes") public class
/// Test2GBCharBlockArray extends LuceneTestCase`.
pub fn class_def() -> Arc<Class> {
    class_annotated(
        "org.apache.lucene.facet.taxonomy.writercache.Test2GBCharBlockArray",
        vec![test("test2GBChars", test_2gb_chars)],
        |b| {
            b.annotate(Monster {
                value: "uses lots of space and takes a few minutes".to_string(),
            })
        },
    )
}

fn test_2gb_chars() -> Result<()> {
    let block_size = 32768;
    let mut array = CharBlockArray::with_block_size(block_size)?;
    let size = TestUtil::next_int(&*LuceneTestCase::random(), 20000, 40000) as usize;
    let chars = vec![0u16; size];
    let mut count: i64 = 0;
    loop {
        count += 1;
        match array.append_chars(&chars, 0, size) {
            Ok(_) => {}
            Err(e) if e.java_simple_name() == "IllegalStateException" => {
                Assert::assert_true(count * size as i64 + block_size as i64 > i64::from(i32::MAX));
                break;
            }
            Err(e) => return Err(e),
        }
        Assert::assert_false_msg(
            &format!(
                "appended {} characters beyond Integer.MAX_VALUE!",
                count * size as i64 - i64::from(i32::MAX)
            ),
            count * size as i64 > i64::from(i32::MAX),
        );
    }
    Ok(())
}
