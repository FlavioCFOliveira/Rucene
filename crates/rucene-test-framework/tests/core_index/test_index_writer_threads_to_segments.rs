//! Port of `lucene/core/src/test/org/apache/lucene/index/TestIndexWriterThreadsToSegments.java`.
//!
//! * A `Thread` whose `run` catches an `Exception` and throws
//!   `new RuntimeException(e)` panics with that throwable.
//! * `CheckSegmentCount` is the barrier action of the `CyclicBarrier`; its
//!   reader lives behind a mutex, since the action runs on whichever
//!   thread trips the barrier.

use std::collections::HashSet;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::jdk::thread::Thread;
use randomizedtesting::junit::Assert;
use rucene::document::{Document, Store, TextField};
use rucene::error::{LuceneError, Result};
use rucene::index::directory_reader::{self, DirectoryReader};
use rucene::index::index_file_names;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::merge_policy::NoMergePolicy;
use rucene::index::segment_reader::SegmentReader;
use rucene::index::terms::Term;
use rucene::index::{IndexReader, SegmentCommitInfo};
use rucene::store::{Directory, DEFAULT_IO_CONTEXT};
use rucene::util::string_helper::{StringHelper, ID_LENGTH};
use rucene::util::Version;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::index::RandomIndexWriter;
use rucene_test_framework::util::java_exceptions::{is_instance, java_throwable};
use rucene_test_framework::util::lucene_test_case::{Nightly, SuppressCodecs, TEST_NIGHTLY};
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::{assert_equals, class_annotated, random, test, verbose};
use crate::support::{CountDownLatch, CyclicBarrier};

const ACE: &str = "org.apache.lucene.store.AlreadyClosedException";
const ISE: &str = "java.lang.IllegalStateException";

/// `TestIndexWriterThreadsToSegments` (`@SuppressCodecs({"SimpleText", "Direct"})`).
pub fn class_def() -> Arc<Class> {
    class_annotated(
        "org.apache.lucene.index.TestIndexWriterThreadsToSegments",
        vec![
            test(
                "testSegmentCountOnFlushBasic",
                test_segment_count_on_flush_basic,
            ),
            test(
                "testSegmentCountOnFlushRandom",
                test_segment_count_on_flush_random,
            ),
            test("testManyThreadsClose", test_many_threads_close),
            test("testDocsStuckInRAMForever", test_docs_stuck_in_ram_forever).annotate(Nightly),
        ],
        |b| {
            b.annotate(SuppressCodecs {
                value: vec!["SimpleText".to_string(), "Direct".to_string()],
            })
        },
    )
}

/// `throw new RuntimeException(e)`.
fn runtime_exception(e: &LuceneError) -> ! {
    Throwable::from_cause("java.lang.RuntimeException", java_throwable(e)).throw()
}

/// `new IndexWriterConfig(new MockAnalyzer(random()))`.
fn plain_iwc() -> IndexWriterConfig {
    IndexWriterConfig::with_analyzer(Some(Arc::new(MockAnalyzer::new(&*random()))))
}

/// `reader` as the `IndexReader` it is.
fn ir(reader: &Arc<dyn DirectoryReader>) -> Arc<dyn IndexReader> {
    Arc::clone(reader) as Arc<dyn IndexReader>
}

/// LUCENE-5644: for first segment, two threads each indexed one doc (likely concurrently), but for
/// second segment, each thread indexed the
/// doc NOT at the same time, and should have shared the same thread state / segment
///
/// Port of `TestIndexWriterThreadsToSegments.testSegmentCountOnFlushBasic`.
fn test_segment_count_on_flush_basic() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let w = IndexWriter::new(dir.as_directory(), plain_iwc())?;
    let starting_gun = Arc::new(CountDownLatch::new(1));
    let start_done = Arc::new(CountDownLatch::new(2));
    let middle_gun = Arc::new(CountDownLatch::new(1));
    let final_gun = Arc::new(CountDownLatch::new(1));
    let mut threads = Vec::with_capacity(2);
    for i in 0..2 {
        let thread_id = i;
        let w = Arc::clone(&w);
        let starting_gun = Arc::clone(&starting_gun);
        let start_done = Arc::clone(&start_done);
        let middle_gun = Arc::clone(&middle_gun);
        let final_gun = Arc::clone(&final_gun);
        let t = Thread::unnamed(move || {
            let run = || -> Result<()> {
                starting_gun.await_();
                let mut doc = Document::new();
                doc.add(Box::new(LuceneTestCase::new_text_field(
                    "field",
                    "here is some text",
                    Store::NO,
                )));
                w.add_document(&doc)?;
                start_done.count_down();
                middle_gun.await_();
                if thread_id == 0 {
                    w.add_document(&doc)?;
                } else {
                    final_gun.await_();
                    w.add_document(&doc)?;
                }
                Ok(())
            };
            if let Err(e) = run() {
                runtime_exception(&e);
            }
        });
        t.start();
        threads.push(t);
    }
    starting_gun.count_down();
    start_done.await_();
    let r = directory_reader::open_with_writer(&*w)?;
    assert_equals(&2, &r.num_docs());
    let num_segments = ir(&r).leaves()?.len();
    // 1 segment if the threads ran sequentially, else 2:
    Assert::assert_true(num_segments <= 2);
    r.close()?;
    middle_gun.count_down();
    let _ = threads[0].join();
    final_gun.count_down();
    let _ = threads[1].join();
    let r = directory_reader::open_with_writer(&*w)?;
    assert_equals(&4, &r.num_docs());
    // Both threads should have shared a single thread state since they did not try to index
    // concurrently:
    assert_equals(&(1 + num_segments), &ir(&r).leaves()?.len());
    r.close()?;
    w.close()?;
    drop(w);
    dir.close()
}

