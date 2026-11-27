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

//! Port of `org.apache.lucene.facet.taxonomy.directory.TestDirectoryTaxonomyWriter`.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, LazyLock, Mutex};

use randomizedtesting::generators::RandomNumbers;
use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::jdk::thread::Thread;
use randomizedtesting::junit::Assert;
use rucene::document::Document;
use rucene::error::Result;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::{IndexWriterConfig, OpenMode};
use rucene::index::segment_infos::SegmentInfos;
use rucene::index::{directory_reader, TwoPhaseCommit};
use rucene::search::index_searcher::IndexSearcher;
use rucene::store::Directory;
use rucene_facet::drill_down_query::DrillDownQuery;
use rucene_facet::facet_field::FacetField;
use rucene_facet::facets_config::FacetsConfig;
use rucene_facet::taxonomy::directory::directory_taxonomy_reader::DirectoryTaxonomyReader;
use rucene_facet::taxonomy::directory::directory_taxonomy_writer::{
    DirectoryTaxonomyWriter, MemoryOrdinalMap, INDEX_EPOCH,
};
use rucene_facet::taxonomy::facet_label::FacetLabel;
use rucene_facet::taxonomy::taxonomy_reader::{self, TaxonomyReader};
use rucene_facet::taxonomy::taxonomy_writer::TaxonomyWriter;
use rucene_facet::taxonomy::writercache::lru_taxonomy_writer_cache::LruTaxonomyWriterCache;
use rucene_facet::taxonomy::writercache::taxonomy_writer_cache::TaxonomyWriterCache;
use rucene_test_framework::util::java_exceptions::{expect_throws, java_throwable};
use rucene_test_framework::util::lucene_test_case::{Nightly, TEST_NIGHTLY, VERBOSE};
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::facet_support::core_support::test;
use crate::facet_support::facet_class;

/// `TestDirectoryTaxonomyWriter`.
pub fn class_def() -> Arc<Class> {
    facet_class(
        "org.apache.lucene.facet.taxonomy.directory.TestDirectoryTaxonomyWriter",
        vec![
            test("testCommit", test_commit),
            test("testCommitUserData", test_commit_user_data),
            test("testRollback", test_rollback),
            test("testRecreateRollback", test_recreate_rollback),
            test("testEnsureOpen", test_ensure_open),
            test("testRecreateAndRefresh", test_recreate_and_refresh),
            test("testBackwardsCompatibility", test_backwards_compatibility),
            test("testConcurrency", test_concurrency),
            test("testReplaceTaxonomy", test_replace_taxonomy),
            test("testReaderFreshness", test_reader_freshness),
            test("testCommitNoEmptyCommits", test_commit_no_empty_commits),
            test("testCloseNoEmptyCommits", test_close_no_empty_commits),
            test(
                "testPrepareCommitNoEmptyCommits",
                test_prepare_commit_no_empty_commits,
            ),
            test("testHugeLabel", test_huge_label).annotate(Nightly),
            test(
                "testReplaceTaxoWithLargeTaxonomy",
                test_replace_taxo_with_large_taxonomy,
            ),
        ],
    )
}

/// A No-Op TaxonomyWriterCache which always discards all given categories,
/// and always returns true in put(), to indicate some cache entries were
/// cleared.
struct NoOpCache;

impl TaxonomyWriterCache for NoOpCache {
    fn close(&self) -> Result<()> {
        Ok(())
    }
    fn get(&self, _category_path: &FacetLabel) -> Result<i32> {
        Ok(-1)
    }
    fn put(&self, _category_path: &FacetLabel, _ordinal: i32) -> Result<bool> {
        Ok(true)
    }
    fn is_full(&self) -> Result<bool> {
        Ok(true)
    }
    fn clear(&self) -> Result<()> {
        Ok(())
    }
    fn size(&self) -> Result<i32> {
        Ok(0)
    }
}

/// `NO_OP_CACHE`: one shared instance, as Java's static field.
static NO_OP_CACHE: LazyLock<Arc<dyn TaxonomyWriterCache>> = LazyLock::new(|| Arc::new(NoOpCache));

