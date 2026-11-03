//! Portability tests for the domain methods of the `search` package that task
//! #172 audited.
//!
//! Every assertion here is anchored on a value produced by **Apache Lucene Core
//! 10.5.0 itself**, not by a transcription of it: the fixture
//! `tests/fixtures/java-codec-harness`, class
//! `org.apache.lucene.search.RuceneSearchDomainFixture`, constructs the real
//! Lucene objects and prints what they compute. That fixture deliberately lives
//! inside Lucene's own package, because `DisjunctionScorer`,
//! `DisjunctionSumScorer` and `DisjunctionMaxScorer` are package-private and
//! could otherwise only be re-implemented, which would prove nothing.
//!
//! The methods covered, and where the port keeps them:
//!
//! | Lucene 10.5.0 | Rucene |
//! |---|---|
//! | `DisjunctionScorer.score()` and `score(DisiWrapper)` | `DisjunctionScorer::score` over each scorer's `DisjunctionScorerImpl::score_top_list` |
//! | `DisjunctionScorer.twoPhaseIterator()` | `Scorer::two_phase_iterator` of each concrete disjunction scorer |
//! | `DoubleComparator/FloatComparator/IntComparator/LongComparator.sortableBytesToLong` | the `sortable_bytes_to_long` of each comparator |
//! | `LateInteractionRescorer.combine` | `LateInteractionFallback::combine` |
//! | `BooleanQuery.equalsTo` | the second conjunct of `BooleanQuery::query_eq` |
//! | `DisjunctionMaxQuery.equalsTo` | the second conjunct of `DisjunctionMaxQuery::query_eq` |
//! | `LogOddsFusionQuery.equalsTo` | the body of `LogOddsFusionQuery::query_eq` |
//! | `PointRangeQuery.equalsTo` | the body of `PointRangeQuery::query_eq` |
//! | `Sort.getSort` | `Sort::fields` |
//! | `SortField.getType` | `SortField::field_type` |
//!
//! Two of these are regression locks for defects this task found (rule 9):
//! `LogOddsFusionQuery` compared its `float[]` fields with IEEE equality, where
//! Java's `Arrays.equals(float[], float[])` compares `Float.floatToIntBits`, so
//! the port disagreed with Lucene about `NaN` (Java: equal, port: not equal) and
//! about `+0.0` versus `-0.0` (Java: not equal, port: equal); and its hash used
//! the raw bits, where `Arrays.hashCode(float[])` canonicalises every `NaN`.

#![deny(unsafe_code)]

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, OnceLock};

use rucene::document::{IntPoint, LongPoint};
use rucene::error::Result;
use rucene::index::Term;
use rucene::search::comparators::{
    DoubleComparator, FloatComparator, IntComparator, LongComparator,
};
use rucene::search::doc_id_set_iterator::{all, DocIdSetIterator};
use rucene::search::{
    BooleanQueryBuilder, ConstantScoreScorer, DisjunctionMaxQuery, DisjunctionMaxScorer,
    DisjunctionSumScorer, DoubleValuesSourceRescorerImpl, LateInteractionFallback,
    LogOddsFusionQuery, Occur, Pruning, Query, Scorable, ScoreMode, Scorer, Sort, SortField,
    SortFieldType, TermQuery, TwoPhaseIterator,
};
use rucene::util::BytesRef;

// ---------------------------------------------------------------------------
// Java harness
// ---------------------------------------------------------------------------

static HARNESS_LOCK: Mutex<()> = Mutex::new(());
static FIXTURE: OnceLock<Vec<String>> = OnceLock::new();

fn harness_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("java-codec-harness")
}

fn which_mvn() -> std::result::Result<String, String> {
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

/// Runs the Java fixture once and returns its output lines.
///
/// A portability test proves compatibility against the reference Java
/// implementation, so it has nothing to assert without the harness: skipping
/// would report success while proving nothing.
fn fixture() -> &'static [String] {
    FIXTURE.get_or_init(|| {
        let _guard = HARNESS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mvn = match which_mvn() {
            Ok(mvn) => mvn,
            Err(reason) => {
                panic!("search-domain portability tests require Maven and a JDK: {reason}")
            }
        };
        let harness = harness_dir();
        assert!(
            harness.join("pom.xml").exists(),
            "pom.xml not found in {}",
            harness.display()
        );
        let output = Command::new(mvn)
            .arg("-q")
            .arg("compile")
            .arg("exec:java")
            .arg("-Dexec.mainClass=org.apache.lucene.search.RuceneSearchDomainFixture")
            .current_dir(&harness)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .expect("failed to spawn Maven");
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        assert!(
            output.status.success(),
            "Java fixture failed:\nstdout:\n{stdout}\nstderr:\n{stderr}"
        );
        assert!(
            stdout.lines().any(|line| line.trim() == "fixture_ok=true"),
            "the Java fixture did not finish:\n{stdout}"
        );
        stdout.lines().map(|line| line.trim().to_string()).collect()
    })
}

