//! The `lucene/codecs` test classes that extend `BasePostingsFormatTestCase`
//! (rmp #360, the #351 slice), each ported in full with the cases it
//! inherits: `TestFixedGapPostingsFormat`,
//! `TestVarGapDocFreqIntervalPostingsFormat`,
//! `TestVarGapFixedIntervalPostingsFormat`, `TestDirectPostingsFormat`,
//! `TestBloomPostingsFormat`, `TestSimpleTextPostingsFormat` (`@Nightly`),
//! `TestFSTPostingsFormat`, `TestUniformSplitPostingFormat`, its subclass
//! `TestSTUniformSplitPostingFormat` (rmp #361) and `TestOrdsBlockTree`
//! (with its own tests).
//!
//! Each suite runs once on the seed `tests.seed` names (a random one when it
//! is unset); `RUCENE_SUITE_SEEDS=<n>` runs it on the first `n` seeds.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::reflect::{Class, Constructor, Method};
use randomizedtesting::junit::{After, Assert, Before};
use rucene::codecs::postings::PostingsFormat;
use rucene::codecs::Codec;
use rucene::document::{Document, Store};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::multi_fields::MultiTerms;
use rucene::index::terms::{SeekStatus, TermsEnum};
use rucene::index::IndexReader;
use rucene::util::BytesRef;
use rucene_codecs::blocktreeords::block_tree_ords_postings_format::BlockTreeOrdsPostingsFormat;
use rucene_codecs::memory::{DirectPostingsFormat, FSTPostingsFormat};
use rucene_codecs::simpletext::codec::SimpleTextCodec;
use rucene_codecs::uniformsplit::sharedterms::st_uniform_split_postings_format::STUniformSplitPostingsFormat;
use rucene_codecs::uniformsplit::uniform_split_postings_format::UniformSplitPostingsFormat;
use rucene_codecs::uniformsplit::uniform_split_terms_writer::{
    DEFAULT_DELTA_NUM_LINES, DEFAULT_TARGET_NUM_BLOCK_LINES,
};
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::codecs::blockterms::lucene_fixed_gap::LuceneFixedGap;
use rucene_test_framework::codecs::blockterms::lucene_var_gap_doc_freq_interval::LuceneVarGapDocFreqInterval;
use rucene_test_framework::codecs::blockterms::lucene_var_gap_fixed_interval::LuceneVarGapFixedInterval;
use rucene_test_framework::codecs::bloom::test_bloom_filtered_lucene_postings::TestBloomFilteredLucenePostings;
use rucene_test_framework::codecs::uniformsplit::sharedterms::STUniformSplitRot13PostingsFormat;
use rucene_test_framework::codecs::uniformsplit::uniform_split_rot13_postings_format::{
    self as rot13, UniformSplitRot13PostingsFormat,
};
use rucene_test_framework::index::base_index_file_format_test_case::{
    rethrow, BaseIndexFileFormatTestCase, FormatTestInstance,
};
use rucene_test_framework::index::base_postings_format_test_case::{
    self, BasePostingsFormatTestCase,
};
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::util::lucene_test_case::{Nightly, VERBOSE};
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

#[path = "../../rucene-test-framework/tests/suite_seeds/mod.rs"]
mod suite_seeds;
use suite_seeds::run_suite;

/// A postings suite whose only member is its codec field (`private final
/// Codec codec = ...`), built when the instance is.
struct CodecSuite {
    codec: Arc<dyn Codec>,
    reuse: bool,
}

impl BaseIndexFileFormatTestCase for CodecSuite {
    fn get_codec(&self) -> Arc<dyn Codec> {
        Arc::clone(&self.codec)
    }

    fn add_random_fields(&self, doc: &mut Document) {
        self.postings_add_random_fields(doc);
    }

    fn as_postings(&self) -> Option<&dyn BasePostingsFormatTestCase> {
        Some(self)
    }
}

impl BasePostingsFormatTestCase for CodecSuite {
    fn is_postings_enum_reuse_implemented(&self) -> bool {
        self.reuse
    }
}

