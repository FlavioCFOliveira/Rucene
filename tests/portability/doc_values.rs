//! Portability tests for the doc-values writers.
//!
//! Every test drives the same document table through Rucene's indexing chain
//! and through a Java harness that indexes the identical documents with
//! Apache Lucene Core 10.5.0, then asserts three ways:
//!
//! 1. the `.dvd` and `.dvm` files the two sides write are **byte for byte
//!    identical**;
//! 2. Rucene reads Lucene's segment and decodes exactly what Lucene's own
//!    reader decodes;
//! 3. Lucene reads Rucene's segment through its real `Lucene90DocValuesFormat`
//!    producer and decodes exactly what it decodes from its own files.
//!
//! The cases span the shape dimensions of the format rather than only its
//! values, because a byte difference can hide behind a shape:
//!
//! * `numeric` — three numeric fields with wide, repeating and constant
//!   values, which selects between the GCD, unique-value-table and
//!   single-value-in-metadata encodings;
//! * `sparse` / `dense` — `IndexedDISI`'s SPARSE and DENSE block encodings,
//!   driven by how many documents inside one 65536-document block carry the
//!   field;
//! * `binary` — lengths from zero (the empty value) upward, dense and sparse;
//! * `sorted` — a dictionary whose first-seen order disagrees with its sorted
//!   order, so a writer that keeps insertion-order ordinals diverges;
//! * `sortednumeric` — one to three values per document, duplicates included;
//!   the format keeps them (it is a list, not a set);
//! * `sortedsetsingle` — every document carries exactly one value, which
//!   Lucene writes through the single-valued route
//!   (`Lucene90DocValuesConsumer.isSingleValued`, whose metadata byte is the
//!   `SORTED` one);
//!   [`sortedsetmulti`] drives the multi-valued addresses route;
//! * `mixed` — all five types in one segment, with gaps in several;
//! * `blocks` — numeric values whose range changes from one 16384-value block
//!   to the next, so `writeValues` takes its per-block layout (`doBlocks`,
//!   `Lucene90DocValuesConsumer.java:485-497`) and the reader its
//!   `VaryingBPVReader`, for a dense, a sparse and a sorted-numeric field
//!   (rmp #239);
//! * `sortedoneterm` — one term in every document, so every ordinal is 0 and
//!   the GCD `writeValues` computes for the ordinals stays 0
//!   (`Lucene90DocValuesConsumer.java:390-401`, rmp #239).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

use rucene::analysis::{Analyzer, StandardAnalyzer};
use rucene::codecs::state::SegmentReadState;
use rucene::codecs::state::SegmentWriteState;
use rucene::codecs::{
    register_codec, register_doc_values_format, Codec, DocValuesProducer, Lucene104Codec,
    Lucene90DocValuesFormat,
};
use rucene::document::{
    BinaryDocValuesField, Document, NumericDocValuesField, SortedDocValuesField,
    SortedNumericDocValuesField, SortedSetDocValuesField,
};
use rucene::index::documents_writer::{BufferedUpdates, IndexingChain};
use rucene::index::field_infos::{FieldInfosBuilder, FieldNumbers};
use rucene::index::index_writer_config::LiveIndexWriterConfig;
use rucene::index::indexing_chain::DefaultIndexingChain;
use rucene::index::{DocValuesType, FieldInfos, SegmentInfo, SegmentInfos};
use rucene::search::{DocIdSetIterator, NO_MORE_DOCS};
use rucene::store::{
    flush_io_context, Directory, FSDirectory, FlushInfo, TrackingDirectoryWrapper,
    DEFAULT_IO_CONTEXT,
};
use rucene::util::{BytesRef, NoOutputInfoStream, Version};

/// The two files a doc-values segment is made of.
const EXTENSIONS: [&str; 2] = ["dvd", "dvm"];

static HARNESS_LOCK: Mutex<()> = Mutex::new(());

fn harness_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("java-codec-harness")
}

fn which_mvn() -> Result<String, String> {
    for candidate in ["mvn", "/usr/bin/mvn", "/usr/local/bin/mvn"] {
        if Command::new(candidate)
            .arg("-v")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
        {
            return Ok(candidate.to_string());
        }
    }
    Err("Maven is not available on PATH".to_string())
}

/// Fails the test when Maven is unavailable.
///
/// A portability test proves compatibility against the reference Java
/// implementation, so it has nothing to assert without the harness: skipping
/// would report success while proving nothing.
fn require_maven() {
    if let Err(reason) = which_mvn() {
        panic!("doc-values portability tests require Maven and a JDK: {reason}");
    }
}

/// What the Java harness reports about the segment it committed.
#[derive(Debug)]
struct JavaSegment {
    name: String,
    id: [u8; 16],
    max_doc: i32,
    /// Whether the segment was bundled into a `.cfs`.
    compound: bool,
    /// One `dv*=...` line per value Lucene's own reader decoded, plus one
    /// `dvdict=...` line per sorted dictionary.
    dump: Vec<String>,
    /// The `.fnm` entry of every field, as Lucene committed it.
    field_infos: Vec<String>,
}

fn run_java_fixture(out_dir: &Path, case: &str) -> Result<JavaSegment, String> {
    let _guard = HARNESS_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let harness = harness_dir();
    if !harness.join("pom.xml").exists() {
        return Err(format!("pom.xml not found in {}", harness.display()));
    }
    let mvn = which_mvn()?;
    let output = Command::new(mvn)
        .arg("-q")
        .arg("compile")
        .arg("exec:java")
        .arg("-Dexec.mainClass=org.apache.lucene.rucene.codec.DocValuesFixture")
        .arg(format!("-Dexec.args={} {}", out_dir.display(), case))
        .current_dir(&harness)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| format!("failed to spawn Maven: {e}"))?;

    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    if !output.status.success() {
        return Err(format!(
            "Java harness failed for case {case}:\nstdout:\n{stdout}\nstderr:\n{stderr}"
        ));
    }
    if !stdout.lines().any(|line| line.trim() == "read_ok=true") {
        return Err(format!("the fixture did not finish:\n{stdout}"));
    }
    parse_segment(&stdout)
}

