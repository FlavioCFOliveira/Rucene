//! Oracle parity of the helpers of `org.apache.lucene.tests.util` (rmp #342)
//! against Apache Lucene 10.5.0.
//!
//! The Java oracle (`org.apache.lucene.tests.util.TestUtilOracle`, in the
//! workspace harness `crates/randomizedtesting/tests/fixtures/oracle/`)
//! records:
//!
//! * `testutil_english.tsv`: `English.intToEnglish` and
//!   `English.longToEnglish` over every int in [-1000, 1000], the `int` and
//!   `long` extremes, the powers of ten around every scale word, and 500 longs
//!   of `new Random(0)` at every magnitude.
//! * `testutil_generators.tsv`: for every `TestUtil` random generator, each
//!   parameter set and each seed of `OracleSupport.seeds()`, the digest of 40
//!   consecutive results of one `new Random(seed)`, the `nextLong()` the
//!   generator leaves next, and the first result of the first seed.
//! * `testutil_patterns.tsv`: per seed, the patterns of 40 `randomPattern`
//!   calls on one `new Random(seed)`.
//! * `testutil_compare.tsv`: `STRING_CODEPOINT_COMPARATOR`,
//!   `bytesRefToString`, `floatUlpEquals` and `doubleUlpEquals` on fixed
//!   inputs.

use randomizedtesting::jdk::random::{JdkRandom, Random};
use rucene::analysis::factories::AbstractAnalysisFactory;
use rucene::util::BytesRef;
use rucene_test_framework::util::{English, TestUtil};

fn rows(name: &str) -> Vec<Vec<String>> {
    let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("read {path}: {e}"))
        .lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
        .map(|l| l.split('\t').map(str::to_string).collect())
        .collect()
}

/// `English` spells every value of the oracle's grid as Lucene does.
#[test]
fn english_is_lucenes() {
    let rows = rows("testutil_english.tsv");
    assert_eq!(rows.len(), 2003 + 57 + 2 + 500);
    for r in &rows {
        let rust = match r[0].as_str() {
            "int" => {
                let i: i32 = r[1].parse().unwrap();
                let mut into = String::new();
                English::int_to_english_into(i, &mut into);
                assert_eq!(into, English::int_to_english(i));
                into
            }
            _ => English::long_to_english(r[1].parse().unwrap()),
        };
        // The fixture's line holds the result's trailing space, if any.
        assert_eq!(rust, r[2], "{} {}", r[0], r[1]);
    }
}

/// The FNV-1a 64-bit digest `TestUtilOracle.Digest` computes.
struct Digest(u64);

impl Digest {
    fn b(&mut self, v: u8) {
        self.0 ^= u64::from(v);
        self.0 = self.0.wrapping_mul(0x100_0000_01b3);
    }
    fn i(&mut self, v: i32) {
        v.to_le_bytes().into_iter().for_each(|b| self.b(b));
    }
    fn units(&mut self, units: &[u16]) {
        self.i(units.len() as i32);
        for u in units {
            u.to_le_bytes().into_iter().for_each(|b| self.b(b));
        }
    }
    fn bytes(&mut self, bytes: &[u8]) {
        self.i(bytes.len() as i32);
        bytes.iter().for_each(|b| self.b(*b));
    }
}

/// One generator result, as the oracle digests and shows it.
enum Out {
    Units(Vec<u16>),
    Bytes(Vec<u8>),
    Term(BytesRef),
    Long(i64),
}

