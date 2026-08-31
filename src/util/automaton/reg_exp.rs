//! Port of `org.apache.lucene.util.automaton.RegExp`.

use std::collections::{HashMap, HashSet};

use crate::error::{LuceneError, Result};

use super::automata::Automata;
use super::automaton::{Automaton, Transition, TransitionAccessor, MAX_CODE_POINT, MIN_CODE_POINT};
use super::automaton_provider::AutomatonProvider;
use super::case_folding::CaseFolding;
use super::operations::{Operations, DEFAULT_DETERMINIZE_WORK_LIMIT};

/// The type of expression represented by a [`RegExp`] node.
///
/// Equivalent to `org.apache.lucene.util.automaton.RegExp.Kind`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    /// The union of two expressions.
    RegexpUnion,
    /// A sequence of two expressions.
    RegexpConcatenation,
    /// The intersection of two expressions.
    RegexpIntersection,
    /// An optional expression.
    RegexpOptional,
    /// An expression that repeats.
    RegexpRepeat,
    /// An expression that repeats a minimum number of times.
    RegexpRepeatMin,
    /// An expression that repeats a minimum and maximum number of times.
    RegexpRepeatMinmax,
    /// The complement of a character class.
    RegexpComplement,
    /// A character.
    RegexpChar,
    /// A character range.
    RegexpCharRange,
    /// A character class (list of ranges).
    RegexpCharClass,
    /// Any character allowed.
    RegexpAnychar,
    /// An empty expression.
    RegexpEmpty,
    /// A string expression.
    RegexpString,
    /// Any string allowed.
    RegexpAnystring,
    /// An automaton expression.
    RegexpAutomaton,
    /// An interval expression.
    RegexpInterval,
    /// The complement of an expression. Deprecated: will be removed in Lucene 11.
    RegexpDeprecatedComplement,
}

impl std::fmt::Display for Kind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match self {
            Kind::RegexpUnion => "REGEXP_UNION",
            Kind::RegexpConcatenation => "REGEXP_CONCATENATION",
            Kind::RegexpIntersection => "REGEXP_INTERSECTION",
            Kind::RegexpOptional => "REGEXP_OPTIONAL",
            Kind::RegexpRepeat => "REGEXP_REPEAT",
            Kind::RegexpRepeatMin => "REGEXP_REPEAT_MIN",
            Kind::RegexpRepeatMinmax => "REGEXP_REPEAT_MINMAX",
            Kind::RegexpComplement => "REGEXP_COMPLEMENT",
            Kind::RegexpChar => "REGEXP_CHAR",
            Kind::RegexpCharRange => "REGEXP_CHAR_RANGE",
            Kind::RegexpCharClass => "REGEXP_CHAR_CLASS",
            Kind::RegexpAnychar => "REGEXP_ANYCHAR",
            Kind::RegexpEmpty => "REGEXP_EMPTY",
            Kind::RegexpString => "REGEXP_STRING",
            Kind::RegexpAnystring => "REGEXP_ANYSTRING",
            Kind::RegexpAutomaton => "REGEXP_AUTOMATON",
            Kind::RegexpInterval => "REGEXP_INTERVAL",
            Kind::RegexpDeprecatedComplement => "REGEXP_DEPRECATED_COMPLEMENT",
        };
        f.write_str(name)
    }
}

/// Regular expression extension to [`Automaton`].
///
/// Equivalent to `org.apache.lucene.util.automaton.RegExp`; see the Lucene javadoc
/// for the full grammar. Regular expressions are built from the following abstract
/// syntax:
///
/// ```text
/// regexp       ::= unionexp
/// unionexp     ::= interexp '|' unionexp | interexp
/// interexp     ::= concatexp '&' interexp | concatexp          [INTERSECTION]
/// concatexp    ::= repeatexp concatexp | repeatexp
/// repeatexp    ::= repeatexp '?' | repeatexp '*' | repeatexp '+'
///                | repeatexp '{' n '}' | repeatexp '{' n ',}'
///                | repeatexp '{' n ',' m '}' | complexp
/// charclassexp ::= '[' charclasses ']' | '[^' charclasses ']' | simpleexp
/// charclasses  ::= charclass charclasses | charclass
/// charclass    ::= charexp '-' charexp | charexp
/// simpleexp    ::= charexp | '.' | '#' | '@' | '"' string '"' | '(' ')'
///                | '(' unionexp ')' | '<' identifier '>' | '<' n '-' m '>'
/// charexp      ::= <Unicode character> | '\d' | '\D' | '\s' | '\S' | '\w' | '\W'
///                | '\' <Unicode character>
/// ```
///
/// # Divergence from Lucene 10.5.0
///
/// The parser tracks its position in UTF-16 code units, exactly like Java's
/// `String.codePointAt` / `Character.charCount`, so the positions reported in error
/// messages match Lucene's. Lucene's `iterativeParseExp` higher-order helper is
/// expressed with function pointers instead of Java functional interfaces.
#[derive(Clone, Debug)]
pub struct RegExp {
    /// The type of expression.
    pub kind: Kind,
    /// First child expression held by a container type expression.
    pub exp1: Option<Box<RegExp>>,
    /// Second child expression held by a container type expression.
    pub exp2: Option<Box<RegExp>>,
    /// String expression.
    pub s: Option<String>,
    /// Character expression.
    pub c: i32,
    /// Lower limit for repeatable type expressions.
    pub min: i32,
    /// Upper limit for repeatable type expressions.
    pub max: i32,
    /// Number of digits for interval type expressions.
    pub digits: i32,
    /// Lower extents for range type expressions.
    pub from: Option<Vec<i32>>,
    /// Upper extents for range type expressions.
    pub to: Option<Vec<i32>>,

    original_string: Option<String>,
    flags: i32,
}

// -----  Syntax flags ( <= 0xff )  ------

impl RegExp {
    /// Syntax flag, enables intersection (`&`).
    pub const INTERSECTION: i32 = 0x0001;

    /// Syntax flag, enables the empty language (`#`).
    pub const EMPTY: i32 = 0x0004;

    /// Syntax flag, enables anystring (`@`).
    pub const ANYSTRING: i32 = 0x0008;

    /// Syntax flag, enables named automata (`<identifier>`).
    pub const AUTOMATON: i32 = 0x0010;

    /// Syntax flag, enables numerical intervals (`<n-m>`).
    pub const INTERVAL: i32 = 0x0020;

