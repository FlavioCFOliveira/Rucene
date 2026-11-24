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

//! The fragments/passages portability test of `lucene/highlighter` (rmp
//! #629): `HighlighterOracle.java fragments` (Lucene 10.5.0) and Rucene, from
//! one seed, index the same twelve texts and give, for the same eight
//! queries (term, phrase, two-term disjunction), the same hits and score
//! bits and the same output of `Highlighter`, `FastVectorHighlighter`,
//! `UnifiedHighlighter` and `MatchHighlighter`. The positive control shows
//! that one changed fragment fails the comparison.

#[path = "../../../tests/sprint17_oracle/mod.rs"]
mod sprint17_oracle;

use std::sync::Arc;

use randomizedtesting::jdk::random::{JdkRandom, Random};
use rucene::analysis::standard::StandardAnalyzer;
use rucene::analysis::Analyzer;
use rucene::document::{Document, Field, FieldType, TextField};
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::{IndexOptions, IndexReader, Term};
use rucene::search::boolean_clause::Occur;
use rucene::search::boolean_query::Builder as BooleanQueryBuilder;
use rucene::search::phrase_query::PhraseQuery;
use rucene::search::query::Query;
use rucene::search::{IndexSearcher, TermQuery};
use rucene::store::{ByteBuffersDirectory, Directory};
use rucene_highlighter::highlight::highlighter::Highlighter;
use rucene_highlighter::highlight::query_scorer::QueryScorer;
use rucene_highlighter::highlight::simple_fragmenter::SimpleFragmenter;
use rucene_highlighter::matchhighlight::field_value_highlighters::FieldValueHighlighters;
use rucene_highlighter::matchhighlight::match_highlighter::MatchHighlighter;
use rucene_highlighter::matchhighlight::passage_formatter::PassageFormatter;
use rucene_highlighter::uhighlight::unified_highlighter::UnifiedHighlighter;
use rucene_highlighter::vectorhighlight::fast_vector_highlighter::FastVectorHighlighter;
use sprint17_oracle::{bits32, corpus, escape, Module};

const SEEDS: [i64; 3] = [0x0629_0001, 0x0629_0002, 0x0629_0003];

/// `escape(s)` of a nullable string.
fn escape_opt(s: Option<&str>) -> String {
    s.map_or_else(|| "null".to_string(), escape)
}