/// The class `name` extending `BasePostingsFormatTestCase`, whose
/// constructor builds `codec()`.
fn codec_suite(
    name: &str,
    codec: fn() -> Arc<dyn Codec>,
    reuse: bool,
) -> randomizedtesting::jdk::reflect::ClassBuilder {
    Class::builder(name)
        .extends(&base_postings_format_test_case::class())
        .constructor(Constructor::no_args(move || {
            FormatTestInstance::new(Arc::new(CodecSuite {
                codec: codec(),
                reuse,
            }))
        }))
}

fn always(format: impl PostingsFormat + 'static) -> Arc<dyn Codec> {
    TestUtil::always_postings_format(Arc::new(format))
}

/// `TestFixedGapPostingsFormat`: `alwaysPostingsFormat(new
/// LuceneFixedGap(nextInt(random(), 1, 1000)))`.
fn fixed_gap_class() -> Arc<Class> {
    codec_suite(
        "org.apache.lucene.codecs.blockterms.TestFixedGapPostingsFormat",
        || {
            always(LuceneFixedGap::with_interval(TestUtil::next_int(
                &*LuceneTestCase::random(),
                1,
                1000,
            )))
        },
        true,
    )
    .build()
}

/// `TestVarGapDocFreqIntervalPostingsFormat`: `alwaysPostingsFormat(new
/// LuceneVarGapFixedInterval(nextInt(random(), 1, 1000)))`.
fn var_gap_doc_freq_interval_class() -> Arc<Class> {
    codec_suite(
        "org.apache.lucene.codecs.blockterms.TestVarGapDocFreqIntervalPostingsFormat",
        || {
            always(LuceneVarGapFixedInterval::with_interval(
                TestUtil::next_int(&*LuceneTestCase::random(), 1, 1000),
            ))
        },
        true,
    )
    .build()
}

/// `TestVarGapFixedIntervalPostingsFormat`: `alwaysPostingsFormat(new
/// LuceneVarGapDocFreqInterval(nextInt(random(), 1, 100), nextInt(random(),
/// 1, 1000)))`.
fn var_gap_fixed_interval_class() -> Arc<Class> {
    codec_suite(
        "org.apache.lucene.codecs.blockterms.TestVarGapFixedIntervalPostingsFormat",
        || {
            let random = LuceneTestCase::random();
            let doc_freq_threshold = TestUtil::next_int(&*random, 1, 100);
            let interval = TestUtil::next_int(&*random, 1, 1000);
            always(LuceneVarGapDocFreqInterval::with_settings(
                doc_freq_threshold,
                interval,
            ))
        },
        true,
    )
    .build()
}

/// `TestDirectPostingsFormat`: `alwaysPostingsFormat(new
/// DirectPostingsFormat())`, without `PostingsEnum` reuse.
fn direct_class() -> Arc<Class> {
    codec_suite(
        "org.apache.lucene.codecs.memory.TestDirectPostingsFormat",
        || always(DirectPostingsFormat::new()),
        false,
    )
    .build()
}

/// `TestBloomPostingsFormat`: `alwaysPostingsFormat(new
/// TestBloomFilteredLucenePostings())`.
fn bloom_class() -> Arc<Class> {
    codec_suite(
        "org.apache.lucene.codecs.bloom.TestBloomPostingsFormat",
        || always(TestBloomFilteredLucenePostings::new()),
        true,
    )
    .build()
}

/// `TestSimpleTextPostingsFormat` (`@Nightly`, "please figure out why I am
/// so horrendously slow!"): `new SimpleTextCodec()`.
fn simple_text_class() -> Arc<Class> {
    codec_suite(
        "org.apache.lucene.codecs.simpletext.TestSimpleTextPostingsFormat",
        || Arc::new(SimpleTextCodec::new()),
        true,
    )
    .annotate(Nightly)
    .build()
}

/// `TestFSTPostingsFormat`: `alwaysPostingsFormat(new FSTPostingsFormat())`.
fn fst_class() -> Arc<Class> {
    codec_suite(
        "org.apache.lucene.codecs.memory.TestFSTPostingsFormat",
        || always(FSTPostingsFormat::new()),
        true,
    )
    .build()
}

