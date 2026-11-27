// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.text.NFRuleSet`.

use std::collections::HashMap;
use std::ops::Deref;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use rucene::error::{LuceneError, Result};

use crate::jdk::{index_of_unit, units, ParsePosition};
use crate::r#impl::number::decimal_quantity_dual_storage_bcd::JavaNumber;
use crate::r#impl::pattern_props::PatternProps;
use crate::text::decimal_format_symbols::DecimalFormatSymbols;
use crate::text::nf_rule::{self, NFRule};
use crate::text::number_format::lock;
use crate::text::rule_based_number_format::{
    array_index, char_at, ends_with, find_rule_set_in, java_math_round, npe, starts_with,
    substring, substring_from, RbnfFields, RuleGraph,
};

/// `NEGATIVE_RULE_INDEX`: the index of the negative-number rule in the
/// non-numerical rules.
pub(crate) const NEGATIVE_RULE_INDEX: usize = 0;
/// `IMPROPER_FRACTION_RULE_INDEX`.
pub(crate) const IMPROPER_FRACTION_RULE_INDEX: usize = 1;
/// `PROPER_FRACTION_RULE_INDEX`.
pub(crate) const PROPER_FRACTION_RULE_INDEX: usize = 2;
/// `DEFAULT_RULE_INDEX`.
pub(crate) const DEFAULT_RULE_INDEX: usize = 3;
/// `INFINITY_RULE_INDEX`.
pub(crate) const INFINITY_RULE_INDEX: usize = 4;
/// `NAN_RULE_INDEX`.
pub(crate) const NAN_RULE_INDEX: usize = 5;

/// `RECURSION_LIMIT` (private): limit of recursion. It's about a 64 bit
/// number formatted in base 2.
const RECURSION_LIMIT: i32 = 64;

/// What `NFRuleSet.parseRules` and the rule and substitution constructors
/// it calls reach through `owner` while a formatter's `init` runs: the rule
/// sets created so far (names and fraction flags), `ruleSetsMap`, the
/// provisional `defaultRuleSet` and the formatter itself.
pub(crate) struct BuildContext<'a> {
    /// `owner.ruleSets`.
    pub(crate) rule_sets: &'a [NFRuleSet],
    /// `owner.ruleSetsMap`.
    pub(crate) rule_sets_map: &'a HashMap<Vec<u16>, usize>,
    /// `owner.getDefaultRuleSet()`.
    pub(crate) default_rule_set: Option<usize>,
    /// The formatter (`owner`, `formatter`).
    pub(crate) owner: &'a Arc<RbnfFields>,
}

impl BuildContext<'_> {
    /// `owner.findRuleSet(String name)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("No rule set named <name>")`.
    pub(crate) fn find_rule_set(&self, name: &[u16]) -> Result<usize> {
        find_rule_set_in(self.rule_sets_map, name)
    }
}

/// The rules `parseRules` builds for one rule set: the normal rules, and
/// the special rules held by `nonNumericalRules` and `fractionRules` (the
/// same objects in Java, one arena here).
#[derive(Debug, Default)]
pub(crate) struct RuleSetBody {
    /// `rules`.
    rules: Vec<NFRule>,
    /// The special rules, which `non_numerical_rules` and `fraction_rules`
    /// index.
    special_rules: Vec<NFRule>,
    /// `nonNumericalRules`.
    non_numerical_rules: [Option<usize>; 6],
    /// `fractionRules` (`null` until the first fraction rule).
    fraction_rules: Option<Vec<usize>>,
}

/// A rule found for a number: a rule of the rule set, or one of the
/// formatter's default infinity and NaN rules.
pub(crate) enum RuleRef<'a> {
    /// A rule of the rule set.
    Borrowed(&'a NFRule),
    /// A default rule of the formatter.
    Shared(Arc<NFRule>),
}

impl Deref for RuleRef<'_> {
    type Target = NFRule;

    fn deref(&self) -> &NFRule {
        match self {
            RuleRef::Borrowed(r) => r,
            RuleRef::Shared(r) => r,
        }
    }
}

