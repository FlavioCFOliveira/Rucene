//! Port of `lucene/core/src/test/org/apache/lucene/index/TestCodecs.java`.
//!
//! The test writes postings straight through the default codec's
//! `FieldsConsumer` from its own in-memory `Fields` (`DataFields`,
//! `DataTerms`, `DataTermsEnum`, `DataPostingsEnum`), then reads them back
//! with the codec's `FieldsProducer`.

use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::Arc;

use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::random::Random;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::jdk::thread::Thread;
use randomizedtesting::junit::{Assert, BeforeClass};
use rucene::codecs::norms::NormsProducer;
use rucene::codecs::postings::FieldsProducer;
use rucene::codecs::state::{SegmentReadState, SegmentWriteState};
use rucene::codecs::Codec;
use rucene::document::{Document, Store, StringField};
use rucene::error::{LuceneError, Result};
use rucene::index::doc_values::{DocValuesIterator, NumericDocValues};
use rucene::index::documents_writer::BufferedUpdates;
use rucene::index::index_writer::IndexWriter;
use rucene::index::postings_enum::{
    ImpactsEnum, PostingsEnum, POSTINGS_ENUM_ALL, POSTINGS_ENUM_FREQS, POSTINGS_ENUM_NONE,
};
use rucene::index::terms::{Fields, SeekStatus, Term, Terms, TermsEnum};
use rucene::index::{
    DocValuesSkipIndexType, DocValuesType, FieldInfo, FieldInfos, FieldInfosBuilder, FieldNumbers,
    IndexOptions, SegmentInfo, VectorEncoding, VectorSimilarityFunction,
};
use rucene::search::{DocIdSetIterator, NO_MORE_DOCS};
use rucene::store::Directory;
use rucene::util::attribute::AttributeSource;
use rucene::util::string_helper::StringHelper;
use rucene::util::{default_info_stream, BytesRef, Version};
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::util::java_exceptions::{is_instance, java_throwable};
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::{assert_equals, assert_equals_msg, class, random, test, verbose};
use crate::support::{open_reader, static_hook};

/// `TestCodecs`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestCodecs",
        vec![
            static_hook("beforeClass", BeforeClass, before_class),
            test("testFixedPostings", test_fixed_postings),
            test("testRandomPostings", test_random_postings),
            test("testDocsOnlyFreq", test_docs_only_freq),
        ],
    )
}

const FIELD_NAMES: [&str; 4] = ["one", "two", "three", "four"];

static NUM_TEST_ITER: AtomicI32 = AtomicI32::new(0);
const NUM_TEST_THREADS: usize = 3;
const NUM_FIELDS: usize = 4;
/// must be > 16 to test skipping
const NUM_TERMS_RAND: i32 = 50;
/// must be > 16 to test skipping
const DOC_FREQ_RAND: i32 = 500;
const TERM_DOC_FREQ_RAND: i32 = 20;

/// `@BeforeClass public static void beforeClass()`.
fn before_class() -> Result<()> {
    NUM_TEST_ITER.store(LuceneTestCase::at_least(20), Ordering::SeqCst);
    Ok(())
}

/// `static class FieldData implements Comparable<FieldData>`.
///
/// The term array is shared (`Arc`) between the test, the `DataFields` the
/// consumer writes from and the verifying threads, as Java shares the
/// array object.
#[derive(Clone)]
struct FieldData {
    field_info: FieldInfo,
    terms: Arc<Vec<TermData>>,
    omit_tf: bool,
    /// Java's `storePayloads`, which no member reads.
    #[allow(dead_code)]
    store_payloads: bool,
}

impl FieldData {
    /// `new FieldData(name, fieldInfos, terms, omitTF, storePayloads)`.
    ///
    /// Java also sets each `terms[i].field = this`, a back reference no
    /// member reads; the port keeps no such reference.
    fn new(
        name: &str,
        field_infos: &mut FieldInfosBuilder,
        mut terms: Vec<TermData>,
        omit_tf: bool,
        store_payloads: bool,
    ) -> Result<Self> {
        // TODO: change this test to use all three
        let field_info = match field_infos.field_info(name) {
            Some(field_info0) => field_info0.clone(),
            None => {
                let index_options = if omit_tf {
                    IndexOptions::DOCS
                } else {
                    IndexOptions::DOCS_AND_FREQS_AND_POSITIONS
                };
                field_infos
                    .add(&FieldInfo::new_full(
                        name,
                        -1,
                        false,
                        false,
                        store_payloads,
                        index_options,
                        DocValuesType::NONE,
                        DocValuesSkipIndexType::NONE,
                        -1,
                        Default::default(),
                        0,
                        0,
                        0,
                        0,
                        VectorEncoding::FLOAT32,
                        VectorSimilarityFunction::EUCLIDEAN,
                        false,
                        false,
                    )?)?
                    .clone()
            }
        };
        // `Arrays.sort(terms)`.
        terms.sort_by(|a, b| a.text.cmp(&b.text));
        Ok(FieldData {
            field_info,
            terms: Arc::new(terms),
            omit_tf,
            store_payloads,
        })
    }
}

