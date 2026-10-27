//! The RAM the writer charges for buffered deletes and doc-values updates,
//! compared with Apache Lucene Core 10.5.0.
//!
//! `FlushByRamOrCountsPolicy.onChange` (`FlushByRamOrCountsPolicy.java:43-55`)
//! reads `DocumentsWriterFlushControl.getDeleteBytesUsed()` both to force the
//! buffered deletes to be applied and to pick a per-thread writer to flush, so
//! the estimate decides where segments end. It is the sum `BufferedUpdates`
//! keeps (`BufferedUpdates.java:175-178`): the `DeletedTerms` pool and maps,
//! `BYTES_PER_DEL_QUERY` per distinct query, and every `FieldUpdatesBuffer`.
//!
//! * `trace` (Java oracle `DeleteRamFixture trace`): term deletes, query
//!   deletes, numeric and binary doc-values updates on a writer with no
//!   buffered document, printing `IndexWriter.ramBytesUsed()` after each one.
//!   With nothing buffered it equals the deletes' bytes, which the Java side
//!   also prints from `getDeleteBytesUsed()`.
//! * `files`: adds, document updates, term deletes and query deletes under a
//!   0.0665 MB RAM buffer; the committed file names and each segment's size,
//!   deletions and deletion generation. The buffer is cut by RAM every 11 to
//!   13 documents, so where it is cut is decided by the deletes' estimate this
//!   file pins. The documents carry nothing but a numeric doc value and a
//!   per-thread writer never holds more than 16 of them (`maxBufferedDocs`),
//!   the range over which the doc-values buffer's own estimate agrees with
//!   Lucene's; indexed terms and doc-values updates are left out because
//!   their RAM is estimated by other components (the postings pools and the
//!   per-segment `DocValuesFieldUpdates`).

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use rucene::codecs::{register_codec, Lucene104Codec};
use rucene::document::{BinaryDocValuesField, Document, NumericDocValuesField, Store, StringField};
use rucene::error::Result;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::merge_policy::NoMergePolicy;
use rucene::index::merge_scheduler::SerialMergeScheduler;
use rucene::index::terms::Term;
use rucene::index::SegmentInfos;
use rucene::search::{Query, TermQuery};
use rucene::store::{Directory, RamDirectory};
use rucene::util::BytesRef;

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

