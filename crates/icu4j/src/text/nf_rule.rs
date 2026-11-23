// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.text.NFRule`.

use rucene::error::{LuceneError, Result};
use rucene_analysis_common::jdk::string::region_matches;

use crate::jdk::text::FieldPosition;
use crate::jdk::{index_of, index_of_unit, units, ParsePosition};
use crate::r#impl::number::decimal_quantity_dual_storage_bcd::JavaNumber;
use crate::r#impl::pattern_props::PatternProps;
use crate::text::decimal_format_symbols::DecimalFormatSymbols;
use crate::text::nf_rule_set::{BuildContext, NFRuleSet, RuleSetBody};
use crate::text::nf_substitution::NFSubstitution;
use crate::text::number_format::INTEGER_FIELD;
use crate::text::plural_format::PluralFormat;
use crate::text::plural_rules::PluralType;
use crate::text::rule_based_number_format::{
    char_at, java_math_round, long_or_double, npe, sb_insert, starts_with, starts_with_at, substring, substring_from,
    RuleGraph,
};

//-----------------------------------------------------------------------
// constants
//-----------------------------------------------------------------------

/// Special base value used to identify a negative-number rule.
/// `NEGATIVE_NUMBER_RULE`.
pub(crate) const NEGATIVE_NUMBER_RULE: i64 = -1;

/// Special base value used to identify an improper fraction (x.x) rule.
/// `IMPROPER_FRACTION_RULE`.
pub(crate) const IMPROPER_FRACTION_RULE: i64 = -2;

/// Special base value used to identify a proper fraction (0.x) rule.
/// `PROPER_FRACTION_RULE`.
pub(crate) const PROPER_FRACTION_RULE: i64 = -3;

/// Special base value used to identify a default rule. `DEFAULT_RULE`.
pub(crate) const DEFAULT_RULE: i64 = -4;

/// Special base value used to identify an infinity rule. `INFINITY_RULE`.
pub(crate) const INFINITY_RULE: i64 = -5;

/// Special base value used to identify a not a number rule. `NAN_RULE`.
pub(crate) const NAN_RULE: i64 = -6;

/// `ZERO`: the `Long` 0.
pub(crate) const ZERO: JavaNumber = JavaNumber::Long(0);

/// `RULE_PREFIXES` (private): the substitution tokens.
const RULE_PREFIXES: [&str; 11] = ["<<", "<%", "<#", "<0", ">>", ">%", ">#", ">0", "=%", "=#", "=0"];

/// A class representing a single rule in a RuleBasedNumberFormat. A rule
/// inserts its text into the result string and then passes control to its
/// substitutions, which do the same thing.
///
/// Equivalent to the package-private class `com.ibm.icu.text.NFRule`.
///
/// **Divergence.** Java's rule keeps its formatter (`final
/// RuleBasedNumberFormat formatter`, `NFRule.java`, field and constructor);
/// the port's rule has no such field: the methods that use it receive the
/// [`RuleGraph`] whose owner is that formatter (see the divergence note of
/// `RuleGraph`), and the constructors that need it receive the
/// [`BuildContext`]. Java's `null` rule text of the second rule of a
/// bracketed description (`new NFRule(ownersOwner, null)`, replaced by
/// `extractSubstitutions` right after) is the empty text.
#[derive(Debug)]
pub(crate) struct NFRule {
    /// `baseValue`: the rule's base value.
    base_value: i64,
    /// `radix`: the rule's radix (the radix to the power of the exponent
    /// equals the rule's divisor).
    radix: i32,
    /// `exponent`: the rule's exponent.
    exponent: i16,
    /// `decimalPoint`: if this is a fraction rule, this is the decimal point
    /// from DecimalFormatSymbols to match.
    decimal_point: u16,
    /// `ruleText`: the rule's rule text. When formatting a number, the
    /// rule's text is inserted into the result string, and then the text
    /// from any substitutions is inserted into the result string.
    rule_text: Vec<u16>,
    /// `rulePatternFormat`: the rule's plural format when defined.
    rule_pattern_format: Option<PluralFormat>,
    /// `sub1`: the rule's first substitution (the one with the lower
    /// offset into the rule text).
    sub1: Option<NFSubstitution>,
    /// `sub2`: the rule's second substitution (the one with the higher
    /// offset into the rule text).
    sub2: Option<NFSubstitution>,
}

impl NFRule {
    //-----------------------------------------------------------------------
    // construction
    //-----------------------------------------------------------------------

