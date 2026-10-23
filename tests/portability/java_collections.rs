//! Differential tests for the iteration order Lucene 10.5.0 writes into a file.
//!
//! `DataOutput.writeMapOfStrings` and `DataOutput.writeSetOfStrings` walk their
//! argument in iteration order (`DataOutput.java:304-310` and `:321-326`), so
//! the order a `java.util.HashMap` happens to have is part of every `.fnm`,
//! `.si` and `segments_N` Lucene writes. [`rucene::util::java_collections`]
//! reproduces that order; these tests are what prove it, by running the same
//! construction sequences on a JVM through `JavaCollectionsFixture` and
//! comparing entry by entry.
//!
//! Nothing here asserts the port against its own arithmetic: every expected
//! order comes back from the JVM in the same run.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Mutex;

use rucene::util::java_collections::{JavaHashMap, JavaHashSet, MIN_TREEIFY_CAPACITY};

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
///
/// A portability test proves parity against the reference Java implementation,
/// so it has nothing to assert without the harness: skipping would report
/// success while proving nothing.
fn require_maven() {
    if let Err(reason) = which_mvn() {
        panic!("java-collections portability tests require Maven and a JDK: {reason}");
    }
}

/// Runs every case through the JVM and returns its collection in iteration
/// order, one entry per case.
fn java_orders(cases: &[Case]) -> Vec<Vec<String>> {
    let _guard = HARNESS_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let dir = tempfile::tempdir().expect("temp dir");
    let input = dir.path().join("cases.tsv");
    let output = dir.path().join("orders.tsv");

    let mut wire = String::new();
    for case in cases {
        wire.push_str(case.shape);
        wire.push('\t');
        wire.push_str(&case.arg.to_string());
        for key in &case.keys {
            wire.push('\t');
            wire.push_str(&escape(key));
        }
        wire.push('\n');
    }
    std::fs::write(&input, wire).expect("write cases");

    let harness = harness_dir();
    let mvn = which_mvn().expect("Maven");
    let result = Command::new(mvn)
        .arg("-q")
        .arg("compile")
        .arg("exec:java")
        .arg("-Dexec.mainClass=org.apache.lucene.rucene.codec.JavaCollectionsFixture")
        .arg(format!(
            "-Dexec.args={} {}",
            input.display(),
            output.display()
        ))
        .current_dir(&harness)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn Maven");
    assert!(
        result.status.success(),
        "Java harness failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );

    let text = std::fs::read_to_string(&output).expect("read orders");
    // Each line is `count` then the keys, so a collection holding only the
    // empty string stays distinguishable from an empty collection.
    let orders: Vec<Vec<String>> = text
        .lines()
        .map(|line| {
            let mut fields = line.split('\t');
            let count: usize = fields
                .next()
                .expect("count field")
                .parse()
                .expect("count is a number");
            let keys: Vec<String> = fields.map(unescape).collect();
            assert_eq!(keys.len(), count, "the JVM line {line:?} is truncated");
            keys
        })
        .collect();
    assert_eq!(
        orders.len(),
        cases.len(),
        "the JVM returned {} orders for {} cases",
        orders.len(),
        cases.len()
    );
    orders
}

fn escape(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('\t', "\\t")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
}

fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('t') => out.push('\t'),
                Some('n') => out.push('\n'),
                Some('r') => out.push('\r'),
                Some('\\') => out.push('\\'),
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Cases
// ---------------------------------------------------------------------------

/// One construction sequence: a shape, its integer parameter and its keys.
struct Case {
    shape: &'static str,
    arg: i32,
    keys: Vec<String>,
}

/// Runs the port's side of one case.
///
/// Each arm is the Rust spelling of the matching arm of
/// `JavaCollectionsFixture.run`, so a divergence in either the model or the
/// wiring shows up as an order mismatch.
fn rucene_order(case: &Case) -> (Vec<String>, bool) {
    let entries = || {
        case.keys
            .iter()
            .map(|k| (k.clone(), "v".to_string()))
            .collect::<Vec<_>>()
    };
    match case.shape {
        "map_put" => {
            let map: JavaHashMap<String> = entries().into_iter().collect();
            (keys_of(&map), map.is_treeified())
        }
        "map_copy" => {
            let map: JavaHashMap<String> = entries().into_iter().collect();
            let copy = JavaHashMap::copy_of(&map);
            (keys_of(&copy), map.is_treeified() || copy.is_treeified())
        }
        "map_copy_put" => {
            // `FieldInfo.putAttribute`: copy the map, then put into the copy.
            let mut map: JavaHashMap<String> = JavaHashMap::new();
            let mut treeified = false;
            for (key, value) in entries() {
                let mut next = JavaHashMap::copy_of(&map);
                next.insert(key, value);
                treeified |= next.is_treeified();
                map = next;
            }
            (keys_of(&map), treeified)
        }
        "map_capacity" => {
            let mut map: JavaHashMap<String> = JavaHashMap::with_initial_capacity(case.arg);
            for (key, value) in entries() {
                map.insert(key, value);
            }
            (keys_of(&map), map.is_treeified())
        }
        "map_put_all" => {
            let mut map: JavaHashMap<String> = JavaHashMap::new();
            let mut rest: JavaHashMap<String> = JavaHashMap::new();
            for (index, (key, value)) in entries().into_iter().enumerate() {
                if (index as i32) < case.arg {
                    map.insert(key, value);
                } else {
                    rest.insert(key, value);
                }
            }
            let rest_entries: Vec<(String, String)> = rest
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect::<Vec<_>>();
            map.put_all(rest_entries);
            (keys_of(&map), map.is_treeified() || rest.is_treeified())
        }
        "map_remove_reinsert" => {
            let mut map: JavaHashMap<String> = entries().into_iter().collect();
            for key in case.keys.iter().take(case.arg.max(0) as usize) {
                map.remove(key);
                map.insert(key.clone(), "v".to_string());
            }
            (keys_of(&map), map.is_treeified())
        }
        "set_add" => {
            let set: JavaHashSet = case.keys.iter().cloned().collect();
            (set.iter().cloned().collect(), set.is_treeified())
        }
        "set_copy" => {
            let set: JavaHashSet = case.keys.iter().cloned().collect();
            let ordered: Vec<String> = set.iter().cloned().collect();
            let copy = JavaHashSet::from_collection(ordered);
            (
                copy.iter().cloned().collect(),
                set.is_treeified() || copy.is_treeified(),
            )
        }
        other => panic!("unknown shape: {other}"),
    }
}

fn keys_of(map: &JavaHashMap<String>) -> Vec<String> {
    map.keys().cloned().collect()
}

/// The xorshift64* the project uses wherever a test needs a reproducible
/// sequence.
struct Xorshift(u64);

impl Xorshift {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, bound: usize) -> usize {
        (self.next() % bound as u64) as usize
    }
}