// -----------------------------------------------------------------------------
// TestUniformSplitPostingFormat
// -----------------------------------------------------------------------------

/// `TestUniformSplitPostingFormat`, and its subclass
/// `TestSTUniformSplitPostingFormat` when `shared_terms` is set (the
/// subclass overrides only `getPostingsFormat()`).
struct TestUniformSplitPostingFormat {
    check_encoding: bool,
    codec: Arc<dyn Codec>,
    should_check_decoder_was_called: AtomicBool,
}

impl TestUniformSplitPostingFormat {
    /// `TestUniformSplitPostingFormat()`: `checkEncoding =
    /// random().nextBoolean(); codec = alwaysPostingsFormat(getPostingsFormat())`.
    fn new() -> Self {
        Self::with_shared_terms(false)
    }

    /// The constructor run by `TestUniformSplitPostingFormat` (`false`) or
    /// by `TestSTUniformSplitPostingFormat` (`true`), whose override of the
    /// virtual `getPostingsFormat()` the constructor calls.
    fn with_shared_terms(shared_terms: bool) -> Self {
        let check_encoding = LuceneTestCase::random().next_boolean();
        let format = if shared_terms {
            Self::st_get_postings_format(check_encoding)
        } else {
            Self::get_postings_format(check_encoding)
        };
        let codec = TestUtil::always_postings_format(format);
        TestUniformSplitPostingFormat {
            check_encoding,
            codec,
            should_check_decoder_was_called: AtomicBool::new(true),
        }
    }

    /// `getPostingsFormat()`.
    fn get_postings_format(check_encoding: bool) -> Arc<dyn PostingsFormat> {
        if check_encoding {
            Arc::new(UniformSplitRot13PostingsFormat::new())
        } else {
            Arc::new(
                UniformSplitPostingsFormat::with_settings(
                    DEFAULT_TARGET_NUM_BLOCK_LINES,
                    DEFAULT_DELTA_NUM_LINES,
                    None,
                    None,
                    LuceneTestCase::random().next_boolean(),
                )
                .unwrap_or_else(|e| rethrow(e)),
            )
        }
    }

    /// `TestSTUniformSplitPostingFormat.getPostingsFormat()`.
    fn st_get_postings_format(check_encoding: bool) -> Arc<dyn PostingsFormat> {
        if check_encoding {
            Arc::new(STUniformSplitRot13PostingsFormat::new())
        } else {
            Arc::new(
                STUniformSplitPostingsFormat::with_settings(
                    DEFAULT_TARGET_NUM_BLOCK_LINES,
                    DEFAULT_DELTA_NUM_LINES,
                    None,
                    None,
                    LuceneTestCase::random().next_boolean(),
                )
                .unwrap_or_else(|e| rethrow(e)),
            )
        }
    }

    /// `initializeInner()`.
    fn initialize_inner(&self) {
        UniformSplitRot13PostingsFormat::reset_encoding_flags();
    }

    /// `checkEncodingCalledInner()`.
    fn check_encoding_called_inner(&self) {
        Assert::assert_true(rot13::BLOCKS_ENCODED.load(Ordering::SeqCst));
        Assert::assert_true(rot13::FIELDS_METADATA_ENCODED.load(Ordering::SeqCst));
        Assert::assert_true(rot13::DICTIONARY_ENCODED.load(Ordering::SeqCst));
        if self.should_check_decoder_was_called.load(Ordering::SeqCst) {
            Assert::assert_true(rot13::DECODER_CALLED.load(Ordering::SeqCst));
        }
    }
}

impl BaseIndexFileFormatTestCase for TestUniformSplitPostingFormat {
    fn get_codec(&self) -> Arc<dyn Codec> {
        Arc::clone(&self.codec)
    }

    fn add_random_fields(&self, doc: &mut Document) {
        self.postings_add_random_fields(doc);
    }

    fn as_postings(&self) -> Option<&dyn BasePostingsFormatTestCase> {
        Some(self)
    }
}

