//! Terms statistics, the multi-block terms index, `addIndexes` and the
//! `CheckIndex` report against Apache Lucene Core 10.5.0 (rmp task #406).
//!
//! The corpus is defined by the same formulas here and in
//! `TermsDictFixture.java`: 3000 documents whose `id`, `kw`, `body` and `tag`
//! fields hold thousands of distinct terms with shared prefixes (three-letter
//! base-26 stems with suffixes, `pre`-prefixed keywords, a non-ASCII keyword,
//! repeated body words), so that the block tree writes many blocks, floor
//! blocks and a multi-level trie. Under each root, `terms` holds that corpus as
//! a single segment (one commit, no compound file, a RAM buffer large enough
//! that only the commit flushes); `src1` (two commits, one deletion) and
//! `src2` (one commit, one deletion) are the sources of `dirs`
//! (`IndexWriter.addIndexes(Directory...)`) and `readers`
//! (`IndexWriter.addIndexes(CodecReader...)` over the source leaves). Rucene
//! writes every index under `R`, Lucene under `L`.
//!
//! * (a) Terms report (`TermsDictFixture.query`, [`query_report`]): per leaf
//!   and field of `terms`, `Terms.size/getSumDocFreq/getSumTotalTermFreq/
//!   getDocCount/getMin/getMax`, every term with `docFreq` and
//!   `totalTermFreq`, and the `seekCeil` status and landing term of every
//!   probe (one enum per field, so the probes also seek backwards).
//! * (c) The same report covers `dirs` and `readers`: per-leaf statistics and
//!   the hits of a term-query set, from both sides on the indexes both sides
//!   merged. The four reports are compared line by line.
//! * (b) Terms-index bytes: `.tim`, `.tip` and `.tmd` of the two `terms`
//!   segments are compared byte for byte after [`masked`] zeroes what is
//!   random per segment: every occurrence of the 16-byte segment id (each
//!   `CodecUtil.writeIndexHeader` embeds it: once in `.tim` and `.tip`, twice
//!   in `.tmd`, whose second header is the postings writer's) and the CRC32
//!   footer that covers it. `indexing_chain.rs` neutralises the id by giving
//!   both segments the same one; an `IndexWriter` draws it at random, so here
//!   it is masked instead.
//! * (c)/(d) `CheckIndex` (`TermsDictFixture.check`, [`check_report`]) at
//!   `Level.MAX_VALUE`, one thread, verbose, on `terms`, `dirs` and `readers`
//!   from both sides: every index must be clean — so each side accepts the
//!   indexes the other side merged — and the full text Lucene prints is
//!   compared line by line after [`normalize_check_line`] replaces only the
//!   non-deterministic parts: the `took`/`Took` timings, the segment and commit
//!   ids, the segment size in MB (the `.si` file carries diagnostics whose
//!   values — OS, JVM, timestamp — legitimately differ between the two
//!   writers) and the diagnostics map itself.
//! * Positive control: Rucene writes a perturbed `terms` corpus (document 0
//!   gains the keyword `aaaa`); its report must differ from Lucene's on `L`
//!   with the same document counts, and its masked `.tim` must differ from
//!   `L`'s.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};

use rucene::codecs::{register_codec, Lucene104Codec};
use rucene::document::{Document, Field, FieldType, Store, StringField};
use rucene::index::check_index::{CheckIndex, Level};
use rucene::index::codec_reader::CodecReader;
use rucene::index::directory_reader;
use rucene::index::filter_directory_reader::SlowCodecReaderWrapper;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::{IndexWriterConfig, OpenMode};
use rucene::index::merge_scheduler::SerialMergeScheduler;
use rucene::index::{IndexOptions, IndexReader, SeekStatus, StoredFields, Term, Terms};
use rucene::search::{IndexSearcher, TermQuery, TotalHits, TotalHitsRelation};
use rucene::store::{Directory, FSDirectory};
use rucene::util::{BytesRef, InfoStream};

// ---------------------------------------------------------------------------
// Corpus: the formulas of TermsDictFixture
// ---------------------------------------------------------------------------

/// `TermsDictFixture.DOCS`.
const DOCS: i32 = 3000;

/// `TermsDictFixture.SOURCES`: the batches of each source index.
const SOURCES: [&[(i32, i32)]; 2] = [&[(0, 200), (200, 400)], &[(400, 700)]];

/// `TermsDictFixture.SOURCE_DELETES`.
const SOURCE_DELETES: [i32; 2] = [13, 450];

/// `TermsDictFixture.FIELDS`.
const FIELDS: [&str; 4] = ["body", "id", "kw", "tag"];