/// Maximum number of simultaneous threads to use for each iteration.
const MAX_THREADS_AT_ONCE: i32 = 10;

/// `CheckSegmentCount`.
struct CheckSegmentCount {
    max_thread_count_per_iter: Arc<AtomicI32>,
    indexing_count: Arc<AtomicI32>,
    r: Mutex<Option<Arc<dyn DirectoryReader>>>,
}

impl CheckSegmentCount {
    fn new(
        w: &IndexWriter,
        max_thread_count_per_iter: Arc<AtomicI32>,
        indexing_count: Arc<AtomicI32>,
    ) -> Result<Self> {
        let r = directory_reader::open_with_writer(w)?;
        assert_equals(&0, &ir(&r).leaves()?.len());
        let checker = CheckSegmentCount {
            max_thread_count_per_iter,
            indexing_count,
            r: Mutex::new(Some(r)),
        };
        checker.set_next_iter_thread_count();
        Ok(checker)
    }

    /// `run()`.
    fn run(&self) {
        let result = (|| -> Result<()> {
            let mut guard = self.r.lock().unwrap_or_else(|p| p.into_inner());
            let r = guard.clone().unwrap_or_else(|| crate::support::npe("r"));
            let old_segment_count = ir(&r).leaves()?.len() as i32;
            let r2 = directory_reader::open_if_changed(Arc::clone(&r))?;
            Assert::assert_true(r2.is_some()); // assertNotNull(r2)
            let r2 = r2.expect("checked above");
            r.close()?;
            *guard = Some(Arc::clone(&r2));
            let max_expected_segments =
                old_segment_count + self.max_thread_count_per_iter.load(Ordering::SeqCst);
            if verbose() {
                println!(
                    "TEST: iter done; now verify oldSegCount={old_segment_count} newSegCount={} maxExpected={max_expected_segments}",
                    ir(&r2).leaves()?.len()
                );
            }
            // NOTE: it won't necessarily be ==, in case some threads were strangely scheduled and never
            // conflicted with one another (should be uncommon...?):
            Assert::assert_true(ir(&r2).leaves()?.len() as i32 <= max_expected_segments);
            self.set_next_iter_thread_count();
            Ok(())
        })();
        if let Err(e) = result {
            runtime_exception(&e);
        }
    }

    fn set_next_iter_thread_count(&self) {
        self.indexing_count.store(0, Ordering::SeqCst);
        self.max_thread_count_per_iter.store(
            TestUtil::next_int(&*random(), 1, MAX_THREADS_AT_ONCE),
            Ordering::SeqCst,
        );
        if verbose() {
            println!(
                "TEST: iter set maxThreadCount={}",
                self.max_thread_count_per_iter.load(Ordering::SeqCst)
            );
        }
    }

    /// `close()`.
    fn close(&self) -> Result<()> {
        let r = self.r.lock().unwrap_or_else(|p| p.into_inner()).take();
        match r {
            Some(r) => r.close(),
            None => crate::support::npe("r"),
        }
    }
}

