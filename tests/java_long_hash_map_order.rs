//! `JavaLongHashMap` iterates exactly as JDK `java.util.HashMap<Long, V>`,
//! treeified bins included.
//!
//! `LongHashMapOrderFixture` (`tests/fixtures/java-codec-harness`, run with
//! `java LongHashMapOrderFixture.java`) replays a script on a real `HashMap`
//! and prints the entry order after every operation: it first fills the table
//! to 64 slots and puts 12 keys into one bucket — `treeifyBin` turns it into a
//! tree, and later puts link each node after its tree parent
//! (`HashMap.TreeNode.putTreeVal`) — then runs 3000 random puts, removes
//! (`removeTreeNode`, `untreeify`) and `putAll` copies over colliding keys, so
//! resizes split tree bins (`TreeNode.split`). The port runs the same script
//! and must print the same lines.

use std::path::Path;
use std::process::Command;

use rucene::util::java_collections::JavaLongHashMap;

fn order(map: &JavaLongHashMap<i32>) -> String {
    map.iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join(",")
}

fn rust_script() -> (Vec<String>, bool) {
    let mut universe = Vec::new();
    for b in [0i64, 1, 5] {
        for j in 0..40i64 {
            universe.push(b + 64 * j);
        }
    }
    for j in 1..=8i64 {
        universe.push((j << 32) | 3);
    }
    let mut lines = Vec::new();
    let mut treeified = false;
    let mut map: JavaLongHashMap<i32> = JavaLongHashMap::new();
    for k in 1..48i64 {
        map.insert(k + 1000, k as i32);
        lines.push(format!("put {}", order(&map)));
    }
    for j in 1..=12i64 {
        map.insert(64 * j, j as i32);
        lines.push(format!("put {}", order(&map)));
    }
    treeified |= map.table_length() >= 64 && map.is_treeified_at(64);
    let mut x: i64 = 42;
    for i in 0..3000i32 {
        x = x
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        let op = ((x as u64) >> 33) % 20;
        let key = universe[(((x as u64) >> 40) % universe.len() as u64) as usize];
        let tag = if op < 13 {
            map.insert(key, i);
            "put"
        } else if op < 19 {
            map.remove(key);
            "remove"
        } else {
            let mut copy = JavaLongHashMap::new();
            let entries: Vec<(i64, i32)> = map.iter().map(|(k, v)| (k, *v)).collect();
            copy.put_all(entries);
            map = copy;
            "putAll"
        };
        treeified |= universe.iter().any(|&k| map.is_treeified_at(k));
        lines.push(format!("{tag} {}", order(&map)));
    }
    (lines, treeified)
}

#[test]
fn iteration_order_matches_java_hash_map_with_treeified_bins() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join(
        "tests/fixtures/java-codec-harness/src/main/java/org/apache/lucene/rucene/codec/\
         LongHashMapOrderFixture.java",
    );
    let output = Command::new("java")
        .arg(&fixture)
        .output()
        .expect("a JDK is required to run the HashMap order fixture");
    assert!(
        output.status.success(),
        "fixture failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let java = String::from_utf8(output.stdout).unwrap();
    let java: Vec<&str> = java.lines().collect();
    let (rust, treeified) = rust_script();
    assert!(treeified, "the script must reach treeified bins");
    assert_eq!(java.len(), rust.len());
    for (step, (j, r)) in java.iter().zip(&rust).enumerate() {
        assert_eq!(*j, r.as_str(), "iteration order diverges at step {step}");
    }
}
