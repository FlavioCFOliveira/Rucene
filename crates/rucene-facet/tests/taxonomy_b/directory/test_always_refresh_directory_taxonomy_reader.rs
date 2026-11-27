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

//! Port of
//! `org.apache.lucene.facet.taxonomy.directory.TestAlwaysRefreshDirectoryTaxonomyReader`.
//!
//! Its only test method is `@Ignore("LUCENE-10482: need to make this work on
//! Windows too")` in Lucene 10.5.0; it is declared with that `@Ignore`, so the
//! runner reports it ignored and never runs it, as JUnit does.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Ignore;
use randomizedtesting::RandomizedTest;
use rucene::error::Result;
use rucene::index::index_writer_config::OpenMode;
use rucene::index::{directory_reader, TwoPhaseCommit};
use rucene::store::{Directory, DEFAULT_IO_CONTEXT};
use rucene::util::IOUtils;
use rucene_facet::facets_collector::FacetsCollector;
use rucene_facet::facets_config::FacetsConfig;
use rucene_facet::taxonomy::directory::directory_taxonomy_reader::{
    DirectoryTaxonomyReader, DirectoryTaxonomyReaderOverrides,
};
use rucene_facet::taxonomy::directory::directory_taxonomy_writer::DirectoryTaxonomyWriter;
use rucene_facet::taxonomy::facet_label::FacetLabel;
use rucene_facet::taxonomy::searcher_taxonomy_manager::SearcherTaxonomyManager;
use rucene_facet::taxonomy::taxonomy_reader::TaxonomyReader;
use rucene_facet::taxonomy::taxonomy_writer::TaxonomyWriter;
use rucene_test_framework::mockfile::ExtrasFS;
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::lucene_test_case::SuppressFileSystems;
use rucene_test_framework::util::LuceneTestCase;

use crate::facet_support::core_support::{assert_equals, test};
use crate::facet_support::{facet_class_annotated, get_taxonomy_facet_counts};

/// `TestAlwaysRefreshDirectoryTaxonomyReader`.
pub fn class_def() -> Arc<Class> {
    facet_class_annotated(
        "org.apache.lucene.facet.taxonomy.directory.TestAlwaysRefreshDirectoryTaxonomyReader",
        vec![test(
            "testAlwaysRefreshDirectoryTaxonomyReader",
            test_always_refresh_directory_taxonomy_reader,
        )
        .annotate(Ignore {
            value: "LUCENE-10482: need to make this work on Windows too".to_string(),
        })],
        // Nefarious FS will delay/stop deletion of index files and this test specifically does that
        |b| {
            b.annotate(SuppressFileSystems {
                value: vec!["WindowsFS".to_string(), "VirusCheckingFS".to_string()],
            })
        },
    )
}

/// Tests the behavior of the `AlwaysRefreshDirectoryTaxonomyReader` by
/// testing if the associated `SearcherTaxonomyManager` can successfully
/// refresh and serve queries if the underlying taxonomy index is changed to
/// an older checkpoint. Ideally, each checkpoint should be self-sufficient and
/// should allow serving search queries when
/// `SearcherTaxonomyManager#maybeRefresh()` is called.
///
/// It does not check whether the private taxoArrays were actually recreated
/// or no. We are (correctly) hiding away that complexity away from the user.
///
/// `exception_type` is Java's `Class<T> exceptionType` (`None`: `null`).
fn test_always_refresh_directory_taxonomy_reader_with(
    dtr_producer: impl Fn(Arc<dyn Directory>) -> Option<DirectoryTaxonomyReader>,
    exception_type: Option<&str>,
) -> Result<()> {
    let taxo_path1 = LuceneTestCase::create_temp_dir();
    let dir1 = LuceneTestCase::new_fs_directory(&taxo_path1);

    let tw1 = DirectoryTaxonomyWriter::with_open_mode(dir1.as_directory(), OpenMode::CREATE)?;
    tw1.add_category(&FacetLabel::new(["a"])?)?;
    tw1.commit()?; // commit1

    let taxo_path2 = LuceneTestCase::create_temp_dir();
    let commit1 = LuceneTestCase::new_fs_directory(&taxo_path2);
    // copy all index files from dir1
    for file in dir1.as_directory().list_all()? {
        if !ExtrasFS::is_extra(&file) {
            // the test framework creates these devious extra files just to chaos test the edge cases
            commit1.as_directory().copy_from(
                &*dir1.as_directory(),
                &file,
                &file,
                &*DEFAULT_IO_CONTEXT,
            )?;
        }
    }

    tw1.add_category(&FacetLabel::new(["b"])?)?;
    tw1.commit()?; // commit2
    tw1.close()?;

    let dr1 = directory_reader::open(dir1.as_directory())?;
    let dtr1 = Arc::new(dtr_producer(dir1.as_directory()).expect("java.lang.NullPointerException"));
    let mgr = SearcherTaxonomyManager::from_readers(Arc::clone(&dr1), Arc::clone(&dtr1), None)?;

    let config = Arc::new(FacetsConfig::new());
    let mut pair = mgr.acquire()?;
    let sfc = FacetsCollector::new();
    // the call flow here initializes {@link DirectoryTaxonomyReader#taxoArrays}. These reused
    // `taxoArrays` form the basis of the inconsistency *
    get_taxonomy_facet_counts(
        &(Arc::clone(pair.taxonomy_reader()) as Arc<dyn TaxonomyReader>),
        &config,
        &sfc,
    )?;

    // now try to go back to checkpoint 1 and refresh the SearcherTaxonomyManager

    // delete all files from commit2
    for file in dir1.as_directory().list_all()? {
        dir1.as_directory().delete_file(&file)?;
    }

    while !dir1.as_directory().get_pending_deletions()?.is_empty() {
        // make the test more robust to the OS taking more time to actually delete files
        RandomizedTest::sleep(5);
    }

    // copy all index files from commit1
    for file in commit1.as_directory().list_all()? {
        if !ExtrasFS::is_extra(&file) {
            dir1.as_directory().copy_from(
                &*commit1.as_directory(),
                &file,
                &file,
                &*DEFAULT_IO_CONTEXT,
            )?;
        }
    }

    match exception_type {
        Some(exception_type) => {
            expect_throws(exception_type, || mgr.maybe_refresh());
        }
        None => {
            mgr.maybe_refresh()?;
            pair = mgr.acquire()?;
            assert_equals(
                &Some(FacetLabel::new(["a"])?),
                &pair.taxonomy_reader().get_path(1)?,
            );
            assert_equals(
                &-1,
                &pair
                    .taxonomy_reader()
                    .get_ordinal(&FacetLabel::new(["b"])?)?,
            );
        }
    }

    mgr.release(pair)?;
    IOUtils::close([
        IOUtils::closeable(|| mgr.close()),
        IOUtils::closeable(|| dtr1.close()),
        IOUtils::closeable(|| dr1.close()),
    ])?;
    // closing commit1 and dir1 throws exceptions because of checksum mismatches
    IOUtils::delete_files_in(
        &*commit1.as_directory(),
        commit1.as_directory().list_all()?.into_iter().map(Some),
    )?;
    IOUtils::delete_files_in(
        &*dir1.as_directory(),
        dir1.as_directory().list_all()?.into_iter().map(Some),
    )?;
    IOUtils::close([
        IOUtils::closeable(|| commit1.close()),
        IOUtils::closeable(|| dir1.close()),
    ])
}

