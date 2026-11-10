//! Postings contracts pinned against Apache Lucene Core 10.5.0.
//!
//! * **rmp #238.** `readLevel0PosData` reads the position and payload skip
//!   data of the next level-0 block from the **doc** file
//!   (`Lucene104PostingsReader.java:792-799`), where the skip data is
//!   interleaved with the doc blocks. The port read it from the positions and
//!   payloads files, so an `advance` that skipped a block positioned the
//!   positions stream from garbage. The test reads a Lucene-written index whose
//!   term occurs in 10000 documents and compares every posting an `advance`
//!   schedule reaches, positions, offsets and payloads included, and the hits
//!   of a phrase query.
//! * **rmp #240.** The `PostingsEnum` flag constants have Lucene's values
//!   (`PostingsEnum.java:34-64`).
//! * **rmp #241.** `ForUtil.decode` reached with a width past its 32-entry mask
//!   table throws `ArrayIndexOutOfBoundsException` (`ForUtil.java:168-170`);
//!   the port panicked. The Java side is measured on `ForUtil` directly; the
//!   Rust side on a `.doc` file whose full block was corrupted, which is the
//!   path that reaches it (`Lucene104PostingsReader.java:594-597`).
//! * **rmp #351.** A level's impacts are the bytes its last skip entry
//!   read, `levelNSerializedImpacts.length = numImpactBytes`
//!   (`Lucene104PostingsReader.java:702, 751, 850`), decoded from
//!   `scratch.reset(bytes, 0, length)` (line 1398). The port decoded the
//!   whole buffer, so a block with fewer impact bytes than an earlier one
//!   returned the earlier block's tail as extra, unordered impacts. The test
//!   reads a Lucene-written index whose level-0 and level-1 blocks alternate
//!   many and one competitive pairs, and compares every level's impacts after
//!   each `advanceShallow`.
//!
//! # Where the expected values come from
//!
//! `PostingsSkipFixture` from `tests/fixtures/java-codec-harness`
//! (`java -cp lucene-core-10.5.0.jar PostingsSkipFixture.java <dir>`) writes
//! the index and prints what Lucene's own reader returns.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex, OnceLock};

use rucene::codecs::lucene103::blocktree::{DEFAULT_MAX_BLOCK_SIZE, DEFAULT_MIN_BLOCK_SIZE};
use rucene::codecs::state::{SegmentReadState, SegmentWriteState};
use rucene::codecs::{
    register_codec, BlockTermState, Codec, CompoundFormat, DocValuesFormat, FieldInfosFormat,
    FieldsConsumer, FieldsProducer, KnnVectorsFormat, LiveDocsFormat,
    Lucene103BlockTreeTermsWriter, Lucene104Codec, Lucene104PostingsFormat,
    Lucene104PostingsWriter, NormsFormat, NormsProducer, PerFieldPostingsFormat, PointsFormat,
    PostingsFormat, PostingsWriterBase, SegmentInfoFormat, StoredFieldsFormat, TermVectorsFormat,
};
use rucene::document::{
    Document, Field, FieldType, NumericDocValuesField, Store, StringField, TextField,
};
use rucene::index::freq_prox_terms_writer::{SortingDocsEnum, SortingPostingsEnum};
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::merge_policy::NoMergePolicy;
use rucene::index::{
    directory_reader, DirectoryReader, FieldInfo, ImpactsEnum, IndexOptions, IndexReader,
    MappingMultiPostingsEnum, PostingsEnum, SeekStatus, Term, TermState, TermsEnum,
    POSTINGS_ENUM_ALL, POSTINGS_ENUM_FREQS, POSTINGS_ENUM_NONE, POSTINGS_ENUM_OFFSETS,
    POSTINGS_ENUM_PAYLOADS, POSTINGS_ENUM_POSITIONS,
};
use rucene::search::{
    IndexSearcher, PhraseQuery, Query, Sort, SortField, SortFieldType, NO_MORE_DOCS,
};
use rucene::store::{ByteBuffersDirectory, DataOutput, Directory, IndexOutput, NIOFSDirectory};
use rucene::util::attribute::AttributeSource;
use rucene::util::{BytesRef, FixedBitSet};

/// `PostingsSkipFixture.DOCS`.
const DOCS: i32 = 10_000;

/// `PostingsSkipFixture.TARGETS`.
const TARGETS: [i32; 16] = [
    0, 1, 255, 256, 257, 300, 600, 1024, 1025, 4097, 8191, 8192, 8193, 8600, 9000, 9999,
];

