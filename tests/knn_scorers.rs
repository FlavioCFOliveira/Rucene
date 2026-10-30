//! The kNN scorers the vector readers hand out, against Lucene 10.5.0.
//!
//! * **rmp #221 — exact search.** `AbstractKnnVectorQuery.exactSearch`
//!   (`AbstractKnnVectorQuery.java:222-226`) scores a segment through
//!   `FloatVectorValues.scorer` / `ByteVectorValues.scorer` and
//!   `VectorScorer.bulk`. It runs when a filter accepts at most `perLeafTopK`
//!   documents of a segment. The searches below use such filters on dense and
//!   sparse fields, float and byte, on the default codec and on the 10.4
//!   scalar-quantized format, and on a segment whose field has no vector; each
//!   must return Lucene's documents with Lucene's scores, bit for bit. The
//!   scorers are also driven directly, document by document and in bulk
//!   (`KnnScorerFixture exact|empty|sq`).
//! * **rmp #207 — acceptance asks about the document.** `KnnVectorValues
//!   .getAcceptOrds` reads `acceptDocs.get(ordToDoc(ord))`
//!   (`KnnVectorValues.java:75-91`), and every scorer built over a field's
//!   values forwards to it. On a sparse field a filter and the deletions must
//!   be honoured on the graph, on the scan, and on the quantized format, and
//!   the cost of asking must stay linear in the field.
//! * **rmp #208 — the Lucene 9.9 quantized read path.**
//!   `Lucene99ScalarQuantizedVectorScorer` and `ScalarQuantizedVectorScorer`
//!   score `LegacyQuantizedByteVectorValues` through their `FlatVectorsScorer`
//!   methods (`Lucene99ScalarQuantizedVectorScorer.java:50-80`,
//!   `ScalarQuantizedVectorScorer.java:61-102`). Lucene 10.5.0's core reaches
//!   that branch only from the 9.9 format's reader, which lives in
//!   `backward-codecs`; the production entry point it calls is
//!   `FlatVectorScorerUtil.getLucene99ScalarQuantizedVectorsScorer()`. The
//!   tests drive that entry point, and the `hnsw` scorer, with in-memory legacy
//!   values — unpacked, and four-bit packed behind a slice — and compare every
//!   score and every `ordToDoc` with Lucene's (`KnnScorerFixture legacy`).
//!
//! The oracle is run from source on the Lucene 10.5.0 jar (`java -cp
//! lucene-core-10.5.0.jar KnnScorerFixture.java <case>`).

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use rucene::codecs::hnsw::scalar_quantized_scorer::{
    ScalarQuantizedRandomVectorScorer, ScalarQuantizedVectorScorer,
};
use rucene::codecs::hnsw::{DefaultFlatVectorScorer, FlatVectorScorerUtil, FlatVectorsScorer};
use rucene::codecs::knn_vectors::KnnVectorsFormat;
use rucene::codecs::per_field::PerFieldKnnVectorsFormat;
use rucene::codecs::{
    register_codec, Codec, CompoundFormat, DocValuesFormat, FieldInfosFormat, LiveDocsFormat,
    Lucene104Codec, NormsFormat, PointsFormat, PostingsFormat, SegmentInfoFormat,
    StoredFieldsFormat, TermVectorsFormat,
};
use rucene::document::{
    Document, IntPoint, KnnByteVectorField, KnnFloatVectorField, Store, StringField,
};
use rucene::error::Result;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::merge_policy::NoMergePolicy;
use rucene::index::terms::Term;
use rucene::index::{
    directory_reader, DirectoryReader, DocAndFloatFeatureBuffer, IndexReader, LeafReader,
    VectorSimilarityFunction,
};
use rucene::internal::vectorization::lucene99_memory_segment_scalar_quantized_vector_scorer::Lucene99MemorySegmentScalarQuantizedVectorScorer;
use rucene::search::knn::{KnnCollector, TopKnnCollector};
use rucene::search::{
    from_iterator_supplier, from_live_docs, range, AcceptDocs, DocIdSetIterator, IndexSearcher,
    KnnByteVectorQuery, KnnFloatVectorQuery, Query, TermQuery, VectorScorer, NO_MORE_DOCS,
};
use rucene::store::mmap::MMapDirectory;
use rucene::store::{
    ByteBuffersDataInput, ByteBuffersIndexInput, Directory, FSDirectory, IOContext, IndexInput,
    ReadAdvice, DEFAULT_IO_CONTEXT,
};
use rucene::util::hnsw::RandomVectorScorer;
use rucene::util::quantization::{
    LegacyQuantizedByteVectorValues, QuantizedByteVectorValues, ScalarQuantizer,
};
use rucene::util::{Bits, BytesRef};

// ---------------------------------------------------------------------------
// The Lucene oracle
// ---------------------------------------------------------------------------

fn lucene_jar() -> PathBuf {
    let home = std::env::var("HOME").expect("HOME is set");
    let jar = Path::new(&home)
        .join(".m2/repository/org/apache/lucene/lucene-core/10.5.0/lucene-core-10.5.0.jar");
    assert!(
        jar.is_file(),
        "the Lucene 10.5.0 jar is required: {}",
        jar.display()
    );
    jar
}

/// Runs `KnnScorerFixture <args>` and returns the lines it printed whose first
/// word is one of `kinds`.
fn run_fixture(args: &[&str], kinds: &[&str]) -> String {
    run_fixture_with(&[], args, kinds)
}

