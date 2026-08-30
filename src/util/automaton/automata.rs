//! Port of `org.apache.lucene.util.automaton.Automata`.

use crate::error::{LuceneError, Result};
use crate::util::{BytesRef, StringHelper};

use super::automaton::{Automaton, AutomatonBuilder, MAX_CODE_POINT, MIN_CODE_POINT};
use super::case_folding::CaseFolding;
use super::operations::Operations;
use super::strings_to_automaton::StringsToAutomaton;

/// [`Automata::make_string_union`] limits terms to this maximum length to ensure
/// the stack does not overflow while building, since the algorithm relies on
/// recursion.
pub const MAX_STRING_UNION_TERM_LENGTH: usize = 1000;

/// Construction of basic automata.
///
/// Equivalent to `org.apache.lucene.util.automaton.Automata`.
pub struct Automata;

impl Automata {
    /// Returns a new (deterministic) automaton with the empty language.
    pub fn make_empty() -> Automaton {
        let mut a = Automaton::new();
        a.finish_state();
        a
    }

    /// Returns a new (deterministic) automaton that accepts only the empty string.
    pub fn make_empty_string() -> Automaton {
        let mut a = Automaton::new();
        a.create_state();
        a.set_accept(0, true);
        a
    }

    /// Returns a new (deterministic) automaton that accepts all strings.
    pub fn make_any_string() -> Automaton {
        let mut a = Automaton::new();
        let s = a.create_state();
        a.set_accept(s, true);
        a.add_transition_range(s, s, MIN_CODE_POINT, MAX_CODE_POINT);
        a.finish_state();
        a
    }

    /// Returns a new (deterministic) automaton that accepts all binary terms.
    pub fn make_any_binary() -> Automaton {
        let mut a = Automaton::new();
        let s = a.create_state();
        a.set_accept(s, true);
        a.add_transition_range(s, s, 0, 255);
        a.finish_state();
        a
    }

    /// Returns a new (deterministic) automaton that accepts all binary terms except
    /// the empty string.
    pub fn make_non_empty_binary() -> Automaton {
        let mut a = Automaton::new();
        let s1 = a.create_state();
        let s2 = a.create_state();
        a.set_accept(s2, true);
        a.add_transition_range(s1, s2, 0, 255);
        a.add_transition_range(s2, s2, 0, 255);
        a.finish_state();
        a
    }

    /// Returns a new (deterministic) automaton that accepts any single codepoint.
    pub fn make_any_char() -> Automaton {
        Self::make_char_range(MIN_CODE_POINT, MAX_CODE_POINT)
    }

    /// Accepts any single character starting from the specified state, returning
    /// the new state.
    pub fn append_any_char(a: &mut Automaton, state: i32) -> i32 {
        let new_state = a.create_state();
        a.add_transition_range(state, new_state, MIN_CODE_POINT, MAX_CODE_POINT);
        new_state
    }

    /// Returns a new (deterministic) automaton that accepts a single codepoint of
    /// the given value.
    pub fn make_char(c: i32) -> Automaton {
        Self::make_char_range(c, c)
    }

    /// Returns a new (deterministic and minimal) automaton that accepts potentially
    /// multiple codepoints of the given value that are case-insensitive equivalents.
    pub fn make_case_insensitive_char(c: i32) -> Automaton {
        Self::make_char_set(&Self::to_case_insensitive_char(c))
    }

    /// Appends the specified character to the specified state, returning a new
    /// state.
    pub fn append_char(a: &mut Automaton, state: i32, c: i32) -> i32 {
        let new_state = a.create_state();
        a.add_transition_range(state, new_state, c, c);
        new_state
    }

    /// Returns a new (deterministic) automaton that accepts a single codepoint whose
    /// value is in the given interval (including both end points).
    pub fn make_char_range(min: i32, max: i32) -> Automaton {
        if min > max {
            return Self::make_empty();
        }
        let mut a = Automaton::new();
        let s1 = a.create_state();
        let s2 = a.create_state();
        a.set_accept(s2, true);
        a.add_transition_range(s1, s2, min, max);
        a.finish_state();
        a
    }

    /// Returns a new minimal automaton that accepts any of the provided codepoints.
    pub fn make_char_set(codepoints: &[i32]) -> Automaton {
        Self::make_char_class(codepoints, codepoints)
    }