fn parse_segment(stdout: &str) -> Result<JavaSegment, String> {
    let mut name = None;
    let mut id = None;
    let mut max_doc = None;
    let mut compound = None;
    let mut dump = Vec::new();
    let mut field_infos = Vec::new();
    for line in stdout.lines() {
        if let Some(value) = line.strip_prefix("segment=") {
            name = Some(value.trim().to_string());
        } else if let Some(value) = line.strip_prefix("segment_id=") {
            id = Some(parse_id(value.trim())?);
        } else if let Some(value) = line.strip_prefix("max_doc=") {
            max_doc = Some(
                value
                    .trim()
                    .parse::<i32>()
                    .map_err(|e| format!("bad max_doc: {e}"))?,
            );
        } else if let Some(value) = line.strip_prefix("compound=") {
            compound = Some(value.trim() == "true");
        } else if let Some(value) = line.strip_prefix("fieldinfo=") {
            field_infos.push(value.trim().to_string());
        } else if line.starts_with("dv") && !line.starts_with("dvType") {
            dump.push(line.trim_end().to_string());
        }
    }
    Ok(JavaSegment {
        name: name.ok_or_else(|| format!("harness printed no segment name:\n{stdout}"))?,
        id: id.ok_or_else(|| format!("harness printed no segment id:\n{stdout}"))?,
        max_doc: max_doc.ok_or_else(|| format!("harness printed no max doc:\n{stdout}"))?,
        compound: compound.ok_or_else(|| format!("harness printed no compound flag:\n{stdout}"))?,
        dump,
        field_infos,
    })
}

fn parse_id(raw: &str) -> Result<[u8; 16], String> {
    if raw.len() != 32 {
        return Err(format!("unexpected segment id {raw:?}"));
    }
    let mut bytes = [0u8; 16];
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&raw[index * 2..index * 2 + 2], 16)
            .map_err(|e| format!("bad segment id {raw:?}: {e}"))?;
    }
    Ok(bytes)
}

/// Renders a [`DocValuesType`] the way `DocValuesType.valueOf` and the
/// fixture's `fieldinfo=` lines spell it.
fn doc_values_type_name(doc_values_type: DocValuesType) -> &'static str {
    match doc_values_type {
        DocValuesType::NONE => "NONE",
        DocValuesType::NUMERIC => "NUMERIC",
        DocValuesType::BINARY => "BINARY",
        DocValuesType::SORTED => "SORTED",
        DocValuesType::SORTED_NUMERIC => "SORTED_NUMERIC",
        DocValuesType::SORTED_SET => "SORTED_SET",
    }
}

/// Reads the doc values Rucene wrote with the real Lucene reader and returns
/// the same lines `DocValuesFixture.dumpLeaf` would have printed.
fn read_with_java(
    dir: &Path,
    segment_name: &str,
    segment_id: [u8; 16],
    max_doc: i32,
    field_infos: &FieldInfos,
) -> Result<Vec<String>, String> {
    let _guard = HARNESS_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let mvn = which_mvn()?;
    let id_hex: String = segment_id
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    // Only the fields that actually carry doc values may be listed: Lucene's
    // reader refuses a metadata entry whose field says `NONE`. The per-field
    // format attributes are the ones the writing codec stored in the field
    // infos: the reader resolves its concrete format through them. Rucene's
    // chain clones each field info, so the attributes the doc-values consumer
    // set during flush are not visible on the finished field infos; they are
    // recomputed here exactly as the writing codec determined them — one
    // format per segment, so the per-format instance counter is 0.
    let per_field_codec = Lucene104Codec::new();
    let fields: Vec<String> = field_infos
        .iter()
        .filter(|info| info.doc_values_type != DocValuesType::NONE)
        .map(|info| {
            let format = per_field_codec
                .get_doc_values_format_for_field(&info.name)
                .name()
                .to_string();
            format!(
                "{}:{}:{}:{}:{}",
                info.name,
                info.number,
                doc_values_type_name(info.doc_values_type),
                format,
                0
            )
        })
        .collect();
    let output = Command::new(mvn)
        .arg("-q")
        .arg("compile")
        .arg("exec:java")
        .arg("-Dexec.mainClass=org.apache.lucene.rucene.codec.DocValuesReaderFixture")
        .arg(format!(
            "-Dexec.args={} {} {} {} {}",
            dir.display(),
            segment_name,
            id_hex,
            max_doc,
            if fields.is_empty() {
                "-".to_string()
            } else {
                fields.join(",")
            }
        ))
        .current_dir(harness_dir())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| format!("failed to spawn Maven: {e}"))?;

    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    if !output.status.success() {
        return Err(format!(
            "Lucene could not read the Rucene-written segment:\nstdout:\n{stdout}\nstderr:\n{stderr}"
        ));
    }
    if !stdout.lines().any(|line| line.trim() == "read_ok=true") {
        return Err(format!("the reader fixture did not finish:\n{stdout}"));
    }
    Ok(stdout
        .lines()
        .filter(|line| line.starts_with("dv"))
        .map(|line| line.trim_end().to_string())
        .collect())
}

// ---------------------------------------------------------------------------
// Rucene side
// ---------------------------------------------------------------------------

fn ensure_codec() -> Arc<dyn Codec> {
    let _ = register_codec("Lucene104", Lucene104Codec::new());
    // The writer stores `PerFieldDocValuesFormat.format=Lucene90` in the field
    // infos; the per-field reader resolves that name through the global
    // registry, exactly as `DocValuesFormat.forName` does in Lucene.
    let _ = register_doc_values_format("Lucene90", Lucene90DocValuesFormat::new());
    rucene::codecs::default_codec().expect("Lucene104 codec is registered")
}

