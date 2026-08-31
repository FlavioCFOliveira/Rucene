//! Term-vectors portability tests against Apache Lucene Core 10.5.0.
//!
//! Each test drives the Java reference harness
//! (`tests/fixtures/java-codec-harness`, class `TermVectorsFixture`) to write a
//! single-segment index whose only content is term vectors, and then proves
//! three things about the same content in Rucene:
//!
//! 1. **Rucene writes what Lucene writes.** The same documents are indexed by
//!    Rucene's [`DefaultIndexingChain`] into a segment carrying the *same* name
//!    and the *same* segment id, and the resulting `.tvd`, `.tvx` and `.tvm`
//!    files are compared **byte for byte** with Lucene's.
//! 2. **Rucene reads what Lucene wrote.** The Java directory is opened with
//!    Rucene — its `segments_N`, its `.si` and its `.fnm` — and every document's
//!    term vectors are decoded; the values are compared with the values the Java
//!    harness printed while reading the very same index back with its own
//!    reader.
//! 3. **Lucene reads what Rucene wrote.** The files Rucene produced are opened
//!    by `TermVectorsReaderFixture` with Lucene's own term-vectors reader, and
//!    what Lucene decodes is compared with what Lucene decoded from its own
//!    index.
//!
//! The document scripts are duplicated on both sides as explicit tables of
//! `(term, positionIncrement, startOffset, endOffset, payload)` tuples, in the
//! same order, so that no analyzer takes part: a byte difference can only come
//! from the term-vectors consumer or from the compressing term-vectors codec.
//! The order of the fields inside a document fixes the field numbers, which the
//! term-vector chunks record.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use rucene::analysis::tokenattributes::{
    CharTermAttribute, OffsetAttribute, PackedTokenAttributeImpl, PayloadAttribute,
    PayloadAttributeImpl, PositionIncrementAttribute,
};
use rucene::analysis::{default_token_attribute_factory, Analyzer, StandardAnalyzer, TokenStream};
use rucene::codecs::term_vectors::TermVectorsReader;
use rucene::codecs::{register_codec, Codec, Lucene104Codec};
use rucene::document::{Document, Field, FieldType};
use rucene::index::documents_writer::{IndexingChain, IndexingChainFlushState};
use rucene::index::field_infos::{FieldInfosBuilder, FieldNumbers};
use rucene::index::index_writer_config::LiveIndexWriterConfig;
use rucene::index::indexing_chain::DefaultIndexingChain;
use rucene::index::{FieldInfos, IndexOptions, SegmentInfo, SegmentInfos, POSTINGS_ENUM_ALL};
use rucene::store::{
    flush_io_context, Directory, FSDirectory, FlushInfo, TrackingDirectoryWrapper,
    DEFAULT_IO_CONTEXT,
};
use rucene::util::{AttributeSource, BytesRef, NoOutputInfoStream, Version};

/// The three files the term-vectors format owns.
const TERM_VECTOR_EXTENSIONS: [&str; 3] = ["tvd", "tvx", "tvm"];

// ---------------------------------------------------------------------------
// The document scripts, mirroring TermVectorsFixture
// ---------------------------------------------------------------------------

/// One scripted token; mirrors `IndexingChainFixture.Tok`.
#[derive(Debug, Clone)]
struct Tok {
    term: String,
    pos_incr: i32,
    start: i32,
    end: i32,
    payload: Option<Vec<u8>>,
}

impl Tok {
    fn of(term: &str, pos_incr: i32, start: i32, end: i32) -> Self {
        Self {
            term: term.to_string(),
            pos_incr,
            start,
            end,
            payload: None,
        }
    }

    fn with_payload(term: &str, pos_incr: i32, start: i32, end: i32, payload: Vec<u8>) -> Self {
        Self {
            term: term.to_string(),
            pos_incr,
            start,
            end,
            payload: Some(payload),
        }
    }
}

/// The term-vector settings of one field; mirrors `TermVectorsFixture.Spec`.
#[derive(Debug, Clone)]
struct Spec {
    name: &'static str,
    options: IndexOptions,
    vectors: bool,
    positions: bool,
    offsets: bool,
    payloads: bool,
}