/// What the fixture printed, and the index it wrote.
struct JavaRun {
    dir: PathBuf,
    /// `PostingsSkipFixture.writeAndReadImpacts`'s index.
    impacts_dir: PathBuf,
    lines: Vec<String>,
    _tmp: tempfile::TempDir,
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

/// Runs the fixture once per test process; every test reads the same index.
fn java_run() -> &'static JavaRun {
    static RUN: OnceLock<JavaRun> = OnceLock::new();
    RUN.get_or_init(|| {
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join(
            "tests/fixtures/java-codec-harness/src/main/java/org/apache/lucene/rucene/codec/\
             PostingsSkipFixture.java",
        );
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("index");
        let impacts_dir = tmp.path().join("impacts");
        let output = Command::new("java")
            .arg("-cp")
            .arg(lucene_jar())
            .arg(&fixture)
            .arg(&dir)
            .arg(&impacts_dir)
            .output()
            .expect("a JDK is required to run the Lucene oracle");
        assert!(
            output.status.success(),
            "PostingsSkipFixture failed:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let lines = String::from_utf8(output.stdout)
            .unwrap()
            .lines()
            .map(str::to_string)
            .collect();
        JavaRun {
            dir,
            impacts_dir,
            lines,
            _tmp: tmp,
        }
    })
}

/// The lines the fixture printed with `prefix `, prefix removed.
fn java_lines(prefix: &str) -> Vec<String> {
    let prefix = format!("{prefix} ");
    java_run()
        .lines
        .iter()
        .filter_map(|line| line.strip_prefix(&prefix).map(str::to_string))
        .collect()
}

fn term(field: &str, text: &str) -> Term {
    Term::new(field, BytesRef::new(text.as_bytes().to_vec()))
}

/// `PostingsSkipFixture.describe`: doc, freq, then pos/start-end/payload.
fn describe(postings: &mut dyn PostingsEnum, doc: i32) -> String {
    if doc == NO_MORE_DOCS {
        return "END".to_string();
    }
    let freq = postings.freq().unwrap();
    let mut out = format!("{doc} freq={freq}");
    for _ in 0..freq {
        let position = postings.next_position().unwrap();
        let payload = match postings.get_payload().unwrap() {
            Some(bytes) => bytes.iter().map(|b| format!("{b:02x}")).collect(),
            None => "-".to_string(),
        };
        out.push_str(&format!(
            " {position}/{}-{}/{payload}",
            postings.start_offset(),
            postings.end_offset()
        ));
    }
    out
}

fn open_searcher() -> IndexSearcher {
    let _ = register_codec("Lucene104", Lucene104Codec::new());
    let directory: Arc<dyn Directory> = Arc::new(NIOFSDirectory::open(&java_run().dir).unwrap());
    let reader = directory_reader::open(directory).unwrap();
    IndexSearcher::new(reader as Arc<dyn IndexReader>).unwrap()
}

#[test]
fn advance_reads_the_position_skip_data_lucene_wrote() {
    let searcher = open_searcher();
    assert!(java_run().lines.iter().any(|l| l == "leaves=1"));
    let leaves = searcher.get_leaf_contexts();
    assert_eq!(leaves.len(), 1);
    let leaf = leaves[0].leaf_reader();

    let mut advanced = Vec::new();
    let mut walked = Vec::new();
    for field in ["body", "plain"] {
        for flags in [POSTINGS_ENUM_POSITIONS, POSTINGS_ENUM_ALL] {
            let mut postings = leaf.postings(&term(field, "a"), flags).unwrap().unwrap();
            for target in TARGETS {
                let doc = postings.advance(target).unwrap();
                advanced.push(format!(
                    "{field} {flags} {target} {}",
                    describe(postings.as_mut(), doc)
                ));
            }
        }
        let mut postings = leaf
            .postings(&term(field, "a"), POSTINGS_ENUM_ALL)
            .unwrap()
            .unwrap();
        let mut doc = postings.next_doc().unwrap();
        while doc != NO_MORE_DOCS {
            walked.push(format!("{field} {}", describe(postings.as_mut(), doc)));
            doc = postings.next_doc().unwrap();
            if doc != NO_MORE_DOCS {
                doc = postings.advance(doc + 397).unwrap();
            }
        }
    }
    assert_eq!(advanced.len(), 64);
    assert_eq!(
        advanced,
        java_lines("advance"),
        "every advanced-to posting must match Lucene's"
    );
    assert_eq!(
        walked,
        java_lines("walk"),
        "the interleaved walk must match"
    );
}

/// `PostingsSkipFixture.IMPACT_TARGETS`.
const IMPACT_TARGETS: [i32; 22] = [
    0, 100, 255, 256, 300, 511, 512, 513, 1000, 8191, 8192, 8193, 9000, 16383, 16384, 16385, 16700,
    17000, 24575, 24576, 25000, 25575,
];

/// `PostingsSkipFixture.describe(Impacts)`: `level:docIdUpTo:freq/norm,...`
/// per level.
fn describe_impacts(impacts: &dyn rucene::index::Impacts) -> String {
    let mut out = String::new();
    for level in 0..impacts.num_levels() {
        let buffer = impacts.get_impacts(level);
        if level > 0 {
            out.push(' ');
        }
        out.push_str(&format!("{level}:{}:", impacts.doc_id_up_to(level)));
        for k in 0..buffer.size {
            if k > 0 {
                out.push(',');
            }
            out.push_str(&format!("{}/{}", buffer.freqs[k], buffer.norms[k]));
        }
    }
    out
}

/// rmp #351: every level's impacts after each `advanceShallow` over the
/// Lucene-written index of alternating rich and poor blocks are Lucene's,
/// so a poor block never shows the tail of the rich block read before it
/// (`Lucene104PostingsReader.java:702, 751, 850, 1398`).
#[test]
fn impacts_of_each_level_match_lucene() {
    let _ = register_codec("Lucene104", Lucene104Codec::new());
    let directory: Arc<dyn Directory> =
        Arc::new(NIOFSDirectory::open(&java_run().impacts_dir).unwrap());
    let reader = directory_reader::open(directory).unwrap();
    assert!(java_run().lines.iter().any(|l| l == "impacts leaves=1"));
    let searcher = IndexSearcher::new(reader as Arc<dyn IndexReader>).unwrap();
    let leaves = searcher.get_leaf_contexts();
    assert_eq!(leaves.len(), 1);
    let terms = leaves[0].leaf_reader().terms("imp").unwrap().unwrap();
    let mut terms_enum = terms.iterator().unwrap();
    assert!(terms_enum
        .seek_exact(&BytesRef::new(b"a".to_vec()))
        .unwrap());
    let mut impacts = terms_enum.impacts(POSTINGS_ENUM_FREQS).unwrap();
    let mut rust = Vec::new();
    for target in IMPACT_TARGETS {
        impacts.advance_shallow(target).unwrap();
        rust.push(format!(
            "{target} {}",
            describe_impacts(impacts.get_impacts().unwrap().as_ref())
        ));
    }
    let java: Vec<String> = java_lines("impacts")
        .into_iter()
        .filter(|l| !l.starts_with("leaves="))
        .collect();
    assert_eq!(java.len(), IMPACT_TARGETS.len());
    assert_eq!(rust, java, "every level's impacts must match Lucene's");
}

/// Round 2, item 3: a docs-and-freqs enumeration walks every block through
/// the fast path of `moveToNextLevel0Block` (`Lucene104PostingsReader.java:780-786`)
/// and must still see Lucene's documents and frequencies.
#[test]
fn docs_and_freqs_walk_matches_lucene() {
    let searcher = open_searcher();
    let leaf = searcher.get_leaf_contexts()[0].leaf_reader();
    let mut rust = Vec::new();
    for field in ["body", "plain"] {
        let mut postings = leaf
            .postings(&term(field, "a"), POSTINGS_ENUM_FREQS)
            .unwrap()
            .unwrap();
        let (mut count, mut doc_sum, mut freq_sum) = (0i64, 0i64, 0i64);
        let mut doc = postings.next_doc().unwrap();
        while doc != NO_MORE_DOCS {
            count += 1;
            doc_sum += i64::from(doc);
            freq_sum += i64::from(postings.freq().unwrap()) * (i64::from(doc) + 1);
            doc = postings.next_doc().unwrap();
        }
        rust.push(format!("{field} {count} {doc_sum} {freq_sum}"));
    }
    assert_eq!(rust, java_lines("freqs"));
}

#[test]
fn phrase_query_hits_match_lucene_over_skipped_blocks() {
    let searcher = open_searcher();
    let mut rust = Vec::new();
    for field in ["body", "plain"] {
        let phrase: Arc<dyn Query> = Arc::new(
            PhraseQuery::new(0, vec![term(field, "a"), term(field, "b")], vec![0, 1]).unwrap(),
        );
        let top = searcher.search(phrase, DOCS).unwrap();
        let mut docs: Vec<i32> = top.score_docs.iter().map(|hit| hit.doc).collect();
        docs.sort_unstable();
        let docs: Vec<String> = docs.iter().map(i32::to_string).collect();
        rust.push(format!(
            "{field} {} {}",
            top.total_hits.value(),
            docs.join(",")
        ));
    }
    assert_eq!(rust, java_lines("phrase"));
}

#[test]
fn postings_flags_have_lucenes_values() {
    let rust = format!(
        "NONE={POSTINGS_ENUM_NONE} FREQS={POSTINGS_ENUM_FREQS} \
         POSITIONS={POSTINGS_ENUM_POSITIONS} OFFSETS={POSTINGS_ENUM_OFFSETS} \
         PAYLOADS={POSTINGS_ENUM_PAYLOADS} ALL={POSTINGS_ENUM_ALL}"
    );
    assert_eq!(vec![rust], java_lines("flags"));
}

#[test]
fn for_util_widths_past_the_mask_table_throw_lucenes_exception() {
    // What Lucene throws, and that it throws before reading a byte.
    let expected: Vec<String> = [32, 33, 40, 127]
        .iter()
        .map(|bits| {
            format!(
                "{bits} java.lang.ArrayIndexOutOfBoundsException \
                 Index {bits} out of bounds for length 32 fp=0"
            )
        })
        .collect();
    assert_eq!(java_lines("forutil"), expected);
}

// ---------------------------------------------------------------------------
// Postings-enum reuse on the real paths (rmp #276)
// ---------------------------------------------------------------------------

/// The address of a postings enum, which says whether two boxes hold the same
/// object, as Java's `==` on references does.
fn identity(postings: &dyn PostingsEnum) -> usize {
    postings as *const dyn PostingsEnum as *const () as usize
}

fn drain_docs(postings: &mut dyn PostingsEnum) -> Vec<i32> {
    let mut docs = Vec::new();
    loop {
        let doc = postings.next_doc().unwrap();
        if doc == NO_MORE_DOCS {
            return docs;
        }
        docs.push(doc);
    }
}

/// `LeafReuseOracle`: two committed segments of three documents, each with
/// the `DOCS` field `id` holding `a` and `b` and the positions field `body`
/// holding `x` and `y`.
fn two_reuse_segments() -> Arc<dyn DirectoryReader> {
    let _ = register_codec("Lucene104", Lucene104Codec::new());
    let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
    let config = IndexWriterConfig::new();
    config
        .set_merge_policy(Some(Arc::new(NoMergePolicy::new())))
        .unwrap();
    let writer = IndexWriter::new(Arc::clone(&dir), config).unwrap();
    for _ in 0..2 {
        for _ in 0..3 {
            let mut doc = Document::new();
            doc.add(Box::new(
                StringField::new("id", "a".to_string(), Store::NO).unwrap(),
            ));
            doc.add(Box::new(
                StringField::new("id", "b".to_string(), Store::NO).unwrap(),
            ));
            doc.add(Box::new(
                TextField::new("body", "x y".to_string(), Store::NO).unwrap(),
            ));
            writer.add_document(&doc).unwrap();
        }
        writer.commit().unwrap();
    }
    writer.close().unwrap();
    directory_reader::open(dir).unwrap()
}

/// `Lucene104PostingsReader.postings` hands back the `BlockPostingsEnum` it is
/// offered exactly when `canReuse` holds: the same reader's `docIn`, the same
/// index options, payloads and flags, and no impacts
/// (`Lucene104PostingsReader.java:256-264`, `:493-500`). Measured against
/// `lucene-core-10.5.0.jar` (`LeafReuseOracle`): `true`, then `false` for
/// other flags, another field's options, another segment, and an impacts
/// enum, each still reading `[0, 1, 2]`.
#[test]
fn the_block_postings_enum_is_reused_exactly_when_lucene_reuses_it() {
    let reader = two_reuse_segments();
    let leaves = Arc::clone(&reader).leaves().unwrap();
    assert_eq!(leaves.len(), 2);
    let leaf0 = leaves[0].leaf_reader();
    let leaf1 = leaves[1].leaf_reader();

    let mut body = leaf0.terms("body").unwrap().unwrap().iterator().unwrap();
    body.next().unwrap();
    let p1 = body.postings(None, POSTINGS_ENUM_FREQS).unwrap();
    let p1_identity = identity(p1.as_ref());
    body.next().unwrap();
    let mut p2 = body.postings(Some(p1), POSTINGS_ENUM_FREQS).unwrap();
    assert_eq!(
        identity(p2.as_ref()),
        p1_identity,
        "same reader, same flags"
    );
    assert_eq!(drain_docs(p2.as_mut()), vec![0, 1, 2]);

    let p2_identity = identity(p2.as_ref());
    let mut p3 = body.postings(Some(p2), POSTINGS_ENUM_POSITIONS).unwrap();
    assert_ne!(identity(p3.as_ref()), p2_identity, "other flags");
    assert_eq!(drain_docs(p3.as_mut()), vec![0, 1, 2]);

    let mut id = leaf0.terms("id").unwrap().unwrap().iterator().unwrap();
    id.next().unwrap();
    let q1 = id.postings(None, POSTINGS_ENUM_FREQS).unwrap();
    let q1_identity = identity(q1.as_ref());
    let q2 = body.postings(Some(q1), POSTINGS_ENUM_FREQS).unwrap();
    assert_ne!(identity(q2.as_ref()), q1_identity, "other index options");

    let mut body1 = leaf1.terms("body").unwrap().unwrap().iterator().unwrap();
    body1.next().unwrap();
    let q2_identity = identity(q2.as_ref());
    let mut other = body1.postings(Some(q2), POSTINGS_ENUM_FREQS).unwrap();
    assert_ne!(identity(other.as_ref()), q2_identity, "other reader");
    assert_eq!(drain_docs(other.as_mut()), vec![0, 1, 2]);

    let impacts: Box<dyn PostingsEnum> = body.impacts(POSTINGS_ENUM_FREQS).unwrap();
    let impacts_identity = identity(impacts.as_ref());
    let after_impacts = body.postings(Some(impacts), POSTINGS_ENUM_FREQS).unwrap();
    assert_ne!(
        identity(after_impacts.as_ref()),
        impacts_identity,
        "an enum built for impacts"
    );
}

/// `BlockPostingsEnum.reset` resets only the fields
/// `Lucene104PostingsReader.java:502-575` lists: the offsets of the last
/// position read are not among them, so an enum reused for the next term
/// answers `startOffset()` and `endOffset()`, before `nextDoc()`, with the
/// previous term's. `LeafReuseOracle`-style probe against
/// `lucene-core-10.5.0.jar` (`PassAOracle`, section 7): `aaa` reads offsets
/// `0..3`, the enum is reused for `bbb`, and before `nextDoc()` it still
/// answers `start=0 end=3`.
#[test]
fn a_reused_block_postings_enum_keeps_the_last_offsets_until_next_doc() {
    let _ = register_codec("Lucene104", Lucene104Codec::new());
    let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
    let field_type = FieldType::new();
    field_type.set_tokenized(true).unwrap();
    field_type
        .set_index_options(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS)
        .unwrap();
    field_type.freeze();
    let writer = IndexWriter::new(Arc::clone(&dir), IndexWriterConfig::new()).unwrap();
    for text in ["aaa bbb", "bbb"] {
        let mut doc = Document::new();
        doc.add(Box::new(
            Field::new("f", text.to_string(), field_type.clone()).unwrap(),
        ));
        writer.add_document(&doc).unwrap();
    }
    writer.close().unwrap();

    let reader = directory_reader::open(dir).unwrap();
    let leaves = Arc::clone(&reader).leaves().unwrap();
    let mut terms = leaves[0]
        .leaf_reader()
        .terms("f")
        .unwrap()
        .unwrap()
        .iterator()
        .unwrap();
    assert_eq!(terms.next().unwrap(), Some(BytesRef::new(b"aaa".to_vec())));
    let mut postings = terms.postings(None, POSTINGS_ENUM_OFFSETS).unwrap();
    assert_eq!(postings.next_doc().unwrap(), 0);
    assert_eq!(postings.freq().unwrap(), 1);
    postings.next_position().unwrap();
    assert_eq!((postings.start_offset(), postings.end_offset()), (0, 3));
    let first = identity(postings.as_ref());

    assert_eq!(terms.next().unwrap(), Some(BytesRef::new(b"bbb".to_vec())));
    let reused = terms
        .postings(Some(postings), POSTINGS_ENUM_OFFSETS)
        .unwrap();
    assert_eq!(identity(reused.as_ref()), first, "the enum is reused");
    assert_eq!((reused.start_offset(), reused.end_offset()), (0, 3));
}

/// `ReusePayload.Toks`: emits `(term, payload)` pairs through a
/// `PackedTokenAttributeImpl` and a `PayloadAttributeImpl`.
#[derive(Debug)]
struct PayloadTokens {
    source: AttributeSource,
    tokens: Vec<(&'static str, Vec<u8>)>,
    upto: usize,
}

impl PayloadTokens {
    fn new(tokens: Vec<(&'static str, Vec<u8>)>) -> Self {
        let mut source =
            AttributeSource::new_with_factory(rucene::analysis::default_token_attribute_factory());
        source
            .add_attribute::<rucene::analysis::tokenattributes::PackedTokenAttributeImpl>()
            .unwrap();
        source.add_attribute_impl_instance(Box::new(
            rucene::analysis::tokenattributes::PayloadAttributeImpl::new(),
        ));
        Self {
            source,
            tokens,
            upto: 0,
        }
    }
}

impl rucene::analysis::TokenStream for PayloadTokens {
    fn increment_token(&mut self) -> rucene::error::Result<bool> {
        use rucene::analysis::tokenattributes::{
            CharTermAttribute, PackedTokenAttributeImpl, PayloadAttribute, PayloadAttributeImpl,
        };
        if self.upto == self.tokens.len() {
            return Ok(false);
        }
        self.source.clear_attributes();
        let (term, payload) = self.tokens[self.upto].clone();
        self.upto += 1;
        self.source
            .get_attribute_mut::<PackedTokenAttributeImpl>()
            .unwrap()
            .append_string(Some(term));
        self.source
            .get_attribute_mut::<PayloadAttributeImpl>()
            .unwrap()
            .set_payload(Some(BytesRef::new(payload)));
        Ok(true)
    }
    fn reset(&mut self) -> rucene::error::Result<()> {
        self.upto = 0;
        Ok(())
    }
    fn end(&mut self) -> rucene::error::Result<()> {
        self.source.end_attributes();
        Ok(())
    }
    fn attribute_source(&self) -> &AttributeSource {
        &self.source
    }
    fn attribute_source_mut(&mut self) -> &mut AttributeSource {
        &mut self.source
    }
}

/// `BlockPostingsEnum` keeps its payload as a `BytesRef` view that only
/// `accumulatePayloadAndOffsets` re-points (`Lucene104PostingsReader.java:1278-1285`)
/// and `getPayload()` returns (`:1340-1346`); `reset` rewinds
/// `payloadByteUpto` to zero but leaves the view, so an enum reused for the
/// next term answers `getPayload()`, before `nextDoc()`, with the previous
/// term's payload. Measured against `lucene-core-10.5.0.jar` (the audit's
/// `ReusePayload.java`): `aaa payload=[1 2]`, then `reused=true payload
/// before nextDoc=[1 2]`.
#[test]
fn a_reused_block_postings_enum_keeps_the_last_payload_until_next_doc() {
    let _ = register_codec("Lucene104", Lucene104Codec::new());
    let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
    let field_type = FieldType::new();
    field_type.set_tokenized(true).unwrap();
    field_type
        .set_index_options(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS)
        .unwrap();
    field_type.freeze();
    let writer = IndexWriter::new(Arc::clone(&dir), IndexWriterConfig::new()).unwrap();
    for tokens in [
        vec![("aaa", vec![1u8, 2]), ("bbb", vec![3u8])],
        vec![("bbb", vec![4u8])],
    ] {
        let stream: std::sync::Arc<
            rucene::util::atomic_ref_cell::AtomicRefCell<dyn rucene::analysis::TokenStream>,
        > = std::sync::Arc::new(rucene::util::atomic_ref_cell::AtomicRefCell::new(
            PayloadTokens::new(tokens),
        ));
        let mut doc = Document::new();
        doc.add(Box::new(
            Field::new_with_token_stream("f", stream, field_type.clone()).unwrap(),
        ));
        writer.add_document(&doc).unwrap();
    }
    writer.close().unwrap();

    let reader = directory_reader::open(dir).unwrap();
    let leaves = Arc::clone(&reader).leaves().unwrap();
    let mut terms = leaves[0]
        .leaf_reader()
        .terms("f")
        .unwrap()
        .unwrap()
        .iterator()
        .unwrap();
    assert_eq!(terms.next().unwrap(), Some(BytesRef::new(b"aaa".to_vec())));
    let mut postings = terms.postings(None, POSTINGS_ENUM_PAYLOADS).unwrap();
    assert_eq!(postings.next_doc().unwrap(), 0);
    postings.next_position().unwrap();
    assert_eq!(postings.get_payload().unwrap(), Some(&[1u8, 2][..]));
    let first = identity(postings.as_ref());

    assert_eq!(terms.next().unwrap(), Some(BytesRef::new(b"bbb".to_vec())));
    let reused = terms
        .postings(Some(postings), POSTINGS_ENUM_PAYLOADS)
        .unwrap();
    assert_eq!(identity(reused.as_ref()), first, "the enum is reused");
    assert_eq!(reused.get_payload().unwrap(), Some(&[1u8, 2][..]));
}

/// A fresh `BlockPostingsEnum` that reads offsets leaves `startOffset` and
/// `endOffset` at Java's `int` default of zero; only an enum that does not
/// read them sets -1 (`Lucene104PostingsReader.java:472-480`). Measured
/// against `lucene-core-10.5.0.jar` (the audit's `FreshOffsets.java`):
/// `fresh: start=0 end=0`, and `after nextDoc: start=0 end=0`.
#[test]
fn a_fresh_block_postings_enum_answers_offsets_of_zero_as_lucene_does() {
    let _ = register_codec("Lucene104", Lucene104Codec::new());
    let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
    let field_type = FieldType::new();
    field_type.set_tokenized(true).unwrap();
    field_type
        .set_index_options(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS)
        .unwrap();
    field_type.freeze();
    let writer = IndexWriter::new(Arc::clone(&dir), IndexWriterConfig::new()).unwrap();
    for text in ["aaa bbb", "bbb"] {
        let mut doc = Document::new();
        doc.add(Box::new(
            Field::new("f", text.to_string(), field_type.clone()).unwrap(),
        ));
        writer.add_document(&doc).unwrap();
    }
    writer.close().unwrap();

    let reader = directory_reader::open(dir).unwrap();
    let leaves = Arc::clone(&reader).leaves().unwrap();
    let mut terms = leaves[0]
        .leaf_reader()
        .terms("f")
        .unwrap()
        .unwrap()
        .iterator()
        .unwrap();
    terms.next().unwrap();
    let mut postings = terms.postings(None, POSTINGS_ENUM_OFFSETS).unwrap();
    assert_eq!((postings.start_offset(), postings.end_offset()), (0, 0));
    postings.next_doc().unwrap();
    assert_eq!((postings.start_offset(), postings.end_offset()), (0, 0));
}

/// One `postings` call a postings writer made: the field, the enum it offered
/// as `reuse` and the one it got back (as [`identity`]), and that one's type.
#[derive(Debug, Clone, PartialEq)]
struct Offer {
    field: String,
    offered: Option<usize>,
    returned: usize,
    kind: &'static str,
}

type OfferLog = Arc<Mutex<Vec<Offer>>>;

/// The type of `postings`, for the enums whose reuse is tested.
fn kind_of(postings: &mut dyn PostingsEnum) -> &'static str {
    let Some(any) = postings.as_any_mut() else {
        return "other";
    };
    if any.is::<MappingMultiPostingsEnum>() {
        "MappingMultiPostingsEnum"
    } else if any.is::<SortingPostingsEnum>() {
        "SortingPostingsEnum"
    } else if any.is::<SortingDocsEnum>() {
        "SortingDocsEnum"
    } else {
        "other"
    }
}

/// A terms enum that records every `postings` call it forwards.
struct RecordingTermsEnum<'a> {
    inner: &'a mut dyn TermsEnum,
    field: String,
    log: OfferLog,
}

impl std::fmt::Display for RecordingTermsEnum<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&*self.inner, f)
    }
}

