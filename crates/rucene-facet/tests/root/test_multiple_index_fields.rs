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

//! Port of `org.apache.lucene.facet.TestMultipleIndexFields`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::{Document, NumericValue, Store, TextField};
use rucene::error::Result;
use rucene::index::directory_reader::DirectoryReader;
use rucene::index::index_writer_config::OpenMode;
use rucene::index::TwoPhaseCommit;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::MatchAllDocsQuery;
use rucene_facet::facet_field::FacetField;
use rucene_facet::facets::Facets;
use rucene_facet::facets_collector_manager::{FacetsCollectorManager, FacetsResult};
use rucene_facet::facets_config::FacetsConfig;
use rucene_facet::label_and_value::LabelAndValue;
use rucene_facet::multi_facets::MultiFacets;
use rucene_facet::taxonomy::directory::directory_taxonomy_reader::DirectoryTaxonomyReader;
use rucene_facet::taxonomy::directory::directory_taxonomy_writer::DirectoryTaxonomyWriter;
use rucene_facet::taxonomy::taxonomy_reader::TaxonomyReader;
use rucene_facet::taxonomy::taxonomy_writer::TaxonomyWriter;
use rucene_test_framework::analysis::mock_tokenizer::WHITESPACE;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::LuceneTestCase;

use crate::facet_support::core_support::test;
use crate::facet_support::{
    assert_facet_result, facet_class, get_taxonomy_facet_counts, get_taxonomy_facet_counts_field,
};

/// `TestMultipleIndexFields`.
pub fn class_def() -> Arc<Class> {
    facet_class(
        "org.apache.lucene.facet.TestMultipleIndexFields",
        vec![
            test("testDefault", test_default),
            test("testCustom", test_custom),
            test("testTwoCustomsSameField", test_two_customs_same_field),
            test("testDifferentFieldsAndText", test_different_fields_and_text),
            test("testSomeSameSomeDifferent", test_some_same_some_different),
        ],
    )
}

/// `CATEGORIES`.
fn categories() -> Result<Vec<FacetField>> {
    Ok(vec![
        FacetField::new("Author", &["Mark Twain"])?,
        FacetField::new("Author", &["Stephen King"])?,
        FacetField::new("Author", &["Kurt Vonnegut"])?,
        FacetField::new("Band", &["Rock & Pop", "The Beatles"])?,
        FacetField::new("Band", &["Punk", "The Ramones"])?,
        FacetField::new("Band", &["Rock & Pop", "U2"])?,
        FacetField::new("Band", &["Rock & Pop", "REM"])?,
        FacetField::new("Band", &["Rock & Pop", "Dave Matthews Band"])?,
        FacetField::new("Composer", &["Bach"])?,
    ])
}

fn get_config() -> FacetsConfig {
    let config = FacetsConfig::new();
    config.set_hierarchical("Band", true);
    config
}

/// The shared fixture of each test: the directories, writers, reader,
/// taxonomy reader and the search result.
struct Setup {
    index_dir: TestDirectory,
    taxo_dir: TestDirectory,
    iw: RandomIndexWriter,
    tw: DirectoryTaxonomyWriter,
    ir: Arc<dyn DirectoryReader>,
    tr: Arc<dyn TaxonomyReader>,
    sfc: FacetsResult,
    config: Arc<FacetsConfig>,
}

/// The common head of each test, `configure` customising `getConfig()`.
fn set_up(configure: impl FnOnce(&FacetsConfig)) -> Result<Setup> {
    let random = LuceneTestCase::random();
    let index_dir = LuceneTestCase::new_directory();
    let taxo_dir = LuceneTestCase::new_directory();

    // create and open an index writer
    let iw = RandomIndexWriter::new_with_config(
        &*random,
        index_dir.as_directory(),
        LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(
            MockAnalyzer::with_automaton(&*random, Arc::clone(&WHITESPACE), false),
        ))),
    )?;
    // create and open a taxonomy writer
    let tw = DirectoryTaxonomyWriter::with_open_mode(taxo_dir.as_directory(), OpenMode::CREATE)?;

    let config = get_config();
    configure(&config);
    let config = Arc::new(config);

    seed_index(&tw, &iw, &config)?;

    let ir = iw.get_reader()?;
    tw.commit()?;

    // prepare index reader and taxonomy.
    let tr: Arc<dyn TaxonomyReader> =
        Arc::new(DirectoryTaxonomyReader::open(taxo_dir.as_directory())?);

    // prepare searcher to search against
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&ir));

    let sfc = perform_search(&searcher)?;
    Ok(Setup {
        index_dir,
        taxo_dir,
        iw,
        tw,
        ir,
        tr,
        sfc,
        config,
    })
}

