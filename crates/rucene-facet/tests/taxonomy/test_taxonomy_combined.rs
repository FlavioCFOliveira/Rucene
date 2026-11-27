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

//! Port of `org.apache.lucene.facet.taxonomy.TestTaxonomyCombined`.

use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::jdk::thread::Thread;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::index::TwoPhaseCommit;
use rucene::store::Directory;
use rucene_facet::taxonomy::directory::directory_taxonomy_reader::DirectoryTaxonomyReader;
use rucene_facet::taxonomy::directory::directory_taxonomy_writer::DirectoryTaxonomyWriter;
use rucene_facet::taxonomy::facet_label::FacetLabel;
use rucene_facet::taxonomy::parallel_taxonomy_arrays::IntArray;
use rucene_facet::taxonomy::taxonomy_reader::{
    self, TaxonomyReader, INVALID_ORDINAL, ROOT_ORDINAL,
};
use rucene_facet::taxonomy::taxonomy_writer::TaxonomyWriter;
use rucene_test_framework::util::java_exceptions::{expect_throws, java_throwable};
use rucene_test_framework::util::LuceneTestCase;

use crate::facet_support::core_support::test;
use crate::facet_support::facet_class;
use crate::root::slow_directory::SlowDirectory;

/// `TestTaxonomyCombined`.
pub fn class_def() -> Arc<Class> {
    facet_class(
        "org.apache.lucene.facet.taxonomy.TestTaxonomyCombined",
        vec![
            test("testWriter", test_writer),
            test("testWriterTwice", test_writer_twice),
            test("testWriterTwice2", test_writer_twice2),
            test("testWriterTwice3", test_writer_twice3),
            test("testWriterSimpler", test_writer_simpler),
            test("testRootOnly", test_root_only),
            test("testRootOnly2", test_root_only2),
            test("testReaderBasic", test_reader_basic),
            test("testReaderParent", test_reader_parent),
            test("testWriterParent1", test_writer_parent1),
            test("testWriterParent2", test_writer_parent2),
            test("testChildrenArrays", test_children_arrays),
            test(
                "testChildrenArraysInvariants",
                test_children_arrays_invariants,
            ),
            test("testChildrenArraysGrowth", test_children_arrays_growth),
            test(
                "testTaxonomyReaderRefreshRaces",
                test_taxonomy_reader_refresh_races,
            ),
            test(
                "testSeparateReaderAndWriter",
                test_separate_reader_and_writer,
            ),
            test(
                "testSeparateReaderAndWriter2",
                test_separate_reader_and_writer2,
            ),
            test("testWriterCheckPaths", test_writer_check_paths),
            test("testWriterCheckPaths2", test_writer_check_paths2),
            test("testNRT", test_nrt),
            test("testThousandsOfCategories", test_thousands_of_categories),
        ],
    )
}

/// The following categories will be added to the taxonomy by fillTaxonomy(), and tested by all
/// tests below:
const CATEGORIES: &[&[&str]] = &[
    &["Author", "Tom Clancy"],
    &["Author", "Richard Dawkins"],
    &["Author", "Richard Adams"],
    &["Price", "10", "11"],
    &["Price", "10", "12"],
    &["Price", "20", "27"],
    &["Date", "2006", "05"],
    &["Date", "2005"],
    &["Date", "2006"],
    &["Subject", "Nonfiction", "Children", "Animals"],
    &["Author", "Stephen Jay Gould"],
    &["Author", "\u{05e0}\u{05d3}\u{05d1}\u{3042}\u{0628}"],
];

/// When adding the above categories with TaxonomyWriter.addCategory(), the following paths are
/// expected to be returned: (note that currently the full path is not returned, and therefore not
/// tested - rather, just the last component, the ordinal, is returned and tested.
const EXPECTED_PATHS: &[&[i32]] = &[
    &[1, 2],
    &[1, 3],
    &[1, 4],
    &[5, 6, 7],
    &[5, 6, 8],
    &[5, 9, 10],
    &[11, 12, 13],
    &[11, 14],
    &[11, 12],
    &[15, 16, 17, 18],
    &[1, 19],
    &[1, 20],
];

/// The taxonomy index is expected to then contain the following generated categories, with
/// increasing ordinals (note how parent categories are be added automatically when subcategories
/// are added).
const EXPECTED_CATEGORIES: &[&[&str]] = &[
    &[], // the root category
    &["Author"],
    &["Author", "Tom Clancy"],
    &["Author", "Richard Dawkins"],
    &["Author", "Richard Adams"],
    &["Price"],
    &["Price", "10"],
    &["Price", "10", "11"],
    &["Price", "10", "12"],
    &["Price", "20"],
    &["Price", "20", "27"],
    &["Date"],
    &["Date", "2006"],
    &["Date", "2006", "05"],
    &["Date", "2005"],
    &["Subject"],
    &["Subject", "Nonfiction"],
    &["Subject", "Nonfiction", "Children"],
    &["Subject", "Nonfiction", "Children", "Animals"],
    &["Author", "Stephen Jay Gould"],
    &["Author", "\u{05e0}\u{05d3}\u{05d1}\u{3042}\u{0628}"],
];

fn label(components: &[&str]) -> Result<FacetLabel> {
    FacetLabel::new(components.iter().copied())
}

/// fillTaxonomy adds the categories in the categories[] array, and asserts that the additions
/// return exactly the ordinals (in the past - paths) specified in expectedPaths[]. Note that this
/// assumes that fillTaxonomy() is called on an empty taxonomy index. Calling it after something
/// else was already added to the taxonomy index will surely have this method fail.
fn fill_taxonomy(tw: &dyn TaxonomyWriter) -> Result<()> {
    for (i, category) in CATEGORIES.iter().enumerate() {
        let ordinal = tw.add_category(&label(category)?)?;
        let expected_ordinal = *EXPECTED_PATHS[i].last().unwrap();
        if ordinal != expected_ordinal {
            Assert::fail(&format!(
                "For category {} expected ordinal {expected_ordinal}, but got {ordinal}",
                showcat_path(Some(category))
            ));
        }
    }
    Ok(())
}