impl TermsEnum for RecordingTermsEnum<'_> {
    fn attributes(&mut self) -> &mut AttributeSource {
        self.inner.attributes()
    }

    fn seek_exact(&mut self, text: &BytesRef) -> rucene::error::Result<bool> {
        self.inner.seek_exact(text)
    }

    fn seek_ceil(&mut self, text: &BytesRef) -> rucene::error::Result<SeekStatus> {
        self.inner.seek_ceil(text)
    }

    fn seek_ord(&mut self, ord: i64) -> rucene::error::Result<()> {
        self.inner.seek_ord(ord)
    }

    fn seek_term_state(
        &mut self,
        text: &BytesRef,
        state: &dyn TermState,
    ) -> rucene::error::Result<()> {
        self.inner.seek_term_state(text, state)
    }

    fn term(&self) -> rucene::error::Result<BytesRef> {
        self.inner.term()
    }

    fn ord(&self) -> rucene::error::Result<i64> {
        self.inner.ord()
    }

    fn doc_freq(&mut self) -> rucene::error::Result<i32> {
        self.inner.doc_freq()
    }

    fn total_term_freq(&mut self) -> rucene::error::Result<i64> {
        self.inner.total_term_freq()
    }

    fn postings(
        &mut self,
        reuse: Option<Box<dyn PostingsEnum>>,
        flags: i32,
    ) -> rucene::error::Result<Box<dyn PostingsEnum>> {
        let offered = reuse.as_deref().map(identity);
        let mut postings = self.inner.postings(reuse, flags)?;
        self.log.lock().unwrap().push(Offer {
            field: self.field.clone(),
            offered,
            returned: identity(postings.as_ref()),
            kind: kind_of(postings.as_mut()),
        });
        Ok(postings)
    }

    fn impacts(&mut self, flags: i32) -> rucene::error::Result<Box<dyn ImpactsEnum>> {
        self.inner.impacts(flags)
    }

    fn term_state(&mut self) -> rucene::error::Result<Box<dyn TermState>> {
        self.inner.term_state()
    }

    fn prefer_seek_exact(&self) -> bool {
        self.inner.prefer_seek_exact()
    }

    fn next(&mut self) -> rucene::error::Result<Option<BytesRef>> {
        self.inner.next()
    }
}

