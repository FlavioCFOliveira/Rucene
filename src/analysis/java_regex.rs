//! The front end of `java.util.regex.Pattern`, translated to `fancy-regex`.
//!
//! `AbstractAnalysisFactory.getPattern` compiles its argument with
//! `java.util.regex.Pattern.compile`, and Rust has no `java.util.regex`. This
//! module parses a pattern in Java's dialect, as the JDK 25 that runs Lucene
//! 10.5.0 parses it, and writes an equivalent pattern in the dialect of
//! `fancy-regex` 0.19, whose character classes are those of `regex-syntax`
//! 0.8 (Unicode 16.0, as the JDK's). Java's meaning is spelled out in the
//! output rather than left to `fancy-regex`: every literal, class, property,
//! anchor and boundary becomes an explicit class or look-around, and no
//! `fancy-regex` flag is used but the case-insensitive back reference.
//!
//! The parser is written from the `java.util.regex.Pattern` Javadoc and from
//! the behaviour the JDK shows on probes: the oracle `RegexOracle.java` of rmp
//! task #243 runs each construct through `Pattern.compile` and
//! `Matcher.find`, `DumpData.java` dumps the character data of [`tables`], and
//! `NameData.java` the character names of [`names`]. OpenJDK's source (GPL
//! with the Classpath Exception) was not copied. The parse follows the order in which the JDK reads a pattern,
//! because that order is observable: `\Q…\E` is replaced by escapes before
//! the parse, comments mode skips whitespace and `#` comments wherever the
//! parser looks ahead, a run of literals is one "slice" whose case folding
//! differs from a lone literal's, and the characters below U+0100 of a class
//! accumulate in one set that every intersection of the class refers to, so
//! characters added after an `&&` count in it too.
//!
//! What `fancy-regex` cannot express, and the pattern therefore cannot
//! reproduce, is declared on
//! [`AbstractAnalysisFactory::get_pattern`](crate::analysis::factories::AbstractAnalysisFactory::get_pattern).

mod names;
mod tables;

use std::collections::HashMap;
use std::fmt::{self, Write as _};
use std::sync::OnceLock;

/// `Pattern.UNIX_LINES`, `(?d)`.
const UNIX_LINES: u32 = 0x01;
/// `Pattern.CASE_INSENSITIVE`, `(?i)`.
const CASE_INSENSITIVE: u32 = 0x02;
/// `Pattern.COMMENTS`, `(?x)`.
const COMMENTS: u32 = 0x04;
/// `Pattern.MULTILINE`, `(?m)`.
const MULTILINE: u32 = 0x08;
/// `Pattern.DOTALL`, `(?s)`.
const DOTALL: u32 = 0x20;
/// `Pattern.UNICODE_CASE`, `(?u)`.
const UNICODE_CASE: u32 = 0x40;
/// `Pattern.CANON_EQ`, `(?c)`.
const CANON_EQ: u32 = 0x80;
/// `Pattern.UNICODE_CHARACTER_CLASS`, `(?U)`.
const UNICODE_CHARACTER_CLASS: u32 = 0x100;

/// The largest repetition count, `Pattern`'s `MAX_REPS`.
const MAX_REPS: u32 = 0x7FFF_FFFF;

/// The nesting of groups at which `fancy-regex` refuses a pattern (its
/// `MAX_RECURSION`); every group of the Java pattern is a group of the
/// translation, so a pattern nested this deep cannot be translated.
const MAX_GROUP_DEPTH: usize = 64;

/// The nesting of classes at which `regex-syntax`, which parses the classes of
/// `fancy-regex`, refuses a pattern (its default `nest_limit`).
const MAX_CLASS_DEPTH: usize = 250;

/// The description of a pattern nested deeper than the engine allows.
const TOO_DEEP: &str = "Pattern nested deeper than this port's regular expression engine allows";

/// Every code point, as a class range.
const ANY: &str = r"\x{0}-\x{10FFFF}";

/// The line terminators of `.`, `^` and `$` outside `UNIX_LINES` mode.
const LINE_TERMINATORS: &str = r"[\n\r\x{85}\x{2028}\x{2029}]";

/// A pattern `java.util.regex.Pattern` refuses.
///
/// Equivalent to `java.util.regex.PatternSyntaxException`: a description of
/// the error, the index near which it was found (`-1` when unknown), and the
/// pattern.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatternSyntaxError {
    description: String,
    index: i64,
    pattern: String,
}

impl fmt::Display for PatternSyntaxError {
    /// The description, the index, the pattern and a caret under the index,
    /// as `PatternSyntaxException.getMessage()` lays them out.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.description)?;
        if self.index >= 0 {
            write!(f, " near index {}", self.index)?;
        }
        write!(f, "\n{}", self.pattern)?;
        if self.index >= 0 && (self.index as usize) < self.pattern.encode_utf16().count() {
            write!(f, "\n{}^", " ".repeat(self.index as usize))?;
        }
        Ok(())
    }
}

impl std::error::Error for PatternSyntaxError {}

/// A translation, with what the engine must know to run it as Java does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Translation {
    /// The `fancy-regex` pattern.
    pub(crate) pattern: String,
    /// Whether the pattern repeats, with a count — `{n,m}` or `{n}` beyond
    /// one iteration, `{n,}` with `n` beyond one — a body that can match the
    /// empty string without only matching it.
    ///
    /// Java's loop over such a body ends at the first iteration that matches
    /// the empty string. The backtracking VM of `fancy-regex` checks for an
    /// empty iteration only in an unbounded repetition, and only once its
    /// minimum is reached; otherwise it backtracks through the ways of
    /// spreading the input over the iterations — `(?m)^(?:x|){3000}(?=b)`
    /// over `xxxx` — and runs every iteration, holding one backtracking entry
    /// for each — `^(?:a?){2147483647}` over `xxxx` — where Java answers at
    /// once.
    pub(crate) bounded_empty_loop: bool,
}

/// Translates a `java.util.regex` pattern into a `fancy-regex` pattern that
/// matches the same text, with what the engine needs to know to run it (see
/// [`Translation`]).
///
/// Equivalent to the parse `Pattern.compile(regex)` makes, with no flags set.
///
/// When `counts_in_vm`, every counted repetition (`{n}`, `{n,}`, `{n,m}`)
/// whose body cannot match the empty string, outside a look-behind, is
/// written so that the backtracking VM of `fancy-regex` runs it with a
/// counter. `fancy-regex` hands every part of a pattern without look-arounds
/// or back references to the automaton of `regex-automata`, which writes a
/// counted repetition out as that many copies of its body and refuses an
/// automaton past its size limit: `\p{L}{1000}` and `a{2147483647}` are
/// refused, where Java's `Pattern` repeats a node with a counter whatever the
/// count. An empty look-ahead, which always matches and consumes nothing,
/// appended to the body makes the repetition one the VM runs itself, with its
/// `RepeatGr` and `RepeatNg` counters, in a program of constant size. A
/// look-behind keeps the automaton form, as `fancy-regex` matches a
/// look-behind of variable length only with its automaton; and so does a body
/// that can match the empty string, which the VM would loop over without
/// Java's check (see [`Translation::bounded_empty_loop`]).
///
/// # Errors
///
/// Returns a [`PatternSyntaxError`] where `Pattern.compile` throws
/// `PatternSyntaxException`, and where the pattern needs a construct this
/// translation does not reproduce (see the module documentation).
pub(crate) fn translate_for_engine(
    regex: &str,
    counts_in_vm: bool,
) -> Result<Translation, PatternSyntaxError> {
    let mut parser = Parser::new(regex);
    let node = parser.compile()?;
    let mut emitter = Emitter {
        bits: &parser.bits,
        group_count: parser.capturing_group_count - 1,
        counts_in_vm,
        look_behind_depth: 0,
        out: String::new(),
    };
    emitter.node(&node);
    Ok(Translation {
        pattern: emitter.out,
        bounded_empty_loop: has_bounded_empty_loop(&node),
    })
}

// ---------------------------------------------------------------------------
// The parsed pattern
// ---------------------------------------------------------------------------

/// A predicate over one code point: a literal, a class, a property or `.`.
#[derive(Debug, Clone)]
enum Pred {
    /// An explicit set of code points.
    Set(CharSet),
    /// A class of `regex-syntax`, brackets included.
    Class(String),
    /// The characters below U+0100 a class accumulates, by index into
    /// [`Parser::bits`]; read once the whole pattern is parsed.
    Bits(usize),
    Union(Vec<Pred>),
    And(Box<Pred>, Box<Pred>),
    Not(Box<Pred>),
}

impl Pred {
    fn class(text: &str) -> Self {
        Pred::Class(text.to_string())
    }

    fn not(self) -> Self {
        Pred::Not(Box::new(self))
    }

    fn and(self, other: Pred) -> Self {
        Pred::And(Box::new(self), Box::new(other))
    }

    fn union(self, other: Pred) -> Self {
        match self {
            Pred::Union(mut members) => {
                members.push(other);
                Pred::Union(members)
            }
            first => Pred::Union(vec![first, other]),
        }
    }

    fn ranges(ranges: &[(u32, u32)]) -> Self {
        let mut set = CharSet::default();
        for &(lo, hi) in ranges {
            set.add_range(lo, hi);
        }
        Pred::Set(set)
    }

    fn chars(chars: &str) -> Self {
        let mut set = CharSet::default();
        for c in chars.chars() {
            set.add(c as u32);
        }
        Pred::Set(set)
    }
}

/// How a quantifier matches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Greed {
    Greedy,
    Lazy,
    Possessive,
}

/// The kind of a parenthesised group.
#[derive(Debug, Clone, PartialEq, Eq)]
enum GroupKind {
    Capturing,
    Named(String),
    NonCapturing,
    Atomic,
    LookAhead,
    NegativeLookAhead,
    LookBehind,
    NegativeLookBehind,
}

/// A node of the parsed pattern.
#[derive(Debug, Clone)]
enum Node {
    Empty,
    /// One code point.
    Char(Pred),
    Sequence(Vec<Node>),
    Alternation(Vec<Node>),
    Group(GroupKind, Box<Node>),
    Repeat {
        body: Box<Node>,
        min: u32,
        max: u32,
        greed: Greed,
    },
    /// A back reference to a group number, case-insensitive when `casei`.
    BackRef {
        group: usize,
        casei: bool,
    },
    /// `\R`, Java's `LineEnding`.
    LineEnding,
    /// `\X`, Java's `XGrapheme`.
    Grapheme,
    /// A zero-width assertion, already written in `fancy-regex` syntax.
    Assertion(String),
}

impl Node {
    /// Whether the node only ever matches the empty string.
    fn is_zero_width(&self) -> bool {
        match self {
            Node::Empty | Node::Assertion(_) => true,
            Node::Group(kind, body) => match kind {
                GroupKind::LookAhead
                | GroupKind::NegativeLookAhead
                | GroupKind::LookBehind
                | GroupKind::NegativeLookBehind => true,
                _ => body.is_zero_width(),
            },
            Node::Sequence(nodes) | Node::Alternation(nodes) => {
                nodes.iter().all(Node::is_zero_width)
            }
            Node::Repeat { body, max, .. } => *max == 0 || body.is_zero_width(),
            Node::Char(_) | Node::BackRef { .. } | Node::LineEnding | Node::Grapheme => false,
        }
    }
}

// ---------------------------------------------------------------------------
// Code point sets and Java's case mappings
// ---------------------------------------------------------------------------

/// A set of code points as sorted, disjoint, non-adjacent ranges.
#[derive(Debug, Clone, Default)]
struct CharSet {
    ranges: Vec<(u32, u32)>,
}

impl CharSet {
    fn add(&mut self, c: u32) {
        self.add_range(c, c);
    }

    fn add_range(&mut self, lo: u32, hi: u32) {
        if lo > hi {
            return;
        }
        let at = self
            .ranges
            .partition_point(|&(_, end)| end.saturating_add(1) < lo);
        let mut lo = lo;
        let mut hi = hi;
        let mut until = at;
        while until < self.ranges.len() && self.ranges[until].0 <= hi.saturating_add(1) {
            lo = lo.min(self.ranges[until].0);
            hi = hi.max(self.ranges[until].1);
            until += 1;
        }
        self.ranges.splice(at..until, [(lo, hi)]);
    }
}

/// `Character.toUpperCase(int)` and `Character.toLowerCase(int)` of `c`.
fn case_of(c: u32) -> (u32, u32) {
    let rows = tables::CASE_MAPPINGS.len() / 3;
    let (mut low, mut high) = (0, rows);
    while low < high {
        let mid = (low + high) / 2;
        let key = tables::CASE_MAPPINGS[3 * mid];
        if key < c {
            low = mid + 1;
        } else if key > c {
            high = mid;
        } else {
            return (
                tables::CASE_MAPPINGS[3 * mid + 1],
                tables::CASE_MAPPINGS[3 * mid + 2],
            );
        }
    }
    (c, c)
}