/// As [`run_fixture`], with `jvm_args` before the class path.
fn run_fixture_with(jvm_args: &[&str], args: &[&str], kinds: &[&str]) -> String {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join(
        "tests/fixtures/java-codec-harness/src/main/java/org/apache/lucene/rucene/codec/\
         KnnScorerFixture.java",
    );
    let output = Command::new("java")
        .args(jvm_args)
        .arg("-cp")
        .arg(lucene_jar())
        .arg(&fixture)
        .args(args)
        .output()
        .expect("a JDK is required to run the Lucene oracle");
    assert!(
        output.status.success(),
        "KnnScorerFixture {args:?} failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("the fixture prints UTF-8")
        .lines()
        .filter(|line| {
            line.split(' ')
                .next()
                .is_some_and(|first| kinds.contains(&first))
        })
        .map(|line| format!("{line}\n"))
        .collect()
}

fn bits(score: f32) -> String {
    format!("{:x}", score.to_bits())
}

fn term(field: &str, text: &str) -> Term {
    Term::new(field, BytesRef::new(text.as_bytes().to_vec()))
}

fn term_query(field: &str, text: &str) -> Arc<dyn Query> {
    Arc::new(TermQuery::new(term(field, text)))
}

fn ensure_codec() {
    let _ = register_codec("Lucene104", Lucene104Codec::new());
}

fn writer_config() -> IndexWriterConfig {
    ensure_codec();
    let mut config = IndexWriterConfig::new();
    config
        .set_merge_policy(Arc::new(NoMergePolicy::new()))
        .unwrap();
    config
}

/// Runs `query` and prints its hits in the fixture's format.
fn search(out: &mut String, searcher: &IndexSearcher, name: &str, query: Arc<dyn Query>, n: i32) {
    let top_docs = searcher
        .search(query, n)
        .unwrap_or_else(|e| panic!("the kNN search {name} failed: {e:?}"));
    out.push_str(&format!("total {name} {}\n", top_docs.total_hits.value()));
    for hit in &top_docs.score_docs {
        out.push_str(&format!("hit {name} {} {}\n", hit.doc, bits(hit.score)));
    }
}

fn float_query(
    field: &str,
    target: &[f32],
    k: i32,
    filter: Option<Arc<dyn Query>>,
) -> Arc<dyn Query> {
    Arc::new(KnnFloatVectorQuery::with_filter(field, target.to_vec(), k, filter).unwrap())
}

fn byte_query(
    field: &str,
    target: &[u8],
    k: i32,
    filter: Option<Arc<dyn Query>>,
) -> Arc<dyn Query> {
    Arc::new(KnnByteVectorQuery::with_filter(field, target.to_vec(), k, filter).unwrap())
}

/// Prints what two scorers over the same values answer: the first document by
/// document, the second in bulk; mirrors `KnnScorerFixture.dump`.
fn dump(
    out: &mut String,
    leaf: i32,
    field: &str,
    one: Option<Box<dyn VectorScorer>>,
    bulk: Option<Box<dyn VectorScorer>>,
) {
    let (Some(mut one), Some(bulk)) = (one, bulk) else {
        out.push_str(&format!("noscorer {leaf} {field}\n"));
        return;
    };
    loop {
        let doc = one.iterator().next_doc().unwrap();
        if doc == NO_MORE_DOCS {
            break;
        }
        out.push_str(&format!(
            "score {leaf} {field} {doc} {}\n",
            bits(one.score().unwrap())
        ));
    }
    let mut bulk = bulk.bulk(None).unwrap();
    let mut buffer = DocAndFloatFeatureBuffer::new();
    loop {
        let max = bulk
            .next_docs_and_scores(NO_MORE_DOCS, None, &mut buffer)
            .unwrap();
        if buffer.size == 0 {
            break;
        }
        out.push_str(&format!("bulkmax {leaf} {field} {}\n", bits(max)));
        for i in 0..buffer.size {
            out.push_str(&format!(
                "bulk {leaf} {field} {} {}\n",
                buffer.docs[i],
                bits(buffer.features[i])
            ));
        }
    }
}

fn dump_float(out: &mut String, leaf: &Arc<dyn LeafReader>, ord: i32, field: &str, query: &[f32]) {
    let scorer = || match leaf.get_float_vector_values(field).unwrap() {
        Some(values) => values.scorer(query).unwrap(),
        None => None,
    };
    dump(out, ord, field, scorer(), scorer());
}

fn dump_byte(out: &mut String, leaf: &Arc<dyn LeafReader>, ord: i32, field: &str, query: &[u8]) {
    let scorer = || match leaf.get_byte_vector_values(field).unwrap() {
        Some(values) => values.scorer(query).unwrap(),
        None => None,
    };
    dump(out, ord, field, scorer(), scorer());
}

fn searcher_over(dir: &Arc<dyn Directory>) -> (Arc<dyn DirectoryReader>, IndexSearcher) {
    let reader = directory_reader::open(Arc::clone(dir)).unwrap();
    let searcher = IndexSearcher::new(Arc::clone(&reader) as Arc<dyn IndexReader>).unwrap();
    (reader, searcher)
}

/// `Lucene104Codec` with every vector field on
/// `Lucene104ScalarQuantizedVectorsFormat`, Lucene's way of choosing a vectors
/// format: an override of `getKnnVectorsFormatForField`, keeping the codec's
/// name, so a reader finds the format through the per-field attributes.
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

fn quantized_config() -> IndexWriterConfig {
    let mut config = writer_config();
    config.set_codec(Arc::new(QuantizedCodec::new())).unwrap();
    config
}

// ---------------------------------------------------------------------------
// rmp #221: exact search, default codec
// ---------------------------------------------------------------------------

/// The fields of document `i`, mirrored from `KnnScorerFixture.document`.
fn exact_document(i: i32) -> Document {
    let f = i as f32;
    let mut doc = Document::new();
    doc.add(Box::new(
        StringField::new(
            "tag",
            (if i % 2 == 0 { "even" } else { "odd" }).to_string(),
            Store::NO,
        )
        .unwrap(),
    ));
    doc.add(Box::new(
        KnnFloatVectorField::with_euclidean("f", &[f, 1.0 + 0.5 * f]).unwrap(),
    ));
    doc.add(Box::new(
        KnnFloatVectorField::new("c", &[1.0 + f, 2.0], VectorSimilarityFunction::COSINE).unwrap(),
    ));
    doc.add(Box::new(
        KnnFloatVectorField::new(
            "m",
            &[0.25 * f, 1.0 - 0.1 * f],
            VectorSimilarityFunction::MAXIMUM_INNER_PRODUCT,
        )
        .unwrap(),
    ));
    if i % 2 == 0 {
        doc.add(Box::new(
            KnnFloatVectorField::with_euclidean("s", &[f, 2.0 - f]).unwrap(),
        ));
    }
    doc.add(Box::new(
        KnnByteVectorField::with_euclidean("b", &[i as i8 as u8, (2 * i + 1) as i8 as u8]).unwrap(),
    ));
    doc.add(Box::new(
        KnnByteVectorField::new(
            "bd",
            &[(3 - i) as i8 as u8, (i + 1) as i8 as u8],
            VectorSimilarityFunction::DOT_PRODUCT,
        )
        .unwrap(),
    ));
    doc
}

/// Writes the six documents as two committed segments of three, as the
/// fixture does, and prints every search and every scorer dump.
fn rucene_exact() -> String {
    ensure_codec();
    let tmp = tempfile::tempdir().unwrap();
    let dir: Arc<dyn Directory> = Arc::new(FSDirectory::open(tmp.path()).unwrap());
    let writer = IndexWriter::new(Arc::clone(&dir), writer_config()).unwrap();
    for i in 0..6 {
        writer.add_document(&exact_document(i)).unwrap();
        if i % 3 == 2 {
            writer.commit().unwrap();
        }
    }
    writer.close().unwrap();

    let (reader, searcher) = searcher_over(&dir);
    assert_eq!(Arc::clone(&reader).leaves().len(), 2, "two segments");
    let mut out = String::new();
    let s = &mut out;
    search(
        s,
        &searcher,
        "f3",
        float_query("f", &[0.0, 1.0], 3, None),
        3,
    );
    search(
        s,
        &searcher,
        "f10",
        float_query("f", &[0.0, 1.0], 10, None),
        10,
    );
    search(
        s,
        &searcher,
        "c3",
        float_query("c", &[1.0, 1.0], 3, None),
        3,
    );
    search(
        s,
        &searcher,
        "m4",
        float_query("m", &[0.5, 1.0], 4, None),
        4,
    );
    search(
        s,
        &searcher,
        "s3",
        float_query("s", &[0.0, 1.0], 3, None),
        3,
    );
    search(s, &searcher, "b3", byte_query("b", &[0, 1], 3, None), 3);
    search(s, &searcher, "bd3", byte_query("bd", &[1, 2], 3, None), 3);
    let even = || Some(term_query("tag", "even"));
    let odd = || Some(term_query("tag", "odd"));
    search(
        s,
        &searcher,
        "f3e",
        float_query("f", &[0.0, 1.0], 3, even()),
        3,
    );
    search(
        s,
        &searcher,
        "c3o",
        float_query("c", &[1.0, 1.0], 3, odd()),
        3,
    );
    search(
        s,
        &searcher,
        "m4o",
        float_query("m", &[0.5, 1.0], 4, odd()),
        4,
    );
    search(
        s,
        &searcher,
        "s3e",
        float_query("s", &[0.0, 1.0], 3, even()),
        3,
    );
    search(s, &searcher, "b3e", byte_query("b", &[0, 1], 3, even()), 3);
    search(s, &searcher, "bd3o", byte_query("bd", &[1, 2], 3, odd()), 3);
    for leaf in Arc::clone(&reader).leaves() {
        let ord = leaf.ord();
        let leaf = leaf.leaf_reader();
        dump_float(s, &leaf, ord, "f", &[0.0, 1.0]);
        dump_float(s, &leaf, ord, "s", &[0.0, 1.0]);
        dump_byte(s, &leaf, ord, "b", &[0, 1]);
        dump_byte(s, &leaf, ord, "bd", &[1, 2]);
    }
    out
}

const SEARCH_KINDS: &[&str] = &["total", "hit"];
const DUMP_KINDS: &[&str] = &["total", "hit", "score", "bulk", "bulkmax", "noscorer"];

/// Every exact kNN search, float and byte, dense and sparse, and every direct
/// scorer and bulk scorer, returns Lucene's documents with Lucene's scores,
/// bit for bit.
#[test]
fn exact_knn_searches_and_scorers_match_lucene() {
    let tmp = tempfile::tempdir().unwrap();
    let index = tmp.path().join("exact");
    let java = run_fixture(&["exact", index.to_str().unwrap()], DUMP_KINDS);
    assert!(
        java.contains("hit bd3o "),
        "the fixture ran every search:\n{java}"
    );
    assert!(
        java.contains("bulk 1 s "),
        "the fixture dumped every scorer:\n{java}"
    );
    assert!(SEARCH_KINDS.iter().all(|kind| java.contains(kind)));
    let rust = rucene_exact();
    assert_eq!(
        rust, java,
        "Rucene (left) and Lucene 10.5.0 (right) disagree"
    );
}

// ---------------------------------------------------------------------------
// rmp #221: a segment whose field has no vector
// ---------------------------------------------------------------------------

/// The only document with field `e` is deleted and the segment force-merged,
/// so the merged segment lists `e` with no vector: the values are empty and
/// have no scorer, and an exact search finds nothing.
#[test]
fn a_field_with_no_vector_in_the_segment_has_no_scorer_as_in_lucene() {
    let tmp = tempfile::tempdir().unwrap();
    let java = run_fixture(
        &["empty", tmp.path().join("empty").to_str().unwrap()],
        &[
            "leaves", "esize", "total", "hit", "noscorer", "score", "bulk",
        ],
    );

    ensure_codec();
    let rtmp = tempfile::tempdir().unwrap();
    let dir: Arc<dyn Directory> = Arc::new(FSDirectory::open(rtmp.path()).unwrap());
    let writer = IndexWriter::new(Arc::clone(&dir), IndexWriterConfig::new()).unwrap();
    for i in 0..4 {
        let mut doc = Document::new();
        doc.add(Box::new(
            StringField::new("id", format!("doc-{i}"), Store::NO).unwrap(),
        ));
        doc.add(Box::new(
            KnnFloatVectorField::with_euclidean("f", &[i as f32, 1.0]).unwrap(),
        ));
        if i == 0 {
            doc.add(Box::new(
                KnnFloatVectorField::with_euclidean("e", &[1.0, 1.0]).unwrap(),
            ));
        }
        writer.add_document(&doc).unwrap();
    }
    writer.commit().unwrap();
    writer
        .delete_documents_by_terms(vec![term("id", "doc-0")])
        .unwrap();
    writer.force_merge(1).unwrap();
    writer.close().unwrap();

    let (reader, searcher) = searcher_over(&dir);
    let leaves = Arc::clone(&reader).leaves();
    let leaf = leaves[0].leaf_reader();
    let mut out = format!(
        "leaves {} maxDoc {} hasE {}\n",
        leaves.len(),
        reader.max_doc(),
        leaf.get_field_infos().field_info("e").is_some()
    );
    let size = match leaf.get_float_vector_values("e").unwrap() {
        Some(values) => values.size().to_string(),
        None => "null".to_string(),
    };
    out.push_str(&format!("esize {size}\n"));
    search(
        &mut out,
        &searcher,
        "e1",
        float_query("e", &[1.0, 1.0], 3, Some(term_query("id", "doc-1"))),
        3,
    );
    dump_float(&mut out, &leaf, 0, "e", &[1.0, 1.0]);
    assert!(java.contains("noscorer 0 e"), "{java}");
    assert_eq!(
        out, java,
        "Rucene (left) and Lucene 10.5.0 (right) disagree"
    );
}

// ---------------------------------------------------------------------------
// rmp #221 and #207: the 10.4 scalar-quantized format
// ---------------------------------------------------------------------------

const SQ_DOCS: i32 = 40;

/// Mirrors `KnnScorerFixture.sqHasQ`.
fn sq_has_q(d: i32) -> bool {
    (d * 7) % 5 < 3
}

/// One segment of 40 documents on the quantized format, two of them deleted:
/// a dense field `qd` and an irregularly sparse field `q`. Searches with no
/// filter, a broad filter (an exhaustive scan through the accepted ordinals)
/// and narrow filters (the exact search through the quantized scorer), plus
/// direct scorer dumps, all match Lucene.
#[test]
fn quantized_format_searches_and_scorers_match_lucene() {
    let tmp = tempfile::tempdir().unwrap();
    let java = run_fixture(&["sq", tmp.path().join("sq").to_str().unwrap()], DUMP_KINDS);
    assert!(java.contains("hit q5r "), "{java}");

    ensure_codec();
    let rtmp = tempfile::tempdir().unwrap();
    let dir: Arc<dyn Directory> = Arc::new(FSDirectory::open(rtmp.path()).unwrap());
    let writer = IndexWriter::new(Arc::clone(&dir), quantized_config()).unwrap();
    for d in 0..SQ_DOCS {
        let f = d as f32;
        let mut doc = Document::new();
        doc.add(Box::new(
            StringField::new("id", format!("doc-{d}"), Store::NO).unwrap(),
        ));
        doc.add(Box::new(
            StringField::new(
                "tag",
                (if d % 2 == 0 { "even" } else { "odd" }).to_string(),
                Store::NO,
            )
            .unwrap(),
        ));
        doc.add(Box::new(IntPoint::new("rank", &[d]).unwrap()));
        doc.add(Box::new(
            KnnFloatVectorField::with_euclidean("qd", &[f * 0.5, 3.0 - f * 0.25, 1.0]).unwrap(),
        ));
        if sq_has_q(d) {
            doc.add(Box::new(
                KnnFloatVectorField::with_euclidean("q", &[f, 0.5 * f, 2.0]).unwrap(),
            ));
        }
        writer.add_document(&doc).unwrap();
    }
    writer.commit().unwrap();
    writer
        .delete_documents_by_terms(vec![term("id", "doc-21"), term("id", "doc-30")])
        .unwrap();
    writer.commit().unwrap();
    writer.close().unwrap();

    let (reader, searcher) = searcher_over(&dir);
    let target = [20.0, 10.0, 2.0];
    let rank = |lo: i32, hi: i32| -> Option<Arc<dyn Query>> {
        Some(IntPoint::new_range_query("rank", &[lo], &[hi]).unwrap())
    };
    let mut out = String::new();
    let s = &mut out;
    search(s, &searcher, "q5", float_query("q", &target, 5, None), 5);
    search(
        s,
        &searcher,
        "q5e",
        float_query("q", &target, 5, Some(term_query("tag", "even"))),
        5,
    );
    search(
        s,
        &searcher,
        "q5r",
        float_query("q", &target, 5, rank(18, 23)),
        5,
    );
    search(
        s,
        &searcher,
        "qd4r",
        float_query("qd", &[5.0, 1.0, 1.0], 4, rank(8, 11)),
        4,
    );
    for leaf in Arc::clone(&reader).leaves() {
        let ord = leaf.ord();
        let leaf = leaf.leaf_reader();
        dump_float(s, &leaf, ord, "q", &target);
        dump_float(s, &leaf, ord, "qd", &[5.0, 1.0, 1.0]);
    }
    assert_eq!(
        out, java,
        "Rucene (left) and Lucene 10.5.0 (right) disagree"
    );
}

// ---------------------------------------------------------------------------
// rmp #207: acceptance is asked of the document, on a sparse field
// ---------------------------------------------------------------------------

const VECTOR_FIELD: &str = "vector";

/// Document `d` carries the vector `[d, 0]` when `d` is odd and no vector when
/// it is even, so ordinal `o` belongs to document `2 * o + 1` and the
/// neighbours of `[0, 0]` are documents 1, 3, 5, … in that order.
fn sparse_document(d: i32) -> Document {
    let mut document = Document::new();
    document.add(Box::new(
        StringField::new("id", format!("doc-{d}"), Store::NO).unwrap(),
    ));
    document.add(Box::new(IntPoint::new("rank", &[d]).unwrap()));
    if d % 2 == 1 {
        document.add(Box::new(
            KnnFloatVectorField::with_euclidean(VECTOR_FIELD, &[d as f32, 0.0]).unwrap(),
        ));
    }
    document
}

/// Writes `doc_count` sparse documents as one segment, optionally deleting the
/// documents whose rank lies in `deleted`.
fn sparse_index(
    doc_count: i32,
    deleted: Option<(i32, i32)>,
    config: IndexWriterConfig,
) -> (tempfile::TempDir, Arc<dyn DirectoryReader>) {
    ensure_codec();
    let tmp = tempfile::tempdir().unwrap();
    let dir: Arc<dyn Directory> = Arc::new(FSDirectory::open(tmp.path()).unwrap());
    let writer = IndexWriter::new(Arc::clone(&dir), config).unwrap();
    for d in 0..doc_count {
        writer.add_document(&sparse_document(d)).unwrap();
    }
    if let Some((lo, hi)) = deleted {
        writer.commit().unwrap();
        let query: Arc<dyn Query> = IntPoint::new_range_query("rank", &[lo], &[hi]).unwrap();
        writer.delete_documents_by_queries(vec![query]).unwrap();
    }
    writer.commit().unwrap();
    writer.close().unwrap();
    let reader = directory_reader::open(dir).unwrap();
    assert_eq!(Arc::clone(&reader).leaves().len(), 1, "one segment");
    (tmp, reader)
}

/// Runs a kNN search for `[0, 0]` over the one leaf of `reader`, honouring the
/// live documents and, when given, the documents `filter` accepts.
fn sparse_search(reader: &Arc<dyn DirectoryReader>, filter: Option<(i32, i32)>) -> Vec<i32> {
    let leaf: Arc<dyn LeafReader> = Arc::clone(reader).leaves()[0].leaf_reader();
    let mut accept_docs: Box<dyn AcceptDocs> = match filter {
        None => Box::new(from_live_docs(leaf.get_live_docs(), leaf.max_doc()).unwrap()),
        Some((lo, hi)) => Box::new(
            from_iterator_supplier(
                move || Ok(Box::new(range(lo, hi)?) as Box<dyn DocIdSetIterator>),
                leaf.get_live_docs(),
                leaf.max_doc(),
            )
            .unwrap(),
        ),
    };
    let mut collector = TopKnnCollector::new(3, i64::MAX);
    leaf.search_nearest_vectors(
        VECTOR_FIELD,
        &[0.0, 0.0],
        &mut collector,
        accept_docs.as_mut(),
    )
    .unwrap();
    collector
        .top_docs()
        .score_docs
        .iter()
        .map(|score_doc| score_doc.doc)
        .collect()
}

/// The length of the segment's HNSW graph file, `.vex`.
fn graph_file_length(dir: &Path) -> u64 {
    std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "vex"))
        .map(|entry| entry.metadata().unwrap().len())
        .sum()
}

