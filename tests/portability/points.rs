//! Portability tests for the point-values writer.
//!
//! Every test drives the same document table through Rucene's indexing chain
//! and through a Java harness that indexes the identical documents with Apache
//! Lucene Core 10.5.0, then asserts three ways:
//!
//! 1. the `.kdd`, `.kdi` and `.kdm` files the two sides write are **byte for
//!    byte identical**;
//! 2. Rucene reads Lucene's segment and decodes exactly what Lucene's own
//!    reader decodes — every point, in the same traversal order, and the same
//!    documents for a pruning range intersection;
//! 3. Lucene reads Rucene's segment through its real `Lucene90PointsFormat`
//!    reader and decodes exactly what it decodes from its own files.
//!
//! The cases span the shape dimensions of the format rather than only its
//! values, because a byte difference can hide behind a shape:
//!
//! * `int1d` — 1300 points in one 4-byte, one-dimensional field, so the BKD
//!   tree has three leaves and a split level, with heavily repeated values;
//! * `multi1d` — one to three values per document and every third document
//!   with none, so `numPoints` and `numDocs` genuinely differ and the leaves
//!   hold repeated doc IDs;
//! * `long1d` — 8 bytes per dimension across the sign boundary;
//! * `bin1d` — 16 bytes per dimension, which is `PointValues.MAX_NUM_BYTES`;
//! * `small` — seven documents, so the tree is a single leaf that is also its
//!   own root;
//! * `mixed` — five one-dimensional fields whose field-hash flush order is
//!   `[3, 0, 1, 2, 4]`, so the per-field entries inside the `.kdm` are in
//!   neither registration nor field-number order. Decoded from the `.kdm`
//!   Lucene itself wrote, the first field number after the 50-byte index
//!   header is 3, not 0.
//!
//! * `nd2` — two indexed dimensions over 900 documents, so the tree has a
//!   split level and the split dimension byte, the per-dimension leaf bounds
//!   and the compressed dimension are all exercised;
//! * `ndleaf` — three indexed dimensions over fewer documents than one leaf
//!   holds, which is where the sorted dimension is chosen with no partition
//!   above it;
//! * `ndsort` — a leaf built so that the sorted dimension depends on whether
//!   the leaf's first point is counted when the per-dimension byte
//!   cardinalities are measured, which is a difference between Java's two
//!   `build` methods and was a real divergence here;
//! * `ndsel` — three data dimensions of which only one is indexed, the single
//!   shape with `numDims != numIndexDims`, where the unindexed dimensions are
//!   what breaks ties inside a leaf;
//! * `ndsplit` — 513 points over two indexed dimensions at a leaf size of 512,
//!   so the tree is exactly two leaves and has exactly one partition, and the
//!   leaf below it begins at whatever point the *selection* left there;
//! * `ndmulti` — **two points per document** over two indexed dimensions and a
//!   three-value alphabet, so points of one document tie under a comparator
//!   that looks at neither the other indexed dimension nor anything else;
//! * `nddeep` — 12000 points over three **correlated** indexed dimensions, deep
//!   enough that the recursion reaches a node where the exact bounds are
//!   recomputed. Random dimensions do not reach it; measured, correlated ones
//!   do.
//!
//! # The multi-dimension cases assert bytes, not just content
//!
//! They did not always. `Lucene90PointsWriter` takes Java's `MutablePointTree`
//! fast path — [`the_codec_takes_the_mutable_fast_path`] pins that branch — and
//! every case above is byte-identical to what Lucene 10.5.0 writes for the same
//! documents. Measured over a 324-shape grid against Lucene itself (1 to 4
//! dimensions, `numIndexDims` at 1 and at `numDims`, `bytesPerDim` of 1, 4 and
//! 16, 20 to 12000 documents, one and two points per document, two value
//! patterns and two cardinalities), **all 324 match**; the module documentation
//! of [`rucene::index::point_values_writer`] records what that took.
//!
//! `ndsort`, `ndsplit`, `ndmulti` and `nddeep` were each built from a measured
//! divergence rather than from a guess at what might matter, and each fails
//! loudly if its fix is reverted. That is the reason to distrust a suite whose
//! cases were all chosen by intuition: not one of those four shapes would have
//! been written without first seeing it diverge, and the three that came last
//! were found only after a wider grid was run than the one that first said
//! "all green".
//!
//! A missing Maven or JDK is a hard failure, not a skip: a portability test has
//! nothing to assert without the reference implementation, so skipping would
//! report success while proving nothing. This matches the other tests under
//! `tests/portability/`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::AtomicI64;
use std::sync::{Arc, Mutex};

