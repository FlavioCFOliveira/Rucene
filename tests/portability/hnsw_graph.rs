//! Portability tests for `HnswGraphProvider.getGraph`.
//!
//! `getGraph` is the public door onto the `.vex` file: it turns the bytes the
//! HNSW writer laid down into the graph a merge reuses and a search walks. This
//! suite therefore asserts the **decoded structure**, not only the bytes — the
//! level count, the entry node, the node count, and every node's neighbour list
//! on every level — from an index the Java harness wrote.
//!
//! Reading Java's own segment is the whole point. Comparing two written graphs
//! would prove nothing here, because `HnswGraphBuilder` seeds its randomness and
//! two implementations can build different (equally valid) graphs from the same
//! vectors; what must agree is how each side *reads* a given `.vex`.
//!
//! Three cases:
//!
//! 1. **a segment above the tiny-segment threshold** — 1000 vectors, which is
//!    where `shouldCreateGraph(100, n)` first says yes, so a real multi-level
//!    graph exists to decode;
//! 2. **a segment below it** — 40 vectors, where Lucene writes no graph at all
//!    and `getGraph` answers `HnswGraph.EMPTY`. A port that returned a graph
//!    over an empty region, or that failed, would diverge on every small
//!    segment;
//! 3. **an unknown field** — which Java answers with `IllegalArgumentException`.
//!
//! A missing Maven or JDK is a hard failure, not a skip.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;

use rucene::codecs::hnsw::graph_provider::HnswGraphProvider;
use rucene::codecs::knn_vectors::KnnVectorsFormat;
use rucene::codecs::lucene99::hnsw_vectors_format::{
    Lucene99HnswVectorsFormat, Lucene99HnswVectorsReader,
};
use rucene::codecs::state::SegmentReadState;
use rucene::codecs::{register_codec, Codec, Lucene104Codec};
use rucene::error::LuceneError;
use rucene::index::{FieldInfo, FieldInfos, SegmentInfo, VectorEncoding, VectorSimilarityFunction};
use rucene::search::NO_MORE_DOCS;
use rucene::store::{Directory, FSDirectory, DEFAULT_IO_CONTEXT};
use rucene::util::hnsw::NodesIterator;
use rucene::util::Version;

const SEGMENT: &str = "_hg";
const SEGMENT_ID: [u8; 16] = [
    0x48, 0x4e, 0x53, 0x57, 0x47, 0x52, 0x41, 0x50, 0x48, 0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06,
];
const FIELD: &str = "vec";

fn codec() -> Arc<dyn Codec> {
    let _ = register_codec("Lucene104", Lucene104Codec::new());
    rucene::codecs::default_codec().expect("Lucene104 codec is registered")
}

fn field_infos(dim: i32) -> FieldInfos {
    let mut info = FieldInfo::new(FIELD, 0);
    info.vector_dimension = dim;
    info.vector_encoding = VectorEncoding::FLOAT32;
    info.vector_similarity_function = VectorSimilarityFunction::EUCLIDEAN;
    FieldInfos::new(vec![info]).expect("field infos")
}

fn segment_info(directory: &Arc<dyn Directory>, num_docs: i32) -> SegmentInfo {
    SegmentInfo::new(
        Arc::clone(directory),
        Version::LATEST,
        Some(Version::LATEST),
        SEGMENT.to_string(),
        num_docs,
        false,
        false,
        codec(),
        HashMap::new(),
        SEGMENT_ID,
        HashMap::new(),
        Default::default(),
    )
    .expect("segment info")
}

fn open_reader(dir: &Path, num_docs: i32, dim: i32) -> Lucene99HnswVectorsReader {
    let directory: Arc<dyn Directory> =
        Arc::new(FSDirectory::open(dir).expect("open the directory"));
    let info = segment_info(&directory, num_docs);
    let infos = field_infos(dim);
    let read_state = SegmentReadState::new(directory.as_ref(), &info, &infos, &*DEFAULT_IO_CONTEXT);
    let format = Lucene99HnswVectorsFormat::new();
    let flat = rucene::codecs::hnsw::flat_vectors::FlatVectorsFormat::fields_reader_flat(
        &rucene::codecs::lucene99::flat_vectors_format::Lucene99FlatVectorsFormat::new(
            rucene::codecs::hnsw::flat_vectors::FlatVectorScorerUtil::get_lucene99_flat_vectors_scorer(),
        ),
        &read_state,
    )
    .expect("flat reader");
    let _ = &format;
    Lucene99HnswVectorsReader::new(&read_state, flat).expect("hnsw reader")
}

