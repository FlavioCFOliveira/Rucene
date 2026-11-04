//! Oracle parity of the test codecs of `org.apache.lucene.tests.codecs`
//! (rmp #340: `asserting`, `cheapbastard`, `compressing`, `vector`; rmp #341:
//! `cranky`) against Apache Lucene 10.5.0.
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

#[path = "../../rucene-codecs/tests/oracle/mod.rs"]
mod oracle;

use std::collections::BTreeMap;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;
use std::sync::{Arc, LazyLock, Mutex, Once};

use oracle::*;
use randomizedtesting::jdk::exception::{panic_message, QuietPanics};
use randomizedtesting::jdk::random::{JdkRandom, Random, RandomFields};
use rucene::codecs::lucene104::Lucene104Codec;
use rucene::codecs::{Codec, DocValuesFormat, KnnVectorsFormat, PostingsFormat};
use rucene::document::IntPoint;
use rucene::error::{LuceneError, Result};
use rucene::index::index_writer::IndexWriter;
use rucene::index::merge_policy::{MergePolicy, TieredMergePolicy};
use rucene::index::{
    LeafReader, SegmentInfo, POSTINGS_ENUM_ALL, POSTINGS_ENUM_FREQS, POSTINGS_ENUM_NONE,
    POSTINGS_ENUM_OFFSETS, POSTINGS_ENUM_POSITIONS,
};
use rucene::search::NO_MORE_DOCS;
use rucene::store::java_nio_file::{java_class_of, java_message_of};
use rucene::store::{Directory, IOContext};
use rucene::util::resource_loader::{ClassLoader, ServiceProvider};
use rucene::util::spi::NamedSPIProvider;
use rucene_test_framework::codecs::asserting::{
    AssertingCodec, AssertingDocValuesFormat, AssertingKnnVectorsFormat, AssertingPostingsFormat,
};
use rucene_test_framework::codecs::cheapbastard::CheapBastardCodec;
use rucene_test_framework::codecs::compressing::dummy::DummyCompressingCodec;
use rucene_test_framework::codecs::compressing::{
    DeflateWithPresetCompressingCodec, FastCompressingCodec, FastDecompressionCompressingCodec,
    HighCompressionCompressingCodec, LZ4WithPresetCompressingCodec,
};
use rucene_test_framework::codecs::cranky::CrankyCodec;
use rucene_test_framework::codecs::vector::ConfigurableMCodec;

