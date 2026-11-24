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

//! Port of `org.apache.lucene.facet.taxonomy.TestOrdinalData`.

use std::collections::HashMap;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::{Document, NumericDocValuesField, Store, StringField};
use rucene::error::{LuceneError, Result};
use rucene::index::directory_reader::DirectoryReader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::{Term, TwoPhaseCommit};
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::query::Query;
use rucene::search::term_query::TermQuery;
use rucene_facet::facet_field::FacetField;
use rucene_facet::facets_config::FacetsConfig;
use rucene_facet::taxonomy::directory::directory_taxonomy_reader::DirectoryTaxonomyReader;
use rucene_facet::taxonomy::directory::reindexing_enriched_directory_taxonomy_writer::{
    OrdinalDataAppender, ReindexingEnrichedDirectoryTaxonomyWriter,
};
use rucene_facet::taxonomy::facet_label::FacetLabel;
use rucene_facet::taxonomy::taxonomy_reader::TaxonomyReader;
use rucene_facet::taxonomy::taxonomy_writer::TaxonomyWriter;
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::LuceneTestCase;

use crate::facet_support::facet_fixture_class;
use crate::facet_support::search_support::{ftest, set_up, tear_down};

/// The instance fields of `TestOrdinalData`.
#[derive(Default)]
pub struct OrdinalData {
    taxo_dir: Option<TestDirectory>,
    taxo_reader: Option<DirectoryTaxonomyReader>,
    taxo_index_reader: Option<Arc<dyn DirectoryReader>>,
    taxo_writer: Option<ReindexingEnrichedDirectoryTaxonomyWriter>,
}

type F = OrdinalData;

/// `TestOrdinalData`.
pub fn class_def() -> Arc<Class> {
    facet_fixture_class::<F>(
        "org.apache.lucene.facet.taxonomy.TestOrdinalData",
        vec![
            set_up(do_set_up),
            tear_down(do_tear_down),
            ftest("testDocValue", test_doc_value),
            ftest("testSearchableField", test_searchable_field),
            ftest("testReindex", test_reindex),
        ],
    )
}

/// `labelToScore`.
fn label_to_score() -> HashMap<String, i64> {
    HashMap::from([("Bob".to_string(), 42), ("Lisa".to_string(), 35)])
}

/// `private record OrdinalDataAppender(Map<String, Long> scores)`.
fn ordinal_data_appender(scores: HashMap<String, i64>) -> OrdinalDataAppender {
    Arc::new(move |facet_label: &FacetLabel, doc: &mut Document| {
        if facet_label.length == 0 {
            return;
        }
        match scores.get(&facet_label.components[facet_label.length - 1]) {
            Some(&score) => {
                doc.add(NumericDocValuesField::new("score", score));
                doc.add(
                    StringField::new("hasScore?", "yes".to_string(), Store::NO)
                        .expect("a valid StringField"),
                );
            }
            None => {
                doc.add(
                    StringField::new("hasScore?", "no".to_string(), Store::NO)
                        .expect("a valid StringField"),
                );
            }
        }
    })
}

fn do_set_up(f: &mut F) -> Result<()> {
    let index_dir = LuceneTestCase::new_directory();
    let taxo_dir = LuceneTestCase::new_directory();

    let index_writer = IndexWriter::new(index_dir.as_directory(), IndexWriterConfig::new())?;
    let taxo_writer = ReindexingEnrichedDirectoryTaxonomyWriter::new(
        taxo_dir.as_directory(),
        Some(ordinal_data_appender(label_to_score())),
    )?;

    let facets_config = FacetsConfig::new();
    facets_config.set_hierarchical("Author", true);
    facets_config.set_multi_valued("Author", true);
    facets_config.set_hierarchical("Publish Date", true);
    facets_config.set_multi_valued("Publish Date", true);

    let mut doc = Document::new();
    doc.add(FacetField::new("Author", &["Bob"])?);
    doc.add(FacetField::new("Publish Date", &["2010", "10", "15"])?);
    index_writer.add_document(&facets_config.build_with_taxonomy(Some(&*taxo_writer), &doc)?)?;

    let mut doc = Document::new();
    doc.add(FacetField::new("Author", &["Lisa"])?);
    doc.add(FacetField::new("Publish Date", &["2010", "10", "20"])?);
    index_writer.add_document(&facets_config.build_with_taxonomy(Some(&*taxo_writer), &doc)?)?;

    index_writer.close()?;
    index_dir.close()?;

    taxo_writer.commit()?;
    let taxo_reader = DirectoryTaxonomyReader::open(taxo_dir.as_directory())?;
    f.taxo_index_reader = Some(taxo_reader.get_internal_index_reader()?);
    f.taxo_reader = Some(taxo_reader);
    f.taxo_writer = Some(taxo_writer);
    f.taxo_dir = Some(taxo_dir);
    Ok(())
}

