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
    FST50.assert_corrupted_files_raise_lucenes_errors();
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

#[test]
fn direct_corrupted_files_raise_lucenes_errors() {
    DIRECT.assert_corrupted_files_raise_lucenes_errors();
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

/// Regression (rmp #528): `FSTTermsReader`'s `seekExact(BytesRef,
/// TermState)` copies the state with `BlockTermState.copyFrom`, whose cast
/// rejects another kind of `TermState`: an `AssertionError` under `-ea`, a
/// `ClassCastException` without it — never a silent no-op.
#[test]
fn fst50_seek_exact_with_a_foreign_term_state_is_rejected() {
    use rucene::document::{Document, Store, StringField};
    use rucene::index::index_writer::IndexWriter;
    use rucene::index::{directory_reader, OrdTermState};
    use rucene::util::BytesRef;

    let dir = TempDir::new("fst50-foreign-state");
    let w = IndexWriter::new(open_dir(&dir.0), new_config(fst50_codec())).unwrap();
    let mut d = Document::new();
    d.add(StringField::new("f", "a".to_string(), Store::NO).unwrap());
    w.add_document(&d).unwrap();
    w.commit().unwrap();
    w.close().unwrap();
    let reader = directory_reader::open(open_dir(&dir.0)).unwrap();
    let leaves = Arc::clone(&reader).leaves().unwrap();
    let terms = leaves[0].leaf_reader().terms("f").unwrap().unwrap();
    let mut te = terms.iterator().unwrap();
    let target = BytesRef::new(b"b".to_vec());
    let foreign = OrdTermState::new();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        te.seek_term_state(&target, &foreign)
    }));
    if cfg!(debug_assertions) {
        assert!(result.is_err(), "the assertion fails under -ea");
    } else {
        let err = result.expect("no assertion without -ea").unwrap_err();
        assert!(
            err.to_string().contains("java.lang.ClassCastException"),
            "{err}"
        );
    }
}