/// `Lucene104PostingsWriter`, with each term's enum seen through a
/// [`RecordingTermsEnum`].
struct RecordingPostingsWriter {
    inner: Lucene104PostingsWriter,
    field: String,
    log: OfferLog,
}

impl PostingsWriterBase for RecordingPostingsWriter {
    fn init(
        &mut self,
        terms_out: &mut dyn IndexOutput,
        state: &SegmentWriteState,
    ) -> rucene::error::Result<()> {
        PostingsWriterBase::init(&mut self.inner, terms_out, state)
    }

    fn write_term(
        &mut self,
        term: &BytesRef,
        terms_enum: &mut dyn TermsEnum,
        docs_seen: &mut FixedBitSet,
        norms: Option<&dyn NormsProducer>,
    ) -> rucene::error::Result<Option<BlockTermState>> {
        let mut recording = RecordingTermsEnum {
            inner: terms_enum,
            field: self.field.clone(),
            log: Arc::clone(&self.log),
        };
        PostingsWriterBase::write_term(&mut self.inner, term, &mut recording, docs_seen, norms)
    }

    fn encode_term(
        &mut self,
        out: &mut dyn DataOutput,
        field_info: &FieldInfo,
        state: &BlockTermState,
        absolute: bool,
    ) -> rucene::error::Result<()> {
        PostingsWriterBase::encode_term(&mut self.inner, out, field_info, state, absolute)
    }

