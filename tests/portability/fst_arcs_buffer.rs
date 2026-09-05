//! Portability tests for `FSTCompiler.FixedLengthArcsBuffer`, for rmp task
//! #163.
//!
//! Every expected value here is **measured from `lucene-core-10.5.0.jar`** by
//! `org.apache.lucene.util.fst.RuceneFixedLengthArcsBufferFixture` in the same
//! run. The fixture lives in `org.apache.lucene.util.fst` because
//! `FixedLengthArcsBuffer` is a package-private nested class of `FSTCompiler`
//! (`FSTCompiler.java:1164`) with package-private methods, so nothing outside
//! the package can drive it.
//!
//! The buffer is the header scratch space for every fixed-length-arc node the
//! FST compiler writes — binary search, direct addressing and continuous alike
//! — so its bytes end up verbatim in the `.tim` term dictionary and in every
//! other FST the codec writes. Nothing in this crate's test suite exercised it
//! before this file: its only callers are two private methods of the compiler,
//! reached only by nodes with enough arcs to be worth expanding.
//!
//! What is pinned:
//!
//! 1. The initial array: eleven zero bytes, position zero. Eleven is exactly
//!    one flags byte plus two five-byte `VInt`s, which the widest-header case
//!    at the end confirms by filling it exactly.
//! 2. `ensureCapacity` over nine capacities, against `ArrayUtil.oversize`
//!    measured beside it — the array grows to `oversize(capacity, 1)`, and
//!    does not shrink or grow when the capacity already fits.
//! 3. That a growth **replaces** the array: the bytes written so far are gone,
//!    the new array is zeroed, and the position is reset to zero, because Java
//!    reseats its `ByteArrayDataOutput` over the new array
//!    (`FSTCompiler.java:1175`). A growth that did not reset the position
//!    would be invisible at the compiler's two call sites, which reset it
//!    themselves a few lines later; this test is the only thing that sees it.
//! 4. That an `ensureCapacity` which does **not** grow leaves the array, the
//!    bytes and the position untouched.
//! 5. `resetPosition`, and that a write after it overwrites from the start
//!    while leaving the rest of the array as it was.
//! 6. The node header itself — flags byte, then two `VInt`s — over all
//!    eighty-one combinations of nine values spanning every `VInt` width, plus
//!    the negative `-1` that encodes as the full five bytes.
//! 7. Three whole FSTs, built by `FSTCompiler` on both sides from the same
//!    terms and compared **byte for byte** after `FST.save`, so the header the
//!    buffer produces is checked where it actually lands. The three term sets
//!    are chosen to make the compiler pick each of the three fixed-length-arc
//!    node kinds in turn — `CONTINUOUS`, `BINARY_SEARCH` and
//!    `DIRECT_ADDRESSING` — and the node-kind counters are asserted alongside
//!    the bytes, so a run that agreed on the bytes by taking a different route
//!    would still be caught.

use std::path::PathBuf;
use std::process::{Command, Stdio};

use rucene::store::{ByteBuffersDataOutput, DataOutput};
use rucene::util::fst::{Builder, FixedLengthArcsBuffer, InputType, PositiveIntOutputs, Util, FST};
use rucene::util::{ArrayUtil, BytesRef, IntsRefBuilder};

const CAPACITIES: [usize; 9] = [1, 10, 11, 12, 13, 20, 100, 1000, 65536];
const VINTS: [i32; 9] = [0, 1, 127, 128, 16383, 16384, 2097151, 2097152, 268435455];

// ---------------------------------------------------------------------------
// Harness
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

fn run_fixture() -> Measured {
    let mvn = which_mvn().unwrap_or_else(|e| {
        panic!("the FixedLengthArcsBuffer portability test needs Maven and a JDK: {e}")
    });
    let output = Command::new(mvn)
        .arg("-q")
        .arg("compile")
        .arg("exec:java")
        .arg("-Dexec.mainClass=org.apache.lucene.util.fst.RuceneFixedLengthArcsBufferFixture")
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
    Measured(
        stdout
            .lines()
            .filter_map(|line| line.trim().split_once('='))
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
    )
}

struct Measured(Vec<(String, String)>);

impl Measured {
    fn one(&self, key: &str) -> String {
        self.0
            .iter()
            .find(|(k, _)| k == key)
            .unwrap_or_else(|| panic!("the fixture printed no {key}"))
            .1
            .clone()
    }
}

/// Renders `bytes[..len]` the way the fixture renders a Java `byte[]`.
fn hex(bytes: &[u8], len: usize) -> String {
    bytes[..len].iter().map(|b| format!("{b:02x}")).collect()
}

// ---------------------------------------------------------------------------
// The test
// ---------------------------------------------------------------------------

