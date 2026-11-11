//! Port of `lucene/core/src/test/org/apache/lucene/search/TestFuzzyQuery.java`.
//!
//! Tests `FuzzyQuery`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::analysis::Analyzer;
use rucene::document::{Document, Store, StringField};
use rucene::error::Result;
use rucene::index::multi_reader::MultiReader;
use rucene::index::IndexReader;
use rucene::search::fuzzy_query::{
    FuzzyQuery, DEFAULT_MAX_EDITS, DEFAULT_MAX_EXPANSIONS, DEFAULT_PREFIX_LENGTH,
    DEFAULT_TRANSPOSITIONS,
};
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::multi_term_query::TopTermsBoostOnlyBooleanQueryRewrite;
use rucene::search::query_visitor::QueryVisitor;
use rucene::search::similarities::ClassicSimilarity;
use rucene::search::{BooleanQueryBuilder, Occur, Query, ScoreDoc};
use rucene::util::automaton::levenshtein_automata::MAXIMUM_SUPPORTED_DISTANCE;
use rucene::util::automaton::run_automaton::ByteRunnable;
use rucene::util::automaton::ByteRunAutomaton;
use rucene_test_framework::analysis::mock_tokenizer::KEYWORD;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_search::support::term;
use crate::core_support::{
    assert_equals, assert_equals_msg, class, iae_message, random, test, JavaHashSet,
};

/// `TestFuzzyQuery`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.TestFuzzyQuery",
        vec![
            test("testBasicPrefix", test_basic_prefix),
            test("testFuzziness", test_fuzziness),
            test(
                "testPrefixLengthEqualStringLength",
                test_prefix_length_equal_string_length,
            ),
            test("test2", test2),
            test(
                "testSingleQueryExactMatchScoresHighest",
                test_single_query_exact_match_scores_highest,
            ),
            test(
                "testMultipleQueriesIdfWorks",
                test_multiple_queries_idf_works,
            ),
            test("testTieBreaker", test_tie_breaker),
            test("testBoostOnlyRewrite", test_boost_only_rewrite),
            test("testGiga", test_giga),
            test(
                "testDistanceAsEditsSearching",
                test_distance_as_edits_searching,
            ),
            test("testValidation", test_validation),
            test("testRandom", test_random),
            test("testVisitor", test_visitor),
        ],
    )
}

/// `new FuzzyQuery(new Term("field", text), maxEdits, prefixLength)`.
fn fq(text: &str, max_edits: i32, prefix_length: i32) -> Result<Arc<dyn Query>> {
    Ok(Arc::new(FuzzyQuery::with_prefix_length(
        term("field", text),
        max_edits,
        prefix_length,
    )?))
}

/// `searcher.search(query, n).scoreDocs`.
fn hits(searcher: &IndexSearcher, query: &Arc<dyn Query>, n: i32) -> Result<Vec<ScoreDoc>> {
    Ok(searcher.search(Arc::clone(query), n)?.score_docs)
}

/// `storedFields.document(doc).get("field")`.
fn field_of(searcher: &IndexSearcher, doc: i32) -> Result<Option<String>> {
    Ok(searcher.stored_fields()?.document(doc)?.get("field"))
}

/// `new RandomIndexWriter(random(), directory)`.
fn riw(
    directory: &rucene_test_framework::util::lucene_test_case::TestDirectory,
) -> Result<RandomIndexWriter> {
    RandomIndexWriter::new_with_mock_analyzer(&*random(), directory.as_directory())
}

/// Port of `TestFuzzyQuery.testBasicPrefix`.
fn test_basic_prefix() -> Result<()> {
    let directory = LuceneTestCase::new_directory();
    let writer = riw(&directory)?;
    add_doc("abc", &writer)?;
    let reader = writer.get_reader()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    writer.close()?;

    let query = fq("abc", DEFAULT_MAX_EDITS, 1)?;
    let hits = hits(&searcher, &query, 1000)?;
    assert_equals(&1, &hits.len());
    drop(searcher);
    reader.close()?;
    directory.close()?;
    Ok(())
}

