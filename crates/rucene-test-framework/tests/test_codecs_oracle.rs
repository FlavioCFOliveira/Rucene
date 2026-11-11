//! Oracle parity of the test codecs of `org.apache.lucene.tests.codecs`
//! (rmp #340: `asserting`, `cheapbastard`, `compressing`, `vector`; rmp #341:
//! `cranky`, `blockterms`, `bloom`, `mockrandom`, `ramonly`, `uniformsplit` and
//! the service files) against Apache Lucene 10.5.0.
//!
//! The Java oracle (`org.apache.lucene.tests.codecs.TestCodecsOracle`, in the
//! workspace harness `crates/randomizedtesting/tests/fixtures/oracle/`)
//! records:
//!
//! * `testcodecs_files.tsv`: every file of the `nocfs` and `cfs` corpus of
//!   `SimpleTextOracle.writeCorpus` written with each codec. Rucene writes the
//!   same corpus with the same codec (the corpus helpers of
//!   `crates/rucene-codecs/tests/oracle/mod.rs`) and every file but
//!   `segments_N` and the `.si` (environment diagnostics) must be Lucene's
//!   byte for byte once Lucene's segment id is replaced by Rucene's.
//! * `testcodecs_misuse.tsv`: the error class and message each misuse of an
//!   `Asserting` object raises, on the `nocfs` index Lucene wrote with
//!   `AssertingCodec`.
//! * `testcodecs_cranky_draws.tsv`: the bound of every `random.nextInt(bound)`
//!   a `CrankyCodec` over `Lucene104Codec` draws while writing the `nocfs`
//!   corpus (a random that never draws 0).
//! * `testcodecs_cranky_seeds.tsv`: per seed 1..=40, the draw at which a
//!   `CrankyCodec` over `new Random(seed)` first fails while writing the
//!   corpus, and the root cause's class and message.
//! * rmp #341: `tc_<format>_{files,reads,enum,errors,spi}.tsv` (the generic
//!   `PostingsFormatOracle` fixtures, through `FormatOracle`) for
//!   `TestBloomFilteredLucenePostings`, the two Rot13 `UniformSplit` formats
//!   and `MockRandomPostingsFormat` over seeds 1 to 3, and `rucene-codecs`'
//!   fixtures for the blockterms formats; `testcodecs_ramonly_*.tsv` for
//!   `RAMOnlyPostingsFormat`; `testcodecs_rot13_flags.tsv`; and
//!   `testcodecs_spi.tsv`, the entries of the crate's service files.

#[path = "../../rucene-codecs/tests/oracle/mod.rs"]
mod oracle;

use std::collections::BTreeMap;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard, Once};

use oracle::*;
use randomizedtesting::jdk::exception::{panic_message, QuietPanics};
use randomizedtesting::jdk::random::{JdkRandom, Random, RandomFields};
use rucene::codecs::lucene104::Lucene104Codec;
use rucene::codecs::Codec;
use rucene::document::IntPoint;
use rucene::error::{LuceneError, LuceneErrorKind, Result};
use rucene::index::index_writer::IndexWriter;
use rucene::index::merge_policy::{MergePolicy, TieredMergePolicy};
use rucene::index::{
    LeafReader, SegmentInfo, POSTINGS_ENUM_ALL, POSTINGS_ENUM_FREQS, POSTINGS_ENUM_NONE,
    POSTINGS_ENUM_OFFSETS, POSTINGS_ENUM_POSITIONS,
};
use rucene::search::{Sort, SortField, SortFieldType, NO_MORE_DOCS};
use rucene::store::java_nio_file::{java_class_of, java_message_of};
use rucene::store::{Directory, IOContext};
use rucene::util::resource_loader::ClassLoader;
use rucene_test_framework::codecs::asserting::AssertingCodec;
use rucene_test_framework::codecs::cheapbastard::CheapBastardCodec;
use rucene_test_framework::codecs::compressing::dummy::DummyCompressingCodec;
use rucene_test_framework::codecs::compressing::{
    DeflateWithPresetCompressingCodec, FastCompressingCodec, FastDecompressionCompressingCodec,
    HighCompressionCompressingCodec, LZ4WithPresetCompressingCodec,
};
use rucene_test_framework::codecs::cranky::CrankyCodec;
use rucene_test_framework::codecs::vector::ConfigurableMCodec;

/// The "class path" of the tests: `lucene-codecs` and the crate's own
/// service files (`rucene_test_framework::class_loader()`), so that the
/// indexes written with the test codecs and formats can be read back by
/// name.
fn test_class_loader() -> &'static Arc<ClassLoader> {
    rucene_test_framework::class_loader()
}

const PREFIX: &str = "testcodecs";

/// `TestCodecsOracle.CODECS`.
const CODECS: [&str; 10] = [
    "Asserting",
    "CheapBastard",
    "FastCompressing",
    "FastDecompressionCompressing",
    "HighCompressionCompressing",
    "DummyCompressing",
    "DeflateWithPresetCompressing",
    "LZ4WithPresetCompressing",
    "ConfigurableM",
    "FastCompressingSuffix",
];

/// `TestCodecsOracle.codec(label)`.
fn codec(label: &str) -> Arc<dyn Codec> {
    match label {
        "Asserting" => Arc::new(AssertingCodec::new()),
        "CheapBastard" => Arc::new(CheapBastardCodec::new()),
        "FastCompressing" => Arc::new(FastCompressingCodec::default_instance().unwrap()),
        "FastDecompressionCompressing" => {
            Arc::new(FastDecompressionCompressingCodec::default_instance().unwrap())
        }
        "HighCompressionCompressing" => {
            Arc::new(HighCompressionCompressingCodec::default_instance().unwrap())
        }
        "DummyCompressing" => Arc::new(DummyCompressingCodec::default_instance().unwrap()),
        "DeflateWithPresetCompressing" => {
            Arc::new(DeflateWithPresetCompressingCodec::default_instance().unwrap())
        }
        "LZ4WithPresetCompressing" => {
            Arc::new(LZ4WithPresetCompressingCodec::default_instance().unwrap())
        }
        "ConfigurableM" => Arc::new(ConfigurableMCodec::new()),
        "FastCompressingSuffix" => {
            Arc::new(FastCompressingCodec::new(1 << 14, 128, true, 10).unwrap())
        }
        _ => unreachable!("{label}"),
    }
}

/// The codecs' names resolve, as the test-framework jar's service file
/// makes them resolve for Lucene, so that the indexes can be read back.
fn register_codecs() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        install_codecs();
        rucene::codecs::doc_values::reload_doc_values_formats(test_class_loader()).unwrap();
    });
}

/// The codecs whose stored fields and term vectors Lucene deflates with
/// `java.util.zip.Deflater`: with the default `zlib-rs` backend Rucene's
/// deflate codewords differ from the JVM's zlib on many inputs (the core's
/// documented `zlib-c` limitation, `Cargo.toml`), so those files are
/// checked by reading instead of by bytes.
const DEFLATE: [&str; 2] = ["HighCompressionCompressing", "DeflateWithPresetCompressing"];

/// Whether `name` (a file, or a compound entry) is a stored-fields or
/// term-vectors file.
fn is_stored_or_vectors(name: &str) -> bool {
    ["fdt", "fdm", "fdx", "tvd", "tvm", "tvx"]
        .iter()
        .any(|ext| name.ends_with(&format!(".{ext}")))
}

/// Every file of `rust` is the file of `java` of the same name, byte for byte
/// once Lucene's segment id is Rucene's, except `segments_N`, the `.si` and
/// the files `skip` names; a compound file is compared entry by entry.
/// Returns the files compared.
fn assert_same_files(
    what: &str,
    java: &BTreeMap<String, Vec<u8>>,
    rust: &BTreeMap<String, Vec<u8>>,
    skip: impl Fn(&str) -> bool,
) -> usize {
    assert_eq!(
        rust.keys().collect::<Vec<_>>(),
        java.keys().collect::<Vec<_>>(),
        "{what}: file listing"
    );
    let mut compared = 0;
    for (name, java_bytes) in java {
        let rust_bytes = &rust[name];
        if name.starts_with("segments_") || name.ends_with(".si") {
            continue;
        }
        if let Some(segment) = name.strip_suffix(".cfs") {
            let cfe = format!("{segment}.cfe");
            let java_entries = compound_entries(&java[&cfe], java_bytes);
            let rust_entries = compound_entries(&rust[&cfe], rust_bytes);
            assert_eq!(
                rust_entries.keys().collect::<Vec<_>>(),
                java_entries.keys().collect::<Vec<_>>(),
                "{what}: {name}: entries"
            );
            for (entry, java_entry) in &java_entries {
                let rust_entry = &rust_entries[entry];
                if skip(entry) {
                    continue;
                }
                let expected = java_bytes_with_rust_id(java_entry, rust_entry);
                assert!(
                    *rust_entry == expected,
                    "{what}: {name}/{entry} differs from Lucene's at byte {} (lengths {} and {})",
                    first_difference(rust_entry, &expected),
                    rust_entry.len(),
                    expected.len()
                );
                compared += 1;
            }
            if !java_entries.keys().any(|e| skip(e)) {
                assert_eq!(rust_bytes.len(), java_bytes.len(), "{what}: {name}: length");
            }
            continue;
        }
        if skip(name) {
            continue;
        }
        let expected = java_bytes_with_rust_id(java_bytes, rust_bytes);
        assert!(
            *rust_bytes == expected,
            "{what}: {name} differs from Lucene's at byte {} (lengths {} and {})",
            first_difference(rust_bytes, &expected),
            rust_bytes.len(),
            expected.len()
        );
        compared += 1;
    }
    compared
}

