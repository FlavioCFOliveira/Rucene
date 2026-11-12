//! Word-list loader ported from `org.apache.lucene.analysis.WordlistLoader`.
//!
//! This module loads stop-word lists from embedded string resources. The
//! word-set entry points accept an iterator of `&str` lines, which is what
//! Java's `BufferedReader.readLine()` loop yields;
//! [`get_buffered_reader`] produces those lines from a
//! [`CharReader`](crate::analysis::CharReader).

#![deny(unsafe_code)]

use crate::analysis::{CharArrayMap, CharArraySet, CharReader};
use crate::error::Result;

const INITIAL_CAPACITY: usize = 16;

/// `java.lang.String.trim()`.
///
/// Java trims "any character whose codepoint is less than or equal to
/// `'\u0020'`", which is neither Unicode whitespace nor a subset of it:
/// `U+0001` is trimmed by Java but not by Rust's [`str::trim`], and `U+00A0`
/// (no-break space) is trimmed by Rust but not by Java. Every loader in this
/// module trims exactly once per line, so the difference decides whether such a
/// line becomes a stop word, becomes a *different* stop word, or is dropped as
/// blank.
fn java_trim(s: &str) -> &str {
    s.trim_matches(|c: char| c <= '\u{20}')
}

/// Splits on `java.util.regex.Pattern`'s `\s+`, dropping empty fields.
///
/// Java's `\s` is the ASCII class `[ \t\n\x0B\f\r]`, not Unicode
/// whitespace, so `String.split("\\s+")` keeps a no-break space inside a word
/// where Rust's [`str::split_whitespace`] would break on it. Java's `split`
/// also yields a leading empty field when the string starts with a separator,
/// which `getSnowballWordSet`'s `words[i].length() > 0` guard discards — so
/// dropping empty fields here is the same behaviour, not a shortcut.
fn java_split_whitespace(line: &str) -> impl Iterator<Item = &str> {
    line.split([' ', '\t', '\n', '\u{0B}', '\u{0C}', '\r'])
        .filter(|word| !word.is_empty())
}

/// Reads every non-blank line from `lines` into a new [`CharArraySet`].
///
/// Each line is trimmed of leading and trailing whitespace.
pub fn get_word_set(lines: impl IntoIterator<Item = impl AsRef<str>>) -> CharArraySet {
    let mut result = CharArraySet::new(INITIAL_CAPACITY, false);
    get_word_set_into(lines, &mut result);
    CharArraySet::unmodifiable_set(Some(result))
}

/// Reads every non-blank line from `lines` into the supplied [`CharArraySet`].
pub fn get_word_set_into(
    lines: impl IntoIterator<Item = impl AsRef<str>>,
    result: &mut CharArraySet,
) {
    for line in lines {
        let word = java_trim(line.as_ref());
        if !word.is_empty() {
            result.add(word);
        }
    }
}

/// Reads every non-blank line from `lines` into the supplied [`CharArraySet`].
///
/// Lines that start with `comment` are skipped.
pub fn get_word_set_with_comment(
    lines: impl IntoIterator<Item = impl AsRef<str>>,
    comment: &str,
    result: &mut CharArraySet,
) {
    for line in lines {
        let line = line.as_ref();
        if !line.starts_with(comment) {
            let word = java_trim(line);
            if !word.is_empty() {
                result.add(word);
            }
        }
    }
}

/// Reads every non-blank, non-comment line from `lines` into a new
/// [`CharArraySet`].
///
/// Equivalent to the overload `WordlistLoader.getWordSet(Reader, String)`,
/// which returns the set unmodifiable.
pub fn get_word_set_ignoring_comments(
    lines: impl IntoIterator<Item = impl AsRef<str>>,
    comment: &str,
) -> CharArraySet {
    let mut result = CharArraySet::new(INITIAL_CAPACITY, false);
    get_word_set_with_comment(lines, comment, &mut result);
    CharArraySet::unmodifiable_set(Some(result))
}