fn no_op_cache() -> Arc<dyn TaxonomyWriterCache> {
    Arc::clone(&NO_OP_CACHE)
}

fn label(components: &[&str]) -> Result<FacetLabel> {
    FacetLabel::new(components.iter().copied())
}

fn with_cache(
    dir: &Arc<dyn Directory>,
    mode: OpenMode,
    cache: Arc<dyn TaxonomyWriterCache>,
) -> Result<DirectoryTaxonomyWriter> {
    DirectoryTaxonomyWriter::with_cache(Arc::clone(dir), mode, Some(cache))
}

fn test_commit() -> Result<()> {
    // Verifies that nothing is committed to the underlying Directory, if
    // commit() wasn't called.
    let dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let ltw = with_cache(&d, OpenMode::CREATE_OR_APPEND, no_op_cache())?;
    Assert::assert_false(directory_reader::index_exists(&*d)?);
    ltw.commit()?; // first commit, so that an index will be created
    ltw.add_category(&label(&["a"])?)?;

    let r = directory_reader::open(Arc::clone(&d))?;
    Assert::assert_equals_msg(
        Some("No categories should have been committed to the underlying directory"),
        1,
        r.num_docs(),
    );
    r.close()?;
    ltw.close()?;
    dir.close()
}

fn test_commit_user_data() -> Result<()> {
    // Verifies taxonomy commit data
    let dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let taxo_writer = with_cache(&d, OpenMode::CREATE_OR_APPEND, no_op_cache())?;
    Assert::assert_true(Arc::ptr_eq(taxo_writer.get_cache(), &no_op_cache()));
    taxo_writer.add_category(&label(&["a"])?)?;
    taxo_writer.add_category(&label(&["b"])?)?;
    let user_commit_data = vec![("testing".to_string(), "1 2 3".to_string())];
    taxo_writer.set_live_commit_data(Some(user_commit_data))?;
    taxo_writer.close()?;
    let r = directory_reader::open(Arc::clone(&d))?;
    Assert::assert_equals_msg(
        Some("2 categories plus root should have been committed to the underlying directory"),
        3,
        r.num_docs(),
    );
    let read_user_commit_data = r.index_commit()?.get_user_data()?;
    Assert::assert_true_msg(
        "wrong value extracted from commit data",
        read_user_commit_data.get("testing").map(String::as_str) == Some("1 2 3"),
    );
    Assert::assert_true_msg(
        &format!("{INDEX_EPOCH} not found in commitData"),
        read_user_commit_data.contains_key(INDEX_EPOCH),
    );
    r.close()?;

    // open DirTaxoWriter again and commit, INDEX_EPOCH should still exist
    // in the commit data, otherwise DirTaxoReader.refresh() might not detect
    // that the taxonomy index has been recreated.
    let taxo_writer = with_cache(&d, OpenMode::CREATE_OR_APPEND, no_op_cache())?;
    taxo_writer.add_category(&label(&["c"])?)?; // add a category so that commit will happen
    taxo_writer.set_live_commit_data(Some(vec![("just".to_string(), "data".to_string())]))?;
    taxo_writer.commit()?;

    // verify taxoWriter.getCommitData()
    let mut data = HashMap::new();
    for (k, v) in taxo_writer.get_live_commit_data()? {
        data.insert(k, v);
    }

    Assert::assert_true_msg(
        &format!("{INDEX_EPOCH} not found in taoxWriter.commitData"),
        data.contains_key(INDEX_EPOCH),
    );
    taxo_writer.close()?;

    let r = directory_reader::open(Arc::clone(&d))?;
    let read_user_commit_data = r.index_commit()?.get_user_data()?;
    Assert::assert_true_msg(
        &format!("{INDEX_EPOCH} not found in commitData"),
        read_user_commit_data.contains_key(INDEX_EPOCH),
    );
    r.close()?;

    dir.close()
}

const ACE: &str = "org.apache.lucene.store.AlreadyClosedException";