fn do_tear_down(f: &mut F) -> Result<()> {
    // `super.tearDown()` runs first in Java; the framework's `tear_down`
    // runs it after: neither touches these fields.
    if let Some(w) = f.taxo_writer.take() {
        w.close()?;
    }
    if let Some(r) = f.taxo_reader.take() {
        r.close()?;
    }
    if let Some(d) = f.taxo_dir.take() {
        d.close()?;
    }
    Ok(())
}

fn test_doc_value(f: &mut F) -> Result<()> {
    let taxo_index_reader = Arc::clone(f.taxo_index_reader.as_ref().unwrap());
    let taxo_reader = f.taxo_reader.as_ref().unwrap();
    let label_to_score = label_to_score();
    // Each unique label will have been assigned a doc.
    // Additionally, we have the root node of the taxonomy.
    Assert::assert_equals(9, taxo_index_reader.max_doc());
    for ctx in taxo_index_reader.leaves()? {
        let leaf_reader = ctx.leaf_reader();
        let Some(mut scores) = leaf_reader.get_numeric_doc_values("score")? else {
            continue;
        };
        for ord in 0..leaf_reader.max_doc() {
            if !scores.advance_exact(ord)? {
                continue;
            }
            let label = taxo_reader.get_path(ctx.doc_base() + ord)?.unwrap();
            let Some(&score) = label_to_score.get(&label.components[label.length - 1]) else {
                return Err(LuceneError::from(std::io::Error::other(format!(
                    "Unexpected score for [{}]",
                    label.components.join(", ")
                ))));
            };
            Assert::assert_equals(score, scores.long_value()?);
        }
    }
    Ok(())
}

fn validate_search_results(
    searcher: &IndexSearcher,
    queries_and_counts: Vec<(Arc<dyn Query>, i64)>,
) -> Result<()> {
    for (q, count) in queries_and_counts {
        let td = searcher.search(q, i32::MAX)?;
        Assert::assert_equals(count, td.total_hits.value());
    }
    Ok(())
}

fn has_score(v: &str) -> Arc<dyn Query> {
    Arc::new(TermQuery::new(Term::from_text("hasScore?", v)))
}

fn test_searchable_field(f: &mut F) -> Result<()> {
    let taxo_searcher =
        LuceneTestCase::new_searcher(Arc::clone(f.taxo_index_reader.as_ref().unwrap()));
    validate_search_results(
        &taxo_searcher,
        vec![(has_score("yes"), 2), (has_score("no"), 6)],
    )
}

fn test_reindex(f: &mut F) -> Result<()> {
    f.taxo_writer
        .as_ref()
        .unwrap()
        .reindex_with_new_ordinal_data(Some(ordinal_data_appender(HashMap::new())))?;
    f.taxo_reader.take().unwrap().close()?;
    let taxo_reader = DirectoryTaxonomyReader::open(f.taxo_dir.as_ref().unwrap().as_directory())?;
    f.taxo_index_reader = Some(taxo_reader.get_internal_index_reader()?);
    f.taxo_reader = Some(taxo_reader);

    let taxo_searcher =
        LuceneTestCase::new_searcher(Arc::clone(f.taxo_index_reader.as_ref().unwrap()));
    validate_search_results(
        &taxo_searcher,
        vec![(has_score("yes"), 0), (has_score("no"), 8)],
    )
}
