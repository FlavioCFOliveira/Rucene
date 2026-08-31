//! Deletion-policy and commit-point portability tests against Apache Lucene
//! Core 10.5.0.
//!
//! These tests drive the Java reference harness in
//! `tests/fixtures/java-codec-harness` (class `DeletionPolicyFixture`), which
//! runs a real `IndexWriter` under each of the ported deletion policies and
//! reports which `segments_N` generations survive. The Rust side then replays
//! the exact same commit sequence through Rucene's policies and asserts that
//! the same generations survive.
//!
//! For [`PersistentSnapshotDeletionPolicy`] the test goes further and checks
//! index-file compatibility in both directions:
//!
//! - Rucene reads the `snapshots_N` file that Java wrote and recovers the same
//!   reference counts;
//! - Rucene writes a `snapshots_N` file that Java reads back with its own
//!   `PersistentSnapshotDeletionPolicy`.
//!
//! Entry order inside `snapshots_N` is not part of the format: Rucene sorts by
//! commit generation, Java emits `HashMap` bucket order. The two files are
//! byte-identical exactly when those orders agree — which they do for the
//! generations the reference fixture produces, and which the test therefore
//! asserts as a *property of that vector*, not as a general guarantee.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
use std::sync::{Arc, Mutex};

use rucene::codecs::{register_codec, Lucene104Codec};
use rucene::error::{LuceneError, Result};
use rucene::index::{
    execute_two_phase_commit, list_commits, IndexCommit, IndexDeletionPolicy,
    KeepLastNCommitsDeletionPolicy, KeepOnlyLastCommitDeletionPolicy, NoDeletionPolicy, OpenMode,
    PersistentSnapshotDeletionPolicy, SnapshotDeletionPolicy, TwoPhaseCommit, TwoPhaseCommitError,
};
use rucene::store::{
    DataOutput, Directory, IOContext, IndexInput, IndexOutput, Lock, NIOFSDirectory,
};

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

/// Runs `DeletionPolicyFixture` for `shape`, using `out_dir` as its workspace.
fn run_java_fixture(out_dir: &Path, shape: &str) -> String {
    let _guard = HARNESS_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let harness = harness_dir();
    assert!(
        harness.join("pom.xml").exists(),
        "pom.xml not found in {}",
        harness.display()
    );

    let mvn = which_mvn();
    let output = Command::new(mvn)
        .arg("-q")
        .arg("-o")
        .arg("compile")
        .arg("exec:java")
        .arg("-Dexec.mainClass=org.apache.lucene.rucene.codec.DeletionPolicyFixture")
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

/// Parses a `1,2,3` list of commit generations.
fn generations(stdout: &str, key: &str) -> Vec<i64> {
    let raw = field(stdout, key);
    if raw.is_empty() {
        return Vec::new();
    }
    raw.split(',')
        .map(|v| v.parse().expect("generation must be an integer"))
        .collect()
}

/// Returns every value emitted under `key`, in fixture order.
fn repeated<'a>(stdout: &'a str, key: &str) -> Vec<&'a str> {
    let prefix = format!("{key}=");
    stdout
        .lines()
        .filter_map(|line| line.strip_prefix(prefix.as_str()))
        .map(str::trim)
        .collect()
}

/// Parses a `gen:refCount,gen:refCount` map emitted under `key`.
fn ref_counts_of(stdout: &str, key: &str) -> HashMap<i64, i32> {
    let raw = field(stdout, key);
    if raw.is_empty() {
        return HashMap::new();
    }
    raw.split(',')
        .map(|entry| {
            let (gen, count) = entry.split_once(':').expect("entry must be gen:count");
            (
                gen.parse().expect("generation must be an integer"),
                count.parse().expect("ref count must be an integer"),
            )
        })
        .collect()
}

/// Parses the `refcounts` map.
fn ref_counts(stdout: &str) -> HashMap<i64, i32> {
    ref_counts_of(stdout, "refcounts")
}

// -----------------------------------------------------------------------------
// Rucene-side commit points and IndexWriter simulation
// -----------------------------------------------------------------------------

/// A commit point that records whether the policy asked for its deletion.
///
/// This mirrors what `IndexFileDeleter.CommitPoint` does in Java: it marks the
/// commit as deleted so the writer can drop it from the live set.
struct RecordingCommit {
    directory: Arc<dyn Directory>,
    generation: i64,
    deleted: AtomicBool,
}

impl std::fmt::Debug for RecordingCommit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // `dyn Directory` is not `Debug`, so the commit is identified by the
        // only attribute the policies observe.
        f.debug_struct("RecordingCommit")
            .field("generation", &self.generation)
            .field("deleted", &self.is_deleted())
            .finish()
    }
}