fn test_rollback() -> Result<()> {
    // Verifies that if rollback is called, DTW is closed.
    let dir = LuceneTestCase::new_directory();
    let dtw = DirectoryTaxonomyWriter::new(dir.as_directory())?;
    Assert::assert_true(
        dtw.get_cache()
            .as_any()
            .is_some_and(|c| c.is::<LruTaxonomyWriterCache>()),
    );
    dtw.add_category(&label(&["a"])?)?;
    dtw.rollback()?;
    // should not have succeeded to add a category following rollback.
    expect_throws(ACE, || dtw.add_category(&label(&["a"])?));

    dir.close()
}

fn test_recreate_rollback() -> Result<()> {
    // Tests rollback with OpenMode.CREATE
    let dir = LuceneTestCase::new_directory();
    DirectoryTaxonomyWriter::new(dir.as_directory())?.close()?;
    Assert::assert_equals(1i64, get_epoch(&dir.as_directory())?);
    DirectoryTaxonomyWriter::with_open_mode(dir.as_directory(), OpenMode::CREATE)?.rollback()?;
    Assert::assert_equals(1i64, get_epoch(&dir.as_directory())?);

    dir.close()
}

fn test_ensure_open() -> Result<()> {
    // verifies that an exception is thrown if DTW was closed
    let dir = LuceneTestCase::new_directory();
    let dtw = DirectoryTaxonomyWriter::new(dir.as_directory())?;
    dtw.close()?;
    // should not succeed to add a category following close.
    expect_throws(ACE, || dtw.add_category(&label(&["a"])?));

    dir.close()
}

fn touch_taxo(taxo_writer: &DirectoryTaxonomyWriter, cp: &FacetLabel) -> Result<()> {
    taxo_writer.add_category(cp)?;
    taxo_writer.set_live_commit_data(Some(vec![("just".to_string(), "data".to_string())]))?;
    taxo_writer.commit()?;
    Ok(())
}

fn epoch_of(taxo_reader: &Arc<dyn TaxonomyReader>) -> Result<i32> {
    Ok(taxo_reader
        .get_commit_user_data()?
        .get(INDEX_EPOCH)
        .expect("index.epoch in the commit data")
        .parse::<i32>()
        .unwrap())
}

fn test_recreate_and_refresh() -> Result<()> {
    // DirTaxoWriter lost the INDEX_EPOCH property if it was opened in
    // CREATE_OR_APPEND (or commit(userData) called twice), which could lead to
    // DirTaxoReader succeeding to refresh().
    let dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let body = || -> Result<()> {
        let taxo_writer = with_cache(&d, OpenMode::CREATE_OR_APPEND, no_op_cache())?;
        touch_taxo(&taxo_writer, &label(&["a"])?)?;

        let taxo_reader: Arc<dyn TaxonomyReader> =
            Arc::new(DirectoryTaxonomyReader::open(Arc::clone(&d))?);

        touch_taxo(&taxo_writer, &label(&["b"])?)?;

        let newtr = taxonomy_reader::open_if_changed(&taxo_reader)?;
        taxo_reader.close()?;
        let taxo_reader = newtr.expect("a changed taxonomy reader");
        Assert::assert_equals(1, epoch_of(&taxo_reader)?);

        // now recreate the taxonomy, and check that the epoch is preserved after opening DirTW again.
        taxo_writer.close()?;

        let taxo_writer = with_cache(&d, OpenMode::CREATE, no_op_cache())?;
        touch_taxo(&taxo_writer, &label(&["c"])?)?;
        taxo_writer.close()?;

        let taxo_writer = with_cache(&d, OpenMode::CREATE_OR_APPEND, no_op_cache())?;
        touch_taxo(&taxo_writer, &label(&["d"])?)?;
        taxo_writer.close()?;

        let newtr = taxonomy_reader::open_if_changed(&taxo_reader)?;
        taxo_reader.close()?;
        let taxo_reader = newtr.expect("a changed taxonomy reader");
        Assert::assert_equals(2, epoch_of(&taxo_reader)?);
        taxo_reader.close()
    };
    let result = body();
    // try (Directory dir = newDirectory()) { ... }
    let closed = dir.close();
    result.and(closed)
}

