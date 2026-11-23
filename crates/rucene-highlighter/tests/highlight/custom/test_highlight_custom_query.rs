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

//! Port of `custom/TestHighlightCustomQuery.java`.

use std::any::Any;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::analysis::{Analyzer, SharedTokenStream};
use rucene::error::Result;
use rucene::index::Term;
use rucene::search::boost_query::BoostQuery;
use rucene::search::query::Query;
use rucene::search::query_visitor::QueryVisitor;
use rucene::search::{IndexSearcher, TermQuery};
use rucene_highlighter::highlight::highlighter::Highlighter;
use rucene_highlighter::highlight::query_scorer::QueryScorer;
use rucene_highlighter::highlight::simple_fragmenter::SimpleFragmenter;
use rucene_highlighter::highlight::simple_html_formatter::SimpleHTMLFormatter;
use rucene_highlighter::highlight::weighted_span_term::WeightedSpanTerm;
use rucene_highlighter::highlight::weighted_span_term_extractor::{
    PositionCheckingMap, WeightedSpanTermExtractor, WeightedSpanTermExtractorOverrides,
};
use rucene_test_framework::analysis::mock_token_filter::ENGLISH_STOPSET;
use rucene_test_framework::analysis::mock_tokenizer::SIMPLE;
use rucene_test_framework::analysis::{CannedTokenStream, MockAnalyzer};

use crate::highlighter_support::core_support::{assert_equals, class, random, test, verbose};
use crate::highlighter_support::search_support::term;

const FIELD_NAME: &str = "contents";

/// `TestHighlightCustomQuery`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.highlight.custom.TestHighlightCustomQuery",
        vec![
            test("testHighlightCustomQuery", test_highlight_custom_query),
            test("testHighlightKnownQuery", test_highlight_known_query),
            test(
                "testHighlightQueryRequiringMetadata",
                test_highlight_query_requiring_metadata,
            ),
        ],
    )
}

fn test_highlight_custom_query() -> Result<()> {
    let s1 = "I call our world Flatland, not because we call it so,";

    // Verify that a query against the default field results in text being
    // highlighted
    // regardless of the field name.

    let q: Arc<dyn Query> = Arc::new(CustomQuery::new(term(FIELD_NAME, "world")));

    let expected = "I call our <B>world</B> Flatland, not because we call it so,";
    let observed = highlight_field(Arc::clone(&q), "SOME_FIELD_NAME", s1)?;
    if verbose() {
        println!("Expected: \"{expected}\nObserved: \"{observed}");
    }
    Assert::assert_equals_str(
        Some("Query in the default field results in text for *ANY* field being highlighted"),
        Some(expected),
        Some(observed.as_str()),
    );

    // Verify that a query against a named field does not result in any
    // highlighting
    // when the query field name differs from the name of the field being
    // highlighted,
    // which in this example happens to be the default field name.
    let q: Arc<dyn Query> = Arc::new(CustomQuery::new(term("text", "world")));

    let expected = s1;
    let observed = highlight_field(Arc::clone(&q), FIELD_NAME, s1)?;
    if verbose() {
        println!("Expected: \"{expected}\nObserved: \"{observed}");
    }
    Assert::assert_equals_str(
        Some(
            "Query in a named field does not result in highlighting when that field isn't in the query",
        ),
        Some(s1),
        Some(highlight_field(q, FIELD_NAME, s1)?.as_str()),
    );
    Ok(())
}

/// The anonymous `WeightedSpanTermExtractor` of `testHighlightKnownQuery`.
#[derive(Debug)]
struct KnownQueryExtractor;

impl WeightedSpanTermExtractorOverrides for KnownQueryExtractor {
    fn extract_unknown_query(
        &self,
        _this: &mut WeightedSpanTermExtractor,
        _query: &Arc<dyn Query>,
        terms: &mut PositionCheckingMap,
    ) -> Result<()> {
        terms.put("foo".to_string(), WeightedSpanTerm::new(3.0, "foo"));
        Ok(())
    }
}

fn test_highlight_known_query() -> Result<()> {
    let mut extractor = WeightedSpanTermExtractor::new();
    extractor.set_overrides(Arc::new(KnownQueryExtractor));
    let query: Arc<dyn Query> = Arc::new(TermQuery::new(term("bar", "quux")));
    let terms = extractor.get_weighted_span_terms(
        &query,
        3.0,
        Box::new(CannedTokenStream::new(Vec::new())),
    )?;
    // no foo
    let keys: Vec<String> = terms.keys().cloned().collect();
    assert_equals(&vec!["quux".to_string()], &keys);
    Ok(())
}

fn test_highlight_query_requiring_metadata() -> Result<()> {
    let text = "The blue galaxy contains many bright stars";

    // Anonymous subclass of the existing CustomQuery to inject the metadata call
    let q: Arc<dyn Query> = Arc::new(CustomQuery {
        term: term(FIELD_NAME, "galaxy"),
        metadata_lookup: true,
    });

    let expected = "The blue <B>galaxy</B> contains many bright stars";
    let observed = highlight_field(q, FIELD_NAME, text)?;

    Assert::assert_equals_str(
        Some(
            "Highlighter should support queries that perform schema/metadata lookups during rewrite",
        ),
        Some(expected),
        Some(observed.as_str()),
    );
    Ok(())
}

