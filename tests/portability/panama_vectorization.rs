//! Panama vectorization and FMA gates against Apache Lucene Core 10.5.0 (rmp
//! task #319).
//!
//! The oracle is `PanamaVectorizationFixture.java`, launched from source
//! against `lucene-core-10.5.0.jar`. This test writes a seeded, generated
//! corpus — float, byte, int4, packed and bit vectors over many dimensions
//! (every length up to 72, then lengths around the lane, unroll and int4 chunk
//! boundaries), NaN, infinities, signed zeros, subnormals and extreme values
//! among the floats, out-of-range bytes for the int4 kernels — and runs it
//! through:
//!
//! * `PanamaVectorUtilSupport` (Java with `--add-modules jdk.incubator.vector`;
//!   Rucene's port constructed directly), every `VectorUtilSupport` method,
//!   plus `MemorySegmentBulkVectorOps` (bulk with heap and segment query, and
//!   the single-pair form), `PanamaDocValuesRangeSupport` and
//!   `FixedBitSet.orMask`, under the four combinations of
//!   `lucene.useVectorFMA` and `lucene.useScalarFMA`;
//! * `DefaultVectorUtilSupport` (Java without the vector module), under both
//!   values of `lucene.useScalarFMA`.
//!
//! Java runs the corpus 40 times, so that C2 compiles the kernels, and prints
//! the last pass after checking it equals the one before. Rucene runs in a
//! child process of this test binary with the same properties as environment
//! variables (its gates are read once per process, as Java's `static final`
//! fields are). The two reports — the gate values, then one line per
//! operation with every float as its `floatToIntBits` — must be identical.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use rucene::internal::vectorization::panama_vector_util_support::use_vector_fma;
use rucene::internal::vectorization::{
    DefaultVectorUtilSupport, DocValuesRangeSupport, MemorySegmentBulkVectorOps,
    PanamaDocValuesRangeSupport, PanamaVectorConstants, PanamaVectorUtilSupport, VectorUtilSupport,
};
use rucene::store::java_nio_file::{self as nio, FileChannel, FileSystems, OpenOption};
use rucene::store::memory_segment::SharedArena;
use rucene::store::{Arena, MemorySegment};
use rucene::util::{vector_util, Constants, FixedBitSet, LongValues};

/// Passes Java runs over the corpus before its results count.
const JAVA_PASSES: &str = "40";

/// Environment variable that turns [`oracle_child`] into the Rucene side.
const CHILD_ENV: &str = "RUCENE_PANAMA_ORACLE";

// ---------------------------------------------------------------------------
// Corpus
// ---------------------------------------------------------------------------

/// SplitMix64.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }

    fn range(&mut self, lo: i64, hi: i64) -> i64 {
        lo + self.below((hi - lo + 1) as u64) as i64
    }

    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// A float of the given kind: 0 uniform in [-1, 1), 1 spread over 40
    /// binary orders of magnitude, 2 uniform with one value in eight special.
    fn float(&mut self, kind: u32) -> f32 {
        const SPECIAL: [f32; 10] = [
            f32::NAN,
            f32::INFINITY,
            f32::NEG_INFINITY,
            0.0,
            -0.0,
            1.0e-45,
            1.0e-40,
            f32::MAX,
            -f32::MAX,
            f32::MIN_POSITIVE,
        ];
        match kind {
            0 => (self.unit() * 2.0 - 1.0) as f32,
            1 => {
                let sign = if self.below(2) == 0 { 1.0 } else { -1.0 };
                let exp = self.range(-20, 20) as i32;
                (sign * (1.0 + self.unit()) * 2f64.powi(exp)) as f32
            }
            _ => {
                if self.below(8) == 0 {
                    SPECIAL[self.below(SPECIAL.len() as u64) as usize]
                } else {
                    (self.unit() * 2.0 - 1.0) as f32
                }
            }
        }
    }

    fn floats(&mut self, n: usize, kind: u32) -> Vec<f32> {
        (0..n).map(|_| self.float(kind)).collect()
    }

    /// Bytes: kind 0 any value, kind 1 in [0, 15].
    fn bytes(&mut self, n: usize, kind: u32) -> Vec<u8> {
        (0..n)
            .map(|_| {
                if kind == 0 {
                    self.next() as u8
                } else {
                    self.below(16) as u8
                }
            })
            .collect()
    }
}

