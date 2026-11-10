//! A throwable that escapes a thread `rucene` started reaches the suite's
//! uncaught-exception handling, as the JVM hands it to the thread's group
//! (`ThreadGroup.uncaughtException`, which randomizedtesting's
//! `RunnerThreadGroup` records and turns into a failure,
//! `ThreadLeakControl.processUncaught`).
//!
//! Java oracle (`lucene-core-10.5.0.jar`, a `FilterCodec` whose stored-fields
//! writer throws `IllegalStateException("boom")` when opened in a `MERGE`
//! context, under `ConcurrentMergeScheduler`): `forceMerge(1)` throws
//! `IllegalStateException: this writer hit an unrecoverable error; cannot
//! complete forceMerge`, and the merge thread `Lucene Merge Thread #0` dies
//! with `org.apache.lucene.index.MergePolicy$MergeException:
//! java.lang.IllegalStateException: boom`, caused by the thrown exception.
//! The suite below expects the first and must fail with the second as an
//! uncaught exception (`Captured an uncaught exception in thread: ...`).
//! Every wait is bounded, so a regression fails instead of hanging.

use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::reflect::{Class, Constructor, Method};
use randomizedtesting::jdk::thread::Thread;
use randomizedtesting::junit::{Failure, JUnitCore, Request, RunListener};
use rucene::codecs::stored_fields::{StoredFieldsFormat, StoredFieldsReader, StoredFieldsWriter};
use rucene::codecs::{Codec, FilterCodec, Lucene104Codec};
use rucene::document::{Document, StoredField};
use rucene::error::LuceneError;
use rucene::index::field_infos::FieldInfos;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::merge_policy::LogMergePolicy;
use rucene::index::merge_scheduler::ConcurrentMergeScheduler;
use rucene::index::segment_info::SegmentInfo;
use rucene::store::{ByteBuffersDirectory, Context, Directory, IOContext};
use rucene_test_framework::util::LuceneTestCase;

/// The stored-fields format of the default codec, whose writer throws
/// `IllegalStateException("boom")` when opened for a merge.
#[derive(Debug)]
struct ThrowingOnMerge;

impl StoredFieldsFormat for ThrowingOnMerge {
    fn name(&self) -> &str {
        "Lucene90StoredFieldsFormat"
    }

    fn fields_reader(
        &self,
        directory: &dyn Directory,
        segment_info: &SegmentInfo,
        field_infos: &FieldInfos,
        context: &dyn IOContext,
    ) -> rucene::error::Result<Box<dyn StoredFieldsReader>> {
        Lucene104Codec::new().stored_fields_format().fields_reader(
            directory,
            segment_info,
            field_infos,
            context,
        )
    }

    fn fields_writer(
        &self,
        directory: &Arc<dyn Directory>,
        segment_info: &SegmentInfo,
        context: &dyn IOContext,
    ) -> rucene::error::Result<Box<dyn StoredFieldsWriter>> {
        if context.context() == Context::Merge {
            Throwable::new("java.lang.IllegalStateException", "boom").throw();
        }
        Lucene104Codec::new()
            .stored_fields_format()
            .fields_writer(directory, segment_info, context)
    }
}

/// The test: two one-document segments, then `forceMerge(1)` under
/// `ConcurrentMergeScheduler`, which must fail with Java's
/// `IllegalStateException` within a minute.
fn test_merge_thread_throws() {
    let codec: Arc<dyn Codec> = Arc::new(
        FilterCodec::new("Lucene104", Arc::new(Lucene104Codec::new()))
            .with_stored_fields_format(ThrowingOnMerge),
    );
    let mut config = IndexWriterConfig::new();
    config.set_codec(codec).unwrap();
    config
        .set_merge_policy(Some(Arc::new(LogMergePolicy::by_doc_count())))
        .unwrap();
    config
        .set_merge_scheduler(Some(Arc::new(ConcurrentMergeScheduler::new())))
        .unwrap();
    let writer = IndexWriter::new(Arc::new(ByteBuffersDirectory::new()), config).unwrap();
    for i in 0..2 {
        let mut doc = Document::new();
        doc.add(Box::new(
            StoredField::new_string("f", format!("v{i}")).unwrap(),
        ));
        writer.add_document(&doc).unwrap();
        writer.commit().unwrap();
    }
    let (tx, rx) = mpsc::channel();
    let merging = Arc::clone(&writer);
    // A thread of the test's group, as a Java test's own threads are, so
    // that the merge thread it starts belongs to that group too.
    let worker = Thread::unnamed(move || {
        let _ = tx.send(merging.force_merge(1));
    });
    worker.start();
    let outcome = rx
        .recv_timeout(Duration::from_secs(60))
        .expect("forceMerge did not end within a minute");
    let _ = worker.join();
    // The merge thread rethrows after it finished the merge; wait for it to
    // end so that its uncaught exception is captured during the test.
    writer.close().ok();
    match outcome {
        Err(LuceneError::IllegalState { message, .. }) => assert_eq!(
            message,
            "this writer hit an unrecoverable error; cannot complete forceMerge"
        ),
        other => panic!("expected IllegalStateException, got {other:?}"),
    }
}

fn class() -> Arc<Class> {
    Class::builder("org.apache.lucene.tests.util.TestCoreThreadUncaught")
        .extends(&LuceneTestCase::class())
        .constructor(Constructor::no_args(|| ()))
        .method(Method::new("testMergeThreadThrows").body_any(test_merge_thread_throws))
        .build()
}

/// One failure: the exception's class, message and cause (class, message).
type FailureRow = (String, String, Option<(String, String)>);

/// The failures of a run.
struct Failures(Mutex<Vec<FailureRow>>);

impl RunListener for Failures {
    fn test_failure(&self, f: &Failure) {
        let t = f.exception();
        self.0.lock().unwrap().push((
            t.class_name().to_string(),
            t.message().unwrap_or("null").to_string(),
            t.cause().map(|c| {
                (
                    c.class_name().to_string(),
                    c.message().unwrap_or("null").to_string(),
                )
            }),
        ));
    }
}

#[test]
fn a_merge_thread_throwable_fails_the_suite_as_uncaught() {
    let failures = Arc::new(Failures(Mutex::new(Vec::new())));
    let mut core = JUnitCore::new();
    core.add_listener(Arc::clone(&failures) as Arc<dyn RunListener>);
    let result = core.run(&Request::a_class(&class()));
    let failures = failures.0.lock().unwrap();
    println!("run={} failures={:?}", result.run_count(), *failures);
    assert_eq!(failures.len(), 1, "{failures:?}");
    let (class, message, cause) = &failures[0];
    assert_eq!(
        class,
        "com.carrotsearch.randomizedtesting.UncaughtExceptionError"
    );
    assert!(
        message.starts_with("Captured an uncaught exception in thread: Thread[id=")
            && message.contains("name=Lucene Merge Thread #0"),
        "{message}"
    );
    assert_eq!(
        cause.as_ref(),
        Some(&(
            "org.apache.lucene.index.MergePolicy$MergeException".to_string(),
            "java.lang.IllegalStateException: boom".to_string()
        ))
    );
}
