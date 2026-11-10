//! Plumbing the `core_index` ports share.

#![allow(dead_code)]

use std::sync::Arc;

use rucene::error::Result;
use rucene::index::directory_reader::{self, DirectoryReader};
use rucene::index::index_writer_config::{IndexWriterConfig, LiveIndexWriterConfig, OpenMode};
use rucene::index::merge_scheduler::ConcurrentMergeScheduler;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::store::Throttling;
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::random;

/// `if (dir instanceof MockDirectoryWrapper) ((MockDirectoryWrapper)
/// dir).setThrottling(MockDirectoryWrapper.Throttling.NEVER)`.
pub fn never_throttle(dir: &TestDirectory) {
    if let Some(mock) = dir.as_mock() {
        mock.set_throttling(Throttling::Never);
    }
}

/// The `IndexWriterConfig` of the `Test2B*` classes:
/// `new IndexWriterConfig(new MockAnalyzer(random()))
/// .setMaxBufferedDocs(IndexWriterConfig.DISABLE_AUTO_FLUSH)
/// .setRAMBufferSizeMB(256.0) .setMergeScheduler(new ConcurrentMergeScheduler())
/// .setMergePolicy(newLogMergePolicy(false, 10))
/// .setOpenMode(IndexWriterConfig.OpenMode.CREATE)
/// .setCodec(TestUtil.getDefaultCodec())`.
pub fn two_b_config() -> Result<IndexWriterConfig> {
    let mut iwc = IndexWriterConfig::with_analyzer(Arc::new(MockAnalyzer::new(&*random())));
    iwc.set_max_buffered_docs(LiveIndexWriterConfig::DISABLE_AUTO_FLUSH)?;
    iwc.set_ram_buffer_size_mb(256.0)?;
    iwc.set_merge_scheduler(Arc::new(ConcurrentMergeScheduler::new()))?;
    iwc.set_merge_policy(Arc::new(
        LuceneTestCase::new_log_merge_policy_use_cfs_merge_factor(false, 10)?,
    ))?;
    iwc.set_open_mode(OpenMode::CREATE)?;
    iwc.set_codec(TestUtil::get_default_codec())?;
    Ok(iwc)
}

/// `DirectoryReader.open(dir)`.
pub fn open_reader(dir: &TestDirectory) -> Result<Arc<dyn DirectoryReader>> {
    directory_reader::open(dir.as_directory())
}

/// `System.out.println(s); System.out.flush();`
pub fn println_flush(s: &str) {
    use std::io::Write;
    println!("{s}");
    let _ = std::io::stdout().flush();
}

/// `MergePolicy mp = w.getConfig().getMergePolicy(); if (mp instanceof
/// LogByteSizeMergePolicy) { // 1 petabyte:
/// ((LogByteSizeMergePolicy) mp).setMaxMergeMB(1024 * 1024 * 1024); }`.
pub fn petabyte_max_merge(w: &rucene::index::index_writer::IndexWriter) {
    let mp = w.get_config().merge_policy();
    if let Some(lmp) = mp
        .as_any()
        .and_then(|a| a.downcast_ref::<rucene::index::merge_policy::LogMergePolicy>())
    {
        if lmp.is_log_byte_size_merge_policy() {
            // 1 petabyte:
            lmp.set_max_merge_mb((1024 * 1024 * 1024) as f64);
        }
    }
}

/// `((ConcurrentMergeScheduler) iwc.getMergeScheduler())`.
pub fn concurrent_merge_scheduler(
    iwc: &IndexWriterConfig,
) -> Arc<dyn rucene::index::merge_scheduler::MergeScheduler> {
    let ms = iwc.merge_scheduler();
    if ms
        .as_any()
        .and_then(|a| a.downcast_ref::<ConcurrentMergeScheduler>())
        .is_none()
    {
        randomizedtesting::jdk::exception::Throwable::new(
            "java.lang.ClassCastException",
            "not a ConcurrentMergeScheduler",
        )
        .throw()
    }
    ms
}

/// The `ConcurrentMergeScheduler` [`concurrent_merge_scheduler`] returned.
pub fn as_cms(
    ms: &Arc<dyn rucene::index::merge_scheduler::MergeScheduler>,
) -> &ConcurrentMergeScheduler {
    ms.as_any()
        .and_then(|a| a.downcast_ref::<ConcurrentMergeScheduler>())
        .expect("checked by concurrent_merge_scheduler")
}

/// `null` dereferenced: Java's `NullPointerException`.
pub fn npe<T>(what: &str) -> T {
    randomizedtesting::jdk::exception::Throwable::new("java.lang.NullPointerException", what)
        .throw()
}

/// `dir.setCheckIndexOnClose(value)` (`BaseDirectoryWrapper`).
pub fn set_check_index_on_close(dir: &TestDirectory, value: bool) {
    match dir {
        TestDirectory::Mock(mock) => mock.set_check_index_on_close(value),
        TestDirectory::Raw(raw) => raw.base().set_check_index_on_close(value),
    }
}