impl BasePostingsFormatTestCase for TestUniformSplitPostingFormat {}

fn uniform_split_class() -> Arc<Class> {
    Class::builder("org.apache.lucene.codecs.uniformsplit.TestUniformSplitPostingFormat")
        .extends(&base_postings_format_test_case::class())
        .constructor(Constructor::no_args(|| {
            FormatTestInstance::new(Arc::new(TestUniformSplitPostingFormat::new()))
        }))
        .method(
            Method::new("initialize")
                .annotate(Before)
                .body(|inst: &mut FormatTestInstance| {
                    inst.subclass::<TestUniformSplitPostingFormat>()
                        .initialize_inner()
                }),
        )
        .method(Method::new("checkEncodingCalled").annotate(After).body(
            |inst: &mut FormatTestInstance| {
                let this = inst.subclass::<TestUniformSplitPostingFormat>();
                if this.check_encoding {
                    this.check_encoding_called_inner();
                }
            },
        ))
        .method(
            Method::new("testRandomExceptions")
                .throws(&["java.lang.Exception"])
                .body(|inst: &mut FormatTestInstance| {
                    let this = inst.subclass::<TestUniformSplitPostingFormat>();
                    this.should_check_decoder_was_called
                        .store(false, Ordering::SeqCst);
                    BaseIndexFileFormatTestCase::test_random_exceptions(&*this)
                        .unwrap_or_else(|e| rethrow(e));
                }),
        )
        .build()
}

/// `org.apache.lucene.codecs.uniformsplit.sharedterms.TestSTUniformSplitPostingFormat
/// extends TestUniformSplitPostingFormat`: tests `STUniformSplitPostingsFormat`
/// with block encoding using ROT13 cypher.
fn st_uniform_split_class() -> Arc<Class> {
    Class::builder(
        "org.apache.lucene.codecs.uniformsplit.sharedterms.TestSTUniformSplitPostingFormat",
    )
    .extends(&uniform_split_class())
    .constructor(Constructor::no_args(|| {
        FormatTestInstance::new(Arc::new(TestUniformSplitPostingFormat::with_shared_terms(
            true,
        )))
    }))
    .build()
}

// -----------------------------------------------------------------------------
// TestOrdsBlockTree
// -----------------------------------------------------------------------------

/// A test method of `TestOrdsBlockTree`: `public void <name>() throws
/// Exception`.
fn ords_test(name: &str, body: fn() -> Result<()>) -> Method {
    Method::new(name)
        .throws(&["java.lang.Exception"])
        .body_any(move || body().unwrap_or_else(|e| rethrow(e)))
}

/// The `MultiTerms` terms enum of `field` over `r`.
fn multi_terms_enum(r: &Arc<dyn IndexReader>, field: &str) -> Result<Box<dyn TermsEnum>> {
    MultiTerms::get_terms(r, field)?
        .unwrap_or_else(|| {
            randomizedtesting::jdk::exception::throw_without_message(
                "java.lang.NullPointerException",
            )
        })
        .iterator()
}

fn bytes(s: &str) -> BytesRef {
    BytesRef::from_char_sequence(s)
}

/// `"" + (char) c`.
fn char_string(c: u32) -> String {
    char::from_u32(c).map(String::from).unwrap_or_default()
}

/// A document of one `newTextField(field, term, NO)` /
/// `newStringField(field, term, NO)`.
fn one_field_doc(field: &str, term: &str, text: bool) -> Document {
    let mut doc = Document::new();
    if text {
        doc.add(Box::new(LuceneTestCase::new_text_field(
            field,
            term,
            Store::NO,
        )));
    } else {
        doc.add(Box::new(LuceneTestCase::new_string_field(
            field,
            term,
            Store::NO,
        )));
    }
    doc
}

/// `assertEquals(new BytesRef(expected), te.next())`.
fn assert_next(te: &mut dyn TermsEnum, expected: &str) -> Result<()> {
    let next = te.next()?;
    if next.as_ref() != Some(&bytes(expected)) {
        let was = next.map_or_else(|| "null".to_string(), |b| b.to_string());
        randomizedtesting::jdk::exception::throw(
            "java.lang.AssertionError",
            format!("expected:<{}> but was:<{was}>", bytes(expected)),
        );
    }
    Ok(())
}

