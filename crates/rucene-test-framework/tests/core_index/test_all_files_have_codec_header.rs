//! Port of `lucene/core/src/test/org/apache/lucene/index/TestAllFilesHaveCodecHeader.java`:
//! test that a plain default puts codec headers in all files.

use std::collections::HashMap;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::codecs::codec_util;
use rucene::error::Result;
use rucene::index::index_file_names;
use rucene::index::SegmentInfos;
use rucene::store::{Directory, READONCE_IO_CONTEXT};
use rucene_test_framework::index::RandomIndexWriter;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::{assert_equals_msg, class, random, test};
use crate::support::{add_line_file_docs, npe};

/// `TestAllFilesHaveCodecHeader`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestAllFilesHaveCodecHeader",
        vec![test("test", test_)],
    )
}

/// Port of `TestAllFilesHaveCodecHeader.test`.
fn test_() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let mut conf = LuceneTestCase::new_index_writer_config_analyzer(Arc::new(
        rucene_test_framework::analysis::MockAnalyzer::new(&*random()),
    ));
    conf.set_codec(TestUtil::get_default_codec())?;
    let riw = RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), conf)?;
    add_line_file_docs(&riw)?;
    riw.close()?;
    check_headers(&dir.as_directory(), &mut HashMap::new())?;
    dir.close()
}

/// `private void checkHeaders(Directory dir, Map<String, String>
/// namesToExtensions)`.
fn check_headers(
    dir: &Arc<dyn Directory>,
    names_to_extensions: &mut HashMap<String, String>,
) -> Result<()> {
    let sis = SegmentInfos::read_latest_commit(Arc::clone(dir))?;
    check_header(
        &**dir,
        &sis.segments_file_name()
            .unwrap_or_else(|| npe("sis.getSegmentsFileName()")),
        names_to_extensions,
        &sis.id(),
    )?;

    for si in sis.as_list() {
        // `assertNotNull(si.info.getId())`: a Rust id is never null.
        let id = si.info.id();
        for file in si.files_ordered()?.iter() {
            check_header(&**dir, file, names_to_extensions, &id)?;
        }
        if si.info.get_use_compound_file() {
            let codec = si.info.codec().unwrap_or_else(|| npe("si.info.getCodec()"));
            let mut cfs_dir = codec
                .compound_format()
                .get_compound_reader(&**dir, &si.info)?;
            let outcome = (|| -> Result<()> {
                for cfs_file in cfs_dir.list_all()? {
                    check_header(&*cfs_dir, &cfs_file, names_to_extensions, &id)?;
                }
                Ok(())
            })();
            let closed = cfs_dir.close();
            outcome?;
            closed?;
        }
    }
    Ok(())
}

/// `private void checkHeader(Directory dir, String file, Map<String, String>
/// namesToExtensions, byte[] id)`.
fn check_header(
    dir: &dyn Directory,
    file: &str,
    names_to_extensions: &mut HashMap<String, String>,
    id: &[u8],
) -> Result<()> {
    let mut input = dir.open_input(file, &*READONCE_IO_CONTEXT)?;
    let outcome = (|| -> Result<()> {
        let val = codec_util::read_be_int(&mut *input)?;
        assert_equals_msg(
            Some(&format!("{file} has no codec header, instead found: {val}")),
            &codec_util::CODEC_MAGIC,
            &val,
        );
        let codec_name = input.read_string()?;
        Assert::assert_false(codec_name.is_empty());
        let extension = match index_file_names::get_extension(file) {
            Some(extension) => extension.to_string(),
            None => {
                Assert::assert_true(file.starts_with(index_file_names::SEGMENTS));
                "<segments> (not a real extension, designates segments file)".to_string()
            }
        };
        let previous = names_to_extensions.insert(codec_name.clone(), extension.clone());
        if let Some(previous) = previous {
            if previous != extension {
                Assert::fail(&format!(
                    "extensions {previous} and {extension} share same codecName {codec_name}"
                ));
            }
        }
        // read version
        input.read_int()?;
        // read object id
        codec_util::check_index_header_id(&mut *input, id)?;
        Ok(())
    })();
    let closed = input.close();
    outcome?;
    closed
}