    /// Returns a new minimal automaton that accepts any of the codepoint ranges.
    ///
    /// # Panics
    ///
    /// Panics if `starts` and `ends` have different lengths, matching Lucene's
    /// `IllegalArgumentException`.
    pub fn make_char_class(starts: &[i32], ends: &[i32]) -> Automaton {
        assert!(starts.len() == ends.len(), "starts must match ends");
        if starts.is_empty() {
            return Self::make_empty();
        }
        let mut a = Automaton::new();
        let s1 = a.create_state();
        let s2 = a.create_state();
        a.set_accept(s2, true);
        for i in 0..starts.len() {
            a.add_transition_range(s1, s2, starts[i], ends[i]);
        }
        a.finish_state();
        a
    }

    /// Constructs a sub-automaton corresponding to decimal numbers of length
    /// `x.len() - n`.
    fn any_of_right_length(builder: &mut AutomatonBuilder, x: &[u8], n: usize) -> i32 {
        let s = builder.create_state();
        if x.len() == n {
            builder.set_accept(s, true);
        } else {
            let dest = Self::any_of_right_length(builder, x, n + 1);
            builder.add_transition_range(s, dest, '0' as i32, '9' as i32);
        }
        s
    }

    /// Constructs a sub-automaton corresponding to decimal numbers of value at least
    /// `x[n..]` and length `x.len() - n`.
    fn at_least(
        builder: &mut AutomatonBuilder,
        x: &[u8],
        n: usize,
        initials: &mut Vec<i32>,
        zeros: bool,
    ) -> i32 {
        let s = builder.create_state();
        if x.len() == n {
            builder.set_accept(s, true);
        } else {
            if zeros {
                initials.push(s);
            }
            let c = x[n];
            let dest = Self::at_least(builder, x, n + 1, initials, zeros && c == b'0');
            builder.add_transition(s, dest, i32::from(c));
            if c < b'9' {
                let dest = Self::any_of_right_length(builder, x, n + 1);
                builder.add_transition_range(s, dest, i32::from(c) + 1, '9' as i32);
            }
        }
        s
    }

    /// Constructs a sub-automaton corresponding to decimal numbers of value at most
    /// `x[n..]` and length `x.len() - n`.
    fn at_most(builder: &mut AutomatonBuilder, x: &[u8], n: usize) -> i32 {
        let s = builder.create_state();
        if x.len() == n {
            builder.set_accept(s, true);
        } else {
            let c = x[n];
            let dest = Self::at_most(builder, x, n + 1);
            builder.add_transition(s, dest, i32::from(c));
            if c > b'0' {
                let dest = Self::any_of_right_length(builder, x, n + 1);
                builder.add_transition_range(s, dest, '0' as i32, i32::from(c) - 1);
            }
        }
        s
    }

    /// Constructs a sub-automaton corresponding to decimal numbers of value between
    /// `x[n..]` and `y[n..]` and of length `x.len() - n` (which must equal
    /// `y.len() - n`).
    fn between(
        builder: &mut AutomatonBuilder,
        x: &[u8],
        y: &[u8],
        n: usize,
        initials: &mut Vec<i32>,
        zeros: bool,
    ) -> i32 {
        let s = builder.create_state();
        if x.len() == n {
            builder.set_accept(s, true);
        } else {
            if zeros {
                initials.push(s);
            }
            let cx = x[n];
            let cy = y[n];
            if cx == cy {
                let dest = Self::between(builder, x, y, n + 1, initials, zeros && cx == b'0');
                builder.add_transition(s, dest, i32::from(cx));
            } else {
                // cx < cy
                let dest = Self::at_least(builder, x, n + 1, initials, zeros && cx == b'0');
                builder.add_transition(s, dest, i32::from(cx));
                let dest = Self::at_most(builder, y, n + 1);
                builder.add_transition(s, dest, i32::from(cy));
                if i32::from(cx) + 1 < i32::from(cy) {
                    let dest = Self::any_of_right_length(builder, x, n + 1);
                    builder.add_transition_range(s, dest, i32::from(cx) + 1, i32::from(cy) - 1);
                }
            }
        }
        s
    }

    fn suffix_is_zeros(br: &BytesRef, len: usize) -> bool {
        for i in len..br.length {
            if br.bytes[br.offset + i] != 0 {
                return false;
            }
        }
        true
    }