/// `testBasic()`.
fn ords_test_basic() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let w =
        RandomIndexWriter::new_with_mock_analyzer(&*LuceneTestCase::random(), dir.as_directory())?;
    w.add_document(&one_field_doc("field", "a b c", true))?;
    let r: Arc<dyn IndexReader> = w.get_reader()?;
    let mut te = multi_terms_enum(&r, "field")?;

    // Test next()
    assert_next(&mut *te, "a")?;
    Assert::assert_equals(0i64, te.ord()?);
    assert_next(&mut *te, "b")?;
    Assert::assert_equals(1i64, te.ord()?);
    assert_next(&mut *te, "c")?;
    Assert::assert_equals(2i64, te.ord()?);
    Assert::assert_true(te.next()?.is_none());

    // Test seekExact by term
    Assert::assert_true(te.seek_exact(&bytes("b"))?);
    Assert::assert_equals(1i64, te.ord()?);
    Assert::assert_true(te.seek_exact(&bytes("a"))?);
    Assert::assert_equals(0i64, te.ord()?);
    Assert::assert_true(te.seek_exact(&bytes("c"))?);
    Assert::assert_equals(2i64, te.ord()?);

    // Test seekExact by ord
    te.seek_ord(1)?;
    Assert::assert_equals(bytes("b"), te.term()?);
    te.seek_ord(0)?;
    Assert::assert_equals(bytes("a"), te.term()?);
    te.seek_ord(2)?;
    Assert::assert_equals(bytes("c"), te.term()?);

    drop(te);
    r.close()?;
    drop(r);
    w.close()?;
    drop(w);
    dir.close()
}

/// The 36 terms `"" + (char) (97 + i)` prefixed by `prefix`, each in its own
/// document of a `newTextField`.
fn add_36(w: &RandomIndexWriter, terms: &mut Vec<String>, prefix: &str) -> Result<()> {
    for i in 0..36 {
        let term = format!("{prefix}{}", char_string(97 + i));
        terms.push(term.clone());
        if *VERBOSE {
            println!("i={i} term={term}");
        }
        w.add_document(&one_field_doc("field", &term, true))?;
    }
    Ok(())
}

/// `testTwoBlocks()`.
fn ords_test_two_blocks() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let w =
        RandomIndexWriter::new_with_mock_analyzer(&*LuceneTestCase::random(), dir.as_directory())?;
    let mut terms: Vec<String> = Vec::new();
    add_36(&w, &mut terms, "")?;
    add_36(&w, &mut terms, "m")?;

    if *VERBOSE {
        println!("TEST: now forceMerge");
    }
    w.force_merge(1)?;
    let r: Arc<dyn IndexReader> = w.get_reader()?;
    let mut te = multi_terms_enum(&r, "field")?;

    Assert::assert_true(te.seek_exact(&bytes("mo"))?);
    Assert::assert_equals(27i64, te.ord()?);

    te.seek_ord(54)?;
    Assert::assert_equals(bytes("s"), te.term()?);

    terms.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));

    for i in (0..terms.len()).rev() {
        te.seek_ord(i as i64)?;
        Assert::assert_equals(i as i64, te.ord()?);
        Assert::assert_equals_str(None, Some(&terms[i]), Some(&te.term()?.utf8_to_string()?));
    }

    let iters = LuceneTestCase::at_least(1000);
    for iter in 0..iters {
        let ord = LuceneTestCase::random().next_int_bound(terms.len() as i32) as usize;
        let term = bytes(&terms[ord]);
        if LuceneTestCase::random().next_boolean() {
            if *VERBOSE {
                println!("TEST: iter={iter} seek to ord={ord} of {}", terms.len());
            }
            te.seek_ord(ord as i64)?;
        } else {
            if *VERBOSE {
                println!(
                    "TEST: iter={iter} seek to term={} ord={ord} of {}",
                    terms[ord],
                    terms.len()
                );
            }
            te.seek_exact(&term)?;
        }
        Assert::assert_equals(ord as i64, te.ord()?);
        Assert::assert_equals(term, te.term()?);
    }

    drop(te);
    r.close()?;
    drop(r);
    w.close()?;
    drop(w);
    dir.close()
}

