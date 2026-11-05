/*
 * Licensed to the Apache Software Foundation (ASF) under one or more
 * contributor license agreements.  See the NOTICE file distributed with
 * this work for additional information regarding copyright ownership.
 * The ASF licenses this file to You under the Apache License, Version 2.0
 * (the "License"); you may not use this file except in compliance with
 * the License.  You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

//! The members of `org.apache.lucene.tests.index.RandomIndexWriter` that
//! `BaseTokenStreamTestCase.checkRandomData`'s indexing branch and
//! `TestMockAnalyzer.testChangeGaps` call, ported in full under the sprint's
//! cycle rule; rmp #347 ports the rest of the class.

use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use randomizedtesting::jdk::random::{JdkRandom, Random};
use rucene::analysis::Analyzer;
use rucene::document::Document;
use rucene::error::Result;
use rucene::index::directory_reader::{self, DirectoryReader};
use rucene::index::filter_directory_reader::{
    DirectoryReaderWrapper, SoftDeletesDirectoryReaderWrapper,
};
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::{IndexWriterConfig, LiveIndexWriterConfig, OpenMode};
use rucene::index::merge_policy::{NoMergePolicy, TieredMergePolicy};
use rucene::internal::tests_hooks::{IndexWriterAccess, TestSecrets};
use rucene::store::Directory;
use rucene::util::InfoStream;

use crate::util::lucene_test_case::{LuceneTestCase, VERBOSE};
use crate::util::{NullInfoStream, TestUtil};

/// `INDEX_WRITER_ACCESS = TestSecrets.getIndexWriterAccess()`.
fn index_writer_access() -> Arc<dyn IndexWriterAccess> {
    TestSecrets::get_index_writer_access()
        .expect("IndexWriter registers its IndexWriterAccess with TestSecrets")
}

/// Port of the nested interface `RandomIndexWriter.TestPoint`.
pub trait TestPoint: Send + Sync {
    /// `apply(message)`.
    fn apply(&self, message: &str);
}

/// The anonymous `TestPoint` of `mockIndexWriter(dir, conf, r)`: "Randomly
/// calls Thread.yield so we mixup thread scheduling".
struct YieldingTestPoint {
    random: JdkRandom,
}

impl TestPoint for YieldingTestPoint {
    fn apply(&self, _message: &str) {
        if self.random.next_int_bound(4) == 2 {
            std::thread::yield_now();
        }
    }
}

/// Port of the nested class `RandomIndexWriter.TestPointInfoStream` (`extends
/// InfoStream`): forwards the `TP` messages to a [`TestPoint`] and every
/// message its delegate enables to the delegate.
pub struct TestPointInfoStream {
    delegate: Arc<dyn InfoStream>,
    test_point: Arc<dyn TestPoint>,
}

impl TestPointInfoStream {
    /// `new TestPointInfoStream(delegate, testPoint)`: a `null` delegate is a
    /// `NullInfoStream`.
    pub fn new(delegate: Option<Arc<dyn InfoStream>>, test_point: Arc<dyn TestPoint>) -> Self {
        TestPointInfoStream {
            delegate: delegate.unwrap_or_else(|| Arc::new(NullInfoStream)),
            test_point,
        }
    }
}

impl InfoStream for TestPointInfoStream {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.tests.index.RandomIndexWriter$TestPointInfoStream"
    }

    fn message(&self, component: &str, message: &str) {
        if component == "TP" {
            self.test_point.apply(message);
        }
        if self.delegate.is_enabled(component) {
            self.delegate.message(component, message);
        }
    }

    fn is_enabled(&self, component: &str) -> bool {
        component == "TP" || self.delegate.is_enabled(component)
    }

    fn close(&self) {
        self.delegate.close();
    }
}

/// The counters of `maybeFlushOrCommit`: Java's plain `docCount`, `flushAt`
/// and `flushAtFactor` fields, which several threads update (racily in Java).
struct FlushCounters {
    doc_count: i32,
    flush_at: i32,
    flush_at_factor: f64,
}

/// Port of `org.apache.lucene.tests.index.RandomIndexWriter` (`implements
/// Closeable`): "Silly class that randomizes the indexing experience. EG it
/// may swap in a different merge policy/scheduler; may commit periodically;
/// may or may not forceMerge in the end, may flush by doc count instead of RAM,
/// etc."
pub struct RandomIndexWriter {
    w: Arc<IndexWriter>,
    r: JdkRandom,
    counters: Mutex<FlushCounters>,
    get_reader_called: AtomicBool,
    /// only if WE created it (then we close it)
    analyzer: Option<Arc<dyn Analyzer>>,
    soft_deletes_ratio: f64,
    config: Arc<LiveIndexWriterConfig>,
    do_random_force_merge: AtomicBool,
    do_random_force_merge_assert: AtomicBool,
}

impl RandomIndexWriter {
    /// `mockIndexWriter(dir, conf, r)`: "Returns an indexwriter that randomly
    /// mixes up thread scheduling (by yielding at test points)".
    ///
    /// # Errors
    ///
    /// What opening the writer throws.
    pub fn mock_index_writer(
        dir: Arc<dyn Directory>,
        conf: IndexWriterConfig,
        r: &dyn Random,
    ) -> Result<Arc<IndexWriter>> {
        // Randomly calls Thread.yield so we mixup thread scheduling
        let random = JdkRandom::new(r.next_long());
        Self::mock_index_writer_with_test_point(
            r,
            dir,
            conf,
            Arc::new(YieldingTestPoint { random }),
        )
    }

    /// `mockIndexWriter(r, dir, conf, testPoint)`: "Returns an indexwriter
    /// that enables the specified test point".
    ///
    /// # Errors
    ///
    /// What opening the reader or the writer throws.
    pub fn mock_index_writer_with_test_point(
        r: &dyn Random,
        dir: Arc<dyn Directory>,
        mut conf: IndexWriterConfig,
        test_point: Arc<dyn TestPoint>,
    ) -> Result<Arc<IndexWriter>> {
        let info_stream = conf.info_stream();
        conf.set_info_stream(Arc::new(TestPointInfoStream::new(
            Some(info_stream),
            test_point,
        )))?;
        let mut reader: Option<Arc<dyn DirectoryReader>> = None;
        if r.next_boolean()
            && directory_reader::index_exists(&*dir)?
            && conf.open_mode() != OpenMode::CREATE
        {
            if *VERBOSE {
                println!("RIW: open writer from reader");
            }
            let opened = directory_reader::open(Arc::clone(&dir))?;
            conf.set_index_commit(Some(Arc::from(opened.index_commit()?)));
            reader = Some(opened);
        }

        let iw = IndexWriter::new_with_test_points(dir, conf);
        if let Some(reader) = reader {
            match &iw {
                Ok(_) => reader.close()?,
                // `IOUtils.closeWhileHandlingException(reader)`
                Err(_) => {
                    let _ = reader.close();
                }
            }
        }
        iw
    }

    /// `new RandomIndexWriter(r, dir, a)`: "create a RandomIndexWriter with a
    /// random config".
    ///
    /// # Errors
    ///
    /// What opening the writer throws.
    pub fn new(r: &dyn Random, dir: Arc<dyn Directory>, a: Arc<dyn Analyzer>) -> Result<Self> {
        let c = LuceneTestCase::new_index_writer_config_with(r, a);
        Self::new_with_config(r, dir, c)
    }

    /// `new RandomIndexWriter(r, dir, c)`: "create a RandomIndexWriter with the
    /// provided config".
    ///
    /// # Errors
    ///
    /// What opening the writer throws.
    pub fn new_with_config(
        r: &dyn Random,
        dir: Arc<dyn Directory>,
        c: IndexWriterConfig,
    ) -> Result<Self> {
        let use_soft_deletes = r.next_boolean();
        Self::new_private(r, dir, c, false, use_soft_deletes)
    }

    /// `new RandomIndexWriter(r, dir, c, useSoftDeletes)`.
    ///
    /// # Errors
    ///
    /// What opening the writer throws.
    pub fn new_with_soft_deletes(
        r: &dyn Random,
        dir: Arc<dyn Directory>,
        c: IndexWriterConfig,
        use_soft_deletes: bool,
    ) -> Result<Self> {
        Self::new_private(r, dir, c, false, use_soft_deletes)
    }

    /// The private constructor `RandomIndexWriter(r, dir, c, closeAnalyzer,
    /// useSoftDeletes)`.
    fn new_private(
        r: &dyn Random,
        dir: Arc<dyn Directory>,
        mut c: IndexWriterConfig,
        close_analyzer: bool,
        use_soft_deletes: bool,
    ) -> Result<Self> {
        // TODO: this should be solved in a different way; Random should not be shared (!).
        let own_random = JdkRandom::new(r.next_long());
        let soft_deletes_ratio = if use_soft_deletes {
            c.set_soft_deletes_field(Some("___soft_deletes".to_string()));
            // `1.d / (double) 1 + r.nextInt(10)`: `(1.0 / 1.0) + nextInt(10)`,
            // which Java's precedence makes `1.0 + nextInt(10)`.
            1.0 + f64::from(r.next_int_bound(10))
        } else {
            0.0
        };

        let no_merge_policy = c
            .merge_policy()
            .as_any()
            .is_some_and(|any| any.is::<NoMergePolicy>());
        let w = Self::mock_index_writer(Arc::clone(&dir), c, r)?;
        let config = Arc::clone(w.get_config());
        let flush_at = TestUtil::next_int(r, 10, 1000);
        let analyzer = if close_analyzer {
            Some(w.get_analyzer()?)
        } else {
            None
        };
        if *VERBOSE {
            println!("RIW dir={}", dir.directory_to_string());
        }

        // Make sure we sometimes test indices that don't get
        // any forced merges:
        let do_random_force_merge = !no_merge_policy && r.next_boolean();
        Ok(RandomIndexWriter {
            w,
            r: own_random,
            counters: Mutex::new(FlushCounters {
                doc_count: 0,
                flush_at,
                flush_at_factor: 1.0,
            }),
            get_reader_called: AtomicBool::new(false),
            analyzer,
            soft_deletes_ratio,
            config,
            do_random_force_merge: AtomicBool::new(do_random_force_merge),
            do_random_force_merge_assert: AtomicBool::new(false),
        })
    }

    /// The public field `w`.
    pub fn w(&self) -> &Arc<IndexWriter> {
        &self.w
    }

    /// `addDocument(doc)`: "Adds a Document."
    ///
    /// # Errors
    ///
    /// What the writer throws.
    pub fn add_document(&self, doc: &Document) -> Result<i64> {
        LuceneTestCase::maybe_change_live_index_writer_config(&self.r, &self.config);
        let seq_no = if self.r.next_int_bound(5) == 3 {
            // TODO: maybe, we should simply buffer up added docs
            // (but we need to clone them), and only when
            // getReader, commit, etc. are called, we do an
            // addDocuments?  Would be better testing.
            self.w.add_documents(std::slice::from_ref(doc))?
        } else {
            self.w.add_document(doc)?
        };

        self.maybe_flush_or_commit()?;

        Ok(seq_no)
    }

    /// `maybeFlushOrCommit()`.
    fn maybe_flush_or_commit(&self) -> Result<()> {
        LuceneTestCase::maybe_change_live_index_writer_config(&self.r, &self.config);
        let doc_count = {
            let mut counters = self.counters.lock().unwrap_or_else(PoisonError::into_inner);
            let hit = counters.doc_count == counters.flush_at;
            counters.doc_count += 1;
            hit.then_some(counters.doc_count)
        };
        if let Some(doc_count) = doc_count {
            if self.r.next_boolean() {
                self.flush_all_buffers_sequentially(doc_count)?;
            } else if self.r.next_boolean() {
                if *VERBOSE {
                    println!("RIW.add/updateDocument: now doing a flush at docCount={doc_count}");
                }
                self.w.flush()?;
            } else {
                if *VERBOSE {
                    println!("RIW.add/updateDocument: now doing a commit at docCount={doc_count}");
                }
                self.w.commit()?;
            }
            let mut counters = self.counters.lock().unwrap_or_else(PoisonError::into_inner);
            let factor = counters.flush_at_factor;
            counters.flush_at +=
                TestUtil::next_int(&self.r, (factor * 10.0) as i32, (factor * 1000.0) as i32);
            if factor < 2e6 {
                // gradually but exponentially increase time b/w flushes
                counters.flush_at_factor *= 1.05;
            }
        }
        Ok(())
    }

    /// `flushAllBuffersSequentially()`.
    fn flush_all_buffers_sequentially(&self, doc_count: i32) -> Result<()> {
        if *VERBOSE {
            println!(
                "RIW.add/updateDocument: now flushing the largest writer at docCount={doc_count}"
            );
        }
        let thread_pool_size = index_writer_access().get_doc_writer_thread_pool_size(&self.w);
        let num_flushes = 1.min(self.r.next_int_bound(thread_pool_size + 1));
        for _ in 0..num_flushes {
            if !self.w.flush_next_buffer()? {
                break; // stop once we didn't flush anything
            }
        }
        Ok(())
    }

    /// `getReader()`: `getReader(true, false)`.
    ///
    /// # Errors
    ///
    /// What the writer and the reader throw.
    pub fn get_reader(&self) -> Result<Arc<dyn DirectoryReader>> {
        LuceneTestCase::maybe_change_live_index_writer_config(&self.r, &self.config);
        self.get_reader_with(true, false)
    }

    /// `setDoRandomForceMerge(v)`.
    pub fn set_do_random_force_merge(&self, v: bool) {
        self.do_random_force_merge.store(v, Ordering::Relaxed);
    }

    /// `setDoRandomForceMergeAssert(v)`.
    pub fn set_do_random_force_merge_assert(&self, v: bool) {
        self.do_random_force_merge_assert
            .store(v, Ordering::Relaxed);
    }

    /// `doRandomForceMerge()`.
    fn do_random_force_merge(&self) -> Result<()> {
        if self.do_random_force_merge.load(Ordering::Relaxed) {
            let access = index_writer_access();
            let seg_count = access.get_segment_count(&self.w)?;
            if self.r.next_boolean() || seg_count == 0 {
                // full forceMerge
                if *VERBOSE {
                    println!("RIW: doRandomForceMerge(1)");
                }
                self.w.force_merge(1)?;
            } else if self.r.next_boolean() {
                // partial forceMerge
                let limit = TestUtil::next_int(&self.r, 1, seg_count);
                if *VERBOSE {
                    println!("RIW: doRandomForceMerge({limit})");
                }
                self.w.force_merge(limit)?;
                let tiered = self
                    .config
                    .merge_policy()
                    .as_any()
                    .is_some_and(|any| any.is::<TieredMergePolicy>());
                if limit == 1 || !tiered {
                    let actual = access.get_segment_count(&self.w)?;
                    randomizedtesting::jdk::exception::java_assert(
                        !self.do_random_force_merge_assert.load(Ordering::Relaxed)
                            || actual <= limit,
                        || format!("limit={limit} actual={actual}"),
                    );
                }
            } else {
                if *VERBOSE {
                    println!("RIW: do random forceMergeDeletes()");
                }
                self.w.force_merge_deletes()?;
            }
        }
        Ok(())
    }

    /// `getReader(applyDeletions, writeAllDeletes)`.
    ///
    /// # Errors
    ///
    /// What the writer and the reader throw.
    pub fn get_reader_with(
        &self,
        apply_deletions: bool,
        write_all_deletes: bool,
    ) -> Result<Arc<dyn DirectoryReader>> {
        LuceneTestCase::maybe_change_live_index_writer_config(&self.r, &self.config);
        self.get_reader_called.store(true, Ordering::Relaxed);
        if self.r.next_int_bound(20) == 2 {
            self.do_random_force_merge()?;
        }
        if !apply_deletions || self.r.next_boolean() {
            // if we have soft deletes we can't open from a directory
            if *VERBOSE {
                println!("RIW.getReader: use NRT reader");
            }
            if self.r.next_int_bound(5) == 1 {
                self.w.commit()?;
            }
            index_writer_access().get_reader(&self.w, apply_deletions, write_all_deletes)
        } else {
            if *VERBOSE {
                println!("RIW.getReader: open new reader");
            }
            self.w.commit()?;
            if self.r.next_boolean() {
                let reader = directory_reader::open(Arc::clone(self.w.get_directory()))?;
                match self.config.soft_deletes_field() {
                    Some(field) => SoftDeletesDirectoryReaderWrapper::new(field).wrap(reader),
                    None => Ok(reader),
                }
            } else {
                index_writer_access().get_reader(&self.w, apply_deletions, write_all_deletes)
            }
        }
    }

    /// `close()`: "Close this writer."
    ///
    /// # Errors
    ///
    /// What the writer throws.
    pub fn close(&self) -> Result<()> {
        let access = index_writer_access();
        let result = (|| -> Result<()> {
            if !access.is_closed(&self.w) {
                LuceneTestCase::maybe_change_live_index_writer_config(&self.r, &self.config);
            }
            // if someone isn't using getReader() API, we want to be sure to
            // forceMerge since presumably they might open a reader on the dir.
            if !self.get_reader_called.load(Ordering::Relaxed)
                && self.r.next_int_bound(8) == 2
                && !access.is_closed(&self.w)
            {
                self.do_random_force_merge()?;
                if !self.config.commit_on_close() {
                    // index may have changed, must commit the changes, or otherwise they are
                    // discarded by the call to close()
                    self.w.commit()?;
                }
            }
            Ok(())
        })();
        match result {
            // `IOUtils.close(w, analyzer)`
            Ok(()) => {
                let closed = self.w.close();
                if let Some(analyzer) = &self.analyzer {
                    analyzer.close();
                }
                closed
            }
            // `IOUtils.closeWhileHandlingException(w, analyzer)`
            Err(e) => {
                let _ = self.w.close();
                if let Some(analyzer) = &self.analyzer {
                    analyzer.close();
                }
                Err(e)
            }
        }
    }

    /// `flush()`: "Writes all in-memory segments to the Directory."
    ///
    /// # Errors
    ///
    /// What the writer throws.
    pub fn flush(&self) -> Result<()> {
        self.w.flush()
    }
}

impl fmt::Debug for RandomIndexWriter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RandomIndexWriter")
            .field("soft_deletes_ratio", &self.soft_deletes_ratio)
            .finish_non_exhaustive()
    }
}
