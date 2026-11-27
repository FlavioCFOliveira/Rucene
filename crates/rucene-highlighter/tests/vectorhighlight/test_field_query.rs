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

//! Port of `TestFieldQuery.java`.

use std::any::Any;
use std::collections::HashMap;
use std::sync::Arc;

use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::index::Term;
use rucene::search::boolean_clause::Occur;
use rucene::search::boolean_query::Builder as BooleanQueryBuilder;
use rucene::search::boost_query::BoostQuery;
use rucene::search::constant_score_query::ConstantScoreQuery;
use rucene::search::prefix_query::PrefixQuery;
use rucene::search::query::Query;
use rucene::search::query_visitor::QueryVisitor;
use rucene::search::regexp_query::RegexpQuery;
use rucene::search::term_range_query::TermRangeQuery;
use rucene::search::wildcard_query::WildcardQuery;
use rucene::search::TermQuery;
use rucene::util::BytesRef;
use rucene_highlighter::vectorhighlight::field_query::{FieldQuery, QueryPhraseMap};
use rucene_highlighter::vectorhighlight::field_term_stack::TermInfo;
use rucene_test_framework::util::LuceneTestCase;

use super::abstract_test_case::{
    analyze, assert_collection_queries, dmq, make1d1f_index, make_index_str_mv, pq, pq_b, pq_f,
    pq_f_b, pq_f_bs, to_phrase_query, tq, tq_b, tq_f, vh_class, Fixture, F, F1, F2,
};
use crate::highlighter_support::core_support::random;
use crate::highlighter_support::search_support::ftest;

/// `initBoost()`: "Set boost to a random value each time it is called"; the
/// value of the field `boost` it sets is returned, as each test reads it
/// right after the call.
fn init_boost() -> f32 {
    if LuceneTestCase::usually() {
        1.0
    } else {
        random().next_float() * 10000.0
    }
}

/// `TestFieldQuery`.
pub fn class_def() -> Arc<Class> {
    vh_class(
        "org.apache.lucene.search.vectorhighlight.TestFieldQuery",
        vec![
            ftest("testFlattenBoolean", test_flatten_boolean),
            ftest(
                "testFlattenDisjunctionMaxQuery",
                test_flatten_disjunction_max_query,
            ),
            ftest("testFlattenTermAndPhrase", test_flatten_term_and_phrase),
            ftest(
                "testFlattenTermAndPhrase2gram",
                test_flatten_term_and_phrase_2gram,
            ),
            ftest("testFlatten1TermPhrase", test_flatten_1_term_phrase),
            ftest("testExpand", test_expand),
            ftest("testNoExpand", test_no_expand),
            ftest("testExpandNotFieldMatch", test_expand_not_field_match),
            ftest("testGetFieldTermMap", test_get_field_term_map),
            ftest("testGetRootMap", test_get_root_map),
            ftest(
                "testGetRootMapNotFieldMatch",
                test_get_root_map_not_field_match,
            ),
            ftest("testGetTermSet", test_get_term_set),
            ftest("testQueryPhraseMap1Term", test_query_phrase_map_1_term),
            ftest("testQueryPhraseMap1Phrase", test_query_phrase_map_1_phrase),
            ftest(
                "testQueryPhraseMap1PhraseAnother",
                test_query_phrase_map_1_phrase_another,
            ),
            ftest(
                "testQueryPhraseMap2Phrases",
                test_query_phrase_map_2_phrases,
            ),
            ftest(
                "testQueryPhraseMap2PhrasesFields",
                test_query_phrase_map_2_phrases_fields,
            ),
            ftest(
                "testQueryPhraseMapOverlapPhrases",
                test_query_phrase_map_overlap_phrases,
            ),
            ftest(
                "testQueryPhraseMapOverlapPhrases2",
                test_query_phrase_map_overlap_phrases_2,
            ),
            ftest(
                "testQueryPhraseMapOverlapPhrases3",
                test_query_phrase_map_overlap_phrases_3,
            ),
            ftest(
                "testQueryPhraseMapOverlap2gram",
                test_query_phrase_map_overlap_2gram,
            ),
            ftest("testSearchPhrase", test_search_phrase),
            ftest("testSearchPhraseSlop", test_search_phrase_slop),
            ftest("testHighlightQuery", test_highlight_query),
            ftest("testPrefixQuery", test_prefix_query),
            ftest("testRegexpQuery", test_regexp_query),
            ftest("testRangeQuery", test_range_query),
            ftest("testStopRewrite", test_stop_rewrite),
            ftest(
                "testFlattenConstantScoreQuery",
                test_flatten_constant_score_query,
            ),
        ],
    )
}

fn boost_query(query: Arc<dyn Query>, boost: f32) -> Result<Arc<dyn Query>> {
    Ok(Arc::new(BoostQuery::new(query, boost)?))
}

fn new_fq(query: &Arc<dyn Query>, phrase_highlight: bool, field_match: bool) -> Result<FieldQuery> {
    FieldQuery::without_reader(query, phrase_highlight, field_match)
}