/// Reads a stop-word list in Snowball format.
///
/// The comment character is `|` and lines may contain multiple
/// whitespace-separated words.
pub fn get_snowball_word_set(lines: impl IntoIterator<Item = impl AsRef<str>>) -> CharArraySet {
    let mut result = CharArraySet::new(INITIAL_CAPACITY, false);
    for line in lines {
        let line = line.as_ref();
        let line = if let Some(idx) = line.find('|') {
            &line[..idx]
        } else {
            line
        };
        for word in java_split_whitespace(line) {
            result.add(word);
        }
    }
    CharArraySet::unmodifiable_set(Some(result))
}

/// Reads every line of `reader`.
///
/// Lines are terminated by a line feed, a carriage return, or a carriage
/// return followed by a line feed, and the terminator is not part of the line;
/// a trailing fragment with no terminator is still a line. That is
/// `java.io.BufferedReader.readLine()`'s contract, which every loader below is
/// written against.
///
/// Divergence: Java's `WordlistLoader.getBufferedReader(Reader)` only wraps the
/// reader — `new BufferedReader(reader)` unless it already is one
/// (`WordlistLoader.java` lines 316-320) — so the caller can then pull lines
/// with `readLine()`. This crate's [`CharReader`](crate::analysis::CharReader)
/// has no line-oriented layer to wrap it in, so the port reads the characters
/// and returns the lines themselves, which is what every caller of the Java
/// method goes on to do.
///
/// # Errors
///
/// Returns whatever the reader returns.
///
/// Equivalent to the private
/// `org.apache.lucene.analysis.WordlistLoader#getBufferedReader`.
pub fn get_buffered_reader(reader: &mut dyn CharReader) -> Result<Vec<String>> {
    let mut buffer = [0u16; 8192];
    let mut units = Vec::new();
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        units.extend_from_slice(&buffer[..read]);
    }
    // A Rust `String` cannot hold a lone surrogate, which becomes U+FFFD
    // (see `utf16_to_string`).
    let text = crate::analysis::character_utils::utf16_to_string(&units);
    Ok(read_lines(&text))
}

/// Splits `text` the way `java.io.BufferedReader.readLine()` does.
///
/// A line is terminated by a line feed, a carriage return, or a carriage return
/// followed by a line feed; the terminator is not part of the line, and a
/// trailing fragment with no terminator is still a line.
pub(crate) fn read_lines(text: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\n' => lines.push(std::mem::take(&mut current)),
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                lines.push(std::mem::take(&mut current));
            }
            _ => current.push(c),
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

/// Reads the data-carrying lines of `stream`, decoded with `char_set`.
///
/// A comment line is any line that starts with `#`. Blank lines are skipped and
/// every kept line is trimmed. A byte-order mark leading the first *kept* line
/// is removed.
///
/// The order of those four steps is Lucene's and is observable:
///
/// 1. the BOM is stripped only while nothing has been kept yet, so a file whose
///    first lines are all comments or blanks still has the mark taken off
///    whichever line is first to survive;
/// 2. the comment test runs **before** the trim, so `"   # x"` is not a comment
///    — it becomes the word `"# x"`;
/// 3. the line is trimmed with [`java_trim`], i.e. `String.trim()`'s rule;
/// 4. only then is a line that has become empty dropped.
///
/// # Errors
///
/// Returns whatever reading `stream` raises, or an error when its bytes are not
/// valid `char_set`.
///
/// Equivalent to
/// `org.apache.lucene.analysis.WordlistLoader#getLines(InputStream, Charset)`.
pub fn get_lines<R: std::io::Read>(stream: R, char_set: &str) -> Result<Vec<String>> {
    let decoded = crate::util::IOUtils::get_decoding_reader(stream, char_set)?;
    let text = decoded.into_inner();

    let mut lines: Vec<String> = Vec::new();
    for line in read_lines(&text) {
        let mut word = line.as_str();
        // Skip the initial BOM marker.
        if lines.is_empty() {
            if let Some(rest) = word.strip_prefix('\u{FEFF}') {
                word = rest;
            }
        }
        // Skip comments.
        if word.starts_with('#') {
            continue;
        }
        let word = java_trim(word);
        // Skip blank lines.
        if word.is_empty() {
            continue;
        }
        lines.push(word.to_string());
    }
    Ok(lines)
}