fn showcat_path(path: Option<&[&str]>) -> String {
    let Some(path) = path else {
        return "<null>".to_string();
    };
    if path.is_empty() {
        return "<empty>".to_string();
    }
    if path.len() == 1 && path[0].is_empty() {
        return "<\"\">".to_string();
    }
    path.join("/")
}

fn showcat(path: Option<&FacetLabel>) -> String {
    match path {
        None => "<null>".to_string(),
        Some(p) if p.length == 0 => "<empty>".to_string(),
        Some(p) => format!("<{p}>"),
    }
}

fn new_tw(dir: &Arc<dyn Directory>) -> Result<DirectoryTaxonomyWriter> {
    DirectoryTaxonomyWriter::new(Arc::clone(dir))
}

fn new_tr(dir: &Arc<dyn Directory>) -> Result<Arc<dyn TaxonomyReader>> {
    Ok(Arc::new(DirectoryTaxonomyReader::open(Arc::clone(dir))?))
}

/// Basic tests for TaxonomyWriter. Basically, we test that IndexWriter.addCategory works, i.e.
/// returns the expected ordinals (this is tested by calling the fillTaxonomy() method above). We
/// do not test here that after writing the index can be read - this will be done in more tests
/// below.
fn test_writer() -> Result<()> {
    let index_dir = LuceneTestCase::new_directory();
    let d = index_dir.as_directory();
    let tw = new_tw(&d)?;
    fill_taxonomy(&tw)?;
    // Also check TaxonomyWriter.getSize() - see that the taxonomy's size
    // is what we expect it to be.
    Assert::assert_equals(EXPECTED_CATEGORIES.len() as i32, tw.get_size()?);
    tw.close()?;
    index_dir.close()
}

/// testWriterTwice is exactly like testWriter, except that after adding all the categories, we add
/// them again, and see that we get the same old ids again - not new categories.
fn test_writer_twice() -> Result<()> {
    let index_dir = LuceneTestCase::new_directory();
    let d = index_dir.as_directory();
    let tw = new_tw(&d)?;
    fill_taxonomy(&tw)?;
    // run fillTaxonomy again - this will try to add the same categories
    // again, and check that we see the same ordinal paths again, not
    // different ones.
    fill_taxonomy(&tw)?;
    // Let's check the number of categories again, to see that no
    // extraneous categories were created:
    Assert::assert_equals(EXPECTED_CATEGORIES.len() as i32, tw.get_size()?);
    tw.close()?;
    index_dir.close()
}

/// testWriterTwice2 is similar to testWriterTwice, except that the index is closed and reopened
/// before attempting to write to it the same categories again. While testWriterTwice can get along
/// with writing and reading correctly just to the cache, testWriterTwice2 checks also the actual
/// disk read part of the writer:
fn test_writer_twice2() -> Result<()> {
    let index_dir = LuceneTestCase::new_directory();
    let d = index_dir.as_directory();
    let tw = new_tw(&d)?;
    fill_taxonomy(&tw)?;
    tw.close()?;
    let tw = new_tw(&d)?;
    // run fillTaxonomy again - this will try to add the same categories
    // again, and check that we see the same ordinals again, not different
    // ones, and that the number of categories hasn't grown by the new
    // additions
    fill_taxonomy(&tw)?;
    Assert::assert_equals(EXPECTED_CATEGORIES.len() as i32, tw.get_size()?);
    tw.close()?;
    index_dir.close()
}

/// testWriterTwice3 is yet another test which tests creating a taxonomy in two separate writing
/// sessions. This test used to fail because of a bug involving commit(), explained below, and now
/// should succeed.
fn test_writer_twice3() -> Result<()> {
    let index_dir = LuceneTestCase::new_directory();
    let d = index_dir.as_directory();
    // First, create and fill the taxonomy
    let tw = new_tw(&d)?;
    fill_taxonomy(&tw)?;
    tw.close()?;
    // Now, open the same taxonomy and add the same categories again.
    // After a few categories, the LuceneTaxonomyWriter implementation
    // will stop looking for each category on disk, and rather read them
    // all into memory and close its reader. The bug was that it closed
    // the reader, but forgot that it did (because it didn't set the reader
    // reference to null).
    let tw = new_tw(&d)?;
    fill_taxonomy(&tw)?;
    // Add one new category, just to make commit() do something:
    tw.add_category(&label(&["hi"])?)?;
    // Do a commit(). Here was a bug - if tw had a reader open, it should
    // be reopened after the commit. However, in our case the reader should
    // not be open (as explained above) but because it was not set to null,
    // we forgot that, tried to reopen it, and got an AlreadyClosedException.
    tw.commit()?;
    Assert::assert_equals(EXPECTED_CATEGORIES.len() as i32 + 1, tw.get_size()?);
    tw.close()?;
    index_dir.close()
}