fn hex_floats(v: &[f32]) -> String {
    if v.is_empty() {
        return "-".into();
    }
    v.iter()
        .map(|x| format!("{:x}", x.to_bits()))
        .collect::<Vec<_>>()
        .join(",")
}

fn hex_bytes(v: &[u8]) -> String {
    if v.is_empty() {
        return "-".into();
    }
    v.iter().map(|b| format!("{b:02x}")).collect()
}

fn join<T: std::fmt::Display>(v: &[T]) -> String {
    if v.is_empty() {
        return "-".into();
    }
    v.iter()
        .map(|x| x.to_string())
        .collect::<Vec<_>>()
        .join(",")
}

/// Writes the corpus; `vector` adds the operations only the Panama provider
/// has (bulk vector ops and the doc-values range support).
fn corpus(vector: bool) -> String {
    let mut rng = Rng(0x5EED_0319);
    let mut out = String::new();
    let mut float_dims: Vec<usize> = (1..=72).collect();
    float_dims.extend([
        95, 96, 97, 127, 128, 129, 255, 256, 257, 383, 384, 385, 512, 768, 1000, 1024, 1536,
    ]);
    // The scalar float kernels of Rucene's `DefaultVectorUtilSupport` carry
    // `VectorUtil`'s `assert Float.isFinite(r)` as a `debug_assert!`, so the
    // scalar corpus keeps its float similarities finite.
    let float_kinds = if vector { 0..3 } else { 0..2 };
    for &dim in &float_dims {
        for kind in float_kinds.clone() {
            let a = rng.floats(dim, kind);
            let b = rng.floats(dim, kind);
            writeln!(out, "F {} {}", hex_floats(&a), hex_floats(&b)).unwrap();
        }
    }
    let mut byte_dims: Vec<usize> = (0..=72).collect();
    byte_dims.extend([
        96, 127, 128, 129, 255, 256, 257, 1000, 1024, 4095, 4096, 4097, 4128, 8192, 8200, 9000,
    ]);
    for &dim in &byte_dims {
        for kind in 0..2 {
            let a = rng.bytes(dim, kind);
            let b = rng.bytes(dim, kind);
            writeln!(out, "B {} {}", hex_bytes(&a), hex_bytes(&b)).unwrap();
            let unpacked = rng.bytes(2 * dim, kind);
            let packed = rng.bytes(dim, 0);
            writeln!(out, "P {} {}", hex_bytes(&unpacked), hex_bytes(&packed)).unwrap();
        }
    }
    let mut bit_dims: Vec<usize> = (0..=40).collect();
    bit_dims.extend([63, 64, 65, 100, 128, 129, 1000]);
    for &d in &bit_dims {
        let q = rng.bytes(4 * d, 0);
        let dv = rng.bytes(d, 0);
        writeln!(out, "Q {} {}", hex_bytes(&q), hex_bytes(&dv)).unwrap();
        let q = rng.bytes(2 * d, 0);
        let dv = rng.bytes(d, 0);
        writeln!(out, "D {} {}", hex_bytes(&q), hex_bytes(&dv)).unwrap();
    }
    for _ in 0..300 {
        let len = rng.below(301) as usize;
        let mut v = Vec::with_capacity(len);
        let mut x = rng.range(-50, 50) as i32;
        for _ in 0..len {
            x += rng.range(0, 5) as i32;
            v.push(x);
        }
        let (lo, hi) = (
            v.first().copied().unwrap_or(0),
            v.last().copied().unwrap_or(0),
        );
        let target = rng.range(i64::from(lo) - 5, i64::from(hi) + 5);
        let from = rng.below(len as u64 + 1) as usize;
        let to = from + rng.below((len - from) as u64 + 1) as usize;
        writeln!(out, "G {target} {from} {to} {}", join(&v)).unwrap();
    }
    let mut quant_dims: Vec<usize> = (1..=70).collect();
    quant_dims.extend([128, 255, 256, 257, 1000]);
    for &dim in &quant_dims {
        for kind in [0, 2] {
            let min_q = -(0.1 + rng.unit() * 0.9) as f32;
            let max_q = (0.1 + rng.unit() * 0.9) as f32;
            let scale = 127f32 / (max_q - min_q);
            let alpha = (max_q - min_q) / 127f32;
            let v = rng.floats(dim, kind);
            writeln!(
                out,
                "M {:x} {:x} {:x} {:x} {}",
                scale.to_bits(),
                alpha.to_bits(),
                min_q.to_bits(),
                max_q.to_bits(),
                hex_floats(&v)
            )
            .unwrap();
            let old_alpha = (rng.unit() * 0.02) as f32;
            let old_min = -(rng.unit()) as f32;
            let bytes = rng.bytes(dim, 0);
            writeln!(
                out,
                "R {:x} {:x} {:x} {:x} {:x} {:x} {}",
                old_alpha.to_bits(),
                old_min.to_bits(),
                scale.to_bits(),
                alpha.to_bits(),
                min_q.to_bits(),
                max_q.to_bits(),
                hex_bytes(&bytes)
            )
            .unwrap();
        }
    }
    for _ in 0..200 {
        let up_to = rng.below(81) as usize;
        let len = up_to + rng.below(9) as usize;
        let docs: Vec<i32> = (0..len).map(|_| rng.range(0, 100_000) as i32).collect();
        let scores: Vec<String> = (0..len)
            .map(|_| {
                let s = if rng.below(10) == 0 {
                    f64::NAN
                } else {
                    rng.unit() * 2.0 - 0.5
                };
                format!("{:x}", s.to_bits())
            })
            .collect();
        let min = rng.unit() * 1.4 - 0.2;
        let scores = if scores.is_empty() {
            "-".to_string()
        } else {
            scores.join(",")
        };
        writeln!(
            out,
            "S {:x} {up_to} {} {scores}",
            min.to_bits(),
            join(&docs)
        )
        .unwrap();
    }
    if vector {
        let mut counts: Vec<usize> = (1..=40).collect();
        counts.extend([63, 64, 65, 100, 128, 129, 256, 1000]);
        for &count in &counts {
            for kind in 0..3 {
                let dims = count + rng.below(3) as usize;
                let data = rng.floats(6 * dims, kind);
                let idx: Vec<u64> = (0..5).map(|_| rng.below(6)).collect();
                writeln!(
                    out,
                    "K {count} {dims} {} {} {} {} {} {}",
                    idx[0],
                    idx[1],
                    idx[2],
                    idx[3],
                    idx[4],
                    hex_floats(&data)
                )
                .unwrap();
            }
        }
        for _ in 0..200 {
            let num_bits = rng.range(1, 600);
            let offset = rng.range(0, 20);
            let from = rng.range(offset, offset + num_bits);
            let to = rng.range(from, offset + num_bits);
            let values: Vec<i64> = (0..to).map(|_| rng.range(-20, 20)).collect();
            let min = rng.range(-25, 25);
            let max = rng.range(-25, 25);
            writeln!(
                out,
                "V {num_bits} {from} {to} {min} {max} {offset} {}",
                join(&values)
            )
            .unwrap();
        }
    }
    for _ in 0..200 {
        let num_bits = rng.range(64, 600) as usize;
        let ops: Vec<String> = (0..40)
            .map(|_| {
                const LENS: [usize; 12] = [1, 2, 3, 4, 7, 8, 16, 31, 32, 33, 63, 64];
                let len = LENS[rng.below(LENS.len() as u64) as usize];
                let start = rng.below((num_bits - len + 1) as u64) as usize;
                let mask = if len == 64 {
                    rng.next()
                } else {
                    rng.next() & ((1u64 << len) - 1)
                };
                format!("{start}:{mask:x}:{len}")
            })
            .collect();
        writeln!(out, "O {num_bits} {}", ops.join(";")).unwrap();
    }
    out
}

