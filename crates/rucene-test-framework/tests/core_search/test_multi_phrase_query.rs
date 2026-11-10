//! Port of `lucene/core/src/test/org/apache/lucene/search/TestMultiPhraseQuery.java`.
//!
//! This class tests the MultiPhraseQuery class.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::{Assert, Ignore};
use rucene::analysis::tokenattributes::{CharTermAttribute, PositionIncrementAttribute};
use rucene::analysis::TokenStream;
use rucene::document::{Document, Field, FieldType, Store, TextField};
use rucene::error::Result;
use rucene::index::directory_reader::{self, DirectoryReader};
use rucene::index::index_writer::IndexWriter;
use rucene::index::multi_fields::MultiTerms;
use rucene::index::{IndexReader, Term};
use rucene::search::multi_phrase_query::{Builder as MultiPhraseQueryBuilder, MultiPhraseQuery};
use rucene::search::phrase_query::Builder as PhraseQueryBuilder;
use rucene::search::{query_to_string, BooleanQueryBuilder, Occur, Query};
use rucene::store::{ByteBuffersDirectory, Directory};
use rucene::util::BytesRef;
use rucene_test_framework::analysis::{CannedTokenStream, Token};
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_search::support::{term, tq};
use crate::core_support::{
    assert_equals, assert_equals_delta, assert_equals_msg, class, random, test, verbose,
};

/// `TestMultiPhraseQuery`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.TestMultiPhraseQuery",
        vec![
            test("testPhrasePrefix", test_phrase_prefix),
            test("testTall", test_tall),
            // @Ignore: LUCENE-3821 fixes sloppy phrase scoring, except for
            // this known problem
            test("testMultiSloppyWithRepeats", test_multi_sloppy_with_repeats).annotate(Ignore {
                value: String::new(),
            }),
            test("testMultiExactWithRepeats", test_multi_exact_with_repeats),
            test(
                "testBooleanQueryContainingSingleTermPrefixQuery",
                test_boolean_query_containing_single_term_prefix_query,
            ),
            test(
                "testPhrasePrefixWithBooleanQuery",
                test_phrase_prefix_with_boolean_query,
            ),
            test("testNoDocs", test_no_docs),
            test("testHashCodeAndEquals", test_hash_code_and_equals),
            test("testEmptyToString", test_empty_to_string),
            test("testZeroPosIncr", test_zero_pos_incr),
            test(
                "testZeroPosIncrSloppyParsedAnd",
                test_zero_pos_incr_sloppy_parsed_and,
            ),
            test(
                "testZeroPosIncrSloppyPqAnd",
                test_zero_pos_incr_sloppy_pq_and,
            ),
            test(
                "testZeroPosIncrSloppyMpqAnd",
                test_zero_pos_incr_sloppy_mpq_and,
            ),
            test(
                "testZeroPosIncrSloppyMpqAndOrMatch",
                test_zero_pos_incr_sloppy_mpq_and_or_match,
            ),
            test(
                "testZeroPosIncrSloppyMpqAndOrNoMatch",
                test_zero_pos_incr_sloppy_mpq_and_or_no_match,
            ),
            test("testNegativeSlop", test_negative_slop),
        ],
    )
}

fn body(text: &str) -> Term {
    term("body", text)
}

fn field(text: &str) -> Term {
    term("field", text)
}

/// `te.term().utf8ToString()`.
fn utf8(te: &dyn rucene::index::TermsEnum) -> Result<String> {
    Ok(te.term()?.utf8_to_string()?)
}

