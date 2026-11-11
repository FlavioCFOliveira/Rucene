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

//! Port of `org.apache.lucene.tests.index.RandomIndexWriter`.

use std::fmt;
use std::panic::{catch_unwind, resume_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use randomizedtesting::jdk::random::{JdkRandom, Random};
use rucene::analysis::Analyzer;
use rucene::document::{Field, FieldType, NumericValue};
use rucene::error::{LuceneError, Result};
use rucene::index::codec_reader::CodecReader;
use rucene::index::directory_reader::{self, DirectoryReader};
use rucene::index::filter_directory_reader::{
    DirectoryReaderWrapper, SoftDeletesDirectoryReaderWrapper,
};
use rucene::index::index_writer::DocStats;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::{IndexWriterConfig, LiveIndexWriterConfig, OpenMode};
use rucene::index::merge_policy::MergeObserver;
use rucene::index::merge_policy::{NoMergePolicy, TieredMergePolicy};
use rucene::index::terms::Term;
use rucene::index::{DocValuesType, DocumentsIterable, DocumentsIterator, IndexableFieldIterable};
use rucene::internal::tests_hooks::{IndexWriterAccess, TestSecrets};
use rucene::search::term_query::TermQuery;
use rucene::search::Query;
use rucene::store::Directory;
use rucene::util::BytesRef;
use rucene::util::InfoStream;

use crate::analysis::MockAnalyzer;
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
        conf: impl Into<Arc<IndexWriterConfig>>,
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
        conf: impl Into<Arc<IndexWriterConfig>>,
        test_point: Arc<dyn TestPoint>,
    ) -> Result<Arc<IndexWriter>> {
        let shared: Arc<IndexWriterConfig> = conf.into();
        let conf = &*shared;
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

        let iw = IndexWriter::new_with_test_points(dir, shared);
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

    /// `new RandomIndexWriter(r, dir)`: "create a RandomIndexWriter with a
    /// random config: Uses MockAnalyzer", which it closes on [`close`](Self::close).
    ///
    /// # Errors
    ///
    /// What opening the writer throws.
    pub fn new_with_mock_analyzer(r: &dyn Random, dir: Arc<dyn Directory>) -> Result<Self> {
        // Java evaluates the arguments of `this(...)` left to right.
        let analyzer: Arc<dyn Analyzer> = Arc::new(MockAnalyzer::new(r));
        let c = LuceneTestCase::new_index_writer_config_with(r, Some(analyzer));
        let use_soft_deletes = r.next_boolean();
        Self::new_private(r, dir, Arc::new(c), true, use_soft_deletes)
    }

    /// `new RandomIndexWriter(r, dir, a)`: "create a RandomIndexWriter with a
    /// random config".
    ///
    /// # Errors
    ///
    /// What opening the writer throws.
    pub fn new(r: &dyn Random, dir: Arc<dyn Directory>, a: Arc<dyn Analyzer>) -> Result<Self> {
        let c = LuceneTestCase::new_index_writer_config_with(r, Some(a));
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
        c: impl Into<Arc<IndexWriterConfig>>,
    ) -> Result<Self> {
        let use_soft_deletes = r.next_boolean();
        Self::new_private(r, dir, c.into(), false, use_soft_deletes)
    }

    /// `new RandomIndexWriter(r, dir, c, useSoftDeletes)`.
    ///
    /// # Errors
    ///
    /// What opening the writer throws.
    pub fn new_with_soft_deletes(
        r: &dyn Random,
        dir: Arc<dyn Directory>,
        c: impl Into<Arc<IndexWriterConfig>>,
        use_soft_deletes: bool,
    ) -> Result<Self> {
        Self::new_private(r, dir, c.into(), false, use_soft_deletes)
    }

    /// The private constructor `RandomIndexWriter(r, dir, c, closeAnalyzer,
    /// useSoftDeletes)`.
    fn new_private(
        r: &dyn Random,
        dir: Arc<dyn Directory>,
        shared: Arc<IndexWriterConfig>,
        close_analyzer: bool,
        use_soft_deletes: bool,
    ) -> Result<Self> {
        let c = &*shared;
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
        let w = Self::mock_index_writer(Arc::clone(&dir), shared, r)?;
        let config = Arc::clone(w.get_config());
        let flush_at = TestUtil::next_int(r, 10, 1000);
        // `closeAnalyzer ? w.getAnalyzer() : null`; `IOUtils.close` skips a
        // `null` analyzer.
        let analyzer = if close_analyzer {
            w.get_analyzer()?
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
    /// `doc` is any `Iterable<T extends IndexableField>`; `None` is Java's
    /// `null`, which the writer rejects.
    ///
    /// # Errors
    ///
    /// What the writer throws.
    pub fn add_document<'d, D>(&self, doc: impl Into<Option<&'d D>>) -> Result<i64>
    where
        D: IndexableFieldIterable + 'd,
    {
        let doc = doc.into();
        LuceneTestCase::maybe_change_live_index_writer_config(&self.r, &self.config);
        let seq_no = if self.r.next_int_bound(5) == 3 {
            // TODO: maybe, we should simply buffer up added docs
            // (but we need to clone them), and only when
            // getReader, commit, etc. are called, we do an
            // addDocuments?  Would be better testing.
            self.w.add_documents(&SingleDocIterable(
                doc.map(|doc| doc as &dyn IndexableFieldIterable),
            ))?
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

    /// `addDocuments(docs)`.
    ///
    /// # Errors
    ///
    /// What the writer throws.
    pub fn add_documents<'d, D>(&self, docs: impl Into<Option<&'d D>>) -> Result<i64>
    where
        D: DocumentsIterable + ?Sized + 'd,
    {
        LuceneTestCase::maybe_change_live_index_writer_config(&self.r, &self.config);
        let seq_no = self.w.add_documents(docs)?;
        self.maybe_flush_or_commit()?;
        Ok(seq_no)
    }

    /// `updateDocuments(delTerm, docs)`.
    ///
    /// # Errors
    ///
    /// What the writer throws.
    pub fn update_documents<'d, D>(
        &self,
        del_term: Term,
        docs: impl Into<Option<&'d D>>,
    ) -> Result<i64>
    where
        D: DocumentsIterable + ?Sized + 'd,
    {
        let docs = docs.into();
        LuceneTestCase::maybe_change_live_index_writer_config(&self.r, &self.config);
        let seq_no = if self.use_soft_deletes() {
            self.w
                .soft_update_documents(del_term, docs, &[self.soft_deletes_field()?])?
        } else if self.r.next_int_bound(10) < 3 {
            // 30% chance
            let query: Arc<dyn Query> = Arc::new(TermQuery::new(del_term));
            self.w.update_documents_by_query(Some(query), docs)?
        } else {
            self.w.update_documents(del_term, docs)?
        };
        self.maybe_flush_or_commit()?;
        Ok(seq_no)
    }

    /// `useSoftDeletes()`: `r.nextDouble() < softDeletesRatio`.
    fn use_soft_deletes(&self) -> bool {
        self.r.next_double() < self.soft_deletes_ratio
    }

    /// `new NumericDocValuesField(config.getSoftDeletesField(), 1)`, as the
    /// `Field` the writer's soft updates take.
    fn soft_deletes_field(&self) -> Result<Field> {
        // Java passes `config.getSoftDeletesField()` even when it is null,
        // which `NumericDocValuesField`'s constructor rejects; a soft update
        // only runs with a positive `softDeletesRatio`, which sets the field.
        let name = self
            .config
            .soft_deletes_field()
            .ok_or_else(|| LuceneError::illegal_argument("name must not be null".to_string()))?;
        let field_type = FieldType::new();
        field_type.set_doc_values_type(DocValuesType::NUMERIC)?;
        field_type.freeze();
        Field::new_with_number(&name, NumericValue::Long(1), field_type)
    }

    /// `updateDocument(t, doc)`: "Updates a document."
    ///
    /// # Errors
    ///
    /// What the writer throws.
    pub fn update_document<'d, D>(&self, t: Term, doc: impl Into<Option<&'d D>>) -> Result<i64>
    where
        D: IndexableFieldIterable + 'd,
    {
        let doc = doc.into();
        // `Arrays.asList(doc)`, which holds a `null` element as is.
        let as_list = SingleDocIterable(doc.map(|doc| doc as &dyn IndexableFieldIterable));
        LuceneTestCase::maybe_change_live_index_writer_config(&self.r, &self.config);
        let seq_no = if self.use_soft_deletes() {
            if self.r.next_int_bound(5) == 3 {
                self.w
                    .soft_update_documents(t, &as_list, &[self.soft_deletes_field()?])?
            } else {
                self.w
                    .soft_update_document(t, doc, &[self.soft_deletes_field()?])?
            }
        } else if self.r.next_int_bound(5) == 3 {
            self.w.update_documents(t, &as_list)?
        } else {
            self.w.update_document(t, doc)?
        };
        self.maybe_flush_or_commit()?;

        Ok(seq_no)
    }

    /// `addIndexes(Directory... dirs)`.
    ///
    /// # Errors
    ///
    /// What the writer throws.
    pub fn add_indexes_dirs(&self, dirs: &[Arc<dyn Directory>]) -> Result<i64> {
        LuceneTestCase::maybe_change_live_index_writer_config(&self.r, &self.config);
        self.w.add_indexes_dirs(dirs)
    }

    /// `addIndexes(CodecReader... readers)`.
    ///
    /// # Errors
    ///
    /// What the writer throws.
    pub fn add_indexes_codec_readers(&self, readers: &[Arc<dyn CodecReader>]) -> Result<i64> {
        LuceneTestCase::maybe_change_live_index_writer_config(&self.r, &self.config);
        self.w.add_indexes_codec_readers(readers)
    }

    /// `updateNumericDocValue(term, field, value)`.
    ///
    /// # Errors
    ///
    /// What the writer throws.
    pub fn update_numeric_doc_value(&self, term: Term, field: &str, value: i64) -> Result<i64> {
        LuceneTestCase::maybe_change_live_index_writer_config(&self.r, &self.config);
        self.w.update_numeric_doc_value(term, field, value)
    }

    /// `updateBinaryDocValue(term, field, value)`.
    ///
    /// # Errors
    ///
    /// What the writer throws.
    pub fn update_binary_doc_value(&self, term: Term, field: &str, value: BytesRef) -> Result<i64> {
        LuceneTestCase::maybe_change_live_index_writer_config(&self.r, &self.config);
        self.w.update_binary_doc_value(term, field, value)
    }

    /// `updateDocValues(term, updates...)`.
    ///
    /// # Errors
    ///
    /// What the writer throws.
    pub fn update_doc_values(&self, term: Term, updates: &[Field]) -> Result<i64> {
        LuceneTestCase::maybe_change_live_index_writer_config(&self.r, &self.config);
        self.w.update_doc_values(term, updates)
    }

    /// `deleteDocuments(Term term)`.
    ///
    /// # Errors
    ///
    /// What the writer throws.
    pub fn delete_documents(&self, term: Term) -> Result<i64> {
        LuceneTestCase::maybe_change_live_index_writer_config(&self.r, &self.config);
        self.w.delete_documents_by_terms(vec![term])
    }

    /// `deleteDocuments(Query q)`.
    ///
    /// # Errors
    ///
    /// What the writer throws.
    pub fn delete_documents_by_query(&self, q: Arc<dyn Query>) -> Result<i64> {
        LuceneTestCase::maybe_change_live_index_writer_config(&self.r, &self.config);
        self.w.delete_documents_by_queries(vec![q])
    }

    /// `commit()`: `commit(r.nextInt(10) == 0)`.
    ///
    /// # Errors
    ///
    /// What the writer throws.
    pub fn commit(&self) -> Result<i64> {
        let flush_concurrently = self.r.next_int_bound(10) == 0;
        self.commit_with(flush_concurrently)
    }

    /// `commit(flushConcurrently)`: when `flushConcurrently`, a second thread
    /// runs `flushAllBuffersSequentially()` while this one commits.
    ///
    /// Java's `return w.commit()` inside the `try` returns once the thread is
    /// joined, whatever the thread threw; when the commit throws, the first
    /// throwable recorded (the thread's or the commit's) is rethrown. Java
    /// adds the later ones to it as suppressed exceptions, which
    /// [`LuceneError`] cannot hold: they are dropped.
    ///
    /// # Errors
    ///
    /// What the writer throws.
    pub fn commit_with(&self, flush_concurrently: bool) -> Result<i64> {
        LuceneTestCase::maybe_change_live_index_writer_config(&self.r, &self.config);
        if flush_concurrently {
            enum Thrown {
                Error(LuceneError),
                Panic(Box<dyn std::any::Any + Send>),
            }
            let throwable_list: Mutex<Vec<Thrown>> = Mutex::new(Vec::new());
            // `new Thread(...)` (`RandomIndexWriter.java`): the thread is in
            // this thread's group, so it runs under the test's randomized
            // context, as `random()` in a directory failure needs.
            let context = randomizedtesting::jdk::thread::Thread::start_context_scoped();
            let committed = std::thread::scope(|scope| {
                let thread = scope.spawn(|| {
                    context(Box::new(|| {
                        let doc_count = self
                            .counters
                            .lock()
                            .unwrap_or_else(PoisonError::into_inner)
                            .doc_count;
                        match catch_unwind(AssertUnwindSafe(|| {
                            self.flush_all_buffers_sequentially(doc_count)
                        })) {
                            Ok(Ok(())) => {}
                            Ok(Err(e)) => throwable_list
                                .lock()
                                .unwrap_or_else(PoisonError::into_inner)
                                .push(Thrown::Error(e)),
                            Err(payload) => throwable_list
                                .lock()
                                .unwrap_or_else(PoisonError::into_inner)
                                .push(Thrown::Panic(payload)),
                        }
                    }))
                });
                let committed = match catch_unwind(AssertUnwindSafe(|| self.w.commit())) {
                    Ok(Ok(seq_no)) => Some(seq_no),
                    Ok(Err(e)) => {
                        throwable_list
                            .lock()
                            .unwrap_or_else(PoisonError::into_inner)
                            .push(Thrown::Error(e));
                        None
                    }
                    Err(payload) => {
                        throwable_list
                            .lock()
                            .unwrap_or_else(PoisonError::into_inner)
                            .push(Thrown::Panic(payload));
                        None
                    }
                };
                // make sure we wait for the thread to join otherwise it might
                // still be processing events and the IW won't be fully closed
                // in the case of a fatal exception
                let _ = thread.join();
                committed
            });
            if let Some(seq_no) = committed {
                return Ok(seq_no);
            }
            let mut throwable_list = throwable_list
                .into_inner()
                .unwrap_or_else(PoisonError::into_inner);
            if !throwable_list.is_empty() {
                match throwable_list.swap_remove(0) {
                    Thrown::Error(e) => return Err(e),
                    Thrown::Panic(payload) => resume_unwind(payload),
                }
            }
        }
        self.w.commit()
    }

    /// `getDocStats()`.
    ///
    /// # Errors
    ///
    /// What the writer throws.
    pub fn get_doc_stats(&self) -> Result<DocStats> {
        self.w.get_doc_stats()
    }

    /// `deleteAll()`.
    ///
    /// # Errors
    ///
    /// What the writer throws.
    pub fn delete_all(&self) -> Result<i64> {
        self.w.delete_all()
    }

    /// `forceMergeDeletes(doWait)`.
    ///
    /// # Errors
    ///
    /// What the writer throws.
    pub fn force_merge_deletes_with(&self, do_wait: bool) -> Result<MergeObserver> {
        LuceneTestCase::maybe_change_live_index_writer_config(&self.r, &self.config);
        self.w.force_merge_deletes_with(do_wait)
    }

    /// `forceMergeDeletes()`.
    ///
    /// # Errors
    ///
    /// What the writer throws.
    pub fn force_merge_deletes(&self) -> Result<MergeObserver> {
        LuceneTestCase::maybe_change_live_index_writer_config(&self.r, &self.config);
        self.w.force_merge_deletes()
    }

    /// `forceMerge(maxSegmentCount)`: "Forces a forceMerge.
    ///
    /// NOTE: this should be avoided in tests unless absolutely necessary, as
    /// it will result in less test coverage."
    ///
    /// # Errors
    ///
    /// What the writer throws.
    pub fn force_merge(&self, max_segment_count: i32) -> Result<()> {
        LuceneTestCase::maybe_change_live_index_writer_config(&self.r, &self.config);
        self.w.force_merge(max_segment_count)
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

/// The one-document `Iterable<Iterable<T>>` `addDocument` hands to
/// `w.addDocuments(...)` (its anonymous class) and the `Arrays.asList(doc)` of
/// `updateDocument`: one element, which may be `null`.
struct SingleDocIterable<'a>(Option<&'a dyn IndexableFieldIterable>);

/// The iterator of [`SingleDocIterable`]: `done` flips on the one `next()`.
struct SingleDocIterator<'a> {
    doc: Option<&'a dyn IndexableFieldIterable>,
    done: bool,
}

impl<'a> DocumentsIterator<'a> for SingleDocIterator<'a> {
    fn has_next(&mut self) -> Result<bool> {
        Ok(!self.done)
    }

    fn next(&mut self) -> Result<Option<&'a dyn IndexableFieldIterable>> {
        if self.done {
            return Err(LuceneError::illegal_state(String::new()));
        }
        self.done = true;
        Ok(self.doc)
    }
}

impl DocumentsIterable for SingleDocIterable<'_> {
    fn iterator(&self) -> Box<dyn DocumentsIterator<'_> + '_> {
        Box::new(SingleDocIterator {
            doc: self.0,
            done: false,
        })
    }
}