use rucene::analysis::{Analyzer, StandardAnalyzer};
use rucene::codecs::points::PointsWriter;
use rucene::codecs::state::SegmentReadState;
use rucene::codecs::{register_codec, Codec, Lucene104Codec, PointsReader};
use rucene::document::{BinaryPoint, Document, Field, FieldType, IntPoint, LongPoint};
use rucene::index::documents_writer::{IndexingChain, IndexingChainFlushState};
use rucene::index::field_infos::{FieldInfosBuilder, FieldNumbers};
use rucene::index::index_writer_config::LiveIndexWriterConfig;
use rucene::index::indexing_chain::DefaultIndexingChain;
use rucene::index::point_values::{IntersectVisitor, Relation};
use rucene::index::point_values_writer::PointValuesWriter;
use rucene::index::{FieldInfos, SegmentInfo, SegmentInfos};
use rucene::store::{
    flush_io_context, Directory, FSDirectory, FlushInfo, TrackingDirectoryWrapper,
    DEFAULT_IO_CONTEXT,
};
use rucene::util::{BytesRef, NoOutputInfoStream, Version};

/// The three files a points segment is made of.
const EXTENSIONS: [&str; 3] = ["kdd", "kdi", "kdm"];

static HARNESS_LOCK: Mutex<()> = Mutex::new(());

// ---------------------------------------------------------------------------
// Java harness driver
// ---------------------------------------------------------------------------

fn harness_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
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

/// Fails the test when Maven is unavailable.
///
/// A portability test proves compatibility against the reference Java
/// implementation, so it has nothing to assert without the harness: skipping
/// would report success while proving nothing.
fn require_maven() {
    if let Err(reason) = which_mvn() {
        panic!("points portability tests require Maven and a JDK: {reason}");
    }
}

/// What the Java harness reports about the segment it committed.
#[derive(Debug)]
struct JavaSegment {
    name: String,
    id: [u8; 16],
    max_doc: i32,
    /// Whether the segment was bundled into a `.cfs`.
    compound: bool,
    /// One `pointstats`/`point`/`range` line per statistic, value and range
    /// result Lucene's own reader produced, in that order per field.
    dump: Vec<String>,
    /// The `.fnm` entry of every field, as Lucene committed it.
    field_infos: Vec<String>,
}

fn run_java_fixture(out_dir: &Path, case: &str) -> Result<JavaSegment, String> {
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
        .arg("-Dexec.mainClass=org.apache.lucene.rucene.codec.PointsFixture")
        .arg(format!("-Dexec.args={} {}", out_dir.display(), case))
        .current_dir(&harness)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| format!("failed to spawn Maven: {e}"))?;

    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    if !output.status.success() {
        return Err(format!(
            "Java harness failed for case {case}:\nstdout:\n{stdout}\nstderr:\n{stderr}"
        ));
    }
    if !stdout.lines().any(|line| line.trim() == "read_ok=true") {
        return Err(format!("the fixture did not finish:\n{stdout}"));
    }
    parse_segment(&stdout)
}

fn parse_segment(stdout: &str) -> Result<JavaSegment, String> {
    let mut name = None;
    let mut id = None;
    let mut max_doc = None;
    let mut compound = None;
    let mut dump = Vec::new();
    let mut field_infos = Vec::new();
    for line in stdout.lines() {
        if let Some(value) = line.strip_prefix("segment=") {
            name = Some(value.trim().to_string());
        } else if let Some(value) = line.strip_prefix("segment_id=") {
            id = Some(parse_id(value.trim())?);
        } else if let Some(value) = line.strip_prefix("max_doc=") {
            max_doc = Some(
                value
                    .trim()
                    .parse::<i32>()
                    .map_err(|e| format!("bad max_doc: {e}"))?,
            );
        } else if let Some(value) = line.strip_prefix("compound=") {
            compound = Some(value.trim() == "true");
        } else if let Some(value) = line.strip_prefix("fieldinfo=") {
            field_infos.push(value.trim().to_string());
        } else if is_dump_line(line) {
            dump.push(line.trim_end().to_string());
        }
    }
    Ok(JavaSegment {
        name: name.ok_or_else(|| format!("harness printed no segment name:\n{stdout}"))?,
        id: id.ok_or_else(|| format!("harness printed no segment id:\n{stdout}"))?,
        max_doc: max_doc.ok_or_else(|| format!("harness printed no max doc:\n{stdout}"))?,
        compound: compound.ok_or_else(|| format!("harness printed no compound flag:\n{stdout}"))?,
        dump,
        field_infos,
    })
}

fn is_dump_line(line: &str) -> bool {
    line.starts_with("pointstats ")
        || line.starts_with("point field=")
        || line.starts_with("range ")
}

fn parse_id(raw: &str) -> Result<[u8; 16], String> {
    if raw.len() != 32 {
        return Err(format!("unexpected segment id {raw:?}"));
    }
    let mut bytes = [0u8; 16];
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&raw[index * 2..index * 2 + 2], 16)
            .map_err(|e| format!("bad segment id {raw:?}: {e}"))?;
    }
    Ok(bytes)
}

