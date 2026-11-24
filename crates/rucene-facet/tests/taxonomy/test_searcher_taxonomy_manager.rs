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

//! Port of `org.apache.lucene.facet.taxonomy.TestSearcherTaxonomyManager`.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::jdk::thread::Thread;
use randomizedtesting::junit::Assert;
use rucene::document::{Document, Store};
use rucene::error::{LuceneError, Result};
use rucene::index::directory_reader::{self, DirectoryReader};
use rucene::index::index_commit::IndexCommit;
use rucene::index::index_deletion_policy::NoDeletionPolicy;
use rucene::index::index_writer::IndexWriter;
use rucene::index::merge_policy::TieredMergePolicy;
use rucene::index::segment_infos::SegmentInfos;
use rucene::index::TwoPhaseCommit;
use rucene::search::refresh_commit_supplier::RefreshCommitSupplier;
use rucene::search::MatchAllDocsQuery;
use rucene_facet::facet_field::FacetField;
use rucene_facet::facets_collector_manager::FacetsCollectorManager;
use rucene_facet::facets_config::FacetsConfig;
use rucene_facet::taxonomy::directory::directory_taxonomy_reader::DirectoryTaxonomyReader;
use rucene_facet::taxonomy::directory::directory_taxonomy_writer::DirectoryTaxonomyWriter;
use rucene_facet::taxonomy::facet_label::FacetLabel;
use rucene_facet::taxonomy::searcher_taxonomy_manager::SearcherTaxonomyManager;
use rucene_facet::taxonomy::taxonomy_reader::TaxonomyReader;
use rucene_facet::taxonomy::taxonomy_writer::TaxonomyWriter;
use rucene_test_framework::util::java_exceptions::{expect_throws, java_throwable};
use rucene_test_framework::util::lucene_test_case::{SuppressCodecs, TEST_NIGHTLY, VERBOSE};
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::facet_support::core_support::test;
use crate::facet_support::{facet_class_annotated, get_taxonomy_facet_counts};

/// `TestSearcherTaxonomyManager`.
pub fn class_def() -> Arc<Class> {
    facet_class_annotated(
        "org.apache.lucene.facet.taxonomy.TestSearcherTaxonomyManager",
        vec![
            test("testNRT", test_nrt),
            test("testDirectory", test_directory),
            test("testReplaceTaxonomyNRT", test_replace_taxonomy_nrt),
            test(
                "testReplaceTaxonomyDirectory",
                test_replace_taxonomy_directory,
            ),
            test("testExceptionDuringRefresh", test_exception_during_refresh),
            test("testStepWiseCommitRefresh", test_step_wise_commit_refresh),
        ],
        |b| {
            b.annotate(SuppressCodecs {
                value: vec!["SimpleText".to_string()],
            })
        },
    )
}

/// `catch (IOException ioe) { throw new RuntimeException(ioe); }` (or
/// `catch (Exception ioe)` for the reopener).
fn rethrow_runtime(e: LuceneError, any_exception: bool) -> ! {
    if any_exception || e.is_io_exception() {
        Throwable::from_cause("java.lang.RuntimeException", java_throwable(&e)).throw()
    }
    java_throwable(&e).throw()
}

/// `private static class IndexerThread extends Thread`.
fn indexer_thread(
    w: Arc<IndexWriter>,
    config: Arc<FacetsConfig>,
    tw: Arc<DirectoryTaxonomyWriter>,
    mgr: Option<Arc<SearcherTaxonomyManager>>,
    ord_limit: i32,
    stop: Arc<AtomicBool>,
) -> impl FnOnce() + Send + 'static {
    move || {
        // `finally { stop.set(true); }`
        struct SetOnDrop(Arc<AtomicBool>);
        impl Drop for SetOnDrop {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }
        let _finally = SetOnDrop(Arc::clone(&stop));

        let random = LuceneTestCase::random();
        let mut seen: HashSet<String> = HashSet::new();
        let mut paths: Vec<String> = Vec::new();
        loop {
            let mut doc = Document::new();
            let num_paths = TestUtil::next_int(&*random, 1, 5);
            for _ in 0..num_paths {
                let path = if !paths.is_empty() && random.next_int_bound(5) != 4 {
                    // Use previous path
                    paths[random.next_int_bound(paths.len() as i32) as usize].clone()
                } else {
                    // Create new path
                    loop {
                        let path = TestUtil::random_realistic_unicode_string(&*random);
                        if !path.is_empty() && !seen.contains(&path) {
                            seen.insert(path.clone());
                            paths.push(path.clone());
                            break path;
                        }
                    }
                };
                match FacetField::new("field", &[&path]) {
                    Ok(ff) => doc.add(ff),
                    Err(e) => rethrow_runtime(e, false),
                }
            }
            let outcome = (|| -> Result<()> {
                w.add_document(&config.build_with_taxonomy(Some(&*tw), &doc)?)?;
                if let Some(mgr) = &mgr {
                    if random.next_double() < 0.02 {
                        w.commit()?;
                        tw.commit()?;
                        mgr.maybe_refresh()?;
                    }
                }
                Ok(())
            })();
            if let Err(e) = outcome {
                rethrow_runtime(e, false);
            }

            let size = tw.get_size().unwrap_or_else(|e| java_throwable(&e).throw());
            if *VERBOSE {
                println!("TW size={size} vs {ord_limit}");
            }

            if size >= ord_limit {
                break;
            }
        }
    }
}

