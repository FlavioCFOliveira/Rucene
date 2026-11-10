//! Port of `lucene/core/src/test/org/apache/lucene/index/TestLongPostings.java`.
//!
//! The field Java adds 1 to 4 times to a document is one shared instance
//! (`Document::add_shared`).

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::analysis::Analyzer;
use rucene::document::{Document, FieldType, Store, TextField};
use rucene::error::Result;
use rucene::index::index_writer_config::{IndexWriterConfig, OpenMode};
use rucene::index::multi_fields::MultiTerms;
use rucene::index::terms::Term;
use rucene::index::{
    IndexOptions, IndexReader, IndexableField, PostingsEnum, POSTINGS_ENUM_ALL,
    POSTINGS_ENUM_FREQS, POSTINGS_ENUM_NONE,
};
use rucene::search::NO_MORE_DOCS;
use rucene::util::{BytesRef, FixedBitSet};
use rucene_test_framework::analysis::attributes::{term_to_bytes_ref_id, Att};
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::index::RandomIndexWriter;
use rucene_test_framework::util::lucene_test_case::SuppressCodecs;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::{assert_equals, class_annotated, random, test, verbose};
use crate::support::npe;

/// `TestLongPostings` (`@SuppressCodecs({"SimpleText", "Direct"})`).
pub fn class_def() -> Arc<Class> {
    class_annotated(
        "org.apache.lucene.index.TestLongPostings",
        vec![
            test("testLongPostings", test_long_postings),
            test(
                "testLongPostingsNoPositions",
                test_long_postings_no_positions,
            ),
        ],
        |b| {
            b.annotate(SuppressCodecs {
                value: vec!["SimpleText".to_string(), "Direct".to_string()],
            })
        },
    )
}

/// Produces a realistic unicode random string that
/// survives MockAnalyzer unchanged:
///
/// Port of `TestLongPostings.getRandomTerm` (`private`).
fn get_random_term(other: Option<&str>) -> Result<String> {
    let a = MockAnalyzer::new(&*random());
    loop {
        let s = TestUtil::random_realistic_unicode_string(&*random());
        if other.is_some_and(|o| s == o) {
            continue;
        }
        let ts = a.token_stream_from_str("foo", &s)?;
        let mut ts = ts.borrow_mut();
        let outcome = (|| -> Result<bool> {
            let term_att = Att::get(ts.attribute_source(), term_to_bytes_ref_id())
                .unwrap_or_else(|| {
                    randomizedtesting::jdk::exception::Throwable::new(
                        "java.lang.IllegalArgumentException",
                        "this AttributeSource does not have the attribute 'TermToBytesRefAttribute'.",
                    )
                    .throw()
                });
            ts.reset()?;

            let mut count = 0;
            let mut changed = false;

            while ts.increment_token()? {
                let term_bytes = term_att.term_to_bytes_ref().get_bytes_ref();
                if count == 0 && term_bytes.utf8_to_string()? != s {
                    // The value was changed during analysis.  Keep iterating so the
                    // tokenStream is exhausted.
                    changed = true;
                }
                count += 1;
            }

            ts.end()?;
            // Did we iterate just once and the value was unchanged?
            Ok(!changed && count == 1)
        })();
        // try-with-resources
        let closed = ts.close();
        let accepted = outcome?;
        closed?;
        if accepted {
            return Ok(s);
        }
    }
}

/// `newIndexWriterConfig(new MockAnalyzer(random())).setOpenMode(CREATE)
/// .setMergePolicy(newLogMergePolicy())`, then the RAM buffer and
/// `setMaxBufferedDocs(-1)`.
fn long_postings_iwc() -> Result<IndexWriterConfig> {
    let iwc = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(MockAnalyzer::new(
        &*random(),
    ))));
    iwc.set_open_mode(OpenMode::CREATE)?;
    iwc.set_merge_policy(Some(Arc::new(LuceneTestCase::new_log_merge_policy())))?;
    iwc.set_ram_buffer_size_mb(16.0 + 16.0 * random().next_double())?;
    iwc.set_max_buffered_docs(-1)?;
    Ok(iwc)
}

/// The doc the next `nextDoc()`/`advance(target)` must land on, starting
/// the scan at `expected`.
fn expected_from(mut expected: i32, num_docs: i32, is_s1: &FixedBitSet, do_s1: bool) -> i32 {
    loop {
        if expected == num_docs {
            return i32::MAX;
        } else if is_s1.get(expected as usize) == do_s1 {
            return expected;
        } else {
            expected += 1;
        }
    }
}

