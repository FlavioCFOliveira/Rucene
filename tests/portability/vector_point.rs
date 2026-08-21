//! Portability tests for the vector and point value abstractions against
//! Apache Lucene Core 10.5.0.
//!
//! Three Java fixtures under `tests/fixtures/java-codec-harness` produce the
//! reference behaviour, and every test here re-computes the same thing with
//! Rucene and compares:
//!
//! * `VectorSimilarityFixture` — `VectorUtil` and `VectorSimilarityFunction`
//!   results, compared **bit for bit** through `Float.floatToRawIntBits`. This
//!   is what proves that Rucene reproduces Lucene's accumulation order, its
//!   `f64` division-and-square-root in `cosine`, and its exact `i32`
//!   accumulation for byte vectors, including dimensions past the 2^24 boundary
//!   where an `f32` accumulator would silently diverge.
//! * `PointTreeFixture` — the geometry of a real BKD tree and the full
//!   `intersect` call trace, compared against the same traversal run over
//!   Rucene's in-memory reference `PointTree`.
//! * `VectorValuesIteratorFixture` — the `(docID, index())` sequences of the
//!   three `KnnVectorValues.DocIndexIterator` factories.
//!
//! # What the point-tree comparison does and does not prove
//!
//! Task #89 ports the *algorithms* (`intersect`, `estimatePointCount`,
//! `estimateDocCount`) but not the BKD-backed `PointTree`, which is task #119.
//! So the leaf contents come from Java (they are index-format data) and the
//! traversal is Rucene's. That split means:
//!
//! * `ONE_LEAF_1D` — the full call trace is compared. A single-leaf BKD tree
//!   has a root that is also its only leaf, which the in-memory tree reproduces
//!   exactly.
//! * `ONE_LEAF_2D` — only the aggregates, the accepted documents and the
//!   estimates are compared. For `numIndexDims != 1` `BKDReader` issues an
//!   extra `compare` against the leaf's stored (narrowed) bounds before
//!   visiting; that refinement belongs to the BKD cursor and lands with #119.
//! * `MULTI_LEAF_1D` — only `size`, `getDocCount`, `getMinPackedValue` and
//!   `getMaxPackedValue` are compared. The internal cell bounds of a
//!   multi-level BKD tree come from its split values, which are not observable
//!   without the BKD cursor; inventing a matching geometry here would prove
//!   nothing.
//!
//! A missing Maven or JDK is a hard failure, not a skip: a portability test has
//! nothing to assert without the reference implementation, so skipping would
//! report success while proving nothing. This matches
//! `tests/portability/codecs.rs` and `tests/portability/indexing_chain.rs`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;

use base64::engine::general_purpose::STANDARD;
use base64::Engine;

use rucene::index::point_values::{InMemoryPointValues, IntersectVisitor, PointValues, Relation};
use rucene::index::vector_values::{
    DenseDocIndexIterator, DocIndexIterator, FromDisiDocIndexIterator, SparseDocIndexIterator,
};
use rucene::index::VectorSimilarityFunction;
use rucene::search::{DocIdSetIterator, NO_MORE_DOCS};
use rucene::util::vector_util;

// ---------------------------------------------------------------------------
// Java harness driver
// ---------------------------------------------------------------------------

fn harness_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/java-codec-harness")
}

/// Serialises Maven invocations, which all share one `target/` directory.
static HARNESS_LOCK: Mutex<()> = Mutex::new(());

fn which_mvn() -> Result<String, String> {
    for candidate in ["mvn", "mvnw"] {
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

/// Fails the test when Maven is unavailable.
fn require_maven() {
    if let Err(reason) = which_mvn() {
        panic!("vector/point portability tests require Maven and a JDK: {reason}");
    }
}

/// Runs one fixture class and returns its stdout.
///
/// `-Dlucene.useScalarFMA=false` is always passed and `--add-modules
/// jdk.incubator.vector` is never passed, so Lucene selects its scalar,
/// non-fused implementation. Anything else would make the captured float bits
/// depend on the host CPU.
fn run_fixture(main_class: &str, out_dir: &Path, case: &str) -> Result<String, String> {
    let _guard = HARNESS_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let harness = harness_dir();
    if !harness.join("pom.xml").exists() {
        return Err(format!("pom.xml not found in {}", harness.display()));
    }
    let mvn = which_mvn()?;
    let output = Command::new(mvn)
        .arg("-q")
        .arg("compile")
        .arg("exec:java")
        .arg("-Dlucene.useScalarFMA=false")
        .arg(format!(
            "-Dexec.mainClass=org.apache.lucene.rucene.codec.{main_class}"
        ))
        .arg(format!("-Dexec.args={} {case}", out_dir.display()))
        .current_dir(&harness)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| format!("failed to spawn Maven: {e}"))?;

    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    if !output.status.success() {
        return Err(format!(
            "{main_class}/{case} failed:\nstdout:\n{stdout}\nstderr:\n{stderr}"
        ));
    }
    Ok(stdout)
}

fn scratch_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rucene-portability-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch directory is creatable");
    dir
}