impl Out {
    fn str(s: String) -> Out {
        Out::Units(s.encode_utf16().collect())
    }
    fn digest(&self, d: &mut Digest) {
        match self {
            Out::Units(u) => d.units(u),
            Out::Bytes(b) => d.bytes(b),
            Out::Term(t) => {
                d.bytes(&t.bytes);
                d.i(t.offset as i32);
                d.i(t.length as i32);
            }
            Out::Long(l) => l.to_le_bytes().into_iter().for_each(|b| d.b(b)),
        }
    }
    fn show(&self) -> String {
        let hex = |b: &[u8]| {
            if b.is_empty() {
                "-".to_string()
            } else {
                b.iter().map(|x| format!("{x:02x}")).collect()
            }
        };
        match self {
            Out::Units(u) => units(u),
            Out::Bytes(b) => hex(b),
            Out::Term(t) => format!("{}/{}", hex(&t.bytes), t.length),
            Out::Long(l) => l.to_string(),
        }
    }
}

/// `OracleSupport.units`.
fn units(u: &[u16]) -> String {
    if u.is_empty() {
        return "-".to_string();
    }
    u.iter()
        .map(|c| format!("{c:04x}"))
        .collect::<Vec<_>>()
        .join(".")
}

fn from_units(s: &str) -> Vec<u16> {
    if s == "-" {
        return Vec::new();
    }
    s.split('.')
        .map(|u| u16::from_str_radix(u, 16).unwrap())
        .collect()
}

fn nums<T: std::str::FromStr>(params: &str) -> Vec<T>
where
    T::Err: std::fmt::Debug,
{
    params.split(',').map(|p| p.parse().unwrap()).collect()
}

/// Draws one result of the generator a fixture row names.
fn draw(generator: &str, params: &str, r: &JdkRandom) -> Out {
    let p = params;
    match (generator, p) {
        ("nextLong", _) => {
            let v: Vec<i64> = nums(p);
            Out::Long(TestUtil::next_long(r, v[0], v[1]))
        }
        ("nextBigInteger", _) => Out::Bytes(TestUtil::next_big_integer(r, nums(p)[0])),
        ("randomSimpleString", "-") => Out::str(TestUtil::random_simple_string(r)),
        ("randomSimpleString", _) => {
            let v: Vec<i32> = nums(p);
            if v.len() == 1 {
                Out::str(TestUtil::random_simple_string_max(r, v[0]))
            } else {
                Out::str(TestUtil::random_simple_string_between(r, v[0], v[1]))
            }
        }
        ("randomSimpleStringRange", _) => {
            let v: Vec<i32> = nums(p);
            Out::str(TestUtil::random_simple_string_range(
                r,
                v[0] as u16,
                v[1] as u16,
                v[2],
            ))
        }
        ("randomUnicodeString", "-") => Out::str(TestUtil::random_unicode_string(r)),
        ("randomUnicodeString", _) => Out::str(TestUtil::random_unicode_string_max(r, nums(p)[0])),
        ("randomFixedLengthUnicodeString", _) => {
            let v: Vec<usize> = nums(p);
            let mut chars = vec![0u16; v[0]];
            TestUtil::random_fixed_length_unicode_string(r, &mut chars, v[1], v[2]);
            Out::Units(chars)
        }
        ("randomRegexpishString", "-") => Out::str(TestUtil::random_regexpish_string(r)),
        ("randomRegexpishString", _) => {
            Out::str(TestUtil::random_regexpish_string_with(r, nums(p)[0]))
        }
        ("randomHtmlishString", _) => Out::str(TestUtil::random_htmlish_string(r, nums(p)[0])),
        ("randomlyRecaseCodePoints", _) => {
            let s = String::from_utf16(&from_units(p)).unwrap();
            Out::str(TestUtil::randomly_recase_code_points(r, &s))
        }
        ("randomRealisticUnicodeString", "-") => {
            Out::str(TestUtil::random_realistic_unicode_string(r))
        }
        ("randomRealisticUnicodeString", _) => {
            let v: Vec<i32> = nums(p);
            if v.len() == 1 {
                Out::str(TestUtil::random_realistic_unicode_string_max(r, v[0]))
            } else {
                Out::str(TestUtil::random_realistic_unicode_string_between(
                    r, v[0], v[1],
                ))
            }
        }
        ("randomFixedByteLengthUnicodeString", _) => Out::str(
            TestUtil::random_fixed_byte_length_unicode_string(r, nums(p)[0]),
        ),
        ("randomBinaryTerm", "-") => Out::Term(TestUtil::random_binary_term(r)),
        ("randomBinaryTerm", _) => Out::Term(TestUtil::random_binary_term_with(r, nums(p)[0])),
        ("randomAnalysisString", _) => {
            let (max, simple) = p.split_once(',').unwrap();
            Out::str(TestUtil::random_analysis_string(
                r,
                max.parse().unwrap(),
                simple.parse().unwrap(),
            ))
        }
        ("randomSubString", _) => {
            let (len, simple) = p.split_once(',').unwrap();
            Out::str(TestUtil::random_sub_string(
                r,
                len.parse().unwrap(),
                simple.parse().unwrap(),
            ))
        }
        ("stringToCharSequence", _) => {
            Out::str(TestUtil::string_to_char_sequence("a\u{e9}\u{10400}", r))
        }
        _ => panic!("unknown generator {generator} {params}"),
    }
}