// ---------------------------------------------------------------------------
// Rucene side
// ---------------------------------------------------------------------------

/// `Integer.toHexString(Float.floatToIntBits(x))`.
fn fx(x: f32) -> String {
    let bits = if x.is_nan() { 0x7fc0_0000 } else { x.to_bits() };
    format!("{bits:x}")
}

fn parse_floats(s: &str) -> Vec<f32> {
    if s == "-" {
        return Vec::new();
    }
    s.split(',')
        .map(|x| f32::from_bits(u32::from_str_radix(x, 16).unwrap()))
        .collect()
}

fn parse_bytes(s: &str) -> Vec<u8> {
    if s == "-" {
        return Vec::new();
    }
    (0..s.len() / 2)
        .map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap())
        .collect()
}

fn parse_list<T: std::str::FromStr>(s: &str) -> Vec<T>
where
    T::Err: std::fmt::Debug,
{
    if s == "-" {
        return Vec::new();
    }
    s.split(',').map(|x| x.parse().unwrap()).collect()
}

fn pf(s: &str) -> f32 {
    f32::from_bits(u32::from_str_radix(s, 16).unwrap())
}

struct VecLongValues(Vec<i64>);

impl LongValues for VecLongValues {
    fn get(&self, index: i64) -> i64 {
        self.0[index as usize]
    }
}