/// Port of `TestFuzzyQuery.testFuzziness`.
fn test_fuzziness() -> Result<()> {
    let directory = LuceneTestCase::new_directory();
    let analyzer: Arc<dyn Analyzer> = Arc::new(MockAnalyzer::new(&*random()));
    let config = LuceneTestCase::new_index_writer_config_analyzer(Some(analyzer));
    config.set_merge_policy(Some(LuceneTestCase::new_merge_policy_include_mock(
        &*random(),
        false,
    )))?;
    let writer = RandomIndexWriter::new_with_config(&*random(), directory.as_directory(), config)?;
    add_doc("aaaaa", &writer)?;
    add_doc("aaaab", &writer)?;
    add_doc("aaabb", &writer)?;
    add_doc("aabbb", &writer)?;
    add_doc("abbbb", &writer)?;
    add_doc("bbbbb", &writer)?;
    add_doc("ddddd", &writer)?;

    let reader = writer.get_reader()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    writer.close()?;

    let count = |text: &str, prefix: i32| -> Result<Vec<ScoreDoc>> {
        hits(&searcher, &fq(text, DEFAULT_MAX_EDITS, prefix)?, 1000)
    };
    let f = |doc: i32| -> Result<Option<String>> { field_of(&searcher, doc) };
    let s = |v: &str| Some(v.to_string());

    let h = count("aaaaa", 0)?;
    assert_equals(&3, &h.len());

    // same with prefix
    assert_equals(&3, &count("aaaaa", 1)?.len());
    assert_equals(&3, &count("aaaaa", 2)?.len());
    assert_equals(&3, &count("aaaaa", 3)?.len());
    assert_equals(&2, &count("aaaaa", 4)?.len());
    assert_equals(&1, &count("aaaaa", 5)?.len());
    assert_equals(&1, &count("aaaaa", 6)?.len());

    // test scoring
    let h = count("bbbbb", 0)?;
    assert_equals_msg(Some("3 documents should match"), &3, &h.len());
    let order = ["bbbbb", "abbbb", "aabbb"];
    let stored_fields = searcher.stored_fields()?;
    for i in 0..h.len() {
        let term = stored_fields.document(h[i].doc)?.get("field");
        // System.out.println(hits[i].score);
        assert_equals(&s(order[i]), &term);
    }

    // test pq size by supplying maxExpansions=2
    // This query would normally return 3 documents, because 3 terms match (see above):
    let query: Arc<dyn Query> = Arc::new(FuzzyQuery::with_expansions(
        term("field", "bbbbb"),
        DEFAULT_MAX_EDITS,
        0,
        2,
        false,
    )?);
    let h = hits(&searcher, &query, 1000)?;
    assert_equals_msg(Some("only 2 documents should match"), &2, &h.len());
    let order = ["bbbbb", "abbbb"];
    for i in 0..h.len() {
        let term = stored_fields.document(h[i].doc)?.get("field");
        // System.out.println(hits[i].score);
        assert_equals(&s(order[i]), &term);
    }

    // not similar enough:
    assert_equals(&0, &count("xxxxx", 0)?.len());
    assert_equals(&0, &count("aaccc", 0)?.len()); // edit distance to "aaaaa" = 3

    // query identical to a word in the index:
    let h = count("aaaaa", 0)?;
    assert_equals(&3, &h.len());
    assert_equals(&f(h[0].doc)?, &s("aaaaa"));
    // default allows for up to two edits:
    assert_equals(&f(h[1].doc)?, &s("aaaab"));
    assert_equals(&f(h[2].doc)?, &s("aaabb"));

    // query similar to a word in the index:
    for prefix in 0..4 {
        // prefix 0, then now with prefix 1, 2 and 3
        let h = count("aaaac", prefix)?;
        assert_equals(&3, &h.len());
        assert_equals(&f(h[0].doc)?, &s("aaaaa"));
        assert_equals(&f(h[1].doc)?, &s("aaaab"));
        assert_equals(&f(h[2].doc)?, &s("aaabb"));
    }
    let h = count("aaaac", 4)?;
    assert_equals(&2, &h.len());
    assert_equals(&f(h[0].doc)?, &s("aaaaa"));
    assert_equals(&f(h[1].doc)?, &s("aaaab"));
    assert_equals(&0, &count("aaaac", 5)?.len());

    for prefix in 0..5 {
        // prefix 0, then now with prefix 1 to 4
        let h = count("ddddX", prefix)?;
        assert_equals(&1, &h.len());
        assert_equals(&f(h[0].doc)?, &s("ddddd"));
    }
    assert_equals(&0, &count("ddddX", 5)?.len());

    // different field = no match:
    let query: Arc<dyn Query> = Arc::new(FuzzyQuery::with_prefix_length(
        term("anotherfield", "ddddX"),
        DEFAULT_MAX_EDITS,
        0,
    )?);
    assert_equals(&0, &hits(&searcher, &query, 1000)?.len());

    drop(stored_fields);
    drop(searcher);
    reader.close()?;
    directory.close()?;
    Ok(())
}

