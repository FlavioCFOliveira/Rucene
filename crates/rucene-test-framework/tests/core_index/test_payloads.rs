//! Port of `lucene/core/src/test/org/apache/lucene/index/TestPayloads.java`.
//!
//! * `PayloadAnalyzer.fieldToData`, a `HashMap` the test changes after the
//!   analyzer is handed to the writer and each `PayloadFilter` reads on
//!   `reset`, is shared behind a mutex.
//! * `PostingsEnum.getPayload()` answers the payload bytes; the
//!   `br.bytes, br.offset, br.length` triple of Java is that slice.
//! * `field.setTokenStream(ts)` on a field already added changes the
//!   document's own field (`crate::support::field_mut`).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::jdk::thread::Thread;
use randomizedtesting::junit::Assert;
use rucene::analysis::tokenattributes::PayloadAttribute;
use rucene::analysis::{
    Analyzer, AnalyzerState, PerFieldReuseStrategy, StringCharReader, TokenFilter,
    TokenFilterLogic, TokenStream, TokenStreamComponents,
};
use rucene::document::{Document, Field, Store, TextField};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::OpenMode;
use rucene::index::multi_fields::MultiTerms;
use rucene::index::postings_enum::POSTINGS_ENUM_PAYLOADS;
use rucene::index::terms::Term;
use rucene::index::{IndexReader, LeafReader, PostingsEnum};
use rucene::search::NO_MORE_DOCS;
use rucene::store::Directory;
use rucene::util::atomic_ref_cell::AtomicRefCell;
use rucene::util::attribute::AttributeSource;
use rucene::util::BytesRef;
use rucene_test_framework::analysis::attributes::{char_term_id, payload_id, Att};
use rucene_test_framework::analysis::canned_token_stream::CannedTokenStream;
use rucene_test_framework::analysis::mock_tokenizer::{MockTokenizer, WHITESPACE};
use rucene_test_framework::analysis::token::Token;
use rucene_test_framework::analysis::{
    components, shared, sink, tokenizer_components, MockAnalyzer,
};
use rucene_test_framework::index::RandomIndexWriter;
use rucene_test_framework::util::java_exceptions::java_throwable;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::{assert_equals, assert_equals_msg, class, random, test};
use crate::support::{field_mut, npe};

/// `TestPayloads`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestPayloads",
        vec![
            test("testPayload", test_payload),
            test("testPayloadFieldBit", test_payload_field_bit),
            test("testPayloadsEncoding", test_payloads_encoding),
            test("testThreadSafety", test_thread_safety),
            test("testAcrossFields", test_across_fields),
            test("testMixupDocs", test_mixup_docs),
            test("testMixupMultiValued", test_mixup_multi_valued),
        ],
    )
}

/// Port of `TestPayloads.testPayload`: simple tests to test the Payload
/// class.
fn test_payload() -> Result<()> {
    let payload = BytesRef::from_char_sequence("This is a test!");
    assert_equals_msg(
        Some("Wrong payload length."),
        &"This is a test!".len(),
        &payload.length,
    );

    let clone = payload.clone();
    assert_equals(&payload.length, &clone.length);
    for i in 0..payload.length {
        assert_equals(
            &payload.bytes[i + payload.offset],
            &clone.bytes[i + clone.offset],
        );
    }
    Ok(())
}

/// `fi.fieldInfo(name).hasPayloads()`.
fn has_payloads(reader: &Arc<dyn LeafReader>, name: &str) -> bool {
    reader
        .get_field_infos()
        .field_info(name)
        .unwrap_or_else(|| npe("fieldInfo"))
        .has_payloads()
}

/// `getOnlyLeafReader(DirectoryReader.open(dir))`.
fn only_leaf(dir: &Arc<dyn Directory>) -> Result<Arc<dyn LeafReader>> {
    LuceneTestCase::get_only_leaf_reader(
        directory_reader::open(Arc::clone(dir))? as Arc<dyn IndexReader>
    )
}

/// `newTextField(name, value, Field.Store.NO)`.
fn text(name: &str, value: &str) -> Box<Field> {
    Box::new(LuceneTestCase::new_text_field(name, value, Store::NO))
}

