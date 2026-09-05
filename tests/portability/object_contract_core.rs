//! Portability tests for the inherited `Object` contracts — `toString()`,
//! `equals()`, `hashCode()` — that Apache Lucene Core 10.5.0 declares on the
//! types this crate ports.
//!
//! Nothing here writes an index file, so nothing here is a byte comparison.
//! What it pins is the other half of a port's fidelity: the strings a Lucene
//! object produces for itself, and the identity it answers with. Those strings
//! are observable — `CheckIndex` output, `IndexWriter` info-stream messages,
//! exception text, Luke — so reproducing the *format* matters, not merely
//! producing some string.
//!
//! Every expected value is **measured from `lucene-core-10.5.0.jar`** by the
//! `ObjectContractsFixture` harness in the same run, never derived from this
//! crate's own formatting: asserting an implementation against itself is
//! exactly what lets a dropped field, a `, ` that should be `,`, or a
//! ties-to-even rounding go unnoticed.
//!
//! Several of Lucene's strings are deliberately odd, and the fixture proves
//! each of them is what Java really emits:
//!
//! * `SparseFixedBitSet.toString()` and `TieredMergePolicy.toString()` never
//!   close the bracket they open.
//! * `ByteBuffersDataInput.toString()` reports `block size: 1` for a
//!   single-block input, because Java sets `blockBits = 32` and `1 << 32` on an
//!   `int` is `1`.
//! * `DenseLiveDocs` renders its deletion rate through `String.format("%.2f%%")`,
//!   whose ties round *away from zero* where Rust's `{:.2}` rounds to even.
//! * `NeighborQueue` names itself `Neighbors`, `FieldReader` names itself
//!   `BlockTreeTerms`, and `SegmentCoreReaders` names itself
//!   `SegmentCoreReader`.
//!
//! The fixture runs with `-Dtests.seed=DEADBEEF` so that
//! `StringHelper.GOOD_FAST_HASH_SEED` — and therefore `BytesRef.hashCode()` —
//! is reproducible; it is drawn from `System.currentTimeMillis()` otherwise, in
//! Java exactly as here.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Arc;

use rucene::analysis::char_array_set::CharArraySet;
use rucene::analysis::tokenattributes::{CharTermAttribute, CharTermAttributeImpl};
use rucene::codecs::compressing::CompressionMode;
use rucene::codecs::hnsw::flat_vectors::DefaultFlatVectorScorer;
use rucene::codecs::hnsw::scalar_quantized_scorer::ScalarQuantizedVectorScorer;
use rucene::codecs::lucene104::scalar_quantized_vectors::Lucene104HnswScalarQuantizedVectorsFormat;
use rucene::codecs::lucene90::stored_fields::Lucene90CompressingStoredFieldsFormat;
use rucene::codecs::lucene90::term_vectors::Lucene90CompressingTermVectorsFormat;
use rucene::codecs::lucene99::flat_vectors_format::Lucene99FlatVectorsFormat;
use rucene::codecs::lucene99::hnsw_vectors_format::Lucene99HnswVectorsFormat;
use rucene::codecs::term_state::{BlockTermState, CompetitiveImpactAccumulator, Impact};
use rucene::index::merge_policy::{
    LogMergePolicy, MergePolicy, NoMergePolicy, TieredMergePolicy, UpgradeIndexMergePolicy,
};
use rucene::index::terms::{OrdTermState, PrefixCodedTermsBuilder};
use rucene::index::Term;
use rucene::store::{
    ByteBuffersDataInput, ByteBuffersDataOutput, ByteBuffersDirectory, DataOutput, Directory,
    NRTCachingDirectory,
};
use rucene::util::automaton::automata::Automata;
use rucene::util::automaton::byte_run_automaton::ByteRunAutomaton;
use rucene::util::bit_set::BitSet;
use rucene::util::bit_sets::{DenseLiveDocs, RoaringDocIdSet, SparseFixedBitSet, SparseLiveDocs};
use rucene::util::bkd::{BKDConfig, HeapPointWriter, OfflinePointWriter};
use rucene::util::doc_id_set::BitDocIdSet;
use rucene::util::extra::LongBitSet;
use rucene::util::hnsw::neighbor::{NeighborArray, NeighborQueue};
use rucene::util::hnsw::on_heap::OnHeapHnswGraph;
use rucene::util::packed::block_packed::{MonotonicBlockPackedReader, MonotonicBlockPackedWriter};
use rucene::util::packed::paged_mutable::{PagedGrowableWriter, PagedMutable};
use rucene::util::packed::PackedInts;
use rucene::util::string_helper::{IntsRef, StringHelper};
use rucene::util::{BytesRef, FixedBitSet};

