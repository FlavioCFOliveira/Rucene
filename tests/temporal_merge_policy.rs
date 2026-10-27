//! `TemporalMergePolicy` against Apache Lucene Core 10.5.0.
//!
//! `TemporalMergeFixture` (`tests/fixtures/java-codec-harness`, run as
//! `java -cp lucene-core-10.5.0.jar TemporalMergeFixture.java <index-dir>`)
//! writes, with Lucene's own `IndexWriter`, segments whose `ts` `LongPoint`
//! values are seconds, milliseconds or microseconds of dates in 2000 and 2001,
//! some in compound files, some with deletions, one without the field; then it
//! asks several configurations of the policy for merges and prints them.
//! Rucene opens the very same index and asks the same questions. The policy
//! reads each segment's date range straight from its files through the codec
//! (`TemporalMergePolicy.java:592-700`), so both sides plan from what Lucene
//! wrote — no ranges are supplied by hand.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use rucene::codecs::Lucene104Codec;
use rucene::error::Result;
use rucene::index::merge_policy::{
    MergeContext, MergePolicy, MergeSpecification, MergeTrigger, TemporalMergePolicy,
};
use rucene::index::{SegmentCommitInfo, SegmentInfos};
use rucene::store::{Directory, FSDirectory};
use rucene::util::{InfoStream, NoOutputInfoStream};

const DAY: i64 = 86_400;

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

/// Runs the fixture over `index_dir` and returns its `<case> <spec>` lines.
fn run_java(index_dir: &Path) -> HashMap<String, String> {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join(
        "tests/fixtures/java-codec-harness/src/main/java/org/apache/lucene/rucene/codec/\
         TemporalMergeFixture.java",
    );
    let output = Command::new("java")
        .arg("-cp")
        .arg(lucene_jar())
        .arg(fixture)
        .arg(index_dir)
        .output()
        .expect("a JDK is required to run the Lucene oracle");
    assert!(
        output.status.success(),
        "TemporalMergeFixture failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .filter_map(|line| line.split_once(' '))
        .map(|(case, spec)| (case.to_string(), spec.to_string()))
        .collect()
}

/// `TemporalMergeFixture.Context`: no merge running, committed deletions.
struct Context;

impl MergeContext for Context {
    fn num_deletes_to_merge(&self, info: &SegmentCommitInfo) -> Result<i32> {
        Ok(info.get_del_count_with_soft(true))
    }

    fn num_deleted_docs(&self, info: &SegmentCommitInfo) -> i32 {
        info.get_del_count_with_soft(true)
    }

    fn get_merging_segments(&self) -> HashSet<String> {
        HashSet::new()
    }

    fn get_info_stream(&self) -> Arc<dyn InfoStream> {
        Arc::new(NoOutputInfoStream)
    }
}

/// `TemporalMergeFixture.render`.
fn render(spec: Option<MergeSpecification>) -> String {
    match spec {
        None => "null".to_string(),
        Some(spec) => spec
            .merges
            .iter()
            .map(|merge| {
                merge
                    .segments
                    .iter()
                    .map(|info| info.info.name.clone())
                    .collect::<Vec<_>>()
                    .join(",")
            })
            .collect::<Vec<_>>()
            .join("|"),
    }
}

/// `TemporalMergeFixture.renderSorted`.
fn render_sorted(spec: Option<MergeSpecification>) -> String {
    match spec {
        None => "null".to_string(),
        Some(spec) => {
            let mut merges: Vec<String> = spec
                .merges
                .iter()
                .map(|merge| {
                    let mut names: Vec<String> = merge
                        .segments
                        .iter()
                        .map(|info| info.info.name.clone())
                        .collect();
                    names.sort();
                    names.join(",")
                })
                .collect();
            merges.sort();
            merges.join("|")
        }
    }
}

/// `TemporalMergeFixture.flat(long)`.
fn flat(base_time_seconds: i64) -> TemporalMergePolicy {
    let mut policy = TemporalMergePolicy::new();
    policy.set_temporal_field("ts").unwrap();
    policy.set_base_time_in_seconds(base_time_seconds).unwrap();
    policy.disable_exponential_buckets();
    policy
}

#[test]
fn rucene_chooses_the_merges_lucene_chooses_from_the_segments_lucene_wrote() {
    let _ = rucene::codecs::register_codec("Lucene104", Lucene104Codec::new());
    let tmp = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
    let index_dir = tmp.path().join("index");
    let lucene = run_java(&index_dir);

    let dir: Arc<dyn Directory> = Arc::new(FSDirectory::open(&index_dir).unwrap());
    let infos = SegmentInfos::read_latest_commit(Arc::clone(&dir)).unwrap();
    assert_eq!(infos.size(), 12, "the fixture writes twelve segments");
    let context = Context;
    let mut rucene: HashMap<String, String> = HashMap::new();

    let mut daily = flat(DAY);
    daily.set_min_threshold(2).unwrap();
    rucene.insert(
        "daily".into(),
        render(
            daily
                .find_merges(MergeTrigger::Explicit, &infos, &context)
                .unwrap(),
        ),
    );

    let mut aggressive = flat(DAY);
    aggressive.set_min_threshold(2).unwrap();
    aggressive.set_max_threshold(3).unwrap();
    aggressive.set_compaction_ratio(1.0).unwrap();
    rucene.insert(
        "aggressive".into(),
        render(
            aggressive
                .find_merges(MergeTrigger::Explicit, &infos, &context)
                .unwrap(),
        ),
    );

    let mut exponential = TemporalMergePolicy::new();
    exponential.set_temporal_field("ts").unwrap();
    rucene.insert(
        "exponential".into(),
        render(
            exponential
                .find_merges(MergeTrigger::Explicit, &infos, &context)
                .unwrap(),
        ),
    );

    let mut too_old = TemporalMergePolicy::new();
    too_old.set_temporal_field("ts").unwrap();
    too_old.set_max_age_seconds(DAY).unwrap();
    rucene.insert(
        "tooOld".into(),
        render(
            too_old
                .find_merges(MergeTrigger::Explicit, &infos, &context)
                .unwrap(),
        ),
    );

    let all: HashMap<String, bool> = infos
        .iter()
        .map(|info| (info.info.name.clone(), true))
        .collect();
    let mut forced = flat(DAY);
    forced.set_min_threshold(2).unwrap();
    rucene.insert(
        "forced".into(),
        render_sorted(
            forced
                .find_forced_merges(&infos, 1, &all, &context)
                .unwrap(),
        ),
    );

    let mut deletes = flat(7 * DAY);
    deletes.set_min_threshold(2).unwrap();
    deletes.set_max_threshold(2).unwrap();
    rucene.insert(
        "deletes".into(),
        render(
            deletes
                .find_forced_deletes_merges(&infos, &context)
                .unwrap(),
        ),
    );

    for case in [
        "daily",
        "aggressive",
        "exponential",
        "tooOld",
        "forced",
        "deletes",
    ] {
        assert_eq!(
            rucene.get(case),
            lucene.get(case),
            "case {case}: Rucene vs Lucene (all cases: rucene={rucene:?} lucene={lucene:?})"
        );
    }
    // The cases select something, so agreement is not vacuous.
    assert_ne!(lucene["daily"], "null");
    assert_ne!(lucene["deletes"], "null");
}