impl Spec {
    fn new(
        name: &'static str,
        options: IndexOptions,
        vectors: bool,
        positions: bool,
        offsets: bool,
        payloads: bool,
    ) -> Self {
        Self {
            name,
            options,
            vectors,
            positions,
            offsets,
            payloads,
        }
    }

    fn field_type(&self) -> FieldType {
        let mut field_type = FieldType::new();
        field_type.set_tokenized(true).expect("tokenized");
        field_type.set_stored(false).expect("stored");
        field_type.set_omit_norms(true).expect("omit norms");
        field_type
            .set_index_options(self.options)
            .expect("index options");
        field_type
            .set_store_term_vectors(self.vectors)
            .expect("store term vectors");
        field_type
            .set_store_term_vector_positions(self.positions)
            .expect("store term vector positions");
        field_type
            .set_store_term_vector_offsets(self.offsets)
            .expect("store term vector offsets");
        field_type
            .set_store_term_vector_payloads(self.payloads)
            .expect("store term vector payloads");
        field_type.freeze();
        field_type
    }
}

/// One value of one field of one document; mirrors `TermVectorsFixture.Val`.
#[derive(Debug, Clone)]
struct Val {
    spec: usize,
    tokens: Vec<Tok>,
}

impl Val {
    fn new(spec: usize, tokens: Vec<Tok>) -> Self {
        Self { spec, tokens }
    }
}

const FULL: IndexOptions = IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS;
const PROX: IndexOptions = IndexOptions::DOCS_AND_FREQS_AND_POSITIONS;

fn specs(case: &str) -> Vec<Spec> {
    match case {
        "basic" | "missing" | "cfs" => vec![
            Spec::new("body", FULL, true, true, true, false),
            Spec::new("title", PROX, true, false, false, false),
            Spec::new("plain", PROX, false, false, false, false),
        ],
        "flags" => vec![
            Spec::new("a_none", PROX, true, false, false, false),
            Spec::new("b_pos", PROX, true, true, false, false),
            Spec::new("c_off", FULL, true, false, true, false),
            Spec::new("d_posoff", FULL, true, true, true, false),
            Spec::new("e_pospay", PROX, true, true, false, true),
            Spec::new("f_all", FULL, true, true, true, true),
        ],
        "payloads" => vec![Spec::new("body", PROX, true, true, false, true)],
        "immense" => vec![
            Spec::new("a", FULL, true, true, true, false),
            Spec::new("b", FULL, true, true, true, true),
        ],
        "multivalue" | "empty" | "chunks" => vec![Spec::new("body", FULL, true, true, true, false)],
        "order" => vec![
            Spec::new("zeta", FULL, true, true, true, false),
            Spec::new("alpha", FULL, true, true, true, false),
            Spec::new("mu", FULL, true, true, true, false),
        ],
        other => panic!("unknown case {other}"),
    }
}

fn documents(case: &str) -> Vec<Vec<Val>> {
    match case {
        "basic" | "cfs" => basic_documents(),
        "flags" => flag_documents(),
        "payloads" => payload_documents(),
        "missing" => missing_documents(),
        "multivalue" => multi_value_documents(),
        "chunks" => chunk_documents(),
        "empty" => empty_documents(),
        "order" => order_documents(),
        "immense" => immense_documents(),
        other => panic!("unknown case {other}"),
    }
}

fn basic_documents() -> Vec<Vec<Val>> {
    vec![
        vec![
            Val::new(
                0,
                vec![
                    Tok::of("alpha", 1, 0, 5),
                    Tok::of("beta", 1, 6, 10),
                    Tok::of("alpha", 1, 11, 16),
                    Tok::of("gamma", 1, 17, 22),
                ],
            ),
            Val::new(
                1,
                vec![Tok::of("lucene", 1, 0, 6), Tok::of("rust", 1, 7, 11)],
            ),
            Val::new(2, vec![Tok::of("ignored", 1, 0, 7)]),
        ],
        vec![Val::new(
            0,
            vec![
                Tok::of("gamma", 1, 0, 5),
                Tok::of("gamma", 0, 0, 5),
                Tok::of("epsilon", 2, 6, 13),
            ],
        )],
        vec![Val::new(2, vec![Tok::of("only", 1, 0, 4)])],
        vec![
            Val::new(1, vec![Tok::of("solo", 1, 0, 4)]),
            Val::new(
                0,
                vec![
                    Tok::of("zeta", 1, 0, 4),
                    Tok::of("zeta", 1, 5, 9),
                    Tok::of("zeta", 1, 10, 14),
                ],
            ),
        ],
    ]
}

