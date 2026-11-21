//! The `BaseCompoundFormatTestCase` suites (rmp #352): the inherited cases
//! of `BaseIndexFileFormatTestCase` and `BaseCompoundFormatTestCase` run,
//! under the ported runner and `LuceneTestCase`'s rule chain, on the codec
//! Lucene's own subclass binds:
//!
//! * `org.apache.lucene.codecs.lucene90.TestLucene90CompoundFormat`
//!   (`lucene/core/src/test`): `private final Codec codec =
//!   TestUtil.getDefaultCodec()`.
//!
//! The own test method of `TestLucene90CompoundFormat`
//! (`testFileLengthOrdering`, sprint 9) is declared on its binding.
//!
//! Each suite runs once on the seed `tests.seed` names (a random one when it
//! is unset); `RUCENE_SUITE_SEEDS=<n>` runs it on the first `n` seeds of
//! `suite_seeds::SEEDS` instead, reporting the pass counts.

use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::exception;
use randomizedtesting::jdk::random::Random;
use randomizedtesting::jdk::reflect::{modifier, Class, Constructor, Method};
use randomizedtesting::jdk::util::shuffle;
use randomizedtesting::junit::Assert;
use rucene::codecs::codec_util;
use rucene::codecs::lucene90::compound;
use rucene::codecs::Codec;
use rucene::document::Document;
use rucene::error::Result;
use rucene::index::index_file_names::{segment_file_name, COMPOUND_FILE_ENTRIES_EXTENSION};
use rucene::store::{DataInput, IndexInput, DEFAULT_IO_CONTEXT};
use rucene_test_framework::index::base_compound_format_test_case::{
    self, BaseCompoundFormatTestCase,
};
use rucene_test_framework::index::base_index_file_format_test_case::{
    format_test, tear_down, BaseIndexFileFormatTestCase, FormatTestInstance,
};
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

mod suite_draws;
mod suite_seeds;
use suite_seeds::run_suite;

/// A compound suite whose `codec` field holds the codec `getCodec()` returns,
/// overriding `shouldTestMergeInstance()` when `merge_instance` is set.
struct CompoundSuite {
    codec: Arc<dyn Codec>,
    merge_instance: bool,
}

impl BaseIndexFileFormatTestCase for CompoundSuite {
    fn get_codec(&self) -> Arc<dyn Codec> {
        Arc::clone(&self.codec)
    }

    fn should_test_merge_instance(&self) -> bool {
        self.merge_instance
    }

    fn add_random_fields(&self, doc: &mut Document) {
        self.compound_add_random_fields(doc);
    }

    fn as_compound(&self) -> Option<&dyn BaseCompoundFormatTestCase> {
        Some(self)
    }
}

impl BaseCompoundFormatTestCase for CompoundSuite {}

/// `TestLucene90CompoundFormat`: `private final Codec codec =
/// TestUtil.getDefaultCodec()`.
fn lucene90_class() -> Arc<Class> {
    Class::builder("org.apache.lucene.codecs.lucene90.TestLucene90CompoundFormat")
        .extends(&base_compound_format_test_case::class())
        .constructor(Constructor::no_args(|| {
            FormatTestInstance::new(Arc::new(CompoundSuite {
                codec: TestUtil::get_default_codec(),
                merge_instance: false,
            }))
        }))
        .method(format_test("testFileLengthOrdering", |_| {
            test_file_length_ordering()
        }))
        .build()
}

