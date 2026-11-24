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

//! Port of `TestMatchRegionRetriever.java`.
//!
//! The highlights collector appends to one list; Java's appends from the
//! blocks `highlightDocuments` processes, the port's consumer is called
//! sequentially. `containsInAnyOrder` makes the order immaterial.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::{Class, Constructor};
use randomizedtesting::junit::{Assert, Before, Test};
use randomizedtesting::RandomizedTest;
use rucene::analysis::{new_stop_filter, Analyzer, CharArraySet};
use rucene::document::{Field, FieldType, Store, TextField};
use rucene::error::{LuceneError, Result};
use rucene::index::{DirectoryReader, IndexOptions, IndexReader, IndexableField, LeafReader};
use rucene::search::boolean_clause::Occur;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::match_all_docs_query::MatchAllDocsQuery;
use rucene::search::phrase_query::PhraseQuery;
use rucene::search::query::Query;
use rucene::search::TermQuery;
use rucene::util::java_collections::JavaHashMap;
use rucene_analysis_common::analysis::core::whitespace_tokenizer::{
    new_whitespace_tokenizer, new_whitespace_tokenizer_with_max_token_len,
};
use rucene_analysis_common::analysis::miscellaneous::per_field_analyzer_wrapper::PerFieldAnalyzerWrapper;
use rucene_analysis_common::analysis::synonym::synonym_graph_filter::new_synonym_graph_filter;
use rucene_analysis_common::analysis::util::char_tokenizer::DEFAULT_MAX_WORD_LEN;
use rucene_highlighter::matchhighlight::match_region_retriever::{
    FieldHits, FieldValueProvider, MatchOffsetsConsumer, MatchRegionRetriever,
};
use rucene_highlighter::matchhighlight::offsets_from_values::OffsetsFromValues;
use rucene_highlighter::matchhighlight::offsets_retrieval_strategy::OffsetsRetrievalStrategy;
use rucene_highlighter::matchhighlight::offsets_retrieval_strategy_supplier::OffsetsRetrievalStrategySupplier;
use rucene_queries::intervals::interval_query::IntervalQuery;
use rucene_queries::intervals::intervals::Intervals;
use rucene_queries::spans::span_near_query::SpanNearQuery;
use rucene_queries::spans::span_term_query::SpanTermQuery;
use rucene_queryparser::flexible::standard::config::standard_query_config_handler::Operator;
use rucene_queryparser::flexible::standard::standard_query_parser::StandardQueryParser;
use rucene_test_framework::util::LuceneTestCase;

use crate::highlighter_support::search_support::term;
use crate::highlighter_support::{chain, instance_class, sink, ti, FnAnalyzer};
use crate::matchhighlight::analyzer_with_gaps::analyzer_with_gaps;
use crate::matchhighlight::ascii_match_range_highlighter::AsciiMatchRangeHighlighter;
use crate::matchhighlight::index_builder::{self, IndexBuilder, ToField};
use crate::matchhighlight::missing_analyzer::MissingAnalyzer;
use crate::matchhighlight::test_match_highlighter::build_synonym_map;

const FLD_ID: &str = index_builder::FLD_ID;
const FLD_TEXT_POS_OFFS1: &str = "field_text_offs1";
const FLD_TEXT_POS_OFFS2: &str = "field_text_offs2";
const FLD_TEXT_POS_OFFS: &str = "field_text_offs";
const FLD_TEXT_POS: &str = "field_text";
const FLD_TEXT_SYNONYMS_POS_OFFS: &str = "field_text_syns_offs";
const FLD_TEXT_SYNONYMS_POS: &str = "field_text_syns";
const FLD_TEXT_NOPOS: &str = "field_text_nopos";
const FLD_NON_EXISTING: &str = "field_missing";

const STOPWORD1: &str = "stopword";

type T = TestMatchRegionRetriever;

