//! Norms portability tests against Apache Lucene Core 10.5.0.
//!
//! Each test drives the Java reference harness
//! (`tests/fixtures/java-codec-harness`, class `NormsFixture`) to write a
//! single-segment index, and then proves three things about the same content in
//! Rucene:
//!
//! 1. **Rucene writes what Lucene writes.** The same documents are indexed by
//!    Rucene's [`DefaultIndexingChain`] into a segment carrying the *same* name
//!    and the *same* segment id, and the resulting `.nvd` and `.nvm` files are
//!    compared **byte for byte** with Lucene's.
//! 2. **Rucene reads what Lucene wrote.** The Java directory is opened with
//!    Rucene — its `segments_N`, its `.si` and its `.fnm` — and every norm is
//!    decoded; the values are compared with the values the Java harness printed
//!    while reading the very same index back with its own reader.
//! 3. **Lucene reads what Rucene wrote.** The files Rucene produced are opened
//!    by `NormsReaderFixture` with Lucene's own norms reader, and what Lucene
//!    decodes is compared with what Lucene decoded from its own index.
//!
//! Direction 2 is the one that a self-consistent pair of a wrong writer and a
//! wrong reader would pass silently, so it is run for every case rather than as
//! a spot check.
//!
//! The document scripts are duplicated on both sides as explicit tables of
//! `(term, positionIncrement, startOffset, endOffset)` tuples, in the same
//! order, so that no analyzer takes part: a byte difference can only come from
//! `NormValuesWriter`, from `Similarity::compute_norm` or from the norms codec.
//! The order of the fields inside a document fixes the field numbers, which
//! order the `.nvm` entries.
//!
//! # Shapes covered
//!
//! The cases span the *shape* of the format, not only its values: the
//! all-documents case that writes no docs-with-field stream at all, the three
//! `IndexedDISI` block encodings (SPARSE, DENSE, ALL), a field that omits norms
//! beside one that does not, a segment where every field omits norms and no
//! file is written, single- and multi-valued fields, a field present with no
//! tokens, a constant field whose value lives in the metadata, and — through
//! custom similarities on both sides — the two-, four- and eight-byte value
//! widths that Lucene's own `computeNorm` can never reach because it always
//! returns a signed byte.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use rucene::analysis::tokenattributes::{
    CharTermAttribute, OffsetAttribute, PackedTokenAttributeImpl, PositionIncrementAttribute,
};
use rucene::analysis::{default_token_attribute_factory, Analyzer, StandardAnalyzer, TokenStream};
use rucene::codecs::norms::NormsProducer;
use rucene::codecs::{register_codec, Codec, Lucene104Codec};
use rucene::document::{Document, Field, FieldType};
use rucene::index::documents_writer::{IndexingChain, IndexingChainFlushState};
use rucene::index::field_infos::{FieldInfosBuilder, FieldNumbers};
use rucene::index::index_writer_config::LiveIndexWriterConfig;
use rucene::index::indexing_chain::{DefaultIndexingChain, FieldInvertState};
use rucene::index::{FieldInfos, IndexOptions, SegmentInfo, SegmentInfos};
use rucene::search::similarities::{compute_default_norm, BM25Similarity, Similarity};
use rucene::search::{DocIdSetIterator, NO_MORE_DOCS};
use rucene::store::{
    flush_io_context, Directory, FSDirectory, FlushInfo, TrackingDirectoryWrapper,
    DEFAULT_IO_CONTEXT,
};
use rucene::util::{AttributeSource, NoOutputInfoStream, Version};

/// The two files the norms format owns.
const NORMS_EXTENSIONS: [&str; 2] = ["nvd", "nvm"];

// ---------------------------------------------------------------------------
// The document scripts, mirroring NormsFixture
// ---------------------------------------------------------------------------

/// One scripted token; mirrors `IndexingChainFixture.Tok` without payloads,
/// which norms never see.
#[derive(Debug, Clone)]
struct Tok {
    term: String,
    pos_incr: i32,
    start: i32,
    end: i32,
}

impl Tok {
    fn of(term: &str, pos_incr: i32, start: i32, end: i32) -> Self {
        Self {
            term: term.to_string(),
            pos_incr,
            start,
            end,
        }
    }
}