fn id(i: i32) -> String {
    format!("id-{i:05}")
}

/// `TermsDictFixture.base26(int, int)`.
fn base26(mut n: i32, len: usize) -> String {
    let mut c = vec![b'a'; len];
    for k in (0..len).rev() {
        c[k] = b'a' + (n % 26) as u8;
        n /= 26;
    }
    String::from_utf8(c).unwrap()
}

fn kw_a(i: i32) -> String {
    format!(
        "{}{}{}",
        base26((i * 37) % 17576, 3),
        if i % 3 == 0 { "ing" } else { "" },
        if i % 7 == 0 { "ed" } else { "" }
    )
}

fn kw_b(i: i32) -> String {
    format!("pre{}{}", base26(i % 676, 2), i % 7)
}

/// `TermsDictFixture.kwC(int)`.
fn kw_c(i: i32) -> String {
    format!("z\u{e9}{}", base26(i / 50, 2))
}

/// `TermsDictFixture.body(int)`.
fn body(i: i32) -> String {
    let mut words: Vec<String> = Vec::new();
    for j in 0..i % 6 + 3 {
        let word = if j % 3 == 2 {
            words[(j - 1) as usize].clone()
        } else {
            format!("t{}", base26((i * 31 + j * 17) % 2500, 3))
        };
        words.push(word);
    }
    words.join(" ")
}

/// `TermsDictFixture.TEXT_TYPE`.
fn text_type() -> FieldType {
    let t = FieldType::new();
    t.set_tokenized(true).unwrap();
    t.set_index_options(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS)
        .unwrap();
    t.freeze();
    t
}

fn bytes(text: &str) -> BytesRef {
    BytesRef::new(text.as_bytes().to_vec())
}

fn term(field: &str, text: &str) -> Term {
    Term::new(field, bytes(text))
}

fn keyword(field: &str, value: String) -> Box<StringField> {
    Box::new(StringField::new(field, value, Store::NO).unwrap())
}

/// `TermsDictFixture.doc(int)`; the perturbed corpus gives document 0 the
/// keyword `aaaa`.
fn doc(i: i32, perturbed: bool) -> Document {
    let mut d = Document::new();
    d.add(Box::new(StringField::new("id", id(i), Store::YES).unwrap()));
    d.add(keyword("kw", kw_a(i)));
    d.add(keyword("kw", kw_b(i)));
    if i % 50 == 0 {
        d.add(keyword("kw", kw_c(i)));
    }
    if perturbed && i == 0 {
        d.add(keyword("kw", "aaaa".to_string()));
    }
    d.add(Box::new(Field::new("body", body(i), text_type()).unwrap()));
    d.add(keyword("tag", format!("g{}", i % 13)));
    d
}

/// `TermsDictFixture.config()`.
fn config() -> IndexWriterConfig {
    let config = IndexWriterConfig::new();
    config.set_open_mode(OpenMode::CREATE).unwrap();
    config
        .set_merge_scheduler(Some(Arc::new(SerialMergeScheduler::new())))
        .unwrap();
    config
}

fn open_dir(path: &Path) -> Arc<dyn Directory> {
    std::fs::create_dir_all(path).unwrap();
    Arc::new(FSDirectory::open(path).unwrap())
}

/// `TermsDictFixture.write(String)`'s single-segment `terms` index.
fn write_terms(root: &Path, perturbed: bool) {
    let _ = register_codec("Lucene104", Lucene104Codec::new());
    let terms_config = config();
    terms_config.set_use_compound_file(false).unwrap();
    terms_config.set_ram_buffer_size_mb(256.0).unwrap();
    let w = IndexWriter::new(open_dir(&root.join("terms")), terms_config).unwrap();
    for i in 0..DOCS {
        w.add_document(&doc(i, perturbed)).unwrap();
    }
    w.commit().unwrap();
    w.close().unwrap();
}

