//! Portability tests for Lucene members that rmp tasks #185, #187, #188 and
//! #189 ported or confirmed under another name.
//!
//! Every assertion here is anchored on a value produced by **Apache Lucene Core
//! 10.5.0 itself**: the fixture `tests/fixtures/java-codec-harness`, class
//! `org.apache.lucene.util.RuceneMemberContractsFixture`, drives the real
//! classes and prints what they compute. It lives inside Lucene's own package
//! because `ByteBlockPool.setBytesRef` is package-private; the other
//! package-private members it measures are reached reflectively.
//!
//! | Lucene 10.5.0 | Rucene |
//! |---|---|
//! | `AfterEffect`, `BasicModel`, `Distribution`, `Independence`, `Lambda`, `Normalization`, `SimilarityBase`, `Axiomatic` `toString` and the six `AxiomaticF*.toString` | `Display` of each implementor |
//! | `IntArrayList`/`LongArrayList` `clone`, `iterator`, `toString` | `Clone`, `iter`, `Display` |
//! | `IntHashSet` `clone`, `equals`, `iterator` | `Clone`, `PartialEq`, `iter` |
//! | `BufferedChecksum.updateShort/Int/Long/Longs` | `BufferedChecksum::update_short/int/long/longs` |
//! | `BufferedChecksumIndexInput.readShort/Int/Long/Longs` | the `DataInput` overrides that call them |
//! | `Lucene90StoredFieldsFormat.impl(Mode)` | `Lucene90StoredFieldsFormat::impl_for_mode` |
//! | `DocValuesRewriteMethod.equals`/`hashCode` | `RewriteMethod::method_eq`/`method_hash` |
//! | `ByteBlockPool.setBytesRef` | `ByteBlockPool::set_bytes_ref` |
//! | `ExactPhraseMatcher.impactsApproximation()`, as `PhraseScorer` uses it | `PhraseMatcher::impacts_approximation` |
//! | `AbstractAnalysisFactory.getPattern` | `AbstractAnalysisFactory::get_pattern` |
//! | `TopFieldCollectorManager.getCollectors` | `TopFieldCollectorManager::get_collectors` |
//! | `CharArrayMap.createEntrySet` and `MapEntry.setValue` | `CharArrayMap::create_entry_set`, `MapEntry::set_value` |

#![deny(unsafe_code)]

use std::collections::HashMap;
use std::fmt::Display;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::AtomicI64;
use std::sync::{Arc, Mutex, OnceLock};

use rucene::analysis::factories::AbstractAnalysisFactory;
use rucene::analysis::CharArrayMap;
use rucene::codecs::lucene90::{Lucene90StoredFieldsFormat, Mode};
use rucene::codecs::{register_codec, Lucene104Codec};
use rucene::document::{Document, Store, TextField};
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::Term;
use rucene::internal::hppc::{IntArrayList, IntHashSet, LongArrayList};
use rucene::search::similarities::{
    AfterEffectB, AfterEffectL, AxiomaticF1EXP, AxiomaticF1LOG, AxiomaticF2EXP, AxiomaticF2LOG,
    AxiomaticF3EXP, AxiomaticF3LOG, BasicModelG, BasicModelIF, BasicModelIn, BasicModelIne,
    DFISimilarity, DFRSimilarity, DistributionLL, DistributionSPL, IBSimilarity,
    IndependenceChiSquared, IndependenceSaturated, IndependenceStandardized, LambdaDF, LambdaTTF,
    NoNormalization, NormalizationH1, NormalizationH2, NormalizationH3, NormalizationZ,
};
use rucene::search::{
    constant_score_rewrite, doc_values_rewrite, CollectorManager, DocValuesRewriteMethod,
    IndexSearcher, PhraseQuery, Query, RewriteMethod, ScoreMode, Sort, TermQuery, TopDocsCollector,
    TopFieldCollectorManager, NO_MORE_DOCS,
};
use rucene::store::{
    BufferedChecksum, BufferedChecksumIndexInput, ByteBuffersDirectory, ChecksumIndexInput,
    DataInput, Directory, IndexOutput, RamDirectory, DEFAULT_IO_CONTEXT,
};
use rucene::util::byte_block_pool::{ByteBlockPool, BYTE_BLOCK_SHIFT, BYTE_BLOCK_SIZE};
use rucene::util::{BytesRef, BytesRefBuilder};

// ---------------------------------------------------------------------------
// Java harness
// ---------------------------------------------------------------------------

static HARNESS_LOCK: Mutex<()> = Mutex::new(());
static FIXTURE: OnceLock<Vec<String>> = OnceLock::new();

fn harness_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("java-codec-harness")
}

fn which_mvn() -> std::result::Result<String, String> {
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

/// Runs the Java fixture once and returns its output lines.
///
/// A portability test proves compatibility against the reference Java
/// implementation, so it has nothing to assert without the harness: skipping
/// would report success while proving nothing.
fn fixture() -> &'static [String] {
    FIXTURE.get_or_init(|| {
        let _guard = HARNESS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mvn = match which_mvn() {
            Ok(mvn) => mvn,
            Err(reason) => {
                panic!("member-contract portability tests require Maven and a JDK: {reason}")
            }
        };
        let harness = harness_dir();
        assert!(
            harness.join("pom.xml").exists(),
            "pom.xml not found in {}",
            harness.display()
        );
        let output = Command::new(mvn)
            .arg("-q")
            .arg("compile")
            .arg("exec:java")
            .arg("-Dexec.mainClass=org.apache.lucene.util.RuceneMemberContractsFixture")
            .current_dir(&harness)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .expect("failed to spawn Maven");
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        assert!(
            output.status.success(),
            "Java fixture failed:\nstdout:\n{stdout}\nstderr:\n{stderr}"
        );
        assert!(
            stdout.lines().any(|line| line.trim() == "fixture_ok=true"),
            "the Java fixture did not finish:\n{stdout}"
        );
        stdout.lines().map(|line| line.to_string()).collect()
    })
}