/// Port of `TestFuzzyQuery.testPrefixLengthEqualStringLength`.
fn test_prefix_length_equal_string_length() -> Result<()> {
    let directory = LuceneTestCase::new_directory();
    let writer = riw(&directory)?;
    add_doc("b*a", &writer)?;
    add_doc("b*ab", &writer)?;
    add_doc("b*abc", &writer)?;
    add_doc("b*abcd", &writer)?;
    let multibyte = "아프리카코끼리속";
    add_doc(multibyte, &writer)?;
    let reader = writer.get_reader()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    writer.close()?;

    let mut max_edits = 0;
    let mut prefix_length = 3;
    let h = hits(&searcher, &fq("b*a", max_edits, prefix_length)?, 1000)?;
    assert_equals(&1, &h.len());

    max_edits = 1;
    let h = hits(&searcher, &fq("b*a", max_edits, prefix_length)?, 1000)?;
    assert_equals(&2, &h.len());

    max_edits = 2;
    let h = hits(&searcher, &fq("b*a", max_edits, prefix_length)?, 1000)?;
    assert_equals(&3, &h.len());

    max_edits = 1;
    // `multibyte.length() - 1` (UTF-16 units; these are all BMP characters).
    prefix_length = multibyte.encode_utf16().count() as i32 - 1;
    let prefix: String = multibyte.chars().take(prefix_length as usize).collect();
    let h = hits(&searcher, &fq(&prefix, max_edits, prefix_length)?, 1000)?;
    assert_equals(&1, &h.len());

    drop(searcher);
    reader.close()?;
    directory.close()?;
    Ok(())
}

/// Port of `TestFuzzyQuery.test2`.
fn test2() -> Result<()> {
    let directory = LuceneTestCase::new_directory();
    let writer = RandomIndexWriter::new(
        &*random(),
        directory.as_directory(),
        Arc::new(MockAnalyzer::with_automaton(
            &*random(),
            Arc::clone(&KEYWORD),
            false,
        )),
    )?;
    for text in [
        "LANGE",
        "LUETH",
        "PIRSING",
        "RIEGEL",
        "TRZECZIAK",
        "WALKER",
        "WBR",
        "WE",
        "WEB",
        "WEBE",
        "WEBER",
        "WEBERE",
        "WEBREE",
        "WEBEREI",
        "WBRE",
        "WITTKOPF",
        "WOJNAROWSKI",
        "WRICKE",
    ] {
        add_doc(text, &writer)?;
    }

    let reader = writer.get_reader()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    writer.close()?;

    let query = fq("WEBER", 2, 1)?;
    // query.setRewriteMethod(FuzzyQuery.SCORING_BOOLEAN_QUERY_REWRITE);
    let h = hits(&searcher, &query, 1000)?;
    assert_equals(&8, &h.len());

    drop(searcher);
    reader.close()?;
    directory.close()?;
    Ok(())
}