/// Another set of tests for the writer, which don't use an array and try to distill the different
/// cases, and therefore may be more helpful for debugging a problem than testWriter() which is
/// hard to know why or where it failed.
fn test_writer_simpler() -> Result<()> {
    let index_dir = LuceneTestCase::new_directory();
    let d = index_dir.as_directory();
    let tw = new_tw(&d)?;
    Assert::assert_equals(1, tw.get_size()?); // the root only
                                              // Test that adding a new top-level category works
    Assert::assert_equals(1, tw.add_category(&label(&["a"])?)?);
    Assert::assert_equals(2, tw.get_size()?);
    // Test that adding the same category again is noticed, and the
    // same ordinal (and not a new one) is returned.
    Assert::assert_equals(1, tw.add_category(&label(&["a"])?)?);
    Assert::assert_equals(2, tw.get_size()?);
    // Test that adding another top-level category returns a new ordinal,
    // not the same one
    Assert::assert_equals(2, tw.add_category(&label(&["b"])?)?);
    Assert::assert_equals(3, tw.get_size()?);
    // Test that adding a category inside one of the above adds just one
    // new ordinal:
    Assert::assert_equals(3, tw.add_category(&label(&["a", "c"])?)?);
    Assert::assert_equals(4, tw.get_size()?);
    // Test that adding the same second-level category doesn't do anything:
    Assert::assert_equals(3, tw.add_category(&label(&["a", "c"])?)?);
    Assert::assert_equals(4, tw.get_size()?);
    // Test that adding a second-level category with two new components
    // indeed adds two categories
    Assert::assert_equals(5, tw.add_category(&label(&["d", "e"])?)?);
    Assert::assert_equals(6, tw.get_size()?);
    // Verify that the parents were added above in the order we expected
    Assert::assert_equals(4, tw.add_category(&label(&["d"])?)?);
    // Similar, but inside a category that already exists:
    Assert::assert_equals(7, tw.add_category(&label(&["b", "d", "e"])?)?);
    Assert::assert_equals(8, tw.get_size()?);
    // And now inside two levels of categories that already exist:
    Assert::assert_equals(8, tw.add_category(&label(&["b", "d", "f"])?)?);
    Assert::assert_equals(9, tw.get_size()?);

    tw.close()?;
    index_dir.close()
}

/// Test writing an empty index, and seeing that a reader finds in it the root category, and only
/// it. We check all the methods on that root category return the expected results.
fn test_root_only() -> Result<()> {
    let index_dir = LuceneTestCase::new_directory();
    let d = index_dir.as_directory();
    let tw = new_tw(&d)?;
    // right after opening the index, it should already contain the
    // root, so have size 1:
    Assert::assert_equals(1, tw.get_size()?);
    tw.close()?;
    let tr = new_tr(&d)?;
    Assert::assert_equals(1, tr.get_size()?);
    Assert::assert_equals(0usize, tr.get_path(0)?.unwrap().length);
    Assert::assert_equals(
        INVALID_ORDINAL,
        tr.get_parallel_taxonomy_arrays()?.parents().get(0),
    );
    Assert::assert_equals(0, tr.get_ordinal(&FacetLabel::empty())?);
    tr.close()?;
    index_dir.close()
}

/// The following test is exactly the same as testRootOnly, except we do not close the writer
/// before opening the reader. We want to see that the root is visible to the reader not only after
/// the writer is closed, but immediately after it is created.
fn test_root_only2() -> Result<()> {
    let index_dir = LuceneTestCase::new_directory();
    let d = index_dir.as_directory();
    let tw = new_tw(&d)?;
    tw.commit()?;
    let tr = new_tr(&d)?;
    Assert::assert_equals(1, tr.get_size()?);
    Assert::assert_equals(0usize, tr.get_path(0)?.unwrap().length);
    Assert::assert_equals(
        INVALID_ORDINAL,
        tr.get_parallel_taxonomy_arrays()?.parents().get(0),
    );
    Assert::assert_equals(0, tr.get_ordinal(&FacetLabel::empty())?);
    tw.close()?;
    tr.close()?;
    index_dir.close()
}

const IAE: &str = "java.lang.IllegalArgumentException";

/// Basic tests for TaxonomyReader's category &lt;=&gt; ordinal transformations (getSize(),
/// getCategory() and getOrdinal()). We test that after writing the index, it can be read and all
/// the categories and ordinals are there just as we expected them to be.
fn test_reader_basic() -> Result<()> {
    let index_dir = LuceneTestCase::new_directory();
    let d = index_dir.as_directory();
    let tw = new_tw(&d)?;
    fill_taxonomy(&tw)?;
    tw.close()?;
    let tr = new_tr(&d)?;

    // test TaxonomyReader.getSize():
    Assert::assert_equals(EXPECTED_CATEGORIES.len() as i32, tr.get_size()?);

    // test round trips of ordinal => category => ordinal
    for i in 0..tr.get_size()? {
        Assert::assert_equals(i, tr.get_ordinal(&tr.get_path(i)?.unwrap())?);
    }

    // test TaxonomyReader.getCategory():
    for i in 1..tr.get_size()? {
        let expected_category = label(EXPECTED_CATEGORIES[i as usize])?;
        let category = tr.get_path(i)?;
        if category.as_ref() != Some(&expected_category) {
            Assert::fail(&format!(
                "For ordinal {i} expected category {}, but got {}",
                showcat(Some(&expected_category)),
                showcat(category.as_ref())
            ));
        }
    }
    //  (also test invalid ordinals:)
    expect_throws(IAE, || tr.get_path(-1));
    expect_throws(IAE, || tr.get_path(tr.get_size()?));
    expect_throws(IAE, || tr.get_path(INVALID_ORDINAL));

    // test TaxonomyReader.getOrdinal():
    for (i, expected) in EXPECTED_CATEGORIES.iter().enumerate().skip(1) {
        let expected_ordinal = i as i32;
        let ordinal = tr.get_ordinal(&label(expected)?)?;
        if expected_ordinal != ordinal {
            Assert::fail(&format!(
                "For category {} expected ordinal {expected_ordinal}, but got {ordinal}",
                showcat_path(Some(expected))
            ));
        }
    }
    // (also test invalid categories:)
    Assert::assert_equals(INVALID_ORDINAL, tr.get_ordinal(&label(&["non-existant"])?)?);
    Assert::assert_equals(
        INVALID_ORDINAL,
        tr.get_ordinal(&label(&["Author", "Jules Verne"])?)?,
    );

    tr.close()?;
    index_dir.close()
}

