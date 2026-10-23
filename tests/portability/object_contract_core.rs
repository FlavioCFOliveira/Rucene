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
///
/// `index_dir` is handed to the fixture so that it builds, with Lucene itself,
/// the index every reader-side measurement is taken from — and so that the
/// assertions below can then open *those very files* with this crate. A reader
/// rendered from a different index would only be compared with itself.
fn run_fixture(index_dir: &std::path::Path) -> Vec<(String, String)> {
    let mvn = which_mvn().unwrap_or_else(|e| {
        panic!("the object-contracts portability test needs Maven and a JDK: {e}")
    });
    let output = Command::new(mvn)
        .arg("-q")
        .arg("compile")
        .arg("exec:java")
        .arg("-Dexec.mainClass=org.apache.lucene.rucene.codec.ObjectContractsFixture")
        .arg(format!("-Dtests.seed={TESTS_SEED}"))
        .arg(format!("-Dexec.args={}", index_dir.display()))
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

    let index_dir = tempfile::tempdir().expect("temp dir for the Lucene-written index");
    let measured = Measured(run_fixture(index_dir.path()));

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
    document_fields(&measured);
    documents(&measured);
    readers(&measured, index_dir.path());
    enumerations(&measured, index_dir.path());
    pending_deletes(&measured, index_dir.path());
    buffered_updates(&measured, index_dir.path());
    writer_configs(&measured);
    segment_infos(&measured, index_dir.path());
    field_updates_buffer(&measured);
}

/// The four `FieldUpdatesBuffer` accessors that answer a *value*, measured out
/// of the jar rather than derived from this port's own arithmetic.
///
/// The buffer is where the port diverges most from Lucene — Java packs the
/// updates into parallel primitive arrays that elide a repeated value, this
/// port keeps one record per update — so what the packed representation
/// answers, and what the record list answers, have to be shown to agree. The
/// three shapes are the three the packing distinguishes: every update carrying
/// the same value (Java's `numericValues` stays one slot long), updates
/// carrying different values (it grows), and a buffer that only ever received
/// no-value updates (it keeps its single seeded slot, so `hasSingleValue()` is
/// still `true` and both extremes are still at their seeds, which Java reports
/// as `0`).
fn field_updates_buffer(m: &Measured) {
    use rucene::index::doc_values_update::DocValuesUpdate;
    use rucene::index::field_updates_buffer::FieldUpdatesBuffer;
    use rucene::index::Term;

    let term = |value: &str| Term::new("id", BytesRef::new(value.as_bytes().to_vec()));

    let mut same = FieldUpdatesBuffer::new(DocValuesUpdate::numeric(term("a"), "price", 7), 3)
        .expect("a numeric buffer");
    same.add_numeric_update(&term("b"), 7, 5).expect("second");
    same.add_numeric_update(&term("c"), 7, 9).expect("third");

    let mut mixed = FieldUpdatesBuffer::new(DocValuesUpdate::numeric(term("a"), "price", -4), 3)
        .expect("a numeric buffer");
    mixed.add_numeric_update(&term("b"), 11, 5).expect("second");
    mixed.add_numeric_update(&term("c"), 0, 9).expect("third");

    let mut none = FieldUpdatesBuffer::new(
        DocValuesUpdate::new(
            rucene::index::DocValuesType::NUMERIC,
            term("a"),
            "price",
            3,
            rucene::index::doc_values_update::DocValuesUpdateValue::None,
        ),
        3,
    )
    .expect("a buffer of resets");
    none.add_reset("price", &term("b"), 5).expect("second");

    for (name, buffer, count) in [
        ("same", &same, 3usize),
        ("mixed", &mixed, 3),
        ("none", &none, 2),
    ] {
        assert_eq!(
            buffer.has_single_value().to_string(),
            m.get(&format!("FieldUpdatesBuffer.hasSingleValue/{name}")),
            "hasSingleValue disagrees for the `{name}` buffer"
        );
        // Java answers `0` where this port answers `None`, the divergence
        // declared on `min_numeric_value`; `unwrap_or(0)` is that mapping and
        // nothing else, so a wrong value still fails.
        assert_eq!(
            buffer.min_numeric_value().unwrap_or(0).to_string(),
            m.get(&format!("FieldUpdatesBuffer.getMinNumeric/{name}")),
            "getMinNumeric disagrees for the `{name}` buffer"
        );
        assert_eq!(
            buffer.max_numeric_value().unwrap_or(0).to_string(),
            m.get(&format!("FieldUpdatesBuffer.getMaxNumeric/{name}")),
            "getMaxNumeric disagrees for the `{name}` buffer"
        );
        for i in 0..count {
            assert_eq!(
                buffer.get_numeric_value(i).to_string(),
                m.get(&format!("FieldUpdatesBuffer.getNumericValue/{name}/{i}")),
                "getNumericValue({i}) disagrees for the `{name}` buffer"
            );
        }
    }
}

