//! Port of `lucene/core/src/test/org/apache/lucene/index/Test2BPoints.java`.

use std::sync::Arc;

use randomizedtesting::annotations::TimeoutSuite;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::codecs::Codec;
use rucene::document::{Document, LongPoint};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::{IndexWriterConfig, LiveIndexWriterConfig, OpenMode};
use rucene::index::merge_scheduler::ConcurrentMergeScheduler;
use rucene::index::IndexReader;
use rucene::search::IndexSearcher;
use rucene::store::{Directory, FSDirectory};
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::store::base_directory_test_case::close_shared;
use rucene_test_framework::util::lucene_test_case::{Monster, SuppressCodecs, VERBOSE};
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::{assert_equals, class_annotated, random, test};
use crate::support::{as_cms, concurrent_merge_scheduler, npe, petabyte_max_merge};

/// `Test2BPoints`.
pub fn class_def() -> Arc<Class> {
    class_annotated(
        "org.apache.lucene.index.Test2BPoints",
        vec![test("test1D", test_1d), test("test2D", test_2d)],
        |b| {
            b.annotate(SuppressCodecs {
                value: vec![
                    "SimpleText".to_string(),
                    "Direct".to_string(),
                    "Compressing".to_string(),
                ],
            })
            .annotate(TimeoutSuite {
                millis: i32::MAX, // hopefully ~24 days is long enough ;)
            })
            .annotate(Monster {
                value: "takes at least 4 hours and consumes many GB of temp disk space".to_string(),
            })
        },
    )
}

/// The `IndexWriterConfig` both tests build.
fn config() -> Result<IndexWriterConfig> {
    let mut iwc = IndexWriterConfig::with_analyzer(Some(Arc::new(MockAnalyzer::new(&*random()))));
    iwc.set_codec(get_codec())?;
    iwc.set_max_buffered_docs(LiveIndexWriterConfig::DISABLE_AUTO_FLUSH)?;
    iwc.set_ram_buffer_size_mb(256.0)?;
    iwc.set_merge_scheduler(Some(Arc::new(ConcurrentMergeScheduler::new())))?;
    iwc.set_merge_policy(Some(Arc::new(
        LuceneTestCase::new_log_merge_policy_use_cfs_merge_factor(false, 10)?,
    )))?;
    iwc.set_open_mode(OpenMode::CREATE)?;
    let ms = concurrent_merge_scheduler(&iwc);
    as_cms(&ms).set_max_merges_and_threads(6, 3)?;
    Ok(iwc)
}

fn test_1d() -> Result<()> {
    let dir: Arc<dyn Directory> = Arc::new(FSDirectory::open(
        &LuceneTestCase::create_temp_dir_prefix("2BPoints1D"),
    )?);

    let iwc = config()?;

    let w = IndexWriter::new(Arc::clone(&dir), iwc)?;

    petabyte_max_merge(&w);

    let num_docs = (i32::MAX / 26) + 1;
    let mut counter: i32 = 0;
    for i in 0..num_docs {
        let mut doc = Document::new();
        for _ in 0..26 {
            let x = ((random().next_int() as i64) << 32) | counter as i64;
            doc.add(Box::new(LongPoint::new("long", &[x])?));
            counter += 1;
        }
        w.add_document(&doc)?;
        if *VERBOSE && i % 100000 == 0 {
            println!("{i} of {num_docs}...");
        }
    }
    w.force_merge(1)?;
    let r = directory_reader::open_with_writer(&*w)?;
    let s = IndexSearcher::new(Arc::clone(&r) as Arc<dyn IndexReader>)?;
    assert_equals(
        &num_docs,
        &s.count(LongPoint::new_range_query(
            "long",
            &[i64::MIN],
            &[i64::MAX],
        )?)?,
    );
    let leaves = Arc::clone(&r).leaves()?;
    let points = leaves[0]
        .leaf_reader()
        .get_point_values("long")?
        .unwrap_or_else(|| npe("getPointValues"));
    Assert::assert_true(points.size()? > i32::MAX as i64);
    r.close()?;
    w.close()?;
    println!("TEST: now CheckIndex");
    TestUtil::check_index(Arc::clone(&dir))?;
    close_shared(dir)
}

fn test_2d() -> Result<()> {
    let dir: Arc<dyn Directory> = Arc::new(FSDirectory::open(
        &LuceneTestCase::create_temp_dir_prefix("2BPoints2D"),
    )?);

    let iwc = config()?;

    let w = IndexWriter::new(Arc::clone(&dir), iwc)?;

    petabyte_max_merge(&w);

    let num_docs = (i32::MAX / 26) + 1;
    let mut counter: i32 = 0;
    for i in 0..num_docs {
        let mut doc = Document::new();
        for _ in 0..26 {
            let x = ((random().next_int() as i64) << 32) | counter as i64;
            let y = ((random().next_int() as i64) << 32) | random().next_int() as i64;
            doc.add(Box::new(LongPoint::new("long", &[x, y])?));
            counter += 1;
        }
        w.add_document(&doc)?;
        if *VERBOSE && i % 100000 == 0 {
            println!("{i} of {num_docs}...");
        }
    }
    w.force_merge(1)?;
    let r = directory_reader::open_with_writer(&*w)?;
    let s = IndexSearcher::new(Arc::clone(&r) as Arc<dyn IndexReader>)?;
    assert_equals(
        &num_docs,
        &s.count(LongPoint::new_range_query(
            "long",
            &[i64::MIN, i64::MIN],
            &[i64::MAX, i64::MAX],
        )?)?,
    );
    let leaves = Arc::clone(&r).leaves()?;
    let points = leaves[0]
        .leaf_reader()
        .get_point_values("long")?
        .unwrap_or_else(|| npe("getPointValues"));
    Assert::assert_true(points.size()? > i32::MAX as i64);
    r.close()?;
    w.close()?;
    println!("TEST: now CheckIndex");
    TestUtil::check_index(Arc::clone(&dir))?;
    close_shared(dir)
}

/// `getCodec()`: `Codec.getDefault()`.
fn get_codec() -> Arc<dyn Codec> {
    rucene::codecs::get_default().unwrap_or_else(|| npe("Codec.getDefault()"))
}