fn test_backwards_compatibility() -> Result<()> {
    // tests that if the taxonomy index doesn't have the INDEX_EPOCH
    // property (supports pre-3.6 indexes), all still works.
    let dir = LuceneTestCase::new_directory();

    // create an empty index first, so that DirTaxoWriter initializes indexEpoch to 1.
    IndexWriter::new(dir.as_directory(), IndexWriterConfig::new())?.close()?;

    let taxo_writer = with_cache(
        &dir.as_directory(),
        OpenMode::CREATE_OR_APPEND,
        no_op_cache(),
    )?;
    taxo_writer.close()?;

    let taxo_reader: Arc<dyn TaxonomyReader> =
        Arc::new(DirectoryTaxonomyReader::open(dir.as_directory())?);
    Assert::assert_equals(1, epoch_of(&taxo_reader)?);
    Assert::assert_true(taxonomy_reader::open_if_changed(&taxo_reader)?.is_none());
    taxo_reader.close()?;

    dir.close()
}

fn test_concurrency() -> Result<()> {
    let ncats = if *TEST_NIGHTLY {
        LuceneTestCase::at_least(100000)
    } else {
        LuceneTestCase::at_least(1000)
    }; // at night, add many categories
    let range = ncats * 3; // affects the categories selection
    let num_cats = Arc::new(AtomicI32::new(ncats));
    let dir = LuceneTestCase::new_directory();
    let values: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));
    let d = LuceneTestCase::random().next_double();
    // (cache, the class name its `toString()` starts with)
    let (cache, cache_name): (Arc<dyn TaxonomyWriterCache>, &str) = if d < 0.7 {
        // this is the fastest, yet most memory consuming
        (
            DirectoryTaxonomyWriter::default_taxonomy_writer_cache(),
            "org.apache.lucene.facet.taxonomy.writercache.LruTaxonomyWriterCache",
        )
    } else if *TEST_NIGHTLY && d > 0.98 {
        // this is the slowest, but tests the writer concurrency when no caching is done.
        // only pick it during NIGHTLY tests, and even then, with very low chances.
        (
            no_op_cache(),
            "org.apache.lucene.facet.taxonomy.directory.TestDirectoryTaxonomyWriter$1",
        )
    } else {
        // this is slower than UTF8, but less memory consuming, and exercises finding categories on
        // disk too.
        (
            Arc::new(LruTaxonomyWriterCache::new(ncats / 10)),
            "org.apache.lucene.facet.taxonomy.writercache.LruTaxonomyWriterCache",
        )
    };
    if *VERBOSE {
        println!("TEST: use cache={cache_name}");
    }
    let tw = Arc::new(with_cache(&dir.as_directory(), OpenMode::CREATE, cache)?);
    let num_threads = RandomNumbers::random_int_between(&*LuceneTestCase::random(), 1, 12);
    let mut add_threads = Vec::new();
    for _ in 0..num_threads {
        let (num_cats, values, tw) = (Arc::clone(&num_cats), Arc::clone(&values), Arc::clone(&tw));
        add_threads.push(Thread::unnamed(move || {
            let random = LuceneTestCase::random();
            while num_cats.fetch_sub(1, Ordering::SeqCst) - 1 > 0 {
                let outcome = (|| -> Result<()> {
                    let value = random.next_int_bound(range);
                    let cp = FacetLabel::new([
                        (value / 1000).to_string(),
                        (value / 10000).to_string(),
                        (value / 100000).to_string(),
                        value.to_string(),
                    ])?;
                    let ord = tw.add_category(&cp)?;
                    Assert::assert_true_msg(
                        &format!("invalid parent for ordinal {ord}, category {cp}"),
                        tw.get_parent(ord)? != -1,
                    );
                    let mut values = values.lock().unwrap_or_else(|e| e.into_inner());
                    for level in 1..=4 {
                        values.insert(FacetsConfig::path_to_string_len(&cp.components, level)?);
                    }
                    Ok(())
                })();
                if let Err(e) = outcome {
                    if e.is_io_exception() {
                        Throwable::from_cause("java.lang.RuntimeException", java_throwable(&e))
                            .throw()
                    }
                    java_throwable(&e).throw()
                }
            }
        }));
    }

    for t in &add_threads {
        t.start();
    }
    for t in &add_threads {
        if let Err(payload) = t.join() {
            std::panic::resume_unwind(payload);
        }
    }
    tw.close()?;

    let dtr = DirectoryTaxonomyReader::open(dir.as_directory())?;
    let values: Vec<String> = values
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .cloned()
        .collect();
    // +1 for root category
    if values.len() as i32 + 1 != dtr.get_size()? {
        for value in &values {
            let label = FacetLabel::new(FacetsConfig::string_to_path(value))?;
            if dtr.get_ordinal(&label)? == -1 {
                println!("FAIL: path={label} not recognized");
            }
        }
        Assert::fail("mismatch number of categories");
    }

    let arrays = dtr.get_parallel_taxonomy_arrays()?;
    let parents = arrays.parents();
    for cat in &values {
        let cp = FacetLabel::new(FacetsConfig::string_to_path(cat))?;
        Assert::assert_true_msg(
            &format!("category not found {cp}"),
            dtr.get_ordinal(&cp)? > 0,
        );
        let level = cp.length;
        let mut parent_ord = 0; // for root, parent is always virtual ROOT (ord=0)
        for i in 0..level {
            let path = cp.subpath(i + 1);
            let ord = dtr.get_ordinal(&path)?;
            Assert::assert_equals_msg(
                Some(&format!("invalid parent for cp={path}")),
                parent_ord,
                parents.get(ord),
            );
            parent_ord = ord; // next level should have this parent
        }
    }

    dtr.close()?;
    dir.close()
}

