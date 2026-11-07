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
//! * `ConcurrentMergeScheduler`: `IllegalStateException: this writer hit an
//!   unrecoverable error; cannot complete forceMerge`, caused by the thrown
//!   exception; the writer is closed and its tragic exception is it;
//! * `SerialMergeScheduler`: the thrown exception itself, the writer closed
//!   with it as its tragedy.
//!
//! Every wait is bounded, so a regression fails instead of hanging.

use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;

use rucene::codecs::stored_fields::{StoredFieldsFormat, StoredFieldsReader, StoredFieldsWriter};
use rucene::codecs::{Codec, FilterCodec, Lucene104Codec};
use rucene::document::{Document, StoredField};
use rucene::error::LuceneError;
use rucene::index::field_infos::FieldInfos;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::merge_policy::LogMergePolicy;
use rucene::index::merge_scheduler::{
    ConcurrentMergeScheduler, MergeScheduler, SerialMergeScheduler,
};
use rucene::index::segment_info::SegmentInfo;
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
    let mut config = IndexWriterConfig::new();
    config.set_codec(codec).unwrap();
    config
        .set_merge_policy(Arc::new(LogMergePolicy::by_doc_count()))
        .unwrap();
    config.set_merge_scheduler(scheduler).unwrap();
    let writer = IndexWriter::new(dir, config).unwrap();
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

/// The tragedy the writer recorded, rendered as `class: message`.
fn tragedy(writer: &IndexWriter) -> String {
    let tragedy = writer.get_tragic_exception().expect("a tragic exception");
    match &*tragedy {
        LuceneError::AssertionError(message) => format!("java.lang.AssertionError: {message}"),
        other => other.to_string(),
    }
}

#[test]
fn a_panicking_merge_fails_force_merge_under_concurrent_merge_scheduler() {
    for thrown in [
        "java.lang.IllegalStateException: boom",
        "java.lang.AssertionError: boom",
    ] {
        let (outcome, writer) = force_merge_with(Arc::new(ConcurrentMergeScheduler::new()), thrown);
        let Outcome::Error(error) = outcome else {
            panic!("forceMerge panicked instead of failing");
        };
        match &error {
            LuceneError::IllegalState { message, cause } => {
                assert_eq!(
                    message,
                    "this writer hit an unrecoverable error; cannot complete forceMerge"
                );
                let cause = cause.as_ref().expect("the tragedy as the cause");
                let cause = match &**cause {
                    LuceneError::AssertionError(m) => format!("java.lang.AssertionError: {m}"),
                    other => other.to_string(),
                };
                assert_eq!(cause, thrown);
            }
            other => panic!("expected IllegalStateException, got {other:?}"),
        }
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