/// The settings of one field; mirrors `NormsFixture.Spec`.
#[derive(Debug, Clone)]
struct Spec {
    name: &'static str,
    options: IndexOptions,
    omit_norms: bool,
}

impl Spec {
    fn new(name: &'static str, options: IndexOptions, omit_norms: bool) -> Self {
        Self {
            name,
            options,
            omit_norms,
        }
    }

    fn field_type(&self) -> FieldType {
        let mut field_type = FieldType::new();
        field_type.set_tokenized(true).expect("tokenized");
        field_type.set_stored(false).expect("stored");
        field_type
            .set_omit_norms(self.omit_norms)
            .expect("omit norms");
        field_type
            .set_index_options(self.options)
            .expect("index options");
        field_type
            .set_store_term_vectors(false)
            .expect("store term vectors");
        field_type.freeze();
        field_type
    }
}

/// One value of one field of one document; mirrors `NormsFixture.Val`.
#[derive(Debug, Clone)]
struct Val {
    spec: usize,
    tokens: Vec<Tok>,
}

const PROX: IndexOptions = IndexOptions::DOCS_AND_FREQS_AND_POSITIONS;
const FREQS: IndexOptions = IndexOptions::DOCS_AND_FREQS;
const DOCS: IndexOptions = IndexOptions::DOCS;

/// Mirrors `NormsFixture.specs`.
fn specs(case: &str) -> Vec<Spec> {
    match case {
        "omitnorms" => vec![
            Spec::new("body", PROX, true),
            Spec::new("title", FREQS, true),
        ],
        "mixedomit" => vec![
            Spec::new("body", PROX, false),
            Spec::new("skipped", FREQS, true),
            Spec::new("title", FREQS, false),
        ],
        "docsonly" => vec![
            Spec::new("body", DOCS, false),
            Spec::new("title", FREQS, false),
        ],
        _ => vec![
            Spec::new("body", PROX, false),
            Spec::new("title", FREQS, false),
        ],
    }
}

/// Mirrors `NormsFixture.similarity`.
fn similarity(case: &str) -> Arc<dyn Similarity> {
    match case {
        "nodiscount" => Arc::new(BM25Similarity::with_discount_overlaps(false)),
        "wide2" => Arc::new(ScaledSimilarity { factor: 300 }),
        "wide4" => Arc::new(ScaledSimilarity { factor: 1_000_000 }),
        "wide8" => Arc::new(ScaledSimilarity {
            factor: 1_000_000_000_000,
        }),
        _ => Arc::new(BM25Similarity::new()),
    }
}

/// Mirrors `NormsFixture.ScaledSimilarity`: the norm is the field length times
/// a fixed factor, which is how the two-, four- and eight-byte widths of the
/// format are reached. Lucene's own `computeNorm` never produces them because
/// it always returns a signed byte.
#[derive(Debug)]
struct ScaledSimilarity {
    factor: i64,
}

impl Similarity for ScaledSimilarity {
    fn compute_norm(&self, state: &FieldInvertState) -> rucene::error::Result<i64> {
        Ok(state.length() as i64 * self.factor)
    }
}

/// A value of `spec` with `count` distinct terms; mirrors `NormsFixture.words`.
fn words(spec: usize, prefix: &str, count: i32) -> Val {
    let mut tokens = Vec::new();
    for i in 0..count {
        let term = format!("{prefix}{i}");
        let len = term.len() as i32;
        tokens.push(Tok::of(&term, 1, i * 6, i * 6 + len));
    }
    Val { spec, tokens }
}