// ---------------------------------------------------------------------------
// Parsing helpers
// ---------------------------------------------------------------------------

/// Splits a `key=value key=value` line (after an optional leading token) into a
/// map.
fn pairs(line: &str, skip: usize) -> HashMap<&str, &str> {
    line.split_whitespace()
        .skip(skip)
        .filter_map(|token| token.split_once('='))
        .collect()
}

fn header(stdout: &str, key: &str) -> String {
    stdout
        .lines()
        .find_map(|line| line.strip_prefix(&format!("{key}=")))
        .unwrap_or_else(|| panic!("fixture printed no {key} line:\n{stdout}"))
        .trim()
        .to_string()
}

fn decode_b64(value: &str) -> Vec<u8> {
    STANDARD
        .decode(value)
        .unwrap_or_else(|e| panic!("fixture emitted invalid Base64 {value:?}: {e}"))
}

fn decode_floats(value: &str) -> Vec<f32> {
    let bytes = decode_b64(value);
    assert_eq!(bytes.len() % 4, 0, "float payload must be a multiple of 4");
    bytes
        .chunks_exact(4)
        .map(|chunk| f32::from_be_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect()
}

fn parse_bits(value: &str) -> u32 {
    u32::from_str_radix(value, 16)
        .unwrap_or_else(|e| panic!("fixture emitted invalid float bits {value:?}: {e}"))
}

/// Asserts that a Rucene float is bit-identical to the Java reference.
fn assert_same_bits(actual: f32, expected_bits: u32, what: &str) {
    assert_eq!(
        actual.to_bits(),
        expected_bits,
        "{what}: Rucene produced {actual} (bits {:08x}), Java produced bits {expected_bits:08x}",
        actual.to_bits()
    );
}

/// Rejects a fixture run made under JVM flags that would make the reference
/// values machine-dependent.
fn assert_deterministic_vector_path(stdout: &str) {
    assert_eq!(
        header(stdout, "has_fast_scalar_fma"),
        "false",
        "the reference JVM used Math.fma; rerun with -Dlucene.useScalarFMA=false"
    );
    assert_eq!(
        header(stdout, "incubator_vector_module"),
        "false",
        "the reference JVM resolved jdk.incubator.vector; the Panama path is not a \
         reproducible reference"
    );
    assert_eq!(header(stdout, "version"), "10.5.0");
}

// ---------------------------------------------------------------------------
// Vector similarity
// ---------------------------------------------------------------------------

#[test]
fn float_vector_similarity_is_bit_identical_to_java() {
    require_maven();
    let dir = scratch_dir("vector-similarity-float");
    let stdout =
        run_fixture("VectorSimilarityFixture", &dir, "FLOAT").unwrap_or_else(|e| panic!("{e}"));
    assert_deterministic_vector_path(&stdout);

    let mut inputs: HashMap<(u32, u32), (Vec<f32>, Vec<f32>)> = HashMap::new();
    let mut compared = 0usize;

    for line in stdout.lines() {
        if let Some(rest) = line.strip_prefix("vec ") {
            let fields = pairs(rest, 0);
            let key = (
                fields["dim"].parse().expect("dim is a number"),
                fields["id"].parse().expect("id is a number"),
            );
            inputs.insert(
                key,
                (decode_floats(fields["a"]), decode_floats(fields["b"])),
            );
        } else if let Some(rest) = line.strip_prefix("f32 ") {
            let fields = pairs(rest, 0);
            let dim: u32 = fields["dim"].parse().expect("dim is a number");
            let id: u32 = fields["id"].parse().expect("id is a number");
            let (a, b) = inputs
                .get(&(dim, id))
                .unwrap_or_else(|| panic!("no vectors emitted for dim={dim} id={id}"));
            assert_eq!(
                a.len(),
                dim as usize,
                "dim={dim} id={id}: wrong input length"
            );

            let label = format!("float dim={dim} id={id}");
            assert_same_bits(
                vector_util::dot_product_f32(a, b).expect("same dimensions"),
                parse_bits(fields["dotProduct"]),
                &format!("{label} dotProduct"),
            );
            assert_same_bits(
                vector_util::cosine_f32(a, b).expect("same dimensions"),
                parse_bits(fields["cosine"]),
                &format!("{label} cosine"),
            );
            assert_same_bits(
                vector_util::square_distance_f32(a, b).expect("same dimensions"),
                parse_bits(fields["squareDistance"]),
                &format!("{label} squareDistance"),
            );
            for (name, function) in similarity_functions() {
                assert_same_bits(
                    function.compare_f32(a, b).expect("same dimensions"),
                    parse_bits(fields[name]),
                    &format!("{label} {name}"),
                );
            }
            compared += 1;
        }
    }

    // Every emitted vector pair must have been compared, and the corpus must
    // not have silently shrunk.
    assert_eq!(
        compared,
        inputs.len(),
        "every emitted pair must be compared"
    );
    assert!(
        compared >= 60,
        "unexpected number of float cases:\n{stdout}"
    );
}

#[test]
fn byte_vector_similarity_is_bit_identical_to_java() {
    require_maven();
    let dir = scratch_dir("vector-similarity-byte");
    let stdout =
        run_fixture("VectorSimilarityFixture", &dir, "BYTE").unwrap_or_else(|e| panic!("{e}"));
    assert_deterministic_vector_path(&stdout);

    let mut inputs: HashMap<(u32, u32), (Vec<u8>, Vec<u8>)> = HashMap::new();
    let mut compared = 0usize;

    for line in stdout.lines() {
        if let Some(rest) = line.strip_prefix("vec ") {
            let fields = pairs(rest, 0);
            let key = (
                fields["dim"].parse().expect("dim is a number"),
                fields["id"].parse().expect("id is a number"),
            );
            inputs.insert(key, (decode_b64(fields["a"]), decode_b64(fields["b"])));
        } else if let Some(rest) = line.strip_prefix("i8 ") {
            let fields = pairs(rest, 0);
            let dim: u32 = fields["dim"].parse().expect("dim is a number");
            let id: u32 = fields["id"].parse().expect("id is a number");
            let (a, b) = inputs
                .get(&(dim, id))
                .unwrap_or_else(|| panic!("no vectors emitted for dim={dim} id={id}"));
            assert_eq!(
                a.len(),
                dim as usize,
                "dim={dim} id={id}: wrong input length"
            );

            let label = format!("byte dim={dim} id={id}");
            // Integer results must be exact, which is the whole point of
            // accumulating in i32 rather than f32.
            assert_eq!(
                vector_util::dot_product_bytes(a, b).expect("same dimensions"),
                fields["dotProduct"].parse::<i32>().expect("an integer"),
                "{label} dotProduct"
            );
            assert_eq!(
                vector_util::square_distance_bytes(a, b).expect("same dimensions"),
                fields["squareDistance"].parse::<i32>().expect("an integer"),
                "{label} squareDistance"
            );
            assert_same_bits(
                vector_util::cosine_bytes(a, b).expect("same dimensions"),
                parse_bits(fields["cosine"]),
                &format!("{label} cosine"),
            );
            assert_same_bits(
                vector_util::dot_product_score(a, b).expect("same dimensions"),
                parse_bits(fields["dotProductScore"]),
                &format!("{label} dotProductScore"),
            );
            for (name, function) in similarity_functions() {
                assert_same_bits(
                    function.compare_bytes(a, b).expect("same dimensions"),
                    parse_bits(fields[name]),
                    &format!("{label} {name}"),
                );
            }
            compared += 1;
        }
    }

    assert_eq!(
        compared,
        inputs.len(),
        "every emitted pair must be compared"
    );
    assert!(compared >= 60, "unexpected number of byte cases:\n{stdout}");
}

fn similarity_functions() -> [(&'static str, VectorSimilarityFunction); 4] {
    [
        ("EUCLIDEAN", VectorSimilarityFunction::EUCLIDEAN),
        ("DOT_PRODUCT", VectorSimilarityFunction::DOT_PRODUCT),
        ("COSINE", VectorSimilarityFunction::COSINE),
        (
            "MAXIMUM_INNER_PRODUCT",
            VectorSimilarityFunction::MAXIMUM_INNER_PRODUCT,
        ),
    ]
}

// ---------------------------------------------------------------------------
// Point tree
// ---------------------------------------------------------------------------

/// The Java side of one `PointTreeFixture` run.
struct JavaPoints {
    num_dims: i32,
    num_index_dims: i32,
    bytes_per_dim: i32,
    size: i64,
    doc_count: i32,
    min_packed: Vec<u8>,
    max_packed: Vec<u8>,
    /// Every stored point, in traversal order.
    points: Vec<(i32, Vec<u8>)>,
    /// Query name to (min, max) bounds.
    queries: Vec<(String, Vec<u8>, Vec<u8>)>,
    /// Query name to the recorded callback trace.
    traces: HashMap<String, Vec<String>>,
    /// Query name to the accepted doc IDs.
    accepted: HashMap<String, Vec<i32>>,
    /// Query name to (estimated point count, estimated doc count).
    estimates: HashMap<String, (i64, i64)>,
}

fn parse_points(stdout: &str) -> JavaPoints {
    let mut points = Vec::new();
    let mut queries = Vec::new();
    let mut traces: HashMap<String, Vec<String>> = HashMap::new();
    let mut accepted: HashMap<String, Vec<i32>> = HashMap::new();
    let mut estimates = HashMap::new();

    for line in stdout.lines() {
        if let Some(rest) = line.strip_prefix("point ") {
            let fields = pairs(rest, 0);
            points.push((
                fields["doc"].parse().expect("doc is a number"),
                decode_b64(fields["value"]),
            ));
        } else if let Some(rest) = line.strip_prefix("query ") {
            let fields = pairs(rest, 0);
            queries.push((
                fields["name"].to_string(),
                decode_b64(fields["min"]),
                decode_b64(fields["max"]),
            ));
        } else if let Some(rest) = line.strip_prefix("trace ") {
            let (name, entry) = rest.split_once(' ').expect("a trace line has a query name");
            traces
                .entry(name.to_string())
                .or_default()
                .push(entry.to_string());
        } else if let Some(rest) = line.strip_prefix("accepted ") {
            let (name, docs) = match rest.split_once(' ') {
                Some((name, docs)) => (name, docs),
                None => (rest.trim(), ""),
            };
            let docs = docs
                .trim()
                .split(',')
                .filter(|token| !token.is_empty())
                .map(|token| token.parse().expect("doc is a number"))
                .collect();
            accepted.insert(name.to_string(), docs);
        } else if let Some(rest) = line.strip_prefix("estimate ") {
            let name = rest.split_whitespace().next().expect("a query name");
            let fields = pairs(rest, 1);
            estimates.insert(
                name.to_string(),
                (
                    fields["point_count"].parse().expect("a number"),
                    fields["doc_count"].parse().expect("a number"),
                ),
            );
        }
    }

    JavaPoints {
        num_dims: header(stdout, "num_dims").parse().expect("a number"),
        num_index_dims: header(stdout, "num_index_dims").parse().expect("a number"),
        bytes_per_dim: header(stdout, "bytes_per_dim").parse().expect("a number"),
        size: header(stdout, "size").parse().expect("a number"),
        doc_count: header(stdout, "doc_count").parse().expect("a number"),
        min_packed: decode_b64(&header(stdout, "min_packed")),
        max_packed: decode_b64(&header(stdout, "max_packed")),
        points,
        queries,
        traces,
        accepted,
        estimates,
    }
}

/// Mirror of the Java fixture's instrumented visitor, emitting the same trace
/// strings so the two can be compared literally.
struct TracingVisitor {
    query_min: Vec<u8>,
    query_max: Vec<u8>,
    num_index_dims: usize,
    bytes_per_dim: usize,
    trace: std::cell::RefCell<Vec<String>>,
    accepted: Vec<i32>,
}

impl TracingVisitor {
    fn new(min: &[u8], max: &[u8], num_index_dims: i32, bytes_per_dim: i32) -> Self {
        Self {
            query_min: min.to_vec(),
            query_max: max.to_vec(),
            num_index_dims: num_index_dims as usize,
            bytes_per_dim: bytes_per_dim as usize,
            trace: std::cell::RefCell::new(Vec::new()),
            accepted: Vec::new(),
        }
    }

    fn dim<'a>(&self, value: &'a [u8], dim: usize) -> &'a [u8] {
        let offset = dim * self.bytes_per_dim;
        &value[offset..offset + self.bytes_per_dim]
    }

    fn relate(&self, cell_min: &[u8], cell_max: &[u8]) -> Relation {
        let mut inside = true;
        for dim in 0..self.num_index_dims {
            if self.dim(cell_max, dim) < self.dim(&self.query_min, dim)
                || self.dim(cell_min, dim) > self.dim(&self.query_max, dim)
            {
                return Relation::CellOutsideQuery;
            }
            if self.dim(cell_min, dim) < self.dim(&self.query_min, dim)
                || self.dim(cell_max, dim) > self.dim(&self.query_max, dim)
            {
                inside = false;
            }
        }
        if inside {
            Relation::CellInsideQuery
        } else {
            Relation::CellCrossesQuery
        }
    }

    fn matches(&self, packed_value: &[u8]) -> bool {
        (0..self.num_index_dims).all(|dim| {
            self.dim(packed_value, dim) >= self.dim(&self.query_min, dim)
                && self.dim(packed_value, dim) <= self.dim(&self.query_max, dim)
        })
    }

    fn trace(&self) -> Vec<String> {
        self.trace.borrow().clone()
    }
}