fn words(bits: &FixedBitSet, out: &mut String) {
    for w in bits.get_bits() {
        write!(out, " {:x}", w.load(Ordering::Relaxed)).unwrap();
    }
}

/// Maps `data` (little-endian floats) as a [`MemorySegment`] through a file.
fn segment_of(data: &[f32], dir: &Path, arena: &SharedArena) -> MemorySegment {
    let bytes: Vec<u8> = data.iter().flat_map(|x| x.to_le_bytes()).collect();
    let path = dir.join("segment.bin");
    std::fs::write(&path, &bytes).unwrap();
    let nio_path = nio::Path::new(FileSystems::get_default(), &path);
    let channel = <dyn FileChannel>::open(&nio_path, &[OpenOption::Read]).unwrap();
    MemorySegment::map(&*channel, 0, bytes.len(), Arc::clone(arena.scope())).unwrap()
}

/// Runs the corpus through Rucene and returns the report Java prints.
fn rucene_report(vector: bool, corpus: &str, dir: &Path) -> String {
    let panama = PanamaVectorUtilSupport::new();
    let scalar = DefaultVectorUtilSupport::new();
    let support: &dyn VectorUtilSupport = if vector { &panama } else { &scalar };
    let mut out = String::new();
    if vector {
        out.push_str("impl=PanamaVectorUtilSupport\n");
        writeln!(
            out,
            "preferredBitSize={}",
            PanamaVectorConstants::preferred_vector_bitsize()
        )
        .unwrap();
        writeln!(
            out,
            "hasFastIntegerVectors={}",
            PanamaVectorConstants::has_fast_integer_vectors()
        )
        .unwrap();
    } else {
        out.push_str("impl=DefaultVectorUtilSupport\n");
    }
    writeln!(out, "HAS_FAST_VECTOR_FMA={}", use_vector_fma()).unwrap();
    writeln!(out, "HAS_FAST_SCALAR_FMA={}", vector_util::use_fma()).unwrap();
    writeln!(
        out,
        "HAS_FAST_COMPRESS_MASK_CAST={}",
        Constants::has_fast_compress_mask_cast()
    )
    .unwrap();

    let arena = SharedArena::new();
    for (i, line) in corpus.lines().filter(|l| !l.is_empty()).enumerate() {
        let t: Vec<&str> = line.split(' ').collect();
        write!(out, "{i} {}", t[0]).unwrap();
        match t[0] {
            "F" => {
                let (a, b) = (parse_floats(t[1]), parse_floats(t[2]));
                write!(
                    out,
                    " {} {} {}",
                    fx(support.dot_product_f32(&a, &b).unwrap()),
                    fx(support.cosine_f32(&a, &b).unwrap()),
                    fx(support.square_distance_f32(&a, &b).unwrap())
                )
                .unwrap();
            }
            "B" => {
                let (a, b) = (parse_bytes(t[1]), parse_bytes(t[2]));
                write!(
                    out,
                    " {} {} {} {} {} {} {} {} {}",
                    support.dot_product_bytes(&a, &b).unwrap(),
                    support.uint8_dot_product(&a, &b).unwrap(),
                    fx(support.cosine_bytes(&a, &b).unwrap()),
                    support.square_distance_bytes(&a, &b).unwrap(),
                    support.uint8_square_distance(&a, &b).unwrap(),
                    support.int4_dot_product(&a, &b).unwrap(),
                    support.int4_square_distance(&a, &b).unwrap(),
                    support.int4_dot_product_both_packed(&a, &b).unwrap(),
                    support.int4_square_distance_both_packed(&a, &b).unwrap()
                )
                .unwrap();
            }
            "P" => {
                let (u, p) = (parse_bytes(t[1]), parse_bytes(t[2]));
                write!(
                    out,
                    " {} {}",
                    support.int4_dot_product_single_packed(&u, &p).unwrap(),
                    support.int4_square_distance_single_packed(&u, &p).unwrap()
                )
                .unwrap();
            }
            "Q" => {
                let r = support
                    .int4_bit_dot_product(&parse_bytes(t[1]), &parse_bytes(t[2]))
                    .unwrap();
                write!(out, " {r}").unwrap();
            }
            "D" => {
                let r = support
                    .int4_dibit_dot_product(&parse_bytes(t[1]), &parse_bytes(t[2]))
                    .unwrap();
                write!(out, " {r}").unwrap();
            }
            "G" => {
                let buffer: Vec<i32> = parse_list(t[4]);
                let r = support.find_next_geq(
                    &buffer,
                    t[1].parse().unwrap(),
                    t[2].parse().unwrap(),
                    t[3].parse().unwrap(),
                );
                write!(out, " {r}").unwrap();
            }
            "M" => {
                let v = parse_floats(t[5]);
                let mut dest = vec![0u8; v.len()];
                let c = support
                    .min_max_scalar_quantize(&v, &mut dest, pf(t[1]), pf(t[2]), pf(t[3]), pf(t[4]))
                    .unwrap();
                write!(out, " {} {}", fx(c), hex_bytes(&dest)).unwrap();
            }
            "R" => {
                let c = support.recalculate_scalar_quantization_offset(
                    &parse_bytes(t[7]),
                    pf(t[1]),
                    pf(t[2]),
                    pf(t[3]),
                    pf(t[4]),
                    pf(t[5]),
                    pf(t[6]),
                );
                write!(out, " {}", fx(c)).unwrap();
            }
            "S" => {
                let min = f64::from_bits(u64::from_str_radix(t[1], 16).unwrap());
                let up_to: usize = t[2].parse().unwrap();
                let mut docs: Vec<i32> = parse_list(t[3]);
                let mut scores: Vec<f64> = if t[4] == "-" {
                    Vec::new()
                } else {
                    t[4].split(',')
                        .map(|x| f64::from_bits(u64::from_str_radix(x, 16).unwrap()))
                        .collect()
                };
                let n = support.filter_by_score(&mut docs, &mut scores, min, up_to);
                write!(out, " {n}").unwrap();
                for d in &docs {
                    write!(out, " {d}").unwrap();
                }
                for s in &scores {
                    let bits = if s.is_nan() {
                        0x7ff8_0000_0000_0000
                    } else {
                        s.to_bits()
                    };
                    write!(out, " {bits:x}").unwrap();
                }
            }
            "K" => {
                let count: usize = t[1].parse().unwrap();
                let dims: i64 = t[2].parse().unwrap();
                let stride = 4 * dims;
                let data = parse_floats(t[8]);
                let seg = segment_of(&data, dir, &arena);
                let idx: Vec<i64> = (3..8).map(|k| t[k].parse().unwrap()).collect();
                let qi = idx[0] as usize;
                let q_array = &data[qi * dims as usize..(qi + 1) * dims as usize];
                let (q, d1, d2, d3, d4) = (
                    idx[0] * stride,
                    idx[1] * stride,
                    idx[2] * stride,
                    idx[3] * stride,
                    idx[4] * stride,
                );
                let mut scores = [0f32; 4];
                let dot = MemorySegmentBulkVectorOps::DOT_INSTANCE;
                let cos = MemorySegmentBulkVectorOps::COS_INSTANCE;
                let sqr = MemorySegmentBulkVectorOps::SQR_INSTANCE;
                let push = |scores: &[f32; 4], out: &mut String| {
                    for s in scores {
                        write!(out, " {}", fx(*s)).unwrap();
                    }
                };
                dot.dot_product_bulk(&seg, &mut scores, q_array, d1, d2, d3, d4, count);
                push(&scores, &mut out);
                dot.dot_product_bulk_at(&seg, &mut scores, q, d1, d2, d3, d4, count);
                push(&scores, &mut out);
                write!(out, " {}", fx(dot.dot_product(&seg, q, d1, count))).unwrap();
                cos.cosine_bulk(&seg, &mut scores, q_array, d1, d2, d3, d4, count);
                push(&scores, &mut out);
                cos.cosine_bulk_at(&seg, &mut scores, q, d1, d2, d3, d4, count);
                push(&scores, &mut out);
                write!(out, " {}", fx(cos.cosine(&seg, q, d1, count))).unwrap();
                sqr.sqr_distance_bulk(&seg, &mut scores, q_array, d1, d2, d3, d4, count);
                push(&scores, &mut out);
                sqr.sqr_distance_bulk_at(&seg, &mut scores, q, d1, d2, d3, d4, count);
                push(&scores, &mut out);
                write!(out, " {}", fx(sqr.sqr_distance(&seg, q, d1, count))).unwrap();
            }
            "V" => {
                let num_bits: usize = t[1].parse().unwrap();
                let values = VecLongValues(parse_list(t[7]));
                let mut bits = FixedBitSet::new(num_bits);
                PanamaDocValuesRangeSupport::INSTANCE.range_into_bit_set(
                    &values,
                    t[2].parse().unwrap(),
                    t[3].parse().unwrap(),
                    t[4].parse().unwrap(),
                    t[5].parse().unwrap(),
                    &mut bits,
                    t[6].parse().unwrap(),
                );
                words(&bits, &mut out);
            }
            "O" => {
                let mut bits = FixedBitSet::new(t[1].parse().unwrap());
                for op in t[2].split(';') {
                    let p: Vec<&str> = op.split(':').collect();
                    bits.or_mask(
                        p[0].parse().unwrap(),
                        u64::from_str_radix(p[1], 16).unwrap(),
                        p[2].parse().unwrap(),
                    );
                }
                words(&bits, &mut out);
            }
            other => panic!("unknown operation {other}"),
        }
        out.push('\n');
    }
    out
}