/// The fixture lines whose first field is `kind`, split on `|`.
fn rows(kind: &str) -> Vec<Vec<String>> {
    let rows: Vec<Vec<String>> = fixture()
        .iter()
        .filter(|line| line.starts_with(&format!("{kind}|")))
        .map(|line| line.split('|').map(|f| f.to_string()).collect())
        .collect();
    assert!(
        !rows.is_empty(),
        "the Java fixture printed no `{kind}` rows"
    );
    rows
}

/// The single fixture row `kind|key|...`, without its first two fields.
fn row(kind: &str, key: &str) -> Vec<String> {
    rows(kind)
        .into_iter()
        .find(|r| r[1] == key)
        .unwrap_or_else(|| panic!("no `{kind}|{key}` row"))[2..]
        .to_vec()
}

// ---------------------------------------------------------------------------
// toString of the similarity components
// ---------------------------------------------------------------------------

/// The Rust value standing for each Java object the fixture prints.
fn similarity(key: &str) -> Box<dyn Display> {
    match key {
        "AfterEffectB" => Box::new(AfterEffectB::new()),
        "AfterEffectL" => Box::new(AfterEffectL::new()),
        "BasicModelG" => Box::new(BasicModelG::new()),
        "BasicModelIF" => Box::new(BasicModelIF::new()),
        "BasicModelIn" => Box::new(BasicModelIn::new()),
        "BasicModelIne" => Box::new(BasicModelIne::new()),
        "DistributionLL" => Box::new(DistributionLL::new()),
        "DistributionSPL" => Box::new(DistributionSPL::new()),
        "IndependenceChiSquared" => Box::new(IndependenceChiSquared::new()),
        "IndependenceSaturated" => Box::new(IndependenceSaturated::new()),
        "IndependenceStandardized" => Box::new(IndependenceStandardized::new()),
        "LambdaDF" => Box::new(LambdaDF::new()),
        "LambdaTTF" => Box::new(LambdaTTF::new()),
        "NoNormalization" => Box::new(NoNormalization::new()),
        "NormalizationH1()" => Box::new(NormalizationH1::default()),
        "NormalizationH1(2.5)" => Box::new(NormalizationH1::with_c(2.5).unwrap()),
        "NormalizationH2()" => Box::new(NormalizationH2::default()),
        "NormalizationH3()" => Box::new(NormalizationH3::default()),
        "NormalizationH3(0.1)" => Box::new(NormalizationH3::with_mu(0.1).unwrap()),
        "NormalizationH3(1e-5)" => Box::new(NormalizationH3::with_mu(1e-5).unwrap()),
        "NormalizationH3(1e10)" => Box::new(NormalizationH3::with_mu(1.0e10).unwrap()),
        "NormalizationZ()" => Box::new(NormalizationZ::default()),
        "NormalizationZ(0.25)" => Box::new(NormalizationZ::with_z(0.25).unwrap()),
        "NormalizationZ(1e-5)" => Box::new(NormalizationZ::with_z(1e-5).unwrap()),
        "AxiomaticF1EXP" => Box::new(AxiomaticF1EXP::new()),
        "AxiomaticF1LOG" => Box::new(AxiomaticF1LOG::new()),
        "AxiomaticF2EXP" => Box::new(AxiomaticF2EXP::new()),
        "AxiomaticF2LOG" => Box::new(AxiomaticF2LOG::new()),
        "AxiomaticF3EXP(0.25,1)" => {
            Box::new(AxiomaticF3EXP::with_s_and_query_len(0.25, 1).unwrap())
        }
        "AxiomaticF3LOG(0.25,1)" => {
            Box::new(AxiomaticF3LOG::with_s_and_query_len(0.25, 1).unwrap())
        }
        "DFR(G,B,H2)" => Box::new(DFRSimilarity::new(
            Box::new(BasicModelG::new()),
            Box::new(AfterEffectB::new()),
            Box::new(NormalizationH2::default()),
        )),
        "IB(LL,DF,H3)" => Box::new(IBSimilarity::new(
            Box::new(DistributionLL::new()),
            Box::new(LambdaDF::new()),
            Box::new(NormalizationH3::default()),
        )),
        "DFI(ChiSquared)" => Box::new(DFISimilarity::new(Box::new(IndependenceChiSquared::new()))),
        other => panic!("unexpected similarity `{other}`"),
    }
}

/// Every abstract `toString()` of the similarity framework is a `Display`
/// supertrait here, and every implementor must render what Java's override
/// returns — including `Float.toString` of the parameters `NormalizationH3`
/// and `NormalizationZ` print, and the names the composite similarities
/// build out of their parts.
#[test]
fn similarity_components_render_what_java_to_string_returns() {
    for r in rows("sim") {
        assert_eq!(similarity(&r[1]).to_string(), r[2], "{}", r[1]);
    }
}

// ---------------------------------------------------------------------------
// hppc
// ---------------------------------------------------------------------------