    fn set_field(&mut self, field_info: &FieldInfo) -> rucene::error::Result<()> {
        self.field = field_info.name.clone();
        PostingsWriterBase::set_field(&mut self.inner, field_info)
    }

    fn close(&mut self) -> rucene::error::Result<()> {
        PostingsWriterBase::close(&mut self.inner)
    }
}

/// The `Lucene104` postings format, writing through a
/// [`RecordingPostingsWriter`]; it reads with the shipped format, whose files
/// these are.
#[derive(Debug)]
struct RecordingPostingsFormat {
    log: OfferLog,
}

impl PostingsFormat for RecordingPostingsFormat {
    fn name(&self) -> &str {
        "Lucene104"
    }

    fn fields_consumer<'a>(
        &self,
        state: &SegmentWriteState<'a>,
    ) -> rucene::error::Result<Box<dyn FieldsConsumer + 'a>> {
        let postings_writer = Box::new(RecordingPostingsWriter {
            inner: Lucene104PostingsWriter::new(state)?,
            field: String::new(),
            log: Arc::clone(&self.log),
        });
        Ok(Box::new(Lucene103BlockTreeTermsWriter::new(
            state,
            postings_writer,
            DEFAULT_MIN_BLOCK_SIZE,
            DEFAULT_MAX_BLOCK_SIZE,
        )?))
    }

    fn fields_producer<'a>(
        &self,
        state: &SegmentReadState<'a>,
    ) -> rucene::error::Result<Box<dyn FieldsProducer>> {
        Lucene104PostingsFormat::new().fields_producer(state)
    }
}

