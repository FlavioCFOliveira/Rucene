//! Port of `lucene/core/src/test/org/apache/lucene/index/TestDuelingCodecs.java`.
//!
//! Compares one codec against another

use std::sync::Arc;

use randomizedtesting::jdk::random::{JdkRandom, Random};
use randomizedtesting::jdk::reflect::{Class, ClassBuilder, Constructor, Method};
use rucene::codecs::Codec;
use rucene::document::{
    NumericDocValuesField, SortedDocValuesField, SortedNumericDocValuesField,
    SortedSetDocValuesField,
};
use rucene::error::Result;
use rucene::index::index_writer::MAX_TERM_LENGTH;
use rucene::index::IndexReader;
use rucene::util::BytesRef;
use rucene_codecs::simpletext::SimpleTextCodec;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::index::random_codec::RandomCodec;
use rucene_test_framework::index::RandomIndexWriter;
use rucene_test_framework::util::java_exceptions::rethrow_java;
use rucene_test_framework::util::lucene_test_case::{TestDirectory, TestReader};
use rucene_test_framework::util::{LineFileDocs, LuceneTestCase, TestUtil};

use crate::core_support::random;

/// The fields of `TestDuelingCodecs`, set by `setUp()`.
#[derive(Default)]
pub struct DuelingCodecsInstance {
    pub left_dir: Option<TestDirectory>,
    pub left_reader: Option<Arc<dyn IndexReader>>,
    pub left_codec: Option<Arc<dyn Codec>>,

    pub right_dir: Option<TestDirectory>,
    pub right_reader: Option<Arc<dyn IndexReader>>,
    pub right_codec: Option<Arc<dyn Codec>>,
    pub left_writer: Option<RandomIndexWriter>,
    pub right_writer: Option<RandomIndexWriter>,
    pub seed: i64,
    /// for debugging
    pub info: String,
}

impl DuelingCodecsInstance {
    /// `leftWriter`.
    pub fn left_writer(&self) -> &RandomIndexWriter {
        self.left_writer
            .as_ref()
            .unwrap_or_else(|| crate::support::npe("leftWriter"))
    }

    /// `rightWriter`.
    pub fn right_writer(&self) -> &RandomIndexWriter {
        self.right_writer
            .as_ref()
            .unwrap_or_else(|| crate::support::npe("rightWriter"))
    }

    /// `leftReader`.
    pub fn left_reader(&self) -> &Arc<dyn IndexReader> {
        self.left_reader
            .as_ref()
            .unwrap_or_else(|| crate::support::npe("leftReader"))
    }

    /// `rightReader`.
    pub fn right_reader(&self) -> &Arc<dyn IndexReader> {
        self.right_reader
            .as_ref()
            .unwrap_or_else(|| crate::support::npe("rightReader"))
    }
}

/// A test method of the class (or of a subclass) over its instance.
pub fn instance_test(name: &str, body: fn(&mut DuelingCodecsInstance) -> Result<()>) -> Method {
    Method::new(name).throws(&["java.lang.Exception"]).body(
        move |inst: &mut DuelingCodecsInstance| body(inst).unwrap_or_else(|e| rethrow_java(e)),
    )
}

/// The builder of `TestDuelingCodecs`, with its `setUp`, `tearDown` and
/// test methods.
fn builder() -> ClassBuilder {
    Class::builder("org.apache.lucene.index.TestDuelingCodecs")
        .extends(&LuceneTestCase::class())
        .constructor(Constructor::no_args(DuelingCodecsInstance::default))
        .method(Method::new("setUp").throws(&["java.lang.Exception"]).body(
            |inst: &mut DuelingCodecsInstance| set_up(inst).unwrap_or_else(|e| rethrow_java(e)),
        ))
        .method(
            Method::new("tearDown")
                .throws(&["java.lang.Exception"])
                .body(|inst: &mut DuelingCodecsInstance| {
                    tear_down(inst).unwrap_or_else(|e| rethrow_java(e))
                }),
        )
        .method(instance_test("testEquals", test_equals))
        .method(instance_test(
            "testCrazyReaderEquals",
            test_crazy_reader_equals,
        ))
}

/// `TestDuelingCodecs`, which `TestDuelingCodecsAtNight` extends.
pub fn class_def() -> Arc<Class> {
    builder().build()
}

