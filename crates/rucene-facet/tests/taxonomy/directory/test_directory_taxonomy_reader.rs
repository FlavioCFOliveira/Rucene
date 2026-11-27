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

//! Port of `org.apache.lucene.facet.taxonomy.directory.TestDirectoryTaxonomyReader`.

use std::collections::HashSet;
use std::sync::Arc;

use randomizedtesting::generators::RandomNumbers;
use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::jdk::thread::Thread;
use randomizedtesting::jdk::util::shuffle;
use randomizedtesting::junit::Assert;
use randomizedtesting::RandomizedTest;
use rucene::error::Result;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::{IndexWriterConfig, OpenMode};
use rucene::index::merge_policy::LogMergePolicy;
use rucene::index::TwoPhaseCommit;
use rucene::store::{ByteBuffersDirectory, Directory};
use rucene::util::Accountable;
use rucene_facet::taxonomy::directory::directory_taxonomy_reader::DirectoryTaxonomyReader;
use rucene_facet::taxonomy::directory::directory_taxonomy_writer::{
    DirectoryTaxonomyWriter, DirectoryTaxonomyWriterOverrides,
};
use rucene_facet::taxonomy::facet_label::FacetLabel;
use rucene_facet::taxonomy::taxonomy_reader::{
    self, TaxonomyReader, INVALID_ORDINAL, ROOT_ORDINAL,
};
use rucene_facet::taxonomy::taxonomy_writer::TaxonomyWriter;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::util::java_exceptions::{expect_throws, java_throwable};
use rucene_test_framework::util::LuceneTestCase;

use crate::facet_support::core_support::test;
use crate::facet_support::facet_class;

/// `TestDirectoryTaxonomyReader`.
pub fn class_def() -> Arc<Class> {
    facet_class(
        "org.apache.lucene.facet.taxonomy.directory.TestDirectoryTaxonomyReader",
        vec![
            test("testCloseAfterIncRef", test_close_after_inc_ref),
            test("testCloseTwice", test_close_twice),
            test("testOpenIfChangedResult", test_open_if_changed_result),
            test("testAlreadyClosed", test_already_closed),
            test(
                "testFreshReadRecreatedTaxonomy",
                test_fresh_read_recreated_taxonomy,
            ),
            test(
                "testOpenIfChangedReadRecreatedTaxonomy",
                test_open_if_changed_read_recreated_taxonomy,
            ),
            test(
                "testOpenIfChangedAndRefCount",
                test_open_if_changed_and_ref_count,
            ),
            test(
                "testOpenIfChangedManySegments",
                test_open_if_changed_many_segments,
            ),
            test(
                "testOpenIfChangedMergedSegment",
                test_open_if_changed_merged_segment,
            ),
            test(
                "testOpenIfChangedNoChangesButSegmentMerges",
                test_open_if_changed_no_changes_but_segment_merges,
            ),
            test(
                "testOpenIfChangedReuseAfterRecreate",
                test_open_if_changed_reuse_after_recreate,
            ),
            test("testOpenIfChangedReuse", test_open_if_changed_reuse),
            test(
                "testOpenIfChangedReplaceTaxonomy",
                test_open_if_changed_replace_taxonomy,
            ),
            test("testGetChildren", test_get_children),
            test("testAccountable", test_accountable),
            test(
                "testGetPathAndOrdinalsRandomMultithreading",
                test_get_path_and_ordinals_random_multithreading,
            ),
        ],
    )
}

fn label(components: &[&str]) -> Result<FacetLabel> {
    FacetLabel::new(components.iter().copied())
}

fn as_taxo(r: DirectoryTaxonomyReader) -> Arc<dyn TaxonomyReader> {
    Arc::new(r)
}

const ACE: &str = "org.apache.lucene.store.AlreadyClosedException";
const IAE: &str = "java.lang.IllegalArgumentException";

fn test_close_after_inc_ref() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let ltw = DirectoryTaxonomyWriter::new(dir.as_directory())?;
    ltw.add_category(&label(&["a"])?)?;
    ltw.close()?;

    let ltr = DirectoryTaxonomyReader::open(dir.as_directory())?;
    ltr.inc_ref()?;
    ltr.close()?;

    // should not fail as we incRef() before close
    ltr.get_size()?;
    ltr.dec_ref()?;

    dir.close()
}

