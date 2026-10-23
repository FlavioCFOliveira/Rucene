//! Portability test for the two numbers a `ByteBlockPool` reports about its
//! memory, both measured from `lucene-core-10.5.0.jar` in the same run.
//!
//! Nothing here writes an index file, so nothing here is a byte comparison.
//! What it pins is the other half of a port's fidelity: the numbers a Lucene
//! object answers about itself. Every expected value comes from the
//! `ByteBlockPoolAccountingFixture` harness, never from this crate's own
//! arithmetic — asserting an implementation against itself is exactly what let
//! the defect this test locks down survive.
//!
//! The two numbers are different things and are routinely conflated:
//!
//! 1. **The heap estimate**, `ByteBlockPool.ramBytesUsed()`. Java sums three
//!    terms: `BASE_RAM_BYTES`, the shallow size of the *whole* `byte[][]`
//!    reference array — the empty slots included — and `sizeOfObject(buf)` for
//!    each block, which adds a 16-byte array header to every one. A pool
//!    holding a single 32 KiB block therefore reports **32880**, not 32768;
//!    dropping the first term alone loses 112 bytes per pool. An empty pool
//!    reports 96, not 0.
//! 2. **The allocator's charge**, what `DirectTrackingAllocator` adds to the
//!    shared `Counter`: exactly `BYTE_BLOCK_SIZE` per block and nothing else.
//!    That is what the indexing chain's RAM budget is spent against, and it is
//!    never equal to the estimate.
//!
//! The reference array starts at `new byte[10][]`, grows through
//! `ArrayUtil.oversize(buffers.length + 1, NUM_BYTES_OBJECT_REF)`, and is never
//! shrunk — so after `reset(false, false)` the pool has given every block back
//! to the allocator (charge 0) while the estimate stays above the 96 bytes it
//! reported when new. The `ram_reset` rows pin that asymmetry, which is what a
//! port that recomputed the shallow size from the blocks it still holds would
//! get wrong.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;

use rucene::util::{Accountable, ByteBlockPool, RamUsageEstimator, BYTE_BLOCK_SIZE};

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

/// Runs the Java harness and returns its `key=value` lines.
fn run_fixture() -> Vec<(String, String)> {
    let mvn = which_mvn().unwrap_or_else(|e| {
        panic!("the byte-block-pool accounting portability test needs Maven and a JDK: {e}")
    });
    let output = Command::new(mvn)
        .arg("-q")
        .arg("compile")
        .arg("exec:java")
        .arg("-Dexec.mainClass=org.apache.lucene.rucene.codec.ByteBlockPoolAccountingFixture")
        .current_dir(harness_dir())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn Maven: {e}"));
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        output.status.success() && stdout.lines().any(|l| l.trim() == "ok=true"),
        "the Java fixture failed:\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    stdout
        .lines()
        .filter_map(|line| line.trim().split_once('='))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

#[test]
fn byte_block_pool_accounting_matches_lucene() {
    let measured = run_fixture();
    let get = |key: &str| -> i64 {
        measured
            .iter()
            .find(|(k, _)| k == key)
            .unwrap_or_else(|| panic!("the fixture printed no {key}"))
            .1
            .parse()
            .unwrap_or_else(|e| panic!("{key} is not a number: {e}"))
    };

    // The constants the estimate is built from.
    assert_eq!(
        ByteBlockPool::BASE_RAM_BYTES,
        get("base"),
        "BASE_RAM_BYTES is shallowSizeOfInstance(ByteBlockPool.class)"
    );
    assert_eq!(BYTE_BLOCK_SIZE as i64, get("block_size"), "BYTE_BLOCK_SIZE");
    assert_eq!(
        RamUsageEstimator::NUM_BYTES_OBJECT_REF,
        get("num_bytes_object_ref"),
    );
    assert_eq!(
        RamUsageEstimator::size_of(&vec![0u8; BYTE_BLOCK_SIZE]),
        get("size_of_block"),
        "sizeOfObject(new byte[BYTE_BLOCK_SIZE]) carries a 16-byte array header"
    );
    assert_eq!(
        RamUsageEstimator::shallow_size_of(&[(); 10]),
        get("shallow_of_ten_refs"),
        "shallowSizeOf(new byte[10][]), the reference array a fresh pool holds"
    );

    // The per-block-count rows, keyed by the number of blocks allocated.
    let mut rows: BTreeMap<i64, BTreeMap<&str, i64>> = BTreeMap::new();
    for (key, value) in &measured {
        let kind = match key.as_str() {
            k @ ("ram" | "charge" | "ram_reset" | "charge_reset") => k,
            _ => continue,
        };
        let (blocks, bytes) = value
            .split_once(':')
            .unwrap_or_else(|| panic!("{key} must be <blocks>:<bytes>, got {value}"));
        let blocks: i64 = blocks.parse().expect("block count");
        let bytes: i64 = bytes.parse().expect("byte count");
        rows.entry(blocks).or_default().insert(kind, bytes);
    }
    assert!(
        !rows.is_empty(),
        "the fixture printed no per-block-count rows"
    );

    for (blocks, expected) in &rows {
        let charged = Arc::new(AtomicI64::new(0));
        let mut pool = ByteBlockPool::new(Arc::clone(&charged));
        for _ in 0..*blocks {
            pool.next_buffer().expect("next_buffer");
        }

        assert_eq!(
            pool.ram_bytes_used(),
            expected["ram"],
            "ramBytesUsed() after {blocks} blocks"
        );
        assert_eq!(
            charged.load(Ordering::Acquire),
            expected["charge"],
            "the allocator's charge after {blocks} blocks is BYTE_BLOCK_SIZE per block, \
             which is not what ramBytesUsed() reports"
        );

        pool.reset();
        assert_eq!(
            pool.ram_bytes_used(),
            expected["ram_reset"],
            "ramBytesUsed() after resetting a pool that held {blocks} blocks: the blocks are \
             gone but the reference array keeps the length it grew to"
        );
        assert_eq!(
            charged.load(Ordering::Acquire),
            expected["charge_reset"],
            "resetting a pool that held {blocks} blocks hands every charged byte back"
        );
    }
}