/// The Rucene side, run by [`panama_and_scalar_kernels_match_lucene`] in a
/// child process whose environment carries the gate properties. Without
/// [`CHILD_ENV`] it does nothing.
#[test]
#[ignore = "child process of panama_and_scalar_kernels_match_lucene"]
fn oracle_child() {
    let Ok(spec) = std::env::var(CHILD_ENV) else {
        return;
    };
    let mut parts = spec.splitn(3, '|');
    let mode = parts.next().unwrap();
    let corpus_path = PathBuf::from(parts.next().unwrap());
    let out_path = PathBuf::from(parts.next().unwrap());
    let corpus = std::fs::read_to_string(&corpus_path).unwrap();
    let dir = out_path.with_extension("seg");
    std::fs::create_dir_all(&dir).unwrap();
    let report = rucene_report(mode == "vector", &corpus, &dir);
    std::fs::write(out_path, report).unwrap();
}

// ---------------------------------------------------------------------------
// Java oracle
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

/// One gate configuration: the provider and the two FMA properties.
#[derive(Clone, Copy, Debug)]
struct Config {
    vector: bool,
    vector_fma: bool,
    scalar_fma: bool,
}

impl Config {
    fn name(self) -> String {
        format!(
            "{}-vfma{}-sfma{}",
            if self.vector { "vector" } else { "scalar" },
            self.vector_fma,
            self.scalar_fma
        )
    }
}

