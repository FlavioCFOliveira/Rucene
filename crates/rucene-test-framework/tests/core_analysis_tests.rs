//! Port of the `lucene/core/src/test/org/apache/lucene/analysis/` test
//! classes of Lucene 10.5.0, each run as a suite by the ported runner:
//! `TestCharFilter`, `TestReusableStringReader`, `TestWordlistLoader`,
//! `TestAbstractAnalysisFactory`, `TestAnalysisSPILoader`,
//! `TestAnalyzerWrapper`, `TestDelegatingAnalyzerWrapper`,
//! `TestCharacterUtils`, `TestCharArrayMap`, `TestCharArraySet`, and the
//! `BaseTokenStreamTestCase` subclasses `TestStopFilter`,
//! `TestCachingTokenFilter`, `TestAutomatonToTokenStream`,
//! `TestGraphTokenFilter`, `TestGraphTokenizers` and
//! `org.apache.lucene.index.TestFieldReuse`. The helper classes
//! `FakeCharFilterFactory` and `FakeTokenFilterFactory`, and the test classes
//! beyond the first three, are in `core_analysis/`.

mod core_analysis;
mod lucene_core_shims;
mod suite_seeds;

use std::sync::Arc;

use lucene_core_shims::{lucene_class, run, t};
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::analysis::wordlist_loader::{
    get_buffered_reader, get_lines, get_snowball_word_set, get_word_set,
    get_word_set_ignoring_comments,
};
use rucene::analysis::{
    CharArraySet, CharFilter, CharFilterFn, CharReader, ReusableStringReader, StringCharReader,
};
use rucene::error::Result;

// -----------------------------------------------------------------------------
// TestCharFilter
// -----------------------------------------------------------------------------