impl RecordingCommit {
    fn at_generation(directory: Arc<dyn Directory>, generation: i64) -> Arc<dyn IndexCommit> {
        Arc::new(Self {
            directory,
            generation,
            deleted: AtomicBool::new(false),
        })
    }
}

impl IndexCommit for RecordingCommit {
    fn get_segments_file_name(&self) -> String {
        format!("segments_{}", self.generation)
    }

    fn get_file_names(&self) -> Result<HashSet<String>> {
        Ok(HashSet::from([self.get_segments_file_name()]))
    }

    fn get_directory(&self) -> Arc<dyn Directory> {
        Arc::clone(&self.directory)
    }

    fn delete(&self) -> Result<()> {
        self.deleted.store(true, AtomicOrdering::SeqCst);
        Ok(())
    }

    fn is_deleted(&self) -> bool {
        self.deleted.load(AtomicOrdering::SeqCst)
    }

    fn get_segment_count(&self) -> i32 {
        1
    }

    fn get_generation(&self) -> i64 {
        self.generation
    }

    fn get_user_data(&self) -> Result<HashMap<String, String>> {
        Ok(HashMap::new())
    }
}

/// Replays the commit protocol an `IndexWriter` follows and returns the
/// generations that survive.
///
/// Lucene calls `onInit` once when the writer opens (with an empty list for a
/// new index) and `onCommit` after every commit, always passing the live
/// commits oldest-first; commits the policy deleted are dropped from that list.
/// `after_commit` runs once per commit, letting a test snapshot a generation
/// exactly where the Java fixture does.
fn replay_commits(
    policy: &dyn IndexDeletionPolicy,
    created: &[i64],
    mut after_commit: impl FnMut(i64),
) -> Vec<i64> {
    let directory: Arc<dyn Directory> = Arc::new(rucene::store::RamDirectory::default());
    let mut live: Vec<Arc<dyn IndexCommit>> = Vec::new();

    policy.on_init(&live).expect("on_init must succeed");

    for generation in created {
        live.push(RecordingCommit::at_generation(
            Arc::clone(&directory),
            *generation,
        ));
        policy.on_commit(&live).expect("on_commit must succeed");
        live.retain(|commit| !commit.is_deleted());
        after_commit(*generation);
    }

    live.iter().map(|commit| commit.get_generation()).collect()
}

/// Returns the generations of `commits`, in order.
fn surviving(commits: &[Arc<dyn IndexCommit>]) -> Vec<i64> {
    commits
        .iter()
        .map(|commit| commit.get_generation())
        .collect()
}

/// Returns the generations of `commits`, sorted — the Java fixture sorts the
/// output of `getSnapshots()`, whose own order is a `HashMap`'s.
fn surviving_of(commits: &[Arc<dyn IndexCommit>]) -> Vec<i64> {
    let mut generations = surviving(commits);
    generations.sort_unstable();
    generations
}

/// Creates a scratch directory under `target/` for a fixture run.
fn scratch_dir(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("portability-deletion-policy")
        .join(name);
    if dir.exists() {
        std::fs::remove_dir_all(&dir).expect("failed to clear scratch directory");
    }
    std::fs::create_dir_all(&dir).expect("failed to create scratch directory");
    dir
}

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

#[test]
fn keep_only_last_matches_lucene() {
    let out = scratch_dir("keep-only-last");
    let stdout = run_java_fixture(&out, "keep-only-last");

    let created = generations(&stdout, "created_generations");
    let expected = generations(&stdout, "surviving_generations");
    assert_eq!(created, vec![1, 2, 3, 4, 5]);
    assert_eq!(expected, vec![5]);

    let surviving = replay_commits(&KeepOnlyLastCommitDeletionPolicy::new(), &created, |_| {});
    assert_eq!(surviving, expected);
}

#[test]
fn keep_last_n_matches_lucene() {
    let out = scratch_dir("keep-last-2");
    let stdout = run_java_fixture(&out, "keep-last-2");

    let created = generations(&stdout, "created_generations");
    let expected = generations(&stdout, "surviving_generations");
    assert_eq!(expected, vec![4, 5]);

    let policy = KeepLastNCommitsDeletionPolicy::new(2).unwrap();
    assert_eq!(replay_commits(&policy, &created, |_| {}), expected);
}

#[test]
fn keep_last_n_larger_than_the_history_matches_lucene() {
    let out = scratch_dir("keep-last-10");
    let stdout = run_java_fixture(&out, "keep-last-10");

    let created = generations(&stdout, "created_generations");
    let expected = generations(&stdout, "surviving_generations");
    assert_eq!(expected, created);

    let policy = KeepLastNCommitsDeletionPolicy::new(10).unwrap();
    assert_eq!(replay_commits(&policy, &created, |_| {}), expected);
}

