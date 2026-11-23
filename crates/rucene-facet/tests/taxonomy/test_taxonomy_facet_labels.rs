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

//! Port of `org.apache.lucene.facet.taxonomy.TestTaxonomyFacetLabels`.

use std::collections::HashSet;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::Document;
use rucene::error::Result;
use rucene::index::index_writer_config::OpenMode;
use rucene::search::{MatchAllDocsQuery, NO_MORE_DOCS};
use rucene_facet::facet_field::FacetField;
use rucene_facet::facets_collector::MatchingDocs;
use rucene_facet::facets_collector_manager::FacetsCollectorManager;
use rucene_facet::facets_config::{FacetsConfig, DEFAULT_INDEX_FIELD_NAME};
use rucene_facet::taxonomy::directory::directory_taxonomy_reader::DirectoryTaxonomyReader;
use rucene_facet::taxonomy::directory::directory_taxonomy_writer::DirectoryTaxonomyWriter;
use rucene_facet::taxonomy::facet_label::FacetLabel;
use rucene_facet::taxonomy::taxonomy_facet_labels::TaxonomyFacetLabels;
use rucene_facet::taxonomy::taxonomy_reader::TaxonomyReader;
use rucene_facet::taxonomy::taxonomy_writer::TaxonomyWriter;
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::util::LuceneTestCase;

use crate::facet_support::core_support::test;
use crate::facet_support::facet_class;

/// `TestTaxonomyFacetLabels`.
pub fn class_def() -> Arc<Class> {
    facet_class(
        "org.apache.lucene.facet.taxonomy.TestTaxonomyFacetLabels",
        vec![test("testBasic", test_basic)],
    )
}

fn prepare_documents() -> Result<Vec<Document>> {
    let rows: [(&str, [&str; 3]); 5] = [
        ("Bob", ["2010", "10", "15"]),
        ("Lisa", ["2010", "10", "20"]),
        ("Tom", ["2012", "1", "1"]),
        ("Susan", ["2012", "1", "7"]),
        ("Frank", ["1999", "5", "5"]),
    ];
    let mut docs = Vec::new();
    for (author, date) in rows {
        let mut doc = Document::new();
        doc.add(FacetField::new("Author", &[author])?);
        doc.add(FacetField::new("Publish Date", &date)?);
        docs.push(doc);
    }
    Ok(docs)
}

fn all_doc_ids(m: &MatchingDocs, decreasing_doc_ids: bool) -> Result<Vec<i32>> {
    let mut disi = m.bits().iterator()?;
    let mut doc_ids = Vec::new();
    while disi.next_doc()? != NO_MORE_DOCS {
        doc_ids.push(disi.doc_id());
    }
    if decreasing_doc_ids {
        doc_ids.reverse();
    }
    Ok(doc_ids)
}

fn lookup_facet_labels(
    taxo_labels: &TaxonomyFacetLabels,
    matching_docs: &[MatchingDocs],
    dimension: Option<&str>,
    decreasing_doc_ids: bool,
) -> Result<Vec<FacetLabel>> {
    let mut facet_labels = Vec::new();
    for m in matching_docs {
        let mut facet_label_reader = taxo_labels.get_facet_label_reader(m.context())?;
        let doc_ids = all_doc_ids(m, decreasing_doc_ids)?;
        for doc_id in doc_ids {
            loop {
                let facet_label = match dimension {
                    Some(d) => facet_label_reader.next_facet_label_for_dimension(doc_id, d)?,
                    None => facet_label_reader.next_facet_label(doc_id)?,
                };
                match facet_label {
                    None => break,
                    Some(l) => facet_labels.push(l),
                }
            }
        }
    }
    Ok(facet_labels)
}

fn set(values: &[&str]) -> HashSet<String> {
    values.iter().map(|s| s.to_string()).collect()
}

fn date_of(l: &FacetLabel) -> String {
    [l.components[1].as_str(), &l.components[2], &l.components[3]].join("/")
}