/// The seed handed to both sides, so `GOOD_FAST_HASH_SEED` is reproducible.
///
/// It has to look like hexadecimal: the Java harness's classpath contains a
/// component that parses `tests.seed` with radix 16.
const TESTS_SEED: &str = "DEADBEEF";

fn harness_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
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

/// Runs the Java harness and returns its `key=value` lines.
fn run_fixture() -> Vec<(String, String)> {
    let mvn = which_mvn().unwrap_or_else(|e| {
        panic!("the object-contracts portability test needs Maven and a JDK: {e}")
    });
    let output = Command::new(mvn)
        .arg("-q")
        .arg("compile")
        .arg("exec:java")
        .arg("-Dexec.mainClass=org.apache.lucene.rucene.codec.ObjectContractsFixture")
        .arg(format!("-Dtests.seed={TESTS_SEED}"))
        .current_dir(harness_dir())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn Maven: {e}"));
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        output.status.success() && stdout.lines().any(|l| l.trim() == "emitted=true"),
        "the Java fixture failed:\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    stdout
        .lines()
        .filter_map(|line| line.split_once('='))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

struct Measured(Vec<(String, String)>);

impl Measured {
    fn get(&self, key: &str) -> &str {
        &self
            .0
            .iter()
            .find(|(k, _)| k == key)
            .unwrap_or_else(|| panic!("the Java fixture printed no `{key}`"))
            .1
    }

    fn int(&self, key: &str) -> i32 {
        self.get(key)
            .parse()
            .unwrap_or_else(|e| panic!("`{key}` is not an i32: {e}"))
    }

    fn boolean(&self, key: &str) -> bool {
        self.get(key)
            .parse()
            .unwrap_or_else(|e| panic!("`{key}` is not a bool: {e}"))
    }
}

fn hash_of<T: Hash>(value: &T) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

#[test]
fn object_contracts_match_lucene() {
    // Set before anything in this binary can read the seed: `GOOD_FAST_HASH_SEED`
    // is drawn once per process, exactly as Java's static initialiser draws it
    // once per JVM.
    std::env::set_var("tests.seed", TESTS_SEED);

    let measured = Measured(run_fixture());

    util_bit_sets(&measured);
    util_refs(&measured);
    util_bkd(&measured);
    util_hnsw(&measured);
    util_packed(&measured);
    util_automaton(&measured);
    store(&measured);
    codecs(&measured);
    index(&measured);
    analysis(&measured);
}

fn util_bit_sets(m: &Measured) {
    let empty = SparseFixedBitSet::new(100);
    assert_eq!(
        empty.to_string(),
        m.get("SparseFixedBitSet.toString/empty100"),
        "SparseFixedBitSet.toString() — note Lucene never closes the parenthesis"
    );

    let mut sparse = SparseFixedBitSet::new(10_000);
    sparse.set(1);
    sparse.set(5_000);
    sparse.set(9_999);
    assert_eq!(
        sparse.to_string(),
        m.get("SparseFixedBitSet.toString/three-of-10000")
    );

    let mut builder = RoaringDocIdSet::builder(1000);
    builder.add(3).expect("add 3");
    builder.add(17).expect("add 17");
    builder.add(999).expect("add 999");
    assert_eq!(
        builder.build().to_string(),
        m.get("RoaringDocIdSet.toString/three-of-1000")
    );
    assert_eq!(
        RoaringDocIdSet::builder(64).build().to_string(),
        m.get("RoaringDocIdSet.toString/empty64")
    );

    let mut for_doc_id_set = SparseFixedBitSet::new(128);
    for_doc_id_set.set(7);
    let set: Arc<dyn BitSet> = Arc::new(for_doc_id_set);
    assert_eq!(
        BitDocIdSet::new(set, 1).expect("cost >= 0").to_string(),
        m.get("BitDocIdSet.toString/sparse128"),
        "the wrapped set is rendered by its own toString(), unclosed parenthesis included"
    );

    let mut live = FixedBitSet::new(200);
    live.set_range(0, 200);
    live.clear(3);
    live.clear(9);
    live.clear(150);
    assert_eq!(
        DenseLiveDocs::builder(live, 200)
            .with_deleted_count(3)
            .build()
            .to_string(),
        m.get("DenseLiveDocs.toString/3-of-200")
    );

    let mut all_live = FixedBitSet::new(8);
    all_live.set_range(0, 8);
    assert_eq!(
        DenseLiveDocs::builder(all_live, 8)
            .with_deleted_count(0)
            .build()
            .to_string(),
        m.get("DenseLiveDocs.toString/0-of-8")
    );

    // 1/8 is 12.5%: the tie that separates Java's HALF_UP from Rust's
    // ties-to-even, which would render `12.5` as `12.50` either way but
    // disagrees one digit further down for other inputs. Kept because it is the
    // exact case the helper exists for.
    let mut one_deleted = FixedBitSet::new(8);
    one_deleted.set_range(0, 8);
    one_deleted.clear(4);
    assert_eq!(
        DenseLiveDocs::builder(one_deleted, 8)
            .with_deleted_count(1)
            .build()
            .to_string(),
        m.get("DenseLiveDocs.toString/1-of-8-tie")
    );

    let mut deleted = SparseFixedBitSet::new(1000);
    deleted.set(4);
    deleted.set(77);
    assert_eq!(
        SparseLiveDocs::builder(deleted, 1000)
            .with_deleted_count(2)
            .build()
            .to_string(),
        m.get("SparseLiveDocs.toString/2-of-1000")
    );

    let mut one_of_eight = SparseFixedBitSet::new(8);
    one_of_eight.set(4);
    assert_eq!(
        SparseLiveDocs::builder(one_of_eight, 8)
            .with_deleted_count(1)
            .build()
            .to_string(),
        m.get("SparseLiveDocs.toString/1-of-8-tie")
    );

    // LongBitSet.hashCode(), and its consistency with equality.
    let lb_empty = LongBitSet::new(64).expect("64 bits");
    assert_eq!(
        lb_empty.hash_code(),
        m.int("LongBitSet.hashCode/empty64"),
        "an empty set must not hash to zero — that is what the 0x98761234 is for"
    );

    let mut lb = LongBitSet::new(200).expect("200 bits");
    lb.set(0);
    lb.set(63);
    lb.set(64);
    lb.set(199);
    assert_eq!(lb.hash_code(), m.int("LongBitSet.hashCode/four-of-200"));

    let mut lb_one = LongBitSet::new(1).expect("1 bit");
    lb_one.set(0);
    assert_eq!(lb_one.hash_code(), m.int("LongBitSet.hashCode/one-of-1"));

    // PartialEq and Hash agree: equal sets hash equally, and a set that differs
    // in one bit hashes differently.
    let mut same = LongBitSet::new(200).expect("200 bits");
    same.set(0);
    same.set(63);
    same.set(64);
    same.set(199);
    assert_eq!(lb, same, "same bits, same length");
    assert_eq!(
        hash_of(&lb),
        hash_of(&same),
        "Hash must agree with PartialEq for LongBitSet"
    );
    let mut different = same.clone();
    different.set(100);
    assert_ne!(lb, different);
    assert_ne!(hash_of(&lb), hash_of(&different));
}

fn util_refs(m: &Measured) {
    assert_eq!(
        IntsRef::default().to_string(),
        m.get("IntsRef.toString/empty")
    );
    let values = vec![0, 1, 15, 16, 255, -1, i32::MIN, i32::MAX];
    assert_eq!(
        IntsRef::new(values.clone()).to_string(),
        m.get("IntsRef.toString/eight"),
        "Integer.toHexString is unsigned, so -1 renders as ffffffff"
    );
    let slice = IntsRef {
        ints: values,
        offset: 2,
        length: 3,
    };
    assert_eq!(slice.to_string(), m.get("IntsRef.toString/slice"));

    // BytesRef.hashCode(): the seeded MurmurHash3 value.
    assert_eq!(
        StringHelper::good_fast_hash_seed(),
        m.int("StringHelper.GOOD_FAST_HASH_SEED"),
        "the seed must be String.hashCode() of the tests.seed property"
    );
    assert_eq!(
        BytesRef::new(Vec::new()).hash_code(),
        m.int("BytesRef.hashCode/empty")
    );
    assert_eq!(
        BytesRef::new(b"abc".to_vec()).hash_code(),
        m.int("BytesRef.hashCode/abc")
    );
    let wide = vec![0u8, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
    assert_eq!(
        BytesRef::new(wide.clone()).hash_code(),
        m.int("BytesRef.hashCode/eleven")
    );
    let sliced = BytesRef {
        bytes: wide,
        offset: 3,
        length: 5,
    };
    assert_eq!(sliced.hash_code(), m.int("BytesRef.hashCode/slice"));

    // Hash agrees with PartialEq: two BytesRef with the same active slice at
    // different offsets are equal and hash equally.
    let a = BytesRef::new(b"hello".to_vec());
    let b = BytesRef {
        bytes: b"XXhelloYY".to_vec(),
        offset: 2,
        length: 5,
    };
    assert_eq!(a, b, "BytesRef equality is over the active slice");
    assert_eq!(
        hash_of(&a),
        hash_of(&b),
        "Hash must agree with PartialEq for BytesRef"
    );
    assert_eq!(
        a.hash_code(),
        b.hash_code(),
        "and so must the Java-exact hash_code()"
    );
    assert_ne!(a, BytesRef::new(b"hellp".to_vec()));
    assert_ne!(hash_of(&a), hash_of(&BytesRef::new(b"hellp".to_vec())));
}

fn util_bkd(m: &Measured) {
    let config = BKDConfig::of(1, 1, 4, 512).expect("valid config");
    let heap = HeapPointWriter::new(config.clone(), 10).expect("heap writer");
    assert_eq!(heap.to_string(), m.get("HeapPointWriter.toString/empty10"));

    let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
    let offline = OfflinePointWriter::new(config, dir, "prefix", "desc").expect("offline writer");
    assert_eq!(
        offline.to_string(),
        m.get("OfflinePointWriter.toString/fresh"),
        "the name has to come from createTempOutput, counter and all"
    );
}

fn util_hnsw(m: &Measured) {
    assert_eq!(
        NeighborArray::new(10, true).to_string(),
        m.get("NeighborArray.toString/empty")
    );
    let mut array = NeighborArray::new(10, true);
    array.add_in_order(1, 0.9).expect("in order");
    array.add_in_order(2, 0.8).expect("in order");
    assert_eq!(array.to_string(), m.get("NeighborArray.toString/two"));

    assert_eq!(
        NeighborQueue::new(4, true).to_string(),
        m.get("NeighborQueue.toString/empty"),
        "NeighborQueue names itself Neighbors, not after its own class"
    );
    let mut queue = NeighborQueue::new(4, true);
    queue.add(1, 0.5);
    queue.add(2, 0.6);
    queue.add(3, 0.7);
    assert_eq!(queue.to_string(), m.get("NeighborQueue.toString/three"));

    assert_eq!(
        OnHeapHnswGraph::new(16, 0).to_string(),
        m.get("OnHeapHnswGraph.toString/empty")
    );
    let mut graph = OnHeapHnswGraph::new(16, 8);
    graph.add_node(0, 0);
    graph.add_node(0, 1);
    graph.add_node(0, 2);
    graph.add_node(1, 2);
    assert_eq!(
        graph.to_string(),
        m.get("OnHeapHnswGraph.toString/threeNodesTwoLevels")
    );
}

fn util_packed(m: &Measured) {
    assert_eq!(
        PagedMutable::new(1000, 1024, 8, PackedInts::COMPACT)
            .expect("paged mutable")
            .to_string(),
        m.get("PagedMutable.toString/1000-pageSize1024"),
        "Java's final AbstractPagedMutable.toString() names the concrete subclass"
    );
    assert_eq!(
        PagedGrowableWriter::new(1000, 1024, 8, PackedInts::COMPACT)
            .expect("paged growable")
            .to_string(),
        m.get("PagedGrowableWriter.toString/1000-pageSize1024")
    );

    let mut out = ByteBuffersDataOutput::new();
    {
        let mut writer = MonotonicBlockPackedWriter::new(&mut out, 64).expect("writer");
        for i in 0..200i64 {
            writer.add(i * 3).expect("add");
        }
        writer.finish().expect("finish");
    }
    let mut input = out.to_data_input().expect("data input");
    let reader = MonotonicBlockPackedReader::of(&mut input, PackedInts::VERSION_CURRENT, 64, 200)
        .expect("reader");
    assert_eq!(
        reader.to_string(),
        m.get("MonotonicBlockPackedReader.toString/200-block64")
    );
}

fn util_automaton(m: &Measured) {
    let empty = ByteRunAutomaton::new(Automata::make_empty(), false).expect("empty automaton");
    assert_eq!(
        empty.run_automaton().hash_code(),
        m.int("RunAutomaton.hashCode/empty")
    );

    let abc = ByteRunAutomaton::new(Automata::make_string("abc"), false).expect("abc automaton");
    assert_eq!(
        abc.run_automaton().hash_code(),
        m.int("RunAutomaton.hashCode/abc")
    );

    let any = ByteRunAutomaton::new(Automata::make_any_string(), false).expect("any automaton");
    assert_eq!(
        any.run_automaton().hash_code(),
        m.int("RunAutomaton.hashCode/anyString")
    );

    // Hash agrees with PartialEq.
    let abc_again =
        ByteRunAutomaton::new(Automata::make_string("abc"), false).expect("abc automaton");
    assert_eq!(abc.run_automaton(), abc_again.run_automaton());
    assert_eq!(
        hash_of(abc.run_automaton()),
        hash_of(abc_again.run_automaton()),
        "Hash must agree with PartialEq for RunAutomaton"
    );
    assert_ne!(abc.run_automaton(), any.run_automaton());
    assert_ne!(
        hash_of(abc.run_automaton()),
        hash_of(any.run_automaton()),
        "these two differ in every hashed field"
    );
}

fn store(m: &Measured) {
    let fresh = ByteBuffersDataOutput::new();
    assert_eq!(
        fresh.to_string(),
        m.get("ByteBuffersDataOutput.toString/fresh")
    );

    let mut written = ByteBuffersDataOutput::new();
    written
        .write_bytes(&vec![0u8; 1500], 0, 1500)
        .expect("write");
    assert_eq!(
        written.to_string(),
        m.get("ByteBuffersDataOutput.toString/1500bytes")
    );

    let mut big = ByteBuffersDataOutput::new();
    big.write_bytes(&vec![0u8; 1_234_567], 0, 1_234_567)
        .expect("write");
    assert_eq!(
        big.to_string(),
        m.get("ByteBuffersDataOutput.toString/1234567bytes"),
        "the block size grows with the content, and %,d groups the digits"
    );

    let single = ByteBuffersDataInput::new(vec![vec![0u8; 1500]]).expect("single block");
    assert_eq!(
        single.to_string(),
        m.get("ByteBuffersDataInput.toString/single-1500"),
        "a one-block input reports `block size: 1`, because Java's `1 << 32` is 1"
    );

    let blocks = vec![vec![0u8; 1024], vec![0u8; 1024], vec![0u8; 7]];
    let mut multi = ByteBuffersDataInput::new(blocks).expect("three blocks");
    assert_eq!(
        multi.to_string(),
        m.get("ByteBuffersDataInput.toString/three-1024")
    );
    multi.seek(1030).expect("seek");
    assert_eq!(
        multi.to_string(),
        m.get("ByteBuffersDataInput.toString/three-1024-pos1030")
    );

    // NRTCachingDirectory renders the wrapped directory with an identity hash,
    // which no port can reproduce, so only the portable suffix is compared.
    let nrt = NRTCachingDirectory::new(Box::new(ByteBuffersDirectory::new()), 5.0, 60.0);
    let rendered = nrt.to_string();
    let suffix = &rendered[rendered
        .find("; maxCacheMB=")
        .expect("the suffix Lucene appends")..];
    assert_eq!(suffix, m.get("NRTCachingDirectory.toString/5-60/suffix"));

    let half = NRTCachingDirectory::new(Box::new(ByteBuffersDirectory::new()), 0.5, 48.0);
    let rendered = half.to_string();
    let suffix = &rendered[rendered
        .find("; maxCacheMB=")
        .expect("the suffix Lucene appends")..];
    assert_eq!(
        suffix,
        m.get("NRTCachingDirectory.toString/0.5-48/suffix"),
        "the megabyte figures go through Double.toString, so 48 prints as 48.0"
    );
}

fn codecs(m: &Measured) {
    assert_eq!(Impact::new(1, 1).to_string(), m.get("Impact.toString/1-1"));
    assert_eq!(
        Impact::new(2_000_000_000, i64::MIN).to_string(),
        m.get("Impact.toString/big")
    );

    assert_eq!(
        CompetitiveImpactAccumulator::default().to_string(),
        m.get("CompetitiveImpactAccumulator.toString/empty")
    );
    let mut acc = CompetitiveImpactAccumulator::default();
    acc.add(3, 5);
    acc.add(10, 40);
    acc.add(7, 12);
    assert_eq!(
        acc.to_string(),
        m.get("CompetitiveImpactAccumulator.toString/three"),
        "the pairs render as a java.util.List does: `[a, b, c]`"
    );

    let fresh = BlockTermState::default();
    assert_eq!(
        fresh.to_string(),
        m.get("BlockTermState.toString/fresh"),
        "BlockTermState.toString() does not call super, so `ord` never appears"
    );
    let filled = BlockTermState {
        ord: 42,
        doc_freq: 7,
        total_term_freq: 19,
        term_block_ord: 3,
        block_file_pointer: 1234,
        ..BlockTermState::default()
    };
    assert_eq!(filled.to_string(), m.get("BlockTermState.toString/filled"));

    let default_scorer = DefaultFlatVectorScorer::INSTANCE;
    assert_eq!(
        default_scorer.to_string(),
        m.get("DefaultFlatVectorScorer.toString")
    );
    assert_eq!(
        ScalarQuantizedVectorScorer::new(Box::new(default_scorer)).to_string(),
        m.get("ScalarQuantizedVectorScorer.toString/default-delegate")
    );
    assert_eq!(
        Lucene99FlatVectorsFormat::new(default_scorer).to_string(),
        m.get("Lucene99FlatVectorsFormat.toString/default-scorer")
    );
    assert_eq!(
        Lucene99HnswVectorsFormat::new().to_string(),
        m.get("Lucene99HnswVectorsFormat.toString/default")
    );
    assert_eq!(
        Lucene99HnswVectorsFormat::with_max_conn_beam_width(32, 200).to_string(),
        m.get("Lucene99HnswVectorsFormat.toString/32-200")
    );

    assert_eq!(
        Lucene90CompressingStoredFieldsFormat::new(
            "Lucene90StoredFieldsFastData",
            CompressionMode::FAST,
            1 << 14,
            128,
            10,
        )
        .expect("format")
        .to_string(),
        m.get("Lucene90CompressingStoredFieldsFormat.toString/best-speed")
    );
    assert_eq!(
        Lucene90CompressingTermVectorsFormat::new(
            "Lucene90TermVectorsData",
            "",
            CompressionMode::FAST,
            1 << 12,
            128,
            10,
        )
        .expect("format")
        .to_string(),
        m.get("Lucene90CompressingTermVectorsFormat.toString/fast"),
        "the last field is named blockSize although the value is the block shift"
    );

    assert_eq!(
        CompressionMode::DEFLATE_WITH_PRESET_DICT.to_string(),
        m.get("DeflateWithPresetDictCompressionMode.toString"),
        "Lucene's DeflateWithPresetDictCompressionMode calls itself BEST_COMPRESSION"
    );
    assert_eq!(
        CompressionMode::LZ4_WITH_PRESET_DICT.to_string(),
        m.get("LZ4WithPresetDictCompressionMode.toString")
    );

    assert_eq!(
        Lucene104HnswScalarQuantizedVectorsFormat::new().to_string(),
        m.get("Lucene104HnswScalarQuantizedVectorsFormat.toString/default"),
        "the nested flat format renders its static scorer and raw format too, \
         and the encoding renders as the Java enum constant name"
    );
}

fn index(m: &Measured) {
    let mut ord = OrdTermState::new();
    assert_eq!(ord.to_string(), m.get("OrdTermState.toString/default"));
    ord.ord = 987_654_321;
    assert_eq!(ord.to_string(), m.get("OrdTermState.toString/987654321"));

    assert_eq!(
        NoMergePolicy::default().to_string(),
        m.get("NoMergePolicy.toString")
    );
    assert_eq!(
        TieredMergePolicy::new().to_string(),
        m.get("TieredMergePolicy.toString/default"),
        "Lucene opens a bracket it never closes, and this reproduces that"
    );

    let mut tuned = TieredMergePolicy::new();
    tuned.set_max_merge_at_once(4).expect("maxMergeAtOnce");
    tuned
        .set_max_merged_segment_mb(512.0)
        .expect("maxMergedSegmentMB");
    tuned.set_floor_segment_mb(4.0).expect("floorSegmentMB");
    tuned.set_segments_per_tier(6.0).expect("segmentsPerTier");
    tuned
        .set_deletes_pct_allowed(25.0)
        .expect("deletesPctAllowed");
    tuned
        .set_force_merge_deletes_pct_allowed(5.0)
        .expect("forceMergeDeletesPctAllowed");
    tuned.set_no_cfs_ratio(0.5).expect("noCFSRatio");
    tuned
        .set_max_cfs_segment_size_mb(64.0)
        .expect("maxCFSSegmentSizeMB");
    tuned
        .set_target_search_concurrency(3)
        .expect("targetSearchConcurrency");
    assert_eq!(tuned.to_string(), m.get("TieredMergePolicy.toString/tuned"));

    assert_eq!(
        LogMergePolicy::by_byte_size().to_string(),
        m.get("LogByteSizeMergePolicy.toString/default"),
        "the port models the two Log policies as one struct, so the class name \
         comes from the size kind"
    );
    assert_eq!(
        LogMergePolicy::by_doc_count().to_string(),
        m.get("LogDocMergePolicy.toString/default")
    );

    let no_merge: Arc<dyn MergePolicy> = Arc::new(NoMergePolicy::default());
    assert_eq!(
        UpgradeIndexMergePolicy::new(Arc::clone(&no_merge)).to_string(),
        m.get("FilterMergePolicy.toString/UpgradeIndexMergePolicy-over-NoMergePolicy"),
        "FilterMergePolicy's subclasses inherit its toString() with their own name"
    );

    // PrefixCodedTerms: equals and a Hash consistent with it.
    let build = |terms: &[&str]| {
        let mut b = PrefixCodedTermsBuilder::new();
        for t in terms {
            b.add(&Term::from_text("f", t)).expect("add term");
        }
        b.finish()
    };
    let first = build(&["aaa", "bbb"]);
    let second = build(&["aaa", "bbb"]);
    let third = build(&["aaa"]);
    assert_eq!(
        first == second,
        m.boolean("PrefixCodedTerms.equals/same-terms")
    );
    assert_eq!(
        first == third,
        m.boolean("PrefixCodedTerms.equals/different-terms")
    );
    assert_eq!(
        hash_of(&first) == hash_of(&second),
        m.boolean("PrefixCodedTerms.hashCode/consistent-with-equals"),
        "Hash must agree with PartialEq for PrefixCodedTerms"
    );
    let mut other_gen = build(&["aaa", "bbb"]);
    other_gen.set_del_gen(9);
    assert_eq!(
        first == other_gen,
        m.boolean("PrefixCodedTerms.equals/different-delGen"),
        "delGen is part of the identity"
    );
    assert_ne!(hash_of(&first), hash_of(&other_gen));

    // Sorter.toString() is getID() verbatim; this port takes the id directly.
    let java_id = m.get("Sorter.getID/price-long");
    assert_eq!(
        rucene::index::index_sorter::Sorter::new(java_id).to_string(),
        m.get("Sorter.toString/price-long")
    );
}

fn analysis(m: &Measured) {
    let mut set = CharArraySet::new(3, false);
    set.add("alpha");
    set.add("beta");
    set.add("gamma");
    let rendered = set.to_string();
    let mut parts: Vec<&str> = rendered[1..rendered.len() - 1].split(", ").collect();
    parts.sort_unstable();
    assert_eq!(
        format!("[{}]", parts.join(", ")),
        m.get("CharArraySet.toString/three-sorted"),
        "the elements match; the order is the hash-table slot order on both sides"
    );
    assert_eq!(
        CharArraySet::new(0, false).to_string(),
        m.get("CharArraySet.toString/empty")
    );

    let mut term = CharTermAttributeImpl::new();
    assert_eq!(
        term.to_string(),
        m.get("CharTermAttributeImpl.toString/empty")
    );
    term.append_string("hello");
    assert_eq!(
        term.to_string(),
        m.get("CharTermAttributeImpl.toString/hello")
    );
    term.set_empty();
    term.append_string("café 日本");
    assert_eq!(
        term.to_string(),
        m.get("CharTermAttributeImpl.toString/unicode"),
        "the bare term, with no class name and no delimiters"
    );
}