/// Port of `TestLongPostings.testLongPostings`.
fn test_long_postings() -> Result<()> {
    // Don't use _TestUtil.getTempDir so that we own the
    // randomness (ie same seed will point to same dir):
    let mut dir = LuceneTestCase::new_fs_directory(&LuceneTestCase::create_temp_dir_prefix(
        &format!("longpostings.{}", random().next_long()),
    ));

    let num_docs = LuceneTestCase::at_least(1000);

    if verbose() {
        println!("TEST: NUM_DOCS={num_docs}");
    }

    let s1 = get_random_term(None)?;
    let s2 = get_random_term(Some(&s1))?;

    if verbose() {
        println!("\nTEST: s1={s1} s2={s2}");
    }

    let mut is_s1 = FixedBitSet::new(num_docs as usize);
    for idx in 0..num_docs as usize {
        if random().next_boolean() {
            is_s1.set(idx);
        }
    }

    let iwc = long_postings_iwc()?;
    let riw = RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), iwc)?;

    for idx in 0..num_docs as usize {
        let mut doc = Document::new();
        let s = if is_s1.get(idx) { &s1 } else { &s2 };
        let f: Arc<dyn IndexableField> =
            Arc::new(LuceneTestCase::new_text_field("field", s, Store::NO));
        let count = TestUtil::next_int(&*random(), 1, 4);
        for _ in 0..count {
            doc.add_shared(Arc::clone(&f));
        }
        riw.add_document(&doc)?;
    }

    let r: Arc<dyn IndexReader> = riw.get_reader()?;
    riw.close()?;

    assert_equals(&num_docs, &r.num_docs());
    Assert::assert_true(r.doc_freq(&Term::from_text("field", &s1))? > 0);
    Assert::assert_true(r.doc_freq(&Term::from_text("field", &s2))? > 0);

    let num = LuceneTestCase::at_least(1000);
    for iter in 0..num {
        let (term, do_s1) = if random().next_boolean() {
            (&s1, true)
        } else {
            (&s2, false)
        };

        if verbose() {
            println!("\nTEST: iter={iter} doS1={do_s1}");
        }

        let mut postings = MultiTerms::get_term_postings_enum(
            &r,
            "field",
            &BytesRef::from_char_sequence(term),
            POSTINGS_ENUM_ALL,
        )?
        .unwrap_or_else(|| npe("postings"));

        let mut doc_id = -1;
        while doc_id < NO_MORE_DOCS {
            let what = random().next_int_bound(3);
            if what == 0 {
                if verbose() {
                    println!("TEST: docID={doc_id}; do next()");
                }
                // nextDoc
                let expected = expected_from(doc_id + 1, num_docs, &is_s1, do_s1);
                doc_id = postings.next_doc()?;
                if verbose() {
                    println!("  got docID={doc_id}");
                }
                assert_equals(&expected, &doc_id);
                if doc_id == NO_MORE_DOCS {
                    break;
                }

                if random().next_int_bound(6) == 3 {
                    if verbose() {
                        println!("    check positions");
                    }
                    check_positions(&mut *postings)?;
                }
            } else {
                // advance
                let target_doc_id = if doc_id == -1 {
                    random().next_int_bound(num_docs + 1)
                } else {
                    doc_id + TestUtil::next_int(&*random(), 1, num_docs - doc_id)
                };
                if verbose() {
                    println!("TEST: docID={doc_id}; do advance({target_doc_id})");
                }
                let expected = expected_from(target_doc_id, num_docs, &is_s1, do_s1);

                doc_id = postings.advance(target_doc_id)?;
                if verbose() {
                    println!("  got docID={doc_id}");
                }
                assert_equals(&expected, &doc_id);
                if doc_id == NO_MORE_DOCS {
                    break;
                }

                if random().next_int_bound(6) == 3 {
                    check_positions(&mut *postings)?;
                }
            }
        }
    }
    r.close()?;
    dir.close()
}

/// The positions check both branches of `testLongPostings` make.
fn check_positions(postings: &mut dyn PostingsEnum) -> Result<()> {
    let freq = postings.freq()?;
    Assert::assert_true((1..=4).contains(&freq));
    for pos in 0..freq {
        assert_equals(&pos, &postings.next_position()?);
        if random().next_boolean() {
            postings.get_payload()?;
            if random().next_boolean() {
                postings.get_payload()?; // get it again
            }
        }
    }
    Ok(())
}

/// a weaker form of testLongPostings, that doesnt check positions
///
/// Port of `TestLongPostings.testLongPostingsNoPositions`.
fn test_long_postings_no_positions() -> Result<()> {
    do_test_long_postings_no_positions(IndexOptions::DOCS)?;
    do_test_long_postings_no_positions(IndexOptions::DOCS_AND_FREQS)
}