/// `static class PositionData`.
struct PositionData {
    pos: i32,
    payload: Option<BytesRef>,
}

/// `static class TermData implements Comparable<TermData>`.
struct TermData {
    text2: String,
    text: BytesRef,
    docs: Vec<i32>,
    positions: Option<Vec<Vec<PositionData>>>,
}

impl TermData {
    fn new(text: String, docs: Vec<i32>, positions: Option<Vec<Vec<PositionData>>>) -> Self {
        TermData {
            text: BytesRef::from_char_sequence(&text),
            text2: text,
            docs,
            positions,
        }
    }

    /// `positions[i]`: Java dereferences the array, null when `omitTF`.
    fn positions(&self, i: usize) -> &[PositionData] {
        match &self.positions {
            Some(p) => &p[i],
            None => crate::support::npe("TermData.positions"),
        }
    }
}

const SEGMENT: &str = "0";

/// `Integer.toString(i, Character.MAX_RADIX)` of a non-negative `int`.
fn to_string_radix36(mut value: i32) -> String {
    if value == 0 {
        return "0".to_string();
    }
    let mut digits = Vec::new();
    while value > 0 {
        let d = (value % 36) as u32;
        digits.push(std::char::from_digit(d, 36).expect("INVARIANT: d < 36"));
        value /= 36;
    }
    digits.iter().rev().collect()
}

/// `TermData[] makeRandomTerms(final boolean omitTF, final boolean storePayloads)`.
fn make_random_terms(omit_tf: bool, store_payloads: bool) -> Vec<TermData> {
    let random = random();
    let num_terms = 1 + random.next_int_bound(NUM_TERMS_RAND);
    // final int numTerms = 2;
    let mut terms = Vec::with_capacity(num_terms as usize);

    let mut terms_seen = std::collections::HashSet::new();

    for _ in 0..num_terms {
        // Make term text
        let text2 = loop {
            let text2 = TestUtil::random_unicode_string(&*random);
            if !terms_seen.contains(&text2) && !text2.ends_with('.') {
                terms_seen.insert(text2.clone());
                break text2;
            }
        };

        let doc_freq = 1 + random.next_int_bound(DOC_FREQ_RAND);
        let mut docs = vec![0i32; doc_freq as usize];
        let mut positions: Option<Vec<Vec<PositionData>>> = if !omit_tf {
            Some(Vec::with_capacity(doc_freq as usize))
        } else {
            None
        };

        let mut doc_id = 0;
        for j in 0..doc_freq as usize {
            doc_id += TestUtil::next_int(&*random, 1, 10);
            docs[j] = doc_id;

            if let Some(positions) = positions.as_mut() {
                let term_freq = 1 + random.next_int_bound(TERM_DOC_FREQ_RAND);
                let mut doc_positions = Vec::with_capacity(term_freq as usize);
                let mut position = 0;
                for _ in 0..term_freq {
                    position += TestUtil::next_int(&*random, 1, 10);

                    let payload = if store_payloads && random.next_int_bound(4) == 0 {
                        let mut bytes = vec![0u8; (1 + random.next_int_bound(5)) as usize];
                        for b in bytes.iter_mut() {
                            *b = random.next_int_bound(255) as u8;
                        }
                        Some(BytesRef::new(bytes))
                    } else {
                        None
                    };

                    doc_positions.push(PositionData {
                        pos: position,
                        payload,
                    });
                }
                positions.push(doc_positions);
            }
        }

        terms.push(TermData::new(text2, docs, positions));
    }

    terms
}

/// `new SegmentInfo(dir, Version.LATEST, Version.LATEST, SEGMENT, 10000,
/// false, false, codec, Collections.emptyMap(), StringHelper.randomId(), new
/// HashMap<>(), null)`.
fn new_segment_info(dir: Arc<dyn Directory>, codec: Arc<dyn Codec>) -> Result<SegmentInfo> {
    SegmentInfo::new(
        dir,
        Version::LATEST,
        Some(Version::LATEST),
        SEGMENT.to_string(),
        10000,
        false,
        false,
        codec,
        std::collections::HashMap::new(),
        StringHelper::random_id(),
        std::collections::HashMap::new(),
        None,
    )
}

/// `Codec.getDefault()`.
fn default_codec() -> Arc<dyn Codec> {
    rucene::codecs::get_default().unwrap_or_else(|| {
        Throwable::new(
            "java.lang.IllegalStateException",
            "no default codec registered",
        )
        .throw()
    })
}