/// The Java enum names, so the two traces are literally comparable.
fn relation_name(relation: Relation) -> &'static str {
    match relation {
        Relation::CellInsideQuery => "CELL_INSIDE_QUERY",
        Relation::CellOutsideQuery => "CELL_OUTSIDE_QUERY",
        Relation::CellCrossesQuery => "CELL_CROSSES_QUERY",
    }
}

impl IntersectVisitor for TracingVisitor {
    fn visit(&mut self, doc_id: i32) -> rucene::error::Result<()> {
        self.trace.borrow_mut().push(format!("visit {doc_id}"));
        self.accepted.push(doc_id);
        Ok(())
    }

    fn visit_with_value(&mut self, doc_id: i32, packed_value: &[u8]) -> rucene::error::Result<()> {
        self.trace
            .borrow_mut()
            .push(format!("visitv {doc_id} {}", STANDARD.encode(packed_value)));
        if self.matches(packed_value) {
            self.accepted.push(doc_id);
        }
        Ok(())
    }

    fn compare(&self, min_packed_value: &[u8], max_packed_value: &[u8]) -> Relation {
        let relation = self.relate(min_packed_value, max_packed_value);
        self.trace.borrow_mut().push(format!(
            "compare {} {} {}",
            STANDARD.encode(min_packed_value),
            STANDARD.encode(max_packed_value),
            relation_name(relation)
        ));
        relation
    }

