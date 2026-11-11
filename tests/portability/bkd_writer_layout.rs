//! Portability test for the bytes `BKDWriter` writes through `add()` and
//! `finish()`, against Apache Lucene Core 10.5.0.
//!
//! The Java fixture `BkdWriterLayoutFixture` writes the `meta`, `index` and
//! `data` files of fixed scenarios with Lucene's `BKDWriter`; this test writes
//! the same scenarios with Rucene's and compares the files byte for byte. The
//! scenarios cover the route `TestBKD` exercises and the codec suites do not:
//! `add()` + `finish()` with small leaves, the heap writer and the offline
//! sort, and more data dimensions than index dimensions (one of them equal for
//! every point). Each scenario is also written through one output for the
//! three roles, as `TestBKD` does, where `BKDWriter.writeIndex` accounts for
//! `metaOut == indexOut` in the index start pointer it records.
//!
//! A missing Maven or JDK is a hard failure, as in the other portability tests:
//! without the reference implementation there is nothing to compare.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

use rucene::store::{DataOutput, Directory, FSDirectory, IndexOutput, DEFAULT_IO_CONTEXT};
use rucene::util::bkd::{BKDConfig, BKDWriter};
use rucene::util::NumericUtils;

/// One scenario; mirrors `BkdWriterLayoutFixture.Scenario`.
struct Scenario {
    name: &'static str,
    num_dims: i32,
    num_index_dims: i32,
    bytes_per_dim: i32,
    max_points_in_leaf_node: i32,
    max_mb_sort_in_heap: f64,
    num_points: i32,
    equal_dim: i32,
}

const SCENARIOS: [Scenario; 3] = [
    Scenario {
        name: "basic1d",
        num_dims: 1,
        num_index_dims: 1,
        bytes_per_dim: 4,
        max_points_in_leaf_node: 2,
        max_mb_sort_in_heap: 1.0,
        num_points: 100,
        equal_dim: -1,
    },
    Scenario {
        name: "heap_data_dims",
        num_dims: 4,
        num_index_dims: 2,
        bytes_per_dim: 4,
        max_points_in_leaf_node: 3,
        max_mb_sort_in_heap: 1.0,
        num_points: 600,
        equal_dim: -1,
    },
    Scenario {
        name: "offline_data_dims",
        num_dims: 4,
        num_index_dims: 2,
        bytes_per_dim: 4,
        max_points_in_leaf_node: 3,
        max_mb_sort_in_heap: 0.01,
        num_points: 2000,
        equal_dim: 1,
    },
];

/// `BkdWriterLayoutFixture.value`: the value of dimension `dim` of point `i`.
fn value(s: &Scenario, i: i32, dim: i32, dest: &mut [u8], offset: usize) {
    let v = if dim == s.equal_dim {
        12345
    } else {
        let mut x = i64::from(i / 3)
            .wrapping_mul(0x9E37_79B9_7F4A_7C15_u64 as i64)
            .wrapping_add(i64::from(dim).wrapping_mul(0xBF58_476D_1CE4_E5B9_u64 as i64));
        x ^= ((x as u64) >> 31) as i64;
        x = x.wrapping_mul(0x94D0_49BB_1331_11EB_u64 as i64);
        x ^= ((x as u64) >> 29) as i64;
        x as i32
    };
    NumericUtils::int_to_sortable_bytes(v, dest, offset);
    for k in 4..s.bytes_per_dim as usize {
        dest[offset + k] = (i * 31 + dim * 7 + k as i32) as u8;
    }
}

/// Writes every scenario with Rucene's `BKDWriter` into `dir`.
fn write_rucene(dir: &Path) {
    let directory: Arc<dyn Directory> = Arc::new(FSDirectory::open(dir).expect("directory"));
    for s in &SCENARIOS {
        let config = BKDConfig::new(
            s.num_dims,
            s.num_index_dims,
            s.bytes_per_dim,
            s.max_points_in_leaf_node,
        )
        .expect("config");
        let mut w = BKDWriter::new_default(
            s.num_points,
            Arc::clone(&directory),
            s.name,
            config,
            s.max_mb_sort_in_heap,
            i64::from(s.num_points),
        )
        .expect("writer");
        let mut packed = vec![0u8; (s.num_dims * s.bytes_per_dim) as usize];
        for i in 0..s.num_points {
            for dim in 0..s.num_dims {
                value(s, i, dim, &mut packed, (dim * s.bytes_per_dim) as usize);
            }
            w.add(&packed, i).expect("add");
        }
        let mut meta = directory
            .create_output(&format!("{}.meta", s.name), &*DEFAULT_IO_CONTEXT)
            .expect("meta");
        let mut index = directory
            .create_output(&format!("{}.index", s.name), &*DEFAULT_IO_CONTEXT)
            .expect("index");
        let mut data = directory
            .create_output(&format!("{}.data", s.name), &*DEFAULT_IO_CONTEXT)
            .expect("data");
        let finalizer = w.finish(data.as_mut()).expect("finish").expect("points");
        finalizer
            .run(meta.as_mut(), index.as_mut())
            .expect("finalizer");
        meta.close().expect("close meta");
        index.close().expect("close index");
        data.close().expect("close data");

        // The same points through one output for the three roles: Java's
        // `finish(out, out, out)`, where `writeIndex` sees `metaOut == indexOut`.
        let mut one = BKDWriter::new_default(
            s.num_points,
            Arc::clone(&directory),
            &format!("{}_one", s.name),
            config_of(s),
            s.max_mb_sort_in_heap,
            i64::from(s.num_points),
        )
        .expect("writer");
        for i in 0..s.num_points {
            for dim in 0..s.num_dims {
                value(s, i, dim, &mut packed, (dim * s.bytes_per_dim) as usize);
            }
            one.add(&packed, i).expect("add");
        }
        let out = OneStream::new(
            directory
                .create_output(&format!("{}.one", s.name), &*DEFAULT_IO_CONTEXT)
                .expect("one"),
        );
        let finalizer = one
            .finish(&mut out.handle())
            .expect("finish")
            .expect("points");
        finalizer
            .run(&mut out.handle(), &mut out.handle())
            .expect("finalizer");
        out.close_stream();
    }
}