/// Reads a stem dictionary into `result`.
///
/// Each line holds a word and its stem separated by a single tab; the stem
/// overrules whatever a stemming algorithm would produce for that word.
///
/// Java indexes the second field of `line.split("\t", 2)` unconditionally,
/// so a line without a tab raises `ArrayIndexOutOfBoundsException`
/// (`WordlistLoader.java` lines 276-286), as here.
///
/// # Errors
///
/// Returns whatever the reader returns, or the JVM's
/// `ArrayIndexOutOfBoundsException` (`Index 1 out of bounds for length 1`)
/// when a line carries no tab.
///
/// Equivalent to `org.apache.lucene.analysis.WordlistLoader#getStemDict`.
pub fn get_stem_dict(reader: &mut dyn CharReader, result: &mut CharArrayMap<String>) -> Result<()> {
    for line in get_buffered_reader(reader)? {
        let mut fields = line.splitn(2, '\t');
        let word = fields.next().unwrap_or_default();
        // `wordstem[1]` of `line.split("\t", 2)`: the JVM's
        // `ArrayIndexOutOfBoundsException` for a line without a tab.
        let stem = fields
            .next()
            .ok_or_else(|| crate::store::array_index_out_of_bounds(1, 1))?;
        result.put(word, stem.to_string());
    }
    Ok(())
}

/// `java.io.BufferedReader.readLine()` over a [`CharReader`]: a line ends at
/// a line feed, a carriage return, or a carriage return followed by a line
/// feed, the terminator is not part of the line, and a trailing fragment with
/// no terminator is still a line. Lines are produced as the reader is
/// consumed, so a caller that stops on a read error has seen exactly the lines
/// before it, as Java's `while ((word = br.readLine()) != null)` loops have.
///
/// Each line is converted with
/// [`utf16_to_string`](crate::analysis::character_utils::utf16_to_string)
/// (a lone surrogate becomes U+FFFD, the declared limit of a Rust `String`).
struct LineReader<'r> {
    reader: &'r mut dyn CharReader,
    buffer: Vec<u16>,
    pos: usize,
    len: usize,
    /// `BufferedReader.skipLF`: the previous line ended with a carriage
    /// return, so a line feed that follows it is part of that terminator.
    skip_lf: bool,
}

impl<'r> LineReader<'r> {
    fn new(reader: &'r mut dyn CharReader) -> Self {
        Self {
            reader,
            buffer: vec![0u16; 8192],
            pos: 0,
            len: 0,
            skip_lf: false,
        }
    }

    /// `BufferedReader.readLine()`: the next line, or `None` at the end of
    /// the stream.
    fn read_line(&mut self) -> Result<Option<String>> {
        let mut line: Vec<u16> = Vec::new();
        let mut read_any = false;
        loop {
            if self.pos >= self.len {
                self.len = self.reader.read(&mut self.buffer)?;
                self.pos = 0;
                if self.len == 0 {
                    return Ok(if read_any {
                        Some(crate::analysis::character_utils::utf16_to_string(&line))
                    } else {
                        None
                    });
                }
            }
            let unit = self.buffer[self.pos];
            self.pos += 1;
            if self.skip_lf {
                self.skip_lf = false;
                if unit == u16::from(b'\n') {
                    continue;
                }
            }
            if unit == u16::from(b'\n') {
                return Ok(Some(crate::analysis::character_utils::utf16_to_string(
                    &line,
                )));
            }
            if unit == u16::from(b'\r') {
                self.skip_lf = true;
                return Ok(Some(crate::analysis::character_utils::utf16_to_string(
                    &line,
                )));
            }
            read_any = true;
            line.push(unit);
        }
    }
}

/// Runs `body` over the lines of `reader` and then closes `reader`, as
/// Java's `try (BufferedReader br = getBufferedReader(reader)) { ... }` does:
/// a close failure after a body failure is suppressed into the body failure.
fn with_lines(reader: &mut dyn CharReader, mut body: impl FnMut(&str)) -> Result<()> {
    let read = {
        let mut lines = LineReader::new(&mut *reader);
        loop {
            match lines.read_line() {
                Ok(Some(line)) => body(&line),
                Ok(None) => break Ok(()),
                Err(e) => break Err(e),
            }
        }
    };
    let closed = reader.close();
    match (read, closed) {
        (Ok(()), closed) => closed,
        (Err(e), Ok(())) => Err(e),
        (Err(mut e), Err(close_error)) => {
            e.add_suppressed(close_error);
            Err(e)
        }
    }
}