/// The fixture lines whose first field is `kind`, split on `|`.
fn rows(kind: &str) -> Vec<Vec<String>> {
    let rows: Vec<Vec<String>> = fixture()
        .iter()
        .filter(|line| line.starts_with(&format!("{kind}|")))
        .map(|line| line.split('|').map(|f| f.to_string()).collect())
        .collect();
    assert!(
        !rows.is_empty(),
        "the Java fixture printed no `{kind}` rows"
    );
    rows
}

fn bits_list(field: &str) -> Vec<f32> {
    field
        .split(',')
        .map(|b| f32::from_bits(b.parse::<i32>().expect("float bits") as u32))
        .collect()
}

fn bits(field: &str) -> f32 {
    f32::from_bits(field.parse::<i32>().expect("float bits") as u32)
}

// ---------------------------------------------------------------------------
// Scorer helpers
// ---------------------------------------------------------------------------

/// A sub-scorer matching only document 0, with a fixed score.
///
/// The Rust twin of the fixture's `Fixed`: a `ConstantScoreScorer` over
/// `DocIdSetIterator.all(1)`, which is exactly what the Java side builds.
fn fixed(score: f32) -> Box<dyn Scorer> {
    Box::new(ConstantScoreScorer::from_iterator(
        score,
        ScoreMode::COMPLETE,
        Box::new(all(1).expect("all(1)")),
    ))
}

/// A two-phase iterator over document 0 whose match cost is `7.0`.
///
/// The Rust twin of the fixture's anonymous `TwoPhaseIterator`.
struct AlwaysMatches {
    approximation: Box<dyn DocIdSetIterator>,
}

impl std::fmt::Debug for AlwaysMatches {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AlwaysMatches").finish()
    }
}

impl TwoPhaseIterator for AlwaysMatches {
    fn approximation(&mut self) -> &mut dyn DocIdSetIterator {
        self.approximation.as_mut()
    }

    fn approximation_ref(&self) -> &dyn DocIdSetIterator {
        self.approximation.as_ref()
    }

    fn matches(&mut self) -> Result<bool> {
        Ok(true)
    }

    fn match_cost(&self) -> f32 {
        7.0
    }
}

/// A sub-scorer matching document 0 through a two-phase iterator.
fn fixed_two_phase(score: f32) -> Box<dyn Scorer> {
    Box::new(ConstantScoreScorer::from_two_phase(
        score,
        ScoreMode::COMPLETE,
        Box::new(AlwaysMatches {
            approximation: Box::new(all(1).expect("all(1)")),
        }),
    ))
}

// ---------------------------------------------------------------------------
// DisjunctionScorer.score() / score(DisiWrapper)
// ---------------------------------------------------------------------------

/// `DisjunctionSumScorer.score(DisiWrapper)` — Lucene accumulates the
/// sub-scores in a `double` and narrows once, so the cases below include values
/// whose sum differs between `f32` and `f64` accumulation.
#[test]
fn disjunction_sum_scorer_score_matches_java() {
    for row in rows("sum") {
        let scores = bits_list(&row[1]);
        let expected_doc: i32 = row[2].parse().expect("doc");
        let expected_bits: i32 = row[3].parse().expect("score bits");

        let subs: Vec<Box<dyn Scorer>> = scores.iter().map(|s| fixed(*s)).collect();
        let mut scorer = DisjunctionSumScorer::new(subs, ScoreMode::COMPLETE, i64::MAX)
            .expect("disjunction sum scorer");
        let doc = DocIdSetIterator::next_doc(&mut scorer).expect("next doc");
        let score = Scorable::score(&mut scorer).expect("score");

        assert_eq!(doc, expected_doc, "doc for {:?}", scores);
        assert_eq!(
            score.to_bits() as i32,
            expected_bits,
            "score for {:?}: rust {} ({}), java {} ({})",
            scores,
            score,
            score.to_bits() as i32,
            f32::from_bits(expected_bits as u32),
            expected_bits
        );
    }
}

