//! Indexing-chain portability tests against Apache Lucene Core 10.5.0.
//!
//! Each test drives the Java reference harness
//! (`tests/fixtures/java-codec-harness`, class `IndexingChainFixture`) to index
//! a scripted list of tokens with the real Lucene `IndexWriter`, then indexes
//! exactly the same tokens with Rucene's [`DefaultIndexingChain`] into a
//! segment carrying the *same* name and the *same* segment id, and compares the
//! resulting postings files **byte for byte**.
//!
//! The token script is supplied to both sides as an explicit table of
//! `(term, positionIncrement, startOffset, endOffset, payload)` tuples, so no
//! analyzer takes part: any byte difference can only come from the indexing
//! chain or from the postings codec.
//!
//! Files compared: `.doc`, `.pos`, `.psm`, `.tim`, `.tip` and `.tmd` — every
//! file the postings format writes. The remaining files of a Lucene segment
//! (`.fnm`, `.si`, stored fields, norms) are written by components that are not
//! part of the indexing chain and are deliberately ignored here.

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
use rucene::codecs::{register_codec, Codec, Lucene104Codec};
use rucene::document::{Document, Field, FieldType};
use rucene::index::documents_writer::{IndexingChain, IndexingChainFlushState};
use rucene::index::field_infos::{FieldInfosBuilder, FieldNumbers};
use rucene::index::index_writer_config::LiveIndexWriterConfig;
use rucene::index::indexing_chain::DefaultIndexingChain;
use rucene::index::{IndexOptions, SegmentInfo};
use rucene::store::{
    flush_io_context, Directory, FSDirectory, FlushInfo, TrackingDirectoryWrapper,
};
use rucene::util::{AttributeSource, BytesRef, NoOutputInfoStream, Version};

/// The single indexed field every case uses; mirrors `IndexingChainFixture.FIELD`.
const FIELD: &str = "body";

/// Postings files the indexing chain is responsible for.
const POSTINGS_EXTENSIONS: [&str; 6] = ["doc", "pos", "psm", "tim", "tip", "tmd"];

/// Extensions compared for `case`.
///
/// `manyterms` is the only case whose terms dictionary spans several blocks,
/// and the term index of a multi-block field is a real prefix trie. Rucene's
/// `TrieBuilder` still serialises a single node — a documented limitation of
/// the block-tree codec, not of the indexing chain — so for that case the term
/// index (`.tip`) and the file pointers the trie records in `.tmd` are left
/// out. The postings (`.doc`, `.pos`, `.psm`) and the terms dictionary
/// (`.tim`), which are what the indexing chain feeds the codec, are still
/// compared byte for byte.
fn compared_extensions(case: &str) -> Vec<&'static str> {
    POSTINGS_EXTENSIONS
        .into_iter()
        .filter(|extension| !(case == "manyterms" && matches!(*extension, "tip" | "tmd")))
        .collect()
}

// ---------------------------------------------------------------------------
// Scripted token stream
// ---------------------------------------------------------------------------

/// One scripted token; mirrors `IndexingChainFixture.Tok`.
#[derive(Debug, Clone)]
struct Tok {
    term: &'static str,
    pos_incr: i32,
    start: i32,
    end: i32,
    payload: Option<Vec<u8>>,
}

impl Tok {
    fn of(term: &'static str, pos_incr: i32, start: i32, end: i32) -> Self {
        Self {
            term,
            pos_incr,
            start,
            end,
            payload: None,
        }
    }

    fn with_payload(
        term: &'static str,
        pos_incr: i32,
        start: i32,
        end: i32,
        payload: Vec<u8>,
    ) -> Self {
        Self {
            term,
            pos_incr,
            start,
            end,
            payload: Some(payload),
        }
    }
}

/// A term whose text is owned, used by the generated `manyterms` case.
#[derive(Debug, Clone)]
struct OwnedTok {
    term: String,
    pos_incr: i32,
    start: i32,
    end: i32,
    payload: Option<Vec<u8>>,
}

