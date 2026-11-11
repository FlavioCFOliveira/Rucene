//! `write.lock` interoperation between Rucene and Apache Lucene Core 10.5.0.
//!
//! Pins rmp #286: Lucene's `NativeFSLockFactory` takes `write.lock` with
//! `FileChannel.tryLock()` (`NativeFSLockFactory.java:112-113`), which the
//! JDK issues on Unix as a POSIX `fcntl(F_SETLK)` write lock over the whole
//! file (OpenJDK 25 `UnixFileDispatcherImpl.c`, `lock0`). Rucene took the lock
//! with `flock`, which Linux does not make exclude `fcntl` locks, so a Lucene
//! writer and a Rucene writer could both hold one index. Each side must now
//! refuse the lock while the other holds it, with Java's
//! `LockObtainFailedException("Lock held by another program: " + realPath)`
//! (`NativeFSLockFactory.java:117`).
//!
//! # Where the expected values come from
//!
//! `LockInteropFixture` from `tests/fixtures/java-codec-harness`
//! (`java -cp lucene-core-10.5.0.jar LockInteropFixture.java <mode> <dir>`)
//! holds an `IndexWriter` open (`hold`) or tries to open one (`try`).
//!
//! Compiled where Rucene takes the lock with `fcntl`: the Unix targets
//! rustix's `fcntl_lock` reaches, the predicate of `Cargo.toml`.

#![cfg(all(
    unix,
    not(any(
        target_os = "emscripten",
        target_os = "espidf",
        target_os = "fuchsia",
        target_os = "horizon",
        target_os = "redox",
        target_os = "vita",
        target_os = "wasi"
    ))
))]

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use rucene::error::{LuceneError, LuceneErrorKind};
use rucene::store::{Directory, FSDirectory};

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

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(
        "tests/fixtures/java-codec-harness/src/main/java/org/apache/lucene/rucene/codec/\
         LockInteropFixture.java",
    )
}

fn java(mode: &str, dir: &Path) -> Command {
    let mut command = Command::new("java");
    command
        .arg("-cp")
        .arg(lucene_jar())
        .arg(fixture())
        .arg(mode)
        .arg(dir);
    command
}

/// What `LockInteropFixture try <dir>` printed.
fn java_try(dir: &Path) -> String {
    let output = java("try", dir)
        .output()
        .expect("a JDK is required to run the Lucene oracle");
    assert!(
        output.status.success(),
        "LockInteropFixture try failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_string()
}

/// The message Java's `NativeFSLockFactory` and Rucene's report for a lock
/// another process holds: `"Lock held by another program: " + realPath`.
fn held_by_another_program(dir: &Path) -> String {
    format!(
        "Lock held by another program: {}",
        dir.canonicalize().unwrap().join("write.lock").display()
    )
}

/// While Rucene holds `write.lock`, Lucene's `new IndexWriter` is refused
/// with `LockObtainFailedException`; once Rucene releases it, Lucene opens
/// the writer.
#[test]
fn lucene_cannot_open_a_writer_while_rucene_holds_the_write_lock() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = FSDirectory::open(tmp.path()).unwrap();
    let lock = dir.obtain_lock("write.lock").unwrap();

    assert_eq!(
        java_try(tmp.path()),
        format!(
            "refused: LockObtainFailedException: {}",
            held_by_another_program(tmp.path())
        )
    );

    lock.close().unwrap();
    assert_eq!(java_try(tmp.path()), "opened");
}

/// While a Lucene `IndexWriter` holds `write.lock`, Rucene's
/// `obtain_lock("write.lock")` is refused with `LockObtainFailedException`;
/// once Lucene closes the writer, Rucene obtains the lock.
#[test]
fn rucene_cannot_obtain_the_write_lock_while_lucene_holds_it() {
    let tmp = tempfile::tempdir().unwrap();
    let mut child = java("hold", tmp.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("a JDK is required to run the Lucene oracle");
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    stdout.read_line(&mut line).unwrap();
    assert_eq!(line.trim(), "held", "LockInteropFixture hold did not start");

    let dir = FSDirectory::open(tmp.path()).unwrap();
    let refused = dir.obtain_lock("write.lock");

    // Let Lucene close its writer before asserting, so that a failure does not
    // leave the JVM waiting.
    let mut stdin = child.stdin.take().unwrap();
    stdin.write_all(b"\n").unwrap();
    drop(stdin);
    let mut rest = String::new();
    std::io::Read::read_to_string(&mut stdout, &mut rest).unwrap();
    let status = child.wait().unwrap();
    assert!(status.success(), "LockInteropFixture hold failed");
    assert_eq!(rest.trim(), "released");

    match refused {
        Err(LuceneError {
            kind: LuceneErrorKind::LockObtainFailed { message, .. },
            ..
        }) => {
            assert_eq!(message, held_by_another_program(tmp.path()))
        }
        other => panic!(
            "expected LockObtainFailedException while Lucene holds write.lock, got {:?}",
            other.map(|lock| lock.ensure_valid())
        ),
    }
    dir.obtain_lock("write.lock")
        .expect("Lucene released write.lock")
        .close()
        .unwrap();
}
