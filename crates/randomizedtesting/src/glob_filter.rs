//! Port of `com.carrotsearch.randomizedtesting.GlobFilter`,
//! `ClassGlobFilter` and `MethodGlobFilter` (randomizedtesting 2.8.1).

use crate::jdk::exception::Throwable;
use crate::junit::{Description, Filter};

/// A filter matching a glob pattern (`GlobFilter`): `*` matches any
/// sequence, `?` any character.
///
/// Java turns the glob into a `java.util.regex.Pattern` (escaping
/// `.$-{}[]()\`) and matches whole strings; the other characters pass to
/// the regex unchanged, so `|`, `^` and `+` keep their regex meaning. The
/// port compiles the same regex text with `GlobRegex`, which implements
/// exactly the regex constructs such a text can hold.
#[derive(Debug, Clone)]
pub struct GlobFilter {
    glob_pattern: String,
    pattern: GlobRegex,
}

impl GlobFilter {
    /// `new GlobFilter(glob)`.
    ///
    /// # Panics
    ///
    /// `java.util.regex.PatternSyntaxException` when the regex is invalid
    /// (a dangling `+`, say), as `Pattern.compile` throws.
    pub fn new(glob: &str) -> GlobFilter {
        let regex = glob_to_pattern(glob);
        let pattern = match GlobRegex::compile(&regex) {
            Ok(p) => p,
            Err(e) => e.throw(),
        };
        GlobFilter {
            glob_pattern: glob.to_string(),
            pattern,
        }
    }

    /// `globMatches(string)`.
    pub fn glob_matches(&self, s: &str) -> bool {
        self.pattern.matches(s)
    }

    /// The glob (`globPattern`).
    pub fn glob_pattern(&self) -> &str {
        &self.glob_pattern
    }
}

/// `globToPattern(glob)`.
fn glob_to_pattern(glob: &str) -> String {
    let mut out = String::from("^");
    for ch in glob.chars() {
        match ch {
            '*' => out.push_str(".*"),
            '?' => out.push('.'),
            '.' | '$' | '-' | '{' | '}' | '[' | ']' | '(' | ')' => {
                out.push('\\');
                out.push(ch);
            }
            '\\' => out.push_str("\\\\"),
            c => out.push(c),
        }
    }
    out.push('$');
    out
}

/// One atom of a [`GlobRegex`] alternative.
#[derive(Debug, Clone, PartialEq)]
enum Atom {
    /// `.`: any character but a line terminator.
    Any,
    /// A literal character.
    Char(char),
    /// `^`: the start of the input.
    Begin,
    /// `$`: the end of the input (or before a final line terminator).
    End,
}

/// The quantifier of an atom.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Quant {
    One,
    Star,
    Plus,
    StarPossessive,
    PlusPossessive,
}

/// The regex subset a glob's pattern text can hold: alternatives (`|`) of
/// atoms (`.`, `^`, `$`, escaped or plain characters), each optionally
/// quantified by `*` or `+`, greedy or possessive (`*+`, `++`), matched
/// against the whole input as `Matcher.matches()` does.
#[derive(Debug, Clone)]
pub struct GlobRegex {
    alternatives: Vec<Vec<(Atom, Quant)>>,
}

fn is_line_terminator(c: char) -> bool {
    matches!(c, '\n' | '\r' | '\u{85}' | '\u{2028}' | '\u{2029}')
}

impl GlobRegex {
    /// `Pattern.compile(regex)` for the subset.
    pub fn compile(regex: &str) -> Result<GlobRegex, Throwable> {
        let chars: Vec<char> = regex.chars().collect();
        let mut alternatives = vec![Vec::new()];
        let mut i = 0;
        while i < chars.len() {
            let c = chars[i];
            let atom = match c {
                '|' => {
                    alternatives.push(Vec::new());
                    i += 1;
                    continue;
                }
                '*' | '+' | '?' => {
                    let cur = alternatives.last_mut().expect("alternative");
                    // A quantifier applies to the previous atom.
                    match cur.last_mut() {
                        Some((_, q)) => {
                            *q = match (*q, c) {
                                (Quant::One, '*') => Quant::Star,
                                (Quant::One, '+') => Quant::Plus,
                                (Quant::Star, '+') => Quant::StarPossessive,
                                (Quant::Plus, '+') => Quant::PlusPossessive,
                                _ => return Err(dangling(c, i, regex)),
                            };
                            i += 1;
                            continue;
                        }
                        _ => return Err(dangling(c, i, regex)),
                    }
                }
                '.' => Atom::Any,
                '^' => Atom::Begin,
                '$' => Atom::End,
                '\\' => {
                    i += 1;
                    match chars.get(i) {
                        Some(&e) => Atom::Char(e),
                        None => {
                            return Err(Throwable::new(
                                "java.util.regex.PatternSyntaxException",
                                format!("Unexpected internal error near index {}\n{}", i, regex),
                            ))
                        }
                    }
                }
                c => Atom::Char(c),
            };
            alternatives
                .last_mut()
                .expect("alternative")
                .push((atom, Quant::One));
            i += 1;
        }
        Ok(GlobRegex { alternatives })
    }

