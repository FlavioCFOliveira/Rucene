// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.text.NFSubstitution` and its package-private subclasses
//! `SameValueSubstitution`, `MultiplierSubstitution`, `ModulusSubstitution`,
//! `IntegralPartSubstitution`, `FractionalPartSubstitution`,
//! `AbsoluteValueSubstitution` and `NumeratorSubstitution`, declared in
//! `NFSubstitution.java`.

use rucene::error::{LuceneError, Result};

use crate::jdk::{units, ParsePosition};
use crate::math::big_decimal::BigDecimal as IcuBigDecimal;
use crate::r#impl::number::decimal_quantity::DecimalQuantity;
use crate::r#impl::number::decimal_quantity_abstract_bcd::DecimalQuantityAbstractBcd;
use crate::r#impl::number::decimal_quantity_dual_storage_bcd::{DecimalQuantity_DualStorageBCD, JavaNumber};
use crate::text::decimal_format::DecimalFormat;
use crate::text::decimal_format_symbols::DecimalFormatSymbols;
use crate::text::nf_rule::{self, checked_div, checked_rem, NFRule};
use crate::text::nf_rule_set::BuildContext;
use crate::text::number_format::NumberFormat;
use crate::text::rule_based_number_format::{
    ends_with, java_math_round, long_or_double, npe, number_int_value, number_long_value, sb_insert, substring,
    substring_from, RuleGraph,
};

/// `MAX_INT64_IN_DOUBLE` (private): the largest `long` a `double` holds
/// exactly.
const MAX_INT64_IN_DOUBLE: i64 = 0x1FFFFFFFFFFFFF;

/// The concrete class of a substitution and its own fields.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Kind {
    /// `SameValueSubstitution`: a substitution that passes the value passed
    /// to it through unchanged. Represented by == in rule descriptions.
    SameValue,
    /// `MultiplierSubstitution`: a substitution that divides the number
    /// being formatted by the rule's divisor and formats the quotient.
    /// Represented by << in normal rules.
    Multiplier {
        /// `divisor`: the divisor of the rule that owns this substitution.
        divisor: i64,
    },
    /// `ModulusSubstitution`: a substitution that divides the number being
    /// formatted by its rule's divisor and formats the remainder.
    /// Represented by ">>" in a regular rule.
    Modulus {
        /// `divisor`: the divisor of the rule owning this substitution.
        divisor: i64,
        /// `ruleToUse`: if this is a >>> substitution, the rule to use to
        /// format the substitution value (the rule set index and the
        /// index of the rule in it); otherwise, `None`.
        rule_to_use: Option<(usize, usize)>,
    },
    /// `IntegralPartSubstitution`: a substitution that formats the number's
    /// integral part. This is represented by << in a fraction rule.
    IntegralPart,
    /// `FractionalPartSubstitution`: a substitution that formats the
    /// fractional part of a number. This is represented by >> in a fraction
    /// rule.
    FractionalPart {
        /// `byDigits`: true if we're formatting "byDigits".
        by_digits: bool,
        /// `useSpaces`: true if we separate digits with spaces.
        use_spaces: bool,
    },
    /// `AbsoluteValueSubstitution`: a substitution that formats the absolute
    /// value of the number. This substitution is represented by >> in a
    /// negative-number rule.
    AbsoluteValue,
    /// `NumeratorSubstitution`: a substitution that multiplies the number
    /// being formatted (which is between 0 and 1) by the base value of the
    /// rule that owns it and formats the result. It is represented by <<
    /// in the rules in a fraction rule set.
    Numerator {
        /// `denominator`: this hold the multiplier.
        denominator: f64,
        /// `withZeros`: whether the substitution emits the leading zeros
        /// of the decimal expansion.
        with_zeros: bool,
    },
}

/// An abstract class defining protocol for substitutions. A substitution
/// is a section of a rule that inserts text into the rule's rule text based
/// on some part of the number being formatted.
///
/// Equivalent to the package-private abstract class
/// `com.ibm.icu.text.NFSubstitution` and its seven subclasses, one type
/// whose `kind` is the concrete class: the overridden methods dispatch on
/// it. The rule set a substitution formats with is an index into the
/// formatter's [`RuleGraph`] (see its divergence note); the rule owning a
/// `MultiplierSubstitution` (`owningRule`) is the rule that calls it, which
/// passes itself.
#[derive(Debug)]
pub(crate) struct NFSubstitution {
    /// `pos`: the substitution's position in the rule text of the rule that
    /// owns it.
    pos: i32,
    /// `ruleSet`: the rule set this substitution uses to format its result,
    /// or `None`. (Either this or `number_format` has to be `None`.)
    rule_set: Option<usize>,
    /// `numberFormat`: the DecimalFormat this substitution uses to format
    /// its result, or `None`. (Either this or `rule_set` has to be `None`.)
    number_format: Option<DecimalFormat>,
    /// The concrete class and its fields.
    kind: Kind,
}

impl NFSubstitution {
    //-----------------------------------------------------------------------
    // construction
    //-----------------------------------------------------------------------

