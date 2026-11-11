//! Port of `lucene/core/src/test/org/apache/lucene/index/TestMultiLevelSkipList.java`.
//!
//! This testcase tests whether multi-level skipping is being used to reduce I/O while skipping
//! through posting lists.
//!
//! Skipping in general is already covered by several other testcases.
//!
//! * `CountingDirectory extends MockDirectoryWrapper` overrides `openInput`:
//!   a `MockDirectoryWrapper` with that override installed
//!   (`set_open_input_override`).
//! * `CountingStream extends FilterIndexInput` counts the bytes it reads
//!   into the test's `counter`, shared through an `Arc`.

use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;

use randomizedtesting::jdk::reflect::{Class, Constructor, Method};
use randomizedtesting::junit::Assert;
use rucene::analysis::{
    Analyzer, AnalyzerState, GlobalReuseStrategy, TokenFilter, TokenFilterLogic, TokenStream,
    TokenStreamComponents,
};
use rucene::document::{Document, Store};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::terms::Term;
use rucene::index::{PostingsEnum, POSTINGS_ENUM_ALL};
use rucene::store::{ByteBuffersDirectory, DataInput, IndexInput};
use rucene::util::atomic_ref_cell::AtomicRefCell;
use rucene::util::attribute::AttributeSource;
use rucene::util::BytesRef;
use rucene_test_framework::analysis::attributes::{payload_id, Att};
use rucene_test_framework::analysis::mock_tokenizer::MockTokenizer;
use rucene_test_framework::analysis::{components, shared, sink};
use rucene_test_framework::store::MockDirectoryWrapper;
use rucene_test_framework::util::java_exceptions::rethrow_java;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::{assert_equals, assert_equals_msg, random};
use crate::support::npe;

/// The instance field `counter` of `TestMultiLevelSkipList`.
#[derive(Default)]
struct Instance {
    counter: Arc<AtomicI32>,
}

/// `TestMultiLevelSkipList`.
pub fn class_def() -> Arc<Class> {
    Class::builder("org.apache.lucene.index.TestMultiLevelSkipList")
        .extends(&LuceneTestCase::class())
        .constructor(Constructor::no_args(Instance::default))
        .method(Method::new("setUp").throws(&["java.lang.Exception"]).body(
            |inst: &mut Instance| {
                LuceneTestCase::super_set_up();
                inst.counter.store(0, Ordering::SeqCst);
            },
        ))
        .method(
            Method::new("testSimpleSkip")
                .throws(&["java.io.IOException"])
                .body(|inst: &mut Instance| {
                    test_simple_skip(inst).unwrap_or_else(|e| rethrow_java(e))
                }),
        )
        .build()
}