/// `newIndexWriterConfig(analyzer)`.
fn config(
    analyzer: &Arc<PayloadAnalyzer>,
) -> rucene::index::index_writer_config::IndexWriterConfig {
    LuceneTestCase::new_index_writer_config_analyzer(
        Some(Arc::clone(analyzer) as Arc<dyn Analyzer>),
    )
}

/// Port of `TestPayloads.testPayloadFieldBit`: tests whether the
/// DocumentWriter and SegmentMerger correctly enable the payload bit in the
/// FieldInfo.
fn test_payload_field_bit() -> Result<()> {
    let mut ram = LuceneTestCase::new_directory();
    let analyzer = Arc::new(PayloadAnalyzer::new());
    let writer = IndexWriter::new(ram.as_directory(), config(&analyzer))?;
    let mut d = Document::new();
    // this field won't have any payloads
    d.add(text("f1", "This field has no payloads"));
    // this field will have payloads in all docs, however not for all term positions,
    // so this field is used to check if the DocumentWriter correctly enables the payloads bit
    // even if only some term positions have payloads
    d.add(text("f2", "This field has payloads in all docs"));
    d.add(text("f2", "This field has payloads in all docs NO PAYLOAD"));
    // this field is used to verify if the SegmentMerger enables payloads for a field if it has
    // payloads
    // enabled in only some documents
    d.add(text("f3", "This field has payloads in some docs"));
    // only add payload data for field f2
    analyzer.set_payload_data("f2", Arc::new(b"somedata".to_vec()), 0, 1);
    writer.add_document(&d)?;
    // flush
    writer.close()?;

    let reader = only_leaf(&ram.as_directory())?;
    Assert::assert_false_msg(
        "Payload field bit should not be set.",
        has_payloads(&reader, "f1"),
    );
    Assert::assert_true_msg(
        "Payload field bit should be set.",
        has_payloads(&reader, "f2"),
    );
    Assert::assert_false_msg(
        "Payload field bit should not be set.",
        has_payloads(&reader, "f3"),
    );
    Arc::clone(&reader).as_index_reader().close()?;

    // now we add another document which has payloads for field f3 and verify if the SegmentMerger
    // enabled payloads for that field
    let analyzer = Arc::new(PayloadAnalyzer::new()); // Clear payload state for each field
    let conf = config(&analyzer);
    conf.set_open_mode(OpenMode::CREATE)?;
    let writer = IndexWriter::new(ram.as_directory(), conf)?;
    let mut d = Document::new();
    d.add(text("f1", "This field has no payloads"));
    d.add(text("f2", "This field has payloads in all docs"));
    d.add(text("f2", "This field has payloads in all docs"));
    d.add(text("f3", "This field has payloads in some docs"));
    // add payload data for field f2 and f3
    analyzer.set_payload_data("f2", Arc::new(b"somedata".to_vec()), 0, 1);
    analyzer.set_payload_data("f3", Arc::new(b"somedata".to_vec()), 0, 3);
    writer.add_document(&d)?;

    // force merge
    writer.force_merge(1)?;
    // flush
    writer.close()?;

    let reader = only_leaf(&ram.as_directory())?;
    Assert::assert_false_msg(
        "Payload field bit should not be set.",
        has_payloads(&reader, "f1"),
    );
    Assert::assert_true_msg(
        "Payload field bit should be set.",
        has_payloads(&reader, "f2"),
    );
    Assert::assert_true_msg(
        "Payload field bit should be set.",
        has_payloads(&reader, "f3"),
    );
    Arc::clone(&reader).as_index_reader().close()?;
    ram.close()
}

/// Port of `TestPayloads.testPayloadsEncoding`: tests if payloads are
/// correctly stored and loaded.
fn test_payloads_encoding() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    perform_test(&dir.as_directory())?;
    dir.close()
}

/// `MultiTerms.getTermPostingsEnum(reader, field, new BytesRef(text))`,
/// dereferenced as Java does.
fn postings(
    reader: &Arc<dyn IndexReader>,
    field: &str,
    text: &str,
) -> Result<Box<dyn PostingsEnum>> {
    Ok(
        MultiTerms::get_term_postings_enum_all(reader, field, &BytesRef::from_char_sequence(text))?
            .unwrap_or_else(|| npe("MultiTerms.getTermPostingsEnum")),
    )
}