    fn grow(&mut self, count: i32) {
        self.trace.borrow_mut().push(format!("grow {count}"));
    }
}

/// Builds Rucene point values holding exactly the points Java stored, as a
/// single leaf.
fn rust_values(java: &JavaPoints) -> InMemoryPointValues {
    InMemoryPointValues::new(
        java.num_dims,
        java.num_index_dims,
        java.bytes_per_dim,
        vec![java.points.clone()],
    )
    .expect("the leaf Java produced satisfies the PointTree contract")
}

fn assert_metadata_matches(java: &JavaPoints, values: &InMemoryPointValues) {
    assert_eq!(values.size(), java.size, "size");
    assert_eq!(values.doc_count(), java.doc_count, "doc count");
    assert_eq!(
        values.num_dimensions().unwrap(),
        java.num_dims,
        "num dimensions"
    );
    assert_eq!(
        values.num_index_dimensions().unwrap(),
        java.num_index_dims,
        "num index dimensions"
    );
    assert_eq!(
        values.bytes_per_dimension().unwrap(),
        java.bytes_per_dim,
        "bytes per dimension"
    );
    assert_eq!(
        values.min_packed_value().unwrap().unwrap(),
        java.min_packed,
        "min packed value"
    );
    assert_eq!(
        values.max_packed_value().unwrap().unwrap(),
        java.max_packed,
        "max packed value"
    );
}