/// `fq.flatten(query, searcher, flatQueries = new HashSet<>(), 1f)`.
fn flatten(f: &Fixture, fq: &FieldQuery, query: &Arc<dyn Query>) -> Result<Vec<Arc<dyn Query>>> {
    let mut flat_queries = Vec::new();
    fq.flatten(query, f.searcher.as_ref(), &mut flat_queries, 1.0)?;
    Ok(flat_queries)
}

fn test_flatten_boolean(f: &mut Fixture) -> Result<()> {
    let boost = init_boost();
    let mut boolean_query_b = BooleanQueryBuilder::new();
    boolean_query_b.add(tq("A"), Occur::MUST)?;
    boolean_query_b.add(tq("B"), Occur::MUST)?;
    boolean_query_b.add(tq("C"), Occur::SHOULD)?;

    let mut inner_query = BooleanQueryBuilder::new();
    inner_query.add(tq("D"), Occur::MUST)?;
    inner_query.add(tq("E"), Occur::MUST)?;
    boolean_query_b.add(Arc::new(inner_query.build()), Occur::MUST_NOT)?;

    let boolean_query = boost_query(Arc::new(boolean_query_b.build()), boost)?;

    let fq = new_fq(&boolean_query, true, true)?;
    let flat_queries = flatten(f, &fq, &boolean_query)?;
    assert_collection_queries(
        &flat_queries,
        &[tq_b(boost, "A"), tq_b(boost, "B"), tq_b(boost, "C")],
    );
    Ok(())
}

fn test_flatten_disjunction_max_query(f: &mut Fixture) -> Result<()> {
    let boost = init_boost();
    let query = dmq(vec![tq("A"), tq("B"), pq_f(&["C", "D"])]);
    let query = boost_query(query, boost)?;
    let fq = new_fq(&query, true, true)?;
    let flat_queries = flatten(f, &fq, &query)?;
    assert_collection_queries(
        &flat_queries,
        &[
            tq_b(boost, "A"),
            tq_b(boost, "B"),
            pq_f_b(boost, &["C", "D"]),
        ],
    );
    Ok(())
}

fn test_flatten_term_and_phrase(f: &mut Fixture) -> Result<()> {
    let boost = init_boost();
    let mut boolean_query_b = BooleanQueryBuilder::new();
    boolean_query_b.add(tq("A"), Occur::MUST)?;
    boolean_query_b.add(pq_f(&["B", "C"]), Occur::MUST)?;
    let boolean_query = boost_query(Arc::new(boolean_query_b.build()), boost)?;

    let fq = new_fq(&boolean_query, true, true)?;
    let flat_queries = flatten(f, &fq, &boolean_query)?;
    assert_collection_queries(
        &flat_queries,
        &[tq_b(boost, "A"), pq_f_b(boost, &["B", "C"])],
    );
    Ok(())
}

fn test_flatten_term_and_phrase_2gram(f: &mut Fixture) -> Result<()> {
    let analyzer_b = Arc::clone(f.analyzer_b.as_ref().expect("setUp ran"));
    let mut query = BooleanQueryBuilder::new();
    query.add(
        Arc::new(TermQuery::new(Term::from_text(F, "AA"))),
        Occur::MUST,
    )?;
    query.add(
        Arc::new(to_phrase_query(&analyze("BCD", F, &*analyzer_b)?, F)),
        Occur::MUST,
    )?;
    query.add(
        Arc::new(to_phrase_query(&analyze("EFGH", F, &*analyzer_b)?, F)),
        Occur::SHOULD,
    )?;

    let built: Arc<dyn Query> = Arc::new(query.build());
    let fq = new_fq(&built, true, true)?;
    let flat_queries = flatten(f, &fq, &(Arc::new(query.build()) as Arc<dyn Query>))?;
    assert_collection_queries(
        &flat_queries,
        &[tq("AA"), pq_f(&["BC", "CD"]), pq_f(&["EF", "FG", "GH"])],
    );
    Ok(())
}

fn test_flatten_1_term_phrase(f: &mut Fixture) -> Result<()> {
    let query = pq_f(&["A"]);
    let fq = new_fq(&query, true, true)?;
    let flat_queries = flatten(f, &fq, &query)?;
    assert_collection_queries(&flat_queries, &[tq("A")]);
    Ok(())
}

fn assert_expand(
    fq: &FieldQuery,
    flat: Vec<Arc<dyn Query>>,
    expected: &[Arc<dyn Query>],
) -> Result<()> {
    assert_collection_queries(&fq.expand_for_tests(flat)?, expected);
    Ok(())
}

