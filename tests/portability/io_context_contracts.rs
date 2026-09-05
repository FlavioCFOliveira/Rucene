//! Portability tests for `org.apache.lucene.store.IOContext` and the things
//! that read it, for rmp task #157.
//!
//! Every expected value here is **measured from `lucene-core-10.5.0.jar`** by
//! `org.apache.lucene.store.RuceneIOContextFixture` in the same run. None is
//! written down from a reading of the Java source, and none is derived from
//! this crate's own arithmetic: an `IOContext` asserted against itself would
//! agree with any mistake it contains.
//!
//! The fixture lives in the `org.apache.lucene.store` package because
//! `DefaultIOContext` is package-private (`DefaultIOContext.java:25`), so its
//! duplicate-hint validation cannot be provoked from outside.
//!
//! What is pinned:
//!
//! 1. The five accessors the `IOContext` interface declares — `context()`,
//!    `mergeInfo()`, `flushInfo()`, `hints()` and `withHints(..)` — on all four
//!    contexts Lucene can produce: `IOContext.merge(MergeInfo)`,
//!    `IOContext.flush(FlushInfo)`, `IOContext.DEFAULT` and
//!    `IOContext.READONCE`.
//! 2. That the merge and flush contexts **ignore** `withHints` and that the
//!    default context **replaces** its hint set rather than merging into it —
//!    two behaviours that look alike from the outside and are not.
//! 3. That `withHints` leaves the receiver untouched, `DEFAULT` and `READONCE`
//!    included, which is what "immutable and thread-safe"
//!    (`IOContext.java:29`) has to mean in practice.
//! 4. `DefaultIOContext`'s rejection of two hints of one type, and of the same
//!    hint twice — Java reaches those two rejections by different routes (the
//!    explicit grouping check, and `Set.of`), and both are an
//!    `IllegalArgumentException`.
//! 5. The `hints(Class)` filter, a default method with no caller anywhere in
//!    Lucene 10.5.0 and therefore no other way to find out whether the port
//!    got it right.
//! 6. `MMapDirectory.ADVISE_BY_CONTEXT` and `MMapDirectory.PRELOAD_HINT`, the
//!    only two places in Lucene's store layer that read a context's hints, over
//!    seven and four contexts respectively — including the case that separates
//!    the rule's first clause from its second: a merge context carrying a
//!    `RANDOM` hint, which yields `SEQUENTIAL` because the context is tested
//!    before the hints are. No Lucene type produces such a context — the merge
//!    and flush factories return an empty hint set and `DefaultIOContext` is
//!    always `DEFAULT` — so both sides implement the interface directly to
//!    build one. Without it the clause order is unobservable: a port that
//!    tested the hints first passed this suite, which is how this case came to
//!    exist.
//! 7. `Constants.DEFAULT_READADVICE`, whose value is `NORMAL` — not the
//!    `RANDOM` that `IOContext.DEFAULT`'s own javadoc claims
//!    (`IOContext.java:48`). The javadoc is stale; the code is the contract.
//!
//! # What this test does not cover
//!
//! `MergeInfo` and `FlushInfo` are Java records, so they carry a generated
//! `toString`. This port derives `Debug` and renders differently. The fields
//! are asserted against the record's own rendering, parsed; the rendering
//! itself is a separate surface and is not claimed here.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Arc;

use rucene::store::{
    flush_io_context, hints_of, merge_io_context, Context, DataAccessHint, DefaultIOContext,
    FileDataHint, FileOpenHint, FileTypeHint, FlushInfo, IOContext, MergeInfo, PreloadHint,
    ReadAdvice, ReadOnceHint, DEFAULT_IO_CONTEXT, READONCE_IO_CONTEXT,
};

#[cfg(feature = "mmap")]
use rucene::store::mmap::MMapDirectory;

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

