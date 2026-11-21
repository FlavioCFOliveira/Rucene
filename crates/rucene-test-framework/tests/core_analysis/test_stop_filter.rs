//! Port of `org.apache.lucene.analysis.TestStopFilter`
//! (`extends BaseTokenStreamTestCase`).

use std::sync::Arc;

use randomizedtesting::jdk::reflect::{Class, Constructor};
use randomizedtesting::jdk::util::shuffle;
use randomizedtesting::junit::Assert;
use rucene::analysis::stop_filter::{make_stop_set, new_stop_filter, StopFilter};
use rucene::analysis::{CharArraySet, StringCharReader, TokenStream};
use rucene::error::Result;
use rucene::util::java_collections::JavaHashSet;
use rucene_test_framework::analysis::attributes::{char_term_id, pos_inc_id, Att};
use rucene_test_framework::analysis::mock_tokenizer::{MockTokenizer, WHITESPACE};
use rucene_test_framework::analysis::BaseTokenStreamTestCase;
use rucene_test_framework::util::lucene_test_case::VERBOSE;
use rucene_test_framework::util::{English, LuceneTestCase};

use crate::lucene_core_shims::t;

const MAX_NUMBER_OF_TOKENS: i32 = 50;

/// `random()`.
fn random() -> Arc<dyn randomizedtesting::jdk::random::Random> {
    LuceneTestCase::random()
}

/// `new MockTokenizer(MockTokenizer.WHITESPACE, false)` over `reader`
/// (`in.setReader(reader)`).
fn whitespace_tokenizer(text: &str) -> Result<MockTokenizer> {
    let mut tokenizer = MockTokenizer::new(Arc::clone(&WHITESPACE), false)?;
    tokenizer.set_reader(Box::new(StringCharReader::new(text)))?;
    Ok(tokenizer)
}

// other StopFilter functionality is already tested by TestStopAnalyzer