fn flag_documents() -> Vec<Vec<Val>> {
    (0..2)
        .map(|doc| {
            (0..6)
                .map(|spec| {
                    Val::new(
                        spec,
                        vec![
                            if doc == 0 {
                                Tok::with_payload("alpha", 1, 0, 5, vec![1, 2, 3])
                            } else {
                                Tok::of("alpha", 1, 0, 5)
                            },
                            Tok::of("beta", 2, 10, 14),
                            Tok::with_payload("alpha", 1, 20, 25, vec![0xFF]),
                        ],
                    )
                })
                .collect()
        })
        .collect()
}

fn payload_documents() -> Vec<Vec<Val>> {
    vec![
        vec![Val::new(
            0,
            vec![
                Tok::with_payload("alpha", 1, 0, 5, vec![1]),
                Tok::of("beta", 1, 6, 10),
                Tok::with_payload("alpha", 1, 11, 16, vec![2, 3, 4]),
            ],
        )],
        vec![Val::new(
            0,
            vec![
                Tok::with_payload("beta", 1, 0, 4, vec![0xFF, 0x00, 0x7F]),
                Tok::with_payload("gamma", 1, 5, 10, Vec::new()),
            ],
        )],
        vec![Val::new(
            0,
            vec![
                Tok::with_payload("alpha", 1, 0, 5, long_payload(40)),
                Tok::with_payload("gamma", 1, 6, 11, long_payload(7)),
            ],
        )],
        vec![Val::new(
            0,
            vec![Tok::of("delta", 1, 0, 5), Tok::of("delta", 1, 6, 11)],
        )],
    ]
}

fn missing_documents() -> Vec<Vec<Val>> {
    (0..5)
        .map(|doc| {
            if doc == 2 {
                vec![Val::new(
                    0,
                    vec![Tok::of("alpha", 1, 0, 5), Tok::of("beta", 1, 6, 10)],
                )]
            } else {
                vec![Val::new(2, vec![Tok::of("plain", 1, 0, 5)])]
            }
        })
        .collect()
}

fn multi_value_documents() -> Vec<Vec<Val>> {
    vec![
        vec![
            Val::new(
                0,
                vec![Tok::of("alpha", 1, 0, 5), Tok::of("beta", 1, 6, 10)],
            ),
            Val::new(
                0,
                vec![Tok::of("alpha", 1, 0, 5), Tok::of("gamma", 1, 6, 11)],
            ),
        ],
        vec![
            Val::new(0, vec![Tok::of("delta", 1, 0, 5)]),
            Val::new(0, vec![Tok::of("delta", 1, 0, 5)]),
            Val::new(0, vec![Tok::of("epsilon", 1, 0, 7)]),
        ],
        vec![
            Val::new(0, vec![Tok::of("beta", 1, 0, 4)]),
            Val::new(0, Vec::new()),
        ],
    ]
}

fn chunk_documents() -> Vec<Vec<Val>> {
    (0..300)
        .map(|doc| {
            let mut tokens = Vec::new();
            let mut offset = 0i32;
            for term in 0..8 {
                let text = format!("term-{doc:04}-{term}-padding-padding");
                let length = text.len() as i32;
                tokens.push(Tok::of(&text, 1, offset, offset + length));
                offset += length + 1;
            }
            vec![Val::new(0, tokens)]
        })
        .collect()
}

