//! `IndexFileDeleter` portability tests against Apache Lucene Core 10.5.0.
//!
//! These tests drive the Java reference harness in
//! `tests/fixtures/java-codec-harness` (class `IndexFileDeleterFixture`), which
//! builds **real** Lucene indexes with a real `IndexWriter` and reports the
//! file-lifecycle decisions Lucene's own `IndexFileDeleter` made on them.
//!
//! The Rust side then opens the very same Java-written directory with Rucene's
//! [`IndexFileDeleter`] and must reach an identical outcome. That proves two
//! things at once, both required by `CLAUDE.md` §14.3:
//!
//! - Rucene **reads** an index written by the Java implementation correctly —
//!   it reconstructs the same commit points, with the same generations and the
//!   same per-commit file sets, from the `segments_N` files on disk;
//! - Rucene **reproduces Lucene's behaviour** on that index — under the same
//!   deletion policy it deletes exactly the files Lucene deletes, and keeps
//!   exactly the files Lucene keeps.
//!
//! Because `IndexWriter` is not ported yet, Rucene cannot *produce* one of these
//! indexes; it can only consume one. That is why every shape here starts from a
//! Java-written directory rather than from a Rucene-written one.

use std::collections::{BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

use rucene::codecs::{register_codec, Lucene104Codec};
use rucene::index::{
    IndexDeletionPolicy, IndexFileDeleter, KeepOnlyLastCommitDeletionPolicy, NoDeletionPolicy,
    OpenMode, PersistentSnapshotDeletionPolicy, SegmentInfos,
};
use rucene::store::{Directory, NIOFSDirectory};
use rucene::util::NoOutputInfoStream;

// -----------------------------------------------------------------------------
// Java harness plumbing
// -----------------------------------------------------------------------------

/// Path to the Java codec harness, relative to `CARGO_MANIFEST_DIR`.
fn harness_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/java-codec-harness")
}

/// Serialises calls into the Java harness so that parallel Maven executions do
/// not fight over the shared `target/` directory.
static HARNESS_LOCK: Mutex<()> = Mutex::new(());

/// Runs `IndexFileDeleterFixture` for `shape`, using `out_dir` as its workspace.
fn run_java_fixture(out_dir: &Path, shape: &str) -> String {
    let _guard = HARNESS_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let harness = harness_dir();
    assert!(
        harness.join("pom.xml").exists(),
        "pom.xml not found in {}",
        harness.display()
    );

    let output = Command::new(which_mvn())
        .arg("-q")
        .arg("-o")
        .arg("compile")
        .arg("exec:java")
        .arg("-Dexec.mainClass=org.apache.lucene.rucene.codec.IndexFileDeleterFixture")
        .arg(format!("-Dexec.args={} {}", out_dir.display(), shape))
        .current_dir(&harness)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("failed to spawn Maven");

    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        output.status.success(),
        "Java fixture `{shape}` failed with status {:?}\nstdout:\n{stdout}\nstderr:\n{stderr}",
        output.status.code()
    );
    assert!(
        stdout.contains("version=10.5.0"),
        "fixture must run against Lucene 10.5.0\ngot:\n{stdout}"
    );
    stdout
}

/// Locates the `mvn` executable.
fn which_mvn() -> PathBuf {
    for name in ["mvn", "mvn.cmd"] {
        if let Ok(output) = Command::new("which").arg(name).output() {
            if output.status.success() {
                let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !path.is_empty() {
                    return PathBuf::from(path);
                }
            }
        }
    }
    panic!("mvn not found on PATH; it is required by the Lucene portability harness");
}

/// Returns the value of a `key=value` line emitted by the fixture.
fn field<'a>(stdout: &'a str, key: &str) -> &'a str {
    let prefix = format!("{key}=");
    stdout
        .lines()
        .find_map(|line| line.strip_prefix(prefix.as_str()))
        .unwrap_or_else(|| panic!("fixture output has no `{key}=` line:\n{stdout}"))
        .trim()
}