#[test]
fn fixed_length_arcs_buffer_matches_lucene_1050() {
    let m = run_fixture();

    // ---- the field initialiser, FSTCompiler.java:1168 ----------------------
    let fresh = FixedLengthArcsBuffer::new();
    assert_eq!(fresh.bytes().len().to_string(), m.one("initial.length"));
    assert_eq!(fresh.position().to_string(), m.one("initial.position"));
    assert_eq!(
        hex(fresh.bytes(), fresh.bytes().len()),
        m.one("initial.bytes"),
        "a Java `new byte[11]` is zero-filled"
    );

    // ---- ensureCapacity, FSTCompiler.java:1172-1178 ------------------------
    for capacity in CAPACITIES {
        let mut buffer = FixedLengthArcsBuffer::new();
        buffer.ensure_capacity(capacity);
        assert_eq!(
            buffer.bytes().len().to_string(),
            m.one(&format!("ensure_capacity.{capacity}.length")),
            "ensureCapacity({capacity})"
        );
        // Pinned beside it so the growth policy is attributable: the array
        // grows to `ArrayUtil.oversize(capacity, Byte.BYTES)`, and this port's
        // `oversize` must agree before its `ensure_capacity` can.
        assert_eq!(
            ArrayUtil::oversize(capacity, 1).to_string(),
            m.one(&format!("oversize.{capacity}")),
            "ArrayUtil.oversize({capacity}, 1)"
        );
    }

    // ---- a growth replaces the array and resets the position ---------------
    let mut grown = FixedLengthArcsBuffer::new();
    grown.write_byte(0xAB).expect("fits in eleven bytes");
    grown.write_byte(0xCD).expect("fits in eleven bytes");
    grown.write_byte(0xEF).expect("fits in eleven bytes");
    assert_eq!(grown.position().to_string(), m.one("grow.position_before"));
    assert_eq!(hex(grown.bytes(), 3), m.one("grow.bytes_before"));
    grown.ensure_capacity(100);
    assert_eq!(
        grown.position().to_string(),
        m.one("grow.position_after"),
        "a growth reseats the output over the new array (`bado.reset(bytes)`), \
         which puts the position back to zero"
    );
    assert_eq!(grown.bytes().len().to_string(), m.one("grow.length_after"));
    assert_eq!(
        hex(grown.bytes(), 3),
        m.one("grow.bytes_after"),
        "the new array is a fresh, zeroed one; the old bytes are not carried over"
    );

    // ---- an ensureCapacity that does not grow changes nothing --------------
    let mut kept = FixedLengthArcsBuffer::new();
    kept.write_byte(0xAB).expect("fits");
    kept.write_byte(0xCD).expect("fits");
    kept.write_byte(0xEF).expect("fits");
    kept.ensure_capacity(5);
    assert_eq!(kept.position().to_string(), m.one("nogrow.position_after"));
    assert_eq!(kept.bytes().len().to_string(), m.one("nogrow.length_after"));
    assert_eq!(hex(kept.bytes(), 3), m.one("nogrow.bytes_after"));

    // ---- resetPosition, FSTCompiler.java:1180-1183 -------------------------
    let mut reset = FixedLengthArcsBuffer::new();
    reset.write_byte(0x11).expect("fits");
    reset.write_byte(0x22).expect("fits");
    reset.reset_position();
    reset.write_byte(0x33).expect("fits");
    assert_eq!(reset.position().to_string(), m.one("reset.position"));
    assert_eq!(
        hex(reset.bytes(), 2),
        m.one("reset.bytes"),
        "the write after the reset overwrites byte 0 and leaves byte 1 as it was"
    );

    // ---- the node header the compiler builds -------------------------------
    let mut header = FixedLengthArcsBuffer::new();
    for first in VINTS {
        for second in VINTS {
            header.reset_position();
            header.write_byte(0x20).expect("fits in eleven bytes");
            header.write_v_int(first).expect("fits in eleven bytes");
            header.write_v_int(second).expect("fits in eleven bytes");
            let position = header.position();
            assert_eq!(
                format!("{position}:{}", hex(header.bytes(), position)),
                m.one(&format!("header.{first}.{second}")),
                "header for ({first}, {second})"
            );
        }
    }

    // ---- the widest header the eleven-byte array holds ---------------------
    let mut widest = FixedLengthArcsBuffer::new();
    widest.reset_position();
    widest.write_byte(0x20).expect("fits");
    widest.write_v_int(-1).expect("fits");
    widest.write_v_int(-1).expect("fits");
    let position = widest.position();
    assert_eq!(position.to_string(), m.one("widest.position"));
    assert_eq!(
        hex(widest.bytes(), position),
        m.one("widest.bytes"),
        "a negative int is five VInt bytes; two of them plus the flags byte are \
         exactly the eleven the buffer starts with"
    );
}

// ---------------------------------------------------------------------------
// Whole FSTs
// ---------------------------------------------------------------------------

