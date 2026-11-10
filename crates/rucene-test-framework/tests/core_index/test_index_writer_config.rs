//! Port of `lucene/core/src/test/org/apache/lucene/index/TestIndexWriterConfig.java`.
//!
//! * `x.getClass() == C.class` is answered with the type's `java_class_name`
//!   or an `as_any` downcast, as each trait offers.
//! * The reflection over `IndexWriterConfig` and `LiveIndexWriterConfig`
//!   (`getDeclaredMethods`, `getDeclaredFields`) is over declared lists: the
//!   getters and setters Rucene's `impl IndexWriterConfig` and `impl
//!   LiveIndexWriterConfig` (`src/index/index_writer_config.rs`) define, under
//!   their Java names (a Rust getter has no `get` prefix), and the instance
//!   fields `IndexWriterConfig.java` declares.
//! * Java's `null` arguments are `None`: the setters and
//!   `SnapshotDeletionPolicy::new` take an `Option`.

use std::collections::HashSet;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::{Assert, Test};
use rucene::document::{Document, Store};
use rucene::error::Result;
use rucene::index::index_deletion_policy::SnapshotDeletionPolicy;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::{IndexWriterConfig, LiveIndexWriterConfig, OpenMode};
use rucene::index::merge_policy::{LogDocSize, LogMergePolicy, MergePolicy, TieredMergePolicy};
use rucene::index::merge_scheduler::{ConcurrentMergeScheduler, SerialMergeScheduler};
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::similarities::{
    tfidf_scorer, ClassicSimilarity, CollectionStatistics, SimScorer, Similarity, TFIDFSimilarity,
    TermStatistics,
};
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::index::RandomIndexWriter;
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{
    assert_equals, assert_equals_delta, assert_null, class, random, test, IAE,
};
use crate::support::npe;

/// `TestIndexWriterConfig`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestIndexWriterConfig",
        vec![
            test("testDefaults", test_defaults).annotate(Test::new()),
            test("testSettersChaining", test_setters_chaining).annotate(Test::new()),
            test("testReuse", test_reuse).annotate(Test::new()),
            test("testOverrideGetters", test_override_getters).annotate(Test::new()),
            test("testConstants", test_constants).annotate(Test::new()),
            test("testToString", test_to_string).annotate(Test::new()),
            test("testInvalidValues", test_invalid_values).annotate(Test::new()),
            test("testLiveChangeToCFS", test_live_change_to_cfs),
        ],
    )
}

/// `private static final class MySimilarity extends ClassicSimilarity`:
/// does not implement anything - used only for type checking on
/// IndexWriterConfig.
#[derive(Debug, Default)]
struct MySimilarity {
    classic: ClassicSimilarity,
}

impl Similarity for MySimilarity {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.index.TestIndexWriterConfig$MySimilarity"
    }

    fn scorer<'a>(
        &'a self,
        boost: f32,
        collection_stats: &CollectionStatistics,
        term_stats: &[TermStatistics],
    ) -> Box<dyn SimScorer + 'a> {
        tfidf_scorer(self, boost, collection_stats, term_stats)
    }
}

impl TFIDFSimilarity for MySimilarity {
    fn tf(&self, freq: f32) -> f32 {
        self.classic.tf(freq)
    }
    fn length_norm(&self, length: i32) -> f32 {
        self.classic.length_norm(length)
    }
    fn idf(&self, doc_freq: i64, doc_count: i64) -> f32 {
        self.classic.idf(doc_freq, doc_count)
    }
}

/// `conf.getMergeScheduler().getClass() == C.class`.
fn merge_scheduler_is<T: 'static>(conf: &IndexWriterConfig) -> bool {
    conf.merge_scheduler()
        .as_any()
        .is_some_and(|a| a.downcast_ref::<T>().is_some())
}

/// `conf.getMergePolicy().getClass() == TieredMergePolicy.class`.
fn is_tiered(mp: &Arc<dyn MergePolicy>) -> bool {
    mp.as_any()
        .is_some_and(|a| a.downcast_ref::<TieredMergePolicy>().is_some())
}

/// `conf.getMergePolicy().getClass() == LogDocMergePolicy.class`.
fn is_log_doc(mp: &Arc<dyn MergePolicy>) -> bool {
    mp.as_any()
        .and_then(|a| a.downcast_ref::<LogMergePolicy>())
        .is_some_and(|l| !l.is_log_byte_size_merge_policy())
}