/// Runs the fixture; returns its stdout and the cold/warm line of its stderr.
fn run_java(config: Config, corpus: &Path) -> (String, String) {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join(
        "tests/fixtures/java-codec-harness/src/main/java/org/apache/lucene/rucene/codec/\
         PanamaVectorizationFixture.java",
    );
    let mut cmd = Command::new("java");
    if config.vector {
        cmd.arg("--add-modules").arg("jdk.incubator.vector");
    }
    let output = cmd
        .arg(format!("-Dlucene.useVectorFMA={}", config.vector_fma))
        .arg(format!("-Dlucene.useScalarFMA={}", config.scalar_fma))
        .arg("-cp")
        .arg(lucene_jar())
        .arg(fixture)
        .arg(corpus)
        .arg(JAVA_PASSES)
        .output()
        .expect("a JDK is required to run the Lucene oracle");
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        output.status.success(),
        "PanamaVectorizationFixture {config:?} failed:\n{stderr}"
    );
    let cold = stderr
        .lines()
        .find(|l| l.starts_with("cold_vs_warm"))
        .unwrap_or("")
        .to_string();
    (String::from_utf8_lossy(&output.stdout).into_owned(), cold)
}

/// Runs [`oracle_child`] in a child process with the gate properties.
fn run_rucene(config: Config, corpus: &Path, dir: &Path) -> String {
    let out = dir.join(format!("{}.rucene", config.name()));
    let mode = if config.vector { "vector" } else { "scalar" };
    let status = Command::new(std::env::current_exe().unwrap())
        .args([
            "oracle_child",
            "--exact",
            "--include-ignored",
            "--test-threads=1",
        ])
        .env(
            CHILD_ENV,
            format!("{mode}|{}|{}", corpus.display(), out.display()),
        )
        .env("lucene.useVectorFMA", config.vector_fma.to_string())
        .env("lucene.useScalarFMA", config.scalar_fma.to_string())
        .status()
        .unwrap();
    assert!(status.success(), "Rucene child {config:?} failed");
    std::fs::read_to_string(out).unwrap()
}

