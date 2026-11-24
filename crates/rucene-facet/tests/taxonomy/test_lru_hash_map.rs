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

//! Port of `org.apache.lucene.facet.taxonomy.TestLRUHashMap`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene_facet::taxonomy::lru_hash_map::LRUHashMap;

use crate::facet_support::core_support::test;
use crate::facet_support::facet_class;

/// `TestLRUHashMap`.
pub fn class_def() -> Arc<Class> {
    facet_class(
        "org.apache.lucene.facet.taxonomy.TestLRUHashMap",
        vec![test("testLRU", test_lru)],
    )
}

// testLRU() tests that the specified size limit is indeed honored, and
// the remaining objects in the map are indeed those that have been most
// recently used
fn test_lru() -> Result<()> {
    let mut lru: LRUHashMap<String, String> = LRUHashMap::new(3);
    let s = |x: &str| x.to_string();
    Assert::assert_equals(0usize, lru.size());
    lru.put(s("one"), s("Hello world"));
    Assert::assert_equals(1usize, lru.size());
    lru.put(s("two"), s("Hi man"));
    Assert::assert_equals(2usize, lru.size());
    lru.put(s("three"), s("Bonjour"));
    Assert::assert_equals(3usize, lru.size());
    lru.put(s("four"), s("Shalom"));
    Assert::assert_equals(3usize, lru.size());
    Assert::assert_true(lru.get(&s("three")).is_some());
    Assert::assert_true(lru.get(&s("two")).is_some());
    Assert::assert_true(lru.get(&s("four")).is_some());
    Assert::assert_true(lru.get(&s("one")).is_none());
    lru.put(s("five"), s("Yo!"));
    Assert::assert_equals(3usize, lru.size());
    Assert::assert_true(lru.get(&s("three")).is_none()); // three was last used, so it got removed
    Assert::assert_true(lru.get(&s("five")).is_some());
    lru.get(&s("four"));
    lru.put(s("six"), s("hi"));
    lru.put(s("seven"), s("hey dude"));
    Assert::assert_equals(3usize, lru.size());
    Assert::assert_true(lru.get(&s("one")).is_none());
    Assert::assert_true(lru.get(&s("two")).is_none());
    Assert::assert_true(lru.get(&s("three")).is_none());
    Assert::assert_true(lru.get(&s("four")).is_some());
    Assert::assert_true(lru.get(&s("five")).is_none());
    Assert::assert_true(lru.get(&s("six")).is_some());
    Assert::assert_true(lru.get(&s("seven")).is_some());
    Ok(())
}