/// Every codec writes the `nocfs` and `cfs` corpus Lucene writes with it.
#[test]
fn test_codecs_write_lucenes_bytes() {
    register_codecs();
    let mut compared = 0;
    for label in CODECS {
        for case in ["nocfs", "cfs"] {
            let tmp = TempDir::new(&format!("testcodecs-{label}-{case}"));
            write_corpus(&tmp.0, case == "cfs", codec(label));
            let what = format!("{label}/{case}");
            let java = java_files(PREFIX, &what);
            let deflate = DEFLATE.contains(&label);
            compared += assert_same_files(&what, &java, &read_dir(&tmp.0), |name| {
                // A `.cfe` holds the entries' offsets, which follow their lengths.
                deflate && (is_stored_or_vectors(name) || name.ends_with(".cfe"))
            });
            if deflate {
                // Rucene reads Lucene's deflated files as it reads its own.
                let java_dir = TempDir::new(&format!("testcodecs-{label}-{case}-java"));
                write_files(&java_dir.0, &java);
                assert_same_lines(&what, &dump(&java_dir.0), &dump(&tmp.0));
            }
        }
    }
    assert!(compared > 400, "{compared} files compared");
}

/// The `Asserting` formats name their per-field files `Asserting`, the
/// others impersonate the default codec's formats.
#[test]
fn test_codecs_names() {
    register_codecs();
    let asserting = AssertingCodec::new();
    assert_eq!(asserting.name(), "Asserting");
    assert_eq!(asserting.to_string(), "Asserting(Lucene104)");
    assert_eq!(
        asserting.get_postings_format_for_field("body").name(),
        "Asserting"
    );
    assert_eq!(
        asserting.get_doc_values_format_for_field("x").name(),
        "Asserting"
    );
    assert_eq!(
        asserting.get_knn_vectors_format_for_field("x").name(),
        "Asserting"
    );
    assert_eq!(CheapBastardCodec::new().name(), "CheapBastard");
    assert_eq!(ConfigurableMCodec::new().name(), "ConfigurableMCodec");
    let fast = FastCompressingCodec::default_instance().unwrap();
    assert_eq!(fast.name(), "FastCompressingStoredFieldsData");
    assert_eq!(
        fast.to_string(),
        "FastCompressingStoredFieldsData(storedFieldsFormat=Lucene90CompressingStoredFieldsFormat(\
         compressionMode=FAST, chunkSize=16384, maxDocsPerChunk=128, blockShift=10), \
         termVectorsFormat=Lucene90CompressingTermVectorsFormat(compressionMode=FAST, \
         chunkSize=16384, maxDocsPerChunk=128, blockSize=10))"
    );
    let cranky = CrankyCodec::new(Arc::new(Lucene104Codec::new()), Arc::new(JdkRandom::new(0)));
    assert_eq!(cranky.name(), "Lucene104");
    assert_eq!(cranky.postings_format().name(), "PerField40");
    assert_eq!(cranky.doc_values_format().name(), "PerFieldDV40");
    assert_eq!(cranky.to_string(), "Cranky(Lucene104)");
}

/// `Codec.toString()` through `dyn Codec`: the name by default
/// (`Codec.java:157-161`), and the overrides of `AssertingCodec`
/// (`AssertingCodec.java:167-176`), `CrankyCodec` (`CrankyCodec.java:95-98`)
/// and `CompressingCodec` (`CompressingCodec.java:147-155`), each printing its
/// delegate through the delegate's own `toString()`. The compressing form is
/// also checked against Java's output in
/// `compressing_random_instances_are_lucenes`.
#[test]
fn test_codecs_to_string() {
    fn java(codec: &dyn Codec) -> String {
        codec.to_string()
    }
    let random = || -> Arc<dyn Random> { Arc::new(JdkRandom::new(0)) };
    assert_eq!(java(&Lucene104Codec::new()), "Lucene104");
    assert_eq!(java(&CheapBastardCodec::new()), "CheapBastard");
    assert_eq!(java(&ConfigurableMCodec::new()), "ConfigurableMCodec");
    assert_eq!(java(&AssertingCodec::new()), "Asserting(Lucene104)");
    assert_eq!(
        java(&CrankyCodec::new(Arc::new(Lucene104Codec::new()), random())),
        "Cranky(Lucene104)"
    );
    assert_eq!(
        java(&CrankyCodec::new(Arc::new(AssertingCodec::new()), random())),
        "Cranky(Asserting(Lucene104))"
    );
    let fast = FastCompressingCodec::default_instance().unwrap();
    let fast_text = "FastCompressingStoredFieldsData(storedFieldsFormat=\
                     Lucene90CompressingStoredFieldsFormat(compressionMode=FAST, chunkSize=16384, \
                     maxDocsPerChunk=128, blockShift=10), termVectorsFormat=\
                     Lucene90CompressingTermVectorsFormat(compressionMode=FAST, chunkSize=16384, \
                     maxDocsPerChunk=128, blockSize=10))";
    assert_eq!(java(&fast), fast_text);
    assert_eq!(
        java(&CrankyCodec::new(Arc::new(fast), random())),
        format!("Cranky({fast_text})")
    );
}

// ---------------------------------------------------------------------------
// Misuse
// ---------------------------------------------------------------------------

/// The Java error (class, message) of an operation that returned `Err` or
/// panicked with a Java `Throwable`.
fn outcome(f: impl FnOnce() -> Result<()>) -> (String, String) {
    let result = {
        let _quiet = QuietPanics::new();
        catch_unwind(AssertUnwindSafe(f))
    };
    match result {
        Ok(Ok(())) => ("none".to_string(), "-".to_string()),
        Ok(Err(LuceneError {
            kind: LuceneErrorKind::AssertionError(m),
            ..
        })) => (
            "java.lang.AssertionError".to_string(),
            if m.is_empty() { "null".to_string() } else { m },
        ),
        Ok(Err(e)) => java_error_of(&e),
        Err(payload) => {
            let text = panic_message(&*payload);
            match text.split_once(": ") {
                Some((class, message)) if class.starts_with("java.") => {
                    (class.to_string(), message.to_string())
                }
                _ if text.starts_with("java.") => (text, "null".to_string()),
                _ => ("panic".to_string(), text),
            }
        }
    }
}

fn positioned(leaf: &dyn LeafReader, field: &str) -> Result<Box<dyn rucene::index::TermsEnum>> {
    let mut te = leaf.terms(field)?.unwrap().iterator()?;
    te.next()?;
    Ok(te)
}

fn segment_info(dir: &Arc<dyn Directory>, codec: &Arc<dyn Codec>, name: &str) -> SegmentInfo {
    SegmentInfo::new(
        Arc::clone(dir),
        rucene::util::Version::LATEST,
        None,
        name.to_string(),
        1,
        false,
        false,
        Arc::clone(codec),
        std::collections::HashMap::new(),
        rucene::util::string_helper::StringHelper::random_id(),
        std::collections::HashMap::new(),
        rucene::search::Sort::default(),
    )
    .unwrap()
}