    /// Creates one or more rules based on the description passed in, adding
    /// the normal rules to `return_list` and the special ones to the body of
    /// the rule set `owner` (an index into the context's rule sets).
    /// `predecessor` is the index in `return_list` of the rule before this
    /// one in the rule set's rule list. Equivalent to the static
    /// `makeRules(String description, NFRuleSet owner, NFRule predecessor,
    /// RuleBasedNumberFormat ownersOwner, List<NFRule> returnList)`.
    ///
    /// # Errors
    ///
    /// The errors of the rule descriptors and of the substitutions.
    pub(crate) fn make_rules(
        description: &[u16],
        owner: usize,
        predecessor: Option<usize>,
        ctx: &BuildContext<'_>,
        body: &mut RuleSetBody,
        return_list: &mut Vec<NFRule>,
    ) -> Result<()> {
        // we know we're making at least one rule, so go ahead and
        // new it up and initialize its basevalue and divisor
        // (this also strips the rule descriptor, if any, off the
        // description string)
        let mut rule1 = NFRule::new(Some(description))?;
        let description = rule1.rule_text.clone();

        // check the description to see whether there's text enclosed
        // in brackets
        let brack1 = index_of_unit(&description, u16::from(b'['), 0);
        let brack2 = if brack1 < 0 { -1 } else { index_of_unit(&description, u16::from(b']'), 0) };

        // if the description doesn't contain a matched pair of brackets,
        // or if it's of a type that doesn't recognize bracketed text,
        // then leave the description alone, initialize the rule's
        // rule text and substitutions, and return that rule
        if brack2 < 0
            || brack1 > brack2
            || rule1.base_value == PROPER_FRACTION_RULE
            || rule1.base_value == NEGATIVE_NUMBER_RULE
            || rule1.base_value == INFINITY_RULE
            || rule1.base_value == NAN_RULE
        {
            rule1.extract_substitutions(owner, &description, predecessor, ctx)?;
        } else {
            // if the description does contain a matched pair of brackets,
            // then it's really shorthand for two rules (with one exception)
            let mut rule2: Option<NFRule> = None;
            let mut sbuf: Vec<u16> = Vec::new();
            let or_else_op = index_of_unit(&description, u16::from(b'|'), 0);

            // we'll actually only split the rule into two rules if its
            // base value is an even multiple of its divisor (or it's one
            // of the special rules)
            if (rule1.base_value > 0
                && checked_rem(rule1.base_value, power(i64::from(rule1.radix), rule1.exponent)?)? == 0)
                || rule1.base_value == IMPROPER_FRACTION_RULE
                || rule1.base_value == DEFAULT_RULE
            {
                // if it passes that test, new up the second rule.  If the
                // rule set both rules will belong to is a fraction rule
                // set, they both have the same base value; otherwise,
                // increment the original rule's base value ("rule1" actually
                // goes SECOND in the rule set's rule list)
                let mut r2 = NFRule::new(None)?;
                if rule1.base_value >= 0 {
                    r2.base_value = rule1.base_value;
                    if !ctx.rule_sets[owner].is_fraction_set() {
                        rule1.base_value = rule1.base_value.wrapping_add(1);
                    }
                } else if rule1.base_value == IMPROPER_FRACTION_RULE {
                    // if the description began with "x.x" and contains bracketed
                    // text, it describes both the improper fraction rule and
                    // the proper fraction rule
                    r2.base_value = PROPER_FRACTION_RULE;
                } else if rule1.base_value == DEFAULT_RULE {
                    // if the description began with "x.0" and contains bracketed
                    // text, it describes both the default rule and the
                    // improper fraction rule
                    r2.base_value = rule1.base_value;
                    rule1.base_value = IMPROPER_FRACTION_RULE;
                }

                // both rules have the same radix and exponent (i.e., the
                // same divisor)
                r2.radix = rule1.radix;
                r2.exponent = rule1.exponent;

                // By default, rule2's rule text omits the stuff in brackets,
                // unless it contains a | between the brackets.
                // Initialize its rule text and substitutions accordingly
                sbuf.extend_from_slice(substring(&description, 0, brack1));
                if or_else_op >= 0 {
                    sbuf.extend_from_slice(substring(&description, or_else_op + 1, brack2));
                }
                if brack2 + 1 < description.len() as i32 {
                    sbuf.extend_from_slice(substring(&description, brack2 + 1, description.len() as i32));
                }
                r2.extract_substitutions(owner, &sbuf, predecessor, ctx)?;
                rule2 = Some(r2);
            }

            // rule1's text includes the text in the brackets but omits
            // the brackets themselves: initialize _its_ rule text and
            // substitutions accordingly
            sbuf.clear();
            sbuf.extend_from_slice(substring(&description, 0, brack1));
            if or_else_op >= 0 {
                sbuf.extend_from_slice(substring(&description, brack1 + 1, or_else_op));
            } else {
                sbuf.extend_from_slice(substring(&description, brack1 + 1, brack2));
            }
            if brack2 + 1 < description.len() as i32 {
                sbuf.extend_from_slice(substring_from(&description, brack2 + 1));
            }
            rule1.extract_substitutions(owner, &sbuf, predecessor, ctx)?;

            // if we only have one rule, return it; if we have two, return
            // a two-element array containing them (notice that rule2 goes
            // BEFORE rule1 in the list: in all cases, rule2 OMITS the
            // material in the brackets and rule1 INCLUDES the material
            // in the brackets)
            if let Some(rule2) = rule2 {
                if rule2.base_value >= 0 {
                    return_list.push(rule2);
                } else {
                    NFRuleSet::set_non_numerical_rule(body, rule2, ctx.owner)?;
                }
            }
        }
        if rule1.base_value >= 0 {
            return_list.push(rule1);
        } else {
            NFRuleSet::set_non_numerical_rule(body, rule1, ctx.owner)?;
        }
        Ok(())
    }

    /// Creates a rule from its description (`None` for the second rule of a
    /// bracketed description): parses the rule descriptor. Equivalent to
    /// `NFRule(RuleBasedNumberFormat formatter, String ruleText)`.
    ///
    /// # Errors
    ///
    /// The errors of the rule descriptor.
    pub(crate) fn new(rule_text: Option<&[u16]>) -> Result<NFRule> {
        let mut rule = NFRule {
            base_value: 0,
            radix: 10,
            exponent: 0,
            decimal_point: 0,
            rule_text: Vec::new(),
            rule_pattern_format: None,
            sub1: None,
            sub2: None,
        };
        if let Some(text) = rule_text {
            rule.rule_text = rule.parse_rule_descriptor(text)?;
        }
        Ok(rule)
    }

    /// This function parses the rule's rule descriptor (i.e., the base value
    /// and/or other tokens that precede the rule's rule text in the
    /// description) and sets the rule's base value, radix, and exponent
    /// accordingly. Equivalent to `parseRuleDescriptor(String description)`
    /// (private); returns the rule text.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("Illegal character <c> in rule
    /// descriptor")`, `IllegalArgumentException("Rule can't have radix of
    /// 0")`, `IllegalArgumentException("Illegal character in rule
    /// descriptor")`.
    ///
    /// # Panics
    ///
    /// `StringIndexOutOfBoundsException` for an empty descriptor (Java's
    /// `charAt(0)`).
    fn parse_rule_descriptor(&mut self, description: &[u16]) -> Result<Vec<u16>> {
        let mut description = description.to_vec();

        // the description consists of a rule descriptor and a rule body,
        // separated by a colon.  The rule descriptor is optional.  If
        // it's omitted, just set the base value to 0.
        let mut p = index_of(&description, &units(":"), 0);
        if p != -1 {
            // copy the descriptor out into its own string and strip it,
            // along with any trailing whitespace, out of the original
            // description
            let descriptor = substring(&description, 0, p).to_vec();
            p += 1;
            while p < description.len() as i32 && PatternProps::is_white_space(i32::from(char_at(&description, p))) {
                p += 1;
            }
            description = substring_from(&description, p).to_vec();

            // check first to see if the rule descriptor matches the token
            // for one of the special rules.  If it does, set the base
            // value to the correct identifier value
            let descriptor_length = descriptor.len() as i32;
            let first_char = char_at(&descriptor, 0);
            let last_char = char_at(&descriptor, descriptor_length - 1);
            if (u16::from(b'0')..=u16::from(b'9')).contains(&first_char) && last_char != u16::from(b'x') {
                // if the rule descriptor begins with a digit, it's a descriptor
                // for a normal rule
                let mut temp_value: i64 = 0;
                let mut c: u16 = 0;
                p = 0;

                // begin parsing the descriptor: copy digits
                // into "tempValue", skip periods, commas, and spaces,
                // stop on a slash or > sign (or at the end of the string),
                // and throw an exception on any other character
                while p < descriptor_length {
                    c = char_at(&descriptor, p);
                    if (u16::from(b'0')..=u16::from(b'9')).contains(&c) {
                        temp_value = temp_value.wrapping_mul(10).wrapping_add(i64::from(c - u16::from(b'0')));
                    } else if c == u16::from(b'/') || c == u16::from(b'>') {
                        break;
                    } else if !PatternProps::is_white_space(i32::from(c)) && c != u16::from(b',') && c != u16::from(b'.')
                    {
                        return Err(illegal_character(c));
                    }
                    p += 1;
                }

                // Set the rule's base value according to what we parsed
                self.set_base_value(temp_value)?;

                // if we stopped the previous loop on a slash, we're
                // now parsing the rule's radix.  Again, accumulate digits
                // in tempValue, skip punctuation, stop on a > mark, and
                // throw an exception on anything else
                if c == u16::from(b'/') {
                    temp_value = 0;
                    p += 1;
                    while p < descriptor_length {
                        c = char_at(&descriptor, p);
                        if (u16::from(b'0')..=u16::from(b'9')).contains(&c) {
                            temp_value = temp_value.wrapping_mul(10).wrapping_add(i64::from(c - u16::from(b'0')));
                        } else if c == u16::from(b'>') {
                            break;
                        } else if !PatternProps::is_white_space(i32::from(c))
                            && c != u16::from(b',')
                            && c != u16::from(b'.')
                        {
                            return Err(illegal_character(c));
                        }
                        p += 1;
                    }

                    // tempValue now contains the rule's radix.  Set it
                    // accordingly, and recalculate the rule's exponent
                    self.radix = temp_value as i32;
                    if self.radix == 0 {
                        return Err(LuceneError::illegal_argument("Rule can't have radix of 0"));
                    }

                    self.exponent = self.expected_exponent()?;
                }

                // if we stopped the previous loop on a > sign, then continue
                // for as long as we still see > signs.  For each one,
                // decrement the exponent (unless the exponent is already 0).
                // If we see another character before reaching the end of
                // the descriptor, that's also a syntax error.
                if c == u16::from(b'>') {
                    while p < descriptor_length {
                        c = char_at(&descriptor, p);
                        if c == u16::from(b'>') && self.exponent > 0 {
                            self.exponent -= 1;
                        } else {
                            return Err(LuceneError::illegal_argument("Illegal character in rule descriptor"));
                        }
                        p += 1;
                    }
                }
            } else if descriptor == units("-x") {
                self.set_base_value(NEGATIVE_NUMBER_RULE)?;
            } else if descriptor_length == 3 {
                if first_char == u16::from(b'0') && last_char == u16::from(b'x') {
                    self.set_base_value(PROPER_FRACTION_RULE)?;
                    self.decimal_point = char_at(&descriptor, 1);
                } else if first_char == u16::from(b'x') && last_char == u16::from(b'x') {
                    self.set_base_value(IMPROPER_FRACTION_RULE)?;
                    self.decimal_point = char_at(&descriptor, 1);
                } else if first_char == u16::from(b'x') && last_char == u16::from(b'0') {
                    self.set_base_value(DEFAULT_RULE)?;
                    self.decimal_point = char_at(&descriptor, 1);
                } else if descriptor == units("NaN") {
                    self.set_base_value(NAN_RULE)?;
                } else if descriptor == units("Inf") {
                    self.set_base_value(INFINITY_RULE)?;
                }
            }
        }
        // else use the default base value for now.

        // finally, if the rule body begins with an apostrophe, strip it off
        // (this is generally used to put whitespace at the beginning of
        // a rule's rule text)
        if !description.is_empty() && description[0] == u16::from(b'\'') {
            description = substring_from(&description, 1).to_vec();
        }

        // return the description with all the stuff we've just waded through
        // stripped off the front.  It now contains just the rule body.
        Ok(description)
    }