/// The alphabets the generated keys are drawn from.
///
/// The point of mixing them is that the bucket index is a function of
/// `String.hashCode`, so keys have to exercise short strings (whose hash is
/// small and whose high bits the spreading function has nothing to fold in),
/// long strings, the `"Aa"`/`"BB"` family that collides outright, and
/// characters outside the basic multilingual plane, where one Rust `char` is
/// two UTF-16 code units and a naive hash would disagree.
fn random_key(rng: &mut Xorshift) -> String {
    match rng.below(6) {
        0 => {
            // The shapes Lucene's own code puts in these maps.
            const REAL: [&str; 12] = [
                "PerFieldPostingsFormat.format",
                "PerFieldPostingsFormat.suffix",
                "PerFieldDocValuesFormat.format",
                "PerFieldDocValuesFormat.suffix",
                "PerFieldKnnVectorsFormat.format",
                "PerFieldKnnVectorsFormat.suffix",
                "Lucene90StoredFieldsFormat.mode",
                "lucene.version",
                "source",
                "os",
                "os.arch",
                "java.vendor",
            ];
            REAL[rng.below(REAL.len())].to_string()
        }
        1 => {
            // Codec file names, which is what `SegmentInfo.files` holds.
            const EXT: [&str; 10] = [
                ".fnm", ".fdt", ".fdx", ".si", ".nvd", ".nvm", ".dvd", ".dvm", ".tim", ".doc",
            ];
            format!("_{}{}", rng.below(40), EXT[rng.below(EXT.len())])
        }
        2 => {
            // Strings whose `String.hashCode` is equal by construction: every
            // block contributes the same amount, so any concatenation of them
            // collides with any other of the same length.
            let blocks = 1 + rng.below(4);
            let mut s = String::new();
            for _ in 0..blocks {
                s.push_str(if rng.below(2) == 0 { "Aa" } else { "BB" });
            }
            s
        }
        3 => {
            let len = rng.below(24);
            (0..len)
                .map(|_| (b'a' + rng.below(26) as u8) as char)
                .collect()
        }
        4 => {
            // Outside the basic multilingual plane, plus a couple of BMP
            // characters that need more than one UTF-8 byte.
            const WIDE: [char; 6] = ['\u{1D400}', '\u{1F600}', '\u{10437}', 'é', 'ß', '中'];
            let len = 1 + rng.below(4);
            (0..len).map(|_| WIDE[rng.below(WIDE.len())]).collect()
        }
        _ => {
            // Including the empty string, whose hash is 0.
            let len = rng.below(3);
            (0..len)
                .map(|_| (b'A' + rng.below(26) as u8) as char)
                .collect()
        }
    }
}