/// Renders every doc value of every field exactly as `DocValuesReaderFixture`
/// does, so the two sides compare as plain strings.
fn dump(producer: &dyn DocValuesProducer, field_infos: &FieldInfos) -> Vec<String> {
    let mut lines = Vec::new();
    for info in field_infos.iter() {
        match info.doc_values_type {
            DocValuesType::NONE => {}
            DocValuesType::NUMERIC => {
                let mut values = producer.get_numeric(info).expect("numeric").unwrap();
                loop {
                    let doc = values.next_doc().expect("next doc");
                    if doc == NO_MORE_DOCS {
                        break;
                    }
                    lines.push(format!(
                        "dvnum={doc} {} {}",
                        info.name,
                        values.long_value().expect("long value")
                    ));
                }
            }
            DocValuesType::BINARY => {
                let mut values = producer.get_binary(info).expect("binary").unwrap();
                loop {
                    let doc = values.next_doc().expect("next doc");
                    if doc == NO_MORE_DOCS {
                        break;
                    }
                    let value = values.binary_value().expect("binary value");
                    lines.push(format!("dvbin={doc} {} {}", info.name, hex(value.slice())));
                }
            }
            DocValuesType::SORTED => {
                let values = producer.get_sorted(info).expect("sorted").unwrap();
                let count = values.get_value_count().expect("value count");
                let dictionary: Vec<String> = (0..count)
                    .map(|ord| hex(values.lookup_ord(ord).expect("term").slice()))
                    .collect();
                lines.push(format!(
                    "dvdict={} {count}:{}",
                    info.name,
                    dictionary.join(",")
                ));
                let mut values = producer.get_sorted(info).expect("sorted").unwrap();
                loop {
                    let doc = values.next_doc().expect("next doc");
                    if doc == NO_MORE_DOCS {
                        break;
                    }
                    let ord = values.ord_value().expect("ord");
                    let term = values.lookup_ord(ord).expect("term");
                    lines.push(format!(
                        "dvsort={doc} {} {ord} {}",
                        info.name,
                        hex(term.slice())
                    ));
                }
            }
            DocValuesType::SORTED_NUMERIC => {
                let mut values = producer
                    .get_sorted_numeric(info)
                    .expect("sorted numeric")
                    .unwrap();
                loop {
                    let doc = values.next_doc().expect("next doc");
                    if doc == NO_MORE_DOCS {
                        break;
                    }
                    let count = values.doc_value_count().expect("count");
                    let mut body = Vec::new();
                    for _ in 0..count {
                        body.push(values.next_value().expect("value").to_string());
                    }
                    lines.push(format!(
                        "dvsortnum={doc} {} {count}:{}",
                        info.name,
                        body.join(",")
                    ));
                }
            }
            DocValuesType::SORTED_SET => {
                let mut values = producer.get_sorted_set(info).expect("sorted set").unwrap();
                let count = values.get_value_count().expect("value count");
                let mut body = Vec::new();
                for ord in 0..count {
                    let term = values.lookup_ord(ord).expect("term");
                    body.push(hex(term.slice()));
                }
                lines.push(format!("dvdict={} {count}:{}", info.name, body.join(",")));
                loop {
                    let doc = values.next_doc().expect("next doc");
                    if doc == NO_MORE_DOCS {
                        break;
                    }
                    let count = values.doc_value_count().expect("doc value count");
                    let mut ords = Vec::new();
                    for _ in 0..count {
                        ords.push(values.next_ord().expect("ord"));
                    }
                    lines.push(format!(
                        "dvsortset={doc} {} {count}:{}",
                        info.name,
                        ords.iter()
                            .map(|ord| ord.to_string())
                            .collect::<Vec<_>>()
                            .join(",")
                    ));
                }
            }
        }
    }
    // The Java fixture's stdout is compared line-by-line after `trim_end`, so an
    // empty binary value ("dvbin=4 bin " in Java, with the trailing space its
    // print leaves behind) must be normalized the same way here.
    lines
        .into_iter()
        .map(|line| line.trim_end().to_string())
        .collect()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Opens a Lucene-written index with Rucene and dumps its doc values.
fn read_java_index(dir: &Path) -> Vec<String> {
    let codec = ensure_codec();
    let directory: Arc<dyn Directory> = Arc::new(FSDirectory::open(dir).expect("directory"));
    let infos = SegmentInfos::read_latest_commit(Arc::clone(&directory)).expect("segments file");
    let segment_info = infos.info(0).info.clone();
    let field_infos = codec
        .field_infos_format()
        .read(directory.as_ref(), &segment_info, "", &*DEFAULT_IO_CONTEXT)
        .expect("field infos");
    let read_state = SegmentReadState::new(
        directory.as_ref(),
        &segment_info,
        &field_infos,
        &*DEFAULT_IO_CONTEXT,
    );
    let producer = codec
        .doc_values_format()
        .fields_producer(&read_state)
        .expect("doc values producer");
    producer.check_integrity().expect("integrity");
    dump(producer.as_ref(), &field_infos)
}

/// Finds the one doc-values file of the given extension for `segment`.
///
/// The per-field format names the files `<segment>_Lucene90_0.<ext>`, so the
/// test locates them by extension instead of assuming the bare name.
fn find_doc_values_file(dir: &Path, segment: &str, extension: &str, case: &str) -> PathBuf {
    let mut matches: Vec<PathBuf> = std::fs::read_dir(dir)
        .expect("segment directory")
        .filter_map(|entry| {
            let path = entry.expect("dir entry").path();
            let name = path.file_name().and_then(|n| n.to_str()).map(String::from);
            name.filter(|n| n.starts_with(segment) && n.ends_with(&format!(".{extension}")))
                .map(|_| path)
        })
        .collect();
    matches.sort();
    assert_eq!(
        matches.len(),
        1,
        "[{case}] expected exactly one .{extension} file for segment {segment}, found {matches:?}"
    );
    matches.remove(0)
}

/// Compares the doc-values files of the two directories byte for byte.
fn assert_doc_values_bytes_equal(java_dir: &Path, rust_dir: &Path, segment: &str, case: &str) {
    if let Ok(keep) = std::env::var("RUCENE_DV_DEBUG_DIR") {
        let _ = std::fs::create_dir_all(&keep);
        for (side, from) in [("java", java_dir), ("rust", rust_dir)] {
            for entry in std::fs::read_dir(from).expect("segment directory") {
                let path = entry.expect("dir entry").path();
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    if name.starts_with(segment) {
                        let _ = std::fs::copy(&path, format!("{keep}/{side}_{case}_{name}"));
                    }
                }
            }
        }
    }
    let mut compared = 0;
    for extension in EXTENSIONS {
        let java_file = find_doc_values_file(java_dir, segment, extension, case);
        let file_name = java_file
            .file_name()
            .and_then(|n| n.to_str())
            .expect("file name")
            .to_string();
        let rust_file = rust_dir.join(&file_name);
        assert!(
            rust_file.exists(),
            "[{case}] Rucene did not write {file_name}"
        );
        let expected = std::fs::read(&java_file).expect("read java file");
        let actual = std::fs::read(&rust_file).expect("read rust file");
        if expected != actual {
            let first = expected
                .iter()
                .zip(actual.iter())
                .position(|(left, right)| left != right)
                .unwrap_or_else(|| std::cmp::min(expected.len(), actual.len()));
            panic!(
                "[{case}] {file_name} differs at byte {first} (lucene {} bytes, rucene {} bytes)\n  lucene: {}\n  rucene: {}",
                expected.len(),
                actual.len(),
                hex_window(&expected, first),
                hex_window(&actual, first)
            );
        }
        compared += 1;
    }
    assert_eq!(
        compared,
        EXTENSIONS.len(),
        "[{case}] both doc-values files must be compared"
    );
}