    /// Searches the rule's rule text for the substitution tokens, creates
    /// the substitutions, and removes the substitution tokens from the
    /// rule's rule text; then creates the plural format of a `$(...)$`
    /// rule. Equivalent to `extractSubstitutions(NFRuleSet owner, String
    /// ruleText, NFRule predecessor)` (private).
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("Rule \"<text>\" does not have a defined
    /// type")`, `IllegalArgumentException("<type> is an unknown type")`;
    /// the errors of the substitutions and of the plural format.
    fn extract_substitutions(
        &mut self,
        owner: usize,
        rule_text: &[u16],
        predecessor: Option<usize>,
        ctx: &BuildContext<'_>,
    ) -> Result<()> {
        self.rule_text = rule_text.to_vec();
        self.sub1 = self.extract_substitution(owner, predecessor, ctx)?;
        if self.sub1.is_none() {
            // Small optimization. There is no need to create a redundant NullSubstitution.
            self.sub2 = None;
        } else {
            self.sub2 = self.extract_substitution(owner, predecessor, ctx)?;
        }
        let rule_text = self.rule_text.clone();
        let plural_rule_start = index_of(&rule_text, &units("$("), 0);
        let plural_rule_end =
            if plural_rule_start >= 0 { index_of(&rule_text, &units(")$"), plural_rule_start) } else { -1 };
        if plural_rule_end >= 0 {
            let end_type = index_of_unit(&rule_text, u16::from(b','), plural_rule_start);
            if end_type < 0 {
                return Err(LuceneError::illegal_argument(format!(
                    "Rule \"{}\" does not have a defined type",
                    String::from_utf16_lossy(&rule_text)
                )));
            }
            let type_ = substring(&self.rule_text, plural_rule_start + 2, end_type).to_vec();
            let plural_type = if type_ == units("cardinal") {
                PluralType::Cardinal
            } else if type_ == units("ordinal") {
                PluralType::Ordinal
            } else {
                return Err(LuceneError::illegal_argument(format!(
                    "{} is an unknown type",
                    String::from_utf16_lossy(&type_)
                )));
            };
            self.rule_pattern_format =
                Some(ctx.owner.create_plural_format(plural_type, substring(&rule_text, end_type + 1, plural_rule_end))?);
        }
        Ok(())
    }

    /// Searches the rule's rule text for the first substitution token,
    /// creates a substitution based on it, and removes the token from the
    /// rule's rule text. Equivalent to `extractSubstitution(NFRuleSet owner,
    /// NFRule predecessor)` (private); `None` is Java's `null`.
    ///
    /// # Errors
    ///
    /// The errors of [`NFSubstitution::make_substitution`].
    fn extract_substitution(
        &mut self,
        owner: usize,
        predecessor: Option<usize>,
        ctx: &BuildContext<'_>,
    ) -> Result<Option<NFSubstitution>> {
        // search the rule's rule text for the first two characters of
        // a substitution token
        let sub_start = index_of_any_rule_prefix(&self.rule_text);
        let mut sub_end;

        // if we didn't find one, create a null substitution positioned
        // at the end of the rule text
        if sub_start == -1 {
            return Ok(None);
        }

        // special-case the ">>>" token, since searching for the > at the
        // end will actually find the > in the middle
        if starts_with_at(&self.rule_text, &units(">>>"), sub_start) {
            sub_end = sub_start + 2;
        } else {
            // otherwise the substitution token ends with the same character
            // it began with
            let c = char_at(&self.rule_text, sub_start);
            sub_end = index_of_unit(&self.rule_text, c, sub_start + 1);
            // special case for '<%foo<<'
            if c == u16::from(b'<')
                && sub_end != -1
                && sub_end < self.rule_text.len() as i32 - 1
                && char_at(&self.rule_text, sub_end + 1) == c
            {
                // ordinals use "=#,##0==%abbrev=" as their rule.  Notice that the '==' in the middle
                // occurs because of the juxtaposition of two different rules.  The check for '<' is a hack
                // to get around this.  Having the duplicate at the front would cause problems with
                // rules like "<<%" to format, say, percents...
                sub_end += 1;
            }
        }

        // if we don't find the end of the token (i.e., if we're on a single,
        // unmatched token character), create a null substitution positioned
        // at the end of the rule
        if sub_end == -1 {
            return Ok(None);
        }

        // if we get here, we have a real substitution token (or at least
        // some text bounded by substitution token characters).  Use
        // makeSubstitution() to create the right kind of substitution
        let token = substring(&self.rule_text, sub_start, sub_end + 1).to_vec();
        let result = NFSubstitution::make_substitution(sub_start, self, predecessor, owner, ctx, &token)?;

        // remove the substitution from the rule text
        let mut new_text = substring(&self.rule_text, 0, sub_start).to_vec();
        new_text.extend_from_slice(substring_from(&self.rule_text, sub_end + 1));
        self.rule_text = new_text;
        Ok(result)
    }

    /// Sets the rule's base value, and causes the radix and exponent to be
    /// recalculated (fixing the divisor copies of the substitutions).
    /// Equivalent to `setBaseValue(long newBaseValue)`.
    ///
    /// # Errors
    ///
    /// The errors of the substitutions' `setDivisor` and of the exponent.
    pub(crate) fn set_base_value(&mut self, new_base_value: i64) -> Result<()> {
        // set the base value
        self.base_value = new_base_value;
        self.radix = 10;

        // if this isn't a special rule, recalculate the radix and exponent
        // (the radix always defaults to 10; if it's supposed to be something
        // else, it's cleaned up by the caller and the exponent is
        // recalculated again-- the only function that does this is
        // NFRule.parseRuleDescriptor() )
        if self.base_value >= 1 {
            self.exponent = self.expected_exponent()?;

            // this function gets called on a fully-constructed rule whose
            // description didn't specify a base value.  This means it
            // has substitutions, and some substitutions hold on to copies
            // of the rule's divisor.  Fix their copies of the divisor.
            if let Some(sub1) = &mut self.sub1 {
                sub1.set_divisor(self.radix, self.exponent)?;
            }
            if let Some(sub2) = &mut self.sub2 {
                sub2.set_divisor(self.radix, self.exponent)?;
            }
        } else {
            // if this is a special rule, its radix and exponent are basically
            // ignored.  Set them to "safe" default values
            self.exponent = 0;
        }
        Ok(())
    }

    /// This calculates the rule's exponent based on its radix and base
    /// value. This will be the highest power the radix can be raised to and
    /// still produce a result less than or equal to the base value.
    /// Equivalent to `expectedExponent()` (private).
    ///
    /// # Errors
    ///
    /// The errors of [`power`].
    fn expected_exponent(&self) -> Result<i16> {
        // since the log of 0, or the log base 0 of something, causes an
        // error, declare the exponent in these cases to be 0 (we also
        // deal with the special-rule identifiers here)
        if self.radix == 0 || self.base_value < 1 {
            return Ok(0);
        }

        // we get rounding error in some cases-- for example, log 1000 / log 10
        // gives us 1.9999999996 instead of 2.  The extra logic here is to take
        // that into account
        let temp_result = (((self.base_value as f64).ln() / f64::from(self.radix).ln()) as i32) as i16;
        if power(i64::from(self.radix), temp_result.wrapping_add(1))? <= self.base_value {
            Ok(temp_result.wrapping_add(1))
        } else {
            Ok(temp_result)
        }
    }

    //-----------------------------------------------------------------------
    // boilerplate
    //-----------------------------------------------------------------------

    /// Tests two rules for equality: base value, radix, exponent, rule text
    /// and substitutions. Equivalent to `equals(Object that)`.
    pub(crate) fn java_equals(&self, that2: &NFRule) -> bool {
        fn subs_equal(a: Option<&NFSubstitution>, b: Option<&NFSubstitution>) -> bool {
            match (a, b) {
                (None, None) => true,
                (Some(a), Some(b)) => a.java_equals(b),
                _ => false,
            }
        }
        self.base_value == that2.base_value
            && self.radix == that2.radix
            && self.exponent == that2.exponent
            && self.rule_text == that2.rule_text
            && subs_equal(self.sub1.as_ref(), that2.sub1.as_ref())
            && subs_equal(self.sub2.as_ref(), that2.sub2.as_ref())
    }

    /// `hashCode()`: not designed (Java asserts and returns 42).
    pub(crate) fn hash_code(&self) -> i32 {
        debug_assert!(false, "hashCode not designed");
        42
    }

    /// Returns a textual representation of the rule. This won't necessarily
    /// be the same as the description that this rule was created with, but
    /// it will produce the same result. Equivalent to `toString()`.
    ///
    /// # Errors
    ///
    /// The errors of the patterns of the substitutions' decimal formats.
    pub(crate) fn to_java_string(&self, g: &RuleGraph) -> Result<Vec<u16>> {
        let mut result: Vec<u16> = Vec::new();
        let decimal_point = if self.decimal_point == 0 { u16::from(b'.') } else { self.decimal_point };

        // start with the rule descriptor.  Special-case the special rules
        if self.base_value == NEGATIVE_NUMBER_RULE {
            result.extend_from_slice(&units("-x: "));
        } else if self.base_value == IMPROPER_FRACTION_RULE {
            result.push(u16::from(b'x'));
            result.push(decimal_point);
            result.extend_from_slice(&units("x: "));
        } else if self.base_value == PROPER_FRACTION_RULE {
            result.push(u16::from(b'0'));
            result.push(decimal_point);
            result.extend_from_slice(&units("x: "));
        } else if self.base_value == DEFAULT_RULE {
            result.push(u16::from(b'x'));
            result.push(decimal_point);
            result.extend_from_slice(&units("0: "));
        } else if self.base_value == INFINITY_RULE {
            result.extend_from_slice(&units("Inf: "));
        } else if self.base_value == NAN_RULE {
            result.extend_from_slice(&units("NaN: "));
        } else {
            // for a normal rule, write out its base value, and if the radix is
            // something other than 10, write out the radix (with the preceding
            // slash, of course).  Then calculate the expected exponent and if
            // if isn't the same as the actual exponent, write an appropriate
            // number of > signs.  Finally, terminate the whole thing with
            // a colon.
            result.extend_from_slice(&units(&self.base_value.to_string()));
            if self.radix != 10 {
                result.push(u16::from(b'/'));
                result.extend_from_slice(&units(&self.radix.to_string()));
            }
            let num_carets = i32::from(self.expected_exponent()?) - i32::from(self.exponent);
            for _ in 0..num_carets {
                result.push(u16::from(b'>'));
            }
            result.extend_from_slice(&units(": "));
        }

        // if the rule text begins with a space, write an apostrophe
        // (whitespace after the rule descriptor is ignored; the
        // apostrophe is used to make the whitespace significant)
        if starts_with(&self.rule_text, &units(" ")) && self.sub1.as_ref().is_none_or(|s| s.get_pos() != 0) {
            result.push(u16::from(b'\''));
        }

        // now, write the rule's rule text, inserting appropriate
        // substitution tokens in the appropriate places
        let mut rule_text_copy = self.rule_text.clone();
        if let Some(sub2) = &self.sub2 {
            sb_insert(&mut rule_text_copy, sub2.get_pos(), &sub2.to_java_string(g)?);
        }
        if let Some(sub1) = &self.sub1 {
            sb_insert(&mut rule_text_copy, sub1.get_pos(), &sub1.to_java_string(g)?);
        }
        result.extend_from_slice(&rule_text_copy);

        // and finally, top the whole thing off with a semicolon and
        // return the result
        result.push(u16::from(b';'));
        Ok(result)
    }

    //-----------------------------------------------------------------------
    // simple accessors
    //-----------------------------------------------------------------------

    /// Returns the rule's decimal point character (fraction rules).
    /// Equivalent to `getDecimalPoint()`.
    pub(crate) fn get_decimal_point(&self) -> u16 {
        self.decimal_point
    }

    /// Returns the rule's base value. Equivalent to `getBaseValue()`.
    pub(crate) fn get_base_value(&self) -> i64 {
        self.base_value
    }

    /// Returns the rule's divisor (the value that cotrols the behavior of
    /// its substitutions). Equivalent to `getDivisor()`.
    ///
    /// # Errors
    ///
    /// The errors of [`power`].
    pub(crate) fn get_divisor(&self) -> Result<i64> {
        power(i64::from(self.radix), self.exponent)
    }

    /// Whether one of the substitutions is a `ModulusSubstitution`.
    /// Equivalent to `hasModulusSubstitution()`.
    pub(crate) fn has_modulus_substitution(&self) -> bool {
        self.sub1.as_ref().is_some_and(NFSubstitution::is_modulus_substitution)
            || self.sub2.as_ref().is_some_and(NFSubstitution::is_modulus_substitution)
    }

    //-----------------------------------------------------------------------
    // formatting
    //-----------------------------------------------------------------------

    /// Formats the number, and inserts the resulting text into
    /// `to_insert_into` at `pos`. Equivalent to `doFormat(long number,
    /// StringBuilder toInsertInto, int pos, int recursionCount)`.
    ///
    /// # Errors
    ///
    /// The errors of the plural format and of the substitutions.
    pub(crate) fn do_format_long(
        &self,
        number: i64,
        to_insert_into: &mut Vec<u16>,
        pos: i32,
        recursion_count: i32,
        g: &RuleGraph,
    ) -> Result<()> {
        // first, insert the rule's rule text into toInsertInto at the
        // specified position, then insert the results of the substitutions
        // into the right places in toInsertInto (notice we do the
        // substitutions in reverse order so that the offsets don't get
        // messed up)
        let mut plural_rule_start = self.rule_text.len() as i32;
        let mut length_offset = 0;
        match &self.rule_pattern_format {
            None => sb_insert(to_insert_into, pos, &self.rule_text),
            Some(rule_pattern_format) => {
                plural_rule_start = index_of(&self.rule_text, &units("$("), 0);
                let plural_rule_end = index_of(&self.rule_text, &units(")$"), plural_rule_start);
                let initial_length = to_insert_into.len() as i32;
                if plural_rule_end < self.rule_text.len() as i32 - 1 {
                    sb_insert(to_insert_into, pos, substring_from(&self.rule_text, plural_rule_end + 2));
                }
                let divisor = power(i64::from(self.radix), self.exponent)?;
                let quotient = checked_div(number, divisor)?;
                sb_insert(to_insert_into, pos, &rule_pattern_format.format_f64(quotient as f64)?);
                if plural_rule_start > 0 {
                    sb_insert(to_insert_into, pos, substring(&self.rule_text, 0, plural_rule_start));
                }
                length_offset = self.rule_text.len() as i32 - (to_insert_into.len() as i32 - initial_length);
            }
        }
        if let Some(sub2) = &self.sub2 {
            let offset = if sub2.get_pos() > plural_rule_start { length_offset } else { 0 };
            sub2.do_substitution_long(number, to_insert_into, pos - offset, recursion_count, self, g)?;
        }
        if let Some(sub1) = &self.sub1 {
            let offset = if sub1.get_pos() > plural_rule_start { length_offset } else { 0 };
            sub1.do_substitution_long(number, to_insert_into, pos - offset, recursion_count, self, g)?;
        }
        Ok(())
    }

    /// Formats the number, and inserts the resulting text into
    /// `to_insert_into` at `pos`. Equivalent to `doFormat(double number,
    /// StringBuilder toInsertInto, int pos, int recursionCount)`.
    ///
    /// # Errors
    ///
    /// As [`do_format_long`](Self::do_format_long).
    pub(crate) fn do_format_double(
        &self,
        number: f64,
        to_insert_into: &mut Vec<u16>,
        pos: i32,
        recursion_count: i32,
        g: &RuleGraph,
    ) -> Result<()> {
        // first, insert the rule's rule text into toInsertInto at the
        // specified position, then insert the results of the substitutions
        // into the right places in toInsertInto
        // [again, we have two copies of this routine that do the same thing
        // so that we don't sacrifice precision in a long by casting it
        // to a double]
        let mut plural_rule_start = self.rule_text.len() as i32;
        let mut length_offset = 0;
        match &self.rule_pattern_format {
            None => sb_insert(to_insert_into, pos, &self.rule_text),
            Some(rule_pattern_format) => {
                plural_rule_start = index_of(&self.rule_text, &units("$("), 0);
                let plural_rule_end = index_of(&self.rule_text, &units(")$"), plural_rule_start);
                let initial_length = to_insert_into.len() as i32;
                if plural_rule_end < self.rule_text.len() as i32 - 1 {
                    sb_insert(to_insert_into, pos, substring_from(&self.rule_text, plural_rule_end + 2));
                }
                let mut plural_val = number;
                let divisor = power(i64::from(self.radix), self.exponent)? as f64;
                if (0.0..1.0).contains(&plural_val) {
                    // We're in a fractional rule, and we have to match the NumeratorSubstitution behavior.
                    // 2.3 can become 0.2999999999999998 for the fraction due to rounding errors.
                    plural_val = java_math_round(plural_val * divisor) as f64;
                } else {
                    plural_val /= divisor;
                }
                sb_insert(to_insert_into, pos, &rule_pattern_format.format_f64((plural_val as i64) as f64)?);
                if plural_rule_start > 0 {
                    sb_insert(to_insert_into, pos, substring(&self.rule_text, 0, plural_rule_start));
                }
                length_offset = self.rule_text.len() as i32 - (to_insert_into.len() as i32 - initial_length);
            }
        }
        if let Some(sub2) = &self.sub2 {
            let offset = if sub2.get_pos() > plural_rule_start { length_offset } else { 0 };
            sub2.do_substitution_double(number, to_insert_into, pos - offset, recursion_count, self, g)?;
        }
        if let Some(sub1) = &self.sub1 {
            let offset = if sub1.get_pos() > plural_rule_start { length_offset } else { 0 };
            sub1.do_substitution_double(number, to_insert_into, pos - offset, recursion_count, self, g)?;
        }
        Ok(())
    }

    /// Used by the rule set to determine whether a rule should be rolled
    /// back (the rule has a modulus substitution, the number is an even
    /// multiple of the divisor and the base value is not). Equivalent to
    /// `shouldRollBack(long number)`.
    ///
    /// # Errors
    ///
    /// The errors of [`power`]; `ArithmeticException("/ by zero")` for a
    /// zero divisor.
    pub(crate) fn should_roll_back(&self, number: i64) -> Result<bool> {
        // we roll back if the rule contains a modulus substitution,
        // the number being formatted is an even multiple of the rule's
        // divisor, and the rule's base value is NOT an even multiple
        // of its divisor
        // In other words, if the original description had
        //    100: << hundred[ >>];
        // that expands into
        //    100: << hundred;
        //    101: << hundred >>;
        // internally.  But when we're formatting 200, if we use the rule
        // at 101, which would normally apply, we get "two hundred zero".
        // To prevent this, we roll back and use the rule at 100 instead.
        // This is the logic that makes this happen: the rule at 101 has
        // a modulus substitution, its base value isn't an even multiple
        // of 100, and the value we're trying to format _is_ an even
        // multiple of 100.  This is called the "rollback rule."
        if !(self.sub1.as_ref().is_some_and(NFSubstitution::is_modulus_substitution)
            || self.sub2.as_ref().is_some_and(NFSubstitution::is_modulus_substitution))
        {
            return Ok(false);
        }
        let divisor = power(i64::from(self.radix), self.exponent)?;
        Ok(checked_rem(number, divisor)? == 0 && checked_rem(self.base_value, divisor)? != 0)
    }

    //-----------------------------------------------------------------------
    // parsing
    //-----------------------------------------------------------------------

    /// Attempts to match the text against the rule, and returns the value
    /// of the match (a `Long` or a `Double`); `parse_position` is left at
    /// the first unmatched character, or unchanged when nothing matched.
    /// Equivalent to `doParse(String text, ParsePosition parsePosition,
    /// boolean isFractionRule, double upperBound, int
    /// nonNumericalExecutedRuleMask, int recursionCount)`.
    ///
    /// # Errors
    ///
    /// The errors of the lenient scanner and of the substitutions;
    /// `NullPointerException` where Java dereferences a `null` partial
    /// result.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn do_parse(
        &self,
        text: &[u16],
        parse_position: &mut ParsePosition,
        is_fraction_rule: bool,
        upper_bound: f64,
        non_numerical_executed_rule_mask: i32,
        recursion_count: i32,
        g: &RuleGraph,
    ) -> Result<JavaNumber> {
        // internally we operate on a copy of the string being parsed
        // (because we're going to change it) and use our own ParsePosition
        let mut pp = ParsePosition::new(0);

        // check to see whether the text before the first substitution
        // matches the text at the beginning of the string being
        // parsed.  If it does, strip that off the front of workText;
        // otherwise, dump out with a mismatch
        let sub1_pos = self.sub1.as_ref().map_or(self.rule_text.len() as i32, NFSubstitution::get_pos);
        let sub2_pos = self.sub2.as_ref().map_or(self.rule_text.len() as i32, NFSubstitution::get_pos);
        let work_text = self.strip_prefix(text, substring(&self.rule_text, 0, sub1_pos), &mut pp, g)?;
        let prefix_length = text.len() as i32 - work_text.len() as i32;

        if pp.get_index() == 0 && sub1_pos != 0 {
            // commented out because ParsePosition doesn't have error index in 1.1.x
            //                parsePosition.setErrorIndex(pp.getErrorIndex());
            return Ok(ZERO);
        }
        if self.base_value == INFINITY_RULE {
            // If you match this, don't try to perform any calculations on it.
            parse_position.set_index(pp.get_index());
            return Ok(JavaNumber::Double(f64::INFINITY));
        }
        if self.base_value == NAN_RULE {
            // If you match this, don't try to perform any calculations on it.
            parse_position.set_index(pp.get_index());
            return Ok(JavaNumber::Double(f64::NAN));
        }

        // this is the fun part.  The basic guts of the rule-matching
        // logic is matchToDelimiter(), which is called twice.  The first
        // time it searches the input string for the rule text BETWEEN
        // the substitutions and tries to match the intervening text
        // in the input string with the first substitution.  If that
        // succeeds, it then calls it again, this time to look for the
        // rule text after the second substitution and to match the
        // intervening input text against the second substitution.
        //
        // For example, say we have a rule that looks like this:
        //    first << middle >> last;
        // and input text that looks like this:
        //    first one middle two last
        // First we use stripPrefix() to match "first " in both places and
        // strip it off the front, leaving
        //    one middle two last
        // Then we use matchToDelimiter() to match " middle " and try to
        // match "one" against a substitution.  If it's successful, we now
        // have
        //    two last
        // We use matchToDelimiter() a second time to match " last" and
        // try to match "two" against a substitution.  If "two" matches
        // the substitution, we have a successful parse.
        //
        // Since it's possible in many cases to find multiple instances
        // of each of these pieces of rule text in the input string,
        // we need to try all the possible combinations of these
        // locations.  This prevents us from prematurely declaring a mismatch,
        // and makes sure we match as much input text as we can.
        let mut high_water_mark = 0i32;
        let mut result = 0f64;
        let mut start = 0i32;
        let temp_base_value = 0f64.max(self.base_value as f64);

        loop {
            // our partial parse result starts out as this rule's base
            // value.  If it finds a successful match, matchToDelimiter()
            // will compose this in some way with what it gets back from
            // the substitution, giving us a new partial parse result
            pp.set_index(0);
            let mut partial_result = self
                .match_to_delimiter(
                    &work_text,
                    start,
                    temp_base_value,
                    substring(&self.rule_text, sub1_pos, sub2_pos),
                    self.rule_pattern_format.as_ref(),
                    &mut pp,
                    self.sub1.as_ref(),
                    upper_bound,
                    non_numerical_executed_rule_mask,
                    recursion_count,
                    g,
                )?
                .ok_or_else(npe)?
                .double_value();

            // if we got a successful match (or were trying to match a
            // null substitution), pp is now pointing at the first unmatched
            // character.  Take note of that, and try matchToDelimiter()
            // on the input text again
            if pp.get_index() != 0 || self.sub1.is_none() {
                start = pp.get_index();

                let work_text2 = substring_from(&work_text, pp.get_index());
                let mut pp2 = ParsePosition::new(0);

                // the second matchToDelimiter() will compose our previous
                // partial result with whatever it gets back from its
                // substitution if there's a successful match, giving us
                // a real result
                partial_result = self
                    .match_to_delimiter(
                        work_text2,
                        0,
                        partial_result,
                        substring_from(&self.rule_text, sub2_pos),
                        self.rule_pattern_format.as_ref(),
                        &mut pp2,
                        self.sub2.as_ref(),
                        upper_bound,
                        non_numerical_executed_rule_mask,
                        recursion_count,
                        g,
                    )?
                    .ok_or_else(npe)?
                    .double_value();

                // if we got a successful match on this second
                // matchToDelimiter() call, update the high-water mark
                // and result (if necessary)
                if (pp2.get_index() != 0 || self.sub2.is_none())
                    && prefix_length + pp.get_index() + pp2.get_index() > high_water_mark
                {
                    high_water_mark = prefix_length + pp.get_index() + pp2.get_index();
                    result = partial_result;
                }
                // commented out because ParsePosition doesn't have error index in 1.1.x
                //                    else {
                //                        int temp = pp2.getErrorIndex() + sub1.getPos() + pp.getIndex();
                //                        if (temp> parsePosition.getErrorIndex()) {
                //                            parsePosition.setErrorIndex(temp);
                //                        }
                //                    }
            }
            // commented out because ParsePosition doesn't have error index in 1.1.x
            //                else {
            //                    int temp = sub1.getPos() + pp.getErrorIndex();
            //                    if (temp > parsePosition.getErrorIndex()) {
            //                        parsePosition.setErrorIndex(temp);
            //                    }
            //                }
            // keep trying to match things until the outer matchToDelimiter()
            // call fails to make a match (each time, it picks up where it
            // left off the previous time)
            if !(sub1_pos != sub2_pos
                && pp.get_index() > 0
                && pp.get_index() < work_text.len() as i32
                && pp.get_index() != start)
            {
                break;
            }
        }

        // update the caller's ParsePosition with our high-water mark
        // (i.e., it now points at the first character this function
        // didn't match-- the ParsePosition is therefore unchanged if
        // we didn't match anything)
        parse_position.set_index(high_water_mark);
        // commented out because ParsePosition doesn't have error index in 1.1.x
        //        if (highWaterMark > 0) {
        //            parsePosition.setErrorIndex(0);
        //        }

        // this is a hack for one unusual condition: Normally, whether this
        // rule belong to a fraction rule set or not is handled by its
        // substitutions.  But if that rule HAS NO substitutions, then
        // we have to account for it here.  By definition, if the matching
        // rule in a fraction rule set has no substitutions, its numerator
        // is 1, and so the result is the reciprocal of its base value.
        if is_fraction_rule && high_water_mark > 0 && self.sub1.is_none() {
            result = 1.0 / result;
        }

        // return the result as a Long if possible, or as a Double
        Ok(long_or_double(result))
    }