/// Checks every row of a generator fixture: the digest of `draws(generator)`
/// results of `draw` on one `new Random(seed)`, the `nextLong()` left next,
/// and the first result (its first 200 characters) of the first seed.
fn check_generator_rows(
    fixture: &str,
    expected_rows: usize,
    draws: impl Fn(&str) -> usize,
    draw: impl Fn(&str, &str, &JdkRandom) -> Out,
) {
    let rows = rows(fixture);
    assert_eq!(rows.len(), expected_rows);
    let mut failures = Vec::new();
    for row in &rows {
        let (generator, params, seed) = (&row[0], &row[1], row[2].parse::<i64>().unwrap());
        let r = JdkRandom::new(seed);
        let mut digest = Digest(0xcbf2_9ce4_8422_2325);
        let mut first = None;
        for _ in 0..draws(generator) {
            let out = draw(generator, params, &r);
            first.get_or_insert_with(|| out.show().chars().take(200).collect::<String>());
            out.digest(&mut digest);
        }
        let got = (format!("{:016x}", digest.0), r.next_long().to_string());
        if got != (row[3].clone(), row[4].clone()) || (row[5] != "-" && first.unwrap() != row[5]) {
            failures.push(format!("{generator} {params} seed {seed}"));
        }
    }
    assert!(
        failures.is_empty(),
        "{} rows differ: {:?}",
        failures.len(),
        failures
    );
}

/// Every generator draws Java's results, and leaves the `Random` where Java
/// leaves it, for every parameter set and seed of the oracle's grid.
#[test]
fn generators_are_lucenes() {
    check_generator_rows("testutil_generators.tsv", 71 * 20, |_| 40, draw);
}

/// `STRING_CODEPOINT_COMPARATOR`, `bytesRefToString` and the ULP comparisons
/// return Java's results.
#[test]
fn comparisons_are_lucenes() {
    let rows = rows("testutil_compare.tsv");
    assert_eq!(rows.len(), 100 + 10 + 11 * 11 * 4 + 10 * 10 * 4);
    for row in &rows {
        let inputs: Vec<&str> = row[1].split(' ').collect();
        let got = match row[0].as_str() {
            "codepoint" => {
                let a = String::from_utf16(&from_units(inputs[0])).unwrap();
                let b = String::from_utf16(&from_units(inputs[1])).unwrap();
                TestUtil::string_codepoint_compare(&a, &b).to_string()
            }
            "bytesRefToString" if row[1] == "null" => TestUtil::bytes_ref_to_string(None),
            "bytesRefToString" => {
                let bytes: Vec<u8> = if row[1] == "-" {
                    Vec::new()
                } else {
                    (0..row[1].len())
                        .step_by(2)
                        .map(|i| u8::from_str_radix(&row[1][i..i + 2], 16).unwrap())
                        .collect()
                };
                let s = TestUtil::bytes_ref_to_string(Some(&BytesRef::new(bytes)));
                units(&s.encode_utf16().collect::<Vec<_>>())
            }
            "floatUlpEquals" => TestUtil::float_ulp_equals(
                f32::from_bits(u32::from_str_radix(inputs[0], 16).unwrap()),
                f32::from_bits(u32::from_str_radix(inputs[1], 16).unwrap()),
                inputs[2].parse().unwrap(),
            )
            .to_string(),
            "doubleUlpEquals" => TestUtil::double_ulp_equals(
                f64::from_bits(u64::from_str_radix(inputs[0], 16).unwrap()),
                f64::from_bits(u64::from_str_radix(inputs[1], 16).unwrap()),
                inputs[2].parse().unwrap(),
            )
            .to_string(),
            other => panic!("unknown kind {other}"),
        };
        assert_eq!(got, row[2], "{} {}", row[0], row[1]);
    }
}

