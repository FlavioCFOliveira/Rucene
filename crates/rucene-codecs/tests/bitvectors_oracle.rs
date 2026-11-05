//! Oracle parity of `org.apache.lucene.codecs.bitvectors` (rmp #337:
//! `HnswBitVectorsFormat`, `FlatBitVectorsScorer`) against Apache Lucene
//! 10.5.0.
//!
//! The Java oracle (`PostingsFormatOracle bitvectors knn
//! HnswBitVectorsFormat vem vex vemf vec`, in the workspace harness
//! `crates/randomizedtesting/tests/fixtures/oracle/`) writes the corpus of
//! `SimpleTextOracle` with `Lucene104Codec` routing the byte-vector fields
//! (`vb`, `e_vb_*`) to the format. The tests write the same corpus with
//! Rucene and compare every file of the format byte for byte (the HNSW
//! graph, built with the format's hamming scorer, included), read both
//! sides' files with Rucene (the dump's kNN searches score with the same
//! scorer), run Lucene's reader on Rucene's files, compare the errors of
//! corrupted copies and resolve the format's service name.

mod oracle;

use std::sync::Arc;

use oracle::*;
use rucene::codecs::hnsw::flat_vectors::FlatVectorsScorer;
use rucene::codecs::Codec;
use rucene::index::VectorSimilarityFunction;
use rucene::util::resource_loader::ClassLoader;
use rucene_codecs::bitvectors::{FlatBitVectorsScorer, HnswBitVectorsFormat};

fn test_class_loader() -> &'static Arc<ClassLoader> {
    rucene_codecs::class_loader()
}

fn bitvectors_codec() -> Arc<dyn Codec> {
    per_field_knn_codec(Arc::new(HnswBitVectorsFormat::new()))
}

/// Every postings format of the corpus is `Lucene104`, so the dump's `live`
/// lines meet core defect 1.
const BITVECTORS: FormatOracle = FormatOracle {
    prefix: "bitvectors",
    format_name: "HnswBitVectorsFormat",
    extensions: &["vem", "vex", "vemf", "vec"],
    codec: bitvectors_codec,
    extra_jars: &[],
    live_impacts_differ: true,
};

#[test]
fn bitvectors_writes_lucenes_bytes() {
    BITVECTORS.assert_writes_lucenes_bytes();
}

#[test]
fn bitvectors_reads_what_lucene_reads() {
    BITVECTORS.assert_reads_what_lucene_reads();
}

#[test]
fn lucene_reads_what_bitvectors_writes() {
    BITVECTORS.assert_lucene_reads_what_rucene_writes();
}

#[test]
fn bitvectors_corrupted_files_raise_lucenes_errors() {
    BITVECTORS.assert_corrupted_files_raise_lucenes_errors();
}

/// `KnnVectorsFormat.forName("HnswBitVectorsFormat")` resolves once the
/// crate's service files are loaded; the format's `toString()` names the
/// flat format and its scorer.
#[test]
fn bitvectors_resolves_by_name() {
    install_codecs();
    let spi = BITVECTORS.spi();
    let format = rucene::codecs::knn_vectors_for_name("HnswBitVectorsFormat")
        .expect("HnswBitVectorsFormat resolves");
    assert_eq!(format.name(), spi["getName"]);
    assert_eq!(HnswBitVectorsFormat::new().to_string(), spi["toString"]);
    assert_eq!(
        spi["forName.getClass"],
        "org.apache.lucene.codecs.bitvectors.HnswBitVectorsFormat"
    );
}

/// `HnswBitVectorsFormat`'s constructor rejects what Java's does, with
/// Java's messages.
#[test]
fn bitvectors_constructor_checks_its_arguments() {
    let message = |r: rucene::error::Result<HnswBitVectorsFormat>| match r {
        Err(rucene::error::LuceneError::IllegalArgument(m)) => m,
        other => panic!("expected IllegalArgumentException, got {other:?}"),
    };
    assert_eq!(
        message(HnswBitVectorsFormat::with_params(0, 100)),
        "maxConn must be positive and less than or equal to 512; maxConn=0"
    );
    assert_eq!(
        message(HnswBitVectorsFormat::with_params(513, 100)),
        "maxConn must be positive and less than or equal to 512; maxConn=513"
    );
    assert_eq!(
        message(HnswBitVectorsFormat::with_params(16, 3201)),
        "beamWidth must be positive and less than or equal to 3200; beamWidth=3201"
    );
    assert_eq!(
        HnswBitVectorsFormat::with_params(512, 3200)
            .unwrap()
            .to_string(),
        "HnswBitVectorsFormat(name=HnswBitVectorsFormat, maxConn=512, beamWidth=3200, \
         flatVectorFormat=Lucene99FlatVectorsFormat(vectorsScorer=FlatBitVectorsScorer()))"
    );
}

