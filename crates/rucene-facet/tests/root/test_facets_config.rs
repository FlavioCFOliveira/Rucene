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

//! Port of `org.apache.lucene.facet.TestFacetsConfig`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::{Document, NumericValue};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::search::match_all_docs_query::MatchAllDocsQuery;
use rucene_facet::facet_field::FacetField;
use rucene_facet::facets_collector_manager::FacetsCollectorManager;
use rucene_facet::facets_config::{DimConfig, DrillDownTermsIndexing, FacetsConfig};
use rucene_facet::taxonomy::directory::directory_taxonomy_reader::DirectoryTaxonomyReader;
use rucene_facet::taxonomy::directory::directory_taxonomy_writer::DirectoryTaxonomyWriter;
use rucene_facet::taxonomy::taxonomy_reader::TaxonomyReader;
use rucene_facet::taxonomy::taxonomy_writer::TaxonomyWriter;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::facet_support::core_support::test;
use crate::facet_support::{facet_class, get_taxonomy_facet_counts};

/// `TestFacetsConfig`.
pub fn class_def() -> Arc<Class> {
    facet_class(
        "org.apache.lucene.facet.TestFacetsConfig",
        vec![
            test("testPathToStringAndBack", test_path_to_string_and_back),
            test("testAddSameDocTwice", test_add_same_doc_twice),
            test("testIsDimConfigured", test_is_dim_configured),
            test("testCustomDefault", test_custom_default),
        ],
    )
}

fn test_path_to_string_and_back() -> Result<()> {
    let random = LuceneTestCase::random();
    let iters = LuceneTestCase::at_least(1000);
    for _ in 0..iters {
        let num_parts = TestUtil::next_int(&*random, 1, 6);
        let mut parts = Vec::new();
        for _ in 0..num_parts {
            let s = loop {
                let s = TestUtil::random_unicode_string(&*random);
                if !s.is_empty() {
                    break s;
                }
            };
            parts.push(s);
        }

        let s = FacetsConfig::path_to_string(&parts)?;
        let parts2 = FacetsConfig::string_to_path(&s);
        Assert::assert_true(parts == parts2);
    }
    Ok(())
}

fn test_add_same_doc_twice() -> Result<()> {
    // LUCENE-5367: this was a problem with the previous code, making sure it
    // works with the new code.
    let index_dir = LuceneTestCase::new_directory();
    let taxo_dir = LuceneTestCase::new_directory();
    let index_writer = IndexWriter::new(
        index_dir.as_directory(),
        LuceneTestCase::new_index_writer_config(),
    )?;
    let taxo_writer = DirectoryTaxonomyWriter::new(taxo_dir.as_directory())?;
    let facets_config = Arc::new(FacetsConfig::new());
    let mut doc = Document::new();
    doc.add(FacetField::new("a", &["b"])?);
    let doc = facets_config.build_with_taxonomy(Some(&taxo_writer), &doc)?;
    // these two addDocument() used to fail
    index_writer.add_document(&doc)?;
    index_writer.add_document(&doc)?;
    index_writer.close()?;
    taxo_writer.close()?;

    let index_reader = directory_reader::open(index_dir.as_directory())?;
    let taxo_reader: Arc<dyn TaxonomyReader> =
        Arc::new(DirectoryTaxonomyReader::open(taxo_dir.as_directory())?);
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&index_reader));
    let fc = searcher.search_with_manager(
        Arc::new(MatchAllDocsQuery::instance()),
        &FacetsCollectorManager::new(),
    )?;

    let facets = get_taxonomy_facet_counts(&taxo_reader, &facets_config, &fc)?;
    let res = facets.get_top_children(10, "a", &[])?.unwrap();
    Assert::assert_equals(1usize, res.label_values.len());
    Assert::assert_equals(NumericValue::Int(2), res.label_values[0].value);
    index_reader.close()?;
    taxo_reader.close()?;

    index_dir.close()?;
    taxo_dir.close()
}

fn test_is_dim_configured() -> Result<()> {
    let config = FacetsConfig::new();
    config.set_index_field_name("configured1", "name");
    config.set_multi_valued("configured2", true);
    config.set_require_dim_count("configured3", true);
    config.set_hierarchical("configured4", true);
    config.set_drill_down_terms_indexing("configured5", DrillDownTermsIndexing::ALL);

    Assert::assert_true(config.is_dim_configured("configured1"));
    Assert::assert_true(config.is_dim_configured("configured2"));
    Assert::assert_true(config.is_dim_configured("configured3"));
    Assert::assert_true(config.is_dim_configured("configured4"));
    Assert::assert_true(config.is_dim_configured("configured5"));

    Assert::assert_false(config.is_dim_configured("unconfigured"));
    Ok(())
}

/// LUCENE-5479
fn test_custom_default() -> Result<()> {
    // `new FacetsConfig() { protected DimConfig getDefaultDimConfig() {...} }`
    let default = DimConfig::new();
    default.set_hierarchical(true);
    let config = FacetsConfig::with_default_dim_config(default);

    Assert::assert_true(config.get_dim_config("foobar").hierarchical());
    Ok(())
}