/// `Character.toUpperCase(int)`.
fn to_upper(c: u32) -> u32 {
    case_of(c).0
}

/// `Character.toLowerCase(int)`.
fn to_lower(c: u32) -> u32 {
    case_of(c).1
}

/// Every code point `c` with `Character.toLowerCase(Character.toUpperCase(c))
/// == lower`: what Java's Unicode-case comparison of one character
/// (`SingleU`, and each character of a `SliceU`) accepts.
fn unicode_case_set(lower: u32) -> CharSet {
    let mut set = CharSet::default();
    if to_lower(to_upper(lower)) == lower {
        set.add(lower);
    }
    for row in tables::CASE_MAPPINGS.as_chunks::<3>().0 {
        if to_lower(row[1]) == lower {
            set.add(row[0]);
        }
    }
    set
}

fn is_ascii_alpha(c: u32) -> bool {
    (u32::from(b'a')..=u32::from(b'z')).contains(&c)
        || (u32::from(b'A')..=u32::from(b'Z')).contains(&c)
}

fn is_ascii_digit(c: u32) -> bool {
    (u32::from(b'0')..=u32::from(b'9')).contains(&c)
}

fn is_ascii_alnum(c: u32) -> bool {
    is_ascii_alpha(c) || is_ascii_digit(c)
}

/// `ASCII.isSpace`: space, tab, line feed, vertical tab, form feed, carriage
/// return.
fn is_ascii_space(c: u32) -> bool {
    matches!(c, 0x20 | 0x09 | 0x0A | 0x0B | 0x0C | 0x0D)
}

fn ascii_to_lower(c: u32) -> u32 {
    if (u32::from(b'A')..=u32::from(b'Z')).contains(&c) {
        c + 0x20
    } else {
        c
    }
}

fn ascii_to_upper(c: u32) -> u32 {
    if (u32::from(b'a')..=u32::from(b'z')).contains(&c) {
        c - 0x20
    } else {
        c
    }
}

fn is_hex_digit(c: u32) -> bool {
    char::from_u32(c).is_some_and(|c| c.is_ascii_hexdigit())
}

fn hex_value(c: u32) -> u32 {
    char::from_u32(c)
        .and_then(|c| c.to_digit(16))
        .expect("a hexadecimal digit")
}

fn is_high_surrogate(c: u32) -> bool {
    (0xD800..=0xDBFF).contains(&c)
}

fn is_low_surrogate(c: u32) -> bool {
    (0xDC00..=0xDFFF).contains(&c)
}

const fn ch(c: char) -> u32 {
    c as u32
}

// ---------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------

/// The parser state, as `Pattern` keeps it while it compiles.
struct Parser {
    /// The pattern as written, for error messages.
    pattern: String,
    /// The pattern's code points after `\Q…\E` removal, followed by two `0`
    /// sentinels.
    temp: Vec<u32>,
    /// The number of code points before the sentinels.
    pattern_length: usize,
    cursor: usize,
    /// The flags in effect, `Pattern.flags0`.
    flags0: u32,
    /// One more than the number of capturing groups opened so far.
    capturing_group_count: usize,
    named_groups: HashMap<String, usize>,
    /// The `BitClass` of every class parsed: its characters below U+0100.
    bits: Vec<[bool; 256]>,
    /// How many groups, and how many classes, enclose the cursor.
    group_depth: usize,
    class_depth: usize,
}

type ParseResult<T> = Result<T, PatternSyntaxError>;

impl Parser {
    fn new(regex: &str) -> Self {
        let code_points: Vec<u32> = regex.chars().map(|c| c as u32).collect();
        let mut temp = remove_qe_quoting(&code_points);
        let pattern_length = temp.len();
        temp.extend([0, 0]);
        Self {
            pattern: regex.to_string(),
            temp,
            pattern_length,
            cursor: 0,
            flags0: 0,
            capturing_group_count: 1,
            named_groups: HashMap::new(),
            bits: Vec::new(),
            group_depth: 0,
            class_depth: 0,
        }
    }

    fn has(&self, flag: u32) -> bool {
        self.flags0 & flag != 0
    }

    fn error(&self, description: impl Into<String>) -> PatternSyntaxError {
        PatternSyntaxError {
            description: description.into(),
            index: self.cursor as i64 - 1,
            pattern: self.pattern.clone(),
        }
    }

    /// The code point at `index`, `0` past the sentinels.
    fn at(&self, index: usize) -> u32 {
        self.temp.get(index).copied().unwrap_or(0)
    }

    /// The code point at the cursor, past whitespace and comments in comments
    /// mode.
    fn peek(&mut self) -> u32 {
        let c = self.at(self.cursor);
        if self.has(COMMENTS) {
            self.peek_past_whitespace(c)
        } else {
            c
        }
    }

    /// The code point at the cursor, past whitespace and comments in comments
    /// mode, consumed.
    fn read(&mut self) -> u32 {
        let c = self.peek();
        self.cursor += 1;
        c
    }

    /// Advances one code point and peeks.
    fn next(&mut self) -> u32 {
        self.cursor += 1;
        self.peek()
    }

    /// Advances one code point and returns the one there, whitespace included.
    fn next_escaped(&mut self) -> u32 {
        self.cursor += 1;
        self.at(self.cursor)
    }

    /// Returns the code point after the cursor, whitespace included, and moves
    /// past it.
    fn skip(&mut self) -> u32 {
        let c = self.at(self.cursor + 1);
        self.cursor += 2;
        c
    }

    fn unread(&mut self) {
        self.cursor -= 1;
    }

    fn peek_past_whitespace(&mut self, mut c: u32) -> u32 {
        while is_ascii_space(c) || c == ch('#') {
            while is_ascii_space(c) {
                self.cursor += 1;
                c = self.at(self.cursor);
            }
            if c == ch('#') {
                c = self.peek_past_line();
            }
        }
        c
    }

    /// Moves past a comment to the line separator that ends it, or to the end.
    fn peek_past_line(&mut self) -> u32 {
        self.cursor += 1;
        let mut c = self.at(self.cursor);
        while c != 0 && !self.is_line_separator(c) {
            self.cursor += 1;
            c = self.at(self.cursor);
        }
        if c == 0 && self.cursor > self.pattern_length {
            self.cursor = self.pattern_length;
            c = self.at(self.cursor);
        }
        c
    }

    fn is_line_separator(&self, c: u32) -> bool {
        if self.has(UNIX_LINES) {
            c == ch('\n')
        } else {
            c == ch('\n') || c == ch('\r') || c | 1 == 0x2029 || c == 0x85
        }
    }

    fn accept(&mut self, expected: char, description: &str) -> ParseResult<()> {
        if self.read() != ch(expected) {
            return Err(self.error(description));
        }
        Ok(())
    }

    fn compile(&mut self) -> ParseResult<Node> {
        let node = self.expr()?;
        if self.cursor != self.pattern_length {
            if self.peek() == ch(')') {
                return Err(self.error("Unmatched closing ')'"));
            }
            return Err(self.error("Unexpected internal error"));
        }
        Ok(node)
    }

    /// Alternatives separated by `|`.
    fn expr(&mut self) -> ParseResult<Node> {
        let mut branches = Vec::new();
        loop {
            branches.push(self.sequence()?);
            if self.peek() != ch('|') {
                break;
            }
            self.next();
        }
        Ok(if branches.len() == 1 {
            branches.pop().expect("one branch")
        } else {
            Node::Alternation(branches)
        })
    }

    /// The nodes of one alternative, each with its quantifier.
    fn sequence(&mut self) -> ParseResult<Node> {
        let mut nodes = Vec::new();
        loop {
            let c = self.peek();
            let node = match char::from_u32(c).unwrap_or('\u{FFFD}') {
                '(' => match self.group0()? {
                    Some(node) => node,
                    None => continue,
                },
                '[' => Node::Char(self.clazz(true)?),
                '\\' => {
                    let c = self.next_escaped();
                    if c == ch('p') || c == ch('P') {
                        Node::Char(self.property(c == ch('P'))?)
                    } else {
                        self.unread();
                        self.atom()?
                    }
                }
                '^' => {
                    self.next();
                    if self.has(MULTILINE) {
                        if self.has(UNIX_LINES) {
                            Node::Assertion(r"(?:\A|(?<=\n))(?!\z)".to_string())
                        } else {
                            Node::Assertion(
                                r"(?:\A|(?<=[\n\x{85}\x{2028}\x{2029}])|(?<=\r)(?!\n))(?!\z)"
                                    .to_string(),
                            )
                        }
                    } else {
                        Node::Assertion(r"\A".to_string())
                    }
                }
                '$' => {
                    self.next();
                    self.dollar(self.has(MULTILINE))
                }
                '.' => {
                    self.next();
                    Node::Char(if self.has(DOTALL) {
                        Pred::class(r"[\x{0}-\x{10FFFF}]")
                    } else if self.has(UNIX_LINES) {
                        Pred::chars("\n").not()
                    } else {
                        Pred::class(LINE_TERMINATORS).not()
                    })
                }
                '|' | ')' => break,
                ']' | '}' => self.atom()?,
                '?' | '*' | '+' => {
                    self.next();
                    return Err(self.error(format!(
                        "Dangling meta character '{}'",
                        char::from_u32(c).expect("ASCII")
                    )));
                }
                '\0' => {
                    if self.cursor >= self.pattern_length {
                        break;
                    }
                    self.atom()?
                }
                _ => self.atom()?,
            };
            let node = self.closure(node)?;
            nodes.push(node);
        }
        Ok(match nodes.len() {
            0 => Node::Empty,
            1 => nodes.pop().expect("one node"),
            _ => Node::Sequence(nodes),
        })
    }

    /// `$`, or `\Z` when not `multiline`: Java's `Dollar` and `UnixDollar`.
    fn dollar(&self, multiline: bool) -> Node {
        Node::Assertion(match (self.has(UNIX_LINES), multiline) {
            (true, true) => r"(?=\n|\z)".to_string(),
            (true, false) => r"(?=\n?\z)".to_string(),
            // Before any line terminator, or at the end; never between `\r`
            // and `\n`.
            (false, true) => format!(r"(?:\z|(?={LINE_TERMINATORS}))(?!(?<=\r)\n)"),
            // At the end, or before a line terminator, `\r\n` included, that
            // ends the input; never between `\r` and `\n`.
            (false, false) => {
                format!(r"(?=(?:\r\n|{LINE_TERMINATORS})?\z)(?!(?<=\r)\n)")
            }
        })
    }

    /// A run of literal characters, or the one escape at its start that is not
    /// a character.
    fn atom(&mut self) -> ParseResult<Node> {
        let mut buffer: Vec<u32> = Vec::new();
        let mut prev = 0;
        let mut c = self.peek();
        loop {
            match char::from_u32(c).unwrap_or('\u{FFFD}') {
                '*' | '+' | '?' | '{' => {
                    if buffer.len() > 1 {
                        // The quantifier applies to the last character only.
                        self.cursor = prev;
                        buffer.pop();
                    }
                    break;
                }
                '$' | '.' | '^' | '(' | '[' | '|' | ')' => break,
                '\\' => {
                    let escaped = self.next_escaped();
                    if escaped == ch('p') || escaped == ch('P') {
                        if !buffer.is_empty() {
                            self.unread();
                            break;
                        }
                        return Ok(Node::Char(self.property(escaped == ch('P'))?));
                    }
                    self.unread();
                    prev = self.cursor;
                    match self.escape(false, buffer.is_empty())? {
                        Escape::Char(value) => {
                            buffer.push(value);
                            c = self.peek();
                            continue;
                        }
                        Escape::Node(node) if buffer.is_empty() => return Ok(node),
                        Escape::Pred(pred) if buffer.is_empty() => return Ok(Node::Char(pred)),
                        _ => {
                            // Not a character: parsed again on its own.
                            self.cursor = prev;
                            break;
                        }
                    }
                }
                '\0' if self.cursor >= self.pattern_length => break,
                _ => {
                    prev = self.cursor;
                    buffer.push(c);
                    c = self.next();
                }
            }
        }
        Ok(match buffer.len() {
            0 => Node::Empty,
            1 => Node::Char(self.single(buffer[0])),
            _ => Node::Sequence(
                buffer
                    .iter()
                    .map(|&c| Node::Char(self.slice_char(c)))
                    .collect(),
            ),
        })
    }

    /// A lone literal: Java's `Single`, `SingleI` or `SingleU`.
    fn single(&self, c: u32) -> Pred {
        if self.has(CASE_INSENSITIVE) {
            if self.has(UNICODE_CASE) {
                let upper = to_upper(c);
                let lower = to_lower(upper);
                if upper != lower {
                    return Pred::Set(unicode_case_set(lower));
                }
            } else if c < 0x80 && is_ascii_alpha(c) {
                let mut set = CharSet::default();
                set.add(ascii_to_lower(c));
                set.add(ascii_to_upper(c));
                return Pred::Set(set);
            }
        }
        let mut set = CharSet::default();
        set.add(c);
        Pred::Set(set)
    }