fn join(t: &Thread) {
    if let Err(payload) = t.join() {
        std::panic::resume_unwind(payload);
    }
}

/// The search loop shared by `testNRT` and `testDirectory`.
fn search_until_stopped(
    mgr: &SearcherTaxonomyManager,
    config: &Arc<FacetsConfig>,
    stop: &AtomicBool,
) -> Result<()> {
    while !stop.load(Ordering::SeqCst) {
        let pair = mgr.acquire()?;
        let outcome = (|| -> Result<()> {
            let sfc = pair.searcher().search_with_manager(
                Arc::new(MatchAllDocsQuery::instance()),
                &FacetsCollectorManager::new(),
            )?;
            let taxo_reader: Arc<dyn TaxonomyReader> =
                Arc::clone(pair.taxonomy_reader()) as Arc<dyn TaxonomyReader>;
            let facets = get_taxonomy_facet_counts(&taxo_reader, config, &sfc)?;
            let result = facets.get_top_children(10, "field", &[])?;
            if pair.searcher().get_index_reader().num_docs() > 0 {
                let result = result.unwrap();
                Assert::assert_true(result.child_count > 0);
                Assert::assert_true(!result.label_values.is_empty());
            }
            Ok(())
        })();
        mgr.release(pair)?;
        outcome?;
    }
    Ok(())
}

fn test_nrt() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let taxo_dir = LuceneTestCase::new_directory();
    let iwc = LuceneTestCase::new_index_writer_config();
    // Don't allow tiny maxBufferedDocs; it can make this
    // test too slow:
    iwc.set_max_buffered_docs(500.max(iwc.max_buffered_docs()))?;

    // MockRandom/AlcololicMergePolicy are too slow:
    let tmp = TieredMergePolicy::new();
    tmp.set_floor_segment_mb(0.001)?;
    iwc.set_merge_policy(Some(Arc::new(tmp)))?;
    let w = IndexWriter::new(dir.as_directory(), iwc)?;
    let tw = Arc::new(DirectoryTaxonomyWriter::new(taxo_dir.as_directory())?);
    let config = Arc::new(FacetsConfig::new());
    config.set_multi_valued("field", true);
    let stop = Arc::new(AtomicBool::new(false));

    // How many unique facets to index before stopping:
    let ord_limit = if *TEST_NIGHTLY { 100000 } else { 6000 };

    let indexer = Thread::new(
        "indexer",
        indexer_thread(
            Arc::clone(&w),
            Arc::clone(&config),
            Arc::clone(&tw),
            None,
            ord_limit,
            Arc::clone(&stop),
        ),
    );

    let mgr = Arc::new(SearcherTaxonomyManager::from_writers_with_deletes(
        &w,
        true,
        None,
        Arc::clone(&tw),
    )?);

    let reopener = {
        let (stop, mgr) = (Arc::clone(&stop), Arc::clone(&mgr));
        Thread::new("reopener", move || {
            let random = LuceneTestCase::random();
            while !stop.load(Ordering::SeqCst) {
                // Sleep for up to 20 msec:
                std::thread::sleep(std::time::Duration::from_millis(
                    random.next_int_bound(20) as u64
                ));

                if *VERBOSE {
                    println!("TEST: reopen");
                }

                if let Err(e) = mgr.maybe_refresh() {
                    rethrow_runtime(e, true);
                }

                if *VERBOSE {
                    println!("TEST: reopen done");
                }
            }
        })
    };

    reopener.start();
    indexer.start();

    let searched = search_until_stopped(&mgr, &config, &stop);
    // finally { indexer.join(); reopener.join(); }
    join(&indexer);
    join(&reopener);
    searched?;

    if *VERBOSE {
        println!("TEST: now stop");
    }

    w.close()?;
    mgr.close()?;
    tw.close()?;
    taxo_dir.close()?;
    dir.close()
}

