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

//! Port of `TestMatchHighlighter.java`.

use std::collections::HashSet;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::{Class, Constructor};
use randomizedtesting::junit::{Before, Test};
use randomizedtesting::RandomizedTest;
use rucene::analysis::standard::new_standard_tokenizer;
use rucene::analysis::{new_lower_case_filter, Analyzer};
use rucene::document::{Field, FieldType, TextField};
use rucene::error::{LuceneError, Result};
use rucene::index::{DirectoryReader, IndexOptions, IndexReader, IndexableField};
use rucene::search::boolean_clause::Occur;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::match_all_docs_query::MatchAllDocsQuery;
use rucene::search::phrase_query::PhraseQuery;
use rucene::search::query::Query;
use rucene::search::sort::{Sort, SortField};
use rucene::search::{TermQuery, TopDocs};
use rucene::util::java_collections::JavaHashMap;
use rucene::util::CharsRef;
use rucene_analysis_common::analysis::core::whitespace_tokenizer::new_whitespace_tokenizer;
use rucene_analysis_common::analysis::en::porter_stem_filter::new_porter_stem_filter;
use rucene_analysis_common::analysis::miscellaneous::per_field_analyzer_wrapper::PerFieldAnalyzerWrapper;
use rucene_analysis_common::analysis::synonym::synonym_graph_filter::new_synonym_graph_filter;
use rucene_analysis_common::analysis::synonym::synonym_map::{
    Builder as SynonymMapBuilder, SynonymMap,
};
use rucene_highlighter::matchhighlight::field_value_highlighters::FieldValueHighlighters;
use rucene_highlighter::matchhighlight::match_highlighter::{
    or, DocHighlights, FieldValueHighlighter, MatchHighlighter, QueryOffsetRange,
};
use rucene_highlighter::matchhighlight::offset_range::OffsetRange;
use rucene_highlighter::matchhighlight::passage_formatter::{MarkerFunction, PassageFormatter};
use rucene_highlighter::matchhighlight::passage_selector::PassageSelector;
use rucene_queries::intervals::interval_query::IntervalQuery;
use rucene_queries::intervals::intervals::Intervals;
use rucene_queryparser::flexible::standard::standard_query_parser::StandardQueryParser;
use rucene_test_framework::util::LuceneTestCase;

use crate::highlighter_support::core_support::assertion_error;
use crate::highlighter_support::search_support::term;
use crate::highlighter_support::{chain, instance_class, sink, ti, FnAnalyzer};
use crate::matchhighlight::analyzer_with_gaps::analyzer_with_gaps;
use crate::matchhighlight::index_builder::{IndexBuilder, ToField};
use crate::matchhighlight::missing_analyzer::MissingAnalyzer;

const FLD_ID: &str = "id";
const FLD_TEXT1: &str = "text1";
const FLD_TEXT2: &str = "text2";

type T = TestMatchHighlighter;

/// `TestMatchHighlighter`.
pub fn class_def() -> Arc<Class> {
    let t = |name: &str, body: fn(&mut T) -> Result<()>| ti::<T>(name, body).annotate(Test::new());
    instance_class(
        "org.apache.lucene.search.matchhighlight.TestMatchHighlighter",
        &LuceneTestCase::class(),
        Constructor::no_args(T::default),
        vec![
            ti::<T>("setup", T::setup).annotate(Before),
            t("testBasicUsage", T::test_basic_usage),
            t("testSynonymHighlight", T::test_synonym_highlight),
            t("testAnalyzedTextIntervals", T::test_analyzed_text_intervals),
            t(
                "testStandardQueryParserIntervalFunctions",
                T::test_standard_query_parser_interval_functions,
            ),
            t(
                "testCustomFieldHighlightHandling",
                T::test_custom_field_highlight_handling,
            ),
            t(
                "testHighlightMoreQueriesAtOnceShowoff",
                T::test_highlight_more_queries_at_once_showoff,
            ),
        ],
    )
}

/// The `TestMatchHighlighter` instance.
#[derive(Default)]
pub struct TestMatchHighlighter {
    type_text_positions_offsets: Option<Arc<FieldType>>,
    type_text_positions: Option<Arc<FieldType>>,
    analyzer: Option<Arc<dyn Analyzer>>,
}

/// `buildSynonymMap(String[][] synonyms)`.
pub fn build_synonym_map(synonyms: &[[&str; 2]]) -> Result<SynonymMap> {
    let mut builder = SynonymMapBuilder::new();
    for pair in synonyms {
        builder.add(
            &CharsRef::from_string(pair[0]),
            &CharsRef::from_string(pair[1]),
            true,
        )?;
    }
    builder.build()
}

/// `Sort.INDEXORDER`.
fn index_order() -> Sort {
    Sort::new_fields(vec![SortField::FIELD_DOC.clone()]).expect("INVARIANT: one sort field")
}

fn search(searcher: &IndexSearcher, query: &Arc<dyn Query>) -> Result<TopDocs> {
    Ok(searcher
        .search_with_sort(Arc::clone(query), 10, &index_order())?
        .to_top_docs())
}

