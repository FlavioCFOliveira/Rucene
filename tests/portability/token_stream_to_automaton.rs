//! Portability tests for `TokenStreamToAutomaton` against Apache Lucene Core
//! 10.5.0.
//!
//! The Java harness (`TokenStreamToAutomatonFixture`) converts scripted token
//! graphs — linear runs, synonyms with position lengths, holes, a token
//! spanning a hole, trailing holes, a final offset gap, Unicode arcs, an empty
//! term, and a sixty-token stream that drives the `RollingBuffer` past its
//! initial eight slots while positions are freed behind the cursor — and prints
//! the automaton: state count, accept states and every transition in sorted
//! order. Rucene must build exactly the same automaton
//! (`TokenStreamToAutomaton.java:113-298`).
//!
//! A missing Maven or JDK is a hard failure.

use std::path::PathBuf;
use std::process::{Command, Stdio};

use rucene::analysis::automaton_conversion::TokenStreamToAutomaton;
use rucene::analysis::tokenattributes::{
    CharTermAttribute as _, OffsetAttribute as _, PackedTokenAttributeImpl,
    PositionIncrementAttribute as _, PositionLengthAttribute as _,
};
use rucene::analysis::{default_token_attribute_factory, TokenStream};
use rucene::util::AttributeSource;

#[derive(Clone)]
struct Tok {
    term: String,
    pos_inc: i32,
    pos_len: i32,
    start: i32,
    end: i32,
}

fn tok(term: &str, pos_inc: i32, pos_len: i32, start: i32, end: i32) -> Tok {
    Tok {
        term: term.to_string(),
        pos_inc,
        pos_len,
        start,
        end,
    }
}

struct Case {
    name: &'static str,
    tokens: Vec<Tok>,
    end_pos_inc: i32,
    final_offset: i32,
    preserve_position_increments: bool,
    final_offset_gap_as_hole: bool,
    unicode_arcs: bool,
}

/// Mirrors `TokenStreamToAutomatonFixture.cases()`.
fn cases() -> Vec<Case> {
    let mut long_tokens = Vec::new();
    for i in 0..60 {
        let pos_inc = if i % 7 == 6 { 2 } else { 1 };
        long_tokens.push(tok(
            &format!("t{i}"),
            pos_inc,
            if i % 5 == 0 { 2 } else { 1 },
            i * 3,
            i * 3 + 2,
        ));
        if i % 5 == 0 {
            long_tokens.push(tok(&format!("s{i}"), 0, 1, i * 3, i * 3 + 2));
        }
    }
    let case = |name, tokens, end_pos_inc, final_offset, preserve, gap, unicode| Case {
        name,
        tokens,
        end_pos_inc,
        final_offset,
        preserve_position_increments: preserve,
        final_offset_gap_as_hole: gap,
        unicode_arcs: unicode,
    };
    vec![
        case(
            "linear",
            vec![
                tok("a", 1, 1, 0, 1),
                tok("b", 1, 1, 2, 3),
                tok("c", 1, 1, 4, 5),
            ],
            0,
            5,
            true,
            false,
            false,
        ),
        case(
            "synonym",
            vec![
                tok("wifi", 1, 2, 0, 5),
                tok("wi", 0, 1, 0, 2),
                tok("fi", 1, 1, 3, 5),
                tok("network", 1, 1, 6, 13),
            ],
            0,
            13,
            true,
            false,
            false,
        ),
        case(
            "holes",
            vec![
                tok("a", 1, 1, 0, 1),
                tok("b", 3, 1, 6, 7),
                tok("c", 1, 1, 8, 9),
            ],
            0,
            9,
            true,
            false,
            false,
        ),
        case(
            "holesflattened",
            vec![
                tok("a", 1, 1, 0, 1),
                tok("b", 3, 1, 6, 7),
                tok("c", 1, 1, 8, 9),
            ],
            2,
            12,
            false,
            false,
            false,
        ),
        case(
            "trailing",
            vec![tok("a", 1, 1, 0, 1), tok("b", 1, 1, 2, 3)],
            2,
            9,
            true,
            false,
            false,
        ),
        case(
            "spanhole",
            vec![
                tok("a", 1, 3, 0, 1),
                tok("b", 3, 1, 6, 7),
                tok("c", 1, 1, 8, 9),
            ],
            0,
            9,
            true,
            false,
            false,
        ),
        case(
            "finalgap",
            vec![tok("a", 1, 1, 0, 1), tok("b", 1, 1, 2, 3)],
            0,
            7,
            true,
            true,
            false,
        ),
        case(
            "unicode",
            vec![
                tok("\u{e9}\u{20ac}", 1, 1, 0, 2),
                tok("\u{1F600}x", 1, 1, 3, 6),
            ],
            0,
            6,
            true,
            false,
            true,
        ),
        case(
            "emptyterm",
            vec![
                tok("a", 1, 1, 0, 1),
                tok("", 1, 1, 2, 2),
                tok("c", 1, 1, 3, 4),
            ],
            0,
            4,
            true,
            false,
            false,
        ),
        case("long", long_tokens, 1, 200, true, false, false),
    ]
}