/// `iw.close(); IOUtils.close(tr, ir, tw, indexDir, taxoDir)`.
fn tear_down(s: Setup) -> Result<()> {
    s.iw.close()?;
    s.tr.close()?;
    s.ir.close()?;
    s.tw.close()?;
    s.index_dir.close()?;
    s.taxo_dir.close()
}

fn test_default() -> Result<()> {
    let s = set_up(|_| {})?;
    // Obtain facets results and hand-test them
    assert_correct_results(&*get_taxonomy_facet_counts(
        &s.tr,
        &s.config,
        s.sfc.facets_collector(),
    )?)?;
    assert_ordinals_exist("$facets", &s.ir)?;
    tear_down(s)
}

fn test_custom() -> Result<()> {
    let s = set_up(|c| c.set_index_field_name("Author", "$author"))?;
    let fc = s.sfc.facets_collector();
    let facets_map = vec![(
        "Author".to_string(),
        get_taxonomy_facet_counts_field(&s.tr, &s.config, fc, "$author")?,
    )];
    let facets = MultiFacets::with_default(
        facets_map,
        Some(get_taxonomy_facet_counts(&s.tr, &s.config, fc)?),
    );
    // Obtain facets results and hand-test them
    assert_correct_results(&facets)?;
    assert_ordinals_exist("$facets", &s.ir)?;
    assert_ordinals_exist("$author", &s.ir)?;
    tear_down(s)
}

fn test_two_customs_same_field() -> Result<()> {
    let s = set_up(|c| {
        c.set_index_field_name("Band", "$music");
        c.set_index_field_name("Composer", "$music");
    })?;
    let fc = s.sfc.facets_collector();
    let facets2 = get_taxonomy_facet_counts_field(&s.tr, &s.config, fc, "$music")?;
    let facets_map = vec![
        ("Band".to_string(), Arc::clone(&facets2)),
        ("Composer".to_string(), facets2),
    ];
    let facets = MultiFacets::with_default(
        facets_map,
        Some(get_taxonomy_facet_counts(&s.tr, &s.config, fc)?),
    );
    // Obtain facets results and hand-test them
    assert_correct_results(&facets)?;
    assert_ordinals_exist("$facets", &s.ir)?;
    assert_ordinals_exist("$music", &s.ir)?;
    assert_ordinals_exist("$music", &s.ir)?;
    tear_down(s)
}

fn assert_ordinals_exist(field: &str, ir: &Arc<dyn DirectoryReader>) -> Result<()> {
    for context in Arc::clone(ir).leaves()? {
        let r = context.leaf_reader();
        if r.get_sorted_numeric_doc_values(field)?.is_some() {
            return Ok(()); // not all segments must have this DocValues
        }
    }
    Assert::fail(&format!("no ordinals found for {field}"))
}

fn test_different_fields_and_text() -> Result<()> {
    let s = set_up(|c| {
        c.set_index_field_name("Band", "$bands");
        c.set_index_field_name("Composer", "$composers");
    })?;
    let fc = s.sfc.facets_collector();
    let facets_map = vec![
        (
            "Band".to_string(),
            get_taxonomy_facet_counts_field(&s.tr, &s.config, fc, "$bands")?,
        ),
        (
            "Composer".to_string(),
            get_taxonomy_facet_counts_field(&s.tr, &s.config, fc, "$composers")?,
        ),
    ];
    let facets = MultiFacets::with_default(
        facets_map,
        Some(get_taxonomy_facet_counts(&s.tr, &s.config, fc)?),
    );
    // Obtain facets results and hand-test them
    assert_correct_results(&facets)?;
    assert_ordinals_exist("$facets", &s.ir)?;
    assert_ordinals_exist("$bands", &s.ir)?;
    assert_ordinals_exist("$composers", &s.ir)?;
    tear_down(s)
}