/// Port of `TestFuzzyQuery.testSingleQueryExactMatchScoresHighest`.
fn test_single_query_exact_match_scores_highest() -> Result<()> {
    // See issue LUCENE-329 - IDF shouldn't wreck similarity ranking
    let directory = LuceneTestCase::new_directory();
    let writer = riw(&directory)?;
    for text in [
        "smith", "smith", "smith", "smith", "smith", "smith", "smythe", "smdssasd",
    ] {
        add_doc(text, &writer)?;
    }

    let reader = writer.get_reader()?;
    let mut searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    searcher.set_similarity(Arc::new(ClassicSimilarity::new())); // avoid randomisation of similarity algo by test framework
    writer.close()?;
    let search_terms = ["smith", "smythe", "smdssasd"];
    let stored_fields = (Arc::clone(&reader) as Arc<dyn IndexReader>).stored_fields()?;
    for search_term in search_terms {
        let h = hits(&searcher, &fq(search_term, 2, 1)?, 1000)?;
        let best_doc = stored_fields.document(h[0].doc)?;
        Assert::assert_true(!h.is_empty());
        let top_match = best_doc.get("field");
        assert_equals(&Some(search_term.to_string()), &top_match);
        if h.len() > 1 {
            let worst_doc = stored_fields.document(h[h.len() - 1].doc)?;
            let worst_match = worst_doc.get("field");
            // `assertNotSame(searchTerm, worstMatch)`: the stored value is a
            // string of its own.
            Assert::assert_true(
                worst_match
                    .as_deref()
                    .is_none_or(|w| !std::ptr::eq(w.as_ptr(), search_term.as_ptr())),
            );
        }
    }
    drop(stored_fields);
    drop(searcher);
    reader.close()?;
    directory.close()?;
    Ok(())
}

/// Port of `TestFuzzyQuery.testMultipleQueriesIdfWorks`.
fn test_multiple_queries_idf_works() -> Result<()> {
    // With issue LUCENE-329 - it could be argued a
    // MultiTermQuery.TopTermsBoostOnlyBooleanQueryRewrite
    // is the solution as it disables IDF.
    // However - IDF is still useful as in this case where there are multiple FuzzyQueries.
    let directory = LuceneTestCase::new_directory();
    let writer = riw(&directory)?;

    for text in [
        "michael smith",
        "michael lucero",
        "doug cutting",
        "doug cuttin",
        "michael wardle",
        "micheal vegas",
        "michael lydon",
    ] {
        add_doc(text, &writer)?;
    }

    let reader = writer.get_reader()?;
    let mut searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    searcher.set_similarity(Arc::new(ClassicSimilarity::new())); // avoid randomisation of similarity algo by test framework

    writer.close()?;

    let mut query = BooleanQueryBuilder::new();
    let common_search_term = "michael";
    let common_query = fq(common_search_term, 2, 1)?;
    query.add(common_query, Occur::SHOULD)?;

    let rare_search_term = "cutting";
    let rare_query = fq(rare_search_term, 2, 1)?;
    query.add(rare_query, Occur::SHOULD)?;
    let h = searcher.search(Arc::new(query.build()), 1000)?.score_docs;

    // Matches on the rare surname should be worth more than matches on the common forename
    assert_equals(&7, &h.len());
    let top_match = field_of(&searcher, h[0].doc)?.expect("NullPointerException");
    Assert::assert_true(top_match.contains(rare_search_term));

    let runner_up_match = field_of(&searcher, h[1].doc)?.expect("NullPointerException");
    Assert::assert_true(runner_up_match.contains("cuttin"));

    let worst_match = field_of(&searcher, h[h.len() - 1].doc)?.expect("NullPointerException");
    Assert::assert_true(worst_match.contains("micheal")); // misspelling of common name

    drop(searcher);
    reader.close()?;
    directory.close()?;
    Ok(())
}