/// `TermsDictFixture.write(String)`: the same operations, in order.
fn write_all(root: &Path) {
    write_terms(root, false);
    for (s, batches) in SOURCES.iter().enumerate() {
        let w = IndexWriter::new(open_dir(&root.join(format!("src{}", s + 1))), config()).unwrap();
        for (b, (from, to)) in batches.iter().enumerate() {
            for i in *from..*to {
                w.add_document(&doc(i, false)).unwrap();
            }
            if b == 0 {
                w.delete_documents_by_terms(vec![term("id", &id(SOURCE_DELETES[s]))])
                    .unwrap();
            }
            w.commit().unwrap();
        }
        w.close().unwrap();
    }
    let src1 = open_dir(&root.join("src1"));
    let src2 = open_dir(&root.join("src2"));

    let w = IndexWriter::new(open_dir(&root.join("dirs")), config()).unwrap();
    w.add_indexes_dirs(&[Arc::clone(&src1), Arc::clone(&src2)])
        .unwrap();
    w.commit().unwrap();
    w.close().unwrap();

    let r1 = directory_reader::open(Arc::clone(&src1)).unwrap();
    let r2 = directory_reader::open(Arc::clone(&src2)).unwrap();
    let readers: Vec<Arc<dyn CodecReader>> = [&r1, &r2]
        .into_iter()
        .flat_map(|r| Arc::clone(r).leaves().unwrap())
        .map(|ctx| SlowCodecReaderWrapper::wrap(ctx.leaf_reader()).unwrap())
        .collect();
    let w = IndexWriter::new(open_dir(&root.join("readers")), config()).unwrap();
    w.add_indexes_codec_readers(&readers).unwrap();
    w.commit().unwrap();
    w.close().unwrap();
}

// ---------------------------------------------------------------------------
// Query set: the output of TermsDictFixture.query, line for line
// ---------------------------------------------------------------------------

/// `TermsDictFixture.TOP_N`.
const TOP_N: i32 = 20;

/// `Float.floatToIntBits`, which collapses every NaN to `0x7fc00000`.
fn bits(f: f32) -> String {
    let raw = if f.is_nan() { 0x7fc0_0000 } else { f.to_bits() };
    format!("{raw:08x}")
}

fn relation(total: &TotalHits) -> &'static str {
    match total.relation() {
        TotalHitsRelation::EQUAL_TO => "EQUAL_TO",
        TotalHitsRelation::GREATER_THAN_OR_EQUAL_TO => "GREATER_THAN_OR_EQUAL_TO",
    }
}

fn stored_id(stored: &dyn StoredFields, doc: i32) -> String {
    let document = stored.document(doc).unwrap();
    document
        .get_field("id")
        .and_then(|f| f.string_value())
        .unwrap_or_else(|| "null".to_string())
}

/// `BytesRef.utf8ToString()`, `null` for a missing value.
fn text(b: Option<&BytesRef>) -> String {
    b.map_or_else(
        || "null".to_string(),
        |b| String::from_utf8_lossy(b.slice()).into_owned(),
    )
}