/// Reads the points Rucene wrote with the real Lucene reader and returns the
/// same lines `PointsFixture` would have printed.
fn read_with_java(
    dir: &Path,
    segment_name: &str,
    segment_id: [u8; 16],
    max_doc: i32,
    case: &str,
    field_infos: &FieldInfos,
) -> Result<Vec<String>, String> {
    let _guard = HARNESS_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let mvn = which_mvn()?;
    let id_hex: String = segment_id
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    // Only the fields that actually carry points may be listed: the points
    // metadata is keyed by field number and Lucene refuses an entry whose
    // field declares no dimensions. Points are not written through a per-field
    // format, so unlike doc values there is no format attribute to recompute.
    let fields: Vec<String> = field_infos
        .iter()
        .filter(|info| info.point_dimension_count != 0)
        .map(|info| {
            format!(
                "{}:{}:{}:{}:{}",
                info.name,
                info.number,
                info.point_dimension_count,
                info.point_index_dimension_count,
                info.point_num_bytes
            )
        })
        .collect();
    let output = Command::new(mvn)
        .arg("-q")
        .arg("compile")
        .arg("exec:java")
        .arg("-Dexec.mainClass=org.apache.lucene.rucene.codec.PointsReaderFixture")
        .arg(format!(
            "-Dexec.args={} {} {} {} {} {}",
            dir.display(),
            segment_name,
            id_hex,
            max_doc,
            case,
            if fields.is_empty() {
                "-".to_string()
            } else {
                fields.join(",")
            }
        ))
        .current_dir(harness_dir())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| format!("failed to spawn Maven: {e}"))?;

    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    if !output.status.success() {
        return Err(format!(
            "Lucene could not read the Rucene-written segment:\nstdout:\n{stdout}\nstderr:\n{stderr}"
        ));
    }
    if !stdout.lines().any(|line| line.trim() == "read_ok=true") {
        return Err(format!("the reader fixture did not finish:\n{stdout}"));
    }
    Ok(stdout
        .lines()
        .filter(|line| is_dump_line(line))
        .map(|line| line.trim_end().to_string())
        .collect())
}

// ---------------------------------------------------------------------------
// The corpus, mirrored from PointsFixture.documents
// ---------------------------------------------------------------------------