/// MultiTermQuery provides (via attribute) information about which values
/// must be competitive to enter the priority queue.
///
/// FuzzyQuery optimizes itself around this information, if the attribute is
/// not implemented correctly, there will be problems!
///
/// Port of `TestFuzzyQuery.testTieBreaker`.
fn test_tie_breaker() -> Result<()> {
    let directory = LuceneTestCase::new_directory();
    let writer = riw(&directory)?;
    add_doc("a123456", &writer)?;
    add_doc("c123456", &writer)?;
    add_doc("d123456", &writer)?;
    add_doc("e123456", &writer)?;

    let directory2 = LuceneTestCase::new_directory();
    let writer2 = riw(&directory2)?;
    add_doc("a123456", &writer2)?;
    add_doc("b123456", &writer2)?;
    add_doc("b123456", &writer2)?;
    add_doc("b123456", &writer2)?;
    add_doc("c123456", &writer2)?;
    add_doc("f123456", &writer2)?;

    let ir1 = writer.get_reader()?;
    let ir2 = writer2.get_reader()?;

    // `new MultiReader(ir1, ir2)`: closeSubReaders is true.
    let mr = Arc::new(MultiReader::new(
        vec![
            Arc::clone(&ir1) as Arc<dyn IndexReader>,
            Arc::clone(&ir2) as Arc<dyn IndexReader>,
        ],
        true,
    )?);
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&mr));
    let fq: Arc<dyn Query> = Arc::new(FuzzyQuery::with_expansions(
        term("field", "z123456"),
        1,
        0,
        2,
        false,
    )?);
    let docs = searcher.search(fq, 2)?;
    assert_equals(&5, &docs.total_hits.value()); // 5 docs, from the a and b's
    drop(searcher);
    mr.close()?;
    ir1.close()?;
    ir2.close()?;
    writer.close()?;
    writer2.close()?;
    directory.close()?;
    directory2.close()?;
    Ok(())
}

/// Test the TopTermsBoostOnlyBooleanQueryRewrite rewrite method.
///
/// Port of `TestFuzzyQuery.testBoostOnlyRewrite`.
fn test_boost_only_rewrite() -> Result<()> {
    let directory = LuceneTestCase::new_directory();
    let writer = riw(&directory)?;
    add_doc("Lucene", &writer)?;
    add_doc("Lucene", &writer)?;
    add_doc("Lucenne", &writer)?;

    let reader = writer.get_reader()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    writer.close()?;

    let query: Arc<dyn Query> = Arc::new(FuzzyQuery::with_rewrite_method(
        term("field", "lucene"),
        DEFAULT_MAX_EDITS,
        DEFAULT_PREFIX_LENGTH,
        DEFAULT_MAX_EXPANSIONS,
        DEFAULT_TRANSPOSITIONS,
        Arc::new(TopTermsBoostOnlyBooleanQueryRewrite::new(50)),
    )?);
    let h = hits(&searcher, &query, 1000)?;
    assert_equals(&3, &h.len());
    // normally, 'Lucenne' would be the first result as IDF will skew the score.
    let stored_fields = (Arc::clone(&reader) as Arc<dyn IndexReader>).stored_fields()?;
    assert_equals(
        &Some("Lucene".to_string()),
        &stored_fields.document(h[0].doc)?.get("field"),
    );
    assert_equals(
        &Some("Lucene".to_string()),
        &stored_fields.document(h[1].doc)?.get("field"),
    );
    assert_equals(
        &Some("Lucenne".to_string()),
        &stored_fields.document(h[2].doc)?.get("field"),
    );
    drop(stored_fields);
    drop(searcher);
    reader.close()?;
    directory.close()?;
    Ok(())
}

/// Port of `TestFuzzyQuery.testGiga`.
fn test_giga() -> Result<()> {
    let index = LuceneTestCase::new_directory();
    let w = riw(&index)?;

    for text in [
        "Lucene in Action",
        "Lucene for Dummies",
        // addDoc("Giga", w);  — commented out in the Java test
        "Giga byte",
        "ManagingGigabytesManagingGigabyte",
        "ManagingGigabytesManagingGigabytes",
        "The Art of Computer Science",
        "J. K. Rowling",
        "JK Rowling",
        "Joanne K Roling",
        "Bruce Willis",
        "Willis bruce",
        "Brute willis",
        "B. willis",
    ] {
        add_doc(text, &w)?;
    }
    let r = w.get_reader()?;
    w.close()?;

    let q: Arc<dyn Query> = Arc::new(FuzzyQuery::with_max_edits(term("field", "giga"), 0)?);

    // 3. search
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&r));
    let h = hits(&searcher, &q, 10)?;
    assert_equals(&1, &h.len());
    assert_equals(
        &Some("Giga byte".to_string()),
        &field_of(&searcher, h[0].doc)?,
    );
    drop(searcher);
    r.close()?;
    w.close()?;
    index.close()?;
    Ok(())
}