    /// This function is used by parse() to match the text being parsed
    /// against a possible prefix string; returns the text with the prefix
    /// removed when it matched, `pp` advanced by the prefix's length.
    /// Equivalent to `stripPrefix(String text, String prefix, ParsePosition
    /// pp)` (private).
    ///
    /// # Errors
    ///
    /// The errors of the lenient scanner.
    fn strip_prefix(&self, text: &[u16], prefix: &[u16], pp: &mut ParsePosition, g: &RuleGraph) -> Result<Vec<u16>> {
        // if the prefix text is empty, dump out without doing anything
        if prefix.is_empty() {
            Ok(text.to_vec())
        } else {
            // otherwise, use prefixLength() to match the beginning of
            // "text" against "prefix".  This function returns the
            // number of characters from "text" that matched (or 0 if
            // we didn't match the whole prefix)
            let pfl = self.prefix_length(text, prefix, g)?;
            if pfl != 0 {
                // if we got a successful match, update the parse position
                // and strip the prefix off of "text"
                pp.set_index(pp.get_index() + pfl);
                Ok(substring_from(text, pfl).to_vec())

                // if we didn't get a successful match, leave everything alone
            } else {
                Ok(text.to_vec())
            }
        }
    }

    /// Used by parse() to match a substitution and any following text
    /// ("following text" is defined as any text up to the next substitution
    /// token or the end of the rule): searches for the delimiter, and
    /// parses the text before it with the substitution. Equivalent to
    /// `matchToDelimiter(String text, int startPos, double baseVal, String
    /// delimiter, PluralFormat pluralFormatDelimiter, ParsePosition pp,
    /// NFSubstitution sub, double upperBound, int
    /// nonNumericalExecutedRuleMask, int recursionCount)` (private); `None`
    /// is a `null` result of the substitution.
    ///
    /// # Errors
    ///
    /// The errors of the lenient scanner and of the substitution;
    /// `NullPointerException` for a delimiter preceded by text and no
    /// substitution.
    #[allow(clippy::too_many_arguments)]
    fn match_to_delimiter(
        &self,
        text: &[u16],
        start_pos: i32,
        base_val: f64,
        delimiter: &[u16],
        plural_format_delimiter: Option<&PluralFormat>,
        pp: &mut ParsePosition,
        sub: Option<&NFSubstitution>,
        upper_bound: f64,
        non_numerical_executed_rule_mask: i32,
        recursion_count: i32,
        g: &RuleGraph,
    ) -> Result<Option<JavaNumber>> {
        // if "delimiter" contains real (i.e., non-ignorable) text, search
        // it for "delimiter" beginning at "start".  If that succeeds, then
        // use "sub"'s doParse() method to match the text before the
        // instance of "delimiter" we just found.
        if !self.all_ignorable(Some(delimiter), g)? {
            let mut temp_pp = ParsePosition::new(0);

            // use findText() to search for "delimiter".  It returns a two-
            // element array: element 0 is the position of the match, and
            // element 1 is the number of characters that matched
            // "delimiter".
            let mut temp = self.find_text(text, delimiter, plural_format_delimiter, start_pos, g)?;
            let mut d_pos = temp[0];
            let mut d_len = temp[1];

            // if findText() succeeded, isolate the text preceding the
            // match, and use "sub" to match that text
            while d_pos >= 0 {
                let sub_text = substring(text, 0, d_pos);
                if !sub_text.is_empty() {
                    let sub = sub.ok_or_else(npe)?;
                    let lenient = g.owner().lenient_parse_enabled();
                    let temp_result = sub.do_parse(
                        sub_text,
                        &mut temp_pp,
                        base_val,
                        upper_bound,
                        lenient,
                        non_numerical_executed_rule_mask,
                        recursion_count,
                        g,
                    )?;

                    // if the substitution could match all the text up to
                    // where we found "delimiter", then this function has
                    // a successful match.  Bump the caller's parse position
                    // to point to the first character after the text
                    // that matches "delimiter", and return the result
                    // we got from parsing the substitution.
                    if temp_pp.get_index() == d_pos {
                        pp.set_index(d_pos + d_len);
                        return Ok(temp_result);
                    }
                    // commented out because ParsePosition doesn't have error index in 1.1.x
                    //                    else {
                    //                        if (tempPP.getErrorIndex() > 0) {
                    //                            pp.setErrorIndex(tempPP.getErrorIndex());
                    //                        } else {
                    //                            pp.setErrorIndex(tempPP.getIndex());
                    //                        }
                    //                    }
                }

                // if we didn't match the substitution, search for another
                // copy of "delimiter" in "text" and repeat the loop if
                // we find it
                temp_pp.set_index(0);
                temp = self.find_text(text, delimiter, plural_format_delimiter, d_pos + d_len, g)?;
                d_pos = temp[0];
                d_len = temp[1];
            }
            // if we make it here, this was an unsuccessful match, and we
            // leave pp unchanged and return 0
            pp.set_index(0);
            Ok(Some(ZERO))

            // if "delimiter" is empty, or consists only of ignorable characters
            // (i.e., is semantically empty), thwe we obviously can't search
            // for "delimiter".  Instead, just use "sub" to parse as much of
            // "text" as possible.
        } else if let Some(sub) = sub {
            let mut temp_pp = ParsePosition::new(0);
            let mut result = ZERO;
            // try to match the whole string against the substitution
            let lenient = g.owner().lenient_parse_enabled();
            let temp_result = sub.do_parse(
                text,
                &mut temp_pp,
                base_val,
                upper_bound,
                lenient,
                non_numerical_executed_rule_mask,
                recursion_count,
                g,
            )?;
            if temp_pp.get_index() != 0 {
                // if there's a successful match (or it's a null
                // substitution), update pp to point to the first
                // character we didn't match, and pass the result from
                // sub.doParse() on through to the caller
                pp.set_index(temp_pp.get_index());
                if let Some(temp_result) = temp_result {
                    result = temp_result;
                }
            }
            // commented out because ParsePosition doesn't have error index in 1.1.x
            //            else {
            //                pp.setErrorIndex(tempPP.getErrorIndex());
            //            }

            // and if we get to here, then nothing matched, so we return
            // 0 and leave pp alone
            Ok(Some(result))
        } else {
            Ok(Some(JavaNumber::Double(base_val)))
        }
    }