    /// Syntax flag, enables all optional regexp syntax.
    pub const ALL: i32 = 0xff;

    /// Syntax flag, enables no optional regexp syntax.
    pub const NONE: i32 = 0x0000;

    // -----  Matching flags ( > 0xff <= 0xffff )  ------

    /// Allows case-insensitive matching of ASCII characters.
    ///
    /// Deprecated in favour of [`RegExp::CASE_INSENSITIVE`], which supports the full
    /// range of Unicode characters. Usage of this flag now has the same behaviour as
    /// [`RegExp::CASE_INSENSITIVE`].
    pub const ASCII_CASE_INSENSITIVE: i32 = 0x0100;

    /// Allows case-insensitive matching of most Unicode characters.
    ///
    /// In general the attempt is to reach parity with `java.util.regex.Pattern`'s
    /// `CASE_INSENSITIVE` and `UNICODE_CASE` flags when doing a case-insensitive
    /// match: common case folding is supported in addition to simple case folding, as
    /// defined by the common (C) and simple (S) mappings in `CaseFolding.txt`.
    pub const CASE_INSENSITIVE: i32 = 0x0200;

    /// Similar to [`RegExp::CASE_INSENSITIVE`] but for character class ranges.
    ///
    /// This flag allows ranges such as `[a-z]` to match `A`, but may result in
    /// performance costs during parsing.
    pub const CASE_INSENSITIVE_RANGE: i32 = 0x0400;

    // -----  Deprecated flags ( > 0xffff )  ------

    /// Allows regexp parsing of the complement (`~`).
    ///
    /// Note that processing the complement can require exponential time, but it is
    /// bounded by an internal limit. Regexes exceeding the limit fail with a
    /// [`TooComplexToDeterminizeException`](super::too_complex_to_determinize_exception::TooComplexToDeterminizeException).
    /// Deprecated: will be removed in Lucene 11.
    pub const DEPRECATED_COMPLEMENT: i32 = 0x10000;

    /// Constructs a new `RegExp` from a string; same as
    /// `RegExp::with_flags(s, RegExp::ALL)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] if an error occurred while parsing
    /// the regular expression.
    pub fn new(s: &str) -> Result<Self> {
        Self::with_flags(s, Self::ALL)
    }

    /// Constructs a new `RegExp` from a string with the given syntax flags.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] if an error occurred while parsing
    /// the regular expression.
    pub fn with_flags(s: &str, syntax_flags: i32) -> Result<Self> {
        Self::with_match_flags(s, syntax_flags, 0)
    }