    /// One character of a run of literals: Java's `SliceI` or `SliceU`.
    fn slice_char(&self, c: u32) -> Pred {
        let mut set = CharSet::default();
        if self.has(CASE_INSENSITIVE) {
            if self.has(UNICODE_CASE) {
                return Pred::Set(unicode_case_set(to_lower(to_upper(c))));
            }
            set.add(ascii_to_lower(c));
            set.add(ascii_to_upper(c));
        }
        set.add(c);
        Pred::Set(set)
    }

    /// A quantifier after `node`, if any.
    fn closure(&mut self, node: Node) -> ParseResult<Node> {
        let c = self.peek();
        let (min, max) = match char::from_u32(c).unwrap_or('\u{FFFD}') {
            '?' => (0, 1),
            '*' => (0, MAX_REPS),
            '+' => (1, MAX_REPS),
            '{' => {
                let first = self.at(self.cursor + 1);
                if !is_ascii_digit(first) {
                    return Err(self.error("Illegal repetition"));
                }
                self.skip();
                let mut digit = first;
                let mut cmin: u64 = 0;
                loop {
                    cmin = cmin * 10 + u64::from(digit - ch('0'));
                    if cmin > u64::from(MAX_REPS) {
                        return Err(self.error("Illegal repetition range"));
                    }
                    digit = self.read();
                    if !is_ascii_digit(digit) {
                        break;
                    }
                }
                let mut cmax = cmin;
                let mut c = digit;
                if c == ch(',') {
                    c = self.read();
                    cmax = u64::from(MAX_REPS);
                    if c != ch('}') {
                        cmax = 0;
                        while is_ascii_digit(c) {
                            cmax = cmax * 10 + u64::from(c - ch('0'));
                            if cmax > u64::from(MAX_REPS) {
                                return Err(self.error("Illegal repetition range"));
                            }
                            c = self.read();
                        }
                    }
                }
                if c != ch('}') {
                    return Err(self.error("Unclosed counted closure"));
                }
                if cmax < cmin {
                    return Err(self.error("Illegal repetition range"));
                }
                self.unread();
                (cmin as u32, cmax as u32)
            }
            _ => return Ok(node),
        };
        let c = self.next();
        let greed = if c == ch('?') {
            self.next();
            Greed::Lazy
        } else if c == ch('+') {
            self.next();
            Greed::Possessive
        } else {
            Greed::Greedy
        };
        Ok(Node::Repeat {
            body: Box::new(node),
            min,
            max,
            greed,
        })
    }

    /// A parenthesised group, or `None` for a group that only sets flags.
    fn group0(&mut self) -> ParseResult<Option<Node>> {
        self.group_depth += 1;
        if self.group_depth >= MAX_GROUP_DEPTH {
            return Err(self.error(TOO_DEEP));
        }
        let group = self.group_body();
        self.group_depth -= 1;
        group
    }

    fn group_body(&mut self) -> ParseResult<Option<Node>> {
        let save = self.flags0;
        let mut c = self.next();
        let kind = if c == ch('?') {
            c = self.next();
            match char::from_u32(c).unwrap_or('\u{FFFD}') {
                ':' => {
                    self.next();
                    GroupKind::NonCapturing
                }
                '=' => {
                    self.next();
                    GroupKind::LookAhead
                }
                '!' => {
                    self.next();
                    GroupKind::NegativeLookAhead
                }
                '>' => {
                    self.next();
                    GroupKind::Atomic
                }
                '<' => {
                    c = self.next();
                    if c == ch('=') {
                        self.next();
                        GroupKind::LookBehind
                    } else if c == ch('!') {
                        self.next();
                        GroupKind::NegativeLookBehind
                    } else {
                        let name = self.group_name()?;
                        if self.named_groups.contains_key(&name) {
                            return Err(self.error(format!(
                                "Named capturing group <{name}> is already defined"
                            )));
                        }
                        self.named_groups
                            .insert(name.clone(), self.capturing_group_count);
                        self.capturing_group_count += 1;
                        GroupKind::Named(name)
                    }
                }
                _ => {
                    self.add_flags();
                    let c = self.read();
                    if c == ch(')') {
                        // Flags alone: they hold until the enclosing group ends.
                        return Ok(None);
                    }
                    if c != ch(':') {
                        return Err(self.error("Unknown inline modifier"));
                    }
                    GroupKind::NonCapturing
                }
            }
        } else {
            self.capturing_group_count += 1;
            GroupKind::Capturing
        };
        let body = self.expr()?;
        self.accept(')', "Unclosed group")?;
        if matches!(kind, GroupKind::LookBehind | GroupKind::NegativeLookBehind)
            && !has_obvious_maximum_length(&body)
        {
            return Err(self.error("Look-behind group does not have an obvious maximum length"));
        }
        self.flags0 = save;
        Ok(Some(Node::Group(kind, Box::new(body))))
    }

    /// A group name, `[a-zA-Z][a-zA-Z0-9]*`, and the `>` after it.
    fn group_name(&mut self) -> ParseResult<String> {
        let first = self.read();
        if !is_ascii_alpha(first) {
            return Err(self.error("capturing group name does not start with a Latin letter"));
        }
        let mut name = String::new();
        name.push(char::from_u32(first).expect("ASCII"));
        let mut c = self.read();
        while is_ascii_alnum(c) {
            name.push(char::from_u32(c).expect("ASCII"));
            c = self.read();
        }
        if c != ch('>') {
            return Err(self.error("named capturing group is missing trailing '>'"));
        }
        Ok(name)
    }

    /// The flags of `(?idmsuxcU-idmsuxcU)` and `(?idmsuxcU-idmsuxcU:`.
    fn add_flags(&mut self) {
        let mut c = self.peek();
        loop {
            match char::from_u32(c).unwrap_or('\u{FFFD}') {
                '-' => {
                    self.sub_flags();
                    return;
                }
                flag => match flag_bits(flag) {
                    Some(bits) => self.flags0 |= bits,
                    None => return,
                },
            }
            c = self.next();
        }
    }

    fn sub_flags(&mut self) {
        let mut c = self.next();
        loop {
            match flag_bits(char::from_u32(c).unwrap_or('\u{FFFD}')) {
                Some(bits) => self.flags0 &= !bits,
                None => return,
            }
            c = self.next();
        }
    }

    /// An escape, the backslash at the cursor.
    fn escape(&mut self, in_class: bool, create: bool) -> ParseResult<Escape> {
        let c = self.skip();
        let unicode = self.has(UNICODE_CHARACTER_CLASS);
        let class = |pred: Pred| Ok(Escape::Pred(pred));
        let unsupported = |parser: &Self| Err(parser.error("Illegal/unsupported escape sequence"));
        match char::from_u32(c).unwrap_or('\u{FFFD}') {
            '0' => return self.octal().map(Escape::Char),
            '1'..='9' => {
                if in_class {
                    return unsupported(self);
                }
                if create {
                    return Ok(Escape::Node(self.reference(c - ch('0'))));
                }
                return Ok(Escape::NotChar);
            }
            'A' if !in_class => return Ok(Escape::Node(Node::Assertion(r"\A".to_string()))),
            'B' if !in_class => return Ok(Escape::Node(self.boundary(false))),
            'D' => return class(digit(unicode).not()),
            'G' if !in_class => return Ok(Escape::Node(Node::Assertion(r"\G".to_string()))),
            'H' => return class(horizontal_space().not()),
            'N' => return self.named_character().map(Escape::Char),
            'R' if !in_class => return Ok(Escape::Node(Node::LineEnding)),
            'S' => return class(space(unicode).not()),
            'V' => return class(vertical_space().not()),
            'W' => return class(word(unicode).not()),
            'X' if !in_class => return Ok(Escape::Node(Node::Grapheme)),
            'Z' if !in_class => return Ok(Escape::Node(self.dollar(false))),
            'a' => return Ok(Escape::Char(0x07)),
            'b' if !in_class => {
                if self.peek() == ch('{') {
                    let after_brace = self.cursor;
                    if self.skip() == ch('g') {
                        if self.read() == ch('}') {
                            return Ok(Escape::Node(Node::Assertion(grapheme_boundary())));
                        }
                        return unsupported(self);
                    }
                    self.cursor = after_brace;
                }
                return Ok(Escape::Node(self.boundary(true)));
            }
            'c' => return self.control().map(Escape::Char),
            'd' => return class(digit(unicode)),
            'e' => return Ok(Escape::Char(0x1B)),
            'f' => return Ok(Escape::Char(0x0C)),
            'h' => return class(horizontal_space()),
            'k' if !in_class => {
                if self.read() != ch('<') {
                    return Err(self.error("\\k is not followed by '<' for named capturing group"));
                }
                let name = self.group_name()?;
                let Some(&group) = self.named_groups.get(&name) else {
                    return Err(
                        self.error(format!("named capturing group <{name}> does not exist"))
                    );
                };
                return Ok(Escape::Node(Node::BackRef {
                    group,
                    casei: self.has(CASE_INSENSITIVE),
                }));
            }
            'n' => return Ok(Escape::Char(0x0A)),
            'r' => return Ok(Escape::Char(0x0D)),
            's' => return class(space(unicode)),
            't' => return Ok(Escape::Char(0x09)),
            'u' => return self.unicode_escape().map(Escape::Char),
            'v' => return class(vertical_space()),
            'w' => return class(word(unicode)),
            'x' => return self.hex_escape().map(Escape::Char),
            'z' if !in_class => return Ok(Escape::Node(Node::Assertion(r"\z".to_string()))),
            _ => {}
        }
        if c < 0x80 && is_ascii_alpha(c) {
            return unsupported(self);
        }
        Ok(Escape::Char(c))
    }

    /// `\0n`, `\0nn` or `\0mnn` (`m` at most 3).
    fn octal(&mut self) -> ParseResult<u32> {
        let is_octal = |c: u32| (ch('0')..=ch('7')).contains(&c);
        let n = self.read();
        if is_octal(n) {
            let m = self.read();
            if is_octal(m) {
                let o = self.read();
                if is_octal(o) && n - ch('0') < 4 {
                    return Ok((n - ch('0')) * 64 + (m - ch('0')) * 8 + (o - ch('0')));
                }
                self.unread();
                return Ok((n - ch('0')) * 8 + (m - ch('0')));
            }
            self.unread();
            return Ok(n - ch('0'));
        }
        Err(self.error("Illegal octal escape sequence"))
    }

    /// `\xhh` or `\x{h…h}`.
    fn hex_escape(&mut self) -> ParseResult<u32> {
        let n = self.read();
        if is_hex_digit(n) {
            let m = self.read();
            if is_hex_digit(m) {
                return Ok(hex_value(n) * 16 + hex_value(m));
            }
        } else if n == ch('{') && is_hex_digit(self.peek()) {
            let mut value: u32 = 0;
            loop {
                let c = self.read();
                if !is_hex_digit(c) {
                    if c != ch('}') {
                        return Err(self.error("Unclosed hexadecimal escape sequence"));
                    }
                    return Ok(value);
                }
                value = value * 16 + hex_value(c);
                if value > 0x10FFFF {
                    return Err(self.error("Hexadecimal codepoint is too big"));
                }
            }
        }
        Err(self.error("Illegal hexadecimal escape sequence"))
    }

    /// `\uhhhh`, two of which joined when they spell a surrogate pair.
    fn unicode_escape(&mut self) -> ParseResult<u32> {
        let n = self.uxxxx()?;
        if is_high_surrogate(n) {
            let resume = self.cursor;
            if self.read() == ch('\\') && self.read() == ch('u') {
                let low = self.uxxxx()?;
                if is_low_surrogate(low) {
                    return Ok(0x10000 + ((n - 0xD800) << 10) + (low - 0xDC00));
                }
            }
            self.cursor = resume;
        }
        Ok(n)
    }

    fn uxxxx(&mut self) -> ParseResult<u32> {
        let mut value = 0;
        for _ in 0..4 {
            let c = self.read();
            if !is_hex_digit(c) {
                return Err(self.error("Illegal Unicode escape sequence"));
            }
            value = value * 16 + hex_value(c);
        }
        Ok(value)
    }

    /// `\cX`: the code point of `X` with bit 6 flipped.
    fn control(&mut self) -> ParseResult<u32> {
        if self.cursor < self.pattern_length {
            return Ok(self.read() ^ 64);
        }
        Err(self.error("Illegal control escape sequence"))
    }

