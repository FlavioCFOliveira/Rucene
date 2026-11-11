//! Port of `lucene/core/src/test/org/apache/lucene/index/TestPostingsOffsets.java`.
//!
//! TODO: we really need to test indexingoffsets, but then getting only docs /
//! docs + freqs. not all codecs store prx separate...
//! TODO: fix sep codec to index offsets so we can greatly reduce this list!

use std::collections::HashMap;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::{Class, Constructor, Method};
use randomizedtesting::junit::Assert;
use rucene::analysis::tokenattributes::{
    CharTermAttribute, OffsetAttribute, PayloadAttribute, PositionIncrementAttribute, TypeAttribute,
};
use rucene::analysis::{
    Analyzer, AnalyzerState, GlobalReuseStrategy, TokenStream, TokenStreamComponents,
};
use rucene::document::{
    Document, Field, FieldType, NumericDocValuesField, Store, StringField, TextField,
};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::doc_values::DocValues;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::multi_fields::MultiTerms;
use rucene::index::postings_enum::{PostingsEnum, POSTINGS_ENUM_ALL, POSTINGS_ENUM_FREQS};
use rucene::index::{IndexOptions, IndexReader};
use rucene::search::NO_MORE_DOCS;
use rucene::util::atomic_ref_cell::AtomicRefCell;
use rucene::util::{BytesRef, IOUtils};
use rucene_test_framework::analysis::mock_tokenizer::{MockTokenizer, KEYWORD};
use rucene_test_framework::analysis::{
    tokenizer_components, CannedTokenStream, MockAnalyzer, MockPayloadAnalyzer, Token,
};
use rucene_test_framework::index::RandomIndexWriter;
use rucene_test_framework::util::java_exceptions::{expect_throws, rethrow_java};
use rucene_test_framework::util::{English, LuceneTestCase, TestUtil};

use crate::core_support::{assert_equals, assert_not_null, random, IAE};

/// The instance field of `TestPostingsOffsets`.
#[derive(Default)]
struct TestPostingsOffsetsInstance {
    /// `IndexWriterConfig iwc`. A writer takes its config, as a Java config
    /// cannot serve two writers either.
    iwc: Option<IndexWriterConfig>,
}

impl TestPostingsOffsetsInstance {
    /// `iwc`, handed to the writer being built.
    fn take_iwc(&mut self) -> IndexWriterConfig {
        self.iwc
            .take()
            .unwrap_or_else(|| crate::support::npe("iwc"))
    }
}

/// A test method over the instance.
fn instance_test(name: &str, body: fn(&mut TestPostingsOffsetsInstance) -> Result<()>) -> Method {
    Method::new(name).throws(&["java.lang.Exception"]).body(
        move |inst: &mut TestPostingsOffsetsInstance| {
            body(inst).unwrap_or_else(|e| rethrow_java(e))
        },
    )
}

/// `TestPostingsOffsets`.
pub fn class_def() -> Arc<Class> {
    Class::builder("org.apache.lucene.index.TestPostingsOffsets")
        .extends(&LuceneTestCase::class())
        .constructor(Constructor::no_args(TestPostingsOffsetsInstance::default))
        .method(instance_test("setUp", set_up))
        .method(instance_test("testBasic", test_basic))
        .method(instance_test("testSkipping", test_skipping))
        .method(instance_test("testPayloads", test_payloads))
        .method(instance_test("testRandom", test_random))
        .method(instance_test("testAddFieldTwice", test_add_field_twice))
        .method(instance_test("testNegativeOffsets", test_negative_offsets))
        .method(instance_test("testIllegalOffsets", test_illegal_offsets))
        .method(instance_test(
            "testIllegalOffsetsAcrossFieldInstances",
            test_illegal_offsets_across_field_instances,
        ))
        .method(instance_test(
            "testBackwardsOffsets",
            test_backwards_offsets,
        ))
        .method(instance_test("testStackedTokens", test_stacked_tokens))
        .method(instance_test("testCrazyOffsetGap", test_crazy_offset_gap))
        .method(instance_test(
            "testLegalbutVeryLargeOffsets",
            test_legalbut_very_large_offsets,
        ))
        .build()
}