/// `try { lookupFacetLabels(...); fail(msg); } catch (IllegalArgumentException
/// ae) { assertTrue(ae.getMessage().contains("docs out of order")); }`.
fn expect_out_of_order(result: Result<Vec<FacetLabel>>, msg: &str) -> Result<()> {
    match result {
        Ok(_) => Assert::fail(msg),
        Err(ae) if ae.is_illegal_argument_exception() => {
            Assert::assert_true(ae.to_string().contains("docs out of order"));
            Ok(())
        }
        Err(e) => Err(e),
    }
}

fn test_basic() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let taxo_dir = LuceneTestCase::new_directory();

    // Writes facet ords to a separate directory from the main index:
    let taxo_writer = Arc::new(DirectoryTaxonomyWriter::with_open_mode(
        taxo_dir.as_directory(),
        OpenMode::CREATE,
    )?);

    let writer =
        RandomIndexWriter::new_with_mock_analyzer(&*LuceneTestCase::random(), dir.as_directory())?;
    let config = FacetsConfig::new();
    config.set_hierarchical("Publish Date", true);

    for doc in prepare_documents()? {
        writer.add_document(&config.build_with_taxonomy(Some(&*taxo_writer), &doc)?)?;
    }

    // NRT open
    let searcher = LuceneTestCase::new_searcher(writer.get_reader()?);

    // NRT open
    let taxo_reader: Arc<dyn TaxonomyReader> = Arc::new(DirectoryTaxonomyReader::from_writer(
        Arc::clone(&taxo_writer),
    )?);

    let fc = searcher.search_with_manager(
        Arc::new(MatchAllDocsQuery::instance()),
        &FacetsCollectorManager::new(),
    )?;

    let taxo_labels = TaxonomyFacetLabels::new(Arc::clone(&taxo_reader), DEFAULT_INDEX_FIELD_NAME);

    // Check labels for all dimensions
    let facet_labels = lookup_facet_labels(&taxo_labels, fc.get_matching_docs()?, None, false)?;
    Assert::assert_equals_msg(
        Some("Incorrect number of facet labels received"),
        10usize,
        facet_labels.len(),
    );

    // Check labels for all dimensions
    Assert::assert_true(
        facet_labels
            .iter()
            .filter(|l| l.components[0] == "Author")
            .map(|l| l.components[1].clone())
            .collect::<HashSet<_>>()
            == set(&["Bob", "Lisa", "Susan", "Frank", "Tom"]),
    );

    Assert::assert_true(
        facet_labels
            .iter()
            .filter(|l| l.components[0] == "Publish Date")
            .map(date_of)
            .collect::<HashSet<_>>()
            == set(&[
                "2010/10/15",
                "2010/10/20",
                "2012/1/1",
                "2012/1/7",
                "1999/5/5",
            ]),
    );

    // Check labels for a specific dimension
    let facet_labels = lookup_facet_labels(
        &taxo_labels,
        fc.get_matching_docs()?,
        Some("Publish Date"),
        false,
    )?;
    Assert::assert_equals_msg(
        Some("Incorrect number of facet labels received for 'Publish Date'"),
        5usize,
        facet_labels.len(),
    );

    Assert::assert_true(
        facet_labels.iter().map(date_of).collect::<HashSet<_>>()
            == set(&[
                "2010/10/15",
                "2010/10/20",
                "2012/1/1",
                "2012/1/7",
                "1999/5/5",
            ]),
    );

    expect_out_of_order(
        lookup_facet_labels(&taxo_labels, fc.get_matching_docs()?, None, true),
        "IllegalArgumentException was not thrown for using docIds supplied in decreasing order",
    )?;

    expect_out_of_order(
        lookup_facet_labels(
            &taxo_labels,
            fc.get_matching_docs()?,
            Some("Publish Date"),
            true,
        ),
        "Assertion error was not thrown for using docIds supplied in decreasing order",
    )?;

    writer.close()?;
    taxo_writer.close()?;
    searcher.get_index_reader().close()?;
    taxo_reader.close()?;
    taxo_dir.close()?;
    dir.close()
}