/// `codec.postingsFormat().fieldsProducer(new SegmentReadState(dir, si,
/// fieldInfos, newIOContext(random())))`.
fn open_producer(
    codec: &Arc<dyn Codec>,
    dir: &Arc<dyn Directory>,
    si: &SegmentInfo,
    field_infos: &FieldInfos,
) -> Result<Box<dyn FieldsProducer>> {
    let context = LuceneTestCase::new_io_context(&*random());
    codec
        .postings_format()
        .fields_producer(&SegmentReadState::new(&**dir, si, field_infos, &*context))
}

/// Port of `TestCodecs.testFixedPostings`.
fn test_fixed_postings() -> Result<()> {
    const NUM_TERMS: i32 = 100;
    let mut terms = Vec::with_capacity(NUM_TERMS as usize);
    for i in 0..NUM_TERMS {
        let docs = vec![i];
        let text = to_string_radix36(i);
        terms.push(TermData::new(text, docs, None));
    }

    let mut builder = FieldInfosBuilder::new(Arc::new(FieldNumbers::new(None, None)?));

    let field = FieldData::new("field", &mut builder, terms, true, false)?;
    // `FieldData` sorted the array in place: Java's `terms` is that array.
    let terms = Arc::clone(&field.terms);
    let mut fields = vec![field];
    let field_infos = builder.finish()?;
    let test_dir = LuceneTestCase::new_directory();
    let dir = test_dir.as_directory();
    let codec = default_codec();
    let si = new_segment_info(Arc::clone(&dir), Arc::clone(&codec))?;

    write(&si, &field_infos, &dir, &mut fields)?;
    let reader = open_producer(&codec, &dir, &si, &field_infos)?;

    {
        let mut fields_enum = reader.iterator();
        let field_name = fields_enum.next();
        crate::core_support::assert_not_null(&field_name);
        let field_name = field_name.expect("checked by assertNotNull");
        let terms2 = reader.terms(&field_name)?;
        crate::core_support::assert_not_null(&terms2);
        let terms2 = terms2.expect("checked by assertNotNull");

        let mut terms_enum = terms2.iterator()?;

        let mut postings_enum: Option<Box<dyn PostingsEnum>> = None;
        for i in 0..NUM_TERMS as usize {
            let term = terms_enum.next()?;
            crate::core_support::assert_not_null(&term);
            let term = term.expect("checked by assertNotNull");
            assert_equals(&terms[i].text2, &term.utf8_to_string()?);

            // do this twice to stress test the codec's reuse, ie,
            // make sure it properly fully resets (rewinds) its
            // internal state:
            for _iter in 0..2 {
                let mut pe = TestUtil::docs_of(
                    &*random(),
                    &mut *terms_enum,
                    postings_enum.take(),
                    POSTINGS_ENUM_NONE,
                )?
                .ok_or_else(rucene::error::LuceneError::null_pointer)?;
                assert_equals(&terms[i].docs[0], &pe.next_doc()?);
                assert_equals(&NO_MORE_DOCS, &pe.next_doc()?);
                postings_enum = Some(pe);
            }
        }
        crate::core_support::assert_null(&terms_enum.next()?);

        for i in 0..NUM_TERMS as usize {
            assert_equals(
                &terms_enum.seek_ceil(&BytesRef::from_char_sequence(&terms[i].text2))?,
                &SeekStatus::FOUND,
            );
        }

        Assert::assert_false(fields_enum.next().is_some());
    }
    reader.close()?;
    test_dir.close()?;
    Ok(())
}

