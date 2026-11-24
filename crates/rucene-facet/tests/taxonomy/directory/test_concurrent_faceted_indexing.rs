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

//! Port of `org.apache.lucene.facet.taxonomy.directory.TestConcurrentFacetedIndexing`.

use std::collections::HashSet;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::jdk::thread::Thread;
use randomizedtesting::junit::Assert;
use rucene::document::Document;
use rucene::error::Result;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::OpenMode;
use rucene_facet::facet_field::FacetField;
use rucene_facet::facets_config::FacetsConfig;
use rucene_facet::taxonomy::directory::directory_taxonomy_reader::DirectoryTaxonomyReader;
use rucene_facet::taxonomy::directory::directory_taxonomy_writer::DirectoryTaxonomyWriter;
use rucene_facet::taxonomy::facet_label::FacetLabel;
use rucene_facet::taxonomy::taxonomy_reader::TaxonomyReader;
use rucene_facet::taxonomy::taxonomy_writer::TaxonomyWriter;
use rucene_facet::taxonomy::writercache::lru_taxonomy_writer_cache::LruTaxonomyWriterCache;
use rucene_facet::taxonomy::writercache::taxonomy_writer_cache::TaxonomyWriterCache;
use rucene_test_framework::util::java_exceptions::java_throwable;
use rucene_test_framework::util::lucene_test_case::TEST_NIGHTLY;
use rucene_test_framework::util::LuceneTestCase;

use crate::facet_support::core_support::test;
use crate::facet_support::facet_class;

/// `TestConcurrentFacetedIndexing`.
pub fn class_def() -> Arc<Class> {
    facet_class(
        "org.apache.lucene.facet.taxonomy.directory.TestConcurrentFacetedIndexing",
        vec![test("testConcurrency", test_concurrency)],
    )
}

/// `NO_OP_CACHE`: a No-Op TaxonomyWriterCache which always discards all
/// given categories, and always returns true in put(), to indicate some cache
/// entries were cleared.
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

/// `newCategory()`.
fn new_category() -> Result<FacetField> {
    let r = LuceneTestCase::random();
    let l1 = format!("l1.{}", r.next_int_bound(10)); // l1.0-l1.9 (10 categories)
    let l2 = format!("l2.{}", r.next_int_bound(30)); // l2.0-l2.29 (30 categories)
    let l3 = format!("l3.{}", r.next_int_bound(100)); // l3.0-l3.99 (100 categories)
    FacetField::new(&l1, &[&l2, &l3])
}

/// `newTaxoWriterCache(int ndocs)`.
fn new_taxo_writer_cache(ndocs: i32) -> Arc<dyn TaxonomyWriterCache> {
    let d = LuceneTestCase::random().next_double();
    if d < 0.7 {
        // same as LruTaxonomyWriterCache but with the default cache size
        DirectoryTaxonomyWriter::default_taxonomy_writer_cache()
    } else if *TEST_NIGHTLY && d > 0.98 {
        // this is the slowest, but tests the writer concurrency when no caching is done.
        // only pick it during NIGHTLY tests, and even then, with very low chances.
        Arc::new(NoOpCache)
    } else {
        // this is slower than UTF8, but less memory consuming, and exercises finding categories on
        // disk too.
        Arc::new(LruTaxonomyWriterCache::new(ndocs / 10))
    }
}

fn test_concurrency() -> Result<()> {
    let num_docs = Arc::new(AtomicI32::new(LuceneTestCase::at_least(10000)));
    let index_dir = LuceneTestCase::new_directory();
    let taxo_dir = LuceneTestCase::new_directory();
    let values: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));
    let iw = IndexWriter::new(
        index_dir.as_directory(),
        LuceneTestCase::new_index_writer_config_analyzer(None),
    )?;
    let tw = Arc::new(DirectoryTaxonomyWriter::with_cache(
        taxo_dir.as_directory(),
        OpenMode::CREATE,
        Some(new_taxo_writer_cache(num_docs.load(Ordering::SeqCst))),
    )?);
    let num_threads = LuceneTestCase::at_least(4);
    let config = Arc::new(FacetsConfig::new());
    for i in 0..10 {
        config.set_hierarchical(&format!("l1.{i}"), true);
        config.set_multi_valued(&format!("l1.{i}"), true);
    }

    let mut index_threads = Vec::new();
    for _ in 0..num_threads {
        let (num_docs, values, iw, tw, config) = (
            Arc::clone(&num_docs),
            Arc::clone(&values),
            Arc::clone(&iw),
            Arc::clone(&tw),
            Arc::clone(&config),
        );
        index_threads.push(Thread::unnamed(move || {
            let random = LuceneTestCase::random();
            while num_docs.fetch_sub(1, Ordering::SeqCst) - 1 > 0 {
                let outcome = (|| -> Result<()> {
                    let mut doc = Document::new();
                    let mut num_cats = random.next_int_bound(3) + 1; // 1-3
                    while num_cats > 0 {
                        num_cats -= 1;
                        let ff = new_category()?;
                        let label = FacetLabel::with_dim(&ff.dim, &ff.path)?;
                        doc.add(ff);
                        // add all prefixes to values
                        let mut level = label.length;
                        while level > 0 {
                            let s = FacetsConfig::path_to_string_len(&label.components, level)?;
                            values.lock().unwrap_or_else(|e| e.into_inner()).insert(s);
                            level -= 1;
                        }
                    }
                    iw.add_document(&config.build_with_taxonomy(Some(&*tw), &doc)?)?;
                    Ok(())
                })();
                if let Err(e) = outcome {
                    // `catch (IOException e) { throw new RuntimeException(e); }`
                    if e.is_io_exception() {
                        Throwable::from_cause("java.lang.RuntimeException", java_throwable(&e))
                            .throw()
                    }
                    java_throwable(&e).throw()
                }
            }
        }));
    }

    for t in &index_threads {
        t.start();
    }
    for t in &index_threads {
        if let Err(payload) = t.join() {
            std::panic::resume_unwind(payload);
        }
    }

    let tr = DirectoryTaxonomyReader::from_writer(Arc::clone(&tw))?;
    let values: Vec<String> = values
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .cloned()
        .collect();
    // +1 for root category
    if values.len() as i32 + 1 != tr.get_size()? {
        for value in &values {
            let label = FacetLabel::new(FacetsConfig::string_to_path(value))?;
            if tr.get_ordinal(&label)? == -1 {
                println!("FAIL: path={label} not recognized");
            }
        }
        Assert::fail("mismatch number of categories");
    }

    let arrays = tr.get_parallel_taxonomy_arrays()?;
    let parents = arrays.parents();
    for cat in &values {
        let cp = FacetLabel::new(FacetsConfig::string_to_path(cat))?;
        Assert::assert_true_msg(
            &format!("category not found {cp}"),
            tr.get_ordinal(&cp)? > 0,
        );
        let level = cp.length;
        let mut parent_ord = 0; // for root, parent is always virtual ROOT (ord=0)
        for i in 0..level {
            let path = cp.subpath(i + 1);
            let ord = tr.get_ordinal(&path)?;
            Assert::assert_equals_msg(
                Some(&format!("invalid parent for cp={path}")),
                parent_ord,
                parents.get(ord),
            );
            parent_ord = ord; // next level should have this parent
        }
    }

    iw.close()?;
    tw.close()?;
    tr.close()?;
    taxo_dir.close()?;
    index_dir.close()
}