/// Parses a comma-separated file list emitted under `key`.
fn file_set(stdout: &str, key: &str) -> BTreeSet<String> {
    let raw = field(stdout, key);
    if raw.is_empty() {
        return BTreeSet::new();
    }
    raw.split(',').map(str::to_string).collect()
}

/// Parses a comma-separated list of commit generations.
fn generations(stdout: &str, key: &str) -> Vec<i64> {
    let raw = field(stdout, key);
    if raw.is_empty() {
        return Vec::new();
    }
    raw.split(',')
        .map(|v| v.parse().expect("generation must be an integer"))
        .collect()
}

// -----------------------------------------------------------------------------
// Rust-side helpers
// -----------------------------------------------------------------------------

/// Registers the real Lucene 10.5.0 codec, so `.si` files written by Java can be
/// read back.
fn register_lucene_codec() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let _ = register_codec("Lucene104", Lucene104Codec::new());
    });
}

/// A scratch directory that removes itself when the test ends.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "rucene-ifd-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&path).expect("failed to create scratch directory");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Copies every regular file from `from` into a fresh `to`.
fn copy_index(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).expect("failed to create copy target");
    for entry in std::fs::read_dir(from).expect("failed to list source index") {
        let entry = entry.expect("failed to read directory entry");
        if entry.file_type().expect("failed to stat entry").is_file() {
            std::fs::copy(entry.path(), to.join(entry.file_name())).expect("failed to copy file");
        }
    }
}