/// `toStringVerbose()` on the three segment types, read back from the very
/// commit Lucene wrote.
///
/// The strings are not reproducible from an object this crate builds: they
/// carry the segment ids and the flush timestamps of the run that produced
/// them, and the diagnostics map renders in `java.util.HashMap` bucket order.
/// Reading Lucene's own `segments_1` is what makes them comparable — and it is
/// the map order that makes this an interesting comparison, because it is
/// answered by
/// [`JavaHashMap`](rucene::util::java_collections::JavaHashMap) rather than by
/// insertion order.
fn segment_infos(m: &Measured, index_dir: &std::path::Path) {
    let _ = rucene::codecs::register_codec("Lucene104", rucene::codecs::Lucene104Codec::new());
    let directory: Arc<dyn Directory> =
        Arc::new(rucene::store::FSDirectory::open(index_dir).expect("open the Java index"));
    let infos = rucene::index::SegmentInfos::read_latest_commit(directory)
        .expect("read the commit Lucene wrote");

    /// Sorts the entries inside every `{...}` block.
    ///
    /// `SegmentInfo`'s diagnostics and attributes go through `Map.copyOf`,
    /// whose iteration order is decided by a salt drawn once per JVM — two runs
    /// of *Lucene itself* disagree, which
    /// `seal_bytes::lucenes_own_si_diagnostics_order_is_not_stable_between_jvms`
    /// measures. The entries and the rest of the rendering are compared exactly;
    /// only the order inside the braces is normalised, because there is no
    /// order there to be faithful to.
    fn map_entries_sorted(rendered: &str) -> String {
        let mut out = String::with_capacity(rendered.len());
        let mut rest = rendered;
        while let Some(open) = rest.find('{') {
            let close = open
                + rest[open..]
                    .find('}')
                    .expect("every `{` a rendering opens is closed");
            out.push_str(&rest[..=open]);
            let mut entries: Vec<&str> = rest[open + 1..close].split(", ").collect();
            entries.sort_unstable();
            out.push_str(&entries.join(", "));
            out.push('}');
            rest = &rest[close + 1..];
        }
        out.push_str(rest);
        out
    }

    assert_eq!(
        map_entries_sorted(&infos.to_string_verbose()),
        map_entries_sorted(m.get("SegmentInfos.toStringVerbose/two-segments"))
    );
    assert_eq!(
        infos.to_string(),
        m.get("SegmentInfos.toString/two-segments"),
        "the non-verbose rendering shares the body and must not have moved"
    );
    for i in 0..infos.size() {
        let commit = infos.info(i);
        assert_eq!(
            map_entries_sorted(&commit.to_string_verbose()),
            map_entries_sorted(m.get(&format!("SegmentCommitInfo.toStringVerbose/{i}")))
        );
        assert_eq!(
            map_entries_sorted(&commit.info.to_string_verbose()),
            map_entries_sorted(m.get(&format!("SegmentInfo.toStringVerbose/{i}")))
        );
    }
}