/// `TermsDictFixture.probes()`, in the same order.
fn probes() -> Vec<String> {
    let mut p: Vec<String> = [
        "",
        "a",
        "b",
        "g1",
        "g12",
        "g9",
        "id-0",
        "id-01234",
        "id-01234x",
        "id-02999",
        "id-3",
        "pre",
        "preab",
        "preab3",
        "preaz9",
        "prezz",
        "t",
        "taa",
        "tdzz",
        "tz",
        "tag",
        "z",
        "z\u{e9}",
        "zz",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    for k in [0, 1, 777, 2999] {
        p.push(kw_a(k));
        p.push(format!("{}0", kw_a(k)));
        p.push(kw_b(k));
        p.push(format!("{}0", id(k)));
    }
    p
}

/// `TermsDictFixture.printStats`.
fn stats_line(field: &str, terms: &dyn Terms) -> String {
    format!(
        "field {field} size={} sumDocFreq={} sumTotalTermFreq={} docCount={} min={} max={}",
        terms.size().unwrap(),
        terms.sum_doc_freq().unwrap(),
        terms.sum_total_term_freq().unwrap(),
        terms.doc_count().unwrap(),
        text(terms.min().unwrap().as_ref()),
        text(terms.max().unwrap().as_ref())
    )
}

/// `TermsDictFixture.printReader`.
fn print_reader(out: &mut String, reader: &Arc<dyn IndexReader>) {
    writeln!(
        out,
        "reader maxDoc={} numDocs={}",
        reader.max_doc(),
        reader.num_docs()
    )
    .unwrap();
    for ctx in &Arc::clone(reader).leaves().unwrap() {
        let leaf = ctx.leaf_reader();
        writeln!(
            out,
            "leaf ord={} docBase={} maxDoc={} numDocs={}",
            ctx.ord(),
            ctx.doc_base(),
            leaf.max_doc(),
            leaf.num_docs()
        )
        .unwrap();
    }
}

fn open_reader(path: &Path) -> Arc<dyn IndexReader> {
    let dir: Arc<dyn Directory> = Arc::new(FSDirectory::open(path).unwrap());
    directory_reader::open(dir).unwrap()
}

/// `TermsDictFixture.dumpTerms`.
fn dump_terms(out: &mut String, reader: &Arc<dyn IndexReader>) {
    for ctx in &Arc::clone(reader).leaves().unwrap() {
        let leaf = ctx.leaf_reader();
        for field in FIELDS {
            let Some(terms) = leaf.terms(field).unwrap() else {
                writeln!(out, "field {field} leaf={} none", ctx.ord()).unwrap();
                continue;
            };
            writeln!(out, "{}", stats_line(field, terms.as_ref())).unwrap();
            let mut te = terms.iterator().unwrap();
            while let Some(t) = te.next().unwrap() {
                writeln!(
                    out,
                    "  term {} docFreq={} ttf={}",
                    text(Some(&t)),
                    te.doc_freq().unwrap(),
                    te.total_term_freq().unwrap()
                )
                .unwrap();
            }
            let mut seeker = terms.iterator().unwrap();
            for probe in probes() {
                let status = seeker.seek_ceil(&bytes(&probe)).unwrap();
                let landing = if status == SeekStatus::END {
                    String::new()
                } else {
                    format!(
                        " {} docFreq={}",
                        text(seeker.term().unwrap().as_ref()),
                        seeker.doc_freq().unwrap()
                    )
                };
                writeln!(out, "  seekCeil {probe} -> {status:?}{landing}").unwrap();
            }
        }
    }
}

/// `TermsDictFixture.queryTerms()`: `(field, text)`.
fn query_terms() -> Vec<(&'static str, String)> {
    vec![
        ("kw", kw_a(5)),
        ("kw", kw_b(30)),
        // The first word of document 10: `(10 * 31) % 2500`.
        ("body", format!("t{}", base26(310, 3))),
        ("tag", "g3".to_string()),
        ("id", id(12)),
        ("id", id(451)),
    ]
}

/// `TermsDictFixture.printAddIndexes`.
fn print_add_indexes(out: &mut String, root: &Path, name: &str) {
    let reader = open_reader(&root.join(name));
    writeln!(out, "index {name}").unwrap();
    print_reader(out, &reader);
    for ctx in &Arc::clone(&reader).leaves().unwrap() {
        let leaf = ctx.leaf_reader();
        for field in FIELDS {
            match leaf.terms(field).unwrap() {
                None => writeln!(out, "leaf={} field {field} none", ctx.ord()).unwrap(),
                Some(terms) => writeln!(
                    out,
                    "leaf={} {}",
                    ctx.ord(),
                    stats_line(field, terms.as_ref())
                )
                .unwrap(),
            }
        }
    }
    let searcher = IndexSearcher::new(Arc::clone(&reader)).unwrap();
    let stored = searcher.stored_fields().unwrap();
    for (field, value) in query_terms() {
        let top = searcher
            .search(Arc::new(TermQuery::new(term(field, &value))), TOP_N)
            .unwrap();
        writeln!(
            out,
            "query {field}:{value} total={} {}",
            top.total_hits.value(),
            relation(&top.total_hits)
        )
        .unwrap();
        for hit in &top.score_docs {
            writeln!(
                out,
                "  hit doc={} score={} id={}",
                hit.doc,
                bits(hit.score),
                stored_id(stored.as_ref(), hit.doc)
            )
            .unwrap();
        }
    }
}

/// Rucene's run of the query set over the indexes under `root`: the same
/// lines `TermsDictFixture query` prints.
fn query_report(root: &Path) -> String {
    let _ = register_codec("Lucene104", Lucene104Codec::new());
    let mut out = String::new();
    let reader = open_reader(&root.join("terms"));
    writeln!(out, "index terms").unwrap();
    print_reader(&mut out, &reader);
    dump_terms(&mut out, &reader);
    print_add_indexes(&mut out, root, "dirs");
    print_add_indexes(&mut out, root, "readers");
    out
}

// ---------------------------------------------------------------------------
// CheckIndex: the output of TermsDictFixture.check
// ---------------------------------------------------------------------------

/// `TermsDictFixture.CHECKED`.
const CHECKED: [&str; 3] = ["terms", "dirs", "readers"];

/// Collects the lines `CheckIndex` writes, as Java's `PrintStream` would.
#[derive(Default)]
struct CollectingInfoStream(Mutex<Vec<String>>);

impl InfoStream for CollectingInfoStream {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.util.PrintStreamInfoStream"
    }

    fn message(&self, _component: &str, message: &str) {
        self.0.lock().unwrap().push(message.to_string());
    }

    fn is_enabled(&self, _component: &str) -> bool {
        true
    }

    fn close(&self) {}
}

