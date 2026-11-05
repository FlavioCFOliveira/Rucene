//! Oracle parity of `org.apache.lucene.codecs.memory` (rmp #334:
//! `FSTPostingsFormat`; rmp #337: `DirectPostingsFormat`) against Apache
//! Lucene 10.5.0.
//!
//! The Java oracle (`PostingsFormatOracle`, in the workspace harness
//! `crates/randomizedtesting/tests/fixtures/oracle/`) writes the corpus of
//! `SimpleTextOracle` with `Lucene104Codec` routing every field but `e_id` to
//! the format. The tests write the same corpus with Rucene and compare every
//! file of the format byte for byte (Lucene's random segment id replaced by
//! Rucene's), read both sides' files with Rucene, run Lucene's reader on
//! Rucene's files, compare the errors of corrupted copies and resolve the
//! format's service name.

mod oracle;

use std::sync::Arc;

use oracle::*;
use rucene::codecs::Codec;
use rucene::util::resource_loader::ClassLoader;
use rucene_codecs::memory::{DirectPostingsFormat, FSTPostingsFormat};

fn test_class_loader() -> &'static Arc<ClassLoader> {
    rucene_codecs::class_loader()
}

fn fst50_codec() -> Arc<dyn Codec> {
    per_field_postings_codec(Arc::new(FSTPostingsFormat::new()))
}

const FST50: FormatOracle = FormatOracle {
    prefix: "fst50",
    format_name: "FST50",
    extensions: &["tfp"],
    codec: fst50_codec,
    extra_jars: &[],
    live_impacts_differ: true,
};

#[test]
fn fst50_writes_lucenes_bytes() {
    FST50.assert_writes_lucenes_bytes();
}

#[test]
fn fst50_reads_what_lucene_reads() {
    FST50.assert_reads_what_lucene_reads();
}

#[test]
fn fst50_enum_trace_matches_lucenes() {
    FST50.assert_enum_trace_matches();
}

#[test]
fn lucene_reads_what_fst50_writes() {
    FST50.assert_lucene_reads_what_rucene_writes();
}

#[test]
fn fst50_corrupted_files_raise_lucenes_errors() {
    assert!(FST50
        .assert_corrupted_files_raise_lucenes_errors()
        .is_empty());
}

/// `PostingsFormat.forName("FST50")` resolves once the crate's service
/// files are loaded; the format's own `toString()` is its bare name.
#[test]
fn fst50_resolves_by_name() {
    install_codecs();
    let spi = FST50.spi();
    let format = rucene::codecs::postings_for_name("FST50").expect("FST50 resolves");
    assert_eq!(format.name(), spi["getName"]);
    assert_eq!(FSTPostingsFormat::new().to_string(), spi["toString"]);
    assert_eq!(
        spi["forName.getClass"],
        "org.apache.lucene.codecs.memory.FSTPostingsFormat"
    );
}

fn direct_codec() -> Arc<dyn Codec> {
    per_field_postings_codec(Arc::new(DirectPostingsFormat::new()))
}

/// `DirectPostingsFormat` writes `Lucene104PostingsFormat`'s files under its
/// own name and reads them into RAM; its enums are its own, so the dump's
/// `live` lines (`SlowImpactsEnum`) are not core defect 1's.
const DIRECT: FormatOracle = FormatOracle {
    prefix: "direct",
    format_name: "Direct",
    extensions: &["tmd", "tim"],
    codec: direct_codec,
    extra_jars: &[],
    live_impacts_differ: false,
};

#[test]
fn direct_writes_lucenes_bytes() {
    DIRECT.assert_writes_lucenes_bytes();
}

#[test]
fn direct_reads_what_lucene_reads() {
    DIRECT.assert_reads_what_lucene_reads();
}

#[test]
fn direct_enum_trace_matches_lucenes() {
    DIRECT.assert_enum_trace_matches();
}

#[test]
fn lucene_reads_what_direct_writes() {
    DIRECT.assert_lucene_reads_what_rucene_writes();
}

/// Core defect 3, outside this crate: `Lucene103BlockTreeTermsReader`
/// (`src/codecs/lucene103/blocktree.rs`, the `.tmd` header check) returns the
/// header's error at once, where Java catches it and runs
/// `CodecUtil.checkFooter(metaIn, priorE)`, which reports the checksum
/// failure of the corrupted file (`Lucene103BlockTreeTermsReader.java:144-229`).
/// A flipped id in `.tmd` therefore raises `file mismatch` in Rucene and
/// `checksum failed` in Lucene.
const TMD_ID_CORE_DEFECT: (&str, &str, &str) = (
    "tmd-id",
    "org.apache.lucene.index.CorruptIndexException",
    "file mismatch, expected id=$ID, got=$BADID \
     (resource=BufferedChecksumIndexInput(NIOFSIndexInput(path=\"$D/_5_Direct_0.tmd\")))",
);

#[test]
fn direct_corrupted_files_raise_lucenes_errors() {
    assert!(DIRECT
        .assert_corrupted_files_raise_lucenes_errors_but(&[TMD_ID_CORE_DEFECT])
        .is_empty());
}

/// `PostingsFormat.forName("Direct")` resolves once the crate's service
/// files are loaded; Java does not override `toString()`.
#[test]
fn direct_resolves_by_name() {
    install_codecs();
    let spi = DIRECT.spi();
    let format = rucene::codecs::postings_for_name("Direct").expect("Direct resolves");
    assert_eq!(format.name(), spi["getName"]);
    assert_eq!(format.to_string(), spi["toString"]);
    assert_eq!(format.to_string(), spi["forName.toString"]);
    assert_eq!(
        spi["forName.getClass"],
        "org.apache.lucene.codecs.memory.DirectPostingsFormat"
    );
}