/// The `BKDConfig` of a scenario.
fn config_of(s: &Scenario) -> BKDConfig {
    BKDConfig::new(
        s.num_dims,
        s.num_index_dims,
        s.bytes_per_dim,
        s.max_points_in_leaf_node,
    )
    .expect("config")
}

/// One `IndexOutput` handed out as several handles, standing for the one object
/// Java passes as the meta, index and data output.
struct OneStream {
    inner: Arc<Mutex<Box<dyn IndexOutput>>>,
    name: String,
}

impl OneStream {
    fn new(out: Box<dyn IndexOutput>) -> Self {
        let name = out.name().to_string();
        Self {
            inner: Arc::new(Mutex::new(out)),
            name,
        }
    }

    fn handle(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
            name: self.name.clone(),
        }
    }

    fn close_stream(&self) {
        self.inner.lock().unwrap().close().expect("close");
    }
}

impl DataOutput for OneStream {
    fn write_byte(&mut self, b: u8) -> rucene::error::Result<()> {
        self.inner.lock().unwrap().write_byte(b)
    }

    fn write_bytes(&mut self, b: &[u8], offset: usize, len: usize) -> rucene::error::Result<()> {
        self.inner.lock().unwrap().write_bytes(b, offset, len)
    }
}

impl IndexOutput for OneStream {
    fn close(&mut self) -> rucene::error::Result<()> {
        Ok(())
    }

    fn file_pointer(&self) -> i64 {
        self.inner.lock().unwrap().file_pointer()
    }

    fn checksum(&self) -> rucene::error::Result<i64> {
        self.inner.lock().unwrap().checksum()
    }

    fn resource_description(&self) -> &str {
        &self.name
    }

    fn name(&self) -> &str {
        &self.name
    }

    /// Every handle is the one stream.
    fn output_identity(&self) -> *const () {
        Arc::as_ptr(&self.inner) as *const ()
    }
}

fn harness_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/java-codec-harness")
}

/// Runs the Java fixture, which writes every scenario into `dir`.
fn write_java(dir: &Path) {
    let output = Command::new("mvn")
        .arg("-q")
        .arg("compile")
        .arg("exec:java")
        .arg("-Dexec.mainClass=org.apache.lucene.rucene.codec.BkdWriterLayoutFixture")
        .arg(format!("-Dexec.args={}", dir.display()))
        .current_dir(harness_dir())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .unwrap_or_else(|e| panic!("the BKD layout portability test needs Maven and a JDK: {e}"));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success() && stdout.lines().any(|l| l.trim() == "write_ok=true"),
        "the Java fixture failed:\nstdout:\n{stdout}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// The first offset at which `a` and `b` differ, or `None` when they are equal.
fn first_divergence(a: &[u8], b: &[u8]) -> Option<usize> {
    a.iter()
        .zip(b)
        .position(|(x, y)| x != y)
        .or_else(|| (a.len() != b.len()).then(|| a.len().min(b.len())))
}

#[test]
fn bkd_writer_add_and_finish_write_lucenes_bytes() {
    let base = std::env::temp_dir().join(format!("rucene-bkd-layout-{}", std::process::id()));
    let java_dir = base.join("java");
    let rucene_dir = base.join("rucene");
    std::fs::create_dir_all(&java_dir).expect("java dir");
    std::fs::create_dir_all(&rucene_dir).expect("rucene dir");
    write_java(&java_dir);
    write_rucene(&rucene_dir);
    let mut report = Vec::new();
    for s in &SCENARIOS {
        for ext in ["meta", "index", "data", "one"] {
            let name = format!("{}.{ext}", s.name);
            let java = std::fs::read(java_dir.join(&name)).expect("java file");
            let rucene = std::fs::read(rucene_dir.join(&name)).expect("rucene file");
            if let Some(at) = first_divergence(&java, &rucene) {
                let window = |b: &[u8]| {
                    let end = (at + 16).min(b.len());
                    let start = at.min(end);
                    b[start..end]
                        .iter()
                        .map(|x| format!("{x:02x}"))
                        .collect::<String>()
                };
                report.push(format!(
                    "{name}: first divergence at byte {at} (java len {}, rucene len {}): java {} rucene {}",
                    java.len(),
                    rucene.len(),
                    window(&java),
                    window(&rucene)
                ));
            }
        }
    }
    let keep = std::env::var_os("RUCENE_KEEP_BKD_LAYOUT").is_some();
    if !keep {
        let _ = std::fs::remove_dir_all(&base);
    }
    assert!(report.is_empty(), "{}", report.join("\n"));
}