/// Port of `TestIndexWriterConfig.testDefaults`.
fn test_defaults() -> Result<()> {
    let conf = IndexWriterConfig::with_analyzer(Some(Arc::new(MockAnalyzer::new(&*random()))));
    assert_equals(
        "org.apache.lucene.tests.analysis.MockAnalyzer",
        conf.analyzer()
            .unwrap_or_else(|| npe("conf.getAnalyzer()"))
            .java_class_name(),
    );
    assert_null(&conf.index_commit().map(|_| ()));
    assert_equals(
        "org.apache.lucene.index.KeepOnlyLastCommitDeletionPolicy",
        conf.index_deletion_policy().java_class_name(),
    );
    Assert::assert_true(merge_scheduler_is::<ConcurrentMergeScheduler>(&conf));
    assert_equals(&OpenMode::CREATE_OR_APPEND, &conf.open_mode());
    // we don't need to assert this, it should be unspecified
    Assert::assert_true(Arc::ptr_eq(
        &IndexSearcher::get_default_similarity(),
        &conf.similarity(),
    ));
    assert_equals_delta(
        LiveIndexWriterConfig::DEFAULT_RAM_BUFFER_SIZE_MB,
        conf.ram_buffer_size_mb(),
        0.0,
    );
    assert_equals(
        &LiveIndexWriterConfig::DEFAULT_MAX_BUFFERED_DOCS,
        &conf.max_buffered_docs(),
    );
    assert_equals(
        &LiveIndexWriterConfig::DEFAULT_READER_POOLING,
        &conf.reader_pooling(),
    );
    assert_null(&conf.merged_segment_warmer().map(|_| ()));
    Assert::assert_true(is_tiered(&conf.merge_policy()));
    Assert::assert_true(
        format!("{:?}", conf.into_live().flush_policy()).starts_with("FlushByRamOrCountsPolicy"),
    );
    let conf = IndexWriterConfig::with_analyzer(Some(Arc::new(MockAnalyzer::new(&*random()))));
    assert_equals(
        &LiveIndexWriterConfig::DEFAULT_RAM_PER_THREAD_HARD_LIMIT_MB,
        &conf.ram_per_thread_hard_limit_mb(),
    );
    assert_equals(
        rucene::codecs::get_default()
            .unwrap_or_else(|| crate::support::npe("codec"))
            .name(),
        conf.codec().name(),
    );
    Assert::assert_true(Arc::ptr_eq(
        &rucene::util::default_info_stream(),
        &conf.info_stream(),
    ));
    assert_equals(
        &LiveIndexWriterConfig::DEFAULT_USE_COMPOUND_FILE_SYSTEM,
        &conf.use_compound_file(),
    );
    Assert::assert_true(conf.check_pending_flush_on_update());
    // Sanity check - validate that all getters are covered.
    let getters: HashSet<&str> = [
        "getAnalyzer",
        "getIndexCommit",
        "getIndexDeletionPolicy",
        "getMaxFieldLength",
        "getMergeScheduler",
        "getOpenMode",
        "getSimilarity",
        "getWriteLockTimeout",
        "getDefaultWriteLockTimeout",
        "getMaxBufferedDeleteTerms",
        "getRAMBufferSizeMB",
        "getMaxBufferedDocs",
        "getIndexingChain",
        "getMergedSegmentWarmer",
        "getMergePolicy",
        "getReaderPooling",
        "getIndexerThreadPool",
        "getFlushPolicy",
        "getRAMPerThreadHardLimitMB",
        "getCodec",
        "getInfoStream",
        "getUseCompoundFile",
        "isCheckPendingFlushOnUpdate",
        "getSoftDeletesField",
    ]
    .into_iter()
    .collect();

    for m in IWC_GETTERS {
        if m.starts_with("get") {
            Assert::assert_true_msg(
                &format!("method {m} is not tested for defaults"),
                getters.contains(m),
            );
        }
    }
    Ok(())
}