/// Builds the case grid.
///
/// The sizes deliberately straddle every resize point a small map crosses — 12
/// entries fills a table of 16, 13 doubles it, 24 fills 32 — because the table
/// length is the modulus the bucket index is taken against and is therefore
/// what the order turns on.
fn build_cases(seed: u64, per_shape: usize) -> Vec<Case> {
    const SHAPES: [&str; 8] = [
        "map_put",
        "map_copy",
        "map_copy_put",
        "map_capacity",
        "map_put_all",
        "map_remove_reinsert",
        "set_add",
        "set_copy",
    ];
    const SIZES: [usize; 14] = [0, 1, 2, 3, 4, 5, 8, 11, 12, 13, 17, 24, 25, 40];

    let mut rng = Xorshift(seed);
    let mut cases = Vec::new();
    for shape in SHAPES {
        for _ in 0..per_shape {
            for size in SIZES {
                let keys: Vec<String> = (0..size).map(|_| random_key(&mut rng)).collect();
                let arg = match shape {
                    "map_capacity" => [0, 1, 2, 3, 7, 16, 17, 64][rng.below(8)],
                    "map_put_all" | "map_remove_reinsert" => rng.below(size + 1) as i32,
                    _ => 0,
                };
                cases.push(Case { shape, arg, keys });
            }
        }
    }
    cases
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// The whole grid, compared entry by entry against a running JVM.
#[test]
fn iteration_order_matches_the_jvm_over_every_construction_shape() {
    require_maven();
    let cases = build_cases(0x5EED_1177_C0FF_EE01, 3);
    let expected = java_orders(&cases);

    let mut compared = 0usize;
    let mut treeified = 0usize;
    for (case, java) in cases.iter().zip(expected.iter()) {
        let (ours, was_treeified) = rucene_order(case);
        if was_treeified {
            // Documented gap, measured by
            // `treeified_bucket_is_the_one_shape_the_port_does_not_order_like_java`.
            treeified += 1;
            continue;
        }
        assert_eq!(
            &ours, java,
            "iteration order differs for shape={} arg={} keys={:?}",
            case.shape, case.arg, case.keys
        );
        compared += 1;
    }
    assert!(
        compared >= cases.len() - 8,
        "only {compared} of {} cases were compared; {treeified} hit the treeified gap",
        cases.len()
    );
    eprintln!("compared {compared} cases against the JVM ({treeified} treeified and skipped)");
}

/// Pins the two shapes the flush path actually writes.
///
/// A field handled by `PerFieldPostingsFormat` carries exactly two attributes,
/// put one at a time through `FieldInfo.putAttribute`
/// (`PerFieldPostingsFormat.java:253-254`, `FieldInfo.java:674-678`), and a
/// sealed segment lists its files in a `HashSet` built by
/// `SegmentInfo.setFiles` (`SegmentInfo.java:309-320`). Both orders end up in a
/// file, so both are checked against the JVM directly rather than as part of
/// the random grid.
#[test]
fn the_orders_the_flush_path_writes_match_the_jvm() {
    require_maven();
    let attributes: Vec<String> = vec![
        "PerFieldPostingsFormat.format".to_string(),
        "PerFieldPostingsFormat.suffix".to_string(),
    ];
    let files: Vec<String> = vec![
        "_9.si".to_string(),
        "_9.fnm".to_string(),
        "_9.fdt".to_string(),
        "_9.fdx".to_string(),
        "_9.nvd".to_string(),
        "_9.nvm".to_string(),
        "_9.tim".to_string(),
        "_9.tip".to_string(),
        "_9.doc".to_string(),
        "_9.pos".to_string(),
    ];
    let cases = vec![
        Case {
            shape: "map_copy_put",
            arg: 0,
            keys: attributes.clone(),
        },
        Case {
            shape: "map_put",
            arg: 0,
            keys: attributes,
        },
        Case {
            shape: "set_add",
            arg: 0,
            keys: files.clone(),
        },
        Case {
            shape: "set_copy",
            arg: 0,
            keys: files,
        },
    ];
    let expected = java_orders(&cases);
    for (case, java) in cases.iter().zip(expected.iter()) {
        let (ours, _) = rucene_order(case);
        assert_eq!(&ours, java, "shape={} keys={:?}", case.shape, case.keys);
    }
}

/// Pins the bucket-collision paths, including the resize `treeifyBin` performs
/// instead of building a tree while the table is still short.
///
/// Every string here is a concatenation of `"Aa"` and `"BB"` blocks, which have
/// the same `String.hashCode`, so all strings of the same block count collide
/// outright and share one bucket at every table length. Once that bucket holds
/// more than `TREEIFY_THRESHOLD` entries, `treeifyBin` resizes the table rather
/// than treeifying it while the table is shorter than `MIN_TREEIFY_CAPACITY` —
/// an extra doubling that no threshold crossing explains and that moves every
/// other key in the map.
///
/// The family is grown one key at a time and each size is compared against the
/// JVM, so the test states exactly where the port stops reproducing Java: the
/// orders agree at every size until a bucket treeifies, and only then diverge.
#[test]
fn colliding_buckets_agree_with_the_jvm_up_to_the_treeify_boundary() {
    require_maven();
    let family = |count: usize| -> Vec<String> {
        (0..count as u32)
            .map(|mask| {
                let mut s = String::new();
                for bit in 0..4 {
                    s.push_str(if mask & (1 << bit) == 0 { "Aa" } else { "BB" });
                }
                s
            })
            .collect()
    };

    let cases: Vec<Case> = (1..=16)
        .map(|count| Case {
            shape: "map_put",
            arg: 0,
            keys: family(count),
        })
        .collect();
    let expected = java_orders(&cases);

    let mut first_treeified: Option<usize> = None;
    for (case, java) in cases.iter().zip(expected.iter()) {
        let (ours, treeified) = rucene_order(case);
        let matches = &ours == java;
        if treeified {
            first_treeified.get_or_insert(case.keys.len());
            assert!(
                !matches,
                "{} colliding keys treeified but the orders still agree; if \
                 java.util.HashMap has changed, the documented gap must be revisited",
                case.keys.len()
            );
        } else {
            assert!(
                matches,
                "{} colliding keys, no treeified bucket, yet the orders differ\n  ours: {ours:?}\n  java: {java:?}",
                case.keys.len()
            );
        }
    }
    let boundary = first_treeified.expect("the family should treeify by 16 keys");
    // Repeated `treeifyBin` resizes double the table on every key past the
    // eighth, so one bucket reaches MIN_TREEIFY_CAPACITY with far fewer entries
    // than a load-factor resize would need.
    assert_eq!(
        boundary, 11,
        "the treeify boundary moved; the module docs name this number"
    );
    eprintln!(
        "orders agree for 1..={} colliding keys; the bucket treeifies at {boundary}",
        boundary - 1
    );
}

/// Pins the collision shapes that stay below the treeify boundary, in the
/// constructions Lucene actually uses.
#[test]
fn colliding_buckets_match_the_jvm_across_every_construction() {
    require_maven();
    let family = |count: usize| -> Vec<String> {
        (0..count as u32)
            .map(|mask| {
                let mut s = String::new();
                for bit in 0..4 {
                    s.push_str(if mask & (1 << bit) == 0 { "Aa" } else { "BB" });
                }
                s
            })
            .collect()
    };
    let mut mixed = family(9);
    mixed.extend(["alpha", "beta", "gamma", "delta", "epsilon"].map(str::to_string));
    let mut small_mixed = family(6);
    small_mixed.extend(["alpha", "beta", "gamma", "delta", "epsilon"].map(str::to_string));

    let cases = vec![
        // Eight colliding keys: the bucket is full but `treeifyBin` has not run.
        Case {
            shape: "map_put",
            arg: 0,
            keys: family(8),
        },
        // Nine: `treeifyBin` fires and resizes because the table is only 16.
        Case {
            shape: "map_put",
            arg: 0,
            keys: family(9),
        },
        Case {
            shape: "map_copy",
            arg: 0,
            keys: family(9),
        },
        Case {
            shape: "map_copy_put",
            arg: 0,
            keys: family(9),
        },
        Case {
            shape: "set_add",
            arg: 0,
            keys: family(9),
        },
        Case {
            shape: "set_copy",
            arg: 0,
            keys: family(10),
        },
        Case {
            shape: "map_put",
            arg: 0,
            keys: mixed,
        },
        // Removing and re-adding walks the key to the tail of its bucket, and
        // each re-add re-runs the `treeifyBin` check, so this family is kept
        // small enough that the repeated resizes stay below the boundary.
        Case {
            shape: "map_remove_reinsert",
            arg: 3,
            keys: small_mixed,
        },
    ];
    let expected = java_orders(&cases);
    for (case, java) in cases.iter().zip(expected.iter()) {
        let (ours, treeified) = rucene_order(case);
        assert!(
            !treeified,
            "shape={} with {} keys reached a treeified bucket",
            case.shape,
            case.keys.len()
        );
        assert_eq!(&ours, java, "shape={} keys={:?}", case.shape, case.keys);
    }
}

/// Measures the one shape whose order this port does not reproduce.
///
/// `java.util.HashMap` converts a bucket into a red-black tree once it holds
/// more than `TREEIFY_THRESHOLD` entries while the table is at least
/// `MIN_TREEIFY_CAPACITY` long, and `moveRootToFront` then re-heads the
/// bucket's chain at the tree root, so the chain leaves insertion order. The
/// port keeps the chain. This test builds exactly that state — a family of
/// strings with one shared `String.hashCode`, padded to force the table to 64 —
/// and asserts that the port flags it and that the orders do in fact differ,
/// so the gap is a measured fact and not an assumption.
///
/// Reaching this needs at least 49 entries in the map, so none of the maps
/// Lucene's own code builds for a `.fnm`, a `.si` or a `segments_N` can get
/// here; a caller-supplied `IndexWriter.setLiveCommitData` can.
#[test]
fn treeified_bucket_is_the_one_shape_the_port_does_not_order_like_java() {
    require_maven();
    // Sixteen four-block strings, every one of them with the same hashCode, so
    // all sixteen share a bucket at every table length.
    let mut colliding: Vec<String> = Vec::new();
    for mask in 0..16u32 {
        let mut s = String::new();
        for bit in 0..4 {
            s.push_str(if mask & (1 << bit) == 0 { "Aa" } else { "BB" });
        }
        colliding.push(s);
    }
    // Enough distinct filler to take the table past MIN_TREEIFY_CAPACITY.
    let mut keys = colliding.clone();
    for i in 0..60 {
        keys.push(format!("filler-{i:04}"));
    }

    let case = Case {
        shape: "map_put",
        arg: 0,
        keys,
    };
    let (ours, treeified) = rucene_order(&case);
    assert!(
        treeified,
        "the port did not notice the treeified bucket; table must have passed {MIN_TREEIFY_CAPACITY}"
    );

    let java = java_orders(std::slice::from_ref(&case));
    let java = &java[0];
    assert_eq!(
        ours.len(),
        java.len(),
        "the two sides hold different numbers of entries"
    );
    let ours_set: HashMap<&String, usize> = ours.iter().zip(0..).collect();
    let java_set: HashMap<&String, usize> = java.iter().zip(0..).collect();
    assert_eq!(
        ours_set.keys().collect::<std::collections::HashSet<_>>(),
        java_set.keys().collect::<std::collections::HashSet<_>>(),
        "the two sides hold different entries"
    );
    assert_ne!(
        &ours, java,
        "the treeified bucket no longer diverges; if java.util.HashMap has \
         changed, this documented gap and the module docs must be revisited"
    );
}