fn new_searcher(reader: &Arc<dyn DirectoryReader>) -> Result<Arc<IndexSearcher>> {
    Ok(Arc::new(IndexSearcher::new(
        Arc::clone(reader) as Arc<dyn IndexReader>
    )?))
}

fn tq(field: &str, text: &str) -> Arc<dyn Query> {
    Arc::new(TermQuery::new(term(field, text)))
}

fn equals_field(name: &'static str) -> Arc<dyn Fn(&str) -> bool + Send + Sync> {
    Arc::new(move |f: &str| f == name)
}

fn formatter() -> PassageFormatter {
    PassageFormatter::new("...", ">", "<")
}

fn set_of(fields: &[&str]) -> HashSet<String> {
    fields.iter().map(|f| (*f).to_string()).collect()
}

/// `new MatchHighlighter(searcher, analyzer)` with `highlighters` appended.
fn match_highlighter(
    searcher: &Arc<IndexSearcher>,
    analyzer: &Arc<dyn Analyzer>,
    highlighters: Vec<Arc<dyn FieldValueHighlighter>>,
) -> Result<MatchHighlighter> {
    let mut highlighter = MatchHighlighter::new(Arc::clone(searcher), Arc::clone(analyzer))?;
    for h in highlighters {
        highlighter.append_field_highlighter(h);
    }
    Ok(highlighter)
}

/// `assertHighlights(docList, expectedFormattedLines...)`.
fn assert_highlights(doc_list: &[Vec<String>], expected_formatted_lines: &[String]) -> Result<()> {
    let mut actual_lines: Vec<String> = Vec::new();
    for (doc, fields) in doc_list.iter().enumerate() {
        for (i, f) in fields.iter().enumerate() {
            actual_lines.push(if i == 0 {
                format!("{doc:2}. {f}")
            } else {
                format!("    {f}")
            });
        }
    }

    let expected_trimmed: Vec<&str> = expected_formatted_lines
        .iter()
        .map(|s| s.trim_matches(|c: char| c <= ' '))
        .collect();
    let actual_trimmed: Vec<&str> = actual_lines
        .iter()
        .map(|s| s.trim_matches(|c: char| c <= ' '))
        .collect();
    if expected_trimmed != actual_trimmed {
        return Err(assertion_error(format!(
            "Actual hits were:\n{}\n\nbut expected them to be:\n{}",
            actual_trimmed.join("\n"),
            expected_trimmed.join("\n")
        )));
    }
    Ok(())
}

fn lines(expected: &[&str]) -> Vec<String> {
    expected.iter().map(|s| (*s).to_string()).collect()
}

/// `toDocList(highlights)`.
fn to_doc_list(highlights: Vec<DocHighlights>) -> Vec<Vec<String>> {
    highlights
        .into_iter()
        .map(|doc_highlights| {
            doc_highlights
                .fields
                .iter()
                .map(|(k, v)| format!("{k}: {}", v.join(", ")))
                .collect()
        })
        .collect()
}

impl TestMatchHighlighter {
    fn analyzer(&self) -> Arc<dyn Analyzer> {
        Arc::clone(self.analyzer.as_ref().expect("setup ran"))
    }

    fn setup(&mut self) -> Result<()> {
        self.type_text_positions = Some(Arc::new({
            // TextField.TYPE_STORED itself
            let ft = FieldType::new_from(TextField::type_stored());
            ft.freeze();
            ft
        }));

        let ft = FieldType::new_from(TextField::type_stored());
        ft.set_index_options(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS)?;
        ft.freeze();
        self.type_text_positions_offsets = Some(Arc::new(ft));

        // Create an analyzer with some synonyms, just to showcase them.
        let synonym_map = Arc::new(build_synonym_map(&[
            ["moon\u{0000}shine", "firewater"],
            ["firewater", "moon\u{0000}shine"],
        ])?);

        // Make a non-empty offset gap so that break iterator doesn't go haywire on multivalues
        // glued together.
        let offset_gap = RandomizedTest::random_int_between(1, 2);
        let position_gap = *RandomizedTest::random_from(&[0, 1, 100]);
        let synonyms_analyzer = analyzer_with_gaps(
            offset_gap,
            position_gap,
            FnAnalyzer::new(
                "org.apache.lucene.search.matchhighlight.TestMatchHighlighter$1",
                move |_field_name| {
                    let map = Arc::clone(&synonym_map);
                    chain(new_whitespace_tokenizer(), move |input| {
                        Ok(sink(new_synonym_graph_filter(input, map, true)?))
                    })
                },
            )
            .arc(),
        );

        let mut field_analyzers: JavaHashMap<Arc<dyn Analyzer>> = JavaHashMap::new();
        field_analyzers.insert(FLD_TEXT1.to_string(), Arc::clone(&synonyms_analyzer));
        field_analyzers.insert(FLD_TEXT2.to_string(), synonyms_analyzer);

        self.analyzer = Some(Arc::new(PerFieldAnalyzerWrapper::new_with_fields(
            Arc::new(MissingAnalyzer::new()),
            Some(field_analyzers),
        )));
        Ok(())
    }

