//! Stored-fields portability tests against Apache Lucene Core 10.5.0.
//!
//! Each test drives the Java reference harness
//! (`tests/fixtures/java-codec-harness`, class `StoredFieldsFixture`) to write
//! a single-segment index whose content is made of stored fields, and then
//! proves three things about the same content in Rucene:
//!
//! 1. **Rucene writes what Lucene writes.** The same documents are indexed by
//!    Rucene's [`DefaultIndexingChain`] into a segment carrying the *same*
//!    name and the *same* segment id, and the resulting `.fdt`, `.fdx` and
//!    `.fdm` files are compared **byte for byte** with Lucene's.
//! 2. **Rucene reads what Lucene wrote.** The Java directory is opened with
//!    Rucene — its `segments_N`, its `.si` and its `.fnm` — and every document
//!    is visited; the values Rucene decodes are compared with the values the
//!    Java harness printed while reading the very same index back with its own
//!    `StoredFieldVisitor`.
//! 3. **A visitor can load a subset.** The same Java-written index is read with
//!    a visitor that answers `NO` for every field but one, and with a visitor
//!    that answers `STOP`, and only the expected values arrive.
//!
//! The document scripts are duplicated on both sides as explicit tables of
//! typed values, in the same order, so that no analyzer and no field-numbering
//! heuristic takes part: a byte difference can only come from the stored-fields
//! consumer or from the compressing stored-fields codec.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

use rucene::codecs::lucene104::codec::Mode as CodecMode;
use rucene::codecs::{register_codec, Codec, Lucene104Codec};
use rucene::document::{
    Document, DoubleField, Field, FieldType, FloatField, IntField, KeywordField, LongField,
    NumericValue, Store, StoredField, StringField, TextField,
};
use rucene::index::documents_writer::{IndexingChain, IndexingChainFlushState};
use rucene::index::field_infos::{FieldInfosBuilder, FieldNumbers};
use rucene::index::index_writer_config::LiveIndexWriterConfig;
use rucene::index::indexing_chain::DefaultIndexingChain;
use rucene::index::{
    FieldInfo, FieldInfos, IndexOptions, SegmentInfo, SegmentInfos, StoredFieldVisitor,
    StoredFieldVisitorStatus,
};
use rucene::store::{
    flush_io_context, Directory, FSDirectory, FlushInfo, TrackingDirectoryWrapper,
    DEFAULT_IO_CONTEXT,
};
use rucene::util::{BytesRef, NoOutputInfoStream, Version};

/// The indexed-and-stored field of the `mixed` case; mirrors
/// `StoredFieldsFixture.INDEXED_FIELD`.
const INDEXED_FIELD: &str = "body";

/// The three files the stored-fields format owns.
const STORED_FIELDS_EXTENSIONS: [&str; 3] = ["fdt", "fdx", "fdm"];

// ---------------------------------------------------------------------------
// The document scripts, mirroring StoredFieldsFixture
// ---------------------------------------------------------------------------

/// One value of one field of one document.
#[derive(Debug, Clone, PartialEq)]
enum Value {
    Str(String),
    Int(i32),
    Long(i64),
    Float(f32),
    Double(f64),
    Bytes(Vec<u8>),
    /// An indexed *and* stored text value.
    IndexedText(String),
    /// A `StringField(name, String, Store.YES)`.
    StringKeyword(String),
    /// A `TextField(name, String, Store.YES)`.
    AnalysedText(String),
    /// A `KeywordField(name, String, Store.YES)`.
    KeywordString(String),
    /// A `KeywordField(name, BytesRef, Store.YES)`.
    KeywordBytes(Vec<u8>),
    /// An `IntField(name, int, Store.YES)`.
    IntField(i32),
    /// A `LongField(name, long, Store.YES)`.
    LongField(i64),
    /// A `FloatField(name, float, Store.YES)`.
    FloatField(f32),
    /// A `DoubleField(name, double, Store.YES)`.
    DoubleField(f64),
}