fn test_close_twice() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let ltw = DirectoryTaxonomyWriter::new(dir.as_directory())?;
    ltw.add_category(&label(&["a"])?)?;
    ltw.close()?;

    let ltr = DirectoryTaxonomyReader::open(dir.as_directory())?;
    ltr.close()?;
    ltr.close()?; // no exception should be thrown
    dir.close()
}

fn test_open_if_changed_result() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let ltw = DirectoryTaxonomyWriter::new(dir.as_directory())?;
    let mut ltr: Option<DirectoryTaxonomyReader> = None;
    let body = (|| -> Result<()> {
        ltw.add_category(&label(&["a"])?)?;
        ltw.commit()?;

        let r = DirectoryTaxonomyReader::open(dir.as_directory())?;
        Assert::assert_true_msg("Nothing has changed", r.open_if_changed()?.is_none());
        let r = ltr.insert(r);

        ltw.add_category(&label(&["b"])?)?;
        ltw.commit()?;

        let newtr = r.open_if_changed()?;
        Assert::assert_true_msg("changes were committed", newtr.is_some());
        let newtr = newtr.unwrap();
        Assert::assert_true_msg("Nothing has changed", newtr.open_if_changed()?.is_none());
        newtr.close()
    })();
    // finally { IOUtils.close(ltw, ltr, dir); }
    let closed = ltw
        .close()
        .and(ltr.map_or(Ok(()), |r| r.close()))
        .and(dir.close());
    body.and(closed)
}

fn test_already_closed() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let ltw = DirectoryTaxonomyWriter::new(dir.as_directory())?;
    ltw.add_category(&label(&["a"])?)?;
    ltw.close()?;

    let ltr = DirectoryTaxonomyReader::open(dir.as_directory())?;
    ltr.close()?;
    expect_throws(ACE, || ltr.get_size());

    dir.close()
}

/// recreating a taxonomy should work well with a freshly opened taxonomy reader
fn test_fresh_read_recreated_taxonomy() -> Result<()> {
    do_test_read_recreated_taxonomy(true)
}

fn test_open_if_changed_read_recreated_taxonomy() -> Result<()> {
    do_test_read_recreated_taxonomy(false)
}

fn do_test_read_recreated_taxonomy(close_reader: bool) -> Result<()> {
    let random = LuceneTestCase::random();
    let n = 10;
    let cp: Vec<FacetLabel> = (0..n)
        .map(|i| label(&["a", &i.to_string()]))
        .collect::<Result<_>>()?;

    let dir = LuceneTestCase::new_directory();
    let mut tw: Option<DirectoryTaxonomyWriter> = None;
    let mut tr: Option<Arc<dyn TaxonomyReader>> = None;
    let body = (|| -> Result<()> {
        let w = DirectoryTaxonomyWriter::new(dir.as_directory())?;
        w.add_category(&label(&["a"])?)?;
        w.close()?;
        tw = Some(w);

        tr = Some(as_taxo(DirectoryTaxonomyReader::open(dir.as_directory())?));
        let base_num_categories = tr.as_ref().unwrap().get_size()?;

        for i in 0..n {
            let k = random.next_int_bound(n);
            let w = DirectoryTaxonomyWriter::with_open_mode(dir.as_directory(), OpenMode::CREATE)?;
            for c in cp.iter().take(k as usize + 1) {
                w.add_category(c)?;
            }
            w.close()?;
            tw = Some(w);
            let old = tr.take().unwrap();
            if close_reader {
                old.close()?;
                tr = Some(as_taxo(DirectoryTaxonomyReader::open(dir.as_directory())?));
            } else {
                let newtr = taxonomy_reader::open_if_changed(&old)?;
                Assert::assert_true(newtr.is_some());
                old.close()?;
                tr = newtr;
            }
            Assert::assert_equals_msg(
                Some(&format!("Wrong #categories in taxonomy (i={i}, k={k})")),
                base_num_categories + 1 + k,
                tr.as_ref().unwrap().get_size()?,
            );
        }
        Ok(())
    })();
    // finally { IOUtils.close(tr, tw, dir); }
    let closed = tr
        .map_or(Ok(()), |r| r.close())
        .and(tw.map_or(Ok(()), |w| w.close()))
        .and(dir.close());
    body.and(closed)
}

