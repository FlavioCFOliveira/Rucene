//! A panic during a merge (rmp #351, E11 round 4): Java's
//! `IndexWriter.merge(OneMerge)` catches any `Throwable` — an assertion
//! included — hands it to `handleMergeException`, finishes the merge in its
//! `finally` and records the tragedy before rethrowing
//! (`IndexWriter.java:4844-4891`); `ConcurrentMergeScheduler.MergeThread.run`
//! catches it too (`ConcurrentMergeScheduler.java:756-765`). A panic is this
//! port's carrier of Java's unchecked throwables, so a panicking codec must
//! end the same way: `forceMerge` fails promptly instead of waiting for a
//! merge that never finished.
//!
//! Java oracle (`lucene-core-10.5.0.jar`, a `FilterCodec` whose stored-fields
//! writer throws when opened in a `MERGE` context, two one-document segments,
//! `LogDocMergePolicy`, `forceMerge(1)`):
//!
//! * `ConcurrentMergeScheduler`: one of three failures, each caused by the
//!   thrown exception, depending on how the `forceMerge` thread interleaves
//!   with the merge thread; once the merge threads finished
//!   (`ConcurrentMergeScheduler.sync()`), the writer is closed and its tragic
//!   exception is the thrown one;
//! * `SerialMergeScheduler`: the thrown exception itself, the writer closed
//!   with it as its tragedy.
//!
//! The three `ConcurrentMergeScheduler` failures are Java's own (rmp #638),
//! all observed against the jar, as the merge thread records the failure in
//! `mergeExceptions` and wakes the waiters under the writer's lock but records
//! the tragedy only after leaving it (`IndexWriter.java:4866-4891`):
//!
//! * the tragedy is set when the wait loop checks: `IllegalStateException:
//!   this writer hit an unrecoverable error; cannot complete forceMerge`
//!   (`IndexWriter.java:2246-2249`) — almost every run;
//! * the wait loop runs between `addMergeException` and `onTragicEvent`:
//!   `IOException: background merge hit exception: <segString>`
//!   (`IndexWriter.java:2252-2261`);
//! * the merge thread fails before `ConcurrentMergeScheduler.merge` loops back
//!   to `maybeStall`: `hasPendingMerges` throws `IllegalStateException: this
//!   writer hit an unrecoverable error; cannot merge`
//!   (`IndexWriter.java:2518-2523`, `ConcurrentMergeScheduler.java:567-573`).
//!
//! In each case `forceMerge` may return before the merge thread closed the
//! writer, so the test waits for the merge threads, as Lucene's own
//! `TestIndexWriterExceptions.testForceMergeExceptions` accepts either an
//! `IllegalStateException` or an `IOException` with a cause.
//!
//! Every wait is bounded, so a regression fails instead of hanging.

use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;

use rucene::codecs::stored_fields::{StoredFieldsFormat, StoredFieldsReader, StoredFieldsWriter};
use rucene::codecs::{Codec, FilterCodec, Lucene104Codec};
use rucene::document::{Document, StoredField};
use rucene::error::{LuceneError, LuceneErrorKind};
use rucene::index::field_infos::FieldInfos;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::merge_policy::LogMergePolicy;
use rucene::index::merge_scheduler::{
    ConcurrentMergeScheduler, MergeScheduler, SerialMergeScheduler,
};
use rucene::index::segment_info::SegmentInfo;
use rucene::store::java_nio_file::java_cause_of;
use rucene::store::{ByteBuffersDirectory, Context, Directory, IOContext};

/// The stored-fields format of the default codec, whose writer panics with
/// `thrown` (a Java throwable's `toString()`) when opened for a merge.
#[derive(Debug)]
struct PanickingOnMerge {
    thrown: &'static str,
}

impl StoredFieldsFormat for PanickingOnMerge {
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
            panic!("{}", self.thrown);
        }
        Lucene104Codec::new()
            .stored_fields_format()
            .fields_writer(directory, segment_info, context)
    }
}

/// What `forceMerge(1)` ended with: its error, or the text of its panic.
enum Outcome {
    Error(LuceneError),
    Panic(String),
}