/// `setUp()`.
fn set_up(inst: &mut DuelingCodecsInstance) -> Result<()> {
    LuceneTestCase::super_set_up();

    // for now it's SimpleText vs Default(random postings format)
    // as this gives the best overall coverage. when we have more
    // codecs we should probably pick 2 from Codec.availableCodecs()

    let left_codec: Arc<dyn Codec> = Arc::new(SimpleTextCodec::new());
    let random_codec = RandomCodec::new(&*random());
    // `rightCodec.toString()`: `RandomCodec` overrides it.
    let right_codec_string = random_codec.to_string();
    let right_codec: Arc<dyn Codec> = Arc::new(random_codec);

    let left_dir =
        LuceneTestCase::new_fs_directory(&LuceneTestCase::create_temp_dir_prefix("leftDir"));
    let right_dir =
        LuceneTestCase::new_fs_directory(&LuceneTestCase::create_temp_dir_prefix("rightDir"));

    inst.seed = random().next_long();

    // must use same seed because of random payloads, etc
    let max_term_length = TestUtil::next_int(&*random(), 1, MAX_TERM_LENGTH as i32);
    let left_analyzer = MockAnalyzer::new(&JdkRandom::new(inst.seed));
    left_analyzer.set_max_token_length(max_term_length);
    let right_analyzer = MockAnalyzer::new(&JdkRandom::new(inst.seed));
    right_analyzer.set_max_token_length(max_term_length);

    // but these can be different
    // TODO: this turns this into a really big test of Multi*, is that what we want?
    let left_config =
        LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(left_analyzer)));
    left_config.set_codec(Arc::clone(&left_codec))?;
    // preserve docids
    left_config.set_merge_policy(Some(Arc::new(LuceneTestCase::new_log_merge_policy())))?;

    let right_config =
        LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(right_analyzer)));
    right_config.set_codec(Arc::clone(&right_codec))?;
    // preserve docids
    right_config.set_merge_policy(Some(Arc::new(LuceneTestCase::new_log_merge_policy())))?;

    // must use same seed because of random docvalues fields, etc
    inst.left_writer = Some(RandomIndexWriter::new_with_config(
        &JdkRandom::new(inst.seed),
        left_dir.as_directory(),
        left_config,
    )?);
    inst.right_writer = Some(RandomIndexWriter::new_with_config(
        &JdkRandom::new(inst.seed),
        right_dir.as_directory(),
        right_config,
    )?);

    // `Codec.toString()` is the codec's name.
    inst.info = format!("left: {} / right: {right_codec_string}", left_codec.name());
    inst.left_codec = Some(left_codec);
    inst.right_codec = Some(right_codec);
    inst.left_dir = Some(left_dir);
    inst.right_dir = Some(right_dir);
    Ok(())
}

/// `tearDown()`: `IOUtils.close(leftWriter, rightWriter, leftReader,
/// rightReader, leftDir, rightDir); super.tearDown();`.
fn tear_down(inst: &mut DuelingCodecsInstance) -> Result<()> {
    let mut first: Option<rucene::error::LuceneError> = None;
    let mut keep = |r: Result<()>| {
        if let Err(e) = r {
            first.get_or_insert(e);
        }
    };
    if let Some(w) = inst.left_writer.take() {
        keep(w.close());
    }
    if let Some(w) = inst.right_writer.take() {
        keep(w.close());
    }
    if let Some(r) = inst.left_reader.take() {
        keep(r.close());
    }
    if let Some(r) = inst.right_reader.take() {
        keep(r.close());
    }
    if let Some(mut d) = inst.left_dir.take() {
        keep(d.close());
    }
    if let Some(mut d) = inst.right_dir.take() {
        keep(d.close());
    }
    if let Some(e) = first {
        return Err(e);
    }
    LuceneTestCase::super_tear_down();
    Ok(())
}