    /// `this::toField`.
    fn to_field(&self) -> ToField {
        let positions_offsets = Arc::clone(
            self.type_text_positions_offsets
                .as_ref()
                .expect("setup ran"),
        );
        let positions = Arc::clone(self.type_text_positions.as_ref().expect("setup ran"));
        Arc::new(move |name: &str, value: &str| -> Arc<dyn IndexableField> {
            let field = match name {
                FLD_TEXT1 => Field::new(
                    name.to_string(),
                    value.to_string(),
                    Arc::clone(&positions_offsets),
                ),
                FLD_TEXT2 => {
                    Field::new(name.to_string(), value.to_string(), Arc::clone(&positions))
                }
                _ => {
                    panic!("java.lang.AssertionError: Don't know how to handle this field: {name}")
                }
            };
            Arc::new(field.expect("INVARIANT: a valid field"))
        })
    }

    fn builder(&self) -> IndexBuilder {
        IndexBuilder::new(self.to_field())
    }

    fn test_basic_usage(&mut self) -> Result<()> {
        let analyzer = self.analyzer();
        self.builder()
            .doc(FLD_TEXT1, &["foo bar baz"])
            .doc(FLD_TEXT1, &["bar foo baz"])
            .doc_with(|fields| {
                fields.add(FLD_TEXT1, &["Very long content but not matching anything."]);
                fields.add(FLD_TEXT2, &["no foo but bar"]);
            })
            .build(self.analyzer(), |reader| {
                let query = {
                    let mut b = rucene::search::boolean_query::Builder::new();
                    b.add(tq(FLD_TEXT1, "foo"), Occur::SHOULD)?;
                    b.add(tq(FLD_TEXT2, "bar"), Occur::SHOULD)?;
                    Arc::new(b.build()) as Arc<dyn Query>
                };

                // In the most basic scenario, we run a search against a query, retrieve
                // top docs...
                let searcher = new_searcher(&reader)?;
                // Sort.INDEXORDER: so that results are consistently ordered.
                let top_docs = search(&searcher, &query)?;

                // ...and would want a fixed set of fields from those documents, some of them
                // possibly highlighted if they matched the query.
                //
                // This configures the highlighter so that the FLD_ID field is always returned
                // verbatim,
                // and FLD_TEXT1 is returned *only if it contained a query match*.
                let mut highlighter = match_highlighter(
                    &searcher,
                    &analyzer,
                    vec![
                        FieldValueHighlighters::verbatim_value(FLD_ID, &[]),
                        FieldValueHighlighters::highlighted(
                            80 * 3,
                            1,
                            formatter(),
                            equals_field(FLD_TEXT1),
                        ),
                        FieldValueHighlighters::skip_remaining(),
                    ],
                )?;

                // Note document field highlights are a stream over documents in topDocs. In the
                // remaining code we will just
                // collect them on the fly into a preformatted string.
                let highlights = highlighter.highlight(&top_docs, &[Arc::clone(&query)])?;
                assert_highlights(
                    &to_doc_list(highlights),
                    &lines(&[
                        " 0. id: 0",
                        "    text1: >foo< bar baz",
                        " 1. id: 1",
                        "    text1: bar >foo< baz",
                        " 2. id: 2",
                    ]),
                )?;

                // In a more realistic use case, you'd want to show the value of a given field
                // *regardless* of whether it
                // contained a highlight or not -- it is odd that document "id: 2" above doesn't have
                // the 'text1' field
                // shown because that field wasn't part of the query match.
                //
                // Let's say the field is also potentially long; if it contains a match,
                // we would want to display the contextual snippet surrounding that match. If it does
                // not contain any
                // matches, we would want to display its content up to a given number of characters
                // (lead lines).
                //
                // Let's do this by adding an appropriate field highlighter on FLD_TEXT1.
                highlighter = match_highlighter(
                    &searcher,
                    &analyzer,
                    vec![
                        FieldValueHighlighters::verbatim_value(FLD_ID, &[]),
                        FieldValueHighlighters::highlighted(
                            80 * 3,
                            1,
                            formatter(),
                            equals_field(FLD_TEXT1),
                        ),
                        FieldValueHighlighters::max_leading_characters(
                            10,
                            "...",
                            set_of(&[FLD_TEXT1]),
                        ),
                        FieldValueHighlighters::skip_remaining(),
                    ],
                )?;

                assert_highlights(
                    &to_doc_list(highlighter.highlight(&top_docs, &[Arc::clone(&query)])?),
                    &lines(&[
                        " 0. id: 0",
                        "    text1: >foo< bar baz",
                        " 1. id: 1",
                        "    text1: bar >foo< baz",
                        " 2. id: 2",
                        "    text1: Very long...",
                    ]),
                )?;

                // Field highlighters can apply to multiple fields and be chained for convenience.
                // For example, this defines a combined highlighter over both FLD_TEXT1 and FLD_TEXT2.
                let fields = set_of(&[FLD_TEXT1, FLD_TEXT2]);
                let contains = fields.clone();
                let highlighted_or_abbreviated = or(
                    FieldValueHighlighters::highlighted(
                        80 * 3,
                        1,
                        formatter(),
                        Arc::new(move |f: &str| contains.contains(f)),
                    ),
                    FieldValueHighlighters::max_leading_characters(10, "...", fields),
                );

                highlighter = match_highlighter(
                    &searcher,
                    &analyzer,
                    vec![
                        FieldValueHighlighters::verbatim_value(FLD_ID, &[]),
                        highlighted_or_abbreviated,
                        FieldValueHighlighters::skip_remaining(),
                    ],
                )?;

                assert_highlights(
                    &to_doc_list(highlighter.highlight(&top_docs, &[query])?),
                    &lines(&[
                        " 0. id: 0",
                        "    text1: >foo< bar baz",
                        " 1. id: 1",
                        "    text1: bar >foo< baz",
                        " 2. id: 2",
                        "    text1: Very long...",
                        "    text2: no foo but >bar<",
                    ]),
                )
            })?;
        Ok(())
    }