/// `IntArrayList`/`LongArrayList`: `toString` is `Arrays.toString` of the
/// elements, `clone()` copies the buffer so the two lists evolve apart, and
/// `iterator()` yields `(index, value)` cursors in order.
#[test]
fn hppc_lists_clone_iterate_and_render_as_java() {
    let mut ints = IntArrayList::from(&[1, -2, i32::MAX, i32::MIN, 0]);
    let rows_int: Vec<Vec<String>> = rows("hppc")
        .into_iter()
        .filter(|r| r[1] == "IntArrayList")
        .collect();
    let get = |rows: &[Vec<String>], what: &str| -> String {
        rows.iter()
            .find(|r| r[2] == what)
            .unwrap_or_else(|| panic!("no {what}"))[3]
            .clone()
    };
    assert_eq!(ints.to_string(), get(&rows_int, "values"));
    let mut ints_copy = ints.clone();
    ints.add(9);
    ints_copy.set(0, 7);
    assert_eq!(ints.to_string(), get(&rows_int, "after_clone_original"));
    assert_eq!(ints_copy.to_string(), get(&rows_int, "after_clone_copy"));
    let walked: String = ints
        .iter()
        .map(|c| format!("{}:{},", c.index, c.value))
        .collect();
    assert_eq!(walked, get(&rows_int, "iterator"));
    assert_eq!(IntArrayList::new().to_string(), get(&rows_int, "empty"));

    let rows_long: Vec<Vec<String>> = rows("hppc")
        .into_iter()
        .filter(|r| r[1] == "LongArrayList")
        .collect();
    let mut longs = LongArrayList::from(&[1, -2, i64::MAX, i64::MIN, 0]);
    assert_eq!(longs.to_string(), get(&rows_long, "values"));
    let mut longs_copy = longs.clone();
    longs.add(9);
    longs_copy.set(0, 7);
    assert_eq!(longs.to_string(), get(&rows_long, "after_clone_original"));
    assert_eq!(longs_copy.to_string(), get(&rows_long, "after_clone_copy"));
    let walked: String = longs
        .iter()
        .map(|c| format!("{}:{},", c.index, c.value))
        .collect();
    assert_eq!(walked, get(&rows_long, "iterator"));
    assert_eq!(LongArrayList::new().to_string(), get(&rows_long, "empty"));
}

/// `IntHashSet`: `equals` compares the keys whatever the insertion order and
/// including the empty-slot key `0`; `clone()` is equal to its source and
/// independent of it; `iterator()` visits every key once.
#[test]
fn hppc_int_hash_set_clone_equals_and_iterate_as_java() {
    let java: Vec<Vec<String>> = rows("hppc")
        .into_iter()
        .filter(|r| r[1] == "IntHashSet")
        .collect();
    let get = |what: &str| -> Vec<String> {
        java.iter()
            .find(|r| r[2] == what)
            .unwrap_or_else(|| panic!("no {what}"))[3..]
            .to_vec()
    };
    let a = IntHashSet::from(&[1, 2, 3, 0]);
    let b = IntHashSet::from(&[0, 3, 2, 1]);
    assert_eq!(
        vec![(a == b).to_string(), (b == a).to_string()],
        get("equals_reordered")
    );
    let mut c = a.clone();
    assert_eq!(vec![(a == c).to_string()], get("equals_clone"));
    c.add(4);
    assert_eq!(
        vec![
            (a == c).to_string(),
            a.size().to_string(),
            c.size().to_string()
        ],
        get("clone_then_add")
    );
    assert_eq!(
        vec![
            (IntHashSet::from(&[0]) == IntHashSet::from(&[0])).to_string(),
            (IntHashSet::from(&[0]) == IntHashSet::new()).to_string()
        ],
        get("empty_key")
    );
    let mut seen: Vec<i32> = c.iter().map(|cursor| cursor.value).collect();
    seen.sort_unstable();
    assert_eq!(vec![format!("{seen:?}")], get("iterator_sorted"));
}

// ---------------------------------------------------------------------------
// BufferedChecksum and BufferedChecksumIndexInput
// ---------------------------------------------------------------------------

/// Replays one `;`-separated scenario the fixture also replays.
fn replay(checksum: &mut BufferedChecksum, scenario: &str) {
    for op in scenario.split(';') {
        let (kind, arg) = op.split_once('=').expect("op");
        match kind {
            "b" => checksum.update(arg.parse::<i32>().unwrap() as u8),
            "s" => checksum.update_short(arg.parse().unwrap()),
            "i" => checksum.update_int(arg.parse().unwrap()),
            "l" => checksum.update_long(arg.parse().unwrap()),
            "L" => {
                let mut parts = arg.splitn(3, ':');
                let offset: usize = parts.next().unwrap().parse().unwrap();
                let len: usize = parts.next().unwrap().parse().unwrap();
                let values: Vec<i64> = parts
                    .next()
                    .unwrap()
                    .split(',')
                    .map(|v| v.parse().unwrap())
                    .collect();
                checksum.update_longs(&values, offset, len).unwrap();
            }
            other => panic!("unexpected op `{other}`"),
        }
    }
}

/// `updateShort`, `updateInt`, `updateLong` and `updateLongs` buffer the
/// little-endian bytes and flush exactly where Java's do, so the checksum is
/// Java's for every buffer size the fixture uses, including the boundaries a
/// value lands on and the two paths of `updateLongs` (a partly filled buffer
/// topped up one long at a time, then whole buffers of longs).
#[test]
fn buffered_checksum_updates_match_java() {
    for r in rows("cks") {
        let size: usize = r[1].parse().unwrap();
        let mut checksum = BufferedChecksum::with_buffer_size(size);
        replay(&mut checksum, &r[2]);
        assert_eq!(
            checksum.get_value().to_string(),
            r[3],
            "size {size}: {}",
            r[2]
        );
    }
}

