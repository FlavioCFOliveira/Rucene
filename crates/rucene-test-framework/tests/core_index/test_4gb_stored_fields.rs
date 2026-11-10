//! Port of `lucene/core/src/test/org/apache/lucene/index/Test4GBStoredFields.java`:
//! this test creates an index with one segment that is a little larger than
//! 4GB.

use std::sync::Arc;

use randomizedtesting::annotations::TimeoutSuite;
use randomizedtesting::generators::RandomNumbers;
use randomizedtesting::jdk::reflect::Class;
use rucene::document::{Document, Field, FieldType};
use rucene::error::Result;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::{IndexWriterConfig, LiveIndexWriterConfig, OpenMode};
use rucene::index::merge_scheduler::ConcurrentMergeScheduler;
use rucene::store::mmap::MMapDirectory;
use rucene::util::BytesRef;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::codecs::compressing::CompressingCodec;
use rucene_test_framework::store::{MockDirectoryWrapper, Throttling};
use rucene_test_framework::util::lucene_test_case::{
    Monster, SuppressCodecs, TestDirectory, VERBOSE,
};
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_equals, assert_not_null, class_annotated, random, test};
use crate::support::{open_reader, petabyte_max_merge};

/// `Test4GBStoredFields`.
pub fn class_def() -> Arc<Class> {
    class_annotated(
        "org.apache.lucene.index.Test4GBStoredFields",
        vec![test("test", test_)],
        |b| {
            b.annotate(SuppressCodecs {
                value: vec!["SimpleText".to_string(), "Compressing".to_string()],
            })
            .annotate(TimeoutSuite {
                millis: 4 * 3_600_000,
            })
            .annotate(Monster {
                value: "consumes a lot of disk space".to_string(),
            })
        },
    )
}

fn test_() -> Result<()> {
    let mock = MockDirectoryWrapper::new(
        &*random(),
        Arc::new(MMapDirectory::open(
            LuceneTestCase::create_temp_dir_prefix("4GBStoredFields"),
        )?),
    );
    mock.set_throttling(Throttling::Never);
    let mut dir = TestDirectory::Mock(mock);

    let mut iwc = IndexWriterConfig::with_analyzer(Some(Arc::new(MockAnalyzer::new(&*random()))));
    iwc.set_max_buffered_docs(LiveIndexWriterConfig::DISABLE_AUTO_FLUSH)?;
    iwc.set_ram_buffer_size_mb(256.0)?;
    iwc.set_merge_scheduler(Some(Arc::new(ConcurrentMergeScheduler::new())))?;
    iwc.set_merge_policy(Some(Arc::new(
        LuceneTestCase::new_log_merge_policy_use_cfs_merge_factor(false, 10)?,
    )))?;
    iwc.set_open_mode(OpenMode::CREATE)?;

    // TODO: we disable "Compressing" since it likes to pick very extreme values which will be too
    // slow for this test.
    // maybe we should factor out crazy cases to ExtremeCompressing? then annotations can handle
    // this stuff...
    if random().next_boolean() {
        iwc.set_codec(Arc::new(CompressingCodec::reasonable_instance(&*random())?))?;
    }

    let w = IndexWriter::new(dir.as_directory(), iwc)?;

    petabyte_max_merge(&w);

    let mut doc = Document::new();
    let ft = FieldType::new();
    ft.set_stored(true)?;
    ft.freeze();
    let value_length = RandomNumbers::random_int_between(&*random(), 1 << 13, 1 << 20);
    let mut value = vec![0u8; value_length as usize];
    for i in 0..value_length as usize {
        // random so that even compressing codecs can't compress it
        value[i] = random().next_int_bound(256) as u8;
    }
    let f = Field::new_with_bytes("fld", BytesRef::new(value.clone()), ft)?;
    doc.add(Box::new(f));

    let num_docs = ((1i64 << 32) / value_length as i64 + 100) as i32;
    for i in 0..num_docs {
        w.add_document(&doc)?;
        if *VERBOSE && i % (num_docs / 10) == 0 {
            println!("{i} of {num_docs}...");
        }
    }
    w.force_merge(1)?;
    w.close()?;
    if *VERBOSE {
        let mut found = false;
        let directory = dir.as_directory();
        for file in directory.list_all()? {
            if file.ends_with(".fdt") {
                let file_length = directory.file_length(&file)?;
                if file_length >= 1i64 << 32 {
                    found = true;
                }
                println!("File length of {file} : {file_length}");
            }
        }
        if !found {
            println!("No .fdt file larger than 4GB, test bug?");
        }
    }

    let rd = open_reader(&dir)?;
    let sd = rd.stored_fields()?.document(num_docs - 1)?;
    assert_not_null(&Some(&sd));
    assert_equals(&1, &sd.get_fields().len());
    let value_ref = sd.get_binary_value("fld");
    assert_not_null(&value_ref);
    assert_equals(&Some(BytesRef::new(value)), &value_ref);
    rd.close()?;

    dir.close()
}
