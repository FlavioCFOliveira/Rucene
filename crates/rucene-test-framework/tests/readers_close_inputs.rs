//! Every reader the port has closes every input it opens (rmp #343).
//!
//! Lucene opens each metadata file inside a try-with-resources (for example
//! `Lucene94FieldInfosFormat.java:134-246`, `SegmentInfos.java:296-304`), so
//! once a `DirectoryReader` is closed no input of the index is left open, and
//! `MockDirectoryWrapper.close()` — which refuses to close over open files
//! (`MockDirectoryWrapper.java:907-920`) — succeeds.
//!
//! For each codec, and each postings and kNN-vectors format reachable through
//! a per-field codec, the corpus of `SimpleTextOracle.writeCorpus` (the shared
//! oracle corpus of `crates/rucene-codecs/tests/oracle/mod.rs`, with and
//! without compound files) is copied into a `MockDirectoryWrapper`; a
//! `DirectoryReader` is opened and closed through it, `CheckIndex` reads the
//! whole index through it at `MIN_LEVEL_FOR_SLOW_CHECKS`, and the wrapper is
//! closed: it must report no open file.
//!
//! `RAMOnlyPostingsFormat` is left out: its postings live in the instance that
//! wrote them, and the instance a reader resolves by name is a new one, so the
//! corpus cannot be read back (as in Lucene, where the format is only used
//! through the codec instance of the writing test).

#[path = "../../rucene-codecs/tests/oracle/mod.rs"]
mod oracle;

use std::sync::Arc;

use oracle::*;
use randomizedtesting::jdk::random::JdkRandom;
use rucene::codecs::lucene104::scalar_quantized_vectors::{
    Lucene104HnswScalarQuantizedVectorsFormat, Lucene104ScalarQuantizedVectorsFormat,
};
use rucene::codecs::lucene104::Lucene104Codec;
use rucene::codecs::Codec;
use rucene::index::directory_reader;
use rucene::index::IndexReader;
use rucene::store::{ByteBuffersDirectory, Directory, DEFAULT_IO_CONTEXT};
use rucene::util::resource_loader::ClassLoader;
use rucene_codecs::bitvectors::HnswBitVectorsFormat;
use rucene_codecs::blocktreeords::BlockTreeOrdsPostingsFormat;
use rucene_codecs::bloom::BloomFilteringPostingsFormat;
use rucene_codecs::memory::{DirectPostingsFormat, FSTPostingsFormat};
use rucene_codecs::simpletext::SimpleTextCodec;
use rucene_codecs::uniformsplit::sharedterms::STUniformSplitPostingsFormat;
use rucene_codecs::uniformsplit::UniformSplitPostingsFormat;
use rucene_test_framework::codecs::asserting::AssertingCodec;
use rucene_test_framework::codecs::blockterms::{
    LuceneFixedGap, LuceneVarGapDocFreqInterval, LuceneVarGapFixedInterval,
};
use rucene_test_framework::codecs::bloom::TestBloomFilteredLucenePostings;
use rucene_test_framework::codecs::cheapbastard::CheapBastardCodec;
use rucene_test_framework::codecs::compressing::dummy::DummyCompressingCodec;
use rucene_test_framework::codecs::compressing::{
    DeflateWithPresetCompressingCodec, FastCompressingCodec, FastDecompressionCompressingCodec,
    HighCompressionCompressingCodec, LZ4WithPresetCompressingCodec,
};
use rucene_test_framework::codecs::mockrandom::MockRandomPostingsFormat;
use rucene_test_framework::codecs::uniformsplit::sharedterms::STUniformSplitRot13PostingsFormat;
use rucene_test_framework::codecs::uniformsplit::UniformSplitRot13PostingsFormat;
use rucene_test_framework::codecs::vector::ConfigurableMCodec;
use rucene_test_framework::store::MockDirectoryWrapper;
use rucene_test_framework::util::TestUtil;

/// The "class path": `lucene-codecs` and the test framework's service files,
/// so that the formats a segment names resolve when it is read back.
fn test_class_loader() -> &'static Arc<ClassLoader> {
    rucene_test_framework::class_loader()
}

fn install_doc_values_formats() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        install_codecs();
        rucene::codecs::doc_values::reload_doc_values_formats(test_class_loader()).unwrap();
    });
}