/// The getters `impl IndexWriterConfig` defines, under their Java names.
const IWC_GETTERS: &[&str] = &[
    "getAnalyzer",
    "getRAMBufferSizeMB",
    "getMaxBufferedDocs",
    "getMergePolicy",
    "getMergedSegmentWarmer",
    "getOpenMode",
    "getIndexCreatedVersionMajor",
    "getIndexDeletionPolicy",
    "getIndexCommit",
    "getSimilarity",
    "getMergeScheduler",
    "getCodec",
    "getInfoStream",
    "getReaderPooling",
    "getRAMPerThreadHardLimitMB",
    "getUseCompoundFile",
    "getCommitOnClose",
    "getIndexSort",
    "getIndexSortFields",
    "getLeafSorter",
    "isCheckPendingFlushOnUpdate",
    "getSoftDeletesField",
    "getMaxFullFlushMergeWaitMillis",
    "getIndexWriterEventListener",
    "getParentField",
];

/// The getters `impl LiveIndexWriterConfig` defines, under their Java names.
const LIVE_GETTERS: &[&str] = &[
    "getAnalyzer",
    "getRAMBufferSizeMB",
    "getMaxBufferedDocs",
    "getMergePolicy",
    "getMergedSegmentWarmer",
    "getOpenMode",
    "getIndexCreatedVersionMajor",
    "getIndexDeletionPolicy",
    "getIndexCommit",
    "getSimilarity",
    "getMergeScheduler",
    "getCodec",
    "getInfoStream",
    "getReaderPooling",
    "getRAMPerThreadHardLimitMB",
    "getFlushPolicy",
    "getUseCompoundFile",
    "getCommitOnClose",
    "getIndexSort",
    "getIndexSortFields",
    "getLeafSorter",
    "isCheckPendingFlushOnUpdate",
    "getSoftDeletesField",
    "getMaxFullFlushMergeWaitMillis",
    "getIndexWriterEventListener",
    "getParentField",
];

/// The non-static setters `impl IndexWriterConfig` defines, under their
/// Java names; each returns `&mut IndexWriterConfig` (inside a `Result`).
const IWC_SETTERS: &[&str] = &[
    "setOpenMode",
    "setIndexCreatedVersionMajor",
    "setIndexDeletionPolicy",
    "setIndexCommit",
    "setSimilarity",
    "setMergeScheduler",
    "setCodec",
    "setReaderPooling",
    "setRAMPerThreadHardLimitMB",
    "setFlushPolicy",
    "setInfoStream",
    "setCommitOnClose",
    "setIndexSort",
    "setLeafSorter",
    "setSoftDeletesField",
    "setIndexWriterEventListener",
    "setParentField",
    "setMaxFullFlushMergeWaitMillis",
    "setRAMBufferSizeMB",
    "setMaxBufferedDocs",
    "setMergePolicy",
    "setMergedSegmentWarmer",
    "setUseCompoundFile",
    "setCheckPendingFlushUpdate",
];

/// The setters `impl LiveIndexWriterConfig` defines (Java's synthetic
/// bridges of the overridden live setters), under their Java names.
const LIVE_SETTERS: &[&str] = &[
    "setRAMBufferSizeMB",
    "setMaxBufferedDocs",
    "setMergePolicy",
    "setMergedSegmentWarmer",
    "setSimilarity",
    "setCodec",
    "setUseCompoundFile",
    "setCheckPendingFlushUpdate",
];

/// Port of `TestIndexWriterConfig.testSettersChaining`.
fn test_setters_chaining() -> Result<()> {
    // Ensures that every setter returns IndexWriterConfig to allow chaining.
    let mut live_setters: HashSet<&str> = HashSet::new();
    let mut all_setters: HashSet<&str> = HashSet::new();
    for m in IWC_SETTERS {
        all_setters.insert(m);
        // every listed setter returns `&mut IndexWriterConfig`
    }
    for m in LIVE_SETTERS {
        live_setters.insert(m);
    }
    for setter in live_setters {
        Assert::assert_true_msg(
            &format!("setter method not overridden by IndexWriterConfig: {setter}"),
            all_setters.contains(setter),
        );
    }
    Ok(())
}

/// Port of `TestIndexWriterConfig.testReuse`.
///
/// **Divergence.** Java opens a second `RandomIndexWriter` on the same
/// `IndexWriterConfig` and expects `IllegalStateException`
/// (`TestIndexWriterConfig.java:144-149`); Rucene's `IndexWriter::new` takes
/// the config by value, so the second use does not compile: the reuse the
/// exception guards against is rejected by the type system, and only the
/// first writer is opened.
fn test_reuse() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    // test that IWC cannot be reused across two IWs
    let conf = LuceneTestCase::new_index_writer_config_analyzer(None);
    RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), conf)?.close()?;

    // this should fail: `conf` was moved into the first writer.

    dir.close()
}