/// `tp.getPayload()`, dereferenced as Java does.
fn payload_of(tp: &dyn PostingsEnum) -> Result<Vec<u8>> {
    Ok(tp
        .get_payload()?
        .unwrap_or_else(|| npe("getPayload()"))
        .to_vec())
}

/// `private void performTest(Directory dir)`: builds an index with payloads
/// in the given Directory and performs different tests to verify the payload
/// encoding.
fn perform_test(dir: &Arc<dyn Directory>) -> Result<()> {
    let analyzer = Arc::new(PayloadAnalyzer::new());
    let conf = config(&analyzer);
    conf.set_open_mode(OpenMode::CREATE)?;
    conf.set_merge_policy(Some(Arc::new(LuceneTestCase::new_log_merge_policy())))?;
    let writer = IndexWriter::new(Arc::clone(dir), conf)?;

    // should be in sync with value in TermInfosWriter
    let skip_interval = 16;

    let num_terms = 5;
    let field_name = "f1";

    let num_docs = skip_interval + 1;
    // create content for the test documents with just a few terms
    let terms = generate_terms(field_name, num_terms);
    let mut sb = String::new();
    for term in &terms {
        sb.push_str(&term.text());
        sb.push(' ');
    }
    let content = sb;

    let payload_data_length = num_terms * num_docs * 2 + num_terms * num_docs * (num_docs - 1) / 2;
    let payload_data = Arc::new(generate_random_data(payload_data_length));

    let mut d = Document::new();
    d.add(text(field_name, &content));
    // add the same document multiple times to have the same payload lengths for all
    // occurrences within two consecutive skip intervals
    let mut offset = 0;
    for _ in 0..2 * num_docs {
        analyzer.set_payload_data(field_name, Arc::clone(&payload_data), offset, 1);
        offset += num_terms;
        writer.add_document(&d)?;
    }

    // make sure we create more than one segment to test merging
    writer.commit()?;

    // now we make sure to have different payload lengths next at the next skip point
    for i in 0..num_docs {
        analyzer.set_payload_data(field_name, Arc::clone(&payload_data), offset, i);
        offset += i * num_terms;
        writer.add_document(&d)?;
    }

    writer.force_merge(1)?;
    // flush
    writer.close()?;

    /*
     * Verify the index
     * first we test if all payloads are stored correctly
     */
    let reader: Arc<dyn IndexReader> = directory_reader::open(Arc::clone(dir))?;

    let mut verify_payload_data = vec![0u8; payload_data_length as usize];
    let mut offset = 0usize;
    let mut tps: Vec<Box<dyn PostingsEnum>> = Vec::with_capacity(num_terms as usize);
    for term in &terms {
        tps.push(postings(&reader, term.field(), &term.text())?);
    }

    while tps[0].next_doc()? != NO_MORE_DOCS {
        for tp in tps.iter_mut().skip(1) {
            tp.next_doc()?;
        }
        let freq = tps[0].freq()?;

        for _ in 0..freq {
            for tp in tps.iter_mut() {
                tp.next_position()?;
                if let Some(br) = tp.get_payload()? {
                    verify_payload_data[offset..offset + br.len()].copy_from_slice(br);
                    offset += br.len();
                }
            }
        }
    }

    assert_byte_array_equals(&payload_data, &verify_payload_data);

    /*
     *  test lazy skipping
     */
    let mut tp = postings(&reader, terms[0].field(), &terms[0].text())?;
    tp.next_doc()?;
    tp.next_position()?;
    // NOTE: prior rev of this test was failing to first
    // call next here:
    tp.next_doc()?;
    // now we don't read this payload
    tp.next_position()?;
    let payload = payload_of(&*tp)?;
    assert_equals_msg(Some("Wrong payload length."), &1, &payload.len());
    assert_equals(&payload[0], &payload_data[num_terms as usize]);
    tp.next_doc()?;
    tp.next_position()?;

    // we don't read this payload and skip to a different document
    tp.advance(5)?;
    tp.next_position()?;
    let payload = payload_of(&*tp)?;
    assert_equals_msg(Some("Wrong payload length."), &1, &payload.len());
    assert_equals(&payload[0], &payload_data[(5 * num_terms) as usize]);

    /*
     * Test different lengths at skip points
     */
    let mut tp = postings(&reader, terms[1].field(), &terms[1].text())?;
    tp.next_doc()?;
    tp.next_position()?;
    assert_equals_msg(Some("Wrong payload length."), &1, &payload_of(&*tp)?.len());
    tp.advance(skip_interval - 1)?;
    tp.next_position()?;
    assert_equals_msg(Some("Wrong payload length."), &1, &payload_of(&*tp)?.len());
    tp.advance(2 * skip_interval - 1)?;
    tp.next_position()?;
    assert_equals_msg(Some("Wrong payload length."), &1, &payload_of(&*tp)?.len());
    tp.advance(3 * skip_interval - 1)?;
    tp.next_position()?;
    assert_equals_msg(
        Some("Wrong payload length."),
        &((3 * skip_interval - 2 * num_docs - 1) as usize),
        &payload_of(&*tp)?.len(),
    );

    reader.close()?;

    // test long payload
    let analyzer = Arc::new(PayloadAnalyzer::new());
    let conf = config(&analyzer);
    conf.set_open_mode(OpenMode::CREATE)?;
    let writer = IndexWriter::new(Arc::clone(dir), conf)?;
    let single_term = "lucene";

    let mut d = Document::new();
    d.add(text(field_name, single_term));
    // add a payload whose length is greater than the buffer size of BufferedIndexOutput
    let payload_data = Arc::new(generate_random_data(2000));
    analyzer.set_payload_data(field_name, Arc::clone(&payload_data), 100, 1500);
    writer.add_document(&d)?;

    writer.force_merge(1)?;
    // flush
    writer.close()?;

    let reader: Arc<dyn IndexReader> = directory_reader::open(Arc::clone(dir))?;
    let mut tp = postings(&reader, field_name, single_term)?;
    tp.next_doc()?;
    tp.next_position()?;

    let br = payload_of(&*tp)?;
    let _verify_payload_data = vec![0u8; br.len()];
    let mut portion = vec![0u8; 1500];
    portion.copy_from_slice(&payload_data[100..1600]);

    assert_byte_array_equals_range(&portion, &br, 0, br.len());
    reader.close()
}