/// This method intended for use with `testHighlightingWithDefaultField()`
fn highlight_field(query: Arc<dyn Query>, field_name: &str, text: &str) -> Result<String> {
    let mock_analyzer = MockAnalyzer::with_filter(
        &*random(),
        Arc::clone(&SIMPLE),
        true,
        Arc::clone(&ENGLISH_STOPSET),
    );
    let token_stream = mock_analyzer.token_stream_from_str(field_name, text)?;
    // Assuming "<B>", "</B>" used to highlight
    let formatter = SimpleHTMLFormatter::new();
    let scorer = my_query_scorer(query, field_name, FIELD_NAME);
    let mut highlighter = Highlighter::with_formatter(Box::new(formatter), Box::new(scorer));
    highlighter.set_text_fragmenter(Box::new(SimpleFragmenter::with_fragment_size(i32::MAX)));

    let rv = highlighter.get_best_fragments_with_separator(
        Box::new(SharedTokenStream::new(Arc::clone(&token_stream))),
        text,
        1,
        "(FIELD TEXT TRUNCATED)",
    )?;
    token_stream.borrow_mut().close()?;
    mock_analyzer.close();
    Ok(if rv.is_empty() { text.to_string() } else { rv })
}

/// `MyWeightedSpanTermExtractor`'s override of `extractUnknownQuery`.
#[derive(Debug)]
pub struct MyWeightedSpanTermExtractor;

impl MyWeightedSpanTermExtractor {
    /// `new MyWeightedSpanTermExtractor()` / `(String defaultField)`.
    pub fn create(default_field: Option<&str>) -> WeightedSpanTermExtractor {
        let mut extractor = match default_field {
            None => WeightedSpanTermExtractor::new(),
            Some(field) => WeightedSpanTermExtractor::with_default_field(Some(field)),
        };
        extractor.set_overrides(Arc::new(MyWeightedSpanTermExtractor));
        extractor
    }
}

impl WeightedSpanTermExtractorOverrides for MyWeightedSpanTermExtractor {
    fn extract_unknown_query(
        &self,
        this: &mut WeightedSpanTermExtractor,
        query: &Arc<dyn Query>,
        terms: &mut PositionCheckingMap,
    ) -> Result<()> {
        let mut boost = 1.0f32;
        let mut query = Arc::clone(query);
        while let Some(bq) = query.as_any().downcast_ref::<BoostQuery>() {
            boost *= bq.get_boost();
            let inner = bq.get_query();
            query = inner;
        }
        if let Some(custom) = query.as_any().downcast_ref::<CustomQuery>() {
            let tq: Arc<dyn Query> = Arc::new(TermQuery::new(custom.term.clone()));
            this.extract_weighted_terms(terms, &tq, boost)?;
        }
        Ok(())
    }
}

/// `new MyQueryScorer(query, field, defaultField)`: a `QueryScorer` whose
/// `newTermExtractor` makes a [`MyWeightedSpanTermExtractor`].
pub fn my_query_scorer(query: Arc<dyn Query>, field: &str, default_field: &str) -> QueryScorer {
    let mut scorer =
        QueryScorer::with_field_and_default_field(query, Some(field), Some(default_field));
    scorer.set_term_extractor_factory(Arc::new(MyWeightedSpanTermExtractor::create));
    scorer
}

/// `CustomQuery`, and (`metadata_lookup`) the anonymous subclass of
/// `testHighlightQueryRequiringMetadata`, whose `rewrite` first reads the
/// field infos of a leaf reader.
#[derive(Debug)]
pub struct CustomQuery {
    term: Term,
    metadata_lookup: bool,
}

impl CustomQuery {
    /// `new CustomQuery(term)`.
    pub fn new(term: Term) -> Self {
        Self {
            term,
            metadata_lookup: false,
        }
    }
}

impl Query for CustomQuery {
    fn to_query_string(&self, field: &str) -> String {
        TermQuery::new(self.term.clone()).to_query_string(field)
    }

    fn rewrite(&self, index_searcher: &IndexSearcher) -> Result<Option<Arc<dyn Query>>> {
        if self.metadata_lookup {
            let reader = Arc::clone(index_searcher.get_index_reader());
            if let Some(leaf) = reader.get_context()?.as_leaf() {
                leaf.leaf_reader().get_field_infos();
            }
        }
        Ok(Some(Arc::new(TermQuery::new(self.term.clone()))))
    }

    fn visit(&self, visitor: &mut dyn QueryVisitor) {
        visitor.consume_terms(self, std::slice::from_ref(&self.term));
    }

    fn query_hash(&self) -> i32 {
        self.class_hash().wrapping_add(self.term.hash_code())
    }

    fn query_eq(&self, other: &dyn Query) -> bool {
        self.same_class_as(other)
            && other
                .as_any()
                .downcast_ref::<CustomQuery>()
                .is_some_and(|o| o.term == self.term)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn java_class_name(&self) -> &'static str {
        if self.metadata_lookup {
            "org.apache.lucene.search.highlight.custom.TestHighlightCustomQuery$2"
        } else {
            "org.apache.lucene.search.highlight.custom.TestHighlightCustomQuery$CustomQuery"
        }
    }
}