fn test_open_if_changed_and_ref_count() -> Result<()> {
    let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new()); // no need for random directories here

    let taxo_writer = DirectoryTaxonomyWriter::new(Arc::clone(&dir))?;
    taxo_writer.add_category(&label(&["a"])?)?;
    taxo_writer.commit()?;

    let taxo_reader = as_taxo(DirectoryTaxonomyReader::open(Arc::clone(&dir))?);
    Assert::assert_equals_msg(Some("wrong refCount"), 1, taxo_reader.get_ref_count());

    taxo_reader.inc_ref()?;
    Assert::assert_equals_msg(Some("wrong refCount"), 2, taxo_reader.get_ref_count());

    taxo_writer.add_category(&label(&["a", "b"])?)?;
    taxo_writer.commit()?;
    let newtr = taxonomy_reader::open_if_changed(&taxo_reader)?;
    Assert::assert_true(newtr.is_some());
    taxo_reader.close()?;
    let taxo_reader = newtr.unwrap();
    Assert::assert_equals_msg(Some("wrong refCount"), 1, taxo_reader.get_ref_count());

    taxo_writer.close()?;
    taxo_reader.close()?;
    dir.close()
}

/// The `DirectoryTaxonomyWriter` defaults, for an override's `super` call.
struct Defaults;
impl DirectoryTaxonomyWriterOverrides for Defaults {}

/// The anonymous writer of `testOpenIfChangedManySegments`: merge factor 2.
struct MergeFactor2;

impl DirectoryTaxonomyWriterOverrides for MergeFactor2 {
    fn create_index_writer_config(&self, open_mode: OpenMode) -> Result<IndexWriterConfig> {
        let conf = Defaults.create_index_writer_config(open_mode)?;
        let mp = conf.merge_policy();
        let lmp = mp
            .as_any()
            .and_then(|a| a.downcast_ref::<LogMergePolicy>())
            .expect("java.lang.ClassCastException: not a LogMergePolicy");
        lmp.set_merge_factor(2)?;
        Ok(conf)
    }
}

fn test_open_if_changed_many_segments() -> Result<()> {
    // test openIfChanged() when the taxonomy contains many segments
    let dir = LuceneTestCase::new_directory();

    let writer = Arc::new(DirectoryTaxonomyWriter::with_overrides(
        dir.as_directory(),
        OpenMode::CREATE_OR_APPEND,
        Some(DirectoryTaxonomyWriter::default_taxonomy_writer_cache()),
        Box::new(MergeFactor2),
    )?);
    let mut reader = as_taxo(DirectoryTaxonomyReader::from_writer(Arc::clone(&writer))?);

    let random = LuceneTestCase::random();
    let num_rounds = random.next_int_bound(10) + 10;
    let mut num_categories = 1; // one for root
    for i in 0..num_rounds {
        let num_cats = random.next_int_bound(4) + 1;
        for j in 0..num_cats {
            writer.add_category(&label(&[&i.to_string(), &j.to_string()])?)?;
        }
        num_categories += num_cats + 1 /* one for round-parent */;
        let newtr = taxonomy_reader::open_if_changed(&reader)?;
        Assert::assert_true(newtr.is_some());
        reader.close()?;
        reader = newtr.unwrap();

        // assert categories
        Assert::assert_equals(num_categories, reader.get_size()?);
        let round_ordinal = reader.get_ordinal(&label(&[&i.to_string()])?)?;
        let arrays = reader.get_parallel_taxonomy_arrays()?;
        let parents = arrays.parents();
        Assert::assert_equals(0, parents.get(round_ordinal)); // round's parent is root
        for j in 0..num_cats {
            let ord = reader.get_ordinal(&label(&[&i.to_string(), &j.to_string()])?)?;
            Assert::assert_equals(round_ordinal, parents.get(ord)); // round's parent is root
        }
    }

    reader.close()?;
    writer.close()?;
    dir.close()
}

/// The anonymous writer of the two merged-segment tests: `openIndexWriter`
/// returns the test's own `iw`.
struct SharedWriter(Arc<IndexWriter>);