    fn test_synonym_highlight(&mut self) -> Result<()> {
        // There is nothing special needed to highlight or process complex queries, synonyms, etc.
        // Synonyms defined in the constructor of this class.
        let analyzer = self.analyzer();
        self.builder()
            .doc(FLD_TEXT1, &["Where the moon shine falls, firewater flows."])
            .build(self.analyzer(), |reader| {
                let searcher = new_searcher(&reader)?;
                let highlighter = match_highlighter(
                    &searcher,
                    &analyzer,
                    vec![
                        FieldValueHighlighters::highlighted(
                            80 * 3,
                            1,
                            formatter(),
                            equals_field(FLD_TEXT1),
                        ),
                        FieldValueHighlighters::skip_remaining(),
                    ],
                )?;

                let mut query = tq(FLD_TEXT1, "firewater");
                assert_highlights(
                    &to_doc_list(
                        highlighter
                            .highlight(&search(&searcher, &query)?, &[Arc::clone(&query)])?,
                    ),
                    &lines(&["0. text1: Where the >moon shine< falls, >firewater< flows."]),
                )?;

                query = Arc::new(PhraseQuery::with_slop(0, FLD_TEXT1, &["moon", "shine"])?);
                assert_highlights(
                    &to_doc_list(
                        highlighter
                            .highlight(&search(&searcher, &query)?, &[Arc::clone(&query)])?,
                    ),
                    &lines(&["0. text1: Where the >moon shine< falls, >firewater< flows."]),
                )
            })?;
        Ok(())
    }

    fn test_analyzed_text_intervals(&mut self) -> Result<()> {
        let synonym_map = Arc::new(build_synonym_map(&[
            ["moon\u{0000}shine", "firewater"],
            ["firewater", "moon\u{0000}shine"],
        ])?);

        let analyzer = FnAnalyzer::new(
            "org.apache.lucene.search.matchhighlight.TestMatchHighlighter$2",
            move |_field_name| {
                let map = Arc::clone(&synonym_map);
                chain(new_standard_tokenizer(), move |ts| {
                    let ts = new_lower_case_filter(ts)?;
                    let ts = new_synonym_graph_filter(Box::new(ts), map, true)?;
                    let ts = new_porter_stem_filter(Box::new(ts))?;
                    Ok(sink(ts))
                })
            },
        )
        .arc();

        self.builder()
            .doc(FLD_TEXT1, &["Where the moon shine falls, firewater flows."])
            .build(Arc::clone(&analyzer), |reader| {
                let searcher = new_searcher(&reader)?;
                let highlighter = match_highlighter(
                    &searcher,
                    &analyzer,
                    vec![
                        FieldValueHighlighters::highlighted(
                            80 * 3,
                            1,
                            formatter(),
                            equals_field(FLD_TEXT1),
                        ),
                        FieldValueHighlighters::skip_remaining(),
                    ],
                )?;

                {
                    // [moon shine, firewater] are synonyms, tokens are lowercased. Porter stemming on.
                    let query: Arc<dyn Query> = Arc::new(IntervalQuery::new(
                        FLD_TEXT1,
                        Intervals::analyzed_text("Firewater Fall", &*analyzer, FLD_TEXT1, 0, true)?,
                    ));

                    assert_highlights(
                        &to_doc_list(
                            highlighter
                                .highlight(&search(&searcher, &query)?, &[Arc::clone(&query)])?,
                        ),
                        &lines(&["0. text1: Where the >moon shine falls<, firewater flows."]),
                    )?;
                }
                Ok(())
            })?;
        Ok(())
    }