/// The two sizes: 2000 documents give 1000 vectors, enough for
/// `Lucene99HnswVectorsWriter.shouldCreateGraph` (`numNodes > log(numNodes) *
/// 100`), so the search runs on the graph; 200 documents give 100 vectors and
/// no graph, so it scans every vector.
const GRAPH_DOCS: i32 = 2_000;
const SCAN_DOCS: i32 = 200;

/// The 2000-document segment stores a graph and the 200-document one does not,
/// so the sparse tests below cover both search paths.
#[test]
fn the_sparse_sizes_cover_the_graph_and_the_scan() {
    // Separate files, so the graph file can be measured.
    let separate_files = || {
        let mut config = writer_config();
        config.set_use_compound_file(false).unwrap();
        config
    };
    let (graph_dir, _r1) = sparse_index(GRAPH_DOCS, None, separate_files());
    let (scan_dir, _r2) = sparse_index(SCAN_DOCS, None, separate_files());
    let with_graph = graph_file_length(graph_dir.path());
    let without_graph = graph_file_length(scan_dir.path());
    // A `.vex` with no graph holds only its codec header and footer.
    eprintln!(".vex: graph case {with_graph} bytes, scan case {without_graph} bytes");
    assert!(
        without_graph > 0 && without_graph < 200,
        ".vex of the scan case is {without_graph} bytes"
    );
    assert!(
        with_graph > 10 * without_graph,
        ".vex of the graph case is {with_graph} bytes, of the scan case {without_graph}"
    );
}