/// `TestMatchRegionRetriever`.
pub fn class_def() -> Arc<Class> {
    let t = |name: &str, body: fn(&mut T) -> Result<()>| ti::<T>(name, body).annotate(Test::new());
    instance_class(
        "org.apache.lucene.search.matchhighlight.TestMatchRegionRetriever",
        &LuceneTestCase::class(),
        Constructor::no_args(T::default),
        vec![
            ti::<T>("setup", T::setup).annotate(Before),
            t("testTermQueryWithOffsets", T::test_term_query_with_offsets),
            t(
                "testTermQueryWithPositions",
                T::test_term_query_with_positions,
            ),
            t(
                "testBooleanMultifieldQueryWithOffsets",
                T::test_boolean_multifield_query_with_offsets,
            ),
            t(
                "testBooleanMultifieldQueryWithPositions",
                T::test_boolean_multifield_query_with_positions,
            ),
            t(
                "testVariousQueryTypesWithOffsets",
                T::test_various_query_types_with_offsets,
            ),
            t(
                "testVariousQueryTypesWithPositions",
                T::test_various_query_types_with_positions,
            ),
            t(
                "testIntervalQueryHighlightCrossingMultivalueBoundary",
                T::test_interval_query_highlight_crossing_multivalue_boundary,
            ),
            t("testIntervalQueries", T::test_interval_queries),
            t(
                "testDegenerateIntervalsWithPositions",
                T::test_degenerate_intervals_with_positions,
            ),
            t(
                "testDegenerateIntervalsWithOffsets",
                T::test_degenerate_intervals_with_offsets,
            ),
            t(
                "testMultivaluedFieldsWithOffsets",
                T::test_multivalued_fields_with_offsets,
            ),
            t(
                "testMultivaluedFieldsWithPositions",
                T::test_multivalued_fields_with_positions,
            ),
            t("testMultiFieldHighlights", T::test_multi_field_highlights),
            t("testNoRewrite", T::test_no_rewrite),
            t(
                "testNestedQueryHitsWithOffsets",
                T::test_nested_query_hits_with_offsets,
            ),
            t(
                "testNestedQueryHitsWithPositions",
                T::test_nested_query_hits_with_positions,
            ),
            t(
                "testGraphQueryWithOffsets",
                T::test_graph_query_with_offsets,
            ),
            t(
                "testGraphQueryWithPositions",
                T::test_graph_query_with_positions,
            ),
            t("testSpanQueryWithOffsets", T::test_span_query_with_offsets),
            t(
                "testSpanQueryWithPositions",
                T::test_span_query_with_positions,
            ),
            t(
                "testTextFieldNoPositionsOffsetFromValues",
                T::test_text_field_no_positions_offset_from_values,
            ),
            t(
                "testTextFieldNoPositionsOffsetsFromTokens",
                T::test_text_field_no_positions_offsets_from_tokens,
            ),
        ],
    )
}

/// The `TestMatchRegionRetriever` instance.
#[derive(Default)]
pub struct TestMatchRegionRetriever {
    type_stored_with_offsets: Option<Arc<FieldType>>,
    type_stored_no_positions: Option<Arc<FieldType>>,
    analyzer: Option<Arc<dyn Analyzer>>,
}

/// `containsInAnyOrder(expected...)`.
fn assert_contains_in_any_order(actual: &[String], expected: &[String]) {
    let mut remaining: Vec<&String> = actual.iter().collect();
    let mut ok = actual.len() == expected.len();
    if ok {
        for e in expected {
            match remaining.iter().position(|a| *a == e) {
                Some(i) => {
                    remaining.remove(i);
                }
                None => {
                    ok = false;
                    break;
                }
            }
        }
    }
    if !ok {
        Assert::fail(&format!(
            "\nExpected: iterable with items {expected:?} in any order\n     but: {actual:?}"
        ));
    }
}

fn s(v: &str) -> String {
    v.to_string()
}

/// The `PhraseQuery(slop, field, terms...)`.
fn pq(slop: i32, field: &str, terms: &[&str]) -> Result<Arc<dyn Query>> {
    Ok(Arc::new(PhraseQuery::with_slop(slop, field, terms)?))
}

fn tq(field: &str, text: &str) -> Arc<dyn Query> {
    Arc::new(TermQuery::new(term(field, text)))
}

fn bq(clauses: Vec<(Arc<dyn Query>, Occur)>) -> Result<Arc<dyn Query>> {
    let mut builder = rucene::search::boolean_query::Builder::new();
    for (q, o) in clauses {
        builder.add(q, o)?;
    }
    Ok(Arc::new(builder.build()))
}

fn iq(
    field: &str,
    source: Arc<dyn rucene_queries::intervals::intervals_source::IntervalsSource>,
) -> Arc<dyn Query> {
    Arc::new(IntervalQuery::new(field, source))
}

fn it(text: &str) -> Arc<dyn rucene_queries::intervals::intervals_source::IntervalsSource> {
    Intervals::term_str(text)
}

/// The highlights collector of `highlights(...)`.
struct HighlightCollector<'a> {
    formatter: &'a AsciiMatchRangeHighlighter,
    highlights: Vec<String>,
}

impl MatchOffsetsConsumer for HighlightCollector<'_> {
    fn accept(
        &mut self,
        _doc_id: i32,
        leaf_reader: &Arc<dyn LeafReader>,
        leaf_doc_id: i32,
        _field_value_provider: &dyn FieldValueProvider,
        field_highlights: &FieldHits,
    ) -> Result<()> {
        let mut sb = String::new();
        let document = leaf_reader.stored_fields()?.document(leaf_doc_id)?;
        // new TreeMap<>(fieldHighlights): FieldHits iterates in key order
        for (field, snippets) in self.formatter.apply(&document, field_highlights.iter())? {
            sb.push_str(&format!("({}: '{}')", field, snippets.join(" | ")));
        }

        if !sb.is_empty() {
            sb.insert_str(
                0,
                &format!("{}: ", document.get(FLD_ID).unwrap_or_else(|| s("null"))),
            );
            self.highlights.push(sb);
        }
        Ok(())
    }
}