/// A collection of rules used by a RuleBasedNumberFormat to format and parse
/// numbers. It is the responsibility of a RuleSet to select an appropriate
/// rule for formatting a particular number and dispatch control to it, and
/// to arbitrate between different rules when parsing a number.
///
/// Equivalent to the package-private class `com.ibm.icu.text.NFRuleSet`.
/// The `owner` is the [`RuleGraph`] the methods receive (see its
/// divergence note); the special rules that `nonNumericalRules` and
/// `fractionRules` share in Java live in one arena that both index.
#[derive(Debug)]
pub(crate) struct NFRuleSet {
    /// `name`: the rule set's name.
    name: Vec<u16>,
    /// `rules`: the rule set's regular rules.
    rules: Vec<NFRule>,
    /// The special rules (see the type documentation).
    special_rules: Vec<NFRule>,
    /// `nonNumericalRules`: the rule set's non-numerical rules like
    /// negative, fractions, infinity and NaN (reassigned by
    /// `setDecimalFormatSymbols`).
    non_numerical_rules: Mutex<[Option<usize>; 6]>,
    /// `fractionRules`: these are a pile of fraction rules in declared
    /// order. They may have alternate ways to represent fractions.
    fraction_rules: Option<Vec<usize>>,
    /// `isFractionRuleSet`: true if the rule set is a fraction rule set (set
    /// by the substitutions of other rule sets while the formatter is
    /// built).
    is_fraction_rule_set: AtomicBool,
    /// `isParseable`: true if the rule set is parseable.
    is_parseable: bool,
}

impl NFRuleSet {
    //-----------------------------------------------------------------------
    // construction
    //-----------------------------------------------------------------------

    /// Constructs a rule set: initializes only the name, removing it from
    /// `descriptions[index]`; the rules are parsed by
    /// [`parse_rules`](Self::parse_rules). Equivalent to `NFRuleSet(
    /// RuleBasedNumberFormat owner, String[] descriptions, int index)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("Empty rule set description")`,
    /// `IllegalArgumentException("Rule set name doesn't end in colon")`.
    ///
    /// # Panics
    ///
    /// `StringIndexOutOfBoundsException` when the name's colon ends the
    /// description (Java's `charAt(++pos)`).
    pub(crate) fn new(descriptions: &mut [Vec<u16>], index: usize) -> Result<NFRuleSet> {
        let mut description = descriptions[index].clone();

        if description.is_empty() {
            return Err(LuceneError::illegal_argument("Empty rule set description"));
        }

        // if the description begins with a rule set name (the rule set
        // name can be omitted in formatter descriptions that consist
        // of only one rule set), copy it out into our "name" member
        // and delete it from the description
        let name;
        let is_parseable;
        if description[0] == u16::from(b'%') {
            let mut pos = index_of_unit(&description, u16::from(b':'), 0);
            if pos == -1 {
                return Err(LuceneError::illegal_argument(
                    "Rule set name doesn't end in colon",
                ));
            } else {
                let mut rule_name = substring(&description, 0, pos).to_vec();
                is_parseable = !ends_with(&rule_name, &units("@noparse"));
                if !is_parseable {
                    rule_name = substring(&rule_name, 0, rule_name.len() as i32 - 8).to_vec();
                    // Remove the @noparse from the name
                }
                name = rule_name;

                while pos < description.len() as i32 && {
                    pos += 1;
                    PatternProps::is_white_space(i32::from(char_at(&description, pos)))
                } {}
                description = substring_from(&description, pos).to_vec();
                descriptions[index] = description.clone();
            }
        } else {
            // if the description doesn't begin with a rule set name, its
            // name is "%default"
            name = units("%default");
            is_parseable = true;
        }

        if description.is_empty() {
            return Err(LuceneError::illegal_argument("Empty rule set description"));
        }

        // all of the other members of NFRuleSet are initialized
        // by parseRules()
        Ok(NFRuleSet {
            name,
            rules: Vec::new(),
            special_rules: Vec::new(),
            non_numerical_rules: Mutex::new([None; 6]),
            fraction_rules: None,
            is_fraction_rule_set: AtomicBool::new(false),
            is_parseable,
        })
    }