/// `testThreeBlocks()`.
fn ords_test_three_blocks() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let w =
        RandomIndexWriter::new_with_mock_analyzer(&*LuceneTestCase::random(), dir.as_directory())?;
    let mut terms: Vec<String> = Vec::new();
    add_36(&w, &mut terms, "")?;
    add_36(&w, &mut terms, "m")?;
    add_36(&w, &mut terms, "mo")?;
    w.force_merge(1)?;
    let r: Arc<dyn IndexReader> = w.get_reader()?;
    let mut te = multi_terms_enum(&r, "field")?;

    if *VERBOSE {
        while te.next()?.is_some() {
            println!("TERM: {} {}", te.ord()?, te.term()?.utf8_to_string()?);
        }
    }

    Assert::assert_true(te.seek_exact(&bytes("mo"))?);
    Assert::assert_equals(27i64, te.ord()?);

    te.seek_ord(90)?;
    Assert::assert_equals(bytes("s"), te.term()?);

    test_enum(&mut *te, &mut terms)?;

    drop(te);
    r.close()?;
    drop(r);
    w.close()?;
    drop(w);
    dir.close()
}

/// `testEnum(te, terms)` (`private`).
fn test_enum(te: &mut dyn TermsEnum, terms: &mut [String]) -> Result<()> {
    terms.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
    for i in (0..terms.len()).rev() {
        if *VERBOSE {
            println!("TEST: seek to ord={i}");
        }
        te.seek_ord(i as i64)?;
        Assert::assert_equals(i as i64, te.ord()?);
        Assert::assert_equals_str(None, Some(&terms[i]), Some(&te.term()?.utf8_to_string()?));
    }

    let iters = LuceneTestCase::at_least(1000);
    for _ in 0..iters {
        let ord = LuceneTestCase::random().next_int_bound(terms.len() as i32) as usize;
        if LuceneTestCase::random().next_boolean() {
            te.seek_ord(ord as i64)?;
            Assert::assert_equals_str(None, Some(&terms[ord]), Some(&te.term()?.utf8_to_string()?));
        } else {
            te.seek_exact(&bytes(&terms[ord]))?;
            Assert::assert_equals(ord as i64, te.ord()?);
        }
    }
    Ok(())
}

/// `new IndexWriter(dir, new IndexWriterConfig(new MockAnalyzer(random())))`.
fn plain_writer(
    dir: &rucene_test_framework::util::lucene_test_case::TestDirectory,
) -> Result<Arc<IndexWriter>> {
    IndexWriter::new(
        dir.as_directory(),
        IndexWriterConfig::with_analyzer(Arc::new(MockAnalyzer::new(&*LuceneTestCase::random()))),
    )
}

/// `testFloorBlocks()`.
fn ords_test_floor_blocks() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let w = plain_writer(&dir)?;
    for i in 0..128 {
        let term = char_string(i);
        if *VERBOSE {
            println!("i={i} term={term} bytes={}", bytes(&term));
        }
        w.add_document(&one_field_doc("field", &term, false))?;
    }
    w.force_merge(1)?;
    let r: Arc<dyn IndexReader> = directory_reader::open_with_writer(&*w)?;
    let mut te = multi_terms_enum(&r, "field")?;

    if *VERBOSE {
        while let Some(term) = te.next()? {
            println!("  {}: {}", te.ord()?, term.utf8_to_string()?);
        }
    }

    Assert::assert_true(te.seek_exact(&bytes("a"))?);
    Assert::assert_equals(97i64, te.ord()?);

    te.seek_ord(98)?;
    Assert::assert_equals(bytes("b"), te.term()?);

    Assert::assert_true(te.seek_exact(&bytes("z"))?);
    Assert::assert_equals(122i64, te.ord()?);

    drop(te);
    r.close()?;
    drop(r);
    w.close()?;
    drop(w);
    dir.close()
}

