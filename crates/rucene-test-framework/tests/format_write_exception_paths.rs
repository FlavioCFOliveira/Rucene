//! Regression tests for the write paths `BaseIndexFileFormatTestCase.testRandomExceptions`
//! found leaking an output on an exception (rmp #351): Java writes the
//! field infos and the compound file in try-with-resources blocks
//! (`Lucene94FieldInfosFormat.java:420`, `Lucene90CompoundFormat.java:97-106`),
//! so an `IOException` thrown while writing still closes every output it had
//! created. Each test makes `MockDirectoryWrapper` throw on the first write
//! and then checks that the wrapper closes, which fails while an output is
//! left open (`MockDirectoryWrapper: cannot close: there are still N open
//! files`).

use std::collections::HashMap;
use std::sync::Arc;

use randomizedtesting::jdk::random::JdkRandom;
use rucene::codecs::compound::CompoundFormat;
use rucene::codecs::field_infos::FieldInfosFormat;
use rucene::codecs::lucene90::Lucene90CompoundFormat;
use rucene::codecs::{Lucene104Codec, Lucene94FieldInfosFormat};
use rucene::index::field_infos::{FieldInfo, FieldInfos};
use rucene::index::segment_info::SegmentInfo;
use rucene::search::Sort;
use rucene::store::{ByteBuffersDirectory, Directory, IOContext, DEFAULT_IO_CONTEXT};
use rucene::util::string_helper::StringHelper;
use rucene::util::Version;
use rucene_test_framework::store::MockDirectoryWrapper;

/// A wrapper over an empty in-memory directory that throws an `IOException`
/// on every write.
fn throwing_dir() -> MockDirectoryWrapper {
    let dir = MockDirectoryWrapper::new(
        &JdkRandom::new(0x351),
        Arc::new(ByteBuffersDirectory::new()),
    );
    dir.set_check_index_on_close(false);
    dir.set_random_io_exception_rate(1.0);
    dir
}

fn segment_info(dir: Arc<dyn Directory>) -> SegmentInfo {
    SegmentInfo::new(
        dir,
        Version::LATEST,
        Some(Version::LATEST),
        "_0".to_string(),
        1,
        false,
        false,
        Arc::new(Lucene104Codec::new()),
        HashMap::new(),
        StringHelper::random_id(),
        HashMap::new(),
        Sort::default(),
    )
    .unwrap()
}

#[test]
fn a_failing_field_infos_write_closes_its_output() {
    let dir = throwing_dir();
    let si = segment_info(dir.as_directory());
    let infos = FieldInfos::new(vec![FieldInfo::new("f", 0)]).unwrap();
    let ctx: &dyn IOContext = &*DEFAULT_IO_CONTEXT;
    let result = Lucene94FieldInfosFormat.write(&*dir.as_directory(), &si, "", &infos, ctx);
    let error = result.expect_err("the first write throws");
    assert!(
        format!("{error}").contains("a random IOException"),
        "{error}"
    );
    dir.set_random_io_exception_rate(0.0);
    drop(si);
    dir.close().expect("no output is left open");
}

#[test]
fn a_failing_compound_write_closes_both_outputs() {
    let dir = throwing_dir();
    let si = segment_info(dir.as_directory());
    si.set_files(std::collections::HashSet::new()).unwrap();
    let ctx: &dyn IOContext = &*DEFAULT_IO_CONTEXT;
    let result = Lucene90CompoundFormat::new().write(&*dir.as_directory(), &si, ctx);
    let error = result.expect_err("the first write throws");
    assert!(
        format!("{error}").contains("a random IOException"),
        "{error}"
    );
    dir.set_random_io_exception_rate(0.0);
    drop(si);
    dir.close().expect("no output is left open");
}

// -----------------------------------------------------------------------------
// The SimpleText write paths (E11 round 4): `SimpleTextFieldInfosFormat.write`
// and `SimpleTextLiveDocsFormat.writeLiveDocs` close their output in a
// `finally` (`closeWhileHandlingException` on failure,
// `SimpleTextFieldInfosFormat.java:350-357`, `SimpleTextLiveDocsFormat.java:127-133`),
// `SimpleTextSegmentInfoFormat.write` in try-with-resources
// (`SimpleTextSegmentInfoFormat.java:239`). Found by
// `TestSimpleTextStoredFieldsFormat.testRandomExceptions` (seeds
// 72C1120625CC95A4: `_g.inf`, 928F93B262B49FA8: `_o.si`, 1BE7AFEE79D33746:
// `_17_1.liv`).
// -----------------------------------------------------------------------------

#[test]
fn a_failing_simple_text_field_infos_write_closes_its_output() {
    use rucene_codecs::simpletext::SimpleTextFieldInfosFormat;
    let dir = throwing_dir();
    let si = segment_info(dir.as_directory());
    let infos = FieldInfos::new(vec![FieldInfo::new("f", 0)]).unwrap();
    let ctx: &dyn IOContext = &*DEFAULT_IO_CONTEXT;
    let result = SimpleTextFieldInfosFormat.write(&*dir.as_directory(), &si, "", &infos, ctx);
    let error = result.expect_err("the first write throws");
    assert!(
        format!("{error}").contains("a random IOException"),
        "{error}"
    );
    dir.set_random_io_exception_rate(0.0);
    drop(si);
    dir.close().expect("no output is left open");
}

#[test]
fn a_failing_simple_text_segment_info_write_closes_its_output() {
    use rucene::codecs::segment_info::SegmentInfoFormat;
    use rucene_codecs::simpletext::SimpleTextSegmentInfoFormat;
    let dir = throwing_dir();
    let si = segment_info(dir.as_directory());
    si.set_files(std::collections::HashSet::new()).unwrap();
    let ctx: &dyn IOContext = &*DEFAULT_IO_CONTEXT;
    let result = SimpleTextSegmentInfoFormat.write(&*dir.as_directory(), &si, ctx);
    let error = result.expect_err("the first write throws");
    assert!(
        format!("{error}").contains("a random IOException"),
        "{error}"
    );
    dir.set_random_io_exception_rate(0.0);
    drop(si);
    dir.close().expect("no output is left open");
}

#[test]
fn a_failing_simple_text_live_docs_write_closes_its_output() {
    use rucene::codecs::LiveDocsFormat;
    use rucene::index::segment_info::SegmentCommitInfo;
    use rucene_codecs::simpletext::SimpleTextLiveDocsFormat;
    let dir = throwing_dir();
    let info = SegmentCommitInfo::new(
        segment_info(dir.as_directory()),
        0,
        0,
        -1,
        -1,
        -1,
        StringHelper::random_id(),
    )
    .unwrap();
    let mut bits = rucene::util::FixedBitSet::new(1);
    bits.set(0);
    let ctx: &dyn IOContext = &*DEFAULT_IO_CONTEXT;
    let result =
        SimpleTextLiveDocsFormat.write_live_docs(&bits, &*dir.as_directory(), &info, 0, ctx);
    let error = result.expect_err("the first write throws");
    assert!(
        format!("{error}").contains("a random IOException"),
        "{error}"
    );
    dir.set_random_io_exception_rate(0.0);
    drop(info);
    dir.close().expect("no output is left open");
}