/// Every codec, by label: the default codec, `SimpleText`, the test codecs,
/// and `Lucene104` over each postings and kNN-vectors format.
fn codecs() -> Vec<(&'static str, Arc<dyn Codec>)> {
    vec![
        ("Lucene104", Arc::new(Lucene104Codec::new())),
        ("SimpleText", Arc::new(SimpleTextCodec::new())),
        ("Asserting", Arc::new(AssertingCodec::new())),
        ("CheapBastard", Arc::new(CheapBastardCodec::new())),
        (
            "FastCompressing",
            Arc::new(FastCompressingCodec::default_instance().unwrap()),
        ),
        (
            "FastDecompressionCompressing",
            Arc::new(FastDecompressionCompressingCodec::default_instance().unwrap()),
        ),
        (
            "HighCompressionCompressing",
            Arc::new(HighCompressionCompressingCodec::default_instance().unwrap()),
        ),
        (
            "DummyCompressing",
            Arc::new(DummyCompressingCodec::default_instance().unwrap()),
        ),
        (
            "DeflateWithPresetCompressing",
            Arc::new(DeflateWithPresetCompressingCodec::default_instance().unwrap()),
        ),
        (
            "LZ4WithPresetCompressing",
            Arc::new(LZ4WithPresetCompressingCodec::default_instance().unwrap()),
        ),
        ("ConfigurableM", Arc::new(ConfigurableMCodec::new())),
        (
            "BlockTreeOrds",
            per_field_postings_codec(Arc::new(BlockTreeOrdsPostingsFormat::new())),
        ),
        (
            "BloomFilter",
            per_field_postings_codec(Arc::new(BloomFilteringPostingsFormat::new(Arc::new(
                rucene::codecs::Lucene104PostingsFormat::new(),
            )))),
        ),
        (
            "Direct",
            per_field_postings_codec(Arc::new(DirectPostingsFormat::new())),
        ),
        (
            "FST",
            per_field_postings_codec(Arc::new(FSTPostingsFormat::new())),
        ),
        (
            "UniformSplit",
            per_field_postings_codec(Arc::new(UniformSplitPostingsFormat::new())),
        ),
        (
            "SharedTermsUniformSplit",
            per_field_postings_codec(Arc::new(STUniformSplitPostingsFormat::new())),
        ),
        (
            "LuceneFixedGap",
            per_field_postings_codec(Arc::new(LuceneFixedGap::new())),
        ),
        (
            "LuceneVarGapFixedInterval",
            per_field_postings_codec(Arc::new(LuceneVarGapFixedInterval::new())),
        ),
        (
            "LuceneVarGapDocFreqInterval",
            per_field_postings_codec(Arc::new(LuceneVarGapDocFreqInterval::new())),
        ),
        (
            "TestBloomFilteredLucenePostings",
            per_field_postings_codec(Arc::new(TestBloomFilteredLucenePostings::new())),
        ),
        (
            "UniformSplitRot13",
            per_field_postings_codec(Arc::new(UniformSplitRot13PostingsFormat::new())),
        ),
        (
            "SharedTermsUniformSplitRot13",
            per_field_postings_codec(Arc::new(STUniformSplitRot13PostingsFormat::new())),
        ),
        (
            "MockRandom",
            per_field_postings_codec(Arc::new(MockRandomPostingsFormat::with_random(
                &JdkRandom::new(1),
            ))),
        ),
        (
            "HnswBitVectorsFormat",
            per_field_knn_codec(Arc::new(HnswBitVectorsFormat::new())),
        ),
        (
            "Lucene104ScalarQuantizedVectorsFormat",
            per_field_knn_codec(Arc::new(Lucene104ScalarQuantizedVectorsFormat::new())),
        ),
        (
            "Lucene104HnswScalarQuantizedVectorsFormat",
            per_field_knn_codec(Arc::new(Lucene104HnswScalarQuantizedVectorsFormat::new())),
        ),
    ]
}

/// Opens and closes a reader over a copy of the corpus in a
/// `MockDirectoryWrapper`, has `CheckIndex` read it through the wrapper, and
/// closes the wrapper; returns the first failure.
fn read_through_wrapper(index: &dyn Directory, seed: i64) -> rucene::error::Result<()> {
    let wrapper =
        MockDirectoryWrapper::new(&JdkRandom::new(seed), Arc::new(ByteBuffersDirectory::new()));
    let names = index.list_all()?;
    for name in &names {
        wrapper.copy_from(index, name, name, &*DEFAULT_IO_CONTEXT)?;
    }
    wrapper.sync(&names)?;

    let reader: Arc<dyn IndexReader> = directory_reader::open(wrapper.as_directory())?;
    assert!(reader.num_docs() > 0);
    reader.close()?;
    drop(reader);

    // `checkIndex(dir, MIN_LEVEL_FOR_SLOW_CHECKS, true, false, null)`: the
    // slow checks read every postings list, term vector and vector.
    TestUtil::check_index_with(wrapper.as_directory(), 3, true, false, None)?;
    // `close()` runs `CheckIndex` again, after refusing to close over an
    // open file.
    let mut wrapper = wrapper;
    wrapper.close()
}

/// The corpus, written to a temporary directory and loaded into memory.
fn corpus(codec: Arc<dyn Codec>, compound: bool) -> ByteBuffersDirectory {
    let tmp = TempDir::new("readers-close-inputs");
    write_corpus(&tmp.0, compound, codec);
    let fs = open_dir(&tmp.0);
    let index = ByteBuffersDirectory::new();
    for name in fs.list_all().unwrap() {
        index
            .copy_from(fs.as_ref(), &name, &name, &*DEFAULT_IO_CONTEXT)
            .unwrap();
    }
    index
}

#[test]
fn every_reader_closes_every_input_it_opens() {
    install_doc_values_formats();
    let mut failures = Vec::new();
    for (label, codec) in codecs() {
        for compound in [false, true] {
            let index = corpus(Arc::clone(&codec), compound);
            if let Err(e) = read_through_wrapper(&index, 7) {
                failures.push(format!("{label} compound={compound}: {e}"));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