/// `DisjunctionMaxScorer.score(DisiWrapper)` — the maximum plus the tie breaker
/// times the sum of the others, with the sum kept in a `double` and the whole
/// expression narrowed once.
#[test]
fn disjunction_max_scorer_score_matches_java() {
    for row in rows("max") {
        let tie = bits(&row[1]);
        let scores = bits_list(&row[2]);
        let expected_doc: i32 = row[3].parse().expect("doc");
        let expected_bits: i32 = row[4].parse().expect("score bits");

        let subs: Vec<Box<dyn Scorer>> = scores.iter().map(|s| fixed(*s)).collect();
        let mut scorer = DisjunctionMaxScorer::new(tie, subs, ScoreMode::COMPLETE, i64::MAX)
            .expect("disjunction max scorer");
        let doc = DocIdSetIterator::next_doc(&mut scorer).expect("next doc");
        let score = Scorable::score(&mut scorer).expect("score");

        assert_eq!(doc, expected_doc, "doc for tie {tie} over {:?}", scores);
        assert_eq!(
            score.to_bits() as i32,
            expected_bits,
            "score for tie {tie} over {:?}: rust {} ({}), java {} ({})",
            scores,
            score,
            score.to_bits() as i32,
            f32::from_bits(expected_bits as u32),
            expected_bits
        );
    }
}

// ---------------------------------------------------------------------------
// DisjunctionScorer.twoPhaseIterator()
// ---------------------------------------------------------------------------

/// `DisjunctionScorer.twoPhaseIterator()` is `null` when no sub-scorer supports
/// approximation, and otherwise a view whose match cost is the cost-weighted
/// mean of the sub-scorers' match costs over *every* sub-scorer's cost.
#[test]
fn disjunction_two_phase_iterator_matches_java() {
    for row in rows("twophase") {
        let label = row[1].as_str();
        let expected_present = row[2] == "present";
        let (first, second) = match label {
            "none" => (false, false),
            "first" => (true, false),
            "both" => (true, true),
            other => panic!("unexpected two-phase case `{other}`"),
        };
        let subs: Vec<Box<dyn Scorer>> = vec![
            if first {
                fixed_two_phase(1.0)
            } else {
                fixed(1.0)
            },
            if second {
                fixed_two_phase(2.0)
            } else {
                fixed(2.0)
            },
        ];
        let mut scorer = DisjunctionSumScorer::new(subs, ScoreMode::COMPLETE, i64::MAX)
            .expect("disjunction sum scorer");

        let match_cost = scorer.two_phase_iterator().map(|tp| tp.match_cost());
        assert_eq!(
            match_cost.is_some(),
            expected_present,
            "two-phase presence for `{label}`"
        );
        if expected_present {
            let expected_bits: i32 = row[3].parse().expect("match cost bits");
            let cost = match_cost.expect("match cost");
            assert_eq!(
                cost.to_bits() as i32,
                expected_bits,
                "match cost for `{label}`: rust {cost}, java {}",
                f32::from_bits(expected_bits as u32)
            );
        }
    }
}

// ---------------------------------------------------------------------------
// {Double,Float,Int,Long}Comparator.sortableBytesToLong
// ---------------------------------------------------------------------------

/// Each numeric comparator decodes the packed point bytes with the width its
/// field uses, and widens the four-byte result with sign extension. The Java
/// side calls the real `protected sortableBytesToLong` override declared on
/// each of the four comparator classes, so the mapping from comparator to
/// decoder is measured, not assumed.
#[test]
fn comparator_sortable_bytes_to_long_matches_java() {
    for row in rows("cmp") {
        let name = row[1].as_str();
        let encoded = hex(&row[2]);
        let expected: i64 = row[3].parse().expect("decoded value");

        // The override each comparator declares, which is exactly what its
        // constructor hands to `NumericComparator`.
        let (decode, width): (fn(&[u8]) -> i64, usize) = match name {
            "Int" => (IntComparator::sortable_bytes_to_long, 4),
            "Float" => (FloatComparator::sortable_bytes_to_long, 4),
            "Long" => (LongComparator::sortable_bytes_to_long, 8),
            "Double" => (DoubleComparator::sortable_bytes_to_long, 8),
            other => panic!("unexpected comparator `{other}`"),
        };
        assert_eq!(width, encoded.len(), "byte width for {name}");
        assert_eq!(decode(&encoded), expected, "{name} decoding {}", row[2]);

        // And the shared `NumericComparator` each one builds dispatches to
        // that same override, as Java's virtual call does.
        let via_base = match name {
            "Int" => IntComparator::new(1, "f", None, false, Pruning::NONE)
                .base()
                .sortable_bytes_to_long(&encoded),
            "Float" => FloatComparator::new(1, "f", None, false, Pruning::NONE)
                .base()
                .sortable_bytes_to_long(&encoded),
            "Long" => LongComparator::new(1, "f", None, false, Pruning::NONE)
                .base()
                .sortable_bytes_to_long(&encoded),
            _ => DoubleComparator::new(1, "f", None, false, Pruning::NONE)
                .base()
                .sortable_bytes_to_long(&encoded),
        };
        assert_eq!(via_base, expected, "{name} base decoding {}", row[2]);
    }
}

