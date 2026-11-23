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

//! Port of `org.apache.lucene.facet.taxonomy.writercache.TestLruTaxonomyWriterCache`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene_facet::taxonomy::facet_label::FacetLabel;
use rucene_facet::taxonomy::writercache::lru_taxonomy_writer_cache::LruTaxonomyWriterCache;
use rucene_facet::taxonomy::writercache::taxonomy_writer_cache::TaxonomyWriterCache;

use crate::facet_support::core_support::test;
use crate::facet_support::facet_class;

/// `TestLruTaxonomyWriterCache`.
pub fn class_def() -> Arc<Class> {
    facet_class(
        "org.apache.lucene.facet.taxonomy.writercache.TestLruTaxonomyWriterCache",
        vec![test(
            "testDefaultLRUTypeIsCollisionSafe",
            test_default_lru_type_is_collision_safe,
        )],
    )
}

fn test_default_lru_type_is_collision_safe() -> Result<()> {
    // These labels are clearly different, but have identical longHashCodes.
    // Note that these labels are clearly contrived. We did encounter
    // collisions in actual production data, but we aren't allowed to publish
    // those.
    let a = FacetLabel::new(["\0", "\u{0003}\u{FFE2}"])?;
    let b = FacetLabel::new(["\u{1}", "\0"])?;
    // If this fails, then the longHashCode implementation has changed. This
    // cannot prevent collisions. (All hashes must allow for collisions.) It
    // will however stop the rest of this test from making sense. To fix, find
    // new colliding labels, or make a subclass of FacetLabel that produces
    // collisions.
    Assert::assert_equals(a.long_hash_code(), b.long_hash_code());
    // Make a cache with capacity > 2 so both our labels will fit. Don't
    // specify an LRUType, since we want to check if the default is
    // collision-safe.
    let cache = LruTaxonomyWriterCache::new(10);
    cache.put(&a, 0)?;
    cache.put(&b, 1)?;
    Assert::assert_equals(cache.get(&a)?, 0);
    Assert::assert_equals(cache.get(&b)?, 1);
    Ok(())
}