/// `public void setUp()`.
fn set_up(inst: &mut TestPostingsOffsetsInstance) -> Result<()> {
    LuceneTestCase::super_set_up();
    inst.iwc = Some(LuceneTestCase::new_index_writer_config_analyzer(Some(
        Arc::new(MockAnalyzer::new(&*random())),
    )));
    Ok(())
}

/// `new FieldType(base)` with
/// `setIndexOptions(DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS)`.
fn offsets_type(base: &FieldType) -> Result<FieldType> {
    let ft = FieldType::new_from(base);
    ft.set_index_options(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS)?;
    Ok(ft)
}

/// `new Field(name, new CannedTokenStream(tokens), ft)`.
fn canned_field(name: &str, tokens: Vec<Token>, ft: &FieldType) -> Result<Field> {
    let stream: Arc<AtomicRefCell<dyn TokenStream>> =
        Arc::new(AtomicRefCell::new(CannedTokenStream::new(tokens)));
    Field::new_with_token_stream(name, stream, FieldType::new_from(ft))
}

/// `MultiTerms.getTermPostingsEnum(r, field, new BytesRef(term))`.
fn term_postings(
    r: &Arc<dyn IndexReader>,
    field: &str,
    term: &str,
) -> Result<Option<Box<dyn PostingsEnum>>> {
    MultiTerms::get_term_postings_enum_all(r, field, &BytesRef::new(term.as_bytes().to_vec()))
}

/// Port of `TestPostingsOffsets.testBasic`.
fn test_basic(inst: &mut TestPostingsOffsetsInstance) -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();

    let w = RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), inst.take_iwc())?;
    let mut doc = Document::new();

    let ft = offsets_type(TextField::type_not_stored())?;
    if random().next_boolean() {
        ft.set_store_term_vectors(true)?;
        ft.set_store_term_vector_positions(random().next_boolean())?;
        ft.set_store_term_vector_offsets(random().next_boolean())?;
    }
    let tokens = vec![
        make_token("a", 1, 0, 6),
        make_token("b", 1, 8, 9),
        make_token("a", 1, 9, 17),
        make_token("c", 1, 19, 50),
    ];
    doc.add(Box::new(canned_field("content", tokens, &ft)?));

    w.add_document(&doc)?;
    let r: Arc<dyn IndexReader> = w.get_reader()?;
    w.close()?;

    let dp = term_postings(&r, "content", "a")?;
    assert_not_null(&dp);
    let mut dp = dp.expect("checked by assertNotNull");
    assert_equals(&0, &dp.next_doc()?);
    assert_equals(&2, &dp.freq()?);
    assert_equals(&0, &dp.next_position()?);
    assert_equals(&0, &dp.start_offset());
    assert_equals(&6, &dp.end_offset());
    assert_equals(&2, &dp.next_position()?);
    assert_equals(&9, &dp.start_offset());
    assert_equals(&17, &dp.end_offset());
    assert_equals(&NO_MORE_DOCS, &dp.next_doc()?);

    let dp = term_postings(&r, "content", "b")?;
    assert_not_null(&dp);
    let mut dp = dp.expect("checked by assertNotNull");
    assert_equals(&0, &dp.next_doc()?);
    assert_equals(&1, &dp.freq()?);
    assert_equals(&1, &dp.next_position()?);
    assert_equals(&8, &dp.start_offset());
    assert_equals(&9, &dp.end_offset());
    assert_equals(&NO_MORE_DOCS, &dp.next_doc()?);

    let dp = term_postings(&r, "content", "c")?;
    assert_not_null(&dp);
    let mut dp = dp.expect("checked by assertNotNull");
    assert_equals(&0, &dp.next_doc()?);
    assert_equals(&1, &dp.freq()?);
    assert_equals(&3, &dp.next_position()?);
    assert_equals(&19, &dp.start_offset());
    assert_equals(&50, &dp.end_offset());
    assert_equals(&NO_MORE_DOCS, &dp.next_doc()?);

    r.close()?;
    dir.close()
}

/// Port of `TestPostingsOffsets.testSkipping`.
fn test_skipping(inst: &mut TestPostingsOffsetsInstance) -> Result<()> {
    do_test_numbers(inst, false)
}

/// Port of `TestPostingsOffsets.testPayloads`.
fn test_payloads(inst: &mut TestPostingsOffsetsInstance) -> Result<()> {
    do_test_numbers(inst, true)
}

