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
//! * Rust's `{:.6}` rounds the exact binary expansion, ties to even, where
//!   Java's `%f` rounds the `Double.toString` shortest digits half up.

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
/// fraction digits, no grouping separators.
///
/// `LMSimilarity`'s descendants build their names with `%f`
/// (`LMDirichletSimilarity.java:135`, `LMJelinekMercerSimilarity.java:117`,
/// `IndriDirichletSimilarity.java:98`), and those names reach `toString()`.
/// `Formatter` widens their `float` argument to `double`, so callers pass
/// `f64::from(x)`.
///
/// JDK 25's `Formatter` rounds the `Double.toString` shortest digits half up,
/// never the exact binary expansion; that algorithm is
/// [`crate::util::java_fmt::format_fixed`], used here with six digits.
pub(crate) fn format_f(value: f64) -> String {
    crate::util::java_fmt::format_fixed(value, 6)
}

#[cfg(test)]
mod tests {
    use super::format_f;
    use crate::search::similarities::{IndriDirichletSimilarity, LMDirichletSimilarity};

    /// Rows where rounding the exact binary expansion disagreed with JDK 25
    /// `String.format(Locale.ROOT, "%f", x)`, which rounds the
    /// `Double.toString` shortest digits half up; expected strings are the
    /// JDK 25.0.5 output. The last rows agreed before the fix and guard the
    /// shared paths (ties, `-0.0`).
    #[test]
    fn format_f_matches_jdk25_formatter() {
        let doubles: [(u64, &str); 9] = [
            (0x4227_e8ef_579c_7a60, "51346123726.239010"),
            (0x4306_ffa5_9090_4dd7, "809192005962170.900000"),
            (0xc2cd_d75e_0f85_6e9b, "-65621666499293.210000"),
            (0x44b5_2d02_c7e1_4af6, "100000000000000000000000.000000"),
            (0x3fc3_3333_3333_3333, "0.150000"),
            (0x8000_0000_0000_0000, "-0.000000"),
            (0x7ff8_0000_0000_0000, "NaN"),
            (0x7ff0_0000_0000_0000, "Infinity"),
            (0xfff0_0000_0000_0000, "-Infinity"),
        ];
        for (bits, expected) in doubles {
            assert_eq!(
                format_f(f64::from_bits(bits)),
                expected,
                "double {bits:#018x}"
            );
        }
        // `Formatter` widens a `Float` argument to `double`, as the
        // `LMSimilarity` callers do with `f64::from`.
        let floats: [(u32, &str); 5] = [
            (0x5b7e_c074, "71706248533245950.000000"),
            (0x6407_8678, "9999999778196308000000.000000"),
            (0x3c00_0000, "0.007813"),
            (0x3cc0_0000, "0.023438"),
            (0x3e19_999a, "0.150000"),
        ];
        for (bits, expected) in floats {
            let value = f64::from(f32::from_bits(bits));
            assert_eq!(format_f(value), expected, "float {bits:#010x}");
        }
    }

    /// `toString()` texts printed by the Lucene 10.5.0 jar on JDK 25.0.5.
    #[test]
    fn lm_similarity_names_match_lucene() {
        let dirichlet = LMDirichletSimilarity::with_mu(f32::from_bits(0x5b7e_c074))
            .expect("finite non-negative mu");
        assert_eq!(
            dirichlet.to_string(),
            "LM Dirichlet(71706248533245950.000000)"
        );
        let indri = IndriDirichletSimilarity::with_mu(f32::from_bits(0x6407_8678));
        assert_eq!(
            indri.to_string(),
            "LM IndriDirichlet(9999999778196308000000.000000)"
        );
    }
}