/// `private void generateRandomData(byte[] data)`: this test needs the random
/// data to be valid unicode.
fn generate_random_data_into(data: &mut [u8]) {
    let s = TestUtil::random_fixed_byte_length_unicode_string(&*random(), data.len() as i32);
    let b = s.as_bytes();
    debug_assert!(b.len() == data.len());
    data[..b.len()].copy_from_slice(b);
}

/// `private byte[] generateRandomData(int n)`.
fn generate_random_data(n: i32) -> Vec<u8> {
    let mut data = vec![0u8; n as usize];
    generate_random_data_into(&mut data);
    data
}

/// `private Term[] generateTerms(String fieldName, int n)`.
///
/// `(int) (Math.log(i) / Math.log(10))` is `Integer.MIN_VALUE` for `i == 0`
/// (`Math.log(0)` is negative infinity), and the subtraction wraps as Java's
/// `int` arithmetic does.
fn generate_terms(field_name: &str, n: i32) -> Vec<Term> {
    let max_digits = ((n as f64).ln() / 10f64.ln()) as i32;
    let mut terms = Vec::with_capacity(n as usize);
    let mut sb = String::new();
    for i in 0..n {
        sb.clear();
        sb.push('t');
        let zeros = max_digits.wrapping_sub(((i as f64).ln() / 10f64.ln()) as i32);
        for _ in 0..zeros {
            sb.push('0');
        }
        sb.push_str(&i.to_string());
        terms.push(Term::from_text(field_name, &sb));
    }
    terms
}

/// `void assertByteArrayEquals(byte[] b1, byte[] b2)`.
fn assert_byte_array_equals(b1: &[u8], b2: &[u8]) {
    if b1.len() != b2.len() {
        Assert::fail(&format!(
            "Byte arrays have different lengths: {}, {}",
            b1.len(),
            b2.len()
        ));
    }

    for i in 0..b1.len() {
        if b1[i] != b2[i] {
            Assert::fail(&format!(
                "Byte arrays different at index {i}: {}, {}",
                b1[i] as i8, b2[i] as i8
            ));
        }
    }
}