/// The "parent" check shared by `testReaderParent` (`parent_of` from the
/// reader's parents array) and `checkWriterParent` (from `tw.getParent`).
fn check_parents(tr: &dyn TaxonomyReader, parent_of: impl Fn(i32) -> Result<i32>) -> Result<()> {
    // check parent of of invalid ordinals:
    for ordinal in 1..tr.get_size()? {
        let me = tr.get_path(ordinal)?.unwrap();
        let parent_ordinal = parent_of(ordinal)?;
        let parent = tr.get_path(parent_ordinal)?;
        let Some(parent) = parent else {
            Assert::fail(&format!(
                "Parent of {ordinal} is {parent_ordinal}, but this is not a valid category."
            ));
        };
        // verify that the parent is indeed my parent, according to the strings
        if me.subpath(me.length - 1) != parent {
            Assert::fail(&format!(
                "Got parent {parent_ordinal} for ordinal {ordinal} but categories are {} and {} respectively.",
                showcat(Some(&parent)),
                showcat(Some(&me))
            ));
        }
    }
    Ok(())
}

/// Tests for TaxonomyReader's getParent() method. We check it by comparing its results to those we
/// could have gotten by looking at the category string paths (where the parentage is obvious).
/// Note that after testReaderBasic(), we already know we can trust the ordinal &lt;=&gt; category
/// conversions.
///
/// Note: At the moment, the parent methods in the reader are deprecated, but this does not mean
/// they should not be tested! Until they are removed (*if* they are removed), these tests should
/// remain to see that they still work correctly.
fn test_reader_parent() -> Result<()> {
    let index_dir = LuceneTestCase::new_directory();
    let d = index_dir.as_directory();
    let tw = new_tw(&d)?;
    fill_taxonomy(&tw)?;
    tw.close()?;
    let tr = new_tr(&d)?;

    // check that the parent of the root ordinal is the invalid ordinal:
    let arrays = tr.get_parallel_taxonomy_arrays()?;
    let parents = arrays.parents();
    Assert::assert_equals(INVALID_ORDINAL, parents.get(0));

    // check parent of non-root ordinals:
    check_parents(&*tr, |ordinal| Ok(parents.get(ordinal)))?;

    tr.close()?;
    index_dir.close()
}

/// Tests for TaxonomyWriter's getParent() method. We check it by comparing its results to those we
/// could have gotten by looking at the category string paths using a TaxonomyReader (where the
/// parentage is obvious). Note that after testReaderBasic(), we already know we can trust the
/// ordinal &lt;=&gt; category conversions from TaxonomyReader.
///
/// <p>The difference between testWriterParent1 and testWriterParent2 is that the former closes the
/// taxonomy writer before reopening it, while the latter does not.
///
/// <p>This test code is virtually identical to that of testReaderParent().
fn test_writer_parent1() -> Result<()> {
    let index_dir = LuceneTestCase::new_directory();
    let d = index_dir.as_directory();
    let tw = new_tw(&d)?;
    fill_taxonomy(&tw)?;
    tw.close()?;
    let tw = new_tw(&d)?;
    let tr = new_tr(&d)?;

    check_writer_parent(&*tr, &tw)?;

    tw.close()?;
    tr.close()?;
    index_dir.close()
}

fn test_writer_parent2() -> Result<()> {
    let index_dir = LuceneTestCase::new_directory();
    let d = index_dir.as_directory();
    let tw = new_tw(&d)?;
    fill_taxonomy(&tw)?;
    tw.commit()?;
    let tr = new_tr(&d)?;

    check_writer_parent(&*tr, &tw)?;

    tw.close()?;
    tr.close()?;
    index_dir.close()
}

const IOOBE: &str = "java.lang.IndexOutOfBoundsException";

fn check_writer_parent(tr: &dyn TaxonomyReader, tw: &dyn TaxonomyWriter) -> Result<()> {
    // check that the parent of the root ordinal is the invalid ordinal:
    Assert::assert_equals(INVALID_ORDINAL, tw.get_parent(0)?);

    // check parent of non-root ordinals:
    check_parents(tr, |ordinal| tw.get_parent(ordinal))?;

    // check parent of of invalid ordinals:
    expect_throws(IOOBE, || tw.get_parent(-1));
    expect_throws(IOOBE, || tw.get_parent(INVALID_ORDINAL));
    expect_throws(IOOBE, || tw.get_parent(tr.get_size()?));
    Ok(())
}

/// Test TaxonomyReader's child browsing method, getChildrenArrays() This only tests for correct
/// behavior on a taxonomy with a single version, and not for correct updating if the reader is
/// refreshed after the taxonomy changed. That is tested in testChildrenArraysGrowth().
// Java's indexed loops over the parallel expected-value tables.
#[allow(clippy::needless_range_loop)]
fn test_children_arrays() -> Result<()> {
    let index_dir = LuceneTestCase::new_directory();
    let d = index_dir.as_directory();
    let tw = new_tw(&d)?;
    fill_taxonomy(&tw)?;
    tw.close()?;
    let tr = new_tr(&d)?;
    let ca = tr.get_parallel_taxonomy_arrays()?;
    let youngest_child_array = ca.children();
    Assert::assert_equals(tr.get_size()?, youngest_child_array.length());
    let older_sibling_array = ca.siblings();
    Assert::assert_equals(tr.get_size()?, older_sibling_array.length());
    for i in 0..EXPECTED_CATEGORIES.len() {
        // find expected children by looking at all expectedCategories
        // for children
        let mut expected_children: Vec<i32> = Vec::new();
        for j in (0..EXPECTED_CATEGORIES.len()).rev() {
            if EXPECTED_CATEGORIES[j].len() != EXPECTED_CATEGORIES[i].len() + 1 {
                continue; // not longer by 1, so can't be a child
            }
            let mut ischild = true;
            for k in 0..EXPECTED_CATEGORIES[i].len() {
                if EXPECTED_CATEGORIES[j][k] != EXPECTED_CATEGORIES[i][k] {
                    ischild = false;
                    break;
                }
            }
            if ischild {
                expected_children.push(j as i32);
            }
        }
        // check that children and expectedChildren are the same, with the
        // correct reverse (youngest to oldest) order:
        if expected_children.is_empty() {
            Assert::assert_equals(INVALID_ORDINAL, youngest_child_array.get(i as i32));
        } else {
            let mut child = youngest_child_array.get(i as i32);
            Assert::assert_equals(expected_children[0], child);
            for expected in expected_children.iter().skip(1) {
                child = older_sibling_array.get(child);
                Assert::assert_equals(*expected, child);
                // if child is INVALID_ORDINAL we should stop, but
                // assertEquals would fail in this case anyway.
            }
            // When we're done comparing, olderSiblingArray should now point
            // to INVALID_ORDINAL, saying there are no more children. If it
            // doesn't, we found too many children...
            Assert::assert_equals(-1, older_sibling_array.get(child));
        }
    }
    tr.close()?;
    index_dir.close()
}