/// Port of `TestCodecs.testRandomPostings`.
fn test_random_postings() -> Result<()> {
    let mut builder = FieldInfosBuilder::new(Arc::new(FieldNumbers::new(None, None)?));

    let mut fields = Vec::with_capacity(NUM_FIELDS);
    for i in 0..NUM_FIELDS {
        let omit_tf = 0 == (i % 3);
        let store_payloads = 1 == (i % 3);
        fields.push(FieldData::new(
            FIELD_NAMES[i],
            &mut builder,
            make_random_terms(omit_tf, store_payloads),
            omit_tf,
            store_payloads,
        )?);
    }

    let test_dir = LuceneTestCase::new_directory();
    let dir = test_dir.as_directory();
    let field_infos = builder.finish()?;

    if verbose() {
        println!("TEST: now write postings");
    }

    let codec = default_codec();
    let si = new_segment_info(Arc::clone(&dir), Arc::clone(&codec))?;
    write(&si, &field_infos, &dir, &mut fields)?;

    if verbose() {
        println!("TEST: now read postings");
    }
    let terms: Arc<Box<dyn FieldsProducer>> =
        Arc::new(open_producer(&codec, &dir, &si, &field_infos)?);
    let fields = Arc::new(fields);

    let mut threads = Vec::with_capacity(NUM_TEST_THREADS - 1);
    for _ in 0..NUM_TEST_THREADS - 1 {
        let verify = Arc::new(Verify::new(&si, Arc::clone(&fields), Arc::clone(&terms)));
        let runner = Arc::clone(&verify);
        let thread = Thread::unnamed(move || runner.run());
        thread.set_daemon(true);
        thread.start();
        threads.push((thread, verify));
    }

    Verify::new(&si, Arc::clone(&fields), Arc::clone(&terms)).run();

    for (thread, verify) in &threads {
        if let Err(payload) = thread.join() {
            std::panic::resume_unwind(payload);
        }
        debug_assert!(!verify.failed.load(Ordering::SeqCst));
    }
    drop(threads);

    let terms = match Arc::try_unwrap(terms) {
        Ok(terms) => terms,
        Err(_) => Throwable::new(
            "java.lang.IllegalStateException",
            "the verifying threads still hold the FieldsProducer",
        )
        .throw(),
    };
    terms.close()?;
    test_dir.close()?;
    Ok(())
}

/// `private static class Verify extends Thread`.
struct Verify {
    terms_dict: Arc<Box<dyn FieldsProducer>>,
    fields: Arc<Vec<FieldData>>,
    failed: AtomicBool,
}

impl Verify {
    /// `Verify(final SegmentInfo si, final FieldData[] fields, final Fields
    /// termsDict)`.
    fn new(
        _si: &SegmentInfo,
        fields: Arc<Vec<FieldData>>,
        terms_dict: Arc<Box<dyn FieldsProducer>>,
    ) -> Self {
        Verify {
            fields,
            terms_dict,
            failed: AtomicBool::new(false),
        }
    }