/// Mirrors `NormsFixture.documents`.
fn documents(case: &str) -> Vec<Vec<Val>> {
    let mut documents = Vec::new();
    match case {
        "dense" | "cfs" | "wide2" | "wide4" | "wide8" => {
            for doc in 0..12 {
                documents.push(vec![
                    words(0, "a", 1 + doc * 3),
                    words(1, "b", 1 + (doc % 4)),
                ]);
            }
        }
        "sparse" => {
            for doc in 0..40 {
                let mut values = Vec::new();
                if doc % 3 == 0 {
                    values.push(words(0, "a", 1 + doc));
                }
                values.push(words(1, "b", 1 + (doc % 7)));
                documents.push(values);
            }
        }
        "disidense" => {
            for doc in 0..10_000 {
                let mut values = Vec::new();
                if doc % 2 == 0 {
                    values.push(words(0, "a", 1 + (doc % 11)));
                }
                values.push(words(1, "b", 1 + (doc % 5)));
                documents.push(values);
            }
        }
        "disiall" => {
            for doc in 0..(65_536 + 64) {
                let mut values = Vec::new();
                if doc < 65_536 {
                    values.push(words(0, "a", 1 + (doc % 3)));
                }
                values.push(words(1, "b", 1 + (doc % 2)));
                documents.push(values);
            }
        }
        "omitnorms" | "mixedomit" => {
            for doc in 0..8 {
                let mut values = vec![words(0, "a", 1 + doc), words(1, "b", 1 + (doc % 3))];
                if case == "mixedomit" {
                    values.push(words(2, "c", 1 + (doc % 5)));
                }
                documents.push(values);
            }
        }
        "docsonly" => {
            for doc in 0..10 {
                let mut repeated = Vec::new();
                for i in 0..(1 + doc) {
                    let term = format!("a{}", i % 3);
                    repeated.push(Tok::of(&term, 1, i * 4, i * 4 + 2));
                }
                documents.push(vec![
                    Val {
                        spec: 0,
                        tokens: repeated,
                    },
                    words(1, "b", 1 + doc),
                ]);
            }
        }
        "overlaps" | "nodiscount" => {
            for doc in 0..10 {
                let mut tokens = Vec::new();
                let mut offset = 0;
                for i in 0..(1 + doc) {
                    tokens.push(Tok::of(&format!("a{i}"), 1, offset, offset + 2));
                    tokens.push(Tok::of(&format!("syn{i}"), 0, offset, offset + 2));
                    offset += 4;
                }
                documents.push(vec![Val { spec: 0, tokens }, words(1, "b", 1 + doc)]);
            }
        }
        "multivalue" => {
            for doc in 0..10 {
                documents.push(vec![
                    words(0, "a", 1 + doc),
                    words(0, "b", 2),
                    words(0, "c", 1 + (doc % 3)),
                    words(1, "d", 1 + (doc % 4)),
                ]);
            }
        }
        "emptyvalue" => {
            for doc in 0..10 {
                let mut values = Vec::new();
                if doc % 3 == 2 {
                    values.push(Val {
                        spec: 0,
                        tokens: Vec::new(),
                    });
                } else if doc % 3 == 1 {
                    values.push(words(0, "a", 1 + doc));
                }
                values.push(words(1, "b", 1 + (doc % 4)));
                documents.push(values);
            }
        }
        "constant" => {
            for _ in 0..10 {
                documents.push(vec![words(0, "a", 4), words(1, "b", 4)]);
            }
        }
        other => panic!("unknown case {other}"),
    }
    documents
}

// ---------------------------------------------------------------------------
// The scripted token stream
// ---------------------------------------------------------------------------