/// Every prefix leads to the same three-way suffix, so the FST folds into a
/// handful of shared nodes and the root becomes a wide, contiguous node — the
/// `CONTINUOUS` header.
fn continuous_terms() -> Vec<(Vec<u8>, i64)> {
    let mut terms = Vec::new();
    for a in b'a'..=b'z' {
        for b in b'a'..=b'z' {
            for c in [b'0', b'5', b'9'] {
                terms.push((vec![a, b, c], a as i64 * 1000 + b as i64 * 10 + c as i64));
            }
        }
    }
    terms
}

/// First labels spread over almost the whole byte range: direct addressing
/// would need too many presence bytes, so the node falls back to
/// `BINARY_SEARCH`.
fn sparse_terms() -> Vec<(Vec<u8>, i64)> {
    let mut terms = Vec::new();
    for (i, first) in [1u8, 50, 100, 150, 200, 250].into_iter().enumerate() {
        for j in 0..8u8 {
            terms.push((vec![first, b'x', j], (i as i64 + 1) * 97 + j as i64));
        }
    }
    terms
}

/// Nearly contiguous first labels with a single gap: `DIRECT_ADDRESSING`.
fn gapped_terms() -> Vec<(Vec<u8>, i64)> {
    let mut terms = Vec::new();
    for (i, first) in [10u8, 11, 12, 13, 14, 16, 17, 18, 19, 20, 21]
        .into_iter()
        .enumerate()
    {
        for j in 0..8u8 {
            terms.push((vec![first, b'y', j], (i as i64 + 3) * 31 + j as i64));
        }
    }
    terms
}

/// Builds the FST and asserts its node-kind counters and its saved bytes
/// against the ones the fixture measured from Lucene.
///
/// The terms arrive in ascending unsigned byte order and are not sorted here,
/// so neither side has to agree on a comparator: Java's `byte` is signed and
/// Rust's `u8` is not, and a first label of 250 would sort differently under
/// the two.
fn assert_fst_matches(m: &Measured, name: &str, terms: Vec<(Vec<u8>, i64)>) {
    let mut compiler = Builder::new(InputType::Byte1, PositiveIntOutputs::get_singleton())
        .build()
        .expect("the compiler builds");
    let mut scratch = IntsRefBuilder::new();
    for (term, output) in &terms {
        let input = Util::to_ints_ref(&BytesRef::new(term.clone()), &mut scratch);
        compiler.add(&input, *output).expect("terms are in order");
    }
    let metadata = compiler.compile().expect("the FST compiles");

    assert_eq!(
        compiler.node_count().to_string(),
        m.one(&format!("fst.{name}.node_count")),
        "{name}: node count"
    );
    assert_eq!(
        compiler.arc_count().to_string(),
        m.one(&format!("fst.{name}.arc_count")),
        "{name}: arc count"
    );
    assert_eq!(
        compiler.binary_search_node_count().to_string(),
        m.one(&format!("fst.{name}.binary_search")),
        "{name}: BINARY_SEARCH node count"
    );
    assert_eq!(
        compiler.direct_addressing_node_count().to_string(),
        m.one(&format!("fst.{name}.direct_addressing")),
        "{name}: DIRECT_ADDRESSING node count"
    );
    assert_eq!(
        compiler.continuous_node_count().to_string(),
        m.one(&format!("fst.{name}.continuous")),
        "{name}: CONTINUOUS node count"
    );

    let fst = FST::from_fst_reader(metadata, compiler.into_fst_reader().expect("reader"))
        .expect("the term set is not empty");
    let mut meta = ByteBuffersDataOutput::new();
    let mut data = ByteBuffersDataOutput::new();
    fst.save(&mut meta, &mut data).expect("save");
    let meta_bytes = meta.to_array_copy();
    let data_bytes = data.to_array_copy();
    assert_eq!(
        hex(&meta_bytes, meta_bytes.len()),
        m.one(&format!("fst.{name}.meta")),
        "{name}: FSTMetadata bytes"
    );
    assert_eq!(
        hex(&data_bytes, data_bytes.len()),
        m.one(&format!("fst.{name}.data")),
        "{name}: FST body bytes"
    );

    // A byte-identical FST that does not answer the same questions would be a
    // contradiction, but the round trip costs nothing and names the failure.
    for (term, output) in &terms {
        assert_eq!(
            Util::get_bytes(&fst, &BytesRef::new(term.clone())).expect("lookup"),
            Some(*output),
            "{name}: {term:?} did not round-trip"
        );
    }
}

#[test]
fn fixed_length_arc_nodes_match_lucene_1050() {
    let m = run_fixture();
    assert_fst_matches(&m, "continuous", continuous_terms());
    assert_fst_matches(&m, "sparse", sparse_terms());
    assert_fst_matches(&m, "gapped", gapped_terms());
}