    /// `public void run()`.
    fn run(&self) {
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.run_()));
        let t = match outcome {
            Ok(Ok(())) => return,
            Ok(Err(e)) => java_throwable(&e),
            Err(payload) => Throwable::from_panic(payload),
        };
        self.failed.store(true, Ordering::SeqCst);
        Throwable::from_cause("java.lang.RuntimeException", t).throw()
    }

    fn verify_docs(
        &self,
        docs: &[i32],
        term: &TermData,
        postings_enum: &mut dyn PostingsEnum,
        do_pos: bool,
    ) -> Result<()> {
        for i in 0..docs.len() {
            let doc = postings_enum.next_doc()?;
            Assert::assert_true(doc != NO_MORE_DOCS);
            assert_equals(&docs[i], &doc);
            if do_pos {
                self.verify_positions(term.positions(i), postings_enum)?;
            }
        }
        assert_equals(&NO_MORE_DOCS, &postings_enum.next_doc()?);
        Ok(())
    }

    fn verify_positions(
        &self,
        positions: &[PositionData],
        pos_enum: &mut dyn PostingsEnum,
    ) -> Result<()> {
        for position in positions {
            let pos = pos_enum.next_position()?;
            assert_equals(&position.pos, &pos);
            if let Some(payload) = &position.payload {
                crate::core_support::assert_not_null(&pos_enum.get_payload()?);
                if random().next_int_bound(3) < 2 {
                    // Verify the payload bytes
                    let other_payload = BytesRef::new(
                        pos_enum
                            .get_payload()?
                            .expect("checked by assertNotNull")
                            .to_vec(),
                    );
                    Assert::assert_true_msg(
                        &format!("expected={payload} got={other_payload}"),
                        *payload == other_payload,
                    );
                }
            } else {
                crate::core_support::assert_null(&pos_enum.get_payload()?.map(|p| p.to_vec()));
            }
        }
        Ok(())
    }

    /// `termsEnum.seekExact(ord)` inside `try { … } catch
    /// (UnsupportedOperationException uoe)`: whether it succeeded.
    fn seek_ord_or_unsupported(terms_enum: &mut dyn TermsEnum, ord: i64) -> Result<bool> {
        match terms_enum.seek_ord(ord) {
            Ok(()) => Ok(true),
            Err(e) if is_instance(&e, "java.lang.UnsupportedOperationException") => Ok(false),
            Err(e) => Err(e),
        }
    }

    /// `public void _run()`.
    fn run_(&self) -> Result<()> {
        let random = random();
        for _iter in 0..NUM_TEST_ITER.load(Ordering::SeqCst) {
            let field = &self.fields[random.next_int_bound(self.fields.len() as i32) as usize];
            let mut terms_enum = self
                .terms_dict
                .terms(field.field_info.get_name())?
                .unwrap_or_else(|| crate::support::npe("termsDict.terms(field)"))
                .iterator()?;

            let mut upto = 0usize;
            // Test straight enum of the terms:
            loop {
                let Some(term) = terms_enum.next()? else {
                    break;
                };
                let expected = BytesRef::from_char_sequence(&field.terms[upto].text2);
                upto += 1;
                Assert::assert_true_msg(
                    &format!("expected={expected} vs actual {term}"),
                    expected.bytes_equals(&term),
                );
            }
            assert_equals(&upto, &field.terms.len());

            // Test random seek:
            let mut term = &field.terms[random.next_int_bound(field.terms.len() as i32) as usize];
            let mut status = terms_enum.seek_ceil(&BytesRef::from_char_sequence(&term.text2))?;
            assert_equals(&status, &SeekStatus::FOUND);
            assert_equals(&(term.docs.len() as i32), &terms_enum.doc_freq()?);
            if field.omit_tf {
                let mut pe =
                    TestUtil::docs_of(&*random, &mut *terms_enum, None, POSTINGS_ENUM_NONE)?
                        .ok_or_else(rucene::error::LuceneError::null_pointer)?;
                self.verify_docs(&term.docs, term, &mut *pe, false)?;
            } else {
                let mut pe = terms_enum
                    .postings(None, POSTINGS_ENUM_ALL)?
                    .ok_or_else(rucene::error::LuceneError::null_pointer)?;
                self.verify_docs(&term.docs, term, &mut *pe, true)?;
            }

            // Test random seek by ord:
            let idx = random.next_int_bound(field.terms.len() as i32) as usize;
            term = &field.terms[idx];
            let success = Self::seek_ord_or_unsupported(&mut *terms_enum, idx as i64)?;
            if success {
                assert_equals(&status, &SeekStatus::FOUND);
                Assert::assert_true(
                    terms_enum
                        .term()?
                        .ok_or_else(rucene::error::LuceneError::null_pointer)?
                        .bytes_equals(&BytesRef::from_char_sequence(&term.text2)),
                );
                assert_equals(&(term.docs.len() as i32), &terms_enum.doc_freq()?);
                if field.omit_tf {
                    let mut pe =
                        TestUtil::docs_of(&*random, &mut *terms_enum, None, POSTINGS_ENUM_NONE)?
                            .ok_or_else(rucene::error::LuceneError::null_pointer)?;
                    self.verify_docs(&term.docs, term, &mut *pe, false)?;
                } else {
                    let mut pe = terms_enum
                        .postings(None, POSTINGS_ENUM_ALL)?
                        .ok_or_else(rucene::error::LuceneError::null_pointer)?;
                    self.verify_docs(&term.docs, term, &mut *pe, true)?;
                }
            }

            // Test seek to non-existent terms:
            if verbose() {
                println!("TEST: seek non-exist terms");
            }
            for _ in 0..100 {
                let text2 = TestUtil::random_unicode_string(&*random) + ".";
                status = terms_enum.seek_ceil(&BytesRef::from_char_sequence(&text2))?;
                Assert::assert_true(status == SeekStatus::NOT_FOUND || status == SeekStatus::END);
            }

            // Seek to each term, backwards:
            if verbose() {
                println!("TEST: seek terms backwards");
            }
            for i in (0..field.terms.len()).rev() {
                assert_equals_msg(
                    Some(&format!(
                        "{}: field={} term={}",
                        Thread::current().name(),
                        field.field_info.get_name(),
                        field.terms[i].text2
                    )),
                    &SeekStatus::FOUND,
                    &terms_enum.seek_ceil(&BytesRef::from_char_sequence(&field.terms[i].text2))?,
                );
                assert_equals(&(field.terms[i].docs.len() as i32), &terms_enum.doc_freq()?);
            }

            // Seek to each term by ord, backwards
            for i in (0..field.terms.len()).rev() {
                if Self::seek_ord_or_unsupported(&mut *terms_enum, i as i64)? {
                    assert_equals(&(field.terms[i].docs.len() as i32), &terms_enum.doc_freq()?);
                    Assert::assert_true(
                        terms_enum
                            .term()?
                            .ok_or_else(rucene::error::LuceneError::null_pointer)?
                            .bytes_equals(&BytesRef::from_char_sequence(&field.terms[i].text2)),
                    );
                }
            }

            // Seek to non-existent empty-string term
            status = terms_enum.seek_ceil(&BytesRef::from_char_sequence(""))?;
            crate::core_support::assert_not_null(&Some(status));
            // assertEquals(TermsEnum.SeekStatus.NOT_FOUND, status);

            // Make sure we're now pointing to first term
            Assert::assert_true(
                terms_enum
                    .term()?
                    .ok_or_else(rucene::error::LuceneError::null_pointer)?
                    .bytes_equals(&BytesRef::from_char_sequence(&field.terms[0].text2)),
            );

            // Test docs enum
            terms_enum.seek_ceil(&BytesRef::from_char_sequence(""))?;
            upto = 0;
            loop {
                term = &field.terms[upto];
                if random.next_int_bound(3) == 1 {
                    let mut postings = if !field.omit_tf {
                        // TODO: we should randomize which postings features are available, but
                        // need to coordinate this with the checks below that rely on such features
                        terms_enum
                            .postings(None, POSTINGS_ENUM_ALL)?
                            .ok_or_else(rucene::error::LuceneError::null_pointer)?
                    } else {
                        TestUtil::docs_of(&*random, &mut *terms_enum, None, POSTINGS_ENUM_FREQS)?
                            .ok_or_else(rucene::error::LuceneError::null_pointer)?
                    };
                    let docs = &term.docs;
                    let mut upto2: i32 = -1;
                    let mut ended = false;
                    while upto2 < docs.len() as i32 - 1 {
                        // Maybe skip:
                        let left = docs.len() as i32 - upto2;
                        let doc;
                        if random.next_int_bound(3) == 1 && left >= 1 {
                            let inc = 1 + random.next_int_bound(left - 1);
                            upto2 += inc;
                            if random.next_int_bound(2) == 1 {
                                doc = postings.advance(docs[upto2 as usize])?;
                                assert_equals(&docs[upto2 as usize], &doc);
                            } else {
                                doc = postings.advance(1 + docs[upto2 as usize])?;
                                if doc == NO_MORE_DOCS {
                                    // skipped past last doc
                                    debug_assert!(upto2 == docs.len() as i32 - 1);
                                    ended = true;
                                    break;
                                } else {
                                    // skipped to next doc
                                    debug_assert!(upto2 < docs.len() as i32 - 1);
                                    if doc >= docs[1 + upto2 as usize] {
                                        upto2 += 1;
                                    }
                                }
                            }
                        } else {
                            doc = postings.next_doc()?;
                            Assert::assert_true(doc != -1);
                            upto2 += 1;
                        }
                        assert_equals(&docs[upto2 as usize], &doc);
                        if !field.omit_tf {
                            assert_equals(
                                &(term.positions(upto2 as usize).len() as i32),
                                &postings.freq()?,
                            );
                            if random.next_int_bound(2) == 1 {
                                self.verify_positions(
                                    term.positions(upto2 as usize),
                                    &mut *postings,
                                )?;
                            }
                        }
                    }

                    if !ended {
                        assert_equals(&NO_MORE_DOCS, &postings.next_doc()?);
                    }
                }
                upto += 1;

                if terms_enum.next()?.is_none() {
                    break;
                }
            }

            assert_equals(&upto, &field.terms.len());
        }
        Ok(())
    }
}