/// `Lucene104Codec` whose postings go through a [`RecordingPostingsFormat`].
#[derive(Debug)]
struct RecordingCodec {
    inner: Lucene104Codec,
    postings: PerFieldPostingsFormat,
}

impl RecordingCodec {
    fn new(log: &OfferLog) -> Self {
        let format: Arc<dyn PostingsFormat> = Arc::new(RecordingPostingsFormat {
            log: Arc::clone(log),
        });
        Self {
            inner: Lucene104Codec::new(),
            postings: PerFieldPostingsFormat::new(move |_field| Arc::clone(&format)),
        }
    }
}

impl Codec for RecordingCodec {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn postings_format(&self) -> &dyn PostingsFormat {
        &self.postings
    }

    fn doc_values_format(&self) -> &dyn DocValuesFormat {
        self.inner.doc_values_format()
    }

    fn stored_fields_format(&self) -> &dyn StoredFieldsFormat {
        self.inner.stored_fields_format()
    }

    fn term_vectors_format(&self) -> &dyn TermVectorsFormat {
        self.inner.term_vectors_format()
    }

    fn field_infos_format(&self) -> &dyn FieldInfosFormat {
        self.inner.field_infos_format()
    }

    fn segment_info_format(&self) -> &dyn SegmentInfoFormat {
        self.inner.segment_info_format()
    }