/// Port of `TestFuzzyQuery.testDistanceAsEditsSearching`.
fn test_distance_as_edits_searching() -> Result<()> {
    let index = LuceneTestCase::new_directory();
    let w = riw(&index)?;
    add_doc("foobar", &w)?;
    add_doc("test", &w)?;
    add_doc("working", &w)?;
    let reader = w.get_reader()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    w.close()?;

    let q: Arc<dyn Query> = Arc::new(FuzzyQuery::with_max_edits(term("field", "fouba"), 2)?);
    let h = hits(&searcher, &q, 10)?;
    assert_equals(&1, &h.len());
    assert_equals(&Some("foobar".to_string()), &field_of(&searcher, h[0].doc)?);

    let q: Arc<dyn Query> = Arc::new(FuzzyQuery::with_max_edits(term("field", "foubara"), 2)?);
    let h = hits(&searcher, &q, 10)?;
    assert_equals(&1, &h.len());
    assert_equals(&Some("foobar".to_string()), &field_of(&searcher, h[0].doc)?);

    iae_message(|| FuzzyQuery::with_max_edits(term("field", "t"), 3));

    drop(searcher);
    reader.close()?;
    index.close()?;
    Ok(())
}

/// Port of `TestFuzzyQuery.testValidation`.
fn test_validation() -> Result<()> {
    let expected =
        iae_message(|| FuzzyQuery::with_expansions(term("field", "foo"), -1, 0, 1, false));
    Assert::assert_true(expected.contains("maxEdits"));

    let expected = iae_message(|| {
        FuzzyQuery::with_expansions(
            term("field", "foo"),
            MAXIMUM_SUPPORTED_DISTANCE + 1,
            0,
            1,
            false,
        )
    });
    Assert::assert_true(expected.contains("maxEdits must be between"));

    let expected =
        iae_message(|| FuzzyQuery::with_expansions(term("field", "foo"), 1, -1, 1, false));
    Assert::assert_true(expected.contains("prefixLength cannot be negative"));

    let expected =
        iae_message(|| FuzzyQuery::with_expansions(term("field", "foo"), 1, 0, -1, false));
    Assert::assert_true(expected.contains("maxExpansions must be positive"));

    let expected =
        iae_message(|| FuzzyQuery::with_expansions(term("field", "foo"), 1, 0, -1, false));
    Assert::assert_true(expected.contains("maxExpansions must be positive"));
    Ok(())
}