/// `testAlwaysRefreshDirectoryTaxonomyReader()`, `@Ignore`d in Lucene 10.5.0.
fn test_always_refresh_directory_taxonomy_reader() -> Result<()> {
    test_always_refresh_directory_taxonomy_reader_with(
        |dir| match DirectoryTaxonomyReader::open(dir) {
            Ok(r) => Some(r),
            Err(e) => {
                // `e.printStackTrace()`
                eprintln!("{e}");
                None
            }
        },
        Some("java.lang.ArrayIndexOutOfBoundsException"),
    )?;
    test_always_refresh_directory_taxonomy_reader_with(
        |dir| match always_refresh_directory_taxonomy_reader(dir) {
            Ok(r) => Some(r),
            Err(e) => {
                // `e.printStackTrace()`
                eprintln!("{e}");
                None
            }
        },
        None,
    )
}

/// A modified DirectoryTaxonomyReader that always recreates a new
/// `AlwaysRefreshDirectoryTaxonomyReader` instance when
/// `AlwaysRefreshDirectoryTaxonomyReader#doOpenIfChanged()` is called. This
/// enables us to easily go forward or backward in time by re-computing the
/// ordinal space during each refresh. This results in an always
/// O(#facet_label) taxonomy array construction time when refresh is called.
///
/// The private class `AlwaysRefreshDirectoryTaxonomyReader extends
/// DirectoryTaxonomyReader`: its override of `doOpenIfChanged`; an instance is
/// a `DirectoryTaxonomyReader` built with these overrides.
struct AlwaysRefreshDirectoryTaxonomyReader;

/// `AlwaysRefreshDirectoryTaxonomyReader(Directory)`: `super(directory)`.
fn always_refresh_directory_taxonomy_reader(
    directory: Arc<dyn Directory>,
) -> Result<DirectoryTaxonomyReader> {
    DirectoryTaxonomyReader::open_with_overrides(
        directory,
        Arc::new(AlwaysRefreshDirectoryTaxonomyReader),
    )
}

impl DirectoryTaxonomyReaderOverrides for AlwaysRefreshDirectoryTaxonomyReader {
    fn do_open_if_changed(
        &self,
        this: &DirectoryTaxonomyReader,
    ) -> Result<Option<DirectoryTaxonomyReader>> {
        // the getInternalIndexReader() function performs the ensureOpen() check
        let Some(reader) = directory_reader::open_if_changed(this.get_internal_index_reader()?)?
        else {
            return Ok(None); // no changes in the directory at all, nothing to do
        };

        // It is important that we create an AlwaysRefreshDirectoryTaxonomyReader here and not a
        // DirectoryTaxonomyReader.
        // Returning a AlwaysRefreshDirectoryTaxonomyReader ensures that the recreated taxonomy
        // reader also uses the overridden doOpenIfChanged
        // method (that always recomputes values).
        // `AlwaysRefreshDirectoryTaxonomyReader(DirectoryReader)`:
        // `super(indexReader, null, null, null, null)`.
        match DirectoryTaxonomyReader::from_parts_with_overrides(
            Arc::clone(&reader),
            None,
            None,
            None,
            None,
            Arc::new(AlwaysRefreshDirectoryTaxonomyReader),
        ) {
            Ok(new_taxonomy_reader) => Ok(Some(new_taxonomy_reader)),
            Err(e) => {
                // `IOUtils.closeWhileHandlingException(reader)`
                let _ = reader.close();
                Err(e)
            }
        }
    }
}