#[test]
fn no_deletion_policy_matches_lucene() {
    let out = scratch_dir("no-deletion");
    let stdout = run_java_fixture(&out, "no-deletion");

    let created = generations(&stdout, "created_generations");
    let expected = generations(&stdout, "surviving_generations");
    assert_eq!(expected, created);

    assert_eq!(
        replay_commits(NoDeletionPolicy::instance().as_ref(), &created, |_| {}),
        expected
    );
}

#[test]
fn snapshot_deletion_policy_pins_the_same_commits_as_lucene() {
    let out = scratch_dir("snapshot");
    let stdout = run_java_fixture(&out, "snapshot");

    let created = generations(&stdout, "created_generations");
    let snapshotted = generations(&stdout, "snapshotted_generations");
    let expected = generations(&stdout, "surviving_generations");
    let expected_count: i32 = field(&stdout, "snapshot_count").parse().unwrap();
    assert_eq!(snapshotted, vec![2, 4]);
    assert_eq!(expected, vec![2, 4, 5]);

    let policy = SnapshotDeletionPolicy::new(Arc::new(KeepOnlyLastCommitDeletionPolicy::new()));
    let mut pinned = Vec::new();
    let surviving = replay_commits(&policy, &created, |generation| {
        // The Java fixture snapshots right after the 2nd and 4th commit.
        if snapshotted.contains(&generation) {
            pinned.push(policy.snapshot().unwrap().get_generation());
        }
    });

    assert_eq!(pinned, snapshotted);
    assert_eq!(surviving, expected);
    assert_eq!(policy.get_snapshot_count(), expected_count);
}

#[test]
fn persistent_snapshot_deletion_policy_is_index_compatible_with_lucene() {
    let out = scratch_dir("persistent-snapshot");
    let stdout = run_java_fixture(&out, "persistent-snapshot");

    let created = generations(&stdout, "created_generations");
    let snapshotted = generations(&stdout, "snapshotted_generations");
    let expected = generations(&stdout, "surviving_generations");
    let expected_counts = ref_counts(&stdout);
    let expected_count: i32 = field(&stdout, "snapshot_count").parse().unwrap();
    let java_save_file = field(&stdout, "last_save_file").to_string();

    assert_eq!(snapshotted, vec![2, 4, 4]);
    assert_eq!(expected, vec![2, 4, 5]);
    assert_eq!(expected_counts, HashMap::from([(2, 1), (4, 2)]));
    assert_eq!(java_save_file, "snapshots_2");

    let java_snapshots_dir = out.join("snapshots");
    let java_bytes = std::fs::read(java_snapshots_dir.join(&java_save_file))
        .expect("Java must have written a snapshots file");

    // --- Direction 1: Rucene reads the file Lucene wrote. ---------------------
    let read_back = PersistentSnapshotDeletionPolicy::with_open_mode(
        Arc::new(KeepOnlyLastCommitDeletionPolicy::new()),
        Arc::new(NIOFSDirectory::open(&java_snapshots_dir).unwrap()),
        OpenMode::APPEND,
    )
    .expect("Rucene must load the Lucene-written snapshots file");
    assert_eq!(read_back.get_snapshot_count(), expected_count);
    assert_eq!(
        read_back.get_last_save_file().as_deref(),
        Some(java_save_file.as_str())
    );

    // --- Direction 2: Rucene reproduces the same state and the same bytes. ----
    let rucene_dir = scratch_dir("persistent-snapshot-rucene");
    let policy = PersistentSnapshotDeletionPolicy::new(
        Arc::new(KeepOnlyLastCommitDeletionPolicy::new()),
        Arc::new(NIOFSDirectory::open(&rucene_dir).unwrap()),
    )
    .unwrap();

    let mut pinned = Vec::new();
    let surviving = replay_commits(&policy, &created, |generation| {
        for _ in 0..snapshotted.iter().filter(|g| **g == generation).count() {
            pinned.push(policy.snapshot().unwrap().get_generation());
        }
    });

    assert_eq!(pinned, snapshotted);
    assert_eq!(surviving, expected);
    assert_eq!(policy.get_snapshot_count(), expected_count);

    let rucene_save_file = policy.get_last_save_file().expect("a file must be saved");
    assert_eq!(rucene_save_file, java_save_file);
    let rucene_bytes = std::fs::read(rucene_dir.join(&rucene_save_file)).unwrap();

    // Byte identity is a property of *this vector*, not a general guarantee.
    // Entry order is not part of the format: Rucene emits ascending commit
    // generations, Java emits `HashMap` bucket order, i.e.
    // `generation & (capacity - 1)`. For the generations this fixture pins —
    // 2 and 4, both below the initial capacity of 16 — the two orders coincide,
    // and only then are the files identical. With, say, generations 2 and 17
    // Java would emit `[17, 2]` and the bytes would differ while both files
    // stayed perfectly readable by both implementations.
    let mut java_generations: Vec<i64> = expected_counts.keys().copied().collect();
    java_generations.sort_unstable();
    assert!(
        java_generations
            .iter()
            .all(|generation| *generation < 16 && *generation >= 0),
        "this vector only pins byte identity while Java's bucket order is \
         ascending; generations {java_generations:?} no longer guarantee that"
    );
    assert_eq!(
        rucene_bytes, java_bytes,
        "for this vector Rucene must write a byte-identical snapshots_N file:\n rucene={rucene_bytes:02x?}\n   java={java_bytes:02x?}"
    );

    // Only the newest save file is kept, exactly as Lucene does.
    let saved: Vec<String> = std::fs::read_dir(&rucene_dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with("snapshots_"))
        .collect();
    assert_eq!(saved, vec![rucene_save_file.clone()]);

    // --- Direction 3: Lucene reads the file Rucene wrote. ---------------------
    let java_read = run_java_fixture(&rucene_dir, "read-snapshots");
    assert_eq!(field(&java_read, "last_save_file"), rucene_save_file);
    assert_eq!(
        field(&java_read, "snapshot_count"),
        expected_count.to_string()
    );
    assert_eq!(ref_counts(&java_read), expected_counts);
}