/// Runs the Java fixture and returns its `key=value` lines.
fn run_fixture() -> Measured {
    let mvn = which_mvn()
        .unwrap_or_else(|e| panic!("the IOContext portability test needs Maven and a JDK: {e}"));
    let output = Command::new(mvn)
        .arg("-q")
        .arg("compile")
        .arg("exec:java")
        .arg("-Dexec.mainClass=org.apache.lucene.store.RuceneIOContextFixture")
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

// ---------------------------------------------------------------------------
// Rendering this port's values the way the fixture renders Java's
// ---------------------------------------------------------------------------

/// Renders one hint as `<SimpleName>.<CONSTANT>`, the fixture's spelling.
///
/// Java gets this from `getClass().getSimpleName()` and the enum's `toString`.
/// Every Lucene hint is an enum, and two of them — `PreloadHint` and
/// `ReadOnceHint` — are single-constant enums that this port renders as unit
/// structs, so the constant is spelled out here rather than derived.
fn hint_name(hint: &Arc<dyn FileOpenHint>) -> String {
    let any = hint.as_any();
    if let Some(h) = any.downcast_ref::<DataAccessHint>() {
        return match h {
            DataAccessHint::Random => "DataAccessHint.RANDOM",
            DataAccessHint::Sequential => "DataAccessHint.SEQUENTIAL",
        }
        .to_string();
    }
    if let Some(h) = any.downcast_ref::<FileTypeHint>() {
        return match h {
            FileTypeHint::Index => "FileTypeHint.INDEX",
            FileTypeHint::Data => "FileTypeHint.DATA",
        }
        .to_string();
    }
    if let Some(h) = any.downcast_ref::<FileDataHint>() {
        return match h {
            FileDataHint::Postings => "FileDataHint.POSTINGS",
            FileDataHint::KnnVectors => "FileDataHint.KNN_VECTORS",
        }
        .to_string();
    }
    if any.downcast_ref::<PreloadHint>().is_some() {
        return "PreloadHint.INSTANCE".to_string();
    }
    if any.downcast_ref::<ReadOnceHint>().is_some() {
        return "ReadOnceHint.INSTANCE".to_string();
    }
    panic!("unmapped hint: {hint:?}");
}

/// Renders a context's hints the way the fixture does: sorted and comma-joined.
///
/// Sorted because `IOContext.hints()` is a `java.util.Set` whose iteration
/// order is unspecified and varies between JVM runs, so order cannot be part of
/// the contract in either direction.
fn hint_set(context: &dyn IOContext) -> String {
    let mut names: Vec<String> = context.hints().iter().map(hint_name).collect();
    names.sort();
    names.join(",")
}

/// Parses `MergeInfo[totalMaxDoc=100, estimatedMergeBytes=1024, ...]`.
fn parse_merge_info(rendered: &str) -> Option<MergeInfo> {
    let body = rendered.strip_prefix("MergeInfo[")?.strip_suffix(']')?;
    let field = |name: &str| -> String {
        body.split(", ")
            .find_map(|part| part.strip_prefix(&format!("{name}=")))
            .unwrap_or_else(|| panic!("no {name} in {rendered}"))
            .to_string()
    };
    Some(MergeInfo::new(
        field("totalMaxDoc").parse().expect("totalMaxDoc"),
        field("estimatedMergeBytes")
            .parse()
            .expect("estimatedMergeBytes"),
        field("isExternal").parse().expect("isExternal"),
        field("mergeMaxNumSegments")
            .parse()
            .expect("mergeMaxNumSegments"),
    ))
}

/// Parses `FlushInfo[numDocs=10, estimatedSegmentSize=512]`.
fn parse_flush_info(rendered: &str) -> Option<FlushInfo> {
    let body = rendered.strip_prefix("FlushInfo[")?.strip_suffix(']')?;
    let field = |name: &str| -> String {
        body.split(", ")
            .find_map(|part| part.strip_prefix(&format!("{name}=")))
            .unwrap_or_else(|| panic!("no {name} in {rendered}"))
            .to_string()
    };
    Some(FlushInfo::new(
        field("numDocs").parse().expect("numDocs"),
        field("estimatedSegmentSize")
            .parse()
            .expect("estimatedSegmentSize"),
    ))
}

fn context_name(context: Context) -> &'static str {
    match context {
        Context::Default => "DEFAULT",
        Context::Merge => "MERGE",
        Context::Flush => "FLUSH",
    }
}