fn test_expand(_f: &mut Fixture) -> Result<()> {
    let dummy = pq_f(&["DUMMY"]);
    let fq = new_fq(&dummy, true, true)?;

    // "a b","b c" => "a b","b c","a b c"
    assert_expand(
        &fq,
        vec![pq_f(&["a", "b"]), pq_f(&["b", "c"])],
        &[pq_f(&["a", "b"]), pq_f(&["b", "c"]), pq_f(&["a", "b", "c"])],
    )?;

    // "a b","b c d" => "a b","b c d","a b c d"
    assert_expand(
        &fq,
        vec![pq_f(&["a", "b"]), pq_f(&["b", "c", "d"])],
        &[
            pq_f(&["a", "b"]),
            pq_f(&["b", "c", "d"]),
            pq_f(&["a", "b", "c", "d"]),
        ],
    )?;

    // "a b c","b c d" => "a b c","b c d","a b c d"
    assert_expand(
        &fq,
        vec![pq_f(&["a", "b", "c"]), pq_f(&["b", "c", "d"])],
        &[
            pq_f(&["a", "b", "c"]),
            pq_f(&["b", "c", "d"]),
            pq_f(&["a", "b", "c", "d"]),
        ],
    )?;

    // "a b c","c d e" => "a b c","c d e","a b c d e"
    assert_expand(
        &fq,
        vec![pq_f(&["a", "b", "c"]), pq_f(&["c", "d", "e"])],
        &[
            pq_f(&["a", "b", "c"]),
            pq_f(&["c", "d", "e"]),
            pq_f(&["a", "b", "c", "d", "e"]),
        ],
    )?;

    // "a b c d","b c" => "a b c d","b c"
    assert_expand(
        &fq,
        vec![pq_f(&["a", "b", "c", "d"]), pq_f(&["b", "c"])],
        &[pq_f(&["a", "b", "c", "d"]), pq_f(&["b", "c"])],
    )?;

    // "a b b","b c" => "a b b","b c","a b b c"
    assert_expand(
        &fq,
        vec![pq_f(&["a", "b", "b"]), pq_f(&["b", "c"])],
        &[
            pq_f(&["a", "b", "b"]),
            pq_f(&["b", "c"]),
            pq_f(&["a", "b", "b", "c"]),
        ],
    )?;

    // "a b","b a" => "a b","b a","a b a", "b a b"
    assert_expand(
        &fq,
        vec![pq_f(&["a", "b"]), pq_f(&["b", "a"])],
        &[
            pq_f(&["a", "b"]),
            pq_f(&["b", "a"]),
            pq_f(&["a", "b", "a"]),
            pq_f(&["b", "a", "b"]),
        ],
    )?;

    // "a b","a b c" => "a b","a b c"
    assert_expand(
        &fq,
        vec![pq_f(&["a", "b"]), pq_f(&["a", "b", "c"])],
        &[pq_f(&["a", "b"]), pq_f(&["a", "b", "c"])],
    )
}

fn test_no_expand(_f: &mut Fixture) -> Result<()> {
    let dummy = pq_f(&["DUMMY"]);
    let fq = new_fq(&dummy, true, true)?;

    // "a b","c d" => "a b","c d"
    assert_expand(
        &fq,
        vec![pq_f(&["a", "b"]), pq_f(&["c", "d"])],
        &[pq_f(&["a", "b"]), pq_f(&["c", "d"])],
    )?;

    // "a","a b" => "a", "a b"
    assert_expand(
        &fq,
        vec![tq("a"), pq_f(&["a", "b"])],
        &[tq("a"), pq_f(&["a", "b"])],
    )?;

    // "a b","b" => "a b", "b"
    assert_expand(
        &fq,
        vec![pq_f(&["a", "b"]), tq("b")],
        &[pq_f(&["a", "b"]), tq("b")],
    )?;

    // "a b c","b c" => "a b c","b c"
    assert_expand(
        &fq,
        vec![pq_f(&["a", "b", "c"]), pq_f(&["b", "c"])],
        &[pq_f(&["a", "b", "c"]), pq_f(&["b", "c"])],
    )?;

    // "a b","a b c" => "a b","a b c"
    assert_expand(
        &fq,
        vec![pq_f(&["a", "b"]), pq_f(&["a", "b", "c"])],
        &[pq_f(&["a", "b"]), pq_f(&["a", "b", "c"])],
    )?;

    // "a b c","b d e" => "a b c","b d e"
    assert_expand(
        &fq,
        vec![pq_f(&["a", "b", "c"]), pq_f(&["b", "d", "e"])],
        &[pq_f(&["a", "b", "c"]), pq_f(&["b", "d", "e"])],
    )
}

fn test_expand_not_field_match(_f: &mut Fixture) -> Result<()> {
    let dummy = pq_f(&["DUMMY"]);
    let fq = new_fq(&dummy, true, false)?;

    // f1:"a b",f2:"b c" => f1:"a b",f2:"b c",f1:"a b c"
    assert_expand(
        &fq,
        vec![pq(F1, &["a", "b"]), pq(F2, &["b", "c"])],
        &[
            pq(F1, &["a", "b"]),
            pq(F2, &["b", "c"]),
            pq(F1, &["a", "b", "c"]),
        ],
    )
}

fn test_get_field_term_map(_f: &mut Fixture) -> Result<()> {
    let query = tq("a");
    let fq = new_fq(&query, true, true)?;

    let pqm = fq.get_field_term_map(F, "a");
    Assert::assert_true(pqm.is_some());
    Assert::assert_true(pqm.unwrap().is_terminal());

    Assert::assert_true(fq.get_field_term_map(F, "b").is_none());

    Assert::assert_true(fq.get_field_term_map(F1, "a").is_none());
    Ok(())
}

/// `fq.getRootMap(query)` as an identity.
fn root_map(fq: &mut FieldQuery, query: Arc<dyn Query>) -> Result<*const QueryPhraseMap> {
    Ok(fq.get_root_map_for_tests(&*query)? as *const QueryPhraseMap)
}