/// One document: an ordered list of `(field name, value)` pairs.
type Doc = Vec<(&'static str, Value)>;

fn documents_in_mode(case: &str, mode: Mode) -> Vec<Doc> {
    match case {
        "strings" => string_documents(),
        "numbers" => number_documents(),
        "binary" => binary_documents(),
        "mixed" => mixed_documents(),
        "empties" => empty_documents(),
        "chunks" => chunk_documents(),
        "sliced" => sliced_documents(mode),
        "redundant" => redundant_documents(),
        "floats" => float_documents(),
        "types" | "cfs" => typed_documents(),
        other => panic!("unknown case: {other}"),
    }
}

fn string_documents() -> Vec<Doc> {
    vec![
        vec![
            ("title", Value::Str("alpha".to_string())),
            ("body", Value::Str("the quick brown fox".to_string())),
        ],
        vec![("title", Value::Str("beta".to_string()))],
        Vec::new(),
        vec![
            ("title", Value::Str(String::new())),
            ("body", Value::Str("ünïcödé ☃ 😀".to_string())),
        ],
        vec![
            ("title", Value::Str("gamma".to_string())),
            ("title", Value::Str("delta".to_string())),
            ("body", Value::Str("epsilon".to_string())),
        ],
    ]
}

fn number_documents() -> Vec<Doc> {
    vec![
        vec![
            ("i", Value::Int(0)),
            ("l", Value::Long(0)),
            ("f", Value::Float(0.0)),
            ("d", Value::Double(0.0)),
        ],
        vec![
            ("i", Value::Int(i32::MAX)),
            ("l", Value::Long(i64::MAX)),
            ("f", Value::Float(f32::MAX)),
            ("d", Value::Double(f64::MAX)),
        ],
        vec![
            ("i", Value::Int(i32::MIN)),
            ("l", Value::Long(i64::MIN)),
            ("f", Value::Float(-0.0)),
            ("d", Value::Double(-0.0)),
        ],
        vec![
            ("i", Value::Int(-1)),
            ("l", Value::Long(86_400_000)),
            ("f", Value::Float(std::f32::consts::PI)),
            ("d", Value::Double(std::f64::consts::E)),
        ],
        vec![
            ("l", Value::Long(1_000)),
            ("l", Value::Long(3_600_000)),
            ("l", Value::Long(1)),
            ("l", Value::Long(4_611_686_018_427_387_904)),
            ("l", Value::Long(-4_611_686_018_427_387_904)),
            ("i", Value::Int(125)),
            ("f", Value::Float(1.0)),
            ("d", Value::Double(1.0)),
        ],
    ]
}

fn binary_documents() -> Vec<Doc> {
    let all_bytes: Vec<u8> = (0..256).map(|byte| byte as u8).collect();
    vec![
        vec![("blob", Value::Bytes(Vec::new()))],
        vec![("blob", Value::Bytes(all_bytes))],
        vec![
            ("blob", Value::Bytes(vec![1, 2, 3])),
            ("blob", Value::Bytes(vec![4, 5])),
        ],
        Vec::new(),
    ]
}

fn mixed_documents() -> Vec<Doc> {
    vec![
        vec![
            (
                INDEXED_FIELD,
                Value::IndexedText("alpha beta gamma".to_string()),
            ),
            ("count", Value::Int(3)),
            ("blob", Value::Bytes(vec![9, 9, 9])),
        ],
        vec![("count", Value::Int(-3))],
        vec![
            (INDEXED_FIELD, Value::IndexedText("delta".to_string())),
            ("ratio", Value::Double(0.125)),
            ("when", Value::Long(1_700_000_000_000)),
        ],
        Vec::new(),
        vec![
            (INDEXED_FIELD, Value::IndexedText("alpha delta".to_string())),
            ("blob", Value::Bytes(vec![0])),
            ("count", Value::Int(1)),
            ("ratio", Value::Float(-1.5)),
        ],
    ]
}

fn empty_documents() -> Vec<Doc> {
    vec![Vec::new(); 7]
}

fn chunk_documents() -> Vec<Doc> {
    (0..1500)
        .map(|index: i32| {
            let mut text = String::new();
            for word in 0..12 {
                text.push_str(&format!("word{} ", (index + word) % 97));
            }
            let mut doc: Doc = vec![("text", Value::Str(text)), ("ord", Value::Int(index))];
            if index % 5 == 0 {
                doc.push((
                    "payload",
                    Value::Bytes(format!("payload-{index}").into_bytes()),
                ));
            }
            doc
        })
        .collect()
}

/// One document large enough to force a *sliced* chunk **in `mode`**.
///
/// Mirrors `StoredFieldsFixture.slicedDocuments(mode)`. The writer slices once
/// the buffered bytes reach twice the chunk size, and the chunk size depends on
/// the mode: 80 KiB for `BEST_SPEED` but 480 KiB for `BEST_COMPRESSION`. A
/// payload sized for the former does **not** slice in the latter, which would
/// leave the deflate preset-dictionary sliced path — the exact shape of the
/// bug this case exists for — untested while the test name claimed otherwise.
fn sliced_documents(mode: Mode) -> Vec<Doc> {
    let target = mode.slicing_payload_bytes();
    let mut huge = String::with_capacity(target + 100);
    let mut index = 0usize;
    while huge.len() < target {
        huge.push_str("chunk");
        huge.push_str(&(index % 1009).to_string());
        huge.push('-');
        huge.push((b'a' + (index % 26) as u8) as char);
        huge.push(' ');
        index += 1;
    }
    let length = huge.chars().count() as i32;
    vec![
        vec![("tag", Value::Str("before".to_string()))],
        vec![("payload", Value::Str(huge)), ("size", Value::Int(length))],
        vec![("tag", Value::Str("after".to_string()))],
    ]
}

/// Every boundary value of the `ZFloat` and `ZDouble` encodings.
///
/// Mirrors `StoredFieldsFixture.floatDocuments()`. The first document holds the
/// two values whose single-byte encoding has header `0x80`, which is the one
/// that underflows a decoder doing `(b & 0x7f) - 1` in unsigned arithmetic.
fn float_documents() -> Vec<Doc> {
    vec![
        vec![("f", Value::Float(-1.0)), ("d", Value::Double(-1.0))],
        [
            0.0f32,
            -0.0f32,
            1.0f32,
            125.0f32,
            126.0f32,
            -2.0f32,
            f32::from_bits(1), // Float.MIN_VALUE, a subnormal
            f32::MIN_POSITIVE, // Float.MIN_NORMAL
        ]
        .into_iter()
        .map(|value| ("f", Value::Float(value)))
        .collect(),
        [
            0.0f64,
            -0.0f64,
            1.0f64,
            124.0f64,
            125.0f64,
            -2.0f64,
            f64::from_bits(1), // Double.MIN_VALUE, a subnormal
            f64::MIN_POSITIVE, // Double.MIN_NORMAL
        ]
        .into_iter()
        .map(|value| ("d", Value::Double(value)))
        .collect(),
        vec![
            ("f", Value::Float(f32::NAN)),
            ("f", Value::Float(f32::INFINITY)),
            ("f", Value::Float(f32::NEG_INFINITY)),
            ("d", Value::Double(f64::NAN)),
            ("d", Value::Double(f64::INFINITY)),
            ("d", Value::Double(f64::NEG_INFINITY)),
        ],
    ]
}

/// Highly redundant prose: the input `BEST_COMPRESSION` exists for.
///
/// Mirrors `StoredFieldsFixture.redundantDocuments()`. `BEST_COMPRESSION` trades
/// speed for ratio, so compressing materially worse than Lucene on this corpus
/// is a regression in the mode's whole purpose — which is exactly what happened
/// while the deflate level was Lucene's 6 on a zlib-ng-derived backend.
fn redundant_documents() -> Vec<Doc> {
    let paragraph = "Apache Lucene is a high-performance, full-featured search engine \
                     library written entirely in Java. ";
    let mut text = String::with_capacity(620_016);
    while text.len() < 620_000 {
        text.push_str(paragraph);
    }
    vec![vec![("prose", Value::Str(text))]]
}

/// One document per stored field class.
///
/// Mirrors `StoredFieldsFixture.typedDocuments()`. Its purpose is the *type
/// byte* each field class writes into the `.fdt` stream: a `KeywordField` built
/// from a `String` must write STRING while one built from bytes writes
/// BYTE_ARR, and so on for every class.
fn typed_documents() -> Vec<Doc> {
    vec![
        vec![
            (
                "s_string",
                Value::StringKeyword("keyword-value".to_string()),
            ),
            (
                "s_text",
                Value::AnalysedText("some analysed text".to_string()),
            ),
            ("s_kw_string", Value::KeywordString("kw-string".to_string())),
            ("s_kw_bytes", Value::KeywordBytes(vec![1, 2, 3])),
            ("s_int", Value::IntField(42)),
            ("s_long", Value::LongField(1_234_567_890_123)),
            ("s_float", Value::FloatField(2.5)),
            ("s_double", Value::DoubleField(-2.5)),
            ("s_bytes", Value::Bytes(vec![0xF0, 0x0D])),
            ("s_only", Value::Str("stored-only".to_string())),
        ],
        vec![
            ("s_string", Value::StringKeyword("second".to_string())),
            ("s_int", Value::IntField(-1)),
        ],
    ]
}

// ---------------------------------------------------------------------------
// Java harness
// ---------------------------------------------------------------------------

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
/// would report success while proving nothing. Matching the other portability
/// suites, a missing toolchain is a hard failure.
fn require_maven() {
    if let Err(reason) = which_mvn() {
        panic!("stored-fields portability tests require Maven and a JDK: {reason}");
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
    /// One entry per document, in doc order: what Lucene's own
    /// `StoredFieldVisitor` saw when reading the index back.
    documents: Vec<Vec<String>>,
}

fn run_java_fixture(out_dir: &Path, case: &str, mode: Mode) -> Result<JavaSegment, String> {
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
        .arg("-Dexec.mainClass=org.apache.lucene.rucene.codec.StoredFieldsFixture")
        .arg(format!(
            "-Dexec.args={} {} {}",
            out_dir.display(),
            case,
            mode.fixture_name()
        ))
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

    let mut name = None;
    let mut id = None;
    let mut max_doc = None;
    let mut compound = None;
    let mut documents: Vec<(i32, Vec<String>)> = Vec::new();
    for line in stdout.lines() {
        if let Some(value) = line.strip_prefix("segment=") {
            name = Some(value.trim().to_string());
        } else if let Some(value) = line.strip_prefix("segment_id=") {
            let raw = value.trim();
            if raw.len() != 32 {
                return Err(format!("unexpected segment id {raw:?}"));
            }
            let mut bytes = [0u8; 16];
            for (index, byte) in bytes.iter_mut().enumerate() {
                *byte = u8::from_str_radix(&raw[index * 2..index * 2 + 2], 16)
                    .map_err(|e| format!("bad segment id {raw:?}: {e}"))?;
            }
            id = Some(bytes);
        } else if let Some(value) = line.strip_prefix("max_doc=") {
            max_doc = Some(
                value
                    .trim()
                    .parse::<i32>()
                    .map_err(|e| format!("bad max_doc: {e}"))?,
            );
        } else if let Some(value) = line.strip_prefix("compound=") {
            compound = Some(value.trim() == "true");
        } else if let Some(value) = line.strip_prefix("doc ") {
            let (doc_id, rest) = value
                .split_once(' ')
                .ok_or_else(|| format!("malformed doc line {line:?}"))?;
            let doc_id = doc_id
                .parse::<i32>()
                .map_err(|e| format!("bad doc id in {line:?}: {e}"))?;
            let fields = if rest.is_empty() {
                Vec::new()
            } else {
                rest.split('|').map(str::to_string).collect()
            };
            documents.push((doc_id, fields));
        }
    }

    documents.sort_by_key(|(doc_id, _)| *doc_id);
    Ok(JavaSegment {
        name: name.ok_or_else(|| format!("harness printed no segment name:\n{stdout}"))?,
        id: id.ok_or_else(|| format!("harness printed no segment id:\n{stdout}"))?,
        max_doc: max_doc.ok_or_else(|| format!("harness printed no max doc:\n{stdout}"))?,
        compound: compound.ok_or_else(|| format!("harness printed no compound flag:\n{stdout}"))?,
        documents: documents.into_iter().map(|(_, fields)| fields).collect(),
    })
}

/// Reads a Rucene-written segment back with the real Lucene stored-fields
/// reader and returns what Lucene decoded, document by document.
///
/// Rucene's indexing chain writes only `.fdt`/`.fdx`/`.fdm`, so Lucene cannot
/// open the directory as an index; `StoredFieldsReaderFixture` rebuilds the
/// segment metadata from these arguments instead, exactly as the Rust side does
/// when it reads a Lucene-written segment.
fn read_with_java(
    dir: &Path,
    segment_name: &str,
    segment_id: [u8; 16],
    max_doc: i32,
    mode: Mode,
    field_infos: &FieldInfos,
) -> Result<Vec<Vec<String>>, String> {
    let _guard = HARNESS_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let mvn = which_mvn()?;
    let id_hex: String = segment_id
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let fields: Vec<String> = field_infos
        .iter()
        .map(|info| format!("{}:{}", info.name, info.number))
        .collect();
    let output = Command::new(mvn)
        .arg("-q")
        .arg("compile")
        .arg("exec:java")
        .arg("-Dexec.mainClass=org.apache.lucene.rucene.codec.StoredFieldsReaderFixture")
        .arg(format!(
            "-Dexec.args={} {} {} {} {} {}",
            dir.display(),
            segment_name,
            id_hex,
            max_doc,
            mode.fixture_name(),
            // A segment can legitimately have no field at all — every document
            // stored nothing — and an empty argument would silently vanish from
            // the Maven command line.
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

    let mut documents: Vec<(i32, Vec<String>)> = Vec::new();
    for line in stdout.lines() {
        if let Some(value) = line.strip_prefix("doc ") {
            let (doc_id, rest) = value
                .split_once(' ')
                .ok_or_else(|| format!("malformed doc line {line:?}"))?;
            let doc_id = doc_id
                .parse::<i32>()
                .map_err(|e| format!("bad doc id in {line:?}: {e}"))?;
            let fields = if rest.is_empty() {
                Vec::new()
            } else {
                rest.split('|').map(str::to_string).collect()
            };
            documents.push((doc_id, fields));
        }
    }
    documents.sort_by_key(|(doc_id, _)| *doc_id);
    Ok(documents.into_iter().map(|(_, fields)| fields).collect())
}

// ---------------------------------------------------------------------------
// Rucene side
// ---------------------------------------------------------------------------

/// The stored-fields mode a case is written in.
///
/// Mirrors `Lucene104Codec.Mode`, whose public constructor
/// `Lucene104Codec(Mode)` makes `BEST_COMPRESSION` a reachable, supported
/// choice rather than an internal detail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    BestSpeed,
    BestCompression,
}

impl Mode {
    fn fixture_name(self) -> &'static str {
        match self {
            Self::BestSpeed => "BEST_SPEED",
            Self::BestCompression => "BEST_COMPRESSION",
        }
    }

    /// Bytes a single stored value must have for its chunk to be *sliced*.
    ///
    /// The writer slices when the buffered bytes reach `2 * chunkSize`, and
    /// `Lucene90StoredFieldsFormat` uses `10 * 8 * 1024` for `BEST_SPEED` and
    /// `10 * 48 * 1024` for `BEST_COMPRESSION`. The margin above the threshold
    /// is deliberate: the chunk also carries the small documents around it.
    fn slicing_payload_bytes(self) -> usize {
        match self {
            Self::BestSpeed => 243_000,
            Self::BestCompression => 1_100_000,
        }
    }

    /// Whether this build can be expected to write byte-identical files.
    ///
    /// `BEST_SPEED` compresses with `org.apache.lucene.util.compress.LZ4`,
    /// which this crate ports directly, so its bytes are reproducible exactly
    /// on every backend.
    ///
    /// `BEST_COMPRESSION` delegates to `java.util.zip.Deflater`, i.e. to
    /// whichever zlib the JVM was linked against. The default backend of this
    /// crate is `zlib-rs`, a port of zlib-ng, whose codewords differ from
    /// zlib's on many inputs — measured 18/80 identical against JDK 21 — so
    /// byte equality is not on the table and the tests prove instead the
    /// property that *is* guaranteed, in both directions: each side reads what
    /// the other wrote.
    ///
    /// Built with `--features zlib-c` the crate links the same C zlib family
    /// the JVM does — measured 80/80 identical — and the byte comparison is
    /// expected to hold, so it is asserted. If a JVM ever ships a zlib-ng-based
    /// `Deflater` this assertion will fail rather than silently weaken, which
    /// is the point: the drift should be visible.
    fn guarantees_identical_bytes(self) -> bool {
        match self {
            Self::BestSpeed => true,
            Self::BestCompression => cfg!(feature = "zlib-c"),
        }
    }

    fn codec(self) -> Arc<dyn Codec> {
        // Registering the default codec is what makes `LiveIndexWriterConfig`
        // constructible at all, so it happens for both modes.
        let default = ensure_codec();
        match self {
            Self::BestSpeed => default,
            // Not registered under a second name: the codec is only ever used
            // directly here, and its SPI name must stay "Lucene104".
            Self::BestCompression => {
                Arc::new(Lucene104Codec::with_mode(CodecMode::BestCompression))
            }
        }
    }
}

fn ensure_codec() -> Arc<dyn Codec> {
    let _ = register_codec("Lucene104", Lucene104Codec::new());
    rucene::codecs::default_codec().expect("Lucene104 codec is registered")
}

/// The field type of `StoredFieldsFixture`'s indexed-and-stored field.
fn indexed_and_stored_type() -> FieldType {
    let mut field_type = FieldType::new();
    field_type.set_stored(true).expect("stored");
    field_type.set_tokenized(true).expect("tokenized");
    field_type.set_omit_norms(true).expect("omit norms");
    field_type
        .set_index_options(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS)
        .expect("index options");
    field_type.freeze();
    field_type
}

fn build_document(script: &Doc) -> Document {
    let mut document = Document::new();
    for (name, value) in script {
        match value {
            Value::Str(text) => document.add(Box::new(
                StoredField::new_string(name, text.clone()).expect("stored string"),
            )),
            Value::Int(number) => document.add(Box::new(
                StoredField::new_number(name, NumericValue::Int(*number)).expect("stored int"),
            )),
            Value::Long(number) => document.add(Box::new(
                StoredField::new_number(name, NumericValue::Long(*number)).expect("stored long"),
            )),
            Value::Float(number) => document.add(Box::new(
                StoredField::new_number(name, NumericValue::Float(*number)).expect("stored float"),
            )),
            Value::Double(number) => document.add(Box::new(
                StoredField::new_number(name, NumericValue::Double(*number))
                    .expect("stored double"),
            )),
            Value::Bytes(bytes) => document.add(Box::new(
                StoredField::new_bytes(name, BytesRef::new(bytes.clone())).expect("stored bytes"),
            )),
            Value::IndexedText(text) => document.add(Box::new(
                Field::new(name, text.clone(), indexed_and_stored_type()).expect("indexed field"),
            )),
            Value::StringKeyword(text) => document.add(Box::new(
                StringField::new(name, text.clone(), Store::YES).expect("string field"),
            )),
            Value::AnalysedText(text) => document.add(Box::new(
                TextField::new(name, text.clone(), Store::YES).expect("text field"),
            )),
            Value::KeywordString(text) => document.add(Box::new(
                KeywordField::new(name, text.clone(), Store::YES).expect("keyword field"),
            )),
            Value::KeywordBytes(bytes) => document.add(Box::new(
                KeywordField::new_with_bytes(name, BytesRef::new(bytes.clone()), Store::YES)
                    .expect("keyword field"),
            )),
            Value::IntField(value) => {
                document.add(Box::new(IntField::new(name, *value, Store::YES)))
            }
            Value::LongField(value) => {
                document.add(Box::new(LongField::new(name, *value, Store::YES)))
            }
            Value::FloatField(value) => {
                document.add(Box::new(FloatField::new(name, *value, Store::YES)))
            }
            Value::DoubleField(value) => {
                document.add(Box::new(DoubleField::new(name, *value, Store::YES)))
            }
        }
    }
    document
}

/// Indexes `scripts` with Rucene's indexing chain into `out_dir`, producing a
/// segment named `segment_name` with segment id `segment_id`.
///
/// Returns the flushed segment and its field metadata, which is what a reader
/// would otherwise recover from `segments_N` and `.fnm`; the indexing chain
/// alone writes neither, because writing them is the `IndexWriter`'s job.
fn write_with_rucene(
    out_dir: &Path,
    segment_name: &str,
    segment_id: [u8; 16],
    scripts: &[Doc],
    mode: Mode,
) -> (SegmentInfo, FieldInfos) {
    let codec = mode.codec();
    let analyzer: Arc<dyn rucene::analysis::Analyzer> =
        Arc::new(rucene::analysis::StandardAnalyzer::new());
    let mut live = LiveIndexWriterConfig::new(analyzer);
    live.set_codec(Arc::clone(&codec));
    let live = Arc::new(live);

    let directory: Box<dyn Directory> = Box::new(FSDirectory::open(out_dir).expect("directory"));
    let tracking = Arc::new(TrackingDirectoryWrapper::new(directory));
    let shared: Arc<dyn Directory> = Arc::clone(&tracking) as Arc<dyn Directory>;

    let make_info = |max_doc: i32| {
        SegmentInfo::new(
            Arc::clone(&shared),
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

    // The DWPT binds the chain while `maxDoc` is still unset, so the fixture
    // does the same.
    let indexing_info = make_info(-1);
    let mut chain =
        DefaultIndexingChain::new_for_segment(live, Arc::clone(&tracking), &indexing_info)
            .expect("bind segment");

    let numbers = Arc::new(FieldNumbers::new(None, None).expect("field numbers"));
    let mut field_infos = FieldInfosBuilder::new(numbers);
    for (doc_id, script) in scripts.iter().enumerate() {
        let document = build_document(script);
        chain
            .process_document(doc_id as i32, &document, true, &mut field_infos)
            .expect("process document");
    }
    let finished = field_infos.finish().expect("field infos");

    let max_doc = scripts.len() as i32;
    let segment_info = make_info(max_doc);
    let info_stream = NoOutputInfoStream;
    let context = flush_io_context(FlushInfo::new(max_doc, 0));
    let state = IndexingChainFlushState {
        info_stream: &info_stream,
        directory: &tracking,
        segment_info: &segment_info,
        field_infos: &finished,
        context: context.as_ref(),
        live_docs: None,
        del_count_on_flush: 0,
        delete_terms: &[],
    };
    chain.flush(&state).expect("flush");
    (segment_info, finished)
}

// ---------------------------------------------------------------------------
// Reading the first chunk header of a `.fdt`
// ---------------------------------------------------------------------------

/// `CodecUtil.CODEC_MAGIC`.
const CODEC_MAGIC: u32 = 0x3fd7_6c17;

/// Reads a Lucene `vInt` at `at`, returning the value and the next offset.
fn read_v_int(bytes: &[u8], at: usize) -> (u32, usize) {
    let mut value = 0u32;
    let mut shift = 0;
    let mut at = at;
    loop {
        let byte = bytes[at];
        at += 1;
        value |= u32::from(byte & 0x7F) << shift;
        if byte & 0x80 == 0 {
            return (value, at);
        }
        shift += 7;
    }
}

/// Returns whether the first chunk of a `.fdt` is *sliced*.
///
/// The chunk header is `vInt docBase` then `vInt token`, whose lowest bit is
/// the sliced flag — `Lucene90CompressingStoredFieldsWriter.writeHeader`. The
/// chunk data begins straight after the codec index header, whose layout is
/// `CODEC_MAGIC`, the codec name as a Lucene string, a big-endian version, a
/// 16-byte segment id, and a length-prefixed segment suffix.
fn first_chunk_is_sliced(fdt: &[u8]) -> bool {
    let magic = u32::from_be_bytes([fdt[0], fdt[1], fdt[2], fdt[3]]);
    assert_eq!(magic, CODEC_MAGIC, "not a codec header");
    let (name_len, at) = read_v_int(fdt, 4);
    let name = std::str::from_utf8(&fdt[at..at + name_len as usize]).expect("codec name");
    assert!(
        name.starts_with("Lucene90StoredFields"),
        "unexpected codec name {name:?}"
    );
    // version (4) + segment id (16), then a one-byte suffix length.
    let at = at + name_len as usize + 4 + 16;
    let suffix_len = fdt[at] as usize;
    let at = at + 1 + suffix_len;
    let (_doc_base, at) = read_v_int(fdt, at);
    let (token, _) = read_v_int(fdt, at);
    token & 1 != 0
}

/// Renders a readable window of `bytes` around `centre`.
fn hex_window(bytes: &[u8], centre: usize) -> String {
    let from = centre.saturating_sub(16);
    let to = std::cmp::min(centre + 16, bytes.len());
    let body: Vec<String> = bytes[from..to].iter().map(|b| format!("{b:02x}")).collect();
    format!("[{from}..{to}] {}", body.join(" "))
}

/// Compares the stored-fields files of the two directories byte for byte.
fn assert_stored_fields_bytes_equal(java_dir: &Path, rust_dir: &Path, segment: &str, case: &str) {
    let mut compared = 0;
    for extension in STORED_FIELDS_EXTENSIONS {
        let file_name = format!("{segment}.{extension}");
        let java_file = java_dir.join(&file_name);
        let rust_file = rust_dir.join(&file_name);
        assert!(
            java_file.exists(),
            "[{case}] Lucene did not write {file_name}"
        );
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
        STORED_FIELDS_EXTENSIONS.len(),
        "[{case}] every stored-fields file must be compared"
    );
}

// ---------------------------------------------------------------------------
// Reading a Java-written index with Rucene
// ---------------------------------------------------------------------------

/// Opens the single segment of a Java-written index with Rucene.
fn open_java_segment(dir: &Path) -> (Arc<dyn Directory>, SegmentInfo, FieldInfos) {
    let codec = ensure_codec();
    let directory: Arc<dyn Directory> = Arc::new(FSDirectory::open(dir).expect("directory"));
    let infos = SegmentInfos::read_latest_commit(directory.as_ref()).expect("segments file");
    assert_eq!(infos.size(), 1, "the fixture writes exactly one segment");
    let segment_info = infos.info(0).info.clone();
    let field_infos = codec
        .field_infos_format()
        .read(directory.as_ref(), &segment_info, "", &*DEFAULT_IO_CONTEXT)
        .expect("field infos");
    (directory, segment_info, field_infos)
}

/// Records what a visitor is handed, in the exact format
/// `StoredFieldsFixture.RecordingVisitor` prints.
#[derive(Debug, Default)]
struct RecordingVisitor {
    wanted: Option<HashSet<String>>,
    stop_at: Option<String>,
    seen: Vec<String>,
}

/// Values at or below this many bytes are rendered verbatim; longer ones are
/// reduced to a length and a digest. Mirrors
/// `StoredFieldsFixture.RecordingVisitor.DIGEST_THRESHOLD`.
///
/// Above the threshold the digest, not a byte-for-byte comparison, is what
/// checks the value — deliberately, so that a 243 KB stored field does not turn
/// the harness transcript into megabytes. For `BEST_SPEED` the whole `.fdt` is
/// compared byte for byte anyway, so nothing is lost; for `BEST_COMPRESSION` on
/// the default backend the digest is the only value-level check, and a
/// 64-bit digest over the exact length is a strong one.
const DIGEST_THRESHOLD: usize = 64;

/// FNV-1a, 64-bit, over the raw bytes.
///
/// Chosen because both the Java harness and this test can compute it from the
/// specification with no shared library, so the digests are comparable.
fn fnv1a64(value: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in value {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// Either `:<hex bytes>` or `#<length>:<fnv1a64>`.
fn render_value(value: &[u8]) -> String {
    if value.len() <= DIGEST_THRESHOLD {
        let mut rendered = String::with_capacity(1 + value.len() * 2);
        rendered.push(':');
        for byte in value {
            rendered.push_str(&format!("{byte:02x}"));
        }
        rendered
    } else {
        format!("#{}:{:016x}", value.len(), fnv1a64(value))
    }
}

impl StoredFieldVisitor for RecordingVisitor {
    fn binary_field(&mut self, info: &FieldInfo, value: &[u8]) -> rucene::error::Result<()> {
        self.seen
            .push(format!("{}=bin{}", info.name, render_value(value)));
        Ok(())
    }

    fn string_field(&mut self, info: &FieldInfo, value: &str) -> rucene::error::Result<()> {
        self.seen.push(format!(
            "{}=str{}",
            info.name,
            render_value(value.as_bytes())
        ));
        Ok(())
    }

    fn int_field(&mut self, info: &FieldInfo, value: i32) -> rucene::error::Result<()> {
        self.seen.push(format!("{}=i32:{value}", info.name));
        Ok(())
    }

    fn long_field(&mut self, info: &FieldInfo, value: i64) -> rucene::error::Result<()> {
        self.seen.push(format!("{}=i64:{value}", info.name));
        Ok(())
    }

    fn float_field(&mut self, info: &FieldInfo, value: f32) -> rucene::error::Result<()> {
        // `Integer.toHexString` prints without leading zeros.
        self.seen
            .push(format!("{}=f32:{:x}", info.name, value.to_bits()));
        Ok(())
    }

    fn double_field(&mut self, info: &FieldInfo, value: f64) -> rucene::error::Result<()> {
        self.seen
            .push(format!("{}=f64:{:x}", info.name, value.to_bits()));
        Ok(())
    }

    fn needs_field(&mut self, info: &FieldInfo) -> rucene::error::Result<StoredFieldVisitorStatus> {
        if self.stop_at.as_deref() == Some(info.name.as_str()) {
            return Ok(StoredFieldVisitorStatus::Stop);
        }
        match &self.wanted {
            None => Ok(StoredFieldVisitorStatus::Yes),
            Some(names) if names.contains(&info.name) => Ok(StoredFieldVisitorStatus::Yes),
            Some(_) => Ok(StoredFieldVisitorStatus::No),
        }
    }
}

/// Visits every document of a Java-written index with `visitor_for`.
fn read_java_index(dir: &Path, visitor_for: impl FnMut() -> RecordingVisitor) -> Vec<Vec<String>> {
    let (_directory, segment_info, field_infos) = open_java_segment(dir);
    read_segment(dir, &segment_info, &field_infos, visitor_for)
}

/// Visits every document of `segment_info`, reading from `dir`.
fn read_segment(
    dir: &Path,
    segment_info: &SegmentInfo,
    field_infos: &FieldInfos,
    mut visitor_for: impl FnMut() -> RecordingVisitor,
) -> Vec<Vec<String>> {
    let directory: Arc<dyn Directory> = Arc::new(FSDirectory::open(dir).expect("directory"));
    // `Lucene90StoredFieldsFormat` reads the compression mode back out of the
    // segment attributes, so the reader side needs no mode of its own.
    let reader = ensure_codec()
        .stored_fields_format()
        .fields_reader(
            directory.as_ref(),
            segment_info,
            field_infos,
            &*DEFAULT_IO_CONTEXT,
        )
        .expect("stored fields reader");
    reader.check_integrity().expect("integrity");
    (0..segment_info.max_doc().expect("max doc"))
        .map(|doc_id| {
            let mut visitor = visitor_for();
            reader.document(doc_id, &mut visitor).expect("document");
            visitor.seen
        })
        .collect()
}

// ---------------------------------------------------------------------------
// The three assertions, per case
// ---------------------------------------------------------------------------

/// Runs one case end to end: byte comparison plus read-back comparison.
fn assert_case_matches_lucene(case: &str) {
    assert_case_matches_lucene_in_mode(case, Mode::BestSpeed);
}

fn assert_case_matches_lucene_in_mode(case: &str, mode: Mode) {
    require_maven();
    let java_tmp = tempfile::tempdir().expect("temp dir");
    let rust_tmp = tempfile::tempdir().expect("temp dir");

    let segment = run_java_fixture(java_tmp.path(), case, mode).expect("java fixture");
    assert!(
        !segment.compound,
        "[{case}] the byte comparison needs loose segment files"
    );
    let scripts = documents_in_mode(case, mode);
    assert_eq!(
        segment.max_doc,
        scripts.len() as i32,
        "[{case}] the two document scripts must have the same length"
    );

    let (rust_segment, rust_field_infos) =
        write_with_rucene(rust_tmp.path(), &segment.name, segment.id, &scripts, mode);

    let java_fdt = std::fs::read(java_tmp.path().join(format!("{}.fdt", segment.name)))
        .expect("read the lucene .fdt");
    let rust_fdt = std::fs::read(rust_tmp.path().join(format!("{}.fdt", segment.name)))
        .expect("read the rucene .fdt");

    if case == "sliced" {
        // The name of this case is a claim about the code path it reaches, and
        // the threshold is mode-dependent, so it is checked rather than assumed.
        assert!(
            first_chunk_is_sliced(&java_fdt),
            "[{case}] the Lucene payload did not slice its first chunk in {mode:?}"
        );
        assert!(
            first_chunk_is_sliced(&rust_fdt),
            "[{case}] the Rucene payload did not slice its first chunk in {mode:?}"
        );
    }

    if mode.guarantees_identical_bytes() {
        assert_stored_fields_bytes_equal(java_tmp.path(), rust_tmp.path(), &segment.name, case);
    } else {
        // Byte equality is not on the table for this backend, but compressing
        // materially worse than Lucene is a regression in the one thing this
        // mode exists for, so the ratio is guarded instead.
        assert!(
            rust_fdt.len() * 100 <= java_fdt.len() * 110,
            "[{case}] Rucene's BEST_COMPRESSION .fdt is {} bytes against Lucene's {}, \
             more than 10% worse",
            rust_fdt.len(),
            java_fdt.len()
        );
    }

    // Rucene must decode the index Lucene wrote into the same values Lucene
    // itself decoded from it.
    let decoded = read_java_index(java_tmp.path(), RecordingVisitor::default);
    assert_eq!(
        decoded, segment.documents,
        "[{case}] Rucene decodes the Lucene-written stored fields differently"
    );

    // The index Rucene wrote must decode to the same values too, which is what
    // makes the byte comparison meaningful rather than accidental. The indexing
    // chain writes no `segments_N` and no `.fnm` — that is the `IndexWriter`'s
    // job — so the segment metadata comes back from the writer rather than from
    // the directory.
    let round_tripped = read_segment(
        rust_tmp.path(),
        &rust_segment,
        &rust_field_infos,
        RecordingVisitor::default,
    );
    assert_eq!(
        round_tripped, segment.documents,
        "[{case}] the segment Rucene wrote does not decode to the Lucene values"
    );

    // And finally the direction that byte equality would otherwise stand in
    // for: real Lucene 10.5.0 reading the bytes Rucene produced.
    let java_read = read_with_java(
        rust_tmp.path(),
        &segment.name,
        segment.id,
        segment.max_doc,
        mode,
        &rust_field_infos,
    )
    .expect("lucene reads the rucene-written segment");
    assert_eq!(
        java_read, segment.documents,
        "[{case}] Lucene decodes the Rucene-written stored fields differently"
    );
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn stored_strings_match_lucene() {
    assert_case_matches_lucene("strings");
}

#[test]
fn stored_numbers_match_lucene() {
    assert_case_matches_lucene("numbers");
}

#[test]
fn stored_binary_values_match_lucene() {
    assert_case_matches_lucene("binary");
}

#[test]
fn stored_and_indexed_fields_match_lucene() {
    assert_case_matches_lucene("mixed");
}

#[test]
fn a_segment_of_empty_documents_matches_lucene() {
    assert_case_matches_lucene("empties");
}

#[test]
fn a_stored_fields_stream_spanning_several_chunks_matches_lucene() {
    assert_case_matches_lucene("chunks");
}

#[test]
fn a_sliced_chunk_matches_lucene() {
    assert_case_matches_lucene("sliced");
}

#[test]
fn every_float_and_double_boundary_matches_lucene() {
    assert_case_matches_lucene("floats");
}

#[test]
fn every_stored_field_class_writes_lucenes_type_byte() {
    assert_case_matches_lucene("types");
}

// -- BEST_COMPRESSION ------------------------------------------------------

#[test]
fn stored_strings_match_lucene_in_best_compression() {
    assert_case_matches_lucene_in_mode("strings", Mode::BestCompression);
}

#[test]
fn stored_numbers_match_lucene_in_best_compression() {
    assert_case_matches_lucene_in_mode("numbers", Mode::BestCompression);
}

#[test]
fn stored_binary_values_match_lucene_in_best_compression() {
    assert_case_matches_lucene_in_mode("binary", Mode::BestCompression);
}

#[test]
fn a_segment_of_empty_documents_matches_lucene_in_best_compression() {
    assert_case_matches_lucene_in_mode("empties", Mode::BestCompression);
}

#[test]
fn a_stored_fields_stream_spanning_several_chunks_matches_lucene_in_best_compression() {
    assert_case_matches_lucene_in_mode("chunks", Mode::BestCompression);
}

#[test]
fn a_sliced_chunk_matches_lucene_in_best_compression() {
    assert_case_matches_lucene_in_mode("sliced", Mode::BestCompression);
}

#[test]
fn redundant_text_compresses_as_well_as_lucene_in_best_compression() {
    assert_case_matches_lucene_in_mode("redundant", Mode::BestCompression);
}

#[test]
fn redundant_text_matches_lucene_in_best_speed() {
    assert_case_matches_lucene_in_mode("redundant", Mode::BestSpeed);
}

#[test]
fn every_stored_field_class_writes_lucenes_type_byte_in_best_compression() {
    assert_case_matches_lucene_in_mode("types", Mode::BestCompression);
}

#[test]
fn rucene_reads_the_stored_fields_of_a_compound_file_segment_written_by_lucene() {
    // The stored-fields reader has to read through the `Directory` it is given
    // — here a compound-file view over `_0.cfs` — because the files of a
    // compound segment exist nowhere else, and a `SegmentInfo` parsed from a
    // `.si` carries only a placeholder directory.
    require_maven();
    let java_tmp = tempfile::tempdir().expect("temp dir");
    let segment = run_java_fixture(java_tmp.path(), "cfs", Mode::BestSpeed).expect("java fixture");
    assert!(
        segment.compound,
        "the fixture must have bundled the segment into a .cfs"
    );
    assert!(
        !java_tmp
            .path()
            .join(format!("{}.fdt", segment.name))
            .exists(),
        "a compound segment has no loose .fdt to fall back on"
    );

    let codec = ensure_codec();
    let directory: Arc<dyn Directory> =
        Arc::new(FSDirectory::open(java_tmp.path()).expect("directory"));
    let infos = SegmentInfos::read_latest_commit(directory.as_ref()).expect("segments file");
    let segment_info = infos.info(0).info.clone();
    let compound = codec
        .compound_format()
        .get_compound_reader(directory.as_ref(), &segment_info)
        .expect("compound reader");
    let field_infos = codec
        .field_infos_format()
        .read(compound.as_ref(), &segment_info, "", &*DEFAULT_IO_CONTEXT)
        .expect("field infos");
    let reader = codec
        .stored_fields_format()
        .fields_reader(
            compound.as_ref(),
            &segment_info,
            &field_infos,
            &*DEFAULT_IO_CONTEXT,
        )
        .expect("stored fields reader");
    reader.check_integrity().expect("integrity");

    let decoded: Vec<Vec<String>> = (0..segment_info.max_doc().expect("max doc"))
        .map(|doc_id| {
            let mut visitor = RecordingVisitor::default();
            reader.document(doc_id, &mut visitor).expect("document");
            visitor.seen
        })
        .collect();
    assert_eq!(
        decoded, segment.documents,
        "Rucene must decode a compound-file segment exactly as Lucene does"
    );
}

#[test]
fn a_visitor_loads_a_subset_of_a_lucene_written_index() {
    require_maven();
    let java_tmp = tempfile::tempdir().expect("temp dir");
    let segment =
        run_java_fixture(java_tmp.path(), "strings", Mode::BestSpeed).expect("java fixture");
    assert!(segment.max_doc > 0);

    let only_title: HashSet<String> = ["title".to_string()].into_iter().collect();
    let decoded = read_java_index(java_tmp.path(), || RecordingVisitor {
        wanted: Some(only_title.clone()),
        ..Default::default()
    });

    // Exactly the `title` values of the full read-back, and nothing else.
    let expected: Vec<Vec<String>> = segment
        .documents
        .iter()
        .map(|fields| {
            fields
                .iter()
                .filter(|value| value.starts_with("title="))
                .cloned()
                .collect()
        })
        .collect();
    assert_eq!(decoded, expected);
    assert!(
        expected.iter().any(|fields| !fields.is_empty()),
        "the subset must not be empty, or the test would prove nothing"
    );
    assert!(
        segment
            .documents
            .iter()
            .any(|fields| fields.iter().any(|value| value.starts_with("body="))),
        "the index must contain a field the visitor rejects, or NO is never exercised"
    );
}

#[test]
fn a_visitor_can_stop_reading_a_lucene_written_index() {
    require_maven();
    let java_tmp = tempfile::tempdir().expect("temp dir");
    let segment =
        run_java_fixture(java_tmp.path(), "strings", Mode::BestSpeed).expect("java fixture");

    // `body` is always the last field of a document that has one, so stopping
    // at it keeps everything before it and drops it and anything after.
    let decoded = read_java_index(java_tmp.path(), || RecordingVisitor {
        stop_at: Some("body".to_string()),
        ..Default::default()
    });
    let expected: Vec<Vec<String>> = segment
        .documents
        .iter()
        .map(|fields| {
            fields
                .iter()
                .take_while(|value| !value.starts_with("body="))
                .cloned()
                .collect()
        })
        .collect();
    assert_eq!(decoded, expected);
    assert!(
        segment
            .documents
            .iter()
            .any(|fields| fields.iter().any(|value| value.starts_with("body="))),
        "STOP must actually cut something off, or the test would prove nothing"
    );
}