/// One point of one field of one document.
#[derive(Clone, Debug, PartialEq)]
enum Pt {
    Ints(&'static str, Vec<i32>),
    /// An int point whose leading `usize` dimensions are the indexed ones.
    Selective(&'static str, Vec<i32>, i32),
    Longs(&'static str, Vec<i64>),
    Binary(&'static str, Vec<u8>),
}

/// The 16-byte value of `bin1d`, mirroring `PointsFixture.binary16`.
fn binary16(doc: i32) -> Vec<u8> {
    let mut value = vec![0u8; 16];
    let high = (doc as i64).wrapping_mul(0x9E3779B97F4A7C15u64 as i64);
    let low = ((doc % 17) as i64).wrapping_mul(0xC2B2AE3D27D4EB4Fu64 as i64);
    for i in 0..8 {
        value[i] = ((high as u64) >> (56 - 8 * i)) as u8;
        value[8 + i] = ((low as u64) >> (56 - 8 * i)) as u8;
    }
    value
}

/// The documents of one case, mirroring `PointsFixture.documents` exactly.
fn documents(case: &str) -> Vec<Vec<Pt>> {
    let mut docs = Vec::new();
    match case {
        "int1d" => {
            for doc in 0..1300 {
                docs.push(vec![Pt::Ints("p", vec![(doc * 7919) % 1000])]);
            }
        }
        "multi1d" => {
            for doc in 0..400 {
                if doc % 3 == 0 {
                    docs.push(Vec::new());
                    continue;
                }
                let mut points = Vec::new();
                for k in 0..=(doc % 3) {
                    points.push(Pt::Ints("m", vec![doc * 10 + k]));
                }
                docs.push(points);
            }
        }
        "long1d" => {
            for doc in 0..700i64 {
                let value = (doc * 1_000_000_007i64) % 4_000_000_000i64 - 2_000_000_000i64;
                docs.push(vec![Pt::Longs("l", vec![value])]);
            }
        }
        "bin1d" => {
            for doc in 0..300 {
                docs.push(vec![Pt::Binary("b", binary16(doc))]);
            }
        }
        "small" => {
            for doc in 0..7 {
                docs.push(vec![Pt::Ints("s", vec![7 - doc])]);
            }
        }
        "mixed" => {
            let names = ["mnum", "mbin", "msort", "msnum", "mss"];
            for doc in 0..600 {
                let mut points = Vec::new();
                for (f, name) in names.iter().enumerate() {
                    let f = f as i32;
                    if doc % (f + 1) == 0 {
                        points.push(Pt::Ints(name, vec![(doc * (f * 31 + 7)) % 997]));
                    }
                }
                docs.push(points);
            }
        }
        "nd2" => {
            for doc in 0..900 {
                docs.push(vec![Pt::Ints(
                    "g",
                    vec![(doc * 7919) % 601, (doc * 104729) % 397],
                )]);
            }
        }
        "ndmulti" => {
            for doc in 0..400 {
                docs.push(vec![
                    Pt::Ints("m2", vec![(doc * 7919) % 3, (doc * 104729) % 3]),
                    Pt::Ints("m2", vec![(doc * 7919) % 3, ((doc * 104729) + 1) % 3]),
                ]);
            }
        }
        "nddeep" => {
            for doc in 0..12000 {
                let v = (doc * 7919) % 1009;
                docs.push(vec![Pt::Ints("d3", vec![v, v, v])]);
            }
        }
        "ndsplit" => {
            // The same xorshift64* the Java fixture runs, so both sides index
            // the identical 513 points.
            let mut state: u64 = 1;
            for _ in 0..513 {
                let mut values = Vec::with_capacity(2);
                for _ in 0..2 {
                    state ^= state >> 12;
                    state ^= state << 25;
                    state ^= state >> 27;
                    values.push((state.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 33) as i32);
                }
                docs.push(vec![Pt::Ints("s2", values)]);
            }
        }
        "ndsel" => {
            for doc in 0..400 {
                docs.push(vec![Pt::Selective(
                    "j",
                    vec![(doc * 7919) % 211, (doc * 104729) % 17, (doc * 2654435) % 5],
                    1,
                )]);
            }
        }
        "ndsort" => {
            for doc in 0..200 {
                let d0 = if doc == 0 { 200 } else { 1 + (doc % 3) };
                let d1 = if doc == 0 { 1 } else { 1 + ((doc * 7) % 3) };
                docs.push(vec![Pt::Ints("k", vec![d0, d1, 7])]);
            }
        }
        "ndleaf" => {
            for doc in 0..300 {
                docs.push(vec![Pt::Ints(
                    "h",
                    vec![(doc * 7919) % 53, (doc * 104729) % 29, (doc * 2654435) % 11],
                )]);
            }
        }
        other => panic!("unknown case: {other}"),
    }
    docs
}

/// The inclusive range each case queries, mirroring `PointsFixture.range`.
fn range(case: &str) -> (Vec<u8>, Vec<u8>) {
    match case {
        "int1d" => (pack_ints(&[250]), pack_ints(&[750])),
        "multi1d" => (pack_ints(&[1000]), pack_ints(&[2500])),
        "long1d" => (pack_longs(&[-500_000_000]), pack_longs(&[500_000_000])),
        "bin1d" => (binary16(0), binary16(299)),
        "small" => (pack_ints(&[3]), pack_ints(&[6])),
        "mixed" => (pack_ints(&[100]), pack_ints(&[400])),
        "nd2" => (pack_ints(&[100, 50]), pack_ints(&[500, 300])),
        "ndleaf" => (pack_ints(&[5, 3, 2]), pack_ints(&[40, 25, 9])),
        "ndsort" => (pack_ints(&[1, 1, 7]), pack_ints(&[2, 2, 7])),
        // Only dimension 0 is indexed, so the range has one dimension.
        "ndsel" => (pack_ints(&[40]), pack_ints(&[160])),
        "ndmulti" => (pack_ints(&[0, 0]), pack_ints(&[1, 1])),
        "nddeep" => (pack_ints(&[200, 200, 200]), pack_ints(&[800, 800, 800])),
        "ndsplit" => (
            pack_ints(&[200_000_000, 200_000_000]),
            pack_ints(&[800_000_000, 800_000_000]),
        ),
        other => panic!("unknown case: {other}"),
    }
}

fn pack_ints(values: &[i32]) -> Vec<u8> {
    let mut packed = vec![0u8; values.len() * 4];
    for (dim, value) in values.iter().enumerate() {
        IntPoint::encode_dimension(*value, &mut packed, dim * 4);
    }
    packed
}

fn pack_longs(values: &[i64]) -> Vec<u8> {
    let mut packed = vec![0u8; values.len() * 8];
    for (dim, value) in values.iter().enumerate() {
        LongPoint::encode_dimension(*value, &mut packed, dim * 8);
    }
    packed
}

// ---------------------------------------------------------------------------
// Rucene side
// ---------------------------------------------------------------------------

fn ensure_codec() -> Arc<dyn Codec> {
    let _ = register_codec("Lucene104", Lucene104Codec::new());
    rucene::codecs::default_codec().expect("Lucene104 codec is registered")
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Collects every visited point, as `PointsFixture.dump` prints them.
struct DumpVisitor {
    field: String,
    lines: Vec<String>,
}

impl IntersectVisitor for DumpVisitor {
    fn visit(&mut self, _doc_id: i32) -> rucene::error::Result<()> {
        panic!("a CELL_CROSSES_QUERY visitor never gets bare doc IDs");
    }

    fn visit_with_value(&mut self, doc_id: i32, packed_value: &[u8]) -> rucene::error::Result<()> {
        self.lines.push(format!(
            "point field={} doc={} value={}",
            self.field,
            doc_id,
            hex(packed_value)
        ));
        Ok(())
    }

    fn compare(&self, _min: &[u8], _max: &[u8]) -> Relation {
        Relation::CellCrossesQuery
    }
}

/// A pruning box intersection, mirroring `PointsFixture.dumpRange`.
struct RangeVisitor {
    lower: Vec<u8>,
    upper: Vec<u8>,
    num_index_dims: usize,
    bytes_per_dim: usize,
    accepted: Vec<i32>,
}

impl RangeVisitor {
    fn dim<'a>(&self, value: &'a [u8], dim: usize) -> &'a [u8] {
        let offset = dim * self.bytes_per_dim;
        &value[offset..offset + self.bytes_per_dim]
    }
}

impl IntersectVisitor for RangeVisitor {
    fn visit(&mut self, doc_id: i32) -> rucene::error::Result<()> {
        self.accepted.push(doc_id);
        Ok(())
    }

    fn visit_with_value(&mut self, doc_id: i32, packed_value: &[u8]) -> rucene::error::Result<()> {
        for dim in 0..self.num_index_dims {
            let value = self.dim(packed_value, dim);
            if value < self.dim(&self.lower, dim) || value > self.dim(&self.upper, dim) {
                return Ok(());
            }
        }
        self.accepted.push(doc_id);
        Ok(())
    }

    fn compare(&self, min: &[u8], max: &[u8]) -> Relation {
        let mut crosses = false;
        for dim in 0..self.num_index_dims {
            let lower = self.dim(&self.lower, dim);
            let upper = self.dim(&self.upper, dim);
            if self.dim(max, dim) < lower || self.dim(min, dim) > upper {
                return Relation::CellOutsideQuery;
            }
            crosses |= self.dim(min, dim) < lower || self.dim(max, dim) > upper;
        }
        if crosses {
            Relation::CellCrossesQuery
        } else {
            Relation::CellInsideQuery
        }
    }
}

/// Renders the points of every field of one segment exactly as
/// `PointsFixture.dump` and `PointsFixture.dumpRange` do, so the two sides
/// compare as plain strings.
fn dump(reader: &dyn PointsReader, field_infos: &FieldInfos, case: &str) -> Vec<String> {
    let (lower, upper) = range(case);
    let mut lines = Vec::new();
    for info in field_infos.iter() {
        if info.point_dimension_count == 0 {
            continue;
        }
        let values = reader.get_values(&info.name).expect("point values");
        lines.push(format!(
            "pointstats field={} size={} docCount={} numDims={} numIndexDims={} bytesPerDim={} min={} max={}",
            info.name,
            values.size(),
            values.doc_count(),
            values.num_dimensions().expect("dims"),
            values.num_index_dimensions().expect("index dims"),
            values.bytes_per_dimension().expect("bytes per dim"),
            hex(&values.min_packed_value().expect("min").expect("a non-empty field")),
            hex(&values.max_packed_value().expect("max").expect("a non-empty field")),
        ));
        let mut dump_visitor = DumpVisitor {
            field: info.name.clone(),
            lines: Vec::new(),
        };
        values.intersect(&mut dump_visitor).expect("intersect");
        lines.extend(dump_visitor.lines);

        let mut range_visitor = RangeVisitor {
            lower: lower.clone(),
            upper: upper.clone(),
            num_index_dims: info.point_index_dimension_count as usize,
            bytes_per_dim: info.point_num_bytes as usize,
            accepted: Vec::new(),
        };
        values
            .intersect(&mut range_visitor)
            .expect("range intersect");
        range_visitor.accepted.sort_unstable();
        range_visitor.accepted.dedup();
        lines.push(format!(
            "range field={} docs={}",
            info.name,
            range_visitor
                .accepted
                .iter()
                .map(|doc| doc.to_string())
                .collect::<Vec<_>>()
                .join(",")
        ));
    }
    lines
}

/// Reads the points of a Lucene-written index with Rucene's own reader.
fn read_java_index(dir: &Path, case: &str) -> Vec<String> {
    let codec = ensure_codec();
    let directory: Arc<dyn Directory> = Arc::new(FSDirectory::open(dir).expect("directory"));
    let infos = SegmentInfos::read_latest_commit(directory.as_ref()).expect("segments file");
    let segment_info = infos.info(0).info.clone();
    let field_infos = codec
        .field_infos_format()
        .read(directory.as_ref(), &segment_info, "", &*DEFAULT_IO_CONTEXT)
        .expect("field infos");
    let read_state = SegmentReadState::new(
        directory.as_ref(),
        &segment_info,
        &field_infos,
        &*DEFAULT_IO_CONTEXT,
    );
    let reader = codec
        .points_format()
        .fields_reader(&read_state)
        .expect("points reader");
    reader.check_integrity().expect("integrity");
    dump(reader.as_ref(), &field_infos, case)
}

/// Writes the case through Rucene's indexing chain, producing one segment.
fn write_with_rucene(
    out_dir: &Path,
    segment_name: &str,
    segment_id: [u8; 16],
    documents: &[Vec<Pt>],
) -> FieldInfos {
    let codec = ensure_codec();

    // The config requires an analyzer, but no field here is tokenized: point
    // fields never consult it.
    let analyzer: Arc<dyn Analyzer> = Arc::new(StandardAnalyzer::new());
    let live = Arc::new(LiveIndexWriterConfig::new(analyzer));
    let directory: Box<dyn Directory> = Box::new(FSDirectory::open(out_dir).expect("directory"));
    let tracking = Arc::new(TrackingDirectoryWrapper::new(directory));
    let make_info = |max_doc: i32| {
        SegmentInfo::new(
            Arc::new(FSDirectory::open(out_dir).expect("directory")),
            Version::LATEST,
            Some(Version::LATEST),
            segment_name.to_string(),
            max_doc,
            false,
            false,
            Arc::clone(&codec),
            HashMap::new(),
            segment_id,
            HashMap::new(),
            Default::default(),
        )
        .expect("segment info")
    };

    let indexing_info = make_info(-1);
    let mut chain = DefaultIndexingChain::new_for_segment(
        Arc::clone(&live),
        Arc::clone(&tracking),
        &indexing_info,
    )
    .expect("bind segment");

    let numbers = Arc::new(FieldNumbers::new(None, None).expect("field numbers"));
    let mut field_infos = FieldInfosBuilder::new(numbers);
    for (doc_id, points) in documents.iter().enumerate() {
        let mut document = Document::new();
        for point in points {
            match point {
                Pt::Ints(name, values) => {
                    document.add(Box::new(IntPoint::new(name, values).expect("int point")))
                }
                Pt::Selective(name, values, index_dims) => {
                    // Lucene has no ready-made point class for selective
                    // indexing either: `IntPoint` always indexes every
                    // dimension, so the field type is built by hand with a
                    // smaller index dimension count.
                    let mut field_type = FieldType::new();
                    field_type
                        .set_dimensions_with_index_count(values.len() as i32, *index_dims, 4)
                        .expect("selective dimensions");
                    field_type.freeze();
                    let mut packed = vec![0u8; values.len() * 4];
                    for (dim, value) in values.iter().enumerate() {
                        IntPoint::encode_dimension(*value, &mut packed, dim * 4);
                    }
                    document.add(Box::new(
                        Field::new_with_bytes(name, BytesRef::new(packed), field_type)
                            .expect("selective point field"),
                    ))
                }
                Pt::Longs(name, values) => {
                    document.add(Box::new(LongPoint::new(name, values).expect("long point")))
                }
                Pt::Binary(name, value) => document.add(Box::new(
                    BinaryPoint::new(name, &[BytesRef::new(value.clone())]).expect("binary point"),
                )),
            }
        }
        chain
            .process_document(doc_id as i32, &document, true, &mut field_infos)
            .unwrap_or_else(|error| panic!("document {doc_id} must index cleanly: {error}"));
    }
    let finished = field_infos.finish().expect("field infos");

    let segment_info = make_info(documents.len() as i32);
    let info_stream = NoOutputInfoStream;
    let context = flush_io_context(FlushInfo::new(documents.len() as i32, 0));
    let state = IndexingChainFlushState {
        info_stream: &info_stream,
        directory: &tracking,
        segment_info: &segment_info,
        field_infos: &finished,
        context: context.as_ref(),
        live_docs: None,
        del_count_on_flush: 0,
        delete_terms: &[],
    };
    chain.flush(&state).expect("flush");
    finished
}

// ---------------------------------------------------------------------------
// Byte comparison
// ---------------------------------------------------------------------------

fn assert_points_bytes_equal(java_dir: &Path, rust_dir: &Path, segment: &str, case: &str) {
    if let Ok(keep) = std::env::var("RUCENE_POINTS_DEBUG_DIR") {
        let _ = std::fs::create_dir_all(&keep);
        for (side, from) in [("java", java_dir), ("rust", rust_dir)] {
            for entry in std::fs::read_dir(from).expect("segment directory") {
                let path = entry.expect("dir entry").path();
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    if name.starts_with(segment) {
                        let _ = std::fs::copy(&path, format!("{keep}/{side}_{case}_{name}"));
                    }
                }
            }
        }
    }
    let mut compared = 0;
    for extension in EXTENSIONS {
        let file_name = format!("{segment}.{extension}");
        let java_file = java_dir.join(&file_name);
        assert!(
            java_file.exists(),
            "[{case}] Lucene did not write {file_name}"
        );
        let rust_file = rust_dir.join(&file_name);
        assert!(
            rust_file.exists(),
            "[{case}] Rucene did not write {file_name}"
        );
        let expected = std::fs::read(&java_file).expect("read java file");
        let actual = std::fs::read(&rust_file).expect("read rust file");
        if expected != actual {
            let first = expected
                .iter()
                .zip(actual.iter())
                .position(|(left, right)| left != right)
                .unwrap_or_else(|| std::cmp::min(expected.len(), actual.len()));
            panic!(
                "[{case}] {file_name} differs at byte {first} (lucene {} bytes, rucene {} bytes)\n  lucene: {}\n  rucene: {}",
                expected.len(),
                actual.len(),
                hex_window(&expected, first),
                hex_window(&actual, first)
            );
        }
        compared += 1;
    }
    assert_eq!(
        compared,
        EXTENSIONS.len(),
        "[{case}] all three points files must be compared"
    );
}

fn hex_window(bytes: &[u8], centre: usize) -> String {
    let from = centre.saturating_sub(16);
    let to = std::cmp::min(centre + 16, bytes.len());
    let body: Vec<String> = bytes[from..to].iter().map(|b| format!("{b:02x}")).collect();
    format!("[{from}..{to}] {}", body.join(" "))
}

// ---------------------------------------------------------------------------
// The shared setup and the two assertion levels
// ---------------------------------------------------------------------------

/// What both assertion levels need: the Java segment, the Rucene segment and
/// the two directories they live in.
struct Comparison {
    segment: JavaSegment,
    field_infos: FieldInfos,
    java_tmp: tempfile::TempDir,
    rust_tmp: tempfile::TempDir,
}

fn run_case(case: &str) -> Comparison {
    require_maven();
    let java_tmp = tempfile::tempdir().expect("temp dir");
    let rust_tmp = tempfile::tempdir().expect("temp dir");

    let segment = run_java_fixture(java_tmp.path(), case).expect("java fixture");
    assert!(
        !segment.compound,
        "[{case}] the fixture must write loose files for a byte comparison"
    );
    let documents = documents(case);
    assert_eq!(
        segment.max_doc,
        documents.len() as i32,
        "[{case}] the two sides must index the same number of documents"
    );

    let field_infos = write_with_rucene(rust_tmp.path(), &segment.name, segment.id, &documents);

    // The field numbers key the `.kdm` entries, so they must agree before the
    // byte comparison can mean anything.
    let rust_field_infos: Vec<String> = field_infos
        .iter()
        .map(|info| {
            format!(
                "{} {} dims={} indexDims={} bytes={}",
                info.number,
                info.name,
                info.point_dimension_count,
                info.point_index_dimension_count,
                info.point_num_bytes
            )
        })
        .collect();
    assert_eq!(
        rust_field_infos, segment.field_infos,
        "[{case}] the field infos must agree before the point values can"
    );

    Comparison {
        segment,
        field_infos,
        java_tmp,
        rust_tmp,
    }
}

/// Runs one case through all three directions of the comparison.
fn assert_case_matches_lucene(case: &str) {
    let run = run_case(case);

    // 1. Rucene writes what Lucene writes.
    assert_points_bytes_equal(
        run.java_tmp.path(),
        run.rust_tmp.path(),
        &run.segment.name,
        case,
    );

    // 2. Rucene reads what Lucene wrote.
    assert_eq!(
        read_java_index(run.java_tmp.path(), case),
        run.segment.dump,
        "[{case}] Rucene must decode Lucene's points exactly as Lucene does"
    );

    // 3. Lucene reads what Rucene wrote.
    let java_read = read_with_java(
        run.rust_tmp.path(),
        &run.segment.name,
        run.segment.id,
        run.segment.max_doc,
        case,
        &run.field_infos,
    )
    .expect("lucene reads the rucene segment");
    assert_eq!(
        java_read, run.segment.dump,
        "[{case}] Lucene must decode Rucene's points exactly as it decodes its own"
    );
}

#[test]
fn one_dimensional_int_points_match_lucene() {
    // 1300 points over a 512-point leaf size: three leaves and a split level,
    // with values repeated so leaves share long common prefixes.
    assert_case_matches_lucene("int1d");
}

#[test]
fn multi_valued_and_sparse_points_match_lucene() {
    // One to three points per document and every third document with none, so
    // numPoints, numDocs and maxDoc are three different numbers.
    assert_case_matches_lucene("multi1d");
}

#[test]
fn eight_byte_points_match_lucene() {
    // Values on both sides of zero, so the sortable long encoding is what puts
    // them in order rather than the raw two's-complement bytes.
    assert_case_matches_lucene("long1d");
}

#[test]
fn sixteen_byte_points_match_lucene() {
    // PointValues.MAX_NUM_BYTES per dimension.
    assert_case_matches_lucene("bin1d");
}

#[test]
fn a_single_leaf_tree_matches_lucene() {
    // Seven documents: the root is also the only leaf.
    assert_case_matches_lucene("small");
}

#[test]
fn the_kdm_field_order_follows_the_field_hash() {
    // Five fields whose field-hash flush order is [3, 0, 1, 2, 4]. Decoded
    // from the `.kdm` Lucene itself writes, the first field number after the
    // 50-byte index header is 3; a port that flushed in field-number order
    // would put 0 there and every later entry at a different offset.
    assert_case_matches_lucene("mixed");
}

#[test]
fn two_dimensional_points_match_lucene() {
    // 900 documents over two indexed dimensions: two leaves and a split level,
    // so the split dimension byte and the per-dimension leaf bounds are all
    // exercised. Byte identity, not just content — see the module docs.
    assert_case_matches_lucene("nd2");
}

#[test]
fn multi_valued_multi_dimension_points_match_lucene() {
    // Regression: `MutablePointTreeReaderUtils.sortByDim` compares the sorted
    // dimension, then the *unindexed* data dimensions, then the doc ID
    // (`MutablePointTreeReaderUtils.java:88-140`). That is not a total order —
    // two points of the same document can tie while still differing in another
    // *indexed* dimension, which the leaf writes in full. Java breaks such a
    // tie with `IntroSorter`, which is unstable; this crate used a stable sort
    // and wrote the pair in the other order. Only a multi-valued field with two
    // or more indexed dimensions can produce the tie.
    assert_case_matches_lucene("ndmulti");
}

#[test]
fn a_deep_multi_dimension_tree_matches_lucene() {
    // Regression: above two index dimensions, `build` recomputes a node's exact
    // bounds every `SPLITS_BEFORE_EXACT_BOUNDS` splits, and never at the root
    // (`BKDWriter.java:1781-1786`). Without it `split` sees bounds inherited
    // from an ancestor — loose on every dimension that ancestor did not split
    // on — and picks a different dimension, which changes the whole subtree
    // below. Reaching the gate needs three index dimensions and a tree at least
    // four splits deep, so more than 8192 points at the default leaf size: no
    // shallower case here can express it.
    assert_case_matches_lucene("nddeep");
}

#[test]
fn a_partitioned_multi_dimension_tree_matches_lucene() {
    // Regression: 513 points over two indexed dimensions and a leaf size of 512
    // is exactly two leaves, so the tree has one partition and the leaf after it
    // starts at whatever point the *selection* left there. `BKDWriter.build`
    // then skips that point when it measures per-dimension byte cardinalities
    // (`BKDWriter.java:1688`), so the choice of sorted dimension depends on the
    // selection's internal arrangement — not just on which points ended up on
    // each side.
    //
    // Until `RadixSelector` was ported this crate sorted instead of selecting,
    // which leaves the smallest point first rather than an arbitrary one, and
    // the two implementations compressed the leaf on different dimensions. No
    // other case here has a partition above a leaf with values spread widely
    // enough for that to bite.
    assert_case_matches_lucene("ndsplit");
}

#[test]
fn selectively_indexed_points_match_lucene() {
    // Three data dimensions, one of them indexed. The tree splits on dimension
    // 0 only, while the leaves carry all three and the two unindexed ones are
    // what breaks ties inside a leaf — the one comparison that is empty in
    // every other case here.
    assert_case_matches_lucene("ndsel");
}

#[test]
fn the_leaf_sorted_dimension_is_chosen_as_lucene_chooses_it() {
    // Regression: `BKDWriter.build` over a MutablePointTree measures the
    // per-dimension byte cardinalities from `from + 1`, not from `from`
    // (`BKDWriter.java:1688`), so the first point of a leaf never contributes.
    // This port counted from `from`, which picked a different sorted dimension
    // whenever the first point carried a byte no other point in the leaf did —
    // a different compressed dimension byte and a different point order in the
    // `.kdd`. This case is built to be exactly that leaf.
    assert_case_matches_lucene("ndsort");
}

#[test]
fn a_single_leaf_multi_dimension_tree_matches_lucene() {
    // Three indexed dimensions over fewer documents than one leaf holds. This
    // is where the leaf's sorted-dimension choice is made with no partition
    // above it, and it is the shape no other case here expresses.
    assert_case_matches_lucene("ndleaf");
}

/// A [`PointsWriter`] that writes nothing and only records whether the tree it
/// was handed is the mutable one.
#[derive(Debug)]
struct FastPathProbe {
    mutable: bool,
}

impl PointsWriter for FastPathProbe {
    fn write_field(
        &mut self,
        field_info: &rucene::index::FieldInfo,
        values: &dyn PointsReader,
    ) -> rucene::error::Result<()> {
        let values = values.get_values(&field_info.name)?;
        let mut tree = values.point_tree()?;
        self.mutable = tree.as_mutable().is_some();
        Ok(())
    }

    fn finish(&mut self) -> rucene::error::Result<()> {
        Ok(())
    }

    fn close(&mut self) -> rucene::error::Result<()> {
        Ok(())
    }
}

#[test]
fn the_codec_takes_the_mutable_fast_path() {
    // Everything above rests on one branch: `Lucene90PointsWriter::write_field`
    // asks the tree for `PointTree::as_mutable` and, when it answers `Some`,
    // hands it to `BKDWriter::write_field` instead of replaying it through
    // `BKDWriter::add`. That is this port's `values instanceof MutablePointTree`
    // (`Lucene90PointsWriter.java:157`). If the flushing tree ever stopped
    // answering `Some`, every byte-identity test above would still pass —
    // through the slower path, and no longer through the one Lucene takes — so
    // the branch is pinned here rather than left implicit.
    let mut field_info = rucene::index::FieldInfo::new("j", 0);
    field_info.point_dimension_count = 3;
    field_info.point_index_dimension_count = 1;
    field_info.point_num_bytes = 4;

    let bytes_used = Arc::new(AtomicI64::new(0));
    let mut buffer = PointValuesWriter::new(field_info, bytes_used);
    for doc in 0..4 {
        let mut packed = vec![0u8; 12];
        for dim in 0..3 {
            IntPoint::encode_dimension(doc * 3 + dim as i32, &mut packed, dim * 4);
        }
        buffer
            .add_packed_value(doc, &BytesRef::new(packed))
            .expect("a well-formed value");
    }

    let mut probe = FastPathProbe { mutable: false };
    buffer.flush(&mut probe).expect("flush");
    assert!(
        probe.mutable,
        "the flushing tree must be the mutable one, or the codec silently \
         falls back to the generic add path"
    );
}