/// `randomPattern` returns Java's patterns, in Java's order, for every seed of
/// the oracle's grid, up to the first pattern that core's
/// `AbstractAnalysisFactory::get_pattern` refuses for its declared engine
/// limit (a counted repetition of a body whose capturing group tries its
/// empty match first). Java accepts such a pattern and the port skips it,
/// after which the seed's two sequences no longer correspond; every such stop
/// is checked to be that refusal and counted.
#[test]
fn random_pattern_is_lucenes() {
    let rows = rows("testutil_patterns.tsv");
    assert_eq!(rows.len(), 20);
    let (mut equal, mut engine_limit_stops) = (0, 0);
    for row in &rows {
        let r = JdkRandom::new(row[0].parse().unwrap());
        for java in &row[1..] {
            let rust = TestUtil::random_pattern(&r);
            if &rust == java {
                equal += 1;
                continue;
            }
            let mut args = std::collections::HashMap::new();
            args.insert("pattern".to_string(), java.clone());
            let refusal = AbstractAnalysisFactory::get_pattern(&mut args, "pattern", "TestUtil")
                .err()
                .unwrap_or_else(|| panic!("seed {}: Java {java:?}, port {rust:?}", row[0]));
            assert!(
                format!("{refusal:?}").contains("Counted repetition"),
                "seed {}: Java {java:?} refused by core: {refusal:?}",
                row[0]
            );
            engine_limit_stops += 1;
            break;
        }
    }
    // 19 seeds equal in full, one up to its 9th pattern (rmp #342 report).
    assert_eq!((equal, engine_limit_stops), (19 * 40 + 8, 1));
}

/// The digest of a body, as `TestUtilOracle.Digest.add(String)`.
fn body_digest(s: &str) -> String {
    let mut d = Digest(0xcbf2_9ce4_8422_2325);
    d.units(&s.encode_utf16().collect::<Vec<_>>());
    format!("{:016x}", d.0)
}