/// Emits a fixed list of tokens, bypassing analysis completely; mirrors
/// `IndexingChainFixture.ScriptedTokenStream` without the payload attribute,
/// which norms never read.
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
        panic!("norms portability tests require Maven and a JDK: {reason}");
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
    /// Whether the committed field infos say the segment has norms.
    has_norms: bool,
    /// One `norm=<doc> <field> <value>` line per norm Lucene's own reader
    /// decoded, plus one `nonorms=<field>` line per field with no norms.
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
        .arg("-Dexec.mainClass=org.apache.lucene.rucene.codec.NormsFixture")
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
    let mut has_norms = None;
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
        } else if let Some(value) = line.strip_prefix("hasnorms=") {
            has_norms = Some(value.trim() == "true");
        } else if let Some(value) = line.strip_prefix("fieldinfo=") {
            field_infos.push(value.trim().to_string());
        } else if line.starts_with("norm=") || line.starts_with("nonorms=") {
            dump.push(line.trim_end().to_string());
        }
    }
    Ok(JavaSegment {
        name: name.ok_or_else(|| format!("harness printed no segment name:\n{stdout}"))?,
        id: id.ok_or_else(|| format!("harness printed no segment id:\n{stdout}"))?,
        max_doc: max_doc.ok_or_else(|| format!("harness printed no max doc:\n{stdout}"))?,
        compound: compound.ok_or_else(|| format!("harness printed no compound flag:\n{stdout}"))?,
        has_norms: has_norms
            .ok_or_else(|| format!("harness printed no hasnorms flag:\n{stdout}"))?,
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

/// Reads the norms Rucene wrote with the real Lucene reader and returns the
/// `norm=` lines it decoded.
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
    // Only the fields that actually have norms may be listed: Lucene's reader
    // refuses a metadata entry whose field says `omitNorms`, and a field that
    // omits them has no entry to find.
    let fields: Vec<String> = field_infos
        .iter()
        .filter(|info| info.has_norms())
        .map(|info| {
            format!(
                "{}:{}:{}",
                info.name,
                info.number,
                index_options(info.index_options)
            )
        })
        .collect();
    let output = Command::new(mvn)
        .arg("-q")
        .arg("compile")
        .arg("exec:java")
        .arg("-Dexec.mainClass=org.apache.lucene.rucene.codec.NormsReaderFixture")
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
        .filter(|line| line.starts_with("norm="))
        .map(|line| line.trim_end().to_string())
        .collect())
}

/// Renders an [`IndexOptions`] the way `IndexOptions.valueOf` expects it.
fn index_options(options: IndexOptions) -> &'static str {
    match options {
        IndexOptions::NONE => "NONE",
        IndexOptions::DOCS => "DOCS",
        IndexOptions::DOCS_AND_FREQS => "DOCS_AND_FREQS",
        IndexOptions::DOCS_AND_FREQS_AND_POSITIONS => "DOCS_AND_FREQS_AND_POSITIONS",
        IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS => {
            "DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS"
        }
        IndexOptions::DOCS_AND_CUSTOM_FREQS => "DOCS_AND_CUSTOM_FREQS",
    }
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
) -> FieldInfos {
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
    let mut live = LiveIndexWriterConfig::new(Arc::clone(&analyzer));
    live.set_similarity(similarity(case));
    let live = Arc::new(live);

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
        chain
            .process_document(doc_id as i32, &document, true, &mut field_infos)
            .unwrap_or_else(|error| panic!("document {doc_id} must index cleanly: {error}"));
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
    finished
}

/// Renders every norm of every field exactly as `NormsFixture.dump` does, so
/// the two sides compare as plain strings.
fn dump(producer: Option<&dyn NormsProducer>, field_infos: &FieldInfos) -> Vec<String> {
    let mut lines = Vec::new();
    for info in field_infos.iter() {
        let Some(producer) = producer.filter(|_| info.has_norms()) else {
            lines.push(format!("nonorms={}", info.name));
            continue;
        };
        let mut norms = producer.get_norms(info).expect("norms");
        loop {
            let doc = norms.next_doc().expect("next doc");
            if doc == NO_MORE_DOCS {
                break;
            }
            lines.push(format!(
                "norm={doc} {} {}",
                info.name,
                norms.long_value().expect("long value")
            ));
        }
    }
    lines
}

/// Opens a Lucene-written index with Rucene and dumps its norms.
fn read_java_index(dir: &Path) -> Vec<String> {
    let codec = ensure_codec();
    let directory: Arc<dyn Directory> = Arc::new(FSDirectory::open(dir).expect("directory"));
    let infos = SegmentInfos::read_latest_commit(directory.as_ref()).expect("segments file");
    let segment_info = infos.info(0).info.clone();
    let field_infos = codec
        .field_infos_format()
        .read(directory.as_ref(), &segment_info, "", &*DEFAULT_IO_CONTEXT)
        .expect("field infos");
    let producer = if field_infos.has_norms() {
        let read_state = rucene::codecs::state::SegmentReadState::new(
            directory.as_ref(),
            &segment_info,
            &field_infos,
            &*DEFAULT_IO_CONTEXT,
        );
        let producer = codec
            .norms_format()
            .norms_producer(&read_state)
            .expect("norms producer");
        producer.check_integrity().expect("integrity");
        Some(producer)
    } else {
        None
    };
    dump(producer.as_deref(), &field_infos)
}