/// Rucene's `TermsDictFixture check`: per index, the status line and the
/// verbose text output, each line prefixed with `  | `.
fn check_report(root: &Path) -> String {
    let _ = register_codec("Lucene104", Lucene104Codec::new());
    let mut out = String::new();
    for name in CHECKED {
        let dir: Arc<dyn Directory> = Arc::new(FSDirectory::open(root.join(name)).unwrap());
        let log = Arc::new(CollectingInfoStream::default());
        let mut checker = CheckIndex::new(dir).unwrap();
        checker.set_level(Level::MAX_VALUE).unwrap();
        checker.set_thread_count(1).unwrap();
        checker.set_info_stream_verbose(Some(log.clone() as Arc<dyn InfoStream>), true);
        let status = checker.check_index().unwrap();
        checker.close().unwrap();
        writeln!(
            out,
            "check {name} clean={} numSegments={} numBadSegments={} totLoseDocCount={}",
            status.clean, status.num_segments, status.num_bad_segments, status.tot_lose_doc_count
        )
        .unwrap();
        // Java's `msg` is a `println` of each message.
        let printed: String = log
            .0
            .lock()
            .unwrap()
            .iter()
            .map(|m| format!("{m}\n"))
            .collect();
        for line in printed.lines() {
            writeln!(out, "  | {line}").unwrap();
        }
    }
    out
}

/// Replaces the text after `marker` up to the next space (or the end of the
/// line) with `with`, everywhere in `line`.
fn replace_token(line: &str, marker: &str, with: &str) -> String {
    let mut out = String::new();
    let mut rest = line;
    while let Some(at) = rest.find(marker) {
        out.push_str(&rest[..at + marker.len()]);
        out.push_str(with);
        let after = &rest[at + marker.len()..];
        rest = &after[after.find(' ').unwrap_or(after.len())..];
    }
    out.push_str(rest);
    out
}

/// Replaces the text between `start` and the next `end` with `with`,
/// everywhere in `line`.
fn replace_between(line: &str, start: &str, end: &str, with: &str) -> String {
    let mut out = String::new();
    let mut rest = line;
    while let Some(at) = rest.find(start) {
        let after = &rest[at + start.len()..];
        let Some(close) = after.find(end) else {
            break;
        };
        out.push_str(&rest[..at + start.len()]);
        out.push_str(with);
        rest = &after[close..];
    }
    out.push_str(rest);
    out
}

/// One `CheckIndex` line with its non-deterministic parts replaced: the
/// `[took S sec]` and `Took S sec total.` timings, the segment and commit ids
/// (`id=`), the segment size in MB and the diagnostics map.
fn normalize_check_line(line: &str) -> String {
    if let Some(at) = line.find("size (MB)=") {
        return format!("{}size (MB)=<size>", &line[..at]);
    }
    if let Some(at) = line.find("diagnostics = ") {
        return format!("{}diagnostics = <diagnostics>", &line[..at]);
    }
    let line = replace_between(line, "[took ", " sec]", "<t>");
    let line = replace_between(&line, "Took ", " sec total.", "<t>");
    replace_token(&line, "id=", "<id>")
}