fn hex_window(bytes: &[u8], centre: usize) -> String {
    let from = centre.saturating_sub(16);
    let to = std::cmp::min(centre + 16, bytes.len());
    let body: Vec<String> = bytes[from..to].iter().map(|b| format!("{b:02x}")).collect();
    format!("[{from}..{to}] {}", body.join(" "))
}

// ---------------------------------------------------------------------------
// Rucene write side
// ---------------------------------------------------------------------------

/// One value of one field of one document, as the case tables express it.
#[derive(Clone, Debug, PartialEq)]
enum Dv {
    Num(i64),
    Bin(&'static [u8]),
    Sorted(&'static [u8]),
    SortedNum(Vec<i64>),
    SortedSet(Vec<&'static [u8]>),
}

/// The fields of one case, in first-appearance order: `(name, type)`.
fn fields(case: &str) -> Vec<(&'static str, DocValuesType)> {
    match case {
        "numeric" => vec![
            ("num", DocValuesType::NUMERIC),
            ("gcd", DocValuesType::NUMERIC),
            ("konst", DocValuesType::NUMERIC),
        ],
        "sparse" => vec![
            ("sparse", DocValuesType::NUMERIC),
            ("all", DocValuesType::NUMERIC),
        ],
        "dense" => vec![
            ("dense", DocValuesType::NUMERIC),
            ("every", DocValuesType::NUMERIC),
        ],
        "binary" => vec![
            ("bin", DocValuesType::BINARY),
            ("sbin", DocValuesType::BINARY),
        ],
        "sorted" => vec![("sort", DocValuesType::SORTED)],
        "sortednumeric" => vec![("snum", DocValuesType::SORTED_NUMERIC)],
        "sortedsetsingle" => vec![("ss", DocValuesType::SORTED_SET)],
        "sortedsetmulti" => vec![("ss", DocValuesType::SORTED_SET)],
        "sortedlongterms" => vec![("lterm", DocValuesType::SORTED)],
        "mixed" => vec![
            ("mnum", DocValuesType::NUMERIC),
            ("mbin", DocValuesType::BINARY),
            ("msort", DocValuesType::SORTED),
            ("msnum", DocValuesType::SORTED_NUMERIC),
            ("mss", DocValuesType::SORTED_SET),
        ],
        "blocks" => vec![
            ("vbpv", DocValuesType::NUMERIC),
            ("svbpv", DocValuesType::NUMERIC),
            ("snvbpv", DocValuesType::SORTED_NUMERIC),
        ],
        "sortedoneterm" => vec![
            ("one", DocValuesType::SORTED),
            ("oneset", DocValuesType::SORTED_SET),
        ],
        "ordblocks" => vec![
            ("ob", DocValuesType::SORTED),
            ("obs", DocValuesType::SORTED_SET),
        ],
        "extreme" => vec![
            ("ex", DocValuesType::NUMERIC),
            ("exs", DocValuesType::SORTED_NUMERIC),
            ("exw", DocValuesType::NUMERIC),
        ],
        "boundary09" | "boundary095" => vec![("bd", DocValuesType::NUMERIC)],
        "extremeblocks" => vec![("xb", DocValuesType::NUMERIC)],
        other => panic!("unknown case: {other}"),
    }
}

/// Builds term `i`: a zero-padded ordinal prefix followed by a rotating
/// alphabet, padded to exactly 400 ASCII bytes.
///
/// Must produce byte-identical terms to `DocValuesFixture.longTerm(int)`;
/// every character is ASCII, so `len()` counts bytes and Java's `length()`
/// counts the same characters.
fn long_term(i: usize) -> String {
    let mut term = format!("t{i:04}-");
    let mut j = 0usize;
    while term.len() < 400 {
        term.push((b'a' + ((i + j) % 26) as u8) as char);
        j += 1;
    }
    term
}

/// The documents of one case, in order; each entry is a
/// `(field index, value)` pair, and a field may appear more than once in
/// one document.
fn documents(case: &str) -> Vec<Vec<(usize, Dv)>> {
    match case {
        "numeric" => (0..12)
            .map(|doc| {
                vec![
                    (0, Dv::Num((doc as i64 - 6) * 1_000_003)),
                    (1, Dv::Num(doc as i64 % 4)),
                    (2, Dv::Num(42)),
                ]
            })
            .collect(),
        "sparse" => (0..40)
            .map(|doc| {
                let mut values = Vec::new();
                if doc % 3 == 0 {
                    values.push((0, Dv::Num(doc as i64 * 13 - 40)));
                }
                values.push((1, Dv::Num(doc as i64 % 9)));
                values
            })
            .collect(),
        "dense" => (0..10_000)
            .map(|doc| {
                let mut values = Vec::new();
                if doc % 2 == 0 {
                    values.push((0, Dv::Num(1 + doc as i64 % 11)));
                }
                values.push((1, Dv::Num(doc as i64)));
                values
            })
            .collect(),
        "binary" => (0..12)
            .map(|doc| {
                let mut values = vec![(0, Dv::Bin(binary_for(doc)))];
                if doc % 5 == 2 {
                    // Lucene's fixture writes the empty value at doc 7, which
                    // the format must not confuse with "absent".
                    values.push((
                        1,
                        Dv::Bin(if doc == 7 {
                            b""
                        } else {
                            leaked(format!("s{doc}"))
                        }),
                    ));
                }
                values
            })
            .collect(),
        "sorted" => (0..10)
            .map(|doc| {
                let dict = ["zz", "apple", "mm", "bee"];
                vec![(0, Dv::Sorted(dict[doc % 4].as_bytes()))]
            })
            .collect(),
        "sortednumeric" => (0..20)
            .map(|doc| {
                let mut values = Vec::new();
                if doc % 2 == 0 {
                    let count = 1 + doc % 3;
                    for i in 0..count {
                        values.push((
                            0,
                            Dv::SortedNum(vec![(doc as i64 * 31 - 9) * (i as i64 + 1)]),
                        ));
                    }
                }
                values
            })
            .collect(),
        "sortedsetsingle" => (0..10)
            .map(|doc| {
                let single = ["ant", "bee", "cow", "dog", "emu", "fox"];
                let mut values = Vec::new();
                if doc % 3 == 0 {
                    values.push((0, Dv::SortedSet(vec![single[doc % 6].as_bytes()])));
                }
                values
            })
            .collect(),
        "sortedsetmulti" => (0..12)
            .map(|doc| {
                let mut values = Vec::new();
                if doc % 2 == 0 {
                    let dict = ["bee", "ant", "cow", "ant", "emu", "bee", "dog", "fox"];
                    let count = 1 + doc % 3;
                    // Same add order as the Java fixture; deduplication is
                    // the writer's job, not the table's.
                    let terms = (0..count)
                        .map(|i| dict[(doc * 2 + i) % 8].as_bytes())
                        .collect();
                    values.push((0, Dv::SortedSet(terms)));
                }
                values
            })
            .collect(),
        "sortedlongterms" => (0..200)
            .map(|doc| {
                // Leaked because the case table is typed `&'static [u8]`, and
                // these terms are generated rather than written out. The test
                // process is short-lived and builds each case once.
                let term: &'static [u8] = Box::leak(long_term(doc).into_bytes().into_boxed_slice());
                vec![(0, Dv::Sorted(term))]
            })
            .collect(),
        "mixed" => (0..12)
            .map(|doc| {
                let dict = ["zz", "apple", "mm", "bee", "kiwi"];
                let set = ["bee", "ant", "cow", "dog"];
                let mut values = Vec::new();
                if doc % 2 == 0 {
                    values.push((0, Dv::Num((doc as i64 - 6) * 77)));
                }
                if doc % 3 == 0 {
                    values.push((1, Dv::Bin(leaked(format!("mb{doc}")))));
                }
                if doc % 3 != 1 {
                    values.push((2, Dv::Sorted(dict[doc % 5].as_bytes())));
                }
                if doc % 4 != 0 {
                    values.push((3, Dv::SortedNum(vec![doc as i64 - 5])));
                    if doc % 4 == 3 {
                        values.push((3, Dv::SortedNum(vec![doc as i64 * 13])));
                    }
                }
                if doc % 2 == 1 {
                    values.push((4, Dv::SortedSet(vec![set[doc % 4].as_bytes()])));
                }
                values
            })
            .collect(),
        "blocks" => (0..34_000)
            .map(|doc: i64| {
                let mut values = vec![(0, Dv::Num(block_value(doc)))];
                if doc % 2 == 0 {
                    values.push((1, Dv::Num(sparse_block_value(doc / 2))));
                }
                let pair = if doc < 8192 {
                    [doc % 3, 1 + doc % 3]
                } else {
                    [doc * 104_729, doc * 104_729 + 5]
                };
                for value in pair {
                    values.push((2, Dv::SortedNum(vec![value])));
                }
                values
            })
            .collect(),
        "sortedoneterm" => (0..10)
            .map(|doc| {
                let mut values = vec![(0, Dv::Sorted(b"only".as_slice()))];
                if doc % 2 == 0 {
                    values.push((1, Dv::SortedSet(vec![b"only".as_slice()])));
                }
                values
            })
            .collect(),
        "ordblocks" => (0..40_000)
            .map(|doc: i64| {
                let term: &'static [u8] = if doc < 16384 {
                    if doc % 2 == 0 {
                        b"a"
                    } else {
                        b"b"
                    }
                } else {
                    leaked(format!("t{doc:05}"))
                };
                let mut values = vec![(0, Dv::Sorted(term))];
                if doc < 16384 {
                    values.push((1, Dv::SortedSet(vec![b"a".as_slice()])));
                } else {
                    values.push((1, Dv::SortedSet(vec![leaked(format!("t{doc:05}"))])));
                    values.push((1, Dv::SortedSet(vec![leaked(format!("u{doc:05}"))])));
                }
                values
            })
            .collect(),
        "extreme" => (0..34_000)
            .map(|doc: i64| {
                let ex = if doc < 16384 {
                    doc % 2
                } else if doc < 32768 {
                    if doc % 2 == 0 {
                        i64::MIN
                    } else {
                        i64::MAX
                    }
                } else {
                    -1
                };
                let exw = match doc % 3 {
                    0 => i64::MIN,
                    1 => i64::MAX,
                    _ => doc,
                };
                vec![
                    (0, Dv::Num(ex)),
                    (1, Dv::SortedNum(vec![i64::MIN])),
                    (1, Dv::SortedNum(vec![i64::MAX])),
                    (2, Dv::Num(exw)),
                ]
            })
            .collect(),
        "extremeblocks" => (0..34_000)
            .map(|doc: i64| {
                let v = if doc < 16384 {
                    doc % 2
                } else if doc < 32768 {
                    match doc % 3 {
                        0 => i64::MIN,
                        1 => i64::MAX,
                        _ => doc,
                    }
                } else {
                    -1
                };
                vec![(0, Dv::Num(v))]
            })
            .collect(),
        "boundary09" | "boundary095" => (0..32_768)
            .map(|doc: i64| {
                let low = if case == "boundary09" { 8 } else { 9 };
                let v = if doc < 16384 {
                    doc % (1 << low)
                } else {
                    (doc * 7) % 1024
                };
                vec![(0, Dv::Num(v))]
            })
            .collect(),
        other => panic!("unknown case: {other}"),
    }
}

/// `DocValuesFixture.blockValue(int)`: one value range per 16384-value block,
/// the last block constant.
fn block_value(doc: i64) -> i64 {
    if doc < 16384 {
        doc % 2
    } else if doc < 32768 {
        doc * 1_000_003 % 1_099_511_627_776
    } else {
        7
    }
}

/// `DocValuesFixture.sparseBlockValue(int)`, for the `i`-th value of the
/// sparse field (document `2 * i`).
fn sparse_block_value(i: i64) -> i64 {
    if i < 16384 {
        i % 4
    } else {
        i * 7919
    }
}

/// Lengths run from zero upward, so the empty value and short prefixes are
/// both exercised inside one dense field.
fn binary_for(doc: i32) -> &'static [u8] {
    if doc == 4 {
        b""
    } else {
        leaked(format!("bin{doc}payload{doc}"))
    }
}

/// Leaks a string for the lifetime of the process; test-only table data.
fn leaked(text: String) -> &'static [u8] {
    Box::leak(text.into_bytes().into_boxed_slice())
}