/// Runs one single-leaf, one-dimensional case end to end and compares the full
/// `intersect` call trace plus both estimators.
fn assert_one_dimensional_case_matches_java(case: &str, scratch: &str) {
    let dir = scratch_dir(scratch);
    let stdout = run_fixture("PointTreeFixture", &dir, case).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(header(&stdout, "version"), "10.5.0");
    // A single-leaf tree is the case whose geometry Rucene can reproduce today.
    assert_eq!(
        header(&stdout, "node_count"),
        "1",
        "[{case}] expected one leaf"
    );

    let java = parse_points(&stdout);
    let values = rust_values(&java);
    assert_metadata_matches(&java, &values);
    assert_eq!(java.points.len() as i64, java.size);
    assert!(!java.queries.is_empty());

    for (name, min, max) in &java.queries {
        let mut visitor = TracingVisitor::new(min, max, java.num_index_dims, java.bytes_per_dim);
        values.intersect(&mut visitor).expect("traversal succeeds");
        assert_eq!(
            visitor.trace(),
            java.traces.get(name).cloned().unwrap_or_default(),
            "[{case}] query {name}: the intersect call trace diverges from Lucene"
        );
        assert_eq!(
            visitor.accepted, java.accepted[name],
            "[{case}] query {name}: accepted documents"
        );

        let mut estimator = TracingVisitor::new(min, max, java.num_index_dims, java.bytes_per_dim);
        let point_count = values.estimate_point_count(&mut estimator).unwrap();
        let doc_count = values.estimate_doc_count(&mut estimator).unwrap();
        assert_eq!(
            (point_count, doc_count),
            java.estimates[name],
            "[{case}] query {name}: estimates"
        );
    }
}

#[test]
fn one_dimensional_single_leaf_intersect_trace_matches_java() {
    require_maven();
    assert_one_dimensional_case_matches_java("ONE_LEAF_1D", "point-tree-1d");
}