    /// Creates a new deterministic, minimal automaton accepting all binary terms in
    /// the specified interval.
    ///
    /// Note that unlike [`Automata::make_decimal_interval`], the returned automaton
    /// is infinite, because terms behave like floating point numbers leading with a
    /// decimal point. However, in the special case where `min == max` and both are
    /// inclusive, the automaton is finite and accepts exactly one term.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when an open-ended bound is marked
    /// exclusive.
    pub fn make_binary_interval(
        min: Option<&BytesRef>,
        min_inclusive: bool,
        max: Option<&BytesRef>,
        max_inclusive: bool,
    ) -> Result<Automaton> {
        if min.is_none() && !min_inclusive {
            return Err(LuceneError::IllegalArgument(
                "minInclusive must be true when min is null (open ended)".to_string(),
            ));
        }

        if max.is_none() && !max_inclusive {
            return Err(LuceneError::IllegalArgument(
                "maxInclusive must be true when max is null (open ended)".to_string(),
            ));
        }

        let empty_min = BytesRef::default();
        let (min, min_inclusive) = match min {
            Some(m) => (m, min_inclusive),
            None => (&empty_min, true),
        };

        let cmp;
        if let Some(max) = max {
            cmp = min.cmp(max);
        } else {
            cmp = std::cmp::Ordering::Less;
            if min.length == 0 {
                return Ok(if min_inclusive {
                    Self::make_any_binary()
                } else {
                    Self::make_non_empty_binary()
                });
            }
        }

        match cmp {
            std::cmp::Ordering::Equal => {
                return Ok(if !min_inclusive || !max_inclusive {
                    Self::make_empty()
                } else {
                    Self::make_binary(min)
                });
            }
            // max < min
            std::cmp::Ordering::Greater => return Ok(Self::make_empty()),
            std::cmp::Ordering::Less => {}
        }

        if let Some(max) = max {
            if StringHelper::starts_with(max, min) && Self::suffix_is_zeros(max, min.length) {
                // Finite case: no sink state!

                let mut max_length = max.length;

                // the == case was handled above
                debug_assert!(max_length > min.length);

                //  bar -> bar\0+
                if !max_inclusive {
                    max_length -= 1;
                }

                if max_length == min.length {
                    return Ok(if !min_inclusive {
                        Self::make_empty()
                    } else {
                        Self::make_binary(min)
                    });
                }

                let mut a = Automaton::new();
                let mut last_state = a.create_state();
                for i in 0..min.length {
                    let state = a.create_state();
                    let label = i32::from(min.bytes[min.offset + i]);
                    a.add_transition(last_state, state, label);
                    last_state = state;
                }

                if min_inclusive {
                    a.set_accept(last_state, true);
                }

                for _ in min.length..max_length {
                    let state = a.create_state();
                    a.add_transition(last_state, state, 0);
                    a.set_accept(state, true);
                    last_state = state;
                }
                a.finish_state();
                return Ok(a);
            }
        }

        let mut a = Automaton::new();
        let start_state = a.create_state();

        let sink_state = a.create_state();
        a.set_accept(sink_state, true);

        // This state accepts all suffixes:
        a.add_transition_range(sink_state, sink_state, 0, 255);

        let mut equal_prefix = true;
        let mut last_state = start_state;
        let mut first_max_state = -1i32;
        let mut shared_prefix_length = 0usize;
        for i in 0..min.length {
            let min_label = i32::from(min.bytes[min.offset + i]);

            let max_label = match max {
                Some(max) if equal_prefix && i < max.length => i32::from(max.bytes[max.offset + i]),
                _ => -1,
            };

            let next_state = if min_inclusive
                && i == min.length - 1
                && (!equal_prefix || min_label != max_label)
            {
                sink_state
            } else {
                a.create_state()
            };

            if equal_prefix {
                if min_label == max_label {
                    // Still in shared prefix
                    a.add_transition(last_state, next_state, min_label);
                } else if max.is_none() {
                    equal_prefix = false;
                    shared_prefix_length = 0;
                    a.add_transition_range(last_state, sink_state, min_label + 1, 0xff);
                    a.add_transition(last_state, next_state, min_label);
                } else {
                    // This is the first point where min & max diverge:
                    let max_ref = max.expect("INVARIANT: max.is_none() handled above");
                    debug_assert!(max_label > min_label);

                    a.add_transition(last_state, next_state, min_label);

                    if max_label > min_label + 1 {
                        a.add_transition_range(
                            last_state,
                            sink_state,
                            min_label + 1,
                            max_label - 1,
                        );
                    }

                    // Now fork off path for max:
                    if max_inclusive || i < max_ref.length - 1 {
                        first_max_state = a.create_state();
                        if i < max_ref.length - 1 {
                            a.set_accept(first_max_state, true);
                        }
                        a.add_transition(last_state, first_max_state, max_label);
                    }
                    equal_prefix = false;
                    shared_prefix_length = i;
                }
            } else {
                // OK, already diverged:
                a.add_transition(last_state, next_state, min_label);
                if min_label < 255 {
                    a.add_transition_range(last_state, sink_state, min_label + 1, 255);
                }
            }
            last_state = next_state;
        }

        // Accept any suffix appended to the min term:
        if !equal_prefix && last_state != sink_state && last_state != start_state {
            a.add_transition_range(last_state, sink_state, 0, 255);
        }

        if min_inclusive {
            // Accept exactly the min term:
            a.set_accept(last_state, true);
        }

        if let Some(max) = max {
            // Now do max:
            if first_max_state == -1 {
                // Min was a full prefix of max
                shared_prefix_length = min.length;
            } else {
                last_state = first_max_state;
                shared_prefix_length += 1;
            }
            for i in shared_prefix_length..max.length {
                let max_label = i32::from(max.bytes[max.offset + i]);
                if max_label > 0 {
                    a.add_transition_range(last_state, sink_state, 0, max_label - 1);
                }
                if max_inclusive || i < max.length - 1 {
                    let next_state = a.create_state();
                    if i < max.length - 1 {
                        a.set_accept(next_state, true);
                    }
                    a.add_transition(last_state, next_state, max_label);
                    last_state = next_state;
                }
            }

            if max_inclusive {
                a.set_accept(last_state, true);
            }
        }

        a.finish_state();

        debug_assert!(a.is_deterministic(), "{}", a.to_dot());

        Ok(a)
    }