/// `void assertByteArrayEquals(byte[] b1, byte[] b2, int b2offset, int
/// b2length)`.
fn assert_byte_array_equals_range(b1: &[u8], b2: &[u8], b2offset: usize, b2length: usize) {
    if b1.len() != b2length {
        Assert::fail(&format!(
            "Byte arrays have different lengths: {}, {b2length}",
            b1.len()
        ));
    }

    for i in 0..b1.len() {
        if b1[i] != b2[b2offset + i] {
            Assert::fail(&format!(
                "Byte arrays different at index {i}: {}, {}",
                b1[i] as i8,
                b2[b2offset + i] as i8
            ));
        }
    }
}

/// `static class PayloadData`.
#[derive(Debug, Clone)]
struct PayloadData {
    data: Arc<Vec<u8>>,
    offset: i32,
    length: i32,
}

/// `Map<String, PayloadData> fieldToData`, shared by the analyzer and its
/// filters.
type FieldToData = Arc<Mutex<HashMap<String, PayloadData>>>;

/// `private static class PayloadAnalyzer extends Analyzer`: this Analyzer
/// uses an MockTokenizer and PayloadFilter.
struct PayloadAnalyzer {
    state: AnalyzerState,
    field_to_data: FieldToData,
}

impl PayloadAnalyzer {
    /// `public PayloadAnalyzer()`: `super(PER_FIELD_REUSE_STRATEGY)`.
    fn new() -> Self {
        PayloadAnalyzer {
            state: AnalyzerState::new(Box::new(PerFieldReuseStrategy)),
            field_to_data: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// `void setPayloadData(String field, byte[] data, int offset, int length)`.
    fn set_payload_data(&self, field: &str, data: Arc<Vec<u8>>, offset: i32, length: i32) {
        self.field_to_data
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(
                field.to_string(),
                PayloadData {
                    data,
                    offset,
                    length,
                },
            );
    }
}

impl std::fmt::Debug for PayloadAnalyzer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PayloadAnalyzer")
    }
}

impl Analyzer for PayloadAnalyzer {
    fn analyzer_state(&self) -> &AnalyzerState {
        &self.state
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.index.TestPayloads$PayloadAnalyzer"
    }

    fn create_components(&self, field_name: &str) -> Result<TokenStreamComponents> {
        let has_payload = self
            .field_to_data
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains_key(field_name);
        let ts = Arc::new(AtomicRefCell::new(MockTokenizer::new(
            Arc::clone(&WHITESPACE),
            false,
        )?));
        if has_payload {
            let filter = payload_filter(shared(&ts), field_name, Arc::clone(&self.field_to_data))?;
            Ok(components(ts, sink(filter)))
        } else {
            Ok(tokenizer_components(ts))
        }
    }
}

/// `private static class PayloadFilter extends TokenFilter`: this Filter adds
/// payloads to the tokens.
#[derive(Debug)]
struct PayloadFilter {
    payload_att: Att,
    term_attribute: Att,
    field_to_data: FieldToData,
    field_name: String,
    payload_data: Option<PayloadData>,
    offset: i32,
}

/// `public PayloadFilter(TokenStream in, String fieldName, Map<String,
/// PayloadData> fieldToData)`.
fn payload_filter(
    mut input: Box<dyn TokenStream>,
    field_name: &str,
    field_to_data: FieldToData,
) -> Result<TokenFilter<PayloadFilter>> {
    let payload_att = Att::add(input.attribute_source_mut(), payload_id())?;
    let term_attribute = Att::add(input.attribute_source_mut(), char_term_id())?;
    Ok(TokenFilter::new(
        input,
        PayloadFilter {
            payload_att,
            term_attribute,
            field_to_data,
            field_name: field_name.to_string(),
            payload_data: None,
            offset: 0,
        },
    ))
}