fn test_get_root_map(_f: &mut Fixture) -> Result<()> {
    let dummy = pq_f(&["DUMMY"]);
    let mut fq = new_fq(&dummy, true, true)?;

    let root_map1 = root_map(&mut fq, tq("a"))?;
    let root_map2 = root_map(&mut fq, tq("a"))?;
    Assert::assert_true(root_map1 == root_map2);
    let root_map3 = root_map(&mut fq, tq("b"))?;
    Assert::assert_true(root_map1 == root_map3);
    let root_map4 = root_map(&mut fq, tq_f(F1, "b"))?;
    Assert::assert_false(root_map4 == root_map3);
    Ok(())
}

fn test_get_root_map_not_field_match(_f: &mut Fixture) -> Result<()> {
    let dummy = pq_f(&["DUMMY"]);
    let mut fq = new_fq(&dummy, true, false)?;

    let root_map1 = root_map(&mut fq, tq("a"))?;
    let root_map2 = root_map(&mut fq, tq("a"))?;
    Assert::assert_true(root_map1 == root_map2);
    let root_map3 = root_map(&mut fq, tq("b"))?;
    Assert::assert_true(root_map1 == root_map3);
    let root_map4 = root_map(&mut fq, tq_f(F1, "b"))?;
    Assert::assert_true(root_map4 == root_map3);
    Ok(())
}

fn test_get_term_set(_f: &mut Fixture) -> Result<()> {
    let mut query = BooleanQueryBuilder::new();
    query.add(
        Arc::new(TermQuery::new(Term::from_text(F, "A"))),
        Occur::MUST,
    )?;
    query.add(
        Arc::new(TermQuery::new(Term::from_text(F, "B"))),
        Occur::MUST,
    )?;
    query.add(
        Arc::new(TermQuery::new(Term::from_text("x", "C"))),
        Occur::SHOULD,
    )?;

    let mut inner_query = BooleanQueryBuilder::new();
    inner_query.add(
        Arc::new(TermQuery::new(Term::from_text(F, "D"))),
        Occur::MUST,
    )?;
    inner_query.add(
        Arc::new(TermQuery::new(Term::from_text(F, "E"))),
        Occur::MUST,
    )?;
    query.add(Arc::new(inner_query.build()), Occur::MUST_NOT)?;

    let fq = new_fq(&(Arc::new(query.build()) as Arc<dyn Query>), true, true)?;
    Assert::assert_equals(2, fq.term_set_map().len());
    let term_set = fq.get_term_set_for_tests(F).expect("a term set");
    Assert::assert_equals(2, term_set.len());
    Assert::assert_true(term_set.contains("A"));
    Assert::assert_true(term_set.contains("B"));
    let term_set = fq.get_term_set_for_tests("x").expect("a term set");
    Assert::assert_equals(1, term_set.len());
    Assert::assert_true(term_set.contains("C"));
    Assert::assert_true(fq.get_term_set_for_tests("y").is_none());
    Ok(())
}

type Maps = HashMap<Option<String>, QueryPhraseMap>;

/// `map.get(key)`.
fn get<'a>(map: &'a Maps, key: Option<&str>) -> Option<&'a QueryPhraseMap> {
    map.get(&key.map(str::to_string))
}

/// `assertEquals(1, map.size()); assertNull(map.get(absent)); assertNotNull(map.get(present));`
/// and the present map.
fn only<'a>(map: &'a Maps, absent: &[Option<&str>], present: Option<&str>) -> &'a QueryPhraseMap {
    Assert::assert_equals(1, map.len());
    for a in absent {
        Assert::assert_true(get(map, *a).is_none());
    }
    let qpm = get(map, present);
    Assert::assert_true(qpm.is_some());
    qpm.unwrap()
}

/// `assertNotNull(qpm.subMap.get(term))` and the sub map.
fn sub<'a>(qpm: &'a QueryPhraseMap, term: &str) -> &'a QueryPhraseMap {
    let child = qpm.sub_map().get(term);
    Assert::assert_true(child.is_some());
    child.unwrap()
}

fn size(qpm: &QueryPhraseMap, expected: usize) {
    Assert::assert_equals(expected, qpm.sub_map().len());
}

fn terminal(qpm: &QueryPhraseMap, expected: bool) {
    Assert::assert_equals(expected, qpm.is_terminal());
}

fn boost(qpm: &QueryPhraseMap, expected: f32) {
    Assert::assert_equals_float_msg(None, expected, qpm.get_boost(), 0.0);
}

/// The single term `a` of `testQueryPhraseMap1Term` under `qpm`.
fn assert_single_a(qpm: &QueryPhraseMap) {
    size(qpm, 1);
    let a = sub(qpm, "a");
    terminal(a, true);
    boost(a, 1.0);
}