fn unsupported() -> LuceneError {
    LuceneError::unsupported_operation(String::new())
}

/// `private static class DataFields extends Fields`.
struct DataFields {
    fields: Vec<FieldData>,
}

impl DataFields {
    fn new(fields: Vec<FieldData>) -> Self {
        // already sorted:
        DataFields { fields }
    }
}

impl Fields for DataFields {
    fn iterator(&self) -> Box<dyn Iterator<Item = String> + '_> {
        Box::new(
            self.fields
                .iter()
                .map(|f| f.field_info.get_name().to_string()),
        )
    }

    fn terms(&self, field: &str) -> Result<Option<Box<dyn Terms>>> {
        // Slow linear search:
        for field_data in &self.fields {
            if field_data.field_info.get_name() == field {
                return Ok(Some(Box::new(DataTerms {
                    field_data: field_data.clone(),
                })));
            }
        }
        Ok(None)
    }

    fn size(&self) -> i32 {
        self.fields.len() as i32
    }
}

/// `private static class DataTerms extends Terms`.
struct DataTerms {
    field_data: FieldData,
}

impl Terms for DataTerms {
    fn iterator(&self) -> Result<Box<dyn TermsEnum>> {
        Ok(Box::new(DataTermsEnum {
            field_data: self.field_data.clone(),
            upto: -1,
            attributes: AttributeSource::new(),
        }))
    }

    fn size(&self) -> Result<i64> {
        Err(unsupported())
    }

    fn sum_total_term_freq(&self) -> Result<i64> {
        Err(unsupported())
    }

    fn sum_doc_freq(&self) -> Result<i64> {
        Err(unsupported())
    }

    fn doc_count(&self) -> Result<i32> {
        Err(unsupported())
    }

    fn has_freqs(&self) -> bool {
        self.field_data
            .field_info
            .get_index_options()
            .subsumes(IndexOptions::DOCS_AND_FREQS)
    }

