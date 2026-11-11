#!/usr/bin/env python3
"""Convert Lucene's Unicode tokenization test classes to Rust.

Lucene 10.5.0's test framework ships two generated classes under
`org.apache.lucene.tests.analysis.standard`:

* `WordBreakTestUnicode_12_1_0` (from `WordBreakTest.txt`): one
  `assertAnalyzesTo(analyzer, input, new String[] {...})` per test sequence,
  each preceded by its `//` comment;
* `EmojiTokenizationTestUnicode_12_1` (from `emoji-test.txt`): a `TESTS`
  array of (description, sequence) pairs and a loop asserting that each
  sequence is one `<EMOJI>` token.

This script reads both Java files, decodes every string literal exactly as
`javac` does (Unicode escapes first, then the escape sequences; UTF-16 that
must decode without an unpaired surrogate), and writes the same public
`test(Analyzer)` methods as Rust into
`crates/rucene-test-framework/src/analysis/standard/`.

    python3 tools/gen/standard_unicode_tests.py [JAVA_DIR] [OUT_DIR] [--check]

`JAVA_DIR` defaults to the local Lucene 10.5.0 clone's
`lucene/test-framework/src/java/org/apache/lucene/tests/analysis/standard`;
`--check` compares instead of writing and exits 1 on a difference.
"""
import hashlib
import re
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from java_source import (  # noqa: E402
    rust_string_literal,
    string_literal_body,
    unicode_preprocess,
    utf16_to_str,
)

DEFAULT_JAVA_DIR = (
    "/tmp/lucene1050/lucene/test-framework/src/java/org/apache/lucene/tests/analysis/standard"
)
DEFAULT_OUT_DIR = "crates/rucene-test-framework/src/analysis/standard"

ASF_HEADER = """/*
 * Licensed to the Apache Software Foundation (ASF) under one or more
 * contributor license agreements.  See the NOTICE file distributed with
 * this work for additional information regarding copyright ownership.
 * The ASF licenses this file to You under the Apache License, Version 2.0
 * (the "License"); you may not use this file except in compliance with
 * the License.  You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */
"""


class Lexer:
    """A cursor over Unicode-preprocessed Java source."""

    def __init__(self, src: str, pos: int = 0):
        self.src = src
        self.pos = pos

    def skip_ws(self):
        while self.pos < len(self.src) and self.src[self.pos] in " \t\r\n":
            self.pos += 1

    def peek(self, text: str) -> bool:
        self.skip_ws()
        return self.src.startswith(text, self.pos)

    def expect(self, text: str):
        self.skip_ws()
        if not self.src.startswith(text, self.pos):
            ctx = self.src[self.pos:self.pos + 60]
            raise SystemExit(f"expected {text!r} at {self.pos}: {ctx!r}")
        self.pos += len(text)

    def line_comment(self):
        """The text of a `//` comment at the cursor, or None."""
        self.skip_ws()
        if not self.src.startswith("//", self.pos):
            return None
        end = self.src.index("\n", self.pos)
        text = self.src[self.pos + 2:end]
        self.pos = end + 1
        return text

    def string_expr(self) -> str:
        """`"..." ( + "..." )*` as a decoded string."""
        units = []
        while True:
            self.skip_ws()
            m = re.compile(r'"((?:[^"\\\n]|\\.)*)"').match(self.src, self.pos)
            if not m:
                raise SystemExit(f"expected a string literal at {self.pos}")
            units.extend(string_literal_body(m.group(1)))
            self.pos = m.end()
            if self.peek("+"):
                self.expect("+")
                continue
            return utf16_to_str(units)

    def string_array_init(self) -> list:
        """`{ "a", "b" [,] }` as decoded strings."""
        self.expect("{")
        items = []
        while not self.peek("}"):
            items.append(self.string_expr())
            if self.peek(","):
                self.expect(",")
        self.expect("}")
        return items


def method_body(src: str, signature: str) -> Lexer:
    i = src.index(signature)
    lx = Lexer(src, i + len(signature))
    lx.expect("{")
    return lx


def word_break(java: str):
    """The (comments, input, expected tokens) of every assertion of `test`."""
    src = unicode_preprocess(java)
    lx = method_body(src, "public static void test(Analyzer analyzer) throws Exception")
    cases = []
    comments = []
    while True:
        c = lx.line_comment()
        if c is not None:
            comments.append(c)
            continue
        if lx.peek("}"):
            break
        lx.expect("assertAnalyzesTo(")
        lx.expect("analyzer")
        lx.expect(",")
        text = lx.string_expr()
        lx.expect(",")
        lx.expect("new String[]")
        expected = lx.string_array_init()
        lx.expect(")")
        lx.expect(";")
        cases.append((comments, text, expected))
        comments = []
    if comments:
        raise SystemExit("trailing comments in WordBreakTestUnicode_12_1_0.test")
    return cases