// -----------------------------------------------------------------------------
// onInit over an existing index: snapshots must survive a writer reopen
// -----------------------------------------------------------------------------

#[test]
fn snapshot_pins_survive_a_writer_reopen_like_lucene() {
    let out = scratch_dir("reopen-snapshot");
    let stdout = run_java_fixture(&out, "reopen-snapshot");

    let phase1_created = generations(&stdout, "phase1_created_generations");
    let phase1_surviving = generations(&stdout, "phase1_surviving_generations");
    let pinned: i64 = field(&stdout, "phase1_snapshotted").parse().unwrap();
    let phase2_created = generations(&stdout, "phase2_created_generations");
    let phase2_surviving = generations(&stdout, "phase2_surviving_generations");
    let loaded_counts = ref_counts_of(&stdout, "phase2_loaded_refcounts");
    let loaded_count: i32 = field(&stdout, "phase2_loaded_count").parse().unwrap();
    let before_oninit = generations(&stdout, "phase2_snapshots_before_oninit");
    let after_oninit = generations(&stdout, "phase2_snapshots_after_oninit");

    assert_eq!(phase1_created, vec![1, 2, 3]);
    assert_eq!(phase1_surviving, vec![3]);
    assert_eq!(pinned, 3);
    assert_eq!(phase2_created, vec![4, 5]);
    assert_eq!(phase2_surviving, vec![3, 5]);
    assert_eq!(loaded_counts, HashMap::from([(3, 1)]));
    assert_eq!(loaded_count, 1);
    assert!(
        before_oninit.is_empty(),
        "Lucene loads the reference counts but attaches no commit until onInit"
    );
    assert_eq!(after_oninit, vec![3]);

    // --- Phase 1: build the index and pin its last commit. -------------------
    let snapshots_dir = scratch_dir("reopen-snapshot-rucene");
    let directory: Arc<dyn Directory> = Arc::new(rucene::store::RamDirectory::default());

    let mut live: Vec<Arc<dyn IndexCommit>> = Vec::new();
    {
        let policy = PersistentSnapshotDeletionPolicy::new(
            Arc::new(KeepOnlyLastCommitDeletionPolicy::new()),
            Arc::new(NIOFSDirectory::open(&snapshots_dir).unwrap()),
        )
        .unwrap();
        policy.on_init(&live).unwrap();
        for generation in &phase1_created {
            live.push(RecordingCommit::at_generation(
                Arc::clone(&directory),
                *generation,
            ));
            policy.on_commit(&live).unwrap();
            live.retain(|commit| !commit.is_deleted());
        }
        assert_eq!(policy.snapshot().unwrap().get_generation(), pinned);
        assert_eq!(surviving(&live), phase1_surviving);
        // The policy goes out of scope here: everything the reopen relies on
        // has to come back from `snapshots_N`.
    }

    // --- Phase 2: a brand-new policy over the same save directory. -----------
    let reopened = PersistentSnapshotDeletionPolicy::with_open_mode(
        Arc::new(KeepOnlyLastCommitDeletionPolicy::new()),
        Arc::new(NIOFSDirectory::open(&snapshots_dir).unwrap()),
        OpenMode::APPEND,
    )
    .expect("the reopen must find the persisted snapshots");
    assert_eq!(reopened.get_snapshot_count(), loaded_count);
    assert!(
        reopened.get_snapshots().is_empty(),
        "no commit can be attached before on_init has seen the index"
    );

    // Reopening the index hands `on_init` the commits that are still on disk;
    // this is the re-attachment loop of `SnapshotDeletionPolicy.java:73-77`,
    // which the CREATE-only shapes never reach.
    let mut live: Vec<Arc<dyn IndexCommit>> = phase1_surviving
        .iter()
        .map(|generation| RecordingCommit::at_generation(Arc::clone(&directory), *generation))
        .collect();
    reopened.on_init(&live).unwrap();
    live.retain(|commit| !commit.is_deleted());

    assert_eq!(surviving_of(&reopened.get_snapshots()), after_oninit);
    assert!(
        reopened.get_index_commit(pinned).is_some(),
        "the pinned generation must be re-attached to a live commit point"
    );

    for generation in &phase2_created {
        live.push(RecordingCommit::at_generation(
            Arc::clone(&directory),
            *generation,
        ));
        reopened.on_commit(&live).unwrap();
        live.retain(|commit| !commit.is_deleted());
    }

    assert_eq!(surviving(&live), phase2_surviving);
    assert_eq!(
        reopened.get_snapshot_count(),
        field(&stdout, "phase2_count").parse::<i32>().unwrap()
    );
}