/// A caller's filter on documents is honoured on a sparse field: the search
/// returns the nearest *documents* the filter accepts.
#[test]
fn a_filter_is_honoured_on_a_sparse_field() {
    for doc_count in [GRAPH_DOCS, SCAN_DOCS] {
        let (_tmp, reader) = sparse_index(doc_count, None, writer_config());
        let half = doc_count / 2;
        assert_eq!(
            sparse_search(&reader, Some((half, doc_count))),
            vec![half + 1, half + 3, half + 5],
            "{doc_count} documents, filter {half}.."
        );
    }
}

/// Deleted documents are not returned from a sparse field, and live documents
/// are not hidden because their ordinal equals a deleted document's id.
#[test]
fn deletions_are_honoured_on_a_sparse_field() {
    for doc_count in [GRAPH_DOCS, SCAN_DOCS] {
        let (_tmp, reader) = sparse_index(doc_count, Some((0, 9)), writer_config());
        assert_eq!(
            sparse_search(&reader, None),
            vec![11, 13, 15],
            "{doc_count} documents, documents 0..=9 deleted"
        );
    }
}

/// A filter and deletions together: the filter accepts documents `half..`,
/// and documents `half..=half + 9` are deleted.
#[test]
fn a_filter_and_deletions_together_are_honoured_on_a_sparse_field() {
    for doc_count in [GRAPH_DOCS, SCAN_DOCS] {
        let half = doc_count / 2;
        let (_tmp, reader) = sparse_index(doc_count, Some((half, half + 9)), writer_config());
        assert_eq!(
            sparse_search(&reader, Some((half, doc_count))),
            vec![half + 11, half + 13, half + 15],
            "{doc_count} documents"
        );
    }
}