impl DirectoryTaxonomyWriterOverrides for SharedWriter {
    fn open_index_writer(
        &self,
        _directory: Arc<dyn Directory>,
        _config: Arc<IndexWriterConfig>,
    ) -> Result<Arc<IndexWriter>> {
        Ok(Arc::clone(&self.0))
    }
}

fn shared_writer_setup(
    dir: &Arc<dyn Directory>,
) -> Result<(Arc<IndexWriter>, Arc<DirectoryTaxonomyWriter>)> {
    let config = IndexWriterConfig::with_analyzer(Some(Arc::new(MockAnalyzer::new(
        &*LuceneTestCase::random(),
    ))));
    config.set_merge_policy(Some(Arc::new(LogMergePolicy::by_byte_size())))?;
    let iw = IndexWriter::new(Arc::clone(dir), config)?;
    let writer = Arc::new(DirectoryTaxonomyWriter::with_overrides(
        Arc::clone(dir),
        OpenMode::CREATE_OR_APPEND,
        Some(DirectoryTaxonomyWriter::default_taxonomy_writer_cache()),
        Box::new(SharedWriter(Arc::clone(&iw))),
    )?);
    Ok((iw, writer))
}

fn test_open_if_changed_merged_segment() -> Result<()> {
    // test openIfChanged() when all index segments were merged - used to be
    // a bug in ParentArray, caught by testOpenIfChangedManySegments - only
    // this test is not random
    let dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();

    // hold onto IW to forceMerge
    // note how we don't close it, since DTW will close it.
    let (iw, writer) = shared_writer_setup(&d)?;

    let reader = as_taxo(DirectoryTaxonomyReader::from_writer(Arc::clone(&writer))?);
    Assert::assert_equals(1, reader.get_size()?);
    Assert::assert_equals(1, reader.get_parallel_taxonomy_arrays()?.parents().length());

    // add category and call forceMerge -- this should flush IW and merge segments down to 1
    // in ParentArray.initFromReader, this used to fail assuming there are no parents.
    writer.add_category(&label(&["1"])?)?;
    iw.force_merge(1)?;

    // now calling openIfChanged should trip on the bug
    let newtr = taxonomy_reader::open_if_changed(&reader)?;
    Assert::assert_true(newtr.is_some());
    reader.close()?;
    let reader = newtr.unwrap();
    Assert::assert_equals(2, reader.get_size()?);
    Assert::assert_equals(2, reader.get_parallel_taxonomy_arrays()?.parents().length());

    reader.close()?;
    writer.close()?;
    dir.close()
}

fn test_open_if_changed_no_changes_but_segment_merges() -> Result<()> {
    // test openIfChanged() when the taxonomy hasn't really changed, but segments
    // were merged. The NRT reader will be reopened, and ParentArray used to assert
    // that the new reader contains more ordinals than were given from the old
    // TaxReader version
    let dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();

    // hold onto IW to forceMerge
    // note how we don't close it, since DTW will close it.
    let (iw, writer) = shared_writer_setup(&d)?;

    // add a category so that the following DTR open will cause a flush and
    // a new segment will be created
    writer.add_category(&label(&["a"])?)?;

    let reader = as_taxo(DirectoryTaxonomyReader::from_writer(Arc::clone(&writer))?);
    Assert::assert_equals(2, reader.get_size()?);
    Assert::assert_equals(2, reader.get_parallel_taxonomy_arrays()?.parents().length());

    // merge all the segments so that NRT reader thinks there's a change
    iw.force_merge(1)?;

    // now calling openIfChanged should trip on the wrong assert in ParetArray's ctor
    let newtr = taxonomy_reader::open_if_changed(&reader)?;
    Assert::assert_true(newtr.is_some());
    reader.close()?;
    let reader = newtr.unwrap();
    Assert::assert_equals(2, reader.get_size()?);
    Assert::assert_equals(2, reader.get_parallel_taxonomy_arrays()?.parents().length());

    reader.close()?;
    writer.close()?;
    dir.close()
}