    /// Used by stripPrefix() to match characters: the number of characters
    /// of `str` that matched `prefix`, 0 when it did not match. Lenient
    /// parsing uses the formatter's scanner. Equivalent to
    /// `prefixLength(String str, String prefix)` (private).
    ///
    /// # Errors
    ///
    /// The errors of the lenient scanner.
    fn prefix_length(&self, str: &[u16], prefix: &[u16], g: &RuleGraph) -> Result<i32> {
        // if we're looking for an empty prefix, it obviously matches
        // zero characters.  Just go ahead and return 0.
        if prefix.is_empty() {
            return Ok(0);
        }

        let scanner = g.owner().get_lenient_scanner()?;
        if let Some(scanner) = scanner {
            // Check if non-lenient rule finds the text before call lenient parsing
            if starts_with(str, prefix) {
                return Ok(prefix.len() as i32);
            }
            return Ok(scanner.prefix_length(str, prefix));
        }

        // If lenient parsing is turned off, forget all that crap above.
        // Just use String.startsWith() and be done with it.
        if starts_with(str, prefix) {
            return Ok(prefix.len() as i32);
        }
        Ok(0)
    }

    /// Searches a string for another string: returns the position and the
    /// length of the match, `[-1, 0]` when none. With a plural format, its
    /// keyword match is looked up together with the rule's prefix and
    /// suffix. Equivalent to `findText(String str, String key, PluralFormat
    /// pluralFormatKey, int startingAt)` (private).
    ///
    /// # Errors
    ///
    /// The errors of the lenient scanner.
    fn find_text(
        &self,
        str: &[u16],
        key: &[u16],
        plural_format_key: Option<&PluralFormat>,
        starting_at: i32,
        g: &RuleGraph,
    ) -> Result<[i32; 2]> {
        let scanner = g.owner().get_lenient_scanner()?;
        if let Some(plural_format_key) = plural_format_key {
            let mut position = FieldPosition::new(INTEGER_FIELD);
            position.set_begin_index(starting_at);
            plural_format_key.parse_type(str, scanner.as_deref(), &mut position);
            let start = position.get_begin_index();
            if start >= 0 {
                let plural_rule_start = index_of(&self.rule_text, &units("$("), 0);
                let plural_rule_suffix = index_of(&self.rule_text, &units(")$"), plural_rule_start) + 2;
                let match_len = position.get_end_index() - start;
                let prefix = substring(&self.rule_text, 0, plural_rule_start);
                let suffix = substring_from(&self.rule_text, plural_rule_suffix);
                if region_matches(str, i64::from(start) - prefix.len() as i64, prefix, 0, prefix.len() as i64)
                    && region_matches(str, i64::from(start + match_len), suffix, 0, suffix.len() as i64)
                {
                    return Ok([start - prefix.len() as i32, match_len + prefix.len() as i32 + suffix.len() as i32]);
                }
            }
            return Ok([-1, 0]);
        }

        if let Some(scanner) = scanner {
            // Check if non-lenient rule finds the text before call lenient parsing
            let pos = [index_of(str, key, starting_at), key.len() as i32];
            if pos[0] >= 0 {
                return Ok(pos);
            } else {
                // if lenient parsing is turned ON, we've got some work ahead of us
                return Ok(scanner.find_text(str, key, starting_at));
            }
        }
        // if lenient parsing is turned off, this is easy. Just call
        // String.indexOf() and we're done
        Ok([index_of(str, key, starting_at), key.len() as i32])
    }

