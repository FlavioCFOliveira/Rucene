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

//! Port of `org.apache.lucene.facet.TestRandomSamplingFacetsCollector`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::{Document, Store, StringField};
use rucene::error::Result;
use rucene::index::Term;
use rucene::search::term_query::TermQuery;
use rucene::search::NO_MORE_DOCS;
use rucene_facet::facet_field::FacetField;
use rucene_facet::facets::Facets;
use rucene_facet::facets_config::FacetsConfig;
use rucene_facet::label_and_value::number_float_value;
use rucene_facet::random_sampling_facets_collector::RandomSamplingFacetsCollector;
use rucene_facet::taxonomy::directory::directory_taxonomy_reader::DirectoryTaxonomyReader;
use rucene_facet::taxonomy::directory::directory_taxonomy_writer::DirectoryTaxonomyWriter;
use rucene_facet::taxonomy::fast_taxonomy_facet_counts::FastTaxonomyFacetCounts;
use rucene_facet::taxonomy::taxonomy_reader::TaxonomyReader;
use rucene_facet::taxonomy::taxonomy_writer::TaxonomyWriter;
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::util::LuceneTestCase;

use crate::facet_support::core_support::test;
use crate::facet_support::facet_class;

/// `TestRandomSamplingFacetsCollector`.
pub fn class_def() -> Arc<Class> {
    facet_class(
        "org.apache.lucene.facet.TestRandomSamplingFacetsCollector",
        vec![test("testRandomSampling", test_random_sampling)],
    )
}

// The first 50 chi-square value for p-value=0.05, taken from:
// http://en.wikibooks.org/wiki/Engineering_Tables/Chi-Squared_Distibution
const CHI_SQUARE_VALUES: [f32; 51] = [
    0.0, 3.841, 5.991, 7.815, 9.488, 11.07, 12.592, 14.067, 15.507, 16.919, 18.307, 19.675, 21.026,
    22.362, 23.685, 24.996, 26.296, 27.587, 28.869, 30.144, 31.41, 32.671, 33.924, 35.172, 36.415,
    37.652, 38.885, 40.113, 41.337, 42.557, 43.773, 44.985, 46.194, 47.4, 48.602, 49.802, 50.998,
    52.192, 53.384, 54.572, 55.758, 56.942, 58.124, 59.304, 60.481, 61.656, 62.83, 64.001, 65.171,
    66.339, 67.505,
];

fn test_random_sampling() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let taxo_dir = LuceneTestCase::new_directory();
    let random = LuceneTestCase::random();
    let taxo_writer = Arc::new(DirectoryTaxonomyWriter::new(taxo_dir.as_directory())?);
    let writer = RandomIndexWriter::new_with_mock_analyzer(&*random, dir.as_directory())?;
    let config = Arc::new(FacetsConfig::new());

    let num_categories = 10;
    let num_docs = LuceneTestCase::at_least(10000);
    for i in 0..num_docs {
        let mut doc = Document::new();
        doc.add(StringField::new(
            "EvenOdd",
            if i % 2 == 0 { "even" } else { "odd" }.to_string(),
            Store::NO,
        )?);
        doc.add(FacetField::new(
            "iMod10",
            &[&(i % num_categories).to_string()],
        )?);
        writer.add_document(&config.build_with_taxonomy(Some(&*taxo_writer), &doc)?)?;
    }

    writer.force_merge(CHI_SQUARE_VALUES.len() as i32 - 1)?;

    // NRT open
    let searcher = LuceneTestCase::new_searcher(writer.get_reader()?);
    let taxo_reader: Arc<dyn TaxonomyReader> = Arc::new(DirectoryTaxonomyReader::from_writer(
        Arc::clone(&taxo_writer),
    )?);
    writer.close()?;
    taxo_writer.close()?;

    // Test empty results
    let fcm = RandomSamplingFacetsCollector::create_manager(num_docs / 10, random.next_long());

    // There should be no divisions by zero
    let collect_random_zero_results = searcher.search_with_manager(
        Arc::new(TermQuery::new(Term::from_text("EvenOdd", "NeverMatches"))),
        &fcm,
    )?;

    // There should be no divisions by zero and no null result
    let zero_docs = collect_random_zero_results.get_matching_docs()?;

    // There should be no results at all
    for doc in zero_docs {
        Assert::assert_equals(0, doc.total_hits());
    }

    // Now start searching and retrieve results.

    // Use a query to select half of the documents.
    let query = Arc::new(TermQuery::new(Term::from_text("EvenOdd", "even")));

    // 10% of total docs, 20% of the hits
    let fcm = RandomSamplingFacetsCollector::create_manager(num_docs / 10, random.next_long());
    let random10_percent = searcher.search_with_manager(query, &fcm)?;
    let matching_docs = random10_percent.get_matching_docs()?;

    // count the total hits and sampled docs, also store the number of sampled
    // docs per segment
    let mut total_sampled_docs = 0;
    let mut total_hits = 0;
    let mut num_sampled_docs = vec![0i32; matching_docs.len()];
    for (i, md) in matching_docs.iter().enumerate() {
        let mut iter = md.bits().iterator()?;
        while iter.next_doc()? != NO_MORE_DOCS {
            num_sampled_docs[i] += 1;
        }
        total_sampled_docs += num_sampled_docs[i];
        total_hits += md.total_hits();
    }

    // compute the chi-square value for the sampled documents' distribution
    let mut chi_square = 0f32;
    for (i, md) in matching_docs.iter().enumerate() {
        let ei = md.total_hits() as f32 / total_hits as f32;
        if ei > 0.0 {
            let oi = num_sampled_docs[i] as f32 / total_sampled_docs as f32;
            chi_square += (f64::from(ei - oi).powf(2.0) / f64::from(ei)) as f32;
        }
    }

    // Verify that the chi-square value isn't too big. According to
    // http://en.wikipedia.org/wiki/Chi-squared_distribution#Table_of_.CF.872_value_vs_p-value,
    // we basically verify that there is a really small chance of hitting a very
    // bad sample (p-value < 0.05), for n-degrees of freedom. The number 'n' depends
    // on the number of segments.
    Assert::assert_true_msg(
        &format!(
            "chisquare not statistically significant enough: {}",
            randomizedtesting::jdk::lang::float_to_string(chi_square)
        ),
        chi_square < CHI_SQUARE_VALUES[num_sampled_docs.len()],
    );

    // Test amortized counts - should be 5X the sampled count, but maximum numDocs/10
    let random10_facet_counts = FastTaxonomyFacetCounts::new(
        Arc::clone(&taxo_reader),
        Arc::clone(&config),
        &random10_percent,
    )?;
    let random10_result = random10_facet_counts.get_top_children(10, "iMod10", &[])?;
    let amortized10_result = random10_percent
        .amortize_facet_counts(random10_result.clone(), &config, &searcher)?
        .unwrap();
    let random10_result = random10_result.unwrap();
    for (amortized, sampled) in amortized10_result
        .label_values
        .iter()
        .zip(&random10_result.label_values)
    {
        // since numDocs may not divide by 10 exactly, allow for some slack in the amortized count
        Assert::assert_equals_double_msg(
            None,
            f64::from(number_float_value(&amortized.value)),
            f64::from((5.0 * number_float_value(&sampled.value)).min(num_docs as f32 / 10.0)),
            1.0,
        );
    }

    searcher.get_index_reader().close()?;
    taxo_reader.close()?;
    dir.close()?;
    taxo_dir.close()
}