/// Port of `TestFuzzyQuery.testRandom` (`@SuppressWarnings({"unchecked",
/// "rawtypes"})`).
fn test_random() -> Result<()> {
    let digits = TestUtil::next_int(&*random(), 2, 3);
    // underestimated total number of unique terms that randomSimpleString
    // maybe generate, it assumes all terms have a length of 7
    let vocabulary_size = digits << 7;
    let num_terms = LuceneTestCase::at_least(100).min(vocabulary_size);
    let mut terms: JavaHashSet<String> = JavaHashSet::new();
    while (terms.size() as i32) < num_terms {
        terms.add(random_simple_string(digits));
    }

    let dir = LuceneTestCase::new_directory();
    let w = riw(&dir)?;
    for term in terms.iter() {
        let mut doc = Document::new();
        doc.add(Box::new(StringField::new(
            "field",
            term.clone(),
            Store::YES,
        )?));
        w.add_document(&doc)?;
    }
    let r = w.get_reader()?;
    w.close()?;
    println!("TEST: reader={r}");
    let s = LuceneTestCase::new_searcher(Arc::clone(&r));
    let iters = LuceneTestCase::at_least(200);
    for _ in 0..iters {
        let query_term = random_simple_string(digits);
        let query_term_len = query_term.chars().count() as i32;
        let prefix_length = random().next_int_bound(query_term_len);
        let query_prefix: String = query_term.chars().take(prefix_length as usize).collect();

        // we don't look at scores here:
        let mut expected: [Vec<TermAndScore>; 3] = [Vec::new(), Vec::new(), Vec::new()];
        for term in terms.iter() {
            if !term.starts_with(&query_prefix) {
                continue;
            }
            let mut ed = get_distance(term, &query_term);
            let score = 1.0f32 - ed as f32 / query_term_len.min(term.chars().count() as i32) as f32;
            while ed < 3 {
                expected[ed as usize].push(TermAndScore {
                    term: term.clone(),
                    score,
                });
                ed += 1;
            }
        }

        for ed in 0..3usize {
            expected[ed].sort_by(|a, b| a.compare_to(b).cmp(&0));
            let queue_size = TestUtil::next_int(&*random(), 1, terms.size() as i32);
            /*
            System.out.println("\nTEST: query=" + queryTerm + " ed=" + ed + " queueSize=" + queueSize + " vs expected match size=" + expected[ed].size() + " prefixLength=" + prefixLength);
            for(TermAndScore ent : expected[ed]) {
              System.out.println("  " + ent);
            }
            */
            let query: Arc<dyn Query> = Arc::new(FuzzyQuery::with_expansions(
                term("field", &query_term),
                ed as i32,
                prefix_length,
                queue_size,
                true,
            )?);
            let hits = s.search(query, terms.size() as i32)?;
            let mut actual: JavaHashSet<String> = JavaHashSet::new();
            let stored_fields = s.stored_fields()?;
            for hit in &hits.score_docs {
                let doc = stored_fields.document(hit.doc)?;
                actual.add(doc.get("field").expect("stored field"));
                // System.out.println("   actual: " + doc.get("field") + " score=" + hit.score);
            }
            let mut expected_top: JavaHashSet<String> = JavaHashSet::new();
            let limit = (queue_size as usize).min(expected[ed].len());
            for i in 0..limit {
                expected_top.add(expected[ed][i].term.clone());
            }

            let equal = actual.size() == expected_top.size()
                && actual.iter().all(|t| expected_top.contains(t));
            if !equal {
                let mut sb = String::new();
                sb.push_str(&format!(
                    "FAILED: query={query_term} ed={ed} queueSize={queue_size} vs expected match size={} prefixLength={prefix_length}\n",
                    expected[ed].len()
                ));

                let mut first = true;
                for term in actual.iter() {
                    if !expected_top.contains(term) {
                        if first {
                            sb.push_str("  these matched but shouldn't:\n");
                            first = false;
                        }
                        sb.push_str(&format!("    {term}\n"));
                    }
                }
                first = true;
                for term in expected_top.iter() {
                    if !actual.contains(term) {
                        if first {
                            sb.push_str("  these did not match but should:\n");
                            first = false;
                        }
                        sb.push_str(&format!("    {term}\n"));
                    }
                }
                Assert::fail(&sb);
            }
        }
    }

    drop(s);
    r.close()?;
    dir.close()?;
    Ok(())
}

/// Port of `TestFuzzyQuery.testVisitor`.
fn test_visitor() -> Result<()> {
    let q = FuzzyQuery::with_max_edits(term("field", "blob"), 2)?;
    let mut visitor = MatchingVisitor { visited: false };
    q.visit(&mut visitor);
    Assert::assert_true(visitor.visited);
    Ok(())
}

/// The anonymous `QueryVisitor` of `testVisitor`.
struct MatchingVisitor {
    visited: bool,
}

