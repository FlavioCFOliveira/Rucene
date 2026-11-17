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

//! Port of `TestSpanMultiTermQueryWrapper.java`: tests for
//! `SpanMultiTermQueryWrapper`, wrapping a few MultiTermQueries.

use std::any::Any;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::{Document, Store};
use rucene::error::{LuceneError, Result};
use rucene::index::directory_reader::DirectoryReader;
use rucene::index::Term;
use rucene::search::fuzzy_query::FuzzyQuery;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::multi_term_query::{MultiTermQuery, RewriteMethod};
use rucene::search::prefix_query::PrefixQuery;
use rucene::search::query::Query;
use rucene::search::regexp_query::RegexpQuery;
use rucene::search::wildcard_query::WildcardQuery;
use rucene_queries::spans::span_first_query::SpanFirstQuery;
use rucene_queries::spans::span_multi_term_query_wrapper::{
    SpanMultiTermQueryWrapper, SpanRewriteMethod,
};
use rucene_queries::spans::span_near_query::SpanNearQuery;
use rucene_queries::spans::span_not_query::SpanNotQuery;
use rucene_queries::spans::span_or_query::SpanOrQuery;
use rucene_queries::spans::span_position_range_query::SpanPositionRangeQuery;
use rucene_queries::spans::span_query::SpanQuery;
use rucene_queries::spans::span_term_query::SpanTermQuery;
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::LuceneTestCase;

use crate::queries_support::core_support::random;
use crate::queries_support::search_support::{field_mut, fixture_class, ftest, set_up, tear_down};

/// The instance of `TestSpanMultiTermQueryWrapper`.
#[derive(Default)]
pub struct SpanMultiTermQueryWrapperTest {
    directory: Option<TestDirectory>,
    reader: Option<Arc<dyn DirectoryReader>>,
    searcher: Option<IndexSearcher>,
}

impl SpanMultiTermQueryWrapperTest {
    fn count(&self, q: Arc<dyn SpanQuery>) -> Result<i64> {
        Ok(i64::from(
            self.searcher
                .as_ref()
                .expect("setUp ran")
                .count(q as Arc<dyn Query>)?,
        ))
    }
}

/// `TestSpanMultiTermQueryWrapper`.
pub fn class_def() -> Arc<Class> {
    fixture_class::<SpanMultiTermQueryWrapperTest>(
        "org.apache.lucene.queries.spans.TestSpanMultiTermQueryWrapper",
        vec![
            set_up(do_set_up),
            tear_down(do_tear_down),
            ftest("testWildcard", test_wildcard),
            ftest("testPrefix", test_prefix),
            ftest("testFuzzy", test_fuzzy),
            ftest("testFuzzy2", test_fuzzy2),
            ftest(
                "testNoSuchMultiTermsInNear",
                test_no_such_multi_terms_in_near,
            ),
            ftest(
                "testNoSuchMultiTermsInNotNear",
                test_no_such_multi_terms_in_not_near,
            ),
            ftest("testNoSuchMultiTermsInOr", test_no_such_multi_terms_in_or),
            ftest(
                "testNoSuchMultiTermsInSpanFirst",
                test_no_such_multi_terms_in_span_first,
            ),
            ftest(
                "testWrappedQueryIsNotModified",
                test_wrapped_query_is_not_modified,
            ),
        ],
    )
}

fn do_set_up(f: &mut SpanMultiTermQueryWrapperTest) -> Result<()> {
    let directory = LuceneTestCase::new_directory();
    let iw = RandomIndexWriter::new_with_mock_analyzer(&*random(), directory.as_directory())?;
    let mut doc = Document::new();
    let field = LuceneTestCase::new_text_field("field", "", Store::NO);
    doc.add(field);

    field_mut(&mut doc, "field").set_string_value("quick brown fox".to_string())?;
    iw.add_document(&doc)?;
    field_mut(&mut doc, "field").set_string_value("jumps over lazy broun dog".to_string())?;
    iw.add_document(&doc)?;
    field_mut(&mut doc, "field")
        .set_string_value("jumps over extremely very lazy broxn dog".to_string())?;
    iw.add_document(&doc)?;
    let reader = iw.get_reader()?;
    iw.close()?;
    f.searcher = Some(LuceneTestCase::new_searcher(Arc::clone(&reader)));
    f.reader = Some(reader);
    f.directory = Some(directory);
    Ok(())
}