/// Reads lines from `reader` into a new, unmodifiable [`CharArraySet`],
/// trimming each line and skipping blank lines.
///
/// Equivalent to `org.apache.lucene.analysis.WordlistLoader.getWordSet(Reader)`
/// (`WordlistLoader.java:74-77`).
///
/// # Errors
///
/// Propagates the reader's I/O errors.
pub fn get_word_set_reader(reader: &mut dyn CharReader) -> Result<CharArraySet> {
    let mut result = CharArraySet::new(INITIAL_CAPACITY, false);
    get_word_set_reader_into(reader, &mut result)?;
    Ok(CharArraySet::unmodifiable_set(Some(result)))
}

/// Reads lines from `reader` into `result`, trimming each line and skipping
/// blank lines; `reader` is closed afterwards.
///
/// Equivalent to `WordlistLoader.getWordSet(Reader, CharArraySet)`
/// (`WordlistLoader.java:52-64`), which returns `result`.
///
/// # Errors
///
/// Propagates the reader's I/O errors.
pub fn get_word_set_reader_into(
    reader: &mut dyn CharReader,
    result: &mut CharArraySet,
) -> Result<()> {
    with_lines(reader, |line| {
        let word = java_trim(line);
        // skip blank lines
        if !word.is_empty() {
            result.add(word);
        }
    })
}

/// Reads lines from `stream`, decoded with `charset`, into a new,
/// unmodifiable [`CharArraySet`].
///
/// Equivalent to `WordlistLoader.getWordSet(InputStream, Charset)`
/// (`WordlistLoader.java:100-103`), and with `charset` `"UTF-8"` to
/// `getWordSet(InputStream)` (`WordlistLoader.java:88-90`). The stream is
/// decoded through
/// [`IOUtils::get_decoding_reader`](crate::util::IOUtils::get_decoding_reader),
/// the port's `IOUtils.getDecodingReader`, so malformed input is an error.
///
/// # Errors
///
/// Propagates I/O and decoding errors.
pub fn get_word_set_stream(stream: &mut dyn std::io::Read, charset: &str) -> Result<CharArraySet> {
    let mut reader = decoding_reader(stream, charset)?;
    get_word_set_reader(&mut reader)
}

/// Reads lines from `reader` into a new, unmodifiable [`CharArraySet`],
/// skipping lines that start with `comment`.
///
/// Equivalent to `WordlistLoader.getWordSet(Reader, String)`
/// (`WordlistLoader.java:141-144`).
///
/// # Errors
///
/// Propagates the reader's I/O errors.
pub fn get_word_set_reader_with_comment(
    reader: &mut dyn CharReader,
    comment: &str,
) -> Result<CharArraySet> {
    let mut result = CharArraySet::new(INITIAL_CAPACITY, false);
    get_word_set_reader_with_comment_into(reader, comment, &mut result)?;
    Ok(CharArraySet::unmodifiable_set(Some(result)))
}

/// Reads lines from `reader` into `result`, skipping lines that start with
/// `comment`; `reader` is closed afterwards.
///
/// Equivalent to `WordlistLoader.getWordSet(Reader, String, CharArraySet)`
/// (`WordlistLoader.java:116-130`), which returns `result`.
///
/// # Errors
///
/// Propagates the reader's I/O errors.
pub fn get_word_set_reader_with_comment_into(
    reader: &mut dyn CharReader,
    comment: &str,
    result: &mut CharArraySet,
) -> Result<()> {
    with_lines(reader, |line| {
        if !line.starts_with(comment) {
            let word = java_trim(line);
            // skip blank lines
            if !word.is_empty() {
                result.add(word);
            }
        }
    })
}