    /// Checks to see whether a string consists entirely of ignorable
    /// characters (only possible with lenient parsing). Equivalent to
    /// `allIgnorable(String str)` (private).
    ///
    /// # Errors
    ///
    /// The errors of the lenient scanner.
    fn all_ignorable(&self, str: Option<&[u16]>, g: &RuleGraph) -> Result<bool> {
        // if the string is empty, we can just return true
        let str = match str {
            None => return Ok(true),
            Some(s) if s.is_empty() => return Ok(true),
            Some(s) => s,
        };
        let scanner = g.owner().get_lenient_scanner()?;
        Ok(scanner.is_some_and(|s| s.all_ignorable(str)))
    }

    /// Applies new decimal format symbols to the substitutions' decimal
    /// formats. Equivalent to `setDecimalFormatSymbols(DecimalFormatSymbols
    /// newSymbols)`.
    ///
    /// # Errors
    ///
    /// The errors of the decimal formats.
    pub(crate) fn set_decimal_format_symbols(&self, new_symbols: &DecimalFormatSymbols) -> Result<()> {
        if let Some(sub1) = &self.sub1 {
            sub1.set_decimal_format_symbols(new_symbols)?;
        }
        if let Some(sub2) = &self.sub2 {
            sub2.set_decimal_format_symbols(new_symbols)?;
        }
        Ok(())
    }
}

