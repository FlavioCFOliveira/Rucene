//! Port of `lucene/core/src/test/org/apache/lucene/index/TestAllFilesHaveChecksumFooter.java`:
//! test that a plain default puts CRC32 footers in all files.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use rucene::codecs::codec_util;
use rucene::error::Result;
use rucene::index::SegmentInfos;
use rucene::store::{Directory, READONCE_IO_CONTEXT};
use rucene_test_framework::index::RandomIndexWriter;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::{class, random, test};
use crate::support::{add_line_file_docs, npe};

/// `TestAllFilesHaveChecksumFooter`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestAllFilesHaveChecksumFooter",
        vec![test("test", test_)],
    )
}

/// Port of `TestAllFilesHaveChecksumFooter.test`.
fn test_() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let conf = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(
        rucene_test_framework::analysis::MockAnalyzer::new(&*random()),
    )));
    conf.set_codec(TestUtil::get_default_codec())?;
    let riw = RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), conf)?;
    add_line_file_docs(&riw)?;
    riw.close()?;
    check_footers(&dir.as_directory())?;
    dir.close()
}

/// `private void checkFooters(Directory dir)`.
fn check_footers(dir: &Arc<dyn Directory>) -> Result<()> {
    let sis = SegmentInfos::read_latest_commit(Arc::clone(dir))?;
    check_footer(
        &**dir,
        &sis.segments_file_name()
            .unwrap_or_else(|| npe("sis.getSegmentsFileName()")),
    )?;
    for si in sis.as_list() {
        for file in si.files_ordered()?.iter() {
            check_footer(&**dir, file)?;
        }
        if si.info.get_use_compound_file() {
            let codec = si.info.codec().unwrap_or_else(|| npe("si.info.getCodec()"));
            let cfs_dir = codec
                .compound_format()
                .get_compound_reader(&**dir, &si.info)?;
            let outcome = (|| -> Result<()> {
                for cfs_file in cfs_dir.list_all()? {
                    check_footer(&*cfs_dir, &cfs_file)?;
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

/// `private void checkFooter(Directory dir, String file)`.
fn check_footer(dir: &dyn Directory, file: &str) -> Result<()> {
    let mut input = dir.open_input(file, &*READONCE_IO_CONTEXT)?;
    let outcome = codec_util::checksum_entire_file(&mut *input);
    let closed = input.close();
    outcome?;
    closed
}