/// `HighlighterOracle.fragments`.
fn rust_fragments(seed: i64) -> String {
    let r = JdkRandom::new(seed);
    let analyzer: Arc<dyn Analyzer> = Arc::new(StandardAnalyzer::new());
    let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
    let ft = FieldType::new_from(TextField::type_stored());
    ft.set_index_options(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS)
        .unwrap();
    ft.set_store_term_vectors(true).unwrap();
    ft.set_store_term_vector_positions(true).unwrap();
    ft.set_store_term_vector_offsets(true).unwrap();
    ft.freeze();
    let ft = Arc::new(ft);
    let num_docs = 12;
    let mut texts = Vec::with_capacity(num_docs);
    {
        let w = IndexWriter::new(
            Arc::clone(&dir),
            IndexWriterConfig::with_analyzer(Some(Arc::clone(&analyzer))),
        )
        .unwrap();
        for _ in 0..num_docs {
            let n = 20 + r.next_int_bound(60) as usize;
            let text = corpus::text(&r, n);
            let mut doc = Document::new();
            doc.add(Field::new("f", text.clone(), Arc::clone(&ft)).unwrap());
            w.add_document(&doc).unwrap();
            texts.push(text);
        }
        w.close().unwrap();
    }
    let directory_reader = directory_reader::open(Arc::clone(&dir)).unwrap();
    let reader: Arc<dyn IndexReader> = directory_reader.clone();
    let searcher = Arc::new(IndexSearcher::new(Arc::clone(&reader)).unwrap());
    let mut out = String::new();
    for q in 0..8 {
        let query: Arc<dyn Query> = match r.next_int_bound(3) {
            0 => Arc::new(TermQuery::new(Term::from_text("f", corpus::word(&r)))),
            1 => {
                let words: Vec<&str> = texts[r.next_int_bound(num_docs as i32) as usize]
                    .split(' ')
                    .collect();
                let at = r.next_int_bound(words.len() as i32 - 1) as usize;
                Arc::new(PhraseQuery::with_slop(0, "f", &[words[at], words[at + 1]]).unwrap())
            }
            _ => {
                let mut b = BooleanQueryBuilder::new();
                b.add(
                    Arc::new(TermQuery::new(Term::from_text("f", corpus::word(&r)))),
                    Occur::SHOULD,
                )
                .unwrap();
                b.add(
                    Arc::new(TermQuery::new(Term::from_text("f", corpus::word(&r)))),
                    Occur::SHOULD,
                )
                .unwrap();
                Arc::new(b.build())
            }
        };
        let top = searcher
            .search(Arc::clone(&query), num_docs as i32)
            .unwrap();
        out.push_str(&format!(
            "query {q} {} {}\n",
            escape(&query.to_query_string("")),
            top.score_docs.len()
        ));
        let fvh = FastVectorHighlighter::new();
        let field_query = fvh
            .get_field_query_with_reader(&query, Some(&reader))
            .unwrap();
        for sd in &top.score_docs {
            let text = &texts[sd.doc as usize];
            let mut highlighter = Highlighter::new(Box::new(QueryScorer::with_field(
                Arc::clone(&query),
                Some("f"),
            )));
            highlighter.set_text_fragmenter(Box::new(SimpleFragmenter::with_fragment_size(20)));
            let hl = highlighter
                .get_best_fragments(&*analyzer, "f", text, 3)
                .unwrap();
            out.push_str(&format!("doc {} {}\n", sd.doc, bits32(sd.score)));
            for fragment in &hl {
                out.push_str(&format!("hl {}\n", escape(fragment)));
            }
            let vh = fvh
                .get_best_fragments(&field_query, &*reader, sd.doc, "f", 30, 3)
                .unwrap()
                .unwrap_or_default();
            for fragment in &vh {
                out.push_str(&format!("fvh {}\n", escape(fragment)));
            }
        }
        let uh = UnifiedHighlighter::builder(Arc::clone(&searcher), Arc::clone(&analyzer)).build();
        let passages = uh.highlight_max_passages("f", &query, &top, 2).unwrap();
        for (i, passage) in passages.iter().enumerate() {
            out.push_str(&format!(
                "uh {} {}\n",
                top.score_docs[i].doc,
                escape_opt(passage.as_deref())
            ));
        }
        let mut mh = MatchHighlighter::new(Arc::clone(&searcher), Arc::clone(&analyzer)).unwrap();
        mh.append_field_highlighter(FieldValueHighlighters::highlighted(
            60,
            2,
            PassageFormatter::new("...", ">", "<"),
            Arc::new(|_: &str| true),
        ));
        for dh in mh.highlight(&top, &[Arc::clone(&query)]).unwrap() {
            out.push_str(&format!("mh {}\n", dh.doc_id));
            for (field, values) in &dh.fields {
                for value in values {
                    out.push_str(&format!("mh {field} {}\n", escape(value)));
                }
            }
        }
    }
    directory_reader.close().unwrap();
    out
}

#[test]
fn highlighter_fragments_oracle() {
    sprint17_oracle::pin_tests_seed();
    for seed in SEEDS {
        let java = sprint17_oracle::run(Module::Highlighter, "fragments", seed, &[]);
        let rust = rust_fragments(seed);
        let lines = java.lines().count();
        for kind in ["hl ", "fvh ", "uh ", "mh "] {
            assert!(
                java.lines().any(|l| l.starts_with(kind)),
                "seed {seed}: the corpus exercises {kind}"
            );
        }
        println!("seed {seed}: {lines} lines");
        sprint17_oracle::assert_same(&java, &rust);
    }
}

/// Positive control: one changed fragment fails the comparison.
#[test]
fn highlighter_fragments_oracle_detects_a_change() {
    sprint17_oracle::pin_tests_seed();
    let java = sprint17_oracle::run(Module::Highlighter, "fragments", SEEDS[0], &[]);
    let rust = rust_fragments(SEEDS[0]);
    assert!(sprint17_oracle::compare(&java, &rust).is_ok());
    let changed = rust.replacen("<B>", "<b>", 1);
    assert_ne!(changed, rust, "the dump holds a highlighted fragment");
    assert!(sprint17_oracle::compare(&java, &changed).is_err());
}