fn test_query_phrase_map_1_term(_f: &mut Fixture) -> Result<()> {
    let query = tq("a");

    // phraseHighlight = true, fieldMatch = true
    let fq = new_fq(&query, true, true)?;
    assert_single_a(only(fq.root_maps(), &[None], Some(F)));

    // phraseHighlight = true, fieldMatch = false
    let fq = new_fq(&query, true, false)?;
    assert_single_a(only(fq.root_maps(), &[Some(F)], None));

    // phraseHighlight = false, fieldMatch = true
    let fq = new_fq(&query, false, true)?;
    assert_single_a(only(fq.root_maps(), &[None], Some(F)));

    // phraseHighlight = false, fieldMatch = false
    let fq = new_fq(&query, false, false)?;
    assert_single_a(only(fq.root_maps(), &[Some(F)], None));

    // boost != 1
    let query = tq_b(2.0, "a");
    let fq = new_fq(&query, true, true)?;
    let qpm = get(fq.root_maps(), Some(F)).expect("the root map");
    boost(qpm.sub_map().get("a").expect("a"), 2.0);
    Ok(())
}

/// `"a b"` with phrase highlighting: `a` (not terminal) -> `b` (terminal, 1).
fn assert_phrase_ab(qpm: &QueryPhraseMap) {
    size(qpm, 1);
    let qpm2 = sub(qpm, "a");
    terminal(qpm2, false);
    size(qpm2, 1);
    let qpm3 = sub(qpm2, "b");
    terminal(qpm3, true);
    boost(qpm3, 1.0);
}

/// `"a b"` without phrase highlighting: `a` (terminal) -> `b`, and `b`.
fn assert_terms_ab(qpm: &QueryPhraseMap) {
    size(qpm, 2);
    let qpm2 = sub(qpm, "a");
    terminal(qpm2, true);
    boost(qpm2, 1.0);
    size(qpm2, 1);
    let qpm3 = sub(qpm2, "b");
    terminal(qpm3, true);
    boost(qpm3, 1.0);

    let qpm2 = sub(qpm, "b");
    terminal(qpm2, true);
    boost(qpm2, 1.0);
}

fn test_query_phrase_map_1_phrase(_f: &mut Fixture) -> Result<()> {
    let query = pq_f(&["a", "b"]);

    // phraseHighlight = true, fieldMatch = true
    let fq = new_fq(&query, true, true)?;
    assert_phrase_ab(only(fq.root_maps(), &[None], Some(F)));

    // phraseHighlight = true, fieldMatch = false
    let fq = new_fq(&query, true, false)?;
    assert_phrase_ab(only(fq.root_maps(), &[Some(F)], None));

    // phraseHighlight = false, fieldMatch = true
    let fq = new_fq(&query, false, true)?;
    assert_terms_ab(only(fq.root_maps(), &[None], Some(F)));

    // phraseHighlight = false, fieldMatch = false
    let fq = new_fq(&query, false, false)?;
    assert_terms_ab(only(fq.root_maps(), &[Some(F)], None));

    // boost != 1
    let query = pq_f_b(2.0, &["a", "b"]);
    // phraseHighlight = false, fieldMatch = false
    let fq = new_fq(&query, false, false)?;
    let qpm = get(fq.root_maps(), None).expect("the root map");
    let qpm2 = qpm.sub_map().get("a").expect("a");
    boost(qpm2, 2.0);
    let qpm3 = qpm2.sub_map().get("b").expect("b");
    boost(qpm3, 2.0);
    let qpm2 = qpm.sub_map().get("b").expect("b");
    boost(qpm2, 2.0);
    Ok(())
}

fn test_query_phrase_map_1_phrase_another(_f: &mut Fixture) -> Result<()> {
    let query = pq_f(&["search", "engines"]);

    // phraseHighlight = true, fieldMatch = true
    let fq = new_fq(&query, true, true)?;
    let qpm = only(fq.root_maps(), &[None], Some(F));
    size(qpm, 1);
    let qpm2 = sub(qpm, "search");
    terminal(qpm2, false);
    size(qpm2, 1);
    let qpm3 = sub(qpm2, "engines");
    terminal(qpm3, true);
    boost(qpm3, 1.0);
    Ok(())
}

/// `"a b"` and `"c d"^2` as two phrases under `qpm`.
fn assert_ab_and_cd2(qpm: &QueryPhraseMap) {
    // "a b"
    let qpm2 = sub(qpm, "a");
    terminal(qpm2, false);
    size(qpm2, 1);
    let qpm3 = sub(qpm2, "b");
    terminal(qpm3, true);
    boost(qpm3, 1.0);

    // "c d"^2
    let qpm2 = sub(qpm, "c");
    terminal(qpm2, false);
    size(qpm2, 1);
    let qpm3 = sub(qpm2, "d");
    terminal(qpm3, true);
    boost(qpm3, 2.0);
}

fn test_query_phrase_map_2_phrases(_f: &mut Fixture) -> Result<()> {
    let mut query = BooleanQueryBuilder::new();
    query.add(pq_f(&["a", "b"]), Occur::SHOULD)?;
    query.add(pq_f_b(2.0, &["c", "d"]), Occur::SHOULD)?;

    // phraseHighlight = true, fieldMatch = true
    let fq = new_fq(&(Arc::new(query.build()) as Arc<dyn Query>), true, true)?;
    let qpm = only(fq.root_maps(), &[None], Some(F));
    size(qpm, 2);
    assert_ab_and_cd2(qpm);
    Ok(())
}

