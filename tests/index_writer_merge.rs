//! Segment merges driven by `IndexWriter`, end to end.
//!
//! Three merge-path defects kept every merge from producing a correct segment,
//! and these tests pin the repaired path against Apache Lucene Core 10.5.0:
//!
//! * **rmp #213.** `SegmentMerger` never recorded the merged segment's
//!   `minVersion` (`SegmentMerger.java:76-89`), so committing *any* merge
//!   failed in `SegmentInfos::apply_merge_changes` with "All segments must
//!   record the minVersion for indices created on or after Lucene 7".
//! * **rmp #215.** `MergeState` held the readers' search instances instead of
//!   their merge instances (`MergeState.java:134-168`). The stored-fields bulk
//!   copy only works on a merge instance — `isLoaded` refuses anything else —
//!   so the first test below also covers it.
//! * **rmp #200.** The vector writers walked the segments themselves instead of
//!   reading `KnnVectorsWriter.MergedVectorValues`, and `KnnVectorsWriter.merge`
//!   was a no-op. The merged vector files — the HNSW `.vem` and `.vex`
//!   included, with or without a graph — are compared byte for byte with the
//!   ones Lucene writes for the same documents, and segments that do not share
//!   their vector fields merge as Lucene's do.
//!
//! # Where the expected bytes come from
//!
//! Each `LUCENE_*` table holds the vector files of the segment a real Lucene
//! 10.5.0 `IndexWriter.forceMerge(1)` wrote for the documents of
//! [`vec_doc`]: the output of `MergedVectorsFixture` in
//! `tests/fixtures/java-codec-harness`
//! (`java -cp lucene-core-10.5.0.jar MergedVectorsFixture.java <dir>
//! <default|sq|hnswsq|sq-then-hnswsq> <true|false|graph>`).
//! A segment id is random, so a forced merge here cannot reproduce Lucene's;
//! those comparisons skip the sixteen id bytes of the header and the CRC of
//! the footer, and check the CRC on their own. The merges with deletions take
//! the reference segment's id and are compared byte for byte, footer included.
//!
//! # What these tests deliberately do not exercise
//!
//! Three defects outside the tasks above stand in the way of a fuller test,
//! and each is avoided rather than worked around:
//!
//! * The documents index no term. The block-tree terms writer's `merge` is
//!   still a no-op, and its `close` then writes the postings header of
//!   `.tmd` with an empty segment suffix, so a merged segment with an indexed
//!   field cannot be reopened.
//! * The merges with deletions run `SegmentMerger` over the committed segments
//!   rather than `IndexWriter::force_merge`: `PendingDeletes::get_live_docs`
//!   moves the live docs out instead of sharing them, so `IndexWriter`'s merge
//!   of a segment with committed deletions trips `verify_doc_counts`. The
//!   segment merger and every codec underneath are the ones `force_merge`
//!   runs.
//! * The merge into the HNSW scalar-quantized format starts from segments of
//!   the flat scalar-quantized format (`SwitchingCodec`), as the Java fixture
//!   `sq-then-hnswsq` does: a segment that format flushes cannot read its
//!   float vectors back ("has no vector values"), so it cannot be a merge
//!   source.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use rucene::codecs::per_field::PerFieldKnnVectorsFormat;
use rucene::codecs::{
    Codec, CompoundFormat, DocValuesFormat, FieldInfosFormat, KnnVectorsFormat, LiveDocsFormat,
    Lucene104Codec, NormsFormat, PointsFormat, PostingsFormat, SegmentInfoFormat,
    StoredFieldsFormat, TermVectorsFormat,
};
use rucene::document::{Document, KnnByteVectorField, KnnFloatVectorField, StoredField};
use rucene::index::codec_reader::CodecReader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::merge::MergeState;
use rucene::index::merge_policy::NoMergePolicy;
use rucene::index::segment_merger::SegmentMerger;
use rucene::index::segment_reader::SegmentReader;
use rucene::index::{
    directory_reader, DirectoryReader, FieldInfosBuilder, FieldNumbers, SegmentInfo, SegmentInfos,
};
use rucene::search::{
    FieldExistsQuery, IndexSearcher, KnnByteVectorQuery, KnnFloatVectorQuery, Query, NO_MORE_DOCS,
};
use rucene::store::{Directory, RamDirectory, DEFAULT_IO_CONTEXT};
use rucene::util::Version;

fn ensure_codec() {
    let _ = rucene::codecs::register_codec("Lucene104", Lucene104Codec::new());
}

/// `Lucene104Codec` with every vector field on one shared
/// `Lucene104ScalarQuantizedVectorsFormat`.
///
/// This is how Lucene selects a vectors format: a subclass overriding
/// `getKnnVectorsFormatForField`, keeping the codec's name, so a reader opens
/// the segment with the stock codec and finds the format through the per-field
/// attributes.
#[derive(Debug)]
struct QuantizedCodec {
    inner: Lucene104Codec,
    knn: PerFieldKnnVectorsFormat,
}

impl QuantizedCodec {
    fn new() -> Self {
        let format = rucene::codecs::knn_vectors_for_name("Lucene104ScalarQuantizedVectorsFormat")
            .expect("the format ships with the crate");
        Self {
            inner: Lucene104Codec::new(),
            knn: PerFieldKnnVectorsFormat::from_map(Default::default(), format),
        }
    }
}

impl Codec for QuantizedCodec {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn postings_format(&self) -> &dyn PostingsFormat {
        self.inner.postings_format()
    }

    fn doc_values_format(&self) -> &dyn DocValuesFormat {
        self.inner.doc_values_format()
    }

    fn stored_fields_format(&self) -> &dyn StoredFieldsFormat {
        self.inner.stored_fields_format()
    }

    fn term_vectors_format(&self) -> &dyn TermVectorsFormat {
        self.inner.term_vectors_format()
    }

    fn field_infos_format(&self) -> &dyn FieldInfosFormat {
        self.inner.field_infos_format()
    }

    fn segment_info_format(&self) -> &dyn SegmentInfoFormat {
        self.inner.segment_info_format()
    }

    fn norms_format(&self) -> &dyn NormsFormat {
        self.inner.norms_format()
    }

    fn live_docs_format(&self) -> &dyn LiveDocsFormat {
        self.inner.live_docs_format()
    }

    fn compound_format(&self) -> &dyn CompoundFormat {
        self.inner.compound_format()
    }

    fn points_format(&self) -> &dyn PointsFormat {
        self.inner.points_format()
    }

    fn knn_vectors_format(&self) -> &dyn KnnVectorsFormat {
        &self.knn
    }
}

/// `Lucene104Codec` whose vector fields use format `before` until `merging` is
/// raised, and format `after` from then on.
///
/// Lucene's `getKnnVectorsFormatForField` is consulted each time a segment is
/// written, so a subclass reading a flag switches formats between the flushes
/// and the merge; the fixture's `sq-then-hnswsq` case does exactly that.
#[derive(Debug)]
struct SwitchingCodec {
    inner: QuantizedCodec,
}

impl SwitchingCodec {
    fn new(before: &str, after: &str, merging: Arc<AtomicBool>) -> Self {
        let before = rucene::codecs::knn_vectors_for_name(before).unwrap();
        let after = rucene::codecs::knn_vectors_for_name(after).unwrap();
        Self {
            inner: QuantizedCodec {
                inner: Lucene104Codec::new(),
                knn: PerFieldKnnVectorsFormat::new(move |_field| {
                    if merging.load(Ordering::SeqCst) {
                        Arc::clone(&after)
                    } else {
                        Arc::clone(&before)
                    }
                }),
            },
        }
    }
}