/// Emits a case's tokens; mirrors `TokenStreamToAutomatonFixture.Scripted`.
struct Scripted {
    source: AttributeSource,
    tokens: Vec<Tok>,
    upto: usize,
    end_pos_inc: i32,
    final_offset: i32,
}

impl std::fmt::Debug for Scripted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Scripted")
            .field("upto", &self.upto)
            .finish_non_exhaustive()
    }
}

impl Scripted {
    fn new(case: &Case) -> Self {
        let mut source = AttributeSource::new_with_factory(default_token_attribute_factory());
        source
            .add_attribute::<PackedTokenAttributeImpl>()
            .expect("packed token attribute");
        Self {
            source,
            tokens: case.tokens.clone(),
            upto: 0,
            end_pos_inc: case.end_pos_inc,
            final_offset: case.final_offset,
        }
    }
}

impl TokenStream for Scripted {
    fn increment_token(&mut self) -> rucene::error::Result<bool> {
        if self.upto == self.tokens.len() {
            return Ok(false);
        }
        self.source.clear_attributes();
        let token = self.tokens[self.upto].clone();
        self.upto += 1;
        let mut packed = self
            .source
            .get_attribute_mut::<PackedTokenAttributeImpl>()
            .expect("packed token attribute");
        packed.append_string(&token.term);
        packed.set_position_increment(token.pos_inc);
        packed.set_position_length(token.pos_len);
        packed.set_offset(token.start, token.end);
        Ok(true)
    }

    fn reset(&mut self) -> rucene::error::Result<()> {
        self.upto = 0;
        Ok(())
    }

    fn end(&mut self) -> rucene::error::Result<()> {
        self.source.end_attributes();
        let mut packed = self
            .source
            .get_attribute_mut::<PackedTokenAttributeImpl>()
            .expect("packed token attribute");
        packed.set_position_increment(self.end_pos_inc);
        packed.set_offset(self.final_offset, self.final_offset);
        Ok(())
    }

    fn attribute_source(&self) -> &AttributeSource {
        &self.source
    }

    fn attribute_source_mut(&mut self) -> &mut AttributeSource {
        &mut self.source
    }
}

fn rucene_lines() -> Vec<String> {
    let mut out = Vec::new();
    for case in cases() {
        let mut converter = TokenStreamToAutomaton::new();
        converter
            .set_preserve_position_increments(case.preserve_position_increments)
            .set_final_offset_gap_as_hole(case.final_offset_gap_as_hole)
            .set_unicode_arcs(case.unicode_arcs);
        let mut stream = Scripted::new(&case);
        let automaton = converter.to_automaton(&mut stream).expect("to automaton");
        out.push(format!("case={}", case.name));
        out.push(format!("states={}", automaton.get_num_states()));
        let mut accept = String::from("accept=");
        for state in 0..automaton.get_num_states() {
            if automaton.is_accept(state) {
                accept.push_str(&format!("{state},"));
            }
        }
        out.push(accept);
        for per_state in automaton.get_sorted_transitions() {
            for t in per_state {
                out.push(format!("t={},{},{},{}", t.source, t.dest, t.min, t.max));
            }
        }
    }
    out
}

fn java_lines() -> Vec<String> {
    let harness =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/java-codec-harness");
    let output = Command::new("mvn")
        .arg("-q")
        .arg("compile")
        .arg("exec:java")
        .arg("-Dexec.mainClass=org.apache.lucene.rucene.codec.TokenStreamToAutomatonFixture")
        .current_dir(harness)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .unwrap_or_else(|e| panic!("TokenStreamToAutomaton portability tests need Maven: {e}"));
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(
        output.status.success() && stdout.lines().any(|l| l.trim() == "done=true"),
        "the Java fixture failed:\nstdout:\n{stdout}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    stdout
        .lines()
        .map(str::trim)
        .filter(|l| {
            l.starts_with("case=")
                || l.starts_with("states=")
                || l.starts_with("accept=")
                || l.starts_with("t=")
        })
        .map(str::to_string)
        .collect()
}

#[test]
fn token_stream_to_automaton_builds_lucenes_automaton() {
    let java = java_lines();
    let rucene = rucene_lines();
    let mut case = String::new();
    for (j, r) in java.iter().zip(&rucene) {
        if let Some(name) = j.strip_prefix("case=") {
            case = name.to_string();
        }
        assert_eq!(
            j, r,
            "case {case}: Rucene's automaton diverges from Lucene's"
        );
    }
    assert_eq!(java.len(), rucene.len(), "line counts differ");
}
