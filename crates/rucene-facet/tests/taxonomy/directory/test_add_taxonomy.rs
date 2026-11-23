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

//! Port of `org.apache.lucene.facet.taxonomy.directory.TestAddTaxonomy`.

use std::collections::HashSet;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;

use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::jdk::thread::Thread;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::index::TwoPhaseCommit;
use rucene_facet::taxonomy::directory::directory_taxonomy_reader::DirectoryTaxonomyReader;
use rucene_facet::taxonomy::directory::directory_taxonomy_writer::{
    DirectoryTaxonomyWriter, DiskOrdinalMap, MemoryOrdinalMap, OrdinalMap,
};
use rucene_facet::taxonomy::facet_label::FacetLabel;
use rucene_facet::taxonomy::taxonomy_reader::TaxonomyReader;
use rucene_facet::taxonomy::taxonomy_writer::TaxonomyWriter;
use rucene_test_framework::util::java_exceptions::java_throwable;
use rucene_test_framework::util::lucene_test_case::{SuppressCodecs, TestDirectory};
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::facet_support::core_support::test;
use crate::facet_support::facet_class_annotated;

/// `TestAddTaxonomy`.
pub fn class_def() -> Arc<Class> {
    facet_class_annotated(
        "org.apache.lucene.facet.taxonomy.directory.TestAddTaxonomy",
        vec![
            test("testAddEmpty", test_add_empty),
            test("testAddToEmpty", test_add_to_empty),
            test("testBig", test_big),
            test("testMedium", test_medium),
            test("testSimple", test_simple),
            test("testConcurrency", test_concurrency),
        ],
        |b| {
            b.annotate(SuppressCodecs {
                value: vec!["SimpleText".to_string()],
            })
        },
    )
}

fn label(components: &[&str]) -> Result<FacetLabel> {
    FacetLabel::new(components.iter().copied())
}

/// `catch (IOException e) { throw new RuntimeException(e); }` in a thread.
fn rethrow_in_thread(e: rucene::error::LuceneError) -> ! {
    if e.is_io_exception() {
        Throwable::from_cause("java.lang.RuntimeException", java_throwable(&e)).throw()
    }
    java_throwable(&e).throw()
}

fn join_all(threads: &[Thread]) {
    for t in threads {
        if let Err(payload) = t.join() {
            std::panic::resume_unwind(payload);
        }
    }
}

fn dotest(ncats: i32, range: i32) -> Result<()> {
    let num_cats = Arc::new(AtomicI32::new(ncats));
    let mut dirs: Vec<TestDirectory> = Vec::new();
    for _ in 0..2 {
        let dir = LuceneTestCase::new_directory();
        let tw = Arc::new(DirectoryTaxonomyWriter::new(dir.as_directory())?);
        let mut add_threads = Vec::new();
        for _ in 0..4 {
            let (num_cats, tw) = (Arc::clone(&num_cats), Arc::clone(&tw));
            add_threads.push(Thread::unnamed(move || {
                let random = LuceneTestCase::random();
                while num_cats.fetch_sub(1, Ordering::SeqCst) - 1 > 0 {
                    let cat = random.next_int_bound(range).to_string();
                    if let Err(e) = label(&["a", &cat]).and_then(|l| tw.add_category(&l)) {
                        rethrow_in_thread(e)
                    }
                }
            }));
        }
        for t in &add_threads {
            t.start();
        }
        join_all(&add_threads);
        tw.close()?;
        dirs.push(dir);
    }

    let tw = DirectoryTaxonomyWriter::new(dirs[0].as_directory())?;
    let mut map = random_ordinal_map()?;
    tw.add_taxonomy(dirs[1].as_directory(), &mut *map)?;
    tw.close()?;

    validate(&dirs[0], &dirs[1], &mut *map)?;

    for d in dirs {
        d.close()?;
    }
    Ok(())
}

fn random_ordinal_map() -> Result<Box<dyn OrdinalMap>> {
    if LuceneTestCase::random().next_boolean() {
        let path = LuceneTestCase::create_temp_file_with("taxoMap", "")?;
        Ok(Box::new(DiskOrdinalMap::new(path.as_std_path())?))
    } else {
        Ok(Box::new(MemoryOrdinalMap::new()))
    }
}

fn validate(dest: &TestDirectory, src: &TestDirectory, ord_map: &mut dyn OrdinalMap) -> Result<()> {
    let dest_tr = DirectoryTaxonomyReader::open(dest.as_directory())?;
    let result = (|| -> Result<()> {
        let dest_size = dest_tr.get_size()?;
        let src_tr = DirectoryTaxonomyReader::open(src.as_directory())?;
        let inner = (|| -> Result<()> {
            let map = ord_map.get_map()?;

            // validate taxo sizes
            let src_size = src_tr.get_size()?;
            Assert::assert_true_msg(
                &format!(
                    "destination taxonomy expected to be larger than source; dest={dest_size} src={src_size}"
                ),
                dest_size >= src_size,
            );

            // validate that all source categories exist in destination, and their
            // ordinals are as expected.
            for j in 1..src_size {
                let cp = src_tr.get_path(j)?.unwrap();
                let dest_ordinal = dest_tr.get_ordinal(&cp)?;
                Assert::assert_true_msg(
                    &format!("{cp} not found in destination"),
                    dest_ordinal > 0,
                );
                Assert::assert_equals(dest_ordinal, map[j as usize]);
            }
            Ok(())
        })();
        let closed = src_tr.close();
        inner.and(closed)
    })();
    let closed = dest_tr.close();
    result.and(closed)
}