impl Codec for SwitchingCodec {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn postings_format(&self) -> &dyn PostingsFormat {
        self.inner.postings_format()
    }

    fn doc_values_format(&self) -> &dyn DocValuesFormat {
        self.inner.doc_values_format()
    }

    fn stored_fields_format(&self) -> &dyn StoredFieldsFormat {
        self.inner.stored_fields_format()
    }

    fn term_vectors_format(&self) -> &dyn TermVectorsFormat {
        self.inner.term_vectors_format()
    }

    fn field_infos_format(&self) -> &dyn FieldInfosFormat {
        self.inner.field_infos_format()
    }

    fn segment_info_format(&self) -> &dyn SegmentInfoFormat {
        self.inner.segment_info_format()
    }

    fn norms_format(&self) -> &dyn NormsFormat {
        self.inner.norms_format()
    }

    fn live_docs_format(&self) -> &dyn LiveDocsFormat {
        self.inner.live_docs_format()
    }

    fn compound_format(&self) -> &dyn CompoundFormat {
        self.inner.compound_format()
    }

    fn points_format(&self) -> &dyn PointsFormat {
        self.inner.points_format()
    }

    fn knn_vectors_format(&self) -> &dyn KnnVectorsFormat {
        self.inner.knn_vectors_format()
    }
}

// ---------------------------------------------------------------------------
// Lucene 10.5.0 reference files (MergedVectorsFixture)
// ---------------------------------------------------------------------------

/// Vector files of the merged segment, case `default false`.
const LUCENE_DEFAULT_NO_DELETIONS: &[(&str, &str)] = &[
    (
        "_2_Lucene99HnswVectorsFormat_0.vec",
        "3fd76c171d4c7563656e653939466c6174566563746f7273466f726d61744461746100000000f0f3\
         33fce4791724f4a7967b3dfae3a21b4c7563656e653939486e7377566563746f7273466f726d6174\
         5f300000000000000000000000000000000000000000000000000000000000000000000000000000\
         0000000000000000000000000000803f0000803f0000c03f00000040000000400000404000002040\
         00008040000040400000a0400000604000c801c702c603c504c405c3000000000000000000000000\
         00008040000000c000000041000080c000000200000002000400ff7f0000ffffc02893e800000000\
         00000000356399ca",
    ),
    (
        "_2_Lucene99HnswVectorsFormat_0.vem",
        "3fd76c171d4c7563656e653939486e7377566563746f7273466f726d61744d65746100000001f0f3\
         33fce4791724f4a7967b3dfae3a21b4c7563656e653939486e7377566563746f7273466f726d6174\
         5f300100000001000000000000005300020600000010000200000000000000000000005300020600\
         00001000030000000100000000000000530002030000001000ffffffffc02893e800000000000000\
         005fc9b048",
    ),
    (
        "_2_Lucene99HnswVectorsFormat_0.vemf",
        "3fd76c171d4c7563656e653939466c6174566563746f7273466f726d61744d65746100000000f0f3\
         33fce4791724f4a7967b3dfae3a21b4c7563656e653939486e7377566563746f7273466f726d6174\
         5f300100000001000000000000008001300206000000ffffffffffffffff0000000000000000ffff\
         ff020000000000000000000000b0010c0206000000ffffffffffffffff0000000000000000ffffff\
         030000000100000000000000c001180203000000d8000000000000001000000000000000000009e8\
         00000000000000100000000000000000000000400000000000000000000000000000000000ffffff\
         ffc02893e800000000000000002edd4019",
    ),
    (
        "_2_Lucene99HnswVectorsFormat_0.vex",
        "3fd76c171e4c7563656e653939486e7377566563746f7273466f726d6174496e64657800000001f0\
         f333fce4791724f4a7967b3dfae3a21b4c7563656e653939486e7377566563746f7273466f726d61\
         745f30c02893e800000000000000003b6d68c1",
    ),
];

/// Vector files of the merged segment, case `default true`.
const LUCENE_DEFAULT_DELETIONS: &[(&str, &str)] = &[
    (
        "_2_Lucene99HnswVectorsFormat_0.vec",
        "3fd76c171d4c7563656e653939466c6174566563746f7273466f726d617444617461000000004c29\
         be03aa824c73b1221d9e7cf57ea51b4c7563656e653939486e7377566563746f7273466f726d6174\
         5f300000000000000000000000000000000000000000000000000000000000000000000000000000\
         0000000000000000000000000000803f000000400000004000004040000020400000a04000006040\
         00c802c603c505c30000000000000000000000000000000000000000000000000000000000000000\
         00008040000000c00000010000000100ff7f0000ffff000000000000000000000000000000000000\
         00000000000000000000000000000000c02893e80000000000000000d3e952b2",
    ),
    (
        "_2_Lucene99HnswVectorsFormat_0.vem",
        "3fd76c171d4c7563656e653939486e7377566563746f7273466f726d61744d657461000000014c29\
         be03aa824c73b1221d9e7cf57ea51b4c7563656e653939486e7377566563746f7273466f726d6174\
         5f300100000001000000000000005300020400000010000200000000000000000000005300020400\
         00001000030000000100000000000000530002020000001000040000000100000000000000530001\
         000000001000ffffffffc02893e800000000000000007d3ceb25",
    ),
    (
        "_2_Lucene99HnswVectorsFormat_0.vemf",
        "3fd76c171d4c7563656e653939466c6174566563746f7273466f726d61744d657461000000004c29\
         be03aa824c73b1221d9e7cf57ea51b4c7563656e653939486e7377566563746f7273466f726d6174\
         5f300100000001000000000000008001200204000000ffffffffffffffff0000000000000000ffff\
         ff020000000000000000000000a001080204000000ffffffffffffffff0000000000000000ffffff\
         030000000100000000000000c001100202000000d0000000000000000e00000000000000000009de\
         000000000000001000000000000000000000803f0000000000000000000000000000000000040000\
         0001000000000000008002000100000000feffffffffffffff0000000000000000ffffffffffffff\
         c02893e8000000000000000044f330fd",
    ),
    (
        "_2_Lucene99HnswVectorsFormat_0.vex",
        "3fd76c171e4c7563656e653939486e7377566563746f7273466f726d6174496e646578000000014c\
         29be03aa824c73b1221d9e7cf57ea51b4c7563656e653939486e7377566563746f7273466f726d61\
         745f30c02893e8000000000000000037296627",
    ),
];