/// Dumps a graph in the same `key=value` / `nbr=level:node:neighbours` shape the
/// Java fixture prints.
fn dump_graph(reader: &Lucene99HnswVectorsReader) -> Vec<String> {
    let mut graph = reader.get_graph(FIELD).expect("get graph");
    let mut out = vec![
        format!("size={}", graph.size()),
        format!("num_levels={}", graph.num_levels().expect("num levels")),
        format!("entry_node={}", graph.entry_node().expect("entry node")),
        format!("max_conn={}", graph.max_conn()),
    ];
    let levels = graph.num_levels().expect("num levels");
    for level in 0..levels {
        let mut nodes = Vec::new();
        let mut it = graph.get_nodes_on_level(level).expect("nodes on level");
        while it.has_next() {
            nodes.push(it.next_int());
        }
        for node in nodes {
            graph.seek(level, node).expect("seek");
            let mut neighbours = Vec::new();
            loop {
                let n = graph.next_neighbor().expect("next neighbour");
                if n == NO_MORE_DOCS {
                    break;
                }
                neighbours.push(n.to_string());
            }
            out.push(format!("nbr={level}:{node}:{}", neighbours.join(",")));
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Java harness
// ---------------------------------------------------------------------------

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

fn run_java(out_dir: &Path, num_docs: i32, dim: i32) -> Vec<String> {
    let mvn = which_mvn()
        .unwrap_or_else(|e| panic!("the HNSW-graph portability test needs Maven and a JDK: {e}"));
    let output = Command::new(mvn)
        .arg("-q")
        .arg("compile")
        .arg("exec:java")
        .arg("-Dexec.mainClass=org.apache.lucene.rucene.codec.HnswGraphProviderFixture")
        .arg(format!(
            "-Dexec.args={} {num_docs} {dim}",
            out_dir.display()
        ))
        .current_dir(harness_dir())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn Maven: {e}"));
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        output.status.success() && stdout.lines().any(|l| l.trim() == "read_ok=true"),
        "the Java fixture failed:\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    stdout
        .lines()
        .map(str::trim)
        .filter(|l| {
            l.starts_with("size=")
                || l.starts_with("num_levels=")
                || l.starts_with("entry_node=")
                || l.starts_with("max_conn=")
                || l.starts_with("nbr=")
        })
        .map(str::to_string)
        .collect()
}

#[test]
fn get_graph_decodes_lucenes_own_vex() {
    let root = std::env::temp_dir().join(format!("rucene-hnsw-graph-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);

    // 1. and 2. — above and below the tiny-segment threshold.
    for (num_docs, dim) in [(1000, 4), (40, 4)] {
        let dir = root.join(format!("n{num_docs}"));
        std::fs::create_dir_all(&dir).expect("case dir");
        let java = run_java(&dir, num_docs, dim);
        let reader = open_reader(&dir, num_docs, dim);
        let rust = dump_graph(&reader);
        assert_eq!(
            java, rust,
            "the graph Rucene decodes from Lucene's own .vex must be Lucene's, for {num_docs} docs"
        );
    }

    // 3. — an unknown field is an IllegalArgumentException on both sides.
    let dir = root.join("n40");
    let reader = open_reader(&dir, 40, 4);
    match reader.get_graph("absent") {
        Err(LuceneError::IllegalArgument(message)) => {
            assert!(
                message.contains("absent"),
                "the message names the missing field, got {message}"
            );
        }
        Err(other) => panic!("an unknown field must be an IllegalArgument, got {other:?}"),
        Ok(_) => panic!("an unknown field must not yield a graph"),
    }

    let _ = std::fs::remove_dir_all(&root);
}