/// `TestCodecsOracle.misuses`, the same misuses in the same order.
fn misuses(nocfs: &Path, scratch: &Path) -> Vec<(String, (String, String))> {
    let mut out = Vec::new();
    let reader = rucene::index::directory_reader::open(open_dir(nocfs)).unwrap();
    let leaves = reader.leaves().unwrap();
    let leaf_arc = leaves[0].leaf_reader();
    let leaf: &dyn LeafReader = &*leaf_arc;
    let mut misuse = |label: &str, f: &dyn Fn() -> Result<()>| {
        out.push((label.to_string(), outcome(f)));
    };
    misuse("te.docFreq unpositioned", &|| {
        leaf.terms("body")?
            .unwrap()
            .iterator()?
            .doc_freq()
            .map(drop)
    });
    misuse("te.term unpositioned", &|| {
        leaf.terms("body")?.unwrap().iterator()?.term().map(drop)
    });
    misuse("te.ord unpositioned", &|| {
        leaf.terms("body")?.unwrap().iterator()?.ord().map(drop)
    });
    misuse("te.postings unpositioned", &|| {
        leaf.terms("body")?
            .unwrap()
            .iterator()?
            .postings(None, POSTINGS_ENUM_FREQS)
            .map(drop)
    });
    misuse("te.totalTermFreq unpositioned", &|| {
        leaf.terms("body")?
            .unwrap()
            .iterator()?
            .total_term_freq()
            .map(drop)
    });
    misuse("te.next after end", &|| {
        let mut te = leaf.terms("id")?.unwrap().iterator()?;
        while te.next()?.is_some() {}
        te.next().map(drop)
    });
    misuse("te.impacts without freqs", &|| {
        positioned(leaf, "body")?
            .impacts(POSTINGS_ENUM_NONE)
            .map(drop)
    });
    misuse("pe.freq before nextDoc", &|| {
        positioned(leaf, "body")?
            .postings(None, POSTINGS_ENUM_FREQS)?
            .ok_or_else(rucene::error::LuceneError::null_pointer)?
            .freq()
            .map(drop)
    });
    misuse("pe.advance backwards", &|| {
        let mut pe = positioned(leaf, "body")?
            .postings(None, POSTINGS_ENUM_FREQS)?
            .ok_or_else(rucene::error::LuceneError::null_pointer)?;
        pe.next_doc()?;
        let doc = pe.doc_id();
        pe.advance(doc).map(drop)
    });
    misuse("pe.nextDoc after end", &|| {
        let mut pe = positioned(leaf, "body")?
            .postings(None, POSTINGS_ENUM_FREQS)?
            .ok_or_else(rucene::error::LuceneError::null_pointer)?;
        while pe.next_doc()? != NO_MORE_DOCS {}
        pe.next_doc().map(drop)
    });
    misuse("pe.advance after end", &|| {
        let mut pe = positioned(leaf, "body")?
            .postings(None, POSTINGS_ENUM_FREQS)?
            .ok_or_else(rucene::error::LuceneError::null_pointer)?;
        while pe.next_doc()? != NO_MORE_DOCS {}
        pe.advance(1).map(drop)
    });
    misuse("pe.nextPosition beyond freq", &|| {
        let mut pe = positioned(leaf, "body")?
            .postings(None, POSTINGS_ENUM_POSITIONS)?
            .ok_or_else(rucene::error::LuceneError::null_pointer)?;
        pe.next_doc()?;
        let freq = pe.freq()?;
        for _ in 0..freq {
            pe.next_position()?;
        }
        pe.next_position().map(drop)
    });
    misuse("pe.nextPosition before nextDoc", &|| {
        positioned(leaf, "body")?
            .postings(None, POSTINGS_ENUM_POSITIONS)?
            .ok_or_else(rucene::error::LuceneError::null_pointer)?
            .next_position()
            .map(drop)
    });
    misuse("pe.getPayload before nextPosition", &|| {
        let mut pe = positioned(leaf, "body")?
            .postings(None, POSTINGS_ENUM_ALL)?
            .ok_or_else(rucene::error::LuceneError::null_pointer)?;
        pe.next_doc()?;
        pe.get_payload().map(drop)
    });
    misuse("pe.startOffset before nextPosition", &|| {
        let mut pe = positioned(leaf, "body")?
            .postings(None, POSTINGS_ENUM_OFFSETS)?
            .ok_or_else(rucene::error::LuceneError::null_pointer)?;
        pe.next_doc()?;
        pe.start_offset();
        Ok(())
    });
    misuse("pe.freq after end", &|| {
        let mut pe = positioned(leaf, "body")?
            .postings(None, POSTINGS_ENUM_FREQS)?
            .ok_or_else(rucene::error::LuceneError::null_pointer)?;
        while pe.next_doc()? != NO_MORE_DOCS {}
        pe.freq().map(drop)
    });
    misuse("pe.freq after reuse", &|| {
        let mut te = positioned(leaf, "body")?;
        let mut pe = te
            .postings(None, POSTINGS_ENUM_FREQS)?
            .ok_or_else(rucene::error::LuceneError::null_pointer)?;
        while pe.next_doc()? != NO_MORE_DOCS {}
        let before = &*pe as *const dyn rucene::index::PostingsEnum as *const ();
        te.next()?;
        let reused = te
            .postings(Some(pe), POSTINGS_ENUM_FREQS)?
            .ok_or_else(rucene::error::LuceneError::null_pointer)?;
        if &*reused as *const dyn rucene::index::PostingsEnum as *const () != before {
            return Err(LuceneError::illegal_state("not reused".to_string()));
        }
        reused.freq().map(drop)
    });
    misuse("terms consumed in another thread", &|| {
        let terms = leaf.terms("body")?.unwrap();
        let thrown = std::thread::scope(|s| {
            s.spawn(|| match terms.iterator() {
                Err(LuceneError {
                    kind: LuceneErrorKind::AssertionError(m),
                    ..
                }) => Some(m),
                Err(e) => Some(e.to_string()),
                Ok(_) => None,
            })
            .join()
            .unwrap()
        });
        match thrown {
            Some(m) => {
                let cut = m.find(" But was acquired in").unwrap_or(m.len());
                Err(LuceneError::from(LuceneErrorKind::AssertionError(
                    m[..cut].to_string(),
                )))
            }
            None => Ok(()),
        }
    });
    misuse("dv.numeric longValue unpositioned", &|| {
        leaf.get_numeric_doc_values("dv_num")?
            .unwrap()
            .long_value()
            .map(drop)
    });
    misuse("dv.numeric advance backwards", &|| {
        let mut dv = leaf.get_numeric_doc_values("dv_num")?.unwrap();
        dv.next_doc()?;
        let doc = dv.doc_id();
        dv.advance(doc).map(drop)
    });
    misuse("dv.numeric advanceExact beyond maxDoc", &|| {
        leaf.get_numeric_doc_values("dv_num")?
            .unwrap()
            .advance_exact(leaf.max_doc())
            .map(drop)
    });
    misuse("dv.binary binaryValue unpositioned", &|| {
        leaf.get_binary_doc_values("dv_bin")?
            .unwrap()
            .binary_value()
            .map(drop)
    });
    misuse("dv.sorted lookupOrd out of range", &|| {
        let dv = leaf.get_sorted_doc_values("dv_sorted")?.unwrap();
        let count = dv.get_value_count()?;
        dv.lookup_ord(count).map(drop)
    });
    misuse("dv.sorted ordValue unpositioned", &|| {
        leaf.get_sorted_doc_values("dv_sorted")?
            .unwrap()
            .ord_value()
            .map(drop)
    });
    misuse("dv.sortedset nextOrd unpositioned", &|| {
        leaf.get_sorted_set_doc_values("dv_sset")?
            .unwrap()
            .next_ord()
            .map(drop)
    });
    misuse("dv.sortednumeric nextValue unpositioned", &|| {
        leaf.get_sorted_numeric_doc_values("dv_snum")?
            .unwrap()
            .next_value()
            .map(drop)
    });
    misuse("livedocs get beyond length", &|| {
        for ctx in &leaves {
            let r = ctx.leaf_reader();
            if let Some(live) = r.get_live_docs() {
                live.get(r.max_doc() as usize);
                return Ok(());
            }
        }
        Err(LuceneError::illegal_state("no deletions".to_string()))
    });
    misuse("livedocs as LiveDocs get beyond length", &|| {
        for ctx in &leaves {
            let r = ctx.leaf_reader();
            if let Some(live) = r.get_live_docs_with_deleted_iterator() {
                if live.deleted_count() != r.max_doc() - r.num_docs() {
                    return Err(LuceneError::illegal_state(format!(
                        "deletedCount {}",
                        live.deleted_count()
                    )));
                }
                live.get(r.max_doc() as usize);
                return Ok(());
            }
        }
        Err(LuceneError::illegal_state("no LiveDocs".to_string()))
    });
    let codec: Arc<dyn Codec> = Arc::new(AssertingCodec::new());
    let dir = open_dir(scratch);
    let ctx: &dyn IOContext = &*rucene::store::DEFAULT_IO_CONTEXT;
    let close_quietly = |r: Result<()>| drop(r);
    misuse("sfw.finishDocument before startDocument", &|| {
        let si = segment_info(&dir, &codec, "_m1");
        let mut w = codec.stored_fields_format().fields_writer(&dir, &si, ctx)?;
        let r = w.finish_document();
        close_quietly(w.close());
        r
    });
    misuse("sfw.startDocument twice", &|| {
        let si = segment_info(&dir, &codec, "_m2");
        let mut w = codec.stored_fields_format().fields_writer(&dir, &si, ctx)?;
        let r = w.start_document().and_then(|_| w.start_document());
        close_quietly(w.close());
        r
    });
    misuse("sfw.finish with a wrong count", &|| {
        let si = segment_info(&dir, &codec, "_m3");
        let mut w = codec.stored_fields_format().fields_writer(&dir, &si, ctx)?;
        let r = w.finish(1);
        close_quietly(w.close());
        r
    });
    misuse("tvw.finishDocument before startDocument", &|| {
        let si = segment_info(&dir, &codec, "_m4");
        let mut w = codec.term_vectors_format().vectors_writer(&dir, &si, ctx)?;
        let r = w.finish_document();
        close_quietly(w.close());
        r
    });
    misuse("tvw.finishDocument with fields left", &|| {
        let si = segment_info(&dir, &codec, "_m5");
        let mut w = codec.term_vectors_format().vectors_writer(&dir, &si, ctx)?;
        let r = w.start_document(1).and_then(|_| w.finish_document());
        close_quietly(w.close());
        r
    });
    misuse("tvw.finish with a wrong count", &|| {
        let si = segment_info(&dir, &codec, "_m6");
        let mut w = codec.term_vectors_format().vectors_writer(&dir, &si, ctx)?;
        let r = w.finish(3);
        close_quietly(w.close());
        r
    });
    out
}

/// Every misuse of an `Asserting` object raises Lucene's error, with
/// Lucene's message, at the same call.
#[test]
fn asserting_misuse_raises_lucenes_errors() {
    register_codecs();
    let tmp = TempDir::new("testcodecs-misuse");
    let nocfs = tmp.0.join("nocfs");
    let scratch = tmp.0.join("scratch");
    std::fs::create_dir_all(&nocfs).unwrap();
    std::fs::create_dir_all(&scratch).unwrap();
    write_files(&nocfs, &java_files(PREFIX, "Asserting/nocfs"));
    let actual = misuses(&nocfs, &scratch);
    let expected: Vec<(String, (String, String))> = rows("testcodecs_misuse.tsv")
        .into_iter()
        .map(|r| (r[0].clone(), (r[1].clone(), r[2].clone())))
        .collect();
    assert_eq!(actual.len(), expected.len());
    let mut mismatches = Vec::new();
    for (a, e) in actual.iter().zip(&expected) {
        assert_eq!(a.0, e.0, "misuse order");
        if a.0 == "livedocs as LiveDocs get beyond length" {
            // Core defect (out of this task's scope): `SegmentReader`'s
            // `SharedBits` handle does not forward `Bits::into_live_docs`, so
            // `getLiveDocsWithDeletedIterator()` never finds the `LiveDocs`
            // Lucene finds (`LeafReader.java:409-412`). The asserting
            // `LiveDocs` is checked on the format itself instead
            // (`asserting_live_docs_format_hands_out_live_docs`).
            assert_eq!(
                a.1,
                (
                    "java.lang.IllegalStateException".to_string(),
                    "no LiveDocs".to_string()
                ),
                "the SharedBits defect is fixed: compare with Lucene again"
            );
            continue;
        }
        if a.1 != e.1 {
            mismatches.push(format!("{}: Rucene {:?}, Lucene {:?}", a.0, a.1, e.1));
        }
    }
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
}

// ---------------------------------------------------------------------------
// Cranky
// ---------------------------------------------------------------------------

