//! Codec-level portability tests against Apache Lucene Core 10.5.0.
//!
//! This module drives the Java reference harness in
//! `tests/fixtures/java-codec-harness` to produce deterministic indexes with
//! the default `Lucene104Codec`. The tests verify that the harness runs
//! successfully and that the resulting directory tree contains the expected
//! index files.
//!
//! Once Rucene's own `IndexWriter` and `Lucene104Codec` implementations are
//! complete, the same harness can be used to generate both sides of a
//! byte-for-byte directory comparison.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Path to the Java codec harness, relative to `CARGO_MANIFEST_DIR`.
fn harness_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/java-codec-harness")
}

/// Runs the Java harness for `shape`, writing the index into `out_dir`.
///
/// Returns the captured stdout so callers can assert on emitted metadata.
fn run_java_harness(out_dir: &Path, shape: &str) -> Result<String, String> {
    let harness = harness_dir();
    if !harness.join("pom.xml").exists() {
        return Err(format!("pom.xml not found in {}", harness.display()));
    }

    let mvn = which_mvn()?;
    let args = format!("{} {}", out_dir.display(), shape);
    let output = Command::new(mvn)
        .arg("-q")
        .arg("compile")
        .arg("exec:java")
        .arg("-Dexec.mainClass=org.apache.lucene.rucene.codec.CodecIndexWriter")
        .arg(format!("-Dexec.args={}", args))
        .current_dir(&harness)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| format!("failed to spawn Maven: {}", e))?;

    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();

    if !output.status.success() {
        return Err(format!(
            "Maven harness failed with status {:?}\nstdout:\n{}\nstderr:\n{}",
            output.status.code(),
            stdout,
            stderr
        ));
    }

    // Forward non-empty stderr so it appears in test output for debugging.
    if !stderr.is_empty() {
        eprintln!("java-codec-harness stderr:\n{}", stderr);
    }

    Ok(stdout)
}

/// Locates the `mvn` executable.
fn which_mvn() -> Result<PathBuf, String> {
    let candidates = ["mvn", "mvn.cmd"];
    for name in candidates {
        if let Ok(output) = Command::new("which").arg(name).output() {
            if output.status.success() {
                let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !path.is_empty() {
                    return Ok(PathBuf::from(path));
                }
            }
        }
    }
    Err("mvn not found on PATH".to_string())
}

/// Collects the names of regular files in `dir`, excluding transient lock files.
fn list_index_files(dir: &Path) -> Result<HashSet<String>, String> {
    let mut names = HashSet::new();
    for entry in std::fs::read_dir(dir)
        .map_err(|e| format!("failed to read output dir {}: {}", dir.display(), e))?
    {
        let entry = entry.map_err(|e| format!("failed to read dir entry: {}", e))?;
        let meta = entry.metadata().map_err(|e| {
            format!(
                "failed to read metadata for {}: {}",
                entry.path().display(),
                e
            )
        })?;
        if meta.is_file() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name != "write.lock" {
                names.insert(name);
            }
        }
    }
    Ok(names)
}

/// Asserts that `stdout` from the harness contains the expected metadata lines.
fn assert_harness_metadata(stdout: &str, shape: &str) {
    assert!(
        stdout.contains(&format!("shape={}", shape)),
        "harness stdout should report shape={}\ngot:\n{}",
        shape,
        stdout
    );
    assert!(
        stdout.contains("version=10.5.0"),
        "harness stdout should report Lucene version 10.5.0\ngot:\n{}",
        stdout
    );
    assert!(
        stdout.contains("codec=Lucene104Codec"),
        "harness stdout should report codec=Lucene104Codec\ngot:\n{}",
        stdout
    );
    assert!(
        stdout.contains("output_dir="),
        "harness stdout should report output_dir\ngot:\n{}",
        stdout
    );
}

#[test]
fn java_harness_writes_text_index() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let out_dir = tmp.path();

    let stdout = run_java_harness(out_dir, "text").expect("harness should succeed for shape=text");
    assert_harness_metadata(&stdout, "text");

    let files = list_index_files(out_dir).expect("should list output files");
    assert!(
        !files.is_empty(),
        "text shape should produce at least one index file"
    );

    // A freshly created single-segment index with compound files enabled
    // contains a segment info file, a compound data file, its entry file, and
    // the segments file. We assert the key structural files are present.
    assert!(
        files.iter().any(|n| n.ends_with(".si")),
        "segment info (.si) file should exist; got: {:?}",
        files
    );
    assert!(
        files.iter().any(|n| n.starts_with("segments_")),
        "segments_N file should exist; got: {:?}",
        files
    );
    assert!(
        files.iter().any(|n| n.ends_with(".cfs")),
        "compound data (.cfs) file should exist; got: {:?}",
        files
    );

    // Every produced file must be non-empty.
    for name in &files {
        let path = out_dir.join(name);
        let len = std::fs::metadata(&path)
            .unwrap_or_else(|e| panic!("metadata for {}: {}", path.display(), e))
            .len();
        assert!(len > 0, "{} should be non-empty", path.display());
    }
}

#[test]
fn java_harness_supports_all_document_shapes() {
    let shapes = [
        "text",
        "docvalues",
        "points",
        "vectors",
        "stored",
        "termvectors",
    ];

    for shape in shapes {
        let tmp = tempfile::tempdir().expect("temp dir");
        let out_dir = tmp.path();

        let stdout =
            run_java_harness(out_dir, shape).unwrap_or_else(|e| panic!("shape={}: {}", shape, e));
        assert_harness_metadata(&stdout, shape);

        let files = list_index_files(out_dir).unwrap_or_else(|e| panic!("shape={}: {}", shape, e));
        assert!(
            !files.is_empty(),
            "shape={} should produce index files",
            shape
        );
        assert!(
            files.iter().any(|n| n.starts_with("segments_")),
            "shape={} should produce a segments_N file; got: {:?}",
            shape,
            files
        );
    }
}

/// Placeholder for the future byte-for-byte comparison.
///
/// When Rucene's `IndexWriter` with `Lucene104Codec` can write the same
/// document shape, this helper will compare the two directory trees file by
/// file. It is currently kept as infrastructure and is intentionally left
/// unused (the corresponding Rust side does not exist yet).
#[allow(dead_code)]
fn assert_directories_equal(left: &Path, right: &Path) -> Result<(), String> {
    let left_files = list_index_files(left)?;
    let right_files = list_index_files(right)?;
    if left_files != right_files {
        return Err(format!(
            "directory file sets differ\nleft:  {:?}\nright: {:?}",
            left_files, right_files
        ));
    }

    for name in &left_files {
        let left_path = left.join(name);
        let right_path = right.join(name);
        let left_bytes = std::fs::read(&left_path)
            .map_err(|e| format!("failed to read {}: {}", left_path.display(), e))?;
        let right_bytes = std::fs::read(&right_path)
            .map_err(|e| format!("failed to read {}: {}", right_path.display(), e))?;
        if left_bytes != right_bytes {
            return Err(format!(
                "{} differs: {} bytes vs {} bytes",
                name,
                left_bytes.len(),
                right_bytes.len()
            ));
        }
    }

    Ok(())
}