/// Port of `TestMultiPhraseQuery.testPhrasePrefix`.
fn test_phrase_prefix() -> Result<()> {
    let mut index_store = LuceneTestCase::new_directory();
    let writer = RandomIndexWriter::new_with_mock_analyzer(&*random(), index_store.as_directory())?;
    add("blueberry pie", &writer)?;
    add("blueberry strudel", &writer)?;
    add("blueberry pizza", &writer)?;
    add("blueberry chewing gum", &writer)?;
    add("bluebird pizza", &writer)?;
    add("bluebird foobar pizza", &writer)?;
    add("piccadilly circus", &writer)?;

    let reader = writer.get_reader()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));

    // search for "blueberry pi*":
    let mut query1builder = MultiPhraseQueryBuilder::new();
    // search for "strawberry pi*":
    let mut query2builder = MultiPhraseQueryBuilder::new();
    query1builder.add(body("blueberry"))?;
    query2builder.add(body("strawberry"))?;

    let mut terms_with_prefix: Vec<Term> = Vec::new();

    // this TermEnum gives "piccadilly", "pie" and "pizza".
    let mut prefix = "pi";
    let ir: Arc<dyn IndexReader> = Arc::clone(&reader) as Arc<dyn IndexReader>;
    let mut te = MultiTerms::get_terms(&ir, "body")?
        .expect("body is indexed")
        .iterator()?;
    te.seek_ceil(&BytesRef::from_char_sequence(prefix))?;
    loop {
        let s = utf8(&*te)?;
        if s.starts_with(prefix) {
            terms_with_prefix.push(body(&s));
        } else {
            break;
        }
        if te.next()?.is_none() {
            break;
        }
    }

    query1builder.add_terms(terms_with_prefix.clone())?;
    let query1 = query1builder.build();
    assert_equals(
        &"body:\"blueberry (piccadilly pie pizza)\"".to_string(),
        &query_to_string(&query1),
    );

    query2builder.add_terms(terms_with_prefix.clone())?;
    let query2 = query2builder.build();
    assert_equals(
        &"body:\"strawberry (piccadilly pie pizza)\"".to_string(),
        &query_to_string(&query2),
    );

    let result = searcher.search(Arc::new(query1), 1000)?.score_docs;
    assert_equals(&2, &result.len());
    let result = searcher.search(Arc::new(query2), 1000)?.score_docs;
    assert_equals(&0, &result.len());

    // search for "blue* pizza":
    let mut query3builder = MultiPhraseQueryBuilder::new();
    terms_with_prefix.clear();
    prefix = "blue";
    te.seek_ceil(&BytesRef::from_char_sequence(prefix))?;

    loop {
        if utf8(&*te)?.starts_with(prefix) {
            terms_with_prefix.push(body(&utf8(&*te)?));
        }
        if te.next()?.is_none() {
            break;
        }
    }

    query3builder.add_terms(terms_with_prefix.clone())?;
    query3builder.add(body("pizza"))?;

    let query3 = query3builder.build();

    let result = searcher.search(Arc::new(query3.clone()), 1000)?.score_docs;
    assert_equals(&2, &result.len()); // blueberry pizza, bluebird pizza
    assert_equals(
        &"body:\"(blueberry bluebird) pizza\"".to_string(),
        &query_to_string(&query3),
    );

    // test slop:
    query3builder.set_slop(1)?;
    let query3: Arc<dyn Query> = Arc::new(query3builder.build());
    let result = searcher.search(Arc::clone(&query3), 1000)?.score_docs;

    // just make sure no exc:
    searcher.explain(query3, 0)?;

    assert_equals(&3, &result.len()); // blueberry pizza, bluebird pizza, bluebird
                                      // foobar pizza

    let mut query4builder = MultiPhraseQueryBuilder::new();
    expect_throws("java.lang.IllegalArgumentException", || {
        query4builder.add(term("field1", "foo"))?;
        query4builder.add(term("field2", "foobar"))?;
        Ok(())
    });

    writer.close()?;
    drop(searcher);
    reader.close()?;
    index_store.close()?;
    Ok(())
}