fn test_open_if_changed_reuse_after_recreate() -> Result<()> {
    // tests that if the taxonomy is recreated, no data is reused from the previous taxonomy
    let dir = LuceneTestCase::new_directory();
    let writer = DirectoryTaxonomyWriter::new(dir.as_directory())?;
    let cp_a = label(&["a"])?;
    writer.add_category(&cp_a)?;
    writer.close()?;

    let r1 = DirectoryTaxonomyReader::open(dir.as_directory())?;
    // fill r1's caches
    assert_paths_and_ordinals(&r1, &[1], &[Some(cp_a.clone())])?;

    // now recreate, add a different category
    let writer = DirectoryTaxonomyWriter::with_open_mode(dir.as_directory(), OpenMode::CREATE)?;
    let cp_b = label(&["b"])?;
    writer.add_category(&cp_b)?;
    writer.close()?;

    let r2 = r1.open_if_changed()?;
    Assert::assert_true(r2.is_some());
    let r2 = r2.unwrap();

    // fill r2's caches
    assert_paths_and_ordinals(&r2, &[1], &[Some(cp_b.clone())])?;

    // check r1 wasn't changed
    assert_getting_ordinals(&r1, &[1, INVALID_ORDINAL], &[cp_a.clone(), cp_b.clone()])?;
    assert_getting_paths(&r1, &[Some(cp_a.clone()), None], &[1, 2])?;

    // check r2 wasn't changed
    assert_getting_ordinals(&r2, &[INVALID_ORDINAL, 1], &[cp_a, cp_b.clone()])?;
    assert_getting_paths(&r2, &[Some(cp_b), None], &[1, 2])?;

    r2.close()?;
    r1.close()?;
    dir.close()
}

fn test_open_if_changed_reuse() -> Result<()> {
    // test the reuse of data from the old DTR instance
    for nrt in [false, true] {
        let dir = LuceneTestCase::new_directory();
        let writer = Arc::new(DirectoryTaxonomyWriter::new(dir.as_directory())?);

        let cp_a = label(&["a"])?;
        writer.add_category(&cp_a)?;
        if !nrt {
            writer.commit()?;
        }

        let r1 = if nrt {
            DirectoryTaxonomyReader::from_writer(Arc::clone(&writer))?
        } else {
            DirectoryTaxonomyReader::open(dir.as_directory())?
        };
        // fill r1's caches
        assert_paths_and_ordinals(&r1, &[1], &[Some(cp_a.clone())])?;

        let cp_b = label(&["b"])?;
        writer.add_category(&cp_b)?;
        if !nrt {
            writer.commit()?;
        }

        let r2 = r1.open_if_changed()?;
        Assert::assert_true(r2.is_some());
        let r2 = r2.unwrap();

        // add r2's categories to the caches
        assert_paths_and_ordinals(&r2, &[1, 2], &[Some(cp_a.clone()), Some(cp_b.clone())])?;

        // check that r1 doesn't see cp_b
        assert_getting_ordinals(&r1, &[1, INVALID_ORDINAL], &[cp_a.clone(), cp_b])?;
        assert_getting_paths(&r1, &[Some(cp_a), None], &[1, 2])?;

        r1.close()?;
        r2.close()?;
        writer.close()?;
        dir.close()?;
    }
    Ok(())
}

fn test_open_if_changed_replace_taxonomy() -> Result<()> {
    // test openIfChanged when replaceTaxonomy is called, which is equivalent to recreate
    // only can work with NRT as well
    let src = LuceneTestCase::new_directory();
    let w = DirectoryTaxonomyWriter::new(src.as_directory())?;
    let cp_b = label(&["b"])?;
    w.add_category(&cp_b)?;
    w.close()?;

    for nrt in [false, true] {
        let dir = LuceneTestCase::new_directory();
        let writer = Arc::new(DirectoryTaxonomyWriter::new(dir.as_directory())?);

        let cp_a = label(&["a"])?;
        writer.add_category(&cp_a)?;
        if !nrt {
            writer.commit()?;
        }

        let r1 = if nrt {
            DirectoryTaxonomyReader::from_writer(Arc::clone(&writer))?
        } else {
            DirectoryTaxonomyReader::open(dir.as_directory())?
        };
        // fill r1's caches
        assert_paths_and_ordinals(&r1, &[1], &[Some(cp_a.clone())])?;

        // now replace taxonomy
        writer.replace_taxonomy(src.as_directory())?;
        if !nrt {
            writer.commit()?;
        }

        let r2 = r1.open_if_changed()?;
        Assert::assert_true(r2.is_some());
        let r2 = r2.unwrap();

        // fill r2's caches
        assert_paths_and_ordinals(&r2, &[1], &[Some(cp_b.clone())])?;

        // check r1 wasn't changed
        assert_getting_ordinals(&r1, &[1, INVALID_ORDINAL], &[cp_a.clone(), cp_b.clone()])?;
        assert_getting_paths(&r1, &[Some(cp_a.clone()), None], &[1, 2])?;

        // check r2 wasn't changed
        assert_getting_ordinals(&r2, &[INVALID_ORDINAL, 1], &[cp_a, cp_b.clone()])?;
        assert_getting_paths(&r2, &[Some(cp_b.clone()), None], &[1, 2])?;

        r2.close()?;
        r1.close()?;
        writer.close()?;
        dir.close()?;
    }

    src.close()
}