/// `LineFileDocs` over the embedded `europarl.lines.txt.gz` yields Java's
/// documents in Java's order for every seed of the oracle (each starting at
/// the skip point its `.seek` draw picks), across the end of the file and
/// its rewind, with Java's fields and field types.
#[test]
fn line_file_docs_are_lucenes() {
    use rucene_test_framework::util::LineFileDocs;

    let rows = rows("testutil_linefiledocs.tsv");
    let (docs_rows, type_rows): (Vec<_>, Vec<_>) = rows.iter().partition(|r| r[0] != "fieldType");
    assert_eq!((docs_rows.len(), type_rows.len()), (8 * 30 + 30 + 19, 7));
    let mut current: Option<(String, LineFileDocs, i32)> = None;
    let mut first_doc = None;
    for row in docs_rows {
        let k: i32 = row[1].parse().unwrap();
        if current.as_ref().map(|c| &c.0) != Some(&row[0]) {
            let random = JdkRandom::new(row[0].parse().unwrap());
            current = Some((
                row[0].clone(),
                LineFileDocs::new(&random, "europarl.lines.txt.gz").unwrap(),
                0,
            ));
        }
        let (_, docs, next) = current.as_mut().unwrap();
        let mut doc = None;
        while *next <= k {
            doc = Some(docs.next_doc().unwrap());
            *next += 1;
        }
        let doc = doc.unwrap();
        let mut body = String::new();
        let mut others = Vec::new();
        let (mut title, mut date) = (String::new(), String::new());
        for f in doc.borrow().get_fields() {
            let value = match f.numeric_value() {
                Some(n) => n.to_string(),
                None => f.string_value().unwrap(),
            };
            match f.name() {
                "title" => {}
                "titleTokenized" => title = value,
                "date" => date = value,
                "body" => body = value,
                name => others.push(format!("{name}={value}")),
            }
        }
        let got = [
            title,
            date,
            body_digest(&body),
            doc.borrow().get_fields().len().to_string(),
            others.join(";"),
        ];
        assert_eq!(&got[..], &row[2..], "seed {} doc {k}", row[0]);
        if first_doc.is_none() {
            first_doc = Some(doc);
        }
    }
    let first_doc = first_doc.unwrap();
    for (f, row) in first_doc.borrow().get_fields().iter().zip(&type_rows) {
        let t = f.field_type();
        let got = format!(
            "{},{},{:?},{},{},{},{},{},{:?},{},{},{}",
            t.stored(),
            t.tokenized(),
            t.index_options(),
            t.store_term_vectors(),
            t.store_term_vector_offsets(),
            t.store_term_vector_positions(),
            t.store_term_vector_payloads(),
            t.omit_norms(),
            t.doc_values_type(),
            t.point_dimension_count(),
            t.point_index_dimension_count(),
            t.point_num_bytes()
        );
        assert_eq!((f.name(), got.as_str()), (row[1].as_str(), row[2].as_str()));
    }
}

/// `Arrays.toString(int[])`.
fn ints_to_string(v: &[i32]) -> String {
    format!(
        "[{}]",
        v.iter().map(i32::to_string).collect::<Vec<_>>().join(", ")
    )
}

/// The `operations` draw of the oracle: the slow operations of
/// `AutomatonTestUtil` on one `randomAutomaton`, and random accepted strings.
fn automaton_operations(r: &JdkRandom) -> String {
    use rucene::util::automaton::Operations;
    use rucene_test_framework::util::automaton::{AutomatonTestUtil, RandomAcceptedStrings};

    let a = AutomatonTestUtil::random_automaton(r);
    let mut sb = format!(
        "{} {}",
        a.get_num_states(),
        AutomatonTestUtil::is_deterministic_slow(&a)
    );
    let ds = AutomatonTestUtil::determinize_simple(&a);
    sb.push('\n');
    sb.push_str(&ds.to_dot());
    let d = Operations::remove_dead_states(&ds);
    let min = AutomatonTestUtil::minimize_simple(&d);
    sb.push('\n');
    sb.push_str(&min.to_dot());
    sb.push_str(&format!(
        "\n{} {} {}",
        AutomatonTestUtil::is_finite(&d),
        AutomatonTestUtil::same_language(&d, &min),
        AutomatonTestUtil::subset_of(&min, &d)
    ));
    if d.get_num_states() > 0 && AutomatonTestUtil::is_finite(&d) {
        let mut fs: Vec<String> = AutomatonTestUtil::get_finite_strings_recursive(&d, 50)
            .iter()
            .map(|s| ints_to_string(s))
            .collect();
        fs.sort();
        sb.push_str(&format!("\n[{}]", fs.join(", ")));
    }
    if d.get_num_states() > 0 {
        let ras = RandomAcceptedStrings::new(&d);
        for _ in 0..10 {
            sb.push('\n');
            sb.push_str(&ints_to_string(&ras.get_random_accepted_string(r)));
        }
    }
    sb
}