/// The checks `doTestNumbers` makes over each position of `dp` in `doc`.
fn check_positions(
    dp: &mut dyn PostingsEnum,
    stored_numbers: &str,
    term: &str,
    with_payloads: bool,
) -> Result<()> {
    dp.next_position()?;
    let start = dp.start_offset();
    debug_assert!(start >= 0);
    let end = dp.end_offset();
    debug_assert!(end >= 0 && end >= start);
    // check that the offsets correspond to the term in the src text
    Assert::assert_true(java_substring(stored_numbers, start, end) == term);
    if with_payloads {
        // check that we have a payload and it starts with "pos"
        assert_not_null(&dp.get_payload()?);
        let payload = dp.get_payload()?.expect("checked by assertNotNull");
        Assert::assert_true(
            BytesRef::new(payload.to_vec())
                .utf8_to_string()?
                .starts_with("pos:"),
        );
    } // note: withPayloads=false doesnt necessarily mean we dont have them from MockAnalyzer!
    Ok(())
}

/// `s.substring(start, end)` over UTF-16 units.
fn java_substring(s: &str, start: i32, end: i32) -> String {
    let units: Vec<u16> = s.encode_utf16().collect();
    String::from_utf16_lossy(&units[start as usize..end as usize])
}

/// `public void doTestNumbers(boolean withPayloads)`.
fn do_test_numbers(inst: &mut TestPostingsOffsetsInstance, with_payloads: bool) -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let analyzer: Arc<dyn Analyzer> = if with_payloads {
        Arc::new(MockPayloadAnalyzer::new())
    } else {
        Arc::new(MockAnalyzer::new(&*random()))
    };
    let iwc = LuceneTestCase::new_index_writer_config_analyzer(Some(analyzer));
    iwc.set_merge_policy(Some(Arc::new(LuceneTestCase::new_log_merge_policy())))?; // will rely on docids a bit for skipping
    inst.iwc = Some(iwc);
    let w = RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), inst.take_iwc())?;

    let ft = offsets_type(TextField::type_stored())?;
    if random().next_boolean() {
        ft.set_store_term_vectors(true)?;
        ft.set_store_term_vector_offsets(random().next_boolean())?;
        ft.set_store_term_vector_positions(random().next_boolean())?;
    }

    let num_docs = LuceneTestCase::at_least(500);
    for i in 0..num_docs {
        let mut doc = Document::new();
        doc.add(Box::new(Field::new(
            "numbers",
            English::int_to_english(i),
            FieldType::new_from(&ft),
        )?));
        doc.add(Box::new(Field::new(
            "oddeven",
            if i % 2 == 0 { "even" } else { "odd" }.to_string(),
            FieldType::new_from(&ft),
        )?));
        doc.add(Box::new(StringField::new("id", i.to_string(), Store::NO)?));
        w.add_document(&doc)?;
    }

    let reader: Arc<dyn IndexReader> = w.get_reader()?;
    w.close()?;

    let terms = [
        "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten", "hundred",
    ];

    for term in terms {
        let mut dp =
            term_postings(&reader, "numbers", term)?.unwrap_or_else(|| crate::support::npe("dp"));
        let stored_fields = reader.stored_fields()?;
        loop {
            let doc = dp.next_doc()?;
            if doc == NO_MORE_DOCS {
                break;
            }
            let stored_numbers = stored_fields
                .document(doc)?
                .get("numbers")
                .unwrap_or_else(|| crate::support::npe("storedNumbers"));
            let freq = dp.freq()?;
            for _i in 0..freq {
                check_positions(&mut *dp, &stored_numbers, term, with_payloads)?;
            }
        }
    }

    // check we can skip correctly
    let num_skipping_tests = LuceneTestCase::at_least(50);

    for _j in 0..num_skipping_tests {
        let num = TestUtil::next_int(&*random(), 100, (num_docs - 1).min(999));
        let mut dp = term_postings(&reader, "numbers", "hundred")?
            .unwrap_or_else(|| crate::support::npe("dp"));
        let stored_fields = reader.stored_fields()?;
        let doc = dp.advance(num)?;
        assert_equals(&num, &doc);
        let freq = dp.freq()?;
        for _i in 0..freq {
            let stored_numbers = stored_fields
                .document(doc)?
                .get("numbers")
                .unwrap_or_else(|| crate::support::npe("storedNumbers"));
            check_positions(&mut *dp, &stored_numbers, "hundred", with_payloads)?;
        }
    }

    // check that other fields (without offsets) work correctly

    for i in 0..num_docs {
        let mut dp = MultiTerms::get_term_postings_enum(
            &reader,
            "id",
            &BytesRef::new(i.to_string().into_bytes()),
            0,
        )?
        .unwrap_or_else(|| crate::support::npe("dp"));
        assert_equals(&i, &dp.next_doc()?);
        assert_equals(&NO_MORE_DOCS, &dp.next_doc()?);
    }

    reader.close()?;
    dir.close()
}