/// Similar to testChildrenArrays, except rather than look at expected results, we test for several
/// "invariants" that the results should uphold, e.g., that a child of a category indeed has this
/// category as its parent. This sort of test can more easily be extended to larger example
/// taxonomies, because we do not need to build the expected list of categories like we did in the
/// above test.
fn test_children_arrays_invariants() -> Result<()> {
    let index_dir = LuceneTestCase::new_directory();
    let d = index_dir.as_directory();
    let tw = new_tw(&d)?;
    fill_taxonomy(&tw)?;
    tw.close()?;
    let tr = new_tr(&d)?;
    let ca = tr.get_parallel_taxonomy_arrays()?;
    let children = ca.children();
    let size = tr.get_size()?;
    Assert::assert_equals(size, children.length());
    let older_sibling_array = ca.siblings();
    Assert::assert_equals(size, older_sibling_array.length());

    // test that the "youngest child" of every category is indeed a child:
    let arrays = tr.get_parallel_taxonomy_arrays()?;
    let parents = arrays.parents();
    for i in 0..size {
        let youngest_child = children.get(i);
        if youngest_child != INVALID_ORDINAL {
            Assert::assert_equals(i, parents.get(youngest_child));
        }
    }

    // test that the "older sibling" of every category is indeed older (lower)
    // (it can also be INVALID_ORDINAL, which is lower than any ordinal)
    for i in 0..size {
        Assert::assert_true_msg(
            &format!("olderSiblingArray[{i}] should be <{i}"),
            older_sibling_array.get(i) < i,
        );
    }

    // test that the "older sibling" of every category is indeed a sibling
    // (they share the same parent)
    for i in 0..size {
        let sibling = older_sibling_array.get(i);
        if sibling == INVALID_ORDINAL {
            continue;
        }
        Assert::assert_equals(parents.get(i), parents.get(sibling));
    }

    // And now for slightly more complex (and less "invariant-like"...)
    // tests:

    // test that the "youngest child" is indeed the youngest (so we don't
    // miss the first children in the chain)
    for i in 0..size {
        // Find the really youngest child:
        let mut j = size - 1;
        while j > i {
            if parents.get(j) == i {
                break; // found youngest child
            }
            j -= 1;
        }
        if j == i {
            // no child found
            j = INVALID_ORDINAL;
        }
        Assert::assert_equals(j, children.get(i));
    }

    // test that the "older sibling" is indeed the least oldest one - and
    // not a too old one or -1 (so we didn't miss some children in the
    // middle or the end of the chain).
    for i in 0..size {
        // Find the youngest older sibling:
        let mut j = i - 1;
        while j >= 0 {
            if parents.get(j) == parents.get(i) {
                break; // found youngest older sibling
            }
            j -= 1;
        }
        if j < 0 {
            // no sibling found
            j = INVALID_ORDINAL;
        }
        Assert::assert_equals(j, older_sibling_array.get(i));
    }

    tr.close()?;
    index_dir.close()
}

fn assert_int_array_equals(expected: &[i32], actual: &dyn IntArray) {
    for (i, e) in expected.iter().enumerate() {
        Assert::assert_equals(*e, actual.get(i as i32));
    }
}

/// Test how getChildrenArrays() deals with the taxonomy's growth:
fn test_children_arrays_growth() -> Result<()> {
    let index_dir = LuceneTestCase::new_directory();
    let d = index_dir.as_directory();
    let tw = new_tw(&d)?;
    tw.add_category(&label(&["hi", "there"])?)?;
    tw.commit()?;
    let tr = new_tr(&d)?;
    let ca = tr.get_parallel_taxonomy_arrays()?;
    Assert::assert_equals(3, tr.get_size()?);
    Assert::assert_equals(3, ca.siblings().length());
    Assert::assert_equals(3, ca.children().length());
    assert_int_array_equals(&[1, 2, -1], ca.children());
    assert_int_array_equals(&[-1, -1, -1], ca.siblings());
    tw.add_category(&label(&["hi", "ho"])?)?;
    tw.add_category(&label(&["hello"])?)?;
    tw.commit()?;
    // Before refresh, nothing changed..
    let newca = tr.get_parallel_taxonomy_arrays()?;
    Assert::assert_true(Arc::ptr_eq(&newca, &ca)); // we got exactly the same object
    Assert::assert_equals(3, tr.get_size()?);
    Assert::assert_equals(3, ca.siblings().length());
    Assert::assert_equals(3, ca.children().length());
    // After the refresh, things change:
    let newtr = taxonomy_reader::open_if_changed(&tr)?;
    Assert::assert_true(newtr.is_some());
    tr.close()?;
    let tr = newtr.unwrap();
    let ca = tr.get_parallel_taxonomy_arrays()?;
    Assert::assert_equals(5, tr.get_size()?);
    Assert::assert_equals(5, ca.siblings().length());
    Assert::assert_equals(5, ca.children().length());
    assert_int_array_equals(&[4, 3, -1, -1, -1], ca.children());
    assert_int_array_equals(&[-1, -1, -1, 2, 1], ca.siblings());
    tw.close()?;
    tr.close()?;
    index_dir.close()
}