/// Same comparison over a multi-valued field, where `size() > getDocCount()`.
///
/// This is the only case that reaches the urn-problem branch of
/// `estimateDocCount`: with one point per document the estimator returns the
/// point estimate unchanged and the formula is never evaluated.
#[test]
fn multi_valued_single_leaf_estimates_match_java() {
    require_maven();
    assert_one_dimensional_case_matches_java("MULTI_VALUED_1D", "point-tree-multivalued");
}

#[test]
fn two_dimensional_single_leaf_results_match_java() {
    require_maven();
    let dir = scratch_dir("point-tree-2d");
    let stdout =
        run_fixture("PointTreeFixture", &dir, "ONE_LEAF_2D").unwrap_or_else(|e| panic!("{e}"));
    let java = parse_points(&stdout);
    assert_eq!(java.num_index_dims, 2);

    let values = rust_values(&java);
    assert_metadata_matches(&java, &values);

    for (name, min, max) in &java.queries {
        let mut visitor = TracingVisitor::new(min, max, java.num_index_dims, java.bytes_per_dim);
        values.intersect(&mut visitor).expect("traversal succeeds");
        // The raw trace is deliberately not compared here: for
        // `numIndexDims != 1` BKDReader issues an extra `compare` against the
        // leaf's narrowed bounds before visiting, which is a property of the
        // BKD cursor rather than of the traversal algorithm. Task #119 brings
        // that cursor, and with it the trace comparison for this case.
        assert_eq!(
            visitor.accepted, java.accepted[name],
            "query {name}: accepted documents"
        );

        let mut estimator = TracingVisitor::new(min, max, java.num_index_dims, java.bytes_per_dim);
        let point_count = values.estimate_point_count(&mut estimator).unwrap();
        let doc_count = values.estimate_doc_count(&mut estimator).unwrap();
        assert_eq!(
            (point_count, doc_count),
            java.estimates[name],
            "query {name}: estimates"
        );
    }
}

#[test]
fn multi_leaf_aggregates_match_java() {
    require_maven();
    let dir = scratch_dir("point-tree-multi");
    let stdout =
        run_fixture("PointTreeFixture", &dir, "MULTI_LEAF_1D").unwrap_or_else(|e| panic!("{e}"));
    let java = parse_points(&stdout);

    // The corpus is big enough for BKD to build several levels; that is what
    // makes the geometry unreproducible here.
    let node_count: usize = header(&stdout, "node_count").parse().expect("a number");
    assert!(
        node_count > 1,
        "the multi-leaf case must actually produce an inner tree, got {node_count} nodes"
    );

    // Only the field-level aggregates are comparable without the BKD cursor;
    // see the module documentation. They still exercise the contract that
    // `size` counts points, `getDocCount` counts documents, and that the packed
    // bounds are the unsigned per-dimension extremes.
    assert_eq!(java.points.len() as i64, java.size);
    assert_eq!(java.size, 2000);
    assert_eq!(java.doc_count, 2000);

    let mut min = java.points[0].1.clone();
    let mut max = java.points[0].1.clone();
    for (_, value) in &java.points {
        if value < &min {
            min = value.clone();
        }
        if value > &max {
            max = value.clone();
        }
    }
    assert_eq!(min, java.min_packed, "min packed value");
    assert_eq!(max, java.max_packed, "max packed value");

    // The points Java reported are in the order the visit contract requires:
    // increasing value, ties by increasing doc id. Rucene's reference tree
    // enforces that invariant, so building it is itself the assertion.
    let values = rust_values(&java);
    assert_eq!(values.size(), java.size);
    assert_eq!(values.doc_count(), java.doc_count);
}

// ---------------------------------------------------------------------------
// Vector value iterators
// ---------------------------------------------------------------------------

/// A doc-id iterator over a fixed list, mirroring the Java fixture's.
struct ArrayDocs {
    docs: Vec<i32>,
    position: i32,
}

impl ArrayDocs {
    fn new(docs: Vec<i32>) -> Self {
        Self { docs, position: -1 }
    }
}

impl DocIdSetIterator for ArrayDocs {
    fn doc_id(&self) -> i32 {
        if self.position < 0 {
            -1
        } else if self.position as usize >= self.docs.len() {
            NO_MORE_DOCS
        } else {
            self.docs[self.position as usize]
        }
    }

    fn next_doc(&mut self) -> rucene::error::Result<i32> {
        self.position += 1;
        Ok(self.doc_id())
    }

    fn advance(&mut self, target: i32) -> rucene::error::Result<i32> {
        loop {
            let doc = self.next_doc()?;
            if doc >= target {
                return Ok(doc);
            }
        }
    }