    /// Construct the subordinate data structures used by the rule set at
    /// `index` of the context. Equivalent to `parseRules(String
    /// description)`; the result is installed by [`install`](Self::install).
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("Rules are not in order, base: <b> <
    /// <default>")`; the errors of the rules.
    pub(crate) fn parse_rules(
        ctx: &BuildContext<'_>,
        index: usize,
        description: &[u16],
    ) -> Result<RuleSetBody> {
        // (the number of elements in the description list isn't necessarily
        // the number of rules-- some descriptions may expend into two rules)
        let mut temp_rules: Vec<NFRule> = Vec::new();
        let mut body = RuleSetBody::default();

        // we keep track of the rule before the one we're currently working
        // on solely to support >>> substitutions
        let mut predecessor: Option<usize> = None;

        // Iterate through the rules.  The rules
        // are separated by semicolons (there's no escape facility: ALL
        // semicolons are rule delimiters)
        let mut old_p = 0i32;
        let description_len = description.len() as i32;
        loop {
            let mut p = index_of_unit(description, u16::from(b';'), old_p);
            if p < 0 {
                p = description_len;
            }

            // makeRules (a factory method on NFRule) will return either
            // a single rule or an array of rules.  Either way, add them
            // to our rule vector
            NFRule::make_rules(
                substring(description, old_p, p),
                index,
                predecessor,
                ctx,
                &mut body,
                &mut temp_rules,
            )?;
            if !temp_rules.is_empty() {
                predecessor = Some(temp_rules.len() - 1);
            }

            old_p = p + 1;
            if old_p >= description_len {
                break;
            }
        }

        // for rules that didn't specify a base value, their base values
        // were initialized to 0.  Make another pass through the list and
        // set all those rules' base values.  We also remove any special
        // rules from the list and put them into their own member variables
        let is_fraction_rule_set = ctx.rule_sets[index].is_fraction_set();
        let mut default_base_value: i64 = 0;
        for rule in temp_rules.iter_mut() {
            let base_value = rule.get_base_value();
            if base_value == 0 {
                // if the rule's base value is 0, fill in a default
                // base value (this will be 1 plus the preceding
                // rule's base value for regular rule sets, and the
                // same as the preceding rule's base value in fraction
                // rule sets)
                rule.set_base_value(default_base_value)?;
            } else {
                // if it's a regular rule that already knows its base value,
                // check to make sure the rules are in order, and update
                // the default base value for the next rule
                if base_value < default_base_value {
                    return Err(LuceneError::illegal_argument(format!(
                        "Rules are not in order, base: {base_value} < {default_base_value}"
                    )));
                }
                default_base_value = base_value;
            }
            if !is_fraction_rule_set {
                default_base_value = default_base_value.wrapping_add(1);
            }
        }

        // finally, we can copy the rules from the vector into a
        // fixed-length array
        body.rules = temp_rules;
        Ok(body)
    }

    /// Stores the rules `parse_rules` built.
    pub(crate) fn install(&mut self, body: RuleSetBody) {
        self.rules = body.rules;
        self.special_rules = body.special_rules;
        *lock(&self.non_numerical_rules) = body.non_numerical_rules;
        self.fraction_rules = body.fraction_rules;
    }

    /// Set one of the non-numerical rules of the body being built.
    /// Equivalent to `setNonNumericalRule(NFRule rule)`.
    ///
    /// # Errors
    ///
    /// The errors of the formatter's decimal format symbols.
    pub(crate) fn set_non_numerical_rule(
        body: &mut RuleSetBody,
        rule: NFRule,
        owner: &RbnfFields,
    ) -> Result<()> {
        let base_value = rule.get_base_value();
        body.special_rules.push(rule);
        let rule = body.special_rules.len() - 1;
        if base_value == nf_rule::NEGATIVE_NUMBER_RULE {
            body.non_numerical_rules[NEGATIVE_RULE_INDEX] = Some(rule);
        } else if base_value == nf_rule::IMPROPER_FRACTION_RULE {
            Self::remember_fraction_rule(body, IMPROPER_FRACTION_RULE_INDEX, rule, owner)?;
        } else if base_value == nf_rule::PROPER_FRACTION_RULE {
            Self::remember_fraction_rule(body, PROPER_FRACTION_RULE_INDEX, rule, owner)?;
        } else if base_value == nf_rule::DEFAULT_RULE {
            Self::remember_fraction_rule(body, DEFAULT_RULE_INDEX, rule, owner)?;
        } else if base_value == nf_rule::INFINITY_RULE {
            body.non_numerical_rules[INFINITY_RULE_INDEX] = Some(rule);
        } else if base_value == nf_rule::NAN_RULE {
            body.non_numerical_rules[NAN_RULE_INDEX] = Some(rule);
        }
        Ok(())
    }

    /// `setBestFractionRule(originalIndex, newRule, true)`: remembers the
    /// rule in `fractionRules`, then selects the best one.
    fn remember_fraction_rule(
        body: &mut RuleSetBody,
        original_index: usize,
        new_rule: usize,
        owner: &RbnfFields,
    ) -> Result<()> {
        body.fraction_rules
            .get_or_insert_with(Vec::new)
            .push(new_rule);
        Self::set_best_fraction_rule(
            &mut body.non_numerical_rules,
            &body.special_rules,
            original_index,
            new_rule,
            owner,
        )
    }

    /// Determine the best fraction rule to use. Rules matching the decimal
    /// point from DecimalFormatSymbols become the main set of rules to use.
    /// Equivalent to `setBestFractionRule(int originalIndex, NFRule newRule,
    /// boolean rememberRule)` after the remembering.
    fn set_best_fraction_rule(
        non_numerical_rules: &mut [Option<usize>; 6],
        special_rules: &[NFRule],
        original_index: usize,
        new_rule: usize,
        owner: &RbnfFields,
    ) -> Result<()> {
        match non_numerical_rules[original_index] {
            None => {
                non_numerical_rules[original_index] = Some(new_rule);
            }
            Some(_) => {
                // We have more than one. Which one is better?
                let decimal_format_symbols = owner.get_decimal_format_symbols()?;
                if decimal_format_symbols.get_decimal_separator()
                    == special_rules[new_rule].get_decimal_point()
                {
                    non_numerical_rules[original_index] = Some(new_rule);
                }
                // else leave it alone
            }
        }
        Ok(())
    }

    /// Flags this rule set as a fraction rule set. This function is used
    /// by the substitutions of other rule sets while the formatter is
    /// built. Equivalent to `makeIntoFractionRuleSet()`.
    pub(crate) fn make_into_fraction_rule_set(&self) {
        self.is_fraction_rule_set.store(true, Ordering::Relaxed);
    }

    //-----------------------------------------------------------------------
    // boilerplate
    //-----------------------------------------------------------------------

    /// The non-numerical rule at `index`, if any.
    fn non_numerical_rule(&self, nn: &[Option<usize>; 6], index: usize) -> Option<&NFRule> {
        nn[index].map(|i| &self.special_rules[i])
    }

    /// Compares two rule sets for equality: name, rule count, fraction flag,
    /// non-numerical rules and rules. Equivalent to `equals(Object that)`.
    pub(crate) fn java_equals(&self, that2: &NFRuleSet) -> bool {
        // otherwise, compare the members one by one...
        if self.name != that2.name
            || self.rules.len() != that2.rules.len()
            || self.is_fraction_set() != that2.is_fraction_set()
        {
            return false;
        }

        // ...then compare the non-numerical rule lists...
        let my_nn = *lock(&self.non_numerical_rules);
        let that_nn = *lock(&that2.non_numerical_rules);
        for i in 0..my_nn.len() {
            let equal = match (
                self.non_numerical_rule(&my_nn, i),
                that2.non_numerical_rule(&that_nn, i),
            ) {
                (None, None) => true,
                (Some(a), Some(b)) => a.java_equals(b),
                _ => false,
            };
            if !equal {
                return false;
            }
        }

        // ...then compare the rule lists...
        for (a, b) in self.rules.iter().zip(&that2.rules) {
            if !a.java_equals(b) {
                return false;
            }
        }

        // ...and if we make it here, they're equal
        true
    }

    /// `hashCode()`: not designed (Java asserts and returns 42).
    #[allow(dead_code)] // Java member with no caller in the closure
    pub(crate) fn hash_code(&self) -> i32 {
        debug_assert!(false, "hashCode not designed");
        42
    }

    /// Builds a textual representation of a rule set. Equivalent to
    /// `toString()`.
    ///
    /// # Errors
    ///
    /// The errors of the patterns of the substitutions' decimal formats.
    pub(crate) fn to_java_string(&self, g: &RuleGraph) -> Result<Vec<u16>> {
        let mut result = Vec::new();

        // the rule set name goes first...
        result.extend_from_slice(&self.name);
        result.extend_from_slice(&units(":\n"));

        // followed by the regular rules...
        for rule in &self.rules {
            result.extend_from_slice(&rule.to_java_string(g)?);
            result.push(u16::from(b'\n'));
        }

        // followed by the special rules (if they exist)
        let nn = *lock(&self.non_numerical_rules);
        for rule in nn.iter().flatten().map(|&i| &self.special_rules[i]) {
            if rule.get_base_value() == nf_rule::IMPROPER_FRACTION_RULE
                || rule.get_base_value() == nf_rule::PROPER_FRACTION_RULE
                || rule.get_base_value() == nf_rule::DEFAULT_RULE
            {
                let fraction_rules = self.fraction_rules.as_ref().ok_or_else(npe)?;
                for fraction_rule in fraction_rules.iter().map(|&i| &self.special_rules[i]) {
                    if fraction_rule.get_base_value() == rule.get_base_value() {
                        result.extend_from_slice(&fraction_rule.to_java_string(g)?);
                        result.push(u16::from(b'\n'));
                    }
                }
            } else {
                result.extend_from_slice(&rule.to_java_string(g)?);
                result.push(u16::from(b'\n'));
            }
        }

        Ok(result)
    }

    //-----------------------------------------------------------------------
    // simple accessors
    //-----------------------------------------------------------------------

    /// Says whether this rule set is a fraction rule set. Equivalent to
    /// `isFractionSet()`.
    pub(crate) fn is_fraction_set(&self) -> bool {
        self.is_fraction_rule_set.load(Ordering::Relaxed)
    }

    /// Returns the rule set's name. Equivalent to `getName()`.
    pub(crate) fn get_name(&self) -> &[u16] {
        &self.name
    }

    /// Return true if the rule set is public (its name does not begin with
    /// `%%`). Equivalent to `isPublic()`.
    pub(crate) fn is_public(&self) -> bool {
        !starts_with(&self.name, &units("%%"))
    }

    /// The regular rule at `index` (the target of a `>>>` substitution).
    pub(crate) fn rule_at(&self, index: usize) -> &NFRule {
        &self.rules[index]
    }

    /// Return true if the rule set can be used for parsing. Equivalent to
    /// `isParseable()`.
    pub(crate) fn is_parseable(&self) -> bool {
        self.is_parseable
    }

    //-----------------------------------------------------------------------
    // formatting
    //-----------------------------------------------------------------------

    /// Formats a long. Selects an appropriate rule and dispatches control to
    /// it. Equivalent to `format(long number, StringBuilder toInsertInto,
    /// int pos, int recursionCount)`.
    ///
    /// # Errors
    ///
    /// `IllegalStateException("Recursion limit exceeded when applying
    /// ruleSet <name>")`; the errors of the rules.
    pub(crate) fn format_long(
        &self,
        number: i64,
        to_insert_into: &mut Vec<u16>,
        pos: i32,
        recursion_count: i32,
        g: &RuleGraph,
    ) -> Result<()> {
        if recursion_count >= RECURSION_LIMIT {
            return Err(self.recursion_limit_exceeded());
        }
        let applicable_rule = self.find_normal_rule(number, g)?;
        applicable_rule.do_format_long(number, to_insert_into, pos, recursion_count + 1, g)
    }

    /// Formats a double. Selects an appropriate rule and dispatches control
    /// to it. Equivalent to `format(double number, StringBuilder
    /// toInsertInto, int pos, int recursionCount)`.
    ///
    /// # Errors
    ///
    /// As [`format_long`](Self::format_long).
    pub(crate) fn format_double(
        &self,
        number: f64,
        to_insert_into: &mut Vec<u16>,
        pos: i32,
        recursion_count: i32,
        g: &RuleGraph,
    ) -> Result<()> {
        if recursion_count >= RECURSION_LIMIT {
            return Err(self.recursion_limit_exceeded());
        }
        let applicable_rule = self.find_rule(number, g)?;
        applicable_rule.do_format_double(number, to_insert_into, pos, recursion_count + 1, g)
    }

    fn recursion_limit_exceeded(&self) -> LuceneError {
        LuceneError::illegal_state(format!(
            "Recursion limit exceeded when applying ruleSet {}",
            String::from_utf16_lossy(&self.name)
        ))
    }

    /// Selects an appropriate rule for formatting the number. Equivalent to
    /// `findRule(double number)`.
    ///
    /// # Errors
    ///
    /// The errors of the default rules and of [`find_normal_rule`].
    pub(crate) fn find_rule(&self, number: f64, g: &RuleGraph) -> Result<RuleRef<'_>> {
        // if this is a fraction rule set, use findFractionRuleSetRule()
        if self.is_fraction_set() {
            return Ok(RuleRef::Borrowed(self.find_fraction_rule_set_rule(number)?));
        }

        let nn = *lock(&self.non_numerical_rules);
        if number.is_nan() {
            return Ok(match self.non_numerical_rule(&nn, NAN_RULE_INDEX) {
                Some(rule) => RuleRef::Borrowed(rule),
                None => RuleRef::Shared(g.owner().get_default_nan_rule()?),
            });
        }

        // if the number is negative, return the negative number rule
        // (if there isn't a negative-number rule, we pretend it's a
        // positive number)
        let mut number = number;
        if number < 0.0 {
            if let Some(rule) = self.non_numerical_rule(&nn, NEGATIVE_RULE_INDEX) {
                return Ok(RuleRef::Borrowed(rule));
            } else {
                number = -number;
            }
        }

        if number.is_infinite() {
            return Ok(match self.non_numerical_rule(&nn, INFINITY_RULE_INDEX) {
                Some(rule) => RuleRef::Borrowed(rule),
                None => RuleRef::Shared(g.owner().get_default_infinity_rule()?),
            });
        }

        // if the number isn't an integer, we use one f the fraction rules...
        if number != number.floor() {
            if number < 1.0 && nn[PROPER_FRACTION_RULE_INDEX].is_some() {
                // if the number is between 0 and 1, return the proper
                // fraction rule
                return Ok(RuleRef::Borrowed(
                    self.non_numerical_rule(&nn, PROPER_FRACTION_RULE_INDEX)
                        .expect("checked"),
                ));
            } else if let Some(rule) = self.non_numerical_rule(&nn, IMPROPER_FRACTION_RULE_INDEX) {
                // otherwise, return the improper fraction rule
                return Ok(RuleRef::Borrowed(rule));
            }
        }

        // if there's a default rule, use it to format the number
        if let Some(rule) = self.non_numerical_rule(&nn, DEFAULT_RULE_INDEX) {
            Ok(RuleRef::Borrowed(rule))
        } else {
            // and if we haven't yet returned a rule, use findNormalRule()
            // to find the applicable rule
            self.find_normal_rule(java_math_round(number), g)
        }
    }