impl TestMatchRegionRetriever {
    fn analyzer(&self) -> Arc<dyn Analyzer> {
        Arc::clone(self.analyzer.as_ref().expect("setup ran"))
    }

    fn setup(&mut self) -> Result<()> {
        let with_offsets = FieldType::new_from(TextField::type_stored());
        with_offsets.set_index_options(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS)?;
        with_offsets.freeze();
        self.type_stored_with_offsets = Some(Arc::new(with_offsets));

        let no_positions = FieldType::new_from(TextField::type_stored());
        no_positions.set_index_options(IndexOptions::DOCS_AND_FREQS)?;
        no_positions.freeze();
        self.type_stored_no_positions = Some(Arc::new(no_positions));

        let offset_gap = RandomizedTest::random_int_between(0, 2);
        let position_gap = *RandomizedTest::random_from(&[0, 1, 100]);
        let whitespace_analyzer = analyzer_with_gaps(
            offset_gap,
            position_gap,
            FnAnalyzer::new(
                "org.apache.lucene.search.matchhighlight.TestMatchRegionRetriever$1",
                |_field_name| {
                    let tokenizer =
                        new_whitespace_tokenizer_with_max_token_len(DEFAULT_MAX_WORD_LEN)?;
                    chain(tokenizer, |input| {
                        let mut stop_set = CharArraySet::new(1, true);
                        stop_set.add(STOPWORD1);
                        Ok(sink(new_stop_filter(input, stop_set)?))
                    })
                },
            )
            .arc(),
        );

        let synonym_map = Arc::new(build_synonym_map(&[
            ["foo\u{0000}bar", "syn1"],
            ["baz", "syn2\u{0000}syn3"],
        ])?);

        let synonyms_analyzer = FnAnalyzer::new(
            "org.apache.lucene.search.matchhighlight.TestMatchRegionRetriever$2",
            move |_field_name| {
                let tokenizer = new_whitespace_tokenizer();
                let map = Arc::clone(&synonym_map);
                chain(tokenizer, move |input| {
                    Ok(sink(new_synonym_graph_filter(input, map, true)?))
                })
            },
        )
        .arc();

        let mut field_analyzers: JavaHashMap<Arc<dyn Analyzer>> = JavaHashMap::new();
        field_analyzers.insert(s(FLD_TEXT_POS), Arc::clone(&whitespace_analyzer));
        field_analyzers.insert(s(FLD_TEXT_POS_OFFS), Arc::clone(&whitespace_analyzer));
        field_analyzers.insert(s(FLD_TEXT_POS_OFFS1), Arc::clone(&whitespace_analyzer));
        field_analyzers.insert(s(FLD_TEXT_POS_OFFS2), Arc::clone(&whitespace_analyzer));
        field_analyzers.insert(s(FLD_TEXT_NOPOS), Arc::clone(&whitespace_analyzer));
        field_analyzers.insert(
            s(FLD_TEXT_SYNONYMS_POS_OFFS),
            Arc::clone(&synonyms_analyzer),
        );
        field_analyzers.insert(s(FLD_TEXT_SYNONYMS_POS), synonyms_analyzer);

        self.analyzer = Some(Arc::new(PerFieldAnalyzerWrapper::new_with_fields(
            Arc::new(MissingAnalyzer::new()),
            Some(field_analyzers),
        )));
        Ok(())
    }

    /// `stdQueryParser.apply(query, defField)`.
    fn std_query_parser(&self, query: &str, def_field: &str) -> Result<Arc<dyn Query>> {
        let mut parser = StandardQueryParser::with_analyzer(Some(self.analyzer()));
        parser.set_default_operator(Some(Operator::And))?;
        // QueryNodeException is rethrown as a RuntimeException
        let parsed = parser
            .parse(query, Some(def_field))
            .map_err(LuceneError::runtime_exception)?;
        Ok(parsed.expect("NullPointerException: no query"))
    }