/// `IntStream.range(0, n)` shuffled by `Collections.shuffle(list, random())`.
fn shuffled_indexes(n: usize) -> Vec<usize> {
    let mut indexes: Vec<usize> = (0..n).collect();
    shuffle(&mut indexes, &*LuceneTestCase::random());
    indexes
}

fn assert_getting_ordinals(
    reader: &DirectoryTaxonomyReader,
    expected_ords: &[i32],
    source_paths: &[FacetLabel],
) -> Result<()> {
    // To exercise mix of cache hit and cache misses for getOrdinal and getBulkOrdinals this method:
    // 1. Randomly gets a few ords using sequential calls.
    // 2. Call bulk get method.
    // 3. Continue sequential calls for the remaining items.
    Assert::assert_equals(expected_ords.len(), source_paths.len());
    let bulk_operations_iteration =
        LuceneTestCase::random().next_int_bound(source_paths.len() as i32) as usize;
    let indexes_shuffled = shuffled_indexes(source_paths.len());

    for &next_index in indexes_shuffled.iter().take(bulk_operations_iteration) {
        Assert::assert_equals(
            expected_ords[next_index],
            reader.get_ordinal(&source_paths[next_index])?,
        );
    }

    let bulk_ord_result = reader.get_bulk_ordinals(source_paths)?;
    Assert::assert_true_msg(
        &format!("arrays differ: {expected_ords:?} vs {bulk_ord_result:?}"),
        expected_ords == bulk_ord_result.as_slice(),
    );

    for &next_index in indexes_shuffled.iter().skip(bulk_operations_iteration) {
        Assert::assert_equals(
            expected_ords[next_index],
            reader.get_ordinal(&source_paths[next_index])?,
        );
    }
    Ok(())
}

/// `expected_paths`: `None` is `ILLEGAL_PATH` (Java compares it by
/// identity).
fn assert_getting_paths(
    reader: &DirectoryTaxonomyReader,
    expected_paths: &[Option<FacetLabel>],
    source_ords: &[i32],
) -> Result<()> {
    // To exercise mix of cache hit and cache misses for getPath and getBulkPath this method:
    // 1. Randomly gets a few paths using sequential calls.
    // 2. Call bulk get method.
    // 3. Continue sequential calls for the remaining items.
    // Note: expectedPaths should refer to ILLEGAL_PATH for ords from sourceOrds that are expected
    // to throw IllegalArgumentException
    Assert::assert_equals(expected_paths.len(), source_ords.len());
    let bulk_operations_iteration =
        LuceneTestCase::random().next_int_bound(source_ords.len() as i32) as usize;
    let indexes_shuffled = shuffled_indexes(source_ords.len());

    let check_one = |next_index: usize| -> Result<()> {
        match &expected_paths[next_index] {
            None => {
                expect_throws(IAE, || reader.get_path(source_ords[next_index]));
            }
            Some(expected) => {
                let actual = reader.get_path(source_ords[next_index])?;
                Assert::assert_true_msg(
                    &format!("expected:<{expected}> but was:<{actual:?}>"),
                    actual.as_ref() == Some(expected),
                );
            }
        }
        Ok(())
    };

    let illegal_path_exception_is_expected = expected_paths.iter().any(Option::is_none);
    for &next_index in indexes_shuffled.iter().take(bulk_operations_iteration) {
        check_one(next_index)?;
    }

    if illegal_path_exception_is_expected {
        expect_throws(IAE, || reader.get_bulk_path(&mut source_ords.to_vec()));
    } else {
        // clone because getBulkPath changes the order of the ords
        let mut source_ords_copy = source_ords.to_vec();
        let bulk_paths_result = reader.get_bulk_path(&mut source_ords_copy)?;
        Assert::assert_true_msg(
            &format!("arrays differ: {expected_paths:?} vs {bulk_paths_result:?}"),
            expected_paths == bulk_paths_result.as_slice(),
        );
    }

    for &next_index in indexes_shuffled.iter().skip(bulk_operations_iteration) {
        check_one(next_index)?;
    }
    Ok(())
}

