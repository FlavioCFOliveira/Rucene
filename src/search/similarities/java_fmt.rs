//! Java number rendering, reproduced for the strings similarities put into
//! their `toString()` and into every [`Explanation`] description.
//!
//! [`Explanation`]: super::Explanation
//!
//! Lucene builds explanation descriptions by string concatenation, so the exact
//! text depends on `Float.toString`, `Double.toString` and `Formatter`'s `%f`.
//! Rust's own float rendering differs from all three, and the explanation tree
//! is part of the contract this port has to reproduce, so the three renderings
//! are implemented here rather than approximated:
//!
//! * Rust's `Display` prints `800` where Java prints `800.0`, and prints
//!   `0.0000000001` where Java prints `1.0E-10`.
//! * Rust's `{:.6}` rounds ties to even where Java's `%f` rounds ties away from
//!   zero (`java.math.RoundingMode.HALF_UP`).

#![deny(unsafe_code)]

/// Renders an `f32` the way `java.lang.Float.toString(float)` does.
///
/// Java uses the shortest decimal that round-trips — which is also what Rust's
/// `Display` produces — but always keeps a fraction digit, and switches to
/// "computerized scientific notation" when the magnitude leaves
/// `[10^-3, 10^7)`.
pub(crate) fn float_to_string(value: f32) -> String {
    if value.is_nan() {
        return "NaN".to_string();
    }
    if value.is_infinite() {
        return if value > 0.0 { "Infinity" } else { "-Infinity" }.to_string();
    }
    let magnitude = f64::from(value.abs());
    if magnitude != 0.0 && !(1e-3..1e7).contains(&magnitude) {
        return with_fraction_digit(&format!("{value:E}"));
    }
    with_fraction_digit(&format!("{value}"))
}

/// Renders an `f64` the way `java.lang.Double.toString(double)` does.
///
/// The rules are the same as [`float_to_string`], applied to the shortest
/// decimal that round-trips as a `double`.
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

/// Renders `value` the way `String.format(Locale.ROOT, "%f", value)` does: six
/// fraction digits, no grouping separators, ties rounded away from zero.
///
/// `LMSimilarity`'s descendants build their names with `%f`
/// (`LMDirichletSimilarity.java:135`, `LMJelinekMercerSimilarity.java:117`,
/// `IndriDirichletSimilarity.java:101`), and those names reach `toString()`.
///
/// Rust's `{:.6}` would round ties to even; Java's `Formatter` rounds them away
/// from zero. The two disagree on values whose exact decimal expansion stops on
/// a `5` in the seventh fraction digit — every odd multiple of `1/128`, for
/// instance — so the digits are taken from the exact expansion and rounded
/// here instead.
pub(crate) fn format_f(value: f64) -> String {
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

    let mut digits: Vec<u8> = integer.bytes().chain(fraction.bytes().take(6)).collect();
    // Pad a fraction shorter than six digits, which cannot happen for the exact
    // expansion above but keeps the indexing below independent of it.
    while digits.len() < integer.len() + 6 {
        digits.push(b'0');
    }
    let round_up = fraction
        .as_bytes()
        .get(6)
        .is_some_and(|digit| *digit >= b'5');
    if round_up {
        let mut carry = true;
        for digit in digits.iter_mut().rev() {
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
            digits.insert(0, b'1');
        }
    }

    let rendered = String::from_utf8(digits).unwrap_or_default();
    let split = rendered.len() - 6;
    let sign = if value.is_sign_negative() { "-" } else { "" };
    format!("{sign}{}.{}", &rendered[..split], &rendered[split..])
}
