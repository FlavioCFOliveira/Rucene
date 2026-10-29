//! The RAM the writer charges while indexing, and where that RAM cuts
//! segments, compared with Apache Lucene Core 10.5.0.
//!
//! `FlushByRamOrCountsPolicy.onChange` (`FlushByRamOrCountsPolicy.java:43-55`)
//! compares the per-thread writers' RAM with the RAM buffer, and
//! `IndexWriter` writes buffered doc-values updates once the reader pool's
//! RAM exceeds it (`IndexWriter.java:1886-1895`), so these estimates decide
//! segment names and doc-values generations.
//!
//! * `trace` (Java oracle `IndexingRamFixture trace <kind> <docs>`): one kind
//!   of document at a time, `IndexWriter.ramBytesUsed()` after each. A
//!   document limit that is never reached makes
//!   `DocumentsWriterFlushControl.doAfterDocument` commit the per-thread RAM
//!   after every document (`DocumentsWriterFlushControl.java:205-213`), so the
//!   figure is the per-thread writer's `ramBytesUsed()`
//!   (`DocumentsWriterPerThread.java:722-728`): the postings' int and byte
//!   blocks and postings arrays (`TermsHashPerField.java`, `IndexingChain.java:2112-2135`),
//!   the stored fields and term vectors, and each doc-values writer.
//! * `dv-updates` (same oracle): `DocValuesFieldUpdates.ramBytesUsed()` of the
//!   numeric, min/max numeric and binary subclasses after each update and
//!   after `finish()` (`DocValuesFieldUpdates.java:416-423`,
//!   `NumericDocValuesFieldUpdates.java:129-134`,
//!   `BinaryDocValuesFieldUpdates.java:135-143`).
//! * `flushing` (same oracle): `IndexWriter.ramBytesUsed()` and
//!   `getFlushingBytes()` read while each per-thread writer flushes; the
//!   flushing bytes are part of the writer's RAM, which is
//!   `getDeleteBytesUsed() + netBytes()` (`DocumentsWriterFlushControl.java:100-102, 499-502`).
//! * `files` (same oracle): mixed documents, term deletes and numeric and
//!   binary doc-values updates under small RAM buffers, over a grid of RAM
//!   buffer sizes, operation counts and seeds; the committed file names and
//!   every segment's size, deletions and generations. Some points of such a
//!   grid have no single Lucene answer: `writeSomeDocValuesUpdates` writes the
//!   readers of equal RAM in the order of a map keyed by identity hash codes
//!   (`ReaderPool.java:47, 280-306`), and with `RAM 0.35 MB, 2000 ops, seed 9`
//!   or `0.3 MB, 2500, 17` the oracle's own doc-values generations change with
//!   `PERTURB`, which only shifts those hash codes. The test asserts that every
//!   point it uses is unchanged by `PERTURB` before comparing.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex, OnceLock, Weak};

use rucene::codecs::{register_codec, Lucene104Codec};
use rucene::document::{
    BinaryDocValuesField, Document, Field, FieldType, NumericDocValuesField, NumericValue,
    SortedDocValuesField, SortedNumericDocValuesField, SortedSetDocValuesField, Store, StoredField,
    StringField, TextField,
};
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::merge_policy::NoMergePolicy;
use rucene::index::merge_scheduler::SerialMergeScheduler;
use rucene::index::terms::Term;
use rucene::index::{DocValuesFieldUpdates, DocValuesType, IndexOptions, SegmentInfos};
use rucene::store::{Directory, RamDirectory};
use rucene::util::{Accountable, BytesRef, InfoStream};

fn ensure_codec() {
    let _ = register_codec("Lucene104", Lucene104Codec::new());
}

fn lucene_jar() -> PathBuf {
    let home = std::env::var("HOME").expect("HOME is set");
    let jar = Path::new(&home)
        .join(".m2/repository/org/apache/lucene/lucene-core/10.5.0/lucene-core-10.5.0.jar");
    assert!(
        jar.is_file(),
        "the Lucene 10.5.0 jar is required: {}",
        jar.display()
    );
    jar
}

/// Runs `IndexingRamFixture` with `args` and returns its standard output.
fn run_java(args: &str) -> String {
    run_java_with_env(args, "RUCENE_UNUSED", "")
}