/// Port of `TestLucene90CompoundFormat.testFileLengthOrdering`.
fn test_file_length_ordering() -> Result<()> {
    let directory = LuceneTestCase::new_directory();
    let dir = directory.as_directory();
    // Setup the test segment
    let segment = "_123";
    let chunk = 1024; // internal buffer size used by the stream
    let si = base_compound_format_test_case::new_segment_info(&dir, segment)?;
    let seg_id = si.id();
    let mut ordered_files: Vec<String> = Vec::new();
    let mut random_file_size = LuceneTestCase::random().next_int_range(0, chunk);
    for i in 0..10 {
        let filename = format!("{segment}.{i}");
        base_compound_format_test_case::create_random_file(
            &*dir,
            &filename,
            random_file_size,
            &seg_id,
        )?;
        // increase the next files size by a random amount
        random_file_size += LuceneTestCase::random().next_int_range(1, 100);
        ordered_files.push(filename);
    }
    let mut shuffled_files = ordered_files.clone();
    shuffle(&mut shuffled_files, &*LuceneTestCase::random());
    si.set_files(shuffled_files)?;
    si.codec()
        .unwrap_or_else(|| exception::throw_without_message("java.lang.NullPointerException"))
        .compound_format()
        .write(&*dir, &si, &*DEFAULT_IO_CONTEXT)?;

    // entries file should contain files ordered by their size
    let entries_file_name = segment_file_name(&si.name, "", COMPOUND_FILE_ENTRIES_EXTENSION);
    {
        let mut entries_stream = dir.open_checksum_input(&entries_file_name)?;
        let result = (|| -> Result<()> {
            codec_util::check_index_header(
                &mut *entries_stream,
                compound::ENTRY_CODEC,
                compound::VERSION_START,
                compound::VERSION_CURRENT,
                &si.id(),
                "",
            )?;
            let num_entries = entries_stream.read_v_int()?;
            let mut last_offset = 0i64;
            let mut last_length = 0i64;
            #[allow(clippy::needless_range_loop)] // Java's indexed loop, kept as the test writes it
            for i in 0..num_entries as usize {
                let id = entries_stream.read_string()?;
                Assert::assert_equals_str(
                    None,
                    Some(&ordered_files[i]),
                    Some(&format!("{segment}{id}")),
                );
                let offset = entries_stream.read_long()?;
                Assert::assert_true(offset > last_offset);
                last_offset = offset;
                let length = entries_stream.read_long()?;
                Assert::assert_true(length >= last_length);
                last_length = length;
            }
            Ok(())
        })();
        // `catch (Throwable) { priorE = exception; } finally { checkFooter(entriesStream, priorE); }`
        codec_util::check_footer_after(&mut *entries_stream, result)?;
        entries_stream.close()?;
    }
    drop(dir);
    directory.close()
}

// -----------------------------------------------------------------------------
// The seeded-draw oracle: TestCompoundSuiteDraws
// -----------------------------------------------------------------------------

/// The rows `TestCompoundSuiteDraws` records: `<method>\tnext\t<long>`.
static DRAWS: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// `TestCompoundSuiteDraws` of `BaseFormatSuitesOracle`: the codec of
/// `TestLucene90CompoundFormat`, recording the next `random().nextLong()` after
/// each test and after the class.
fn draws_class() -> Arc<Class> {
    Class::builder("org.apache.lucene.tests.index.TestCompoundSuiteDraws")
        .extends(&base_compound_format_test_case::class())
        .constructor(Constructor::no_args(|| {
            FormatTestInstance::new(Arc::new(CompoundSuite {
                codec: TestUtil::get_default_codec(),
                merge_instance: false,
            }))
        }))
        .method(
            Method::new("tearDown")
                .throws(&["java.lang.Exception"])
                .body(|inst: &mut FormatTestInstance| {
                    DRAWS.lock().unwrap().push(format!(
                        "{}\tnext\t{}",
                        LuceneTestCase::get_test_name().unwrap_or_default(),
                        LuceneTestCase::random().next_long()
                    ));
                    tear_down(inst);
                }),
        )
        .method(
            Method::new("recordClassDraw")
                .modifiers(modifier::PUBLIC | modifier::STATIC)
                .annotate(randomizedtesting::junit::AfterClass)
                .static_body(|| {
                    DRAWS.lock().unwrap().push(format!(
                        "#class\tnext\t{}",
                        LuceneTestCase::random().next_long()
                    ));
                }),
        )
        .build()
}

/// One suite of this binary at a time: the runner's state is process-wide.
static SUITES: Mutex<()> = Mutex::new(());

/// Seeded-draw parity with Lucene (rmp #352): every row of
/// `base_compound_draws.tsv` (`BaseFormatSuitesOracle`) must be equal.
#[test]
fn test_compound_suite_draws_match_lucene() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    suite_draws::assert_draws_match("base_compound_draws.tsv", draws_class, &DRAWS);
}

#[test]
fn test_lucene90_compound_format() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    run_suite(lucene90_class);
}