/// Port of `TestIndexWriterConfig.testOverrideGetters`.
fn test_override_getters() -> Result<()> {
    // Test that IndexWriterConfig overrides all getters, so that javadocs
    // contain all methods for the users. Also, ensures that IndexWriterConfig
    // doesn't declare getters that are not declared on LiveIWC.
    let mut live_getters: HashSet<&str> = HashSet::new();
    for m in LIVE_GETTERS {
        if m.starts_with("get") {
            live_getters.insert(m);
        }
    }

    for m in IWC_GETTERS {
        if m.starts_with("get") {
            // every listed getter is declared by `IndexWriterConfig`
            Assert::assert_true_msg(
                &format!("method {m} not declared on LiveIndexWriterConfig"),
                live_getters.contains(m),
            );
        }
    }
    Ok(())
}

/// Port of `TestIndexWriterConfig.testConstants`.
fn test_constants() -> Result<()> {
    // Tests that the values of the constants does not change
    assert_equals(&-1, &LiveIndexWriterConfig::DISABLE_AUTO_FLUSH);
    assert_equals(
        &LiveIndexWriterConfig::DISABLE_AUTO_FLUSH,
        &LiveIndexWriterConfig::DEFAULT_MAX_BUFFERED_DELETE_TERMS,
    );
    assert_equals(
        &LiveIndexWriterConfig::DISABLE_AUTO_FLUSH,
        &LiveIndexWriterConfig::DEFAULT_MAX_BUFFERED_DOCS,
    );
    assert_equals_delta(16.0, LiveIndexWriterConfig::DEFAULT_RAM_BUFFER_SIZE_MB, 0.0);
    assert_equals(&true, &LiveIndexWriterConfig::DEFAULT_READER_POOLING);
    assert_equals(
        &true,
        &LiveIndexWriterConfig::DEFAULT_USE_COMPOUND_FILE_SYSTEM,
    );
    Ok(())
}

/// The instance fields `IndexWriterConfig.java` declares (`writer`, line 112).
const IWC_FIELDS: &[&str] = &["writer"];

/// Port of `TestIndexWriterConfig.testToString`.
fn test_to_string() -> Result<()> {
    let str =
        IndexWriterConfig::with_analyzer(Some(Arc::new(MockAnalyzer::new(&*random())))).to_string();
    for f in IWC_FIELDS {
        if *f == "indexingChain" {
            // indexingChain is a package-private setting and thus is not output by
            // toString.
            continue;
        }
        if *f == "inUseByIndexWriter" {
            continue;
        }
        Assert::assert_true_msg(&format!("{f} not found in toString"), str.contains(f));
    }
    Ok(())
}