impl From<&Tok> for OwnedTok {
    fn from(token: &Tok) -> Self {
        Self {
            term: token.term.to_string(),
            pos_incr: token.pos_incr,
            start: token.start,
            end: token.end,
            payload: token.payload.clone(),
        }
    }
}

/// Emits a fixed list of tokens, bypassing analysis completely.
///
/// Mirrors `IndexingChainFixture.ScriptedTokenStream`.
#[derive(Debug)]
struct ScriptedTokenStream {
    source: AttributeSource,
    tokens: Vec<OwnedTok>,
    upto: usize,
    final_offset: i32,
}

impl ScriptedTokenStream {
    fn new(tokens: Vec<OwnedTok>) -> Self {
        let mut source = AttributeSource::new_with_factory(default_token_attribute_factory());
        source
            .add_attribute::<PackedTokenAttributeImpl>()
            .expect("packed token attribute");
        // The default token-attribute factory does not know how to build a
        // payload attribute, exactly as in Lucene where only analyzers that
        // emit payloads add one; the fixture therefore installs the instance.
        source.add_attribute_impl_instance(Box::new(PayloadAttributeImpl::new()));
        Self {
            source,
            tokens,
            upto: 0,
            final_offset: 0,
        }
    }
}

impl TokenStream for ScriptedTokenStream {
    fn increment_token(&mut self) -> rucene::error::Result<bool> {
        if self.upto == self.tokens.len() {
            return Ok(false);
        }
        self.source.clear_attributes();
        let token = self.tokens[self.upto].clone();
        self.upto += 1;
        {
            let mut packed = self
                .source
                .get_attribute_mut::<PackedTokenAttributeImpl>()
                .expect("packed token attribute");
            packed.append_string(&token.term);
            packed.set_position_increment(token.pos_incr);
            packed.set_offset(token.start, token.end);
        }
        {
            let mut payload = self
                .source
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
        self.source.end_attributes();
        let mut packed = self
            .source
            .get_attribute_mut::<PackedTokenAttributeImpl>()
            .expect("packed token attribute");
        packed.set_offset(self.final_offset, self.final_offset);
        Ok(())
    }

    fn attribute_source(&self) -> &AttributeSource {
        &self.source
    }

    fn attribute_source_mut(&mut self) -> &mut AttributeSource {
        &mut self.source
    }
}

// ---------------------------------------------------------------------------
// Document scripts, mirroring IndexingChainFixture
// ---------------------------------------------------------------------------

fn index_options(case: &str) -> IndexOptions {
    match case {
        "docs" => IndexOptions::DOCS,
        "freqs" => IndexOptions::DOCS_AND_FREQS,
        "positions" | "payloads" | "manyterms" | "emptyvalue" => {
            IndexOptions::DOCS_AND_FREQS_AND_POSITIONS
        }
        "offsets" | "multivalue" | "stats" | "statsmulti" => {
            IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS
        }
        other => panic!("unknown case: {other}"),
    }
}

fn documents(case: &str) -> Vec<Vec<Vec<OwnedTok>>> {
    match case {
        "docs" | "freqs" | "positions" | "offsets" | "stats" => base_documents(),
        "statsmulti" => multi_value_documents(),
        "payloads" => payload_documents(),
        "multivalue" => multi_value_documents(),
        "manyterms" => many_terms_documents(),
        "emptyvalue" => empty_value_documents(),
        other => panic!("unknown case: {other}"),
    }
}

fn owned(tokens: &[Tok]) -> Vec<OwnedTok> {
    tokens.iter().map(OwnedTok::from).collect()
}

fn base_documents() -> Vec<Vec<Vec<OwnedTok>>> {
    vec![
        vec![owned(&[
            Tok::of("alpha", 1, 0, 5),
            Tok::of("beta", 1, 6, 10),
            Tok::of("alpha", 1, 11, 16),
            Tok::of("gamma", 1, 17, 22),
        ])],
        vec![owned(&[
            Tok::of("beta", 1, 0, 4),
            Tok::of("delta", 3, 10, 15),
            Tok::of("alpha", 1, 16, 21),
        ])],
        vec![owned(&[
            Tok::of("gamma", 1, 0, 5),
            Tok::of("gamma", 0, 0, 5),
            Tok::of("epsilon", 2, 6, 13),
        ])],
        vec![Vec::new()],
        vec![owned(&[
            Tok::of("alpha", 1, 0, 5),
            Tok::of("alpha", 1, 6, 11),
            Tok::of("alpha", 1, 12, 17),
            Tok::of("zeta", 1, 18, 22),
        ])],
        vec![owned(&[
            Tok::of("beta", 1, 0, 4),
            Tok::of("gamma", 1, 5, 10),
            Tok::of("delta", 1, 11, 16),
            Tok::of("epsilon", 1, 17, 24),
            Tok::of("zeta", 1, 25, 29),
        ])],
    ]
}

fn long_payload(length: usize) -> Vec<u8> {
    (0..length).map(|i| (i * 7 + 1) as u8).collect()
}

fn payload_documents() -> Vec<Vec<Vec<OwnedTok>>> {
    vec![
        vec![owned(&[
            Tok::with_payload("alpha", 1, 0, 5, vec![1]),
            Tok::of("beta", 1, 6, 10),
            Tok::with_payload("alpha", 1, 11, 16, vec![2, 3, 4]),
        ])],
        vec![owned(&[
            Tok::with_payload("beta", 1, 0, 4, vec![0xFF, 0x00, 0x7F]),
            Tok::with_payload("gamma", 1, 5, 10, Vec::new()),
        ])],
        vec![owned(&[
            Tok::with_payload("alpha", 1, 0, 5, long_payload(40)),
            Tok::with_payload("gamma", 1, 6, 11, long_payload(7)),
        ])],
        vec![owned(&[
            Tok::of("delta", 1, 0, 5),
            Tok::with_payload("delta", 1, 6, 11, vec![9, 9]),
        ])],
    ]
}

fn multi_value_documents() -> Vec<Vec<Vec<OwnedTok>>> {
    vec![
        vec![
            owned(&[Tok::of("alpha", 1, 0, 5), Tok::of("beta", 1, 6, 10)]),
            owned(&[Tok::of("alpha", 1, 0, 5), Tok::of("gamma", 1, 6, 11)]),
        ],
        vec![
            owned(&[Tok::of("delta", 1, 0, 5)]),
            owned(&[Tok::of("delta", 1, 0, 5)]),
            owned(&[Tok::of("epsilon", 1, 0, 7)]),
        ],
        vec![owned(&[Tok::of("beta", 1, 0, 4)]), Vec::new()],
    ]
}

fn many_terms_documents() -> Vec<Vec<Vec<OwnedTok>>> {
    let mut documents = Vec::new();
    for doc_id in 0..200 {
        let mut tokens = Vec::new();
        let mut offset = 0i32;
        for term in 0..60 {
            if (doc_id + term) % 3 != 0 {
                continue;
            }
            let text = format!("term{term:04}");
            let repeats = (term % 3) + 1;
            for _ in 0..repeats {
                tokens.push(OwnedTok {
                    term: text.clone(),
                    pos_incr: 1,
                    start: offset,
                    end: offset + text.len() as i32,
                    payload: None,
                });
                offset += text.len() as i32 + 1;
            }
        }
        documents.push(vec![tokens]);
    }
    documents
}

fn empty_value_documents() -> Vec<Vec<Vec<OwnedTok>>> {
    vec![
        vec![Vec::new()],
        vec![owned(&[Tok::of("solo", 1, 0, 4)])],
        vec![Vec::new()],
        vec![owned(&[Tok::of("solo", 1, 0, 4), Tok::of("solo", 1, 5, 9)])],
        vec![Vec::new()],
    ]
}

// ---------------------------------------------------------------------------
// Java harness driver
// ---------------------------------------------------------------------------

fn harness_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/java-codec-harness")
}