/// `TestCodecsOracle.Recording`: a `java.util.Random` that records the bound
/// of every `nextInt(bound)`, and answers 1 for a draw of 0 when
/// `never_zero`.
struct Recording {
    inner: JdkRandom,
    bounds: Mutex<Vec<i32>>,
    never_zero: bool,
}

impl Recording {
    fn new(seed: i64, never_zero: bool) -> Arc<Recording> {
        Arc::new(Recording {
            inner: JdkRandom::new(seed),
            bounds: Mutex::new(Vec::new()),
            never_zero,
        })
    }

    fn bounds(&self) -> Vec<i32> {
        self.bounds.lock().unwrap().clone()
    }
}

impl Random for Recording {
    fn fields(&self) -> &RandomFields {
        self.inner.fields()
    }

    fn next_int_bound(&self, bound: i32) -> i32 {
        let v = self.inner.next_int_bound(bound);
        self.bounds.lock().unwrap().push(bound);

        if self.never_zero && v == 0 {
            1
        } else {
            v
        }
    }
}

/// `SimpleTextOracle.writeCorpus(dir, false, codec)` that stops at the first
/// error, as Java's does when an operation throws (the writer is then closed
/// by try-with-resources, its errors suppressed).
fn try_write_corpus(path: &Path, codec: Arc<dyn Codec>) -> Result<()> {
    let config = new_config(codec);
    let policy = TieredMergePolicy::new();
    policy.set_no_cfs_ratio(0.0)?;
    config.set_merge_policy(Some(Arc::new(policy)))?;
    config.set_use_compound_file(false)?;
    let w = IndexWriter::new(open_dir(path), config)?;
    let result = (|| -> Result<()> {
        let add_batch = |batch: usize| -> Result<()> {
            let (from, to) = BATCHES[batch];
            for i in from..to {
                w.add_document(&doc(i))?;
            }
            Ok(())
        };
        add_batch(0)?;
        w.commit()?;
        w.delete_documents_by_terms(vec![term("id", &id(3))])?;
        w.delete_documents_by_terms(vec![term("id", &id(17))])?;
        w.delete_documents_by_queries(vec![IntPoint::new_range_query("p_int", &[50], &[60])?])?;
        add_batch(1)?;
        w.update_numeric_doc_value(term("id", &id(5)), "dv_num", 777)?;
        w.update_binary_doc_value(term("id", &id(130)), "dv_bin", bytes("updated-130"))?;
        w.commit()?;
        add_batch(2)?;
        w.commit()?;
        w.force_merge(2)?;
        add_batch(3)?;
        w.delete_documents_by_terms(vec![term("id", &id(250))])?;
        w.delete_documents_by_queries(vec![IntPoint::new_range_query("p_int", &[190], &[192])?])?;
        w.update_numeric_doc_value(term("id", &id(300)), "dv_num", -999)?;
        w.update_binary_doc_value(term("id", &id(10)), "dv_bin", bytes("updated-10"))?;
        w.commit()?;
        w.delete_documents_by_queries(vec![IntPoint::new_range_query("p_2d", &[0, 3], &[19, 4])?])?;
        w.force_merge_deletes()?;
        w.commit()?;
        Ok(())
    })();
    let closed = w.close();
    result.and(closed)
}