/// Port of `TestPostingsOffsets.testRandom`.
fn test_random(inst: &mut TestPostingsOffsetsInstance) -> Result<()> {
    // token -> docID -> tokens
    let mut actual_tokens: HashMap<String, HashMap<i32, Vec<Token>>> = HashMap::new();

    let mut dir = LuceneTestCase::new_directory();
    let w = RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), inst.take_iwc())?;

    let num_docs = LuceneTestCase::at_least(20);
    // final int numDocs = atLeast(5);

    let ft = FieldType::new_from(TextField::type_not_stored());

    // TODO: randomize what IndexOptions we use; also test
    // changing this up in one IW buffered segment...:
    ft.set_index_options(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS)?;
    if random().next_boolean() {
        ft.set_store_term_vectors(true)?;
        ft.set_store_term_vector_offsets(random().next_boolean())?;
        ft.set_store_term_vector_positions(random().next_boolean())?;
    }

    for doc_count in 0..num_docs {
        let mut doc = Document::new();
        doc.add(Box::new(NumericDocValuesField::new("id", doc_count as i64)));
        let mut tokens: Vec<Token> = Vec::new();
        let num_tokens = LuceneTestCase::at_least(100);
        // final int numTokens = atLeast(20);
        let mut pos = -1;
        let mut offset = 0;
        // System.out.println("doc id=" + docCount);
        for token_count in 0..num_tokens {
            let text = if random().next_boolean() {
                "a"
            } else if random().next_boolean() {
                "b"
            } else if random().next_boolean() {
                "c"
            } else {
                "d"
            };

            let mut pos_incr = if random().next_boolean() {
                1
            } else {
                random().next_int_bound(5)
            };
            if token_count == 0 && pos_incr == 0 {
                pos_incr = 1;
            }
            let off_incr = if random().next_boolean() {
                0
            } else {
                random().next_int_bound(5)
            };
            let token_offset = random().next_int_bound(5);

            let mut token = make_token(
                text,
                pos_incr,
                offset + off_incr,
                offset + off_incr + token_offset,
            );
            pos += pos_incr;
            // stuff abs position into type:
            // (Java sets it after sharing the token with both lists, which
            // see the change; the copies here are taken after it.)
            token.set_type(pos.to_string());
            actual_tokens
                .entry(text.to_string())
                .or_default()
                .entry(doc_count)
                .or_default()
                .push(token.clone());
            tokens.push(token);
            offset += off_incr + token_offset;
            // System.out.println("  " + token + " posIncr=" + token.getPositionIncrement() + " pos=" +
            // pos + " off=" + token.startOffset() + "/" + token.endOffset() + " (freq=" +
            // postingsByDoc.get(docCount).size() + ")");
        }
        doc.add(Box::new(canned_field("content", tokens, &ft)?));
        w.add_document(&doc)?;
    }
    let r = w.get_reader()?;
    w.close()?;

    let terms = ["a", "b", "c", "d"];
    for ctx in Arc::clone(&r).leaves()? {
        // TODO: improve this
        let sub = ctx.leaf_reader();
        // System.out.println("\nsub=" + sub);
        let mut terms_enum = sub
            .terms("content")?
            .unwrap_or_else(|| crate::support::npe("terms"))
            .iterator()?;
        let mut docs: Option<Box<dyn PostingsEnum>> = None;
        let mut docs_and_positions: Option<Box<dyn PostingsEnum>> = None;
        let mut doc_id_to_id = vec![0i32; sub.max_doc() as usize];
        let mut values = DocValues::get_numeric(&*sub, "id")?;
        for i in 0..sub.max_doc() {
            assert_equals(&i, &values.next_doc()?);
            doc_id_to_id[i as usize] = values.long_value()? as i32;
        }

        for term in terms {
            // System.out.println("  term=" + term);
            if terms_enum.seek_exact(&BytesRef::new(term.as_bytes().to_vec()))? {
                let d = terms_enum.postings(docs.take(), POSTINGS_ENUM_FREQS)?;
                assert_not_null(&d);
                let mut d = d.ok_or_else(rucene::error::LuceneError::null_pointer)?;
                // System.out.println("    doc/freq");
                loop {
                    let doc = d.next_doc()?;
                    if doc == NO_MORE_DOCS {
                        break;
                    }
                    let expected = actual_tokens
                        .get(term)
                        .and_then(|m| m.get(&doc_id_to_id[doc as usize]));
                    // System.out.println("      doc=" + docIDToID[doc] + " docID=" + doc + " " +
                    // expected.size() + " freq");
                    assert_not_null(&expected);
                    assert_equals(&(expected.expect("checked").len() as i32), &d.freq()?);
                }
                docs = Some(d);

                // explicitly exclude offsets here
                let dap = terms_enum.postings(docs_and_positions.take(), POSTINGS_ENUM_ALL)?;
                assert_not_null(&dap);
                let mut dap = dap.ok_or_else(rucene::error::LuceneError::null_pointer)?;
                // System.out.println("    doc/freq/pos");
                loop {
                    let doc = dap.next_doc()?;
                    if doc == NO_MORE_DOCS {
                        break;
                    }
                    let expected = actual_tokens
                        .get(term)
                        .and_then(|m| m.get(&doc_id_to_id[doc as usize]));
                    // System.out.println("      doc=" + docIDToID[doc] + " " + expected.size() + " freq");
                    assert_not_null(&expected);
                    let expected = expected.expect("checked");
                    assert_equals(&(expected.len() as i32), &dap.freq()?);
                    for token in expected {
                        let pos: i32 = token.type_value().parse().unwrap_or_else(|_| {
                            randomizedtesting::jdk::exception::Throwable::new(
                                "java.lang.NumberFormatException",
                                format!("For input string: \"{}\"", token.type_value()),
                            )
                            .throw()
                        });
                        // System.out.println("        pos=" + pos);
                        assert_equals(&pos, &dap.next_position()?);
                    }
                }

                // `termsEnum.postings(docsAndPositions, PostingsEnum.ALL)`
                // reuses that enum, so both names then refer to it.
                let dapo = terms_enum.postings(Some(dap), POSTINGS_ENUM_ALL)?;
                assert_not_null(&dapo);
                let mut dapo = dapo.ok_or_else(rucene::error::LuceneError::null_pointer)?;
                // System.out.println("    doc/freq/pos/offs");
                loop {
                    let doc = dapo.next_doc()?;
                    if doc == NO_MORE_DOCS {
                        break;
                    }
                    let expected = actual_tokens
                        .get(term)
                        .and_then(|m| m.get(&doc_id_to_id[doc as usize]));
                    // System.out.println("      doc=" + docIDToID[doc] + " " + expected.size() + " freq");
                    assert_not_null(&expected);
                    let expected = expected.expect("checked");
                    assert_equals(&(expected.len() as i32), &dapo.freq()?);
                    for token in expected {
                        let pos: i32 = token.type_value().parse().unwrap_or_else(|_| {
                            randomizedtesting::jdk::exception::Throwable::new(
                                "java.lang.NumberFormatException",
                                format!("For input string: \"{}\"", token.type_value()),
                            )
                            .throw()
                        });
                        // System.out.println("        pos=" + pos);
                        assert_equals(&pos, &dapo.next_position()?);
                        assert_equals(&token.start_offset(), &dapo.start_offset());
                        assert_equals(&token.end_offset(), &dapo.end_offset());
                    }
                }
                docs_and_positions = Some(dapo);
            }
        }
        // TODO: test advance:
    }
    r.close()?;
    dir.close()
}

