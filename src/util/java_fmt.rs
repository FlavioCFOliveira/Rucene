//! Java number rendering, reproduced for the strings Lucene's `toString()`
//! bodies build by concatenation.
//!
//! Lucene's `toString()` implementations are plain string concatenation, so the
//! exact text they produce depends on how Java renders the values that go into
//! them — `Double.toString`, `Integer.toHexString` and `java.util.Formatter`'s
//! `%f` and `%,d` conversions. Rust's own rendering differs from all four, and
//! those strings are observable (`CheckIndex` output, `IndexWriter` info-stream
//! messages, exception text, Luke), so the renderings are reproduced here
//! rather than approximated:
//!
//! * Rust's `Display` prints `800` where Java prints `800.0`, and prints
//!   `0.0000000001` where Java prints `1.0E-10`.
//! * Rust's `{:.2}` rounds ties to even where Java's `%f` rounds ties away from
//!   zero (`java.math.RoundingMode.HALF_UP`).
//! * Rust has no equivalent of `%,d`, which groups the integer digits in threes
//!   under `Locale.ROOT`.
//! * Rust's `{:x}` on a negative `i32` prints a `-` sign, where Java's
//!   `Integer.toHexString` renders the two's-complement bit pattern unsigned.
//!
//! # Relationship to `search::similarities::java_fmt`
//!
//! `crate::search::similarities::java_fmt` already reproduces `Float.toString`,
//! `Double.toString` and `%f` for the explanation tree. That module sits under
//! `search`, and Lucene's `util` package does not depend on `search`, so this
//! module does not call into it; consolidating the two would mean moving the
//! `search` copy, which is outside the scope of the task that introduced this
//! file.

#![deny(unsafe_code)]

/// Renders an `f64` the way `java.lang.Double.toString(double)` does.
///
/// Java uses the shortest decimal that round-trips — which is also what Rust's
/// `Display` produces — but always keeps at least one fraction digit, and
/// switches to "computerized scientific notation" when the magnitude leaves
/// `[10^-3, 10^7)`.
pub(crate) fn double_to_string(value: f64) -> String {
    if value.is_nan() {
        return "NaN".to_string();
    }
    if value.is_infinite() {
        return if value > 0.0 { "Infinity" } else { "-Infinity" }.to_string();
    }
    let magnitude = value.abs();
    if magnitude != 0.0 && !(1e-3..1e7).contains(&magnitude) {
        return with_fraction_digit(&format!("{value:E}"));
    }
    with_fraction_digit(&format!("{value}"))
}

/// Adds the fraction digit Java always prints and Rust omits: `800` becomes
/// `800.0`, and the mantissa of `1E-10` becomes `1.0E-10`.
fn with_fraction_digit(rendered: &str) -> String {
    match rendered.find('E') {
        Some(exponent) => {
            let (mantissa, exponent) = rendered.split_at(exponent);
            if mantissa.contains('.') {
                rendered.to_string()
            } else {
                format!("{mantissa}.0{exponent}")
            }
        }
        None => {
            if rendered.contains('.') {
                rendered.to_string()
            } else {
                format!("{rendered}.0")
            }
        }
    }
}

