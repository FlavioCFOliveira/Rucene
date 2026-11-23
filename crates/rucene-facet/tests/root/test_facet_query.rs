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

//! Port of `org.apache.lucene.facet.TestFacetQuery`.

use std::sync::{Arc, Mutex, MutexGuard};

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::{AfterClass, Assert, BeforeClass};
use rucene::document::{Document, Store, TextField};
use rucene::error::Result;
use rucene::index::directory_reader::DirectoryReader;
use rucene::index::IndexableField;
use rucene::search::index_searcher::IndexSearcher;
use rucene_facet::facet_query::FacetQuery;
use rucene_facet::facets_config::FacetsConfig;
use rucene_facet::multi_facet_query::MultiFacetQuery;
use rucene_facet::sortedset::sorted_set_doc_values_facet_field::SortedSetDocValuesFacetField;
use rucene_test_framework::analysis::mock_tokenizer::WHITESPACE;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::LuceneTestCase;

use crate::facet_support::core_support::test;
use crate::facet_support::facet_class;
use crate::facet_support::search_support::static_hook;

/// `TestFacetQuery`.
pub fn class_def() -> Arc<Class> {
    facet_class(
        "org.apache.lucene.facet.TestFacetQuery",
        vec![
            static_hook("createTestIndex", BeforeClass, create_test_index),
            static_hook("closeTestIndex", AfterClass, close_test_index),
            test("testSingleValued", test_single_valued),
            test("testMultiValued", test_multi_valued),
        ],
    )
}

/// The static fields of `TestFacetQuery`.
#[derive(Default)]
struct Statics {
    index_directory: Option<TestDirectory>,
    index_writer: Option<RandomIndexWriter>,
    index_reader: Option<Arc<dyn DirectoryReader>>,
    searcher: Option<IndexSearcher>,
    #[allow(dead_code)]
    config: Option<FacetsConfig>,
}

static STATICS: Mutex<Option<Statics>> = Mutex::new(None);

fn statics() -> MutexGuard<'static, Option<Statics>> {
    STATICS.lock().unwrap_or_else(|e| e.into_inner())
}

/// `DOC_SINGLEVALUED`, `DOC_MULTIVALUED`, `DOC_NOFACET`.
fn docs() -> Result<Vec<Vec<Arc<dyn IndexableField>>>> {
    Ok(vec![
        vec![Arc::new(SortedSetDocValuesFacetField::new(
            "Author",
            &["Mark Twain"],
        )?)],
        vec![Arc::new(SortedSetDocValuesFacetField::new(
            "Author",
            &["Kurt Vonnegut"],
        )?)],
        vec![Arc::new(TextField::new(
            "Hello",
            "World".to_string(),
            Store::YES,
        )?)],
    ])
}

fn create_test_index() -> Result<()> {
    let random = LuceneTestCase::random();
    let index_directory = LuceneTestCase::new_directory();
    // create and open an index writer
    let index_writer = RandomIndexWriter::new_with_config(
        &*random,
        index_directory.as_directory(),
        LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(
            MockAnalyzer::with_automaton(&*random, Arc::clone(&WHITESPACE), false),
        ))),
    )?;
    let config = FacetsConfig::new();
    index_documents(&index_writer, &config, docs()?)?;
    let index_reader = index_writer.get_reader()?;
    // prepare searcher to search against
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&index_reader));
    *statics() = Some(Statics {
        index_directory: Some(index_directory),
        index_writer: Some(index_writer),
        index_reader: Some(index_reader),
        searcher: Some(searcher),
        config: Some(config),
    });
    Ok(())
}

fn index_documents(
    index_writer: &RandomIndexWriter,
    config: &FacetsConfig,
    docs: Vec<Vec<Arc<dyn IndexableField>>>,
) -> Result<()> {
    for fields in docs {
        for field in fields {
            let mut doc = Document::new();
            doc.add_shared(field);
            index_writer.add_document(&config.build(&doc)?)?;
        }
    }
    Ok(())
}

fn close_test_index() -> Result<()> {
    let s = statics().take().unwrap_or_default();
    if let Some(r) = s.index_reader {
        r.close()?;
    }
    if let Some(w) = s.index_writer {
        w.close()?;
    }
    if let Some(d) = s.index_directory {
        d.close()?;
    }
    Ok(())
}

fn searcher() -> IndexSearcher {
    statics()
        .as_ref()
        .and_then(|s| s.searcher.clone())
        .expect("searcher set by createTestIndex")
}

fn test_single_valued() -> Result<()> {
    let top_docs = searcher().search(
        Arc::new(FacetQuery::with_default_config("Author", &["Mark Twain"])?),
        10,
    )?;
    Assert::assert_equals(1i64, top_docs.total_hits.value());
    Ok(())
}

fn test_multi_valued() -> Result<()> {
    let top_docs = searcher().search(
        Arc::new(MultiFacetQuery::with_default_config(
            "Author",
            &[&["Mark Twain"], &["Kurt Vonnegut"]],
        )?),
        10,
    )?;
    Assert::assert_equals(2i64, top_docs.total_hits.value());
    Ok(())
}