// Test that getParentArrays is valid when retrieved during refresh
fn test_taxonomy_reader_refresh_races() -> Result<()> {
    // compute base child arrays - after first chunk, and after the other
    let index_dir_base = LuceneTestCase::new_directory();
    let d = index_dir_base.as_directory();
    let tw_base = new_tw(&d)?;
    tw_base.add_category(&label(&["a", "0"])?)?;
    let ab_path = label(&["a", "b"])?;
    tw_base.add_category(&ab_path)?;
    tw_base.commit()?;
    let tr_base = new_tr(&d)?;

    let ca1 = tr_base.get_parallel_taxonomy_arrays()?;

    let ab_ord = tr_base.get_ordinal(&ab_path)?;
    let ab_young_child_base1 = ca1.children().get(ab_ord);

    let num_categories = LuceneTestCase::at_least(200);
    for i in 0..num_categories {
        tw_base.add_category(&label(&["a", "b", &i.to_string()])?)?;
    }
    tw_base.close()?;

    let new_taxo_reader = taxonomy_reader::open_if_changed(&tr_base)?;
    Assert::assert_true(new_taxo_reader.is_some());
    tr_base.close()?;
    let tr_base = new_taxo_reader.unwrap();

    let ca2 = tr_base.get_parallel_taxonomy_arrays()?;
    let ab_young_child_base2 = ca2.children().get(ab_ord);

    let num_retries = LuceneTestCase::at_least(10);
    for retry in 0..num_retries {
        assert_consistent_youngest_child(
            &ab_path,
            ab_ord,
            ab_young_child_base1,
            ab_young_child_base2,
            retry,
            num_categories,
        )?;
    }

    tr_base.close()?;
    index_dir_base.close()
}

fn assert_consistent_youngest_child(
    ab_path: &FacetLabel,
    ab_ord: i32,
    ab_young_child_base1: i32,
    ab_young_child_base2: i32,
    retry: i32,
    num_categories: i32,
) -> Result<()> {
    let index_dir = Arc::new(SlowDirectory::new(-1, None)); // no slowness for initialization
    let d: Arc<dyn Directory> = Arc::clone(&index_dir) as Arc<dyn Directory>;
    let tw = new_tw(&d)?;
    tw.add_category(&label(&["a", "0"])?)?;
    tw.add_category(ab_path)?;
    tw.commit()?;

    let tr: Arc<dyn TaxonomyReader> = new_tr(&d)?;
    for i in 0..num_categories {
        let cp = label(&["a", "b", &i.to_string()])?;
        tw.add_category(&cp)?;
        Assert::assert_equals_msg(
            Some(&format!(
                "Ordinal of {cp} must be invalid until Taxonomy Reader was refreshed"
            )),
            INVALID_ORDINAL,
            tr.get_ordinal(&cp)?,
        );
    }
    tw.close()?;

    let stop = Arc::new(AtomicBool::new(false));
    let error: Arc<Mutex<Option<Throwable>>> = Arc::new(Mutex::new(None));
    let retrieval = Arc::new(AtomicI32::new(0));

    let thread = {
        let (stop, error, retrieval, tr) = (
            Arc::clone(&stop),
            Arc::clone(&error),
            Arc::clone(&retrieval),
            Arc::clone(&tr),
        );
        let ab_path = ab_path.clone();
        Thread::new("Child Arrays Verifier", move || {
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
                || -> Result<()> {
                    while !stop.load(Ordering::SeqCst) {
                        let last_ord = tr.get_parallel_taxonomy_arrays()?.parents().length() - 1;
                        Assert::assert_true_msg(
                            &format!("path of last-ord {last_ord} is not found!"),
                            tr.get_path(last_ord)?.is_some(),
                        );
                        let ca = tr.get_parallel_taxonomy_arrays()?;
                        let retrieval_now = retrieval.fetch_add(1, Ordering::SeqCst);
                        // assertChildrenArrays(ca, retry, retrieval)
                        let ab_young_child = ca.children().get(ab_ord);
                        Assert::assert_true_msg(
                        &format!(
                            "Retry {retry}: retrieval: {retrieval_now}: wrong youngest child for category {ab_path} (ord={ab_ord}) - must be either {ab_young_child_base1} or {ab_young_child_base2} but was: {ab_young_child}"
                        ),
                        ab_young_child_base1 == ab_young_child
                            || ab_young_child_base2 == ca.children().get(ab_ord),
                    );
                        std::thread::sleep(std::time::Duration::from_millis(10));
                        // don't starve refresh()'s CPU, which sleeps every 50 bytes for 1 ms
                    }
                    Ok(())
                },
            ));
            let e = match outcome {
                Ok(Ok(())) => return,
                Ok(Err(e)) => java_throwable(&e),
                Err(payload) => Throwable::from_panic(payload),
            };
            *error.lock().unwrap_or_else(|p| p.into_inner()) = Some(e);
            stop.store(true, Ordering::SeqCst);
        })
    };

    thread.start();

    index_dir.set_sleep_millis(1); // some delay for refresh
    let new_taxo_reader = taxonomy_reader::open_if_changed(&tr)?;
    if let Some(r) = new_taxo_reader {
        r.close()?;
    }

    stop.store(true, Ordering::SeqCst);
    if let Err(payload) = thread.join() {
        std::panic::resume_unwind(payload);
    }
    let error = error.lock().unwrap_or_else(|p| p.into_inner()).take();
    if let Some(e) = &error {
        Assert::fail(&format!(
            "Unexpcted exception at retry {retry} retrieval {}: \n{e:?}",
            retrieval.load(Ordering::SeqCst)
        ));
    }

    tr.close()
}