/// `BufferedChecksumIndexInput.readShort/readInt/readLong/readLongs` read
/// through the wrapped input and checksum the values they read, so the
/// values and the checksum are Java's — which is also the plain CRC-32 of the
/// bytes read.
#[test]
fn buffered_checksum_index_input_reads_and_checksums_as_java() {
    let java = rows("bcii").remove(0);
    let dir = RamDirectory::default();
    {
        let mut out: Box<dyn IndexOutput> =
            dir.create_output("fixture", &*DEFAULT_IO_CONTEXT).unwrap();
        out.write_byte(0x7f).unwrap();
        out.write_short(-2).unwrap();
        out.write_int(-559038737).unwrap();
        out.write_long(81985529216486895).unwrap();
        for v in [i64::MIN, -1, 0, 1, i64::MAX] {
            out.write_long(v).unwrap();
        }
        out.write_short(0x1234).unwrap();
        out.close().unwrap();
    }
    let mut input =
        BufferedChecksumIndexInput::new(dir.open_input("fixture", &*DEFAULT_IO_CONTEXT).unwrap());
    let b = input.read_byte().unwrap() as i8;
    let s = input.read_short().unwrap();
    let i = input.read_int().unwrap();
    let l = input.read_long().unwrap();
    let mut dst = [0i64; 7];
    input.read_longs(&mut dst, 1, 5).unwrap();
    let tail = input.read_short().unwrap();
    let checksum = input.get_checksum().unwrap();
    let dst_text = format!(
        "[{}]",
        dst.iter()
            .map(|v| v.to_string())
            .collect::<Vec<_>>()
            .join(", ")
    );
    assert_eq!(
        vec![
            b.to_string(),
            s.to_string(),
            i.to_string(),
            l.to_string(),
            dst_text,
            tail.to_string(),
            checksum.to_string(),
        ],
        java[1..8].to_vec()
    );
    assert_eq!(
        java[7], java[8],
        "Java's checksum is the CRC-32 of the bytes"
    );
}

/// `readShort`, `readInt`, `readLong` and `readLongs` read the value through
/// the wrapped input and only then checksum it
/// (`BufferedChecksumIndexInput.java:50-74`), so a read that runs out of input
/// part-way through a value leaves the checksum where it was. Reading the
/// value byte by byte through the checksumming `read_byte` — what the
/// `DataInput` defaults would do without the overrides — would checksum the
/// partial bytes, which is what this observes.
#[test]
fn buffered_checksum_index_input_checksums_only_whole_values() {
    let dir = RamDirectory::default();
    for r in rows("bcii_eof") {
        let (kind, partial): (&str, u8) = (&r[1], r[2].parse().unwrap());
        let name = format!("eof-{kind}");
        {
            let mut out: Box<dyn IndexOutput> =
                dir.create_output(&name, &*DEFAULT_IO_CONTEXT).unwrap();
            out.write_byte(0x2a).unwrap();
            for b in 1..=partial {
                out.write_byte(b).unwrap();
            }
            out.close().unwrap();
        }
        let mut input =
            BufferedChecksumIndexInput::new(dir.open_input(&name, &*DEFAULT_IO_CONTEXT).unwrap());
        input.read_byte().unwrap();
        let outcome = match kind {
            "short" => input.read_short().map(|_| ()),
            "int" => input.read_int().map(|_| ()),
            "long" => input.read_long().map(|_| ()),
            _ => input.read_longs(&mut [0i64; 2], 0, 2),
        };
        assert_eq!(
            if outcome.is_ok() { "read" } else { "eof" },
            r[3],
            "{kind} over {partial} bytes"
        );
        assert_eq!(
            input.get_checksum().unwrap().to_string(),
            r[4],
            "{kind}: the partial value must not be checksummed"
        );
    }
}

// ---------------------------------------------------------------------------
// Lucene90StoredFieldsFormat.impl(Mode)
// ---------------------------------------------------------------------------

/// `impl(mode)` builds the compressing format of each mode with Lucene's
/// exact parameters, which its `toString()` spells out.
#[test]
fn stored_fields_impl_builds_the_format_java_builds() {
    for (mode, name) in [
        (Mode::BestSpeed, "BEST_SPEED"),
        (Mode::BestCompression, "BEST_COMPRESSION"),
    ] {
        let format = Lucene90StoredFieldsFormat::with_mode(mode);
        assert_eq!(
            format.impl_for_mode(mode).to_string(),
            row("sfimpl", name)[0],
            "{name}"
        );
    }
}

// ---------------------------------------------------------------------------
// DocValuesRewriteMethod.equals / hashCode
// ---------------------------------------------------------------------------

/// `hashCode()` is the constant `641`; `equals` holds between any two
/// instances, the `DOC_VALUES_REWRITE` singleton included, and fails against
/// another rewrite method.
#[test]
fn doc_values_rewrite_method_equals_and_hash_as_java() {
    let method = DocValuesRewriteMethod;
    assert_eq!(method.method_hash().to_string(), row("dvrw", "hash")[0]);
    assert_eq!(
        method.method_eq(&DocValuesRewriteMethod).to_string(),
        row("dvrw", "equals_new")[0]
    );
    assert_eq!(
        doc_values_rewrite().method_eq(&method).to_string(),
        row("dvrw", "equals_singleton")[0]
    );
    assert_eq!(
        method
            .method_eq(constant_score_rewrite().as_ref())
            .to_string(),
        row("dvrw", "equals_constant_score")[0]
    );
}