    /// If the value passed to findRule() is a positive integer,
    /// findRule() uses this function to select the appropriate rule (a
    /// binary search, with the rollback rule). Equivalent to
    /// `findNormalRule(long number)`.
    ///
    /// # Errors
    ///
    /// `IllegalStateException("The rule set <name> cannot format the value
    /// <n>")`, `IllegalStateException("The rule set <name> cannot roll back
    /// from the rule '<rule>'")`; `NullPointerException` when the rule set
    /// has neither rules nor a default rule.
    fn find_normal_rule(&self, number: i64, g: &RuleGraph) -> Result<RuleRef<'_>> {
        // if this is a fraction rule set, use findFractionRuleSetRule()
        // to find the rule (we should only go into this clause if the
        // value is 0)
        if self.is_fraction_set() {
            return Ok(RuleRef::Borrowed(
                self.find_fraction_rule_set_rule(number as f64)?,
            ));
        }

        let nn = *lock(&self.non_numerical_rules);
        // if the number is negative, return the negative-number rule
        // (if there isn't one, pretend the number is positive)
        let mut number = number;
        if number < 0 {
            if let Some(rule) = self.non_numerical_rule(&nn, NEGATIVE_RULE_INDEX) {
                return Ok(RuleRef::Borrowed(rule));
            } else {
                number = number.wrapping_neg();
            }
        }