    fn cost(&self) -> i64 {
        self.docs.len() as i64
    }
}

/// Lines of the shape `seq <name> next doc=.. index=.. run_end=..`.
fn sequence_lines(stdout: &str, prefix: &str, name: &str) -> Vec<HashMap<String, String>> {
    stdout
        .lines()
        .filter_map(|line| line.strip_prefix(&format!("{prefix} {name} ")))
        .filter(|rest| rest.starts_with("next "))
        .map(|rest| {
            pairs(rest, 1)
                .into_iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect()
        })
        .collect()
}

#[test]
fn doc_index_iterators_match_java() {
    require_maven();
    let dir = scratch_dir("vector-iterators");
    let stdout = run_fixture("VectorValuesIteratorFixture", &dir, "ITERATORS")
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(header(&stdout, "version"), "10.5.0");

    // --- dense -------------------------------------------------------------
    let dense_expected = sequence_lines(&stdout, "seq", "dense");
    assert_eq!(dense_expected.len(), 5);
    let mut dense = DenseDocIndexIterator::new(5);
    assert_eq!(dense.doc_id(), -1);
    assert_eq!(dense.index(), -1);
    for expected in &dense_expected {
        let doc = dense.next_doc().unwrap();
        assert_eq!(doc.to_string(), expected["doc"], "dense doc id");
        assert_eq!(dense.index().to_string(), expected["index"], "dense index");
        assert_eq!(
            dense.doc_id_run_end().unwrap().to_string(),
            expected["run_end"],
            "dense docIDRunEnd"
        );
    }
    assert_eq!(dense.next_doc().unwrap(), NO_MORE_DOCS);

    for expected in advance_lines(&stdout, "dense") {
        let mut it = DenseDocIndexIterator::new(5);
        let doc = it.advance(expected["target"].parse().unwrap()).unwrap();
        assert_eq!(doc.to_string(), expected["doc"], "dense advance doc id");
        assert_eq!(
            it.index().to_string(),
            expected["index"],
            "dense advance index"
        );
    }

    // --- sparse ------------------------------------------------------------
    let ord_to_doc: Vec<i32> = stdout
        .lines()
        .filter_map(|line| line.strip_prefix("ord_to_doc name=sparse "))
        .map(|rest| pairs(rest, 0)["doc"].parse().expect("a number"))
        .collect();
    assert_eq!(ord_to_doc, vec![2, 4, 9, 15]);

    let make_sparse = || {
        let mapping = ord_to_doc.clone();
        SparseDocIndexIterator::new(
            mapping.len() as i32,
            std::sync::Arc::new(move |ord: i32| mapping[ord as usize]),
        )
    };

    let sparse_expected = sequence_lines(&stdout, "seq", "sparse");
    assert_eq!(sparse_expected.len(), 4);
    let mut sparse = make_sparse();
    assert_eq!(sparse.doc_id(), -1);
    for expected in &sparse_expected {
        let doc = sparse.next_doc().unwrap();
        assert_eq!(doc.to_string(), expected["doc"], "sparse doc id");
        assert_eq!(
            sparse.index().to_string(),
            expected["index"],
            "sparse index"
        );
        assert_eq!(
            sparse.doc_id_run_end().unwrap().to_string(),
            expected["run_end"],
            "sparse docIDRunEnd"
        );
    }
    assert_eq!(sparse.next_doc().unwrap(), NO_MORE_DOCS);
    // Java stores NO_MORE_DOCS in the ordinal once exhausted.
    assert_eq!(sparse.index(), NO_MORE_DOCS);

    for expected in advance_lines(&stdout, "sparse") {
        let mut it = make_sparse();
        let doc = it.advance(expected["target"].parse().unwrap()).unwrap();
        assert_eq!(doc.to_string(), expected["doc"], "sparse advance doc id");
        assert_eq!(
            it.index().to_string(),
            expected["index"],
            "sparse advance index"
        );
    }

    // --- fromDISI ----------------------------------------------------------
    let disi_docs = vec![0, 5, 9, 12];
    let disi_expected = sequence_lines(&stdout, "seq", "from_disi");
    assert_eq!(disi_expected.len(), 4);
    let mut disi = FromDisiDocIndexIterator::new(Box::new(ArrayDocs::new(disi_docs.clone())));
    for expected in &disi_expected {
        let doc = disi.next_doc().unwrap();
        assert_eq!(doc.to_string(), expected["doc"], "fromDISI doc id");
        assert_eq!(
            disi.index().to_string(),
            expected["index"],
            "fromDISI index"
        );
    }
    assert_eq!(disi.next_doc().unwrap(), NO_MORE_DOCS);
    let end = stdout
        .lines()
        .find_map(|line| line.strip_prefix("seq from_disi end "))
        .expect("the fixture prints the exhausted state");
    let end = pairs(end, 0);
    assert_eq!(
        disi.index().to_string(),
        end["index"],
        "fromDISI final index"
    );

    // The regression that matters: Java's `advance` never touches the ordinal.
    let steps: Vec<HashMap<&str, &str>> = stdout
        .lines()
        .filter_map(|line| line.strip_prefix("step from_disi "))
        .map(|rest| pairs(rest, 0))
        .collect();
    assert_eq!(steps.len(), 5);
    let mut mixed = FromDisiDocIndexIterator::new(Box::new(ArrayDocs::new(disi_docs)));
    for step in steps {
        let observed = match step["op"] {
            "start" => mixed.doc_id(),
            "next" => mixed.next_doc().unwrap(),
            "advance(12)" => mixed.advance(12).unwrap(),
            other => panic!("unknown fixture step {other:?}"),
        };
        assert_eq!(
            observed.to_string(),
            step["doc"],
            "fromDISI step {}: doc id",
            step["op"]
        );
        assert_eq!(
            mixed.index().to_string(),
            step["index"],
            "fromDISI step {}: index() must follow Lucene, which leaves the ordinal \
             untouched by advance",
            step["op"]
        );
    }
}