fn test_query_phrase_map_2_phrases_fields(_f: &mut Fixture) -> Result<()> {
    let mut query = BooleanQueryBuilder::new();
    query.add(pq(F1, &["a", "b"]), Occur::SHOULD)?;
    query.add(pq_b(2.0, F2, &["c", "d"]), Occur::SHOULD)?;
    let query: Arc<dyn Query> = Arc::new(query.build());

    // phraseHighlight = true, fieldMatch = true
    let fq = new_fq(&query, true, true)?;
    let map = fq.root_maps();
    Assert::assert_equals(2, map.len());
    Assert::assert_true(get(map, None).is_none());

    // "a b"
    let qpm = get(map, Some(F1));
    Assert::assert_true(qpm.is_some());
    let qpm = qpm.unwrap();
    size(qpm, 1);
    let qpm2 = sub(qpm, "a");
    terminal(qpm2, false);
    size(qpm2, 1);
    let qpm3 = sub(qpm2, "b");
    terminal(qpm3, true);
    boost(qpm3, 1.0);

    // "c d"^2
    let qpm = get(map, Some(F2));
    Assert::assert_true(qpm.is_some());
    let qpm = qpm.unwrap();
    size(qpm, 1);
    let qpm2 = sub(qpm, "c");
    terminal(qpm2, false);
    size(qpm2, 1);
    let qpm3 = sub(qpm2, "d");
    terminal(qpm3, true);
    boost(qpm3, 2.0);

    // phraseHighlight = true, fieldMatch = false
    let fq = new_fq(&query, true, false)?;
    let qpm = only(fq.root_maps(), &[Some(F1), Some(F2)], None);
    size(qpm, 2);
    assert_ab_and_cd2(qpm);
    Ok(())
}

/*
 * <t>...terminal
 *
 * a-b-c-<t>
 *     +-d-<t>
 * b-c-d-<t>
 * +-d-<t>
 */
fn test_query_phrase_map_overlap_phrases(_f: &mut Fixture) -> Result<()> {
    let mut query = BooleanQueryBuilder::new();
    query.add(pq_f(&["a", "b", "c"]), Occur::SHOULD)?;
    query.add(pq_f_b(2.0, &["b", "c", "d"]), Occur::SHOULD)?;
    query.add(pq_f_b(3.0, &["b", "d"]), Occur::SHOULD)?;

    // phraseHighlight = true, fieldMatch = true
    let fq = new_fq(&(Arc::new(query.build()) as Arc<dyn Query>), true, true)?;
    let qpm = only(fq.root_maps(), &[None], Some(F));
    size(qpm, 2);

    // "a b c"
    let qpm2 = sub(qpm, "a");
    terminal(qpm2, false);
    size(qpm2, 1);
    let qpm3 = sub(qpm2, "b");
    terminal(qpm3, false);
    size(qpm3, 1);
    let qpm4 = sub(qpm3, "c");
    terminal(qpm4, true);
    boost(qpm4, 1.0);
    let qpm5 = sub(qpm4, "d");
    terminal(qpm5, true);
    boost(qpm5, 1.0);

    // "b c d"^2, "b d"^3
    let qpm2 = sub(qpm, "b");
    terminal(qpm2, false);
    size(qpm2, 2);
    let qpm3 = sub(qpm2, "c");
    terminal(qpm3, false);
    size(qpm3, 1);
    let qpm4 = sub(qpm3, "d");
    terminal(qpm4, true);
    boost(qpm4, 2.0);
    let qpm3 = sub(qpm2, "d");
    terminal(qpm3, true);
    boost(qpm3, 3.0);
    Ok(())
}

/*
 * <t>...terminal
 *
 * a-b-<t>
 *   +-c-<t>
 */
fn test_query_phrase_map_overlap_phrases_2(_f: &mut Fixture) -> Result<()> {
    let mut query = BooleanQueryBuilder::new();
    query.add(pq_f(&["a", "b"]), Occur::SHOULD)?;
    query.add(pq_f_b(2.0, &["a", "b", "c"]), Occur::SHOULD)?;

    // phraseHighlight = true, fieldMatch = true
    let fq = new_fq(&(Arc::new(query.build()) as Arc<dyn Query>), true, true)?;
    let qpm = only(fq.root_maps(), &[None], Some(F));
    size(qpm, 1);

    // "a b"
    let qpm2 = sub(qpm, "a");
    terminal(qpm2, false);
    size(qpm2, 1);
    let qpm3 = sub(qpm2, "b");
    terminal(qpm3, true);
    boost(qpm3, 1.0);

    // "a b c"^2
    size(qpm3, 1);
    let qpm4 = sub(qpm3, "c");
    terminal(qpm4, true);
    boost(qpm4, 2.0);
    Ok(())
}

/*
 * <t>...terminal
 *
 * a-a-a-<t>
 *     +-a-<t>
 *       +-a-<t>
 *         +-a-<t>
 */
