//! Word-list loader ported from `org.apache.lucene.analysis.WordlistLoader`.
//!
//! This module loads stop-word lists from embedded string resources. The
//! word-set entry points accept an iterator of `&str` lines, which is what
//! Java's `BufferedReader.readLine()` loop yields;
//! [`get_buffered_reader`] produces those lines from a
//! [`CharReader`](crate::analysis::CharReader).

#![deny(unsafe_code)]

use crate::analysis::{CharArrayMap, CharArraySet, CharReader};
use crate::error::{LuceneError, Result};

const INITIAL_CAPACITY: usize = 16;

/// Reads every non-blank line from `lines` into a new [`CharArraySet`].
///
/// Each line is trimmed of leading and trailing whitespace.
pub fn get_word_set(lines: impl IntoIterator<Item = impl AsRef<str>>) -> CharArraySet {
    let mut result = CharArraySet::new(INITIAL_CAPACITY, false);
    get_word_set_into(lines, &mut result);
    CharArraySet::unmodifiable_set(result)
}

/// Reads every non-blank line from `lines` into the supplied [`CharArraySet`].
pub fn get_word_set_into(
    lines: impl IntoIterator<Item = impl AsRef<str>>,
    result: &mut CharArraySet,
) {
    for line in lines {
        let word = line.as_ref().trim();
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
            let word = line.trim();
            if !word.is_empty() {
                result.add(word);
            }
        }
    }
}

/// Reads every non-blank, non-comment line from `lines` into a new
/// [`CharArraySet`].
pub fn get_word_set_ignoring_comments(
    lines: impl IntoIterator<Item = impl AsRef<str>>,
    comment: &str,
) -> CharArraySet {
    let mut result = CharArraySet::new(INITIAL_CAPACITY, false);
    get_word_set_with_comment(lines, comment, &mut result);
    CharArraySet::unmodifiable_set(result)
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
        for word in line.split_whitespace() {
            if !word.is_empty() {
                result.add(word);
            }
        }
    }
    CharArraySet::unmodifiable_set(result)
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
    let mut buffer = ['\0'; 8192];
    let mut text = String::new();
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        text.extend(buffer[..read].iter());
    }

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
    Ok(lines)
}

/// Reads a stem dictionary into `result`.
///
/// Each line holds a word and its stem separated by a single tab; the stem
/// overrules whatever a stemming algorithm would produce for that word.
///
/// Divergence: Java indexes the second field of `line.split("\t", 2)`
/// unconditionally, so a line without a tab raises
/// `ArrayIndexOutOfBoundsException` (`WordlistLoader.java` lines 274-283). The
/// port reports that malformed line as an error instead, because the method
/// already returns `Result` and a panic would give the caller no way to say
/// which line was wrong.
///
/// # Errors
///
/// Returns whatever the reader returns, or
/// [`LuceneError::IllegalArgument`] when a line carries no tab.
///
/// Equivalent to `org.apache.lucene.analysis.WordlistLoader#getStemDict`.
pub fn get_stem_dict(reader: &mut dyn CharReader, result: &mut CharArrayMap<String>) -> Result<()> {
    for line in get_buffered_reader(reader)? {
        let mut fields = line.splitn(2, '\t');
        let word = fields.next().unwrap_or_default();
        let stem = fields.next().ok_or_else(|| {
            LuceneError::IllegalArgument(format!(
                "stem dictionary line has no tab separating word and stem: {line}"
            ))
        })?;
        result.put(word, stem.to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_word_set_trims_and_skips_blank_lines() {
        let text = "  hello  \n\nworld\n  \n";
        let set = get_word_set(text.lines());
        assert_eq!(set.size(), 2);
        assert!(set.contains("hello"));
        assert!(set.contains("world"));
    }

    #[test]
    fn get_word_set_ignoring_comments_parses_correctly() {
        let text = "alpha\n# this is a comment\nbeta\n# another\ngamma";
        let set = get_word_set_ignoring_comments(text.lines(), "#");
        assert_eq!(set.size(), 3);
        assert!(set.contains("alpha"));
        assert!(set.contains("beta"));
        assert!(set.contains("gamma"));
    }

    #[test]
    fn get_snowball_word_set_parses_comments_and_whitespace() {
        let text = "a an the | English articles\nis are was were | verbs";
        let set = get_snowball_word_set(text.lines());
        assert_eq!(set.size(), 7);
        for word in ["a", "an", "the", "is", "are", "was", "were"] {
            assert!(set.contains(word));
        }
    }
}