/// Port of `TestPostingsOffsets.testAddFieldTwice`.
fn test_add_field_twice(_inst: &mut TestPostingsOffsetsInstance) -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let iw = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    let mut doc = Document::new();
    let custom_type3 = FieldType::new_from(TextField::type_stored());
    custom_type3.set_store_term_vectors(true)?;
    custom_type3.set_store_term_vector_positions(true)?;
    custom_type3.set_store_term_vector_offsets(true)?;
    custom_type3.set_index_options(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS)?;
    let custom_type3 = Arc::new(custom_type3);
    doc.add(Box::new(Field::new(
        "content3",
        "here is more content with aaa aaa aaa".to_string(),
        Arc::clone(&custom_type3),
    )?));
    doc.add(Box::new(Field::new(
        "content3",
        "here is more content with aaa aaa aaa".to_string(),
        Arc::clone(&custom_type3),
    )?));
    iw.add_document(&doc)?;
    iw.close()?;
    dir.close() // checkindex
}

// NOTE: the next two tests aren't that good as we need an EvilToken...

/// Port of `TestPostingsOffsets.testNegativeOffsets`.
fn test_negative_offsets(inst: &mut TestPostingsOffsetsInstance) -> Result<()> {
    expect_throws(IAE, || {
        check_tokens(inst, vec![make_token("foo", 1, -1, -1)])
    });
    Ok(())
}