    /// `pattern.matcher(s).matches()`.
    pub fn matches(&self, s: &str) -> bool {
        let input: Vec<char> = s.chars().collect();
        self.alternatives
            .iter()
            .any(|alt| match_from(alt, &input, 0))
    }
}

fn atom_matches(atom: &Atom, input: &[char], pos: usize) -> Option<usize> {
    match atom {
        Atom::Any => match input.get(pos) {
            Some(&c) if !is_line_terminator(c) => Some(pos + 1),
            _ => None,
        },
        Atom::Char(c) => (input.get(pos) == Some(c)).then_some(pos + 1),
        Atom::Begin => (pos == 0).then_some(pos),
        Atom::End => {
            let rest = &input[pos.min(input.len())..];
            let at_end = rest.is_empty()
                || (rest.len() == 1 && is_line_terminator(rest[0]))
                || (rest.len() == 2 && rest[0] == '\r' && rest[1] == '\n');
            at_end.then_some(pos)
        }
    }
}

/// Backtracking match of `alt` at `pos` that must consume the whole input.
fn match_from(alt: &[(Atom, Quant)], input: &[char], pos: usize) -> bool {
    let Some(((atom, quant), rest)) = alt.split_first() else {
        return pos == input.len();
    };
    if matches!(atom, Atom::Begin | Atom::End) && *quant != Quant::One {
        // A quantified anchor: `*` makes it optional, `+` requires it once.
        let required = matches!(quant, Quant::Plus | Quant::PlusPossessive);
        return match atom_matches(atom, input, pos) {
            Some(p) => match_from(rest, input, p),
            None if !required => match_from(rest, input, pos),
            None => false,
        };
    }
    match quant {
        Quant::One => match atom_matches(atom, input, pos) {
            Some(p) => match_from(rest, input, p),
            None => false,
        },
        Quant::Star | Quant::Plus | Quant::StarPossessive | Quant::PlusPossessive => {
            let mut ends = vec![pos];
            let mut p = pos;
            while let Some(n) = atom_matches(atom, input, p) {
                if n == p {
                    break;
                }
                ends.push(n);
                p = n;
            }
            let min = if matches!(quant, Quant::Plus | Quant::PlusPossessive) {
                1
            } else {
                0
            };
            if ends.len() <= min {
                return false;
            }
            if matches!(quant, Quant::StarPossessive | Quant::PlusPossessive) {
                return match_from(rest, input, *ends.last().expect("end"));
            }
            ends[min..]
                .iter()
                .rev()
                .any(|&e| match_from(rest, input, e))
        }
    }
}

/// `PatternSyntaxException` for a quantifier without an operand.
fn dangling(c: char, index: usize, regex: &str) -> Throwable {
    Throwable::new(
        "java.util.regex.PatternSyntaxException",
        format!(
            "Dangling meta character '{c}' near index {index}\n{regex}\n{}^",
            " ".repeat(index)
        ),
    )
}

/// Filters by the class name of a description (`ClassGlobFilter`).
#[derive(Debug, Clone)]
pub struct ClassGlobFilter(GlobFilter);

impl ClassGlobFilter {
    /// `new ClassGlobFilter(globPattern)`.
    pub fn new(glob: &str) -> ClassGlobFilter {
        ClassGlobFilter(GlobFilter::new(glob))
    }
}

impl Filter for ClassGlobFilter {
    /// A description without a class name passes.
    fn should_run(&self, description: &Description) -> bool {
        match description.class_name() {
            None => true,
            Some(c) => self.0.glob_matches(&c),
        }
    }

    fn describe(&self) -> String {
        format!("Class matches: {}", self.0.glob_pattern)
    }
}

/// Filters by the method name of a description (`MethodGlobFilter`).
#[derive(Debug, Clone)]
pub struct MethodGlobFilter(GlobFilter);

impl MethodGlobFilter {
    /// `new MethodGlobFilter(globPattern)`.
    pub fn new(glob: &str) -> MethodGlobFilter {
        MethodGlobFilter(GlobFilter::new(glob))
    }
}

impl Filter for MethodGlobFilter {
    /// A description without a method name passes.
    fn should_run(&self, description: &Description) -> bool {
        match description.method_name() {
            None => true,
            Some(m) => self.0.glob_matches(&m),
        }
    }

    fn describe(&self) -> String {
        format!("Method matches: {}", self.0.glob_pattern)
    }
}