fn test_directory() -> Result<()> {
    let index_dir = LuceneTestCase::new_directory();
    let taxo_dir = LuceneTestCase::new_directory();
    let w = IndexWriter::new(
        index_dir.as_directory(),
        LuceneTestCase::new_index_writer_config(),
    )?;
    let tw = Arc::new(DirectoryTaxonomyWriter::new(taxo_dir.as_directory())?);
    // first empty commit
    w.commit()?;
    tw.commit()?;
    let mgr = Arc::new(SearcherTaxonomyManager::from_directories(
        index_dir.as_directory(),
        taxo_dir.as_directory(),
        None,
    )?);
    let config = Arc::new(FacetsConfig::new());
    config.set_multi_valued("field", true);
    let stop = Arc::new(AtomicBool::new(false));

    // How many unique facets to index before stopping:
    let ord_limit = if *TEST_NIGHTLY { 100000 } else { 600 };

    let indexer = Thread::unnamed(indexer_thread(
        Arc::clone(&w),
        Arc::clone(&config),
        Arc::clone(&tw),
        Some(Arc::clone(&mgr)),
        ord_limit,
        Arc::clone(&stop),
    ));
    indexer.start();

    let searched = search_until_stopped(&mgr, &config, &stop);
    // finally { indexer.join(); }
    join(&indexer);
    searched?;

    if *VERBOSE {
        println!("TEST: now stop");
    }

    w.close()?;
    mgr.close()?;
    tw.close()?;
    taxo_dir.close()?;
    index_dir.close()
}

fn test_replace_taxonomy_nrt() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let taxo_dir = LuceneTestCase::new_directory();
    let w = IndexWriter::new(
        dir.as_directory(),
        LuceneTestCase::new_index_writer_config(),
    )?;
    let tw = Arc::new(DirectoryTaxonomyWriter::new(taxo_dir.as_directory())?);

    let taxo_dir2 = LuceneTestCase::new_directory();
    let tw2 = DirectoryTaxonomyWriter::new(taxo_dir2.as_directory())?;
    tw2.close()?;

    let mgr = SearcherTaxonomyManager::from_writers_with_deletes(&w, true, None, Arc::clone(&tw))?;
    w.add_document(&Document::new())?;

    tw.replace_taxonomy(taxo_dir2.as_directory())?;
    taxo_dir2.close()?;

    expect_throws("java.lang.IllegalStateException", || mgr.maybe_refresh());

    w.close()?;
    mgr.close()?;
    tw.close()?;
    taxo_dir.close()?;
    dir.close()
}

fn test_replace_taxonomy_directory() -> Result<()> {
    let index_dir = LuceneTestCase::new_directory();
    let taxo_dir = LuceneTestCase::new_directory();
    let w = IndexWriter::new(
        index_dir.as_directory(),
        LuceneTestCase::new_index_writer_config(),
    )?;
    let tw = DirectoryTaxonomyWriter::new(taxo_dir.as_directory())?;
    w.commit()?;
    tw.commit()?;

    let taxo_dir2 = LuceneTestCase::new_directory();
    let tw2 = DirectoryTaxonomyWriter::new(taxo_dir2.as_directory())?;
    tw2.add_category(&FacetLabel::new(["a", "b"])?)?;
    tw2.close()?;

    let mgr = SearcherTaxonomyManager::from_directories(
        index_dir.as_directory(),
        taxo_dir.as_directory(),
        None,
    )?;
    let pair = mgr.acquire()?;
    let size = pair.taxonomy_reader().get_size();
    mgr.release(pair)?;
    Assert::assert_equals(1, size?);

    w.add_document(&Document::new())?;
    tw.replace_taxonomy(taxo_dir2.as_directory())?;
    taxo_dir2.close()?;
    w.commit()?;
    tw.commit()?;

    mgr.maybe_refresh()?;
    let pair = mgr.acquire()?;
    let size = pair.taxonomy_reader().get_size();
    mgr.release(pair)?;
    Assert::assert_equals(3, size?);

    w.close()?;
    mgr.close()?;
    tw.close()?;
    taxo_dir.close()?;
    index_dir.close()
}