/// LUCENE-2580
///
/// Port of `TestMultiPhraseQuery.testTall`.
fn test_tall() -> Result<()> {
    let mut index_store = LuceneTestCase::new_directory();
    let writer = RandomIndexWriter::new_with_mock_analyzer(&*random(), index_store.as_directory())?;
    add("blueberry chocolate pie", &writer)?;
    add("blueberry chocolate tart", &writer)?;
    let r = writer.get_reader()?;
    writer.close()?;

    let searcher = LuceneTestCase::new_searcher(Arc::clone(&r));
    let mut qb = MultiPhraseQueryBuilder::new();
    qb.add(body("blueberry"))?;
    qb.add(body("chocolate"))?;
    qb.add_terms(vec![body("pie"), body("tart")])?;
    assert_equals(&2, &searcher.count(Arc::new(qb.build()))?);
    drop(searcher);
    r.close()?;
    index_store.close()?;
    Ok(())
}

/// Port of `TestMultiPhraseQuery.testMultiSloppyWithRepeats`.
fn test_multi_sloppy_with_repeats() -> Result<()> {
    let mut index_store = LuceneTestCase::new_directory();
    let writer = RandomIndexWriter::new_with_mock_analyzer(&*random(), index_store.as_directory())?;
    add("a b c d e f g h i k", &writer)?;
    let r = writer.get_reader()?;
    writer.close()?;

    let searcher = LuceneTestCase::new_searcher(Arc::clone(&r));

    let mut qb = MultiPhraseQueryBuilder::new();
    // this will fail, when the scorer would propagate [a] rather than [a,b],
    qb.add_terms(vec![body("a"), body("b")])?;
    qb.add_terms(vec![body("a")])?;
    qb.set_slop(6)?;
    assert_equals(&1, &searcher.count(Arc::new(qb.build()))?); // should match on "a b"

    drop(searcher);
    r.close()?;
    index_store.close()?;
    Ok(())
}

/// Port of `TestMultiPhraseQuery.testMultiExactWithRepeats`.
fn test_multi_exact_with_repeats() -> Result<()> {
    let mut index_store = LuceneTestCase::new_directory();
    let writer = RandomIndexWriter::new_with_mock_analyzer(&*random(), index_store.as_directory())?;
    add("a b c d e f g h i k", &writer)?;
    let r = writer.get_reader()?;
    writer.close()?;

    let searcher = LuceneTestCase::new_searcher(Arc::clone(&r));
    let mut qb = MultiPhraseQueryBuilder::new();
    qb.add_at(vec![body("a"), body("d")], 0)?;
    qb.add_at(vec![body("a"), body("f")], 2)?;
    assert_equals(&1, &searcher.count(Arc::new(qb.build()))?); // should match on "a b"
    drop(searcher);
    r.close()?;
    index_store.close()?;
    Ok(())
}

fn add(s: &str, writer: &RandomIndexWriter) -> Result<()> {
    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_text_field(
        "body",
        s,
        Store::YES,
    )));
    writer.add_document(&doc)?;
    Ok(())
}

/// Port of `TestMultiPhraseQuery.testBooleanQueryContainingSingleTermPrefixQuery`.
fn test_boolean_query_containing_single_term_prefix_query() -> Result<()> {
    // this tests against bug 33161 (now fixed)
    // In order to cause the bug, the outer query must have more than one term
    // and all terms required.
    // The contained PhraseMultiQuery must contain exactly one term array.
    let mut index_store = LuceneTestCase::new_directory();
    let writer = RandomIndexWriter::new_with_mock_analyzer(&*random(), index_store.as_directory())?;
    add("blueberry pie", &writer)?;
    add("blueberry chewing gum", &writer)?;
    add("blue raspberry pie", &writer)?;

    let reader = writer.get_reader()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    // This query will be equivalent to +body:pie +body:"blue*"
    let mut q = BooleanQueryBuilder::new();
    q.add(tq("body", "pie"), Occur::MUST)?;

    let mut trouble_builder = MultiPhraseQueryBuilder::new();
    trouble_builder.add_terms(vec![body("blueberry"), body("blue")])?;
    q.add(Arc::new(trouble_builder.build()), Occur::MUST)?;

    // exception will be thrown here without fix
    let hits = searcher.search(Arc::new(q.build()), 1000)?.score_docs;

    assert_equals_msg(Some("Wrong number of hits"), &2, &hits.len());

    // just make sure no exc:
    searcher.explain(Arc::new(q.build()), 0)?;

    writer.close()?;
    drop(searcher);
    reader.close()?;
    index_store.close()?;
    Ok(())
}