/// The deepest cause of `e` as Java's (class, message): the cranky
/// `IOException` an `IndexWriter` error carries.
fn root_cause(e: &LuceneError) -> (String, String) {
    let mut current: &(dyn std::error::Error + 'static) = e;
    let mut deepest_lucene: &LuceneError = e;
    while let Some(next) = current.source() {
        if let Some(l) = next.downcast_ref::<LuceneError>() {
            deepest_lucene = l;
        }
        if let Some(io) = next.downcast_ref::<std::io::Error>() {
            return (
                java_class_of(io).to_string(),
                java_message_of(io).unwrap_or_else(|| "null".to_string()),
            );
        }
        current = next;
    }
    match deepest_lucene {
        LuceneError {
            kind: LuceneErrorKind::Io(io),
            ..
        } => (
            java_class_of(io).to_string(),
            java_message_of(io).unwrap_or_else(|| "null".to_string()),
        ),
        other => java_error_of(other),
    }
}

/// The index of the first draw of 0 in `bounds` replayed from `seed`, or the
/// number of draws (`TestCodecsOracle.firstFailure`).
fn first_failure(bounds: &[i32], seed: i64) -> usize {
    let replay = JdkRandom::new(seed);
    bounds
        .iter()
        .position(|&b| replay.next_int_bound(b) == 0)
        .unwrap_or(bounds.len())
}

/// Lucene's draws of `testcodecs_cranky_draws.tsv`: (bound, cranky method).
fn lucene_draws() -> Vec<(i32, String)> {
    rows("testcodecs_cranky_draws.tsv")
        .into_iter()
        .map(|r| (r[1].parse().unwrap(), r[2].clone()))
        .collect()
}

const MIN_PACKED: &str = "CrankyPointsFormat$CrankyPointsReader$1.getMinPackedValue";
const MAX_PACKED: &str = "CrankyPointsFormat$CrankyPointsReader$1.getMaxPackedValue";

/// Whether Lucene's draws `j` and `j + 1` are the `getMinPackedValue()` /
/// `getMaxPackedValue()` pair `PointRangeQuery`'s scorer supplier reads a
/// second time for its all-docs-match test (`PointRangeQuery.java:280-281`),
/// right after the pair of its bounds test (`:263-264`): Rucene's
/// `point_range_query.rs` reuses the first pair instead (core divergence,
/// out of this task's scope), so those two draws are Lucene's only.
fn is_reread_pair(java: &[(i32, String)], j: usize) -> bool {
    j >= 2
        && j + 1 < java.len()
        && java[j - 2].1 == MIN_PACKED
        && java[j - 1].1 == MAX_PACKED
        && java[j].1 == MIN_PACKED
        && java[j + 1].1 == MAX_PACKED
}

/// Rucene's draws against Lucene's: identical but for the re-read pairs of
/// [`is_reread_pair`]; returns the indices (in Lucene's draws) of the pairs
/// skipped, or the first mismatch.
fn align(rust: &[i32], java: &[(i32, String)]) -> std::result::Result<Vec<usize>, String> {
    let (mut i, mut j) = (0, 0);
    let mut skipped = Vec::new();
    while i < rust.len() || j < java.len() {
        if i < rust.len() && j < java.len() && rust[i] == java[j].0 && !is_reread_pair(java, j) {
            i += 1;
            j += 1;
        } else if is_reread_pair(java, j) {
            skipped.push(j);
            j += 2;
        } else {
            return Err(format!(
                "Rucene draw {i} {:?}, Lucene draw {j} {:?}",
                rust.get(i..(i + 6).min(rust.len())),
                java.get(j..(j + 6).min(java.len()))
            ));
        }
    }
    Ok(skipped)
}

/// A `CrankyCodec` writing the corpus draws Lucene's numbers, with Lucene's
/// bounds, in Lucene's order — but for the draws of the core divergence of
/// [`is_reread_pair`] — and the corpus it writes is the delegate's.
#[test]
fn cranky_draws_lucenes_sequence() {
    register_codecs();
    let random = Recording::new(0, true);
    let tmp = TempDir::new("testcodecs-cranky-draws");
    write_corpus(
        &tmp.0,
        false,
        Arc::new(CrankyCodec::new(
            Arc::new(Lucene104Codec::new()),
            Arc::clone(&random) as Arc<dyn Random>,
        )),
    );
    let java = lucene_draws();
    let skipped = align(&random.bounds(), &java).unwrap_or_else(|e| panic!("{e}"));
    // Every re-read of Lucene's, and nothing else, is what Rucene skips: one
    // per segment a point-range query of the corpus's deletes runs on whose
    // points all documents have (`values.getDocCount() == reader.maxDoc()`).
    let rereads: Vec<usize> = (0..java.len())
        .filter(|&j| is_reread_pair(&java, j))
        .collect();
    assert_eq!(skipped, rereads, "re-read pairs");
    assert_eq!(skipped.len(), 5);
    // Cranky impersonates its delegate: the files are the default codec's.
    let plain = TempDir::new("testcodecs-cranky-plain");
    write_corpus(&plain.0, false, Arc::new(Lucene104Codec::new()));
    let cranky_files = read_dir(&tmp.0);
    let plain_files = read_dir(&plain.0);
    assert_eq!(
        cranky_files.keys().collect::<Vec<_>>(),
        plain_files.keys().collect::<Vec<_>>()
    );
    for (name, bytes) in &plain_files {
        if name.starts_with("segments_") || name.ends_with(".si") {
            continue;
        }
        let expected = java_bytes_with_rust_id(bytes, &cranky_files[name]);
        assert!(cranky_files[name] == expected, "{name}");
    }
}

/// For every seed, a `CrankyCodec` over `new Random(seed)` fails at Lucene's
/// draw with Lucene's `IOException`. A seed whose first failure in Lucene
/// comes after the first draw of the core divergence of [`is_reread_pair`]
/// draws from a shifted sequence in Rucene: it is checked to fail with a
/// cranky `IOException` only, and counted.
#[test]
fn cranky_fails_where_lucene_fails() {
    register_codecs();
    let java = lucene_draws();
    let divergence = (0..java.len())
        .find(|&j| is_reread_pair(&java, j))
        .expect("Lucene re-reads the packed values");
    let mut mismatches = Vec::new();
    let mut exact = 0;
    let mut shifted = Vec::new();
    for row in rows("testcodecs_cranky_seeds.tsv") {
        let seed: i64 = row[0].parse().unwrap();
        let random = Recording::new(seed, false);
        let tmp = TempDir::new(&format!("testcodecs-cranky-{seed}"));
        let result = try_write_corpus(
            &tmp.0,
            Arc::new(CrankyCodec::new(
                Arc::new(Lucene104Codec::new()),
                Arc::clone(&random) as Arc<dyn Random>,
            )),
        );
        let (class, message) = match &result {
            Ok(()) => ("none".to_string(), "-".to_string()),
            Err(e) => root_cause(e),
        };
        let draws = first_failure(&random.bounds(), seed).to_string();
        let actual = vec![row[0].clone(), draws, class, message];
        let java_draws: usize = row[1].parse().unwrap();
        if java_draws < divergence {
            exact += 1;
            if actual != row {
                mismatches.push(format!("Rucene {actual:?}, Lucene {row:?}"));
            }
        } else {
            shifted.push(seed);
            if actual[2] != "java.io.IOException" || !actual[3].starts_with("Fake IOException") {
                mismatches.push(format!("seed {seed}: not a cranky failure: {actual:?}"));
            }
        }
    }
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
    assert!(
        exact >= 20,
        "{exact} seeds compared exactly, shifted: {shifted:?}"
    );
}

/// `CompressingCodec`'s random factories draw Lucene's codec and parameters
/// for the same seed (or raise Lucene's error for parameters out of range).
#[test]
fn compressing_random_instances_are_lucenes() {
    type Factory =
        fn(&dyn Random) -> Result<rucene_test_framework::codecs::compressing::CompressingCodec>;
    use rucene_test_framework::codecs::compressing::CompressingCodec;
    let factories: [(&str, Factory); 3] = [
        ("randomInstance", |r| CompressingCodec::random_instance(r)),
        ("reasonableInstance", |r| {
            CompressingCodec::reasonable_instance(r)
        }),
        ("randomInstance(suffix)", |r| {
            CompressingCodec::random_instance_with_suffix(r, true)
        }),
    ];
    let rows = rows("testcodecs_compressing_random.tsv");
    assert_eq!(rows.len(), 90);
    for row in rows {
        let seed: i64 = row[0].parse().unwrap();
        let (_, factory) = factories.iter().find(|(n, _)| *n == row[1]).unwrap();
        let actual = match factory(&JdkRandom::new(seed)) {
            // Through `dyn Codec`, as Java's `"" + codec` resolves the override.
            Ok(codec) => vec![(&codec as &dyn Codec).to_string()],
            Err(e) => {
                let (class, message) = java_error_of(&e);
                vec![class, message]
            }
        };
        assert_eq!(actual, row[2..].to_vec(), "seed {seed} {}", row[1]);
    }
}

/// `AssertingLiveDocsFormat.readLiveDocs` hands out an `AssertingLiveDocs`
/// (Lucene's narrowing: the default format's bits are `LiveDocs`), whose
/// `deletedCount` is the segment's and whose `get` asserts its bound with
/// Lucene's message (`testcodecs_misuse.tsv`, `livedocs as LiveDocs get beyond
/// length`).
#[test]
fn asserting_live_docs_format_hands_out_live_docs() {
    register_codecs();
    let tmp = TempDir::new("testcodecs-livedocs");
    write_files(&tmp.0, &java_files(PREFIX, "Asserting/nocfs"));
    let dir = open_dir(&tmp.0);
    let infos = rucene::index::SegmentInfos::read_latest_commit(Arc::clone(&dir)).unwrap();
    let sci = infos
        .iter()
        .find(|sci| sci.get_del_count() > 0)
        .expect("a segment with deletions");
    let format = AssertingCodec::new();
    let ctx: &dyn IOContext = &*rucene::store::DEFAULT_IO_CONTEXT;
    let bits = format
        .live_docs_format()
        .read_live_docs(&*dir, sci, ctx)
        .unwrap();
    let live = bits
        .into_live_docs()
        .expect("Lucene90's live docs are LiveDocs");
    assert_eq!(live.deleted_count(), sci.get_del_count());
    let max_doc = sci.info.max_doc().unwrap();
    let expected = rows("testcodecs_misuse.tsv")
        .into_iter()
        .find(|r| r[0] == "livedocs as LiveDocs get beyond length")
        .unwrap();
    let (class, message) = outcome(|| {
        live.get(max_doc as usize);
        Ok(())
    });
    assert_eq!(vec![class, message], expected[1..].to_vec());
}

// ---------------------------------------------------------------------------
// rmp #341: the postings formats of `blockterms`, `bloom`, `mockrandom`,
// `ramonly` and `uniformsplit`, and the crate's service files
// ---------------------------------------------------------------------------

use rucene_test_framework::codecs::blockterms::{
    LuceneFixedGap, LuceneVarGapDocFreqInterval, LuceneVarGapFixedInterval,
};
use rucene_test_framework::codecs::bloom::TestBloomFilteredLucenePostings;
use rucene_test_framework::codecs::mockrandom::MockRandomPostingsFormat;
use rucene_test_framework::codecs::ramonly::RAMOnlyPostingsFormat;
use rucene_test_framework::codecs::uniformsplit::sharedterms::STUniformSplitRot13PostingsFormat;
use rucene_test_framework::codecs::uniformsplit::uniform_split_rot13_postings_format as rot13;
use rucene_test_framework::codecs::uniformsplit::UniformSplitRot13PostingsFormat;

fn fixed_gap_codec() -> Arc<dyn Codec> {
    per_field_postings_codec(Arc::new(LuceneFixedGap::new()))
}
fn var_gap_fixed_codec() -> Arc<dyn Codec> {
    per_field_postings_codec(Arc::new(LuceneVarGapFixedInterval::new()))
}
fn var_gap_doc_freq_codec() -> Arc<dyn Codec> {
    per_field_postings_codec(Arc::new(LuceneVarGapDocFreqInterval::new()))
}
fn test_bloom_codec() -> Arc<dyn Codec> {
    per_field_postings_codec(Arc::new(TestBloomFilteredLucenePostings::new()))
}
fn rot13_codec() -> Arc<dyn Codec> {
    per_field_postings_codec(Arc::new(UniformSplitRot13PostingsFormat::new()))
}
fn st_rot13_codec() -> Arc<dyn Codec> {
    per_field_postings_codec(Arc::new(STUniformSplitRot13PostingsFormat::new()))
}
/// `new MockRandomPostingsFormat(new Random(SEED))`, a new one per corpus, as
/// `TestCodecsOracle` makes it.
fn mock_random_codec<const SEED: i64>() -> Arc<dyn Codec> {
    per_field_postings_codec(Arc::new(MockRandomPostingsFormat::with_random(
        &JdkRandom::new(SEED),
    )))
}

/// `TEST_FRAMEWORK_JARS` and Hamcrest 2.2, which `LuceneTestCase` needs:
/// `MockRandomPostingsFormat` reads `LuceneTestCase.VERBOSE`.
const TEST_FRAMEWORK_RUNTIME_JARS: &[&str] = &[
    "org/apache/lucene/lucene-test-framework/10.5.0/lucene-test-framework-10.5.0.jar",
    "com/carrotsearch/randomizedtesting/randomizedtesting-runner/2.8.1/randomizedtesting-runner-2.8.1.jar",
    "junit/junit/4.13.1/junit-4.13.1.jar",
    "org/hamcrest/hamcrest/2.2/hamcrest-2.2.jar",
];

const fn postings_oracle(
    prefix: &'static str,
    format_name: &'static str,
    extensions: &'static [&'static str],
    codec: fn() -> Arc<dyn Codec>,
) -> FormatOracle {
    FormatOracle {
        prefix,
        format_name,
        extensions,
        codec,
        extra_jars: TEST_FRAMEWORK_RUNTIME_JARS,
        // Every format keeps its postings in `Lucene104PostingsWriter`'s files.
        live_impacts_differ: true,
    }
}

/// The formats and their oracles: `TestCodecsOracle`'s `tc_*` fixtures, and
/// `lucene-codecs`' fixtures of the blockterms assemblies (written by
/// `PostingsFormatOracle` with these classes, found by name).
const POSTINGS_ORACLES: [FormatOracle; 9] = [
    postings_oracle(
        "fixedgap",
        "LuceneFixedGap",
        &["tib", "tii"],
        fixed_gap_codec,
    ),
    postings_oracle(
        "vargap_fixed",
        "LuceneVarGapFixedInterval",
        &["tib", "tiv", "tmv"],
        var_gap_fixed_codec,
    ),
    postings_oracle(
        "vargap_docfreq",
        "LuceneVarGapDocFreqInterval",
        &["tib", "tiv", "tmv"],
        var_gap_doc_freq_codec,
    ),
    postings_oracle(
        "tc_bloom",
        "TestBloomFilteredLucenePostings",
        &["blm"],
        test_bloom_codec,
    ),
    postings_oracle(
        "tc_rot13",
        "UniformSplitRot13",
        &["ustb", "ustd"],
        rot13_codec,
    ),
    postings_oracle(
        "tc_strot13",
        "STUniformSplitRot13",
        &["stustb", "stustd"],
        st_rot13_codec,
    ),
    postings_oracle(
        "tc_mockrandom_s1",
        "MockRandom",
        &["sd"],
        mock_random_codec::<1>,
    ),
    postings_oracle(
        "tc_mockrandom_s2",
        "MockRandom",
        &["sd"],
        mock_random_codec::<2>,
    ),
    postings_oracle(
        "tc_mockrandom_s3",
        "MockRandom",
        &["sd"],
        mock_random_codec::<3>,
    ),
];

/// The two Rot13 formats, which also write the `shared` case.
const ROT13_ORACLES: [usize; 2] = [4, 5];

