//! Word-list loader ported from `org.apache.lucene.analysis.WordlistLoader`.
//!
//! This module loads stop-word lists from embedded string resources. For now
//! all APIs accept an iterator of `&str` lines; file and byte-stream wrappers
//! can be added later on top of the existing [`crate::analysis::CharReader`].

#![deny(unsafe_code)]

use crate::analysis::CharArraySet;

const INITIAL_CAPACITY: usize = 16;

/// Reads every non-blank line from `lines` into a new [`CharArraySet`].
///
/// Each line is trimmed of leading and trailing whitespace.
pub fn get_word_set(lines: impl IntoIterator<Item = impl AsRef<str>>) -> CharArraySet {
    let mut result = CharArraySet::new(INITIAL_CAPACITY, false);
    get_word_set_into(lines, &mut result);
    result
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
    result
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
    result
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
