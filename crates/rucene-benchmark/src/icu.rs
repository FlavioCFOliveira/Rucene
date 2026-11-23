//! The part of icu4j 77.1 the benchmark uses: `RuleBasedNumberFormat(Locale.ROOT,
//! RuleBasedNumberFormat.SPELLOUT)`.
//!
//! Per the user's decision on rmp #611, the root locale's spellout rules are
//! ported, not the RBNF engine: the root `%spellout-numbering` rule set is
//! `0: =#,##0.#=; -x: −>%spellout-numbering>;` (observed with icu4j 77.1,
//! finding 4449 on #611), so a `long` formats as its decimal digits grouped
//! by three with `,` (the root `DecimalFormat` `#,##0.#`), a negative one
//! prefixed with U+2212 MINUS SIGN; `Long.MIN_VALUE`, whose absolute value
//! has no `long`, is formatted by the rule set's `DecimalFormat` instead,
//! with its ASCII `-` (`RuleBasedNumberFormat.java:1980-1982`).

/// Equivalent to `com.ibm.icu.text.RuleBasedNumberFormat` built as
/// `new RuleBasedNumberFormat(Locale.ROOT, RuleBasedNumberFormat.SPELLOUT)`.
#[derive(Clone, Copy, Debug, Default)]
pub struct RuleBasedNumberFormat;

/// `#,##0.#` of a non-negative integer: digits grouped by three with `,`.
fn grouped(digits: &str) -> String {
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    let first = digits.len() % 3;
    for (i, c) in digits.chars().enumerate() {
        if i != 0 && (i + 3 - first) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

impl RuleBasedNumberFormat {
    /// `new RuleBasedNumberFormat(Locale.ROOT, RuleBasedNumberFormat.SPELLOUT)`.
    pub fn root_spellout() -> Self {
        RuleBasedNumberFormat
    }

    /// `format(long number)`.
    pub fn format(&self, number: i64) -> String {
        if number == i64::MIN {
            // `getDecimalFormat().format(number)`: the plain decimal format.
            return format!("-{}", grouped(&number.unsigned_abs().to_string()));
        }
        if number < 0 {
            // `-x: −>%spellout-numbering>;`
            return format!("\u{2212}{}", self.format(-number));
        }
        // `0: =#,##0.#=;`
        grouped(&number.to_string())
    }
}