fn assert_paths_and_ordinals(
    reader: &DirectoryTaxonomyReader,
    ords: &[i32],
    paths: &[Option<FacetLabel>],
) -> Result<()> {
    // use this method to fill DTR caches
    assert_getting_paths(reader, paths, ords)?;
    let labels: Vec<FacetLabel> = paths
        .iter()
        .map(|p| p.clone().expect("a legal path"))
        .collect();
    assert_getting_ordinals(reader, ords, &labels)
}

fn test_get_children() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let taxo_writer = DirectoryTaxonomyWriter::new(dir.as_directory())?;
    let num_categories = LuceneTestCase::at_least(10);
    let mut num_a = 0;
    let mut num_b = 0;
    let random = LuceneTestCase::random();
    // add the two categories for which we'll also add children (so asserts are simpler)
    taxo_writer.add_category(&label(&["a"])?)?;
    taxo_writer.add_category(&label(&["b"])?)?;
    for i in 0..num_categories {
        if random.next_boolean() {
            taxo_writer.add_category(&label(&["a", &i.to_string()])?)?;
            num_a += 1;
        } else {
            taxo_writer.add_category(&label(&["b", &i.to_string()])?)?;
            num_b += 1;
        }
    }
    // add category with no children
    taxo_writer.add_category(&label(&["c"])?)?;
    taxo_writer.close()?;

    let taxo_reader = DirectoryTaxonomyReader::open(dir.as_directory())?;

    // non existing category
    let mut it = taxo_reader.get_children(taxo_reader.get_ordinal(&label(&["invalid"])?)?)?;
    Assert::assert_equals(INVALID_ORDINAL, it.next());

    // a category with no children
    let mut it = taxo_reader.get_children(taxo_reader.get_ordinal(&label(&["c"])?)?)?;
    Assert::assert_equals(INVALID_ORDINAL, it.next());

    // arbitrary negative ordinal
    let mut it = taxo_reader.get_children(-2)?;
    Assert::assert_equals(INVALID_ORDINAL, it.next());

    // root's children
    let mut roots: HashSet<String> = ["a", "b", "c"].iter().map(|s| s.to_string()).collect();
    let mut it = taxo_reader.get_children(ROOT_ORDINAL)?;
    while !roots.is_empty() {
        let root = taxo_reader.get_path(it.next())?.unwrap();
        Assert::assert_equals(1usize, root.length);
        Assert::assert_true(roots.remove(&root.components[0]));
    }
    Assert::assert_equals(INVALID_ORDINAL, it.next());

    for i in 0..2 {
        let cp = if i == 0 {
            label(&["a"])?
        } else {
            label(&["b"])?
        };
        let ordinal = taxo_reader.get_ordinal(&cp)?;
        let mut it = taxo_reader.get_children(ordinal)?;
        let mut num_children = 0;
        loop {
            let child = it.next();
            if child == INVALID_ORDINAL {
                break;
            }
            let path = taxo_reader.get_path(child)?.unwrap();
            Assert::assert_equals(2usize, path.length);
            Assert::assert_equals(path.components[0].as_str(), if i == 0 { "a" } else { "b" });
            num_children += 1;
        }
        let expected = if i == 0 { num_a } else { num_b };
        Assert::assert_equals_msg(Some("invalid num children"), expected, num_children);
    }
    taxo_reader.close()?;

    dir.close()
}