/// `LiveIndexWriterConfig.toString()` and the `IndexWriterConfig` override,
/// which appends one `writer=` line to it.
///
/// Twenty-two of the twenty-three clauses are compared byte for byte. The
/// twenty-third, `eventListener=`, renders an object with no `toString()` of
/// its own, so Java prints `name@<identity hash>`; the hash has no Rust
/// counterpart and the crate substitutes the object's address, so only that
/// hexadecimal is erased. Erasing it is not a way of avoiding the comparison:
/// the class name in front of the `@` — the anonymous
/// `IndexWriterEventListener$1` — is exactly what is being pinned, and it is
/// the part a port gets wrong.
fn writer_configs(m: &Measured) {
    use rucene::index::index_writer_config::{IndexWriterConfig, LiveIndexWriterConfig, OpenMode};
    use rucene::index::merge_policy::NoMergePolicy;
    use rucene::search::{Sort, SortField, SortFieldType};

    /// The escaping the Java fixture applies before printing a multi-line
    /// value, so the two sides are compared in the same shape.
    fn escape(rendered: &str) -> String {
        rendered.replace('\\', "\\\\").replace('\n', "\\n")
    }

    const LISTENER: &str = "IndexWriterEventListener$1@";

    /// Erases the identity hash of the one clause that carries one.
    fn erase_identity(escaped: &str) -> String {
        match escaped.find(LISTENER) {
            Some(at) => {
                let from = at + LISTENER.len();
                let end = escaped[from..]
                    .find("\\n")
                    .map_or(escaped.len(), |i| from + i);
                format!("{}<identity>{}", &escaped[..from], &escaped[end..])
            }
            None => panic!("no `{LISTENER}` clause in `{escaped}`"),
        }
    }

    let defaults = IndexWriterConfig::new();
    assert_eq!(
        erase_identity(&escape(&defaults.to_string())),
        erase_identity(m.get("IndexWriterConfig.toString/default")),
        "the default IndexWriterConfig renders every clause Lucene renders"
    );

    let live = LiveIndexWriterConfig::new(IndexWriterConfig::new().analyzer_arc());
    assert_eq!(
        erase_identity(&escape(&live.to_string())),
        erase_identity(m.get("LiveIndexWriterConfig.toString/default")),
        "LiveIndexWriterConfig.toString() is the same text without `writer=`"
    );

    let mut tuned = IndexWriterConfig::new();
    tuned
        .set_ram_buffer_size_mb(64.0)
        .expect("a valid RAM buffer size");
    tuned
        .set_max_buffered_docs(1000)
        .expect("a valid doc count");
    tuned.set_use_compound_file(false);
    tuned.set_commit_on_close(false);
    tuned.set_soft_deletes_field(Some("__soft_deletes".to_string()));
    tuned
        .set_index_sort(
            Sort::new_fields(vec![SortField::new(
                Some("price".to_string()),
                SortFieldType::Long,
            )
            .expect("a long sort field")])
            .expect("one sort field"),
        )
        .expect("a sortable field type");
    tuned
        .set_merge_policy(Arc::new(NoMergePolicy::new()))
        .expect("a merge policy");
    tuned.set_open_mode(OpenMode::CREATE).expect("an open mode");
    assert_eq!(
        erase_identity(&escape(&tuned.to_string())),
        erase_identity(m.get("IndexWriterConfig.toString/tuned")),
        "a tuned config renders the settings that were changed"
    );
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

// ---------------------------------------------------------------------------
// Document and the fields it renders
// ---------------------------------------------------------------------------

/// Every `IndexableField` implementation renders through the inherited
/// `Field.toString()`; these pin what each one's `fieldsData` looks like.
fn document_fields(m: &Measured) {
    use rucene::document::{
        BinaryDocValuesField, BinaryPoint, DoubleDocValuesField, FloatDocValuesField, KeywordField,
        NumericDocValuesField, NumericValue, SortedDocValuesField, SortedNumericDocValuesField,
        SortedSetDocValuesField, Store, StoredField, StringField, TextField,
    };

    assert_eq!(
        StringField::new("id", "1".to_string(), Store::NO)
            .expect("string field")
            .to_string(),
        m.get("field.StringField.string")
    );
    assert_eq!(
        StringField::new("id", "1".to_string(), Store::YES)
            .expect("string field")
            .to_string(),
        m.get("field.StringField.stored")
    );
    assert_eq!(
        StringField::new_with_bytes("id", BytesRef::new(b"abc".to_vec()), Store::NO)
            .expect("string field")
            .to_string(),
        m.get("field.StringField.bytes"),
        "a BytesRef value renders as Lucene's `[61 62 63]`"
    );
    assert_eq!(
        TextField::new("body", "hello world".to_string(), Store::NO)
            .expect("text field")
            .to_string(),
        m.get("field.TextField.string")
    );
    assert_eq!(
        TextField::new("body", "hello world".to_string(), Store::YES)
            .expect("text field")
            .to_string(),
        m.get("field.TextField.stored")
    );
    assert_eq!(
        StoredField::new_string("s", "text".to_string())
            .expect("stored field")
            .to_string(),
        m.get("field.StoredField.string")
    );
    assert_eq!(
        StoredField::new_number("s", NumericValue::Int(42))
            .expect("stored field")
            .to_string(),
        m.get("field.StoredField.int")
    );
    assert_eq!(
        StoredField::new_number("s", NumericValue::Long(42))
            .expect("stored field")
            .to_string(),
        m.get("field.StoredField.long")
    );
    assert_eq!(
        StoredField::new_number("s", NumericValue::Float(2.5))
            .expect("stored field")
            .to_string(),
        m.get("field.StoredField.float")
    );
    assert_eq!(
        StoredField::new_number("s", NumericValue::Double(2.5))
            .expect("stored field")
            .to_string(),
        m.get("field.StoredField.double")
    );
    assert_eq!(
        StoredField::new_bytes("s", BytesRef::new(b"abc".to_vec()))
            .expect("stored field")
            .to_string(),
        m.get("field.StoredField.bytes")
    );
    assert_eq!(
        KeywordField::new("k", "v".to_string(), Store::NO)
            .expect("keyword field")
            .to_string(),
        m.get("field.KeywordField.string")
    );
    assert_eq!(
        KeywordField::new("k", "v".to_string(), Store::YES)
            .expect("keyword field")
            .to_string(),
        m.get("field.KeywordField.stored")
    );
    assert_eq!(
        KeywordField::new_with_bytes("k", BytesRef::new(b"abc".to_vec()), Store::NO)
            .expect("keyword field")
            .to_string(),
        m.get("field.KeywordField.bytes")
    );
    assert_eq!(
        BinaryPoint::new("b", &[BytesRef::new(vec![1u8, 2, 3])])
            .expect("binary point")
            .to_string(),
        m.get("field.BinaryPoint")
    );
    assert_eq!(
        NumericDocValuesField::new("n", 17).to_string(),
        m.get("field.NumericDocValuesField")
    );
    assert_eq!(
        FloatDocValuesField::new("n", 2.5).to_string(),
        m.get("field.FloatDocValuesField"),
        "Java stores `Float.floatToRawIntBits`, so the raw bits are what render"
    );
    assert_eq!(
        DoubleDocValuesField::new("n", 2.5).to_string(),
        m.get("field.DoubleDocValuesField"),
        "likewise `Double.doubleToRawLongBits`"
    );
    assert_eq!(
        SortedDocValuesField::new("s", BytesRef::new(b"abc".to_vec())).to_string(),
        m.get("field.SortedDocValuesField")
    );
    assert_eq!(
        SortedSetDocValuesField::new("s", BytesRef::new(b"abc".to_vec())).to_string(),
        m.get("field.SortedSetDocValuesField")
    );
    assert_eq!(
        SortedNumericDocValuesField::new("s", 17).to_string(),
        m.get("field.SortedNumericDocValuesField")
    );
    assert_eq!(
        BinaryDocValuesField::new("b", BytesRef::new(b"abc".to_vec())).to_string(),
        m.get("field.BinaryDocValuesField")
    );
}

/// `Document.toString()` is polymorphic over `IndexableField`, so what it pins
/// is that every field renders through its own `toString()`.
fn documents(m: &Measured) {
    use rucene::document::{Document, IntPoint, Store, StringField};

    assert_eq!(
        Document::new().to_string(),
        m.get("Document.toString/empty")
    );

    let mut one = Document::new();
    one.add(Box::new(
        StringField::new("id", "1".to_string(), Store::NO).expect("string field"),
    ));
    assert_eq!(
        one.to_string(),
        m.get("Document.toString/single-string-field")
    );

    let mut two = Document::new();
    two.add(Box::new(
        StringField::new("id", "1".to_string(), Store::NO).expect("string field"),
    ));
    two.add(Box::new(IntPoint::new("count", &[42]).expect("int point")));
    assert_eq!(
        two.to_string(),
        m.get("Document.toString/string-and-point"),
        "the `IntPoint` renders through its own override, the `StringField` through `Field`'s"
    );
}

// ---------------------------------------------------------------------------
// The reader hierarchy
// ---------------------------------------------------------------------------

/// Opens, with this crate, the very index the Java fixture wrote.
fn open_java_index(index_dir: &std::path::Path) -> Arc<dyn rucene::index::DirectoryReader> {
    // The codec SPI has no `META-INF/services` equivalent here, so the codec the
    // Java fixture wrote with has to be seeded before a segment naming it can be
    // read back.
    let _ = rucene::codecs::register_codec("Lucene104", rucene::codecs::Lucene104Codec::new());
    let directory: Arc<dyn Directory> =
        Arc::new(rucene::store::FSDirectory::open(index_dir).expect("open the Java index"));
    rucene::index::open_directory_reader(directory).expect("open a directory reader over it")
}

fn readers(m: &Measured, index_dir: &std::path::Path) {
    use rucene::index::filter_directory_reader::{
        DirectoryReaderWrapper, ExitableDirectoryReaderWrapper, ExitableSubReaderWrapper,
        SlowCodecReaderWrapper, SubReaderWrapper,
    };
    use rucene::index::filter_reader::FilterLeafReader;
    use rucene::index::leaf_reader::LeafReader;
    use rucene::index::reader_context::IndexReaderContext;
    use rucene::index::sorting_codec_reader::SlowCompositeCodecReaderWrapper;
    use rucene::index::{MultiReader, ParallelLeafReader};

    let reader = open_java_index(index_dir);
    assert_eq!(
        reader.to_string(),
        m.get("StandardDirectoryReader.toString/two-segments"),
        "the segments file, the commit version and every segment reader"
    );

    let leaves = Arc::clone(&reader).leaves();
    let leaf0 = leaves[0].leaf_reader();
    let leaf1 = leaves[1].leaf_reader();

    assert_eq!(
        leaf0.to_string(),
        m.get("SegmentReader.toString/segment-0"),
        "`si.toString(si.info.maxDoc() - numDocs - si.getDelCount())`"
    );
    assert_eq!(
        leaves[0].to_string(),
        m.get("LeafReaderContext.toString/leaf-0")
    );
    assert_eq!(
        leaves[1].to_string(),
        m.get("LeafReaderContext.toString/leaf-1"),
        "docBase and ord both move with the ordinal"
    );

    // `Arc<dyn LeafReader>` does not coerce to `Arc<dyn IndexReader>` in this
    // port — the two are related by a blanket impl, not by inheritance — so the
    // composite view comes off the leaf context, which carries both handles.
    let multi = MultiReader::new(vec![leaves[0].reader(), leaves[1].reader()], false)
        .expect("multi reader");
    assert_eq!(
        multi.to_string(),
        m.get("CompositeReader.toString/MultiReader-two-leaves"),
        "`CompositeReader.toString()`, inherited: the sub-readers are space-separated"
    );
    assert_eq!(
        MultiReader::new(Vec::new(), false)
            .expect("empty multi reader")
            .to_string(),
        m.get("CompositeReader.toString/MultiReader-empty")
    );

    assert_eq!(
        FilterLeafReader::new(Arc::clone(&leaf0)).to_string(),
        m.get("FilterLeafReader.toString/over-segment-0"),
        "Lucene hardcodes the name `FilterLeafReader`, it does not ask for its own"
    );

    assert_eq!(
        ParallelLeafReader::new(vec![Arc::clone(&leaf0)])
            .expect("parallel leaf reader")
            .to_string(),
        m.get("ParallelLeafReader.toString/one-leaf")
    );
    assert_eq!(
        ParallelLeafReader::new(Vec::new())
            .expect("empty parallel leaf reader")
            .to_string(),
        m.get("ParallelLeafReader.toString/no-leaves")
    );
    // Java iterates an `IdentityHashMap`-backed set here, so the *order* of the
    // readers is unspecified — two instances built from the same two readers in
    // the same JVM were observed to render them in opposite orders. What is
    // compared is therefore the set and the separator, not the sequence.
    let two = ParallelLeafReader::new(vec![Arc::clone(&leaf0), Arc::clone(&leaf1)])
        .expect("parallel leaf reader")
        .to_string();
    assert_eq!(
        sorted_inside("ParallelLeafReader(", &two),
        sorted_inside(
            "ParallelLeafReader(",
            m.get("ParallelLeafReader.toString/two-leaves")
        ),
        "the same two readers, comma-separated; only Java's unspecified order differs"
    );

    let exitable = ExitableDirectoryReaderWrapper::new(Arc::new(NeverTimesOut))
        .wrap(Arc::clone(&reader))
        .expect("wrap as an exitable reader");
    assert_eq!(
        exitable.to_string(),
        m.get("ExitableDirectoryReader.toString/two-segments"),
        "the subclass renders its *delegate*, not the wrapped leaves"
    );
    // `leaves()` must hand back the *wrapped* leaves, as
    // `FilterDirectoryReader`'s constructor arranges by passing
    // `wrapper.wrap(in.getSequentialSubReaders())` to `super`
    // (`FilterDirectoryReader.java:91-93`). Both routes to the wrapped leaf are
    // pinned here: through the reader, and straight off the `SubReaderWrapper`.
    let exitable_leaves = Arc::clone(&exitable).leaves();
    assert_eq!(
        exitable_leaves.len(),
        2,
        "the wrapping preserves the leaves"
    );
    assert_eq!(
        exitable_leaves[0].leaf_reader().to_string(),
        m.get("ExitableFilterAtomicReader.toString/leaf-0"),
        "a leaf reached through the exitable reader is the wrapped one"
    );
    let wrapped_leaf = ExitableSubReaderWrapper::new(Arc::new(NeverTimesOut))
        .wrap(Arc::clone(&leaf0))
        .expect("wrap the leaf");
    assert_eq!(
        wrapped_leaf.to_string(),
        m.get("ExitableFilterAtomicReader.toString/leaf-0"),
        "`ExitableFilterAtomicReader` overrides no toString(), so `FilterLeafReader`'s applies"
    );

    let slow = SlowCodecReaderWrapper::wrap(Arc::new(
        ParallelLeafReader::new(vec![Arc::clone(&leaf0), Arc::clone(&leaf1)]).unwrap(),
    ) as Arc<dyn LeafReader>)
    .expect("slow codec reader");
    assert_eq!(
        sorted_inside(
            "SlowCodecReaderWrapper(ParallelLeafReader(",
            &slow.to_string()
        ),
        sorted_inside(
            "SlowCodecReaderWrapper(ParallelLeafReader(",
            m.get("SlowCodecReaderWrapper.toString/over-parallel")
        ),
        "the wrapped reader is rendered by its own toString(), not by Rust's `Debug`"
    );

    let composite = SlowCompositeCodecReaderWrapper::wrap(vec![
        rucene::index::filter_directory_reader::SlowCodecReaderWrapper::wrap(Arc::clone(&leaf0))
            .expect("codec view"),
        rucene::index::filter_directory_reader::SlowCodecReaderWrapper::wrap(Arc::clone(&leaf1))
            .expect("codec view"),
    ])
    .expect("slow composite codec reader");
    assert_eq!(
        identity_erased(&composite.to_string()),
        m.get("SlowCompositeCodecReaderWrapper.toString/shape"),
        "no toString() of its own, so `Object`'s binary name and `name@hex` shape"
    );
}

/// A [`QueryTimeout`](rucene::index::index_utilities::QueryTimeout) that never trips.
#[derive(Debug)]
struct NeverTimesOut;

impl rucene::index::index_utilities::QueryTimeout for NeverTimesOut {
    fn should_exit(&self) -> bool {
        false
    }
}

/// Replaces the hexadecimal identity Java's `Object.toString()` ends with, so
/// that the class name and the shape can be compared while the digits — which
/// no port can reproduce — are not.
fn identity_erased(rendered: &str) -> String {
    match rendered.rfind('@') {
        Some(at) => format!("{}@<identity>", &rendered[..at]),
        None => rendered.to_string(),
    }
}

/// Sorts the comma-separated elements a `prefix(...)` rendering holds, so that
/// an order Java leaves unspecified does not decide the comparison.
fn sorted_inside(prefix: &str, rendered: &str) -> String {
    let body = rendered
        .strip_prefix(prefix)
        .unwrap_or_else(|| panic!("`{rendered}` does not start with `{prefix}`"));
    let body = body.trim_end_matches(')');
    let mut parts: Vec<&str> = body.split(", ").collect();
    parts.sort_unstable();
    format!("{prefix}{})", parts.join(", "))
}

// ---------------------------------------------------------------------------
// Term and postings enumerations
// ---------------------------------------------------------------------------

fn enumerations(m: &Measured, index_dir: &std::path::Path) {
    use rucene::index::index_reader::IndexReader;
    use rucene::index::{MultiTerms, TermsEnumIndex};

    let reader = open_java_index(index_dir);
    // `Arc<dyn DirectoryReader>` does not coerce to `Arc<dyn IndexReader>`, so
    // the erased view comes off the reader's own top-level context.
    let as_index_reader: Arc<dyn IndexReader> = Arc::clone(&reader).get_context().reader();

    let terms = MultiTerms::get_terms(&as_index_reader, "body")
        .expect("read the merged terms")
        .expect("the `body` field is indexed in both segments");
    let multi_terms_enum = terms.iterator().expect("iterate the merged terms");
    assert_eq!(
        rendered_shape(&multi_terms_enum.to_string()),
        rendered_shape(m.get("MultiTermsEnum.toString/body-unpositioned")),
        "the wrapper name, the `ReaderSlice` record rendering, the `:` and the sub-enum's class"
    );

    let postings = MultiTerms::get_term_postings_enum(
        &as_index_reader,
        "body",
        &BytesRef::new(b"alpha".to_vec()),
        rucene::index::postings_enum::POSTINGS_ENUM_ALL,
    )
    .expect("read the merged postings")
    .expect("`alpha` occurs in both segments");
    assert_eq!(
        rendered_shape(&postings.to_string()),
        rendered_shape(m.get("MultiPostingsEnum.toString/body-alpha")),
        "the class renames itself `MultiDocsAndPositionsEnum`"
    );

    let leaves = Arc::clone(&reader).leaves();
    let leaf0 = leaves[0].leaf_reader();
    let leaf_terms = leaf0
        .terms("body")
        .expect("read the leaf terms")
        .expect("the `body` field is indexed");
    let index = TermsEnumIndex::new(leaf_terms.iterator().expect("iterate"), 0);
    let bare = leaf_terms.iterator().expect("iterate");
    assert_eq!(
        identity_erased(&index.to_string()),
        identity_erased(&bare.to_string()),
        "`TermsEnumIndex.toString()` is `Objects.toString(termsEnum)` and nothing else"
    );
    assert_eq!(
        identity_erased(&index.to_string()),
        identity_erased(m.get("TermsEnumIndex.toString/leaf-0-unpositioned")),
        "the class of the wrapped enumeration is Lucene's own blocktree `SegmentTermsEnum`"
    );
}

/// Replaces every `…@<hex>` identity inside a rendering, so that the structure
/// — the wrapper names, the `ReaderSlice` records, the separators and the class
/// names — is what is compared, and the digits Java draws from a JVM-internal
/// PRNG are not.
fn rendered_shape(rendered: &str) -> String {
    let mut out = String::with_capacity(rendered.len());
    let mut rest = rendered;
    while let Some(at) = rest.find('@') {
        out.push_str(&rest[..at]);
        out.push_str("@<identity>");
        rest = &rest[at + 1..];
        let end = rest
            .find(|c: char| !c.is_ascii_hexdigit())
            .unwrap_or(rest.len());
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}

// ---------------------------------------------------------------------------
// Pending deletes
// ---------------------------------------------------------------------------

fn pending_deletes(m: &Measured, index_dir: &std::path::Path) {
    use rucene::index::{PendingDeletes, PendingSoftDeletes, ReadersAndUpdates, SegmentInfos};

    let _ = rucene::codecs::register_codec("Lucene104", rucene::codecs::Lucene104Codec::new());
    let directory: Arc<dyn Directory> =
        Arc::new(rucene::store::FSDirectory::open(index_dir).expect("open the Java index"));
    let infos = SegmentInfos::read_latest_commit(directory).expect("read segments_N");
    let info = infos.info(0).clone();

    let mut hard = PendingDeletes::new(&info);
    assert_eq!(
        hard.to_string_with_info(&info),
        m.get("PendingDeletes.toString/fresh"),
        "Lucene never closes the parenthesis it opens here"
    );
    hard.get_mutable_bits(&info);
    assert_eq!(
        hard.to_string_with_info(&info),
        m.get("PendingDeletes.toString/writeable")
    );
    hard.delete(1, &info).expect("delete document 1");
    assert_eq!(
        hard.to_string_with_info(&info),
        m.get("PendingDeletes.toString/one-deleted")
    );

    let soft = PendingSoftDeletes::new("__soft_deletes", &info);
    assert_eq!(
        soft.to_string_with_info(&info),
        m.get("PendingSoftDeletes.toString/fresh"),
        "the embedded hardDeletes tracker is rendered whole, its parenthesis open too"
    );

    let rau = ReadersAndUpdates::new(10, info.clone(), None);
    assert_eq!(
        rau.to_string(),
        m.get("ReadersAndUpdates.toString/fresh"),
        "the class renames itself `ReadersAndLiveDocs`"
    );
    let soft_rau = ReadersAndUpdates::new(10, info.clone(), Some("__soft_deletes"));
    assert_eq!(
        soft_rau.to_string(),
        m.get("ReadersAndUpdates.toString/soft-deletes")
    );
}

// ---------------------------------------------------------------------------
// Buffered updates
// ---------------------------------------------------------------------------

fn buffered_updates(m: &Measured, index_dir: &std::path::Path) {
    use rucene::index::doc_values_update::{DocValuesUpdate, DocValuesUpdateValue};
    use rucene::index::{BufferedUpdates, DocumentsWriterDeleteQueue, FrozenBufferedUpdates, Term};
    use rucene::search::MatchAllDocsQuery;

    assert_eq!(
        BufferedUpdates::new("_0").to_string(),
        m.get("BufferedUpdates.toString/empty")
    );

    let mut terms = BufferedUpdates::new("_0");
    terms.add_term(Term::new("id", BytesRef::new(b"0-0".to_vec())), 5);
    terms.add_term(Term::new("id", BytesRef::new(b"0-1".to_vec())), 7);
    assert_eq!(
        terms.to_string(),
        m.get("BufferedUpdates.toString/two-terms"),
        "note the trailing space Lucene leaves after `unique deleted terms`"
    );

    let mut queries = BufferedUpdates::new("_0");
    queries.add_query(Arc::new(MatchAllDocsQuery::instance()), 3);
    assert_eq!(
        queries.to_string(),
        m.get("BufferedUpdates.toString/one-query"),
        "`deleted queries` is plural even for one, and `bytesUsed` is charged per query"
    );

    let queue = Arc::new(DocumentsWriterDeleteQueue::new(Arc::new(
        rucene::util::NoOutputInfoStream,
    )));
    assert_eq!(
        queue.to_string(),
        m.get("DocumentsWriterDeleteQueue.toString/fresh")
    );

    // `DocumentsWriterPerThread.toString()` renders the buffered updates, the
    // segment name, the abort flag, the in-RAM document count, the delete queue
    // and the deleted-docID count, in that order and with Java's punctuation.
    let _ = rucene::codecs::register_codec("Lucene104", rucene::codecs::Lucene104Codec::new());
    let directory: Arc<dyn Directory> =
        Arc::new(rucene::store::FSDirectory::open(index_dir).expect("open the Java index"));
    let config = Arc::new(
        rucene::index::index_writer_config::LiveIndexWriterConfig::new(Arc::new(
            rucene::analysis::standard::StandardAnalyzer::new(),
        )),
    );
    let field_numbers =
        Arc::new(rucene::index::field_infos::FieldNumbers::new(None, None).expect("field numbers"));
    let dwpt = rucene::index::DocumentsWriterPerThread::new(
        10,
        "_9".to_string(),
        Arc::clone(&directory),
        Arc::clone(&directory),
        Arc::clone(&config),
        Arc::clone(&queue),
        rucene::index::field_infos::FieldInfosBuilder::new(field_numbers),
        Arc::new(std::sync::atomic::AtomicI64::new(0)),
        Arc::new(std::sync::atomic::AtomicI32::new(rucene::index::MAX_DOCS)),
        false,
        Box::new(rucene::index::indexing_chain::DefaultIndexingChain::new(
            Arc::clone(&config),
        )),
    )
    .expect("build a DWPT");
    assert_eq!(
        dwpt.to_string(),
        m.get("DocumentsWriterPerThread.toString/fresh")
    );

    let mut everything = BufferedUpdates::new("_0");
    everything.add_term(Term::new("id", BytesRef::new(b"0-0".to_vec())), 5);
    everything.add_query(
        Arc::new(rucene::search::TermQuery::new(Term::new(
            "id",
            BytesRef::new(b"0-2".to_vec()),
        ))),
        9,
    );
    everything
        .add_numeric_update(
            &DocValuesUpdate::new(
                rucene::index::DocValuesType::NUMERIC,
                Term::new("id", BytesRef::new(b"0-1".to_vec())),
                "price",
                4,
                DocValuesUpdateValue::Numeric(33),
            ),
            4,
        )
        .expect("buffer a numeric update");
    everything.set_generation(12);
    assert_eq!(
        everything.to_string(),
        m.get("BufferedUpdates.toString/terms-query-and-updates"),
        "the trailing space of the terms clause leaves two spaces before the query clause"
    );

    // `FrozenBufferedUpdates` is where this port's freezing model shows: Java
    // packs the terms into `PrefixCodedTerms` and the queries into an array and
    // reports *their* RAM, while this port keeps the source `BufferedUpdates`
    // and reports its own. The `bytesUsed=` clause therefore cannot agree — an
    // empty Java packet already reports `bytesUsed=16` for an empty
    // `PrefixCodedTerms` — so what is compared is every other clause, in Java's
    // order and wording. See the divergence recorded on `Display for
    // FrozenBufferedUpdates`.
    let frozen_empty = FrozenBufferedUpdates::new(BufferedUpdates::new("_0"), None, 0);
    assert_eq!(
        frozen_empty.to_string(),
        without_bytes_used(m.get("FrozenBufferedUpdates.toString/empty")),
        "an unpushed packet renders `delGen=-1`: Java reads the field, not the asserting accessor"
    );

    let mut source = BufferedUpdates::new("_0");
    source.add_term(Term::new("id", BytesRef::new(b"0-0".to_vec())), 5);
    source.add_term(Term::new("id", BytesRef::new(b"0-1".to_vec())), 7);
    source
        .add_numeric_update(
            &DocValuesUpdate::new(
                rucene::index::DocValuesType::NUMERIC,
                Term::new("id", BytesRef::new(b"0-1".to_vec())),
                "price",
                4,
                DocValuesUpdateValue::Numeric(33),
            ),
            4,
        )
        .expect("buffer a numeric update");
    let frozen = FrozenBufferedUpdates::new(source, None, 0);
    frozen.set_del_gen(7);
    assert_eq!(
        without_bytes_used(&frozen.to_string()),
        without_bytes_used(m.get("FrozenBufferedUpdates.toString/terms-and-updates")),
        "`unique deleteTerms=` then `fieldUpdates=`, guarded on the field count but printing \
         the update count"
    );

    let mut private_source = BufferedUpdates::new("_0");
    private_source.add_query(Arc::new(MatchAllDocsQuery::instance()), 3);
    let private_frozen = FrozenBufferedUpdates::new(private_source, Some("_0".to_string()), 0);
    let rendered = without_bytes_used(&private_frozen.to_string());
    let expected = without_bytes_used(m.get("FrozenBufferedUpdates.toString/private-segment"));
    assert!(
        expected.starts_with("delGen=-1 numDeleteQueries=1 privateSegment="),
        "the Java shape this comparison relies on: {expected}"
    );
    assert!(
        rendered.starts_with("delGen=-1 numDeleteQueries=1 privateSegment=_0"),
        "the query clause and the private-segment clause, in Java's order: {rendered}"
    );
}

/// Strips the `bytesUsed=<n>` clause from a `FrozenBufferedUpdates` rendering.
///
/// Java's figure is the RAM of the frozen `PrefixCodedTerms` and `Query[]`;
/// this port freezes by keeping the source `BufferedUpdates`, so the two count
/// different objects. Everything else in the string is comparable.
fn without_bytes_used(rendered: &str) -> String {
    let mut out = Vec::new();
    let mut rest = rendered;
    while let Some(at) = rest.find(" bytesUsed=") {
        out.push(&rest[..at]);
        rest = &rest[at + " bytesUsed=".len()..];
        let end = rest
            .find(|c: char| !c.is_ascii_digit() && c != '-')
            .unwrap_or(rest.len());
        rest = &rest[end..];
    }
    out.push(rest);
    out.concat()
}