// ---------------------------------------------------------------------------
// ByteBlockPool.setBytesRef
// ---------------------------------------------------------------------------

/// `setBytesRef` references the block directly when the slice fits in it —
/// up to and including the slice that ends on the block's last byte — and
/// copies through the builder otherwise, leaving the builder's length alone.
/// The Java side reports which array the result points into, the offset in
/// it, and the CRC-32 of the slice; the Rust side must agree on all of them.
#[test]
fn byte_block_pool_set_bytes_ref_matches_java() {
    let mut pool = ByteBlockPool::new(Arc::new(AtomicI64::new(0)));
    pool.next_buffer().unwrap();
    let total = 2 * BYTE_BLOCK_SIZE + 100;
    let data: Vec<u8> = (0..total).map(|i| (i % 251) as u8).collect();
    pool.append(&data).unwrap();

    for r in rows("bbp") {
        let offset: i64 = r[1].parse().unwrap();
        let length: usize = r[2].parse().unwrap();
        let mut builder = BytesRefBuilder::new();
        let (where_, result_offset, crc, result_ptr) = {
            let block = pool
                .buffer((offset >> BYTE_BLOCK_SHIFT) as usize)
                .as_ptr_range();
            let slice = pool.set_bytes_ref(&mut builder, offset, length);
            let ptr = slice.as_ptr();
            let where_ = if block.contains(&ptr) {
                "block"
            } else {
                "builder"
            };
            let result_offset = if where_ == "block" {
                (ptr as usize - block.start as usize).to_string()
            } else {
                "0".to_string()
            };
            let mut crc = crc32fast::Hasher::new();
            crc.update(slice);
            (where_, result_offset, crc.finalize(), ptr as usize)
        };
        if where_ == "builder" {
            // The copy lives at the start of the builder's own buffer.
            assert_eq!(
                result_ptr,
                builder.bytes().as_ptr() as usize,
                "offset {offset} length {length}"
            );
        }
        assert_eq!(
            vec![
                where_.to_string(),
                result_offset,
                length.to_string(),
                builder.length().to_string(),
                crc.to_string(),
            ],
            r[3..8].to_vec(),
            "offset {offset} length {length}"
        );
    }
}

// ---------------------------------------------------------------------------
// PhraseMatcher.impactsApproximation, through PhraseScorer
// ---------------------------------------------------------------------------

/// An exact phrase scored in `TOP_SCORES` mode iterates the matcher's
/// impacts-aware approximation, and the scorer reaches it through
/// `impactsApproximation()`: a minimum competitive score above every score
/// makes it jump straight to `NO_MORE_DOCS`, where the plain iteration stops
/// on the first candidate; and the shallow advance and the max score come
/// from its max-score cache. The bits of the max score are Java's.
#[test]
fn phrase_scorer_reaches_the_impacts_approximation_as_java() {
    let _ = register_codec("Lucene104", Lucene104Codec::new());
    let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
    {
        let writer = IndexWriter::new(Arc::clone(&dir), IndexWriterConfig::new()).unwrap();
        for text in ["x y", "a b", "a b a b", "b a"] {
            let mut doc = Document::new();
            doc.add(Box::new(
                TextField::new("body", text.to_string(), Store::NO).unwrap(),
            ));
            writer.add_document(&doc).unwrap();
        }
        writer.commit().unwrap();
        writer.close().unwrap();
    }
    let reader = directory_reader::open(Arc::clone(&dir)).unwrap();
    let searcher = IndexSearcher::new(reader as Arc<dyn rucene::index::IndexReader>).unwrap();
    let phrase: Arc<dyn Query> = Arc::new(
        PhraseQuery::new(
            0,
            vec![
                Term::new("body", BytesRef::new(b"a".to_vec())),
                Term::new("body", BytesRef::new(b"b".to_vec())),
            ],
            vec![0, 1],
        )
        .unwrap(),
    );
    let query = searcher.rewrite(phrase).unwrap();
    let weight = searcher
        .create_weight(query, ScoreMode::TOP_SCORES, 1.0)
        .unwrap();
    let leaf = Arc::clone(&searcher.get_leaf_contexts()[0]);

    let mut first = weight.scorer(&leaf).unwrap().expect("the phrase matches");
    let doc = first
        .two_phase_iterator()
        .expect("a phrase scorer is two-phase")
        .approximation()
        .next_doc()
        .unwrap();
    assert_eq!(doc.to_string(), row("phrase", "approx_first")[0]);

    let mut pruned = weight.scorer(&leaf).unwrap().expect("the phrase matches");
    pruned.set_min_competitive_score(f32::MAX).unwrap();
    let doc = pruned
        .two_phase_iterator()
        .expect("a phrase scorer is two-phase")
        .approximation()
        .next_doc()
        .unwrap();
    assert_eq!(doc, NO_MORE_DOCS);
    assert_eq!(doc.to_string(), row("phrase", "approx_after_min_score")[0]);

    let mut bounds = weight.scorer(&leaf).unwrap().expect("the phrase matches");
    assert_eq!(
        bounds.advance_shallow(0).unwrap().to_string(),
        row("phrase", "advance_shallow_0")[0]
    );
    assert_eq!(
        (bounds.get_max_score(NO_MORE_DOCS).unwrap().to_bits() as i32).to_string(),
        row("phrase", "max_score_bits")[0]
    );
}