/// `testNonRootFloorBlocks()`.
fn ords_test_non_root_floor_blocks() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let w = plain_writer(&dir)?;
    let mut terms: Vec<String> = Vec::new();
    for i in 0..36 {
        let term = char_string(97 + i);
        terms.push(term.clone());
        if *VERBOSE {
            println!("i={i} term={term}");
        }
        w.add_document(&one_field_doc("field", &term, false))?;
    }
    for i in 0..128 {
        let term = format!("m{}", char_string(i));
        terms.push(term.clone());
        if *VERBOSE {
            println!("i={i} term={term} bytes={}", bytes(&term));
        }
        w.add_document(&one_field_doc("field", &term, false))?;
    }
    w.force_merge(1)?;
    let r: Arc<dyn IndexReader> = directory_reader::open_with_writer(&*w)?;
    let mut te = multi_terms_enum(&r, "field")?;

    let mut ord: i64 = 0;
    while let Some(term) = te.next()? {
        if *VERBOSE {
            println!("TEST: {}: {}", te.ord()?, term.utf8_to_string()?);
        }
        Assert::assert_equals(ord, te.ord()?);
        ord += 1;
    }

    test_enum(&mut *te, &mut terms)?;

    drop(te);
    r.close()?;
    drop(r);
    w.close()?;
    drop(w);
    dir.close()
}

/// `testSeveralNonRootBlocks()`.
fn ords_test_several_non_root_blocks() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let w = plain_writer(&dir)?;
    let mut terms: Vec<String> = Vec::new();
    for i in 0..30 {
        for j in 0..30 {
            let term = format!("{}{}", char_string(97 + i), char_string(97 + j));
            terms.push(term.clone());
            if *VERBOSE {
                println!("term={term}");
            }
            w.add_document(&one_field_doc("body", &term, true))?;
        }
    }
    w.force_merge(1)?;
    let r: Arc<dyn IndexReader> = directory_reader::open_with_writer(&*w)?;
    let mut te = multi_terms_enum(&r, "body")?;

    for i in 0..30 {
        for j in 0..30 {
            let term = format!("{}{}", char_string(97 + i), char_string(97 + j));
            if *VERBOSE {
                println!("TEST: check term={term}");
            }
            let next = te.next()?.unwrap_or_else(|| {
                randomizedtesting::jdk::exception::throw_without_message(
                    "java.lang.NullPointerException",
                )
            });
            Assert::assert_equals_str(None, Some(&term), Some(&next.utf8_to_string()?));
            Assert::assert_equals(i64::from(30 * i + j), te.ord()?);
        }
    }

    test_enum(&mut *te, &mut terms)?;

    te.seek_ord(0)?;
    Assert::assert_equals_str(None, Some("aa"), Some(&te.term()?.utf8_to_string()?));

    drop(te);
    r.close()?;
    drop(r);
    w.close()?;
    drop(w);
    dir.close()
}

/// `testSeekCeilNotFound()`.
fn ords_test_seek_ceil_not_found() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let w =
        RandomIndexWriter::new_with_mock_analyzer(&*LuceneTestCase::random(), dir.as_directory())?;
    // Get empty string in there!
    w.add_document(&one_field_doc("field", "", false))?;

    for i in 0..36 {
        let mut doc = Document::new();
        let term = char_string(97 + i);
        let term2 = format!("a{}", char_string(97 + i));
        doc.add(Box::new(LuceneTestCase::new_string_field(
            "field",
            &term,
            Store::NO,
        )));
        doc.add(Box::new(LuceneTestCase::new_string_field(
            "field",
            &term2,
            Store::NO,
        )));
        w.add_document(&doc)?;
    }

    w.force_merge(1)?;
    let r: Arc<dyn IndexReader> = w.get_reader()?;
    let mut te = multi_terms_enum(&r, "field")?;
    Assert::assert_equals(
        JavaSeekStatus(SeekStatus::NOT_FOUND),
        JavaSeekStatus(te.seek_ceil(&BytesRef::new(vec![0x22]))?),
    );
    Assert::assert_equals_str(None, Some("a"), Some(&te.term()?.utf8_to_string()?));
    Assert::assert_equals(1i64, te.ord()?);

    drop(te);
    r.close()?;
    drop(r);
    w.close()?;
    drop(w);
    dir.close()
}