fn test_exact_case() -> Result<()> {
    // `new CharArraySet(asSet("is", "the", "Time"), false)`
    let stop_words = CharArraySet::from_iter(["is", "the", "Time"], false);
    let r#in = whitespace_tokenizer("Now is The Time")?;
    let mut stream = new_stop_filter(Box::new(r#in), stop_words)?;
    BaseTokenStreamTestCase::assert_token_stream_contents(&mut stream, &["Now", "The"])
}

fn test_stop_filter() -> Result<()> {
    let stop_words = ["is", "the", "Time"];
    // `StopFilter.makeStopSet(String...)`: `makeStopSet(stopWords, false)`.
    let stop_set = make_stop_set(stop_words, false);
    let r#in = whitespace_tokenizer("Now is The Time")?;
    let mut stream = new_stop_filter(Box::new(r#in), stop_set)?;
    BaseTokenStreamTestCase::assert_token_stream_contents(&mut stream, &["Now", "The"])
}

/// `logStopwords(String name, Collection<String> stopwords)` (`private
/// static`): `stopwords.toString()` is `[a, b]`.
fn log_stopwords<'a>(name: &str, stopwords: impl IntoIterator<Item = &'a String>) {
    let stopwords: Vec<&String> = stopwords.into_iter().collect();
    log(&format!(
        "stopword list [{}]: {}",
        name,
        if stopwords.is_empty() {
            "Empty".to_string()
        } else {
            format!(
                "[{}]",
                stopwords
                    .iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        }
    ));
}

/// Randomly generate a document and a list of stopwords to apply
///
/// `number_of_tokens` max number of tokens in the document; `sb` will
/// contain the text at the end of the method; `stopwords` will contain the
/// list of the stopwords at the end of the method; `stopword_positions` will
/// contain the position of the stopwords at the end of the method.
fn generate_test_set_with_stopwords_and_stopword_positions(
    number_of_tokens: i32,
    sb: &mut String,
    stopwords: &mut Vec<String>,
    stopword_positions: &mut Vec<i32>,
) {
    let rand = random();
    for i in 0..number_of_tokens {
        let token = English::int_to_english(i).trim().to_string();
        sb.push_str(&token);
        sb.push(' ');
        if i == 0 || rand.next_boolean() {
            // with probability 0.5 will tell if this is a stopword or
            // no - adding always the first token to make sure that the
            // list of stopwords is not empty;
            stopwords.push(token);
            stopword_positions.push(i);
        }
    }
    log(&format!("Number of tokens : {number_of_tokens}"));
    log(&format!("Document : {sb}"));
    log_stopwords("Stopwords", stopwords.iter());
}

/// Check that the positions of the terms in a document keep into account the
/// fact that some of the words were filtered by the StopwordFilter
fn test_token_position_with_stopword_filter() -> Result<()> {
    // at least 1 token
    let number_of_tokens = random().next_int_bound(MAX_NUMBER_OF_TOKENS - 1) + 1;
    let mut sb = String::new();
    let mut stopwords: Vec<String> = Vec::with_capacity(number_of_tokens as usize);
    let mut stopword_positions: Vec<i32> = Vec::with_capacity(number_of_tokens as usize);
    generate_test_set_with_stopwords_and_stopword_positions(
        number_of_tokens,
        &mut sb,
        &mut stopwords,
        &mut stopword_positions,
    );

    // `StopFilter.makeStopSet(List<String>)`: `makeStopSet(list, false)`.
    let stop_set = make_stop_set(&stopwords, false);
    log_stopwords("All stopwords", stopwords.iter());
    // with increments
    let r#in = whitespace_tokenizer(&sb)?;
    let mut stopfilter = new_stop_filter(Box::new(r#in), stop_set)?;
    do_test_stopwords_positions(&mut stopfilter, &stopword_positions, number_of_tokens)
}

/// Check that the positions of the terms in a document keep into account the
/// fact that some of the words were filtered by two StopwordFilters
/// concatenated together.
fn test_token_positions_with_concatenated_stopword_filters() -> Result<()> {
    // at least 1 token
    let number_of_tokens = random().next_int_bound(MAX_NUMBER_OF_TOKENS - 1) + 1;
    let mut sb = String::new();
    let mut stopwords: Vec<String> = Vec::with_capacity(number_of_tokens as usize);
    let mut stopword_positions: Vec<i32> = Vec::new();
    generate_test_set_with_stopwords_and_stopword_positions(
        number_of_tokens,
        &mut sb,
        &mut stopwords,
        &mut stopword_positions,
    );

    // we want to make sure that concatenating two list of stopwords
    // produce the same results of using one unique list of stopwords.
    // So we first generate a list of stopwords:
    // e.g.: [a, b, c, d, e]
    // and then we split the list in two disjoint partitions
    // e.g. [a, c, e] [b, d]
    let partition = random().next_int_bound(stopwords.len() as i32) as usize;
    shuffle(&mut stopwords, &*random());
    let stopwords_random_partition: Vec<String> = stopwords[..partition].to_vec();
    let mut stopwords_remaining = JavaHashSet::from_collection(stopwords.iter().cloned());
    // remove the first partition from all the stopwords
    for word in &stopwords_random_partition {
        stopwords_remaining.remove(word);
    }

    let first_stop_set = make_stop_set(&stopwords_random_partition, false);
    log_stopwords("Stopwords-first", stopwords_random_partition.iter());
    let second_stop_set: CharArraySet = make_stop_set(
        stopwords_remaining.iter().cloned().collect::<Vec<String>>(),
        false,
    );
    log_stopwords("Stopwords-second", stopwords_remaining.iter());

    let in1 = whitespace_tokenizer(&sb)?;

    // Here we create a stopFilter with the stopwords in the first partition and then we
    // concatenate it with the stopFilter created with the stopwords in the second partition
    let stop_filter = new_stop_filter(Box::new(in1), first_stop_set)?; // first part of the set
    let mut concatenated_stop_filter = new_stop_filter(Box::new(stop_filter), second_stop_set)?; // two stop filters concatenated!

    // ... and finally we check that the positions of the filtered tokens matched using the
    // concatenated
    // stopFilters match the positions of the filtered tokens using the unique original list of
    // stopwords
    do_test_stopwords_positions(
        &mut concatenated_stop_filter,
        &stopword_positions,
        number_of_tokens,
    )
}

// LUCENE-3849: make sure after .end() we see the "ending" posInc
fn test_end_stopword() -> Result<()> {
    // `StopFilter.makeStopSet("of")`
    let stop_set = make_stop_set(["of"], false);
    let r#in = whitespace_tokenizer("test of")?;
    let mut stopfilter = new_stop_filter(Box::new(r#in), stop_set)?;
    BaseTokenStreamTestCase::assert_token_stream_contents_full(
        &mut stopfilter,
        &["test"],
        Some(&[0]),
        Some(&[4]),
        None,
        Some(&[1]),
        None,
        Some(7),
        Some(1),
        None,
        true,
        None,
        None,
        None,
    )
}

/// `doTestStopwordsPositions(StopFilter stopfilter, List<Integer>
/// stopwordPositions, int numberOfTokens)` (`private`).
fn do_test_stopwords_positions(
    stopfilter: &mut StopFilter,
    stopword_positions: &[i32],
    number_of_tokens: i32,
) -> Result<()> {
    let term_att = Att::get(stopfilter.attribute_source(), char_term_id())
        .expect("getAttribute(CharTermAttribute.class)");
    Att::get(stopfilter.attribute_source(), pos_inc_id())
        .expect("getAttribute(PositionIncrementAttribute.class)");
    stopfilter.reset()?;
    log("Test stopwords positions:");
    for i in 0..number_of_tokens {
        if stopword_positions.contains(&i) {
            // if i is in stopwordPosition it is a stopword and we skip this position
            continue;
        }
        Assert::assert_true(stopfilter.increment_token()?);
        log(&format!("token {}: {}", i, term_att.term_string()));
        let token = English::int_to_english(i).trim().to_string();
        Assert::assert_equals_str(
            Some(format!("expecting token {i} to be {token}").as_str()),
            Some(token.as_str()),
            Some(term_att.term_string().as_str()),
        );
    }
    Assert::assert_false(stopfilter.increment_token()?);
    stopfilter.end()?;
    stopfilter.close()?;
    log("----------");
    Ok(())
}

// print debug info depending on VERBOSE
fn log(s: &str) {
    if *VERBOSE {
        println!("{s}");
    }
}

/// The class `org.apache.lucene.analysis.TestStopFilter`.
pub fn class() -> Arc<Class> {
    Class::builder("org.apache.lucene.analysis.TestStopFilter")
        .extends(&BaseTokenStreamTestCase::class())
        .constructor(Constructor::no_args(|| ()))
        .method(t("testExactCase", test_exact_case))
        .method(t("testStopFilter", test_stop_filter))
        .method(t(
            "testTokenPositionWithStopwordFilter",
            test_token_position_with_stopword_filter,
        ))
        .method(t(
            "testTokenPositionsWithConcatenatedStopwordFilters",
            test_token_positions_with_concatenated_stopword_filters,
        ))
        .method(t("testEndStopword", test_end_stopword))
        .build()
}