fn do_tear_down(f: &mut SpanMultiTermQueryWrapperTest) -> Result<()> {
    f.searcher = None;
    f.reader.take().expect("setUp ran").close()?;
    f.directory.take().expect("setUp ran").close()
}

fn term(text: &str) -> Term {
    Term::from_text("field", text)
}

/// `new SpanMultiTermQueryWrapper<>(query)`.
fn wrap(query: Arc<dyn MultiTermQuery>) -> Arc<dyn SpanQuery> {
    Arc::new(SpanMultiTermQueryWrapper::new(query))
}

fn assert_count(expected: i64, actual: i64) {
    Assert::assert_equals(expected, actual);
}

fn test_wildcard(f: &mut SpanMultiTermQueryWrapperTest) -> Result<()> {
    let wq = Arc::new(WildcardQuery::new(term("bro?n"))?);
    let swq = wrap(wq);
    // will only match quick brown fox
    let sfq: Arc<dyn SpanQuery> = Arc::new(SpanFirstQuery::new(swq, 2));
    assert_count(1, f.count(sfq)?);
    Ok(())
}

fn test_prefix(f: &mut SpanMultiTermQueryWrapperTest) -> Result<()> {
    let wq = Arc::new(WildcardQuery::new(term("extrem*"))?);
    let swq = wrap(wq);
    // will only match "jumps over extremely very lazy broxn dog"
    let sfq: Arc<dyn SpanQuery> = Arc::new(SpanFirstQuery::new(swq, 3));
    assert_count(1, f.count(sfq)?);
    Ok(())
}

fn test_fuzzy(f: &mut SpanMultiTermQueryWrapperTest) -> Result<()> {
    let fq = Arc::new(FuzzyQuery::new(term("broan"))?);
    let sfq = wrap(fq);
    // will not match quick brown fox
    let sprq: Arc<dyn SpanQuery> = Arc::new(SpanPositionRangeQuery::new(sfq, 3, 6));
    assert_count(2, f.count(sprq)?);
    Ok(())
}

fn test_fuzzy2(f: &mut SpanMultiTermQueryWrapperTest) -> Result<()> {
    // maximum of 1 term expansion
    let fq = Arc::new(FuzzyQuery::with_expansions(term("broan"), 1, 0, 1, false)?);
    let sfq = wrap(fq);
    // will only match jumps over lazy broun dog
    let sprq: Arc<dyn SpanQuery> = Arc::new(SpanPositionRangeQuery::new(sfq, 0, 100));
    assert_count(1, f.count(sprq)?);
    Ok(())
}

fn near(clauses: Vec<Arc<dyn SpanQuery>>) -> Result<Arc<dyn SpanQuery>> {
    Ok(Arc::new(SpanNearQuery::new(clauses, 1, true)?))
}

fn test_no_such_multi_terms_in_near(f: &mut SpanMultiTermQueryWrapperTest) -> Result<()> {
    // test to make sure non existent multiterms aren't throwing null pointer exceptions
    let fuzzy_no_such = Arc::new(FuzzyQuery::with_expansions(term("noSuch"), 1, 0, 1, false)?);
    let span_no_such = wrap(fuzzy_no_such);
    let term_q: Arc<dyn SpanQuery> = Arc::new(SpanTermQuery::new(term("brown")));
    let n = near(vec![Arc::clone(&term_q), Arc::clone(&span_no_such)])?;
    assert_count(0, f.count(n)?);

    // flip order
    let n = near(vec![Arc::clone(&span_no_such), Arc::clone(&term_q)])?;
    assert_count(0, f.count(n)?);

    let wc_no_such = Arc::new(WildcardQuery::new(term("noSuch*"))?);
    let span_wc_no_such = wrap(wc_no_such);
    let n = near(vec![Arc::clone(&term_q), span_wc_no_such])?;
    assert_count(0, f.count(n)?);

    let rgx_no_such = Arc::new(RegexpQuery::new(term("noSuch"))?);
    let span_rgx_no_such = wrap(rgx_no_such);
    let n = near(vec![Arc::clone(&term_q), span_rgx_no_such])?;
    assert_count(0, f.count(n)?);

    let prfx_no_such = Arc::new(PrefixQuery::new(term("noSuch"))?);
    let span_prfx_no_such = wrap(prfx_no_such);
    let n = near(vec![Arc::clone(&term_q), Arc::clone(&span_prfx_no_such)])?;
    assert_count(0, f.count(n)?);

    // test single noSuch
    let n = near(vec![Arc::clone(&span_prfx_no_such)])?;
    assert_count(0, f.count(n)?);

    // test double noSuch
    let n = near(vec![Arc::clone(&span_prfx_no_such), span_prfx_no_such])?;
    assert_count(0, f.count(n)?);
    Ok(())
}