    fn norms_format(&self) -> &dyn NormsFormat {
        self.inner.norms_format()
    }

    fn live_docs_format(&self) -> &dyn LiveDocsFormat {
        self.inner.live_docs_format()
    }

    fn compound_format(&self) -> &dyn CompoundFormat {
        self.inner.compound_format()
    }

    fn points_format(&self) -> &dyn PointsFormat {
        self.inner.points_format()
    }

    fn knn_vectors_format(&self) -> &dyn KnnVectorsFormat {
        self.inner.knn_vectors_format()
    }
}

/// Asserts that, within each field, every term after the first was offered
/// the enum the previous term got back, and got that very enum back, of
/// `kind_of_field(field)`; and that the first term of a field was offered the
/// last enum of the field before, as `PushPostingsWriterBase` never drops it.
fn assert_each_term_reuses_the_previous_enum(
    offers: &[Offer],
    kind_of_field: impl Fn(&str) -> &'static str,
) {
    assert!(!offers.is_empty());
    assert_eq!(offers[0].offered, None, "nothing to offer the first term");
    for pair in offers.windows(2) {
        let (previous, offer) = (&pair[0], &pair[1]);
        assert_eq!(
            offer.offered,
            Some(previous.returned),
            "`postingsEnum = termsEnum.postings(postingsEnum, enumFlags)`: {offer:?}"
        );
        if offer.field == previous.field {
            assert_eq!(
                offer.returned, previous.returned,
                "the reuse branch hands back the enum it was offered: {offer:?}"
            );
        }
    }
    for offer in offers {
        assert_eq!(offer.kind, kind_of_field(&offer.field), "{offer:?}");
    }
    let mut fields: Vec<&str> = offers.iter().map(|offer| offer.field.as_str()).collect();
    fields.dedup();
    for field in fields {
        let terms = offers.iter().filter(|offer| offer.field == field).count();
        assert!(terms > 1, "field {field} must have several terms to reuse");
    }
}