    /// Parses the description, creates the right kind of substitution, and
    /// initializes it based on the description. `rule_set` is the index of
    /// the rule set containing the rule, `rule_predecessor` the index of the
    /// rule before it in that rule set's rule list. Equivalent to the
    /// static `makeSubstitution(int pos, NFRule rule, NFRule
    /// rulePredecessor, NFRuleSet ruleSet, RuleBasedNumberFormat formatter,
    /// String description)`; `None` is Java's `null` (a null
    /// substitution).
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("<< not allowed in negative-number rule")`,
    /// `IllegalArgumentException(">> not allowed in fraction rule set")`,
    /// `IllegalArgumentException("Illegal substitution character")`; the
    /// errors of the substitution constructors.
    pub(crate) fn make_substitution(
        pos: i32,
        rule: &NFRule,
        rule_predecessor: Option<usize>,
        rule_set: usize,
        ctx: &BuildContext<'_>,
        description: &[u16],
    ) -> Result<Option<NFSubstitution>> {
        // if the description is empty, return a NullSubstitution
        if description.is_empty() {
            return Ok(None);
        }

        let base_value = rule.get_base_value();
        let is_fraction_rule = base_value == nf_rule::IMPROPER_FRACTION_RULE
            || base_value == nf_rule::PROPER_FRACTION_RULE
            || base_value == nf_rule::DEFAULT_RULE;
        let substitution = match description[0] {
            0x3C /* '<' */ => {
                if base_value == nf_rule::NEGATIVE_NUMBER_RULE {
                    // throw an exception if the rule is a negative number rule
                    //CLOVER:OFF
                    // If you look at the call hierarchy of this method, the rule would
                    // never be directly modified by the user and therefore makes the
                    // following pointless unless the user changes the ruleset.
                    return Err(LuceneError::illegal_argument("<< not allowed in negative-number rule"));
                    //CLOVER:ON
                } else if is_fraction_rule {
                    // if the rule is a fraction rule, return an IntegralPartSubstitution
                    Self::new(pos, Some(rule_set), description, Kind::IntegralPart, ctx)?
                } else if ctx.rule_sets[rule_set].is_fraction_set() {
                    // if the rule set containing the rule is a fraction
                    // rule set, return a NumeratorSubstitution
                    Self::new_numerator(pos, base_value as f64, ctx.default_rule_set, description, ctx)?
                } else {
                    // otherwise, return a MultiplierSubstitution
                    Self::new_multiplier(pos, rule, Some(rule_set), description, ctx)?
                }
            }
            0x3E /* '>' */ => {
                if base_value == nf_rule::NEGATIVE_NUMBER_RULE {
                    // if the rule is a negative-number rule, return
                    // an AbsoluteValueSubstitution
                    Self::new(pos, Some(rule_set), description, Kind::AbsoluteValue, ctx)?
                } else if is_fraction_rule {
                    // if the rule is a fraction rule, return a
                    // FractionalPartSubstitution
                    Self::new_fractional_part(pos, Some(rule_set), description, ctx)?
                } else if ctx.rule_sets[rule_set].is_fraction_set() {
                    // if the rule set owning the rule is a fraction rule set,
                    // throw an exception
                    //CLOVER:OFF
                    // If you look at the call hierarchy of this method, the rule would
                    // never be directly modified by the user and therefore makes the
                    // following pointless unless the user changes the ruleset.
                    return Err(LuceneError::illegal_argument(">> not allowed in fraction rule set"));
                    //CLOVER:ON
                } else {
                    // otherwise, return a ModulusSubstitution
                    Self::new_modulus(pos, rule, rule_predecessor, rule_set, description, ctx)?
                }
            }
            0x3D /* '=' */ => Self::new_same_value(pos, Some(rule_set), description, ctx)?,
            _ => {
                // and if it's anything else, throw an exception
                //CLOVER:OFF
                // If you look at the call hierarchy of this method, the rule would
                // never be directly modified by the user and therefore makes the
                // following pointless unless the user changes the ruleset.
                return Err(LuceneError::illegal_argument("Illegal substitution character"));
                //CLOVER:ON
            }
        };
        Ok(Some(substitution))
    }

    /// Base constructor for substitutions: strips the token characters,
    /// then resolves the rule set or the decimal format of the description.
    /// Equivalent to `NFSubstitution(int pos, NFRuleSet ruleSet, String
    /// description)` (the subclass constructors that only call it, too).
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("Illegal substitution syntax")`,
    /// `IllegalArgumentException("No rule set named <name>")`;
    /// `NullPointerException` for a named rule set or a pattern when
    /// `rule_set` is `None`; the errors of the decimal format.
    fn new(pos: i32, rule_set: Option<usize>, description: &[u16], kind: Kind, ctx: &BuildContext<'_>) -> Result<Self> {
        // initialize the substitution's position in its parent rule
        let description_len = description.len();

        // the description should begin and end with the same character.
        // If it doesn't that's a syntax error.  Otherwise,
        // makeSubstitution() was the only thing that needed to know
        // about these characters, so strip them off
        let description = if description_len >= 2 && description[0] == description[description_len - 1] {
            &description[1..description_len - 1]
        } else if description_len != 0 {
            return Err(LuceneError::illegal_argument("Illegal substitution syntax"));
        } else {
            description
        };

        let (rule_set, number_format) = if description.is_empty() {
            // if the description was just two paired token characters
            // (i.e., "<<" or ">>"), it uses the rule set it belongs to to
            // format its result
            (rule_set, None)
        } else if description[0] == u16::from(b'%') {
            // if the description contains a rule set name, that's the rule
            // set we use to format the result: get a reference to the
            // names rule set
            rule_set.ok_or_else(npe)?;
            (Some(ctx.find_rule_set(description)?), None)
        } else if description[0] == u16::from(b'#') || description[0] == u16::from(b'0') {
            // if the description begins with 0 or #, treat it as a
            // DecimalFormat pattern, and initialize a DecimalFormat with
            // that pattern (then set it to use the DecimalFormatSymbols
            // belonging to our formatter)
            rule_set.ok_or_else(npe)?;
            let number_format = ctx.owner.get_decimal_format()?.clone_decimal_format()?;
            number_format.apply_pattern(Some(description))?;
            (None, Some(number_format))
        } else if description[0] == u16::from(b'>') {
            // if the description is ">>>", this substitution bypasses the
            // usual rule-search process and always uses the rule that precedes
            // it in its own rule set's rule list (this is used for place-value
            // notations: formats where you want to see a particular part of
            // a number even when it's 0)
            (rule_set, None) // was null, thai rules added to control space
        } else {
            // and of the description is none of these things, it's a syntax error
            return Err(LuceneError::illegal_argument("Illegal substitution syntax"));
        };
        Ok(NFSubstitution { pos, rule_set, number_format, kind })
    }

    /// Equivalent to `SameValueSubstitution(int pos, NFRuleSet ruleSet,
    /// String description)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("== is not a legal token")`; the errors of
    /// the base constructor.
    fn new_same_value(pos: i32, rule_set: Option<usize>, description: &[u16], ctx: &BuildContext<'_>) -> Result<Self> {
        let s = Self::new(pos, rule_set, description, Kind::SameValue, ctx)?;
        if description == units("==").as_slice() {
            return Err(LuceneError::illegal_argument("== is not a legal token"));
        }
        Ok(s)
    }

    /// Equivalent to `MultiplierSubstitution(int pos, NFRule rule, NFRuleSet
    /// ruleSet, String description)`.
    ///
    /// # Errors
    ///
    /// `IllegalStateException("Substitution with divisor 0 <text> | <text>")`
    /// for a zero divisor; the errors of the base constructor.
    ///
    /// # Panics
    ///
    /// `StringIndexOutOfBoundsException` when building that message with a
    /// position past the description (as in Java).
    fn new_multiplier(
        pos: i32,
        rule: &NFRule,
        rule_set: Option<usize>,
        description: &[u16],
        ctx: &BuildContext<'_>,
    ) -> Result<Self> {
        let mut s = Self::new(pos, rule_set, description, Kind::SameValue, ctx)?;

        // the owning rule's divisor affects the behavior of this
        // substitution.  Rather than keeping a back-pointer to the
        // rule, we keep a copy of the divisor
        let divisor = rule.get_divisor()?;

        if divisor == 0 {
            // this will cause recursion
            return Err(LuceneError::illegal_state(format!(
                "Substitution with divisor 0 {} | {}",
                String::from_utf16_lossy(substring(description, 0, pos)),
                String::from_utf16_lossy(substring_from(description, pos))
            )));
        }
        s.kind = Kind::Multiplier { divisor };
        Ok(s)
    }

    /// Equivalent to `ModulusSubstitution(int pos, NFRule rule, NFRule
    /// rulePredecessor, NFRuleSet ruleSet, String description)`; a `>>>`
    /// token keeps the predecessor of the rule in its rule set.
    ///
    /// # Errors
    ///
    /// `IllegalStateException("Substitution with bad divisor (0) <text> |
    /// <text>")` for a zero divisor; the errors of the base constructor.
    ///
    /// # Panics
    ///
    /// As [`new_multiplier`](Self::new_multiplier).
    fn new_modulus(
        pos: i32,
        rule: &NFRule,
        rule_predecessor: Option<usize>,
        rule_set: usize,
        description: &[u16],
        ctx: &BuildContext<'_>,
    ) -> Result<Self> {
        let mut s = Self::new(pos, Some(rule_set), description, Kind::SameValue, ctx)?;

        // the owning rule's divisor controls the behavior of this
        // substitution: rather than keeping a backpointer to the rule,
        // we keep a copy of the divisor
        let divisor = rule.get_divisor()?;

        if divisor == 0 {
            // this will cause recursion
            return Err(LuceneError::illegal_state(format!(
                "Substitution with bad divisor ({}) {} | {}",
                divisor,
                String::from_utf16_lossy(substring(description, 0, pos)),
                String::from_utf16_lossy(substring_from(description, pos))
            )));
        }

        // the >>> token doesn't alter how this substitution calculates the
        // values it uses for formatting and parsing, but it changes
        // what's done with that value after it's obtained: >>> short-
        // circuits the rule-search process and goes straight to the
        // specified rule to format the substitution value
        let rule_to_use =
            if description == units(">>>").as_slice() { rule_predecessor.map(|r| (rule_set, r)) } else { None };
        s.kind = Kind::Modulus { divisor, rule_to_use };
        Ok(s)
    }

    /// Equivalent to `FractionalPartSubstitution(int pos, NFRuleSet ruleSet,
    /// String description)`: `>>`, `>>>` and the owning rule set format by
    /// digits; another rule set becomes a fraction rule set.
    ///
    /// # Errors
    ///
    /// `NullPointerException` when the description is a decimal pattern
    /// (Java's `this.ruleSet.makeIntoFractionRuleSet()` on `null`); the
    /// errors of the base constructor.
    fn new_fractional_part(
        pos: i32,
        rule_set: Option<usize>,
        description: &[u16],
        ctx: &BuildContext<'_>,
    ) -> Result<Self> {
        let mut s = Self::new(pos, rule_set, description, Kind::SameValue, ctx)?;
        let (by_digits, use_spaces);
        if description == units(">>").as_slice() || description == units(">>>").as_slice() || rule_set == s.rule_set
        {
            by_digits = true;
            use_spaces = description != units(">>>").as_slice();
        } else {
            by_digits = false;
            use_spaces = true;
            ctx.rule_sets[s.rule_set.ok_or_else(npe)?].make_into_fraction_rule_set();
        }
        s.kind = Kind::FractionalPart { by_digits, use_spaces };
        Ok(s)
    }

    /// Equivalent to `NumeratorSubstitution(int pos, double denominator,
    /// NFRuleSet ruleSet, String description)`: a trailing `<<` asks for
    /// the leading zeros.
    ///
    /// # Errors
    ///
    /// The errors of the base constructor.
    fn new_numerator(
        pos: i32,
        denominator: f64,
        rule_set: Option<usize>,
        description: &[u16],
        ctx: &BuildContext<'_>,
    ) -> Result<Self> {
        let mut s = Self::new(pos, rule_set, &Self::fixdesc(description), Kind::SameValue, ctx)?;

        // this substitution's behavior depends on the rule's base value
        // Rather than keeping a backpointer to the rule, we copy its
        // base value here
        let with_zeros = ends_with(description, &units("<<"));
        s.kind = Kind::Numerator { denominator, with_zeros };
        Ok(s)
    }

    /// `NumeratorSubstitution.fixdesc(String description)` (static): drops
    /// the last character of a description ending in `<<`.
    fn fixdesc(description: &[u16]) -> Vec<u16> {
        if ends_with(description, &units("<<")) {
            substring(description, 0, description.len() as i32 - 1).to_vec()
        } else {
            description.to_vec()
        }
    }

    /// Set's the substitution's divisor (multiplier and modulus
    /// substitutions; a no-op for the others). Equivalent to
    /// `setDivisor(int radix, short exponent)`.
    ///
    /// # Errors
    ///
    /// `IllegalStateException("Substitution with divisor 0")`,
    /// `IllegalStateException("Substitution with bad divisor")`; the errors
    /// of `NFRule.power`.
    pub(crate) fn set_divisor(&mut self, radix: i32, exponent: i16) -> Result<()> {
        match &mut self.kind {
            Kind::Multiplier { divisor } => {
                *divisor = nf_rule::power(i64::from(radix), exponent)?;
                if *divisor == 0 {
                    return Err(LuceneError::illegal_state("Substitution with divisor 0"));
                }
            }
            Kind::Modulus { divisor, .. } => {
                *divisor = nf_rule::power(i64::from(radix), exponent)?;
                if *divisor == 0 {
                    // this will cause recursion
                    return Err(LuceneError::illegal_state("Substitution with bad divisor"));
                }
            }
            // a no-op for all substitutions except multiplier and modulus substitutions
            _ => {}
        }
        Ok(())
    }

    //-----------------------------------------------------------------------
    // boilerplate
    //-----------------------------------------------------------------------

    /// Compares two substitutions for equality: same class, position, rule
    /// set presence and decimal format, and the subclass's fields.
    /// Equivalent to `equals(Object that)` of `NFSubstitution` and its
    /// overrides.
    pub(crate) fn java_equals(&self, that2: &NFSubstitution) -> bool {
        // compare class and all of the fields all substitutions have
        // in common
        if std::ptr::eq(self, that2) {
            return true;
        }
        if std::mem::discriminant(&self.kind) != std::mem::discriminant(&that2.kind) {
            return false;
        }
        let base_equal = self.pos == that2.pos
            && (self.rule_set.is_some() || that2.rule_set.is_none()) // can't compare tree structure, no .equals or recurse
            && match (&self.number_format, &that2.number_format) {
                (None, other) => other.is_none(),
                (Some(a), Some(b)) => a.equals_decimal_format(b),
                (Some(_), None) => false,
            };
        if !base_equal {
            return false;
        }
        match (self.kind, that2.kind) {
            (Kind::Multiplier { divisor: a }, Kind::Multiplier { divisor: b }) => a == b,
            (Kind::Modulus { divisor: a, .. }, Kind::Modulus { divisor: b, .. }) => a == b,
            (
                Kind::Numerator { denominator: d1, with_zeros: z1 },
                Kind::Numerator { denominator: d2, with_zeros: z2 },
            ) => d1 == d2 && z1 == z2,
            _ => true,
        }
    }

    /// `hashCode()`: not designed (Java asserts and returns 42).
    pub(crate) fn hash_code(&self) -> i32 {
        debug_assert!(false, "hashCode not designed");
        42
    }

    /// Returns a textual description of the substitution: the token
    /// characters around the rule set name or the decimal pattern.
    /// Equivalent to `toString()`.
    ///
    /// # Errors
    ///
    /// The errors of the decimal format's pattern; `NullPointerException`
    /// for a substitution with neither.
    pub(crate) fn to_java_string(&self, g: &RuleGraph) -> Result<Vec<u16>> {
        // use tokenChar() to get the character at the beginning and
        // end of the substitution token.  In between them will go
        // either the name of the rule set it uses, or the pattern of
        // the DecimalFormat it uses
        let token = self.token_char();
        let mut result = vec![token];
        match self.rule_set {
            Some(rs) => result.extend_from_slice(g.rule_set(rs).get_name()),
            None => result.extend_from_slice(&self.number_format.as_ref().ok_or_else(npe)?.to_pattern()?),
        }
        result.push(token);
        Ok(result)
    }

    //-----------------------------------------------------------------------
    // formatting
    //-----------------------------------------------------------------------

    /// Performs a mathematical operation on the number, formats it using
    /// either the rule set or the DecimalFormat, and inserts the result
    /// into `to_insert_into` at `position + pos`. `owning_rule` is the rule
    /// that owns this substitution. Equivalent to `doSubstitution(long
    /// number, StringBuilder toInsertInto, int position, int
    /// recursionCount)` and its override in `ModulusSubstitution`.
    ///
    /// # Errors
    ///
    /// The errors of the rule set, of the rules and of the decimal format.
    pub(crate) fn do_substitution_long(
        &self,
        number: i64,
        to_insert_into: &mut Vec<u16>,
        position: i32,
        recursion_count: i32,
        owning_rule: &NFRule,
        g: &RuleGraph,
    ) -> Result<()> {
        if let Kind::Modulus { rule_to_use: Some((rule_set, rule)), .. } = self.kind {
            // a >>> substitution goes straight to a particular rule to
            // format the substitution value
            let number_to_format = self.transform_long(number)?;
            return g.rule_set(rule_set).rule_at(rule).do_format_long(
                number_to_format,
                to_insert_into,
                position + self.pos,
                recursion_count,
                g,
            );
        }
        // if this isn't a >>> substitution, just use the inherited version
        // of this function (which uses either a rule set or a DecimalFormat
        // to format its substitution value)
        if let Some(rule_set) = self.rule_set {
            // Perform a transformation on the number that is dependent
            // on the type of substitution this is, then just call its
            // rule set's format() method to format the result
            let number_to_format = self.transform_long(number)?;

            g.rule_set(rule_set).format_long(number_to_format, to_insert_into, position + self.pos, recursion_count, g)
        } else {
            let number_format = self.number_format.as_ref().ok_or_else(npe)?;
            if number <= MAX_INT64_IN_DOUBLE {
                // or perform the transformation on the number,
                // then use that formatter's format() method
                // to format the result
                let number_to_format = self.transform_double(number as f64, owning_rule, g);
                sb_insert(to_insert_into, position + self.pos, &number_format.format_f64(number_to_format)?);
            } else {
                // We have gone beyond double precision. Something has to give.
                // We're favoring accuracy of the large number over potential rules
                // that round like a CompactDecimalFormat, which is not a common use case.
                //
                // Perform a transformation on the number that is dependent
                // on the type of substitution this is, then just call its
                // rule set's format() method to format the result
                let number_to_format = self.transform_long(number)?;
                sb_insert(to_insert_into, position + self.pos, &number_format.format_i64(number_to_format)?);
            }
            Ok(())
        }
    }

    /// Performs a mathematical operation on the number, formats it using
    /// either the rule set or the DecimalFormat, and inserts the result
    /// into `to_insert_into` at `position + pos`. Equivalent to
    /// `doSubstitution(double number, StringBuilder toInsertInto, int
    /// position, int recursionCount)` and its overrides in
    /// `ModulusSubstitution`, `FractionalPartSubstitution` and
    /// `NumeratorSubstitution`.
    ///
    /// # Errors
    ///
    /// As [`do_substitution_long`](Self::do_substitution_long).
    pub(crate) fn do_substitution_double(
        &self,
        number: f64,
        to_insert_into: &mut Vec<u16>,
        position: i32,
        recursion_count: i32,
        owning_rule: &NFRule,
        g: &RuleGraph,
    ) -> Result<()> {
        match self.kind {
            Kind::Modulus { rule_to_use: Some((rule_set, rule)), .. } => {
                // a >>> substitution goes straight to a particular rule to
                // format the substitution value
                let number_to_format = self.transform_double(number, owning_rule, g);
                g.rule_set(rule_set).rule_at(rule).do_format_double(
                    number_to_format,
                    to_insert_into,
                    position + self.pos,
                    recursion_count,
                    g,
                )
            }
            Kind::FractionalPart { by_digits: true, use_spaces } => {
                // if we're in "byDigits" mode, transform the value into an integer
                // by moving the decimal point eight places to the right and
                // pulling digits off the right one at a time, formatting each digit
                // as an integer using this substitution's owning rule set
                // (this is slower, but more accurate, than doing it from the
                // other end)
                let rule_set = g.rule_set(self.rule_set.ok_or_else(npe)?);
                let mut fq = DecimalQuantity_DualStorageBCD::from_double(number);
                fq.round_to_infinity(); // ensure doubles are resolved using slow path

                let mut pad = false;
                let mut mag = fq.get_lower_display_magnitude();
                while mag < 0 {
                    if pad && use_spaces {
                        sb_insert(to_insert_into, position + self.pos, &[u16::from(b' ')]);
                    } else {
                        pad = true;
                    }
                    let digit = fq.get_digit(mag);
                    mag += 1;
                    rule_set.format_long(i64::from(digit), to_insert_into, position + self.pos, recursion_count, g)?;
                }
                Ok(())
            }
            Kind::Numerator { denominator, with_zeros } => {
                // perform a transformation on the number being formatted that
                // is dependent on the type of substitution this is
                //String s = toInsertInto.toString();
                let number_to_format = self.transform_double(number, owning_rule, g);
                let mut position = position;

                if with_zeros {
                    if let Some(rule_set) = self.rule_set {
                        // if there are leading zeros in the decimal expansion then emit them
                        let mut nf = number_to_format as i64;
                        let len = to_insert_into.len() as i32;
                        loop {
                            nf = nf.wrapping_mul(10);
                            if (nf as f64) >= denominator {
                                break;
                            }
                            sb_insert(to_insert_into, position + self.pos, &[u16::from(b' ')]);
                            g.rule_set(rule_set).format_long(0, to_insert_into, position + self.pos, recursion_count, g)?;
                        }
                        position += to_insert_into.len() as i32 - len;
                    }
                }

                // if the result is an integer, from here on out we work in integer
                // space (saving time and memory and preserving accuracy)
                self.format_transformed(number_to_format, to_insert_into, position, recursion_count, g)
            }
            _ => {
                // perform a transformation on the number being formatted that
                // is dependent on the type of substitution this is
                let number_to_format = self.transform_double(number, owning_rule, g);

                if number_to_format.is_infinite() {
                    // This is probably a minus rule. Combine it with an infinite rule.
                    let rule_set = g.rule_set(self.rule_set.ok_or_else(npe)?);
                    let infinite_rule = rule_set.find_rule(f64::INFINITY, g)?;
                    return infinite_rule.do_format_double(
                        number_to_format,
                        to_insert_into,
                        position + self.pos,
                        recursion_count,
                        g,
                    );
                }

                // if the result is an integer, from here on out we work in integer
                // space (saving time and memory and preserving accuracy)
                self.format_transformed(number_to_format, to_insert_into, position, recursion_count, g)
            }
        }
    }

    /// The tail shared by `doSubstitution(double, ...)` and its numerator
    /// override: an integral value formats in integer space with the rule
    /// set; otherwise the rule set or the decimal format formats the
    /// double.
    fn format_transformed(
        &self,
        number_to_format: f64,
        to_insert_into: &mut Vec<u16>,
        position: i32,
        recursion_count: i32,
        g: &RuleGraph,
    ) -> Result<()> {
        match self.rule_set {
            Some(rule_set) if number_to_format == number_to_format.floor() => g.rule_set(rule_set).format_long(
                number_to_format as i64,
                to_insert_into,
                position + self.pos,
                recursion_count,
                g,
            ),
            // if the result isn't an integer, then call either our rule set's
            // format() method or our DecimalFormat's format() method to
            // format the result
            Some(rule_set) => {
                g.rule_set(rule_set).format_double(number_to_format, to_insert_into, position + self.pos, recursion_count, g)
            }
            None => {
                let number_format = self.number_format.as_ref().ok_or_else(npe)?;
                sb_insert(to_insert_into, position + self.pos, &number_format.format_f64(number_to_format)?);
                Ok(())
            }
        }
    }

    /// Subclasses override this function to perform some kind of
    /// mathematical operation on the number. Equivalent to
    /// `transformNumber(long number)` of each subclass.
    ///
    /// # Errors
    ///
    /// `ArithmeticException("/ by zero")` for a zero divisor.
    fn transform_long(&self, number: i64) -> Result<i64> {
        Ok(match self.kind {
            Kind::SameValue | Kind::IntegralPart => number,
            Kind::Multiplier { divisor } => (checked_div(number, divisor)? as f64).floor() as i64,
            Kind::Modulus { divisor, .. } => checked_rem(number, divisor)?,
            Kind::FractionalPart { .. } => 0,
            Kind::AbsoluteValue => number.wrapping_abs(),
            Kind::Numerator { denominator, .. } => java_math_round(number as f64 * denominator),
        })
    }

    /// Subclasses override this function to perform some kind of
    /// mathematical operation on the number. Equivalent to
    /// `transformNumber(double number)` of each subclass; a multiplier
    /// substitution floors the quotient when it has a rule set, when its
    /// owning rule has a modulus substitution, or when the formatter rounds
    /// with `ROUND_FLOOR`.
    fn transform_double(&self, number: f64, owning_rule: &NFRule, g: &RuleGraph) -> f64 {
        match self.kind {
            Kind::SameValue => number,
            Kind::Multiplier { divisor } => {
                // Most of the time, when a number is handled by an NFSubstitution, we do a floor() on it, but
                // if a substitution uses a DecimalFormat to format the number instead of a ruleset, we generally
                // don't want to do a floor()-- we want to keep the value intact so that the DecimalFormat can
                // either include the fractional part or round properly.  The big exception to this is here in
                // MultiplierSubstitution.  If the rule includes two substitutions, the MultiplierSubstitution
                // (which is handling the larger part of the number) really _does_ want to do a floor(), because
                // the ModulusSubstitution (which is handling the smaller part of the number) will take
                // care of the fractional part.  (Consider something like `1/12: <0< feet >0.0> inches;`.)
                // But if there is no ModulusSubstitution, we're shortening the number in some way-- the "larger part"
                // of the number is the only part we're keeping.  Even if the DecimalFormat doesn't include the
                // fractional part in its output, we still want it to round.  (Consider something like `1/1000: <0<K;`.)
                // (TODO: The ROUND_FLOOR thing is a kludge to preserve the previous floor-always behavior.  What we
                // probably really want to do is just set the rounding mode on the DecimalFormat to match the rounding
                // mode on the RuleBasedNumberFormat and then pass the number to it whole and let it do its own rounding.
                // But before making that change, we'd have to make sure it didn't have undesirable side effects.)
                if self.rule_set.is_some()
                    || owning_rule.has_modulus_substitution()
                    || g.owner().get_rounding_mode() == IcuBigDecimal::ROUND_FLOOR
                {
                    (number / divisor as f64).floor()
                } else {
                    number / divisor as f64
                }
            }
            Kind::Modulus { divisor, .. } => number % divisor as f64,
            Kind::IntegralPart => number.floor(),
            Kind::FractionalPart { .. } => number - number.floor(),
            Kind::AbsoluteValue => number.abs(),
            Kind::Numerator { denominator, .. } => java_math_round(number * denominator) as f64,
        }
    }

    //-----------------------------------------------------------------------
    // parsing
    //-----------------------------------------------------------------------

    /// Parses a string using the rule set or DecimalFormat belonging to
    /// this substitution, and composes the result with `base_value`; when
    /// nothing matched, `parse_position` is unchanged and the result is the
    /// partial one (`None` for a failed decimal-format parse). Equivalent
    /// to `doParse(String text, ParsePosition parsePosition, double
    /// baseValue, double upperBound, boolean lenientParse, int
    /// nonNumericalExecutedRuleMask, int recursionCount)` and its overrides
    /// in `ModulusSubstitution`, `FractionalPartSubstitution` and
    /// `NumeratorSubstitution`.
    ///
    /// # Errors
    ///
    /// The errors of the rule sets, the rules and the decimal formats;
    /// `NullPointerException` where Java dereferences `null`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn do_parse(
        &self,
        text: &[u16],
        parse_position: &mut ParsePosition,
        base_value: f64,
        upper_bound: f64,
        lenient_parse: bool,
        non_numerical_executed_rule_mask: i32,
        recursion_count: i32,
        g: &RuleGraph,
    ) -> Result<Option<JavaNumber>> {
        match self.kind {
            Kind::Modulus { rule_to_use: Some((rule_set, rule)), .. } => {
                // but if it IS a >>> substitution, we have to do it here: we
                // use the specific rule's doParse() method, and then we have to
                // do some of the other work of NFRuleSet.parse()
                let temp_result = g.rule_set(rule_set).rule_at(rule).do_parse(
                    text,
                    parse_position,
                    false,
                    upper_bound,
                    non_numerical_executed_rule_mask,
                    recursion_count,
                    g,
                )?;

                if parse_position.get_index() != 0 {
                    let result = self.compose_rule_value(temp_result.double_value(), base_value);
                    Ok(Some(long_or_double(result)))
                } else {
                    Ok(Some(temp_result))
                }
            }
            Kind::FractionalPart { by_digits: false, .. } => {
                // if we're not in byDigits mode, we can just use the inherited
                // doParse()
                self.base_do_parse(
                    text,
                    parse_position,
                    base_value,
                    0.0,
                    lenient_parse,
                    non_numerical_executed_rule_mask,
                    recursion_count,
                    g,
                )
            }
            Kind::FractionalPart { by_digits: true, .. } => {
                // if we ARE in byDigits mode, parse the text one digit at a time
                // using this substitution's owning rule set (we do this by setting
                // upperBound to 10 when calling doParse() ) until we reach
                // nonmatching text
                let mut work_text: &[u16] = text;
                let mut work_pos = ParsePosition::new(1);
                let mut fq = DecimalQuantity_DualStorageBCD::new();
                let mut total_digits = 0;
                while !work_text.is_empty() && work_pos.get_index() != 0 {
                    work_pos.set_index(0);
                    let rule_set = g.rule_set(self.rule_set.ok_or_else(npe)?);
                    let mut digit = number_int_value(&rule_set.parse(
                        work_text,
                        &mut work_pos,
                        10.0,
                        non_numerical_executed_rule_mask,
                        recursion_count,
                        g,
                    )?);
                    if lenient_parse && work_pos.get_index() == 0 {
                        let decimal_format = g.owner().get_decimal_format()?;
                        if let Some(n) = NumberFormat::parse(&*decimal_format, work_text, &mut work_pos)? {
                            digit = number_int_value(&n);
                        }
                    }

                    if work_pos.get_index() != 0 {
                        fq.append_digit(digit as i8, 0, true);
                        total_digits += 1;

                        parse_position.set_index(parse_position.get_index() + work_pos.get_index());
                        work_text = substring_from(work_text, work_pos.get_index());
                        while !work_text.is_empty() && work_text[0] == u16::from(b' ') {
                            work_text = substring_from(work_text, 1);
                            parse_position.set_index(parse_position.get_index() + 1);
                        }
                    }
                }
                fq.adjust_magnitude(-total_digits)?;
                let result = fq.to_double();

                let result = self.compose_rule_value(result, base_value);
                Ok(Some(JavaNumber::Double(result)))
            }
            Kind::Numerator { with_zeros, .. } => {
                // we don't have to do anything special to do the parsing here,
                // but we have to turn lenient parsing off-- if we leave it on,
                // it SERIOUSLY messes up the algorithm

                // if withZeros is true, we need to count the zeros
                // and use that to adjust the parse result
                let mut zero_count = 0;
                let mut owned_text: Vec<u16> = text.to_vec();
                if with_zeros {
                    let mut work_text: &[u16] = text;
                    let mut work_pos = ParsePosition::new(1);
                    //int digit;

                    while !work_text.is_empty() && work_pos.get_index() != 0 {
                        work_pos.set_index(0);
                        let rule_set = g.rule_set(self.rule_set.ok_or_else(npe)?);
                        /*digit = */
                        number_int_value(&rule_set.parse(
                            work_text,
                            &mut work_pos,
                            1.0,
                            non_numerical_executed_rule_mask,
                            recursion_count,
                            g,
                        )?); // parse zero or nothing at all
                        if work_pos.get_index() == 0 {
                            // we failed, either there were no more zeros, or the number was formatted with digits
                            // either way, we're done
                            break;
                        }

                        zero_count += 1;
                        parse_position.set_index(parse_position.get_index() + work_pos.get_index());
                        work_text = substring_from(work_text, work_pos.get_index());
                        while !work_text.is_empty() && work_text[0] == u16::from(b' ') {
                            work_text = substring_from(work_text, 1);
                            parse_position.set_index(parse_position.get_index() + 1);
                        }
                    }

                    owned_text = substring_from(text, parse_position.get_index()).to_vec(); // arrgh!
                    parse_position.set_index(0);
                }

                // we've parsed off the zeros, now let's parse the rest from our current position
                let result = self.base_do_parse(
                    &owned_text,
                    parse_position,
                    if with_zeros { 1.0 } else { base_value },
                    upper_bound,
                    false,
                    non_numerical_executed_rule_mask,
                    recursion_count,
                    g,
                )?;

                if with_zeros {
                    // any base value will do in this case.  is there a way to
                    // force this to not bother trying all the base values?

                    // compute the 'effective' base and prescale the value down
                    let n = number_long_value(result.as_ref().ok_or_else(npe)?);
                    let mut d: i64 = 1;
                    while d <= n {
                        d = d.wrapping_mul(10);
                    }
                    // now add the zeros
                    while zero_count > 0 {
                        d = d.wrapping_mul(10);
                        zero_count -= 1;
                    }
                    // d is now our true denominator
                    return Ok(Some(JavaNumber::Double(n as f64 / d as f64)));
                }

                Ok(result)
            }
            _ => self.base_do_parse(
                text,
                parse_position,
                base_value,
                upper_bound,
                lenient_parse,
                non_numerical_executed_rule_mask,
                recursion_count,
                g,
            ),
        }
    }

    /// The body of `NFSubstitution.doParse` (the inherited version the
    /// overrides call through `super`).
    #[allow(clippy::too_many_arguments)]
    fn base_do_parse(
        &self,
        text: &[u16],
        parse_position: &mut ParsePosition,
        base_value: f64,
        upper_bound: f64,
        lenient_parse: bool,
        non_numerical_executed_rule_mask: i32,
        recursion_count: i32,
        g: &RuleGraph,
    ) -> Result<Option<JavaNumber>> {
        // figure out the highest base value a rule can have and match
        // the text being parsed (this varies according to the type of
        // substitutions: multiplier, modulus, and numerator substitutions
        // restrict the search to rules with base values lower than their
        // own; same-value substitutions leave the upper bound wherever
        // it was, and the others allow any rule to match
        let upper_bound = self.calc_upper_bound(upper_bound);

        // use our rule set to parse the text.  If that fails and
        // lenient parsing is enabled (this is always false if the
        // formatter's lenient-parsing mode is off, but it may also
        // be false even when the formatter's lenient-parse mode is
        // on), then also try parsing the text using a default-
        // constructed NumberFormat
        let temp_result = if let Some(rule_set) = self.rule_set {
            let rule_set = g.rule_set(rule_set);
            let mut temp_result = Some(rule_set.parse(
                text,
                parse_position,
                upper_bound,
                non_numerical_executed_rule_mask,
                recursion_count,
                g,
            )?);
            if lenient_parse && !rule_set.is_fraction_set() && parse_position.get_index() == 0 {
                let decimal_format = g.owner().get_decimal_format()?;
                temp_result = NumberFormat::parse(&*decimal_format, text, parse_position)?;
            }
            temp_result
            // ...or use our DecimalFormat to parse the text
        } else {
            let number_format = self.number_format.as_ref().ok_or_else(npe)?;
            NumberFormat::parse(number_format, text, parse_position)?
        };

        // if the parse was successful, we've already advanced the caller's
        // parse position (this is the one function that doesn't have one
        // of its own).  Derive a parse result and return it as a Long,
        // if possible, or a Double
        if parse_position.get_index() != 0 {
            let result = temp_result.ok_or_else(npe)?.double_value();

            // composeRuleValue() produces a full parse result from
            // the partial parse result passed to this function from
            // the caller (this is either the owning rule's base value
            // or the partial result obtained from composing the
            // owning rule's base value with its other substitution's
            // parse result) and the partial parse result obtained by
            // matching the substitution (which will be the same value
            // the caller would get by parsing just this part of the
            // text with RuleBasedNumberFormat.parse() ).  How the two
            // values are used to derive the full parse result depends
            // on the types of substitutions: For a regular rule, the
            // ultimate result is its multiplier substitution's result
            // times the rule's divisor (or the rule's base value) plus
            // the modulus substitution's result (which will actually
            // supersede part of the rule's base value).  For a negative-
            // number rule, the result is the negative of its substitution's
            // result.  For a fraction rule, it's the sum of its two
            // substitution results.  For a rule in a fraction rule set,
            // it's the numerator substitution's result divided by
            // the rule's base value.  Results from same-value substitutions
            // propagate back upward, and null substitutions don't affect
            // the result.
            let result = self.compose_rule_value(result, base_value);
            Ok(Some(long_or_double(result)))
            // if the parse was UNsuccessful, return 0
        } else {
            Ok(temp_result)
        }
    }

    /// Derives a new value from the two values passed in. Equivalent to
    /// `composeRuleValue(double newRuleValue, double oldRuleValue)` of each
    /// subclass.
    fn compose_rule_value(&self, new_rule_value: f64, old_rule_value: f64) -> f64 {
        match self.kind {
            Kind::SameValue => new_rule_value,
            Kind::Multiplier { divisor } => new_rule_value * divisor as f64,
            Kind::Modulus { divisor, .. } => (old_rule_value - (old_rule_value % divisor as f64)) + new_rule_value,
            Kind::IntegralPart | Kind::FractionalPart { .. } => new_rule_value + old_rule_value,
            Kind::AbsoluteValue => -new_rule_value,
            Kind::Numerator { .. } => new_rule_value / old_rule_value,
        }
    }

    /// Calculates an upper bound when searching for a rule that matches
    /// this substitution. Equivalent to `calcUpperBound(double
    /// oldUpperBound)` of each subclass.
    fn calc_upper_bound(&self, old_upper_bound: f64) -> f64 {
        match self.kind {
            Kind::SameValue => old_upper_bound,
            Kind::Multiplier { divisor } | Kind::Modulus { divisor, .. } => divisor as f64,
            Kind::IntegralPart | Kind::AbsoluteValue => f64::MAX,
            Kind::FractionalPart { .. } => 0.0, // this value is ignored
            Kind::Numerator { denominator, .. } => denominator,
        }
    }

    //-----------------------------------------------------------------------
    // simple accessors
    //-----------------------------------------------------------------------

    /// Returns the substitution's position in the rule that owns it.
    /// Equivalent to `getPos()`.
    pub(crate) fn get_pos(&self) -> i32 {
        self.pos
    }

    /// Returns the character used in the textual representation of
    /// substitutions of this type. Equivalent to `tokenChar()` of each
    /// subclass.
    fn token_char(&self) -> u16 {
        match self.kind {
            Kind::SameValue => u16::from(b'='),
            Kind::Multiplier { .. } | Kind::IntegralPart | Kind::Numerator { .. } => u16::from(b'<'),
            Kind::Modulus { .. } | Kind::FractionalPart { .. } | Kind::AbsoluteValue => u16::from(b'>'),
        }
    }

    /// Returns true if this is a modulus substitution. Equivalent to
    /// `isModulusSubstitution()`.
    pub(crate) fn is_modulus_substitution(&self) -> bool {
        matches!(self.kind, Kind::Modulus { .. })
    }

    /// Applies new decimal format symbols to the substitution's decimal
    /// format. Equivalent to `setDecimalFormatSymbols(DecimalFormatSymbols
    /// newSymbols)`.
    ///
    /// # Errors
    ///
    /// The decimal format's.
    pub(crate) fn set_decimal_format_symbols(&self, new_symbols: &DecimalFormatSymbols) -> Result<()> {
        if let Some(number_format) = &self.number_format {
            number_format.set_decimal_format_symbols(new_symbols)?;
        }
        Ok(())
    }
}