/// Compares the norms files of the two directories byte for byte.
fn assert_norms_bytes_equal(java_dir: &Path, rust_dir: &Path, segment: &str, case: &str) {
    let mut compared = 0;
    for extension in NORMS_EXTENSIONS {
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
        NORMS_EXTENSIONS.len(),
        "[{case}] both norms files must be compared"
    );
}

fn hex_window(bytes: &[u8], centre: usize) -> String {
    let from = centre.saturating_sub(16);
    let to = std::cmp::min(centre + 16, bytes.len());
    let body: Vec<String> = bytes[from..to].iter().map(|b| format!("{b:02x}")).collect();
    format!("[{from}..{to}] {}", body.join(" "))
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
    let scripts = documents(case);
    assert_eq!(
        segment.max_doc,
        scripts.len() as i32,
        "[{case}] the two sides must index the same number of documents"
    );

    let field_infos = write_with_rucene(rust_tmp.path(), case, &segment.name, segment.id, &scripts);

    // The field numbers order the `.nvm` entries, so they must agree before the
    // byte comparison can mean anything.
    let rust_field_infos: Vec<String> = field_infos
        .iter()
        .map(|info| {
            format!(
                "{} {} omitNorms={} hasNorms={} indexOptions={}",
                info.number,
                info.name,
                info.omits_norms(),
                info.has_norms(),
                index_options(info.index_options)
            )
        })
        .collect();
    assert_eq!(
        rust_field_infos, segment.field_infos,
        "[{case}] the field infos must agree before the norms can"
    );
    assert_eq!(
        field_infos.has_norms(),
        segment.has_norms,
        "[{case}] the two sides must agree on whether the segment has norms"
    );

    // 1. Rucene writes what Lucene writes.
    assert_norms_bytes_equal(java_tmp.path(), rust_tmp.path(), &segment.name, case);

    // 2. Rucene reads what Lucene wrote.
    assert_eq!(
        read_java_index(java_tmp.path()),
        segment.dump,
        "[{case}] Rucene must decode Lucene's norms exactly as Lucene does"
    );

    // 3. Lucene reads what Rucene wrote.
    if field_infos.has_norms() {
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
            .filter(|line| line.starts_with("norm="))
            .cloned()
            .collect();
        assert_eq!(
            java_read, expected,
            "[{case}] Lucene must decode Rucene's norms exactly as it decodes its own"
        );
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn norms_of_an_all_documents_field_match_lucene() {
    // `docsWithFieldOffset == -1`: no docs-with-field stream is written at all.
    assert_case_matches_lucene("dense");
}

#[test]
fn norms_of_a_sparse_field_match_lucene() {
    // Few enough documents that `IndexedDISI` writes its SPARSE block: one
    // short per document.
    assert_case_matches_lucene("sparse");
}

#[test]
fn norms_of_a_field_in_an_indexed_disi_dense_block_match_lucene() {
    // More than 4095 documents in one block, so `IndexedDISI` switches to a
    // bitmap plus a rank table. This is the encoding a plain `FixedBitSet`
    // dump would silently pass its own round-trip on while being unreadable by
    // Lucene.
    assert_case_matches_lucene("disidense");
}

#[test]
fn norms_of_a_field_in_an_indexed_disi_all_block_match_lucene() {
    // A whole 65536-document block, which `IndexedDISI` writes with neither a
    // bitmap nor shorts, inside a segment that is still not all-documents.
    assert_case_matches_lucene("disiall");
}

#[test]
fn a_segment_whose_fields_all_omit_norms_writes_no_norms_files() {
    require_maven();
    let java_tmp = tempfile::tempdir().expect("temp dir");
    let rust_tmp = tempfile::tempdir().expect("temp dir");
    let segment = run_java_fixture(java_tmp.path(), "omitnorms").expect("java fixture");
    assert!(
        !segment.has_norms,
        "the fixture must produce a segment with no norms"
    );
    for extension in NORMS_EXTENSIONS {
        assert!(
            !java_tmp
                .path()
                .join(format!("{}.{extension}", segment.name))
                .exists(),
            "Lucene must not write a .{extension} when every field omits norms"
        );
    }

    let scripts = documents("omitnorms");
    let field_infos = write_with_rucene(
        rust_tmp.path(),
        "omitnorms",
        &segment.name,
        segment.id,
        &scripts,
    );
    assert!(!field_infos.has_norms());
    for extension in NORMS_EXTENSIONS {
        assert!(
            !rust_tmp
                .path()
                .join(format!("{}.{extension}", segment.name))
                .exists(),
            "Rucene must not write a .{extension} when every field omits norms"
        );
    }
    // Both readers must agree that there is nothing to read.
    assert_eq!(read_java_index(java_tmp.path()), segment.dump);
}

#[test]
fn a_field_that_omits_norms_is_skipped_beside_fields_that_do_not() {
    assert_case_matches_lucene("mixedomit");
}

#[test]
fn norms_of_a_docs_only_field_count_unique_terms_like_lucene() {
    assert_case_matches_lucene("docsonly");
}

#[test]
fn overlap_tokens_are_discounted_like_lucene() {
    assert_case_matches_lucene("overlaps");
}

#[test]
fn overlap_tokens_are_kept_when_discount_overlaps_is_off() {
    assert_case_matches_lucene("nodiscount");
}

#[test]
fn norms_of_a_multi_valued_field_match_lucene() {
    assert_case_matches_lucene("multivalue");
}

#[test]
fn a_field_present_with_no_tokens_gets_a_zero_norm_like_lucene() {
    assert_case_matches_lucene("emptyvalue");
}

#[test]
fn a_constant_norm_is_stored_in_the_metadata_like_lucene() {
    assert_case_matches_lucene("constant");
}

#[test]
fn two_byte_norms_match_lucene() {
    assert_case_matches_lucene("wide2");
}

#[test]
fn four_byte_norms_match_lucene() {
    assert_case_matches_lucene("wide4");
}

#[test]
fn eight_byte_norms_match_lucene() {
    assert_case_matches_lucene("wide8");
}

#[test]
fn rucene_reads_the_norms_of_a_compound_file_segment_written_by_lucene() {
    // The norms reader has to read through the `Directory` it is given — here a
    // compound-file view over `_0.cfs` — because the files of a compound
    // segment exist nowhere else.
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
            .join(format!("{}.nvd", segment.name))
            .exists(),
        "a compound segment has no loose .nvd to fall back on"
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
    assert!(field_infos.has_norms());
    let read_state = rucene::codecs::state::SegmentReadState::new(
        compound.as_ref(),
        &segment_info,
        &field_infos,
        &*DEFAULT_IO_CONTEXT,
    );
    let producer = codec
        .norms_format()
        .norms_producer(&read_state)
        .expect("norms producer");
    producer.check_integrity().expect("integrity");

    assert_eq!(
        dump(Some(producer.as_ref()), &field_infos),
        segment.dump,
        "Rucene must decode a compound-file segment exactly as Lucene does"
    );
}

#[test]
fn the_default_norm_encoding_matches_lucene_for_every_length() {
    // `Similarity.computeNorm` is a pure function of the invert state, so its
    // whole domain can be compared against the Java encoder without indexing
    // anything. `SmallFloat.intToByte4` is what Lucene applies, and the value
    // that reaches the format is that byte *sign-extended* to a long.
    for length in [
        0,
        1,
        2,
        23,
        24,
        25,
        26,
        31,
        32,
        33,
        63,
        64,
        127,
        128,
        255,
        256,
        1_000,
        4_096,
        65_535,
        65_536,
        1_000_000,
        i32::MAX,
    ] {
        let mut state = FieldInvertState::new(10, "body".to_string(), FREQS);
        state.set_length(length);
        let norm = compute_default_norm(&state, true).expect("norm");
        let encoded = rucene::util::SmallFloat::int_to_byte4(length).expect("encoded");
        assert_eq!(
            norm, encoded as i8 as i64,
            "length {length} must encode to the sign-extended byte {encoded}"
        );
        assert!(
            (-128..=127).contains(&norm),
            "length {length} produced {norm}, which is not a signed byte"
        );
    }
}