fn test_some_same_some_different() -> Result<()> {
    let s = set_up(|c| {
        c.set_index_field_name("Band", "$music");
        c.set_index_field_name("Composer", "$music");
        c.set_index_field_name("Author", "$literature");
    })?;
    let fc = s.sfc.facets_collector();
    let facets2 = get_taxonomy_facet_counts_field(&s.tr, &s.config, fc, "$music")?;
    let facets_map = vec![
        ("Band".to_string(), Arc::clone(&facets2)),
        ("Composer".to_string(), facets2),
        (
            "Author".to_string(),
            get_taxonomy_facet_counts_field(&s.tr, &s.config, fc, "$literature")?,
        ),
    ];
    let facets = MultiFacets::with_default(
        facets_map,
        Some(get_taxonomy_facet_counts(&s.tr, &s.config, fc)?),
    );
    // Obtain facets results and hand-test them
    assert_correct_results(&facets)?;
    assert_ordinals_exist("$music", &s.ir)?;
    assert_ordinals_exist("$literature", &s.ir)?;
    // `IOUtils.close(tr, ir, iw, tw, indexDir, taxoDir)` after `iw.close()`:
    // closing the writer twice is a no-op.
    tear_down(s)
}

fn lv(label: &str, value: i32) -> LabelAndValue {
    LabelAndValue::new(label, NumericValue::Int(value))
}

/// `String.valueOf(facets.getTopChildren(...))`.
fn top_children_string(facets: &dyn Facets, dim: &str, path: &[&str]) -> Result<String> {
    Ok(facets
        .get_top_children(10, dim, path)?
        .map_or_else(|| "null".to_string(), |r| r.to_string()))
}

fn assert_correct_results(facets: &dyn Facets) -> Result<()> {
    Assert::assert_equals(
        NumericValue::Int(5),
        facets.get_specific_value("Band", &[])?,
    );
    Assert::assert_equals(
        "dim=Band path=[] value=5 childCount=2\n  Rock & Pop (4)\n  Punk (1)\n",
        top_children_string(facets, "Band", &[])?.as_str(),
    );
    assert_facet_result(
        &facets.get_all_children("Band", &[])?.unwrap(),
        "Band",
        &[],
        2,
        NumericValue::Int(5),
        &[lv("Punk", 1), lv("Rock & Pop", 4)],
    );
    Assert::assert_equals(
        "dim=Band path=[Rock & Pop] value=4 childCount=4\n  The Beatles (1)\n  U2 (1)\n  REM (1)\n  Dave Matthews Band (1)\n",
        top_children_string(facets, "Band", &["Rock & Pop"])?.as_str(),
    );
    assert_facet_result(
        &facets.get_all_children("Band", &["Rock & Pop"])?.unwrap(),
        "Band",
        &["Rock & Pop"],
        4,
        NumericValue::Int(4),
        &[
            lv("Dave Matthews Band", 1),
            lv("REM", 1),
            lv("The Beatles", 1),
            lv("U2", 1),
        ],
    );
    Assert::assert_equals(
        "dim=Author path=[] value=3 childCount=3\n  Mark Twain (1)\n  Stephen King (1)\n  Kurt Vonnegut (1)\n",
        top_children_string(facets, "Author", &[])?.as_str(),
    );
    assert_facet_result(
        &facets.get_all_children("Author", &[])?.unwrap(),
        "Author",
        &[],
        3,
        NumericValue::Int(3),
        &[
            lv("Kurt Vonnegut", 1),
            lv("Mark Twain", 1),
            lv("Stephen King", 1),
        ],
    );
    Ok(())
}

fn perform_search(searcher: &IndexSearcher) -> Result<FacetsResult> {
    FacetsCollectorManager::search(
        searcher,
        Arc::new(MatchAllDocsQuery::instance()),
        10,
        &FacetsCollectorManager::new(),
    )
}

fn seed_index(
    tw: &dyn TaxonomyWriter,
    iw: &RandomIndexWriter,
    config: &FacetsConfig,
) -> Result<()> {
    for ff in categories()? {
        let mut doc = Document::new();
        doc.add(ff);
        doc.add(TextField::new("content", "alpha".to_string(), Store::YES)?);
        iw.add_document(&config.build_with_taxonomy(Some(tw), &doc)?)?;
    }
    Ok(())
}