/// LUCENE-5644: index docs w/ multiple threads but in between flushes we limit how many threads
/// can index concurrently in the next
/// iteration, and then verify that no more segments were flushed than number of threads:
///
/// Port of `TestIndexWriterThreadsToSegments.testSegmentCountOnFlushRandom`.
fn test_segment_count_on_flush_random() -> Result<()> {
    let mut dir = LuceneTestCase::new_fs_directory(&LuceneTestCase::create_temp_dir());
    let iwc = plain_iwc();
    // Never trigger flushes (so we only flush on getReader):
    iwc.set_max_buffered_docs(100000000)?;
    iwc.set_ram_buffer_size_mb(-1.0)?;

    // Never trigger merges (so we can simplistically count flushed segments):
    iwc.set_merge_policy(Some(Arc::new(NoMergePolicy::new())))?;

    let w = IndexWriter::new(dir.as_directory(), iwc)?;

    // How many threads are indexing in the current cycle:
    let indexing_count = Arc::new(AtomicI32::new(0));

    // How many threads we will use on each cycle:
    let max_thread_count = Arc::new(AtomicI32::new(0));

    let checker = Arc::new(CheckSegmentCount::new(
        &w,
        Arc::clone(&max_thread_count),
        Arc::clone(&indexing_count),
    )?);

    // We spin up 10 threads up front, but then in between flushes we limit how many can run on each
    // iteration
    let iters = if *TEST_NIGHTLY { 300 } else { 10 };
    let mut threads = Vec::with_capacity(MAX_THREADS_AT_ONCE as usize);

    // We use this to stop all threads once they've indexed their docs in the current iter, and pull
    // a new NRT reader, and verify the
    // segment count:
    let barrier = {
        let checker = Arc::clone(&checker);
        Arc::new(CyclicBarrier::with_action(MAX_THREADS_AT_ONCE, move || {
            checker.run()
        }))
    };

    for _ in 0..MAX_THREADS_AT_ONCE {
        let w = Arc::clone(&w);
        let indexing_count = Arc::clone(&indexing_count);
        let max_thread_count = Arc::clone(&max_thread_count);
        let barrier = Arc::clone(&barrier);
        let t = Thread::unnamed(move || {
            let run = || -> Result<()> {
                for _ in 0..iters {
                    if indexing_count.fetch_add(1, Ordering::SeqCst) + 1
                        <= max_thread_count.load(Ordering::SeqCst)
                    {
                        if verbose() {
                            println!("TEST: {}: do index", Thread::current().name());
                        }

                        // We get to index on this cycle:
                        let mut doc = Document::new();
                        doc.add(Box::new(TextField::new(
                            "field",
                            "here is some text that is a bit longer than normal trivial text"
                                .to_string(),
                            Store::NO,
                        )?));
                        for _ in 0..200 {
                            w.add_document(&doc)?;
                        }
                    } else {
                        // We lose: no indexing for us on this cycle
                        if verbose() {
                            println!("TEST: {}: don't index", Thread::current().name());
                        }
                    }
                    if let Err(be) = barrier.await_() {
                        Throwable::from_cause(
                            "java.lang.RuntimeException",
                            Throwable::new(be.java_class(), ""),
                        )
                        .throw();
                    }
                }
                Ok(())
            };
            if let Err(e) = run() {
                runtime_exception(&e);
            }
        });
        t.start();
        threads.push(t);
    }

    for t in &threads {
        let _ = t.join();
    }

    // IOUtils.close(checker, w, dir)
    let mut first: Option<LuceneError> = None;
    if let Err(e) = checker.close() {
        first.get_or_insert(e);
    }
    if let Err(e) = w.close() {
        first.get_or_insert(e);
    }
    drop(w);
    if let Err(e) = dir.close() {
        first.get_or_insert(e);
    }
    match first {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

/// Port of `TestIndexWriterThreadsToSegments.testManyThreadsClose`.
fn test_many_threads_close() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let r = random();
    let iwc =
        LuceneTestCase::new_index_writer_config_with(&*r, Some(Arc::new(MockAnalyzer::new(&*r))));
    iwc.set_commit_on_close(false);
    let w = Arc::new(RandomIndexWriter::new_with_config(
        &*r,
        dir.as_directory(),
        iwc,
    )?);
    TestUtil::reduce_open_files(w.w())?;
    w.set_do_random_force_merge(false);
    let n = TestUtil::next_int(&*random(), 4, 30);
    let mut threads = Vec::with_capacity(n as usize);
    let starting_gun = Arc::new(CountDownLatch::new(1));
    for _ in 0..n {
        let w = Arc::clone(&w);
        let starting_gun = Arc::clone(&starting_gun);
        let t = Thread::unnamed(move || {
            let run = || -> Result<()> {
                starting_gun.await_();
                let mut doc = Document::new();
                doc.add(Box::new(TextField::new(
                    "field",
                    "here is some text that is a bit longer than normal trivial text".to_string(),
                    Store::NO,
                )?));
                for _ in 0..1000 {
                    w.add_document(&doc)?;
                }
                Ok(())
            };
            match run() {
                Ok(()) => {}
                Err(ace) if is_instance(&ace, ACE) => {
                    // ok
                }
                Err(e) => runtime_exception(&e),
            }
        });
        t.start();
        threads.push(t);
    }
    starting_gun.count_down();

    std::thread::sleep(std::time::Duration::from_millis(100));
    match w.close() {
        Ok(()) => {}
        Err(ise) if is_instance(&ise, ISE) => {
            // OK but not required
        }
        Err(e) => return Err(e),
    }
    for t in &threads {
        let _ = t.join();
    }
    w.close()?;
    drop(w);
    dir.close()
}

/// TODO: can we make this test more efficient so it doesn't need to be nightly?
///
/// Port of `TestIndexWriterThreadsToSegments.testDocsStuckInRAMForever`.
fn test_docs_stuck_in_ram_forever() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let iwc = plain_iwc();
    iwc.set_ram_buffer_size_mb(0.2)?;
    let codec = TestUtil::get_default_codec();
    iwc.set_codec(Arc::clone(&codec))?;
    iwc.set_merge_policy(Some(Arc::new(NoMergePolicy::new())))?;
    let w = IndexWriter::new(dir.as_directory(), iwc)?;
    let starting_gun = Arc::new(CountDownLatch::new(1));
    let mut threads = Vec::with_capacity(2);
    for i in 0..2 {
        let thread_id = i;
        let w = Arc::clone(&w);
        let starting_gun = Arc::clone(&starting_gun);
        let t = Thread::unnamed(move || {
            let run = || -> Result<()> {
                starting_gun.await_();
                for _ in 0..1000 {
                    let mut doc = Document::new();
                    doc.add(Box::new(LuceneTestCase::new_string_field(
                        "field",
                        &format!("threadID{thread_id}"),
                        Store::NO,
                    )));
                    w.add_document(&doc)?;
                }
                Ok(())
            };
            if let Err(e) = run() {
                runtime_exception(&e);
            }
        });
        t.start();
        threads.push(t);
    }

    starting_gun.count_down();
    for t in &threads {
        let _ = t.join();
    }

    let mut seg_seen: HashSet<String> = HashSet::new();
    let mut thread0_count = 0;
    let mut thread1_count = 0;

    // At this point the writer should have 2 thread states w/ docs; now we index with only 1 thread
    // until we see all 1000 thread0 & thread1
    // docs flushed.  If the writer incorrectly holds onto previously indexed docs forever then this
    // will run forever:
    let mut counter: i64 = 0;
    let mut check_at: i64 = 100;
    while thread0_count < 1000 || thread1_count < 1000 {
        let mut doc = Document::new();
        doc.add(Box::new(LuceneTestCase::new_string_field(
            "field",
            "threadIDmain",
            Store::NO,
        )));
        w.add_document(&doc)?;
        let c = counter;
        counter += 1;
        if c == check_at {
            for file_name in dir.as_directory().list_all()? {
                if file_name.ends_with(".si") {
                    let seg_name = index_file_names::parse_segment_name(&file_name).to_string();
                    if !seg_seen.contains(&seg_name) {
                        seg_seen.insert(seg_name.clone());
                        let id = read_segment_info_id(&*dir.as_directory(), &file_name)?;
                        let mut si = TestUtil::get_default_codec().segment_info_format().read(
                            dir.as_directory(),
                            &seg_name,
                            &id,
                            &*DEFAULT_IO_CONTEXT,
                        )?;
                        si.set_codec(Arc::clone(&codec));
                        let sci = SegmentCommitInfo::new(
                            si,
                            0,
                            0,
                            -1,
                            -1,
                            -1,
                            StringHelper::random_id(),
                        )?;
                        let sr = SegmentReader::new(
                            sci,
                            i32::from(Version::LATEST.major),
                            &*DEFAULT_IO_CONTEXT,
                        )?;
                        let counted = (|| -> Result<()> {
                            thread0_count += sr.doc_freq(&Term::from_text("field", "threadID0"))?;
                            thread1_count += sr.doc_freq(&Term::from_text("field", "threadID1"))?;
                            Ok(())
                        })();
                        sr.close()?;
                        counted?;
                    }
                }
            }

            check_at = (check_at as f64 * 1.25) as i64;
            counter = 0;
        }
    }

    w.close()?;
    drop(w);
    dir.close()
}

/// TODO: remove this hack and fix this test to be better?
/// the whole thing relies on default codec too...
///
/// The package-private `readSegmentInfoID(Directory, String)`.
fn read_segment_info_id(dir: &dyn Directory, file: &str) -> Result<Vec<u8>> {
    let mut r#in = dir.open_input(file, &*DEFAULT_IO_CONTEXT)?;
    r#in.read_int()?; // magic
    r#in.read_string()?; // codec name
    r#in.read_int()?; // version
    let mut id = vec![0u8; ID_LENGTH];
    let len = id.len();
    r#in.read_bytes(Some(&mut id), 0, len)?;
    Ok(id)
}