def emoji(java: str):
    """The (description, sequence) pairs of `TESTS`."""
    src = unicode_preprocess(java)
    i = src.index("private static final String[] TESTS =")
    lx = Lexer(src, i + len("private static final String[] TESTS ="))
    lx.expect("new String[]")
    items = lx.string_array_init()
    lx.expect(";")
    if len(items) % 2:
        raise SystemExit("TESTS has an odd number of entries")
    # The loop the Rust method reproduces, checked verbatim.
    loop = re.sub(r"\s+", " ", src[src.index("public static void test("):i])
    want = (
        'public static void test(Analyzer analyzer) throws Exception { '
        'for (int i = 0; i < TESTS.length; i += 2) { String test = TESTS[i + 1]; try { '
        'BaseTokenStreamTestCase.assertAnalyzesTo( analyzer, test, new String[] {test}, '
        'new String[] {"<EMOJI>"}); } catch (Throwable t) { '
        'throw new Exception("Failed to tokenize \\"" + TESTS[i] + "\\":", t); } } } '
    )
    if loop != want:
        raise SystemExit(f"EmojiTokenizationTestUnicode_12_1.test changed:\n{loop}")
    return [(items[k], items[k + 1]) for k in range(0, len(items), 2)]


def header(java_name: str, raw: str) -> str:
    sha = hashlib.sha256(raw.encode("utf-8")).hexdigest()
    return (
        ASF_HEADER
        + "\n"
        + f"// @generated by tools/gen/standard_unicode_tests.py from Lucene 10.5.0's\n"
        + f"// lucene/test-framework/src/java/org/apache/lucene/tests/analysis/standard/{java_name}.java\n"
        + f"// (sha256 {sha}). DO NOT EDIT: rerun the script."
    )


def rust_word_break(raw: str) -> str:
    cases = word_break(raw)
    out = [header("WordBreakTestUnicode_12_1_0", raw)]
    out.append("""
//! Port of `org.apache.lucene.tests.analysis.standard.WordBreakTestUnicode_12_1_0`.

use rucene::analysis::Analyzer;
use rucene::error::Result;

use crate::analysis::BaseTokenStreamTestCase;

/// This class was automatically generated by generateJavaUnicodeWordBreakTest.pl from:
/// <http://www.unicode.org/Public/12.1.0/ucd/auxiliary/WordBreakTest.txt>
///
/// WordBreakTest.txt indicates the points in the provided character sequences at which conforming
/// implementations must and must not break words. This class tests for expected token extraction
/// from each of the test sequences in WordBreakTest.txt, where the expected tokens are those
/// character sequences bounded by word breaks and containing at least one character from one of the
/// following character sets:
///
/// `\\p{Script = Han}` (From <http://www.unicode.org/Public/12.1.0/ucd/Scripts.txt>) `\\p{Script =
/// Hiragana}` `\\p{LineBreak = Complex_Context}` (From
/// <http://www.unicode.org/Public/12.1.0/ucd/LineBreak.txt>) `\\p{WordBreak = ALetter}` (From
/// <http://www.unicode.org/Public/12.1.0/ucd/auxiliary/WordBreakProperty.txt>) `\\p{WordBreak =
/// Hebrew_Letter}` `\\p{WordBreak = Katakana}` `\\p{WordBreak = Numeric}` `\\p{Extended_Pictographic}`
/// (From <http://www.unicode.org/Public/emoji/12.1/emoji-data.txt>)
///
/// Equivalent to `org.apache.lucene.tests.analysis.standard.WordBreakTestUnicode_12_1_0`; the
/// Rust source is generated from the Java class by `tools/gen/standard_unicode_tests.py`.
#[allow(non_camel_case_types)]
#[derive(Debug)]
pub struct WordBreakTestUnicode_12_1_0;

impl WordBreakTestUnicode_12_1_0 {
    /// Runs every test sequence of `WordBreakTest.txt` through `analyzer`.
    ///
    /// Equivalent to `WordBreakTestUnicode_12_1_0.test(Analyzer)`.
    ///
    /// # Errors
    ///
    /// What the analyzer raises.
    ///
    /// # Panics
    ///
    /// With the `AssertionError` of `assertAnalyzesTo` when a sequence is not
    /// tokenized as expected.
    #[rustfmt::skip]
    pub fn test(analyzer: &dyn Analyzer) -> Result<()> {""")
    first = True
    for comments, text, expected in cases:
        if not first:
            out.append("")
        first = False
        for c in comments:
            out.append(("        //" + c).rstrip())
        toks = ", ".join(rust_string_literal(t) for t in expected)
        out.append(
            f"        BaseTokenStreamTestCase::assert_analyzes_to(analyzer, "
            f"{rust_string_literal(text)}, &[{toks}])?;"
        )
    out.append("        Ok(())")
    out.append("    }")
    out.append("}")
    return "\n".join(out) + "\n", len(cases)