fn get_epoch(taxo_dir: &Arc<dyn Directory>) -> Result<i64> {
    let infos = SegmentInfos::read_latest_commit(Arc::clone(taxo_dir))?;
    Ok(infos
        .user_data()
        .get(INDEX_EPOCH)
        .expect("index.epoch in the commit data")
        .parse::<i64>()
        .unwrap())
}

fn test_replace_taxonomy() -> Result<()> {
    let input = LuceneTestCase::new_directory();
    let taxo_writer = DirectoryTaxonomyWriter::new(input.as_directory())?;
    let ord_a = taxo_writer.add_category(&label(&["a"])?)?;
    taxo_writer.close()?;

    let dir = LuceneTestCase::new_directory();
    let taxo_writer = DirectoryTaxonomyWriter::new(dir.as_directory())?;
    let ord_b = taxo_writer.add_category(&label(&["b"])?)?;
    taxo_writer.add_category(&label(&["c"])?)?;
    taxo_writer.commit()?;

    let orig_epoch = get_epoch(&dir.as_directory())?;

    // replace the taxonomy with the input one
    taxo_writer.replace_taxonomy(input.as_directory())?;

    // LUCENE-4633: make sure that category "a" is not added again in any case
    taxo_writer.add_taxonomy(input.as_directory(), &mut MemoryOrdinalMap::new())?;
    Assert::assert_equals_msg(
        Some("no categories should have been added"),
        2,
        taxo_writer.get_size()?,
    ); // root + 'a'
    Assert::assert_equals_msg(
        Some("category 'a' received new ordinal?"),
        ord_a,
        taxo_writer.add_category(&label(&["a"])?)?,
    );

    // add the same category again -- it should not receive the same ordinal !
    let new_ord_b = taxo_writer.add_category(&label(&["b"])?)?;
    // `assertNotSame(msg, ordB, newOrdB)`: boxed through the `Integer`
    // cache, so "same" means equal.
    Assert::assert_true_msg(
        "new ordinal cannot be the original ordinal",
        ord_b != new_ord_b,
    );
    Assert::assert_equals_msg(
        Some("ordinal should have been 2 since only one category was added by replaceTaxonomy"),
        2,
        new_ord_b,
    );

    taxo_writer.close()?;

    let new_epoch = get_epoch(&dir.as_directory())?;
    Assert::assert_true_msg(
        "index epoch should have been updated after replaceTaxonomy",
        orig_epoch < new_epoch,
    );

    dir.close()?;
    input.close()
}

