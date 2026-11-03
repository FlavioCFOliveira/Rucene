//! Java number rendering, reproduced for the strings Lucene's `toString()`
//! bodies build by concatenation, and the JDK's lenient UTF-8 decoding,
//! reproduced for the strings Lucene builds from bytes
//! ([`java_utf8_to_string`]).
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
        if i > 0 && (bytes.len() - i).is_multiple_of(3) {
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

/// Decodes `bytes` as `new String(bytes, StandardCharsets.UTF_8)` does.
///
/// **This has no counterpart in Lucene 10.5.0**: Java delegates to the JDK,
/// whose decoder replaces every malformed sequence with U+FFFD and never
/// fails. Its notion of a malformed sequence is not Unicode's "maximal
/// subpart" that [`String::from_utf8_lossy`] follows — an encoded surrogate
/// such as `ED A0 80` is one replacement in Java and three in Rust — so the
/// JDK's rules (`java.lang.String.decodeUTF8_UTF16` and its `malformed3` /
/// `malformed4` helpers in OpenJDK 21-25) are reproduced here, and checked
/// against JDK 25 by the codec-header portability test.
///
/// Used by
/// [`check_index_header_suffix`](crate::codecs::codec_util::check_index_header_suffix)
/// and by [`DataInput::read_string`](crate::store::DataInput::read_string), whose
/// Java counterparts both build the string with that constructor
/// (`CodecUtil.java:383`, `DataInput.java:234`).
pub(crate) fn java_utf8_to_string(bytes: &[u8]) -> String {
    const REPL: u16 = 0xFFFD;
    fn is_not_continuation(b: u8) -> bool {
        b & 0xC0 != 0x80
    }
    fn is_malformed3(b1: u8, b2: u8, b3: u8) -> bool {
        (b1 == 0xE0 && b2 & 0xE0 == 0x80) || is_not_continuation(b2) || is_not_continuation(b3)
    }
    fn is_malformed3_2(b1: u8, b2: u8) -> bool {
        (b1 == 0xE0 && b2 & 0xE0 == 0x80) || is_not_continuation(b2)
    }
    fn is_malformed4(b2: u8, b3: u8, b4: u8) -> bool {
        is_not_continuation(b2) || is_not_continuation(b3) || is_not_continuation(b4)
    }
    fn is_malformed4_2(b1: u8, b2: u8) -> bool {
        (b1 == 0xF0 && !(0x90..=0xBF).contains(&b2))
            || (b1 == 0xF4 && b2 & 0xF0 != 0x80)
            || is_not_continuation(b2)
    }
    /// How many bytes of a malformed three-byte sequence are skipped.
    fn malformed3(src: &[u8], sp: usize) -> usize {
        let (b1, b2) = (src[sp], src[sp + 1]);
        if (b1 == 0xE0 && b2 & 0xE0 == 0x80) || is_not_continuation(b2) {
            1
        } else {
            2
        }
    }
    /// How many bytes of a malformed four-byte sequence are skipped.
    fn malformed4(src: &[u8], sp: usize) -> usize {
        let (b1, b2) = (src[sp], src[sp + 1]);
        if b1 > 0xF4
            || (b1 == 0xF0 && !(0x90..=0xBF).contains(&b2))
            || (b1 == 0xF4 && b2 & 0xF0 != 0x80)
            || is_not_continuation(b2)
        {
            1
        } else if is_not_continuation(src[sp + 2]) {
            2
        } else {
            3
        }
    }

    let mut out: Vec<u16> = Vec::with_capacity(bytes.len());
    let (mut sp, sl) = (0usize, bytes.len());
    while sp < sl {
        let b1 = bytes[sp];
        sp += 1;
        if b1 < 0x80 {
            out.push(u16::from(b1));
        } else if b1 >> 5 == 0b110 && b1 & 0x1E != 0 {
            if sp < sl {
                let b2 = bytes[sp];
                sp += 1;
                if is_not_continuation(b2) {
                    out.push(REPL);
                    sp -= 1;
                } else {
                    out.push((u16::from(b1 & 0x1F) << 6) | u16::from(b2 & 0x3F));
                }
                continue;
            }
            out.push(REPL);
            break;
        } else if b1 >> 4 == 0b1110 {
            if sp + 1 < sl {
                let (b2, b3) = (bytes[sp], bytes[sp + 1]);
                sp += 2;
                if is_malformed3(b1, b2, b3) {
                    out.push(REPL);
                    sp -= 3;
                    sp += malformed3(bytes, sp);
                } else {
                    let c = (u16::from(b1 & 0x0F) << 12)
                        | (u16::from(b2 & 0x3F) << 6)
                        | u16::from(b3 & 0x3F);
                    // An encoded surrogate is one replacement.
                    out.push(if (0xD800..=0xDFFF).contains(&c) {
                        REPL
                    } else {
                        c
                    });
                }
                continue;
            }
            if sp < sl && is_malformed3_2(b1, bytes[sp]) {
                out.push(REPL);
                continue;
            }
            out.push(REPL);
            break;
        } else if b1 >> 3 == 0b11110 {
            if sp + 2 < sl {
                let (b2, b3, b4) = (bytes[sp], bytes[sp + 1], bytes[sp + 2]);
                sp += 3;
                let uc = (u32::from(b1 & 0x07) << 18)
                    | (u32::from(b2 & 0x3F) << 12)
                    | (u32::from(b3 & 0x3F) << 6)
                    | u32::from(b4 & 0x3F);
                if is_malformed4(b2, b3, b4) || !(0x10000..=0x10FFFF).contains(&uc) {
                    out.push(REPL);
                    sp -= 4;
                    sp += malformed4(bytes, sp);
                } else {
                    let v = uc - 0x10000;
                    out.push(0xD800 | (v >> 10) as u16);
                    out.push(0xDC00 | (v & 0x3FF) as u16);
                }
                continue;
            }
            if b1 > 0xF4 || (sp < sl && is_malformed4_2(b1, bytes[sp])) {
                out.push(REPL);
                continue;
            }
            sp += 1;
            out.push(REPL);
            if sp < sl && is_not_continuation(bytes[sp]) {
                continue;
            }
            break;
        } else {
            out.push(REPL);
        }
    }
    // Every surrogate pushed above is half of a well-formed pair.
    String::from_utf16_lossy(&out)
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