    fn has_offsets(&self) -> bool {
        self.field_data
            .field_info
            .get_index_options()
            .subsumes(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS)
    }

    fn has_positions(&self) -> bool {
        self.field_data
            .field_info
            .get_index_options()
            .subsumes(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS)
    }

    fn has_payloads(&self) -> bool {
        self.field_data.field_info.has_payloads()
    }
}

/// `private static class DataTermsEnum extends BaseTermsEnum`.
struct DataTermsEnum {
    field_data: FieldData,
    upto: i32,
    attributes: AttributeSource,
}

impl fmt::Display for DataTermsEnum {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("DataTermsEnum")
    }
}

impl TermsEnum for DataTermsEnum {
    fn attributes(&mut self) -> &mut AttributeSource {
        &mut self.attributes
    }

    fn next(&mut self) -> Result<Option<BytesRef>> {
        self.upto += 1;
        if self.upto as usize == self.field_data.terms.len() {
            return Ok(None);
        }

        self.term()
    }

    fn term(&self) -> Result<Option<BytesRef>> {
        Ok(Some(self.field_data.terms[self.upto as usize].text.clone()))
    }

    fn seek_ceil(&mut self, text: &BytesRef) -> Result<SeekStatus> {
        // Stupid linear impl:
        for (i, term) in self.field_data.terms.iter().enumerate() {
            match term.text.cmp(text) {
                std::cmp::Ordering::Equal => {
                    self.upto = i as i32;
                    return Ok(SeekStatus::FOUND);
                }
                std::cmp::Ordering::Greater => {
                    self.upto = i as i32;
                    return Ok(SeekStatus::NOT_FOUND);
                }
                std::cmp::Ordering::Less => {}
            }
        }

        Ok(SeekStatus::END)
    }

    fn seek_ord(&mut self, _ord: i64) -> Result<()> {
        Err(unsupported())
    }

    fn ord(&self) -> Result<i64> {
        Err(unsupported())
    }

    fn doc_freq(&mut self) -> Result<i32> {
        Err(unsupported())
    }

    fn total_term_freq(&mut self) -> Result<i64> {
        Err(unsupported())
    }

    fn postings(
        &mut self,
        _reuse: Option<Box<dyn PostingsEnum>>,
        _flags: i32,
    ) -> Result<Option<Box<dyn PostingsEnum>>> {
        Ok(Some(Box::new(DataPostingsEnum {
            terms: Arc::clone(&self.field_data.terms),
            term: self.upto as usize,
            doc_upto: -1,
            pos_upto: 0,
        })))
    }

    fn impacts(&mut self, _flags: i32) -> Result<Box<dyn ImpactsEnum>> {
        Err(unsupported())
    }
}

/// `private static class DataPostingsEnum extends PostingsEnum`; `termData`
/// is `terms[term]`.
struct DataPostingsEnum {
    terms: Arc<Vec<TermData>>,
    term: usize,
    doc_upto: i32,
    pos_upto: i32,
}

impl DataPostingsEnum {
    fn term_data(&self) -> &TermData {
        &self.terms[self.term]
    }

    fn position(&self) -> &PositionData {
        &self.term_data().positions(self.doc_upto as usize)[self.pos_upto as usize]
    }
}

impl fmt::Display for DataPostingsEnum {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("DataPostingsEnum")
    }
}

impl DocIdSetIterator for DataPostingsEnum {
    fn cost(&self) -> i64 {
        Throwable::new("java.lang.UnsupportedOperationException", "").throw()
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.doc_upto += 1;
        if self.doc_upto as usize == self.term_data().docs.len() {
            return Ok(NO_MORE_DOCS);
        }
        self.pos_upto = -1;
        Ok(self.doc_id())
    }

    fn doc_id(&self) -> i32 {
        self.term_data().docs[self.doc_upto as usize]
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        // Slow linear impl:
        self.next_doc()?;
        while self.doc_id() < target {
            self.next_doc()?;
        }

        Ok(self.doc_id())
    }
}

impl PostingsEnum for DataPostingsEnum {
    fn freq(&self) -> Result<i32> {
        Ok(self.term_data().positions(self.doc_upto as usize).len() as i32)
    }

    fn next_position(&mut self) -> Result<i32> {
        self.pos_upto += 1;
        Ok(self.position().pos)
    }

    fn get_payload(&self) -> Result<Option<&[u8]>> {
        Ok(self.position().payload.as_ref().map(|p| p.slice()))
    }

    fn start_offset(&self) -> i32 {
        Throwable::new("java.lang.UnsupportedOperationException", "").throw()
    }

    fn end_offset(&self) -> i32 {
        Throwable::new("java.lang.UnsupportedOperationException", "").throw()
    }
}