fn test_reader_freshness() -> Result<()> {
    // ensures that the internal index reader is always kept fresh. Previously,
    // this simple scenario failed, if the cache just evicted the category that
    // is being added.
    let dir = LuceneTestCase::new_directory();
    let taxo_writer = with_cache(&dir.as_directory(), OpenMode::CREATE, no_op_cache())?;
    let o1 = taxo_writer.add_category(&label(&["a"])?)?;
    let o2 = taxo_writer.add_category(&label(&["a"])?)?;
    Assert::assert_true_msg(
        "ordinal for same category that is added twice should be the same !",
        o1 == o2,
    );
    taxo_writer.close()?;
    dir.close()
}

fn last_gen(dir: &Arc<dyn Directory>) -> Result<i64> {
    SegmentInfos::get_last_commit_generation_dir(&**dir)
}

fn test_commit_no_empty_commits() -> Result<()> {
    // LUCENE-4972: DTW used to create empty commits even if no changes were made
    let dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let taxo_writer = DirectoryTaxonomyWriter::new(Arc::clone(&d))?;
    taxo_writer.add_category(&label(&["a"])?)?;
    taxo_writer.commit()?;

    let gen1 = last_gen(&d)?;
    taxo_writer.commit()?;
    let gen2 = last_gen(&d)?;
    Assert::assert_equals_msg(
        Some("empty commit should not have changed the index"),
        gen1,
        gen2,
    );

    taxo_writer.close()?;
    dir.close()
}

fn test_close_no_empty_commits() -> Result<()> {
    // LUCENE-4972: DTW used to create empty commits even if no changes were made
    let dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let taxo_writer = DirectoryTaxonomyWriter::new(Arc::clone(&d))?;
    taxo_writer.add_category(&label(&["a"])?)?;
    taxo_writer.commit()?;

    let gen1 = last_gen(&d)?;
    taxo_writer.close()?;
    let gen2 = last_gen(&d)?;
    Assert::assert_equals_msg(
        Some("empty commit should not have changed the index"),
        gen1,
        gen2,
    );

    taxo_writer.close()?;
    dir.close()
}

fn test_prepare_commit_no_empty_commits() -> Result<()> {
    // LUCENE-4972: DTW used to create empty commits even if no changes were made
    let dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let taxo_writer = DirectoryTaxonomyWriter::new(Arc::clone(&d))?;
    taxo_writer.add_category(&label(&["a"])?)?;
    taxo_writer.prepare_commit()?;
    taxo_writer.commit()?;

    let gen1 = last_gen(&d)?;
    taxo_writer.prepare_commit()?;
    taxo_writer.commit()?;
    let gen2 = last_gen(&d)?;
    Assert::assert_equals_msg(
        Some("empty commit should not have changed the index"),
        gen1,
        gen2,
    );

    taxo_writer.close()?;
    dir.close()
}