    /// Constructs a new `RegExp` from a string with the given syntax and match flags.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] if an error occurred while parsing
    /// the regular expression, or if an illegal flag combination was given.
    pub fn with_match_flags(s: &str, syntax_flags: i32, match_flags: i32) -> Result<Self> {
        if (syntax_flags & !Self::DEPRECATED_COMPLEMENT) > Self::ALL {
            return Err(LuceneError::IllegalArgument(
                "Illegal syntax flag".to_string(),
            ));
        }

        if match_flags > 0 && match_flags <= Self::ALL {
            return Err(LuceneError::IllegalArgument(
                "Illegal match flag".to_string(),
            ));
        }
        let flags = syntax_flags | match_flags;
        let mut parser = Parser::new(s, flags);
        let e = if s.is_empty() {
            Self::make_string(flags, String::new())
        } else {
            let e = parser.parse_union_exp()?;
            if parser.pos < parser.input.len() {
                return Err(LuceneError::IllegalArgument(format!(
                    "end-of-string expected at position {}",
                    parser.pos
                )));
            }
            e
        };
        Ok(Self {
            original_string: Some(s.to_string()),
            ..e
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn new_node(
        flags: i32,
        kind: Kind,
        exp1: Option<Box<RegExp>>,
        exp2: Option<Box<RegExp>>,
        s: Option<String>,
        c: i32,
        min: i32,
        max: i32,
        digits: i32,
        from: Option<Vec<i32>>,
        to: Option<Vec<i32>>,
    ) -> Self {
        Self {
            original_string: None,
            kind,
            flags,
            exp1,
            exp2,
            s,
            c,
            min,
            max,
            digits,
            from,
            to,
        }
    }

    /// Simplified construction of container nodes.
    fn new_container_node(
        flags: i32,
        kind: Kind,
        exp1: Option<RegExp>,
        exp2: Option<RegExp>,
    ) -> Self {
        Self::new_node(
            flags,
            kind,
            exp1.map(Box::new),
            exp2.map(Box::new),
            None,
            0,
            0,
            0,
            0,
            None,
            None,
        )
    }

    /// Simplified construction of repeating nodes.
    fn new_repeating_node(flags: i32, kind: Kind, exp: RegExp, min: i32, max: i32) -> Self {
        Self::new_node(
            flags,
            kind,
            Some(Box::new(exp)),
            None,
            None,
            0,
            min,
            max,
            0,
            None,
            None,
        )
    }

    /// Simplified construction of leaf nodes.
    #[allow(clippy::too_many_arguments)]
    fn new_leaf_node(
        flags: i32,
        kind: Kind,
        s: Option<String>,
        c: i32,
        min: i32,
        max: i32,
        digits: i32,
        from: Option<Vec<i32>>,
        to: Option<Vec<i32>>,
    ) -> Self {
        Self::new_node(flags, kind, None, None, s, c, min, max, digits, from, to)
    }

    /// Constructs a new [`Automaton`] from this `RegExp`; same as
    /// `to_automaton_with(None, None)`.
    ///
    /// # Errors
    ///
    /// Returns an error if a named automaton is not available, or if determinizing
    /// the automaton exceeds the work limit.
    pub fn to_automaton(&self) -> Result<Automaton> {
        self.to_automaton_with(None, None)
    }

    /// Constructs a new [`Automaton`] from this `RegExp`, resolving named automata
    /// through the given map and/or provider.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] if this regular expression uses a
    /// named identifier that is not available, and a resource-limit error if
    /// determinizing exceeds the work limit.
    pub fn to_automaton_with(
        &self,
        automata: Option<&HashMap<String, Automaton>>,
        automaton_provider: Option<&dyn AutomatonProvider>,
    ) -> Result<Automaton> {
        let a = match self.kind {
            Kind::RegexpUnion => {
                let mut list = Vec::new();
                Self::find_leaves(
                    self.exp1(),
                    Kind::RegexpUnion,
                    &mut list,
                    automata,
                    automaton_provider,
                )?;
                Self::find_leaves(
                    self.exp2(),
                    Kind::RegexpUnion,
                    &mut list,
                    automata,
                    automaton_provider,
                )?;
                Operations::union(&list)
            }
            Kind::RegexpConcatenation => {
                let mut list = Vec::new();
                Self::find_leaves(
                    self.exp1(),
                    Kind::RegexpConcatenation,
                    &mut list,
                    automata,
                    automaton_provider,
                )?;
                Self::find_leaves(
                    self.exp2(),
                    Kind::RegexpConcatenation,
                    &mut list,
                    automata,
                    automaton_provider,
                )?;
                Operations::concatenate(&list)
            }
            Kind::RegexpIntersection => Operations::intersection(
                &self
                    .exp1()
                    .to_automaton_with(automata, automaton_provider)?,
                &self
                    .exp2()
                    .to_automaton_with(automata, automaton_provider)?,
            ),
            Kind::RegexpOptional => Operations::optional(
                &self
                    .exp1()
                    .to_automaton_with(automata, automaton_provider)?,
            ),
            Kind::RegexpRepeat => Operations::repeat(
                &self
                    .exp1()
                    .to_automaton_with(automata, automaton_provider)?,
            ),
            Kind::RegexpRepeatMin => {
                let a = self
                    .exp1()
                    .to_automaton_with(automata, automaton_provider)?;
                Operations::repeat_min(&a, self.min)
            }
            Kind::RegexpRepeatMinmax => {
                let a = self
                    .exp1()
                    .to_automaton_with(automata, automaton_provider)?;
                Operations::repeat_min_max(&a, self.min, self.max)
            }
            Kind::RegexpComplement => {
                // we don't support arbitrary complement, just "negated character
                // class": this is just a list of characters (e.g. "a") or ranges
                // (e.g. "b-d")
                let a = self
                    .exp1()
                    .to_automaton_with(automata, automaton_provider)?;
                Operations::complement(&a, i32::MAX)?
            }
            Kind::RegexpDeprecatedComplement => {
                // to ease transitions for users only, support arbitrary complement
                // but bounded by DEFAULT_DETERMINIZE_WORK_LIMIT: must not be
                // configurable.
                let a = self
                    .exp1()
                    .to_automaton_with(automata, automaton_provider)?;
                Operations::complement(&a, DEFAULT_DETERMINIZE_WORK_LIMIT)?
            }
            Kind::RegexpChar => {
                if self.check(Self::ASCII_CASE_INSENSITIVE | Self::CASE_INSENSITIVE) {
                    Automata::make_case_insensitive_char(self.c)
                } else {
                    Automata::make_char(self.c)
                }
            }
            Kind::RegexpCharRange => {
                Automata::make_char_range(self.range_from(0), self.range_to(0))
            }
            Kind::RegexpCharClass => Automata::make_char_class(self.froms(), self.tos()),
            Kind::RegexpAnychar => Automata::make_any_char(),
            Kind::RegexpEmpty => Automata::make_empty(),
            Kind::RegexpString => {
                let s = self.string();
                if self.check(Self::ASCII_CASE_INSENSITIVE | Self::CASE_INSENSITIVE) {
                    Automata::make_case_insensitive_string(s)
                } else {
                    Automata::make_string(s)
                }
            }
            Kind::RegexpAnystring => Automata::make_any_string(),
            Kind::RegexpAutomaton => {
                let name = self.string();
                let mut aa = automata.and_then(|m| m.get(name)).cloned();
                if aa.is_none() {
                    if let Some(provider) = automaton_provider {
                        aa = Some(provider.get_automaton(name)?);
                    }
                }
                match aa {
                    Some(aa) => aa,
                    None => {
                        return Err(LuceneError::IllegalArgument(format!(
                            "'{}' not found",
                            name
                        )))
                    }
                }
            }
            Kind::RegexpInterval => {
                Automata::make_decimal_interval(self.min, self.max, self.digits)?
            }
        };
        Ok(a)
    }

    fn exp1(&self) -> &RegExp {
        self.exp1
            .as_deref()
            .expect("INVARIANT: this kind always carries exp1")
    }

    fn exp2(&self) -> &RegExp {
        self.exp2
            .as_deref()
            .expect("INVARIANT: this kind always carries exp2")
    }

    fn string(&self) -> &str {
        self.s
            .as_deref()
            .expect("INVARIANT: this kind always carries a string")
    }

    fn froms(&self) -> &[i32] {
        self.from
            .as_deref()
            .expect("INVARIANT: this kind always carries range starts")
    }

    fn tos(&self) -> &[i32] {
        self.to
            .as_deref()
            .expect("INVARIANT: this kind always carries range ends")
    }

    fn range_from(&self, i: usize) -> i32 {
        self.froms()[i]
    }

    fn range_to(&self, i: usize) -> i32 {
        self.tos()[i]
    }

    fn check(&self, flag: i32) -> bool {
        (self.flags & flag) != 0
    }

    /// Uses the Unicode spec to generate case-insensitive alternates.
    ///
    /// See [`RegExp::CASE_INSENSITIVE`] for details on case folding within the
    /// Unicode spec.
    fn to_case_insensitive_char(codepoint: i32) -> Vec<i32> {
        let mut list: Vec<i32> = Vec::new();
        CaseFolding::expand(codepoint, |variant| list.push(variant));
        list.sort_unstable();
        list
    }

    /// Expands a range to include case-insensitive matches.
    ///
    /// This is expensive: a case-insensitive range involves iterating over the range
    /// space, adding alternatives. Jump on the grenade here, containing the CPU and
    /// memory explosion just to this method, activated by an optional flag.
    fn expand_case_insensitive_range(
        start: i32,
        end: i32,
        range_starts: &mut Vec<i32>,
        range_ends: &mut Vec<i32>,
    ) -> Result<()> {
        if start > end {
            return Err(LuceneError::IllegalArgument(format!(
                "invalid range: from ({}) cannot be > to ({})",
                start, end
            )));
        }

        // contain the explosion of transitions by using a throwaway state
        let mut scratch = Automaton::new();
        let state = scratch.create_state();

        // iterate over the range, adding the codepoint and any alternatives as
        // transitions
        for i in start..=end {
            let mut variants: Vec<i32> = Vec::new();
            CaseFolding::expand(i, |ch| variants.push(ch));
            for ch in variants {
                scratch.add_transition(state, state, ch);
            }
        }

        // compress transitions
        scratch.finish_state();

        // add compressed ranges to the list
        let mut transition = Transition::new();
        let num_transitions = scratch.init_transition(state, &mut transition);
        for _ in 0..num_transitions {
            scratch.get_next_transition(&mut transition);
            range_starts.push(transition.min);
            range_ends.push(transition.max);
        }
        Ok(())
    }

    fn find_leaves(
        exp: &RegExp,
        kind: Kind,
        list: &mut Vec<Automaton>,
        automata: Option<&HashMap<String, Automaton>>,
        automaton_provider: Option<&dyn AutomatonProvider>,
    ) -> Result<()> {
        if exp.kind == kind {
            Self::find_leaves(exp.exp1(), kind, list, automata, automaton_provider)?;
            Self::find_leaves(exp.exp2(), kind, list, automata, automaton_provider)?;
        } else {
            list.push(exp.to_automaton_with(automata, automaton_provider)?);
        }
        Ok(())
    }

    /// The string that was used to construct this regex. Compare to
    /// its [`Display`](std::fmt::Display) form.
    ///
    /// Only the root node carries it; sub-expressions return `None`.
    pub fn get_original_string(&self) -> Option<&str> {
        self.original_string.as_deref()
    }

    fn to_string_builder(&self, b: &mut String) {
        match self.kind {
            Kind::RegexpUnion => {
                b.push('(');
                self.exp1().to_string_builder(b);
                b.push('|');
                self.exp2().to_string_builder(b);
                b.push(')');
            }
            Kind::RegexpConcatenation => {
                self.exp1().to_string_builder(b);
                self.exp2().to_string_builder(b);
            }
            Kind::RegexpIntersection => {
                b.push('(');
                self.exp1().to_string_builder(b);
                b.push('&');
                self.exp2().to_string_builder(b);
                b.push(')');
            }
            Kind::RegexpOptional => {
                b.push('(');
                self.exp1().to_string_builder(b);
                b.push_str(")?");
            }
            Kind::RegexpRepeat => {
                b.push('(');
                self.exp1().to_string_builder(b);
                b.push_str(")*");
            }
            Kind::RegexpRepeatMin => {
                b.push('(');
                self.exp1().to_string_builder(b);
                b.push_str(&format!("){{{},}}", self.min));
            }
            Kind::RegexpRepeatMinmax => {
                b.push('(');
                self.exp1().to_string_builder(b);
                b.push_str(&format!("){{{},{}}}", self.min, self.max));
            }
            Kind::RegexpComplement | Kind::RegexpDeprecatedComplement => {
                b.push_str("~(");
                self.exp1().to_string_builder(b);
                b.push(')');
            }
            Kind::RegexpChar => {
                b.push('\\');
                append_code_point(b, self.c);
            }
            Kind::RegexpCharRange => {
                b.push_str("[\\");
                append_code_point(b, self.range_from(0));
                b.push_str("-\\");
                append_code_point(b, self.range_to(0));
                b.push(']');
            }
            Kind::RegexpCharClass => {
                b.push('[');
                for i in 0..self.froms().len() {
                    if self.range_from(i) == self.range_to(i) {
                        b.push('\\');
                        append_code_point(b, self.range_from(i));
                    } else {
                        b.push('\\');
                        append_code_point(b, self.range_from(i));
                        b.push_str("-\\");
                        append_code_point(b, self.range_to(i));
                    }
                }
                b.push(']');
            }
            Kind::RegexpAnychar => b.push('.'),
            Kind::RegexpEmpty => b.push('#'),
            Kind::RegexpString => {
                b.push('"');
                b.push_str(self.string());
                b.push('"');
            }
            Kind::RegexpAnystring => b.push('@'),
            Kind::RegexpAutomaton => {
                b.push('<');
                b.push_str(self.string());
                b.push('>');
            }
            Kind::RegexpInterval => {
                let s1 = self.min.to_string();
                let s2 = self.max.to_string();
                b.push('<');
                if self.digits > 0 {
                    for _ in s1.len() as i32..self.digits {
                        b.push('0');
                    }
                }
                b.push_str(&s1);
                b.push('-');
                if self.digits > 0 {
                    for _ in s2.len() as i32..self.digits {
                        b.push('0');
                    }
                }
                b.push_str(&s2);
                b.push('>');
            }
        }
    }

    /// Like the [`Display`](std::fmt::Display) form, but more verbose (shows the hierarchy more
    /// clearly).
    pub fn to_string_tree(&self) -> String {
        let mut b = String::new();
        self.to_string_tree_indented(&mut b, "");
        b
    }

    fn to_string_tree_indented(&self, b: &mut String, indent: &str) {
        match self.kind {
            // binary
            Kind::RegexpUnion | Kind::RegexpConcatenation | Kind::RegexpIntersection => {
                b.push_str(indent);
                b.push_str(&self.kind.to_string());
                b.push('\n');
                let inner = format!("{}  ", indent);
                self.exp1().to_string_tree_indented(b, &inner);
                self.exp2().to_string_tree_indented(b, &inner);
            }
            // unary
            Kind::RegexpOptional
            | Kind::RegexpRepeat
            | Kind::RegexpComplement
            | Kind::RegexpDeprecatedComplement => {
                b.push_str(indent);
                b.push_str(&self.kind.to_string());
                b.push('\n');
                let inner = format!("{}  ", indent);
                self.exp1().to_string_tree_indented(b, &inner);
            }
            Kind::RegexpRepeatMin => {
                b.push_str(indent);
                b.push_str(&self.kind.to_string());
                b.push_str(" min=");
                b.push_str(&self.min.to_string());
                b.push('\n');
                let inner = format!("{}  ", indent);
                self.exp1().to_string_tree_indented(b, &inner);
            }
            Kind::RegexpRepeatMinmax => {
                b.push_str(indent);
                b.push_str(&self.kind.to_string());
                b.push_str(" min=");
                b.push_str(&self.min.to_string());
                b.push_str(" max=");
                b.push_str(&self.max.to_string());
                b.push('\n');
                let inner = format!("{}  ", indent);
                self.exp1().to_string_tree_indented(b, &inner);
            }
            Kind::RegexpChar => {
                b.push_str(indent);
                b.push_str(&self.kind.to_string());
                b.push_str(" char=");
                append_code_point(b, self.c);
                b.push('\n');
            }
            Kind::RegexpCharRange => {
                b.push_str(indent);
                b.push_str(&self.kind.to_string());
                b.push_str(" from=");
                append_code_point(b, self.range_from(0));
                b.push_str(" to=");
                append_code_point(b, self.range_to(0));
                b.push('\n');
            }
            Kind::RegexpCharClass => {
                b.push_str(indent);
                b.push_str(&self.kind.to_string());
                b.push_str(" starts=");
                b.push_str(&to_hex_string(self.froms()));
                b.push_str(" ends=");
                b.push_str(&to_hex_string(self.tos()));
                b.push('\n');
            }
            Kind::RegexpAnychar | Kind::RegexpEmpty => {
                b.push_str(indent);
                b.push_str(&self.kind.to_string());
                b.push('\n');
            }
            Kind::RegexpString => {
                b.push_str(indent);
                b.push_str(&self.kind.to_string());
                b.push_str(" string=");
                b.push_str(self.string());
                b.push('\n');
            }
            Kind::RegexpAnystring => {
                b.push_str(indent);
                b.push_str(&self.kind.to_string());
                b.push('\n');
            }
            Kind::RegexpAutomaton => {
                b.push_str(indent);
                b.push_str(&self.kind.to_string());
                b.push('\n');
            }
            Kind::RegexpInterval => {
                b.push_str(indent);
                b.push_str(&self.kind.to_string());
                let s1 = self.min.to_string();
                let s2 = self.max.to_string();
                b.push('<');
                if self.digits > 0 {
                    for _ in s1.len() as i32..self.digits {
                        b.push('0');
                    }
                }
                b.push_str(&s1);
                b.push('-');
                if self.digits > 0 {
                    for _ in s2.len() as i32..self.digits {
                        b.push('0');
                    }
                }
                b.push_str(&s2);
                b.push('>');
                b.push('\n');
            }
        }
    }

    /// Returns the set of automaton identifiers that occur in this regular
    /// expression.
    pub fn get_identifiers(&self) -> HashSet<String> {
        let mut set = HashSet::new();
        self.collect_identifiers(&mut set);
        set
    }

    fn collect_identifiers(&self, set: &mut HashSet<String>) {
        match self.kind {
            Kind::RegexpUnion | Kind::RegexpConcatenation | Kind::RegexpIntersection => {
                self.exp1().collect_identifiers(set);
                self.exp2().collect_identifiers(set);
            }
            Kind::RegexpOptional
            | Kind::RegexpRepeat
            | Kind::RegexpRepeatMin
            | Kind::RegexpRepeatMinmax
            | Kind::RegexpComplement
            | Kind::RegexpDeprecatedComplement => {
                self.exp1().collect_identifiers(set);
            }
            Kind::RegexpAutomaton => {
                set.insert(self.string().to_string());
            }
            _ => {}
        }
    }

    // -----------------------------------------------------------------------
    // Node factories
    // -----------------------------------------------------------------------

    fn make_union(flags: i32, exp1: RegExp, exp2: RegExp) -> RegExp {
        Self::new_container_node(flags, Kind::RegexpUnion, Some(exp1), Some(exp2))
    }

    fn make_concatenation(flags: i32, exp1: RegExp, exp2: RegExp) -> RegExp {
        let is_char_or_string =
            |e: &RegExp| e.kind == Kind::RegexpChar || e.kind == Kind::RegexpString;
        if is_char_or_string(&exp1) && is_char_or_string(&exp2) {
            return Self::make_string_pair(flags, &exp1, &exp2);
        }
        let (rexp1, rexp2);
        if exp1.kind == Kind::RegexpConcatenation
            && is_char_or_string(exp1.exp2())
            && is_char_or_string(&exp2)
        {
            rexp2 = Self::make_string_pair(flags, exp1.exp2(), &exp2);
            rexp1 = exp1.exp1().clone();
        } else if is_char_or_string(&exp1)
            && exp2.kind == Kind::RegexpConcatenation
            && is_char_or_string(exp2.exp1())
        {
            rexp1 = Self::make_string_pair(flags, &exp1, exp2.exp1());
            rexp2 = exp2.exp2().clone();
        } else {
            rexp1 = exp1;
            rexp2 = exp2;
        }
        Self::new_container_node(flags, Kind::RegexpConcatenation, Some(rexp1), Some(rexp2))
    }

    fn make_string_pair(flags: i32, exp1: &RegExp, exp2: &RegExp) -> RegExp {
        let mut b = String::new();
        if exp1.kind == Kind::RegexpString {
            b.push_str(exp1.string());
        } else {
            append_code_point(&mut b, exp1.c);
        }
        if exp2.kind == Kind::RegexpString {
            b.push_str(exp2.string());
        } else {
            append_code_point(&mut b, exp2.c);
        }
        Self::make_string(flags, b)
    }

    fn make_intersection(flags: i32, exp1: RegExp, exp2: RegExp) -> RegExp {
        Self::new_container_node(flags, Kind::RegexpIntersection, Some(exp1), Some(exp2))
    }

    fn make_optional(flags: i32, exp: RegExp) -> RegExp {
        Self::new_container_node(flags, Kind::RegexpOptional, Some(exp), None)
    }

    fn make_repeat(flags: i32, exp: RegExp) -> RegExp {
        Self::new_container_node(flags, Kind::RegexpRepeat, Some(exp), None)
    }

    fn make_repeat_min(flags: i32, exp: RegExp, min: i32) -> RegExp {
        Self::new_repeating_node(flags, Kind::RegexpRepeatMin, exp, min, 0)
    }

    fn make_repeat_min_max(flags: i32, exp: RegExp, min: i32, max: i32) -> RegExp {
        Self::new_repeating_node(flags, Kind::RegexpRepeatMinmax, exp, min, max)
    }

    fn make_complement(flags: i32, exp: RegExp) -> RegExp {
        Self::new_container_node(flags, Kind::RegexpComplement, Some(exp), None)
    }

    /// Creates a node that will compute the complement of an arbitrary expression.
    ///
    /// Deprecated: will be removed in Lucene 11.
    fn make_deprecated_complement(flags: i32, exp: RegExp) -> RegExp {
        Self::new_container_node(flags, Kind::RegexpDeprecatedComplement, Some(exp), None)
    }

    fn make_char(flags: i32, c: i32) -> RegExp {
        Self::new_leaf_node(flags, Kind::RegexpChar, None, c, 0, 0, 0, None, None)
    }

    fn make_char_range(flags: i32, from: i32, to: i32) -> Result<RegExp> {
        if from > to {
            return Err(LuceneError::IllegalArgument(format!(
                "invalid range: from ({}) cannot be > to ({})",
                from, to
            )));
        }
        Ok(Self::new_leaf_node(
            flags,
            Kind::RegexpCharRange,
            None,
            0,
            0,
            0,
            0,
            Some(vec![from]),
            Some(vec![to]),
        ))
    }

    fn make_char_class(flags: i32, from: Vec<i32>, to: Vec<i32>) -> Result<RegExp> {
        if from.len() != to.len() {
            return Err(LuceneError::IllegalState(format!(
                "invalid class: from.length ({}) != to.length ({})",
                from.len(),
                to.len()
            )));
        }
        for i in 0..from.len() {
            if from[i] > to[i] {
                return Err(LuceneError::IllegalArgument(format!(
                    "invalid range: from ({}) cannot be > to ({})",
                    from[i], to[i]
                )));
            }
        }
        Ok(Self::new_leaf_node(
            flags,
            Kind::RegexpCharClass,
            None,
            0,
            0,
            0,
            0,
            Some(from),
            Some(to),
        ))
    }

    fn make_any_char(flags: i32) -> RegExp {
        Self::new_container_node(flags, Kind::RegexpAnychar, None, None)
    }

    fn make_empty(flags: i32) -> RegExp {
        Self::new_container_node(flags, Kind::RegexpEmpty, None, None)
    }

    fn make_string(flags: i32, s: String) -> RegExp {
        Self::new_leaf_node(flags, Kind::RegexpString, Some(s), 0, 0, 0, 0, None, None)
    }

    fn make_any_string(flags: i32) -> RegExp {
        Self::new_container_node(flags, Kind::RegexpAnystring, None, None)
    }

    fn make_automaton(flags: i32, s: String) -> RegExp {
        Self::new_leaf_node(
            flags,
            Kind::RegexpAutomaton,
            Some(s),
            0,
            0,
            0,
            0,
            None,
            None,
        )
    }

    fn make_interval(flags: i32, min: i32, max: i32, digits: i32) -> RegExp {
        Self::new_leaf_node(
            flags,
            Kind::RegexpInterval,
            None,
            0,
            min,
            max,
            digits,
            None,
            None,
        )
    }
}

impl std::fmt::Display for RegExp {
    /// Constructs a string from the parsed regular expression.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut b = String::new();
        self.to_string_builder(&mut b);
        f.write_str(&b)
    }
}

/// Prints like `[U+002A U+FD72 U+1FFFF]`.
fn to_hex_string(range: &[i32]) -> String {
    let mut sb = String::new();
    sb.push('[');
    for codepoint in range {
        if sb.len() > 1 {
            sb.push(' ');
        }
        sb.push_str(&format!("U+{:04X}", codepoint));
    }
    sb.push(']');
    sb
}

/// Appends a code point to `b`, the way Java's `StringBuilder.appendCodePoint` does.
///
/// A code point that is not a Unicode scalar value (an unpaired surrogate) cannot be
/// represented in a Rust `String`; it is rendered as the replacement character.
fn append_code_point(b: &mut String, c: i32) {
    match u32::try_from(c).ok().and_then(char::from_u32) {
        Some(ch) => b.push(ch),
        None => b.push(char::REPLACEMENT_CHARACTER),
    }
}

// -----------------------------------------------------------------------------
// Parser
// -----------------------------------------------------------------------------

/// The mutable parsing state of a [`RegExp`].
///
/// Lucene keeps `pos` on the `RegExp` instance itself; Rust prefers a separate
/// parser so the produced node can stay immutable.
struct Parser {
    /// The original string, as UTF-16 code units, so positions match Java's.
    input: Vec<u16>,
    pos: usize,
    flags: i32,
}

impl Parser {
    fn new(s: &str, flags: i32) -> Self {
        Self {
            input: s.encode_utf16().collect(),
            pos: 0,
            flags,
        }
    }

    fn code_point_at(&self, pos: usize) -> i32 {
        let unit = self.input[pos];
        if (0xD800..0xDC00).contains(&unit) && pos + 1 < self.input.len() {
            let low = self.input[pos + 1];
            if (0xDC00..0xE000).contains(&low) {
                return 0x10000
                    + (((u32::from(unit) - 0xD800) << 10) | (u32::from(low) - 0xDC00)) as i32;
            }
        }
        i32::from(unit)
    }

    fn char_count(c: i32) -> usize {
        if c >= 0x10000 {
            2
        } else {
            1
        }
    }

    fn substring(&self, start: usize, end: usize) -> String {
        String::from_utf16_lossy(&self.input[start..end])
    }

    fn peek(&self, s: &str) -> bool {
        self.more() && s.chars().any(|c| c as i32 == self.code_point_at(self.pos))
    }

    fn match_ch(&mut self, c: i32) -> bool {
        if self.pos >= self.input.len() {
            return false;
        }
        if self.code_point_at(self.pos) == c {
            self.pos += Self::char_count(c);
            return true;
        }
        false
    }

    fn more(&self) -> bool {
        self.pos < self.input.len()
    }

    fn next(&mut self) -> Result<i32> {
        if !self.more() {
            return Err(LuceneError::IllegalArgument(
                "unexpected end-of-string".to_string(),
            ));
        }
        let ch = self.code_point_at(self.pos);
        self.pos += Self::char_count(ch);
        Ok(ch)
    }

    fn check(&self, flag: i32) -> bool {
        (self.flags & flag) != 0
    }

    fn parse_union_exp(&mut self) -> Result<RegExp> {
        self.iterative_parse_exp(
            Self::parse_inter_exp,
            |p| p.match_ch('|' as i32),
            RegExp::make_union,
        )
    }

    fn parse_inter_exp(&mut self) -> Result<RegExp> {
        self.iterative_parse_exp(
            Self::parse_concat_exp,
            |p| p.check(RegExp::INTERSECTION) && p.match_ch('&' as i32),
            RegExp::make_intersection,
        )
    }

    fn parse_concat_exp(&mut self) -> Result<RegExp> {
        self.iterative_parse_exp(
            Self::parse_repeat_exp,
            |p| p.more() && !p.peek(")|") && (!p.check(RegExp::INTERSECTION) || !p.peek("&")),
            RegExp::make_concatenation,
        )
    }

    fn iterative_parse_exp(
        &mut self,
        gather: fn(&mut Self) -> Result<RegExp>,
        stop: fn(&mut Self) -> bool,
        associative_reduce: fn(i32, RegExp, RegExp) -> RegExp,
    ) -> Result<RegExp> {
        let mut result = gather(self)?;
        while stop(self) {
            let e = gather(self)?;
            result = associative_reduce(self.flags, result, e);
        }
        Ok(result)
    }

    fn parse_repeat_exp(&mut self) -> Result<RegExp> {
        let mut e = self.parse_compl_exp()?;
        while self.peek("?*+{") {
            if self.match_ch('?' as i32) {
                e = RegExp::make_optional(self.flags, e);
            } else if self.match_ch('*' as i32) {
                e = RegExp::make_repeat(self.flags, e);
            } else if self.match_ch('+' as i32) {
                e = RegExp::make_repeat_min(self.flags, e, 1);
            } else if self.match_ch('{' as i32) {
                let mut start = self.pos;
                while self.peek("0123456789") {
                    self.next()?;
                }
                if start == self.pos {
                    return Err(LuceneError::IllegalArgument(format!(
                        "integer expected at position {}",
                        self.pos
                    )));
                }
                let n = parse_int(&self.substring(start, self.pos))?;
                let mut m = -1i32;
                if self.match_ch(',' as i32) {
                    start = self.pos;
                    while self.peek("0123456789") {
                        self.next()?;
                    }
                    if start != self.pos {
                        m = parse_int(&self.substring(start, self.pos))?;
                    }
                } else {
                    m = n;
                }
                if !self.match_ch('}' as i32) {
                    return Err(LuceneError::IllegalArgument(format!(
                        "expected '}}' at position {}",
                        self.pos
                    )));
                }
                if m != -1 && n > m {
                    return Err(LuceneError::IllegalArgument(format!(
                        "invalid repetition range(out of order): {}..{}",
                        n, m
                    )));
                }
                if m == -1 {
                    e = RegExp::make_repeat_min(self.flags, e, n);
                } else {
                    e = RegExp::make_repeat_min_max(self.flags, e, n, m);
                }
            }
        }
        Ok(e)
    }

    fn parse_compl_exp(&mut self) -> Result<RegExp> {
        if self.check(RegExp::DEPRECATED_COMPLEMENT) && self.match_ch('~' as i32) {
            let inner = self.parse_compl_exp()?;
            Ok(RegExp::make_deprecated_complement(self.flags, inner))
        } else {
            self.parse_char_class_exp()
        }
    }

    fn parse_char_class_exp(&mut self) -> Result<RegExp> {
        if self.match_ch('[' as i32) {
            let mut negate = false;
            if self.match_ch('^' as i32) {
                negate = true;
            }
            let mut e = self.parse_char_classes()?;
            if negate {
                e = RegExp::make_intersection(
                    self.flags,
                    RegExp::make_any_char(self.flags),
                    RegExp::make_complement(self.flags, e),
                );
            }
            if !self.match_ch(']' as i32) {
                return Err(LuceneError::IllegalArgument(format!(
                    "expected ']' at position {}",
                    self.pos
                )));
            }
            Ok(e)
        } else {
            self.parse_simple_exp()
        }
    }

    fn parse_char_classes(&mut self) -> Result<RegExp> {
        let mut starts: Vec<i32> = Vec::new();
        let mut ends: Vec<i32> = Vec::new();

        loop {
            // look for an escape
            if self.match_ch('\\' as i32) {
                if self.peek("\\ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz") {
                    // special "escape" or invalid escape
                    self.expand_pre_defined(&mut starts, &mut ends)?;
                } else {
                    // escaped character, don't parse it
                    let c = self.next()?;
                    starts.push(c);
                    ends.push(c);
                }
            } else {
                // parse a character
                let c = self.parse_char_exp()?;

                if self.match_ch('-' as i32) {
                    if self.check(RegExp::CASE_INSENSITIVE_RANGE) {
                        let end = self.parse_char_exp()?;
                        RegExp::expand_case_insensitive_range(c, end, &mut starts, &mut ends)?;
                    } else {
                        // simple range from c-d
                        starts.push(c);
                        ends.push(self.parse_char_exp()?);
                    }
                } else if self.check(RegExp::ASCII_CASE_INSENSITIVE | RegExp::CASE_INSENSITIVE) {
                    // single case-insensitive character
                    for form in RegExp::to_case_insensitive_char(c) {
                        starts.push(form);
                        ends.push(form);
                    }
                } else {
                    // single character
                    starts.push(c);
                    ends.push(c);
                }
            }

            if !self.more() || self.peek("]") {
                break;
            }
        }

        // not sure why we bother optimizing nodes, same automaton...
        // definitely saves time vs fixing toString()-based tests.
        if starts.len() == 1 {
            if starts[0] == ends[0] {
                Ok(RegExp::make_char(self.flags, starts[0]))
            } else {
                RegExp::make_char_range(self.flags, starts[0], ends[0])
            }
        } else {
            RegExp::make_char_class(self.flags, starts, ends)
        }
    }

    fn expand_pre_defined(&mut self, starts: &mut Vec<i32>, ends: &mut Vec<i32>) -> Result<()> {
        if self.peek("\\") {
            // escape
            starts.push('\\' as i32);
            ends.push('\\' as i32);
            self.next()?;
        } else if self.peek("d") {
            // digit: [0-9]
            starts.push('0' as i32);
            ends.push('9' as i32);
            self.next()?;
        } else if self.peek("D") {
            // non-digit: [^0-9]
            starts.push(MIN_CODE_POINT);
            ends.push('0' as i32 - 1);
            starts.push('9' as i32 + 1);
            ends.push(MAX_CODE_POINT);
            self.next()?;
        } else if self.peek("s") {
            // whitespace: [\t-\n\r ]
            starts.push('\t' as i32);
            ends.push('\n' as i32);
            starts.push('\r' as i32);
            ends.push('\r' as i32);
            starts.push(' ' as i32);
            ends.push(' ' as i32);
            self.next()?;
        } else if self.peek("S") {
            // non-whitespace: [^\t-\n\r ]
            starts.push(MIN_CODE_POINT);
            ends.push('\t' as i32 - 1);
            starts.push('\n' as i32 + 1);
            ends.push('\r' as i32 - 1);
            starts.push('\r' as i32 + 1);
            ends.push(' ' as i32 - 1);
            starts.push(' ' as i32 + 1);
            ends.push(MAX_CODE_POINT);
            self.next()?;
        } else if self.peek("w") {
            // word: [0-9A-Z_a-z]
            starts.push('0' as i32);
            ends.push('9' as i32);
            starts.push('A' as i32);
            ends.push('Z' as i32);
            starts.push('_' as i32);
            ends.push('_' as i32);
            starts.push('a' as i32);
            ends.push('z' as i32);
            self.next()?;
        } else if self.peek("W") {
            // non-word: [^0-9A-Z_a-z]
            starts.push(MIN_CODE_POINT);
            ends.push('0' as i32 - 1);
            starts.push('9' as i32 + 1);
            ends.push('A' as i32 - 1);
            starts.push('Z' as i32 + 1);
            ends.push('_' as i32 - 1);
            starts.push('_' as i32 + 1);
            ends.push('a' as i32 - 1);
            starts.push('z' as i32 + 1);
            ends.push(MAX_CODE_POINT);
            self.next()?;
        } else if self.peek("abcefghijklmnopqrtuvxyz") || self.peek("ABCEFGHIJKLMNOPQRTUVXYZ") {
            // From the java.util.regex.Pattern javadoc: "It is an error to use a
            // backslash prior to any alphabetic character that does not denote an
            // escaped construct".
            // Lucene concatenates the raw `int` returned by next(), so the message
            // carries the code point as a decimal number rather than as a character.
            let c = self.next()?;
            return Err(LuceneError::IllegalArgument(format!(
                "invalid character class \\{}",
                c
            )));
        }
        Ok(())
    }

    fn match_predefined_character_class(&mut self) -> Result<Option<RegExp>> {
        // See the java.util.regex predefined character classes.
        if self.match_ch('\\' as i32)
            && self.peek("\\ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz")
        {
            let mut starts = Vec::new();
            let mut ends = Vec::new();
            self.expand_pre_defined(&mut starts, &mut ends)?;
            return RegExp::make_char_class(self.flags, starts, ends).map(Some);
        }

        Ok(None)
    }

    fn parse_simple_exp(&mut self) -> Result<RegExp> {
        if self.match_ch('.' as i32) {
            Ok(RegExp::make_any_char(self.flags))
        } else if self.check(RegExp::EMPTY) && self.match_ch('#' as i32) {
            Ok(RegExp::make_empty(self.flags))
        } else if self.check(RegExp::ANYSTRING) && self.match_ch('@' as i32) {
            Ok(RegExp::make_any_string(self.flags))
        } else if self.match_ch('"' as i32) {
            let start = self.pos;
            while self.more() && !self.peek("\"") {
                self.next()?;
            }
            if !self.match_ch('"' as i32) {
                return Err(LuceneError::IllegalArgument(format!(
                    "expected '\"' at position {}",
                    self.pos
                )));
            }
            Ok(RegExp::make_string(
                self.flags,
                self.substring(start, self.pos - 1),
            ))
        } else if self.match_ch('(' as i32) {
            if self.match_ch(')' as i32) {
                return Ok(RegExp::make_string(self.flags, String::new()));
            }
            let e = self.parse_union_exp()?;
            if !self.match_ch(')' as i32) {
                return Err(LuceneError::IllegalArgument(format!(
                    "expected ')' at position {}",
                    self.pos
                )));
            }
            Ok(e)
        } else if (self.check(RegExp::AUTOMATON) || self.check(RegExp::INTERVAL))
            && self.match_ch('<' as i32)
        {
            let start = self.pos;
            while self.more() && !self.peek(">") {
                self.next()?;
            }
            if !self.match_ch('>' as i32) {
                return Err(LuceneError::IllegalArgument(format!(
                    "expected '>' at position {}",
                    self.pos
                )));
            }
            let s = self.substring(start, self.pos - 1);
            match s.find('-') {
                None => {
                    if !self.check(RegExp::AUTOMATON) {
                        return Err(LuceneError::IllegalArgument(format!(
                            "interval syntax error at position {}",
                            self.pos - 1
                        )));
                    }
                    Ok(RegExp::make_automaton(self.flags, s))
                }
                Some(i) => {
                    if !self.check(RegExp::INTERVAL) {
                        return Err(LuceneError::IllegalArgument(format!(
                            "illegal identifier at position {}",
                            self.pos - 1
                        )));
                    }
                    let interval_error = || {
                        LuceneError::IllegalArgument(format!(
                            "interval syntax error at position {}",
                            self.pos - 1
                        ))
                    };
                    if i == 0 || i == s.len() - 1 || Some(i) != s.rfind('-') {
                        return Err(interval_error());
                    }
                    let smin = &s[0..i];
                    let smax = &s[i + 1..];
                    let mut imin = parse_int(smin).map_err(|_| interval_error())?;
                    let mut imax = parse_int(smax).map_err(|_| interval_error())?;
                    let digits = if smin.len() == smax.len() {
                        smin.len() as i32
                    } else {
                        0
                    };
                    if imin > imax {
                        std::mem::swap(&mut imin, &mut imax);
                    }
                    Ok(RegExp::make_interval(self.flags, imin, imax, digits))
                }
            }
        } else {
            if let Some(predefined) = self.match_predefined_character_class()? {
                return Ok(predefined);
            }
            let c = self.parse_char_exp()?;
            Ok(RegExp::make_char(self.flags, c))
        }
    }

    fn parse_char_exp(&mut self) -> Result<i32> {
        self.match_ch('\\' as i32);
        self.next()
    }
}

/// Parses a decimal integer the way `Integer.parseInt` does; a `NumberFormatException`
/// in Java is an `IllegalArgumentException`, so it maps to the same error here.
fn parse_int(s: &str) -> Result<i32> {
    s.parse::<i32>()
        .map_err(|e| LuceneError::IllegalArgument(format!("For input string: \"{}\": {}", s, e)))
}