    /// Returns a new automaton that accepts strings representing decimal (base 10)
    /// non-negative integers in the given interval.
    ///
    /// If `digits > 0` a fixed number of digits is used (strings must be prefixed by
    /// zeros to obtain the right length); otherwise the number of digits is not fixed
    /// and any number of leading zeros is accepted.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] if `min > max` or if numbers in the
    /// interval cannot be expressed with the given fixed number of digits.
    pub fn make_decimal_interval(min: i32, max: i32, digits: i32) -> Result<Automaton> {
        let mut x = min.to_string();
        let mut y = max.to_string();
        if min > max || (digits > 0 && y.len() as i32 > digits) {
            return Err(LuceneError::IllegalArgument(format!(
                "invalid decimal interval: min={} max={} digits={}",
                min, max, digits
            )));
        }
        let d = if digits > 0 { digits } else { y.len() as i32 };
        let mut bx = String::new();
        for _ in x.len() as i32..d {
            bx.push('0');
        }
        bx.push_str(&x);
        x = bx;
        let mut by = String::new();
        for _ in y.len() as i32..d {
            by.push('0');
        }
        by.push_str(&y);
        y = by;

        let mut builder = AutomatonBuilder::new();

        if digits <= 0 {
            // Reserve the "real" initial state:
            builder.create_state();
        }

        let mut initials: Vec<i32> = Vec::new();

        Self::between(
            &mut builder,
            x.as_bytes(),
            y.as_bytes(),
            0,
            &mut initials,
            digits <= 0,
        );

        let mut a1 = builder.finish();

        if digits <= 0 {
            a1.add_transition(0, 0, '0' as i32);
            for p in initials {
                a1.add_epsilon(0, p);
            }
            a1.finish_state();
        }

        Ok(Operations::remove_dead_states(&a1))
    }

    /// Returns a new (deterministic) automaton that accepts the single given string.
    pub fn make_string(s: &str) -> Automaton {
        let mut a = Automaton::new();
        let mut last_state = a.create_state();
        for cp in s.chars() {
            let state = a.create_state();
            a.add_transition(last_state, state, cp as i32);
            last_state = state;
        }

        a.set_accept(last_state, true);
        a.finish_state();

        debug_assert!(a.is_deterministic());
        debug_assert!(!Operations::has_dead_states(&a));

        a
    }

    /// Returns a new (deterministic and minimal) automaton that accepts the single
    /// given string and its case-insensitive equivalents.
    pub fn make_case_insensitive_string(s: &str) -> Automaton {
        let mut a = Automaton::new();
        let mut last_state = a.create_state();
        for cp in s.chars() {
            let state = a.create_state();
            for alt in Self::to_case_insensitive_char(cp as i32) {
                a.add_transition(last_state, state, alt);
            }
            last_state = state;
        }

        a.set_accept(last_state, true);
        a.finish_state();

        debug_assert!(a.is_deterministic());
        debug_assert!(!Operations::has_dead_states(&a));

        a
    }