    fn test_standard_query_parser_interval_functions(&mut self) -> Result<()> {
        let analyzer = FnAnalyzer::new(
            "org.apache.lucene.search.matchhighlight.TestMatchHighlighter$3",
            |_field_name| {
                chain(new_standard_tokenizer(), |ts| {
                    Ok(sink(new_lower_case_filter(ts)?))
                })
            },
        )
        .arc();

        // Rerun the same test on fields with offsets and without offsets.
        for field in [FLD_TEXT1, FLD_TEXT2] {
            let input_document = "The quick brown fox jumps over the lazy dog";

            let query_result_pairs: &[&[&str]] = &[
                &[
                    "fn:ordered(brown dog)",
                    "0. %s: The quick >brown fox jumps over the lazy dog<",
                ],
                &[
                    "fn:within(fn:or(lazy quick) 1 fn:or(dog fox))",
                    "0. %s: The quick brown fox jumps over the >lazy< dog",
                ],
                &[
                    "fn:containedBy(fox fn:ordered(brown fox dog))",
                    "0. %s: The quick brown >fox< jumps over the lazy dog",
                ],
                &[
                    "fn:atLeast(2 quick fox \"furry dog\")",
                    "0. %s: The >quick brown fox< jumps over the lazy dog",
                ],
                &[
                    "fn:maxgaps(0 fn:ordered(fn:or(quick lazy) fn:or(fox dog)))",
                    "0. %s: The quick brown fox jumps over the >lazy dog<",
                ],
                &[
                    "fn:maxgaps(1 fn:ordered(fn:or(quick lazy) fn:or(fox dog)))",
                    "0. %s: The >quick brown fox< jumps over the >lazy dog<",
                ],
                &[
                    "fn:maxwidth(2 fn:ordered(fn:or(quick lazy) fn:or(fox dog)))",
                    "0. %s: The quick brown fox jumps over the >lazy dog<",
                ],
                &[
                    "fn:maxwidth(3 fn:ordered(fn:or(quick lazy) fn:or(fox dog)))",
                    "0. %s: The >quick brown fox< jumps over the >lazy dog<",
                ],
                &[
                    "fn:or(quick \"fox\")",
                    "0. %s: The >quick< brown >fox< jumps over the lazy dog",
                ],
                &["fn:or(\"quick fox\")"],
                &[
                    "fn:phrase(quick brown fox)",
                    "0. %s: The >quick brown fox< jumps over the lazy dog",
                ],
                &[
                    "fn:wildcard(jump*)",
                    "0. %s: The quick brown fox >jumps< over the lazy dog",
                ],
                &[
                    "fn:wildcard(br*n)",
                    "0. %s: The quick >brown< fox jumps over the lazy dog",
                ],
                &[
                    "fn:fuzzyTerm(fxo)",
                    "0. %s: The quick brown >fox< jumps over the lazy dog",
                ],
                &[
                    "fn:or(dog fox)",
                    "0. %s: The quick brown >fox< jumps over the lazy >dog<",
                ],
                &[
                    "fn:phrase(fn:ordered(quick fox) jumps)",
                    "0. %s: The >quick brown fox jumps< over the lazy dog",
                ],
                &[
                    "fn:ordered(quick jumps dog)",
                    "0. %s: The >quick brown fox jumps over the lazy dog<",
                ],
                &[
                    "fn:ordered(quick fn:or(fox dog))",
                    "0. %s: The >quick brown fox< jumps over the lazy dog",
                ],
                &[
                    "fn:ordered(quick jumps fn:or(fox dog))",
                    "0. %s: The >quick brown fox jumps over the lazy dog<",
                ],
                &[
                    "fn:unordered(dog jumps quick)",
                    "0. %s: The >quick brown fox jumps over the lazy dog<",
                ],
                &[
                    "fn:unordered(fn:or(fox dog) quick)",
                    "0. %s: The >quick brown fox< jumps over the lazy dog",
                ],
                &[
                    "fn:unordered(fn:phrase(brown fox) fn:phrase(fox jumps))",
                    "0. %s: The quick >brown fox jumps< over the lazy dog",
                ],
                &["fn:ordered(fn:phrase(brown fox) fn:phrase(fox jumps))"],
                &["fn:unorderedNoOverlaps(fn:phrase(brown fox) fn:phrase(fox jumps))"],
                &[
                    "fn:before(fn:or(brown lazy) fox)",
                    "0. %s: The quick >brown< fox jumps over the lazy dog",
                ],
                &[
                    "fn:before(fn:or(brown lazy) fn:or(dog fox))",
                    "0. %s: The quick >brown< fox jumps over the >lazy< dog",
                ],
                &[
                    "fn:after(fn:or(brown lazy) fox)",
                    "0. %s: The quick brown fox jumps over the >lazy< dog",
                ],
                &[
                    "fn:after(fn:or(brown lazy) fn:or(dog fox))",
                    "0. %s: The quick brown fox jumps over the >lazy< dog",
                ],
                &[
                    "fn:within(fn:or(fox dog) 1 fn:or(quick lazy))",
                    "0. %s: The quick brown fox jumps over the lazy >dog<",
                ],
                &[
                    "fn:within(fn:or(fox dog) 2 fn:or(quick lazy))",
                    "0. %s: The quick brown >fox< jumps over the lazy >dog<",
                ],
                &[
                    "fn:notWithin(fn:or(fox dog) 1 fn:or(quick lazy))",
                    "0. %s: The quick brown >fox< jumps over the lazy dog",
                ],
                &[
                    "fn:containedBy(fn:or(fox dog) fn:ordered(quick lazy))",
                    "0. %s: The quick brown >fox< jumps over the lazy dog",
                ],
                &[
                    "fn:notContainedBy(fn:or(fox dog) fn:ordered(quick lazy))",
                    "0. %s: The quick brown fox jumps over the lazy >dog<",
                ],
                &[
                    "fn:containing(fn:atLeast(2 quick fox dog) jumps)",
                    "0. %s: The quick brown >fox jumps over the lazy dog<",
                ],
                &[
                    "fn:notContaining(fn:ordered(fn:or(the The) fn:or(fox dog)) brown)",
                    "0. %s: The quick brown fox jumps over >the lazy dog<",
                ],
                &[
                    "fn:overlapping(fn:phrase(brown fox) fn:phrase(fox jumps))",
                    "0. %s: The quick >brown fox< jumps over the lazy dog",
                ],
                &[
                    "fn:overlapping(fn:or(fox dog) fn:extend(lazy 2 2))",
                    "0. %s: The quick brown fox jumps over the lazy >dog<",
                ],
                &[
                    "fn:nonOverlapping(fn:phrase(brown fox) fn:phrase(lazy dog))",
                    "0. %s: The quick >brown fox< jumps over the lazy dog",
                ],
                &[
                    "fn:nonOverlapping(fn:or(fox dog) fn:extend(lazy 2 2))",
                    "0. %s: The quick brown >fox< jumps over the lazy dog",
                ],
                &[
                    "fn:atLeast(2 fn:unordered(furry dog) fn:unordered(brown dog) lazy quick)",
                    "0. %s: The >quick >brown fox jumps over the lazy<<> dog<",
                ],
                &[
                    "fn:extend(fox 1 2)",
                    "0. %s: The quick >brown fox jumps over< the lazy dog",
                ],
                &[
                    "fn:extend(fn:or(dog fox) 2 0)",
                    "0. %s: The >quick brown fox< jumps over >the lazy dog<",
                ],
                &[
                    "fn:containedBy(fn:or(fox dog) fn:extend(lazy 3 3))",
                    "0. %s: The quick brown fox jumps over the lazy >dog<",
                ],
                &[
                    "fn:notContainedBy(fn:or(fox dog) fn:extend(lazy 3 3))",
                    "0. %s: The quick brown >fox< jumps over the lazy dog",
                ],
                &[
                    "fn:containing(fn:extend(fn:or(lazy brown) 1 1) fn:or(fox dog))",
                    "0. %s: The >quick brown fox< jumps over >the lazy dog<",
                ],
                &[
                    "fn:notContaining(fn:extend(fn:or(fox dog) 1 0) fn:or(brown yellow))",
                    "0. %s: The quick brown fox jumps over the >lazy dog<",
                ],
            ];

            // Verify assertions.
            self.builder()
                // Just one document and multiple interval queries to check.
                .doc(field, &[input_document])
                .build(Arc::clone(&analyzer), |reader| {
                    let searcher = new_searcher(&reader)?;
                    let highlighter = match_highlighter(
                        &searcher,
                        &analyzer,
                        vec![
                            FieldValueHighlighters::highlighted(
                                80 * 3,
                                1,
                                formatter(),
                                Arc::new(|_fld: &str| true),
                            ),
                            FieldValueHighlighters::skip_remaining(),
                        ],
                    )?;

                    let qp = StandardQueryParser::with_analyzer(Some(Arc::clone(&analyzer)));

                    // Run all pairs of query-expected highlight.
                    let mut errors: Vec<String> = Vec::new();
                    for query_highlight_pair in query_result_pairs {
                        assert!(!query_highlight_pair.is_empty());
                        let query_string = query_highlight_pair[0];
                        let query = qp
                            .parse(query_string, Some(field))?
                            .expect("NullPointerException: no query");
                        let expected: Vec<String> = query_highlight_pair[1..]
                            .iter()
                            .map(|v| v.replacen("%s", field, 1))
                            .collect();

                        let checked =
                            assert_highlights(
                                &to_doc_list(highlighter.highlight(
                                    &search(&searcher, &query)?,
                                    &[Arc::clone(&query)],
                                )?),
                                &expected,
                            );
                        if let Err(e) = checked {
                            if !rucene_test_framework::util::java_exceptions::is_instance(
                                &e,
                                "java.lang.AssertionError",
                            ) {
                                return Err(e);
                            }
                            errors.push(format!(
                                "MISMATCH: query: {query_string}\n{}",
                                assertion_message(&e)
                            ));
                        }
                    }
                    if !errors.is_empty() {
                        return Err(assertion_error(errors.join("\n\n")));
                    }
                    Ok(())
                })?;
        }
        Ok(())
    }