/// Port of `TestLongPostings.doTestLongPostingsNoPositions`.
fn do_test_long_postings_no_positions(options: IndexOptions) -> Result<()> {
    // Don't use _TestUtil.getTempDir so that we own the
    // randomness (ie same seed will point to same dir):
    let mut dir = LuceneTestCase::new_fs_directory(&LuceneTestCase::create_temp_dir_prefix(
        &format!("longpostings.{}", random().next_long()),
    ));

    let num_docs = LuceneTestCase::at_least(1000);

    if verbose() {
        println!("TEST: NUM_DOCS={num_docs}");
    }

    let s1 = get_random_term(None)?;
    let s2 = get_random_term(Some(&s1))?;

    if verbose() {
        println!("\nTEST: s1={s1} s2={s2}");
    }

    let mut is_s1 = FixedBitSet::new(num_docs as usize);
    for idx in 0..num_docs as usize {
        if random().next_boolean() {
            is_s1.set(idx);
        }
    }

    let r: Arc<dyn IndexReader> = {
        let iwc = long_postings_iwc()?;
        let riw = RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), iwc)?;

        let ft = FieldType::new_from(TextField::type_not_stored());
        ft.set_index_options(options)?;
        for idx in 0..num_docs as usize {
            let mut doc = Document::new();
            let s = if is_s1.get(idx) { &s1 } else { &s2 };
            let f: Arc<dyn IndexableField> = Arc::new(LuceneTestCase::new_field("field", s, &ft));
            let count = TestUtil::next_int(&*random(), 1, 4);
            for _ in 0..count {
                doc.add_shared(Arc::clone(&f));
            }
            riw.add_document(&doc)?;
        }

        let r = riw.get_reader()?;
        riw.close()?;
        r
    };

    assert_equals(&num_docs, &r.num_docs());
    Assert::assert_true(r.doc_freq(&Term::from_text("field", &s1))? > 0);
    Assert::assert_true(r.doc_freq(&Term::from_text("field", &s2))? > 0);

    let num = LuceneTestCase::at_least(1000);
    for iter in 0..num {
        let (term, do_s1) = if random().next_boolean() {
            (&s1, true)
        } else {
            (&s2, false)
        };

        if verbose() {
            println!("\nTEST: iter={iter} doS1={do_s1} term={term}");
        }

        // `docs` and `postings` are the same enum when the options carry
        // frequencies (`docs = postings = TestUtil.docs(…, FREQS)`).
        let has_postings = options != IndexOptions::DOCS;
        let flags = if has_postings {
            POSTINGS_ENUM_FREQS
        } else {
            POSTINGS_ENUM_NONE
        };
        let docs = TestUtil::docs(
            &*random(),
            &r,
            "field",
            &BytesRef::from_char_sequence(term),
            None,
            flags,
        )?;
        debug_assert!(docs.is_some());
        let mut docs = docs.unwrap_or_else(|| npe("docs"));

        let mut doc_id = -1;
        while doc_id < NO_MORE_DOCS {
            let what = random().next_int_bound(3);
            if what == 0 {
                if verbose() {
                    println!("TEST: docID={doc_id}; do next()");
                }
                // nextDoc
                let expected = expected_from(doc_id + 1, num_docs, &is_s1, do_s1);
                doc_id = docs.next_doc()?;
                if verbose() {
                    println!("  got docID={doc_id}");
                }
                assert_equals(&expected, &doc_id);
                if doc_id == NO_MORE_DOCS {
                    break;
                }

                if random().next_int_bound(6) == 3 && has_postings {
                    let freq = docs.freq()?;
                    Assert::assert_true((1..=4).contains(&freq));
                }
            } else {
                // advance
                let target_doc_id = if doc_id == -1 {
                    random().next_int_bound(num_docs + 1)
                } else {
                    doc_id + TestUtil::next_int(&*random(), 1, num_docs - doc_id)
                };
                if verbose() {
                    println!("TEST: docID={doc_id}; do advance({target_doc_id})");
                }
                let expected = expected_from(target_doc_id, num_docs, &is_s1, do_s1);

                doc_id = docs.advance(target_doc_id)?;
                if verbose() {
                    println!("  got docID={doc_id}");
                }
                assert_equals(&expected, &doc_id);
                if doc_id == NO_MORE_DOCS {
                    break;
                }

                if random().next_int_bound(6) == 3 && has_postings {
                    let freq = docs.freq()?;
                    Assert::assert_true_msg(
                        &format!("got invalid freq={freq}"),
                        (1..=4).contains(&freq),
                    );
                }
            }
        }
    }
    r.close()?;
    dir.close()
}