/// Every format writes Lucene's bytes for the `nocfs`, `cfs` and `edge`
/// corpus: for `MockRandom` the seed file and the files of every terms
/// dictionary its seeds pick; for the Rot13 formats the encoded blocks,
/// fields metadata and dictionary.
#[test]
fn postings_formats_write_lucenes_bytes() {
    register_codecs();
    for oracle in &POSTINGS_ORACLES {
        let _serial = rot13_serial(oracle);
        // Lucene writes a `.blm`'s filters in `HashMap<FieldInfo, ?>` order
        // (identity hash codes); Rucene in field order (`rucene-codecs`'
        // declared divergence): the filters are compared in Rucene's order.
        let compared = oracle.assert_writes_lucenes_bytes_with(blm_in_rucenes_order);
        assert!(compared > 0, "{}: {compared} files compared", oracle.prefix);
    }
}

/// `MockRandom`'s seeds pick every terms dictionary Lucene's do: the format
/// files of the three seeds hold each branch's extensions.
#[test]
fn mock_random_seeds_pick_every_terms_dictionary() {
    let mut extensions = std::collections::BTreeSet::new();
    for oracle in &POSTINGS_ORACLES[6..] {
        for case in ["nocfs", "cfs", "edge"] {
            for name in java_files(oracle.prefix, case).keys() {
                if name.contains("_MockRandom_") {
                    extensions.insert(name.rsplit('.').next().unwrap().to_string());
                }
            }
        }
    }
    // FSTTermsWriter (tfp), Lucene103BlockTreeTermsWriter (tim/tip/tmd),
    // BlockTermsWriter with a fixed-gap (tii) and a variable-gap (tiv/tmv)
    // index (tib), OrdsBlockTreeTermsWriter (tio/tipo), and the seed (sd).
    for ext in [
        "sd", "tfp", "tim", "tip", "tmd", "tib", "tii", "tiv", "tmv", "tio", "tipo",
    ] {
        assert!(extensions.contains(ext), "{ext} in {extensions:?}");
    }
}

#[test]
fn postings_formats_read_what_lucene_reads() {
    register_codecs();
    for oracle in &POSTINGS_ORACLES {
        let _serial = rot13_serial(oracle);
        oracle.assert_reads_what_lucene_reads();
    }
}

/// Core defect 5 of `rucene-codecs`' bloom oracle, outside this crate: the
/// intersection of `Lucene103`'s terms dictionary (the bloom format's
/// delegate, and the `MockRandom` segments whose seed picks the block tree)
/// checks Java's `assert state != -1` with `debug_assert!`, a panic where
/// Lucene under `-ea` throws the `AssertionError` its trace lists. The trace
/// lists the panic as that error, and requires it on both sides.
const LUCENE103_INTERSECT_PANIC: &str = "assertion failed: state != -1";

#[test]
fn postings_formats_enum_trace_matches_lucenes() {
    register_codecs();
    // Per side (Lucene's files, Rucene's), the core panics met.
    let mut panics = [0usize; 2];
    for oracle in &POSTINGS_ORACLES {
        let _serial = rot13_serial(oracle);
        for case in ["nocfs", "cfs", "edge"] {
            let expected: Vec<String> = rows(&format!("{}_enum.tsv", oracle.prefix))
                .into_iter()
                .filter(|r| r[0] == case)
                .map(|r| r[1].clone())
                .collect();
            let java_dir = TempDir::new(&format!("{}-enum-java-{case}", oracle.prefix));
            write_files(&java_dir.0, &java_files(oracle.prefix, case));
            let rust_dir = TempDir::new(&format!("{}-enum-rust-{case}", oracle.prefix));
            oracle.write_case(case, &rust_dir.0);
            for (i, (side, dir)) in [("Lucene's", &java_dir.0), ("Rucene's", &rust_dir.0)]
                .into_iter()
                .enumerate()
            {
                let known = cfg!(debug_assertions).then_some(LUCENE103_INTERSECT_PANIC);
                let trace = enum_trace_with(dir, known, &mut panics[i]);
                let what = format!("{}: {case}: enum trace of {side} files", oracle.prefix);
                assert_same_lines(&what, &trace, &expected);
            }
        }
    }
    if cfg!(debug_assertions) {
        assert!(
            panics.iter().all(|&p| p > 0),
            "no `{LUCENE103_INTERSECT_PANIC}` panic on one side ({panics:?}): the core is \
             fixed, remove the exclusion"
        );
    }
}

#[test]
fn lucene_reads_what_postings_formats_write() {
    register_codecs();
    for oracle in &POSTINGS_ORACLES {
        let _serial = rot13_serial(oracle);
        oracle.assert_lucene_reads_what_rucene_writes();
    }
}

#[test]
fn postings_formats_corrupted_files_raise_lucenes_errors() {
    register_codecs();
    // The variable-gap readers check `.tmv`'s footer with the prior error of
    // its corrupted header; the message is Lucene's, the prior error being
    // suppressed (`CodecUtil.java:506-508`), as in `rucene-codecs`'
    // blockterms oracle.
    for o in &POSTINGS_ORACLES {
        let _serial = rot13_serial(o);
        o.assert_corrupted_files_raise_lucenes_errors();
    }
}

/// The `shared` case of the Rot13 formats (terms in several fields, every
/// index option, a merge).
#[test]
fn rot13_shared_case_matches_lucene() {
    register_codecs();
    for i in ROT13_ORACLES {
        let f = &POSTINGS_ORACLES[i];
        let _serial = rot13_serial(f);
        let write = |path: &Path| write_shared(path, (f.codec)());
        assert!(
            f.assert_case_writes_lucenes_bytes("shared", write, |_, java, _| java) > 0,
            "{}",
            f.prefix
        );
        f.assert_case_reads_what_lucene_reads("shared", write);
        f.assert_case_enum_trace_matches("shared", write);
        f.assert_case_lucene_reads_what_rucene_writes("shared", write);
    }
}

/// Each format resolves by its name and prints Lucene's `toString()`.
#[test]
fn postings_formats_resolve_by_name() {
    register_codecs();
    let concrete: [(&str, String); 7] = [
        ("fixedgap", LuceneFixedGap::new().to_string()),
        ("vargap_fixed", LuceneVarGapFixedInterval::new().to_string()),
        (
            "vargap_docfreq",
            LuceneVarGapDocFreqInterval::new().to_string(),
        ),
        (
            "tc_bloom",
            TestBloomFilteredLucenePostings::new().to_string(),
        ),
        (
            "tc_rot13",
            UniformSplitRot13PostingsFormat::new().to_string(),
        ),
        (
            "tc_strot13",
            STUniformSplitRot13PostingsFormat::new().to_string(),
        ),
        (
            "tc_mockrandom_s1",
            MockRandomPostingsFormat::new().to_string(),
        ),
    ];
    for (prefix, to_string) in concrete {
        let oracle = POSTINGS_ORACLES
            .iter()
            .find(|o| o.prefix == prefix)
            .unwrap();
        let spi = oracle.spi();
        let found = rucene::codecs::postings_for_name(&spi["getName"])
            .unwrap_or_else(|| panic!("{} resolves", spi["getName"]));
        assert_eq!(found.name(), spi["getName"], "{prefix}");
        assert_eq!(to_string, spi["toString"], "{prefix}");
        assert_eq!(to_string, spi["forName.toString"], "{prefix}");
    }
}

/// Every entry of `lucene-test-framework`'s four service files is a provider
/// of the crate's class loader, in Lucene's order, and its name resolves to a
/// provider of the same name (and, for codecs, Lucene's `toString()`).
#[test]
fn test_framework_services_resolve() {
    register_codecs();
    let rows = rows("testcodecs_spi.tsv");
    assert_eq!(rows.len(), 20);
    let loader = rucene_test_framework::class_loader();
    let services = [
        "Codec",
        "DocValuesFormat",
        "KnnVectorsFormat",
        "PostingsFormat",
    ];
    let mut own: Vec<(String, String)> = Vec::new();
    for service in services {
        let name = format!("org.apache.lucene.codecs.{service}");
        let all = loader.get_resources(&name);
        let parent = rucene_codecs::class_loader().get_resources(&name).len();
        for p in &all[parent..] {
            own.push((service.to_string(), p.class_name().to_string()));
        }
    }
    let java: Vec<(String, String)> = rows.iter().map(|r| (r[0].clone(), r[1].clone())).collect();
    assert_eq!(own, java);
    for r in &rows {
        let (service, name, to_string) = (&r[0], &r[2], &r[3]);
        let found = match service.as_str() {
            "Codec" => {
                let c = rucene::codecs::for_name(name).expect("codec resolves");
                assert_eq!(&c.to_string(), to_string);
                c.name().to_string()
            }
            "DocValuesFormat" => rucene::codecs::doc_values::doc_values_for_name(name)
                .expect("doc values format resolves")
                .name()
                .to_string(),
            "KnnVectorsFormat" => rucene::codecs::knn_vectors::knn_vectors_for_name(name)
                .expect("knn format resolves")
                .name()
                .to_string(),
            _ => rucene::codecs::postings_for_name(name)
                .expect("postings format resolves")
                .name()
                .to_string(),
        };
        assert_eq!(&found, name, "{}", r[1]);
    }
}

/// Serializes the tests that write, read or reset through the Rot13 formats:
/// their encoder and decoder record calls in process-wide static flags (as
/// Java's static fields), so a concurrent write or read through either format
/// changes what `rot13_encoding_flags_are_lucenes` observes.
static ROT13_FLAGS: Mutex<()> = Mutex::new(());

/// Holds [`ROT13_FLAGS`] while `oracle` writes or reads through a Rot13
/// format; `None` for the other formats.
fn rot13_serial(oracle: &FormatOracle) -> Option<MutexGuard<'static, ()>> {
    oracle
        .format_name
        .contains("Rot13")
        .then(|| ROT13_FLAGS.lock().unwrap_or_else(|e| e.into_inner()))
}