fn advance_lines<'a>(stdout: &'a str, name: &str) -> Vec<HashMap<&'a str, &'a str>> {
    let prefix = format!("advance {name} ");
    stdout
        .lines()
        .filter_map(|line| line.strip_prefix(prefix.as_str()))
        .map(|rest| pairs(rest, 0))
        .collect()
}

#[test]
fn indexed_vector_values_metadata_matches_java() {
    require_maven();
    let dir = scratch_dir("vector-indexed");
    let stdout = run_fixture("VectorValuesIteratorFixture", &dir, "INDEXED")
        .unwrap_or_else(|e| panic!("{e}"));

    for field in ["dense", "sparse"] {
        let meta = stdout
            .lines()
            .filter_map(|line| line.strip_prefix("indexed "))
            .map(|rest| pairs(rest, 0))
            .find(|fields| fields["field"] == field)
            .unwrap_or_else(|| panic!("no metadata for field {field}:\n{stdout}"));

        let dimension: i32 = meta["dimension"].parse().expect("a number");
        let encoding_ordinal: usize = meta["encoding_ordinal"].parse().expect("a number");
        // FLOAT32 is ordinal 1 and four bytes wide, so the byte length is the
        // formula `dimension() * getEncoding().byteSize`.
        assert_eq!(encoding_ordinal, 1, "{field}: FLOAT32 ordinal");
        assert_eq!(
            meta["byte_length"].parse::<i32>().expect("a number"),
            dimension * 4,
            "{field}: vector byte length"
        );

        let ord_to_doc: Vec<i32> = stdout
            .lines()
            .filter_map(|line| line.strip_prefix("indexed_ord "))
            .map(|rest| pairs(rest, 0))
            .filter(|fields| fields["field"] == field)
            .map(|fields| fields["doc"].parse().expect("a number"))
            .collect();
        assert_eq!(
            ord_to_doc.len(),
            meta["size"].parse::<usize>().expect("a number"),
            "{field}: one doc per ordinal"
        );

        // Rebuild the sequence Lucene's reader produced from the ord-to-doc
        // mapping alone, using the iterator shape that matches the field.
        let expected: Vec<(i32, i32)> = stdout
            .lines()
            .filter_map(|line| line.strip_prefix("indexed_seq "))
            .map(|rest| pairs(rest, 0))
            .filter(|fields| fields["field"] == field)
            .map(|fields| {
                (
                    fields["doc"].parse().expect("a number"),
                    fields["index"].parse().expect("a number"),
                )
            })
            .collect();

        let mapping = ord_to_doc.clone();
        let mut iterator: Box<dyn DocIndexIterator> = if ord_to_doc
            .iter()
            .enumerate()
            .all(|(ord, doc)| ord as i32 == *doc)
        {
            Box::new(DenseDocIndexIterator::new(ord_to_doc.len() as i32))
        } else {
            Box::new(SparseDocIndexIterator::new(
                mapping.len() as i32,
                std::sync::Arc::new(move |ord: i32| mapping[ord as usize]),
            ))
        };

        let mut observed = Vec::new();
        loop {
            let doc = iterator.next_doc().unwrap();
            if doc == NO_MORE_DOCS {
                break;
            }
            observed.push((doc, iterator.index()));
        }
        assert_eq!(observed, expected, "{field}: (docID, index()) sequence");
    }
}