    /// Returns a new (deterministic) automaton that accepts the single given binary
    /// term.
    pub fn make_binary(term: &BytesRef) -> Automaton {
        let mut a = Automaton::new();
        let mut last_state = a.create_state();
        for i in 0..term.length {
            let state = a.create_state();
            let label = i32::from(term.bytes[term.offset + i]);
            a.add_transition(last_state, state, label);
            last_state = state;
        }

        a.set_accept(last_state, true);
        a.finish_state();

        debug_assert!(a.is_deterministic());
        debug_assert!(!Operations::has_dead_states(&a));

        a
    }

    /// Returns a new (deterministic) automaton that accepts the single given string
    /// from the specified Unicode code points.
    pub fn make_string_from_code_points(word: &[i32], offset: usize, length: usize) -> Automaton {
        let mut a = Automaton::new();
        a.create_state();
        let mut s = 0;
        for &label in &word[offset..offset + length] {
            let s2 = a.create_state();
            a.add_transition(s, s2, label);
            s = s2;
        }
        a.set_accept(s, true);
        a.finish_state();

        a
    }

    /// Returns a new (deterministic and minimal) automaton that accepts the union of
    /// the given collection of UTF-8 encoded strings, which must be in sorted order.
    ///
    /// The resulting automaton is codepoint based (full Unicode codepoints on
    /// transitions).
    ///
    /// # Errors
    ///
    /// Returns an error if the input is not sorted, or if a term is longer than
    /// [`MAX_STRING_UNION_TERM_LENGTH`], or if a term is not valid UTF-8.
    pub fn make_string_union<I>(utf8_strings: I) -> Result<Automaton>
    where
        I: IntoIterator<Item = BytesRef>,
    {
        let mut it = utf8_strings.into_iter().peekable();
        if it.peek().is_none() {
            Ok(Self::make_empty())
        } else {
            StringsToAutomaton::build(it, false)
        }
    }

    /// Returns a new (deterministic and minimal) automaton that accepts the union of
    /// the given collection of UTF-8 encoded strings, which must be in sorted order.
    ///
    /// The resulting automaton is binary based (UTF-8 encoded byte transition
    /// labels).
    ///
    /// # Errors
    ///
    /// Returns an error if the input is not sorted, or if a term is longer than
    /// [`MAX_STRING_UNION_TERM_LENGTH`].
    pub fn make_binary_string_union<I>(utf8_strings: I) -> Result<Automaton>
    where
        I: IntoIterator<Item = BytesRef>,
    {
        let mut it = utf8_strings.into_iter().peekable();
        if it.peek().is_none() {
            Ok(Self::make_empty())
        } else {
            StringsToAutomaton::build(it, true)
        }
    }

    /// Same as [`Automata::make_string_union`], but for an iterator that is consumed
    /// as-is; an empty iterator yields a single-state, non-accepting automaton
    /// rather than the zero-state one.
    ///
    /// This corresponds to Lucene's `makeStringUnion(BytesRefIterator)` overload.
    ///
    /// # Errors
    ///
    /// Returns an error if the input is not sorted, or if a term is longer than
    /// [`MAX_STRING_UNION_TERM_LENGTH`], or if a term is not valid UTF-8.
    pub fn make_string_union_from_iter<I>(utf8_strings: I) -> Result<Automaton>
    where
        I: IntoIterator<Item = BytesRef>,
    {
        StringsToAutomaton::build(utf8_strings, false)
    }

    /// Same as [`Automata::make_binary_string_union`], but for an iterator that is
    /// consumed as-is.
    ///
    /// This corresponds to Lucene's `makeBinaryStringUnion(BytesRefIterator)`
    /// overload.
    ///
    /// # Errors
    ///
    /// Returns an error if the input is not sorted, or if a term is longer than
    /// [`MAX_STRING_UNION_TERM_LENGTH`].
    pub fn make_binary_string_union_from_iter<I>(utf8_strings: I) -> Result<Automaton>
    where
        I: IntoIterator<Item = BytesRef>,
    {
        StringsToAutomaton::build(utf8_strings, true)
    }

    /// Uses the Unicode spec to generate case-insensitive alternates.
    ///
    /// See [`RegExp::CASE_INSENSITIVE`](super::reg_exp::RegExp) for details on case
    /// folding within the Unicode spec. Returns the original codepoint and the set
    /// of alternates, sorted ascending (duplicates are kept, exactly as Lucene's
    /// `IntArrayList.sort()` leaves them).
    fn to_case_insensitive_char(codepoint: i32) -> Vec<i32> {
        let mut list: Vec<i32> = Vec::new();
        CaseFolding::expand(codepoint, |variant| list.push(variant));
        list.sort_unstable();
        list
    }
}