// -----------------------------------------------------------------------------
// release(): unpinning must free exactly the same commits
// -----------------------------------------------------------------------------

#[test]
fn release_unpins_the_same_commits_as_lucene() {
    let out = scratch_dir("release");
    let stdout = run_java_fixture(&out, "release");

    let created = generations(&stdout, "created_generations");
    let snapshotted = generations(&stdout, "snapshotted_generations");
    let released: i64 = field(&stdout, "released").parse().unwrap();
    let surviving_before = generations(&stdout, "surviving_before_release");
    let expected = generations(&stdout, "surviving_generations");
    let counts_before = ref_counts_of(&stdout, "refcounts_before_release");
    let counts_after = ref_counts_of(&stdout, "refcounts_after_release");
    let count_before: i32 = field(&stdout, "count_before_release").parse().unwrap();
    let count_after: i32 = field(&stdout, "count_after_release").parse().unwrap();
    let snapshots_after = generations(&stdout, "snapshots_after_release");

    assert_eq!(created, vec![1, 2, 3, 4, 5, 6]);
    assert_eq!(snapshotted, vec![2, 4]);
    assert_eq!(released, 2);
    assert_eq!(surviving_before, vec![2, 4, 5]);
    assert_eq!(expected, vec![4, 6]);
    assert_eq!(counts_before, HashMap::from([(2, 1), (4, 1)]));
    assert_eq!(counts_after, HashMap::from([(4, 1)]));
    assert_eq!((count_before, count_after), (2, 1));
    assert_eq!(snapshots_after, vec![4]);

    let policy = SnapshotDeletionPolicy::new(Arc::new(KeepOnlyLastCommitDeletionPolicy::new()));
    let directory: Arc<dyn Directory> = Arc::new(rucene::store::RamDirectory::default());
    let mut live: Vec<Arc<dyn IndexCommit>> = Vec::new();
    policy.on_init(&live).unwrap();

    // The fixture takes its two snapshots after the 2nd and 4th commit and
    // releases the older one right before the last commit.
    let last = *created.last().unwrap();
    for generation in &created {
        if *generation == last {
            assert_eq!(surviving(&live), surviving_before);
            assert_eq!(policy.get_snapshot_count(), count_before);

            let pinned = policy
                .get_index_commit(released)
                .expect("the released generation must be held before release()");
            policy.release(pinned.as_ref()).unwrap();

            assert_eq!(policy.get_snapshot_count(), count_after);
            assert_eq!(surviving_of(&policy.get_snapshots()), snapshots_after);
            assert!(
                live.iter().any(|c| c.get_generation() == released),
                "releasing must not delete anything by itself; the next \
                 checkpoint does"
            );
        }
        live.push(RecordingCommit::at_generation(
            Arc::clone(&directory),
            *generation,
        ));
        policy.on_commit(&live).unwrap();
        live.retain(|commit| !commit.is_deleted());
        if snapshotted.contains(generation) {
            assert_eq!(policy.snapshot().unwrap().get_generation(), *generation);
        }
    }

    assert_eq!(surviving(&live), expected);
    assert!(
        policy.get_index_commit(released).is_none(),
        "a released generation must no longer be reachable"
    );
}

// -----------------------------------------------------------------------------
// ReaderCommit / list_commits over an index Lucene wrote
// -----------------------------------------------------------------------------

/// One commit as the Java fixture reports it.
#[derive(Debug, PartialEq, Eq)]
struct CommitRecord {
    generation: i64,
    segments_file_name: String,
    segment_count: i32,
    files: Vec<String>,
    user_data: Vec<(String, String)>,
}