fn advice_name(advice: ReadAdvice) -> &'static str {
    match advice {
        ReadAdvice::Normal => "NORMAL",
        ReadAdvice::Random => "RANDOM",
        ReadAdvice::Sequential => "SEQUENTIAL",
    }
}

/// Asserts the five interface accessors of `context` against the `prefix` rows
/// the fixture measured.
fn assert_accessors(m: &Measured, prefix: &str, context: &dyn IOContext) {
    assert_eq!(
        context_name(context.context()),
        m.one(&format!("{prefix}.context")),
        "{prefix}: context()"
    );

    let java_merge = m.one(&format!("{prefix}.merge_info"));
    assert_eq!(
        context.merge_info(),
        parse_merge_info(&java_merge),
        "{prefix}: mergeInfo(), Java said {java_merge}"
    );

    let java_flush = m.one(&format!("{prefix}.flush_info"));
    assert_eq!(
        context.flush_info(),
        parse_flush_info(&java_flush),
        "{prefix}: flushInfo(), Java said {java_flush}"
    );

    assert_eq!(
        hint_set(context),
        m.one(&format!("{prefix}.hints")),
        "{prefix}: hints()"
    );
    assert_eq!(
        context.hints().len().to_string(),
        m.one(&format!("{prefix}.hint_count")),
        "{prefix}: hints().size()"
    );
}

/// A context that is [`Context::Merge`] **and** carries hints.
///
/// Mirrors the anonymous `IOContext` the fixture declares. No Lucene type
/// produces one, and neither does this port; both sides build it by
/// implementing the interface, which is what a downstream implementor can also
/// do, since `IOContext` is public.
#[derive(Debug)]
struct CustomMergeContext {
    info: MergeInfo,
    hints: Vec<Arc<dyn FileOpenHint>>,
}

impl IOContext for CustomMergeContext {
    fn context(&self) -> Context {
        Context::Merge
    }

    fn merge_info(&self) -> Option<MergeInfo> {
        Some(self.info)
    }

    fn flush_info(&self) -> Option<FlushInfo> {
        None
    }

    fn hints(&self) -> &[Arc<dyn FileOpenHint>] {
        &self.hints
    }

    fn with_hints(&self, _hints: &[Arc<dyn FileOpenHint>]) -> Box<dyn IOContext> {
        Box::new(Self {
            info: self.info,
            hints: self.hints.clone(),
        })
    }
}

// ---------------------------------------------------------------------------
// The test
// ---------------------------------------------------------------------------

