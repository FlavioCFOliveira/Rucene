//! Port of `lucene/core/src/test/org/apache/lucene/index/TestBagOfPostings.java`.
//!
//! Simple test that adds numeric terms, where each term has the docFreq of its integer value, and checks that the docFreq is correct.
//!
//! `ConcurrentLinkedQueue<String>` is a `Mutex<VecDeque<String>>`; the
//! `CountDownLatch` starting gun is [`CountDownLatch`].

use std::collections::{HashSet, VecDeque};
use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::jdk::thread::Thread;
use rucene::document::{Document, Field, FieldType, Store};
use rucene::error::Result;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::index::{MockRandomMergePolicy, RandomIndexWriter};
use rucene_test_framework::util::java_exceptions::java_throwable;
use rucene_test_framework::util::lucene_test_case::{
    SuppressCodecs, RANDOM_MULTIPLIER, TEST_NIGHTLY,
};
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::{assert_equals, class_annotated, random, shuffle, test, verbose};
use crate::support::{npe, CountDownLatch};

/// `TestBagOfPostings`.
pub fn class_def() -> Arc<Class> {
    class_annotated(
        "org.apache.lucene.index.TestBagOfPostings",
        vec![test("test", test_)],
        |b| {
            // at night this makes like 200k/300k docs and will make Direct's heart beat!
            b.annotate(SuppressCodecs {
                value: vec!["Direct".to_string()],
            })
        },
    )
}

/// Port of `TestBagOfPostings.test`.
fn test_() -> Result<()> {
    let mut postings_list: Vec<String> = Vec::new();
    let mut num_terms = LuceneTestCase::at_least(300);
    let max_terms_per_doc = TestUtil::next_int(&*random(), 10, 20);
    let is_simple_text = "SimpleText" == TestUtil::get_postings_format("field");

    let iwc = LuceneTestCase::new_index_writer_config_with(
        &*random(),
        Arc::new(MockAnalyzer::new(&*random())),
    );

    let is_mock_random = iwc
        .merge_policy()
        .as_any()
        .is_some_and(|a| a.is::<MockRandomMergePolicy>());
    if (is_simple_text || is_mock_random) && (*TEST_NIGHTLY || *RANDOM_MULTIPLIER > 1) {
        // Otherwise test can take way too long (> 2 hours)
        num_terms /= 2;
    }

    if verbose() {
        println!("maxTermsPerDoc={max_terms_per_doc}");
        println!("numTerms={num_terms}");
    }

    for i in 0..num_terms {
        let term = i.to_string();
        for _ in 0..i {
            postings_list.push(term.clone());
        }
    }
    shuffle(&mut postings_list, &*random());

    let postings: Arc<Mutex<VecDeque<String>>> =
        Arc::new(Mutex::new(postings_list.into_iter().collect()));

    let mut dir =
        LuceneTestCase::new_fs_directory(&LuceneTestCase::create_temp_dir_prefix("bagofpostings"));

    let iw = Arc::new(RandomIndexWriter::new_with_config(
        &*random(),
        dir.as_directory(),
        iwc,
    )?);

    let thread_count = TestUtil::next_int(&*random(), 1, 5);
    if verbose() {
        println!("config: {}", iw.w().get_config());
        println!("threadCount={thread_count}");
    }

    let mut threads = Vec::with_capacity(thread_count as usize);
    let starting_gun = Arc::new(CountDownLatch::new(1));

    for _ in 0..thread_count {
        let postings = Arc::clone(&postings);
        let iw = Arc::clone(&iw);
        let starting_gun = Arc::clone(&starting_gun);
        let thread = Thread::unnamed(move || {
            let outcome =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<()> {
                    // `Field field = newTextField("field", "", Field.Store.NO)`: its
                    // type; each document carries a field of that type with the
                    // current text (guide section 6).
                    let field_type = FieldType::new_from(
                        LuceneTestCase::new_text_field("field", "", Store::NO).field_type(),
                    );
                    starting_gun.await_();
                    while !postings
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .is_empty()
                    {
                        let mut text = String::new();
                        let mut visited: HashSet<String> = HashSet::new();
                        for _ in 0..max_terms_per_doc {
                            let token = postings
                                .lock()
                                .unwrap_or_else(|e| e.into_inner())
                                .pop_front();
                            let Some(token) = token else {
                                break;
                            };
                            if visited.contains(&token) {
                                // Put it back:
                                postings
                                    .lock()
                                    .unwrap_or_else(|e| e.into_inner())
                                    .push_back(token);
                                break;
                            }
                            text.push(' ');
                            text.push_str(&token);
                            visited.insert(token);
                        }
                        let mut document = Document::new();
                        document.add(Box::new(Field::new("field", text, field_type.clone())?));
                        iw.add_document(&document)?;
                    }
                    Ok(())
                }));
            let e = match outcome {
                Ok(Ok(())) => return,
                Ok(Err(e)) => java_throwable(&e),
                Err(payload) => Throwable::from_panic(payload),
            };
            Throwable::from_cause("java.lang.RuntimeException", e).throw()
        });
        thread.start();
        threads.push(thread);
    }
    starting_gun.count_down();
    for t in &threads {
        let _ = t.join();
    }

    iw.force_merge(1)?;
    let ir = iw.get_reader()?;
    let leaves = std::sync::Arc::clone(&ir).leaves()?;
    assert_equals(&1, &leaves.len());
    let air = leaves[0].leaf_reader();
    let terms = air.terms("field")?.unwrap_or_else(|| npe("terms"));
    // numTerms-1 because there cannot be a term 0 with 0 postings:
    assert_equals(&((num_terms - 1) as i64), &terms.size()?);
    let mut terms_enum = terms.iterator()?;
    while let Some(term) = terms_enum.next()? {
        let term_text = term.utf8_to_string()?;
        let value: i32 = term_text.parse().unwrap_or_else(|_| {
            Throwable::new("java.lang.NumberFormatException", term_text.clone()).throw()
        });
        assert_equals(&value, &terms_enum.doc_freq()?);
        // don't really need to check more than this, as CheckIndex
        // will verify that docFreq == actual number of documents seen
        // from a postingsEnum.
    }
    ir.close()?;
    iw.close()?;
    dir.close()
}
