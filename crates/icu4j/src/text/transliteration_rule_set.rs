// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.text.TransliterationRuleSet`.

use std::sync::Arc;

use rucene::error::{LuceneError, Result};

use crate::jdk::to_rust_string;
use crate::text::replaceable::Replaceable;
use crate::text::transliteration_rule::TransliterationRule;
use crate::text::transliterator::Position;
use crate::text::unicode_matcher::{U_MATCH, U_PARTIAL_MATCH};
use crate::text::unicode_set::UnicodeSet;
use crate::text::utf16::UTF16;

/// A set of rules for a `RuleBasedTransliterator`. This set encodes the
/// transliteration in one direction from one set of characters or short
/// strings to another. A `RuleBasedTransliterator` consists of up to two
/// such sets, one for the forward direction, and one for the reverse.
///
/// A `TransliterationRuleSet` has one important operation, that of finding
/// a matching rule at a given point in the text. This is accomplished by
/// the [`transliterate`](Self::transliterate) method.
///
/// Equivalent to the package-private class
/// `com.ibm.icu.text.TransliterationRuleSet`.
#[derive(Debug, Default)]
pub struct TransliterationRuleSet {
    /// Vector of rules, in the order added.
    rule_vector: Vec<Arc<TransliterationRule>>,
    /// Length of the longest preceding context.
    max_context_length: i32,
    /// Sorted and indexed table of rules. This is created by `freeze()`
    /// from the rules in `rule_vector`. `rules.length >= ruleVector.size()`,
    /// and the references in `rules[]` are aliases of the references in
    /// `ruleVector`. A single rule in `ruleVector` is listed one or more
    /// times in `rules[]`.
    rules: Option<Vec<Arc<TransliterationRule>>>,
    /// Index table. For text having a first character c, compute x = c &
    /// 0xFF. Now use `rules[index[x]..index[x+1]-1]`. This index table is
    /// created by `freeze()`.
    index: Vec<i32>,
}

impl TransliterationRuleSet {
    /// `TransliterationRuleSet()`: an empty rule set.
    pub fn new() -> Self {
        Self {
            rule_vector: Vec::new(),
            max_context_length: 0,
            rules: None,
            index: Vec::new(),
        }
    }

    /// `getMaximumContextLength()`: the maximum context length.
    pub fn get_maximum_context_length(&self) -> i32 {
        self.max_context_length
    }

    /// `addRule(TransliterationRule rule)`: adds a rule to this set. Rules
    /// are added in order, and order is significant.
    pub fn add_rule(&mut self, rule: TransliterationRule) {
        let len = rule.get_ante_context_length();
        self.rule_vector.push(Arc::new(rule));
        if len > self.max_context_length {
            self.max_context_length = len;
        }

        self.rules = None;
    }

    /// `freeze()`: closes this rule set to further additions, checks it for
    /// masked rules, and indexes it for optimal performance.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` if some rules are masked.
    #[allow(clippy::needless_range_loop)] // indexed loop mirrors the Java for-loop
    pub fn freeze(&mut self) -> Result<()> {
        // Construct the rule array and index table.  We reorder the
        // rules by sorting them into 256 bins.  Each bin contains all
        // rules matching the index value for that bin.  A rule
        // matches an index value if string whose first key character
        // has a low byte equal to the index value can match the rule.
        //
        // Each bin contains zero or more rules, in the same order
        // they were found originally.  However, the total rules in
        // the bins may exceed the number in the original vector,
        // since rules that have a variable as their first key
        // character will generally fall into more than one bin.
        //
        // That is, each bin contains all rules that either have that
        // first index value, or have a set containing the index
        // value.
        let n = self.rule_vector.len();
        let mut index = vec![0i32; 257]; // [sic] Extra entry used as sentinel
        let mut v: Vec<Arc<TransliterationRule>> = Vec::with_capacity(2 * n);

        // Precompute the index values.  This saves a LOT of time.
        let mut index_value = vec![0i32; n];
        for (j, r) in self.rule_vector.iter().enumerate() {
            index_value[j] = r.get_index_value();
        }
        for x in 0..256 {
            index[x] = v.len() as i32;
            for j in 0..n {
                if index_value[j] >= 0 {
                    if index_value[j] == x as i32 {
                        v.push(self.rule_vector[j].clone());
                    }
                } else {
                    // If the indexValue is < 0, then the first key character
                    // is a set, and we must use the more time-consuming
                    // matchesIndexValue check.  In practice this happens
                    // rarely, so we seldom tread this code path.
                    let r = &self.rule_vector[j];
                    if r.matches_index_value(x as i32) {
                        v.push(r.clone());
                    }
                }
            }
        }
        index[256] = v.len() as i32;

        // Freeze things into an array.
        let rules = v;

        let mut errors: Option<Vec<u16>> = None;

        // Check for masking.  This is MUCH faster than our old check,
        // which was each rule against each following rule, since we
        // only have to check for masking within each bin now.  It's
        // 256*O(n2^2) instead of O(n1^2), where n1 is the total rule
        // count, and n2 is the per-bin rule count.  But n2<<n1, so
        // it's a big win.
        for x in 0..256usize {
            let mut j = index[x];
            while j < index[x + 1] - 1 {
                let r1 = &rules[j as usize];
                for k in (j + 1)..index[x + 1] {
                    let r2 = &rules[k as usize];
                    if r1.masks(r2) {
                        let e = errors.get_or_insert_with(Vec::new);
                        if !e.is_empty() {
                            e.push(b'\n' as u16);
                        }
                        e.extend_from_slice(&crate::jdk::units("Rule "));
                        e.extend_from_slice(&r1.to_java_string()?);
                        e.extend_from_slice(&crate::jdk::units(" masks "));
                        e.extend_from_slice(&r2.to_java_string()?);
                    }
                }
                j += 1;
            }
        }

        self.index = index;
        self.rules = Some(rules);

        if let Some(errors) = errors {
            return Err(LuceneError::illegal_argument(to_rust_string(&errors)));
        }
        Ok(())
    }