fn hex(text: &str) -> Vec<u8> {
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).expect("hex byte"))
        .collect()
}

// ---------------------------------------------------------------------------
// LateInteractionRescorer.combine
// ---------------------------------------------------------------------------

/// `LateInteractionRescorer.combine` narrows the `double` similarity to a
/// `float`, and falls back either to `0f` or to the first-pass score when the
/// document has no value in the late-interaction field.
#[test]
fn late_interaction_combine_matches_java() {
    for row in rows("combine") {
        let fallback = match row[1].as_str() {
            "zero" => LateInteractionFallback::Zero,
            "fallback" => LateInteractionFallback::FirstPassScore,
            other => panic!("unexpected combine case `{other}`"),
        };
        let first_pass = bits(&row[2]);
        let present: bool = row[3].parse().expect("value present");
        let source = f64::from_bits(row[4].parse::<i64>().expect("source bits") as u64);
        let expected_bits: i32 = row[5].parse().expect("combined bits");

        let combined = fallback.combine(first_pass, present, source);
        assert_eq!(
            combined.to_bits() as i32,
            expected_bits,
            "combine({first_pass}, {present}, {source}) for {:?}: rust {combined}, java {}",
            fallback,
            f32::from_bits(expected_bits as u32)
        );
    }
}

// ---------------------------------------------------------------------------
// equalsTo
// ---------------------------------------------------------------------------

fn tq(text: &str) -> Arc<dyn Query> {
    Arc::new(TermQuery::new(Term::new(
        "f",
        BytesRef::new(text.as_bytes().to_vec()),
    )))
}

fn bq(min_should_match: i32, clauses: &[(&str, Occur)]) -> Arc<dyn Query> {
    let mut builder = BooleanQueryBuilder::default();
    builder.set_minimum_number_should_match(min_should_match);
    for (text, occur) in clauses {
        builder.add(tq(text), *occur).expect("add clause");
    }
    Arc::new(builder.build())
}

/// Asserts that the port's `query_eq`/`query_hash` agree with the verdict the
/// real Lucene `equals`/`hashCode` returned for the same pair.
fn assert_pair(label: &str, a: &Arc<dyn Query>, b: &Arc<dyn Query>) {
    let row = rows("eq")
        .into_iter()
        .find(|row| row[1] == label)
        .unwrap_or_else(|| panic!("the Java fixture printed no `eq` row for `{label}`"));
    let java_equals: bool = row[2].parse().expect("equals");
    let java_hash_equal: bool = row[3].parse().expect("hash equality");

    assert_eq!(
        a.query_eq(b.as_ref()),
        java_equals,
        "`{label}`: query_eq disagrees with Lucene's equals"
    );
    assert_eq!(
        b.query_eq(a.as_ref()),
        java_equals,
        "`{label}`: query_eq is not symmetric"
    );
    assert_eq!(
        a.query_hash() == b.query_hash(),
        java_hash_equal,
        "`{label}`: hash agreement disagrees with Lucene's hashCode"
    );
}