fn test_exception_during_refresh() -> Result<()> {
    let index_dir = LuceneTestCase::new_directory();
    let taxo_dir = LuceneTestCase::new_directory();
    let w = IndexWriter::new(
        index_dir.as_directory(),
        LuceneTestCase::new_index_writer_config(),
    )?;
    let tw = DirectoryTaxonomyWriter::new(taxo_dir.as_directory())?;
    w.commit()?;
    tw.commit()?;

    let mgr = SearcherTaxonomyManager::from_directories(
        index_dir.as_directory(),
        taxo_dir.as_directory(),
        None,
    )?;

    tw.add_category(&FacetLabel::new(["a", "b"])?)?;
    w.add_document(&Document::new())?;

    tw.commit()?;
    w.commit()?;

    // intentionally corrupt the taxo index:
    let infos = SegmentInfos::read_latest_commit(taxo_dir.as_directory())?;
    taxo_dir.as_directory().delete_file(
        &infos
            .segments_file_name()
            .expect("a committed segments file"),
    )?;
    expect_throws("org.apache.lucene.index.IndexNotFoundException", || {
        mgr.maybe_refresh_blocking()
    });

    w.close()?;
    tw.close()?;
    mgr.close()?;
    index_dir.close()?;
    taxo_dir.close()
}

/// Returns the first commit with generation higher than current reader commit
#[derive(Debug)]
pub struct NextCommitSelector;

impl RefreshCommitSupplier for NextCommitSelector {
    fn get_searcher_refresh_commit(
        &self,
        reader: &Arc<dyn DirectoryReader>,
    ) -> Result<Option<Arc<dyn IndexCommit>>> {
        let commits = directory_reader::list_commits(reader.directory())?;
        let current = reader.index_commit()?;
        for commit in commits {
            if commit.get_generation() > current.get_generation() {
                return Ok(Some(commit));
            }
        }
        // we're already on latest commit
        Ok(None)
    }
}

fn test_step_wise_commit_refresh() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let taxo_dir = LuceneTestCase::new_directory();
    let iwc = LuceneTestCase::new_index_writer_config();
    iwc.set_index_deletion_policy(Some(NoDeletionPolicy::instance()))?;
    let w = IndexWriter::new(dir.as_directory(), iwc)?;
    let tw = DirectoryTaxonomyWriter::new(taxo_dir.as_directory())?;

    let mut doc_id = 0;
    // create initial commit
    for _ in 0..20 {
        let mut doc = Document::new();
        doc.add(LuceneTestCase::new_string_field(
            "docId",
            &format!("doc-{doc_id}"),
            Store::YES,
        ));
        doc_id += 1;
        w.add_document(&doc)?;
    }
    w.commit()?;
    tw.commit()?;

    let mut color_index = 0;
    let colors = ["red", "green", "blue", "yellow"];
    let config = FacetsConfig::new();
    let taxo_reader = Arc::new(DirectoryTaxonomyReader::open(taxo_dir.as_directory())?);
    let sat = SearcherTaxonomyManager::from_readers_with_commit_supplier(
        directory_reader::open(dir.as_directory())?,
        taxo_reader,
        None,
        Some(Arc::new(NextCommitSelector)),
    )?;

    let num_commits = 5;
    for _ in 0..num_commits {
        for _ in 0..20 {
            let mut doc = Document::new();
            doc.add(LuceneTestCase::new_string_field(
                "docId",
                &format!("doc-{doc_id}"),
                Store::YES,
            ));
            doc_id += 1;
            doc.add(FacetField::new(
                "Color",
                &[colors[color_index % colors.len()]],
            )?);
            color_index += 1;
            let doc = config.build_with_taxonomy(Some(&tw), &doc)?;
            w.add_document(&doc)?;
        }
        w.commit()?;
        tw.commit()?;
    }

    // maybeRefresh only refreshes on the next incremental commit
    // so it takes us numCommits to get to latest
    let mut steps_to_current = 0;
    while !sat.is_searcher_current()? {
        let old_gen = sat.get_searcher_commit_generation()?;
        sat.maybe_refresh_blocking()?;
        let new_gen = sat.get_searcher_commit_generation()?;
        Assert::assert_equals(new_gen, old_gen + 1);
        steps_to_current += 1;
        Assert::assert_true(sat.is_taxonomy_current()?);
    }
    Assert::assert_equals(num_commits, steps_to_current);

    sat.close()?;
    w.close()?;
    tw.close()?;
    dir.close()?;
    taxo_dir.close()
}