    /// `transliterate(Replaceable text, Transliterator.Position pos,
    /// boolean incremental)`: transliterates `text` starting at `pos.start`,
    /// up to but not including `pos.limit`, with at most one rule. Returns
    /// true if the transliteration should continue (a match, or no match
    /// with `pos.start` advanced), false on a partial match at the end of
    /// an incremental text.
    ///
    /// # Errors
    ///
    /// The errors of the rules.
    ///
    /// # Panics
    ///
    /// Java's `NullPointerException` before [`freeze`](Self::freeze).
    pub fn transliterate(
        &self,
        text: &dyn Replaceable,
        pos: &mut Position,
        incremental: bool,
    ) -> Result<bool> {
        let rules = self
            .rules
            .as_ref()
            .unwrap_or_else(|| panic!("java.lang.NullPointerException: Cannot load from object array because \"this.rules\" is null"));
        let index_byte = (text.char32_at(pos.start) & 0xFF) as usize;
        for i in self.index[index_byte]..self.index[index_byte + 1] {
            let m = rules[i as usize].match_and_replace(text, pos, incremental)?;
            match m {
                U_MATCH => return Ok(true),
                U_PARTIAL_MATCH => return Ok(false),
                _ => {}
            }
        }
        // No match or partial match from any rule
        pos.start += UTF16::get_char_count(text.char32_at(pos.start));
        Ok(true)
    }

    /// `toRules(boolean escapeUnprintable)`: a representation of this rule
    /// set as rule source.
    ///
    /// # Errors
    ///
    /// The errors of [`TransliterationRule::to_rule`].
    pub fn to_rules(&self, escape_unprintable: bool) -> Result<Vec<u16>> {
        let mut rule_source = Vec::new();
        for (i, r) in self.rule_vector.iter().enumerate() {
            if i != 0 {
                rule_source.push(b'\n' as u16);
            }
            rule_source.extend_from_slice(&r.to_rule(escape_unprintable)?);
        }
        Ok(rule_source)
    }

    /// `addSourceTargetSet(UnicodeSet filter, UnicodeSet sourceSet,
    /// UnicodeSet targetSet)`.
    ///
    /// # Errors
    ///
    /// The errors of the `UnicodeSet` operations.
    pub fn add_source_target_set(
        &self,
        filter: &UnicodeSet,
        source_set: &mut UnicodeSet,
        target_set: &mut UnicodeSet,
    ) -> Result<()> {
        let mut current_filter = UnicodeSet::from_set(filter);
        let mut revisiting = UnicodeSet::new();
        for r in &self.rule_vector {
            revisiting.clear()?;
            r.add_source_target_set(&current_filter, source_set, target_set, &mut revisiting)?;
            current_filter.add_all(&revisiting)?;
        }
        Ok(())
    }
}