/// Port of `TestPostingsOffsets.testIllegalOffsets`.
fn test_illegal_offsets(inst: &mut TestPostingsOffsetsInstance) -> Result<()> {
    expect_throws(IAE, || check_tokens(inst, vec![make_token("foo", 1, 1, 0)]));
    Ok(())
}

/// Port of `TestPostingsOffsets.testIllegalOffsetsAcrossFieldInstances`.
fn test_illegal_offsets_across_field_instances(
    inst: &mut TestPostingsOffsetsInstance,
) -> Result<()> {
    expect_throws(IAE, || {
        check_tokens2(
            inst,
            vec![make_token("use", 1, 150, 160)],
            vec![make_token("use", 1, 50, 60)],
        )
    });
    Ok(())
}

/// Port of `TestPostingsOffsets.testBackwardsOffsets`.
fn test_backwards_offsets(inst: &mut TestPostingsOffsetsInstance) -> Result<()> {
    expect_throws(IAE, || {
        check_tokens(
            inst,
            vec![
                make_token("foo", 1, 0, 3),
                make_token("foo", 1, 4, 7),
                make_token("foo", 0, 3, 6),
            ],
        )
    });
    Ok(())
}

/// Port of `TestPostingsOffsets.testStackedTokens`.
fn test_stacked_tokens(inst: &mut TestPostingsOffsetsInstance) -> Result<()> {
    check_tokens(
        inst,
        vec![
            make_token("foo", 1, 0, 3),
            make_token("foo", 0, 0, 3),
            make_token("foo", 0, 0, 3),
        ],
    )
}

/// The anonymous `Analyzer` of `testCrazyOffsetGap`: a keyword tokenizer and
/// an offset gap of -10.
#[derive(Debug)]
struct CrazyOffsetGapAnalyzer {
    state: AnalyzerState,
}

impl Analyzer for CrazyOffsetGapAnalyzer {
    fn analyzer_state(&self) -> &AnalyzerState {
        &self.state
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.index.TestPostingsOffsets$1"
    }

    fn create_components(&self, _field_name: &str) -> Result<TokenStreamComponents> {
        Ok(tokenizer_components(Arc::new(AtomicRefCell::new(
            MockTokenizer::new(Arc::clone(&KEYWORD), false)?,
        ))))
    }

    fn get_offset_gap(&self, _field_name: &str) -> i32 {
        -10
    }
}

/// Port of `TestPostingsOffsets.testCrazyOffsetGap`.
fn test_crazy_offset_gap(_inst: &mut TestPostingsOffsetsInstance) -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let analyzer: Arc<dyn Analyzer> = Arc::new(CrazyOffsetGapAnalyzer {
        state: AnalyzerState::new(Box::new(GlobalReuseStrategy)),
    });
    let iw = IndexWriter::new(
        dir.as_directory(),
        IndexWriterConfig::with_analyzer(Some(analyzer)),
    )?;
    // add good document
    let mut doc = Document::new();
    iw.add_document(&doc)?;
    expect_throws(IAE, || -> Result<()> {
        let ft = offsets_type(TextField::type_not_stored())?;
        let ft = Arc::new(ft);
        doc.add(Box::new(Field::new(
            "foo",
            "bar".to_string(),
            Arc::clone(&ft),
        )?));
        doc.add(Box::new(Field::new(
            "foo",
            "bar".to_string(),
            Arc::clone(&ft),
        )?));
        iw.add_document(&doc)?;
        Ok(())
    });
    iw.commit()?;
    iw.close()?;

    // make sure we see our good doc
    let r = directory_reader::open(dir.as_directory())?;
    assert_equals(&1, &r.num_docs());
    r.close()?;
    dir.close()
}