impl TokenFilterLogic for PayloadFilter {
    fn increment_token(
        &mut self,
        _source: &mut AttributeSource,
        input: &mut dyn TokenStream,
    ) -> Result<bool> {
        let has_next = input.increment_token()?;
        if !has_next {
            return Ok(false);
        }

        let payload_data = self
            .payload_data
            .as_ref()
            .unwrap_or_else(|| npe("payloadData"));
        // Some values of the same field are to have payloads and others not
        if self.offset + payload_data.length <= payload_data.data.len() as i32
            && !self.term_attribute.term_string().ends_with("NO PAYLOAD")
        {
            let p = BytesRef::new(
                payload_data.data
                    [self.offset as usize..(self.offset + payload_data.length) as usize]
                    .to_vec(),
            );
            self.payload_att.payload_mut().set_payload(Some(p));
            self.offset += payload_data.length;
        } else {
            self.payload_att.payload_mut().set_payload(None);
        }

        Ok(true)
    }

    fn reset(&mut self, _source: &mut AttributeSource, input: &mut dyn TokenStream) -> Result<()> {
        input.reset()?;
        self.payload_data = self
            .field_to_data
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&self.field_name)
            .cloned();
        self.offset = self
            .payload_data
            .as_ref()
            .unwrap_or_else(|| npe("payloadData"))
            .offset;
        Ok(())
    }
}

/// Port of `TestPayloads.testThreadSafety`.
fn test_thread_safety() -> Result<()> {
    let num_threads = 5;
    let num_docs = LuceneTestCase::at_least(50);
    let pool = Arc::new(ByteArrayPool::new(num_threads, 5));

    let mut dir = LuceneTestCase::new_directory();
    let writer = IndexWriter::new(
        dir.as_directory(),
        LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(MockAnalyzer::new(
            &*random(),
        )))),
    )?;
    let field = "test";

    let mut ingesters = Vec::with_capacity(num_threads);
    for _ in 0..num_threads {
        let writer = Arc::clone(&writer);
        let pool = Arc::clone(&pool);
        let ingester = Thread::unnamed(move || {
            let outcome = (|| -> Result<()> {
                for _ in 0..num_docs {
                    let mut d = Document::new();
                    let stream: Arc<AtomicRefCell<dyn TokenStream>> = Arc::new(AtomicRefCell::new(
                        PoolingPayloadTokenStream::new(Arc::clone(&pool))?,
                    ));
                    d.add(Box::new(TextField::new_with_token_stream(field, stream)?));
                    writer.add_document(&d)?;
                }
                Ok(())
            })();
            if let Err(e) = outcome {
                let t = java_throwable(&e);
                eprintln!("{t}");
                Assert::fail(&t.to_string());
            }
        });
        ingester.start();
        ingesters.push(ingester);
    }

    for ingester in &ingesters {
        if let Err(payload) = ingester.join() {
            std::panic::resume_unwind(payload);
        }
    }
    writer.close()?;
    let reader: Arc<dyn IndexReader> = directory_reader::open(dir.as_directory())?;
    let mut terms = MultiTerms::get_terms(&reader, field)?
        .unwrap_or_else(|| npe("MultiTerms.getTerms"))
        .iterator()?;
    let mut tp: Option<Box<dyn PostingsEnum>> = None;
    while terms.next()?.is_some() {
        let term_text = terms.term()?.utf8_to_string()?;
        let mut p = terms.postings(tp.take(), POSTINGS_ENUM_PAYLOADS)?;
        while p.next_doc()? != NO_MORE_DOCS {
            let freq = p.freq()?;
            for _ in 0..freq {
                p.next_position()?;
                let payload = BytesRef::new(payload_of(&*p)?);
                assert_equals(&term_text, &payload.utf8_to_string()?);
            }
        }
        tp = Some(p);
    }
    reader.close()?;
    dir.close()?;
    assert_equals(&pool.size(), &num_threads);
    Ok(())
}

/// `private class PoolingPayloadTokenStream extends TokenStream`.
struct PoolingPayloadTokenStream {
    source: AttributeSource,
    payload: Vec<u8>,
    first: bool,
    pool: Arc<ByteArrayPool>,
    term: String,
    term_att: Att,
    payload_att: Att,
}

impl std::fmt::Debug for PoolingPayloadTokenStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PoolingPayloadTokenStream")
    }
}