/// Serialises Maven invocations, which all share one `target/` directory.
static HARNESS_LOCK: Mutex<()> = Mutex::new(());

fn which_mvn() -> Result<String, String> {
    for candidate in ["mvn", "mvnw"] {
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

/// Metadata the Java harness prints about the segment it committed.
#[derive(Debug)]
struct JavaSegment {
    name: String,
    id: [u8; 16],
    max_doc: i32,
    /// One entry per `Similarity.computeNorm` call, in document order.
    invert_states: Vec<InvertStats>,
}

/// The inversion statistics of one field of one document.
///
/// Mirrors the getters of `org.apache.lucene.index.FieldInvertState`.
#[derive(Debug, PartialEq, Eq)]
struct InvertStats {
    field: String,
    length: i32,
    num_overlap: i32,
    unique_term_count: i32,
    max_term_frequency: i32,
    position: i32,
    offset: i32,
}

impl InvertStats {
    /// Parses one `invert_state field=... length=...` line of the harness.
    fn parse(line: &str) -> Result<Self, String> {
        let mut field = None;
        let mut values: HashMap<&str, i32> = HashMap::new();
        for pair in line.split_whitespace().skip(1) {
            let (key, value) = pair
                .split_once('=')
                .ok_or_else(|| format!("malformed pair {pair:?}"))?;
            if key == "field" {
                field = Some(value.to_string());
            } else {
                values.insert(
                    match key {
                        "length" => "length",
                        "numOverlap" => "num_overlap",
                        "uniqueTermCount" => "unique_term_count",
                        "maxTermFrequency" => "max_term_frequency",
                        "position" => "position",
                        "offset" => "offset",
                        other => return Err(format!("unexpected key {other:?}")),
                    },
                    value.parse::<i32>().map_err(|e| format!("{pair:?}: {e}"))?,
                );
            }
        }
        let get = |key: &str| -> Result<i32, String> {
            values
                .get(key)
                .copied()
                .ok_or_else(|| format!("missing {key} in {line:?}"))
        };
        Ok(Self {
            field: field.ok_or_else(|| format!("missing field in {line:?}"))?,
            length: get("length")?,
            num_overlap: get("num_overlap")?,
            unique_term_count: get("unique_term_count")?,
            max_term_frequency: get("max_term_frequency")?,
            position: get("position")?,
            offset: get("offset")?,
        })
    }
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
        .arg("-Dexec.mainClass=org.apache.lucene.rucene.codec.IndexingChainFixture")
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

    let mut name = None;
    let mut id = None;
    let mut max_doc = None;
    let mut invert_states = Vec::new();
    for line in stdout.lines() {
        if line.starts_with("invert_state ") {
            invert_states.push(InvertStats::parse(line)?);
        } else if let Some(value) = line.strip_prefix("segment=") {
            name = Some(value.trim().to_string());
        } else if let Some(value) = line.strip_prefix("segment_id=") {
            let raw = value.trim();
            let mut bytes = [0u8; 16];
            if raw.len() != 32 {
                return Err(format!("unexpected segment id {raw:?}"));
            }
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
        }
    }

    Ok(JavaSegment {
        name: name.ok_or_else(|| format!("harness printed no segment name:\n{stdout}"))?,
        id: id.ok_or_else(|| format!("harness printed no segment id:\n{stdout}"))?,
        max_doc: max_doc.ok_or_else(|| format!("harness printed no max doc:\n{stdout}"))?,
        invert_states,
    })
}

/// Fails the test when Maven is unavailable.
///
/// A portability test proves byte compatibility against the reference Java
/// implementation, so it has nothing to assert without the harness: skipping
/// would report success while proving nothing. Matching
/// `tests/portability/codecs.rs`, a missing toolchain is a hard failure.
fn require_maven() {
    if let Err(reason) = which_mvn() {
        panic!("indexing-chain portability tests require Maven and a JDK: {reason}");
    }
}

// ---------------------------------------------------------------------------
// Rucene side
// ---------------------------------------------------------------------------

fn ensure_codec() -> Arc<dyn Codec> {
    let _ = register_codec("Lucene104", Lucene104Codec::new());
    rucene::codecs::default_codec().expect("Lucene104 codec is registered")
}

/// Indexes `documents` with Rucene's indexing chain into `out_dir`, producing a
/// segment named `segment_name` with segment id `segment_id`.
fn write_with_rucene(
    out_dir: &Path,
    case: &str,
    segment_name: &str,
    segment_id: [u8; 16],
    documents: &[Vec<Vec<OwnedTok>>],
) {
    let codec = ensure_codec();
    let options = index_options(case);

    let mut field_type = FieldType::new();
    field_type.set_tokenized(true).expect("tokenized");
    field_type.set_omit_norms(true).expect("omit norms");
    field_type
        .set_index_options(options)
        .expect("index options");
    field_type.freeze();

    // The analyzer never tokenizes here: every field carries an explicit token
    // stream. It is consulted only for the multi-valued gaps, so the one
    // requirement is that its gaps match the Lucene `WhitespaceAnalyzer` the
    // Java fixture uses, which inherits the `Analyzer` defaults of 0 and 1.
    let analyzer: Arc<dyn Analyzer> = Arc::new(StandardAnalyzer::new());
    assert_eq!(
        analyzer.get_position_increment_gap(FIELD),
        0,
        "the two analyzers must apply the same position gap"
    );
    assert_eq!(
        analyzer.get_offset_gap(FIELD),
        1,
        "the two analyzers must apply the same offset gap"
    );
    let live = Arc::new(LiveIndexWriterConfig::new(Arc::clone(&analyzer)));

    let mut chain = DefaultIndexingChain::new(Arc::clone(&live));
    let numbers = Arc::new(FieldNumbers::new(None, None).expect("field numbers"));
    let mut field_infos = FieldInfosBuilder::new(numbers);

    for (doc_id, values) in documents.iter().enumerate() {
        let mut document = Document::new();
        for tokens in values {
            let stream: Rc<RefCell<dyn TokenStream>> =
                Rc::new(RefCell::new(ScriptedTokenStream::new(tokens.clone())));
            document.add(Box::new(
                Field::new_with_token_stream(FIELD, stream, field_type.clone())
                    .expect("token stream field"),
            ));
        }
        chain
            .process_document(doc_id as i32, &document, true, &mut field_infos)
            .expect("process document");
    }

    let finished = field_infos.finish().expect("field infos");
    let directory: Box<dyn Directory> = Box::new(FSDirectory::open(out_dir).expect("directory"));
    let tracking = TrackingDirectoryWrapper::new(directory);

    let segment_info = SegmentInfo::new(
        Arc::new(FSDirectory::open(out_dir).expect("directory")),
        Version::LATEST,
        Some(Version::LATEST),
        segment_name.to_string(),
        documents.len() as i32,
        false,
        false,
        codec,
        HashMap::new(),
        segment_id,
        HashMap::new(),
        Default::default(),
    )
    .expect("segment info");

    let info_stream = NoOutputInfoStream;
    let context = flush_io_context(FlushInfo::new(documents.len() as i32, 0));
    let flush_state = IndexingChainFlushState {
        info_stream: &info_stream,
        directory: &tracking,
        segment_info: &segment_info,
        field_infos: &finished,
        context: context.as_ref(),
        live_docs: None,
        del_count_on_flush: 0,
        delete_terms: &[],
    };
    chain.flush(&flush_state).expect("flush");
}

/// Compares the postings files of the two directories byte for byte.
fn assert_postings_bytes_equal(java_dir: &Path, rust_dir: &Path, segment: &str, case: &str) {
    let mut compared = 0;
    for extension in compared_extensions(case) {
        let file_name = format!("{segment}_Lucene104_0.{extension}");
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
    assert!(
        compared > 0,
        "[{case}] no postings file was compared; the fixture produced nothing"
    );
}

/// Renders a readable window of `bytes` around `centre`.
fn hex_window(bytes: &[u8], centre: usize) -> String {
    let from = centre.saturating_sub(16);
    let to = std::cmp::min(centre + 16, bytes.len());
    let body: Vec<String> = bytes[from..to].iter().map(|b| format!("{b:02x}")).collect();
    format!("[{from}..{to}] {}", body.join(" "))
}

/// Runs one case end to end.
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

    write_with_rucene(rust_tmp.path(), case, &segment.name, segment.id, &documents);
    assert_postings_bytes_equal(java_tmp.path(), rust_tmp.path(), &segment.name, case);
}

/// Indexes the documents of `case` with Rucene and returns, per document, the
/// statistics its chain accumulated in [`FieldInvertState`].
///
/// Documents whose field produced no token are skipped, because Lucene's
/// `PerField.finish` short-circuits the norm — and therefore the recording
/// similarity — when `invertState.length` is zero.
fn rucene_invert_states(case: &str, documents: &[Vec<Vec<OwnedTok>>]) -> Vec<InvertStats> {
    ensure_codec();
    let options = index_options(case);
    let mut field_type = FieldType::new();
    field_type.set_tokenized(true).expect("tokenized");
    field_type
        .set_index_options(options)
        .expect("index options");
    field_type.freeze();

    let analyzer: Arc<dyn Analyzer> = Arc::new(StandardAnalyzer::new());
    let live = Arc::new(LiveIndexWriterConfig::new(analyzer));
    let mut chain = DefaultIndexingChain::new(live);
    let numbers = Arc::new(FieldNumbers::new(None, None).expect("field numbers"));
    let mut field_infos = FieldInfosBuilder::new(numbers);

    let mut out = Vec::new();
    for (doc_id, values) in documents.iter().enumerate() {
        let mut document = Document::new();
        for tokens in values {
            let stream: Rc<RefCell<dyn TokenStream>> =
                Rc::new(RefCell::new(ScriptedTokenStream::new(tokens.clone())));
            document.add(Box::new(
                Field::new_with_token_stream(FIELD, stream, field_type.clone())
                    .expect("token stream field"),
            ));
        }
        chain
            .process_document(doc_id as i32, &document, true, &mut field_infos)
            .expect("process document");
        let state = chain.field_invert_state(FIELD).expect("invert state");
        if state.length() == 0 {
            continue;
        }
        out.push(InvertStats {
            field: state.name().to_string(),
            length: state.length(),
            num_overlap: state.num_overlap(),
            unique_term_count: state.unique_term_count(),
            max_term_frequency: state.max_term_frequency(),
            position: state.position(),
            offset: state.offset(),
        });
    }
    out
}

/// Compares Rucene's inversion statistics with Lucene's, measured through a
/// recording `Similarity` rather than read off the Java source.
fn assert_invert_states_match_lucene(case: &str) {
    require_maven();
    let tmp = tempfile::tempdir().expect("temp dir");
    let segment = run_java_fixture(tmp.path(), case).expect("java fixture");
    assert!(
        !segment.invert_states.is_empty(),
        "[{case}] the harness recorded no statistics; the test would be vacuous"
    );
    let actual = rucene_invert_states(case, &documents(case));
    assert_eq!(
        actual, segment.invert_states,
        "[{case}] FieldInvertState statistics diverge from Lucene"
    );
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn field_invert_state_matches_lucene_for_single_valued_fields() {
    assert_invert_states_match_lucene("stats");
}

#[test]
fn field_invert_state_matches_lucene_for_multi_valued_fields() {
    assert_invert_states_match_lucene("statsmulti");
}

#[test]
fn postings_match_lucene_for_docs_only() {
    assert_case_matches_lucene("docs");
}

#[test]
fn postings_match_lucene_for_docs_and_freqs() {
    assert_case_matches_lucene("freqs");
}

#[test]
fn postings_match_lucene_for_docs_freqs_and_positions() {
    assert_case_matches_lucene("positions");
}

#[test]
fn postings_match_lucene_for_docs_freqs_positions_and_offsets() {
    assert_case_matches_lucene("offsets");
}

#[test]
fn postings_match_lucene_for_payloads() {
    assert_case_matches_lucene("payloads");
}

#[test]
fn postings_match_lucene_for_multi_valued_fields() {
    assert_case_matches_lucene("multivalue");
}

#[test]
fn postings_match_lucene_for_documents_without_tokens() {
    assert_case_matches_lucene("emptyvalue");
}

#[test]
fn postings_match_lucene_for_many_terms_and_documents() {
    assert_case_matches_lucene("manyterms");
}