fn test_add_empty() -> Result<()> {
    let dest = LuceneTestCase::new_directory();
    let dest_tw = DirectoryTaxonomyWriter::new(dest.as_directory())?;
    dest_tw.add_category(&label(&["Author", "Rob Pike"])?)?;
    dest_tw.add_category(&label(&["Aardvarks", "Bob"])?)?;
    dest_tw.commit()?;

    let src = LuceneTestCase::new_directory();
    DirectoryTaxonomyWriter::new(src.as_directory())?.close()?; // create an empty taxonomy

    let mut map = random_ordinal_map()?;
    dest_tw.add_taxonomy(src.as_directory(), &mut *map)?;
    dest_tw.close()?;

    validate(&dest, &src, &mut *map)?;

    dest.close()?;
    src.close()
}

fn test_add_to_empty() -> Result<()> {
    let dest = LuceneTestCase::new_directory();

    let src = LuceneTestCase::new_directory();
    let src_tw = DirectoryTaxonomyWriter::new(src.as_directory())?;
    src_tw.add_category(&label(&["Author", "Rob Pike"])?)?;
    src_tw.add_category(&label(&["Aardvarks", "Bob"])?)?;
    src_tw.close()?;

    let dest_tw = DirectoryTaxonomyWriter::new(dest.as_directory())?;
    let mut map = random_ordinal_map()?;
    dest_tw.add_taxonomy(src.as_directory(), &mut *map)?;
    dest_tw.close()?;

    validate(&dest, &src, &mut *map)?;

    dest.close()?;
    src.close()
}

// A more comprehensive and big random test.
fn test_big() -> Result<()> {
    dotest(200, 10000)?;
    dotest(1000, 20000)?;
    dotest(400000, 1000000)
}

// a reasonable random test
fn test_medium() -> Result<()> {
    let random = LuceneTestCase::random();
    let num_tests = LuceneTestCase::at_least(3);
    for _ in 0..num_tests {
        let ncats = TestUtil::next_int(&*random, 2, 100);
        let range = TestUtil::next_int(&*random, 100, 1000);
        dotest(ncats, range)?;
    }
    Ok(())
}

fn test_simple() -> Result<()> {
    let dest = LuceneTestCase::new_directory();
    let tw1 = DirectoryTaxonomyWriter::new(dest.as_directory())?;
    tw1.add_category(&label(&["Author", "Mark Twain"])?)?;
    tw1.add_category(&label(&["Animals", "Dog"])?)?;
    tw1.add_category(&label(&["Author", "Rob Pike"])?)?;

    let src = LuceneTestCase::new_directory();
    let tw2 = DirectoryTaxonomyWriter::new(src.as_directory())?;
    tw2.add_category(&label(&["Author", "Rob Pike"])?)?;
    tw2.add_category(&label(&["Aardvarks", "Bob"])?)?;
    tw2.close()?;

    let mut map = random_ordinal_map()?;

    tw1.add_taxonomy(src.as_directory(), &mut *map)?;
    tw1.close()?;

    validate(&dest, &src, &mut *map)?;

    dest.close()?;
    src.close()
}

fn test_concurrency() -> Result<()> {
    // tests that addTaxonomy and addCategory work in parallel
    let num_categories = LuceneTestCase::at_least(10000);

    // build an input taxonomy index
    let src = LuceneTestCase::new_directory();
    let tw = DirectoryTaxonomyWriter::new(src.as_directory())?;
    for i in 0..num_categories {
        tw.add_category(&label(&["a", &i.to_string()])?)?;
    }
    tw.close()?;

    // now add the taxonomy to an empty taxonomy, while adding the categories
    // again, in parallel -- in the end, no duplicate categories should exist.
    let dest = LuceneTestCase::new_directory();
    let dest_tw = Arc::new(DirectoryTaxonomyWriter::new(dest.as_directory())?);
    let tw_thread = Arc::clone(&dest_tw);
    let t = Thread::unnamed(move || {
        for i in 0..num_categories {
            // shouldn't happen - if it does, let the test fail on uncaught exception.
            if let Err(e) = label(&["a", &i.to_string()]).and_then(|l| tw_thread.add_category(&l)) {
                rethrow_in_thread(e)
            }
        }
    });
    t.start();

    let mut map = MemoryOrdinalMap::new();
    dest_tw.add_taxonomy(src.as_directory(), &mut map)?;
    join_all(std::slice::from_ref(&t));
    dest_tw.close()?;

    // now validate

    let dtr = DirectoryTaxonomyReader::open(dest.as_directory())?;
    // +2 to account for the root category + "a"
    Assert::assert_equals(num_categories + 2, dtr.get_size()?);
    let mut categories = HashSet::new();
    for i in 1..dtr.get_size()? {
        let cat = dtr.get_path(i)?.unwrap();
        Assert::assert_true_msg(
            &format!("category {cat} already existed"),
            categories.insert(cat),
        );
    }
    dtr.close()?;

    src.close()?;
    dest.close()
}