def rust_emoji(raw: str) -> str:
    pairs = emoji(raw)
    out = [header("EmojiTokenizationTestUnicode_12_1", raw)]
    out.append("""
//! Port of `org.apache.lucene.tests.analysis.standard.EmojiTokenizationTestUnicode_12_1`.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::Arc;

use randomizedtesting::jdk::exception::Throwable;
use rucene::analysis::Analyzer;
use rucene::error::{LuceneError, Result};

use crate::analysis::BaseTokenStreamTestCase;

/// This class was automatically generated by generateEmojiTokenizationTest.pl. from:
/// <http://www.unicode.org/Public/emoji/12.1/emoji-test.txt>
///
/// Equivalent to `org.apache.lucene.tests.analysis.standard.EmojiTokenizationTestUnicode_12_1`;
/// the Rust source is generated from the Java class by `tools/gen/standard_unicode_tests.py`.
#[allow(non_camel_case_types)]
#[derive(Debug)]
pub struct EmojiTokenizationTestUnicode_12_1;

impl EmojiTokenizationTestUnicode_12_1 {
    /// Asserts that `analyzer` tokenizes every sequence of `emoji-test.txt` as
    /// one `<EMOJI>` token.
    ///
    /// Equivalent to `EmojiTokenizationTestUnicode_12_1.test(Analyzer)`: a
    /// failure is rethrown as `new Exception("Failed to tokenize \\"" +
    /// TESTS[i] + "\\":", t)`, an error of the analyzer as a
    /// `java.lang.Exception` error whose cause is that error, a failed
    /// assertion as a `java.lang.Exception` throwable whose cause is the
    /// assertion.
    ///
    /// # Errors
    ///
    /// The `java.lang.Exception` wrapping what the analyzer raises.
    ///
    /// # Panics
    ///
    /// With the `java.lang.Exception` wrapping the `AssertionError` of
    /// `assertAnalyzesTo` when a sequence is not one `<EMOJI>` token.
    pub fn test(analyzer: &dyn Analyzer) -> Result<()> {
        for pair in TESTS.chunks(2) {
            let test = pair[1];
            let message = format!("Failed to tokenize \\"{}\\":", pair[0]);
            match catch_unwind(AssertUnwindSafe(|| {
                BaseTokenStreamTestCase::assert_analyzes_to_types_only(
                    analyzer,
                    test,
                    &[test],
                    &["<EMOJI>"],
                )
            })) {
                Ok(Ok(())) => {}
                Ok(Err(t)) => {
                    return Err(LuceneError::Unchecked {
                        class: "java.lang.Exception",
                        message: Some(message),
                        cause: Some(Arc::new(t)),
                    })
                }
                Err(payload) => Throwable::with_cause(
                    "java.lang.Exception",
                    Some(message),
                    Throwable::from_panic(payload),
                )
                .throw(),
            }
        }
        Ok(())
    }
}
""")
    out.append("/// `TESTS`: each line of `emoji-test.txt`, then its code point sequence.")
    out.append("#[rustfmt::skip]")
    out.append(f"static TESTS: [&str; {2 * len(pairs)}] = [")
    for desc, seq in pairs:
        out.append(f"    {rust_string_literal(desc)},")
        out.append(f"    {rust_string_literal(seq)},")
    out.append("];")
    return "\n".join(out) + "\n", len(pairs)


def main(argv: list) -> int:
    check = "--check" in argv
    args = [a for a in argv if a != "--check"]
    java_dir = Path(args[0] if len(args) > 0 else DEFAULT_JAVA_DIR)
    out_dir = Path(args[1] if len(args) > 1 else DEFAULT_OUT_DIR)
    jobs = [
        ("WordBreakTestUnicode_12_1_0.java", "word_break_test_unicode_12_1_0.rs", rust_word_break),
        ("EmojiTokenizationTestUnicode_12_1.java", "emoji_tokenization_test_unicode_12_1.rs", rust_emoji),
    ]
    status = 0
    for java_name, rust_name, convert in jobs:
        raw = (java_dir / java_name).read_text(encoding="utf-8")
        text, count = convert(raw)
        target = out_dir / rust_name
        if check:
            if not target.exists() or target.read_text(encoding="utf-8") != text:
                print(f"{target} differs from what {java_name} generates", file=sys.stderr)
                status = 1
            else:
                print(f"{target} is current ({count} cases)")
        else:
            out_dir.mkdir(parents=True, exist_ok=True)
            target.write_text(text, encoding="utf-8")
            print(f"wrote {target} ({count} cases)")
    return status


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