// ---------------------------------------------------------------------------
// Rucene write side
// ---------------------------------------------------------------------------

/// Indexes `documents` with Rucene into `out_dir`, under the segment name and
/// id Lucene chose, and returns the field infos of the flushed segment.
fn write_with_rucene(
    out_dir: &Path,
    case: &str,
    segment_name: &str,
    segment_id: [u8; 16],
    documents: &[Vec<(usize, Dv)>],
) -> FieldInfos {
    let codec = ensure_codec();
    let specs = fields(case);

    // The config requires an analyzer, but no field here is tokenized: doc
    // values never consult it.
    let analyzer: Arc<dyn Analyzer> = Arc::new(StandardAnalyzer::new());
    let live = Arc::new(LiveIndexWriterConfig::new(analyzer));
    let directory: Box<dyn Directory> = Box::new(FSDirectory::open(out_dir).expect("directory"));
    let tracking = Arc::new(TrackingDirectoryWrapper::new(directory));
    let make_info = |max_doc: i32| {
        SegmentInfo::new(
            Arc::new(FSDirectory::open(out_dir).expect("directory")),
            Version::LATEST,
            Some(Version::LATEST),
            segment_name.to_string(),
            max_doc,
            false,
            false,
            Arc::clone(&codec),
            HashMap::new(),
            segment_id,
            HashMap::new(),
            Default::default(),
        )
        .expect("segment info")
    };

    let indexing_info = make_info(-1);
    let mut chain = DefaultIndexingChain::new_for_segment(
        Arc::clone(&live),
        Arc::clone(&tracking),
        &indexing_info,
    )
    .expect("bind segment");

    let numbers = Arc::new(FieldNumbers::new(None, None).expect("field numbers"));
    let mut field_infos = FieldInfosBuilder::new(numbers);
    for (doc_id, values) in documents.iter().enumerate() {
        let mut document = Document::new();
        for &(field, ref value) in values {
            let name = specs[field].0;
            match *value {
                Dv::Num(value) => document.add(Box::new(NumericDocValuesField::new(name, value))),
                Dv::Bin(bytes) => document.add(Box::new(BinaryDocValuesField::new(
                    name,
                    BytesRef::new(bytes.to_vec()),
                ))),
                Dv::Sorted(bytes) => document.add(Box::new(SortedDocValuesField::new(
                    name,
                    BytesRef::new(bytes.to_vec()),
                ))),
                Dv::SortedNum(ref values) => {
                    for &value in values {
                        document.add(Box::new(SortedNumericDocValuesField::new(name, value)));
                    }
                }
                Dv::SortedSet(ref terms) => {
                    for bytes in terms {
                        document.add(Box::new(SortedSetDocValuesField::new(
                            name,
                            BytesRef::new(bytes.to_vec()),
                        )));
                    }
                }
            }
        }
        chain
            .process_document(doc_id as i32, &document, true, &mut field_infos)
            .unwrap_or_else(|error| panic!("document {doc_id} must index cleanly: {error}"));
    }
    let finished = field_infos.finish().expect("field infos");

    let segment_info = make_info(documents.len() as i32);
    let info_stream = NoOutputInfoStream;
    let context = flush_io_context(FlushInfo::new(documents.len() as i32, 0));
    let seg_updates = BufferedUpdates::default();
    let mut state = SegmentWriteState::new(
        &info_stream,
        Arc::clone(&tracking) as Arc<dyn Directory>,
        &segment_info,
        &finished,
        &seg_updates,
        context.as_ref(),
    );
    chain.flush(&mut state).expect("flush");
    finished
}