/// Test that if separate reader and writer objects are opened, new categories written into the
/// writer are available to a reader only after a commit(). Note that this test obviously doesn't
/// cover all the different concurrency scenarios, all different methods, and so on. We may want to
/// write more tests of this sort.
///
/// <p>This test simulates what would happen when there are two separate processes, one doing
/// indexing, and the other searching, and each opens its own object (with obviously no connection
/// between the objects) using the same disk files. Note, though, that this test does not test what
/// happens when the two processes do their actual work at exactly the same time. It also doesn't
/// test multi-threading.
fn test_separate_reader_and_writer() -> Result<()> {
    let index_dir = LuceneTestCase::new_directory();
    let d = index_dir.as_directory();
    let tw = new_tw(&d)?;
    tw.commit()?;
    let tr = new_tr(&d)?;

    Assert::assert_equals(1, tr.get_size()?); // the empty taxonomy has size 1 (the root)
    tw.add_category(&label(&["Author"])?)?;
    Assert::assert_equals(1, tr.get_size()?); // still root only...
    Assert::assert_true(taxonomy_reader::open_if_changed(&tr)?.is_none()); // this is not enough, because tw.commit() hasn't been done yet
    Assert::assert_equals(1, tr.get_size()?); // still root only...
    tw.commit()?;
    Assert::assert_equals(1, tr.get_size()?); // still root only...
    let new_taxo_reader = taxonomy_reader::open_if_changed(&tr)?;
    Assert::assert_true(new_taxo_reader.is_some());
    tr.close()?;
    let tr = new_taxo_reader.unwrap();

    let author = 1;
    // `catch (ArrayIndexOutOfBoundsException e) { fail(...) }`: an
    // out-of-bounds read panics in the port; reaching the assertion is the
    // success path.
    Assert::assert_equals(
        ROOT_ORDINAL,
        tr.get_parallel_taxonomy_arrays()?.parents().get(author),
    );
    // ok
    Assert::assert_equals(2, tr.get_size()?); // finally, see there are two categories

    // now, add another category, and verify that after commit and refresh
    // the parent of this category is correct (this requires the reader
    // to correctly update its prefetched parent vector), and that the
    // old information also wasn't ruined:
    tw.add_category(&label(&["Author", "Richard Dawkins"])?)?;
    let dawkins = 2;
    tw.commit()?;
    let new_taxo_reader = taxonomy_reader::open_if_changed(&tr)?;
    Assert::assert_true(new_taxo_reader.is_some());
    tr.close()?;
    let tr = new_taxo_reader.unwrap();
    let arrays = tr.get_parallel_taxonomy_arrays()?;
    let parents = arrays.parents();
    Assert::assert_equals(author, parents.get(dawkins));
    Assert::assert_equals(ROOT_ORDINAL, parents.get(author));
    Assert::assert_equals(INVALID_ORDINAL, parents.get(ROOT_ORDINAL));
    Assert::assert_equals(3, tr.get_size()?);
    tw.close()?;
    tr.close()?;
    index_dir.close()
}

fn test_separate_reader_and_writer2() -> Result<()> {
    let index_dir = LuceneTestCase::new_directory();
    let d = index_dir.as_directory();
    let tw = new_tw(&d)?;
    tw.commit()?;
    let tr = new_tr(&d)?;

    // Test getOrdinal():
    let author = label(&["Author"])?;

    Assert::assert_equals(1, tr.get_size()?); // the empty taxonomy has size 1 (the root)
    Assert::assert_equals(INVALID_ORDINAL, tr.get_ordinal(&author)?);
    tw.add_category(&author)?;
    // before commit and refresh, no change:
    Assert::assert_equals(INVALID_ORDINAL, tr.get_ordinal(&author)?);
    Assert::assert_equals(1, tr.get_size()?); // still root only...
    Assert::assert_true(taxonomy_reader::open_if_changed(&tr)?.is_none()); // this is not enough, because tw.commit() hasn't been done yet
    Assert::assert_equals(INVALID_ORDINAL, tr.get_ordinal(&author)?);
    Assert::assert_equals(1, tr.get_size()?); // still root only...
    tw.commit()?;
    // still not enough before refresh:
    Assert::assert_equals(INVALID_ORDINAL, tr.get_ordinal(&author)?);
    Assert::assert_equals(1, tr.get_size()?); // still root only...
    let new_taxo_reader = taxonomy_reader::open_if_changed(&tr)?;
    Assert::assert_true(new_taxo_reader.is_some());
    tr.close()?;
    let tr = new_taxo_reader.unwrap();
    Assert::assert_equals(1, tr.get_ordinal(&author)?);
    Assert::assert_equals(2, tr.get_size()?);
    tw.close()?;
    tr.close()?;
    index_dir.close()
}

fn ancestor_failure(i: usize, j: usize, expected_ordinal: i32, ordinal: i32) -> ! {
    Assert::fail(&format!(
        "For category {} expected ancestor level {} was {expected_ordinal}, but got {ordinal}",
        showcat_path(Some(CATEGORIES[i])),
        EXPECTED_PATHS[i].len() - 1 - j
    ))
}

// fillTaxonomyCheckPaths adds the categories in the categories[] array,
// and asserts that the additions return exactly paths specified in
// expectedPaths[]. This is the same add fillTaxonomy() but also checks
// the correctness of getParent(), not just addCategory().
// Note that this assumes that fillTaxonomyCheckPaths() is called on an empty
// taxonomy index. Calling it after something else was already added to the
// taxonomy index will surely have this method fail.
fn fill_taxonomy_check_paths(tw: &dyn TaxonomyWriter) -> Result<()> {
    for (i, category) in CATEGORIES.iter().enumerate() {
        let mut ordinal = tw.add_category(&label(category)?)?;
        let mut expected_ordinal = *EXPECTED_PATHS[i].last().unwrap();
        if ordinal != expected_ordinal {
            Assert::fail(&format!(
                "For category {} expected ordinal {expected_ordinal}, but got {ordinal}",
                showcat_path(Some(category))
            ));
        }
        for j in (0..EXPECTED_PATHS[i].len() - 1).rev() {
            ordinal = tw.get_parent(ordinal)?;
            expected_ordinal = EXPECTED_PATHS[i][j];
            if ordinal != expected_ordinal {
                ancestor_failure(i, j, expected_ordinal, ordinal);
            }
        }
    }
    Ok(())
}