fn empty_documents() -> Vec<Vec<Val>> {
    vec![
        vec![Val::new(0, Vec::new())],
        vec![Val::new(0, vec![Tok::of("solo", 1, 0, 4)])],
        Vec::new(),
        vec![Val::new(
            0,
            vec![Tok::of("solo", 1, 0, 4), Tok::of("solo", 1, 5, 9)],
        )],
        vec![Val::new(0, Vec::new())],
    ]
}

fn order_documents() -> Vec<Vec<Val>> {
    vec![
        vec![
            Val::new(0, vec![Tok::of("one", 1, 0, 3)]),
            Val::new(1, vec![Tok::of("two", 1, 0, 3)]),
            Val::new(2, vec![Tok::of("three", 1, 0, 5)]),
        ],
        vec![
            Val::new(2, vec![Tok::of("four", 1, 0, 4)]),
            Val::new(0, vec![Tok::of("five", 1, 0, 4)]),
        ],
    ]
}

/// Three documents, the middle of which has a good field `a` followed by a
/// field `b` whose third token exceeds `MAX_TERM_LENGTH`.
///
/// Mirrors `TermVectorsFixture.immenseDocuments`. The over-long term is a
/// document-level failure — the document is dropped and indexing continues —
/// and what the case proves is that `b` contributes *nothing*: Lucene marks a
/// field as indexed only after `invert` returns normally
/// (`IndexingChain.java:1411-1418`), so neither `b`'s first two tokens nor its
/// payload flag reach the segment.
fn immense_documents() -> Vec<Vec<Val>> {
    let immense: String = (0..40_000)
        .map(|i| char::from(b'a' + (i % 26) as u8))
        .collect();
    vec![
        vec![Val::new(0, vec![Tok::of("alpha", 1, 0, 5)])],
        vec![
            Val::new(0, vec![Tok::of("beta", 1, 0, 4)]),
            Val::new(
                1,
                vec![
                    Tok::with_payload("one", 1, 0, 3, vec![7, 7]),
                    Tok::of("two", 1, 4, 7),
                    Tok::of(&immense, 1, 8, 12),
                ],
            ),
        ],
        vec![Val::new(0, vec![Tok::of("gamma", 1, 0, 5)])],
    ]
}

/// Mirrors `IndexingChainFixture.longPayload`.
fn long_payload(length: usize) -> Vec<u8> {
    (0..length).map(|i| (i * 7 + 1) as u8).collect()
}

// ---------------------------------------------------------------------------
// Scripted token stream
// ---------------------------------------------------------------------------

/// Emits a fixed list of tokens, bypassing analysis completely; mirrors
/// `IndexingChainFixture.ScriptedTokenStream`.
struct ScriptedTokenStream {
    tokens: Vec<Tok>,
    upto: usize,
    final_offset: i32,
    attributes: AttributeSource,
}

impl ScriptedTokenStream {
    fn new(tokens: Vec<Tok>) -> Self {
        let mut attributes = AttributeSource::new_with_factory(default_token_attribute_factory());
        attributes
            .add_attribute::<PackedTokenAttributeImpl>()
            .expect("packed token attribute");
        // The default token-attribute factory does not know how to build a
        // payload attribute, exactly as in Lucene where only analyzers that
        // emit payloads add one; the fixture therefore installs the instance.
        attributes.add_attribute_impl_instance(Box::new(PayloadAttributeImpl::new()));
        Self {
            tokens,
            upto: 0,
            final_offset: 0,
            attributes,
        }
    }
}

impl std::fmt::Debug for ScriptedTokenStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScriptedTokenStream")
            .field("tokens", &self.tokens.len())
            .finish()
    }
}

impl TokenStream for ScriptedTokenStream {
    fn increment_token(&mut self) -> rucene::error::Result<bool> {
        if self.upto == self.tokens.len() {
            return Ok(false);
        }
        self.attributes.clear_attributes();
        let token = self.tokens[self.upto].clone();
        self.upto += 1;
        {
            let mut packed = self
                .attributes
                .get_attribute_mut::<PackedTokenAttributeImpl>()
                .expect("packed attribute");
            packed.append_string(&token.term);
            packed.set_position_increment(token.pos_incr);
            packed.set_offset(token.start, token.end);
        }
        {
            let mut payload = self
                .attributes
                .get_attribute_mut::<PayloadAttributeImpl>()
                .expect("payload attribute");
            payload.set_payload(token.payload.clone().map(BytesRef::new));
        }
        self.final_offset = token.end;
        Ok(true)
    }