/// The same filter and deletions on the 10.4 quantized format, whose reader
/// scans every accepted ordinal of the quantized vectors.
#[test]
fn a_filter_and_deletions_are_honoured_on_a_quantized_sparse_field() {
    let half = SCAN_DOCS / 2;
    let (_tmp, reader) = sparse_index(SCAN_DOCS, Some((half, half + 9)), quantized_config());
    assert_eq!(
        sparse_search(&reader, Some((half, SCAN_DOCS))),
        vec![half + 11, half + 13, half + 15]
    );
    let (_tmp, reader) = sparse_index(SCAN_DOCS, None, quantized_config());
    assert_eq!(
        sparse_search(&reader, Some((half, SCAN_DOCS))),
        vec![half + 1, half + 3, half + 5]
    );
}

// ---------------------------------------------------------------------------
// rmp #207: asking about the document stays linear in the field
// ---------------------------------------------------------------------------

/// Whether document `d` has a vector in the scaling index: runs of 512
/// documents with vectors alternate with runs of 512 without, thinned by a
/// multiplicative hash, so the ordinal-to-document mapping drifts from its
/// linear fit and needs real bits per value (the `2 * ord + 1` pattern encodes
/// at zero bits).
fn scaling_has_vector(d: i32) -> bool {
    (d >> 9) & 1 == 0 && (u64::from(d as u32).wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 61) < 6
}