/// Port of `TestMultiPhraseQuery.testPhrasePrefixWithBooleanQuery`.
fn test_phrase_prefix_with_boolean_query() -> Result<()> {
    let mut index_store = LuceneTestCase::new_directory();
    let writer = RandomIndexWriter::new_with_mock_analyzer(&*random(), index_store.as_directory())?;
    add_typed("This is a test", "object", &writer)?;
    add_typed("a note", "note", &writer)?;

    let reader = writer.get_reader()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));

    // This query will be equivalent to +type:note +body:"a t*"
    let mut q = BooleanQueryBuilder::new();
    q.add(tq("type", "note"), Occur::MUST)?;

    let mut trouble_builder = MultiPhraseQueryBuilder::new();
    trouble_builder.add(body("a"))?;
    trouble_builder.add_terms(vec![body("test"), body("this")])?;
    q.add(Arc::new(trouble_builder.build()), Occur::MUST)?;

    // exception will be thrown here without fix for #35626:
    let hits = searcher.search(Arc::new(q.build()), 1000)?.score_docs;
    assert_equals_msg(Some("Wrong number of hits"), &0, &hits.len());
    writer.close()?;
    drop(searcher);
    reader.close()?;
    index_store.close()?;
    Ok(())
}

/// Port of `TestMultiPhraseQuery.testNoDocs`.
fn test_no_docs() -> Result<()> {
    let mut index_store = LuceneTestCase::new_directory();
    let writer = RandomIndexWriter::new_with_mock_analyzer(&*random(), index_store.as_directory())?;
    add_typed("a note", "note", &writer)?;

    let reader = writer.get_reader()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));

    let mut qb = MultiPhraseQueryBuilder::new();
    qb.add(body("a"))?;
    qb.add_terms(vec![body("nope"), body("nope")])?;
    let q: Arc<dyn Query> = Arc::new(qb.build());
    assert_equals_msg(
        Some("Wrong number of hits"),
        &0,
        &searcher.count(Arc::clone(&q))?,
    );

    // just make sure no exc:
    searcher.explain(q, 0)?;

    writer.close()?;
    drop(searcher);
    reader.close()?;
    index_store.close()?;
    Ok(())
}

/// `assertEquals(query1, query2)` over `Query.equals`.
fn assert_query_equals(q1: &MultiPhraseQuery, q2: &MultiPhraseQuery) {
    if !q1.query_eq(q2) {
        Assert::fail(&format!(
            "expected:<{}> but was:<{}>",
            query_to_string(q1),
            query_to_string(q2)
        ));
    }
}

/// Port of `TestMultiPhraseQuery.testHashCodeAndEquals`.
fn test_hash_code_and_equals() -> Result<()> {
    let mut query1builder = MultiPhraseQueryBuilder::new();
    let mut query1 = query1builder.build();

    let mut query2builder = MultiPhraseQueryBuilder::new();
    let mut query2 = query2builder.build();

    assert_equals(&query1.query_hash(), &query2.query_hash());
    assert_query_equals(&query1, &query2);

    let term1 = term("someField", "someText");

    query1builder.add(term1.clone())?;
    query1 = query1builder.build();

    query2builder.add(term1)?;
    query2 = query2builder.build();

    assert_equals(&query1.query_hash(), &query2.query_hash());
    assert_query_equals(&query1, &query2);

    let term2 = term("someField", "someMoreText");

    query1builder.add(term2.clone())?;
    query1 = query1builder.build();

    Assert::assert_false(query1.query_hash() == query2.query_hash());
    Assert::assert_false(query1.query_eq(&query2));

    query2builder.add(term2)?;
    query2 = query2builder.build();

    assert_equals(&query1.query_hash(), &query2.query_hash());
    assert_query_equals(&query1, &query2);
    Ok(())
}

