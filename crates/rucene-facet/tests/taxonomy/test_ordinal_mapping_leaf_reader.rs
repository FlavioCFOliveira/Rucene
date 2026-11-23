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

//! Port of `org.apache.lucene.facet.taxonomy.TestOrdinalMappingLeafReader`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::{BinaryDocValuesField, Document, NumericValue};
use rucene::error::Result;
use rucene::index::index_writer::IndexWriter;
use rucene::index::multi_doc_values::MultiDocValues;
use rucene::index::{directory_reader, IndexReader, TwoPhaseCommit};
use rucene::search::MatchAllDocsQuery;
use rucene::util::BytesRef;
use rucene_facet::facet_field::FacetField;
use rucene_facet::facets::Facets;
use rucene_facet::facets_collector_manager::FacetsCollectorManager;
use rucene_facet::facets_config::FacetsConfig;
use rucene_facet::label_and_value::number_int_value;
use rucene_facet::taxonomy::directory::directory_taxonomy_reader::DirectoryTaxonomyReader;
use rucene_facet::taxonomy::directory::directory_taxonomy_writer::{
    DirectoryTaxonomyWriter, MemoryOrdinalMap,
};
use rucene_facet::taxonomy::fast_taxonomy_facet_counts::FastTaxonomyFacetCounts;
use rucene_facet::taxonomy::taxonomy_merge_utils::TaxonomyMergeUtils;
use rucene_facet::taxonomy::taxonomy_reader::TaxonomyReader;
use rucene_facet::taxonomy::taxonomy_writer::TaxonomyWriter;
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::util::lucene_test_case::{TestDirectory, VERBOSE};
use rucene_test_framework::util::LuceneTestCase;

use crate::facet_support::facet_fixture_class;
use crate::facet_support::search_support::{ftest, set_up};

const NUM_DOCS: i32 = 100;

/// The instance of `TestOrdinalMappingLeafReader`: `facetConfig`.
pub struct OrdinalMappingLeafReader {
    facet_config: Arc<FacetsConfig>,
}

impl Default for OrdinalMappingLeafReader {
    fn default() -> Self {
        Self {
            facet_config: Arc::new(FacetsConfig::new()),
        }
    }
}

type F = OrdinalMappingLeafReader;

/// `TestOrdinalMappingLeafReader`.
pub fn class_def() -> Arc<Class> {
    facet_fixture_class::<F>(
        "org.apache.lucene.facet.taxonomy.TestOrdinalMappingLeafReader",
        vec![
            set_up(do_set_up),
            ftest("testTaxonomyMergeUtils", test_taxonomy_merge_utils),
        ],
    )
}

fn do_set_up(f: &mut F) -> Result<()> {
    f.facet_config.set_multi_valued("tag", true);
    f.facet_config.set_index_field_name("tag", "$tags"); // add custom index field name
    Ok(())
}

fn test_taxonomy_merge_utils(f: &mut F) -> Result<()> {
    let src_index_dir = LuceneTestCase::new_directory();
    let src_taxo_dir = LuceneTestCase::new_directory();
    build_index_with_facets(f, &src_index_dir, &src_taxo_dir, true)?;

    let target_index_dir = LuceneTestCase::new_directory();
    let target_taxo_dir = LuceneTestCase::new_directory();
    build_index_with_facets(f, &target_index_dir, &target_taxo_dir, false)?;

    let dest_index_writer = IndexWriter::new(
        target_index_dir.as_directory(),
        LuceneTestCase::new_index_writer_config_analyzer(None),
    )?;
    let dest_taxo_writer = DirectoryTaxonomyWriter::new(target_taxo_dir.as_directory())?;
    let merged = TaxonomyMergeUtils::merge(
        src_index_dir.as_directory(),
        src_taxo_dir.as_directory(),
        &mut MemoryOrdinalMap::new(),
        &dest_index_writer,
        &dest_taxo_writer,
        &f.facet_config,
    );
    // finally { IOUtils.close(destIndexWriter, destTaxoWriter); }
    let closed = dest_index_writer.close().and(dest_taxo_writer.close());
    merged?;
    closed?;

    verify_results(f, &target_index_dir, &target_taxo_dir)?;
    target_index_dir.close()?;
    target_taxo_dir.close()?;
    src_index_dir.close()?;
    src_taxo_dir.close()
}