    /// `this::toField`.
    fn to_field(&self) -> ToField {
        let with_offsets = Arc::clone(self.type_stored_with_offsets.as_ref().expect("setup ran"));
        let no_positions = Arc::clone(self.type_stored_no_positions.as_ref().expect("setup ran"));
        Arc::new(move |name: &str, value: &str| -> Arc<dyn IndexableField> {
            let field = match name {
                FLD_TEXT_NOPOS => Field::new(s(name), s(value), Arc::clone(&no_positions)),
                FLD_TEXT_POS | FLD_TEXT_SYNONYMS_POS => {
                    return Arc::new(
                        TextField::new(s(name), s(value), Store::YES)
                            .expect("INVARIANT: a valid text field"),
                    )
                }
                FLD_TEXT_POS_OFFS
                | FLD_TEXT_POS_OFFS1
                | FLD_TEXT_POS_OFFS2
                | FLD_TEXT_SYNONYMS_POS_OFFS => {
                    Field::new(s(name), s(value), Arc::clone(&with_offsets))
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

    fn test_term_query_with_offsets(&mut self) -> Result<()> {
        self.check_term_query(FLD_TEXT_POS_OFFS)
    }

    fn test_term_query_with_positions(&mut self) -> Result<()> {
        self.check_term_query(FLD_TEXT_POS)
    }

    fn check_term_query(&self, field: &str) -> Result<()> {
        self.builder()
            .doc(field, &["foo bar baz"])
            .doc(field, &["bar foo baz"])
            .doc(field, &["bar baz foo"])
            .doc(field, &["bar bar bar irrelevant"])
            .build(self.analyzer(), |reader| {
                assert_contains_in_any_order(
                    &self.highlights(&reader, tq(field, "foo"))?,
                    &[
                        format!("0: ({field}: '>foo< bar baz')"),
                        format!("1: ({field}: 'bar >foo< baz')"),
                        format!("2: ({field}: 'bar baz >foo<')"),
                    ],
                );
                Ok(())
            })?;
        Ok(())
    }

    fn test_boolean_multifield_query_with_offsets(&mut self) -> Result<()> {
        self.check_boolean_multifield_query(FLD_TEXT_POS_OFFS)
    }

    fn test_boolean_multifield_query_with_positions(&mut self) -> Result<()> {
        self.check_boolean_multifield_query(FLD_TEXT_POS)
    }

    fn check_boolean_multifield_query(&self, field: &str) -> Result<()> {
        let query = bq(vec![
            (pq(1, field, &["foo", "baz"])?, Occur::SHOULD),
            (tq(FLD_NON_EXISTING, "abc"), Occur::SHOULD),
            (tq(field, "xyz"), Occur::MUST_NOT),
        ])?;

        self.builder()
            .doc(field, &["foo bar baz abc"])
            .doc(field, &["bar foo baz def"])
            .doc(field, &["bar baz foo xyz"])
            .build(self.analyzer(), |reader| {
                assert_contains_in_any_order(
                    &self.highlights(&reader, Arc::clone(&query))?,
                    &[
                        format!("0: ({field}: '>foo bar baz< abc')"),
                        format!("1: ({field}: 'bar >foo baz< def')"),
                    ],
                );
                Ok(())
            })?;
        Ok(())
    }

    fn test_various_query_types_with_offsets(&mut self) -> Result<()> {
        self.check_various_query_types(FLD_TEXT_POS_OFFS)
    }

    fn test_various_query_types_with_positions(&mut self) -> Result<()> {
        self.check_various_query_types(FLD_TEXT_POS)
    }

    fn check_various_query_types(&self, field: &str) -> Result<()> {
        self.builder()
            .doc(field, &["foo bar baz abc"])
            .doc(field, &["bar foo baz def"])
            .doc(field, &["bar baz foo xyz"])
            .build(self.analyzer(), |reader| {
                let h = |q: &str| self.highlights(&reader, self.std_query_parser(q, field)?);
                assert_contains_in_any_order(
                    &h("foo baz")?,
                    &[
                        format!("0: ({field}: '>foo< bar >baz< abc')"),
                        format!("1: ({field}: 'bar >foo< >baz< def')"),
                        format!("2: ({field}: 'bar >baz< >foo< xyz')"),
                    ],
                );

                assert_contains_in_any_order(
                    &h("foo OR xyz")?,
                    &[
                        format!("0: ({field}: '>foo< bar baz abc')"),
                        format!("1: ({field}: 'bar >foo< baz def')"),
                        format!("2: ({field}: 'bar baz >foo< >xyz<')"),
                    ],
                );

                assert_contains_in_any_order(
                    &h("bas~2")?,
                    &[
                        format!("0: ({field}: 'foo >bar< >baz< >abc<')"),
                        format!("1: ({field}: '>bar< foo >baz< def')"),
                        format!("2: ({field}: '>bar< >baz< foo xyz')"),
                    ],
                );

                assert_contains_in_any_order(
                    &h("\"foo bar\"")?,
                    &[format!("0: ({field}: '>foo bar< baz abc')")],
                );

                assert_contains_in_any_order(
                    &h("\"foo bar\"~3")?,
                    &[
                        format!("0: ({field}: '>foo bar< baz abc')"),
                        format!("1: ({field}: '>bar foo< baz def')"),
                        format!("2: ({field}: '>bar baz foo< xyz')"),
                    ],
                );

                assert_contains_in_any_order(
                    &h("ba*")?,
                    &[
                        format!("0: ({field}: 'foo >bar< >baz< abc')"),
                        format!("1: ({field}: '>bar< foo >baz< def')"),
                        format!("2: ({field}: '>bar< >baz< foo xyz')"),
                    ],
                );

                assert_contains_in_any_order(
                    &h("[bar TO bas]")?,
                    &[
                        format!("0: ({field}: 'foo >bar< baz abc')"),
                        format!("1: ({field}: '>bar< foo baz def')"),
                        format!("2: ({field}: '>bar< baz foo xyz')"),
                    ],
                );

                // Note how document '2' has 'bar' that isn't highlighted (because this
                // document is excluded in the first clause).
                assert_contains_in_any_order(
                    &h("([bar TO baz] -xyz) OR baz")?,
                    &[
                        format!("0: ({field}: 'foo >bar< >>baz<< abc')"),
                        format!("1: ({field}: '>bar< foo >>baz<< def')"),
                        format!("2: ({field}: 'bar >baz< foo xyz')"),
                    ],
                );

                let all = self.highlights(&reader, Arc::new(MatchAllDocsQuery::instance()))?;
                if !all.is_empty() {
                    Assert::fail(&format!(
                        "\nExpected: a collection with size <0>\n     but: collection size was <{}>",
                        all.len()
                    ));
                }
                Ok(())
            })?;

        self.builder()
            .doc(field, &["foo baz foo"])
            .doc(field, &["bas baz foo"])
            .doc(field, &["bar baz foo xyz"])
            .build(self.analyzer(), |reader| {
                assert_contains_in_any_order(
                    &self
                        .highlights(&reader, self.std_query_parser("[bar TO baz] -bar", field)?)?,
                    &[
                        format!("0: ({field}: 'foo >baz< foo')"),
                        format!("1: ({field}: '>bas< >baz< foo')"),
                    ],
                );
                Ok(())
            })?;
        Ok(())
    }

    fn test_interval_query_highlight_crossing_multivalue_boundary(&mut self) -> Result<()> {
        let field = FLD_TEXT_POS;
        self.builder()
            .doc(field, &["foo", "bar"])
            .build(self.analyzer(), |reader| {
                assert_contains_in_any_order(
                    &self.highlights(
                        &reader,
                        iq(field, Intervals::unordered(vec![it("foo"), it("bar")])),
                    )?,
                    &[s("0: (field_text: '>foo< | >bar<')")],
                );
                Ok(())
            })?;
        Ok(())
    }

    fn test_interval_queries(&mut self) -> Result<()> {
        let field = FLD_TEXT_POS_OFFS;
        self.builder()
            .doc(field, &["foo baz foo"])
            .doc(field, &["bas baz foo"])
            .doc(field, &["bar baz foo xyz"])
            .build(self.analyzer(), |reader| {
                assert_contains_in_any_order(
                    &self.highlights(
                        &reader,
                        iq(
                            field,
                            Intervals::unordered(vec![it("foo"), it("bas"), it("baz")]),
                        ),
                    )?,
                    &[s("1: (field_text_offs: '>bas baz foo<')")],
                );

                assert_contains_in_any_order(
                    &self.highlights(
                        &reader,
                        iq(
                            field,
                            Intervals::maxgaps(
                                1,
                                Intervals::unordered(vec![it("foo"), it("bar")]),
                            )?,
                        ),
                    )?,
                    &[s("2: (field_text_offs: '>bar baz foo< xyz')")],
                );

                assert_contains_in_any_order(
                    &self.highlights(
                        &reader,
                        iq(
                            field,
                            Intervals::containing(
                                Intervals::unordered(vec![it("foo"), it("bar")]),
                                it("foo"),
                            )?,
                        ),
                    )?,
                    &[s("2: (field_text_offs: '>bar baz foo< xyz')")],
                );

                assert_contains_in_any_order(
                    &self.highlights(
                        &reader,
                        iq(
                            field,
                            Intervals::contained_by(
                                it("foo"),
                                Intervals::unordered(vec![it("foo"), it("bar")]),
                            )?,
                        ),
                    )?,
                    &[s("2: (field_text_offs: 'bar baz >foo< xyz')")],
                );

                assert_contains_in_any_order(
                    &self.highlights(
                        &reader,
                        iq(
                            field,
                            Intervals::overlapping(
                                it("foo"),
                                Intervals::unordered(vec![it("foo"), it("bar")]),
                            ),
                        ),
                    )?,
                    &[s("2: (field_text_offs: 'bar baz >foo< xyz')")],
                );
                Ok(())
            })?;
        Ok(())
    }

    fn test_degenerate_intervals_with_positions(&mut self) -> Result<()> {
        self.test_degenerate_intervals(FLD_TEXT_POS)
    }

    fn test_degenerate_intervals_with_offsets(&mut self) -> Result<()> {
        self.test_degenerate_intervals(FLD_TEXT_POS_OFFS)
    }

    /// `testDegenerateIntervals(String field)`: public, but with a
    /// parameter, so not a test method.
    fn test_degenerate_intervals(&self, field: &str) -> Result<()> {
        self.builder()
            .doc(field, &[&format!("foo {STOPWORD1} bar")])
            .build(self.analyzer(), |reader| {
                assert_contains_in_any_order(
                    &self.highlights(&reader, iq(field, Intervals::extend(it("bar"), 1, 3)))?,
                    &[format!("0: ({field}: 'foo {STOPWORD1} >bar<')")],
                );

                assert_contains_in_any_order(
                    &self.highlights(&reader, iq(field, Intervals::extend(it("bar"), 5, 100)))?,
                    &[format!("0: ({field}: '>foo {STOPWORD1} bar<')")],
                );
                Ok(())
            })?;
        Ok(())
    }

    fn test_multivalued_fields_with_offsets(&mut self) -> Result<()> {
        self.check_multivalued_fields(FLD_TEXT_POS_OFFS)
    }

    fn test_multivalued_fields_with_positions(&mut self) -> Result<()> {
        self.check_multivalued_fields(FLD_TEXT_POS)
    }

    /// `checkMultivaluedFields(String field)`: public, but with a parameter.
    fn check_multivalued_fields(&self, field: &str) -> Result<()> {
        self.builder()
            .doc(field, &["foo bar", "baz abc", "bad baz"])
            .doc(field, &["bar foo", "baz def"])
            .doc(field, &["bar baz", "foo xyz"])
            .build(self.analyzer(), |reader| {
                assert_contains_in_any_order(
                    &self.highlights(&reader, self.std_query_parser("baz", field)?)?,
                    &[
                        format!("0: ({field}: '>baz< abc | bad >baz<')"),
                        format!("1: ({field}: '>baz< def')"),
                        format!("2: ({field}: 'bar >baz<')"),
                    ],
                );
                Ok(())
            })?;
        Ok(())
    }

    fn test_multi_field_highlights(&mut self) -> Result<()> {
        for field_pairs in [
            [FLD_TEXT_POS_OFFS1, FLD_TEXT_POS_OFFS2],
            [FLD_TEXT_POS, FLD_TEXT_POS_OFFS2],
            [FLD_TEXT_POS_OFFS1, FLD_TEXT_POS],
        ] {
            let field1 = field_pairs[0];
            let field2 = field_pairs[1];
            self.builder()
                .doc_with(|fields| {
                    fields.add(field1, &["foo bar", "baz abc"]);
                    fields.add(field2, &["foo baz", "loo bar"]);
                })
                .build(self.analyzer(), |reader| {
                    let mut ordered = [
                        format!("({field1}: '>baz< abc')"),
                        format!("({field2}: 'loo >bar<')"),
                    ];
                    ordered.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
                    let ordered = ordered.concat();

                    assert_contains_in_any_order(
                        &self.highlights(
                            &reader,
                            self.std_query_parser(
                                &format!("{field1}:baz OR {field2}:bar"),
                                field1,
                            )?,
                        )?,
                        &[format!("0: {ordered}")],
                    );
                    Ok(())
                })?;
        }
        Ok(())
    }

    /// Rewritten Boolean queries may omit matches from `SHOULD` clauses.
    /// Check that this isn't the case.
    fn test_no_rewrite(&mut self) -> Result<()> {
        let field1 = FLD_TEXT_POS_OFFS1;
        let field2 = FLD_TEXT_POS_OFFS2;
        self.builder()
            .doc_with(|fields| {
                fields.add(field1, &["0100"]);
                fields.add(field2, &["loo bar"]);
            })
            .doc_with(|fields| {
                fields.add(field1, &["0200"]);
                fields.add(field2, &["foo bar"]);
            })
            .build(self.analyzer(), |reader| {
                let expected = format!("0: ({field1}: '>0100<')({field2}: 'loo >bar<')");
                assert_contains_in_any_order(
                    &self.highlights(
                        &reader,
                        self.std_query_parser(&format!("+{field1}:01* OR {field2}:bar"), field1)?,
                    )?,
                    std::slice::from_ref(&expected),
                );

                assert_contains_in_any_order(
                    &self.highlights(
                        &reader,
                        self.std_query_parser(&format!("+{field1}:01* AND {field2}:bar"), field1)?,
                    )?,
                    &[expected],
                );
                Ok(())
            })?;
        Ok(())
    }

    fn test_nested_query_hits_with_offsets(&mut self) -> Result<()> {
        self.check_nested_query_hits(FLD_TEXT_POS_OFFS)
    }

    fn test_nested_query_hits_with_positions(&mut self) -> Result<()> {
        self.check_nested_query_hits(FLD_TEXT_POS)
    }

    /// `checkNestedQueryHits(String field)`: public, but with a parameter.
    fn check_nested_query_hits(&self, field: &str) -> Result<()> {
        self.builder()
            .doc(field, &["foo bar baz abc"])
            .build(self.analyzer(), |reader| {
                assert_contains_in_any_order(
                    &self.highlights(
                        &reader,
                        bq(vec![
                            (pq(1, field, &["foo", "baz"])?, Occur::SHOULD),
                            (tq(field, "bar"), Occur::SHOULD),
                        ])?,
                    )?,
                    &[format!("0: ({field}: '>foo >bar< baz< abc')")],
                );

                assert_contains_in_any_order(
                    &self.highlights(
                        &reader,
                        bq(vec![
                            (pq(1, field, &["foo", "baz"])?, Occur::SHOULD),
                            (tq(field, "bar"), Occur::SHOULD),
                            (tq(field, "baz"), Occur::SHOULD),
                        ])?,
                    )?,
                    &[format!("0: ({field}: '>foo >bar< >baz<< abc')")],
                );
                Ok(())
            })?;
        Ok(())
    }

    fn test_graph_query_with_offsets(&mut self) -> Result<()> {
        self.check_graph_query(FLD_TEXT_SYNONYMS_POS_OFFS)
    }

    fn test_graph_query_with_positions(&mut self) -> Result<()> {
        self.check_graph_query(FLD_TEXT_SYNONYMS_POS)
    }

    fn check_graph_query(&self, field: &str) -> Result<()> {
        self.builder()
            .doc(field, &["foo bar baz"])
            .doc(field, &["bar foo baz"])
            .doc(field, &["bar baz foo"])
            .doc(field, &["bar bar bar irrelevant"])
            .build(self.analyzer(), |reader| {
                assert_contains_in_any_order(
                    &self.highlights(&reader, tq(field, "syn1"))?,
                    &[format!("0: ({field}: '>foo bar< baz')")],
                );

                // [syn2 syn3] = baz
                // so both these queries highlight baz.
                assert_contains_in_any_order(
                    &self.highlights(&reader, tq(field, "syn3"))?,
                    &[
                        format!("0: ({field}: 'foo bar >baz<')"),
                        format!("1: ({field}: 'bar foo >baz<')"),
                        format!("2: ({field}: 'bar >baz< foo')"),
                    ],
                );
                assert_contains_in_any_order(
                    &self.highlights(
                        &reader,
                        self.std_query_parser(&format!("{field}:\"syn2 syn3\""), field)?,
                    )?,
                    &[
                        format!("0: ({field}: 'foo bar >baz<')"),
                        format!("1: ({field}: 'bar foo >baz<')"),
                        format!("2: ({field}: 'bar >baz< foo')"),
                    ],
                );
                assert_contains_in_any_order(
                    &self.highlights(
                        &reader,
                        self.std_query_parser(&format!("{field}:\"foo syn2 syn3\""), field)?,
                    )?,
                    &[format!("1: ({field}: 'bar >foo baz<')")],
                );
                Ok(())
            })?;
        Ok(())
    }

    fn test_span_query_with_offsets(&mut self) -> Result<()> {
        self.check_span_queries(FLD_TEXT_POS_OFFS)
    }

    fn test_span_query_with_positions(&mut self) -> Result<()> {
        self.check_span_queries(FLD_TEXT_POS)
    }

    fn check_span_queries(&self, field: &str) -> Result<()> {
        let st = |text: &str| Arc::new(SpanTermQuery::new(term(field, text)));
        self.builder()
            .doc(field, &["foo bar baz"])
            .doc(field, &["bar foo baz"])
            .doc(field, &["bar baz foo"])
            .doc(field, &["bar bar bar irrelevant"])
            .build(self.analyzer(), |reader| {
                let mut b = SpanNearQuery::new_ordered_near_query(field);
                b.add_clause(st("bar"))?;
                b.add_clause(st("foo"))?;
                assert_contains_in_any_order(
                    &self.highlights(&reader, Arc::new(b.build()?))?,
                    &[format!("1: ({field}: '>bar foo< baz')")],
                );

                let mut b = SpanNearQuery::new_ordered_near_query(field);
                b.add_clause(st("bar"))?;
                b.add_gap(1)?;
                b.add_clause(st("foo"))?;
                assert_contains_in_any_order(
                    &self.highlights(&reader, Arc::new(b.build()?))?,
                    &[format!("2: ({field}: '>bar baz foo<')")],
                );

                let mut b = SpanNearQuery::new_unordered_near_query(field);
                b.add_clause(st("foo"))?;
                b.add_clause(st("bar"))?;
                assert_contains_in_any_order(
                    &self.highlights(&reader, Arc::new(b.build()?))?,
                    &[
                        format!("0: ({field}: '>foo bar< baz')"),
                        format!("1: ({field}: '>bar foo< baz')"),
                    ],
                );

                let mut b = SpanNearQuery::new_unordered_near_query(field);
                b.add_clause(st("foo"))?;
                b.add_clause(st("bar"))?;
                b.set_slop(1);
                assert_contains_in_any_order(
                    &self.highlights(&reader, Arc::new(b.build()?))?,
                    &[
                        format!("0: ({field}: '>foo bar< baz')"),
                        format!("1: ({field}: '>bar foo< baz')"),
                        format!("2: ({field}: '>bar baz foo<')"),
                    ],
                );
                Ok(())
            })?;
        Ok(())
    }

    /// This test runs a term query against a field with no stored positions
    /// or offsets. This test checks the `OffsetsFromValues` strategy that
    /// returns highlights over entire indexed values.
    fn test_text_field_no_positions_offset_from_values(&mut self) -> Result<()> {
        let field = FLD_TEXT_NOPOS;
        self.builder()
            .doc(FLD_TEXT_NOPOS, &["foo bar"])
            .doc(FLD_TEXT_NOPOS, &["foo bar", "baz baz"])
            .build(self.analyzer(), |reader| {
                let defaults = MatchRegionRetriever::compute_offset_retrieval_strategies(
                    Arc::clone(&reader) as Arc<dyn IndexReader>,
                    self.analyzer(),
                )?;
                let analyzer = self.analyzer();
                let custom_suppliers: Arc<dyn OffsetsRetrievalStrategySupplier> =
                    Arc::new(move |fld: &str| -> Arc<dyn OffsetsRetrievalStrategy> {
                        if fld == field {
                            Arc::new(OffsetsFromValues::new(field, Arc::clone(&analyzer)))
                        } else {
                            defaults.apply(field)
                        }
                    });

                assert_contains_in_any_order(
                    &self.highlights_with(custom_suppliers, &reader, tq(field, "bar"))?,
                    &[
                        format!("0: ({field}: '>foo bar<')"),
                        format!("1: ({field}: '>foo bar< | >baz baz<')"),
                    ],
                );
                Ok(())
            })?;
        Ok(())
    }

    /// This test runs a term query against a field with no stored positions
    /// or offsets. Such field structure is often useful for multivalued
    /// "keyword-like" fields.
    fn test_text_field_no_positions_offsets_from_tokens(&mut self) -> Result<()> {
        let field = FLD_TEXT_NOPOS;
        self.builder()
            .doc_with(|fields| {
                fields.add(FLD_TEXT_NOPOS, &["foo bar"]);
                fields.add(FLD_TEXT_POS, &["bar bar"]);
            })
            .doc(FLD_TEXT_NOPOS, &["foo bar", "baz bar"])
            .build(self.analyzer(), |reader| {
                assert_contains_in_any_order(
                    &self.highlights(&reader, tq(field, "bar"))?,
                    &[
                        format!("0: ({field}: 'foo >bar<')"),
                        format!("1: ({field}: 'foo >bar< | baz >bar<')"),
                    ],
                );
                Ok(())
            })?;
        Ok(())
    }

    fn highlights(
        &self,
        reader: &Arc<dyn DirectoryReader>,
        query: Arc<dyn Query>,
    ) -> Result<Vec<String>> {
        self.highlights_with(
            MatchRegionRetriever::compute_offset_retrieval_strategies(
                Arc::clone(reader) as Arc<dyn IndexReader>,
                self.analyzer(),
            )?,
            reader,
            query,
        )
    }

    fn highlights_with(
        &self,
        offsets_strategy_supplier: Arc<dyn OffsetsRetrievalStrategySupplier>,
        reader: &Arc<dyn DirectoryReader>,
        query: Arc<dyn Query>,
    ) -> Result<Vec<String>> {
        let searcher = Arc::new(IndexSearcher::new(
            Arc::clone(reader) as Arc<dyn IndexReader>
        )?);
        let max_docs = 1000;
        let rewritten_query = searcher.rewrite(query)?;
        let top_docs = searcher.search(Arc::clone(&rewritten_query), max_docs)?;

        let formatter = AsciiMatchRangeHighlighter::new(self.analyzer());
        let mut highlight_collector = HighlightCollector {
            formatter: &formatter,
            highlights: Vec::new(),
        };

        let fields_to_load_unconditionally = Arc::new(|_field_name: &str| false);
        let fields_to_load_if_with_hits = Arc::new(|_field_name: &str| true);
        let highlighter = MatchRegionRetriever::with_supplier(
            Arc::clone(&searcher),
            rewritten_query,
            offsets_strategy_supplier,
            fields_to_load_unconditionally,
            fields_to_load_if_with_hits,
        )?;
        let max_blocks_processed_in_parallel = RandomizedTest::random_int_between(1, 5);
        let max_block_size = *RandomizedTest::random_from(&[2, 10, 100]);
        let mut doc_ids: Vec<i32> = top_docs.score_docs.iter().map(|sd| sd.doc).collect();
        doc_ids.sort_unstable();
        highlighter.highlight_documents_in_blocks(
            &doc_ids,
            &mut highlight_collector,
            Arc::new(|_field: &str| i32::MAX),
            max_block_size,
            max_blocks_processed_in_parallel as usize,
        )?;

        Ok(highlight_collector.highlights)
    }
}