/// `AutomatonTestUtil.randomRegexp`, `randomSingleAutomaton` and
/// `randomAutomaton` return Java's values for every seed (the automata
/// compared by `toDot()`), and the slow operations and `RandomAcceptedStrings`
/// return Java's results on them.
#[test]
fn automaton_test_util_is_lucenes() {
    use rucene_test_framework::util::automaton::AutomatonTestUtil;
    check_generator_rows(
        "testutil_automaton.tsv",
        4 * 20,
        |g| match g {
            "randomRegexp" => 40,
            "operations" => 5,
            _ => 10,
        },
        |g, _, r| match g {
            "randomRegexp" => Out::str(AutomatonTestUtil::random_regexp(r)),
            "randomSingleAutomaton" => {
                Out::str(AutomatonTestUtil::random_single_automaton(r).to_dot())
            }
            "randomAutomaton" => Out::str(AutomatonTestUtil::random_automaton(r).to_dot()),
            "operations" => Out::str(automaton_operations(r)),
            other => panic!("unknown generator {other}"),
        },
    );
}

/// `FSTTester.doTest` over seeded pairs builds Java's FST (the bytes of its
/// `save`), counts Java's nodes and arcs, and draws from its `Random` exactly
/// as Java's verification does, in both input modes and for no pairs.
#[test]
fn fst_tester_is_lucenes() {
    use std::collections::BTreeSet;

    use rucene::store::{ByteBuffersDataOutput, ByteBuffersDirectory};
    use rucene::util::fst::PositiveIntOutputs;
    use rucene::util::string_helper::IntsRef;
    use rucene_test_framework::util::fst::{FSTTester, InputOutput};

    let rows = rows("testutil_fsttester.tsv");
    assert_eq!(rows.len(), 20);
    for row in &rows {
        let seed: i64 = row[0].parse().unwrap();
        let mode: i32 = row[1].parse().unwrap();
        let r = JdkRandom::new(seed);
        let num_words = if row == &rows[0] || row == &rows[10] {
            0
        } else {
            r.next_int_bound(300)
        };
        let mut terms = BTreeSet::new();
        for _ in 0..num_words {
            let s = FSTTester::<PositiveIntOutputs>::get_random_string(&r);
            terms.insert(
                FSTTester::<PositiveIntOutputs>::to_ints_ref(&s, mode)
                    .slice()
                    .to_vec(),
            );
        }
        let pairs: Vec<InputOutput<i64>> = terms
            .into_iter()
            .map(|t| InputOutput {
                input: IntsRef::new(t),
                output: i64::from(r.next_int_bound(1000)),
            })
            .collect();
        let tester_random = JdkRandom::new(r.next_long());
        let dir = ByteBuffersDirectory::new();
        let mut tester = FSTTester::new(
            &tester_random,
            &dir,
            mode,
            pairs,
            PositiveIntOutputs::get_singleton(),
        );
        let fst = tester.do_test().unwrap();
        let bytes = match fst {
            None => "-\t-".to_string(),
            Some(fst) => {
                let mut meta = ByteBuffersDataOutput::new();
                let mut data = ByteBuffersDataOutput::new();
                fst.save(&mut meta, &mut data).unwrap();
                let mut a = meta.to_array_copy().unwrap();
                a.extend(data.to_array_copy().unwrap());
                let mut d = Digest(0xcbf2_9ce4_8422_2325);
                d.bytes(&a);
                format!("{}\t{:016x}", a.len(), d.0)
            }
        };
        let got = format!(
            "{}\t{}\t{}\t{}\t{}",
            tester.pairs.len(),
            tester.node_count,
            tester.arc_count,
            bytes,
            tester_random.next_long()
        );
        assert_eq!(got, row[2..].join("\t"), "seed {seed} mode {mode}");
    }
}