/// Vector files of the merged segment, case `sq false`.
const LUCENE_SQ_NO_DELETIONS: &[(&str, &str)] = &[
    (
        "_2_Lucene104ScalarQuantizedVectorsFormat_0.vec",
        "3fd76c171d4c7563656e653939466c6174566563746f7273466f726d617444617461000000009d9f\
         7965bef73609739bed40a7870f3d274c7563656e653130345363616c61725175616e74697a656456\
         6563746f7273466f726d61745f300000000000000000000000000000000000000000000000000000\
         0000000000000000000000000000803f0000803f0000c03f00000040000000400000404000002040\
         00008040000040400000a0400000604000c801c702c603c504c405c3000000000000000000000000\
         00008040000000c000000041000080c000000200000002000400ff7f0000ffffc02893e800000000\
         0000000050665696",
    ),
    (
        "_2_Lucene104ScalarQuantizedVectorsFormat_0.vemf",
        "3fd76c171d4c7563656e653939466c6174566563746f7273466f726d61744d657461000000009d9f\
         7965bef73609739bed40a7870f3d274c7563656e653130345363616c61725175616e74697a656456\
         6563746f7273466f726d61745f300100000001000000000000008001300206000000ffffffffffff\
         ffff0000000000000000ffffff020000000000000000000000b0010c0206000000ffffffffffffff\
         ff0000000000000000ffffff030000000100000000000000c001180203000000d800000000000000\
         1000000000000000000009e800000000000000100000000000000000000000400000000000000000\
         000000000000000000ffffffffc02893e8000000000000000064a7483e",
    ),
    (
        "_2_Lucene104ScalarQuantizedVectorsFormat_0.vemq",
        "3fd76c17294c7563656e653130345363616c61725175616e74697a6564566563746f7273466f726d\
         61744d657461000000009d9f7965bef73609739bed40a7870f3d274c7563656e653130345363616c\
         61725175616e74697a6564566563746f7273466f726d61745f30010000000100000000000000026c\
         6c0600000020400000104000003541ffffffffffffffff0000000000000000ffffff030000000100\
         00000000000002d80136030000008040000000c00000a0410e010000000000001000000000000000\
         0000091e010000000000001000000000000000000000004000000000000000000000000000000000\
         00ffffffffc02893e800000000000000009efa51c6",
    ),
    (
        "_2_Lucene104ScalarQuantizedVectorsFormat_0.veq",
        "3fd76c17294c7563656e653130345363616c61725175616e74697a6564566563746f7273466f726d\
         617444617461000000009d9f7965bef73609739bed40a7870f3d274c7563656e653130345363616c\
         61725175616e74697a6564566563746f7273466f726d61745f30000000ff000020c00000a0bf0000\
         fa40ff00000000ff0000c0bf000040bf00003440ff00000000ff000000bf000080be0000a03eff00\
         0000ff000000803e0000003f0000a03eff000000ff000000403f0000c03f00003440ff000000ff00\
         0000a03f000020400000fa40ff00000000ff000080c0000000400000a041ff000000000000000000\
         000000000000000000000000ff00000000c0000080400000a041ff00000000000200000002000400\
         ff7f0000ffffc02893e800000000000000003f941fc1",
    ),
];

/// Vector files of the merged segment, case `sq true`.
const LUCENE_SQ_DELETIONS: &[(&str, &str)] = &[
    (
        "_2_Lucene104ScalarQuantizedVectorsFormat_0.vec",
        "3fd76c171d4c7563656e653939466c6174566563746f7273466f726d61744461746100000000878a\
         a2302188325d88880697c5233853274c7563656e653130345363616c61725175616e74697a656456\
         6563746f7273466f726d61745f300000000000000000000000000000000000000000000000000000\
         0000000000000000000000000000803f000000400000004000004040000020400000a04000006040\
         00c802c603c505c30000000000000000000000000000000000000000000000000000000000000000\
         00008040000000c00000010000000100ff7f0000ffff000000000000000000000000000000000000\
         00000000000000000000000000000000c02893e800000000000000008b0f0886",
    ),
    (
        "_2_Lucene104ScalarQuantizedVectorsFormat_0.vemf",
        "3fd76c171d4c7563656e653939466c6174566563746f7273466f726d61744d65746100000000878a\
         a2302188325d88880697c5233853274c7563656e653130345363616c61725175616e74697a656456\
         6563746f7273466f726d61745f300100000001000000000000008001200204000000ffffffffffff\
         ffff0000000000000000ffffff020000000000000000000000a001080204000000ffffffffffffff\
         ff0000000000000000ffffff030000000100000000000000c001100202000000d000000000000000\
         0e00000000000000000009de000000000000001000000000000000000000803f0000000000000000\
         0000000000000000000400000001000000000000008002000100000000feffffffffffffff000000\
         0000000000ffffffffffffffc02893e80000000000000000e8891bae",
    ),
    (
        "_2_Lucene104ScalarQuantizedVectorsFormat_0.vemq",
        "3fd76c17294c7563656e653130345363616c61725175616e74697a6564566563746f7273466f726d\
         61744d65746100000000878aa2302188325d88880697c5233853274c7563656e653130345363616c\
         61725175616e74697a6564566563746f7273466f726d61745f30010000000100000000000000026c\
         480400000020400000104000003541ffffffffffffffff0000000000000000ffffff030000000100\
         00000000000002b40124020000008040000000c00000a041d8000000000000000e00000000000000\
         000009e6000000000000001000000000000000000000803f00000000000000000000000000000000\
         0004000000010000000000000001e8010000feffffffffffffff0000000000000000ffffffffffff\
         ffc02893e80000000000000000c4d9d13b",
    ),
    (
        "_2_Lucene104ScalarQuantizedVectorsFormat_0.veq",
        "3fd76c17294c7563656e653130345363616c61725175616e74697a6564566563746f7273466f726d\
         61744461746100000000878aa2302188325d88880697c5233853274c7563656e653130345363616c\
         61725175616e74697a6564566563746f7273466f726d61745f30000000ff000020c00000a0bf0000\
         fa40ff00000000ff000000bf000080be0000a03eff000000ff000000803e0000003f0000a03eff00\
         0000ff000000a03f000020400000fa40ff00000000ff000080c0000000400000a041ff0000000000\
         000000000000000000000000000000000000010000000100ff7f0000ffff0000c02893e800000000\
         0000000079cb94e9",
    ),
];

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Document `i` of `MergedVectorsFixture.doc(int, boolean)`, field for field
/// and in the same order, so that field numbers agree with Lucene's.
fn vec_doc(i: i32, deletable: bool) -> Document {
    let mut doc = Document::new();
    doc.add(Box::new(
        StoredField::new_string("id", format!("doc-{i}")).unwrap(),
    ));
    doc.add(Box::new(
        KnnFloatVectorField::with_euclidean("f", &[i as f32, 1.0 + i as f32 * 0.5]).unwrap(),
    ));
    doc.add(Box::new(
        KnnByteVectorField::with_euclidean("b", &[i as u8, (200 - i) as u8]).unwrap(),
    ));
    if i % 2 == 0 {
        // Java's `-i` is an `int`, so document 0 stores +0.0, not -0.0.
        doc.add(Box::new(
            KnnFloatVectorField::with_euclidean("s", &[i as f32 * 2.0, (-i) as f32]).unwrap(),
        ));
    }
    if deletable && (i == 1 || i == 4) {
        doc.add(Box::new(
            KnnFloatVectorField::with_euclidean("del", &[1.0]).unwrap(),
        ));
    }
    doc
}

fn from_hex(hex: &str) -> Vec<u8> {
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
        .collect()
}

fn read_file(dir: &dyn Directory, name: &str) -> Vec<u8> {
    let length = dir.file_length(name).unwrap() as usize;
    let mut input = dir.open_input(name, &*DEFAULT_IO_CONTEXT).unwrap();
    let mut bytes = vec![0u8; length];
    input.read_bytes(&mut bytes, 0, length).unwrap();
    bytes
}

/// Where the sixteen-byte segment id starts in a file that begins with
/// `CodecUtil.writeIndexHeader`: magic, codec name, version, then the id.
fn segment_id_offset(file: &[u8]) -> usize {
    let codec_name_length = file[4] as usize;
    assert!(codec_name_length < 128, "a one-byte vInt");
    4 + 1 + codec_name_length + 4
}