    /// `\N{name}`: the code point `Character.codePointOf` finds for the
    /// name between the braces, as written.
    fn named_character(&mut self) -> ParseResult<u32> {
        if self.read() != ch('{') {
            return Err(self.error("Illegal character name escape sequence"));
        }
        let start = self.cursor;
        loop {
            let c = self.read();
            if c == ch('}') {
                break;
            }
            if self.cursor > self.pattern_length {
                return Err(self.error("Unclosed character name escape sequence"));
            }
        }
        let name: String = self.temp[start..self.cursor - 1]
            .iter()
            .filter_map(|&c| char::from_u32(c))
            .collect();
        code_point_of(&name).ok_or_else(|| self.error(format!("Unknown character name [{name}]")))
    }

    /// `\1`…`\9` and the digits after them that still name an opened group.
    fn reference(&mut self, mut group: u32) -> Node {
        loop {
            let c = self.peek();
            if !is_ascii_digit(c) {
                break;
            }
            let extended = group * 10 + (c - ch('0'));
            if (self.capturing_group_count as u32) - 1 < extended {
                break;
            }
            group = extended;
            self.read();
        }
        Node::BackRef {
            group: group as usize,
            casei: self.has(CASE_INSENSITIVE),
        }
    }

    /// `\b` (`both`) or `\B`: Java's `Bound`.
    ///
    /// A position is a boundary where exactly one of the characters around it
    /// is a word character: `\w` (Unicode's under `UNICODE_CHARACTER_CLASS`),
    /// or a non-spacing mark that ends a run of non-spacing marks after a
    /// letter or digit. Java walks back over that run one UTF-16 unit at a
    /// time and stops at a surrogate, so the base and the marks before the
    /// last one must lie in the Basic Multilingual Plane.
    fn boundary(&self, both: bool) -> Node {
        let word = Emitter::class_of(&word(self.has(UNICODE_CHARACTER_CLASS)), &self.bits);
        let base = r"[[\p{gc=L}\p{gc=Nd}]&&[\x{0}-\x{FFFF}]]";
        let mark = r"[\p{gc=Mn}&&[\x{0}-\x{FFFF}]]";
        let left = format!(r"(?<={word}|{base}{mark}+)");
        let right = format!(r"(?={word}|\p{{gc=Mn}}(?<={base}{mark}*\p{{gc=Mn}}))");
        Node::Assertion(if both {
            format!("(?:{left}(?!{right})|(?!{left}){right})")
        } else {
            format!("(?:{left}{right}|(?!{left})(?!{right}))")
        })
    }

    /// `\p` or `\P` and the property after it, the cursor on the `p`.
    fn property(&mut self, complement: bool) -> ParseResult<Pred> {
        let c = self.next();
        let name: String = if c == ch('{') {
            self.next();
            let start = self.cursor;
            loop {
                let c = self.read();
                if c == ch('}') {
                    break;
                }
                if self.cursor > self.pattern_length {
                    return Err(self.error("Unclosed character family"));
                }
            }
            if start + 1 >= self.cursor {
                return Err(self.error("Empty character family"));
            }
            self.temp[start..self.cursor - 1]
                .iter()
                .filter_map(|&c| char::from_u32(c))
                .collect()
        } else {
            self.read();
            char::from_u32(c).map(String::from).unwrap_or_default()
        };
        let pred = self.family(&name)?;
        Ok(if complement { pred.not() } else { pred })
    }

    /// The predicate of a property name, Java's `family`.
    fn family(&self, name: &str) -> ParseResult<Pred> {
        let casei = self.has(CASE_INSENSITIVE);
        if let Some(equals) = name.find('=') {
            let value = &name[equals + 1..];
            let key = name[..equals].to_lowercase();
            let pred = match key.as_str() {
                "sc" | "script" => for_unicode_script(value),
                "blk" | "block" => for_unicode_block(value),
                "gc" | "general_category" => for_property(value, casei),
                _ => None,
            };
            return pred.ok_or_else(|| {
                self.error(format!(
                    "Unknown Unicode property {{name=<{key}>, value=<{value}>}}"
                ))
            });
        }
        let pred = if let Some(block) = name.strip_prefix("In") {
            for_unicode_block(block)
        } else if let Some(short) = name.strip_prefix("Is") {
            for_unicode_property(short, casei)
                .or_else(|| for_property(short, casei))
                .or_else(|| for_unicode_script(short))
        } else {
            let posix = if self.has(UNICODE_CHARACTER_CLASS) {
                for_posix_name(name, casei)
            } else {
                None
            };
            posix.or_else(|| for_property(name, casei))
        };
        pred.ok_or_else(|| self.error(format!("Unknown character property name {{{name}}}")))
    }

    /// A character class, the cursor on its `[`: Java's `clazz`.
    fn clazz(&mut self, consume: bool) -> ParseResult<Pred> {
        self.class_depth += 1;
        if self.class_depth >= MAX_CLASS_DEPTH {
            return Err(self.error(TOO_DEEP));
        }
        let class = self.class_body(consume);
        self.class_depth -= 1;
        class
    }

    fn class_body(&mut self, consume: bool) -> ParseResult<Pred> {
        let mut prev: Option<Pred> = None;
        let mut curr: Option<Pred> = None;
        let bits = self.bits.len();
        self.bits.push([false; 256]);
        let mut is_neg = false;
        let mut has_bits = false;
        let mut c = self.next();
        if c == ch('^') && self.at(self.cursor - 1) == ch('[') {
            c = self.next();
            is_neg = true;
        }
        loop {
            match char::from_u32(c).unwrap_or('\u{FFFD}') {
                '[' => {
                    let nested = self.clazz(true)?;
                    curr = Some(nested.clone());
                    prev = Some(match prev {
                        None => nested,
                        Some(p) => p.union(nested),
                    });
                    c = self.peek();
                    continue;
                }
                '&' => {
                    c = self.next();
                    if c == ch('&') {
                        c = self.next();
                        let mut right: Option<Pred> = None;
                        while c != ch(']') && c != ch('&') {
                            let operand = if c == ch('[') {
                                self.clazz(true)?
                            } else {
                                self.unread();
                                self.clazz(false)?
                            };
                            right = Some(match right {
                                None => operand,
                                Some(r) => r.union(operand),
                            });
                            c = self.peek();
                        }
                        if has_bits {
                            match prev.take() {
                                None => {
                                    prev = Some(Pred::Bits(bits));
                                    curr = Some(Pred::Bits(bits));
                                }
                                Some(p) => prev = Some(p.union(Pred::Bits(bits))),
                            }
                            has_bits = false;
                        }
                        if let Some(r) = &right {
                            curr = Some(r.clone());
                        }
                        match prev.take() {
                            None => match right {
                                None => return Err(self.error("Bad class syntax")),
                                Some(r) => prev = Some(r),
                            },
                            Some(p) => match &curr {
                                None => return Err(self.error("Bad intersection syntax")),
                                Some(cur) => prev = Some(p.and(cur.clone())),
                            },
                        }
                        continue;
                    }
                    // A literal `&`.
                    self.unread();
                }
                '\0' if self.cursor >= self.pattern_length => {
                    return Err(self.error("Unclosed character class"));
                }
                ']' if prev.is_some() || has_bits => {
                    if consume {
                        self.next();
                    }
                    let result = match prev {
                        None => Pred::Bits(bits),
                        Some(p) if has_bits => p.union(Pred::Bits(bits)),
                        Some(p) => p,
                    };
                    return Ok(if is_neg { result.not() } else { result });
                }
                // A `]` first in the class, and any other character, is a
                // member.
                _ => {}
            }
            curr = self.range(bits)?;
            match &curr {
                None => has_bits = true,
                Some(cur) => {
                    prev = Some(match prev {
                        None => cur.clone(),
                        Some(p) => p.union(cur.clone()),
                    });
                }
            }
            c = self.peek();
        }
    }

    /// One member of a class: a character, a range, an escape or a property;
    /// `None` when the character went into the class's `bits`.
    fn range(&mut self, bits: usize) -> ParseResult<Option<Pred>> {
        let mut c = self.peek();
        if c == ch('\\') {
            c = self.next_escaped();
            if c == ch('p') || c == ch('P') {
                return self.property(c == ch('P')).map(Some);
            }
            self.unread();
            match self.escape(true, true)? {
                Escape::Char(value) => c = value,
                Escape::Pred(pred) => return Ok(Some(pred)),
                Escape::Node(_) | Escape::NotChar => {
                    return Err(self.error("Illegal/unsupported escape sequence"))
                }
            }
        } else {
            self.next();
        }
        if self.peek() == ch('-') {
            let end_range = self.at(self.cursor + 1);
            if end_range == ch('[') {
                return Ok(self.bits_or_single(bits, c));
            }
            if end_range != ch(']') {
                self.next();
                let mut m = self.peek();
                if m == ch('\\') {
                    m = match self.escape(true, false)? {
                        Escape::Char(value) => value,
                        _ => return Err(self.error("Illegal character range")),
                    };
                } else {
                    self.next();
                }
                if m < c {
                    return Err(self.error("Illegal character range"));
                }
                return Ok(Some(self.char_range(c, m)));
            }
        }
        Ok(self.bits_or_single(bits, c))
    }

    /// Adds `c` to the class's `bits` when it is below U+0100 and its case
    /// partners are too, else returns its own predicate.
    fn bits_or_single(&mut self, bits: usize, c: u32) -> Option<Pred> {
        let unicode_casei = self.has(CASE_INSENSITIVE) && self.has(UNICODE_CASE);
        let partner_outside = matches!(
            c,
            0xFF | 0xB5 | 0x49 | 0x69 | 0x53 | 0x73 | 0x4B | 0x6B | 0xC5 | 0xE5
        );
        if c < 256 && !(unicode_casei && partner_outside) {
            let mut members = vec![c];
            if self.has(CASE_INSENSITIVE) {
                if c < 0x80 {
                    members.extend([ascii_to_upper(c), ascii_to_lower(c)]);
                } else if self.has(UNICODE_CASE) {
                    members.extend([to_lower(c), to_upper(c)]);
                }
            }
            for member in members {
                // The partners of the characters below U+0100 not excluded
                // above are below U+0100 too.
                if let Some(slot) = self.bits[bits].get_mut(member as usize) {
                    *slot = true;
                }
            }
            return None;
        }
        Some(self.single(c))
    }

    /// The range `lo-hi` of a class: Java's `Range`, `CIRange` or `CIRangeU`.
    fn char_range(&self, lo: u32, hi: u32) -> Pred {
        let mut set = CharSet::default();
        set.add_range(lo, hi);
        if self.has(CASE_INSENSITIVE) {
            let in_range = |c: u32| (lo..=hi).contains(&c);
            if self.has(UNICODE_CASE) {
                for row in tables::CASE_MAPPINGS.as_chunks::<3>().0 {
                    let upper = row[1];
                    if in_range(upper) || in_range(to_lower(upper)) {
                        set.add(row[0]);
                    }
                }
            } else {
                for c in (0..0x80).filter(|&c| is_ascii_alpha(c)) {
                    if in_range(ascii_to_upper(c)) || in_range(ascii_to_lower(c)) {
                        set.add(c);
                    }
                }
            }
        }
        Pred::Set(set)
    }
}

/// Whether Java's `TreeInfo` finds a node deterministic: no alternation and
/// no repetition of variable count, look-arounds aside, which Java does not
/// study.
fn is_deterministic(node: &Node) -> bool {
    match node {
        Node::Alternation(_) => false,
        Node::Repeat { body, min, max, .. } => min == max && is_deterministic(body),
        Node::Sequence(nodes) => nodes.iter().all(is_deterministic),
        Node::Group(
            GroupKind::LookAhead
            | GroupKind::NegativeLookAhead
            | GroupKind::LookBehind
            | GroupKind::NegativeLookBehind,
            _,
        ) => true,
        Node::Group(_, body) => is_deterministic(body),
        Node::Empty
        | Node::Char(_)
        | Node::BackRef { .. }
        | Node::LineEnding
        | Node::Grapheme
        | Node::Assertion(_) => true,
    }
}