/// Runs one case of `DeleteRamFixture` and returns its standard output.
fn run_java(case: &str) -> String {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join(
        "tests/fixtures/java-codec-harness/src/main/java/org/apache/lucene/rucene/codec/\
         DeleteRamFixture.java",
    );
    let output = Command::new("java")
        .arg("-cp")
        .arg(lucene_jar())
        .arg(fixture)
        .arg(case)
        .output()
        .expect("a JDK is required to run the Lucene oracle");
    assert!(
        output.status.success(),
        "DeleteRamFixture {case} failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

/// `DeleteRamFixture.doc(int)`, field for field and in the same order.
fn doc(id: i32) -> Document {
    let mut d = Document::new();
    d.add(Box::new(
        StringField::new("id", format!("d{id}"), Store::NO).unwrap(),
    ));
    d.add(Box::new(
        StringField::new("grp", format!("g{}", id % 3), Store::NO).unwrap(),
    ));
    d.add(Box::new(NumericDocValuesField::new("num", i64::from(id))));
    d.add(Box::new(BinaryDocValuesField::new(
        "bin",
        bytes(&format!("b{id}")),
    )));
    d
}

/// `DeleteRamFixture.valueDoc(int)`: a document whose only field is a numeric
/// doc value, so no term is indexed and the per-thread writer's RAM is the
/// doc-values buffer and the buffered deletes alone.
fn value_doc(id: i32) -> Document {
    let mut d = Document::new();
    d.add(Box::new(NumericDocValuesField::new("num", i64::from(id))));
    d
}

fn bytes(text: &str) -> BytesRef {
    BytesRef::new(text.as_bytes().to_vec())
}

fn term(field: &str, text: &str) -> Term {
    Term::from_text(field, text)
}

fn term_query(field: &str, text: &str) -> Arc<dyn Query> {
    Arc::new(TermQuery::new(term(field, text)))
}

type Op = Box<dyn Fn(&IndexWriter) -> Result<i64>>;

/// `DeleteRamFixture.trace()`: the deletes' RAM after every operation.
#[test]
fn buffered_deletes_ram_matches_lucene() {
    ensure_codec();
    let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
    let mut config = IndexWriterConfig::new();
    config
        .set_merge_scheduler(Arc::new(SerialMergeScheduler::new()))
        .unwrap();
    let writer = IndexWriter::new(Arc::clone(&dir), config).unwrap();
    for i in 0..4 {
        writer.add_document(&doc(i)).unwrap();
    }
    writer.commit().unwrap();
    let mut ops: Vec<(String, Op)> = Vec::new();
    for k in 0..20 {
        ops.push((
            format!("term id x{k}"),
            Box::new(move |w| w.delete_documents_by_terms(vec![term("id", &format!("x{k}"))])),
        ));
    }
    ops.push((
        "term id x3 again".to_string(),
        Box::new(|w| w.delete_documents_by_terms(vec![term("id", "x3")])),
    ));
    ops.push((
        "term grp new field".to_string(),
        Box::new(|w| w.delete_documents_by_terms(vec![term("grp", "zz")])),
    ));
    for k in 0..3 {
        ops.push((
            format!("query id q{k}"),
            Box::new(move |w| {
                w.delete_documents_by_queries(vec![term_query("id", &format!("q{k}"))])
            }),
        ));
    }
    ops.push((
        "query id q0 again".to_string(),
        Box::new(|w| w.delete_documents_by_queries(vec![term_query("id", "q0")])),
    ));
    for k in 0..3 {
        ops.push((
            format!("numeric d{k}"),
            Box::new(move |w| {
                w.update_numeric_doc_value(term("id", &format!("d{k}")), "num", 100 + k)
            }),
        ));
    }
    ops.push((
        "numeric grp g1".to_string(),
        Box::new(|w| w.update_numeric_doc_value(term("grp", "g1"), "num", 7)),
    ));
    for k in 0..3 {
        ops.push((
            format!("binary d{k}"),
            Box::new(move |w| {
                w.update_binary_doc_value(
                    term("id", &format!("d{k}")),
                    "bin",
                    bytes(&format!("v{k}")),
                )
            }),
        ));
    }
    ops.push((
        "binary grp g2 long".to_string(),
        Box::new(|w| {
            w.update_binary_doc_value(
                term("grp", "g2"),
                "bin",
                bytes("a much longer binary value"),
            )
        }),
    ));
    let mut lines = Vec::new();
    let ram = writer.ram_bytes_used().unwrap();
    lines.push(format!("start ram={ram} del={ram}"));
    for (name, op) in &ops {
        op(&writer).unwrap();
        // No document is buffered, so `ramBytesUsed()` is the deletes' bytes
        // alone, which is what the Java side prints as `del=`.
        let ram = writer.ram_bytes_used().unwrap();
        lines.push(format!("{name} ram={ram} del={ram}"));
    }
    writer.rollback().unwrap();
    assert_eq!(lines.join("\n"), run_java("trace").trim());
}

/// `DeleteRamFixture.files()`: where a 0.0665 MB RAM buffer cuts segments under
/// a mixed workload, and the files the commit holds.
#[test]
fn ram_flushes_with_buffered_deletes_cut_segments_as_lucene() {
    ensure_codec();
    let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
    let mut config = IndexWriterConfig::new();
    config
        .set_merge_scheduler(Arc::new(SerialMergeScheduler::new()))
        .unwrap();
    config
        .set_merge_policy(Arc::new(NoMergePolicy::new()))
        .unwrap();
    config.set_ram_buffer_size_mb(0.0665).unwrap();
    config.set_max_buffered_docs(16).unwrap();
    let writer = IndexWriter::new(Arc::clone(&dir), config).unwrap();
    for i in 0..400 {
        match i % 5 {
            0 | 1 => writer.add_document(&value_doc(i)).map(|_| ()),
            2 => writer
                .update_document(term("id", &format!("d{}", i - 2)), &value_doc(i))
                .map(|_| ()),
            3 => writer
                .delete_documents_by_terms(vec![term("id", &format!("d{}", i - 2))])
                .map(|_| ()),
            _ => writer
                .delete_documents_by_queries(vec![term_query("id", &format!("nomatch{i}"))])
                .and_then(|_| {
                    writer.delete_documents_by_terms(vec![term("grp", &format!("zz{i}"))])
                })
                .map(|_| ()),
        }
        .unwrap();
    }
    writer.commit().unwrap();
    let mut files: Vec<String> = dir
        .list_all()
        .unwrap()
        .into_iter()
        .filter(|f| f != "write.lock")
        .collect();
    files.sort();
    let mut lines = vec![format!("files={}", files.join(","))];
    let infos = SegmentInfos::read_latest_commit(Arc::clone(&dir)).unwrap();
    for info in infos.iter() {
        lines.push(format!(
            "{} maxDoc={} del={} delGen={}",
            info.info.name,
            info.info.max_doc().unwrap(),
            info.get_del_count(),
            info.get_del_gen()
        ));
    }
    writer.close().unwrap();
    assert_eq!(lines.join("\n"), run_java("files").trim());
}