// ---------------------------------------------------------------------------
// The three-way assertion
// ---------------------------------------------------------------------------

/// Runs one case through all three directions of the comparison.
fn assert_case_matches_lucene(case: &str) {
    require_maven();
    let java_tmp = tempfile::tempdir().expect("temp dir");
    let rust_tmp = tempfile::tempdir().expect("temp dir");

    let segment = run_java_fixture(java_tmp.path(), case).expect("java fixture");
    assert!(
        !segment.compound,
        "[{case}] the fixture must write loose files for a byte comparison"
    );
    let documents = documents(case);
    assert_eq!(
        segment.max_doc,
        documents.len() as i32,
        "[{case}] the two sides must index the same number of documents"
    );

    let field_infos =
        write_with_rucene(rust_tmp.path(), case, &segment.name, segment.id, &documents);

    // The field numbers order the `.dvm` entries, so they must agree before
    // the byte comparison can mean anything.
    let rust_field_infos: Vec<String> = field_infos
        .iter()
        .map(|info| {
            format!(
                "{} {} dv={}",
                info.number,
                info.name,
                doc_values_type_name(info.doc_values_type)
            )
        })
        .collect();
    assert_eq!(
        rust_field_infos, segment.field_infos,
        "[{case}] the field infos must agree before the doc values can"
    );

    // 1. Rucene writes what Lucene writes.
    assert_doc_values_bytes_equal(java_tmp.path(), rust_tmp.path(), &segment.name, case);

    // 2. Rucene reads what Lucene wrote.
    assert_eq!(
        read_java_index(java_tmp.path()),
        segment.dump,
        "[{case}] Rucene must decode Lucene's doc values exactly as Lucene does"
    );

    // 3. Lucene reads what Rucene wrote.
    let java_read = read_with_java(
        rust_tmp.path(),
        &segment.name,
        segment.id,
        segment.max_doc,
        &field_infos,
    )
    .expect("lucene reads the rucene segment");
    let expected: Vec<String> = segment
        .dump
        .iter()
        .filter(|line| line.starts_with("dv"))
        .cloned()
        .collect();
    assert_eq!(
        java_read, expected,
        "[{case}] Lucene must decode Rucene's doc values exactly as it decodes its own"
    );
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn numeric_doc_values_match_lucene() {
    // One field per encoding: GCD compression, a repeating value table and a
    // constant stored once in the metadata with no data at all.
    assert_case_matches_lucene("numeric");
}

#[test]
fn sparse_numeric_doc_values_match_lucene() {
    // Few enough present documents that `IndexedDISI` writes its SPARSE
    // block: one short per document.
    assert_case_matches_lucene("sparse");
}

#[test]
fn dense_indexed_disi_blocks_match_lucene() {
    // Ten thousand documents, half of them carrying the field: more than the
    // 4095 entries `IndexedDISI` stores as shorts, so its block switches to
    // the DENSE bitmap-plus-rank encoding.
    assert_case_matches_lucene("dense");
}

#[test]
fn binary_doc_values_match_lucene() {
    // Dense lengths from zero upward (including the empty value) beside a
    // sparse field whose own value at doc 7 is also empty.
    assert_case_matches_lucene("binary");
}

#[test]
fn sorted_doc_values_match_lucene() {
    // First-seen term order ("zz" before "apple") disagrees with sorted
    // order, so a writer that keeps insertion-order ordinals diverges.
    assert_case_matches_lucene("sorted");
}

#[test]
fn sorted_numeric_doc_values_match_lucene() {
    // One to three values per document, duplicates included: a list, not a
    // set.
    assert_case_matches_lucene("sortednumeric");
}

#[test]
fn single_valued_sorted_set_doc_values_match_lucene() {
    // Every document carries exactly one value, so Lucene writes the
    // SORTED-like single-valued route (`isSingleValued`).
    assert_case_matches_lucene("sortedsetsingle");
}

#[test]
fn multi_valued_sorted_set_doc_values_match_lucene() {
    // Genuinely multi-valued, with cross-document repeats: the addresses
    // route.
    assert_case_matches_lucene("sortedsetmulti");
}

#[test]
fn sorted_long_terms_doc_values_match_lucene() {
    // 200 distinct 400-byte terms: four LZ4 blocks of 64 terms each, roughly
    // 25 KB per block. That is past the 16384 bytes Lucene allocates for
    // `termsDictBuffer`, so `maybeGrowBuffer` has to grow it -- the only case
    // in this suite that reaches the growth path, the multi-block path, or an
    // LZ4 block with real redundancy to compress.
    assert_case_matches_lucene("sortedlongterms");
}

#[test]
fn mixed_doc_values_match_lucene() {
    // All five types in one segment, with gaps in several of them.
    assert_case_matches_lucene("mixed");
}

#[test]
fn per_block_numeric_doc_values_match_lucene() {
    // rmp #239. Per-block ranges far apart (0..1, then up to 2^40, then a
    // constant block) make the per-block widths save more than 10% of the
    // packed bits, so Lucene writes the `doBlocks` layout. Rucene must write
    // the same bytes, read Lucene's layout through `VaryingBPVReader`, and
    // write one Lucene reads back -- dense, sparse and sorted-numeric.
    assert_case_matches_lucene("blocks");
}

#[test]
fn single_term_sorted_doc_values_match_lucene() {
    // rmp #239. Every ordinal is 0, so the GCD `writeValues` computes for the
    // ordinals stays 0 and that 0 is what the metadata records; forcing the
    // GCD to 1 for ordinals wrote a different `.dvm`.
    assert_case_matches_lucene("sortedoneterm");
}

#[test]
fn per_block_ordinals_match_lucene() {
    // Round 3 (audit): sorted and sorted-set ordinals that take the per-block
    // layout themselves.
    assert_case_matches_lucene("ordblocks");
}

#[test]
fn extreme_values_match_lucene() {
    // Round 3 (audit): `Long.MIN_VALUE`/`MAX_VALUE` beside small values, where
    // the consumer's `long` arithmetic wraps.
    assert_case_matches_lucene("extreme");
}

#[test]
fn per_block_savings_just_under_the_threshold_match_lucene() {
    assert_case_matches_lucene("boundary09");
}

#[test]
fn per_block_savings_just_over_the_threshold_match_lucene() {
    assert_case_matches_lucene("boundary095");
}

#[test]
fn extreme_values_in_blocks_match_lucene() {
    assert_case_matches_lucene("extremeblocks");
}

// ---------------------------------------------------------------------------
// Corrupt per-block files (round 3)
// ---------------------------------------------------------------------------

/// Writes Lucene's `blocks` segment into a fresh directory and returns it.
fn lucene_blocks_segment() -> tempfile::TempDir {
    require_maven();
    let tmp = tempfile::tempdir().expect("temp dir");
    run_java_fixture(tmp.path(), "blocks").expect("java fixture");
    tmp
}

/// Opens the doc-values producer of the single segment in `dir` without
/// checking file integrity, which is what a reader does on open.
fn open_producer(dir: &Path) -> Result<(Box<dyn DocValuesProducer>, FieldInfos), String> {
    let codec = ensure_codec();
    let directory: Arc<dyn Directory> =
        Arc::new(FSDirectory::open(dir).map_err(|e| e.to_string())?);
    let infos =
        SegmentInfos::read_latest_commit(Arc::clone(&directory)).map_err(|e| e.to_string())?;
    let segment_info = infos.info(0).info.clone();
    let field_infos = codec
        .field_infos_format()
        .read(directory.as_ref(), &segment_info, "", &*DEFAULT_IO_CONTEXT)
        .map_err(|e| e.to_string())?;
    let read_state = SegmentReadState::new(
        directory.as_ref(),
        &segment_info,
        &field_infos,
        &*DEFAULT_IO_CONTEXT,
    );
    let producer = codec
        .doc_values_format()
        .fields_producer(&read_state)
        .map_err(|e| e.to_string())?;
    Ok((producer, field_infos))
}

#[test]
fn a_corrupt_jump_table_entry_is_an_error_not_a_panic() {
    // Round 3 (audit): `VaryingBPVReader` subtracts `valuesOffset` from the
    // jump-table entry in Java's wrapping `long` arithmetic, and the read that
    // follows rejects the position (`Lucene90DocValuesProducer.java:2337-2352`).
    // Entry 2 of `vbpv`'s jump table lies at byte 84079 of Lucene's `.dvd`.
    let tmp = lucene_blocks_segment();
    let dvd = find_doc_values_file(tmp.path(), "_0", "dvd", "blocks");
    let mut bytes = std::fs::read(&dvd).unwrap();
    assert_eq!(
        i64::from_le_bytes(bytes[84079..84087].try_into().unwrap()),
        84054,
        "the jump-table entry the mutation targets"
    );
    bytes[84079..84087].copy_from_slice(&(i64::MIN + 1).to_le_bytes());
    std::fs::write(&dvd, &bytes).unwrap();

    let (producer, field_infos) = open_producer(tmp.path()).unwrap();
    let mut values = producer
        .get_numeric(field_infos.field_info("vbpv").unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(values.advance(33000).unwrap(), 33000);
    assert!(
        values.long_value().is_err(),
        "the corrupt offset must be rejected by the read it feeds"
    );
}

/// Reads every numeric and sorted-numeric value, stopping at the first error.
fn read_every_value(
    producer: &dyn DocValuesProducer,
    field_infos: &FieldInfos,
) -> rucene::error::Result<()> {
    for info in field_infos.iter() {
        match info.doc_values_type {
            DocValuesType::NUMERIC => {
                let mut values = producer.get_numeric(info)?.unwrap();
                while values.next_doc()? != NO_MORE_DOCS {
                    values.long_value()?;
                }
            }
            DocValuesType::SORTED_NUMERIC => {
                let mut values = producer.get_sorted_numeric(info)?.unwrap();
                while values.next_doc()? != NO_MORE_DOCS {
                    for _ in 0..values.doc_value_count()? {
                        values.next_value()?;
                    }
                }
            }
            _ => {}
        }
    }
    Ok(())
}

#[test]
fn random_corruption_of_per_block_values_never_panics() {
    // Round 3 (audit): flips bytes in the value region of Lucene's `blocks`
    // `.dvd` and reads every value; Java answers with an exception or with
    // values, never with a crash, so neither may this port.
    let source = lucene_blocks_segment();
    let mut seed: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut panics = Vec::new();
    for iteration in 0..300 {
        let tmp = tempfile::tempdir().unwrap();
        for entry in std::fs::read_dir(source.path()).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_file() {
                std::fs::copy(entry.path(), tmp.path().join(entry.file_name())).unwrap();
            }
        }
        let dvd = find_doc_values_file(tmp.path(), "_0", "dvd", "blocks");
        let mut bytes = std::fs::read(&dvd).unwrap();
        for _ in 0..1 + iteration % 4 {
            seed = seed
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            let at = 60 + (seed >> 33) as usize % (bytes.len() - 76);
            seed = seed
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            bytes[at] = (seed >> 56) as u8;
        }
        std::fs::write(&dvd, &bytes).unwrap();
        let path = tmp.path().to_path_buf();
        let outcome = std::panic::catch_unwind(move || {
            if let Ok((producer, field_infos)) = open_producer(&path) {
                // An error is a legitimate answer to a corrupt file.
                let _ = read_every_value(producer.as_ref(), &field_infos);
            }
        });
        if outcome.is_err() {
            panics.push(iteration);
        }
    }
    assert!(panics.is_empty(), "iterations that panicked: {panics:?}");
}