impl PoolingPayloadTokenStream {
    /// `PoolingPayloadTokenStream(ByteArrayPool pool)`.
    fn new(pool: Arc<ByteArrayPool>) -> Result<Self> {
        let mut payload = pool.get();
        generate_random_data_into(&mut payload);
        let term = String::from_utf8_lossy(&payload).into_owned();
        let mut source =
            AttributeSource::new_with_factory(rucene::analysis::default_token_attribute_factory());
        let payload_att = Att::add(&mut source, payload_id())?;
        let term_att = Att::add(&mut source, char_term_id())?;
        Ok(PoolingPayloadTokenStream {
            source,
            payload,
            first: true,
            pool,
            term,
            term_att,
            payload_att,
        })
    }
}

impl TokenStream for PoolingPayloadTokenStream {
    fn increment_token(&mut self) -> Result<bool> {
        if !self.first {
            return Ok(false);
        }
        self.first = false;
        self.source.clear_attributes();
        self.term_att
            .char_term_mut()
            .append_string(Some(&self.term));
        self.payload_att
            .payload_mut()
            .set_payload(Some(BytesRef::new(self.payload.clone())));
        Ok(true)
    }

    fn close(&mut self) -> Result<()> {
        // `pool.release(payload)`: the array goes back to the pool (a copy of
        // it here, the stream keeping its own).
        self.pool.release(self.payload.clone());
        Ok(())
    }

    fn attribute_source(&self) -> &AttributeSource {
        &self.source
    }

    fn attribute_source_mut(&mut self) -> &mut AttributeSource {
        &mut self.source
    }
}

/// `private static class ByteArrayPool`.
struct ByteArrayPool {
    pool: Mutex<Vec<Vec<u8>>>,
}

impl ByteArrayPool {
    fn new(capacity: usize, size: usize) -> Self {
        let mut pool = Vec::new();
        for _ in 0..capacity {
            pool.push(vec![0u8; size]);
        }
        ByteArrayPool {
            pool: Mutex::new(pool),
        }
    }

    /// `synchronized byte[] get()`: `pool.remove(0)`.
    fn get(&self) -> Vec<u8> {
        let mut pool = self.pool.lock().unwrap_or_else(|e| e.into_inner());
        if pool.is_empty() {
            randomizedtesting::jdk::exception::Throwable::new(
                "java.lang.IndexOutOfBoundsException",
                "Index 0 out of bounds for length 0",
            )
            .throw()
        }
        pool.remove(0)
    }

    /// `synchronized void release(byte[] b)`.
    fn release(&self, b: Vec<u8>) {
        self.pool.lock().unwrap_or_else(|e| e.into_inner()).push(b);
    }

    /// `synchronized int size()`.
    fn size(&self) -> usize {
        self.pool.lock().unwrap_or_else(|e| e.into_inner()).len()
    }
}

/// Port of `TestPayloads.testAcrossFields`.
fn test_across_fields() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let writer = RandomIndexWriter::new(
        &*random(),
        dir.as_directory(),
        Arc::new(MockAnalyzer::with_automaton(
            &*random(),
            Arc::clone(&WHITESPACE),
            true,
        )),
    )?;
    let mut doc = Document::new();
    doc.add(Box::new(TextField::new(
        "hasMaybepayload",
        "here we go".to_string(),
        Store::YES,
    )?));
    writer.add_document(&doc)?;
    writer.close()?;

    let writer = RandomIndexWriter::new(
        &*random(),
        dir.as_directory(),
        Arc::new(MockAnalyzer::with_automaton(
            &*random(),
            Arc::clone(&WHITESPACE),
            true,
        )),
    )?;
    let mut doc = Document::new();
    doc.add(Box::new(TextField::new(
        "hasMaybepayload2",
        "here we go".to_string(),
        Store::YES,
    )?));
    writer.add_document(&doc)?;
    writer.add_document(&doc)?;
    writer.force_merge(1)?;
    writer.close()?;

    dir.close()
}

/// `new MockTokenizer(MockTokenizer.WHITESPACE, true)` with
/// `setReader(new StringReader(text))`, as a token stream value.
fn whitespace_stream(text: &str) -> Result<Arc<AtomicRefCell<dyn TokenStream>>> {
    let mut ts = MockTokenizer::new(Arc::clone(&WHITESPACE), true)?;
    ts.set_reader(Box::new(StringCharReader::new(text)))?;
    Ok(Arc::new(AtomicRefCell::new(ts)))
}