/// Indexes `docs` documents with the `DOCS` field `id` and the positions
/// field `body`, three terms each; `rank` sorts them in reverse.
fn reuse_doc(i: i32) -> Document {
    let mut doc = Document::new();
    let term = ["a", "b", "c"][(i % 3) as usize];
    doc.add(Box::new(
        StringField::new("id", term.to_string(), Store::NO).unwrap(),
    ));
    doc.add(Box::new(
        TextField::new(
            "body",
            ["x", "y", "z"][(i % 3) as usize].to_string(),
            Store::NO,
        )
        .unwrap(),
    ));
    doc.add(Box::new(NumericDocValuesField::new("rank", -i64::from(i))));
    doc
}

/// A real merge writes every term with the `MappingMultiPostingsEnum` the
/// previous term of the field got: `PushPostingsWriterBase.writeTerm` offers
/// it (`PushPostingsWriterBase.java:125`) and `MappedMultiTermsEnum.postings`
/// takes its reuse branch (`MappedMultiFields.java:119-121`).
#[test]
fn a_merge_reuses_the_mapping_postings_enum_of_the_previous_term() {
    let _ = register_codec("Lucene104", Lucene104Codec::new());
    let log: OfferLog = Arc::new(Mutex::new(Vec::new()));
    let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
    let config = IndexWriterConfig::new();
    config
        .set_codec(Arc::new(RecordingCodec::new(&log)))
        .unwrap();
    let writer = IndexWriter::new(Arc::clone(&dir), config).unwrap();
    for segment in 0..2 {
        for i in 0..6 {
            writer.add_document(&reuse_doc(segment * 6 + i)).unwrap();
        }
        writer.commit().unwrap();
    }
    log.lock().unwrap().clear();
    writer.force_merge(1).unwrap();
    writer.commit().unwrap();
    writer.close().unwrap();

    let offers = log.lock().unwrap().clone();
    assert_eq!(
        offers.len(),
        6,
        "three terms in each of two fields: {offers:?}"
    );
    assert_each_term_reuses_the_previous_enum(&offers, |_| "MappingMultiPostingsEnum");
}

/// A flush under an index sort that reorders the documents writes every term
/// with the `Sorting*Enum` the previous term of the field got:
/// `SortingTermsEnum.postings` takes its reuse branch
/// (`FreqProxTermsWriter.java:183`, `:208`).
#[test]
fn a_sorted_flush_reuses_the_sorting_postings_enum_of_the_previous_term() {
    let _ = register_codec("Lucene104", Lucene104Codec::new());
    let log: OfferLog = Arc::new(Mutex::new(Vec::new()));
    let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
    let config = IndexWriterConfig::new();
    config
        .set_codec(Arc::new(RecordingCodec::new(&log)))
        .unwrap();
    config
        .set_index_sort(
            Sort::new_fields(vec![SortField::new(
                Some("rank".to_string()),
                SortFieldType::Long,
            )
            .unwrap()])
            .unwrap(),
        )
        .unwrap();
    let writer = IndexWriter::new(Arc::clone(&dir), config).unwrap();
    for i in 0..6 {
        writer.add_document(&reuse_doc(i)).unwrap();
    }
    writer.commit().unwrap();
    writer.close().unwrap();

    let offers = log.lock().unwrap().clone();
    assert_eq!(
        offers.len(),
        6,
        "three terms in each of two fields: {offers:?}"
    );
    assert_each_term_reuses_the_previous_enum(&offers, |field| match field {
        "body" => "SortingPostingsEnum",
        _ => "SortingDocsEnum",
    });
}