// ---------------------------------------------------------------------------
// AbstractAnalysisFactory.getPattern
// ---------------------------------------------------------------------------

/// The Java simple name of the fixture's factory, which Java's message reads
/// off `getClass()`.
const PROBE_FACTORY: &str = "ProbeFactory";

fn unhex(text: &str) -> String {
    let bytes: Vec<u8> = (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).expect("hex byte"))
        .collect();
    String::from_utf8(bytes).expect("UTF-8")
}

fn hex(text: &str) -> String {
    text.bytes().map(|b| format!("{b:02x}")).collect()
}

/// What `getPattern` makes of `pattern`, in the fixture's encoding:
/// `OK|<code-point start>:<hex match>,…` over `sample`, or `ERR|`.
fn pattern_outcome(pattern: &str, sample: &str) -> String {
    let mut args = HashMap::from([("pattern".to_string(), pattern.to_string())]);
    match AbstractAnalysisFactory::get_pattern(&mut args, "pattern", PROBE_FACTORY) {
        Ok(regex) => {
            let found: Vec<String> = regex
                .find_iter(sample)
                .map(|m| {
                    let m = m.expect("match");
                    format!(
                        "{}:{}",
                        sample[..m.start()].chars().count(),
                        hex(m.as_str())
                    )
                })
                .collect();
            format!("OK|{}", found.join(","))
        }
        Err(_) => "ERR|".to_string(),
    }
}

/// The `(pattern, sample)` rows of the fixture's corpus on which
/// `java.util.regex` and `fancy-regex` are measured to differ — the closed
/// list `AbstractAnalysisFactory::get_pattern` declares, in corpus order.
const DECLARED_PATTERN_DIFFERENCES: &[(&str, &str)] = &[
    ("\\0101", "A\u{7}"),
    ("\\07", "\u{7}a"),
    ("\\0377", "\u{ff}a"),
    ("\\0400", " 0"),
    ("\\uD83D\\uDE00", "a\u{1f600}"),
    ("\\N{LATIN SMALL LETTER E WITH ACUTE}", "e\u{e9}"),
    ("\\cA", "x\u{1}y"),
    ("\\c?", "x\u{7f}y"),
    ("[[:alpha:]]+", "ab:12"),
    ("[a--b]", "a-b"),
    ("[~~a]+", "~a~"),
    ("[\\d-z]+", "1-z"),
    (".+", "a\rb\u{85}c\u{2028}d\ne\u{2029}f"),
    ("\\d+", "12\u{663}\u{ff14}"),
    ("\\D+", "12\u{663}\u{ff14}a"),
    ("\\h+", "a \t\u{a0}\u{2003}b\u{3000}c\u{200b}d"),
    ("\\H+", "a \t\u{a0}b"),
    ("\\s+", "a \t\u{b}\u{c}\r\u{a0}\u{2003}\u{85}b"),
    ("\\S+", "a\u{a0}b"),
    ("\\v+", "a\n\u{b}\u{c}\r\u{85}\u{2028}\u{2029}b"),
    ("\\V+", "a\u{2028}b\u{b}c"),
    ("\\w+", "caf\u{e9}_1\u{663}"),
    ("\\W+", "caf\u{e9}_1\u{663}!"),
    ("\\p{Lower}+", "ab\u{e9}z"),
    ("\\p{Upper}+", "AB\u{c9}Z"),
    ("\\p{Alpha}+", "aZ\u{e9}"),
    ("\\p{Digit}+", "19\u{663}"),
    ("\\p{Alnum}+", "a9\u{e9}\u{663}"),
    ("\\p{Punct}+", "!/:@[`{~\u{ab}\u{a1}"),
    ("\\p{Graph}+", "a!\u{e9} b"),
    ("\\p{Print}+", "a \u{e9}"),
    ("\\p{Blank}+", " \t\u{a0}a"),
    ("\\p{Cntrl}+", "\u{1}\u{7f}\u{85}a"),
    ("\\p{XDigit}+", "fF9g\u{ff10}"),
    ("\\p{Space}+", " \t\u{b}\u{a0}a"),
    ("\\P{Lower}+", "abC\u{e9}"),
    ("\\p{javaLowerCase}+", "a\u{aa}\u{e9}A"),
    ("\\p{javaUpperCase}+", "A\u{c9}a"),
    ("\\p{javaWhitespace}+", " \u{a0}\u{2003}\u{1c}a"),
    ("\\p{javaMirrored}+", "()<>\u{ab}a"),
    ("\\p{InGreek}+", "\u{3b1}\u{3b2}a"),
    ("\\P{InGreek}+", "\u{3b1}a"),
    ("\\p{blk=Greek}+", "\u{3b1}a"),
    ("\\p{Latin}+", "a\u{3b1}"),
    ("\\p{Greek}+", "a\u{3b1}"),
    ("(?m)^a", "a\nb\ra\r\na\u{85}a\u{2028}a"),
    ("(?m)^", ""),
    ("(?m)^", "a\n"),
    ("a$", "a\n"),
    ("a$", "a\r\n"),
    ("a$", "a\r"),
    ("a$", "a\u{85}"),
    ("$", "a\r\n"),
    ("(?m)a$", "a\ra\r\na\u{2028}b"),
    ("(?m)$", "a\r\nb"),
    ("\\b\\w+\\b", "caf\u{e9} na\u{ef}ve"),
    ("\\b", "a\u{301}b \u{e9}"),
    ("\\B", "ab \u{e9}"),
    ("\\b{g}", "e\u{301}x"),
    ("b\\Z", "ab\r\n"),
    ("\\R\\n", "\r\n"),
    ("\\X", "e\u{301}x"),
    ("a{,3}", "aaaa"),
    ("{2}", "a"),
    ("a{", "a{"),
    ("a{x}", "a{x}"),
    ("(a)\\2", "aa"),
    ("(a)\\11", "aa1"),
    ("(?<a_b>x)", "x"),
    ("(?<n>a)(?<n>b)", "ab"),
    ("\\Qa.b\\E", "a.b axb"),
    ("\\Q[\\E", "a["),
    ("[\\Qa-c\\E]+", "a-c b"),
    ("\\<", "a<b"),
    ("\\>", "a>b"),
    ("\\Q1\\E", "1"),
    ("(?P<n>a)", "a"),
    ("(?i)\u{e9}", "\u{c9}"),
    ("(?i)k", "\u{212a}"),
    ("(?i)s", "\u{17f}"),
    ("(?i)[a-z]+", "\u{212a}Z"),
    ("(?d).", "\r"),
    ("(?d)a$", "a\r"),
    ("(?x)[a b]+", "ab "),
    ("(?U)\\w+", "caf\u{e9}"),
    ("(?U)a+", "aaa"),
    ("(?R)a$", "a\r\n"),
    ("(?c)a", "a"),
    ("(a)?(?(1)b|c)", "abc"),
    ("a\\Kb", "ab"),
    ("a(?#comment)b", "ab"),
    ("\\u{e9}", "\u{e9}"),
    ("\\U000000E9", "\u{e9}"),
    ("x{2}{3}", "xxxxxx"),
    ("(?=a)*b", "ab"),
    ("a*", "baaa"),
    ("a|", "ba"),
    ("[a&&]", "a"),
    ("(?i)[\u{e0}-\u{ff}]", "\u{c0}"),
    ("\\p{Lowercase}+", "ab\u{aa}c"),
    ("\\p{Alnum}+", "a1\u{e9}"),
    ("(?U)\\p{Alpha}+", "a\u{e9}"),
    ("\\p{all}+", "ab"),
    ("(?i)\\x{212a}", "k"),
    ("\\s*", "a b"),
    (",?", "a,b"),
    ("x*", "xxa"),
    ("\\d*", "12ab3"),
    ("(?=b)|a", "ab"),
    ("x*", "a\u{1f600}b"),
    ("", "\u{1f600}"),
];