/// How many ordinal-to-document readers one filtered search and one sweep of
/// the accepted-ordinals view build, on an index of `doc_count` documents.
///
/// The search runs on the quantized format, whose reader scans every ordinal
/// through the scorer's accepted ordinals; the sweep asks the raw values'
/// view about every ordinal. Counting reader constructions on this thread
/// (`util::packed::monotonic_reader_constructions`) is deterministic: Lucene
/// builds the reader once per values object (`OffHeapFloatVectorValues.java:225`,
/// `OffHeapScalarQuantizedVectorValues.java:377`), so the count must not
/// depend on how many ordinals are asked about.
fn readers_built_per_search(doc_count: i32) -> (u64, u64, usize) {
    use rucene::util::packed::monotonic_reader_constructions;
    ensure_codec();
    let tmp = tempfile::tempdir().unwrap();
    let dir: Arc<dyn Directory> = Arc::new(FSDirectory::open(tmp.path()).unwrap());
    let writer = IndexWriter::new(Arc::clone(&dir), quantized_config()).unwrap();
    for d in 0..doc_count {
        let mut doc = Document::new();
        if scaling_has_vector(d) {
            doc.add(Box::new(
                KnnFloatVectorField::with_euclidean(VECTOR_FIELD, &[d as f32, 0.0]).unwrap(),
            ));
        }
        writer.add_document(&doc).unwrap();
    }
    writer.commit().unwrap();
    writer.close().unwrap();
    let reader = directory_reader::open(dir).unwrap();
    let leaf: Arc<dyn LeafReader> = Arc::clone(&reader).leaves()[0].leaf_reader();
    let max_doc = leaf.max_doc();
    // 32 documents inside the first run that has vectors.
    let (lo, hi) = (64, 96);

    let before_search = monotonic_reader_constructions();
    let mut accept_docs = from_iterator_supplier(
        move || Ok(Box::new(range(lo, hi)?) as Box<dyn DocIdSetIterator>),
        None,
        max_doc,
    )
    .unwrap();
    let mut collector = TopKnnCollector::new(3, i64::MAX);
    leaf.search_nearest_vectors(VECTOR_FIELD, &[0.0, 0.0], &mut collector, &mut accept_docs)
        .unwrap();
    let search = monotonic_reader_constructions() - before_search;
    let docs = collector.top_docs().score_docs;
    assert!(!docs.is_empty() && docs.iter().all(|hit| (lo..hi).contains(&hit.doc)));

    let values = leaf.get_float_vector_values(VECTOR_FIELD).unwrap().unwrap();
    let filter = rucene::util::MatchAllBits::new(max_doc as usize);
    let before_sweep = monotonic_reader_constructions();
    let view = values.get_accept_ords(Some(&filter)).unwrap();
    let accepted = (0..values.size() as usize)
        .filter(|&ord| view.get(ord))
        .count();
    assert_eq!(accepted, values.size() as usize);
    let sweep = monotonic_reader_constructions() - before_sweep;
    (search, sweep, values.size() as usize)
}

/// Asking about documents costs a constant number of reader constructions per
/// search, whatever the size of the field: a filtered search opens its values
/// once, and sweeping the accepted-ordinals view over every ordinal builds no
/// reader at all. A reader rebuilt per lookup would make both counts grow
/// with the number of ordinals.
#[test]
fn asking_about_documents_builds_no_reader_per_ordinal() {
    let (small_search, small_sweep, small_size) = readers_built_per_search(4_096);
    let (large_search, large_sweep, large_size) = readers_built_per_search(16_384);
    assert!(
        large_size > 3 * small_size,
        "{small_size} -> {large_size} vectors"
    );
    assert_eq!(
        small_search, large_search,
        "readers built by one filtered search: {small_search} for {small_size} vectors, \
         {large_search} for {large_size}"
    );
    assert!(small_search <= 2, "one search built {small_search} readers");
    assert_eq!(
        (small_sweep, large_sweep),
        (0, 0),
        "sweeping the accepted-ordinals view builds no reader"
    );
}

// ---------------------------------------------------------------------------
// rmp #208: the Lucene 9.9 quantized read path
// ---------------------------------------------------------------------------

const DIMENSION: usize = 4;
const SIZE: usize = 5;
const QUERY: [f32; DIMENSION] = [0.75, -0.5, 1.25, 0.25];

/// Mirrors `KnnScorerFixture.component`.
fn component(ord: usize, j: usize) -> f32 {
    ((ord * 7 + j * 3) % 9) as f32 / 2.0 - 1.0
}

/// Quantized vectors held in memory, counting how many vectors are read;
/// ordinal `ord` belongs to document `2 * ord + 1`, as in the fixture. When
/// `slice` is present the four-bit components are also stored packed two to a
/// byte behind it, each vector followed by its correction, as the 9.9 format
/// stores them.
struct MemoryValues {
    quantizer: ScalarQuantizer,
    vectors: Arc<Vec<Vec<u8>>>,
    offsets: Arc<Vec<f32>>,
    reads: Arc<AtomicUsize>,
    slice: Option<Box<dyn IndexInput>>,
    /// Whether the slice holds the components packed two to a byte.
    packed: bool,
}

impl MemoryValues {
    fn new(bits: u8, similarity: VectorSimilarityFunction) -> Self {
        let quantizer = ScalarQuantizer::new(-1.0, 3.0, bits).unwrap();
        let mut vectors = Vec::new();
        let mut offsets = Vec::new();
        for ord in 0..SIZE {
            let raw: Vec<f32> = (0..DIMENSION).map(|j| component(ord, j)).collect();
            let mut quantized = vec![0u8; DIMENSION];
            offsets.push(
                quantizer
                    .quantize(&raw, &mut quantized, similarity)
                    .unwrap(),
            );
            vectors.push(quantized);
        }
        Self {
            quantizer,
            vectors: Arc::new(vectors),
            offsets: Arc::new(offsets),
            reads: Arc::new(AtomicUsize::new(0)),
            slice: None,
            packed: false,
        }
    }

