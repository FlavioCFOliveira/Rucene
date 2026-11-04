//! Oracle parity of `org.apache.lucene.codecs.memory` (rmp #334:
//! `FSTPostingsFormat`) against Apache Lucene 10.5.0.
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
use rucene_codecs::memory::FSTPostingsFormat;

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