/// `new CountingDirectory(delegate)`: `super(random(), delegate)`, with the
/// `openInput` override.
fn counting_directory(this: &Instance) -> MockDirectoryWrapper {
    let dir = MockDirectoryWrapper::new(&*random(), Arc::new(ByteBuffersDirectory::new()));
    let counter = Arc::clone(&this.counter);
    dir.set_open_input_override(Some(Arc::new(move |file_name, r#in| {
        if file_name.ends_with(".frq") {
            Ok(Box::new(CountingStream::new(r#in, Arc::clone(&counter))) as Box<dyn IndexInput>)
        } else {
            Ok(r#in)
        }
    })));
    dir
}

/// Port of `TestMultiLevelSkipList.testSimpleSkip`.
fn test_simple_skip(this: &mut Instance) -> Result<()> {
    let dir = counting_directory(this);
    let iwc =
        LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(PayloadAnalyzer::new())));
    iwc.set_codec(TestUtil::always_postings_format(
        TestUtil::get_default_postings_format(),
    ))?;
    iwc.set_merge_policy(Some(Arc::new(LuceneTestCase::new_log_merge_policy())))?;
    let writer = IndexWriter::new(dir.as_directory(), iwc)?;
    let term = Term::from_text("test", "a");
    for _ in 0..5000 {
        let mut d1 = Document::new();
        d1.add(Box::new(LuceneTestCase::new_text_field(
            term.field(),
            &term.text(),
            Store::NO,
        )));
        writer.add_document(&d1)?;
    }
    writer.commit()?;
    writer.force_merge(1)?;
    writer.close()?;
    drop(writer);

    let reader = LuceneTestCase::get_only_leaf_reader(directory_reader::open(dir.as_directory())?)?;

    for _ in 0..2 {
        this.counter.store(0, Ordering::SeqCst);
        let mut tp = reader
            .postings(&term, POSTINGS_ENUM_ALL)?
            .unwrap_or_else(|| npe("tp"));
        check_skip_to(this, &mut *tp, 14, 185)?; // no skips
        check_skip_to(this, &mut *tp, 17, 190)?; // one skip on level 0
        check_skip_to(this, &mut *tp, 287, 200)?; // one skip on level 1, two on level 0

        // this test would fail if we had only one skip level,
        // because than more bytes would be read from the freqStream
        check_skip_to(this, &mut *tp, 4800, 250)?; // one skip on level 2
    }
    Ok(())
}

/// Port of `TestMultiLevelSkipList.checkSkipTo`.
fn check_skip_to(
    this: &Instance,
    tp: &mut dyn PostingsEnum,
    target: i32,
    max_counter: i32,
) -> Result<()> {
    tp.advance(target)?;
    let counter = this.counter.load(Ordering::SeqCst);
    if max_counter < counter {
        Assert::fail(&format!("Too many bytes read: {counter} vs {max_counter}"));
    }

    assert_equals_msg(
        Some(&format!(
            "Wrong document {} after skipTo target {target}",
            tp.doc_id()
        )),
        &target,
        &tp.doc_id(),
    );
    assert_equals_msg(
        Some(&format!("Frequency is not 1: {}", tp.freq()?)),
        &1,
        &tp.freq()?,
    );
    tp.next_position()?;
    let b: Vec<u8> = tp.get_payload()?.unwrap_or_else(|| npe("payload")).to_vec();
    assert_equals(&1, &b.len());
    let byte = b[0] as i8;
    assert_equals_msg(
        Some(&format!("Wrong payload for the target {target}: {byte}")),
        &(target as i8),
        &byte,
    );
    Ok(())
}

/// `private static class PayloadAnalyzer extends Analyzer`.
struct PayloadAnalyzer {
    state: AnalyzerState,
    payload_count: Arc<AtomicI32>,
}

impl PayloadAnalyzer {
    fn new() -> Self {
        PayloadAnalyzer {
            state: AnalyzerState::new(Box::new(GlobalReuseStrategy)),
            payload_count: Arc::new(AtomicI32::new(-1)),
        }
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
        "org.apache.lucene.index.TestMultiLevelSkipList$PayloadAnalyzer"
    }

    fn create_components(&self, _field_name: &str) -> Result<TokenStreamComponents> {
        let tokenizer = Arc::new(AtomicRefCell::new(MockTokenizer::whitespace_lower_case()?));
        let filter = payload_filter(Arc::clone(&self.payload_count), shared(&tokenizer))?;
        Ok(components(tokenizer, sink(filter)))
    }
}

/// `private static class PayloadFilter extends TokenFilter`.
#[derive(Debug)]
struct PayloadFilter {
    payload_att: Att,
    payload_count: Arc<AtomicI32>,
}

/// `new PayloadFilter(payloadCount, input)`: `addAttribute(PayloadAttribute.class)`.
fn payload_filter(
    payload_count: Arc<AtomicI32>,
    mut input: Box<dyn TokenStream>,
) -> Result<TokenFilter<PayloadFilter>> {
    let payload_att = Att::add(input.attribute_source_mut(), payload_id())?;
    Ok(TokenFilter::new(
        input,
        PayloadFilter {
            payload_att,
            payload_count,
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
        if has_next {
            let count = self.payload_count.fetch_add(1, Ordering::SeqCst) + 1;
            self.payload_att
                .payload_mut()
                .set_payload(Some(BytesRef::new(vec![count as u8])));
        }
        Ok(has_next)
    }
}

/// `class CountingStream extends FilterIndexInput`.
struct CountingStream {
    resource_description: String,
    r#in: Box<dyn IndexInput>,
    counter: Arc<AtomicI32>,
}

impl CountingStream {
    /// `CountingStream(IndexInput input)`: `super("CountingStream(" + input + ")", input)`.
    fn new(input: Box<dyn IndexInput>, counter: Arc<AtomicI32>) -> Self {
        CountingStream {
            resource_description: format!("CountingStream({})", input.to_java_string()),
            r#in: input,
            counter,
        }
    }
}

impl DataInput for CountingStream {
    fn to_java_string(&self) -> String {
        self.resource_description.clone()
    }

    fn read_byte(&mut self) -> Result<u8> {
        self.counter.fetch_add(1, Ordering::SeqCst);
        self.r#in.read_byte()
    }

    fn read_bytes(&mut self, b: Option<&mut [u8]>, offset: usize, len: usize) -> Result<()> {
        self.counter.fetch_add(len as i32, Ordering::SeqCst);
        self.r#in.read_bytes(b, offset, len)
    }

    /// `IndexInput.skipBytes`: `seek(getFilePointer() + numBytes)`.
    fn skip_bytes(&mut self, num_bytes: i64) -> Result<()> {
        let pos = self.r#in.file_pointer()? + num_bytes;
        self.r#in.seek(pos)
    }
}

impl IndexInput for CountingStream {
    fn close(&mut self) -> Result<()> {
        self.r#in.close()
    }

    fn file_pointer(&self) -> Result<i64> {
        self.r#in.file_pointer()
    }

    fn length(&self) -> i64 {
        self.r#in.length()
    }

    fn seek(&mut self, pos: i64) -> Result<()> {
        self.r#in.seek(pos)
    }

    fn slice(
        &self,
        slice_description: &str,
        offset: i64,
        length: i64,
    ) -> Result<Box<dyn IndexInput>> {
        Ok(Box::new(CountingStream::new(
            self.r#in.slice(slice_description, offset, length)?,
            Arc::clone(&self.counter),
        )))
    }

    fn clone_input(&self) -> Result<Box<dyn IndexInput>> {
        Ok(Box::new(CountingStream::new(
            self.r#in.clone_input()?,
            Arc::clone(&self.counter),
        )))
    }

    fn resource_description(&self) -> &str {
        &self.resource_description
    }
}