/// The anonymous `NormsProducer` of `write`: every document has the norm 1.
#[derive(Debug, Clone)]
struct FakeNorms {
    max_doc: i32,
}

impl NormsProducer for FakeNorms {
    fn get_norms(&self, _field: &FieldInfo) -> Result<Option<Box<dyn NumericDocValues>>> {
        Ok(Some(Box::new(FakeNormValues {
            doc: -1,
            max_doc: self.max_doc,
        })))
    }

    fn check_integrity(&self) -> Result<()> {
        Ok(())
    }

    fn get_merge_instance(&self) -> Result<Box<dyn NormsProducer>> {
        Ok(Box::new(self.clone()))
    }

    fn close(&self) -> Result<()> {
        Ok(())
    }
}

/// The anonymous `NumericDocValues` of the fake norms.
struct FakeNormValues {
    doc: i32,
    max_doc: i32,
}

impl fmt::Display for FakeNormValues {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("FakeNormValues")
    }
}

impl DocIdSetIterator for FakeNormValues {
    fn next_doc(&mut self) -> Result<i32> {
        self.advance(self.doc + 1)
    }

    fn doc_id(&self) -> i32 {
        self.doc
    }

    fn cost(&self) -> i64 {
        i64::from(self.max_doc)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.doc = if target >= self.max_doc {
            NO_MORE_DOCS
        } else {
            target
        };
        Ok(self.doc)
    }
}

impl DocValuesIterator for FakeNormValues {
    fn advance_exact(&mut self, target: i32) -> Result<bool> {
        self.doc = target;
        Ok(true)
    }
}

impl NumericDocValues for FakeNormValues {
    fn long_value(&self) -> Result<i64> {
        Ok(1)
    }
}

/// `private void write(SegmentInfo si, final FieldInfos fieldInfos, final
/// Directory dir, final FieldData[] fields)`.
///
/// Java passes `null` as the segment's buffered updates; Rucene's
/// `SegmentWriteState` takes a value, so an empty `BufferedUpdates` stands
/// for it, as `RandomPostingsTester.buildIndex` does.
fn write(
    si: &SegmentInfo,
    field_infos: &FieldInfos,
    dir: &Arc<dyn Directory>,
    fields: &mut [FieldData],
) -> Result<()> {
    let codec = si
        .codec()
        .unwrap_or_else(|| crate::support::npe("si.getCodec()"));
    let info_stream = default_info_stream();
    let seg_updates = BufferedUpdates::new(SEGMENT);
    let context = LuceneTestCase::new_io_context(&*random());
    let state = SegmentWriteState::new(
        &*info_stream,
        Arc::clone(dir),
        si,
        field_infos,
        &seg_updates,
        &*context,
    );

    // `Arrays.sort(fields)`: by field name.
    fields.sort_by(|a, b| a.field_info.get_name().cmp(b.field_info.get_name()));
    let mut consumer = codec.postings_format().fields_consumer(&state)?;
    let fake_norms = FakeNorms {
        max_doc: si.max_doc()?,
    };
    match consumer.write(&DataFields::new(fields.to_vec()), Some(&fake_norms)) {
        Ok(()) => consumer.close(),
        Err(e) => {
            let _ = consumer.close();
            Err(e)
        }
    }
}

/// Port of `TestCodecs.testDocsOnlyFreq`.
fn test_docs_only_freq() -> Result<()> {
    // tests that when fields are indexed with DOCS_ONLY, the Codec
    // returns 1 in docsEnum.freq()
    let dir = LuceneTestCase::new_directory();
    let random = random();
    let writer = IndexWriter::new(
        dir.as_directory(),
        LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(MockAnalyzer::new(
            &*random,
        )))),
    )?;
    // we don't need many documents to assert this, but don't use one document either
    let num_docs = LuceneTestCase::at_least_with(&*random, 50);
    for _ in 0..num_docs {
        let mut doc = Document::new();
        doc.add(Box::new(StringField::new(
            "f",
            "doc".to_string(),
            Store::NO,
        )?));
        writer.add_document(&doc)?;
    }
    writer.close()?;

    let term = Term::new("f", BytesRef::from_char_sequence("doc"));
    let reader = open_reader(&dir)?;
    for ctx in Arc::clone(&reader).leaves()? {
        let mut de = ctx
            .leaf_reader()
            .postings(&term, POSTINGS_ENUM_FREQS)?
            .unwrap_or_else(|| crate::support::npe("postings(term)"));
        while de.next_doc()? != NO_MORE_DOCS {
            assert_equals_msg(
                Some(&format!("wrong freq for doc {}", de.doc_id())),
                &1,
                &de.freq()?,
            );
        }
    }
    reader.close()?;

    dir.close()?;
    Ok(())
}