fn java_commits(stdout: &str) -> Vec<CommitRecord> {
    repeated(stdout, "commit")
        .into_iter()
        .map(|line| {
            let parts: Vec<&str> = line.split(';').collect();
            assert_eq!(parts.len(), 5, "malformed commit line: {line}");
            CommitRecord {
                generation: parts[0].parse().expect("generation must be an integer"),
                segments_file_name: parts[1].to_string(),
                segment_count: parts[2].parse().expect("segment count must be an integer"),
                files: parts[3]
                    .split(',')
                    .filter(|f| !f.is_empty())
                    .map(str::to_string)
                    .collect(),
                user_data: parts[4]
                    .split(',')
                    .filter(|e| !e.is_empty())
                    .map(|entry| {
                        let (k, v) = entry.split_once(':').expect("entry must be key:value");
                        (k.to_string(), v.to_string())
                    })
                    .collect(),
            }
        })
        .collect()
}

#[test]
fn list_commits_matches_lucene_on_a_java_written_index() {
    let out = scratch_dir("list-commits");
    let stdout = run_java_fixture(&out, "list-commits");

    let created = generations(&stdout, "created_generations");
    let expected = java_commits(&stdout);
    assert_eq!(created, vec![1, 2, 3, 4, 5]);
    assert_eq!(expected.len(), 5, "every commit must be retained");
    assert!(
        expected.iter().any(|c| !c.user_data.is_empty()),
        "at least one commit must carry user data, or the comparison is weaker \
         than it looks"
    );

    // The index was written by Lucene 10.5.0 with its default codec.
    let _ = register_codec("Lucene104", Lucene104Codec::new());

    let index_dir = out.join("index");
    let directory: Arc<dyn Directory> = Arc::new(NIOFSDirectory::open(&index_dir).unwrap());
    let actual: Vec<CommitRecord> = list_commits(directory)
        .expect("Rucene must list the commits of a Lucene-written index")
        .into_iter()
        .map(|commit| {
            let mut files: Vec<String> = commit.get_file_names().unwrap().into_iter().collect();
            files.sort();
            let mut user_data: Vec<(String, String)> =
                commit.get_user_data().unwrap().into_iter().collect();
            user_data.sort();
            CommitRecord {
                generation: commit.get_generation(),
                segments_file_name: commit.get_segments_file_name(),
                segment_count: commit.get_segment_count(),
                files,
                user_data,
            }
        })
        .collect();

    assert_eq!(
        actual, expected,
        "Rucene's list_commits must report exactly what DirectoryReader.listCommits does"
    );
}

// -----------------------------------------------------------------------------
// TwoPhaseCommitTool
// -----------------------------------------------------------------------------

/// A [`TwoPhaseCommit`] that records every call, mirroring the Java fixture's
/// `Recorder` down to its `toString()`.
struct Recorder {
    name: &'static str,
    trace: Arc<Mutex<Vec<String>>>,
    fail_at: Option<&'static str>,
}

impl std::fmt::Debug for Recorder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // `TwoPhaseCommitError` renders the object with `Debug`, where Java uses
        // `toString()`; matching the rendering makes the messages comparable.
        write!(f, "Recorder({})", self.name)
    }
}

impl Recorder {
    fn new(
        name: &'static str,
        trace: &Arc<Mutex<Vec<String>>>,
        fail_at: Option<&'static str>,
    ) -> Self {
        Self {
            name,
            trace: Arc::clone(trace),
            fail_at,
        }
    }

    fn record(&self, step: &str) {
        self.trace
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(format!("{}:{step}", self.name));
    }
}

impl TwoPhaseCommit for Recorder {
    fn prepare_commit(&self) -> Result<i64> {
        self.record("prepare");
        if self.fail_at == Some("prepare") {
            return Err(LuceneError::Io(std::io::Error::other("prepare exploded")));
        }
        Ok(1)
    }

    fn commit(&self) -> Result<i64> {
        self.record("commit");
        if self.fail_at == Some("commit") {
            return Err(LuceneError::Io(std::io::Error::other("commit exploded")));
        }
        Ok(2)
    }

    fn rollback(&self) -> Result<()> {
        self.record("rollback");
        Ok(())
    }
}