/// `new IllegalArgumentException("Illegal character " + c + " in rule
/// descriptor")`.
fn illegal_character(c: u16) -> LuceneError {
    LuceneError::illegal_argument(format!("Illegal character {} in rule descriptor", String::from_utf16_lossy(&[c])))
}

/// Searches the rule text for the first of the substitution tokens.
/// Equivalent to `indexOfAnyRulePrefix(String ruleText)` (private static).
fn index_of_any_rule_prefix(rule_text: &[u16]) -> i32 {
    let mut result = -1;
    if !rule_text.is_empty() {
        for string in RULE_PREFIXES {
            let pos = index_of(rule_text, &units(string), 0);
            if pos != -1 && (result == -1 || pos < result) {
                result = pos;
            }
        }
    }
    result
}

/// This is an equivalent to Math.pow that accurately works on 64-bit
/// numbers (wrapping as Java's `long` arithmetic). Equivalent to `power(long
/// base, short exponent)` (static).
///
/// # Errors
///
/// `IllegalArgumentException("Exponent can not be negative")`,
/// `IllegalArgumentException("Base can not be negative")`.
pub(crate) fn power(base: i64, exponent: i16) -> Result<i64> {
    if exponent < 0 {
        return Err(LuceneError::illegal_argument("Exponent can not be negative"));
    }
    if base < 0 {
        return Err(LuceneError::illegal_argument("Base can not be negative"));
    }
    let mut base = base;
    let mut exponent = exponent;
    let mut result: i64 = 1;
    while exponent > 0 {
        if (exponent & 1) == 1 {
            result = result.wrapping_mul(base);
        }
        base = base.wrapping_mul(base);
        exponent >>= 1;
    }
    Ok(result)
}

/// Java's `long` division `a / b`.
///
/// # Errors
///
/// `ArithmeticException("/ by zero")`.
pub(crate) fn checked_div(a: i64, b: i64) -> Result<i64> {
    if b == 0 {
        return Err(crate::jdk::arithmetic("/ by zero"));
    }
    Ok(a.wrapping_div(b))
}

/// Java's `long` remainder `a % b`.
///
/// # Errors
///
/// `ArithmeticException("/ by zero")`.
pub(crate) fn checked_rem(a: i64, b: i64) -> Result<i64> {
    if b == 0 {
        return Err(crate::jdk::arithmetic("/ by zero"));
    }
    Ok(a.wrapping_rem(b))
}