/// A `SeekStatus` as JUnit prints it.
#[derive(PartialEq)]
struct JavaSeekStatus(SeekStatus);

impl std::fmt::Display for JavaSeekStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self.0 {
            SeekStatus::END => "END",
            SeekStatus::FOUND => "FOUND",
            SeekStatus::NOT_FOUND => "NOT_FOUND",
        })
    }
}

/// `TestOrdsBlockTree`: `alwaysPostingsFormat(new
/// BlockTreeOrdsPostingsFormat())`, with its own tests.
fn ords_block_tree_class() -> Arc<Class> {
    codec_suite(
        "org.apache.lucene.codecs.blocktreeords.TestOrdsBlockTree",
        || always(BlockTreeOrdsPostingsFormat::new()),
        true,
    )
    .method(ords_test("testBasic", ords_test_basic))
    .method(ords_test("testTwoBlocks", ords_test_two_blocks))
    .method(ords_test("testThreeBlocks", ords_test_three_blocks))
    .method(ords_test("testFloorBlocks", ords_test_floor_blocks))
    .method(ords_test(
        "testNonRootFloorBlocks",
        ords_test_non_root_floor_blocks,
    ))
    .method(ords_test(
        "testSeveralNonRootBlocks",
        ords_test_several_non_root_blocks,
    ))
    .method(ords_test(
        "testSeekCeilNotFound",
        ords_test_seek_ceil_not_found,
    ))
    .build()
}

/// One suite of this binary at a time: the runner's state is process-wide.
static SUITES: Mutex<()> = Mutex::new(());

macro_rules! suite_test {
    ($name:ident, $class:ident) => {
        #[test]
        fn $name() {
            let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
            run_suite($class);
        }
    };
}

suite_test!(test_fixed_gap_postings_format, fixed_gap_class);
suite_test!(test_st_uniform_split_posting_format, st_uniform_split_class);
suite_test!(
    test_var_gap_doc_freq_interval_postings_format,
    var_gap_doc_freq_interval_class
);
suite_test!(
    test_var_gap_fixed_interval_postings_format,
    var_gap_fixed_interval_class
);
suite_test!(test_direct_postings_format, direct_class);
suite_test!(test_bloom_postings_format, bloom_class);
suite_test!(test_simple_text_postings_format, simple_text_class);
suite_test!(test_fst_postings_format, fst_class);
suite_test!(test_uniform_split_posting_format, uniform_split_class);
suite_test!(test_ords_block_tree, ords_block_tree_class);

/// Regression: `TestDirectPostingsFormat.testInvertedWrite` on the master
/// seed `9D704B962994A060` hung. Its `FieldsConsumer.write` calls `random()`
/// on the merge thread, which had no randomized context; the merge thread
/// died without finishing its merge and `forceMerge` waited for ever. The
/// suite runs on its own thread, bounded so that a hang fails the test.
#[test]
fn test_direct_postings_format_inverted_write_merge_thread_random() {
    use randomizedtesting::jdk::system;
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    system::set_property("tests.seed", "9D704B962994A060");
    system::set_property("tests.method", "testInvertedWrite");
    let (done, finished) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let outcome = std::panic::catch_unwind(|| run_suite(direct_class));
        let _ = done.send(outcome.is_ok());
    });
    let outcome = finished.recv_timeout(std::time::Duration::from_secs(300));
    system::clear_property("tests.method");
    system::clear_property("tests.seed");
    match outcome {
        Ok(passed) => assert!(passed, "the suite failed"),
        Err(_) => panic!("testInvertedWrite did not finish within 300 s: hung"),
    }
}