/// The first differing lines of two reports, or `None` when they agree.
fn diff(name: &str, java: &str, rucene: &str) -> Option<String> {
    if java == rucene {
        return None;
    }
    let l: Vec<&str> = java.lines().collect();
    let r: Vec<&str> = rucene.lines().collect();
    let mut out = format!(
        "{name}: Java ({} lines) differs from Rucene ({} lines):\n",
        l.len(),
        r.len()
    );
    let mut shown = 0;
    let mut total = 0;
    for k in 0..l.len().max(r.len()) {
        let a = l.get(k).copied().unwrap_or("<end>");
        let b = r.get(k).copied().unwrap_or("<end>");
        if a != b {
            total += 1;
            if shown < 8 {
                let cut = |s: &str| s.chars().take(300).collect::<String>();
                writeln!(
                    out,
                    "  line {}\n    java:   {}\n    rucene: {}",
                    k + 1,
                    cut(a),
                    cut(b)
                )
                .unwrap();
                shown += 1;
            }
        }
    }
    writeln!(out, "  {total} differing lines").unwrap();
    Some(out)
}

#[test]
fn panama_and_scalar_kernels_match_lucene() {
    let dir = tempfile::Builder::new()
        .prefix("panama-oracle")
        .tempdir()
        .unwrap();
    let vector_corpus = dir.path().join("vector.corpus");
    let scalar_corpus = dir.path().join("scalar.corpus");
    std::fs::write(&vector_corpus, corpus(true)).unwrap();
    std::fs::write(&scalar_corpus, corpus(false)).unwrap();

    let mut configs = Vec::new();
    for vector_fma in [false, true] {
        for scalar_fma in [false, true] {
            configs.push(Config {
                vector: true,
                vector_fma,
                scalar_fma,
            });
        }
    }
    for scalar_fma in [false, true] {
        configs.push(Config {
            vector: false,
            vector_fma: false,
            scalar_fma,
        });
    }

    let results: Vec<(Config, String, String, String)> = std::thread::scope(|s| {
        let handles: Vec<_> = configs
            .iter()
            .map(|&config| {
                let corpus = if config.vector {
                    &vector_corpus
                } else {
                    &scalar_corpus
                };
                let dir = dir.path();
                s.spawn(move || {
                    let (java, cold) = run_java(config, corpus);
                    let rucene = run_rucene(config, corpus, dir);
                    (config, java, cold, rucene)
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });

    let mut failures = Vec::new();
    for (config, java, cold, rucene) in &results {
        let lines = java.lines().count();
        eprintln!("{}: {lines} lines; Java {cold}", config.name());
        // The fixture really ran the configuration asked for.
        let expect_impl = if config.vector {
            "impl=PanamaVectorUtilSupport"
        } else {
            "impl=DefaultVectorUtilSupport"
        };
        assert!(
            java.starts_with(expect_impl),
            "{}: {java:.200}",
            config.name()
        );
        assert!(
            java.contains(&format!("HAS_FAST_SCALAR_FMA={}\n", config.scalar_fma)),
            "{}",
            config.name()
        );
        if config.vector {
            assert!(
                java.contains(&format!("HAS_FAST_VECTOR_FMA={}\n", config.vector_fma)),
                "{}",
                config.name()
            );
        }
        if let Some(d) = diff(&config.name(), java, rucene) {
            failures.push(d);
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