/// The scorer refuses float vectors and float targets, as Java's
/// `instanceof ByteVectorValues` checks do.
#[test]
fn bit_scorer_refuses_float_vectors() {
    let values = rucene::index::vector_values::from_floats(vec![vec![1.0f32, 2.0]], 2);
    let err = FlatBitVectorsScorer::new()
        .get_random_vector_scorer_float(
            VectorSimilarityFunction::EUCLIDEAN,
            Box::new(values),
            &[1.0, 2.0],
        )
        .err()
        .expect("float targets are refused");
    assert_eq!(
        java_error_of(&err),
        (
            "java.lang.IllegalArgumentException".to_string(),
            "bit vectors do not support float[] targets".to_string()
        )
    );
}

/// The format refuses a float-vector field: Java's `FlatBitVectorsWriter`
/// throws `IllegalArgumentException("HnswBitVectorsFormat only supports BYTE
/// encoding")` from `addField`, which the first document carrying the field
/// reaches.
#[test]
fn bitvectors_refuses_float_fields() {
    use rucene::document::{Document, KnnFloatVectorField};
    let tmp = TempDir::new("bitvectors-float");
    let w = rucene::index::index_writer::IndexWriter::new(
        open_dir(&tmp.0),
        new_config(bitvectors_codec()),
    )
    .unwrap();
    let mut doc = Document::new();
    doc.add(Box::new(
        KnnFloatVectorField::new("vb", &[1.0, 2.0], VectorSimilarityFunction::EUCLIDEAN).unwrap(),
    ));
    let err = w.add_document(&doc).expect_err("a float field is refused");
    assert_eq!(
        java_error_of(&err),
        (
            "java.lang.IllegalArgumentException".to_string(),
            "HnswBitVectorsFormat only supports BYTE encoding".to_string()
        )
    );
}

/// The oracle's corpus is too small for a graph (`HNSW_GRAPH_THRESHOLD`), so
/// this test writes one segment of 1000 byte vectors, `v[j] = (31i + 17j +
/// (ij mod 13)) & 0xff` over 8 bytes, whose graph Lucene builds with the
/// format's hamming scorer. The four files' bodies (between the index header
/// and the footer, which hold the random segment id) must hash to what
/// Lucene 10.5.0 wrote for the same documents (`HnswBitVectorsFormat` for
/// every field, no compound file, `NoMergePolicy`, one commit; FNV-1a 64 of
/// Lucene's files, measured 2026-11-04). A graph built with
/// `Lucene99HnswVectorsFormat`'s scorer instead differs: node 0 has 14
/// neighbours on level 0 there and 32 here.
#[test]
fn bitvectors_graph_of_a_large_segment_matches_lucenes() {
    use rucene::document::{Document, KnnByteVectorField};
    let tmp = TempDir::new("bitvectors-graph");
    let mut config = new_config(per_field_knn_codec(Arc::new(HnswBitVectorsFormat::new())));
    config.set_use_compound_file(false).unwrap();
    config
        .set_merge_policy(Arc::new(rucene::index::merge_policy::NoMergePolicy::new()))
        .unwrap();
    config.set_max_buffered_docs(100_000).unwrap();
    config.set_ram_buffer_size_mb(256.0).unwrap();
    let w = rucene::index::index_writer::IndexWriter::new(open_dir(&tmp.0), config).unwrap();
    for i in 0..1000i32 {
        let v: Vec<u8> = (0..8)
            .map(|j| ((i * 31 + j * 17 + (i * j) % 13) & 0xff) as u8)
            .collect();
        let mut doc = Document::new();
        doc.add(Box::new(
            KnnByteVectorField::new("vb", &v, VectorSimilarityFunction::EUCLIDEAN).unwrap(),
        ));
        w.add_document(&doc).unwrap();
    }
    w.commit().unwrap();
    w.close().unwrap();
    let files = read_dir(&tmp.0);
    let body_hash = |ext: &str| {
        let bytes = &files[&format!("_0_HnswBitVectorsFormat_0.{ext}")];
        let header = id_offset(bytes) + 16;
        let header = header + 1 + bytes[header] as usize;
        (bytes.len(), fnv(&bytes[header..bytes.len() - 16]))
    };
    assert_eq!(body_hash("vec"), (8096, 0xa0aa_0557_30d2_cbce));
    assert_eq!(body_hash("vem"), (236, 0xdf58_c2a6_78bc_6e05));
    assert_eq!(body_hash("vemf"), (136, 0x30df_d7b7_4934_3ddd));
    assert_eq!(body_hash("vex"), (26112, 0x9fec_fbbc_fe2a_53fe));
}