#[test]
fn io_context_matches_lucene_1050() {
    let m = run_fixture();

    let merge_info = MergeInfo::new(100, 1024, false, 1);
    let flush_info = FlushInfo::new(10, 512);

    // ---- IOContext.merge(MergeInfo), IOContext.java:64 ---------------------
    let merge = merge_io_context(merge_info);
    assert_accessors(&m, "merge", merge.as_ref());
    let mi = merge
        .merge_info()
        .expect("a merge context carries a MergeInfo");
    assert_eq!(mi.total_max_doc.to_string(), m.one("merge.total_max_doc"));
    assert_eq!(
        mi.estimated_merge_bytes.to_string(),
        m.one("merge.estimated_merge_bytes")
    );
    assert_eq!(mi.is_external.to_string(), m.one("merge.is_external"));
    assert_eq!(
        mi.merge_max_num_segments.to_string(),
        m.one("merge.merge_max_num_segments")
    );

    // ---- IOContext.flush(FlushInfo), IOContext.java:95 ---------------------
    let flush = flush_io_context(flush_info);
    assert_accessors(&m, "flush", flush.as_ref());
    let fi = flush
        .flush_info()
        .expect("a flush context carries a FlushInfo");
    assert_eq!(fi.num_docs.to_string(), m.one("flush.num_docs"));
    assert_eq!(
        fi.estimated_segment_size.to_string(),
        m.one("flush.estimated_segment_size")
    );

    // ---- withHints is ignored by the merge and flush contexts --------------
    //
    // Java returns the receiver itself (`IOContext.java:87`, `:118`), which the
    // fixture confirms with `same_instance=true`. `with_hints` takes `&self`
    // and hands back an owned `Box`, so it cannot return the receiver; it
    // returns a context that answers every accessor identically, which is the
    // declared divergence on `merge_io_context`. That the two are
    // indistinguishable through the interface is exactly what is asserted here.
    assert_eq!("true", m.one("merge_hinted.same_instance"));
    let merge_hinted = merge.with_hints(&[
        Arc::new(DataAccessHint::Random),
        Arc::new(FileTypeHint::Data),
    ]);
    assert_accessors(&m, "merge_hinted", merge_hinted.as_ref());

    assert_eq!("true", m.one("flush_hinted.same_instance"));
    let flush_hinted = flush.with_hints(&[
        Arc::new(DataAccessHint::Sequential),
        Arc::new(PreloadHint::instance()),
    ]);
    assert_accessors(&m, "flush_hinted", flush_hinted.as_ref());

    // ---- IOContext.DEFAULT and IOContext.READONCE --------------------------
    assert_accessors(&m, "default", &*DEFAULT_IO_CONTEXT);
    assert_accessors(&m, "readonce", &*READONCE_IO_CONTEXT);

    // ---- DEFAULT.withHints replaces the hint set and leaves DEFAULT alone ---
    let default_hinted = DEFAULT_IO_CONTEXT.with_hints(&[
        Arc::new(DataAccessHint::Random),
        Arc::new(FileTypeHint::Index),
        Arc::new(FileDataHint::Postings),
    ]);
    assert_accessors(&m, "default_hinted", default_hinted.as_ref());
    assert_eq!("false", m.one("default_hinted.same_instance"));
    assert_accessors(&m, "default_after", &*DEFAULT_IO_CONTEXT);

    // READONCE already carries two hints; withHints replaces them, it does not
    // merge into them, so the result carries exactly one.
    let readonce_hinted = READONCE_IO_CONTEXT.with_hints(&[Arc::new(FileTypeHint::Data)]);
    assert_accessors(&m, "readonce_hinted", readonce_hinted.as_ref());
    assert_accessors(&m, "readonce_after", &*READONCE_IO_CONTEXT);

    // ---- the hints(Class) filter, IOContext.java:136 -----------------------
    let filtered: Vec<String> = hints_of::<DataAccessHint>(default_hinted.as_ref())
        .iter()
        .map(|h| match h {
            DataAccessHint::Random => "RANDOM".to_string(),
            DataAccessHint::Sequential => "SEQUENTIAL".to_string(),
        })
        .collect();
    assert_eq!(
        filtered.join(","),
        m.one("default_hinted.filter_data_access")
    );
    assert_eq!(
        hints_of::<ReadOnceHint>(default_hinted.as_ref())
            .len()
            .to_string(),
        m.one("default_hinted.filter_read_once_count")
    );
    assert_eq!(
        hints_of::<DataAccessHint>(merge.as_ref()).len().to_string(),
        m.one("merge.filter_data_access_count")
    );

    // ---- DefaultIOContext rejects two hints of one type ---------------------
    assert_eq!("true", m.one("two_of_one_type.threw"));
    assert!(
        DefaultIOContext::new(vec![
            Arc::new(DataAccessHint::Random),
            Arc::new(DataAccessHint::Sequential),
        ])
        .is_err(),
        "two hints of one type must be rejected, as DefaultIOContext.java:30-37 rejects them"
    );
    assert_eq!("true", m.one("same_hint_twice.threw"));
    assert!(
        DefaultIOContext::new(vec![
            Arc::new(DataAccessHint::Random),
            Arc::new(DataAccessHint::Random),
        ])
        .is_err(),
        "the same hint twice must be rejected, as Set.of rejects a duplicate element"
    );
    let two_types = DefaultIOContext::new(vec![
        Arc::new(DataAccessHint::Random),
        Arc::new(ReadOnceHint::instance()),
    ])
    .expect("two hints of different types are accepted");
    assert_accessors(&m, "two_types", &two_types);

    // ---- a context that is MERGE and carries hints -------------------------
    let custom_merge_random_probe = CustomMergeContext {
        info: merge_info,
        hints: vec![Arc::new(DataAccessHint::Random)],
    };
    assert_accessors(&m, "custom_merge_random", &custom_merge_random_probe);

    // The mmap directory is the only reader of a context's hints, so the two
    // rules below are only measurable when the feature is on.
    #[cfg(feature = "mmap")]
    {
        // ---- MMapDirectory.ADVISE_BY_CONTEXT, MMapDirectory.java:157 -----------
        let advise = MMapDirectory::advise_by_context();
        let default_random = DEFAULT_IO_CONTEXT.with_hints(&[Arc::new(DataAccessHint::Random)]);
        let default_sequential =
            DEFAULT_IO_CONTEXT.with_hints(&[Arc::new(DataAccessHint::Sequential)]);
        let custom_merge_random = CustomMergeContext {
            info: merge_info,
            hints: vec![Arc::new(DataAccessHint::Random)],
        };
        let advice_cases: Vec<(&str, &dyn IOContext)> = vec![
            ("merge", merge.as_ref()),
            ("flush", flush.as_ref()),
            ("default", &*DEFAULT_IO_CONTEXT),
            ("default_random", default_random.as_ref()),
            ("default_sequential", default_sequential.as_ref()),
            ("readonce", &*READONCE_IO_CONTEXT),
            (
                "custom_merge_random",
                &custom_merge_random as &dyn IOContext,
            ),
        ];
        for (name, context) in advice_cases {
            let got = advise("_0.dat", context).expect("ADVISE_BY_CONTEXT always answers");
            assert_eq!(
                advice_name(got),
                m.one(&format!("advise_by_context.{name}")),
                "ADVISE_BY_CONTEXT for {name}"
            );
        }
        assert_eq!(
            advice_name(rucene::util::Constants::default_read_advice()),
            m.one("constants.default_read_advice")
        );

        // ---- MMapDirectory.PRELOAD_HINT, MMapDirectory.java:103 ---------------
        let preload = MMapDirectory::preload_hint();
        let default_preload = DEFAULT_IO_CONTEXT.with_hints(&[Arc::new(PreloadHint::instance())]);
        let custom_merge_preload = CustomMergeContext {
            info: merge_info,
            hints: vec![Arc::new(PreloadHint::instance())],
        };
        for (name, context) in [
            ("default", &*DEFAULT_IO_CONTEXT as &dyn IOContext),
            ("default_preload", default_preload.as_ref()),
            ("merge", merge.as_ref()),
            (
                "custom_merge_preload",
                &custom_merge_preload as &dyn IOContext,
            ),
        ] {
            assert_eq!(
                preload("_0.dat", context).to_string(),
                m.one(&format!("preload_hint.{name}")),
                "PRELOAD_HINT for {name}"
            );
        }
    }

    // ---- MergeInfo / FlushInfo equality ------------------------------------
    assert_eq!(
        (merge_info == MergeInfo::new(100, 1024, false, 1)).to_string(),
        m.one("merge_info.equals_same")
    );
    assert_eq!(
        (merge_info == MergeInfo::new(100, 1024, true, 1)).to_string(),
        m.one("merge_info.equals_other")
    );
    assert_eq!(
        (flush_info == FlushInfo::new(10, 512)).to_string(),
        m.one("flush_info.equals_same")
    );
}