/// `BooleanQuery.equalsTo` — the minimum-should-match count plus the clause
/// sets, in which `MUST` and `SHOULD` are multisets (duplicates count) while
/// `FILTER` and `MUST_NOT` are sets (duplicates collapse), and no set is
/// ordered.
#[test]
fn boolean_query_equals_to_matches_java() {
    let ab = [("a", Occur::SHOULD), ("b", Occur::SHOULD)];
    let ba = [("b", Occur::SHOULD), ("a", Occur::SHOULD)];
    let aab = [
        ("a", Occur::SHOULD),
        ("a", Occur::SHOULD),
        ("b", Occur::SHOULD),
    ];
    let filter_aa = [("a", Occur::FILTER), ("a", Occur::FILTER)];
    let filter_a = [("a", Occur::FILTER)];
    let must_not_aa = [("a", Occur::MUST_NOT), ("a", Occur::MUST_NOT)];
    let must_not_a = [("a", Occur::MUST_NOT)];
    let must_aa = [("a", Occur::MUST), ("a", Occur::MUST)];
    let must_a = [("a", Occur::MUST)];

    assert_pair("bool.should_order", &bq(0, &ab), &bq(0, &ba));
    assert_pair("bool.should_duplicate", &bq(0, &ab), &bq(0, &aab));
    assert_pair("bool.min_should_match", &bq(0, &ab), &bq(1, &ab));
    assert_pair(
        "bool.filter_duplicate",
        &bq(0, &filter_aa),
        &bq(0, &filter_a),
    );
    assert_pair(
        "bool.must_not_duplicate",
        &bq(0, &must_not_aa),
        &bq(0, &must_not_a),
    );
    assert_pair("bool.must_duplicate", &bq(0, &must_aa), &bq(0, &must_a));
    assert_pair("bool.same", &bq(2, &aab), &bq(2, &aab));
    assert_pair("bool.occur_differs", &bq(0, &filter_a), &bq(0, &must_a));
}

/// `DisjunctionMaxQuery.equalsTo` — the tie breaker plus the disjuncts, which
/// Lucene holds in a `Multiset`, so the comparison is order-insensitive even
/// though the class javadoc says otherwise. The `+0.0` / `-0.0` case also locks
/// in Lucene's own `equals`/`hashCode` disagreement there: the tie breaker is
/// compared with `==` but hashed with `floatToIntBits`.
#[test]
fn disjunction_max_query_equals_to_matches_java() {
    let ab = vec![tq("a"), tq("b")];
    let ba = vec![tq("b"), tq("a")];
    let dmq = |queries: Vec<Arc<dyn Query>>, tie: f32| -> Arc<dyn Query> {
        Arc::new(DisjunctionMaxQuery::new(queries, tie).expect("disjunction max query"))
    };

    assert_pair("dismax.same", &dmq(ab.clone(), 0.3), &dmq(ab.clone(), 0.3));
    assert_pair("dismax.order", &dmq(ab.clone(), 0.3), &dmq(ba, 0.3));
    assert_pair("dismax.tie", &dmq(ab.clone(), 0.3), &dmq(ab.clone(), 0.4));
    assert_pair(
        "dismax.tie_zero_sign",
        &dmq(ab.clone(), 0.0),
        &dmq(ab, -0.0),
    );
}

/// `LogOddsFusionQuery.equalsTo` — `alpha` compared with `==`, the clauses as a
/// multiset, and the three `float[]` fields with `Arrays.equals`, which
/// compares `Float.floatToIntBits`: every `NaN` equals every other `NaN`, and
/// `+0.0` does not equal `-0.0`. Regression lock for the defect task #172
/// found, where the port used IEEE equality instead.
#[test]
fn log_odds_fusion_query_equals_to_matches_java() {
    let nan1 = f32::from_bits(0x7fc0_0000);
    let nan2 = f32::from_bits(0x7fc0_0001);
    let clauses = || vec![tq("a"), tq("b")];
    let lofq = |alpha: f32,
                weights: Option<Vec<f32>>,
                min: Option<Vec<f32>>,
                max: Option<Vec<f32>>|
     -> Arc<dyn Query> {
        Arc::new(
            LogOddsFusionQuery::new(clauses(), alpha, weights, min, max)
                .expect("log-odds fusion query"),
        )
    };

    assert_pair(
        "logodds.same",
        &lofq(0.5, Some(vec![0.25, 0.75]), None, None),
        &lofq(0.5, Some(vec![0.25, 0.75]), None, None),
    );
    assert_pair(
        "logodds.weights_differ",
        &lofq(0.5, Some(vec![0.25, 0.75]), None, None),
        &lofq(0.5, Some(vec![0.5, 0.5]), None, None),
    );
    assert_pair(
        "logodds.alpha_differs",
        &lofq(0.5, None, None, None),
        &lofq(0.25, None, None, None),
    );
    assert_pair(
        "logodds.nan_same_bits",
        &lofq(0.5, None, Some(vec![nan1, 1.0]), Some(vec![2.0, 3.0])),
        &lofq(0.5, None, Some(vec![nan1, 1.0]), Some(vec![2.0, 3.0])),
    );
    assert_pair(
        "logodds.nan_other_bits",
        &lofq(0.5, None, Some(vec![nan1, 1.0]), Some(vec![2.0, 3.0])),
        &lofq(0.5, None, Some(vec![nan2, 1.0]), Some(vec![2.0, 3.0])),
    );
    assert_pair(
        "logodds.zero_sign_weights",
        &lofq(0.5, Some(vec![0.0, 1.0]), None, None),
        &lofq(0.5, Some(vec![-0.0, 1.0]), None, None),
    );
    assert_pair(
        "logodds.zero_sign_logit",
        &lofq(0.5, None, Some(vec![0.0, 1.0]), Some(vec![2.0, 3.0])),
        &lofq(0.5, None, Some(vec![-0.0, 1.0]), Some(vec![2.0, 3.0])),
    );
    assert_pair(
        "logodds.logit_absent_vs_present",
        &lofq(0.5, None, None, None),
        &lofq(0.5, None, Some(vec![0.0, 1.0]), Some(vec![2.0, 3.0])),
    );
}