/// Whether Java bounds the length of a look-behind's body.
///
/// Java refuses a look-behind that contains, outside a nested look-around, a
/// back reference, or a group repeated by anything but `?` and not
/// possessively whose body has alternatives or a repetition of variable
/// count, or whose longest match times the repetition's maximum exceeds
/// `MAX_REPS` (an unbounded repetition of a body longer than one character,
/// a body with `\X`): the cases observed on the JDK, which repeats such a
/// group with its `Loop` node or cannot bound the group's length.
fn has_obvious_maximum_length(node: &Node) -> bool {
    match node {
        Node::BackRef { .. } => false,
        Node::Group(
            GroupKind::LookAhead
            | GroupKind::NegativeLookAhead
            | GroupKind::LookBehind
            | GroupKind::NegativeLookBehind,
            _,
        ) => true,
        Node::Group(_, body) => has_obvious_maximum_length(body),
        Node::Sequence(nodes) | Node::Alternation(nodes) => {
            nodes.iter().all(has_obvious_maximum_length)
        }
        Node::Repeat {
            body,
            min,
            max,
            greed,
        } => {
            let group_loop = matches!(
                **body,
                Node::Group(
                    GroupKind::Capturing | GroupKind::Named(_) | GroupKind::NonCapturing,
                    _
                )
            ) && !(*min == 0 && *max == 1)
                && *greed != Greed::Possessive
                && (!is_deterministic(body)
                    || max_length(body)
                        .is_none_or(|length| length * u64::from(*max) > u64::from(MAX_REPS)));
            !group_loop && has_obvious_maximum_length(body)
        }
        Node::Empty | Node::Char(_) | Node::LineEnding | Node::Grapheme | Node::Assertion(_) => {
            true
        }
    }
}

/// The longest match of a node, in characters, as Java's `TreeInfo` bounds
/// it; `None` when unbounded.
fn max_length(node: &Node) -> Option<u64> {
    match node {
        Node::Empty | Node::Assertion(_) => Some(0),
        Node::Char(_) => Some(1),
        Node::LineEnding => Some(2),
        Node::Grapheme | Node::BackRef { .. } => None,
        Node::Group(
            GroupKind::LookAhead
            | GroupKind::NegativeLookAhead
            | GroupKind::LookBehind
            | GroupKind::NegativeLookBehind,
            _,
        ) => Some(0),
        Node::Group(_, body) => max_length(body),
        Node::Sequence(nodes) => nodes
            .iter()
            .try_fold(0u64, |total, n| Some(total.saturating_add(max_length(n)?))),
        Node::Alternation(nodes) => nodes
            .iter()
            .try_fold(0u64, |longest, n| Some(longest.max(max_length(n)?))),
        Node::Repeat { body, max, .. } => {
            max_length(body).map(|length| length.saturating_mul(u64::from(*max)))
        }
    }
}

/// The shortest match of a node, in characters: `0` when it can match the
/// empty string.
fn min_length(node: &Node) -> u64 {
    match node {
        Node::Empty | Node::Assertion(_) | Node::BackRef { .. } => 0,
        Node::Char(_) | Node::LineEnding | Node::Grapheme => 1,
        Node::Group(
            GroupKind::LookAhead
            | GroupKind::NegativeLookAhead
            | GroupKind::LookBehind
            | GroupKind::NegativeLookBehind,
            _,
        ) => 0,
        Node::Group(_, body) => min_length(body),
        Node::Sequence(nodes) => nodes
            .iter()
            .fold(0u64, |total, n| total.saturating_add(min_length(n))),
        Node::Alternation(nodes) => nodes.iter().map(min_length).min().unwrap_or(0),
        Node::Repeat { body, min, .. } => min_length(body).saturating_mul(u64::from(*min)),
    }
}

/// Whether a counted repetition of a body that can match the empty string,
/// without only matching it, occurs anywhere in `node`, look-arounds
/// included (see [`Translation::bounded_empty_loop`]): every repetition but
/// `?`, `*`, `+` and their spellings `{0,1}`, `{1}`, `{0,}` and `{1,}`.
fn has_bounded_empty_loop(node: &Node) -> bool {
    match node {
        Node::Repeat { body, min, max, .. } => {
            let counted = !(*max <= 1 || (*max == MAX_REPS && *min <= 1));
            (counted && !body.is_zero_width() && min_length(body) == 0)
                || has_bounded_empty_loop(body)
        }
        Node::Group(_, body) => has_bounded_empty_loop(body),
        Node::Sequence(nodes) | Node::Alternation(nodes) => {
            nodes.iter().any(has_bounded_empty_loop)
        }
        Node::Empty
        | Node::Char(_)
        | Node::BackRef { .. }
        | Node::LineEnding
        | Node::Grapheme
        | Node::Assertion(_) => false,
    }
}

/// Whether `node`, or a node inside it outside look-arounds, satisfies `test`.
fn contains(node: &Node, test: &dyn Fn(&Node) -> bool) -> bool {
    test(node)
        || match node {
            Node::Group(
                GroupKind::LookAhead
                | GroupKind::NegativeLookAhead
                | GroupKind::LookBehind
                | GroupKind::NegativeLookBehind,
                _,
            ) => false,
            Node::Group(_, body) | Node::Repeat { body, .. } => contains(body, test),
            Node::Sequence(nodes) | Node::Alternation(nodes) => {
                nodes.iter().any(|n| contains(n, test))
            }
            _ => false,
        }
}

/// What an escape stands for.
enum Escape {
    Char(u32),
    Pred(Pred),
    Node(Node),
    /// An escape that is not a character, met where only characters are
    /// collected; it is parsed again on its own.
    NotChar,
}

/// The flag bits an inline flag letter sets or clears.
fn flag_bits(flag: char) -> Option<u32> {
    Some(match flag {
        'i' => CASE_INSENSITIVE,
        'm' => MULTILINE,
        's' => DOTALL,
        'd' => UNIX_LINES,
        'u' => UNICODE_CASE,
        'c' => CANON_EQ,
        'x' => COMMENTS,
        'U' => UNICODE_CHARACTER_CLASS | UNICODE_CASE,
        _ => return None,
    })
}