/// The dialect contract of `get_pattern`, measured row by row against
/// `java.util.regex`: every corpus row not declared above must compile and
/// match exactly as Java does (same acceptance, same matches at the same
/// code-point positions), and every declared row must still differ. A new
/// difference fails the first half; a declared one that closes — a
/// `fancy-regex` upgrade, say — fails the second, until the declaration is
/// updated.
#[test]
fn pattern_dialect_differences_are_exactly_the_declared_ones() {
    let mut rows_seen = 0;
    let mut declared_seen = 0;
    for r in rows("pat").into_iter().filter(|r| r[1] == "row") {
        rows_seen += 1;
        let (pattern, sample) = (unhex(&r[2]), unhex(&r[3]));
        let java = format!("{}|{}", r[4], r.get(5).map_or("", String::as_str));
        let rust = pattern_outcome(&pattern, &sample);
        if DECLARED_PATTERN_DIFFERENCES.contains(&(pattern.as_str(), sample.as_str())) {
            declared_seen += 1;
            assert_ne!(
                rust, java,
                "declared difference closed: {pattern:?} over {sample:?}"
            );
        } else {
            assert_eq!(
                rust, java,
                "undeclared difference: {pattern:?} over {sample:?}"
            );
        }
    }
    assert_eq!(rows_seen, 207, "the corpus size");
    assert_eq!(
        declared_seen,
        DECLARED_PATTERN_DIFFERENCES.len(),
        "every declared difference is a corpus row"
    );
}