fn test_huge_label() -> Result<()> {
    let random = LuceneTestCase::random();
    let index_dir = LuceneTestCase::new_directory();
    let taxo_dir = LuceneTestCase::new_directory();
    let index_writer = IndexWriter::new(
        index_dir.as_directory(),
        LuceneTestCase::new_index_writer_config(),
    )?;
    let taxo_writer =
        DirectoryTaxonomyWriter::with_open_mode(taxo_dir.as_directory(), OpenMode::CREATE)?;
    let config = FacetsConfig::new();

    // Add one huge label:
    let len = FacetLabel::MAX_CATEGORY_PATH_LENGTH as i32 - 4; // for the dimension and separator
    let bigs = TestUtil::random_simple_string_between(&*random, len, len);
    let ff = FacetField::new("dim", &[&bigs])?;
    let cp = label(&["dim", &bigs])?;
    let ordinal = taxo_writer.add_category(&cp)?;
    let mut doc = Document::new();
    doc.add(ff);
    index_writer.add_document(&config.build_with_taxonomy(Some(&taxo_writer), &doc)?)?;

    // Add tiny ones to cause a re-hash
    for _ in 0..3 {
        let s = TestUtil::random_simple_string_between(&*random, 1, 10);
        taxo_writer.add_category(&label(&["dim", &s])?)?;
        let mut doc = Document::new();
        doc.add(FacetField::new("dim", &[&s])?);
        index_writer.add_document(&config.build_with_taxonomy(Some(&taxo_writer), &doc)?)?;
    }

    // when too large components were allowed to be added, this resulted in a new added category
    Assert::assert_equals(ordinal, taxo_writer.add_category(&cp)?);

    index_writer.close()?;
    taxo_writer.close()?;

    let index_reader = directory_reader::open(index_dir.as_directory())?;
    let taxo_reader = DirectoryTaxonomyReader::open(taxo_dir.as_directory())?;
    let searcher =
        IndexSearcher::new(Arc::clone(&index_reader) as Arc<dyn rucene::index::IndexReader>)?;
    let mut ddq = DrillDownQuery::new(Arc::new(FacetsConfig::new()));
    ddq.add("dim", &[&bigs])?;
    Assert::assert_equals(1i64, searcher.search(Arc::new(ddq), 10)?.total_hits.value());

    index_reader.close()?;
    taxo_reader.close()?;
    index_dir.close()?;
    taxo_dir.close()
}

fn test_replace_taxo_with_large_taxonomy() -> Result<()> {
    let src_taxo_dir = LuceneTestCase::new_directory();
    let target_taxo_dir = LuceneTestCase::new_directory();

    // build source, large, taxonomy
    let taxo_writer = DirectoryTaxonomyWriter::new(src_taxo_dir.as_directory())?;
    let ord = taxo_writer.add_category(&label(&["A", "1", "1", "1", "1", "1", "1"])?)?;
    taxo_writer.close()?;

    let taxo_writer = DirectoryTaxonomyWriter::new(target_taxo_dir.as_directory())?;
    let ordinal = taxo_writer.add_category(&label(&["B", "1"])?)?;
    Assert::assert_equals(1, taxo_writer.get_parent(ordinal)?); // call getParent to initialize taxoArrays
    taxo_writer.commit()?;

    taxo_writer.replace_taxonomy(src_taxo_dir.as_directory())?;
    Assert::assert_equals(ord - 1, taxo_writer.get_parent(ord)?);
    taxo_writer.close()?;

    src_taxo_dir.close()?;
    target_taxo_dir.close()
}

/// Regression for rmp #659: Lucene's `IndexWriter implements TwoPhaseCommit`
/// (`IndexWriter.java:198-199`), so `TwoPhaseCommitTool.execute(indexWriter,
/// taxoWriter)` runs the two-phase commit over an index writer and a taxonomy
/// writer together, and both commit.
#[test]
fn two_phase_commit_tool_commits_an_index_writer_and_a_taxonomy_writer() -> Result<()> {
    use rucene::index::execute_two_phase_commit;
    use rucene::store::ByteBuffersDirectory;

    let index_dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
    let taxo_dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
    let index_writer = IndexWriter::new(Arc::clone(&index_dir), IndexWriterConfig::new())?;
    let taxo_writer = DirectoryTaxonomyWriter::new(Arc::clone(&taxo_dir))?;
    index_writer.add_document(&Document::new())?;
    taxo_writer.add_category(&label(&["a"])?)?;
    Assert::assert_false(directory_reader::index_exists(&*index_dir)?);

    let objects: [Option<&dyn TwoPhaseCommit>; 2] = [Some(&*index_writer), Some(&taxo_writer)];
    execute_two_phase_commit(&objects)?;

    let index_reader = directory_reader::open(Arc::clone(&index_dir))?;
    Assert::assert_equals(1, index_reader.num_docs());
    index_reader.close()?;
    let taxo_reader = directory_reader::open(Arc::clone(&taxo_dir))?;
    // The root category and "a".
    Assert::assert_equals(2, taxo_reader.num_docs());
    taxo_reader.close()?;
    index_writer.close()?;
    taxo_writer.close()
}