/// Checks that the footer's checksum is the CRC-32 of everything before it,
/// as `CodecUtil.writeFooter` writes it.
fn assert_valid_footer(name: &str, file: &[u8]) {
    let body = &file[..file.len() - 8];
    let stored = u64::from_be_bytes(file[file.len() - 8..].try_into().unwrap());
    assert_eq!(
        stored,
        u64::from(crc32fast::hash(body)),
        "{name}: the footer checksum must cover the file"
    );
}

/// Asserts that every reference file exists in `dir` with Lucene's bytes,
/// except for the segment id and the checksum that covers it.
fn assert_same_files_except_segment_id(dir: &dyn Directory, reference: &[(&str, &str)]) {
    for (name, hex) in reference {
        let mut expected = from_hex(hex);
        let mut actual = read_file(dir, name);
        assert_valid_footer(name, &actual);
        assert_eq!(actual.len(), expected.len(), "{name}: length");
        let id = segment_id_offset(&expected);
        let footer = expected.len() - 8;
        for bytes in [&mut expected, &mut actual] {
            bytes[id..id + 16].fill(0);
            bytes[footer..].fill(0);
        }
        assert_eq!(actual, expected, "{name}: bytes");
    }
}

/// Writes documents 0-2 and 3-5 as two committed segments.
fn two_segments(
    codec: Option<Arc<dyn Codec>>,
    deletable: bool,
) -> (Arc<dyn Directory>, IndexWriter) {
    ensure_codec();
    let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
    let mut config = IndexWriterConfig::new();
    if let Some(codec) = codec {
        config.set_codec(codec).unwrap();
    }
    if deletable {
        // No merge runs before the test drives one itself.
        config
            .set_merge_policy(Arc::new(NoMergePolicy::new()))
            .unwrap();
    }
    let writer = IndexWriter::new(Arc::clone(&dir), config).unwrap();
    for i in 0..3 {
        writer.add_document(&vec_doc(i, deletable)).unwrap();
    }
    writer.commit().unwrap();
    for i in 3..6 {
        writer.add_document(&vec_doc(i, deletable)).unwrap();
    }
    writer.commit().unwrap();
    (dir, writer)
}

/// Every `(doc, vector)` of float field `field` across the index.
fn float_vectors(reader: &Arc<dyn DirectoryReader>, field: &str) -> Vec<(i32, Vec<f32>)> {
    let mut out = Vec::new();
    for leaf in Arc::clone(reader).leaves() {
        let leaf_reader = leaf.leaf_reader();
        let values = leaf_reader.get_float_vector_values(field).unwrap().unwrap();
        let mut iterator = values.iterator().unwrap();
        loop {
            let doc = iterator.next_doc().unwrap();
            if doc == NO_MORE_DOCS {
                break;
            }
            out.push((
                leaf.doc_base() + doc,
                values.vector_value(iterator.index()).unwrap(),
            ));
        }
    }
    out
}

/// Every `(doc, vector)` of byte field `field` across the index.
fn byte_vectors(reader: &Arc<dyn DirectoryReader>, field: &str) -> Vec<(i32, Vec<u8>)> {
    let mut out = Vec::new();
    for leaf in Arc::clone(reader).leaves() {
        let leaf_reader = leaf.leaf_reader();
        let values = leaf_reader.get_byte_vector_values(field).unwrap().unwrap();
        let mut iterator = values.iterator().unwrap();
        loop {
            let doc = iterator.next_doc().unwrap();
            if doc == NO_MORE_DOCS {
                break;
            }
            out.push((
                leaf.doc_base() + doc,
                values.vector_value(iterator.index()).unwrap(),
            ));
        }
    }
    out
}

/// Force-merges the two segments and checks what a reader then sees.
fn force_merge_vectors(codec: Option<Arc<dyn Codec>>) -> Arc<dyn Directory> {
    let (dir, writer) = two_segments(codec, false);
    writer.force_merge(1).unwrap();
    writer.commit().unwrap();
    writer.close().unwrap();
    assert_eq!(
        SegmentInfos::read_latest_commit(Arc::clone(&dir))
            .unwrap()
            .size(),
        1
    );

    let reader = directory_reader::open(Arc::clone(&dir)).unwrap();
    assert_eq!((reader.num_docs(), reader.max_doc()), (6, 6));
    assert_eq!(
        float_vectors(&reader, "f"),
        (0..6)
            .map(|i| (i, vec![i as f32, 1.0 + i as f32 * 0.5]))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        byte_vectors(&reader, "b"),
        (0..6)
            .map(|i| (i, vec![i as u8, (200 - i) as u8]))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        float_vectors(&reader, "s"),
        vec![
            (0, vec![0.0, 0.0]),
            (2, vec![4.0, -2.0]),
            (4, vec![8.0, -4.0])
        ]
    );
    dir
}