/// `PointRangeQuery.equalsTo` — the field, the dimension count, the bytes per
/// dimension and the two packed points. `equals` guards it with `sameClassAs`,
/// which separates an `IntPoint` range from a `LongPoint` range even when the
/// packed bytes coincide.
#[test]
fn point_range_query_equals_to_matches_java() {
    assert_pair(
        "point.same",
        &IntPoint::new_range_query("f", &[1, 2], &[3, 4]).expect("int range"),
        &IntPoint::new_range_query("f", &[1, 2], &[3, 4]).expect("int range"),
    );
    assert_pair(
        "point.bounds_differ",
        &IntPoint::new_range_query("f", &[1, 2], &[3, 4]).expect("int range"),
        &IntPoint::new_range_query("f", &[1, 2], &[3, 5]).expect("int range"),
    );
    assert_pair(
        "point.field_differs",
        &IntPoint::new_range_query("f", &[1], &[3]).expect("int range"),
        &IntPoint::new_range_query("g", &[1], &[3]).expect("int range"),
    );
    assert_pair(
        "point.dims_differ",
        &IntPoint::new_range_query("f", &[1], &[3]).expect("int range"),
        &IntPoint::new_range_query("f", &[1, 1], &[3, 3]).expect("int range"),
    );
    assert_pair(
        "point.class_differs",
        &IntPoint::new_range_query("f", &[1], &[3]).expect("int range"),
        &LongPoint::new_range_query("f", &[1], &[3]).expect("long range"),
    );
}

// ---------------------------------------------------------------------------
// Sort.getSort / SortField.getType
// ---------------------------------------------------------------------------

/// `Sort.getSort()` returns the criteria in the order they were supplied, and
/// `SortField.getType()` the declared type of each. The Java side prints the
/// enum constant name, which is what `SortFieldType::java_name` must produce.
#[test]
fn sort_accessors_match_java() {
    let sort = Sort::new_fields(vec![
        SortField::new_reverse(Some("a".to_string()), SortFieldType::String, true)
            .expect("sort field"),
        SortField::new(Some("b".to_string()), SortFieldType::Int).expect("sort field"),
        SortField::new(None, SortFieldType::Score).expect("sort field"),
        SortField::new(None, SortFieldType::Doc).expect("sort field"),
        SortField::new(Some("c".to_string()), SortFieldType::Long).expect("sort field"),
        SortField::new(Some("d".to_string()), SortFieldType::Float).expect("sort field"),
        SortField::new(Some("e".to_string()), SortFieldType::Double).expect("sort field"),
        SortField::new(Some("g".to_string()), SortFieldType::StringVal).expect("sort field"),
    ])
    .expect("sort");

    let sort_rows = rows("sort");
    let java_length = sort_rows
        .iter()
        .find(|row| row[1] == "length")
        .map(|row| row[2].clone())
        .expect("the Java fixture printed no `sort|length` row");
    assert_eq!(
        sort.fields().len().to_string(),
        java_length,
        "Sort::fields length"
    );

    let java = rows("sortfield");
    assert_eq!(java.len(), sort.fields().len(), "sort field count");
    for (field, row) in sort.fields().iter().zip(java.iter()) {
        let expected_field = if row[1] == "null" {
            None
        } else {
            Some(row[1].as_str())
        };
        assert_eq!(field.field(), expected_field, "SortField::field");
        assert_eq!(
            field.field_type().java_name(),
            row[2],
            "SortField::field_type"
        );
        assert_eq!(
            field.reverse().to_string(),
            row[3],
            "SortField::reverse for {}",
            row[2]
        );
    }
}