/// Writes two one-document segments with the panicking codec and runs
/// `forceMerge(1)` on another thread, waiting at most a minute for it.
fn force_merge_with(
    scheduler: Arc<dyn MergeScheduler>,
    thrown: &'static str,
) -> (Outcome, Arc<IndexWriter>) {
    let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
    let codec: Arc<dyn Codec> = Arc::new(
        FilterCodec::new("Lucene104", Arc::new(Lucene104Codec::new()))
            .with_stored_fields_format(PanickingOnMerge { thrown }),
    );
    let config = IndexWriterConfig::new();
    config.set_codec(codec).unwrap();
    config
        .set_merge_policy(Some(Arc::new(LogMergePolicy::by_doc_count())))
        .unwrap();
    config.set_merge_scheduler(Some(scheduler)).unwrap();
    let writer = IndexWriter::new(dir, config).unwrap();
    for i in 0..2 {
        let mut doc = Document::new();
        doc.add(StoredField::new_string("f", format!("v{i}")).unwrap());
        writer.add_document(&doc).unwrap();
        writer.commit().unwrap();
    }
    let (tx, rx) = mpsc::channel();
    let merging = Arc::clone(&writer);
    std::thread::spawn(move || {
        let outcome =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| merging.force_merge(1)));
        let _ = tx.send(match outcome {
            Ok(Ok(())) => panic!("forceMerge succeeded"),
            Ok(Err(error)) => Outcome::Error(error),
            Err(payload) => Outcome::Panic(
                payload
                    .downcast_ref::<String>()
                    .cloned()
                    .unwrap_or_default(),
            ),
        });
    });
    let outcome = rx
        .recv_timeout(Duration::from_secs(60))
        .expect("forceMerge did not end within a minute: the failed merge was never finished");
    (outcome, writer)
}

/// `error` rendered as a Java throwable's `toString()`, `class: message`.
fn throwable(error: &LuceneError) -> String {
    match error {
        LuceneError {
            kind: LuceneErrorKind::AssertionError(message),
            ..
        } => format!("java.lang.AssertionError: {message}"),
        other => other.to_string(),
    }
}

/// The tragedy the writer recorded, rendered as `class: message`.
fn tragedy(writer: &IndexWriter) -> String {
    throwable(&writer.get_tragic_exception().expect("a tragic exception"))
}

#[test]
fn a_panicking_merge_fails_force_merge_under_concurrent_merge_scheduler() {
    for thrown in [
        "java.lang.IllegalStateException: boom",
        "java.lang.AssertionError: boom",
    ] {
        let scheduler = Arc::new(ConcurrentMergeScheduler::new());
        let (outcome, writer) = force_merge_with(Arc::clone(&scheduler) as _, thrown);
        let Outcome::Error(error) = outcome else {
            panic!("forceMerge panicked instead of failing");
        };
        // Java's three outcomes, by interleaving (see the module docs); each
        // is caused by the thrown exception.
        let cause = match &error {
            LuceneError {
                kind: LuceneErrorKind::IllegalState { message, cause },
                ..
            } => {
                assert!(
                    [
                        "this writer hit an unrecoverable error; cannot complete forceMerge",
                        "this writer hit an unrecoverable error; cannot merge",
                    ]
                    .contains(&message.as_str()),
                    "unexpected IllegalStateException message: {message}"
                );
                throwable(cause.as_ref().expect("the tragedy as the cause"))
            }
            LuceneError {
                kind: LuceneErrorKind::Io(io),
                ..
            } => {
                assert_eq!(
                    io.to_string(),
                    "background merge hit exception: _0(10.5.0):c1 _1(10.5.0):c1 into _2 \
                     [maxNumSegments=1]"
                );
                throwable(java_cause_of(io).expect("the merge's exception as the cause"))
            }
            other => panic!("expected IllegalStateException or IOException, got {other:?}"),
        };
        assert_eq!(cause, thrown);
        // `forceMerge` may return before the merge thread recorded the
        // tragedy and closed the writer: wait for it, as
        // `ConcurrentMergeScheduler.sync()` does, at most a minute.
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            scheduler.sync();
            let _ = tx.send(());
        });
        rx.recv_timeout(Duration::from_secs(60))
            .expect("the merge thread did not finish within a minute");
        assert!(!writer.is_open());
        assert_eq!(tragedy(&writer), thrown);
    }
}

#[test]
fn a_panicking_merge_rethrows_out_of_force_merge_under_serial_merge_scheduler() {
    for thrown in [
        "java.lang.IllegalStateException: boom",
        "java.lang.AssertionError: boom",
    ] {
        let (outcome, writer) = force_merge_with(Arc::new(SerialMergeScheduler::new()), thrown);
        let Outcome::Panic(text) = outcome else {
            panic!("forceMerge returned an error instead of rethrowing the panic");
        };
        assert_eq!(text, thrown);
        assert!(!writer.is_open());
        assert_eq!(tragedy(&writer), thrown);
    }
}