    /// The same values, packed behind a slice as `KnnScorerFixture` packs them.
    fn packed(mut self) -> Self {
        let half = (DIMENSION + 1) >> 1;
        let mut bytes = Vec::new();
        for ord in 0..SIZE {
            for i in 0..half {
                bytes.push((self.vectors[ord][i] << 4) | self.vectors[ord][half + i]);
            }
            bytes.extend_from_slice(&self.offsets[ord].to_bits().to_le_bytes());
        }
        let input = ByteBuffersDataInput::new(vec![bytes]).unwrap();
        self.slice = Some(Box::new(ByteBuffersIndexInput::new(input, "compressed")));
        self.packed = true;
        self
    }

    fn duplicate(&self) -> Self {
        Self {
            quantizer: self.quantizer,
            vectors: Arc::clone(&self.vectors),
            offsets: Arc::clone(&self.offsets),
            reads: Arc::clone(&self.reads),
            slice: self
                .slice
                .as_ref()
                .map(|slice| slice.clone_input().unwrap()),
            packed: self.packed,
        }
    }
}

impl QuantizedByteVectorValues for MemoryValues {
    fn vector_value(&mut self, ord: i32) -> Result<&[u8]> {
        self.reads.fetch_add(1, Ordering::Relaxed);
        Ok(&self.vectors[ord as usize])
    }

    fn get_score_correction_constant(&mut self, ord: i32) -> Result<f32> {
        Ok(self.offsets[ord as usize])
    }

    fn size(&self) -> i32 {
        SIZE as i32
    }

    fn dimension(&self) -> i32 {
        DIMENSION as i32
    }

    fn ord_to_doc(&self, ord: i32) -> i32 {
        2 * ord + 1
    }

    fn get_accept_ords<'a>(&self, accept_docs: Option<&'a dyn Bits>) -> Option<Box<dyn Bits + 'a>> {
        rucene::index::accept_ords_with(SIZE as i32, |ord| 2 * ord + 1, accept_docs)
    }

    fn copy(&self) -> Result<Box<dyn QuantizedByteVectorValues>> {
        Ok(Box::new(self.duplicate()))
    }

    fn vector_byte_length(&self) -> i32 {
        if self.packed {
            ((DIMENSION + 1) >> 1) as i32
        } else {
            DIMENSION as i32
        }
    }
}

impl LegacyQuantizedByteVectorValues for MemoryValues {
    fn scalar_quantizer(&self) -> &ScalarQuantizer {
        &self.quantizer
    }

    fn copy_legacy(&self) -> Result<Box<dyn LegacyQuantizedByteVectorValues>> {
        Ok(Box::new(self.duplicate()))
    }

    fn get_slice(&self) -> Option<&dyn IndexInput> {
        self.slice.as_deref()
    }

    fn get_slice_mut(&mut self) -> Option<&mut dyn IndexInput> {
        match &mut self.slice {
            Some(slice) => Some(slice.as_mut()),
            None => None,
        }
    }
}

const SIMILARITIES: [(VectorSimilarityFunction, &str); 4] = [
    (VectorSimilarityFunction::EUCLIDEAN, "EUCLIDEAN"),
    (VectorSimilarityFunction::DOT_PRODUCT, "DOT_PRODUCT"),
    (VectorSimilarityFunction::COSINE, "COSINE"),
    (
        VectorSimilarityFunction::MAXIMUM_INNER_PRODUCT,
        "MAXIMUM_INNER_PRODUCT",
    ),
];

/// Scores `values` through `scorer` and prints what the fixture prints.
fn score_legacy(
    out: &mut String,
    name: &str,
    prefix: &str,
    bits: u8,
    similarity: (VectorSimilarityFunction, &str),
    scorer: &dyn FlatVectorsScorer,
    values: MemoryValues,
) {
    let (similarity, similarity_name) = similarity;
    let mut query = scorer
        .get_random_vector_scorer_legacy_quantized(similarity, Box::new(values.duplicate()), &QUERY)
        .unwrap();
    for ord in 0..SIZE as i32 {
        let score = query.score(ord).unwrap();
        out.push_str(&format!(
            "{name} {prefix}query {bits} {similarity_name} {ord} {} {}\n",
            query.ord_to_doc(ord),
            self::bits(score)
        ));
    }
    let mut supplied = scorer
        .get_random_vector_scorer_supplier_legacy_quantized(similarity, Box::new(values))
        .unwrap()
        .scorer()
        .unwrap();
    supplied.set_scoring_ordinal(0).unwrap();
    for ord in 0..SIZE as i32 {
        let score = supplied.score(ord).unwrap();
        out.push_str(&format!(
            "{name} {prefix}supplier {bits} {similarity_name} {ord} {} {}\n",
            supplied.ord_to_doc(ord),
            self::bits(score)
        ));
    }
}

/// Scores the fixture's legacy values through `scorer`, printing the lines
/// `KnnScorerFixture legacy` prints for it.
fn rucene_legacy_scores(name: &str, scorer: &dyn FlatVectorsScorer) -> String {
    let mut out = String::new();
    for bits in [7u8, 4] {
        for similarity in SIMILARITIES {
            let values = MemoryValues::new(bits, similarity.0);
            score_legacy(
                &mut out,
                name,
                "",
                bits,
                similarity,
                scorer,
                values.duplicate(),
            );
            if bits == 4 {
                score_legacy(
                    &mut out,
                    name,
                    "c",
                    bits,
                    similarity,
                    scorer,
                    values.packed(),
                );
            }
        }
    }
    out
}

/// The production entry point of the 9.9 quantized read path,
/// `FlatVectorScorerUtil::get_lucene99_scalar_quantized_vectors_scorer`, and
/// `ScalarQuantizedVectorScorer` over `DefaultFlatVectorScorer`, score legacy
/// quantized vectors exactly as Lucene does: every similarity, seven and four
/// bits, unpacked and packed behind a slice (`CompressedInt4DotProduct`),
/// through both the query scorer and the supplier, and map every ordinal to
/// Lucene's document.
#[test]
fn the_lucene99_quantized_read_path_scores_as_lucene() {
    let java = run_fixture(&["legacy"], &["lucene99", "hnsw"]);
    assert_eq!(
        java.lines().count(),
        240,
        "the fixture scored every case:\n{java}"
    );

    let lucene99 = FlatVectorScorerUtil::get_lucene99_scalar_quantized_vectors_scorer();
    let hnsw = ScalarQuantizedVectorScorer::new(Box::new(DefaultFlatVectorScorer::INSTANCE));
    let mut rust = rucene_legacy_scores("lucene99", &lucene99);
    rust.push_str(&rucene_legacy_scores("hnsw", &hnsw));
    assert_eq!(
        rust, java,
        "Rucene (left) and Lucene 10.5.0 (right) disagree"
    );
}