fn test_query_phrase_map_overlap_phrases_3(_f: &mut Fixture) -> Result<()> {
    let mut query = BooleanQueryBuilder::new();
    query.add(pq_f(&["a", "a", "a", "a"]), Occur::SHOULD)?;
    query.add(pq_f_b(2.0, &["a", "a", "a"]), Occur::SHOULD)?;

    // phraseHighlight = true, fieldMatch = true
    let fq = new_fq(&(Arc::new(query.build()) as Arc<dyn Query>), true, true)?;
    let qpm = only(fq.root_maps(), &[None], Some(F));
    size(qpm, 1);

    // "a a a"
    let qpm2 = sub(qpm, "a");
    terminal(qpm2, false);
    size(qpm2, 1);
    let qpm3 = sub(qpm2, "a");
    terminal(qpm3, false);
    size(qpm3, 1);
    let qpm4 = sub(qpm3, "a");
    terminal(qpm4, true);

    // "a a a a"
    size(qpm4, 1);
    let qpm5 = sub(qpm4, "a");
    terminal(qpm5, true);

    // "a a a a a"
    size(qpm5, 1);
    let qpm6 = sub(qpm5, "a");
    terminal(qpm6, true);

    // "a a a a a a"
    size(qpm6, 1);
    let qpm7 = sub(qpm6, "a");
    terminal(qpm7, true);
    Ok(())
}

fn test_query_phrase_map_overlap_2gram(f: &mut Fixture) -> Result<()> {
    let analyzer_b = Arc::clone(f.analyzer_b.as_ref().expect("setUp ran"));
    let mut query = BooleanQueryBuilder::new();
    query.add(
        Arc::new(to_phrase_query(&analyze("abc", F, &*analyzer_b)?, F)),
        Occur::MUST,
    )?;
    query.add(
        Arc::new(to_phrase_query(&analyze("bcd", F, &*analyzer_b)?, F)),
        Occur::MUST,
    )?;
    let query: Arc<dyn Query> = Arc::new(query.build());

    // phraseHighlight = true, fieldMatch = true
    let fq = new_fq(&query, true, true)?;
    let qpm = only(fq.root_maps(), &[None], Some(F));
    size(qpm, 2);

    // "ab bc"
    let qpm2 = sub(qpm, "ab");
    terminal(qpm2, false);
    size(qpm2, 1);
    let qpm3 = sub(qpm2, "bc");
    terminal(qpm3, true);
    boost(qpm3, 1.0);

    // "ab bc cd"
    size(qpm3, 1);
    let qpm4 = sub(qpm3, "cd");
    terminal(qpm4, true);
    boost(qpm4, 1.0);

    // "bc cd"
    let qpm2 = sub(qpm, "bc");
    terminal(qpm2, false);
    size(qpm2, 1);
    let qpm3 = sub(qpm2, "cd");
    terminal(qpm3, true);
    boost(qpm3, 1.0);

    // phraseHighlight = false, fieldMatch = true
    let fq = new_fq(&query, false, true)?;
    let qpm = only(fq.root_maps(), &[None], Some(F));
    size(qpm, 3);

    // "ab bc"
    let qpm2 = sub(qpm, "ab");
    terminal(qpm2, true);
    boost(qpm2, 1.0);
    size(qpm2, 1);
    let qpm3 = sub(qpm2, "bc");
    terminal(qpm3, true);
    boost(qpm3, 1.0);

    // "ab bc cd"
    size(qpm3, 1);
    let qpm4 = sub(qpm3, "cd");
    terminal(qpm4, true);
    boost(qpm4, 1.0);

    // "bc cd"
    let qpm2 = sub(qpm, "bc");
    terminal(qpm2, true);
    boost(qpm2, 1.0);
    size(qpm2, 1);
    let qpm3 = sub(qpm2, "cd");
    terminal(qpm3, true);
    boost(qpm3, 1.0);

    // "cd"
    let qpm2 = sub(qpm, "cd");
    terminal(qpm2, true);
    boost(qpm2, 1.0);
    size(qpm2, 0);
    Ok(())
}

fn ti(text: &str, start: i32, end: i32, position: i32) -> TermInfo {
    TermInfo::new(text, start, end, position, 1.0)
}

fn test_search_phrase(_f: &mut Fixture) -> Result<()> {
    let query = pq_f(&["a", "b", "c"]);

    // phraseHighlight = true, fieldMatch = true
    let fq = new_fq(&query, true, true)?;

    // "a"
    let mut phrase_candidate = vec![ti("a", 0, 1, 0)];
    Assert::assert_true(fq.search_phrase(F, &phrase_candidate).is_none());
    // "a b"
    phrase_candidate.push(ti("b", 2, 3, 1));
    Assert::assert_true(fq.search_phrase(F, &phrase_candidate).is_none());
    // "a b c"
    phrase_candidate.push(ti("c", 4, 5, 2));
    Assert::assert_true(fq.search_phrase(F, &phrase_candidate).is_some());
    Assert::assert_true(fq.search_phrase("x", &phrase_candidate).is_none());

    // phraseHighlight = true, fieldMatch = false
    let fq = new_fq(&query, true, false)?;

    // "a b c"
    Assert::assert_true(fq.search_phrase(F, &phrase_candidate).is_some());
    Assert::assert_true(fq.search_phrase("x", &phrase_candidate).is_some());

    // phraseHighlight = false, fieldMatch = true
    let fq = new_fq(&query, false, true)?;

    // "a"
    phrase_candidate.clear();
    phrase_candidate.push(ti("a", 0, 1, 0));
    Assert::assert_true(fq.search_phrase(F, &phrase_candidate).is_some());
    // "a b"
    phrase_candidate.push(ti("b", 2, 3, 1));
    Assert::assert_true(fq.search_phrase(F, &phrase_candidate).is_none());
    // "a b c"
    phrase_candidate.push(ti("c", 4, 5, 2));
    Assert::assert_true(fq.search_phrase(F, &phrase_candidate).is_some());
    Assert::assert_true(fq.search_phrase("x", &phrase_candidate).is_none());
    Ok(())
}