    fn test_custom_field_highlight_handling(&mut self) -> Result<()> {
        // Match highlighter is a showcase of individual components in this package, suitable
        // to create any kind of field-display designs.
        //
        // In this example we will build a custom field highlighting handler that
        // highlights matches over a multivalued field, shows that field's values if it received
        // no matches and limits the number of values displayed to at most 2 (with an appropriate
        // message).
        let analyzer = self.analyzer();
        self.builder()
            // Just one document, one field, four values.
            .doc(
                FLD_TEXT1,
                &["foo bar", "bar foo baz", "bar baz foo", "baz baz baz"],
            )
            .build(self.analyzer(), |reader| {
                let searcher = new_searcher(&reader)?;

                // Let's start with the simple predefined highlighter so that the field's value shows
                // and is highlighted when it was part of the hit.
                let highlighted = FieldValueHighlighters::highlighted(
                    80 * 3,
                    2,
                    formatter(),
                    equals_field(FLD_TEXT1),
                );
                let mut highlighter = match_highlighter(
                    &searcher,
                    &analyzer,
                    vec![
                        Arc::clone(&highlighted),
                        FieldValueHighlighters::skip_remaining(),
                    ],
                )?;

                let query = tq(FLD_TEXT1, "foo");
                let top_docs = search(&searcher, &query)?;

                // Note the highlighter is configured with at most 2 snippets so the match on the
                // third value ("bar baz foo") is omitted. Ellipsis isn't inserted too because
                // values are displayed in full.
                assert_highlights(
                    &to_doc_list(highlighter.highlight(&top_docs, &[Arc::clone(&query)])?),
                    &lines(&["0. text1: >foo< bar, bar >foo< baz"]),
                )?;

                // So the above works fine if the field received a match but omits it otherwise. We
                // can
                // force the display of this field by chaining with verbatim value highlighter:
                highlighter = match_highlighter(
                    &searcher,
                    &analyzer,
                    vec![
                        or(
                            Arc::clone(&highlighted),
                            FieldValueHighlighters::verbatim_value(FLD_TEXT1, &[]),
                        ),
                        FieldValueHighlighters::skip_remaining(),
                    ],
                )?;

                let match_all: Arc<dyn Query> = Arc::new(MatchAllDocsQuery::instance());
                assert_highlights(
                    &to_doc_list(highlighter.highlight(&top_docs, &[Arc::clone(&match_all)])?),
                    &lines(&["0. text1: foo bar, bar foo baz, bar baz foo, baz baz baz"]),
                )?;

                // But this is not exactly what we'd like because we want to limit the display of
                // values to the first two.
                // Let's just write a custom field highlighter handler that does it.

                // We can now chain it as usual and contemplate the result.
                highlighter = match_highlighter(
                    &searcher,
                    &analyzer,
                    vec![
                        or(
                            Arc::clone(&highlighted),
                            Arc::new(AtMostNValuesHighlighter {
                                field: FLD_TEXT1.to_string(),
                                limit: 2,
                            }),
                        ),
                        FieldValueHighlighters::skip_remaining(),
                    ],
                )?;

                assert_highlights(
                    &to_doc_list(highlighter.highlight(&top_docs, &[Arc::clone(&query)])?),
                    &lines(&["0. text1: >foo< bar, bar >foo< baz"]),
                )?;
                assert_highlights(
                    &to_doc_list(highlighter.highlight(&top_docs, &[match_all])?),
                    &lines(&["0. text1: foo bar, bar foo baz, [2 omitted]"]),
                )
            })?;
        Ok(())
    }