/// Commits the delete of documents 1 and 4, then merges the two segments
/// with `SegmentMerger` — the merger `IndexWriter` drives — into a segment
/// named and identified as Lucene's reference segment.
fn merge_without_deleted(
    codec: Arc<dyn Codec>,
    custom: bool,
    reference: &[(&str, &str)],
) -> Arc<dyn Directory> {
    let (dir, writer) = two_segments(custom.then(|| Arc::clone(&codec)), true);
    let delete: Arc<dyn Query> = Arc::new(FieldExistsQuery::new("del"));
    writer.delete_documents_by_queries(vec![delete]).unwrap();
    writer.commit().unwrap();
    writer.close().unwrap();

    let infos = SegmentInfos::read_latest_commit(Arc::clone(&dir)).unwrap();
    let readers: Vec<Arc<dyn CodecReader>> = infos
        .iter()
        .map(|info| {
            Arc::new(
                SegmentReader::new(
                    info.clone(),
                    infos.index_created_version_major(),
                    &*DEFAULT_IO_CONTEXT,
                )
                .unwrap(),
            ) as Arc<dyn CodecReader>
        })
        .collect();
    assert_eq!(
        readers
            .iter()
            .map(|r| (r.max_doc(), r.num_docs()))
            .collect::<Vec<_>>(),
        vec![(3, 2), (3, 2)],
        "documents 1 and 4 are deleted"
    );

    let mut builder = FieldInfosBuilder::new(Arc::new(FieldNumbers::new(None, None).unwrap()));
    for reader in &readers {
        for field in reader.get_field_infos().iter() {
            builder.add(field).unwrap();
        }
    }
    let live_docs: Vec<_> = readers.iter().map(|r| r.get_live_docs()).collect();
    let max_docs: Vec<i32> = readers.iter().map(|r| r.max_doc()).collect();
    let num_docs: Vec<i32> = readers.iter().map(|r| r.num_docs()).collect();
    let reference_file = from_hex(reference[0].1);
    let id_at = segment_id_offset(&reference_file);
    let id: [u8; 16] = reference_file[id_at..id_at + 16].try_into().unwrap();

    let target: Arc<dyn Directory> = Arc::new(RamDirectory::new());
    let segment_info = SegmentInfo::new(
        Arc::clone(&target),
        Version::LATEST,
        None,
        "_2".to_string(),
        num_docs.iter().sum(),
        false,
        false,
        Arc::clone(&codec),
        Default::default(),
        id,
        Default::default(),
        Default::default(),
    )
    .unwrap();
    let mut merger = SegmentMerger::new(
        &readers,
        segment_info,
        builder.finish().unwrap(),
        MergeState::build_doc_maps(&live_docs, &max_docs, &num_docs),
        false,
        codec,
        Arc::clone(&target),
        Arc::from(rucene::store::merge_io_context(
            rucene::store::MergeInfo::new(0, -1, false, -1),
        )),
        Arc::new(rucene::util::NoOutputInfoStream),
    )
    .unwrap();
    merger.merge().unwrap();
    target
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Two commits of one document each, force-merged into one segment, which a
/// new reader opens with both documents (rmp #213).
///
/// The documents store a field and index nothing, for the reason the module
/// documentation gives; the stored fields take the bulk-copy route, so the
/// merge only succeeds when the merge state holds merge instances (rmp #215).
#[test]
fn force_merge_turns_two_commits_into_one_segment_with_every_document() {
    ensure_codec();
    let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
    let writer = IndexWriter::new(Arc::clone(&dir), IndexWriterConfig::new()).unwrap();
    for id in ["a", "b"] {
        let mut doc = Document::new();
        doc.add(Box::new(
            StoredField::new_string("id", id.to_string()).unwrap(),
        ));
        writer.add_document(&doc).unwrap();
        writer.commit().unwrap();
    }
    let sources = SegmentInfos::read_latest_commit(Arc::clone(&dir)).unwrap();
    assert_eq!(sources.size(), 2);
    assert!(sources
        .iter()
        .all(|info| info.info.min_version() == Some(Version::LATEST)));

    writer.force_merge(1).unwrap();
    writer.commit().unwrap();
    writer.close().unwrap();

    let infos = SegmentInfos::read_latest_commit(Arc::clone(&dir)).unwrap();
    assert_eq!(infos.size(), 1, "one segment after force_merge(1)");
    let merged = infos.info(0);
    assert_eq!(merged.info.max_doc().unwrap(), 2);
    assert_eq!(
        merged.info.min_version(),
        Some(Version::LATEST),
        "the merged segment records the oldest minVersion of its sources"
    );

    let reader = directory_reader::open(Arc::clone(&dir)).unwrap();
    assert_eq!((reader.num_docs(), reader.max_doc()), (2, 2));
    let stored = reader.stored_fields().unwrap();
    let ids: Vec<String> = (0..2)
        .map(|doc| {
            stored
                .document(doc)
                .unwrap()
                .get_field("id")
                .and_then(|field| field.string_value())
                .unwrap()
        })
        .collect();
    assert_eq!(ids, vec!["a".to_string(), "b".to_string()]);
}

/// The `SegmentInfo` that `force_merge` commits carries every change the merge
/// made to it, as Lucene's single shared object does.
///
/// During a merge Lucene mutates one `SegmentInfo`, shared by `IndexWriter`,
/// `SegmentMerger`, `MergeState` and every codec writer: `setMaxDoc`
/// (`MergeState.java:173`), `minVersion` (`SegmentMerger.java:89`), the
/// stored-fields mode attribute (`Lucene90StoredFieldsFormat.java:142`) and the
/// files (`IndexWriter.java:5404`, `Lucene99SegmentInfoFormat.java:176`). The
/// expected values were measured on a real Lucene 10.5.0 `IndexWriter` running
/// the same two commits and `forceMerge(1)`: `maxDoc=2 minVersion=10.5.0
/// compound=false`, `attributes={Lucene90StoredFieldsFormat.mode=BEST_SPEED}`,
/// `source=merge mergeFactor=2 mergeMaxNumSegments=1`, and
/// `files=[_2.fdm, _2.fdt, _2.fdx, _2.fnm, _2.si]`.
#[test]
fn the_committed_segment_info_carries_every_change_the_merge_made() {
    ensure_codec();
    let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
    let writer = IndexWriter::new(Arc::clone(&dir), IndexWriterConfig::new()).unwrap();
    for id in ["a", "b"] {
        let mut doc = Document::new();
        doc.add(Box::new(
            StoredField::new_string("id", id.to_string()).unwrap(),
        ));
        writer.add_document(&doc).unwrap();
        writer.commit().unwrap();
    }
    writer.force_merge(1).unwrap();
    writer.commit().unwrap();
    writer.close().unwrap();

    // Read back from the `.si` file the commit points at.
    let infos = SegmentInfos::read_latest_commit(Arc::clone(&dir)).unwrap();
    assert_eq!(infos.size(), 1);
    let merged = &infos.info(0).info;
    assert_eq!(merged.name, "_2");
    assert_eq!(merged.max_doc().unwrap(), 2);
    assert_eq!(merged.min_version(), Some(Version::LATEST));
    assert!(!merged.get_use_compound_file());
    assert_eq!(
        merged
            .get_attributes()
            .into_iter()
            .collect::<std::collections::BTreeMap<_, _>>(),
        std::collections::BTreeMap::from([(
            "Lucene90StoredFieldsFormat.mode".to_string(),
            "BEST_SPEED".to_string()
        )]),
        "the stored-fields format records its mode on the merged segment"
    );
    let diagnostics = merged.get_diagnostics();
    for (key, value) in [
        ("source", "merge"),
        ("mergeFactor", "2"),
        ("mergeMaxNumSegments", "1"),
    ] {
        assert_eq!(
            diagnostics.get(key).map(String::as_str),
            Some(value),
            "{key}"
        );
    }
    let mut files: Vec<String> = merged.files().unwrap().into_iter().collect();
    files.sort();
    assert_eq!(files, vec!["_2.fdm", "_2.fdt", "_2.fdx", "_2.fnm", "_2.si"]);
}

/// A document of `MergedVectorsFixture.graphDoc(int)`: integer arithmetic only,
/// so the vectors are exactly Lucene's.
fn graph_doc(i: i32) -> Document {
    let mut doc = Document::new();
    doc.add(Box::new(
        StoredField::new_string("id", format!("doc-{i}")).unwrap(),
    ));
    doc.add(Box::new(
        KnnFloatVectorField::with_euclidean(
            "f",
            &[
                ((i * 7919) % 1009) as f32 / 7.0,
                ((i * 104729) % 1013) as f32 / 11.0,
            ],
        )
        .unwrap(),
    ));
    doc.add(Box::new(
        KnnByteVectorField::with_euclidean("b", &[((i * 31) % 251) as u8, ((i * 17) % 241) as u8])
            .unwrap(),
    ));
    doc
}

/// Documents per source segment of the graph case, as in the fixture: each
/// source is too small for a graph of its own (400 is not above
/// `ln(400) * 100`), the merged 800 are.
const GRAPH_DOCS_PER_SEGMENT: i32 = 400;

/// Dimensions of the float vectors of [`wide_graph_doc`]
/// (`MergedVectorsFixture.WIDE_DIMENSIONS`).
const WIDE_DIMENSIONS: i32 = 16;

/// Document `i` of the fixture's `wide-graph` case
/// (`MergedVectorsFixture.wideGraphDoc`): [`graph_doc`] with a float vector of
/// [`WIDE_DIMENSIONS`] dimensions. Two dimensions quantize almost exactly, so a
/// graph built from quantized scores matches the one built from raw scores;
/// sixteen do not, which is what tells the two merge branches apart.
fn wide_graph_doc(i: i32) -> Document {
    let mut doc = Document::new();
    doc.add(Box::new(
        StoredField::new_string("id", format!("doc-{i}")).unwrap(),
    ));
    let f: Vec<f32> = (0..WIDE_DIMENSIONS)
        .map(|j| ((i * 7919 + j * 104729) % 1009) as f32 / 7.0)
        .collect();
    doc.add(Box::new(
        KnnFloatVectorField::with_euclidean("f", &f).unwrap(),
    ));
    doc.add(Box::new(
        KnnByteVectorField::with_euclidean("b", &[((i * 31) % 251) as u8, ((i * 17) % 241) as u8])
            .unwrap(),
    ));
    doc
}

/// What `MergedVectorsFixture <dir> <codec> graph` printed for one vector
/// file: its length and the CRC-32 of its bytes with the segment id and the
/// footer checksum zeroed.
type FileSummary = (&'static str, usize, u32);

const LUCENE_DEFAULT_GRAPH: &[FileSummary] = &[
    ("_2_Lucene99HnswVectorsFormat_0.vec", 8144, 1926929054),
    ("_2_Lucene99HnswVectorsFormat_0.vem", 355, 1798965451),
    ("_2_Lucene99HnswVectorsFormat_0.vemf", 182, 4203148412),
    ("_2_Lucene99HnswVectorsFormat_0.vex", 17786, 1708547974),
];

/// `MergedVectorsFixture <dir> sq-then-hnswsq graph`: the sources are written
/// with `Lucene104ScalarQuantizedVectorsFormat` and merged with
/// `Lucene104HnswScalarQuantizedVectorsFormat`.
const LUCENE_QUANTIZED_TO_HNSW_QUANTIZED_GRAPH: &[FileSummary] = &[
    (
        "_2_Lucene104HnswBinaryQuantizedVectorsFormat_0.vec",
        8144,
        1764646690,
    ),
    (
        "_2_Lucene104HnswBinaryQuantizedVectorsFormat_0.vem",
        371,
        1332006794,
    ),
    (
        "_2_Lucene104HnswBinaryQuantizedVectorsFormat_0.vemf",
        198,
        602918231,
    ),
    (
        "_2_Lucene104HnswBinaryQuantizedVectorsFormat_0.vemq",
        180,
        4041717773,
    ),
    (
        "_2_Lucene104HnswBinaryQuantizedVectorsFormat_0.veq",
        14528,
        4004729023,
    ),
    (
        "_2_Lucene104HnswBinaryQuantizedVectorsFormat_0.vex",
        17802,
        2870311080,
    ),
];

/// The vector files of the `sq-then-hnswsq wide-graph` case.
const LUCENE_QUANTIZED_TO_HNSW_QUANTIZED_WIDE_GRAPH: &[FileSummary] = &[
    (
        "_2_Lucene104HnswBinaryQuantizedVectorsFormat_0.vec",
        52944,
        2333235778,
    ),
    (
        "_2_Lucene104HnswBinaryQuantizedVectorsFormat_0.vem",
        371,
        1269483592,
    ),
    (
        "_2_Lucene104HnswBinaryQuantizedVectorsFormat_0.vemf",
        200,
        2359741830,
    ),
    (
        "_2_Lucene104HnswBinaryQuantizedVectorsFormat_0.vemq",
        237,
        2082232662,
    ),
    (
        "_2_Lucene104HnswBinaryQuantizedVectorsFormat_0.veq",
        25728,
        2311740537,
    ),
    (
        "_2_Lucene104HnswBinaryQuantizedVectorsFormat_0.vex",
        18121,
        1377150082,
    ),
];

/// The top ten hits of the `sq-then-hnswsq wide-graph` merged segment for
/// `KnnFloatVectorQuery("f", [50, 51, .., 65], 10)`, as `(doc, score bits)`.
const LUCENE_WIDE_GRAPH_HITS_F: &[(i32, u32)] = &[
    (696, 942281167),
    (175, 942266136),
    (419, 942258196),
    (663, 942249970),
    (142, 942232679),
    (386, 942223618),
    (630, 942214279),
    (109, 942194772),
    (353, 942184612),
    (597, 942174181),
];

/// The top ten hits Lucene's merged segment answers for
/// `KnnFloatVectorQuery("f", [50, 40], 10)` and
/// `KnnByteVectorQuery("b", [100, 50], 10)`, as `(doc, score bits)`.
const LUCENE_GRAPH_HITS_F: &[(i32, u32)] = &[
    (466, 1038876888),
    (479, 1033606559),
    (222, 1033605381),
    (235, 1027454606),
    (453, 1020422859),
    (209, 1020231911),
    (492, 1016880850),
    (796, 1016575014),
    (123, 1016340458),
    (248, 1014825715),
];
const LUCENE_GRAPH_HITS_B: &[(i32, u32)] = &[
    (570, 1065353216),
    (400, 1042983595),
    (740, 1042983595),
    (230, 1027804209),
    (60, 1018304067),
    (3, 1017159841),
    (173, 1014513168),
    (343, 1010043713),
    (797, 1010043713),
    (627, 1006766113),
];

/// CRC-32 of `file` with the header's segment id and the footer checksum
/// zeroed, as `MergedVectorsFixture.maskedCrc` computes it.
fn masked_crc(file: &[u8]) -> u32 {
    let mut masked = file.to_vec();
    let id = segment_id_offset(&masked);
    masked[id..id + 16].fill(0);
    let footer = masked.len() - 8;
    masked[footer..].fill(0);
    crc32fast::hash(&masked)
}

/// Force-merges two 400-document segments into one large enough for a graph;
/// `merging`, when given, is raised just before the merge.
fn force_merge_graph(
    codec: Option<Arc<dyn Codec>>,
    merging: Option<Arc<AtomicBool>>,
    doc: fn(i32) -> Document,
) -> Arc<dyn Directory> {
    ensure_codec();
    let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
    let mut config = IndexWriterConfig::new();
    if let Some(codec) = codec {
        config.set_codec(codec).unwrap();
    }
    let writer = IndexWriter::new(Arc::clone(&dir), config).unwrap();
    for i in 0..GRAPH_DOCS_PER_SEGMENT {
        writer.add_document(&doc(i)).unwrap();
    }
    writer.commit().unwrap();
    for i in GRAPH_DOCS_PER_SEGMENT..2 * GRAPH_DOCS_PER_SEGMENT {
        writer.add_document(&doc(i)).unwrap();
    }
    writer.commit().unwrap();
    if let Some(merging) = merging {
        merging.store(true, Ordering::SeqCst);
    }
    writer.force_merge(1).unwrap();
    writer.commit().unwrap();
    writer.close().unwrap();
    dir
}

/// Asserts each vector file has Lucene's length and masked CRC, and a valid
/// footer of its own.
fn assert_same_file_summaries(dir: &dyn Directory, reference: &[FileSummary]) {
    for &(name, length, crc) in reference {
        let file = read_file(dir, name);
        assert_valid_footer(name, &file);
        assert_eq!(file.len(), length, "{name}: length");
        assert_eq!(masked_crc(&file), crc, "{name}: bytes");
    }
}

/// Runs one kNN query on the merged index and returns its hits as
/// `(doc, score bits)`.
fn knn_hits(dir: &Arc<dyn Directory>, query: Arc<dyn Query>) -> Vec<(i32, u32)> {
    let reader = directory_reader::open(Arc::clone(dir)).unwrap();
    let searcher = IndexSearcher::new(reader as Arc<dyn rucene::index::IndexReader>).unwrap();
    searcher
        .search(query, 10)
        .unwrap()
        .score_docs
        .iter()
        .map(|hit| (hit.doc, hit.score.to_bits()))
        .collect()
}

/// A merge whose segment is large enough for a graph builds it in the second
/// phase and writes Lucene's graph and metadata: the deferred work of
/// `Lucene99HnswVectorsWriter.mergeOneField` (`Lucene99HnswVectorsWriter.java:423-461`),
/// through the flat vectors' own scorer.
#[test]
fn a_merged_segment_large_enough_gets_lucenes_hnsw_graph() {
    let dir = force_merge_graph(None, None, graph_doc);
    assert_same_file_summaries(dir.as_ref(), LUCENE_DEFAULT_GRAPH);
    let float_query: Arc<dyn Query> =
        Arc::new(KnnFloatVectorQuery::new("f", vec![50.0, 40.0], 10).unwrap());
    assert_eq!(knn_hits(&dir, float_query), LUCENE_GRAPH_HITS_F);
    let byte_query: Arc<dyn Query> =
        Arc::new(KnnByteVectorQuery::new("b", vec![100, 50], 10).unwrap());
    assert_eq!(knn_hits(&dir, byte_query), LUCENE_GRAPH_HITS_B);
}

/// Merging into the HNSW scalar-quantized format builds the float field's
/// graph through the quantized reader's `getRandomVectorScorerSupplierForMerge`
/// and the byte field's through the flat scorer
/// (`Lucene99HnswVectorsWriter.java:436-453`), and writes Lucene's files.
///
/// The sources are written with the flat scalar-quantized format, as the
/// fixture's `sq-then-hnswsq` case does by switching formats before
/// `forceMerge`: segments flushed by this port's HNSW scalar-quantized writer
/// cannot have their float vectors read back (see the module documentation).
#[test]
fn a_merge_into_the_hnsw_quantized_format_gets_lucenes_files() {
    let merging = Arc::new(AtomicBool::new(false));
    let codec = SwitchingCodec::new(
        "Lucene104ScalarQuantizedVectorsFormat",
        "Lucene104HnswBinaryQuantizedVectorsFormat",
        Arc::clone(&merging),
    );
    let dir = force_merge_graph(Some(Arc::new(codec)), Some(merging), graph_doc);
    assert_same_file_summaries(dir.as_ref(), LUCENE_QUANTIZED_TO_HNSW_QUANTIZED_GRAPH);
}

/// The float field's graph is built from the quantized reader's scores, not
/// the raw vectors': with [`WIDE_DIMENSIONS`]-dimension vectors the two give
/// different graphs, and the merged files are Lucene's, as are the hits the
/// merged segment answers (`MergedVectorsFixture sq-then-hnswsq wide-graph`).
#[test]
fn a_wide_merge_into_the_hnsw_quantized_format_builds_from_quantized_scores() {
    let merging = Arc::new(AtomicBool::new(false));
    let codec = SwitchingCodec::new(
        "Lucene104ScalarQuantizedVectorsFormat",
        "Lucene104HnswBinaryQuantizedVectorsFormat",
        Arc::clone(&merging),
    );
    let dir = force_merge_graph(Some(Arc::new(codec)), Some(merging), wide_graph_doc);
    assert_same_file_summaries(dir.as_ref(), LUCENE_QUANTIZED_TO_HNSW_QUANTIZED_WIDE_GRAPH);
    let target: Vec<f32> = (0..WIDE_DIMENSIONS).map(|j| 50.0 + j as f32).collect();
    let float_query: Arc<dyn Query> = Arc::new(KnnFloatVectorQuery::new("f", target, 10).unwrap());
    assert_eq!(knn_hits(&dir, float_query), LUCENE_WIDE_GRAPH_HITS_F);
    let byte_query: Arc<dyn Query> =
        Arc::new(KnnByteVectorQuery::new("b", vec![100, 50], 10).unwrap());
    assert_eq!(knn_hits(&dir, byte_query), LUCENE_GRAPH_HITS_B);
}

/// Rewrites the field number of the first entry of the `.vemq` file `name`
/// from `from` to `to`, and recomputes the footer checksum: the segment's
/// field infos still list field `from` with vectors, but its scalar-quantized
/// metadata no longer holds an entry for it.
fn retarget_first_vemq_entry(dir: &dyn Directory, name: &str, from: i32, to: i32) {
    let mut bytes = read_file(dir, name);
    let suffix = segment_id_offset(&bytes) + 16;
    let at = suffix + 1 + bytes[suffix] as usize;
    let field_number = |bytes: &[u8]| i32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
    assert_eq!(
        field_number(&bytes),
        from,
        "the first entry is field {from}"
    );
    bytes[at..at + 4].copy_from_slice(&to.to_le_bytes());
    let body = bytes.len() - 8;
    let checksum = u64::from(crc32fast::hash(&bytes[..body]));
    bytes[body..].copy_from_slice(&checksum.to_be_bytes());
    dir.delete_file(name).unwrap();
    let mut out = dir.create_output(name, &*DEFAULT_IO_CONTEXT).unwrap();
    out.write_bytes(&bytes, 0, bytes.len()).unwrap();
    out.close().unwrap();
}

/// A merged field no source reader holds fails the merge with an error, not a
/// panic, and leaves the writer able to close.
///
/// Segment `_0` has fields `v` and `w`, segment `_1` only `w`; `_0`'s `.vemq`
/// is then rewritten so that its entry for `v` names `w`, so its reader opens
/// and answers "absent" for `v`, and the merged view of `v` has no sub. Lucene
/// 10.5.0 fails `forceMerge` on that index with `IndexOutOfBoundsException:
/// Index 0 out of bounds for length 0` from `MergedFloat32VectorValues.dimension()`
/// (`KnnVectorsWriter.java:440`), called by `Lucene99FlatVectorsWriter.writeVectorData`
/// (`:340`) (`SqMergeProbe` on the auditor's index). The merge runs into the
/// flat scalar-quantized format and into the HNSW one, whose writer must not
/// be left unusable.
#[test]
fn a_merged_field_no_source_holds_fails_the_merge_without_a_panic() {
    for merge_format in [
        "Lucene104ScalarQuantizedVectorsFormat",
        "Lucene104HnswBinaryQuantizedVectorsFormat",
    ] {
        ensure_codec();
        let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
        let merging = Arc::new(AtomicBool::new(false));
        let config = || {
            let mut config = IndexWriterConfig::new();
            config
                .set_codec(Arc::new(SwitchingCodec::new(
                    "Lucene104ScalarQuantizedVectorsFormat",
                    merge_format,
                    Arc::clone(&merging),
                )))
                .unwrap();
            config.set_use_compound_file(false).unwrap();
            config
        };
        let writer = IndexWriter::new(Arc::clone(&dir), config()).unwrap();
        for i in 0..2 {
            let mut doc = Document::new();
            doc.add(Box::new(
                KnnFloatVectorField::with_euclidean("v", &[i as f32, 1.0]).unwrap(),
            ));
            doc.add(Box::new(
                KnnFloatVectorField::with_euclidean("w", &[1.0, i as f32]).unwrap(),
            ));
            writer.add_document(&doc).unwrap();
        }
        writer.commit().unwrap();
        for i in 0..2 {
            let mut doc = Document::new();
            doc.add(Box::new(
                KnnFloatVectorField::with_euclidean("w", &[i as f32, 2.0]).unwrap(),
            ));
            writer.add_document(&doc).unwrap();
        }
        writer.commit().unwrap();
        writer.close().unwrap();

        // Positive control: before the rewrite, segment `_0` answers `v`.
        let reader = directory_reader::open(Arc::clone(&dir)).unwrap();
        let first = Arc::clone(&reader).leaves()[0].leaf_reader();
        assert!(first.get_float_vector_values("v").unwrap().is_some());
        drop(reader);

        retarget_first_vemq_entry(
            dir.as_ref(),
            "_0_Lucene104ScalarQuantizedVectorsFormat_0.vemq",
            0,
            1,
        );
        let reader = directory_reader::open(Arc::clone(&dir)).unwrap();
        let first = Arc::clone(&reader).leaves()[0].leaf_reader();
        assert!(first.get_float_vector_values("v").unwrap().is_none());
        assert!(first.get_float_vector_values("w").unwrap().is_some());
        drop(reader);

        merging.store(true, Ordering::SeqCst);
        let writer = IndexWriter::new(Arc::clone(&dir), config()).unwrap();
        let error = writer
            .force_merge(1)
            .expect_err("a merged field without a source fails the merge");
        assert!(
            format!("{error:?}").contains("Index 0 out of bounds for length 0"),
            "{merge_format}: {error:?}"
        );
        writer.close().unwrap();
    }
}

/// Records the messages of the components it is enabled for.
#[derive(Debug)]
struct RecordingInfoStream {
    component: &'static str,
    messages: std::sync::Mutex<Vec<String>>,
}

impl rucene::util::InfoStream for RecordingInfoStream {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.tests.util.RecordingInfoStream"
    }

    fn message(&self, component: &str, message: &str) {
        assert_eq!(component, self.component, "only the enabled component logs");
        self.messages.lock().unwrap().push(message.to_string());
    }

    fn is_enabled(&self, component: &str) -> bool {
        component == self.component
    }

    fn close(&self) {}
}

/// The scalar-quantized merge reports each float field's vector count on the
/// `QVEC` component with Java's text
/// (`Lucene104ScalarQuantizedVectorsWriter.java:333-336`). For this merge
/// Lucene 10.5.0 reports `Vectors' count:6` for `f` and `Vectors' count:3` for
/// `s`, and nothing for the byte field `b` (`QvecProbe`).
#[test]
fn the_quantized_merge_reports_each_fields_vector_count() {
    ensure_codec();
    let stream = Arc::new(RecordingInfoStream {
        component: "QVEC",
        messages: std::sync::Mutex::new(Vec::new()),
    });
    let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
    let mut config = IndexWriterConfig::new();
    config.set_codec(Arc::new(QuantizedCodec::new())).unwrap();
    config
        .set_info_stream(Arc::clone(&stream) as Arc<dyn rucene::util::InfoStream>)
        .unwrap();
    let writer = IndexWriter::new(Arc::clone(&dir), config).unwrap();
    for i in 0..3 {
        writer.add_document(&vec_doc(i, false)).unwrap();
    }
    writer.commit().unwrap();
    for i in 3..6 {
        writer.add_document(&vec_doc(i, false)).unwrap();
    }
    writer.commit().unwrap();
    // Only the merge's messages are compared.
    stream.messages.lock().unwrap().clear();
    writer.force_merge(1).unwrap();
    writer.close().unwrap();
    assert_eq!(
        *stream.messages.lock().unwrap(),
        vec![
            "Vectors' count:6".to_string(),
            "Vectors' count:3".to_string()
        ]
    );
}

/// Segments that do not share their vector fields merge: the second segment's
/// per-field reader answers "absent" for `v2`, Java's `null`
/// (`PerFieldKnnVectorsFormat.java:285-293`), and the scalar-quantized writer
/// skips it when it combines the centroids
/// (`Lucene104ScalarQuantizedVectorsWriter.java:404-406, 449-452`).
/// Lucene 10.5.0 merges the same two segments into one whose `v2` holds two
/// vectors (`SqProbe`).
#[test]
fn segments_that_do_not_share_their_vector_fields_merge() {
    for codec in [
        None,
        Some(Arc::new(QuantizedCodec::new()) as Arc<dyn Codec>),
    ] {
        ensure_codec();
        let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
        let mut config = IndexWriterConfig::new();
        if let Some(codec) = codec.clone() {
            config.set_codec(codec).unwrap();
        }
        let writer = IndexWriter::new(Arc::clone(&dir), config).unwrap();
        for i in 0..2 {
            let mut doc = Document::new();
            doc.add(Box::new(
                KnnFloatVectorField::with_euclidean("v1", &[i as f32, 1.0]).unwrap(),
            ));
            doc.add(Box::new(
                KnnFloatVectorField::with_euclidean("v2", &[1.0, i as f32]).unwrap(),
            ));
            writer.add_document(&doc).unwrap();
        }
        writer.commit().unwrap();
        for i in 0..2 {
            let mut doc = Document::new();
            doc.add(Box::new(
                KnnFloatVectorField::with_euclidean("v1", &[i as f32, 2.0]).unwrap(),
            ));
            writer.add_document(&doc).unwrap();
        }
        writer.commit().unwrap();
        writer.force_merge(1).unwrap();
        writer.commit().unwrap();
        writer.close().unwrap();

        let reader = directory_reader::open(Arc::clone(&dir)).unwrap();
        assert_eq!(Arc::clone(&reader).leaves().len(), 1);
        assert_eq!(
            float_vectors(&reader, "v1"),
            vec![
                (0, vec![0.0, 1.0]),
                (1, vec![1.0, 1.0]),
                (2, vec![0.0, 2.0]),
                (3, vec![1.0, 2.0])
            ]
        );
        assert_eq!(
            float_vectors(&reader, "v2"),
            vec![(0, vec![1.0, 0.0]), (1, vec![1.0, 1.0])],
            "only the first segment's documents have v2"
        );
    }
}

/// `force_merge` with the stock codec writes Lucene's flat vector files for
/// a float field, a byte field and a sparse float field (rmp #200).
#[test]
fn force_merged_vectors_of_the_default_codec_are_lucenes() {
    let dir = force_merge_vectors(None);
    assert_same_files_except_segment_id(dir.as_ref(), LUCENE_DEFAULT_NO_DELETIONS);
}

/// `force_merge` with the scalar-quantized format writes Lucene's raw and
/// quantized vector files.
#[test]
fn force_merged_vectors_of_the_quantized_format_are_lucenes() {
    let dir = force_merge_vectors(Some(Arc::new(QuantizedCodec::new())));
    assert_same_files_except_segment_id(dir.as_ref(), LUCENE_SQ_NO_DELETIONS);
}

/// Merging segments with deleted documents writes Lucene's files byte for
/// byte: the deleted vectors are left out, every survivor takes its merged
/// document id, the `del` field keeps an entry with no vector, and the
/// quantized format recomputes its centroid from every vector, deleted ones
/// included, as `calculateCentroid` does.
#[test]
fn merged_vectors_without_the_deleted_documents_are_lucenes_byte_for_byte() {
    ensure_codec();
    let default_codec = rucene::codecs::for_name("Lucene104").unwrap();
    for (codec, custom, reference) in [
        (default_codec, false, LUCENE_DEFAULT_DELETIONS),
        (
            Arc::new(QuantizedCodec::new()) as Arc<dyn Codec>,
            true,
            LUCENE_SQ_DELETIONS,
        ),
    ] {
        let merged = merge_without_deleted(codec, custom, reference);
        for (name, hex) in reference {
            assert_eq!(
                read_file(merged.as_ref(), name),
                from_hex(hex),
                "{name}: bytes"
            );
        }
    }
}