/// Reads lines from `stream` decoded with `charset`, skipping lines that start
/// with `comment`.
///
/// Equivalent to `WordlistLoader.getWordSet(InputStream, Charset, String)`
/// (`WordlistLoader.java:171-174`), and with `charset` `"UTF-8"` to
/// `getWordSet(InputStream, String)` (`WordlistLoader.java:156-158`).
///
/// # Errors
///
/// Propagates I/O and decoding errors.
pub fn get_word_set_stream_with_comment(
    stream: &mut dyn std::io::Read,
    charset: &str,
    comment: &str,
) -> Result<CharArraySet> {
    let mut reader = decoding_reader(stream, charset)?;
    get_word_set_reader_with_comment(&mut reader, comment)
}

/// Reads a snowball-format stop word file from `reader` into a new,
/// unmodifiable [`CharArraySet`]: `|` starts a comment, and a line may hold
/// several words.
///
/// Equivalent to `WordlistLoader.getSnowballWordSet(Reader)`
/// (`WordlistLoader.java:222-225`).
///
/// # Errors
///
/// Propagates the reader's I/O errors.
pub fn get_snowball_word_set_reader(reader: &mut dyn CharReader) -> Result<CharArraySet> {
    let mut result = CharArraySet::new(INITIAL_CAPACITY, false);
    get_snowball_word_set_reader_into(reader, &mut result)?;
    Ok(CharArraySet::unmodifiable_set(Some(result)))
}

/// Reads a snowball-format stop word file from `reader` into `result`;
/// `reader` is closed afterwards.
///
/// Equivalent to `WordlistLoader.getSnowballWordSet(Reader, CharArraySet)`
/// (`WordlistLoader.java:192-206`), which returns `result`.
///
/// # Errors
///
/// Propagates the reader's I/O errors.
pub fn get_snowball_word_set_reader_into(
    reader: &mut dyn CharReader,
    result: &mut CharArraySet,
) -> Result<()> {
    with_lines(reader, |line| {
        let line = match line.find('|') {
            Some(comment) => &line[..comment],
            None => line,
        };
        for word in java_split_whitespace(line) {
            result.add(word);
        }
    })
}

/// Reads a snowball-format stop word file from `stream` decoded with
/// `charset`.
///
/// Equivalent to `WordlistLoader.getSnowballWordSet(InputStream, Charset)`
/// (`WordlistLoader.java:260-263`), and with `charset` `"UTF-8"` to
/// `getSnowballWordSet(InputStream)` (`WordlistLoader.java:240-242`).
///
/// # Errors
///
/// Propagates I/O and decoding errors.
pub fn get_snowball_word_set_stream(
    stream: &mut dyn std::io::Read,
    charset: &str,
) -> Result<CharArraySet> {
    let mut reader = decoding_reader(stream, charset)?;
    get_snowball_word_set_reader(&mut reader)
}

/// `IOUtils.getDecodingReader(stream, charset)` as a [`CharReader`].
fn decoding_reader(
    stream: &mut dyn std::io::Read,
    charset: &str,
) -> Result<crate::analysis::StringCharReader> {
    let decoded = crate::util::IOUtils::get_decoding_reader(stream, charset)?;
    Ok(crate::analysis::StringCharReader::new(decoded.into_inner()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_word_set_trims_and_skips_blank_lines() {
        let text = "  hello  \n\nworld\n  \n";
        let set = get_word_set(text.lines());
        assert_eq!(set.size(), 2);
        assert!(set.contains(Some("hello")));
        assert!(set.contains(Some("world")));
    }

    #[test]
    fn get_word_set_ignoring_comments_parses_correctly() {
        let text = "alpha\n# this is a comment\nbeta\n# another\ngamma";
        let set = get_word_set_ignoring_comments(text.lines(), "#");
        assert_eq!(set.size(), 3);
        assert!(set.contains(Some("alpha")));
        assert!(set.contains(Some("beta")));
        assert!(set.contains(Some("gamma")));
    }

    #[test]
    fn get_snowball_word_set_parses_comments_and_whitespace() {
        let text = "a an the | English articles\nis are was were | verbs";
        let set = get_snowball_word_set(text.lines());
        assert_eq!(set.size(), 7);
        for word in ["a", "an", "the", "is", "are", "was", "were"] {
            assert!(set.contains(Some(word)));
        }
    }
}