/// Port of `TestPostingsOffsets.testLegalbutVeryLargeOffsets`.
fn test_legalbut_very_large_offsets(_inst: &mut TestPostingsOffsetsInstance) -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let iw = IndexWriter::new(
        dir.as_directory(),
        LuceneTestCase::new_index_writer_config_analyzer(None),
    )?;
    let mut doc = Document::new();
    let mut t1 = Token::with_offsets("foo", 0, i32::MAX - 500);
    if random().next_boolean() {
        t1.set_payload(Some(BytesRef::new(b"test".to_vec())));
    }
    let t2 = Token::with_offsets("foo", i32::MAX - 500, i32::MAX);
    let ft = offsets_type(TextField::type_not_stored())?;
    // store some term vectors for the checkindex cross-check
    ft.set_store_term_vectors(true)?;
    ft.set_store_term_vector_positions(true)?;
    ft.set_store_term_vector_offsets(true)?;
    let field = canned_field("foo", vec![t1, t2], &ft)?;
    doc.add(Box::new(field));
    iw.add_document(&doc)?;
    iw.close()?;
    dir.close()
}

// TODO: more tests with other possibilities

/// `new FieldType(TextField.TYPE_NOT_STORED)` with offsets and term vectors
/// (positions and offsets), as `checkTokens` builds it.
fn check_tokens_type() -> Result<FieldType> {
    let ft = offsets_type(TextField::type_not_stored())?;
    // store some term vectors for the checkindex cross-check
    ft.set_store_term_vectors(true)?;
    ft.set_store_term_vector_positions(true)?;
    ft.set_store_term_vector_offsets(true)?;
    Ok(ft)
}

/// `private void checkTokens(Token[]... fields)`: one `body` field per token
/// array, in one document, then `riw.close()`; the directory is closed on
/// success, and `IOUtils.closeWhileHandlingException(riw, dir)` runs on
/// failure.
fn check_tokens_fields(
    inst: &mut TestPostingsOffsetsInstance,
    fields: Vec<Vec<Token>>,
) -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let riw = RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), inst.take_iwc())?;
    let result = (|| -> Result<()> {
        let ft = check_tokens_type()?;

        let mut doc = Document::new();
        for tokens in fields {
            doc.add(Box::new(canned_field("body", tokens, &ft)?));
        }
        riw.add_document(&doc)?;
        riw.close()?;
        Ok(())
    })();
    match result {
        Ok(()) => IOUtils::close([IOUtils::closeable(|| dir.close())]),
        Err(e) => {
            let _ = IOUtils::close_while_handling_exception([
                IOUtils::closeable(|| riw.close()),
                IOUtils::closeable(|| dir.close()),
            ]);
            Err(e)
        }
    }
}

/// `private void checkTokens(Token[] field1, Token[] field2)`.
fn check_tokens2(
    inst: &mut TestPostingsOffsetsInstance,
    field1: Vec<Token>,
    field2: Vec<Token>,
) -> Result<()> {
    check_tokens_fields(inst, vec![field1, field2])
}

/// `private void checkTokens(Token[] tokens)`.
fn check_tokens(inst: &mut TestPostingsOffsetsInstance, tokens: Vec<Token>) -> Result<()> {
    check_tokens_fields(inst, vec![tokens])
}

/// `private Token makeToken(String text, int posIncr, int startOffset, int endOffset)`.
fn make_token(text: &str, pos_incr: i32, start_offset: i32, end_offset: i32) -> Token {
    let mut t = Token::new();
    t.append_string(Some(text));
    t.set_position_increment(pos_incr);
    t.set_offset(start_offset, end_offset);
    t
}