/// The nodes the 9.9 format stores for `values`: each quantized vector —
/// packed two components to a byte when `packed` — followed by its correction.
fn node_bytes(values: &MemoryValues, packed: bool) -> Vec<u8> {
    let half = (DIMENSION + 1) >> 1;
    let mut bytes = Vec::new();
    for ord in 0..SIZE {
        if packed {
            for i in 0..half {
                bytes.push((values.vectors[ord][i] << 4) | values.vectors[ord][half + i]);
            }
        } else {
            bytes.extend_from_slice(&values.vectors[ord]);
        }
        bytes.extend_from_slice(&values.offsets[ord].to_bits().to_le_bytes());
    }
    bytes
}

/// When the values' slice is a memory-mapped input,
/// `Lucene99MemorySegmentScalarQuantizedVectorScorer` scores straight from
/// the mapped nodes, as Java's `instanceof MemorySegmentAccessInput` branch
/// does (`Lucene99MemorySegmentScalarQuantizedVectorScorer.java:50-69`): no
/// vector is read through the values, and every score and `ordToDoc` is
/// Lucene's.
#[test]
fn the_memory_segment_quantized_path_reads_the_mapped_nodes() {
    // The oracle maps the same nodes with `MMapDirectory` and enables the
    // vector API, so `FlatVectorScorerUtil` answers
    // `Lucene99MemorySegmentScalarQuantizedVectorScorer` and its
    // memory-segment branch scores them.
    let java_dir = tempfile::tempdir().unwrap();
    let java = run_fixture_with(
        &["--add-modules", "jdk.incubator.vector"],
        &["legacy-mmap", java_dir.path().to_str().unwrap()],
        &["lucene99", "scorer"],
    );
    assert!(
        java.starts_with("scorer Lucene99MemorySegmentScalarQuantizedVectorScorer()\n"),
        "the oracle took the memory-segment scorer:\n{java}"
    );
    let java: String = java
        .lines()
        .skip(1)
        .map(|line| format!("{line}\n"))
        .collect();
    let tmp = tempfile::tempdir().unwrap();
    // The provider Lucene's `MMapDirectory` delegates to is used directly,
    // so that each file is opened with the exact chunk size and read advice
    // the oracle used.
    let provider = MMapDirectory::lookup_provider();
    let arenas = provider.attachment();
    let scorer = Lucene99MemorySegmentScalarQuantizedVectorScorer::INSTANCE;
    let mut rust = String::new();
    let mut file = 0;
    for bits in [7u8, 4] {
        for similarity in SIMILARITIES {
            let packings: &[bool] = if bits == 4 { &[false, true] } else { &[false] };
            for &packed in packings {
                let mut values = MemoryValues::new(bits, similarity.0);
                let name = format!("nodes{file}");
                file += 1;
                std::fs::write(tmp.path().join(&name), node_bytes(&values, packed)).unwrap();
                let input = provider
                    .open_input(
                        &tmp.path().join(&name),
                        30,
                        &*DEFAULT_IO_CONTEXT,
                        Arc::new(|_: &dyn IOContext| ReadAdvice::Normal),
                        false,
                        false,
                        None,
                        &arenas,
                    )
                    .unwrap();
                assert!(
                    input.as_memory_segment_access_input().is_some(),
                    "a mapped input"
                );
                values.slice = Some(Box::new(input));
                values.packed = packed;
                let reads = Arc::clone(&values.reads);
                let prefix = if packed { "c" } else { "" };
                score_legacy(
                    &mut rust, "lucene99", prefix, bits, similarity, &scorer, values,
                );
                assert_eq!(
                    reads.load(Ordering::Relaxed),
                    0,
                    "the mapped path reads no vector through the values"
                );
            }
        }
    }
    assert_eq!(
        rust, java,
        "Rucene (left) and Lucene 10.5.0 (right) disagree"
    );
}

/// A [`Bits`] accepting exactly one document.
#[derive(Debug)]
struct OneDocument {
    doc: usize,
    length: usize,
}

impl Bits for OneDocument {
    fn get(&self, index: usize) -> bool {
        index == self.doc
    }

    fn length(&self) -> usize {
        self.length
    }
}

/// The query scorer of the quantized branch reads each stored vector only when
/// asked to score it, and answers `ordToDoc` and `getAcceptOrds` from the
/// values, as Java's `AbstractRandomVectorScorer` does
/// (`ScalarQuantizedVectorScorer.java:90-98`, `RandomVectorScorer.java:94-107`).
#[test]
fn the_quantized_query_scorer_reads_lazily_and_maps_through_the_values() {
    let values = MemoryValues::new(7, VectorSimilarityFunction::DOT_PRODUCT);
    let reads = Arc::clone(&values.reads);
    let mut scorer = ScalarQuantizedRandomVectorScorer::new(
        VectorSimilarityFunction::DOT_PRODUCT,
        Box::new(values),
        &QUERY,
    )
    .unwrap();
    assert_eq!(
        reads.load(Ordering::Relaxed),
        0,
        "building the scorer reads no vector"
    );
    scorer.score(3).unwrap();
    assert_eq!(
        reads.load(Ordering::Relaxed),
        1,
        "scoring one ordinal reads one vector"
    );

    assert_eq!(scorer.max_ord(), SIZE as i32);
    assert_eq!(scorer.ord_to_doc(2), 5, "ordinal 2 belongs to document 5");
    assert!(scorer.get_accept_ords(None).is_none());
    let accepted = OneDocument { doc: 5, length: 11 };
    let accept_ords = scorer
        .get_accept_ords(Some(&accepted))
        .expect("a filter yields a view");
    let accepted_ords: Vec<usize> = (0..SIZE).filter(|&ord| accept_ords.get(ord)).collect();
    assert_eq!(
        accepted_ords,
        vec![2],
        "only the ordinal of document 5 is accepted"
    );
    assert_eq!(accept_ords.length(), SIZE);
    assert_eq!(reads.load(Ordering::Relaxed), 1, "the view reads no vector");
}