/// `title.split("\\s+")`: Java's `String.split` over runs of `\s`
/// (`[ \t\n\x0B\f\r]`), trailing empty strings removed.
fn split_whitespace(s: &str) -> Vec<String> {
    let is_ws = |c: char| matches!(c, ' ' | '\t' | '\n' | '\u{0B}' | '\u{0C}' | '\r');
    if !s.contains(is_ws) {
        return vec![s.to_string()];
    }
    let mut parts: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut in_ws = false;
    for c in s.chars() {
        if is_ws(c) {
            if !in_ws {
                parts.push(std::mem::take(&mut current));
                in_ws = true;
            }
        } else {
            in_ws = false;
            current.push(c);
        }
    }
    parts.push(current);
    while parts.last().is_some_and(String::is_empty) {
        parts.pop();
    }
    parts
}

/// populates a writer with random stuff. this must be fully reproducable with the seed!
pub fn create_random_index(numdocs: i32, writer: &RandomIndexWriter, seed: i64) -> Result<()> {
    let random = JdkRandom::new(seed);
    // primary source for our data is from linefiledocs, it's realistic.
    let line_file_docs = LineFileDocs::with_default_file(&random)?;

    // TODO: we should add other fields that use things like docs&freqs but omit positions,
    // because linefiledocs doesn't cover all the possibilities.
    for _ in 0..numdocs {
        let document = line_file_docs.next_doc()?;
        let mut document = document.borrow_mut();
        // grab the title and add some SortedSet instances for fun
        let title = document
            .get("titleTokenized")
            .unwrap_or_else(|| crate::support::npe("title"));
        let split = split_whitespace(&title);
        document.remove_fields("sortedset");
        for trash in &split {
            document.add(Box::new(SortedSetDocValuesField::new(
                "sortedset",
                BytesRef::from_char_sequence(trash),
            )));
        }
        // add a sorted dv field sometimes
        document.remove_fields("sparseset");
        if random.next_boolean() {
            document.add(Box::new(SortedDocValuesField::new(
                "sparseset",
                BytesRef::from_char_sequence(&title),
            )));
        }
        // add a numeric dv field sometimes
        document.remove_fields("sparsenumeric");
        if random.next_int_bound(4) == 2 {
            document.add(Box::new(NumericDocValuesField::new(
                "sparsenumeric",
                random.next_int() as i64,
            )));
        }
        // add sortednumeric sometimes
        document.remove_fields("sparsesortednum");
        if random.next_int_bound(5) == 1 {
            document.add(Box::new(SortedNumericDocValuesField::new(
                "sparsesortednum",
                random.next_long(),
            )));
            if random.next_boolean() {
                document.add(Box::new(SortedNumericDocValuesField::new(
                    "sparsesortednum",
                    random.next_long(),
                )));
            }
        }
        writer.add_document(&*document)?;
    }

    line_file_docs.close();
    Ok(())
}

/// checks the two indexes are equivalent
/// Port of `TestDuelingCodecs.testEquals`.
// we use a small amount of docs here, so it works with any codec
fn test_equals(inst: &mut DuelingCodecsInstance) -> Result<()> {
    let numdocs = LuceneTestCase::at_least(20);
    create_random_index(numdocs, inst.left_writer(), inst.seed)?;
    create_random_index(numdocs, inst.right_writer(), inst.seed)?;

    inst.left_reader = Some(inst.left_writer().get_reader()? as Arc<dyn IndexReader>);
    inst.right_reader = Some(inst.right_writer().get_reader()? as Arc<dyn IndexReader>);

    LuceneTestCase::assert_reader_equals(&inst.info, inst.left_reader(), inst.right_reader());
    Ok(())
}

/// Port of `TestDuelingCodecs.testCrazyReaderEquals`.
fn test_crazy_reader_equals(inst: &mut DuelingCodecsInstance) -> Result<()> {
    let numdocs = LuceneTestCase::at_least(20);
    create_random_index(numdocs, inst.left_writer(), inst.seed)?;
    create_random_index(numdocs, inst.right_writer(), inst.seed)?;

    inst.left_reader = Some(
        LuceneTestCase::wrap_reader(TestReader::from(inst.left_writer().get_reader()?))?
            .index_reader(),
    );
    inst.right_reader = Some(
        LuceneTestCase::wrap_reader(TestReader::from(inst.right_writer().get_reader()?))?
            .index_reader(),
    );

    // check that our readers are valid
    TestUtil::check_reader(inst.left_reader())?;
    TestUtil::check_reader(inst.right_reader())?;

    LuceneTestCase::assert_reader_equals(&inst.info, inst.left_reader(), inst.right_reader());
    Ok(())
}