fn test_accountable() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let taxo_writer = DirectoryTaxonomyWriter::new(dir.as_directory())?;
    let num_categories = LuceneTestCase::at_least(10);
    let random = LuceneTestCase::random();
    // add the two categories for which we'll also add children (so asserts are simpler)
    taxo_writer.add_category(&label(&["a"])?)?;
    taxo_writer.add_category(&label(&["b"])?)?;
    for i in 0..num_categories {
        if random.next_boolean() {
            taxo_writer.add_category(&label(&["a", &i.to_string()])?)?;
        } else {
            taxo_writer.add_category(&label(&["b", &i.to_string()])?)?;
        }
    }
    // add category with no children
    taxo_writer.add_category(&label(&["c"])?)?;
    taxo_writer.close()?;

    let taxo_reader = DirectoryTaxonomyReader::open(dir.as_directory())?;
    taxo_reader.get_parallel_taxonomy_arrays()?; // increases memory usage as a side-effect
    Assert::assert_true(taxo_reader.ram_bytes_used() > 0);
    Assert::assert_true(!taxo_reader.child_resources().is_empty());
    taxo_reader.close()?;
    dir.close()
}

fn test_get_path_and_ordinals_random_multithreading() -> Result<()> {
    let src = LuceneTestCase::new_directory();
    let w = DirectoryTaxonomyWriter::new(src.as_directory())?;
    let max_number_of_labels_to_index = 1000;
    let max_number_of_unique_labels_to_index = max_number_of_labels_to_index / 2;
    let cache_size = max_number_of_unique_labels_to_index / 2; // to cause some cache evictions
    let len = RandomizedTest::random_int_between(1, max_number_of_labels_to_index) as usize;
    let random = LuceneTestCase::random();
    // add random labels to the taxonomy
    let random_array: Vec<String> = (0..len)
        .map(|_| {
            random
                .next_int_bound(max_number_of_unique_labels_to_index)
                .to_string()
        })
        .collect();

    let mut all_paths = Vec::with_capacity(len);
    for s in &random_array {
        let path = label(&[s])?;
        w.add_category(&path)?;
        all_paths.push(path);
        if random.next_boolean() {
            w.commit()?;
        }
    }
    w.commit()?;
    w.close()?;

    let r1 = Arc::new(DirectoryTaxonomyReader::open(src.as_directory())?);
    r1.set_cache_size(cache_size as usize)?;

    let all_ordinals = Arc::new(r1.get_bulk_ordinals(&all_paths)?);
    let all_paths = Arc::new(all_paths);

    // Assert getPath and getBulkPath first, then assert getOrdinal and getBulkOrdinals.
    // Create multiple threads to check result correctness and thread contention in the cache.
    for assert_getting_ordinals_flag in [false, true] {
        let num_threads = RandomNumbers::random_int_between(&*LuceneTestCase::random(), 1, 12);
        let mut add_threads = Vec::new();
        for _ in 0..num_threads {
            let (r1, all_ordinals, all_paths) = (
                Arc::clone(&r1),
                Arc::clone(&all_ordinals),
                Arc::clone(&all_paths),
            );
            add_threads.push(Thread::unnamed(move || {
                let random = LuceneTestCase::random();
                let num_thread_iterations = random.next_int_bound(10);
                for _ in 0..num_thread_iterations {
                    // length of the FacetLabel array that we are going to check
                    let num_of_ordinals_to_check =
                        RandomizedTest::random_int_between(1, all_ordinals.len() as i32) as usize;
                    let mut ordinals = Vec::with_capacity(num_of_ordinals_to_check);
                    let mut path = Vec::with_capacity(num_of_ordinals_to_check);

                    for _ in 0..num_of_ordinals_to_check {
                        // we deliberately allow it to choose repeat indexes as this will exercise
                        // the cache
                        let ordinal_index =
                            random.next_int_bound(all_ordinals.len() as i32) as usize;
                        ordinals.push(all_ordinals[ordinal_index]);
                        path.push(all_paths[ordinal_index].clone());
                    }

                    // main check for correctness is done in this method
                    let outcome = if assert_getting_ordinals_flag {
                        assert_getting_ordinals(&r1, &ordinals, &path)
                    } else {
                        let expected: Vec<Option<FacetLabel>> =
                            path.into_iter().map(Some).collect();
                        assert_getting_paths(&r1, &expected, &ordinals)
                    };
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
    }

    r1.close()?;
    src.close()
}