fn verify_results(f: &F, index_dir: &TestDirectory, taxo_dir: &TestDirectory) -> Result<()> {
    let index_reader = directory_reader::open(index_dir.as_directory())?;
    let taxo_reader: Arc<dyn TaxonomyReader> =
        Arc::new(DirectoryTaxonomyReader::open(taxo_dir.as_directory())?);
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&index_reader));
    let result = FacetsCollectorManager::search(
        &searcher,
        Arc::new(MatchAllDocsQuery::instance()),
        10,
        &FacetsCollectorManager::new(),
    )?;
    let collector = result.facets_collector();

    // tag facets
    let tag_facets = FastTaxonomyFacetCounts::with_index_field_name(
        "$tags",
        Arc::clone(&taxo_reader),
        Arc::clone(&f.facet_config),
        collector,
    )?;
    let result = tag_facets.get_top_children(10, "tag", &[])?.unwrap();
    for lv in &result.label_values {
        if *VERBOSE {
            println!("{lv}");
        }
        Assert::assert_equals(NUM_DOCS, number_int_value(&lv.value));
    }

    // id facets
    let id_facets = FastTaxonomyFacetCounts::new(
        Arc::clone(&taxo_reader),
        Arc::clone(&f.facet_config),
        collector,
    )?;
    let id_result = id_facets.get_top_children(10, "id", &[])?.unwrap();
    Assert::assert_equals(NUM_DOCS, id_result.child_count);
    Assert::assert_equals(NumericValue::Int(NUM_DOCS * 2), id_result.value); // each "id" appears twice

    let ir: Arc<dyn IndexReader> = Arc::clone(&index_reader) as Arc<dyn IndexReader>;
    let mut bdv = MultiDocValues::get_binary_values(&ir, "bdv")?.expect("bdv values");
    let mut cbdv = MultiDocValues::get_binary_values(&ir, "cbdv")?.expect("cbdv values");
    for i in 0..index_reader.max_doc() {
        Assert::assert_equals(i, bdv.next_doc()?);
        Assert::assert_equals(i, cbdv.next_doc()?);
        Assert::assert_equals(
            cbdv.binary_value()?
                .utf8_to_string()?
                .parse::<i32>()
                .unwrap(),
            bdv.binary_value()?
                .utf8_to_string()?
                .parse::<i32>()
                .unwrap()
                * 2,
        );
    }
    index_reader.close()?;
    taxo_reader.close()
}

fn build_index_with_facets(
    f: &F,
    index_dir: &TestDirectory,
    taxo_dir: &TestDirectory,
    asc: bool,
) -> Result<()> {
    let config = LuceneTestCase::new_index_writer_config_analyzer(None);
    let writer = RandomIndexWriter::new_with_config(
        &*LuceneTestCase::random(),
        index_dir.as_directory(),
        config,
    )?;
    let taxonomy_writer = DirectoryTaxonomyWriter::new(taxo_dir.as_directory())?;
    for i in 1..=NUM_DOCS {
        let mut doc = Document::new();
        for j in i..=NUM_DOCS {
            let facet_value = if asc { j } else { NUM_DOCS - j };
            doc.add(FacetField::new("tag", &[&facet_value.to_string()])?);
        }
        // add a facet under default dim config
        doc.add(FacetField::new("id", &[&i.to_string()])?);

        // make sure OrdinalMappingLeafReader ignores non-facet BinaryDocValues fields
        doc.add(BinaryDocValuesField::new(
            "bdv",
            BytesRef::from_char_sequence(&i.to_string()),
        ));
        doc.add(BinaryDocValuesField::new(
            "cbdv",
            BytesRef::from_char_sequence(&(i * 2).to_string()),
        ));
        writer.add_document(
            &f.facet_config
                .build_with_taxonomy(Some(&taxonomy_writer), &doc)?,
        )?;
    }
    taxonomy_writer.commit()?;
    taxonomy_writer.close()?;
    writer.commit()?;
    writer.close()
}