/// `CharFilter1` (`static class … extends CharFilter`): reads through its
/// input and corrects an offset by one.
fn char_filter1(r#in: Box<dyn CharReader>) -> Box<dyn CharFilter> {
    Box::new(CharFilterFn::new(r#in, |current_off| current_off + 1))
}

/// `CharFilter2` (`static class … extends CharFilter`): reads through its
/// input and corrects an offset by two.
fn char_filter2(r#in: Box<dyn CharReader>) -> Box<dyn CharFilter> {
    Box::new(CharFilterFn::new(r#in, |current_off| current_off + 2))
}

/// A `CharFilter` as the `Reader` the next filter wraps.
fn as_reader(filter: Box<dyn CharFilter>) -> Box<dyn CharReader> {
    struct Reader(Box<dyn CharFilter>);
    impl std::fmt::Debug for Reader {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            self.0.fmt(f)
        }
    }
    impl CharReader for Reader {
        fn read(&mut self, buf: &mut [u16]) -> Result<usize> {
            self.0.read(buf)
        }
        fn close(&mut self) -> Result<()> {
            self.0.close()
        }
        fn as_char_filter(&self) -> Option<&dyn CharFilter> {
            self.0.as_char_filter()
        }
    }
    Box::new(Reader(filter))
}

/// `TestCharFilter.testCharFilter1()`.
fn char_filter_test_char_filter1() -> Result<()> {
    let cs = char_filter1(Box::new(StringCharReader::new("")));
    Assert::assert_equals_msg(Some("corrected offset is invalid"), 1, cs.correct_offset(0));
    Ok(())
}

/// `TestCharFilter.testCharFilter2()`.
fn char_filter_test_char_filter2() -> Result<()> {
    let cs = char_filter2(Box::new(StringCharReader::new("")));
    Assert::assert_equals_msg(Some("corrected offset is invalid"), 2, cs.correct_offset(0));
    Ok(())
}

/// `TestCharFilter.testCharFilter12()`.
fn char_filter_test_char_filter12() -> Result<()> {
    let cs = char_filter2(as_reader(char_filter1(Box::new(StringCharReader::new("")))));
    Assert::assert_equals_msg(Some("corrected offset is invalid"), 3, cs.correct_offset(0));
    Ok(())
}

/// `TestCharFilter.testCharFilter11()`.
fn char_filter_test_char_filter11() -> Result<()> {
    let cs = char_filter1(as_reader(char_filter1(Box::new(StringCharReader::new("")))));
    Assert::assert_equals_msg(Some("corrected offset is invalid"), 2, cs.correct_offset(0));
    Ok(())
}

fn char_filter_class() -> Arc<Class> {
    lucene_class("org.apache.lucene.analysis.TestCharFilter")
        .method(t("testCharFilter1", char_filter_test_char_filter1))
        .method(t("testCharFilter2", char_filter_test_char_filter2))
        .method(t("testCharFilter12", char_filter_test_char_filter12))
        .method(t("testCharFilter11", char_filter_test_char_filter11))
        .build()
}

// -----------------------------------------------------------------------------
// TestReusableStringReader
// -----------------------------------------------------------------------------

/// `Reader.read(char[] cbuf, int off, int len)` over Rucene's `CharReader`.
///
/// **Divergence from Lucene 10.5.0.** Rucene's `CharReader::read` fills a
/// slice and answers 0 at the end of the stream; Java's `Reader` answers
/// -1 there, and 0 only for an empty request. This maps the one to the
/// other, and `read()` reads a one-unit array.
fn jread(reader: &mut dyn CharReader, cbuf: &mut [u16], off: usize, len: usize) -> Result<i32> {
    if len == 0 {
        return Ok(0);
    }
    let n = reader.read(&mut cbuf[off..off + len])?;
    Ok(if n == 0 { -1 } else { n as i32 })
}

/// `Reader.read()`.
fn jread1(reader: &mut dyn CharReader) -> Result<i32> {
    let mut c = [0u16; 1];
    let n = jread(reader, &mut c, 0, 1)?;
    Ok(if n == -1 { -1 } else { c[0] as i32 })
}

/// `TestReusableStringReader.test()`.
///
/// `Reader.read(CharBuffer)` reads into the buffer's remaining window, the
/// slice `read(char[], int, int)` reads into.
fn reusable_string_reader_test() -> Result<()> {
    let mut reader = ReusableStringReader::new();
    Assert::assert_equals(-1, jread1(&mut reader)?);
    Assert::assert_equals(-1, jread(&mut reader, &mut [0u16; 1], 0, 1)?);
    Assert::assert_equals(-1, jread(&mut reader, &mut [0u16; 2], 1, 1)?);
    // `reader.read(CharBuffer.wrap(new char[2]))`
    Assert::assert_equals(-1, jread(&mut reader, &mut [0u16; 2], 0, 2)?);

    reader.set_value("foobar");
    let mut buf = [0u16; 4];
    Assert::assert_equals(4, jread(&mut reader, &mut buf, 0, 4)?);
    Assert::assert_equals_str(None, Some("foob"), Some(&String::from_utf16_lossy(&buf)));
    Assert::assert_equals(2, jread(&mut reader, &mut buf, 0, 4)?);
    Assert::assert_equals_str(None, Some("ar"), Some(&String::from_utf16_lossy(&buf[..2])));
    Assert::assert_equals(-1, jread(&mut reader, &mut buf, 0, 4)?);
    reader.close()?;

    reader.set_value("foobar");
    Assert::assert_equals(0, jread(&mut reader, &mut buf, 1, 0)?);
    Assert::assert_equals(3, jread(&mut reader, &mut buf, 1, 3)?);
    Assert::assert_equals_str(
        None,
        Some("foo"),
        Some(&String::from_utf16_lossy(&buf[1..4])),
    );
    // `reader.read(CharBuffer.wrap(buf, 2, 2))`
    Assert::assert_equals(2, jread(&mut reader, &mut buf, 2, 2)?);
    Assert::assert_equals_str(
        None,
        Some("ba"),
        Some(&String::from_utf16_lossy(&buf[2..4])),
    );
    Assert::assert_equals(
        'r',
        char::from_u32(jread1(&mut reader)? as u32).unwrap_or('\u{ffff}'),
    );
    Assert::assert_equals(-1, jread(&mut reader, &mut buf, 0, 4)?);
    reader.close()?;

    reader.set_value("foobar");
    let mut sb = String::new();
    loop {
        let ch = jread1(&mut reader)?;
        if ch == -1 {
            break;
        }
        sb.push(char::from_u32(ch as u32).unwrap_or('\u{fffd}'));
    }
    reader.close()?;
    Assert::assert_equals_str(None, Some("foobar"), Some(&sb));
    Ok(())
}

fn reusable_string_reader_class() -> Arc<Class> {
    lucene_class("org.apache.lucene.analysis.TestReusableStringReader")
        .method(t("test", reusable_string_reader_test))
        .build()
}

// -----------------------------------------------------------------------------
// TestWordlistLoader
// -----------------------------------------------------------------------------

/// The lines `WordlistLoader.getWordSet(Reader)` reads from `s`
/// (`getBufferedReader(reader)` and its line loop).
///
/// **Divergence from Lucene 10.5.0.** Rucene's `WordlistLoader` takes the
/// lines `BufferedReader.readLine()` yields; the reader is turned into them
/// by `getBufferedReader`, so `getWordSet(new StringReader(s))` and
/// `getWordSet(new BufferedReader(new StringReader(s)))` read the same lines.
fn lines_of(s: &str) -> Result<Vec<String>> {
    get_buffered_reader(&mut StringCharReader::new(s))
}

/// `TestWordlistLoader.testWordlistLoading()`.
fn wordlist_test_wordlist_loading() -> Result<()> {
    let s = "ONE\n  two \nthree\n\n";
    let word_set1 = get_word_set(lines_of(s)?);
    check_set(&word_set1);
    // `new BufferedReader(new StringReader(s))`
    let word_set2 = get_word_set(lines_of(s)?);
    check_set(&word_set2);
    Ok(())
}

/// `TestWordlistLoader.testComments()`.
fn wordlist_test_comments() -> Result<()> {
    let s = "ONE\n  two \nthree\n#comment";
    let word_set1 = get_word_set_ignoring_comments(lines_of(s)?, "#");
    check_set(&word_set1);
    Assert::assert_false(word_set1.contains(Some("#comment")));
    Assert::assert_false(word_set1.contains(Some("comment")));
    Ok(())
}

/// `checkSet(CharArraySet)` (`private`).
fn check_set(wordset: &CharArraySet) {
    Assert::assert_equals(3usize, wordset.size());
    Assert::assert_true(wordset.contains(Some("ONE"))); // case is not modified
    Assert::assert_true(wordset.contains(Some("two"))); // surrounding whitespace is removed
    Assert::assert_true(wordset.contains(Some("three")));
    Assert::assert_false(wordset.contains(Some("four")));
}

/// `TestWordlistLoader.testSnowballListLoading()`: stopwords in snowball
/// format.
fn wordlist_test_snowball_list_loading() -> Result<()> {
    let s = concat!(
        "|comment\n",            // commented line
        " |comment\n",           // commented line with leading whitespace
        "\n",                    // blank line
        "  \t\n",                // line with only whitespace
        " |comment | comment\n", // commented line with comment
        "ONE\n",                 // stopword, in uppercase
        "   two   \n",           // stopword with leading/trailing space
        " three   four five \n", // multiple stopwords
        "six seven | comment\n"  // multiple stopwords + comment
    );
    let wordset = get_snowball_word_set(lines_of(s)?);
    Assert::assert_equals(7usize, wordset.size());
    Assert::assert_true(wordset.contains(Some("ONE")));
    Assert::assert_true(wordset.contains(Some("two")));
    Assert::assert_true(wordset.contains(Some("three")));
    Assert::assert_true(wordset.contains(Some("four")));
    Assert::assert_true(wordset.contains(Some("five")));
    Assert::assert_true(wordset.contains(Some("six")));
    Assert::assert_true(wordset.contains(Some("seven")));
    Ok(())
}

/// `TestWordlistLoader.testGetLines()`.
fn wordlist_test_get_lines() -> Result<()> {
    let s = "One \n#Comment \n \n Two \n  Three  \n";
    let charset = "UTF-8";
    let s_byte_arr = s.as_bytes().to_vec();
    let s_input_stream = std::io::Cursor::new(s_byte_arr);
    let lines = get_lines(s_input_stream, charset)?;
    Assert::assert_equals(3usize, lines.len());
    Assert::assert_equals_str(None, Some("One"), Some(&lines[0]));
    Assert::assert_equals_str(None, Some("Two"), Some(&lines[1]));
    Assert::assert_equals_str(None, Some("Three"), Some(&lines[2]));
    Ok(())
}

fn wordlist_loader_class() -> Arc<Class> {
    lucene_class("org.apache.lucene.analysis.TestWordlistLoader")
        .method(t("testWordlistLoading", wordlist_test_wordlist_loading))
        .method(t("testComments", wordlist_test_comments))
        .method(t(
            "testSnowballListLoading",
            wordlist_test_snowball_list_loading,
        ))
        .method(t("testGetLines", wordlist_test_get_lines))
        .build()
}

// -----------------------------------------------------------------------------
// The suites
// -----------------------------------------------------------------------------

#[test]
fn test_char_filter() {
    run(char_filter_class);
}

#[test]
fn test_reusable_string_reader() {
    run(reusable_string_reader_class);
}

#[test]
fn test_wordlist_loader() {
    run(wordlist_loader_class);
}

#[test]
fn test_abstract_analysis_factory() {
    run(core_analysis::test_abstract_analysis_factory::class);
}

#[test]
fn test_analysis_spi_loader() {
    core_analysis::fake_factories::install_core_test_services();
    run(core_analysis::test_analysis_spi_loader::class);
}

#[test]
fn test_analyzer_wrapper() {
    run(core_analysis::test_analyzer_wrapper::class);
}

#[test]
fn test_delegating_analyzer_wrapper() {
    run(core_analysis::test_delegating_analyzer_wrapper::class);
}

#[test]
fn test_character_utils() {
    run(core_analysis::test_character_utils::class);
}

#[test]
fn test_char_array_map() {
    run(core_analysis::test_char_array_map::class);
}

#[test]
fn test_char_array_set() {
    run(core_analysis::test_char_array_set::class);
}

#[test]
fn test_stop_filter() {
    run(core_analysis::test_stop_filter::class);
}

#[test]
fn test_caching_token_filter() {
    run(core_analysis::test_caching_token_filter::class);
}

#[test]
fn test_automaton_to_token_stream() {
    run(core_analysis::test_automaton_to_token_stream::class);
}

#[test]
fn test_graph_token_filter() {
    run(core_analysis::test_graph_token_filter::class);
}

#[test]
fn test_graph_tokenizers() {
    run(core_analysis::test_graph_tokenizers::class);
}

#[test]
fn test_field_reuse() {
    run(core_analysis::test_field_reuse::class);
}