#[test]
fn two_phase_commit_execute_matches_lucene() {
    let out = scratch_dir("two-phase-commit");
    let stdout = run_java_fixture(&out, "two-phase-commit");

    // --- All objects succeed; the `null` element is skipped. -----------------
    let trace = Arc::new(Mutex::new(Vec::new()));
    let a = Recorder::new("a", &trace, None);
    let c = Recorder::new("c", &trace, None);
    execute_two_phase_commit(&[Some(&a as &dyn TwoPhaseCommit), None, Some(&c)]).unwrap();
    assert_eq!(
        trace.lock().unwrap().join(","),
        field(&stdout, "success_trace")
    );

    // --- The second object fails to prepare. ---------------------------------
    let trace = Arc::new(Mutex::new(Vec::new()));
    let a = Recorder::new("a", &trace, None);
    let b = Recorder::new("b", &trace, Some("prepare"));
    let c = Recorder::new("c", &trace, None);
    let err = execute_two_phase_commit(&[Some(&a as &dyn TwoPhaseCommit), Some(&b), Some(&c)])
        .expect_err("a failed prepare must be reported");
    assert!(
        matches!(err, TwoPhaseCommitError::PrepareCommitFail { .. }),
        "{err:?}"
    );
    assert_eq!(
        field(&stdout, "prepare_fail_type"),
        "PrepareCommitFailException"
    );
    assert_eq!(err.to_string(), field(&stdout, "prepare_fail_message"));
    assert_eq!(
        trace.lock().unwrap().join(","),
        field(&stdout, "prepare_fail_trace"),
        "every object must be rolled back, including the ones never prepared"
    );

    // --- The second object fails to commit. ----------------------------------
    let trace = Arc::new(Mutex::new(Vec::new()));
    let a = Recorder::new("a", &trace, None);
    let b = Recorder::new("b", &trace, Some("commit"));
    let c = Recorder::new("c", &trace, None);
    let err = execute_two_phase_commit(&[Some(&a as &dyn TwoPhaseCommit), Some(&b), Some(&c)])
        .expect_err("a failed commit must be reported");
    assert!(
        matches!(err, TwoPhaseCommitError::CommitFail { .. }),
        "{err:?}"
    );
    assert_eq!(field(&stdout, "commit_fail_type"), "CommitFailException");
    assert_eq!(err.to_string(), field(&stdout, "commit_fail_message"));
    assert_eq!(
        trace.lock().unwrap().join(","),
        field(&stdout, "commit_fail_trace"),
        "an object that already committed must still be rolled back"
    );
}

// -----------------------------------------------------------------------------
// Deliberate divergence: a `snapshots_N` that fails to close
// -----------------------------------------------------------------------------

/// An [`IndexOutput`] whose `close()` fails, as a disk that fills up on the
/// final flush would. Mirrors the fixture's `FailingCloseOutput`.
struct FailingCloseOutput {
    inner: Box<dyn IndexOutput>,
}

impl DataOutput for FailingCloseOutput {
    fn write_byte(&mut self, b: u8) -> Result<()> {
        self.inner.write_byte(b)
    }

    fn write_bytes(&mut self, b: &[u8], offset: usize, length: usize) -> Result<()> {
        self.inner.write_bytes(b, offset, length)
    }
}

impl IndexOutput for FailingCloseOutput {
    fn close(&mut self) -> Result<()> {
        self.inner.close()?;
        Err(LuceneError::Io(std::io::Error::other(
            "no space left on device",
        )))
    }

    fn file_pointer(&self) -> i64 {
        self.inner.file_pointer()
    }

    fn checksum(&self) -> Result<i64> {
        self.inner.checksum()
    }

    fn resource_description(&self) -> &str {
        self.inner.resource_description()
    }

    fn name(&self) -> &str {
        self.inner.name()
    }
}

/// A [`Directory`] whose outputs can be made to fail on `close()`. Mirrors the
/// fixture's `FailingCloseDirectory`.
struct FailingCloseDirectory {
    inner: NIOFSDirectory,
    failing: AtomicBool,
}

impl Directory for FailingCloseDirectory {
    fn list_all(&self) -> Result<Vec<String>> {
        self.inner.list_all()
    }

    fn delete_file(&self, name: &str) -> Result<()> {
        self.inner.delete_file(name)
    }

    fn file_length(&self, name: &str) -> Result<i64> {
        self.inner.file_length(name)
    }

    fn create_output(&self, name: &str, context: &dyn IOContext) -> Result<Box<dyn IndexOutput>> {
        let out = self.inner.create_output(name, context)?;
        if self.failing.load(AtomicOrdering::SeqCst) {
            Ok(Box::new(FailingCloseOutput { inner: out }))
        } else {
            Ok(out)
        }
    }

    fn create_temp_output(
        &self,
        prefix: &str,
        suffix: &str,
        context: &dyn IOContext,
    ) -> Result<Box<dyn IndexOutput>> {
        self.inner.create_temp_output(prefix, suffix, context)
    }

    fn sync(&self, names: &[String]) -> Result<()> {
        self.inner.sync(names)
    }

    fn sync_metadata(&self) -> Result<()> {
        self.inner.sync_metadata()
    }

    fn rename(&self, source: &str, dest: &str) -> Result<()> {
        self.inner.rename(source, dest)
    }

    fn open_input(&self, name: &str, context: &dyn IOContext) -> Result<Box<dyn IndexInput>> {
        self.inner.open_input(name, context)
    }