/// The Rot13 formats' encoding flags after writing the `nocfs` corpus and
/// after opening it are Lucene's (`testcodecs_rot13_flags.tsv`): the
/// overridden `writeDictionary` and `writeEncodedFieldsMetadata` record the
/// block, fields-metadata and dictionary encodings in Lucene's order.
#[test]
fn rot13_encoding_flags_are_lucenes() {
    register_codecs();
    let _serial = ROT13_FLAGS.lock().unwrap_or_else(|e| e.into_inner());
    use std::sync::atomic::Ordering::SeqCst;
    let flags = || {
        format!(
            "{}\t{}\t{}\t{}\t{}",
            rot13::ENCODER_CALLED.load(SeqCst),
            rot13::BLOCKS_ENCODED.load(SeqCst),
            rot13::FIELDS_METADATA_ENCODED.load(SeqCst),
            rot13::DICTIONARY_ENCODED.load(SeqCst),
            rot13::DECODER_CALLED.load(SeqCst)
        )
    };
    let mut lines = Vec::new();
    for name in ["UniformSplitRot13", "STUniformSplitRot13"] {
        UniformSplitRot13PostingsFormat::reset_encoding_flags();
        let tmp = TempDir::new(&format!("rot13-flags-{name}"));
        let format = rucene::codecs::postings_for_name(name).unwrap();
        write_corpus(&tmp.0, false, per_field_postings_codec(format));
        lines.push(format!("{name}\twrite\t{}", flags()));
        let reader = rucene::index::directory_reader::open(open_dir(&tmp.0)).unwrap();
        lines.push(format!("{name}\tread\t{}", flags()));
        drop(reader);
    }
    let java: Vec<String> = rows("testcodecs_rot13_flags.tsv")
        .iter()
        .map(|r| r.join("\t"))
        .collect();
    assert_eq!(lines, java);
}

/// `Rot13CypherTestUtil`: encode then decode is the identity, `decode`
/// skips the 7-byte offset, and the encoder and decoder it hands out do the
/// same; a negative decoded length fails as Java's `new byte[-1]` does.
#[test]
fn rot13_cypher_round_trips() {
    use rucene::store::{ByteArrayDataInput, DataInput};
    use rucene_test_framework::codecs::uniformsplit::Rot13CypherTestUtil;
    let plain: Vec<u8> = (0..=255u8).collect();
    let encoded = Rot13CypherTestUtil::encode(&mut ByteArrayDataInput::new(&plain), 256).unwrap();
    assert_eq!(encoded.len(), 263);
    assert_eq!(&encoded[..7], &[0; 7]);
    assert_eq!(encoded[7], 13);
    assert_eq!(encoded[7 + 250], 250u8.wrapping_add(13));
    let decoded = Rot13CypherTestUtil::decode(&mut ByteArrayDataInput::new(&encoded), 263).unwrap();
    assert_eq!(decoded, plain);
    let mut input = ByteArrayDataInput::new(&encoded);
    let e = Rot13CypherTestUtil::decode(&mut input, 6).unwrap_err();
    assert_eq!(
        java_error_of(&e),
        (
            "java.lang.NegativeArraySizeException".to_string(),
            "-1".to_string()
        )
    );
    let bytes = Rot13CypherTestUtil::get_block_encoder()
        .encode(&mut ByteArrayDataInput::new(&plain), 256)
        .unwrap();
    assert_eq!(bytes.size(), 263);
    let round = Rot13CypherTestUtil::get_block_decoder()
        .decode(&mut ByteArrayDataInput::new(&encoded), 263)
        .unwrap();
    assert_eq!(round.slice(), &plain[..]);
    let _ = input.read_byte();
}

/// The read-time `MockRandomPostingsFormat()` refuses to write, as Java's
/// throwing random does on its first draw.
#[test]
fn mock_random_read_time_format_refuses_to_write() {
    register_codecs();
    let tmp = TempDir::new("mockrandom-read-time");
    let codec = per_field_postings_codec(Arc::new(MockRandomPostingsFormat::new()));
    let e = try_write_corpus(&tmp.0, codec).unwrap_err();
    assert_eq!(
        root_cause(&e),
        (
            "java.lang.IllegalStateException".to_string(),
            "Please use MockRandomPostingsFormat(Random)".to_string()
        )
    );
}

// -- RAMOnly ----------------------------------------------------------------

/// [`root_cause`], with the Java class and message of the errors
/// `java_error_of` does not name: `UnsupportedOperationException` and
/// `AssertionError` (`null` for an `assert` without a message).
fn java_root_cause(e: &LuceneError) -> (String, String) {
    let mut deepest = e;
    let mut current: &(dyn std::error::Error + 'static) = e;
    while let Some(next) = current.source() {
        if let Some(l) = next.downcast_ref::<LuceneError>() {
            deepest = l;
        }
        current = next;
    }
    match deepest {
        LuceneError {
            kind: LuceneErrorKind::UnsupportedOperation { message: m, .. },
            ..
        } => (
            "java.lang.UnsupportedOperationException".to_string(),
            m.clone(),
        ),
        LuceneError {
            kind: LuceneErrorKind::AssertionError(m),
            ..
        } => (
            "java.lang.AssertionError".to_string(),
            if m.is_empty() {
                "null".to_string()
            } else {
                m.clone()
            },
        ),
        _ => root_cause(e),
    }
}

/// `TestCodecsOracle.writeRamOnly(Directory, Codec, boolean, boolean)`.
fn write_ram_only(path: &Path, codec: Arc<dyn Codec>, freqs: bool, merge: bool) -> Result<()> {
    use rucene::document::{Document, Field, Store, StringField};
    let config = new_config(codec);
    let policy = TieredMergePolicy::new();
    policy.set_no_cfs_ratio(0.0)?;
    config.set_merge_policy(Some(Arc::new(policy)))?;
    config.set_use_compound_file(false)?;
    let freqs_type = shared_type(rucene::index::IndexOptions::DOCS_AND_FREQS);
    let positions = shared_type(rucene::index::IndexOptions::DOCS_AND_FREQS_AND_POSITIONS);
    let words = ["a", "b\u{e9}", "\u{1d400}x", "\u{e000}y", "z", "\u{1f600}"];
    let w = IndexWriter::new(open_dir(path), config)?;
    let result = (|| -> Result<()> {
        for batch in 0..2 {
            for i in batch * 60..batch * 60 + 60 {
                let mut d = Document::new();
                d.add(Box::new(StringField::new(
                    "e_id",
                    format!("{i:04}"),
                    Store::NO,
                )?));
                if freqs {
                    d.add(Box::new(Field::new(
                        "rf",
                        words[i % 6].to_string(),
                        freqs_type.clone(),
                    )?));
                }
                let rq = |w: &str| format!("{w}{}", i % 13);
                let rp = |w: &str| format!("{w}{}", i % 17);
                d.add(Box::new(Field::new(
                    "rq",
                    rq(words[i % 6]),
                    positions.clone(),
                )?));
                d.add(Box::new(Field::new(
                    "rq",
                    rq(words[(i / 2) % 6]),
                    positions.clone(),
                )?));
                d.add(Box::new(Field::new(
                    "rp",
                    rp(words[i % 5]),
                    positions.clone(),
                )?));
                d.add(Box::new(Field::new(
                    "rp",
                    rp(words[i % 3]),
                    positions.clone(),
                )?));
                w.add_document(&d)?;
            }
            w.commit()?;
            if batch == 0 {
                for i in (0..60).step_by(7) {
                    w.delete_documents_by_terms(vec![term("e_id", &format!("{i:04}"))])?;
                }
            }
        }
        if merge {
            w.force_merge(1)?;
            w.commit()?;
        }
        Ok(())
    })();
    let closed = w.close();
    result.and(closed)
}

/// `TestCodecsOracle.probe`: the value, `null`, `x<hex>` for a term, or `!`
/// and the simple name of the Java exception (a returned error or a panic
/// with a Java throwable).
fn probe<T: std::fmt::Display>(f: impl FnOnce() -> Result<T>) -> String {
    let result = {
        let _quiet = QuietPanics::new();
        catch_unwind(AssertUnwindSafe(f))
    };
    match result {
        Ok(Ok(v)) => v.to_string(),
        Ok(Err(e)) => format!("!{}", e.java_simple_name()),
        Err(payload) => {
            let text = panic_message(&*payload);
            let class = text.split(':').next().unwrap_or(&text);
            format!("!{}", class.rsplit('.').next().unwrap_or(class))
        }
    }
}

/// A term as `TestCodecsOracle.probe` prints it.
struct X(Option<rucene::util::BytesRef>);

impl std::fmt::Display for X {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.0 {
            Some(b) => write!(f, "x{}", hex(b.slice())),
            None => f.write_str("null"),
        }
    }
}

/// A `SeekStatus` as Java prints it.
fn status(s: rucene::index::SeekStatus) -> String {
    format!("{s:?}")
}