/// `new CannedTokenStream(withPayload)`, `withPayload` being `new
/// Token("withPayload", 0, 11)` with the payload `"test"`; checks
/// `ts.hasAttribute(PayloadAttribute.class)`.
fn with_payload_stream() -> Arc<AtomicRefCell<dyn TokenStream>> {
    let mut with_payload = Token::with_offsets("withPayload", 0, 11);
    with_payload.set_payload(Some(BytesRef::from_char_sequence("test")));
    let ts = CannedTokenStream::new(vec![with_payload]);
    Assert::assert_true(ts.attribute_source().has_attribute_by_id(payload_id()));
    Arc::new(AtomicRefCell::new(ts))
}

/// `de.getPayload()` as the `BytesRef` Java compares.
fn payload_ref(de: &dyn PostingsEnum) -> Result<Option<BytesRef>> {
    Ok(de.get_payload()?.map(|p| BytesRef::new(p.to_vec())))
}

/// Port of `TestPayloads.testMixupDocs`: some docs have payload att, some
/// not.
fn test_mixup_docs() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let iwc = LuceneTestCase::new_index_writer_config_analyzer(None);
    iwc.set_merge_policy(Some(Arc::new(LuceneTestCase::new_log_merge_policy())))?;
    let writer = RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), iwc)?;
    let mut doc = Document::new();
    let ts = whitespace_stream("here we go")?;
    let field = Field::new_with_token_stream("field", ts, TextField::type_not_stored().clone())?;
    doc.add(Box::new(field));
    writer.add_document(&doc)?;
    let ts = with_payload_stream();
    field_mut(&mut doc, "field").set_token_stream_value(ts)?;
    writer.add_document(&doc)?;
    let ts = whitespace_stream("another")?;
    field_mut(&mut doc, "field").set_token_stream_value(ts)?;
    writer.add_document(&doc)?;
    let reader = writer.get_reader()?;
    let ir: Arc<dyn IndexReader> = Arc::clone(&reader) as Arc<dyn IndexReader>;
    let mut te = MultiTerms::get_terms(&ir, "field")?
        .unwrap_or_else(|| npe("MultiTerms.getTerms"))
        .iterator()?;
    Assert::assert_true(te.seek_exact(&BytesRef::from_char_sequence("withPayload"))?);
    let mut de = te.postings(None, POSTINGS_ENUM_PAYLOADS)?;
    de.next_doc()?;
    de.next_position()?;
    assert_equals(
        &Some(BytesRef::from_char_sequence("test")),
        &payload_ref(&*de)?,
    );
    writer.close()?;
    reader.close()?;
    dir.close()
}

/// Port of `TestPayloads.testMixupMultiValued`: some field instances have
/// payload att, some not.
fn test_mixup_multi_valued() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let writer = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    let mut doc = Document::new();
    let ts = whitespace_stream("here we go")?;
    let field = Field::new_with_token_stream(
        "field",
        Arc::clone(&ts),
        TextField::type_not_stored().clone(),
    )?;
    field.set_token_stream_value(ts)?;
    doc.add(Box::new(field));
    let ts = with_payload_stream();
    let field2 = Field::new_with_token_stream("field", ts, TextField::type_not_stored().clone())?;
    doc.add(Box::new(field2));
    let ts = whitespace_stream("nopayload")?;
    let field3 = Field::new_with_token_stream("field", ts, TextField::type_not_stored().clone())?;
    doc.add(Box::new(field3));
    writer.add_document(&doc)?;
    let reader = writer.get_reader()?;
    let sr = LuceneTestCase::get_only_leaf_reader(Arc::clone(&reader) as Arc<dyn IndexReader>)?;
    let mut de = sr
        .postings(
            &Term::from_text("field", "withPayload"),
            POSTINGS_ENUM_PAYLOADS,
        )?
        .unwrap_or_else(|| npe("postings"));
    de.next_doc()?;
    de.next_position()?;
    assert_equals(
        &Some(BytesRef::from_char_sequence("test")),
        &payload_ref(&*de)?,
    );
    writer.close()?;
    reader.close()?;
    dir.close()
}