/// A check report with every `CheckIndex` line normalised.
fn normalized(report: &str) -> String {
    report
        .lines()
        .map(|l| match l.strip_prefix("  | ") {
            Some(text) => format!("  | {}\n", normalize_check_line(text)),
            None => format!("{l}\n"),
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Terms-index files
// ---------------------------------------------------------------------------

/// `CodecUtil.CODEC_MAGIC`.
const CODEC_MAGIC: [u8; 4] = [0x3f, 0xd7, 0x6c, 0x17];

/// Reads a Lucene `vInt` at the start of `bytes`: `(value, length)`.
fn read_vint(bytes: &[u8]) -> (usize, usize) {
    let mut value = 0usize;
    for (k, b) in bytes.iter().enumerate().take(5) {
        value |= ((b & 0x7f) as usize) << (7 * k);
        if b & 0x80 == 0 {
            return (value, k + 1);
        }
    }
    panic!("malformed vInt in a codec header");
}

/// A codec file with its non-deterministic bytes zeroed: every occurrence of
/// the 16-byte segment id the file's index header carries
/// (`CodecUtil.writeIndexHeader`: magic, codec name as a `vInt`-prefixed
/// string, version, id) and the 8-byte CRC32 of the footer, which covers it.
fn masked(bytes: &[u8]) -> Vec<u8> {
    let mut b = bytes.to_vec();
    assert!(
        b.len() > 4 + 16 + 16 && b[0..4] == CODEC_MAGIC,
        "not a codec file"
    );
    let (name_len, vint_len) = read_vint(&b[4..]);
    let id_at = 4 + vint_len + name_len + 4;
    let id = b[id_at..id_at + 16].to_vec();
    let mut k = 0;
    while k + 16 <= b.len() {
        if b[k..k + 16] == id[..] {
            b[k..k + 16].fill(0);
            k += 16;
        } else {
            k += 1;
        }
    }
    let n = b.len();
    b[n - 8..].fill(0);
    b
}

/// The masked terms-dictionary files (`.tim`, `.tip`, `.tmd`) of the index at
/// `path`, by file name.
fn terms_files(path: &Path) -> Vec<(String, Vec<u8>)> {
    let mut files: Vec<(String, Vec<u8>)> = std::fs::read_dir(path)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| {
            p.extension()
                .is_some_and(|e| e == "tim" || e == "tip" || e == "tmd")
        })
        .map(|p| {
            (
                p.file_name().unwrap().to_string_lossy().into_owned(),
                masked(&std::fs::read(&p).unwrap()),
            )
        })
        .collect();
    files.sort();
    files
}

/// The differences between two sets of masked files, or `None`.
fn files_diff(
    left_name: &str,
    left: &[(String, Vec<u8>)],
    right_name: &str,
    right: &[(String, Vec<u8>)],
) -> Option<String> {
    let left_names: Vec<&String> = left.iter().map(|(n, _)| n).collect();
    let right_names: Vec<&String> = right.iter().map(|(n, _)| n).collect();
    if left_names != right_names {
        return Some(format!(
            "terms files of {left_name} {left_names:?} differ from those of {right_name} {right_names:?}"
        ));
    }
    let mut out = String::new();
    for ((name, a), (_, b)) in left.iter().zip(right) {
        if a != b {
            let first = a
                .iter()
                .zip(b.iter())
                .position(|(x, y)| x != y)
                .unwrap_or(a.len().min(b.len()));
            writeln!(
                out,
                "  {name}: {left_name} {} bytes, {right_name} {} bytes, first difference at offset {first}",
                a.len(),
                b.len()
            )
            .unwrap();
        }
    }
    if out.is_empty() {
        None
    } else {
        Some(format!(
            "masked terms files of {left_name} differ from {right_name}:\n{out}"
        ))
    }
}

// ---------------------------------------------------------------------------
// Java oracle
// ---------------------------------------------------------------------------

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

/// Runs `TermsDictFixture <mode> <root>`: `(success, stdout, stderr)`.
fn run_java(mode: &str, root: &Path) -> (bool, String, String) {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join(
        "tests/fixtures/java-codec-harness/src/main/java/org/apache/lucene/rucene/codec/\
         TermsDictFixture.java",
    );
    let output = Command::new("java")
        .arg("-cp")
        .arg(lucene_jar())
        .arg(fixture)
        .arg(mode)
        .arg(root)
        .output()
        .expect("a JDK is required to run the Lucene oracle");
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// The stderr lines of a Java run without the JVM's start-up warnings.
fn java_errors(stderr: &str) -> String {
    stderr
        .lines()
        .filter(|l| {
            !(l.starts_with("WARNING:")
                || l.contains("VectorizationProvider lookup")
                || l.trim().is_empty())
        })
        .collect::<Vec<_>>()
        .join("\n")
}

// ---------------------------------------------------------------------------
// Comparison
// ---------------------------------------------------------------------------

/// The first differing lines of two reports, or `None` when they agree.
fn diff(left_name: &str, left: &str, right_name: &str, right: &str) -> Option<String> {
    if left == right {
        return None;
    }
    let l: Vec<&str> = left.lines().collect();
    let r: Vec<&str> = right.lines().collect();
    let mut out = format!(
        "{left_name} ({} lines) differs from {right_name} ({} lines):\n",
        l.len(),
        r.len()
    );
    let mut shown = 0;
    let mut section = "";
    let mut context = "";
    for k in 0..l.len().max(r.len()) {
        let a = l.get(k).copied().unwrap_or("<end>");
        let b = r.get(k).copied().unwrap_or("<end>");
        if a.starts_with("index ") || a.starts_with("check ") {
            section = a;
        }
        if a.starts_with("field ") || a.starts_with("query ") || a.starts_with("leaf") {
            context = a;
        }
        if a != b {
            writeln!(
                out,
                "  line {} [{section} / {context}]\n    {left_name}: {a}\n    {right_name}: {b}",
                k + 1
            )
            .unwrap();
            shown += 1;
            if shown == 40 {
                out.push_str("  ...\n");
                break;
            }
        }
    }
    Some(out)
}

/// The `reader` lines of a report: the document counts of every index.
fn reader_lines(report: &str) -> Vec<&str> {
    report
        .lines()
        .filter(|l| l.starts_with("reader "))
        .collect()
}

fn panic_message(panic: Box<dyn std::any::Any + Send>) -> String {
    panic
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| panic.downcast_ref::<&str>().map(|s| s.to_string()))
        .unwrap_or_else(|| "panic".to_string())
}

// ---------------------------------------------------------------------------
// The test
// ---------------------------------------------------------------------------

#[test]
fn terms_dictionary_add_indexes_and_check_index_match_lucene() {
    let tmp = tempfile::Builder::new()
        .prefix("rucene-terms-dictionary")
        .tempdir()
        .unwrap();
    let r = tmp.path().join("rucene");
    let l = tmp.path().join("lucene");
    let p = tmp.path().join("rucene-perturbed");
    for dir in [&r, &l, &p] {
        std::fs::create_dir_all(dir).unwrap();
    }
    let mut failures: Vec<String> = Vec::new();
    let mut report = String::new();

    // Both sides write every index.
    write_all(&r);
    let (ok, _, err) = run_java("write", &l);
    assert!(ok, "Lucene failed to write the indexes:\n{err}");

    let rust_run = |f: fn(&Path) -> String, root: &Path| {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(root))).map_err(panic_message)
    };
    let mut java_run = |mode: &str, root: &Path, name: &str| -> Option<String> {
        let (ok, out, err) = run_java(mode, root);
        if ok {
            Some(out)
        } else {
            failures.push(format!("{name} failed:\n{out}\n{}", java_errors(&err)));
            None
        }
    };

    // (a)/(c) The terms report and the addIndexes query set, from both sides
    // over both roots.
    let lucene_on_r = java_run("query", &r, "Lucene query set on R");
    let lucene_on_l = java_run("query", &l, "Lucene query set on L");
    // (c)/(d) CheckIndex, from both sides over both roots.
    let lucene_check_r = java_run("check", &r, "Lucene CheckIndex on R");
    let lucene_check_l = java_run("check", &l, "Lucene CheckIndex on L");
    let mut rust = |f: fn(&Path) -> String, root: &Path, name: &str| -> Option<String> {
        rust_run(f, root)
            .map_err(|e| failures.push(format!("{name} panicked: {e}")))
            .ok()
    };
    let rucene_on_r = rust(query_report, &r, "Rucene query set on R");
    let rucene_on_l = rust(query_report, &l, "Rucene query set on L");
    let rucene_check_r = rust(check_report, &r, "Rucene CheckIndex on R");
    let rucene_check_l = rust(check_report, &l, "Rucene CheckIndex on L");

    // The oracle must dump thousands of terms of every field, land every
    // probe, and every query must have hits: an empty result would make the
    // comparisons vacuous.
    let oracle = lucene_on_l
        .as_ref()
        .expect("the Lucene report on L is required");
    assert!(
        oracle.lines().filter(|l| l.starts_with("  term ")).count() > 3 * DOCS as usize,
        "the oracle must dump the corpus terms"
    );
    for field in FIELDS {
        assert!(
            oracle.contains(&format!("\nfield {field} size=")),
            "the oracle must dump field {field}"
        );
    }
    for status in ["FOUND", "NOT_FOUND", "END"] {
        assert!(
            oracle.contains(&format!(" -> {status}")),
            "the probes must reach {status}"
        );
    }
    let mut lines = oracle.lines().peekable();
    let mut query_count = 0;
    while let Some(line) = lines.next() {
        if line.starts_with("query ") {
            query_count += 1;
            assert!(
                lines.peek().is_some_and(|next| next.starts_with("  hit ")),
                "the oracle query has no hits: {line}"
            );
        }
    }
    assert_eq!(
        query_count,
        2 * query_terms().len(),
        "the oracle must run the query set on dirs and readers"
    );
    writeln!(
        report,
        "== oracle report ({} lines, {query_count} queries)",
        oracle.lines().count()
    )
    .unwrap();

    let pairs = [
        ("Lucene on R", &lucene_on_r, "Rucene on R", &rucene_on_r),
        ("Lucene on L", &lucene_on_l, "Rucene on L", &rucene_on_l),
        ("Lucene on L", &lucene_on_l, "Lucene on R", &lucene_on_r),
        ("Rucene on L", &rucene_on_l, "Rucene on R", &rucene_on_r),
    ];
    for (a_name, a, b_name, b) in pairs {
        if let (Some(a), Some(b)) = (a, b) {
            if let Some(d) = diff(a_name, a, b_name, b) {
                failures.push(d);
            } else {
                writeln!(report, "== {a_name} == {b_name}").unwrap();
            }
        }
    }

    // CheckIndex: every index clean on both sides, and the normalised text
    // the same.
    let checks = [
        ("Lucene CheckIndex on R", &lucene_check_r),
        ("Lucene CheckIndex on L", &lucene_check_l),
        ("Rucene CheckIndex on R", &rucene_check_r),
        ("Rucene CheckIndex on L", &rucene_check_l),
    ];
    for (name, check) in &checks {
        if let Some(check) = check {
            let statuses: Vec<&str> = check.lines().filter(|l| l.starts_with("check ")).collect();
            if statuses.len() != CHECKED.len()
                || statuses.iter().any(|s| !s.contains(" clean=true "))
            {
                failures.push(format!(
                    "{name} does not report every index clean:\n{check}"
                ));
            }
        }
    }
    let oracle_check = lucene_check_l
        .as_ref()
        .expect("the Lucene CheckIndex report on L is required");
    assert!(
        oracle_check.contains("  |     test: terms, freq, prox...OK"),
        "the oracle CheckIndex report must hold the postings check"
    );
    let normalized_checks: Vec<(&str, Option<String>)> = checks
        .iter()
        .map(|(name, c)| (*name, c.as_deref().map(normalized)))
        .collect();
    let check_pairs = [(0, 2), (1, 3), (1, 0), (3, 2)];
    for (a, b) in check_pairs {
        let (a_name, a) = &normalized_checks[a];
        let (b_name, b) = &normalized_checks[b];
        if let (Some(a), Some(b)) = (a, b) {
            if let Some(d) = diff(a_name, a, b_name, b) {
                failures.push(d);
            } else {
                writeln!(report, "== {a_name} == {b_name} (normalised)").unwrap();
            }
        }
    }

    // (b) Terms-index bytes of the two single-segment indexes.
    let r_files = terms_files(&r.join("terms"));
    let l_files = terms_files(&l.join("terms"));
    assert_eq!(
        l_files.len(),
        3,
        "Lucene's single segment must have one .tim, .tip and .tmd: {:?}",
        l_files.iter().map(|(n, _)| n).collect::<Vec<_>>()
    );
    match files_diff("R", &r_files, "L", &l_files) {
        Some(d) => failures.push(d),
        None => writeln!(
            report,
            "== masked terms files R == L: {:?}",
            l_files
                .iter()
                .map(|(n, b)| format!("{n} ({} bytes)", b.len()))
                .collect::<Vec<_>>()
        )
        .unwrap(),
    }

    // Positive control: one more keyword in document 0 must change the terms
    // report, with the same document counts, and the terms-dictionary bytes.
    write_terms(&p, true);
    let perturbed = std::panic::catch_unwind(|| {
        let mut out = String::new();
        let reader = open_reader(&p.join("terms"));
        writeln!(out, "index terms").unwrap();
        print_reader(&mut out, &reader);
        dump_terms(&mut out, &reader);
        out
    })
    .unwrap_or_else(|e| panic!("Rucene terms report on P panicked: {}", panic_message(e)));
    let oracle_terms: String = oracle
        .lines()
        .take_while(|l| !l.starts_with("index dirs"))
        .map(|l| format!("{l}\n"))
        .collect();
    assert_eq!(
        reader_lines(&perturbed),
        reader_lines(&oracle_terms),
        "the perturbed corpus keeps the document counts"
    );
    let control = diff("Lucene on L", &oracle_terms, "Rucene on P", &perturbed);
    assert!(
        control.is_some(),
        "the comparison must fail on the perturbed corpus"
    );
    writeln!(
        report,
        "== positive control: Lucene on L != Rucene on P\n{}",
        control
            .unwrap()
            .lines()
            .take(4)
            .collect::<Vec<_>>()
            .join("\n")
    )
    .unwrap();
    let files_control = files_diff("L", &l_files, "P", &terms_files(&p.join("terms")));
    assert!(
        files_control.is_some(),
        "the masked terms comparison must fail on the perturbed corpus"
    );
    writeln!(
        report,
        "== positive control: masked terms files L != P\n{}",
        files_control.unwrap().trim_end()
    )
    .unwrap();

    println!("{report}");
    assert!(
        failures.is_empty(),
        "{} terms-dictionary failure(s):\n\n{}",
        failures.len(),
        failures.join("\n\n")
    );
}