    fn test_highlight_more_queries_at_once_showoff(&mut self) -> Result<()> {
        // Match highlighter underlying components are powerful enough to build interesting,
        // if not always super-practical, things. In this case, we would like to highlight
        // a set of matches of *more than one* query over the same set of input documents. This includes
        // highest-scoring passage resolution (from multiple hits) and different highlight markers
        // for each query.
        let analyzer = self.analyzer();
        self.builder()
            .doc(FLD_TEXT1, &["foo bar baz"])
            .doc(FLD_TEXT1, &["foo baz bar"])
            .build(self.analyzer(), |reader| {
                // Let's start with the two queries. The first one will be an unordered
                // query for (foo, baz) with a max gap of 1; let's use intervals for this.
                let q1: Arc<dyn Query> = Arc::new(IntervalQuery::new(
                    FLD_TEXT1,
                    Intervals::maxgaps(
                        1,
                        Intervals::unordered(vec![
                            Intervals::term_str("foo"),
                            Intervals::term_str("baz"),
                        ]),
                    )?,
                ));

                // The second one will be a simpler term query for "bar".
                let q2 = tq(FLD_TEXT1, "bar");

                // Let's fetch matching documents by combining the two into a Boolean query.
                let query = {
                    let mut b = rucene::search::boolean_query::Builder::new();
                    b.add(Arc::clone(&q1), Occur::SHOULD)?;
                    b.add(Arc::clone(&q2), Occur::SHOULD)?;
                    Arc::new(b.build()) as Arc<dyn Query>
                };

                let searcher = new_searcher(&reader)?;
                let top_docs = search(&searcher, &query)?;

                // If we use the "regular" highlighter, the result will be slightly odd: a nested
                // highlight over "bar" within the first match. Also, you can't distinguish which of
                // the sub-queries
                // caused which highlight marker... but if it were HTML then you could give the span
                // some semi-translucent background and layered matches would be visible.
                let mut highlighter = match_highlighter(
                    &searcher,
                    &analyzer,
                    vec![
                        FieldValueHighlighters::highlighted(
                            80 * 3,
                            1,
                            PassageFormatter::new("...", "<span>", "</span>"),
                            equals_field(FLD_TEXT1),
                        ),
                        FieldValueHighlighters::skip_remaining(),
                    ],
                )?;

                assert_highlights(
                    &to_doc_list(highlighter.highlight(&top_docs, &[query])?),
                    &lines(&[
                        "0. text1: <span>foo <span>bar</span> baz</span>",
                        "1. text1: <span>foo baz</span> <span>bar</span>",
                    ]),
                )?;

                // To separate highlights for multiple queries we'll pass them separately to the
                // highlighter and differentiate highlight markers upon their application. Let's start
                // with the customized
                // field highlighter first. This utilizes the fact that match ranges passed from
                // MatchHighlighter
                // contain a reference to the original query which brought up the match.

                // And this is pretty much it. We now set up query classes to display, set up the
                // highlighter...
                let query_class_map: Vec<(Arc<dyn Query>, String)> = vec![
                    (Arc::clone(&q1), "q1".to_string()),
                    (Arc::clone(&q2), "q2".to_string()),
                ];
                highlighter = match_highlighter(
                    &searcher,
                    &analyzer,
                    vec![
                        Arc::new(SeparateMarkerFieldHighlighter {
                            field: FLD_TEXT1.to_string(),
                            query_class_map: Arc::new(query_class_map),
                        }),
                        FieldValueHighlighters::skip_remaining(),
                    ],
                )?;

                // ...and run highlighting. Note the query passed to the highlighter are individual
                // sub-clauses
                // of the Boolean query used to fetch documents.
                assert_highlights(
                    &to_doc_list(highlighter.highlight(&top_docs, &[q1, q2])?),
                    &lines(&[
                        "0. text1: <span class='q1'>foo <span class='q2'>bar</span> baz</span>",
                        "1. text1: <span class='q1'>foo baz</span> <span class='q2'>bar</span>",
                    ]),
                )
            })?;
        Ok(())
    }
}