/// The "class path" of the tests: `lucene-codecs` and the asserting
/// providers Lucene's test-framework jar lists in its service files (the
/// crate's own service files are task #341's), so that the indexes written
/// with `AssertingCodec` can be read back by name.
fn test_class_loader() -> &'static Arc<ClassLoader> {
    static LOADER: LazyLock<Arc<ClassLoader>> = LazyLock::new(|| {
        ClassLoader::with_services(
            "rucene-test-framework-tests",
            Some(Arc::clone(rucene_codecs::class_loader())),
            vec![
                ServiceProvider::new(
                    "org.apache.lucene.codecs.Codec",
                    "org.apache.lucene.tests.codecs.asserting.AssertingCodec",
                    NamedSPIProvider::new(|| Arc::new(AssertingCodec::new()) as Arc<dyn Codec>),
                ),
                ServiceProvider::new(
                    "org.apache.lucene.codecs.Codec",
                    "org.apache.lucene.tests.codecs.cheapbastard.CheapBastardCodec",
                    NamedSPIProvider::new(|| codec("CheapBastard")),
                ),
                ServiceProvider::new(
                    "org.apache.lucene.codecs.Codec",
                    "org.apache.lucene.tests.codecs.compressing.DeflateWithPresetCompressingCodec",
                    NamedSPIProvider::new(|| codec("DeflateWithPresetCompressing")),
                ),
                ServiceProvider::new(
                    "org.apache.lucene.codecs.Codec",
                    "org.apache.lucene.tests.codecs.compressing.FastCompressingCodec",
                    NamedSPIProvider::new(|| codec("FastCompressing")),
                ),
                ServiceProvider::new(
                    "org.apache.lucene.codecs.Codec",
                    "org.apache.lucene.tests.codecs.compressing.FastDecompressionCompressingCodec",
                    NamedSPIProvider::new(|| codec("FastDecompressionCompressing")),
                ),
                ServiceProvider::new(
                    "org.apache.lucene.codecs.Codec",
                    "org.apache.lucene.tests.codecs.compressing.HighCompressionCompressingCodec",
                    NamedSPIProvider::new(|| codec("HighCompressionCompressing")),
                ),
                ServiceProvider::new(
                    "org.apache.lucene.codecs.Codec",
                    "org.apache.lucene.tests.codecs.compressing.LZ4WithPresetCompressingCodec",
                    NamedSPIProvider::new(|| codec("LZ4WithPresetCompressing")),
                ),
                ServiceProvider::new(
                    "org.apache.lucene.codecs.Codec",
                    "org.apache.lucene.tests.codecs.compressing.dummy.DummyCompressingCodec",
                    NamedSPIProvider::new(|| codec("DummyCompressing")),
                ),
                ServiceProvider::new(
                    "org.apache.lucene.codecs.Codec",
                    "org.apache.lucene.tests.codecs.vector.ConfigurableMCodec",
                    NamedSPIProvider::new(|| codec("ConfigurableM")),
                ),
                ServiceProvider::new(
                    "org.apache.lucene.codecs.DocValuesFormat",
                    "org.apache.lucene.tests.codecs.asserting.AssertingDocValuesFormat",
                    NamedSPIProvider::new(|| {
                        Arc::new(AssertingDocValuesFormat::new()) as Arc<dyn DocValuesFormat>
                    }),
                ),
                ServiceProvider::new(
                    "org.apache.lucene.codecs.KnnVectorsFormat",
                    "org.apache.lucene.tests.codecs.asserting.AssertingKnnVectorsFormat",
                    NamedSPIProvider::new(|| {
                        Arc::new(AssertingKnnVectorsFormat::new()) as Arc<dyn KnnVectorsFormat>
                    }),
                ),
                ServiceProvider::new(
                    "org.apache.lucene.codecs.PostingsFormat",
                    "org.apache.lucene.tests.codecs.asserting.AssertingPostingsFormat",
                    NamedSPIProvider::new(|| {
                        Arc::new(AssertingPostingsFormat::new()) as Arc<dyn PostingsFormat>
                    }),
                ),
            ],
        )
    });
    &LOADER
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
        Ok(Err(LuceneError::AssertionError(m))) => (
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
            .freq()
            .map(drop)
    });
    misuse("pe.advance backwards", &|| {
        let mut pe = positioned(leaf, "body")?.postings(None, POSTINGS_ENUM_FREQS)?;
        pe.next_doc()?;
        let doc = pe.doc_id();
        pe.advance(doc).map(drop)
    });
    misuse("pe.nextDoc after end", &|| {
        let mut pe = positioned(leaf, "body")?.postings(None, POSTINGS_ENUM_FREQS)?;
        while pe.next_doc()? != NO_MORE_DOCS {}
        pe.next_doc().map(drop)
    });
    misuse("pe.advance after end", &|| {
        let mut pe = positioned(leaf, "body")?.postings(None, POSTINGS_ENUM_FREQS)?;
        while pe.next_doc()? != NO_MORE_DOCS {}
        pe.advance(1).map(drop)
    });
    misuse("pe.nextPosition beyond freq", &|| {
        let mut pe = positioned(leaf, "body")?.postings(None, POSTINGS_ENUM_POSITIONS)?;
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
            .next_position()
            .map(drop)
    });
    misuse("pe.getPayload before nextPosition", &|| {
        let mut pe = positioned(leaf, "body")?.postings(None, POSTINGS_ENUM_ALL)?;
        pe.next_doc()?;
        pe.get_payload().map(drop)
    });
    misuse("pe.startOffset before nextPosition", &|| {
        let mut pe = positioned(leaf, "body")?.postings(None, POSTINGS_ENUM_OFFSETS)?;
        pe.next_doc()?;
        pe.start_offset();
        Ok(())
    });
    misuse("pe.freq after end", &|| {
        let mut pe = positioned(leaf, "body")?.postings(None, POSTINGS_ENUM_FREQS)?;
        while pe.next_doc()? != NO_MORE_DOCS {}
        pe.freq().map(drop)
    });
    misuse("pe.freq after reuse", &|| {
        let mut te = positioned(leaf, "body")?;
        let mut pe = te.postings(None, POSTINGS_ENUM_FREQS)?;
        while pe.next_doc()? != NO_MORE_DOCS {}
        let before = &*pe as *const dyn rucene::index::PostingsEnum as *const ();
        te.next()?;
        let reused = te.postings(Some(pe), POSTINGS_ENUM_FREQS)?;
        if &*reused as *const dyn rucene::index::PostingsEnum as *const () != before {
            return Err(LuceneError::illegal_state("not reused".to_string()));
        }
        reused.freq().map(drop)
    });
    misuse("terms consumed in another thread", &|| {
        let terms = leaf.terms("body")?.unwrap();
        let thrown = std::thread::scope(|s| {
            s.spawn(|| match terms.iterator() {
                Err(LuceneError::AssertionError(m)) => Some(m),
                Err(e) => Some(e.to_string()),
                Ok(_) => None,
            })
            .join()
            .unwrap()
        });
        match thrown {
            Some(m) => {
                let cut = m.find(" But was acquired in").unwrap_or(m.len());
                Err(LuceneError::AssertionError(m[..cut].to_string()))
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
    let mut config = new_config(codec);
    let policy = TieredMergePolicy::new();
    policy.set_no_cfs_ratio(0.0)?;
    config.set_merge_policy(Arc::new(policy))?;
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
        LuceneError::Io(io) => (
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