/// Renders `value` the way `String.format(Locale.ROOT, "%.<digits>f", value)`
/// does: exactly `digits` fraction digits, no grouping separators, ties rounded
/// away from zero.
///
/// Rust's `{:.n}` would round ties to even; Java's `Formatter` rounds them away
/// from zero (`RoundingMode.HALF_UP`). The two disagree on values whose exact
/// decimal expansion stops on a `5` in the first dropped position, so the digits
/// are taken from the exact expansion and rounded here instead.
pub(crate) fn format_fixed(value: f64, digits: usize) -> String {
    if value.is_nan() {
        return "NaN".to_string();
    }
    if value.is_infinite() {
        return if value > 0.0 { "Infinity" } else { "-Infinity" }.to_string();
    }
    // A finite `f64` has an exact decimal expansion of at most 1074 fraction
    // digits (the smallest subnormal), so this rendering is exact, never
    // rounded, and the rounding below is therefore a single rounding step.
    let exact = format!("{:.*}", 1074, value.abs());
    let (integer, fraction) = match exact.split_once('.') {
        Some(split) => split,
        None => (exact.as_str(), ""),
    };

    let mut rendered: Vec<u8> = integer
        .bytes()
        .chain(fraction.bytes().take(digits))
        .collect();
    // Pad a fraction shorter than `digits`, which cannot happen for the exact
    // expansion above but keeps the indexing below independent of it.
    while rendered.len() < integer.len() + digits {
        rendered.push(b'0');
    }
    let round_up = fraction
        .as_bytes()
        .get(digits)
        .is_some_and(|digit| *digit >= b'5');
    if round_up {
        let mut carry = true;
        for digit in rendered.iter_mut().rev() {
            if !carry {
                break;
            }
            if *digit == b'9' {
                *digit = b'0';
            } else {
                *digit += 1;
                carry = false;
            }
        }
        if carry {
            rendered.insert(0, b'1');
        }
    }

    let rendered = String::from_utf8(rendered).unwrap_or_default();
    let split = rendered.len() - digits;
    let sign = if value.is_sign_negative() { "-" } else { "" };
    if digits == 0 {
        format!("{sign}{rendered}")
    } else {
        format!("{sign}{}.{}", &rendered[..split], &rendered[split..])
    }
}

/// Renders `value` the way `String.format(Locale.ROOT, "%,d", value)` does:
/// the decimal digits grouped in threes by a comma, which is `Locale.ROOT`'s
/// grouping separator and grouping size.
pub(crate) fn format_grouped(value: i64) -> String {
    let negative = value < 0;
    // `i64::MIN` has no positive counterpart, so group the digits of the
    // rendered magnitude rather than negating the value.
    let digits = value.unsigned_abs().to_string();
    let bytes = digits.as_bytes();
    let mut grouped = String::with_capacity(digits.len() + digits.len() / 3 + 1);
    if negative {
        grouped.push('-');
    }
    for (i, byte) in bytes.iter().enumerate() {
        if i > 0 && (bytes.len() - i) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(*byte as char);
    }
    grouped
}

/// Renders `value` the way `java.lang.Integer.toHexString(int)` does: the
/// two's-complement bit pattern as unsigned lowercase hexadecimal, with no
/// leading zeros and no sign.
pub(crate) fn to_hex_string(value: i32) -> String {
    format!("{:x}", value as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn double_to_string_keeps_a_fraction_digit_and_switches_to_scientific() {
        assert_eq!(double_to_string(800.0), "800.0");
        assert_eq!(double_to_string(0.0), "0.0");
        assert_eq!(double_to_string(-0.0), "-0.0");
        assert_eq!(double_to_string(1e-10), "1.0E-10");
        assert_eq!(double_to_string(1e7), "1.0E7");
        assert_eq!(double_to_string(9_999_999.0), "9999999.0");
        assert_eq!(double_to_string(0.001), "0.001");
        assert_eq!(double_to_string(f64::NAN), "NaN");
        assert_eq!(double_to_string(f64::INFINITY), "Infinity");
    }

    #[test]
    fn format_fixed_rounds_ties_away_from_zero() {
        // Rust's own `{:.2}` renders 0.125 as "0.12" (ties to even); Java's
        // "%.2f" renders it as "0.13".
        assert_eq!(format!("{:.2}", 0.125_f64), "0.12");
        assert_eq!(format_fixed(0.125, 2), "0.13");
        assert_eq!(format_fixed(0.0, 2), "0.00");
        assert_eq!(format_fixed(9.999, 2), "10.00");
        assert_eq!(format_fixed(-0.125, 2), "-0.13");
        assert_eq!(format_fixed(1.5, 0), "2");
    }

    #[test]
    fn format_grouped_groups_in_threes() {
        assert_eq!(format_grouped(0), "0");
        assert_eq!(format_grouped(999), "999");
        assert_eq!(format_grouped(1000), "1,000");
        assert_eq!(format_grouped(1_234_567), "1,234,567");
        assert_eq!(format_grouped(-1_234_567), "-1,234,567");
        assert_eq!(format_grouped(i64::MIN), "-9,223,372,036,854,775,808");
    }

    #[test]
    fn to_hex_string_is_unsigned() {
        assert_eq!(to_hex_string(0), "0");
        assert_eq!(to_hex_string(255), "ff");
        assert_eq!(to_hex_string(-1), "ffffffff");
    }
}