fn not_near(
    include: Arc<dyn SpanQuery>,
    exclude: Arc<dyn SpanQuery>,
    pre: i32,
    post: i32,
) -> Result<Arc<dyn SpanQuery>> {
    Ok(Arc::new(SpanNotQuery::with_pre_post(
        include, exclude, pre, post,
    )?))
}

fn test_no_such_multi_terms_in_not_near(f: &mut SpanMultiTermQueryWrapperTest) -> Result<()> {
    // test to make sure non existent multiterms aren't throwing non-matching field exceptions
    let fuzzy_no_such = Arc::new(FuzzyQuery::with_expansions(term("noSuch"), 1, 0, 1, false)?);
    let span_no_such = wrap(fuzzy_no_such);
    let term_q: Arc<dyn SpanQuery> = Arc::new(SpanTermQuery::new(term("brown")));
    let nn = not_near(Arc::clone(&term_q), Arc::clone(&span_no_such), 0, 0)?;
    assert_count(1, f.count(nn)?);

    // flip
    let nn = not_near(Arc::clone(&span_no_such), Arc::clone(&term_q), 0, 0)?;
    assert_count(0, f.count(nn)?);

    // both noSuch
    let nn = not_near(Arc::clone(&span_no_such), Arc::clone(&span_no_such), 0, 0)?;
    assert_count(0, f.count(nn)?);

    let wc_no_such = Arc::new(WildcardQuery::new(term("noSuch*"))?);
    let span_wc_no_such = wrap(wc_no_such);
    let nn = not_near(Arc::clone(&term_q), span_wc_no_such, 0, 0)?;
    assert_count(1, f.count(nn)?);

    let rgx_no_such = Arc::new(RegexpQuery::new(term("noSuch"))?);
    let span_rgx_no_such = wrap(rgx_no_such);
    let nn = not_near(Arc::clone(&term_q), span_rgx_no_such, 1, 1)?;
    assert_count(1, f.count(nn)?);

    let prfx_no_such = Arc::new(PrefixQuery::new(term("noSuch"))?);
    let span_prfx_no_such = wrap(prfx_no_such);
    let nn = not_near(term_q, span_prfx_no_such, 1, 1)?;
    assert_count(1, f.count(nn)?);
    Ok(())
}

fn or(clauses: Vec<Arc<dyn SpanQuery>>) -> Result<Arc<dyn SpanQuery>> {
    Ok(Arc::new(SpanOrQuery::new(clauses)?))
}

fn test_no_such_multi_terms_in_or(f: &mut SpanMultiTermQueryWrapperTest) -> Result<()> {
    // test to make sure non existent multiterms aren't throwing null pointer exceptions
    let fuzzy_no_such = Arc::new(FuzzyQuery::with_expansions(term("noSuch"), 1, 0, 1, false)?);
    let span_no_such = wrap(fuzzy_no_such);
    let term_q: Arc<dyn SpanQuery> = Arc::new(SpanTermQuery::new(term("brown")));
    let n = or(vec![Arc::clone(&term_q), Arc::clone(&span_no_such)])?;
    assert_count(1, f.count(n)?);

    // flip
    let n = or(vec![span_no_such, Arc::clone(&term_q)])?;
    assert_count(1, f.count(n)?);

    let wc_no_such = Arc::new(WildcardQuery::new(term("noSuch*"))?);
    let span_wc_no_such = wrap(wc_no_such);
    let n = or(vec![Arc::clone(&term_q), span_wc_no_such])?;
    assert_count(1, f.count(n)?);

    let rgx_no_such = Arc::new(RegexpQuery::new(term("noSuch"))?);
    let span_rgx_no_such = wrap(rgx_no_such);
    let n = or(vec![Arc::clone(&term_q), span_rgx_no_such])?;
    assert_count(1, f.count(n)?);

    let prfx_no_such = Arc::new(PrefixQuery::new(term("noSuch"))?);
    let span_prfx_no_such = wrap(prfx_no_such);
    let n = or(vec![term_q, Arc::clone(&span_prfx_no_such)])?;
    assert_count(1, f.count(n)?);

    let n = or(vec![Arc::clone(&span_prfx_no_such)])?;
    assert_count(0, f.count(n)?);

    let n = or(vec![Arc::clone(&span_prfx_no_such), span_prfx_no_such])?;
    assert_count(0, f.count(n)?);
    Ok(())
}