fn add_typed(s: &str, r#type: &str, writer: &RandomIndexWriter) -> Result<()> {
    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_text_field(
        "body",
        s,
        Store::YES,
    )));
    doc.add(Box::new(LuceneTestCase::new_string_field(
        "type",
        r#type,
        Store::NO,
    )));
    writer.add_document(&doc)?;
    Ok(())
}

/// LUCENE-2526
///
/// Port of `TestMultiPhraseQuery.testEmptyToString`.
fn test_empty_to_string() -> Result<()> {
    query_to_string(&MultiPhraseQueryBuilder::new().build());
    Ok(())
}

/// `new TextField("field", new CannedTokenStream(tokens))`.
fn canned_text_field(tokens: Vec<Token>) -> Result<Box<Field>> {
    let stream: Rc<RefCell<dyn TokenStream>> =
        Rc::new(RefCell::new(CannedTokenStream::new(tokens)));
    Ok(Box::new(Field::new_with_token_stream(
        "field",
        stream,
        FieldType::new_from(TextField::type_not_stored()),
    )?))
}

/// Port of `TestMultiPhraseQuery.testZeroPosIncr`.
fn test_zero_pos_incr() -> Result<()> {
    let mut dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::default());
    let tokens = vec![make_token("a", 1), make_token("b", 0), make_token("c", 0)];

    let writer = RandomIndexWriter::new_with_mock_analyzer(&*random(), Arc::clone(&dir))?;
    let mut doc = Document::new();
    doc.add(canned_text_field(tokens.clone())?);
    writer.add_document(&doc)?;
    let mut doc = Document::new();
    doc.add(canned_text_field(tokens)?);
    writer.add_document(&doc)?;
    let r = writer.get_reader()?;
    writer.close()?;
    let s = LuceneTestCase::new_searcher(Arc::clone(&r));
    let mut mpqb = MultiPhraseQueryBuilder::new();
    // mpq.setSlop(1);

    // NOTE: not great that if we do the else clause here we
    // get different scores!  MultiPhraseQuery counts that
    // phrase as occurring twice per doc (it should be 1, I
    // think?).  This is because MultipleTermPositions is able to
    // return the same position more than once (0, in this
    // case):
    #[allow(clippy::overly_complex_bool_expr)]
    if true {
        mpqb.add_at(vec![field("b"), field("c")], 0)?;
        mpqb.add_at(vec![field("a")], 0)?;
    } else {
        mpqb.add_at(vec![field("a")], 0)?;
        mpqb.add_at(vec![field("b"), field("c")], 0)?;
    }
    let hits = s.search(Arc::new(mpqb.build()), 2)?;
    assert_equals(&2, &hits.total_hits.value());
    assert_equals_delta(
        hits.score_docs[0].score as f64,
        hits.score_docs[1].score as f64,
        1e-5,
    );
    drop(s);
    r.close()?;
    drop(r);
    // `dir.close()`: the directory is shared with the writer and reader that
    // were just closed; once they are gone this is its only handle.
    if let Some(dir) = Arc::get_mut(&mut dir) {
        dir.close()?;
    }
    Ok(())
}

fn make_token(text: &str, pos_incr: i32) -> Token {
    let mut t = Token::new();
    t.append_string(Some(text));
    t.set_position_increment(pos_incr);
    t
}