        // we have to repeat the preceding two checks, even though we
        // do them in findRule(), because the version of format() that
        // takes a long bypasses findRule() and goes straight to this
        // function.  This function does skip the fraction rules since
        // we know the value is an integer (it also skips the default
        // rule, since it's considered a fraction rule.  Skipping the
        // default rule in this function is also how we avoid infinite
        // recursion)

        // binary-search the rule list for the applicable rule
        // (a rule is used for all values from its base value to
        // the next rule's base value)
        let mut lo = 0usize;
        let mut hi = self.rules.len();
        if hi > 0 {
            while lo < hi {
                let mid = (lo + hi) >> 1;
                let rule_base_value = self.rules[mid].get_base_value();
                if rule_base_value == number {
                    return Ok(RuleRef::Borrowed(&self.rules[mid]));
                } else if rule_base_value > number {
                    hi = mid;
                } else {
                    lo = mid + 1;
                }
            }
            if hi == 0 {
                // bad rule set
                return Err(LuceneError::illegal_state(format!(
                    "The rule set {} cannot format the value {}",
                    String::from_utf16_lossy(&self.name),
                    number
                )));
            }
            let mut result = &self.rules[hi - 1];

            // use shouldRollBack() to see whether we need to invoke the
            // rollback rule (see shouldRollBack()'s documentation for
            // an explanation of the rollback rule).  If we do, roll back
            // one rule and return that one instead of the one we'd normally
            // return
            if result.should_roll_back(number)? {
                if hi == 1 {
                    // bad rule set
                    return Err(LuceneError::illegal_state(format!(
                        "The rule set {} cannot roll back from the rule '{}'",
                        String::from_utf16_lossy(&self.name),
                        String::from_utf16_lossy(&result.to_java_string(g)?)
                    )));
                }
                result = &self.rules[hi - 2];
            }
            return Ok(RuleRef::Borrowed(result));
        }
        // else use the default rule
        self.non_numerical_rule(&nn, DEFAULT_RULE_INDEX)
            .map(RuleRef::Borrowed)
            .ok_or_else(npe)
    }

    /// If this rule is a fraction rule set, this function is used by
    /// findRule() to select the most appropriate rule for formatting the
    /// number. Equivalent to `findFractionRuleSetRule(double number)`.
    ///
    /// # Errors
    ///
    /// `ArithmeticException("/ by zero")` when the least common multiple of
    /// the base values is 0.
    ///
    /// # Panics
    ///
    /// `ArrayIndexOutOfBoundsException` for a rule set without rules.
    fn find_fraction_rule_set_rule(&self, number: f64) -> Result<&NFRule> {
        // the obvious way to do this (multiply the value being formatted
        // by each rule's base value until you get an integral result)
        // doesn't work because of rounding error.  This method is more
        // accurate

        // find the least common multiple of the rules' base values
        // and multiply this by the number being formatted.  This is
        // all the precision we need, and we can do all of the rest
        // of the math using integer arithmetic
        let mut least_common_multiple =
            self.rules[array_index(0, self.rules.len())].get_base_value();
        for rule in &self.rules[1..] {
            least_common_multiple = Self::lcm(least_common_multiple, rule.get_base_value())?;
        }
        let numerator = java_math_round(number * least_common_multiple as f64);

        // for each rule, do the following...
        let mut difference = i64::MAX;
        let mut winner = 0usize;
        for (i, rule) in self.rules.iter().enumerate() {
            // "numerator" is the numerator of the fraction is the
            // denominator is the LCD.  The numerator if the the rule's
            // base value is the denominator is "numerator" times the
            // base value divided by the LCD.  Here we check to see if
            // that's an integer, and if not, how close it is to being
            // an integer.
            if least_common_multiple == 0 {
                return Err(crate::jdk::arithmetic("/ by zero"));
            }
            let mut temp_difference = numerator
                .wrapping_mul(rule.get_base_value())
                .wrapping_rem(least_common_multiple);

            // normalize the result of the above calculation: we want
            // the numerator's distance from the CLOSEST multiple
            // of the LCD
            if least_common_multiple.wrapping_sub(temp_difference) < temp_difference {
                temp_difference = least_common_multiple.wrapping_sub(temp_difference);
            }

            // if this is as close as we've come, keep track of how close
            // that is, and the line number of the rule that did it.  If
            // we've scored a direct hit, we don't have to look at any more
            // rules
            if temp_difference < difference {
                difference = temp_difference;
                winner = i;
                if difference == 0 {
                    break;
                }
            }
        }

        // if we have two successive rules that both have the winning base
        // value, then the first one (the one we found above) is used if
        // the numerator of the fraction is 1 and the second one is used if
        // the numerator of the fraction is anything else (this lets us
        // do things like "one third"/"two thirds" without having to define
        // a whole bunch of extra rule sets)
        if winner + 1 < self.rules.len()
            && self.rules[winner + 1].get_base_value() == self.rules[winner].get_base_value()
        {
            let base = self.rules[winner].get_base_value() as f64;
            if java_math_round(number * base) < 1 || java_math_round(number * base) >= 2 {
                winner += 1;
            }
        }

        // finally, return the winning rule
        Ok(&self.rules[winner])
    }

    /// Calculates the least common multiple of x and y (binary gcd algorithm
    /// from Knuth, "The Art of Computer Programming," vol. 2, 1st ed., pp.
    /// 298-299). Equivalent to `lcm(long x, long y)` (private static);
    /// `lcm(0, 0)` loops forever, as in Java.
    fn lcm(x: i64, y: i64) -> Result<i64> {
        let mut x1 = x;
        let mut y1 = y;

        let mut p2 = 0i32;
        while (x1 & 1) == 0 && (y1 & 1) == 0 {
            p2 = p2.wrapping_add(1);
            x1 >>= 1;
            y1 >>= 1;
        }

        let mut t: i64 = if (x1 & 1) == 1 { y1.wrapping_neg() } else { x1 };

        while t != 0 {
            while (t & 1) == 0 {
                t >>= 1;
            }
            if t > 0 {
                x1 = t;
            } else {
                y1 = t.wrapping_neg();
            }
            t = x1.wrapping_sub(y1);
        }

        let gcd = x1.wrapping_shl(p2 as u32);

        // x * y == gcd(x, y) * lcm(x, y)
        if gcd == 0 {
            return Err(crate::jdk::arithmetic("/ by zero"));
        }
        Ok(x.wrapping_div(gcd).wrapping_mul(y))
    }

    //-----------------------------------------------------------------------
    // parsing
    //-----------------------------------------------------------------------

    /// Parses a string: tries the non-numerical rules not yet executed, then
    /// the regular rules from the most significant down, and keeps the
    /// longest match. Equivalent to `parse(String text, ParsePosition
    /// parsePosition, double upperBound, int nonNumericalExecutedRuleMask,
    /// int recursionCount)`.
    ///
    /// # Errors
    ///
    /// `IllegalStateException("Recursion limit exceeded when applying
    /// ruleSet <name>")`; the errors of the rules.
    pub(crate) fn parse(
        &self,
        text: &[u16],
        parse_position: &mut ParsePosition,
        upper_bound: f64,
        non_numerical_executed_rule_mask: i32,
        recursion_count: i32,
        g: &RuleGraph,
    ) -> Result<JavaNumber> {
        // try matching each rule in the rule set against the text being
        // parsed.  Whichever one matches the most characters is the one
        // that determines the value we return.
        let mut high_water_mark = ParsePosition::new(0);
        let mut result = JavaNumber::Long(0);

        // dump out if we've reached the recursion limit
        if recursion_count >= RECURSION_LIMIT {
            return Err(self.recursion_limit_exceeded());
        }

        // dump out if there's no text to parse
        if text.is_empty() {
            return Ok(result);
        }

        let mut non_numerical_executed_rule_mask = non_numerical_executed_rule_mask;
        // Try each of the negative rules, fraction rules, infinity rules and NaN rules
        let nn = *lock(&self.non_numerical_rules);
        for (non_numerical_rule_idx, rule) in nn.iter().enumerate() {
            if let Some(rule) = rule.map(|i| &self.special_rules[i]) {
                if ((non_numerical_executed_rule_mask >> non_numerical_rule_idx) & 1) == 0 {
                    // Mark this rule as being executed so that we don't try to execute it again.
                    non_numerical_executed_rule_mask |= 1 << non_numerical_rule_idx;

                    let temp_result = rule.do_parse(
                        text,
                        parse_position,
                        false,
                        upper_bound,
                        non_numerical_executed_rule_mask,
                        recursion_count + 1,
                        g,
                    )?;
                    if parse_position.get_index() > high_water_mark.get_index() {
                        result = temp_result;
                        high_water_mark.set_index(parse_position.get_index());
                    }
                    // commented out because the error-index API on ParsePosition isn't there in 1.1.x
                    //            if (parsePosition.getErrorIndex() > highWaterMark.getErrorIndex()) {
                    //                highWaterMark.setErrorIndex(parsePosition.getErrorIndex());
                    //            }
                    parse_position.set_index(0);
                }
            }
        }

        // finally, go through the regular rules one at a time.  We start
        // at the end of the list because we want to try matching the most
        // significant rule first (this helps ensure that we parse
        // "five thousand three hundred six" as
        // "(five thousand) (three hundred) (six)" rather than
        // "((five thousand three) hundred) (six)").  Skip rules whose
        // base values are higher than the upper bound (again, this helps
        // limit ambiguity by making sure the rules that match a rule's
        // are less significant than the rule containing the substitutions)/
        let is_fraction_rule_set = self.is_fraction_set();
        for rule in self.rules.iter().rev() {
            if high_water_mark.get_index() >= text.len() as i32 {
                break;
            }
            if !is_fraction_rule_set && rule.get_base_value() as f64 >= upper_bound {
                continue;
            }

            let temp_result = rule.do_parse(
                text,
                parse_position,
                is_fraction_rule_set,
                upper_bound,
                non_numerical_executed_rule_mask,
                recursion_count + 1,
                g,
            )?;
            if parse_position.get_index() > high_water_mark.get_index() {
                result = temp_result;
                high_water_mark.set_index(parse_position.get_index());
            }
            // commented out because the error-index API on ParsePosition isn't there in 1.1.x
            //            if (parsePosition.getErrorIndex() > highWaterMark.getErrorIndex()) {
            //                highWaterMark.setErrorIndex(parsePosition.getErrorIndex());
            //            }
            parse_position.set_index(0);
        }

        // finally, update the parse position we were passed to point to the
        // first character we didn't use, and return the result that
        // corresponds to that string of characters
        parse_position.set_index(high_water_mark.get_index());
        // commented out because the error-index API on ParsePosition isn't there in 1.1.x
        //        if (parsePosition.getIndex() == 0) {
        //            parsePosition.setErrorIndex(highWaterMark.getErrorIndex());
        //        }

        Ok(result)
    }

    /// Applies new decimal format symbols to the rules' decimal formats and
    /// re-selects the best fraction rules. Equivalent to
    /// `setDecimalFormatSymbols(DecimalFormatSymbols newSymbols)`.
    ///
    /// # Errors
    ///
    /// The errors of the decimal formats and of the formatter's symbols.
    pub(crate) fn set_decimal_format_symbols(
        &self,
        new_symbols: &DecimalFormatSymbols,
        g: &RuleGraph,
    ) -> Result<()> {
        for rule in &self.rules {
            rule.set_decimal_format_symbols(new_symbols)?;
        }
        // Switch the fraction rules to mirror the DecimalFormatSymbols.
        if let Some(fraction_rules) = &self.fraction_rules {
            let owner = g.owner();
            let mut nn = lock(&self.non_numerical_rules);
            for non_numerical_idx in IMPROPER_FRACTION_RULE_INDEX..=DEFAULT_RULE_INDEX {
                if nn[non_numerical_idx].is_some() {
                    for &rule in fraction_rules {
                        let current = nn[non_numerical_idx].expect("checked above");
                        if self.special_rules[current].get_base_value()
                            == self.special_rules[rule].get_base_value()
                        {
                            Self::set_best_fraction_rule(
                                &mut nn,
                                &self.special_rules,
                                non_numerical_idx,
                                rule,
                                &owner,
                            )?;
                        }
                    }
                }
            }
        }

        let nn = *lock(&self.non_numerical_rules);
        for rule in nn.iter().flatten().map(|&i| &self.special_rules[i]) {
            rule.set_decimal_format_symbols(new_symbols)?;
        }
        Ok(())
    }
}