/// `TestCodecsOracle.ramOnlyTrace(Path)`.
fn ram_only_trace(path: &Path) -> Vec<String> {
    use rucene::index::{IndexReader, POSTINGS_ENUM_ALL};
    use rucene::util::BytesRef;
    let reader: Arc<dyn IndexReader> =
        rucene::index::directory_reader::open(open_dir(path)).unwrap();
    let mut out = Vec::new();
    for ctx in Arc::clone(&reader).leaves().unwrap() {
        let leaf = ctx.leaf_reader();
        out.push(format!("leaf {} maxDoc={}", ctx.ord(), leaf.max_doc()));
        let fis: Vec<rucene::index::FieldInfo> = leaf.get_field_infos().iter().cloned().collect();
        for fi in &fis {
            if fi.name == "e_id" {
                continue;
            }
            let Some(terms) = leaf.terms(&fi.name).unwrap() else {
                continue;
            };
            out.push(format!(
                " field {} size={} sttf={} sdf={} dc={} {}/{}/{}/{}",
                fi.name,
                terms.size().unwrap(),
                terms.sum_total_term_freq().unwrap(),
                terms.sum_doc_freq().unwrap(),
                terms.doc_count().unwrap(),
                terms.has_freqs(),
                terms.has_positions(),
                terms.has_offsets(),
                terms.has_payloads()
            ));
            let mut all: Vec<BytesRef> = Vec::new();
            let mut te = terms.iterator().unwrap();
            while let Some(t) = te.next().unwrap() {
                all.push(t.clone());
                out.push(format!(
                    "  t {} df={} ttf={}",
                    hex(t.slice()),
                    te.doc_freq().unwrap(),
                    te.total_term_freq().unwrap()
                ));
                let mut pe = te.postings(None, POSTINGS_ENUM_ALL).unwrap().unwrap();
                loop {
                    let d = pe.next_doc().unwrap();
                    if d == NO_MORE_DOCS {
                        break;
                    }
                    let freq = pe.freq().unwrap();
                    let mut sb = format!("   d={d} f={freq} cost={}", pe.cost());
                    for _ in 0..freq {
                        let pos = pe.next_position().unwrap();
                        sb.push_str(&format!(
                            " {pos}/{}/{}/",
                            pe.start_offset(),
                            pe.end_offset()
                        ));
                        let payload = pe.get_payload().unwrap().map(|p| BytesRef::new(p.to_vec()));
                        sb.push_str(&X(payload).to_string());
                    }
                    out.push(sb);
                }
                out.push(format!("   end docID={}", probe(|| Ok(pe.doc_id()))));
            }
            let mut targets = vec![BytesRef::new(Vec::new())];
            for t in &all {
                targets.push(t.clone());
                let mut past = t.slice().to_vec();
                past.push(0);
                targets.push(BytesRef::new(past));
            }
            targets.push(BytesRef::new(vec![0xff, 0xff]));
            let mut seek = terms.iterator().unwrap();
            let mut sb = "  seek".to_string();
            for target in &targets {
                sb.push_str(&format!(
                    " {}:{}",
                    hex(target.slice()),
                    probe(|| seek.seek_ceil(target).map(status))
                ));
                sb.push_str(&format!(":{}", probe(|| seek.term().map(X))));
                sb.push_str(&format!(":{}", probe(|| seek.doc_freq())));
                sb.push_str(&format!(":{}", probe(|| seek.total_term_freq())));
                sb.push_str(&format!(
                    ":{}",
                    probe(|| seek
                        .postings(None, POSTINGS_ENUM_NONE)?
                        .ok_or_else(rucene::error::LuceneError::null_pointer)?
                        .next_doc())
                ));
                sb.push_str(&format!(":{}", probe(|| seek.next().map(X))));
            }
            out.push(sb);
            let mut exact = terms.iterator().unwrap();
            let mut sb = "  exact".to_string();
            for target in &targets {
                sb.push_str(&format!(
                    " {}:{}",
                    hex(target.slice()),
                    probe(|| exact.seek_exact(target))
                ));
            }
            out.push(sb);
            let mut fresh = terms.iterator().unwrap();
            let term = probe(|| fresh.term().map(X));
            let ord = probe(|| fresh.ord());
            let seek_ord = probe(|| fresh.seek_ord(0).map(|()| "null"));
            out.push(format!("  fresh term={term} ord={ord} seekOrd={seek_ord}"));
        }
    }
    out
}

/// The `RAMDocsEnum@<hash>` identity in a message, which differs per run.
fn mask_identity(message: &str) -> String {
    let mut out = String::new();
    let mut rest = message;
    while let Some(at) = rest.find("RAMDocsEnum@") {
        let end = at + "RAMDocsEnum@".len();
        out.push_str(&rest[..end]);
        out.push_str("HASH");
        rest = rest[end..].trim_start_matches(|c: char| c.is_ascii_hexdigit());
    }
    out.push_str(rest);
    out
}

/// `RAMOnlyPostingsFormat` against `testcodecs_ramonly_*.tsv`, in Lucene's
/// order (the format found by name numbers its slots from 0, so this one test
/// owns it in this process):
///
/// * the `ramonly` index, written and read with the format found by name:
///   every file is Lucene's (the `.id` slot files included), and what Rucene
///   reads (`ramOnlyTrace`, the enum trace) is what Lucene reads in its JVM;
/// * a new instance fails on each corpus where Lucene's does, with Lucene's
///   root cause: offsets refused, a `DOCS` field's `new int[-1]`, the
///   `finishDoc` assertion of a field with freqs and no positions, and the
///   merge of positions stored as `-1`;
/// * `advance` on a fresh enum throws `docID()`'s `NullPointerException`.
#[test]
fn ram_only_matches_lucene() {
    register_codecs();
    let tmp = TempDir::new("ramonly");
    let ram_only = rucene::codecs::postings_for_name("RAMOnly").expect("RAMOnly resolves");
    write_ram_only(&tmp.0, per_field_postings_codec(ram_only), false, false).unwrap();
    let java = java_files("testcodecs_ramonly", "ramonly");
    let compared = assert_same_files("ramonly", &java, &read_dir(&tmp.0), |_| false);
    assert!(compared > 0);
    let java_reads: Vec<String> = rows("testcodecs_ramonly_reads.tsv")
        .into_iter()
        .map(|r| r[1..].join("\t"))
        .collect();
    assert_same_lines("ramonly reads", &ram_only_trace(&tmp.0), &java_reads);
    let java_enum: Vec<String> = rows("testcodecs_ramonly_enum.tsv")
        .into_iter()
        .map(|r| r[1..].join("\t"))
        .collect();
    assert_same_lines("ramonly enum", &enum_trace(&tmp.0), &java_enum);

    let mut corpus = Vec::new();
    for case in ["nocfs", "cfs", "edge", "freqs", "merge"] {
        let dir = TempDir::new(&format!("ramonly-{case}"));
        let codec = per_field_postings_codec(Arc::new(RAMOnlyPostingsFormat::new()));
        let result = {
            let _quiet = QuietPanics::new();
            catch_unwind(AssertUnwindSafe(|| match case {
                "nocfs" => try_write_corpus(&dir.0, codec),
                "cfs" => {
                    // The corpus fails at its first flush: its first batch.
                    let config = new_config(codec);
                    let w = IndexWriter::new(open_dir(&dir.0), config)?;
                    let added =
                        (0..BATCHES[0].1).try_for_each(|i| w.add_document(&doc(i)).map(drop));
                    let committed = added.and_then(|()| w.commit());
                    let closed = w.close();
                    committed.and(closed)
                }
                "edge" => {
                    // As `writeEdge`, which fails at its first flush.
                    let config = new_config(codec);
                    config.set_use_compound_file(false)?;
                    config.set_soft_deletes_field(Some("e_soft".to_string()));
                    config.set_index_sort(Sort::new_fields(vec![SortField::new(
                        Some("e_sortkey".to_string()),
                        SortFieldType::Long,
                    )?])?)?;
                    let w = IndexWriter::new(open_dir(&dir.0), config)?;
                    let added =
                        (0..EDGE_DOCS).try_for_each(|i| w.add_document(&edge_doc(i)).map(drop));
                    let committed = added
                        .and_then(|()| w.delete_documents_by_terms(vec![term("e_id", "e9")]))
                        .and_then(|_| w.commit());
                    let closed = w.close();
                    committed.and(closed)
                }
                _ => write_ram_only(&dir.0, codec, case == "freqs", case == "merge"),
            }))
        };
        let (class, message) = match result {
            Ok(Ok(())) => ("none".to_string(), "-".to_string()),
            Ok(Err(e)) => java_root_cause(&e),
            Err(payload) => ("panic".to_string(), panic_message(&*payload)),
        };
        corpus.push(format!("{case}\t{class}\t{}", mask_identity(&message)));
    }
    let reader = rucene::index::directory_reader::open(open_dir(&tmp.0)).unwrap();
    let (class, message) = outcome(|| {
        let leaves = Arc::clone(&reader).leaves()?;
        let terms = leaves[0].leaf_reader().terms("rp")?.expect("rp has terms");
        let mut te = terms.iterator()?;
        te.next()?;
        te.postings(None, POSTINGS_ENUM_FREQS)?
            .ok_or_else(rucene::error::LuceneError::null_pointer)?
            .advance(1)?;
        Ok(())
    });
    corpus.push(format!("advance\t{class}\t{message}"));
    let java: Vec<String> = rows("testcodecs_ramonly_corpus.tsv")
        .iter()
        .map(|r| mask_identity(&r.join("\t")))
        .collect();
    assert_eq!(corpus, java);
}

/// An instance that did not write the segment holds no slot for its id: its
/// producer is Java's `null` (no terms; `checkIntegrity` and
/// `getMergeInstance` throw `PerFieldPostingsFormat`'s
/// `NullPointerException`s).
#[test]
fn ram_only_other_instance_reads_no_postings() {
    use rucene::codecs::{PostingsFormat, SegmentReadState};
    let writer = Arc::new(RAMOnlyPostingsFormat::new());
    let tmp = TempDir::new("ramonly-other");
    write_ram_only(&tmp.0, per_field_postings_codec(writer), false, false).unwrap();
    let dir = open_dir(&tmp.0);
    let infos = rucene::index::SegmentInfos::read_latest_commit(Arc::clone(&dir)).unwrap();
    let si = infos.info(0).info.clone();
    let field_infos = rucene::index::FieldInfos::default();
    let context = &*rucene::store::DEFAULT_IO_CONTEXT;
    // The per-field suffix of the format's first (only) instance.
    let state =
        SegmentReadState::with_suffix(&*dir, &si, &field_infos, context, "RAMOnly_0".to_string());
    let producer = RAMOnlyPostingsFormat::new()
        .fields_producer(&state)
        .unwrap();
    assert!(producer.terms("rp").unwrap().is_none());
    assert_eq!(
        java_error_of(&producer.check_integrity().unwrap_err()).0,
        "java.lang.NullPointerException"
    );
    assert!(producer.get_merge_instance().is_err());
}