    fn obtain_lock(&self, name: &str) -> Result<Box<dyn Lock>> {
        self.inner.obtain_lock(name)
    }

    fn close(&mut self) -> Result<()> {
        self.inner.close()
    }

    fn get_pending_deletions(&self) -> Result<HashSet<String>> {
        self.inner.get_pending_deletions()
    }
}

/// Locks in the one deliberate behavioural divergence of this port.
///
/// Both implementations create `snapshots_N` with `CREATE_NEW` semantics. Lucene
/// sets its `success` flag before the try-with-resources closes the output
/// (`PersistentSnapshotDeletionPolicy.java:174-186`), so a failing `close()`
/// leaves the file behind; the next `snapshot()` then fails a second time, with
/// `FileAlreadyExistsException`, and only the attempt after that succeeds.
/// Rucene deletes the unusable file immediately, so the next attempt already
/// succeeds — it never surfaces the spurious `AlreadyExists` failure.
///
/// This is error recovery only: the bytes, the file name and the generation
/// sequence of a successfully written save file are unaffected, so the format
/// stays interoperable in both directions.
#[test]
fn persist_close_failure_diverges_from_lucene_only_in_recovery() {
    let out = scratch_dir("persist-close-failure");
    let stdout = run_java_fixture(&out, "persist-close-failure");

    // --- What Lucene 10.5.0 does, measured. ----------------------------------
    assert_eq!(field(&stdout, "first_snapshot_error"), "IOException");
    assert_eq!(field(&stdout, "count_after_failure"), "0");
    assert_eq!(field(&stdout, "last_save_file_after_failure"), "null");
    assert_eq!(
        field(&stdout, "files_after_failure"),
        "snapshots_0",
        "Lucene keeps the save file it could not close"
    );
    assert_eq!(
        field(&stdout, "second_snapshot_error"),
        "FileAlreadyExistsException",
        "the leftover blocks the very next attempt"
    );
    assert_eq!(field(&stdout, "files_after_retry"), "");
    assert_eq!(
        field(&stdout, "third_snapshot_error"),
        "none",
        "the failed retry's own cleanup unblocks the generation again"
    );
    assert_eq!(field(&stdout, "count_after_third"), "1");
    assert_eq!(field(&stdout, "files_after_third"), "snapshots_0");

    // --- What Rucene does with the identical fault. --------------------------
    let snapshots_dir = scratch_dir("persist-close-failure-rucene");
    let dir = Arc::new(FailingCloseDirectory {
        inner: NIOFSDirectory::open(&snapshots_dir).unwrap(),
        failing: AtomicBool::new(false),
    });
    let policy = PersistentSnapshotDeletionPolicy::new(
        Arc::new(KeepOnlyLastCommitDeletionPolicy::new()),
        Arc::clone(&dir) as Arc<dyn Directory>,
    )
    .unwrap();

    let index: Arc<dyn Directory> = Arc::new(rucene::store::RamDirectory::default());
    let live = vec![RecordingCommit::at_generation(Arc::clone(&index), 1)];
    policy.on_init(&live).unwrap();

    dir.failing.store(true, AtomicOrdering::SeqCst);
    let err = policy
        .snapshot()
        .expect_err("the failing close must be reported, exactly as in Java");
    assert!(
        matches!(&err, LuceneError::Io(io) if io.to_string().contains("no space left")),
        "unexpected error: {err:?}"
    );
    // Same as Java up to here.
    assert_eq!(policy.get_snapshot_count(), 0);
    assert_eq!(policy.get_last_save_file(), None);
    // And here is the divergence: no leftover.
    assert!(
        snapshot_file_names(&snapshots_dir).is_empty(),
        "Rucene must not leave a save file it could not close"
    );

    dir.failing.store(false, AtomicOrdering::SeqCst);
    let pinned = policy
        .snapshot()
        .expect("Rucene's next attempt succeeds where Lucene's raises AlreadyExists");
    assert_eq!(pinned.get_generation(), 1);
    assert_eq!(policy.get_snapshot_count(), 1);
    assert_eq!(policy.get_last_save_file().as_deref(), Some("snapshots_0"));
    assert_eq!(snapshot_file_names(&snapshots_dir), vec!["snapshots_0"]);

    // The end state is the same one Lucene reaches, one failed call later.
    assert_eq!(
        field(&stdout, "files_after_third"),
        snapshot_file_names(&snapshots_dir).join(",")
    );
    assert_eq!(
        field(&stdout, "count_after_third"),
        policy.get_snapshot_count().to_string()
    );
}

/// Returns the `snapshots_*` files in `dir`, sorted.
fn snapshot_file_names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with("snapshots_"))
        .collect();
    names.sort();
    names
}