    fn reset(&mut self) -> rucene::error::Result<()> {
        self.upto = 0;
        self.final_offset = 0;
        Ok(())
    }

    fn end(&mut self) -> rucene::error::Result<()> {
        self.attributes.end_attributes();
        let mut packed = self
            .attributes
            .get_attribute_mut::<PackedTokenAttributeImpl>()
            .expect("packed attribute");
        packed.set_offset(self.final_offset, self.final_offset);
        Ok(())
    }

    fn attribute_source(&self) -> &AttributeSource {
        &self.attributes
    }

    fn attribute_source_mut(&mut self) -> &mut AttributeSource {
        &mut self.attributes
    }
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
/// would report success while proving nothing.
fn require_maven() {
    if let Err(reason) = which_mvn() {
        panic!("term-vectors portability tests require Maven and a JDK: {reason}");
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
    /// The term vectors Lucene's own reader decoded, one line per term plus one
    /// line per document.
    dump: Vec<String>,
    /// The `.fnm` entry of every field, as Lucene committed it.
    field_infos: Vec<String>,
    /// The message of every document `IndexWriter` refused, in document order.
    rejected: Vec<String>,
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
        .arg("-Dexec.mainClass=org.apache.lucene.rucene.codec.TermVectorsFixture")
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
    let mut rejected = Vec::new();
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
        } else if let Some(value) = line.strip_prefix("rejected=") {
            rejected.push(value.trim_end().to_string());
        } else if line.starts_with("doc ")
            || line.starts_with("docnull ")
            || line.starts_with("tv ")
        {
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
        rejected,
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

/// Reads the term vectors Rucene wrote with the real Lucene reader and returns
/// the lines it decoded.
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
    let fields: Vec<String> = field_infos
        .iter()
        .map(|info| format!("{}:{}", info.name, info.number))
        .collect();
    let output = Command::new(mvn)
        .arg("-q")
        .arg("compile")
        .arg("exec:java")
        .arg("-Dexec.mainClass=org.apache.lucene.rucene.codec.TermVectorsReaderFixture")
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
        .filter(|line| {
            line.starts_with("doc ") || line.starts_with("docnull ") || line.starts_with("tv ")
        })
        .map(|line| line.trim_end().to_string())
        .collect())
}

// ---------------------------------------------------------------------------
// Rucene side
// ---------------------------------------------------------------------------

fn ensure_codec() -> Arc<dyn Codec> {
    let _ = register_codec("Lucene104", Lucene104Codec::new());
    rucene::codecs::default_codec().expect("Lucene104 codec is registered")
}

/// Indexes `documents` with Rucene into `out_dir`, under the segment name and
/// id Lucene chose, and returns the field infos of the flushed segment.
fn write_with_rucene(
    out_dir: &Path,
    case: &str,
    segment_name: &str,
    segment_id: [u8; 16],
    documents: &[Vec<Val>],
) -> RuceneSegment {
    let codec = ensure_codec();
    let specs = specs(case);
    let field_types: Vec<FieldType> = specs.iter().map(Spec::field_type).collect();

    // The analyzer never tokenizes here: every field carries an explicit token
    // stream. It is consulted only for the multi-valued gaps, so the one
    // requirement is that its gaps match the Lucene `WhitespaceAnalyzer` the
    // Java fixture uses, which inherits the `Analyzer` defaults of 0 and 1.
    let analyzer: Arc<dyn Analyzer> = Arc::new(StandardAnalyzer::new());
    for spec in &specs {
        assert_eq!(analyzer.get_position_increment_gap(spec.name), 0);
        assert_eq!(analyzer.get_offset_gap(spec.name), 1);
    }
    let live = Arc::new(LiveIndexWriterConfig::new(Arc::clone(&analyzer)));

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

    // The `DocumentsWriterPerThread` binds the chain while `maxDoc` is still
    // unset and hands `flush` a different `SegmentInfo`.
    let indexing_info = make_info(-1);
    let mut chain = DefaultIndexingChain::new_for_segment(
        Arc::clone(&live),
        Arc::clone(&tracking),
        &indexing_info,
    )
    .expect("bind segment");

    let numbers = Arc::new(FieldNumbers::new(None, None).expect("field numbers"));
    let mut field_infos = FieldInfosBuilder::new(numbers);
    let mut rejected = Vec::new();
    for (doc_id, values) in documents.iter().enumerate() {
        let mut document = Document::new();
        for value in values {
            let stream: Rc<RefCell<dyn TokenStream>> =
                Rc::new(RefCell::new(ScriptedTokenStream::new(value.tokens.clone())));
            document.add(Box::new(
                Field::new_with_token_stream(
                    specs[value.spec].name,
                    stream,
                    field_types[value.spec].clone(),
                )
                .expect("token stream field"),
            ));
        }
        // A document-level failure drops the document and indexing continues,
        // exactly as `IndexWriter.addDocument` does when it catches an
        // `IllegalArgumentException`; the doc id is still consumed.
        if let Err(error) = chain.process_document(doc_id as i32, &document, true, &mut field_infos)
        {
            assert!(
                chain.take_aborting_error().is_none(),
                "document {doc_id} must fail at document level, not abort the segment: {error}"
            );
            // Rucene's `Display` prefixes the error kind, Java's
            // `getMessage()` does not; only the message itself is compared.
            rejected.push(
                error
                    .to_string()
                    .strip_prefix("illegal argument: ")
                    .unwrap_or(&error.to_string())
                    .to_string(),
            );
        }
    }
    let finished = field_infos.finish().expect("field infos");

    let segment_info = make_info(documents.len() as i32);
    let info_stream = NoOutputInfoStream;
    let context = flush_io_context(FlushInfo::new(documents.len() as i32, 0));
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
    RuceneSegment {
        field_infos: finished,
        rejected,
    }
}

/// What indexing a script with Rucene produced.
struct RuceneSegment {
    field_infos: FieldInfos,
    /// The message of every document the chain refused, in document order.
    rejected: Vec<String>,
}

impl RuceneSegment {
    /// Renders the field infos the way `TermVectorsFixture` prints Lucene's.
    fn field_info_lines(&self) -> Vec<String> {
        self.field_infos
            .iter()
            .map(|info| {
                format!(
                    "{} {} vectors={} payloads={}",
                    info.number,
                    info.name,
                    info.has_term_vectors(),
                    info.has_payloads()
                )
            })
            .collect()
    }
}

/// Renders every term vector of every document exactly as
/// `TermVectorsFixture.dump` does, so the two sides compare as plain strings.
fn dump(reader: &dyn TermVectorsReader, max_doc: i32) -> Vec<String> {
    let mut lines = Vec::new();
    for doc_id in 0..max_doc {
        let Some(fields) = reader.get(doc_id).expect("term vectors") else {
            lines.push(format!("docnull {doc_id}"));
            continue;
        };
        let names: Vec<String> = fields.iterator().collect();
        lines.push(format!("doc {doc_id} {}", names.join("|")));
        for name in &names {
            let Some(terms) = fields.terms(name).expect("terms") else {
                continue;
            };
            let has_positions = terms.has_positions();
            let has_offsets = terms.has_offsets();
            let has_payloads = terms.has_payloads();
            let mut iterator = terms.iterator().expect("terms enum");
            while let Some(term) = iterator.next().expect("next term") {
                let freq = iterator.total_term_freq().expect("total term freq") as i32;
                let mut postings = iterator
                    .postings(None, POSTINGS_ENUM_ALL)
                    .expect("postings");
                postings.next_doc().expect("next doc");
                let mut positions = Vec::new();
                let mut offsets = Vec::new();
                let mut payloads = Vec::new();
                if has_positions || has_offsets {
                    for _ in 0..freq {
                        let position = postings.next_position().expect("next position");
                        if has_positions {
                            positions.push(position.to_string());
                        }
                        if has_offsets {
                            offsets.push(format!(
                                "{}:{}",
                                postings.start_offset(),
                                postings.end_offset()
                            ));
                        }
                        if has_payloads {
                            payloads.push(match postings.get_payload().expect("payload") {
                                None => ".".to_string(),
                                Some(bytes) => {
                                    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
                                }
                            });
                        }
                    }
                }
                lines.push(format!(
                    "tv {doc_id} {name} P{} O{} Y{} {} {freq} {} {} {}",
                    usize::from(has_positions),
                    usize::from(has_offsets),
                    usize::from(has_payloads),
                    String::from_utf8(term.slice().to_vec()).expect("utf-8 term"),
                    join(has_positions, &positions),
                    join(has_offsets, &offsets),
                    join(has_payloads, &payloads),
                ));
            }
        }
    }
    lines
}

fn join(present: bool, values: &[String]) -> String {
    if present {
        values.join(";")
    } else {
        "-".to_string()
    }
}

/// Opens a Lucene-written index with Rucene and dumps its term vectors.
fn read_java_index(dir: &Path) -> Vec<String> {
    let codec = ensure_codec();
    let directory: Arc<dyn Directory> = Arc::new(FSDirectory::open(dir).expect("directory"));
    let infos = SegmentInfos::read_latest_commit(directory.as_ref()).expect("segments file");
    let segment_info = infos.info(0).info.clone();
    let field_infos = codec
        .field_infos_format()
        .read(directory.as_ref(), &segment_info, "", &*DEFAULT_IO_CONTEXT)
        .expect("field infos");
    let reader = codec
        .term_vectors_format()
        .vectors_reader(
            directory.as_ref(),
            &segment_info,
            &field_infos,
            &*DEFAULT_IO_CONTEXT,
        )
        .expect("term vectors reader");
    reader.check_integrity().expect("integrity");
    dump(reader.as_ref(), segment_info.max_doc().expect("max doc"))
}

/// Compares the term-vector files of the two directories byte for byte.
fn assert_term_vector_bytes_equal(java_dir: &Path, rust_dir: &Path, segment: &str, case: &str) {
    let mut compared = 0;
    for extension in TERM_VECTOR_EXTENSIONS {
        let file_name = format!("{segment}.{extension}");
        let java_file = java_dir.join(&file_name);
        let rust_file = rust_dir.join(&file_name);
        match (java_file.exists(), rust_file.exists()) {
            (false, false) => continue,
            (true, false) => panic!("[{case}] Rucene did not write {file_name}"),
            (false, true) => panic!("[{case}] Rucene wrote {file_name}, Lucene did not"),
            (true, true) => {}
        }
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
        TERM_VECTOR_EXTENSIONS.len(),
        "[{case}] every term-vector file must be compared"
    );
}

fn hex_window(bytes: &[u8], centre: usize) -> String {
    let from = centre.saturating_sub(16);
    let to = std::cmp::min(centre + 16, bytes.len());
    let body: Vec<String> = bytes[from..to].iter().map(|b| format!("{b:02x}")).collect();
    format!("[{from}..{to}] {}", body.join(" "))
}

/// Runs one case end to end: byte comparison, Rucene reading Lucene's index and
/// Lucene reading Rucene's files.
fn assert_case_matches_lucene(case: &str) {
    require_maven();
    let java_tmp = tempfile::tempdir().expect("temp dir");
    let rust_tmp = tempfile::tempdir().expect("temp dir");

    let segment = run_java_fixture(java_tmp.path(), case).expect("java fixture");
    let documents = documents(case);
    assert_eq!(
        segment.max_doc,
        documents.len() as i32,
        "[{case}] the two document scripts must have the same length"
    );
    assert!(
        !segment.compound,
        "[{case}] the byte comparison needs loose files"
    );

    let rucene = write_with_rucene(rust_tmp.path(), case, &segment.name, segment.id, &documents);

    // 0. The two engines rejected the same documents, for the same reason, and
    //    committed the same field metadata. A document Lucene refuses must not
    //    leave a partial trace behind — neither in the term vectors nor in the
    //    field infos — so this is checked before the bytes are compared, since
    //    it is what explains a difference in them.
    assert_eq!(
        rucene.rejected, segment.rejected,
        "[{case}] the two engines must refuse the same documents with the same message"
    );
    assert_eq!(
        rucene.field_info_lines(),
        segment.field_infos,
        "[{case}] the committed field infos must match Lucene's"
    );

    // 1. Rucene writes what Lucene writes.
    assert_term_vector_bytes_equal(java_tmp.path(), rust_tmp.path(), &segment.name, case);

    // 2. Rucene reads what Lucene wrote.
    assert_eq!(
        read_java_index(java_tmp.path()),
        segment.dump,
        "[{case}] Rucene must decode Lucene's term vectors exactly as Lucene does"
    );

    // 3. Lucene reads what Rucene wrote.
    let decoded = read_with_java(
        rust_tmp.path(),
        &segment.name,
        segment.id,
        segment.max_doc,
        &rucene.field_infos,
    )
    .expect("lucene reads the rucene files");
    assert_eq!(
        decoded, segment.dump,
        "[{case}] Lucene must decode Rucene's term vectors exactly as its own"
    );
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn a_field_with_vectors_beside_one_without_matches_lucene() {
    assert_case_matches_lucene("basic");
}

#[test]
fn every_legal_flag_combination_matches_lucene() {
    assert_case_matches_lucene("flags");
}

#[test]
fn payloads_present_absent_empty_and_long_match_lucene() {
    assert_case_matches_lucene("payloads");
}

#[test]
fn documents_without_vectors_are_filled_the_way_lucene_fills_them() {
    assert_case_matches_lucene("missing");
}

#[test]
fn a_multi_valued_vector_field_matches_lucene() {
    assert_case_matches_lucene("multivalue");
}

#[test]
fn a_stream_spanning_several_chunks_matches_lucene() {
    assert_case_matches_lucene("chunks");
}

#[test]
fn empty_values_and_empty_documents_match_lucene() {
    assert_case_matches_lucene("empty");
}

#[test]
fn fields_are_written_in_the_order_lucene_writes_them() {
    assert_case_matches_lucene("order");
}

#[test]
fn a_field_that_fails_mid_document_contributes_nothing() {
    // A term above `MAX_TERM_LENGTH` is a document-level failure: the document
    // is dropped and indexing continues. Lucene marks a field as indexed only
    // after `invert` returns normally (`IndexingChain.java:1411-1418`), so the
    // failed field's first two tokens never reach the term vectors and its
    // payload flag never reaches the field infos. Marking the field before the
    // call puts both on disk for a document nobody can read, and the `.tvd`,
    // `.tvm` and `.fnm` then differ from Lucene's.
    assert_case_matches_lucene("immense");
}

#[test]
fn rucene_reads_the_term_vectors_of_a_compound_file_segment_written_by_lucene() {
    // The term-vectors reader has to read through the `Directory` it is given —
    // here a compound-file view over `_0.cfs` — because the files of a compound
    // segment exist nowhere else, and a `SegmentInfo` parsed from a `.si`
    // carries only a placeholder directory.
    require_maven();
    let java_tmp = tempfile::tempdir().expect("temp dir");
    let segment = run_java_fixture(java_tmp.path(), "cfs").expect("java fixture");
    assert!(
        segment.compound,
        "the fixture must have bundled the segment into a .cfs"
    );
    assert!(
        !java_tmp
            .path()
            .join(format!("{}.tvd", segment.name))
            .exists(),
        "a compound segment has no loose .tvd to fall back on"
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
        .term_vectors_format()
        .vectors_reader(
            compound.as_ref(),
            &segment_info,
            &field_infos,
            &*DEFAULT_IO_CONTEXT,
        )
        .expect("term vectors reader");
    reader.check_integrity().expect("integrity");

    assert_eq!(
        dump(reader.as_ref(), segment.max_doc),
        segment.dump,
        "Rucene must decode a compound-file segment exactly as Lucene does"
    );
}