fn test_search_phrase_slop(_f: &mut Fixture) -> Result<()> {
    // "a b c"~0
    let query = pq_f(&["a", "b", "c"]);

    // phraseHighlight = true, fieldMatch = true
    let fq = new_fq(&query, true, true)?;

    // "a b c" w/ position-gap = 2
    let mut phrase_candidate = vec![ti("a", 0, 1, 0), ti("b", 2, 3, 2), ti("c", 4, 5, 4)];
    Assert::assert_true(fq.search_phrase(F, &phrase_candidate).is_none());

    // "a b c"~1
    let query = pq_f_bs(1.0, 1, &["a", "b", "c"]);

    // phraseHighlight = true, fieldMatch = true
    let fq = new_fq(&query, true, true)?;

    // "a b c" w/ position-gap = 2
    Assert::assert_true(fq.search_phrase(F, &phrase_candidate).is_some());

    // "a b c" w/ position-gap = 3
    phrase_candidate.clear();
    phrase_candidate.extend([ti("a", 0, 1, 0), ti("b", 2, 3, 3), ti("c", 4, 5, 6)]);
    Assert::assert_true(fq.search_phrase(F, &phrase_candidate).is_none());
    Ok(())
}

fn test_highlight_query(f: &mut Fixture) -> Result<()> {
    make_index_str_mv(f)?;
    defg_multi_term_query_test(f, Arc::new(WildcardQuery::new(Term::from_text(F, "d*g"))?))
}

fn test_prefix_query(f: &mut Fixture) -> Result<()> {
    make_index_str_mv(f)?;
    defg_multi_term_query_test(f, Arc::new(PrefixQuery::new(Term::from_text(F, "de"))?))
}

fn test_regexp_query(f: &mut Fixture) -> Result<()> {
    make_index_str_mv(f)?;
    let term = Term::from_text(F, "d[a-z].g");
    defg_multi_term_query_test(f, Arc::new(RegexpQuery::new(term)?))
}

fn test_range_query(f: &mut Fixture) -> Result<()> {
    make_index_str_mv(f)?;
    defg_multi_term_query_test(
        f,
        Arc::new(TermRangeQuery::new(
            F,
            Some(BytesRef::from_char_sequence("d")),
            Some(BytesRef::from_char_sequence("e")),
            true,
            true,
        )?),
    )
}

fn defg_multi_term_query_test(f: &Fixture, query: Arc<dyn Query>) -> Result<()> {
    let fq = FieldQuery::new(&query, f.reader.as_ref(), true, true)?;
    let qpm = fq.get_field_term_map(F, "defg");
    Assert::assert_true(qpm.is_some());
    Assert::assert_true(fq.get_field_term_map(F, "dog").is_none());
    let phrase_candidate = vec![ti("defg", 0, 12, 0)];
    Assert::assert_true(fq.search_phrase(F, &phrase_candidate).is_some());
    Ok(())
}

/// The anonymous `Query` of `testStopRewrite`: its `equals` and `hashCode`
/// throw `AssertionError`.
#[derive(Debug)]
struct DummyQuery;

impl Query for DummyQuery {
    fn to_query_string(&self, _field: &str) -> String {
        "DummyQuery".to_string()
    }

    fn visit(&self, _visitor: &mut dyn QueryVisitor) {}

    fn query_eq(&self, _other: &dyn Query) -> bool {
        Throwable::new("java.lang.AssertionError", "").throw()
    }

    fn query_hash(&self) -> i32 {
        Throwable::new("java.lang.AssertionError", "").throw()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.search.vectorhighlight.TestFieldQuery$1"
    }
}

fn test_stop_rewrite(f: &mut Fixture) -> Result<()> {
    let q: Arc<dyn Query> = Arc::new(DummyQuery);
    make1d1f_index(f, "a")?;
    Assert::assert_true(f.reader.is_some());
    FieldQuery::new(&q, f.reader.as_ref(), true, true)?;
    Ok(())
}

fn test_flatten_constant_score_query(f: &mut Fixture) -> Result<()> {
    let boost = init_boost();
    let query: Arc<dyn Query> = Arc::new(ConstantScoreQuery::new(pq_f(&["A"])));
    let query = boost_query(query, boost)?;
    let fq = new_fq(&query, true, true)?;
    let flat_queries = flatten(f, &fq, &query)?;
    assert_collection_queries(&flat_queries, &[tq_b(boost, "A")]);
    Ok(())
}