// After fillTaxonomy returned successfully, checkPaths() checks that
// the getParent() calls return as expected, from the table
// Java's indexed loops over the parallel expected-value tables.
#[allow(clippy::needless_range_loop)]
fn check_paths(tw: &dyn TaxonomyWriter) -> Result<()> {
    for i in 0..CATEGORIES.len() {
        let mut ordinal = *EXPECTED_PATHS[i].last().unwrap();
        for j in (0..EXPECTED_PATHS[i].len() - 1).rev() {
            ordinal = tw.get_parent(ordinal)?;
            let expected_ordinal = EXPECTED_PATHS[i][j];
            if ordinal != expected_ordinal {
                ancestor_failure(i, j, expected_ordinal, ordinal);
            }
        }
        Assert::assert_equals(ROOT_ORDINAL, tw.get_parent(EXPECTED_PATHS[i][0])?);
    }
    Assert::assert_equals(INVALID_ORDINAL, tw.get_parent(ROOT_ORDINAL)?);
    Ok(())
}

/// Basic test for TaxonomyWriter.getParent(). This is similar to testWriter above, except we also
/// check the parents of the added categories, not just the categories themselves.
fn test_writer_check_paths() -> Result<()> {
    let index_dir = LuceneTestCase::new_directory();
    let d = index_dir.as_directory();
    let tw = new_tw(&d)?;
    fill_taxonomy_check_paths(&tw)?;
    // Also check TaxonomyWriter.getSize() - see that the taxonomy's size
    // is what we expect it to be.
    Assert::assert_equals(EXPECTED_CATEGORIES.len() as i32, tw.get_size()?);
    tw.close()?;
    index_dir.close()
}

/// testWriterCheckPaths2 is the path-checking variant of testWriterTwice and testWriterTwice2.
/// After adding all the categories, we add them again, and see that we get the same old ids and
/// paths. We repeat the path checking yet again after closing and opening the index for writing
/// again - to see that the reading of existing data from disk works as well.
fn test_writer_check_paths2() -> Result<()> {
    let index_dir = LuceneTestCase::new_directory();
    let d = index_dir.as_directory();
    let tw = new_tw(&d)?;
    fill_taxonomy(&tw)?;
    check_paths(&tw)?;
    fill_taxonomy(&tw)?;
    check_paths(&tw)?;
    tw.close()?;

    let tw = new_tw(&d)?;
    check_paths(&tw)?;
    fill_taxonomy(&tw)?;
    check_paths(&tw)?;
    tw.close()?;
    index_dir.close()
}

fn test_nrt() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let writer = Arc::new(DirectoryTaxonomyWriter::new(dir.as_directory())?);
    let reader: Arc<dyn TaxonomyReader> =
        Arc::new(DirectoryTaxonomyReader::from_writer(Arc::clone(&writer))?);

    let cp = label(&["a"])?;
    writer.add_category(&cp)?;
    let new_reader = taxonomy_reader::open_if_changed(&reader)?;
    Assert::assert_true_msg("expected a new instance", new_reader.is_some());
    let new_reader = new_reader.unwrap();
    Assert::assert_equals(2, new_reader.get_size()?);
    // `assertNotSame(INVALID_ORDINAL, newReader.getOrdinal(cp))`: both
    // boxed through the `Integer` cache (-128..127), so "same" means equal.
    Assert::assert_true(INVALID_ORDINAL != new_reader.get_ordinal(&cp)?);
    reader.close()?;
    let reader = new_reader;

    writer.close()?;
    reader.close()?;

    dir.close()
}

fn many_categories(count: i32, round_size: i32) -> Vec<Vec<String>> {
    let mut result: Vec<Option<Vec<String>>> = vec![None; (count / round_size + 1) as usize];
    let mut k = 0;
    loop {
        k += round_size;
        let mut round = Vec::new();
        let mut i = k - round_size + 1;
        while i <= k && i < count {
            round.push(format!("category {i} of {k}"));
            i += 1;
        }
        result[(k / round_size - 1) as usize] = Some(round);
        if k > count {
            break;
        }
    }
    result
        .into_iter()
        .map(|r| r.expect("INVARIANT: every round is filled"))
        .collect()
}

fn test_thousands_of_categories() -> Result<()> {
    let random = LuceneTestCase::random();
    let round_size = random.next_int_range(2, 4);
    let size = random.next_int_range(16384, 32768);
    let index_dir = LuceneTestCase::new_directory();
    let d = index_dir.as_directory();
    let tw = new_tw(&d)?;
    let many_categories = many_categories(size, round_size);
    for elem in &many_categories {
        if !elem.is_empty() {
            tw.add_category(&FacetLabel::new(elem.iter().cloned())?)?;
        }
    }
    tw.close()?;
    let tr = new_tr(&d)?;
    let ca = tr.get_parallel_taxonomy_arrays()?;
    let parents = ca.parents();
    let children = ca.children();
    Assert::assert_equals(size, parents.length());
    Assert::assert_equals(size, children.length());
    let mut j = 1;
    while j < size - round_size {
        // Top level categories all have root as their parent.
        Assert::assert_equals(0, parents.get(j));
        for i in j..j + round_size - 1 {
            // Children extend in a chain from the top level category.
            // The parent/child relationships are symmetric.
            Assert::assert_equals(i + 1, children.get(i));
            if i > j {
                Assert::assert_equals(i - 1, parents.get(i));
            }
        }
        j += round_size;
    }
    let siblings = ca.siblings();
    Assert::assert_equals(size, siblings.length());
    let mut i = 1;
    while i < size - round_size {
        // Each top-level category (after the first) has the previous top-level category as their
        // older sibling.
        Assert::assert_equals(i, siblings.get(i + round_size));
        i += round_size;
    }
    tr.close()?;
    index_dir.close()
}