/// The message of an `AssertionError` (`e.getMessage()`).
fn assertion_message(e: &LuceneError) -> String {
    rucene_test_framework::util::java_exceptions::java_throwable(e)
        .message()
        .unwrap_or("null")
        .to_string()
}

/// The local class `AtMostNValuesHighlighter` of
/// `testCustomFieldHighlightHandling`.
struct AtMostNValuesHighlighter {
    field: String,
    limit: usize,
}

impl FieldValueHighlighter for AtMostNValuesHighlighter {
    fn is_applicable(&self, field: &str, _has_matches: bool) -> bool {
        field == self.field
    }

    fn format(
        &self,
        _field: &str,
        values: &[String],
        _contiguous_value: &str,
        _value_ranges: &[OffsetRange],
        _match_offsets: Option<&[QueryOffsetRange]>,
    ) -> Result<Option<Vec<String>>> {
        if values.len() <= self.limit {
            Ok(Some(values.to_vec()))
        } else {
            let mut collected: Vec<String> = values[..self.limit].to_vec();
            let remaining = values.len() - collected.len();
            collected.push(format!("[{remaining} omitted]"));
            Ok(Some(collected))
        }
    }

    fn always_fetched_fields(&self) -> Vec<String> {
        vec![self.field.clone()]
    }
}

/// The local class `SeparateMarkerFieldHighlighter` of
/// `testHighlightMoreQueriesAtOnceShowoff`.
struct SeparateMarkerFieldHighlighter {
    field: String,
    /// `Map<Query, String> queryClassMap` (keys compared by `equals`).
    query_class_map: Arc<Vec<(Arc<dyn Query>, String)>>,
}

impl FieldValueHighlighter for SeparateMarkerFieldHighlighter {
    fn is_applicable(&self, field: &str, has_matches: bool) -> bool {
        field == self.field && has_matches
    }

    fn format(
        &self,
        _field: &str,
        _values: &[String],
        contiguous_value: &str,
        value_ranges: &[OffsetRange],
        match_offsets: Option<&[QueryOffsetRange]>,
    ) -> Result<Option<Vec<String>>> {
        let passage_selector = PassageSelector::new();
        let max_passage_window = 80;
        let max_passages = 3;
        let match_offsets: Vec<OffsetRange> = match_offsets
            .unwrap_or_else(|| panic!("java.lang.NullPointerException: no match offsets"))
            .iter()
            .map(QueryOffsetRange::as_offset_range)
            .collect();
        let best_passages = passage_selector.pick_best_in_ranges(
            contiguous_value,
            &match_offsets,
            max_passage_window,
            max_passages,
            value_ranges,
        )?;

        // We know the offset ranges passed to us by MatchHighlighter are instances of
        // QueryOffsetRange
        // so we compute the class based on that.
        let map = Arc::clone(&self.query_class_map);
        let query_to_class = move |range: &OffsetRange| -> String {
            let query = range
                .query()
                .unwrap_or_else(|| panic!("java.lang.ClassCastException: not a QueryOffsetRange"));
            map.iter()
                .find(|(q, _)| q.query_eq(&**query))
                .map_or_else(|| "null".to_string(), |(_, c)| c.clone())
        };

        let start: MarkerFunction = Arc::new(move |range: &OffsetRange| {
            format!("<span class='{}'>", query_to_class(range))
        });
        let end: MarkerFunction = Arc::new(|_range: &OffsetRange| "</span>".to_string());
        let passage_formatter = PassageFormatter::with_functions("...", start, end);
        Ok(Some(passage_formatter.format(
            contiguous_value,
            &best_passages,
            value_ranges,
        )?))
    }
}