fn test_no_such_multi_terms_in_span_first(f: &mut SpanMultiTermQueryWrapperTest) -> Result<()> {
    // this hasn't been a problem
    let fuzzy_no_such = Arc::new(FuzzyQuery::with_expansions(term("noSuch"), 1, 0, 1, false)?);
    let span_no_such = wrap(fuzzy_no_such);
    let span_first: Arc<dyn SpanQuery> = Arc::new(SpanFirstQuery::new(span_no_such, 10));
    assert_count(0, f.count(span_first)?);

    let wc_no_such = Arc::new(WildcardQuery::new(term("noSuch*"))?);
    let span_wc_no_such = wrap(wc_no_such);
    let span_first: Arc<dyn SpanQuery> = Arc::new(SpanFirstQuery::new(span_wc_no_such, 10));
    assert_count(0, f.count(span_first)?);

    let rgx_no_such = Arc::new(RegexpQuery::new(term("noSuch"))?);
    let span_rgx_no_such = wrap(rgx_no_such);
    let span_first: Arc<dyn SpanQuery> = Arc::new(SpanFirstQuery::new(span_rgx_no_such, 10));
    assert_count(0, f.count(span_first)?);

    let prfx_no_such = Arc::new(PrefixQuery::new(term("noSuch"))?);
    let span_prfx_no_such = wrap(prfx_no_such);
    let span_first: Arc<dyn SpanQuery> = Arc::new(SpanFirstQuery::new(span_prfx_no_such, 10));
    assert_count(0, f.count(span_first)?);
    Ok(())
}

/// The anonymous `SpanRewriteMethod` of `testWrappedQueryIsNotModified`,
/// whose `rewrite` returns `null`.
///
/// Rust's [`SpanRewriteMethod::rewrite_span`] returns a query, never `null`;
/// it answers the `NullPointerException` the caller of Java's `null` would
/// raise. The test never rewrites.
#[derive(Debug)]
struct NullSpanRewriteMethod;

impl SpanRewriteMethod for NullSpanRewriteMethod {
    fn rewrite_span(
        &self,
        _index_searcher: &IndexSearcher,
        _query: Arc<dyn MultiTermQuery>,
    ) -> Result<Arc<dyn SpanQuery>> {
        Err(LuceneError::null_pointer())
    }
}

impl RewriteMethod for NullSpanRewriteMethod {
    fn rewrite(
        &self,
        index_searcher: &IndexSearcher,
        query: Arc<dyn MultiTermQuery>,
    ) -> Result<Arc<dyn Query>> {
        Ok(self.rewrite_span(index_searcher, query)?)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    /// `Object.hashCode()`: the identity, standing for the address.
    fn method_hash(&self) -> i32 {
        (self as *const Self as usize) as i32
    }
}

fn test_wrapped_query_is_not_modified(_f: &mut SpanMultiTermQueryWrapperTest) -> Result<()> {
    let pq = Arc::new(PrefixQuery::new(term("test"))?);
    let pq_hash = pq.query_hash();
    let mut wrapper = SpanMultiTermQueryWrapper::new(Arc::clone(&pq) as Arc<dyn MultiTermQuery>);
    Assert::assert_equals(pq_hash, pq.query_hash());
    wrapper.set_rewrite_method(Arc::new(NullSpanRewriteMethod));
    Assert::assert_equals(pq_hash, pq.query_hash());
    Ok(())
}