/// A pattern that does not parse is refused with Java's exact message, and
/// the argument is consumed either way; a missing argument fails as
/// `require` does (`AbstractAnalysisFactory.java:119-125,265-277`).
#[test]
fn pattern_errors_carry_java_messages() {
    for r in rows("pat").into_iter().filter(|r| r[1] == "reject") {
        let pattern = unhex(&r[2]);
        let mut args = HashMap::from([("pattern".to_string(), pattern.clone())]);
        match AbstractAnalysisFactory::get_pattern(&mut args, "pattern", PROBE_FACTORY) {
            Err(rucene::error::LuceneError::IllegalArgument(message)) => {
                assert_eq!(message, r[3], "{pattern:?}")
            }
            other => panic!("{pattern:?} must be refused, got {other:?}"),
        }
        assert_eq!(args.len().to_string(), r[4], "{pattern:?} is consumed");
    }

    let mut empty = HashMap::new();
    match AbstractAnalysisFactory::get_pattern(&mut empty, "pattern", PROBE_FACTORY) {
        Err(rucene::error::LuceneError::IllegalArgument(message)) => {
            assert_eq!(message, row("pat", "missing")[0])
        }
        other => panic!("a missing argument must be refused, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// TopFieldCollectorManager.getCollectors
// ---------------------------------------------------------------------------

/// `getCollectors()` returns the collectors the manager created, and they are
/// the very collectors the search filled (`TopFieldCollectorManager.java:173-198`):
/// their hit counts add up to the reduced total, and `reduce` has drained
/// their queues through `topDocs()`, as Java's does on the same objects. A
/// collector created afterwards is recorded too, and collecting into it shows
/// through the handle the manager returns.
#[test]
#[allow(deprecated)]
fn top_field_collector_manager_returns_the_collectors_it_created() {
    let _ = register_codec("Lucene104", Lucene104Codec::new());
    let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
    {
        let writer = IndexWriter::new(Arc::clone(&dir), IndexWriterConfig::new()).unwrap();
        for text in ["a", "a b", "b", "a a", "c"] {
            let mut doc = Document::new();
            doc.add(Box::new(
                TextField::new("body", text.to_string(), Store::NO).unwrap(),
            ));
            writer.add_document(&doc).unwrap();
        }
        writer.commit().unwrap();
        writer.close().unwrap();
    }
    let reader = directory_reader::open(Arc::clone(&dir)).unwrap();
    let searcher = IndexSearcher::new(reader as Arc<dyn rucene::index::IndexReader>).unwrap();
    let query: Arc<dyn Query> = Arc::new(TermQuery::new(Term::new(
        "body",
        BytesRef::new(b"a".to_vec()),
    )));

    // `IndexSearcher.search(Query, CollectorManager)` by hand: one collector
    // per slice — this reader has one — then `reduce`.
    let manager = TopFieldCollectorManager::with_num_hits(Sort::new(), 10, i32::MAX).unwrap();
    assert!(manager.get_collectors().is_empty());
    let mut slice = CollectorManager::new_collector(&manager).unwrap();
    searcher
        .search_with_collector(Arc::clone(&query), &mut slice)
        .unwrap();
    let docs = manager.reduce(vec![slice]).unwrap();
    assert_eq!(docs.total_hits.value(), 3);

    let collectors = manager.get_collectors();
    assert!(
        !collectors.is_empty(),
        "the search's collectors are recorded"
    );
    let collected: i32 = collectors.iter().map(|c| c.total_hits()).sum();
    assert_eq!(
        i64::from(collected),
        docs.total_hits.value(),
        "the recorded collectors are the ones the search filled"
    );
    for collector in &collectors {
        assert_eq!(collector.pq_size(), 0, "reduce drained this collector");
        assert!(!collector.is_early_terminated());
    }

    let mut extra = CollectorManager::new_collector(&manager).unwrap();
    searcher
        .search_with_collector(Arc::clone(&query), &mut extra)
        .unwrap();
    let recorded = manager.get_collectors();
    assert_eq!(recorded.len(), collectors.len() + 1);
    let last = recorded.last().expect("the extra collector is recorded");
    assert_eq!(last.total_hits(), 3, "the handle sees what was collected");
    assert_eq!(last.pq_size(), 3);
}

// ---------------------------------------------------------------------------
// CharArrayMap.createEntrySet
// ---------------------------------------------------------------------------

/// The entry set of a modifiable map replaces values through `setValue`,
/// returning the old one; an unmodifiable map's entry set refuses both
/// `setValue` and `clear` (`CharArrayMap.java:322-324,455-460,505-509,598-601`).
/// Java's `unmodifiableMap` shares the arrays of the map it wraps, so its
/// rows are taken before the modifiable map is cleared; this port's
/// `unmodifiable_map` takes the map, so it is given a copy.
#[test]
fn char_array_map_entry_set_writes_as_java() {
    let get = |what: &str| row("cam", what)[0].clone();
    let mut map: CharArrayMap<i32> = CharArrayMap::new(4, false);
    map.put("a", 1);
    map.put("b", 2);
    assert_eq!(map.create_entry_set().size().to_string(), get("size"));

    let set_a = {
        let mut entries = map.create_entry_set();
        let mut entry = entries
            .iter_mut()
            .find(|e| e.get_key() == ['a'])
            .expect("a is mapped");
        let old = entry.set_value(10).unwrap();
        format!("old={old},entry={entry}")
    };
    assert_eq!(set_a, get("set_a"));
    assert_eq!(map.get("a").unwrap().to_string(), get("get_a"));
    assert_eq!(
        map.create_entry_set().contains(&['a'], &10).to_string(),
        get("contains_a10")
    );
    assert_eq!(
        map.create_entry_set().contains(&['a'], &1).to_string(),
        get("contains_a1")
    );

    let mut read_only = CharArrayMap::unmodifiable_map(map.copy());
    let set_ro = {
        let mut entries = read_only.create_entry_set();
        let mut entry = entries
            .iter_mut()
            .find(|e| e.get_key() == ['b'])
            .expect("b is mapped");
        match entry.set_value(20) {
            Err(rucene::error::LuceneError::UnsupportedOperation(_)) => {
                "UnsupportedOperationException".to_string()
            }
            other => format!("{other:?}"),
        }
    };
    assert_eq!(set_ro, get("set_ro"));
    assert_eq!(read_only.get("b").unwrap().to_string(), get("get_b"));
    let clear_ro = match read_only.create_entry_set().clear() {
        Err(rucene::error::LuceneError::UnsupportedOperation(_)) => "UnsupportedOperationException",
        Err(_) => "other error",
        Ok(()) => "cleared",
    };
    assert_eq!(clear_ro, get("clear_ro"));

    map.create_entry_set().clear().unwrap();
    assert_eq!(map.size().to_string(), get("cleared_size"));
}