/// Returns the sorted listing of `dir`, as the Java fixture reports it.
fn listing(dir: &Path) -> BTreeSet<String> {
    std::fs::read_dir(dir)
        .expect("failed to list directory")
        .map(|e| {
            e.expect("failed to read entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect()
}

/// Opens `dir` and builds an `IndexFileDeleter` over it under `policy`, exactly
/// as `IndexWriter` does on start-up.
fn open_deleter(
    dir: &Path,
    policy: Arc<dyn IndexDeletionPolicy>,
) -> (IndexFileDeleter, SegmentInfos) {
    register_lucene_codec();
    let directory: Arc<dyn Directory> = Arc::new(NIOFSDirectory::open(dir).unwrap());
    let mut sis = SegmentInfos::read_latest_commit(directory.as_ref())
        .expect("Rucene must read the Java-written commit");
    let files = directory.list_all().unwrap();
    let deleter = IndexFileDeleter::new(
        &files,
        Arc::clone(&directory),
        Arc::clone(&directory),
        policy,
        &mut sis,
        Arc::new(NoOutputInfoStream),
        true,
        false,
    )
    .expect("IndexFileDeleter must initialise over a Java-written index");
    (deleter, sis)
}

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

/// Rucene must reconstruct exactly the commit points Lucene sees in a
/// multi-generation index, with the same generations and the same file sets.
///
/// This is the read half of the parity claim: the commit points are rebuilt from
/// the `segments_N` files Java wrote, using the real `Lucene104` codec.
#[test]
fn reconstructs_every_commit_point_of_a_java_written_index() {
    let scratch = Scratch::new("retained");
    let stdout = run_java_fixture(scratch.path(), "retained");

    let java_gens = generations(&stdout, "commit_gens");
    assert_eq!(java_gens, vec![1, 2, 3, 4, 5], "fixture sanity check");

    let index = scratch.path().join("index");
    // NoDeletionPolicy keeps every generation, so the deleter must report all of
    // them, in the same oldest-to-newest order Lucene sorts them into.
    let (deleter, _sis) = open_deleter(&index, NoDeletionPolicy::instance());

    let rust_gens: Vec<i64> = deleter
        .commits()
        .iter()
        .map(|c| c.get_generation())
        .collect();
    assert_eq!(
        rust_gens, java_gens,
        "Rucene must rebuild the same commit generations Lucene reports"
    );

    for gen in &java_gens {
        let java_files = file_set(&stdout, &format!("commit_files_{gen}"));
        let commit = deleter
            .commits()
            .into_iter()
            .find(|c| c.get_generation() == *gen)
            .expect("generation must be present");
        let rust_files: BTreeSet<String> = commit.get_file_names().unwrap().into_iter().collect();
        assert_eq!(
            rust_files, java_files,
            "commit {gen}: Rucene's file set must match Lucene's"
        );
    }

    // Nothing may have been deleted: every generation is still referenced.
    assert_eq!(
        listing(&index),
        file_set(&stdout, "listing"),
        "no file may be removed while every commit is retained"
    );
}

/// Under `KeepOnlyLastCommitDeletionPolicy`, Rucene must delete exactly the
/// files Lucene deletes when reopening the same multi-generation index.
///
/// The Java fixture performs the reopen itself on its own copy and reports the
/// resulting listing; Rucene performs it on an independent copy of the identical
/// starting state. The two listings must agree file for file.
#[test]
fn keep_only_last_deletes_exactly_what_lucene_deletes() {
    let scratch = Scratch::new("keeponlylast");
    let stdout = run_java_fixture(scratch.path(), "reopen-keep-only-last");

    let java_before = file_set(&stdout, "before");
    let java_after = file_set(&stdout, "after");
    assert_ne!(
        java_before, java_after,
        "fixture sanity check: the reopen must actually delete something"
    );
    assert_eq!(generations(&stdout, "after_commit_gens"), vec![5]);

    // Start from the identical state Java started from.
    let rucene_dir = scratch.path().join("rucene");
    copy_index(&scratch.path().join("index"), &rucene_dir);
    assert_eq!(
        listing(&rucene_dir),
        java_before,
        "the copy must reproduce the starting state exactly"
    );

    let (deleter, _sis) = open_deleter(&rucene_dir, Arc::new(KeepOnlyLastCommitDeletionPolicy));

    assert_eq!(
        listing(&rucene_dir),
        java_after,
        "Rucene must delete exactly the files Lucene deletes"
    );

    let gens: Vec<i64> = deleter
        .commits()
        .iter()
        .map(|c| c.get_generation())
        .collect();
    assert_eq!(
        gens,
        generations(&stdout, "after_commit_gens"),
        "only the newest commit may survive"
    );

    // The surviving commit's file set must still match Lucene's.
    let commit = &deleter.commits()[0];
    let rust_files: BTreeSet<String> = commit.get_file_names().unwrap().into_iter().collect();
    assert_eq!(rust_files, file_set(&stdout, "after_commit_files_5"));
}

/// Rucene must remove exactly the unreferenced files Lucene removes on init, and
/// leave alone the ones Lucene leaves alone.
///
/// The distinction is not obvious: a file matching the codec name pattern is
/// adopted with a reference count of zero and then deleted, whereas a file that
/// matches neither the codec pattern nor `segments`/`pending_segments` is not
/// the deleter's business at all.
#[test]
fn removes_the_same_orphans_lucene_removes() {
    let scratch = Scratch::new("orphans");
    let stdout = run_java_fixture(scratch.path(), "orphan-cleanup");

    let java_before = file_set(&stdout, "before");
    let java_after = file_set(&stdout, "after");
    assert!(
        java_before.contains("_9.fdt") && !java_after.contains("_9.fdt"),
        "fixture sanity check: Lucene must delete the orphan"
    );
    assert!(
        java_after.contains("unrelated.txt"),
        "fixture sanity check: Lucene must keep a non-index file"
    );

    // Rebuild Java's *before* state on a clean copy: the fixture already
    // performed its own reopen, so we re-create the debris rather than copying
    // the post-reopen directory.
    let rucene_dir = scratch.path().join("rucene");
    copy_index(&scratch.path().join("orphans"), &rucene_dir);
    for orphan in ["_9.fdt", "_9.si"] {
        std::fs::write(rucene_dir.join(orphan), [0u8; 4]).unwrap();
    }
    std::fs::write(rucene_dir.join("unrelated.txt"), [0u8; 4]).unwrap();
    assert_eq!(
        listing(&rucene_dir),
        java_before,
        "the reconstructed starting state must match Java's"
    );

    let (_deleter, _sis) = open_deleter(&rucene_dir, Arc::new(KeepOnlyLastCommitDeletionPolicy));

    assert_eq!(
        listing(&rucene_dir),
        java_after,
        "Rucene must delete exactly the orphans Lucene deletes, and no more"
    );
}

/// A commit pinned by a persisted snapshot must survive in Rucene exactly as it
/// survives in Lucene.
///
/// The Java fixture writes five commits under `PersistentSnapshotDeletionPolicy`
/// with generation 2 snapshotted, so Lucene keeps generations 2 and 5 even
/// though the wrapped policy is `KeepOnlyLastCommitDeletionPolicy`. Rucene loads
/// the same persisted `snapshots_N` state and must reach the same conclusion.
#[test]
fn a_persisted_snapshot_pins_its_commit_in_rucene_as_in_lucene() {
    let scratch = Scratch::new("snapshot");
    let stdout = run_java_fixture(scratch.path(), "snapshot-pin");

    let pinned_gen: i64 = field(&stdout, "pinned_gen").parse().unwrap();
    let java_gens = generations(&stdout, "commit_gens");
    let java_listing = file_set(&stdout, "listing");
    assert_eq!(pinned_gen, 2, "fixture sanity check");
    assert_eq!(
        java_gens,
        vec![2, 5],
        "fixture sanity check: the snapshot must hold generation 2 open"
    );

    register_lucene_codec();

    // Work on a copy so the Java artefacts stay intact for diagnosis.
    let rucene_dir = scratch.path().join("rucene");
    let rucene_state = scratch.path().join("rucene-state");
    copy_index(&scratch.path().join("snapshot"), &rucene_dir);
    copy_index(&scratch.path().join("snapshot-state"), &rucene_state);

    // Load the snapshot state Java persisted. APPEND reads the existing
    // `snapshots_N` rather than starting a fresh one.
    let state_dir: Arc<dyn Directory> = Arc::new(NIOFSDirectory::open(&rucene_state).unwrap());
    let policy = PersistentSnapshotDeletionPolicy::with_open_mode(
        Arc::new(KeepOnlyLastCommitDeletionPolicy),
        state_dir,
        OpenMode::APPEND,
    )
    .expect("Rucene must read the snapshots file Java wrote");

    let directory: Arc<dyn Directory> = Arc::new(NIOFSDirectory::open(&rucene_dir).unwrap());
    let mut sis = SegmentInfos::read_latest_commit(directory.as_ref()).unwrap();
    let files = directory.list_all().unwrap();
    let deleter = IndexFileDeleter::new(
        &files,
        Arc::clone(&directory),
        Arc::clone(&directory),
        Arc::new(policy),
        &mut sis,
        Arc::new(NoOutputInfoStream),
        true,
        false,
    )
    .expect("IndexFileDeleter must initialise under a persisted snapshot policy");

    let gens: Vec<i64> = deleter
        .commits()
        .iter()
        .map(|c| c.get_generation())
        .collect();
    assert_eq!(
        gens, java_gens,
        "the snapshotted generation must survive `on_init` in Rucene as it does in Lucene"
    );

    assert_eq!(
        listing(&rucene_dir),
        java_listing,
        "no file the snapshot pins may be deleted"
    );

    // The pinned commit's own files must all still be present and referenced.
    let pinned = deleter
        .commits()
        .into_iter()
        .find(|c| c.get_generation() == pinned_gen)
        .expect("the pinned generation must be among the live commits");
    let pinned_files: HashSet<String> = pinned.get_file_names().unwrap();
    assert_eq!(
        pinned_files,
        file_set(&stdout, "pinned_files").into_iter().collect(),
        "Rucene must see the same pinned file set Lucene reported"
    );
    for file in &pinned_files {
        assert!(
            deleter.exists(file),
            "{file} is pinned by the snapshot and must hold a reference"
        );
        assert!(
            rucene_dir.join(file).exists(),
            "{file} is pinned by the snapshot and must still be on disk"
        );
    }
}