/// `INCR_0_DOC_TOKENS`.
fn incr_0_doc_tokens() -> Vec<Token> {
    vec![
        make_token("x", 1),
        make_token("a", 1),
        make_token("1", 0),
        make_token("m", 1), // not existing, relying on slop=2
        make_token("b", 1),
        make_token("1", 0),
        make_token("n", 1), // not existing, relying on slop=2
        make_token("c", 1),
        make_token("y", 1),
    ]
}

/// `INCR_0_QUERY_TOKENS_AND`.
fn incr_0_query_tokens_and() -> Vec<Token> {
    vec![
        make_token("a", 1),
        make_token("1", 0),
        make_token("b", 1),
        make_token("1", 0),
        make_token("c", 1),
    ]
}

/// `INCR_0_QUERY_TOKENS_AND_OR_MATCH`.
fn incr_0_query_tokens_and_or_match() -> Vec<Vec<Token>> {
    vec![
        vec![make_token("a", 1)],
        vec![make_token("x", 1), make_token("1", 0)],
        vec![make_token("b", 2)],
        vec![make_token("x", 2), make_token("1", 0)],
        vec![make_token("c", 3)],
    ]
}

/// `INCR_0_QUERY_TOKENS_AND_OR_NO_MATCHN`.
fn incr_0_query_tokens_and_or_no_matchn() -> Vec<Vec<Token>> {
    vec![
        vec![make_token("x", 1)],
        vec![make_token("a", 1), make_token("1", 0)],
        vec![make_token("x", 2)],
        vec![make_token("b", 2), make_token("1", 0)],
        vec![make_token("c", 3)],
    ]
}

/// using query parser, MPQ will be created, and will not be strict about
/// having all query terms in each position - one of each position is
/// sufficient (OR logic)
///
/// Port of `TestMultiPhraseQuery.testZeroPosIncrSloppyParsedAnd`.
fn test_zero_pos_incr_sloppy_parsed_and() -> Result<()> {
    let mut qb = MultiPhraseQueryBuilder::new();
    qb.add_at(vec![field("a"), field("1")], -1)?;
    qb.add_at(vec![field("b"), field("1")], 0)?;
    qb.add_at(vec![field("c")], 1)?;
    do_test_zero_pos_incr_sloppy(Arc::new(qb.build()), 0)?;
    qb.set_slop(1)?;
    do_test_zero_pos_incr_sloppy(Arc::new(qb.build()), 0)?;
    qb.set_slop(2)?;
    do_test_zero_pos_incr_sloppy(Arc::new(qb.build()), 1)
}

fn do_test_zero_pos_incr_sloppy(q: Arc<dyn Query>, n_expected: i64) -> Result<()> {
    let mut dir = LuceneTestCase::new_directory(); // random dir
    let cfg = LuceneTestCase::new_index_writer_config_analyzer(None);
    let writer = IndexWriter::new(dir.as_directory(), cfg)?;
    let mut doc = Document::new();
    doc.add(canned_text_field(incr_0_doc_tokens())?);
    writer.add_document(&doc)?;
    let r: Arc<dyn DirectoryReader> = directory_reader::open_with_writer(&*writer)?;
    writer.close()?;
    let s = LuceneTestCase::new_searcher(Arc::clone(&r));

    if verbose() {
        println!("QUERY={}", query_to_string(&*q));
    }

    let hits = s.search(q, 1)?;
    assert_equals_msg(
        Some("wrong number of results"),
        &n_expected,
        &hits.total_hits.value(),
    );

    if verbose() {
        for hit in 0..hits.total_hits.value() as usize {
            let sd = &hits.score_docs[hit];
            println!("  hit doc={} score={}", sd.doc, sd.score);
        }
    }

    drop(s);
    r.close()?;
    dir.close()?;
    Ok(())
}

