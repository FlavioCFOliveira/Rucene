//! Port of `lucene/core/src/test/org/apache/lucene/search/TestSameScoresWithThreads.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::jdk::thread::Thread;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::index::index_writer::MAX_TERM_LENGTH;
use rucene::index::multi_fields::MultiTerms;
use rucene::index::{IndexReader, Term};
use rucene::search::term_query::TermQuery;
use rucene::search::TopDocs;
use rucene::util::BytesRef;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::util::java_exceptions::rethrow_java;
use rucene_test_framework::util::line_file_docs::LineFileDocs;
use rucene_test_framework::util::lucene_test_case::TEST_NIGHTLY;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_search::support::CountDownLatch;
use crate::core_support::{assert_equals, assert_equals_msg, class, random, shuffle, test};

/// `TestSameScoresWithThreads`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.TestSameScoresWithThreads",
        vec![test("test", test_same_scores_with_threads)],
    )
}

/// Port of `TestSameScoresWithThreads.test`.
fn test_same_scores_with_threads() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let analyzer = MockAnalyzer::new(&*random());
    analyzer.set_max_token_length(TestUtil::next_int(&*random(), 1, MAX_TERM_LENGTH as i32));
    let w = RandomIndexWriter::new(&*random(), dir.as_directory(), Arc::new(analyzer))?;
    let docs = LineFileDocs::with_default_file(&*random())?;
    let chars_to_index = LuceneTestCase::at_least(100000);
    let mut chars_indexed = 0;
    while chars_indexed < chars_to_index {
        let doc = docs.next_doc()?;
        let doc = doc.borrow();
        // `doc.get("body").length()`: UTF-16 units.
        chars_indexed += doc.get("body").map_or(0, |b| b.encode_utf16().count()) as i32;
        w.add_document(&*doc)?;
    }
    let r = w.get_reader()?;
    w.close()?;

    let s = Arc::new(LuceneTestCase::new_searcher(Arc::clone(&r)));
    let ir: Arc<dyn IndexReader> = Arc::clone(&r) as Arc<dyn IndexReader>;
    let terms = MultiTerms::get_terms(&ir, "body")?.unwrap_or_else(|| {
        randomizedtesting::jdk::exception::throw_without_message("java.lang.NullPointerException")
    });
    let mut term_count = 0;
    let mut terms_enum = terms.iterator()?;
    while terms_enum.next()?.is_some() {
        term_count += 1;
    }
    Assert::assert_true(term_count > 0);

    // Target ~10 terms to search:
    let chance = 10.0 / f64::from(term_count);
    let mut terms_enum = terms.iterator()?;
    // `HashMap<BytesRef, TopDocs>`: the entries, in the order Java iterates
    // them is only shuffled before use, so insertion order is kept.
    let mut answers: Vec<(BytesRef, TopDocs)> = Vec::new();
    while terms_enum.next()?.is_some() {
        if random().next_double() <= chance {
            let term = BytesRef::deep_copy_of(&terms_enum.term()?);
            let top_docs = s.search(
                Arc::new(TermQuery::new(Term::new("body", term.clone()))),
                100,
            )?;
            answers.push((term, top_docs));
        }
    }

    if !answers.is_empty() {
        let answers = Arc::new(answers);
        let starting_gun = Arc::new(CountDownLatch::new(1));
        let num_threads = if *TEST_NIGHTLY {
            TestUtil::next_int(&*random(), 2, 5)
        } else {
            2
        };
        let mut threads = Vec::with_capacity(num_threads as usize);
        for _thread_id in 0..num_threads {
            let starting_gun = Arc::clone(&starting_gun);
            let answers = Arc::clone(&answers);
            let s = Arc::clone(&s);
            let thread = Thread::unnamed(move || {
                let run = || -> Result<()> {
                    starting_gun.await_();
                    for _i in 0..20 {
                        let mut shuffled: Vec<&(BytesRef, TopDocs)> = answers.iter().collect();
                        shuffle(&mut shuffled, &*random());
                        for ent in shuffled {
                            let actual = s.search(
                                Arc::new(TermQuery::new(Term::new("body", ent.0.clone()))),
                                100,
                            )?;
                            let expected = &ent.1;
                            assert_equals(&expected.total_hits.value(), &actual.total_hits.value());
                            assert_equals_msg(
                                Some(&format!("query={}", ent.0.utf8_to_string()?)),
                                &expected.score_docs.len(),
                                &actual.score_docs.len(),
                            );
                            for hit in 0..expected.score_docs.len() {
                                assert_equals(
                                    &expected.score_docs[hit].doc,
                                    &actual.score_docs[hit].doc,
                                );
                                // Floats really should be identical:
                                Assert::assert_true(
                                    expected.score_docs[hit].score == actual.score_docs[hit].score,
                                );
                            }
                        }
                    }
                    Ok(())
                };
                // `catch (Exception e) { throw new RuntimeException(e); }`
                if let Err(e) = run() {
                    rethrow_java(e)
                }
            });
            thread.start();
            threads.push(thread);
        }
        starting_gun.count_down();
        for thread in &threads {
            let _ = thread.join();
        }
    }
    docs.close();
    drop(s);
    r.close()?;
    dir.close()?;
    Ok(())
}
