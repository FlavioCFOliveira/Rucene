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

//! Port of `TestSpanSimilarity.java`.

use std::fmt::Display;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::{Document, FieldType, TextField};
use rucene::error::Result;
use rucene::index::Term;
use rucene::search::query::Query;
use rucene::search::similarities::{
    AfterEffect, AfterEffectB, AfterEffectL, AxiomaticF1EXP, AxiomaticF1LOG, AxiomaticF2EXP,
    AxiomaticF2LOG, AxiomaticF3EXP, AxiomaticF3LOG, BM25Similarity, BasicModel, BasicModelG,
    BasicModelIF, BasicModelIn, BasicModelIne, BooleanSimilarity, ClassicSimilarity, DFISimilarity,
    DFRSimilarity, Distribution, DistributionLL, DistributionSPL, IBSimilarity, Independence,
    IndependenceChiSquared, IndependenceSaturated, IndependenceStandardized, LMDirichletSimilarity,
    LMJelinekMercerSimilarity, Lambda, LambdaDF, LambdaTTF, NoNormalization, Normalization,
    NormalizationH1, NormalizationH2, NormalizationH3, NormalizationZ, Similarity,
};
use rucene_queries::spans::span_or_query::SpanOrQuery;
use rucene_queries::spans::span_term_query::SpanTermQuery;
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::util::LuceneTestCase;

use crate::queries_support::core_support::random;
use crate::queries_support::search_support::{fixture_class, ftest, object_to_string, set_up};

/// The instance of `TestSpanSimilarity`: `sims`, each with its
/// `toString()`.
#[derive(Default)]
pub struct SpanSimilarity {
    sims: Vec<(Arc<dyn Similarity>, String)>,
}

/// Adds a similarity whose class overrides `toString()`.
fn push_sim<S: Similarity + Display + 'static>(
    sims: &mut Vec<(Arc<dyn Similarity>, String)>,
    s: S,
) {
    let name = s.to_string();
    sims.push((Arc::new(s), name));
}

/// The DFR basic models to test (`BASIC_MODELS`).
fn basic_models() -> Vec<fn() -> Box<dyn BasicModel>> {
    vec![
        || Box::new(BasicModelG::new()),
        || Box::new(BasicModelIF::new()),
        || Box::new(BasicModelIn::new()),
        || Box::new(BasicModelIne::new()),
    ]
}

/// The DFR aftereffects to test (`AFTER_EFFECTS`).
fn after_effects() -> Vec<fn() -> Box<dyn AfterEffect>> {
    vec![|| Box::new(AfterEffectB::new()), || {
        Box::new(AfterEffectL::new())
    }]
}

/// `NORMALIZATIONS`.
fn normalizations() -> Vec<fn() -> Box<dyn Normalization>> {
    vec![
        || Box::new(NormalizationH1::default()),
        || Box::new(NormalizationH2::default()),
        || Box::new(NormalizationH3::default()),
        || Box::new(NormalizationZ::default()),
        || Box::new(NoNormalization::new()),
    ]
}

/// The distributions for IB (`DISTRIBUTIONS`).
fn distributions() -> Vec<fn() -> Box<dyn Distribution>> {
    vec![|| Box::new(DistributionLL::new()), || {
        Box::new(DistributionSPL::new())
    }]
}

/// Lambdas for IB (`LAMBDAS`).
fn lambdas() -> Vec<fn() -> Box<dyn Lambda>> {
    vec![|| Box::new(LambdaDF::new()), || Box::new(LambdaTTF::new())]
}

/// Independence measures for DFI (`INDEPENDENCE_MEASURES`).
fn independence_measures() -> Vec<fn() -> Box<dyn Independence>> {
    vec![
        || Box::new(IndependenceStandardized::new()),
        || Box::new(IndependenceSaturated::new()),
        || Box::new(IndependenceChiSquared::new()),
    ]
}

/// `TestSpanSimilarity`.
pub fn class_def() -> Arc<Class> {
    fixture_class::<SpanSimilarity>(
        "org.apache.lucene.queries.spans.TestSpanSimilarity",
        vec![set_up(do_set_up), ftest("testCrazySpans", test_crazy_spans)],
    )
}

fn do_set_up(f: &mut SpanSimilarity) -> Result<()> {
    let mut sims = Vec::new();
    push_sim(&mut sims, ClassicSimilarity::new());
    push_sim(&mut sims, BM25Similarity::new());
    let boolean = BooleanSimilarity::new();
    let name = object_to_string(
        "org.apache.lucene.search.similarities.BooleanSimilarity",
        &boolean,
    );
    sims.push((Arc::new(boolean), name));
    push_sim(&mut sims, AxiomaticF1EXP::new());
    push_sim(&mut sims, AxiomaticF1LOG::new());
    push_sim(&mut sims, AxiomaticF2EXP::new());
    push_sim(&mut sims, AxiomaticF2LOG::new());
    push_sim(&mut sims, AxiomaticF3EXP::with_s_and_query_len(0.25, 3)?);
    push_sim(&mut sims, AxiomaticF3LOG::with_s_and_query_len(0.25, 3)?);
    // TODO: not great that we dup this all with TestSimilarityBase
    for basic_model in basic_models() {
        for after_effect in after_effects() {
            for normalization in normalizations() {
                push_sim(
                    &mut sims,
                    DFRSimilarity::new(basic_model(), after_effect(), normalization()),
                );
            }
        }
    }
    for distribution in distributions() {
        for lambda in lambdas() {
            for normalization in normalizations() {
                push_sim(
                    &mut sims,
                    IBSimilarity::new(distribution(), lambda(), normalization()),
                );
            }
        }
    }
    push_sim(&mut sims, LMDirichletSimilarity::new());
    push_sim(&mut sims, LMJelinekMercerSimilarity::with_lambda(0.1)?);
    push_sim(&mut sims, LMJelinekMercerSimilarity::with_lambda(0.7)?);
    for independence in independence_measures() {
        push_sim(&mut sims, DFISimilarity::new(independence()));
    }
    f.sims = sims;
    Ok(())
}

/// make sure all sims work with spanOR(termX, termY) where termY does not
/// exist
fn test_crazy_spans(f: &mut SpanSimilarity) -> Result<()> {
    // historically this was a problem, but sim's no longer have to score terms that dont exist
    let dir = LuceneTestCase::new_directory();
    let iw = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    let mut doc = Document::new();
    let ft = FieldType::new_from(TextField::type_not_stored());
    doc.add(Box::new(LuceneTestCase::new_field("foo", "bar", &ft)));
    iw.add_document(&doc)?;
    let ir = iw.get_reader()?;
    iw.close()?;
    let mut is = LuceneTestCase::new_searcher(Arc::clone(&ir));

    for (sim, sim_string) in &f.sims {
        is.set_similarity(Arc::clone(sim));
        let s1 = Arc::new(SpanTermQuery::new(Term::from_text("foo", "bar")));
        let s2 = Arc::new(SpanTermQuery::new(Term::from_text("foo", "baz")));
        let query: Arc<dyn Query> = Arc::new(SpanOrQuery::new(vec![s1, s2])?);
        let td = is.search(query, 10)?;
        Assert::assert_equals(1i64, td.total_hits.value());
        let score = td.score_docs[0].score;
        Assert::assert_false_msg(&format!("negative score for {sim_string}"), score < 0.0);
        Assert::assert_false_msg(&format!("inf score for {sim_string}"), score.is_infinite());
        Assert::assert_false_msg(&format!("nan score for {sim_string}"), score.is_nan());
    }
    ir.close()?;
    dir.close()
}