/// Removes `\Q…\E` quoting: every quoted letter stays as it is, a quoted
/// digit becomes a hexadecimal escape and any other quoted character is
/// preceded by a backslash, so the parser reads each as a literal.
fn remove_qe_quoting(src: &[u32]) -> Vec<u32> {
    let len = src.len();
    let mut i = 0;
    while i + 1 < len {
        if src[i] == ch('\\') {
            if src[i + 1] == ch('Q') {
                break;
            }
            i += 2;
        } else {
            i += 1;
        }
    }
    if i + 1 >= len {
        return src.to_vec();
    }
    let mut out = src[..i].to_vec();
    i += 2;
    let mut in_quote = true;
    while i < len {
        let c = src[i];
        i += 1;
        if in_quote {
            if c == ch('\\') && i < len && src[i] == ch('E') {
                i += 1;
                in_quote = false;
            } else if is_ascii_alpha(c) {
                out.push(c);
            } else if is_ascii_digit(c) {
                out.extend([ch('\\'), ch('x'), ch('3'), c]);
            } else {
                out.extend([ch('\\'), c]);
            }
        } else if c == ch('\\') {
            if i < len && src[i] == ch('Q') {
                i += 1;
                in_quote = true;
            } else {
                out.push(c);
                if i < len {
                    out.push(src[i]);
                    i += 1;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

// ---------------------------------------------------------------------------
// The predefined classes and the properties
// ---------------------------------------------------------------------------

/// `\d`: ASCII digits, or `\p{IsDigit}` under `UNICODE_CHARACTER_CLASS`.
fn digit(unicode: bool) -> Pred {
    if unicode {
        Pred::class(r"[\p{gc=Nd}]")
    } else {
        Pred::ranges(&[(0x30, 0x39)])
    }
}

/// `\s`: `[ \t\n\x0B\f\r]`, or `\p{IsWhite_Space}` under
/// `UNICODE_CHARACTER_CLASS`.
fn space(unicode: bool) -> Pred {
    if unicode {
        Pred::class(r"[\p{White_Space}]")
    } else {
        Pred::chars(" \t\n\u{b}\u{c}\r")
    }
}

/// `\w`: `[a-zA-Z_0-9]`, or Java's Unicode word characters under
/// `UNICODE_CHARACTER_CLASS`.
fn word(unicode: bool) -> Pred {
    if unicode {
        Pred::class(
            r"[\p{Alphabetic}\p{gc=Mn}\p{gc=Me}\p{gc=Mc}\p{gc=Nd}\p{gc=Pc}\p{Join_Control}]",
        )
    } else {
        Pred::ranges(&[(0x30, 0x39), (0x41, 0x5A), (0x5F, 0x5F), (0x61, 0x7A)])
    }
}

/// `\h`, horizontal whitespace.
fn horizontal_space() -> Pred {
    Pred::ranges(&[
        (0x09, 0x09),
        (0x20, 0x20),
        (0xA0, 0xA0),
        (0x1680, 0x1680),
        (0x180E, 0x180E),
        (0x2000, 0x200A),
        (0x202F, 0x202F),
        (0x205F, 0x205F),
        (0x3000, 0x3000),
    ])
}

/// `\v`, vertical whitespace.
fn vertical_space() -> Pred {
    Pred::ranges(&[(0x0A, 0x0D), (0x85, 0x85), (0x2028, 0x2029)])
}

/// `Character.isIdentifierIgnorable`.
fn identifier_ignorable() -> Pred {
    Pred::class(r"[\x{0}-\x{8}\x{E}-\x{1B}\x{7F}-\x{9F}\p{gc=Cf}]")
}

/// The general categories, the POSIX classes, `all` and the
/// `java.lang.Character` classes, with Java's names: `CharPredicates.forProperty`.
fn for_property(name: &str, casei: bool) -> Option<Pred> {
    const CATEGORIES: &[&str] = &[
        "Cn", "Lu", "Ll", "Lt", "Lm", "Lo", "Mn", "Me", "Mc", "Nd", "Nl", "No", "Zs", "Zl", "Zp",
        "Cc", "Cf", "Co", "Cs", "Pd", "Ps", "Pe", "Pc", "Po", "Sm", "Sc", "Sk", "So", "Pi", "Pf",
        "L", "M", "N", "Z", "C", "P", "S", "LC",
    ];
    if CATEGORIES.contains(&name) {
        if casei && matches!(name, "Lu" | "Ll" | "Lt") {
            return Some(Pred::class(r"[\p{gc=Lu}\p{gc=Ll}\p{gc=Lt}]"));
        }
        if name == "Cs" {
            // The surrogates, which no Rust string holds.
            return Some(Pred::Set(CharSet::default()));
        }
        return Some(Pred::Class(format!(r"[\p{{gc={name}}}]")));
    }
    Some(match name {
        "LD" => Pred::class(r"[\p{gc=L}\p{gc=Nd}]"),
        "L1" => Pred::ranges(&[(0, 0xFF)]),
        "all" => Pred::ranges(&[(0, 0x10FFFF)]),
        "ASCII" => Pred::ranges(&[(0, 0x7F)]),
        "Alnum" => Pred::ranges(&[(0x30, 0x39), (0x41, 0x5A), (0x61, 0x7A)]),
        "Alpha" => Pred::ranges(&[(0x41, 0x5A), (0x61, 0x7A)]),
        "Blank" => Pred::chars(" \t"),
        "Cntrl" => Pred::ranges(&[(0, 0x1F), (0x7F, 0x7F)]),
        "Digit" => Pred::ranges(&[(0x30, 0x39)]),
        "Graph" => Pred::ranges(&[(0x21, 0x7E)]),
        "Lower" if casei => Pred::ranges(&[(0x41, 0x5A), (0x61, 0x7A)]),
        "Lower" => Pred::ranges(&[(0x61, 0x7A)]),
        "Print" => Pred::ranges(&[(0x20, 0x7E)]),
        "Punct" => Pred::chars("!\"#$%&'()*+,-./:;<=>?@[\\]^_`{|}~"),
        "Space" => Pred::chars(" \t\n\u{b}\u{c}\r"),
        "Upper" if casei => Pred::ranges(&[(0x41, 0x5A), (0x61, 0x7A)]),
        "Upper" => Pred::ranges(&[(0x41, 0x5A)]),
        "XDigit" => Pred::ranges(&[(0x30, 0x39), (0x41, 0x46), (0x61, 0x66)]),
        "javaLowerCase" if casei => Pred::class(r"[\p{Lowercase}\p{Uppercase}\p{gc=Lt}]"),
        "javaLowerCase" => Pred::class(r"[\p{Lowercase}]"),
        "javaUpperCase" if casei => Pred::class(r"[\p{Lowercase}\p{Uppercase}\p{gc=Lt}]"),
        "javaUpperCase" => Pred::class(r"[\p{Uppercase}]"),
        "javaAlphabetic" => Pred::class(r"[\p{Alphabetic}]"),
        "javaIdeographic" => Pred::class(r"[\p{Ideographic}]"),
        "javaTitleCase" if casei => Pred::class(r"[\p{Lowercase}\p{Uppercase}\p{gc=Lt}]"),
        "javaTitleCase" => Pred::class(r"[\p{gc=Lt}]"),
        "javaDigit" => Pred::class(r"[\p{gc=Nd}]"),
        "javaDefined" => Pred::class(r"[\p{gc=Cn}]").not(),
        "javaLetter" => Pred::class(r"[\p{gc=L}]"),
        "javaLetterOrDigit" => Pred::class(r"[\p{gc=L}\p{gc=Nd}]"),
        "javaJavaIdentifierStart" => Pred::class(r"[\p{gc=L}\p{gc=Nl}\p{gc=Sc}\p{gc=Pc}]"),
        "javaJavaIdentifierPart" => Pred::class(
            r"[\p{gc=L}\p{gc=Sc}\p{gc=Pc}\p{gc=Nd}\p{gc=Nl}\p{gc=Mc}\p{gc=Mn}]",
        )
        .union(identifier_ignorable()),
        "javaUnicodeIdentifierStart" => {
            Pred::class(r"[\p{gc=L}\p{gc=Nl}\p{Other_ID_Start}]")
        }
        "javaUnicodeIdentifierPart" => Pred::class(
            r"[\p{gc=L}\p{gc=Pc}\p{gc=Nd}\p{gc=Nl}\p{gc=Mc}\p{gc=Mn}\p{Other_ID_Start}\p{Other_ID_Continue}]",
        )
        .union(identifier_ignorable()),
        "javaIdentifierIgnorable" => identifier_ignorable(),
        "javaSpaceChar" => Pred::class(r"[\p{gc=Zs}\p{gc=Zl}\p{gc=Zp}]"),
        "javaWhitespace" => Pred::class(
            r"[[\p{gc=Zs}\p{gc=Zl}\p{gc=Zp}--[\x{A0}\x{2007}\x{202F}]]\x{9}-\x{D}\x{1C}-\x{1F}]",
        ),
        "javaISOControl" => Pred::ranges(&[(0, 0x1F), (0x7F, 0x9F)]),
        "javaMirrored" => Pred::class(r"[\p{Bidi_Mirrored}]"),
        _ => return None,
    })
}

/// The binary properties `\p{IsName}` accepts, and the POSIX classes as
/// `UNICODE_CHARACTER_CLASS` reads them, the name compared upper-cased:
/// `CharPredicates.forUnicodeProperty`.
fn for_unicode_property(name: &str, casei: bool) -> Option<Pred> {
    let cased = || Pred::class(r"[\p{Lowercase}\p{Uppercase}\p{gc=Lt}]");
    Some(match name.to_uppercase().as_str() {
        "ALPHABETIC" => Pred::class(r"[\p{Alphabetic}]"),
        "ASSIGNED" => Pred::class(r"[\p{gc=Cn}]").not(),
        "CONTROL" => Pred::class(r"[\p{gc=Cc}]"),
        "EMOJI" => Pred::class(r"[\p{Emoji}]"),
        "EMOJI_PRESENTATION" => Pred::class(r"[\p{Emoji_Presentation}]"),
        "EMOJI_MODIFIER" => Pred::class(r"[\p{Emoji_Modifier}]"),
        "EMOJI_MODIFIER_BASE" => Pred::class(r"[\p{Emoji_Modifier_Base}]"),
        "EMOJI_COMPONENT" => Pred::class(r"[\p{Emoji_Component}]"),
        "EXTENDED_PICTOGRAPHIC" => Pred::class(r"[\p{Extended_Pictographic}]"),
        "HEXDIGIT" | "HEX_DIGIT" => Pred::class(r"[\p{gc=Nd}\p{Hex_Digit}]"),
        "IDEOGRAPHIC" => Pred::class(r"[\p{Ideographic}]"),
        "JOINCONTROL" | "JOIN_CONTROL" => Pred::class(r"[\p{Join_Control}]"),
        "LETTER" => Pred::class(r"[\p{gc=L}]"),
        "LOWERCASE" if casei => cased(),
        "LOWERCASE" => Pred::class(r"[\p{Lowercase}]"),
        "NONCHARACTERCODEPOINT" | "NONCHARACTER_CODE_POINT" => {
            Pred::class(r"[\p{Noncharacter_Code_Point}]")
        }
        "TITLECASE" if casei => cased(),
        "TITLECASE" => Pred::class(r"[\p{gc=Lt}]"),
        "PUNCTUATION" => Pred::class(r"[\p{gc=P}]"),
        "UPPERCASE" if casei => cased(),
        "UPPERCASE" => Pred::class(r"[\p{Uppercase}]"),
        "WHITESPACE" | "WHITE_SPACE" => Pred::class(r"[\p{White_Space}]"),
        "WORD" => word(true),
        // The POSIX names, as `UNICODE_CHARACTER_CLASS` reads them.
        _ => return for_posix_name(name, casei),
    })
}

/// The POSIX classes under `UNICODE_CHARACTER_CLASS`, the name compared
/// upper-cased: `CharPredicates.forPOSIXName`.
fn for_posix_name(name: &str, casei: bool) -> Option<Pred> {
    // `[^\p{IsWhite_Space}\p{gc=Cc}\p{gc=Cs}\p{gc=Cn}]`, less the surrogates.
    let graph = || Pred::class(r"[\p{White_Space}\p{gc=Cc}\p{gc=Cn}]").not();
    let blank = || Pred::class(r"[\p{White_Space}--[\p{gc=Zl}\p{gc=Zp}\x{A}-\x{D}\x{85}]]");
    Some(match name.to_uppercase().as_str() {
        "ALPHA" => Pred::class(r"[\p{Alphabetic}]"),
        "LOWER" | "UPPER" if casei => Pred::class(r"[\p{Lowercase}\p{Uppercase}\p{gc=Lt}]"),
        "LOWER" => Pred::class(r"[\p{Lowercase}]"),
        "UPPER" => Pred::class(r"[\p{Uppercase}]"),
        "SPACE" => Pred::class(r"[\p{White_Space}]"),
        "PUNCT" => Pred::class(r"[\p{gc=P}]"),
        "XDIGIT" => Pred::class(r"[\p{gc=Nd}\p{Hex_Digit}]"),
        "ALNUM" => Pred::class(r"[\p{Alphabetic}\p{gc=Nd}]"),
        "CNTRL" => Pred::class(r"[\p{gc=Cc}]"),
        "DIGIT" => Pred::class(r"[\p{gc=Nd}]"),
        "BLANK" => blank(),
        "GRAPH" => graph(),
        "PRINT" => graph()
            .union(blank())
            .and(Pred::class(r"[\p{gc=Cc}]").not()),
        _ => return None,
    })
}

/// A script by the name of its `Character.UnicodeScript` constant or by its
/// alias, compared upper-cased: `CharPredicates.forUnicodeScript`.
fn for_unicode_script(name: &str) -> Option<Pred> {
    let upper = name.to_uppercase();
    tables::SCRIPTS
        .iter()
        .find(|(constant, alias)| *constant == upper || *alias == upper)
        .map(|(constant, _)| {
            if *constant == "UNKNOWN" {
                // `Zzzz`: the code points no script claims.
                let all: String = tables::SCRIPTS
                    .iter()
                    .filter(|(c, _)| *c != "UNKNOWN")
                    .map(|(c, _)| format!(r"\p{{sc={c}}}"))
                    .collect();
                Pred::Class(format!("[{all}]")).not()
            } else {
                Pred::Class(format!(r"[\p{{sc={constant}}}]"))
            }
        })
}

/// A block by any of the names `Character.UnicodeBlock.forName` accepts,
/// compared upper-cased: `CharPredicates.forUnicodeBlock`.
fn for_unicode_block(name: &str) -> Option<Pred> {
    let upper = name.to_uppercase();
    tables::BLOCKS
        .iter()
        .find(|(_, _, constant, names)| {
            *constant == upper
                || names
                    .iter()
                    .any(|n| *n == upper || n.replace(' ', "") == upper)
        })
        .map(|&(lo, hi, _, _)| Pred::ranges(&[(lo, hi)]))
}

// ---------------------------------------------------------------------------
// Character names
// ---------------------------------------------------------------------------

/// The value of one symbol of [`names`]: `#` is 0, `~` is 91.
fn name_symbol(byte: u8) -> usize {
    usize::from(byte - b'#')
}

/// The words of [`names::NAME_WORDS`], decoded on first use.
fn name_words() -> &'static [String] {
    static WORDS: OnceLock<Vec<String>> = OnceLock::new();
    WORDS.get_or_init(|| {
        let mut words: Vec<String> = Vec::new();
        for line in names::NAME_WORDS.lines().filter(|line| !line.is_empty()) {
            let shared = name_symbol(line.as_bytes()[0]);
            let mut word = words.last().map_or("", |w| &w[..shared]).to_string();
            word.push_str(&line[1..]);
            words.push(word);
        }
        words
    })
}

/// The next word index of a line of [`names::NAMES`], at `*at`.
fn next_word_index(line: &[u8], at: &mut usize) -> usize {
    let lead = name_symbol(line[*at]);
    *at += 1;
    if lead < 40 {
        return lead;
    }
    if lead < 80 {
        let index = 40 + (lead - 40) * 92 + name_symbol(line[*at]);
        *at += 1;
        return index;
    }
    let index =
        3720 + (lead - 80) * 92 * 92 + name_symbol(line[*at]) * 92 + name_symbol(line[*at + 1]);
    *at += 2;
    index
}

/// The code point whose `Character.getName` is `name`, upper-case, among the
/// names [`names`] holds.
fn named_code_point(name: &str) -> Option<u32> {
    let words = name_words();
    let query = name
        .split(' ')
        .map(|word| words.binary_search_by(|w| w.as_str().cmp(word)).ok())
        .collect::<Option<Vec<usize>>>()?;
    let mut code_points = names::NAMED_RUNS
        .iter()
        .flat_map(|&(first, count)| first..first + count);
    let mut current: Vec<usize> = Vec::new();
    for line in names::NAMES.lines().filter(|line| !line.is_empty()) {
        let code_point = code_points.next()?;
        let bytes = line.as_bytes();
        current.truncate(name_symbol(bytes[0]));
        let mut at = 1;
        while at < bytes.len() {
            current.push(next_word_index(bytes, &mut at));
        }
        if current == query {
            return Some(code_point);
        }
    }
    None
}

/// `Character.codePointOf(name)`, `None` where it throws: the name, without
/// the characters up to U+0020 around it (`String.trim`) and upper-cased, is
/// either a name `Character.getName` gives, or the algorithmic name of an
/// assigned code point without one — its block's constant with `_` read as a
/// space, a space, and the code point in upper-case hexadecimal with no
/// leading zero — observed on the JDK.
fn code_point_of(name: &str) -> Option<u32> {
    let name = name.trim_matches(|c: char| c <= ' ').to_uppercase();
    if let Some(code_point) = named_code_point(&name) {
        return Some(code_point);
    }
    let (block, hex) = name.rsplit_once(' ')?;
    let code_point = u32::from_str_radix(hex, 16).ok()?;
    let algorithmic = names::ALGORITHMIC_NAMES
        .iter()
        .any(|&(lo, hi)| (lo..=hi).contains(&code_point));
    let &(_, _, constant, _) = tables::BLOCKS
        .iter()
        .find(|&&(lo, hi, _, _)| (lo..=hi).contains(&code_point))?;
    (algorithmic && constant.replace('_', " ") == block && format!("{code_point:X}") == hex)
        .then_some(code_point)
}

// ---------------------------------------------------------------------------
// Grapheme clusters
// ---------------------------------------------------------------------------

/// The control characters of the grapheme rules: `Control`, `CR` and `LF`.
const GRAPHEME_CONTROL: &str = r"[\p{gcb=Control}\p{gcb=CR}\p{gcb=LF}]";

/// What may follow the core of a grapheme cluster: `Extend`, `ZWJ` and
/// `SpacingMark`.
const GRAPHEME_POSTCORE: &str = r"[\p{gcb=Extend}\p{gcb=ZWJ}\p{gcb=SpacingMark}]";

/// A class of the `Indic_Conjunct_Break` values in [`tables`].
fn incb_class(ranges: &[(u32, u32)]) -> String {
    let mut set = CharSet::default();
    for &(lo, hi) in ranges {
        set.add_range(lo, hi);
    }
    Emitter::set_class(&set)
}

/// The consonant, the `Extend`-or-`Linker` class and the linker of rule GB9c.
fn conjunct_classes() -> (String, String, String) {
    let linker = incb_class(tables::INCB_LINKER);
    let extend_or_linker = format!("[{}{linker}]", incb_class(tables::INCB_EXTEND));
    (incb_class(tables::INCB_CONSONANT), extend_or_linker, linker)
}

/// `\X`: the longest extended grapheme cluster of Unicode 16.0 (UAX #29,
/// rules GB3 to GB13) that starts where the match is, with no regard to what
/// precedes it, as Java's `XGrapheme` finds it; matched atomically, as Java
/// never backtracks into it. Written as UAX #29's regular expression for an
/// extended grapheme cluster, whose alternatives, tried in order, find the
/// longest cluster.
fn grapheme_cluster() -> String {
    let (consonant, extend_or_linker, linker) = conjunct_classes();
    let hangul = r"(?:\p{gcb=L}*(?:\p{gcb=V}+|\p{gcb=LV}\p{gcb=V}*|\p{gcb=LVT})\p{gcb=T}*|\p{gcb=L}+|\p{gcb=T}+)";
    let pictographic =
        r"\p{Extended_Pictographic}(?:\p{gcb=Extend}*\p{gcb=ZWJ}\p{Extended_Pictographic})*";
    let regional = r"\p{gcb=Regional_Indicator}\p{gcb=Regional_Indicator}";
    let conjunct =
        format!("{consonant}(?:{extend_or_linker}*{linker}{extend_or_linker}*{consonant})+");
    format!(
        r"(?>\r\n|{GRAPHEME_CONTROL}|\p{{gcb=Prepend}}*(?:{hangul}|{regional}|{pictographic}|{conjunct}|[{ANY}--{GRAPHEME_CONTROL}]){GRAPHEME_POSTCORE}*)"
    )
}

/// `\b{g}`: a boundary of the Unicode 16.0 extended grapheme clusters of the
/// whole input (UAX #29, rules GB1 to GB999), as Java's `GraphemeBound`
/// finds it.
fn grapheme_boundary() -> String {
    let (consonant, extend_or_linker, linker) = conjunct_classes();
    let ri = r"\p{gcb=Regional_Indicator}";
    let no_break = [
        // GB6, GB7, GB8: Hangul syllables.
        r"(?<=\p{gcb=L})(?=[\p{gcb=L}\p{gcb=V}\p{gcb=LV}\p{gcb=LVT}])".to_string(),
        r"(?<=[\p{gcb=LV}\p{gcb=V}])(?=[\p{gcb=V}\p{gcb=T}])".to_string(),
        r"(?<=[\p{gcb=LVT}\p{gcb=T}])(?=\p{gcb=T})".to_string(),
        // GB9, GB9a: before Extend, ZWJ and SpacingMark.
        format!("(?={GRAPHEME_POSTCORE})"),
        // GB9b: after Prepend.
        r"(?<=\p{gcb=Prepend})".to_string(),
        // GB9c: Indic conjuncts.
        format!("(?<={consonant}{extend_or_linker}*{linker}{extend_or_linker}*)(?={consonant})"),
        // GB11: emoji ZWJ sequences.
        r"(?<=\p{Extended_Pictographic}\p{gcb=Extend}*\p{gcb=ZWJ})(?=\p{Extended_Pictographic})"
            .to_string(),
        // GB12, GB13: after an odd number of regional indicators.
        format!(r"(?<=(?:\A|[{ANY}--{ri}])(?:{ri}{ri})*{ri})(?={ri})"),
    ]
    .join("|");
    // GB1, GB2; GB3; GB4, GB5; then the rules that do not break.
    format!(
        r"(?:\A|\z|(?!(?<=\r)\n)(?:(?<={GRAPHEME_CONTROL})|(?={GRAPHEME_CONTROL})|(?!{no_break})))"
    )
}

// ---------------------------------------------------------------------------
// Emitter
// ---------------------------------------------------------------------------

/// Writes the parsed pattern in `fancy-regex` syntax.
struct Emitter<'a> {
    bits: &'a [[bool; 256]],
    /// The number of capturing groups in the pattern.
    group_count: usize,
    /// Whether counted repetitions outside look-behinds are written for the
    /// VM (see [`translate_for_engine`]).
    counts_in_vm: bool,
    /// How many look-behinds enclose the node being written.
    look_behind_depth: usize,
    out: String,
}

impl Emitter<'_> {
    fn node(&mut self, node: &Node) {
        match node {
            Node::Empty => {}
            Node::Char(pred) => {
                let class = Self::class_of(pred, self.bits);
                self.out.push_str(&class);
            }
            Node::Sequence(nodes) => {
                for node in nodes {
                    self.node(node);
                }
            }
            Node::Alternation(nodes) => {
                self.out.push_str("(?:");
                for (i, node) in nodes.iter().enumerate() {
                    if i > 0 {
                        self.out.push('|');
                    }
                    self.node(node);
                }
                self.out.push(')');
            }
            Node::Group(kind, body) => {
                self.out.push_str(match kind {
                    GroupKind::Capturing => "(",
                    GroupKind::Named(_) => "(?<",
                    GroupKind::NonCapturing => "(?:",
                    GroupKind::Atomic => "(?>",
                    GroupKind::LookAhead => "(?=",
                    GroupKind::NegativeLookAhead => "(?!",
                    GroupKind::LookBehind => "(?<=",
                    GroupKind::NegativeLookBehind => "(?<!",
                });
                if let GroupKind::Named(name) = kind {
                    self.out.push_str(name);
                    self.out.push('>');
                }
                let look_behind =
                    matches!(kind, GroupKind::LookBehind | GroupKind::NegativeLookBehind);
                self.look_behind_depth += usize::from(look_behind);
                self.node(body);
                self.look_behind_depth -= usize::from(look_behind);
                self.out.push(')');
            }
            Node::Repeat {
                body,
                min,
                max,
                greed,
            } => self.repeat(body, *min, *max, *greed),
            Node::BackRef { group, casei } => {
                if *group > self.group_count || *group == 0 {
                    // Java compiles a reference to a group the pattern does
                    // not have, and it never matches: an empty class, which
                    // can be repeated as Java repeats the reference.
                    self.out.push_str(r"[^\x{0}-\x{10FFFF}]");
                } else if *casei {
                    let _ = write!(self.out, r"(?i:\{group})");
                } else {
                    let _ = write!(self.out, r"(?:\{group})");
                }
            }
            Node::LineEnding => {
                self.out
                    .push_str(r"(?:\r\n|[\n\x{B}\x{C}\r\x{85}\x{2028}\x{2029}])");
            }
            Node::Grapheme => self.out.push_str(&grapheme_cluster()),
            Node::Assertion(text) => self.out.push_str(text),
        }
    }

    /// A quantified node.
    ///
    /// Java repeats a node that only matches the empty string without ever
    /// looping on it, and `fancy-regex` refuses to repeat such a node, so the
    /// choice Java makes is written out: `?` tries the node, then goes on
    /// without it (`??` the other way round); `*` and `{0,n}` try it once and
    /// go on whether it matched or not, except greedily over a capturing
    /// group, whose zero-length iteration Java does not record, and except
    /// lazily, which goes on without it; a positive minimum requires one
    /// match.
    ///
    /// Java repeats a single node, and a group without alternatives or
    /// variable repetitions inside, by matching one iteration at a time
    /// without backtracking into a finished iteration; a `?` over a group
    /// does backtrack into it. The only such node with a choice inside is
    /// `\R`, which can give back the `\n` of a `\r\n`; each such iteration
    /// is therefore atomic.
    fn repeat(&mut self, body: &Node, min: u32, max: u32, greed: Greed) {
        if body.is_zero_width() {
            let mut inner = Emitter {
                bits: self.bits,
                group_count: self.group_count,
                counts_in_vm: self.counts_in_vm,
                look_behind_depth: self.look_behind_depth,
                out: String::new(),
            };
            inner.node(body);
            let inner = inner.out;
            let capturing = matches!(
                body,
                Node::Group(GroupKind::Capturing | GroupKind::Named(_), _)
            );
            let _ = if max == 0 {
                write!(self.out, "(?:(?!){inner}|)")
            } else if min > 0 {
                write!(self.out, "(?:{inner})")
            } else if max == 1 {
                match greed {
                    Greed::Greedy => write!(self.out, "(?:{inner}|)"),
                    Greed::Lazy => write!(self.out, "(?:|{inner})"),
                    Greed::Possessive => write!(self.out, "(?>{inner}|)"),
                }
            } else {
                match greed {
                    Greed::Greedy if !capturing => write!(self.out, "(?:{inner}|)"),
                    Greed::Possessive => write!(self.out, "(?>{inner}|)"),
                    _ => write!(self.out, "(?:(?!){inner}|)"),
                }
            };
            return;
        }
        let atomic_iteration = matches!(body, Node::LineEnding)
            || (matches!(
                body,
                Node::Group(
                    GroupKind::Capturing | GroupKind::Named(_) | GroupKind::NonCapturing,
                    _
                )
            ) && !(min == 0 && max == 1)
                && is_deterministic(body)
                && contains(body, &|n| matches!(n, Node::LineEnding)));
        let possessive = greed == Greed::Possessive;
        if possessive {
            self.out.push_str("(?>");
        }
        // A count the VM runs: see `translate_for_engine`.
        let counted_in_vm = self.counts_in_vm
            && self.look_behind_depth == 0
            && !matches!((min, max), (0, 1) | (0, MAX_REPS) | (1, MAX_REPS))
            && min_length(body) > 0;
        if counted_in_vm {
            self.out.push_str("(?:");
        }
        if matches!(body, Node::LineEnding) {
            self.out
                .push_str(r"(?>\r\n|[\n\x{B}\x{C}\r\x{85}\x{2028}\x{2029}])");
        } else if atomic_iteration {
            self.out.push_str("(?>");
            self.node(body);
            self.out.push(')');
        } else if matches!(body, Node::Char(_) | Node::Group(..) | Node::Grapheme) {
            // Already a single atom of the output.
            self.node(body);
        } else {
            self.out.push_str("(?:");
            self.node(body);
            self.out.push(')');
        }
        if counted_in_vm {
            self.out.push_str("(?=))");
        }
        let _ = match (min, max) {
            (0, 1) => write!(self.out, "?"),
            (0, MAX_REPS) => write!(self.out, "*"),
            (1, MAX_REPS) => write!(self.out, "+"),
            (min, MAX_REPS) => write!(self.out, "{{{min},}}"),
            (min, max) if min == max => write!(self.out, "{{{min}}}"),
            (min, max) => write!(self.out, "{{{min},{max}}}"),
        };
        if greed == Greed::Lazy {
            self.out.push('?');
        }
        if possessive {
            self.out.push(')');
        }
    }

    /// The `regex-syntax` class, brackets included, of a predicate.
    fn class_of(pred: &Pred, bits: &[[bool; 256]]) -> String {
        match pred {
            Pred::Set(set) => Self::set_class(set),
            Pred::Class(text) => text.clone(),
            Pred::Bits(index) => {
                let mut set = CharSet::default();
                for (c, &present) in bits[*index].iter().enumerate() {
                    if present {
                        set.add(c as u32);
                    }
                }
                Self::set_class(&set)
            }
            Pred::Union(members) => {
                let mut out = String::from("[");
                for member in members {
                    out.push_str(&Self::class_of(member, bits));
                }
                out.push(']');
                out
            }
            Pred::And(left, right) => format!(
                "[{}&&{}]",
                Self::class_of(left, bits),
                Self::class_of(right, bits)
            ),
            // A difference rather than `[^…]`: `regex-syntax` complements a
            // class with members on both sides of the surrogates, in separate
            // ranges, into one that also holds U+D7FF and U+E000.
            Pred::Not(inner) => format!("[{ANY}--{}]", Self::class_of(inner, bits)),
        }
    }

    /// An explicit set. No Rust string holds a surrogate code point, so the
    /// surrogates are left out, and the ranges on either side of them are
    /// joined into one range across them: `regex-syntax` complements a class
    /// whose only gap is the surrogates into `[\x{D7FF}\x{E000}]`, where a
    /// range across them complements correctly.
    fn set_class(set: &CharSet) -> String {
        let surrogate = |c: u32| (0xD800..=0xDFFF).contains(&c);
        let mut ranges: Vec<(u32, u32)> = Vec::new();
        for &(lo, hi) in &set.ranges {
            let lo = if surrogate(lo) { 0xE000 } else { lo };
            let hi = if surrogate(hi) { 0xD7FF } else { hi };
            if lo > hi {
                continue;
            }
            match ranges.last_mut() {
                Some(last) if last.1 == 0xD7FF && lo == 0xE000 => last.1 = hi,
                _ => ranges.push((lo, hi)),
            }
        }
        if ranges.is_empty() {
            return r"[^\x{0}-\x{10FFFF}]".to_string();
        }
        let mut out = String::from("[");
        for (lo, hi) in ranges {
            let _ = if lo == hi {
                write!(out, r"\x{{{lo:X}}}")
            } else {
                write!(out, r"\x{{{lo:X}}}-\x{{{hi:X}}}")
            };
        }
        out.push(']');
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The pattern [`translate_for_engine`] writes, counted repetitions left
    /// to the automaton.
    fn translate(regex: &str) -> Result<String, PatternSyntaxError> {
        translate_for_engine(regex, false).map(|translation| translation.pattern)
    }

    #[test]
    fn case_table_is_sorted() {
        let keys: Vec<u32> = tables::CASE_MAPPINGS.iter().step_by(3).copied().collect();
        assert!(keys.windows(2).all(|w| w[0] < w[1]));
        assert_eq!(to_upper(u32::from(b'a')), u32::from(b'A'));
        assert_eq!(to_lower(0x130), u32::from(b'i'));
        assert_eq!(to_upper(0x1F80), 0x1F88);
    }

    /// What Java makes of `pattern` over `sample`, in the encoding of the
    /// `RuceneMemberContractsFixture` corpus: `OK|<code-point start>:<hex
    /// match>,…` over `Matcher.find()`, or `ERR` when `Pattern.compile`
    /// throws.
    fn outcome(pattern: &str, sample: &str) -> String {
        let Ok(translated) = translate(pattern) else {
            return "ERR".to_string();
        };
        let Ok(regex) = fancy_regex::Regex::new(&translated) else {
            return "ERR".to_string();
        };
        let mut found = Vec::new();
        let mut from = 0;
        while let Some(m) = regex.find_from_pos(sample, from).expect("match") {
            let hex: String = m.as_str().bytes().map(|b| format!("{b:02x}")).collect();
            found.push(format!("{}:{hex}", sample[..m.start()].chars().count()));
            from = m.end();
            if m.start() == m.end() {
                match sample[from..].chars().next() {
                    Some(c) => from += c.len_utf8(),
                    None => break,
                }
            }
        }
        format!("OK|{}", found.join(","))
    }

    /// Constructs outside the fixture's corpus whose Java meaning the parser
    /// reproduces, each pinned to what JDK 25.0.4 printed for it (oracle
    /// `RegexOracle.java` of rmp task #243, probe list `unit.py`): case folding
    /// of a lone literal, a run of literals and a class member; property names
    /// and their case-insensitive forms; the characters a class accumulates
    /// before and after `&&`; comments mode; `\Q…\E`; inline flag scope;
    /// quantifiers over quantifiers, over nothing and over zero-width nodes;
    /// `\R` under repetition; look-behind length checks; back reference
    /// numbering; anchors; boundaries; grapheme clusters.
    #[test]
    fn translation_matches_the_jdk_on_constructs_outside_the_corpus() {
        let rows: &[(&str, &str, &str)] = &[
            ("(?iu)\u{df}", "\u{1e9e}", "OK|"),
            ("(?iu)\u{df}x", "\u{1e9e}x", "OK|0:e1ba9e78"),
            ("(?iu)\u{df}x+", "\u{1e9e}x", "OK|"),
            ("(?iu)[\u{df}]", "\u{1e9e}", "OK|"),
            ("(?iu)\u{1e9e}", "\u{df}\u{1e9e}", "OK|0:c39f,1:e1ba9e"),
            ("(?iu)i", "iI\u{130}\u{131}", "OK|0:69,1:49,2:c4b0,3:c4b1"),
            ("(?i)[h-j]", "iI\u{130}\u{131}", "OK|0:69,1:49"),
            (
                "(?iu)[h-j]",
                "iI\u{130}\u{131}",
                "OK|0:69,1:49,2:c4b0,3:c4b1",
            ),
            ("(?i)\\x{212a}", "kK\u{212a}", "OK|2:e284aa"),
            ("(?iu)[k]", "kK\u{212a}", "OK|0:6b,1:4b,2:e284aa"),
            (
                "(?iu)[\u{b5}]",
                "\u{b5}\u{39c}\u{3bc}",
                "OK|0:c2b5,1:ce9c,2:cebc",
            ),
            (
                "(?i)\\p{Lu}+",
                "aZ\u{e9}\u{c9}\u{1c5}",
                "OK|0:615ac3a9c389c785",
            ),
            ("(?i)\\p{Lower}+", "aZ\u{e9}\u{c9}", "OK|0:615a"),
            ("(?U)\\p{lower}", "a", "OK|0:61"),
            ("\\p{IsLower}+", "ab\u{e9}", "OK|0:6162c3a9"),
            ("\\p{Latin}", "a", "ERR"),
            ("\\p{InGreek_and_Coptic}", "a", "ERR"),
            ("\\p{InBasic_Latin}+", "a\u{e9}", "OK|0:61"),
            ("[a&&&b]", "ab&", "OK|0:61,1:62,2:26"),
            ("[ab&&[b]c]", "abc", "OK|1:62"),
            ("[a&&]", "a", "OK|0:61"),
            ("[&&]", "&a", "ERR"),
            ("[[x]a&&]", "x", "ERR"),
            ("[a&&&&b]", "ab", "OK|"),
            ("[\\d-a]", "a-1", "OK|0:61,1:2d,2:31"),
            ("[a-\\d]", "a", "ERR"),
            ("[^]a]", "]ab", "OK|2:62"),
            ("(?x)[a#]\u{a}b]", "a]b", "OK|0:61,2:62"),
            ("(?x)\\p{ L}", "a", "OK|0:61"),
            ("(?x)\\p{ IsL }", "a", "ERR"),
            ("(?x)a{ 2}", "aa", "ERR"),
            ("(?x)a #c\u{2028}b", "ab", "OK|"),
            ("(?x)a #c\u{d}b", "ab", "OK|0:6162"),
            ("(?x)[ ^a]", "a^b", "OK|0:61,1:5e"),
            ("(?x)( ? : a)", "a", "OK|0:61"),
            ("\\x\\Q41\\E", "A", "ERR"),
            ("\\c\\Qa\\E", "!\u{1}", "OK|0:21"),
            ("\\Q\\\\E", "a\\b", "OK|1:5c"),
            ("\\E", "E", "ERR"),
            ("[a\\Q-\\Ec]", "a-bc", "OK|0:61,1:2d,3:63"),
            ("(a(?i)b)c", "aBc aBC", "OK|0:614263"),
            ("(?U-u)(?i)\u{e9}", "\u{c9}", "OK|"),
            ("(?U)(?-u)\\w", "\u{e9}", "OK|0:c3a9"),
            ("a**", "aa", "ERR"),
            ("a*{2}", "aa", "OK|0:6161,2:"),
            ("{2}", "a", "OK|0:,1:"),
            ("x{2}{3}", "xxxxxx", "OK|0:7878,2:7878,4:7878"),
            ("(?=a)*b", "ab", "OK|1:62"),
            ("(?=a)+b", "ab", "OK|"),
            ("()*\\1", "a", "OK|"),
            ("()??\\1", "a", "OK|0:,1:"),
            ("()*+\\1", "a", "OK|0:,1:"),
            ("(?:(?=(a)))*\\1", "a", "OK|0:61"),
            ("\\R\\n", "\u{d}\u{a}", "OK|0:0d0a"),
            ("\\R{2}", "\u{d}\u{a}", "OK|"),
            ("(?:\\R){2}", "\u{d}\u{a}", "OK|"),
            ("(?:\\R)?\\n", "\u{d}\u{a}", "OK|0:0d0a"),
            ("(?<=(?:ab)*)c", "abc", "ERR"),
            ("(?<=(a)*)d", "aad", "OK|2:64"),
            ("(a)(?<=\\1)b", "ab", "ERR"),
            ("(?<=(a|bc)?)d", "abcd", "OK|3:64"),
            ("(a)\\10", "aa0", "OK|0:616130"),
            (
                "(a)(b)(c)(d)(e)(f)(g)(h)(i)(j)\\10",
                "abcdefghijj",
                "OK|0:6162636465666768696a6a",
            ),
            ("(a)\\2", "aa", "OK|"),
            ("(?<n>a)(?<n>b)", "ab", "ERR"),
            ("\\k<n>(?<n>a)", "aa", "ERR"),
            ("$", "a\u{d}\u{a}", "OK|1:,3:"),
            ("(?m)^", "a\u{a}", "OK|0:"),
            ("(?md)$", "a\u{d}\u{a}b\u{a}", "OK|2:,4:,5:"),
            ("\\b", "a\u{301}b \u{e9}", "OK|0:,3:"),
            ("\\b", "\u{e9}\u{301} ", "OK|1:,2:"),
            ("\\X", "\u{915}\u{94d}\u{937}", "OK|0:e0a495e0a58de0a4b7"),
            (
                "\\X",
                "\u{1f1fa}\u{1f1f8}\u{1f1eb}",
                "OK|0:f09f87baf09f87b8,2:f09f87ab",
            ),
            ("\\b{g}", "\u{1f1fa}\u{1f1f8}\u{1f1eb}", "OK|0:,2:,3:"),
            ("\\b{g}", "e\u{301}x", "OK|0:,2:,3:"),
            ("a\\X", "a\u{308}b", "OK|0:61cc88"),
        ];
        for &(pattern, sample, java) in rows {
            assert_eq!(
                outcome(pattern, sample),
                java,
                "{pattern:?} over {sample:?}"
            );
        }
    }

    /// `\N{name}` resolves its name as `Character.codePointOf` does: a name
    /// `Character.getName` gives, upper-cased and without the characters up
    /// to U+0020 around it, or the algorithmic name of an assigned code point
    /// without one. Each row is pinned to what JDK 25.0.4 printed for it
    /// (oracle `RegexOracle.java` of rmp task #243, probe list
    /// `unit_names.py`); the oracle's exhaustive run over every named code
    /// point is the differential `n1.py`. An unknown name fails as Java's
    /// `Pattern` does, with `Unknown character name [<name>]` near the closing
    /// brace (`\N{NOPE}`: index 7).
    #[test]
    fn named_characters_resolve_as_character_code_point_of() {
        let rows: &[(&str, &str, &str)] = &[
            ("\\N{LATIN SMALL LETTER A}", "a", "OK|0:61"),
            ("\\N{latin small letter a}", "a", "OK|0:61"),
            ("\\N{ \u{9}LINE FEED (LF) }", "a\u{a}b", "OK|1:0a"),
            ("\\N{LINE FEED}", "\u{a}", "ERR"),
            ("\\N{ZERO WIDTH NO-BREAK SPACE}", "\u{feff}", "OK|0:efbbbf"),
            ("\\N{BYTE ORDER MARK}", "\u{feff}", "ERR"),
            ("\\N{lat\u{131}n small letter a}", "a", "OK|0:61"),
            ("\\N{\u{17f}pace}", " ", "OK|0:20"),
            ("\\N{\u{2003}LATIN SMALL LETTER A}", "a", "ERR"),
            ("\\N{LATIN  SMALL LETTER A}", "a", "ERR"),
            (
                "\\N{DESERET CAPITAL LETTER LONG I}",
                "\u{10400}",
                "OK|0:f0909080",
            ),
            (
                "\\N{CJK UNIFIED IDEOGRAPHS 4E00}",
                "\u{4e00}",
                "OK|0:e4b880",
            ),
            (
                "\\N{cjk unified ideographs 9fff}",
                "\u{9fff}",
                "OK|0:e9bfbf",
            ),
            ("\\N{CJK UNIFIED IDEOGRAPHS 04E00}", "\u{4e00}", "ERR"),
            ("\\N{CJK UNIFIED IDEOGRAPHS +4E00}", "\u{4e00}", "ERR"),
            ("\\N{HANGUL SYLLABLES AC00}", "\u{ac00}", "OK|0:eab080"),
            ("\\N{HANGUL SYLLABLE GA}", "\u{ac00}", "ERR"),
            ("\\N{LATIN 1 SUPPLEMENT 84}", "\u{84}", "OK|0:c284"),
            ("\\N{LATIN-1 SUPPLEMENT 84}", "\u{84}", "ERR"),
            ("\\N{BASIC LATIN 41}", "A", "ERR"),
            ("\\N{TANGUT SUPPLEMENT 18D00}", "\u{18d00}", "OK|0:f098b480"),
            (
                "\\N{SUPPLEMENTARY PRIVATE USE AREA B 10FFFD}",
                "\u{10fffd}",
                "OK|0:f48fbfbd",
            ),
            ("\\N{HIGH SURROGATES D800}", "a", "OK|"),
            ("\\N{NOPE}", "a", "ERR"),
            (
                "[\\N{LATIN SMALL LETTER A}-\\N{LATIN SMALL LETTER C}]+",
                "abcd",
                "OK|0:616263",
            ),
            ("(?x)\\N {LATIN SMALL LETTER A}", "a", "OK|0:61"),
            ("\\N{LATIN SMALL LETTER A", "a", "ERR"),
            ("\\N", "a", "ERR"),
            ("\\N{}", "a", "ERR"),
        ];
        for &(pattern, sample, java) in rows {
            assert_eq!(
                outcome(pattern, sample),
                java,
                "{pattern:?} over {sample:?}"
            );
        }
        let error = translate(r"\N{NOPE}").expect_err("an unknown name");
        assert_eq!(
            (error.description.as_str(), error.index),
            ("Unknown character name [NOPE]", 7)
        );
    }

    #[test]
    fn char_set_merges_ranges() {
        let mut set = CharSet::default();
        set.add_range(5, 7);
        set.add_range(1, 2);
        set.add(3);
        set.add_range(9, 12);
        set.add(8);
        assert_eq!(set.ranges, vec![(1, 3), (5, 12)]);
    }
}