impl QueryVisitor for MatchingVisitor {
    fn consume_terms_matching(
        &mut self,
        _query: &dyn Query,
        _field: &str,
        automaton: &dyn Fn() -> ByteRunAutomaton,
    ) {
        self.visited = true;
        let a = automaton();
        assert_matches(&a, "blob");
        assert_matches(&a, "bolb");
        assert_matches(&a, "blobby");
        assert_no_matches(&a, "bolbby");
    }
}

fn add_doc(text: &str, writer: &RandomIndexWriter) -> Result<()> {
    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_text_field(
        "field",
        text,
        Store::YES,
    )));
    writer.add_document(&doc)?;
    Ok(())
}

fn random_simple_string(digits: i32) -> String {
    let term_length = TestUtil::next_int(&*random(), 1, 8);
    let mut chars = String::with_capacity(term_length as usize);
    for _ in 0..term_length {
        chars.push(char::from(b'a' + random().next_int_bound(digits) as u8));
    }
    chars
}

/// `private record TermAndScore(String term, float score) implements
/// Comparable<TermAndScore>`.
#[derive(Debug, Clone)]
struct TermAndScore {
    term: String,
    score: f32,
}

impl TermAndScore {
    fn compare_to(&self, other: &TermAndScore) -> i32 {
        // higher score sorts first, and if scores are tied, lower term sorts first
        if self.score > other.score {
            -1
        } else if self.score < other.score {
            1
        } else {
            // `String.compareTo`: UTF-16 order (these terms are ASCII).
            match self.term.encode_utf16().cmp(other.term.encode_utf16()) {
                std::cmp::Ordering::Less => -1,
                std::cmp::Ordering::Equal => 0,
                std::cmp::Ordering::Greater => 1,
            }
        }
    }
}

// Poached from LuceneLevenshteinDistance.java (from suggest module): it supports transpositions
// (treats them as ed=1, not ed=2)
fn get_distance(target: &str, other: &str) -> i32 {
    // NOTE: if we cared, we could 3*m space instead of m*n space, similar to
    // what LevenshteinDistance does, except cycling thru a ring of three
    // horizontal cost arrays... but this comparator is never actually used by
    // DirectSpellChecker, it's only used for merging results from multiple shards
    // in "distributed spellcheck", and it's inefficient in other ways too...

    // cheaper to do this up front once
    let target_points = to_ints_ref(target);
    let other_points = to_ints_ref(other);
    let n = target_points.len();
    let m = other_points.len();
    let mut d = vec![vec![0i32; m + 1]; n + 1]; // cost array

    if n == 0 || m == 0 {
        if n == m {
            return 0;
        } else {
            return n.max(m) as i32;
        }
    }

    for i in 0..=n {
        d[i][0] = i as i32;
    }

    for j in 0..=m {
        d[0][j] = j as i32;
    }

    for j in 1..=m {
        let t_j = other_points[j - 1]; // jth character of t

        for i in 1..=n {
            let cost = if target_points[i - 1] == t_j { 0 } else { 1 };
            // minimum of cell to the left+1, to the top+1, diagonally left and up +cost
            d[i][j] = (d[i - 1][j] + 1)
                .min(d[i][j - 1] + 1)
                .min(d[i - 1][j - 1] + cost);
            // transposition
            if i > 1
                && j > 1
                && target_points[i - 1] == other_points[j - 2]
                && target_points[i - 2] == other_points[j - 1]
            {
                d[i][j] = d[i][j].min(d[i - 2][j - 2] + cost);
            }
        }
    }

    d[n][m]
}

/// `toIntsRef(String)`: the code points of `s`.
fn to_ints_ref(s: &str) -> Vec<i32> {
    s.chars().map(|c| c as i32).collect()
}

fn assert_matches(automaton: &ByteRunAutomaton, text: &str) {
    let b = LuceneTestCase::new_bytes_ref_str(text);
    Assert::assert_true(automaton.run_range(&b.bytes, b.offset, b.length));
}

fn assert_no_matches(automaton: &ByteRunAutomaton, text: &str) {
    let b = LuceneTestCase::new_bytes_ref_str(text);
    Assert::assert_false(automaton.run_range(&b.bytes, b.offset, b.length));
}