/// [`run_java`] with the environment variable `key` set to `value`.
fn run_java_with_env(args: &str, key: &str, value: &str) -> String {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join(
        "tests/fixtures/java-codec-harness/src/main/java/org/apache/lucene/rucene/codec/\
         IndexingRamFixture.java",
    );
    let output = Command::new("java")
        .env(key, value)
        .arg("-cp")
        .arg(lucene_jar())
        .arg(fixture)
        .args(args.split(' '))
        .output()
        .expect("a JDK is required to run the Lucene oracle");
    assert!(
        output.status.success(),
        "IndexingRamFixture {args} failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

fn bytes(text: &str) -> BytesRef {
    BytesRef::new(text.as_bytes().to_vec())
}

/// `IndexingRamFixture.TV_TYPE`: `TextField.TYPE_NOT_STORED` with term vectors,
/// positions and offsets.
fn tv_type() -> FieldType {
    let mut field_type = FieldType::new();
    field_type.set_tokenized(true).unwrap();
    field_type
        .set_index_options(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS)
        .unwrap();
    field_type.set_store_term_vectors(true).unwrap();
    field_type.set_store_term_vector_positions(true).unwrap();
    field_type.set_store_term_vector_offsets(true).unwrap();
    field_type.freeze();
    field_type
}

/// `IndexingRamFixture.DOCS_FREQS_TYPE`: `TextField.TYPE_NOT_STORED` indexed
/// with `DOCS_AND_FREQS`.
fn docs_freqs_type() -> FieldType {
    let mut field_type = FieldType::new();
    field_type.set_tokenized(true).unwrap();
    field_type
        .set_index_options(IndexOptions::DOCS_AND_FREQS)
        .unwrap();
    field_type.freeze();
    field_type
}

/// `IndexingRamFixture.text(int)`.
fn text(id: i32) -> String {
    let words = id % 7 + 1;
    (0..words)
        .map(|j| format!("w{}", (id * 31 + j * 17) % 50))
        .collect::<Vec<_>>()
        .join(" ")
}

/// `IndexingRamFixture.doc(String, int)`, field for field and in the same
/// order.
fn doc(kind: &str, id: i32) -> Document {
    let mut d = Document::new();
    match kind {
        "string" => d.add(Box::new(
            StringField::new("id", format!("d{id}"), Store::NO).unwrap(),
        )),
        "text" => d.add(Box::new(
            TextField::new("body", text(id), Store::NO).unwrap(),
        )),
        "freqs" => d.add(Box::new(
            Field::new("freqs", text(id), docs_freqs_type()).unwrap(),
        )),
        "tv" => d.add(Box::new(Field::new("tv", text(id), tv_type()).unwrap())),
        "stored" => {
            d.add(Box::new(
                StoredField::new_string("s", format!("v{id}")).unwrap(),
            ));
            d.add(Box::new(
                StoredField::new_number("n", NumericValue::Int(id)).unwrap(),
            ));
        }
        "numeric" => d.add(Box::new(NumericDocValuesField::new(
            "num",
            i64::from(id) * 1000,
        ))),
        "binary" => d.add(Box::new(BinaryDocValuesField::new(
            "bin",
            bytes(&format!("b{id}")),
        ))),
        "sorted" => d.add(Box::new(SortedDocValuesField::new(
            "sorted",
            bytes(&format!("s{}", id % 13)),
        ))),
        "sortedset" => {
            for j in 0..=id % 3 {
                d.add(Box::new(SortedSetDocValuesField::new(
                    "sset",
                    bytes(&format!("t{}", (id + j) % 11)),
                )));
            }
        }
        "sortednumeric" => {
            for j in 0..=id % 3 {
                d.add(Box::new(SortedNumericDocValuesField::new(
                    "snum",
                    i64::from(id) * i64::from(j),
                )));
            }
        }
        "mixed" => {
            d.add(Box::new(
                StringField::new("id", format!("d{id}"), Store::YES).unwrap(),
            ));
            d.add(Box::new(
                StringField::new("grp", format!("g{}", id % 5), Store::NO).unwrap(),
            ));
            d.add(Box::new(
                TextField::new("body", text(id), Store::NO).unwrap(),
            ));
            d.add(Box::new(NumericDocValuesField::new("num", i64::from(id))));
            d.add(Box::new(BinaryDocValuesField::new(
                "bin",
                bytes(&format!("b{id}")),
            )));
            d.add(Box::new(SortedDocValuesField::new(
                "sorted",
                bytes(&format!("s{}", id % 13)),
            )));
        }
        other => panic!("unknown kind {other}"),
    }
    d
}

/// `IndexingRamFixture.trace(String, int)`.
fn trace(kind: &str, docs: i32) -> String {
    let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
    let mut config = IndexWriterConfig::new();
    config
        .set_merge_scheduler(Arc::new(SerialMergeScheduler::new()))
        .unwrap();
    config.set_ram_buffer_size_mb(256.0).unwrap();
    config.set_max_buffered_docs(1 << 30).unwrap();
    let writer = IndexWriter::new(dir, config).unwrap();
    let mut lines = vec![format!("start ram={}", writer.ram_bytes_used().unwrap())];
    for i in 0..docs {
        writer.add_document(&doc(kind, i)).unwrap();
        lines.push(format!("{i} ram={}", writer.ram_bytes_used().unwrap()));
    }
    writer.rollback().unwrap();
    lines.join("\n")
}

/// The first line on which `actual` and `expected` differ, for a readable
/// failure on long traces.
fn first_difference(actual: &str, expected: &str) -> Option<String> {
    let mut a = actual.lines();
    let mut e = expected.lines();
    loop {
        match (a.next(), e.next()) {
            (None, None) => return None,
            (x, y) if x == y => continue,
            (x, y) => return Some(format!("rucene: {x:?}\nlucene: {y:?}")),
        }
    }
}

/// Records in `failures` where `actual` first departs from `expected`, so one
/// run reports every case of the grid.
fn check(failures: &mut Vec<String>, actual: &str, expected: &str, what: &str) {
    if let Some(difference) = first_difference(actual, expected.trim()) {
        failures.push(format!("{what}: first difference\n{difference}"));
    }
}

fn assert_no_failures(failures: &[String]) {
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

/// Every document kind, 300 documents each: the per-thread writer's RAM after
/// every document is Lucene's, byte for byte.
#[test]
fn per_document_ram_is_lucenes() {
    ensure_codec();
    let mut failures = Vec::new();
    for kind in [
        "string",
        "text",
        "freqs",
        "tv",
        "stored",
        "numeric",
        "binary",
        "sorted",
        "sortedset",
        "sortednumeric",
        "mixed",
    ] {
        check(
            &mut failures,
            &trace(kind, 300),
            &run_java(&format!("trace {kind} 300")),
            kind,
        );
    }
    assert_no_failures(&failures);
}

/// `IndexingRamFixture.dvUpdates(String, int, int, int)`.
fn dv_updates(kind: &str, max_doc: i32, count: i32, stride: i32) -> String {
    let mut updates = match kind {
        "numeric" => DocValuesFieldUpdates::new(max_doc, 7, "f", DocValuesType::NUMERIC),
        "numeric-range" => DocValuesFieldUpdates::new_numeric_with_range(
            7,
            "f",
            5,
            5 + i64::from(count) * 977,
            max_doc,
        ),
        "binary" => DocValuesFieldUpdates::new(max_doc, 7, "f", DocValuesType::BINARY),
        other => panic!("unknown kind {other}"),
    };
    let mut lines = vec![format!("start ram={}", updates.ram_bytes_used())];
    for i in 0..count {
        let doc = (i64::from(i) * i64::from(stride) % i64::from(max_doc)) as i32;
        if kind == "binary" {
            updates.add_binary(doc, &bytes(&format!("value{i}")));
        } else {
            updates.add_long(doc, 5 + i64::from(i) * 977);
        }
        lines.push(format!("{i} ram={}", updates.ram_bytes_used()));
    }
    updates.finish().unwrap();
    lines.push(format!("finish ram={}", updates.ram_bytes_used()));
    lines.join("\n")
}

/// The three `DocValuesFieldUpdates` subclasses, over segment sizes, update
/// counts that cross a 1024-entry page and strides that repeat documents.
#[test]
fn doc_values_field_updates_ram_is_lucenes() {
    let mut failures = Vec::new();
    for kind in ["numeric", "numeric-range", "binary"] {
        for (max_doc, count, stride) in [(1, 1, 1), (100, 3, 7), (5000, 1500, 13), (70000, 40, 997)]
        {
            let args = format!("{kind} {max_doc} {count} {stride}");
            check(
                &mut failures,
                &dv_updates(kind, max_doc, count, stride),
                &run_java(&format!("dv-updates {args}")),
                &args,
            );
        }
    }
    assert_no_failures(&failures);
}

/// `IndexingRamFixture.files(double, int, long)`; with `trace`, the line the
/// fixture prints after every operation when `TRACE` is set comes first.
fn files(ram_mb: f64, ops: i32, seed: i64, trace: bool) -> String {
    let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
    let mut config = IndexWriterConfig::new();
    config
        .set_merge_scheduler(Arc::new(SerialMergeScheduler::new()))
        .unwrap();
    config
        .set_merge_policy(Arc::new(NoMergePolicy::new()))
        .unwrap();
    config.set_ram_buffer_size_mb(ram_mb).unwrap();
    let writer = IndexWriter::new(Arc::clone(&dir), config).unwrap();
    let mut lines = Vec::new();
    let mut x = seed;
    let mut next = 0i32;
    for op in 0..ops {
        x = x
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        let r = ((x as u64) >> 33) % 100;
        let target = if next == 0 {
            0
        } else {
            (((x as u64) >> 17) % next as u64) as i32
        };
        let what;
        if r < 55 || next == 0 {
            writer.add_document(&doc("mixed", next)).unwrap();
            what = format!("add {next}");
            next += 1;
        } else if r < 65 {
            writer
                .update_document(
                    Term::from_text("id", &format!("d{target}")),
                    &doc("mixed", next),
                )
                .unwrap();
            what = format!("update d{target} {next}");
            next += 1;
        } else if r < 72 {
            writer
                .delete_documents_by_terms(vec![Term::from_text("id", &format!("d{target}"))])
                .unwrap();
            what = format!("delete d{target}");
        } else if r < 84 {
            writer
                .update_numeric_doc_value(
                    Term::from_text("id", &format!("d{target}")),
                    "num",
                    i64::from(op) * 31,
                )
                .unwrap();
            what = format!("numeric d{target}");
        } else if r < 94 {
            writer
                .update_binary_doc_value(
                    Term::from_text("id", &format!("d{target}")),
                    "bin",
                    bytes(&format!("u{op}")),
                )
                .unwrap();
            what = format!("binary d{target}");
        } else {
            writer
                .update_numeric_doc_value(
                    Term::from_text("grp", &format!("g{}", target % 5)),
                    "num",
                    i64::from(op),
                )
                .unwrap();
            what = format!("numeric g{}", target % 5);
        }
        if trace {
            lines.push(format!(
                "op {op} {what} ram={}",
                writer.ram_bytes_used().unwrap()
            ));
        }
    }
    writer.commit().unwrap();
    let mut names: Vec<String> = dir
        .list_all()
        .unwrap()
        .into_iter()
        .filter(|name| name != "write.lock")
        .collect();
    names.sort();
    lines.push(format!("files={}", names.join(",")));
    let infos = SegmentInfos::read_latest_commit(Arc::clone(&dir)).unwrap();
    for info in infos.iter() {
        lines.push(format!(
            "{} maxDoc={} del={} delGen={} fieldInfosGen={} dvGen={}",
            info.info.name,
            info.info.max_doc().unwrap(),
            info.get_del_count(),
            info.get_del_gen(),
            info.get_field_infos_gen(),
            info.get_doc_values_gen()
        ));
    }
    writer.close().unwrap();
    lines.join("\n")
}

/// Mixed indexing, deletes and doc-values updates under small RAM buffers:
/// the RAM after every operation, and the segments Lucene cuts, with their
/// names, sizes, deletions and generations.
#[test]
fn ram_flushes_cut_lucenes_segments() {
    ensure_codec();
    let mut failures = Vec::new();
    for (ram_mb, ops, seed) in [
        (0.25, 800, 3),
        (0.28, 1200, 77),
        (0.3, 1500, 5),
        (0.4, 3000, 3),
        (0.45, 3500, 23),
        (0.5, 4000, 1),
        (0.5, 6000, 13),
        (0.6, 5000, 31),
        (1.0, 8000, 42),
    ] {
        let args = format!("{ram_mb} {ops} {seed}");
        let expected = run_java(&format!("files {args}"));
        // The case must not depend on the JVM's identity hash codes (see
        // `ReaderPool::get_readers_by_ram`): shifting them leaves Lucene's
        // output unchanged.
        assert_eq!(
            expected,
            run_java_with_env(&format!("files {args}"), "PERTURB", "501"),
            "{args}: Lucene's own output depends on identity hash codes"
        );
        check(
            &mut failures,
            &files(ram_mb, ops, seed, false),
            &expected,
            &args,
        );
        // `IndexWriter.ramBytesUsed()` after every operation, byte for byte.
        check(
            &mut failures,
            &files(ram_mb, ops, seed, true),
            &run_java_with_env(&format!("files {args}"), "TRACE", "1"),
            &format!("{args} trace"),
        );
    }
    assert_no_failures(&failures);
}

/// `IndexingRamFixture.flushing`'s `InfoStream`: on the message
/// `DocumentsWriterPerThread.flush` logs first, records the writer's RAM and
/// flushing bytes.
#[derive(Default)]
struct FlushProbe {
    writer: OnceLock<Weak<IndexWriter>>,
    lines: Mutex<Vec<String>>,
}

impl InfoStream for FlushProbe {
    fn java_class_name(&self) -> &'static str {
        "FlushProbe"
    }

    fn message(&self, component: &str, message: &str) {
        if component != "DWPT" || !message.starts_with("flush postings as segment") {
            return;
        }
        let writer = self
            .writer
            .get()
            .and_then(Weak::upgrade)
            .expect("the writer is set before the first flush");
        let line = format!(
            "{message} ram={} flushing={}",
            writer.ram_bytes_used().unwrap(),
            writer.get_flushing_bytes().unwrap()
        );
        self.lines.lock().unwrap().push(line);
    }

    fn is_enabled(&self, component: &str) -> bool {
        component == "DWPT"
    }

    fn close(&self) {}
}

/// `IndexingRamFixture.flushing(int, int)`.
fn flushing(docs: i32, max_buffered_docs: i32) -> String {
    let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
    let probe = Arc::new(FlushProbe::default());
    let mut config = IndexWriterConfig::new();
    config
        .set_merge_scheduler(Arc::new(SerialMergeScheduler::new()))
        .unwrap();
    config
        .set_merge_policy(Arc::new(NoMergePolicy::new()))
        .unwrap();
    config.set_max_buffered_docs(max_buffered_docs).unwrap();
    config
        .set_info_stream(Arc::clone(&probe) as Arc<dyn InfoStream>)
        .unwrap();
    let writer = IndexWriter::new(dir, config).unwrap();
    assert!(probe.writer.set(Arc::downgrade(&writer)).is_ok());
    for i in 0..docs {
        writer.add_document(&doc("mixed", i)).unwrap();
        if i % 5 == 4 {
            writer
                .delete_documents_by_terms(vec![Term::from_text("id", &format!("d{}", i - 2))])
                .unwrap();
        }
    }
    writer.commit().unwrap();
    writer.close().unwrap();
    let lines = probe.lines.lock().unwrap();
    lines.join("\n")
}

/// The bytes of a per-thread writer that is flushing still count towards the
/// writer's RAM, as Lucene's `netBytes()` counts them.
#[test]
fn flushing_bytes_are_part_of_the_writers_ram() {
    ensure_codec();
    let mut failures = Vec::new();
    for (docs, max_buffered_docs) in [(40, 7), (61, 13), (25, 2)] {
        let args = format!("{docs} {max_buffered_docs}");
        check(
            &mut failures,
            &flushing(docs, max_buffered_docs),
            &run_java(&format!("flushing {args}")),
            &args,
        );
    }
    assert_no_failures(&failures);
}