/// Port of `TestIndexWriterConfig.testInvalidValues`.
fn test_invalid_values() -> Result<()> {
    let mut conf = IndexWriterConfig::with_analyzer(Some(Arc::new(MockAnalyzer::new(&*random()))));

    // Test IndexDeletionPolicy
    assert_equals(
        "org.apache.lucene.index.KeepOnlyLastCommitDeletionPolicy",
        conf.index_deletion_policy().java_class_name(),
    );
    conf.set_index_deletion_policy(Some(Arc::new(SnapshotDeletionPolicy::new(None))))?;
    assert_equals(
        "org.apache.lucene.index.SnapshotDeletionPolicy",
        conf.index_deletion_policy().java_class_name(),
    );
    expect_throws(IAE, || conf.set_index_deletion_policy(None).map(|_| ()));

    // Test MergeScheduler
    Assert::assert_true(merge_scheduler_is::<ConcurrentMergeScheduler>(&conf));
    conf.set_merge_scheduler(Some(Arc::new(SerialMergeScheduler::new())))?;
    Assert::assert_true(merge_scheduler_is::<SerialMergeScheduler>(&conf));
    expect_throws(IAE, || conf.set_merge_scheduler(None).map(|_| ()));

    // Test Similarity:
    // we shouldnt assert what the default is, just that it's not null.
    Assert::assert_true(Arc::ptr_eq(
        &IndexSearcher::get_default_similarity(),
        &conf.similarity(),
    ));
    conf.set_similarity(Some(Arc::new(MySimilarity::default())))?;
    assert_equals(
        "org.apache.lucene.index.TestIndexWriterConfig$MySimilarity",
        conf.similarity().java_class_name(),
    );
    expect_throws(IAE, || conf.set_similarity(None).map(|_| ()));

    expect_throws(IAE, || conf.set_max_buffered_docs(1).map(|_| ()));

    expect_throws(IAE, || -> Result<()> {
        // Disable both MAX_BUF_DOCS and RAM_SIZE_MB
        conf.set_max_buffered_docs(4)?;
        conf.set_ram_buffer_size_mb(LiveIndexWriterConfig::DISABLE_AUTO_FLUSH as f64)?;
        conf.set_max_buffered_docs(LiveIndexWriterConfig::DISABLE_AUTO_FLUSH)?;
        Ok(())
    });

    conf.set_ram_buffer_size_mb(LiveIndexWriterConfig::DEFAULT_RAM_BUFFER_SIZE_MB)?;
    conf.set_max_buffered_docs(LiveIndexWriterConfig::DEFAULT_MAX_BUFFERED_DOCS)?;
    expect_throws(IAE, || {
        conf.set_ram_buffer_size_mb(LiveIndexWriterConfig::DISABLE_AUTO_FLUSH as f64)
            .map(|_| ())
    });

    expect_throws(IAE, || {
        conf.set_ram_per_thread_hard_limit_mb(2048).map(|_| ())
    });

    expect_throws(IAE, || conf.set_ram_per_thread_hard_limit_mb(0).map(|_| ()));

    // Test MergePolicy
    Assert::assert_true(is_tiered(&conf.merge_policy()));
    conf.set_merge_policy(Some(Arc::new(LogMergePolicy::new(Arc::new(LogDocSize)))))?;
    Assert::assert_true(is_log_doc(&conf.merge_policy()));
    expect_throws(IAE, || conf.set_merge_policy(None).map(|_| ()));
    Ok(())
}

/// Port of `TestIndexWriterConfig.testLiveChangeToCFS`.
fn test_live_change_to_cfs() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let mut iwc = IndexWriterConfig::with_analyzer(Some(Arc::new(MockAnalyzer::new(&*random()))));
    iwc.set_merge_policy(Some(LuceneTestCase::new_log_merge_policy_use_cfs(true)))?;
    // Start false:
    iwc.set_use_compound_file(false)?;
    iwc.merge_policy().set_no_cfs_ratio(0.0)?;
    let w = IndexWriter::new(dir.as_directory(), iwc)?;
    // Change to true:
    w.get_config().set_use_compound_file(true);

    let newest_uses_cfs = |w: &IndexWriter| -> Result<bool> {
        Ok(w.newest_segment()?
            .unwrap_or_else(|| crate::support::npe("newestSegment"))
            .info
            .get_use_compound_file())
    };

    let mut fields = vec![LuceneTestCase::new_string_field("field", "foo", Store::NO)];
    let doc_of = |fields: &[rucene::document::Field]| -> Result<Document> {
        let mut doc = Document::new();
        for f in fields {
            doc.add(Box::new(crate::support::copy_field(f)?));
        }
        Ok(doc)
    };
    w.add_document(&doc_of(&fields)?)?;
    w.commit()?;
    Assert::assert_true_msg("Expected CFS after commit", newest_uses_cfs(&w)?);

    fields.push(LuceneTestCase::new_string_field("field", "foo", Store::NO));
    w.add_document(&doc_of(&fields)?)?;
    w.commit()?;
    w.force_merge(1)?;
    w.commit()?;

    // no compound files after merge
    Assert::assert_false_msg("Expected Non-CFS after merge", newest_uses_cfs(&w)?);

    let lmp = w.get_config().merge_policy();
    lmp.set_no_cfs_ratio(1.0)?;
    lmp.set_max_cfs_segment_size_mb(f64::INFINITY)?;

    w.add_document(&doc_of(&fields)?)?;
    w.force_merge(1)?;
    w.commit()?;
    Assert::assert_true_msg("Expected CFS after merge", newest_uses_cfs(&w)?);
    w.close()?;
    dir.close()
}