/// PQ AND Mode - Manually creating a phrase query
///
/// Port of `TestMultiPhraseQuery.testZeroPosIncrSloppyPqAnd`.
fn test_zero_pos_incr_sloppy_pq_and() -> Result<()> {
    let mut builder = PhraseQueryBuilder::new();
    let mut pos = -1;
    for tap in incr_0_query_tokens_and() {
        pos += tap.get_position_increment();
        builder.add_at(field(&tap.term()), pos)?;
    }
    builder.set_slop(0);
    do_test_zero_pos_incr_sloppy(Arc::new(builder.build()?), 0)?;
    builder.set_slop(1);
    do_test_zero_pos_incr_sloppy(Arc::new(builder.build()?), 0)?;
    builder.set_slop(2);
    do_test_zero_pos_incr_sloppy(Arc::new(builder.build()?), 1)
}

/// MPQ AND Mode - Manually creating a multiple phrase query
///
/// Port of `TestMultiPhraseQuery.testZeroPosIncrSloppyMpqAnd`.
fn test_zero_pos_incr_sloppy_mpq_and() -> Result<()> {
    let mut mpqb = MultiPhraseQueryBuilder::new();
    let mut pos = -1;
    for tap in incr_0_query_tokens_and() {
        pos += tap.get_position_increment();
        mpqb.add_at(vec![field(&tap.term())], pos)?; // AND logic
    }
    do_test_zero_pos_incr_sloppy(Arc::new(mpqb.build()), 0)?;
    mpqb.set_slop(1)?;
    do_test_zero_pos_incr_sloppy(Arc::new(mpqb.build()), 0)?;
    mpqb.set_slop(2)?;
    do_test_zero_pos_incr_sloppy(Arc::new(mpqb.build()), 1)
}

/// MPQ Combined AND OR Mode - Manually creating a multiple phrase query
///
/// Port of `TestMultiPhraseQuery.testZeroPosIncrSloppyMpqAndOrMatch`.
fn test_zero_pos_incr_sloppy_mpq_and_or_match() -> Result<()> {
    let mut mpqb = MultiPhraseQueryBuilder::new();
    for tap in incr_0_query_tokens_and_or_match() {
        let terms = tap_terms(&tap);
        let pos = tap[0].get_position_increment() - 1;
        mpqb.add_at(terms, pos)?; // AND logic in pos, OR across lines
    }
    do_test_zero_pos_incr_sloppy(Arc::new(mpqb.build()), 0)?;
    mpqb.set_slop(1)?;
    do_test_zero_pos_incr_sloppy(Arc::new(mpqb.build()), 0)?;
    mpqb.set_slop(2)?;
    do_test_zero_pos_incr_sloppy(Arc::new(mpqb.build()), 1)
}

/// MPQ Combined AND OR Mode - Manually creating a multiple phrase query -
/// with no match
///
/// Port of `TestMultiPhraseQuery.testZeroPosIncrSloppyMpqAndOrNoMatch`.
fn test_zero_pos_incr_sloppy_mpq_and_or_no_match() -> Result<()> {
    let mut mpqb = MultiPhraseQueryBuilder::new();
    for tap in incr_0_query_tokens_and_or_no_matchn() {
        let terms = tap_terms(&tap);
        let pos = tap[0].get_position_increment() - 1;
        mpqb.add_at(terms, pos)?; // AND logic in pos, OR across lines
    }
    do_test_zero_pos_incr_sloppy(Arc::new(mpqb.build()), 0)?;
    mpqb.set_slop(2)?;
    do_test_zero_pos_incr_sloppy(Arc::new(mpqb.build()), 0)
}

fn tap_terms(tap: &[Token]) -> Vec<Term> {
    tap.iter().map(|t| field(&t.term())).collect()
}

/// Port of `TestMultiPhraseQuery.testNegativeSlop`.
fn test_negative_slop() -> Result<()> {
    let mut query_builder = MultiPhraseQueryBuilder::new();
    query_builder.add(field("two"))?;
    query_builder.add(field("one"))?;
    expect_throws("java.lang.IllegalArgumentException", || {
        query_builder.set_slop(-2)?;
        Ok(())
    });
    Ok(())
}
